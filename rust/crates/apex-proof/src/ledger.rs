//! File-backed ledger: an append-only, one-record-per-line JSONL file where
//! every line is signed with ed25519 and chained by SHA-256.
//!
//! ## Persisted line (byte-exact)
//!
//! ```json
//! {"record":{...},"signature":"<128 lowercase hex>","pubkey":"<64 lowercase hex>"}
//! ```
//!
//! Verification does NOT merely parse the line: it re-serializes the parsed
//! record with the parsed signature/pubkey and requires the on-disk bytes to
//! equal that canonical serialization exactly. Unknown injected fields,
//! whitespace changes, reordered keys and uppercase-hex edits are therefore
//! rejected, not silently accepted (attacker-injected annotations cannot
//! survive, and hex case ambiguity has exactly one legal encoding).
//!
//! The signature covers the 32-byte record hash (which itself covers the whole
//! canonical record, including `prevHash`), so a valid line binds payload,
//! chain position and signer with one signature check.
//!
//! ## Fail-closed appends
//!
//! [`Ledger::open`] verifies the whole existing file before the handle is
//! usable; every [`Ledger::append`] re-verifies the file **tail beyond the last
//! verified offset** (a tampered history is a typed error, never a silent
//! append; the tail check keeps appends O(new bytes) instead of O(history)).
//! Writes are `sync_data`-durable; a failed write is rolled back by truncation,
//! and a failed rollback is reported loudly instead of being discarded.
//!
//! The ledger is single-writer by contract: concurrent handles must be
//! externally coordinated (the JSONL format itself stays append-safe against
//! torn single writes, and any torn write is detected as [`LedgerError::Truncated`]).
//!
//! On unix the file is created with mode `0600` (read/write only for the
//! owner) using [`std::os::unix::fs::OpenOptionsExt`]; existing files keep
//! their current mode.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use apex_core::PlatformId;
use ed25519_dalek::{Signature, Signer, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::chain::{self, ChainValidator, ZERO_HASH};
use crate::errors::LedgerError;
use crate::record::LedgerRecord;
use crate::Keypair;

/// Serialized shape of one persisted line: `{"record":...,"signature":hex,
/// "pubkey":hex}` (field order is part of the byte-exact format).
#[derive(Debug, Serialize)]
struct PersistedLine<'a> {
    record: &'a LedgerRecord,
    signature: String,
    pubkey: String,
}

/// Owned mirror of [`PersistedLine`] used when reading.
#[derive(Debug, Deserialize)]
struct StoredLine {
    record: LedgerRecord,
    signature: String,
    pubkey: String,
}

/// Result of verifying a full file: the records, the chain tip and the byte
/// length that was verified (append fast-path offset).
struct Verified {
    records: Vec<LedgerRecord>,
    last_hash: [u8; 32],
    verified_offset: u64,
}

/// An appended-to file ledger bound to one ed25519 key.
///
/// The key is fixed at [`Ledger::open`]; mixing keys in one file is a
/// verification error, not a feature.
#[derive(Debug)]
pub struct Ledger {
    path: PathBuf,
    file: File,
    keypair: Keypair,
    verifying_key: VerifyingKey,
    records: Vec<LedgerRecord>,
    last_hash: [u8; 32],
    next_seq: u64,
    /// Byte length of the file that has been fully verified so far.
    verified_offset: u64,
}

impl Ledger {
    /// Open (or create) the ledger at `path` and verify every existing line
    /// against `keypair`'s public key before returning.
    pub fn open(path: impl AsRef<Path>, keypair: Keypair) -> Result<Self, LedgerError> {
        let path = path.as_ref().to_path_buf();
        let verifying_key = keypair.verifying_key();
        let file = open_append(&path)?;
        let verified = read_and_verify(&path, &verifying_key)?;
        let next_seq = u64::try_from(verified.records.len())
            .map_err(|_| LedgerError::Validation("ledger has too many records".into()))?;
        Ok(Ledger {
            path,
            file,
            keypair,
            verifying_key,
            records: verified.records,
            last_hash: verified.last_hash,
            next_seq,
            verified_offset: verified.verified_offset,
        })
    }

    /// Verify a ledger file against `expected_pubkey` without opening it for
    /// appends. Rejects (with the 1-based line of the first offender) a
    /// tampered payload/seq/prevHash/hash, a bad signature, a non-canonical
    /// line encoding, a truncated final line, swapped or reordered lines, a
    /// different public key, a sequence gap or duplicate, a broken genesis
    /// link, and an invalid billing proof.
    pub fn verify(
        path: impl AsRef<Path>,
        expected_pubkey: &VerifyingKey,
    ) -> Result<(), LedgerError> {
        read_and_verify(path.as_ref(), expected_pubkey).map(|_| ())
    }

    /// Seal, sign and append one record after re-verifying the on-disk tail.
    /// Returns the sealed record. On any error the file and the handle state
    /// are unchanged (or the partial write is rolled back and reported).
    pub fn append(&mut self, draft: LedgerRecord) -> Result<LedgerRecord, LedgerError> {
        self.refresh_from_disk()?;
        let mut record = draft;
        record.seal(self.next_seq, self.last_hash)?;
        // Same-record validation as the read path: chain rules AND money
        // invariants (a forged BillingProof must never reach the disk).
        let mut validator = ChainValidator::resume(self.last_hash, self.next_seq);
        validator.check(&record, self.records.len() + 1)?;
        let next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or_else(|| LedgerError::Validation("ledger sequence space exhausted".into()))?;
        let signature = self.keypair.sign(&record.hash());
        let bytes = canonical_line(&record, &signature, &self.verifying_key)?;
        let end = self.file.metadata()?.len();
        if let Err(error) = self
            .file
            .write_all(&bytes)
            .and_then(|()| self.file.sync_data())
        {
            return Err(undo_partial_write(&self.file, end, error));
        }
        self.last_hash = record.hash();
        self.next_seq = next_seq;
        self.verified_offset = end + bytes.len() as u64;
        self.records.push(record.clone());
        Ok(record)
    }

    /// Append a `rollbackPerformed` record for `rolled_back_seq`.
    ///
    /// Refused with a typed error when the target sequence does not exist
    /// ([`LedgerError::UnknownRollbackTarget`]), belongs to another platform or
    /// ad unit / is not a change ([`LedgerError::RollbackTargetMismatch`]), is
    /// itself a rollback ([`LedgerError::RollbackOfRollback`]) or was already
    /// rolled back ([`LedgerError::DuplicateRollback`]).
    pub fn append_rollback(
        &mut self,
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: &str,
        rolled_back_seq: u64,
        reason: &str,
    ) -> Result<LedgerRecord, LedgerError> {
        self.refresh_from_disk()?;
        chain::check_rollback_target(self.records.iter(), platform, ad_unit_id, rolled_back_seq)?;
        self.append(LedgerRecord::rollback_performed(
            at_unix_ms,
            platform,
            ad_unit_id,
            rolled_back_seq,
            reason,
        ))
    }

    /// Path of the ledger file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of records in the ledger.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True for an empty (fresh) ledger.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Record at `index`, in append order.
    pub fn get(&self, index: usize) -> Option<&LedgerRecord> {
        self.records.get(index)
    }

    /// All records, in append order.
    pub fn records(&self) -> &[LedgerRecord] {
        &self.records
    }

    /// The public key this ledger signs with.
    pub fn public_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    /// Fail closed: the file on disk must verify with our key and continue our
    /// chain. The already-verified prefix is trusted (it was fully verified at
    /// open and is only extended below); any NEW bytes are verified here, so a
    /// rewritten history or tampered tail is caught before the write.
    fn refresh_from_disk(&mut self) -> Result<(), LedgerError> {
        let len = self.file.metadata()?.len();
        if len == self.verified_offset {
            return Ok(());
        }
        if len < self.verified_offset {
            // The file shrank below what we fully verified: it was cut/truncated.
            let whole = std::fs::read(&self.path).unwrap_or_default();
            let complete = whole.iter().filter(|byte| **byte == b'\n').count();
            return Err(LedgerError::Truncated { line: complete + 1 });
        }
        let mut reader = File::open(&self.path)?;
        reader.seek(SeekFrom::Start(self.verified_offset))?;
        let mut tail = Vec::new();
        reader.read_to_end(&mut tail)?;
        let base_line = self.records.len();
        if tail.last() != Some(&b'\n') {
            let complete = tail.iter().filter(|byte| **byte == b'\n').count();
            return Err(LedgerError::Truncated {
                line: base_line + complete + 1,
            });
        }
        let mut validator = ChainValidator::resume(self.last_hash, self.next_seq);
        let mut tail_records = Vec::new();
        let mut tail_error: Option<LedgerError> = None;
        for (index, chunk) in tail[..tail.len() - 1]
            .split(|byte| *byte == b'\n')
            .enumerate()
        {
            let line = base_line + index + 1;
            match verify_stored_chunk(chunk, line, &self.verifying_key)
                .and_then(|record| validator.check(&record, line).map(|()| record))
            {
                Ok(record) => tail_records.push(record),
                Err(error) => {
                    tail_error = Some(error);
                    break;
                }
            }
        }
        if let Some(error) = tail_error {
            // The verified prefix is only trusted by OFFSET, so a tamper that changes
            // the prefix LENGTH can slice a line in half and surface as `Malformed`
            // here. Fall back to a full re-verification to produce the truthful
            // taxonomy (e.g. HashMismatch on the tampered line) before giving up.
            return match read_and_verify(&self.path, &self.verifying_key) {
                Err(truth) => Err(truth),
                Ok(verified) if verified.records.len() > self.records.len() => {
                    Err(LedgerError::StateDiverged {
                        line: self.records.len() + 1,
                    })
                }
                Ok(_) => Err(error),
            };
        }
        for record in tail_records {
            self.last_hash = record.hash();
            self.next_seq = self.next_seq.saturating_add(1);
            self.records.push(record);
        }
        self.verified_offset = len;
        Ok(())
    }
}

fn open_append(path: &Path) -> Result<File, LedgerError> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

/// The one and only legal byte encoding of a persisted line (with newline).
pub(crate) fn canonical_line(
    record: &LedgerRecord,
    signature: &Signature,
    pubkey: &VerifyingKey,
) -> Result<Vec<u8>, LedgerError> {
    let line = PersistedLine {
        record,
        signature: hex::encode(signature.to_bytes()),
        pubkey: hex::encode(pubkey.to_bytes()),
    };
    let mut bytes = serde_json::to_vec(&line)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Roll back a partial write; a failed rollback is surfaced loudly instead of
/// being discarded (the ledger must never pretend the file is clean).
fn undo_partial_write(file: &File, end: u64, original: std::io::Error) -> LedgerError {
    match file.set_len(end) {
        Ok(()) => LedgerError::Io(original),
        Err(trunc) => LedgerError::Io(std::io::Error::new(
            trunc.kind(),
            format!("write failed ({original}); partial-write rollback also failed: {trunc}"),
        )),
    }
}

fn read_and_verify(path: &Path, expected_pubkey: &VerifyingKey) -> Result<Verified, LedgerError> {
    let bytes = std::fs::read(path)?;
    let mut records = Vec::new();
    let mut validator = ChainValidator::new();
    if !bytes.is_empty() {
        if bytes.last() != Some(&b'\n') {
            let complete_lines = bytes.iter().filter(|byte| **byte == b'\n').count();
            return Err(LedgerError::Truncated {
                line: complete_lines + 1,
            });
        }
        for (index, chunk) in bytes[..bytes.len() - 1]
            .split(|byte| *byte == b'\n')
            .enumerate()
        {
            let line = index + 1;
            let record = verify_stored_chunk(chunk, line, expected_pubkey)?;
            validator.check(&record, line)?;
            records.push(record);
        }
    }
    let last_hash = records.last().map_or(ZERO_HASH, |record| record.hash());
    Ok(Verified {
        records,
        last_hash,
        verified_offset: bytes.len() as u64,
    })
}

/// Verify one raw line (without its newline): parse, public key, signature,
/// and byte-exact canonical re-serialization.
fn verify_stored_chunk(
    chunk: &[u8],
    line: usize,
    expected_pubkey: &VerifyingKey,
) -> Result<LedgerRecord, LedgerError> {
    let stored: StoredLine =
        serde_json::from_slice(chunk).map_err(|error| LedgerError::Malformed {
            line,
            detail: error.to_string(),
        })?;
    check_pubkey(&stored.pubkey, expected_pubkey, line)?;
    let signature = parse_signature(&stored.signature).ok_or(LedgerError::BadSignature { line })?;
    expected_pubkey
        .verify_strict(&stored.record.hash(), &signature)
        .map_err(|_| LedgerError::BadSignature { line })?;
    // Byte-exact canonical encoding: rejects injected unknown fields,
    // whitespace edits, reordered keys and hex-case aliases.
    let canonical = canonical_line(&stored.record, &signature, expected_pubkey)?;
    if canonical[..canonical.len() - 1] != *chunk {
        return Err(LedgerError::Malformed {
            line,
            detail: "line bytes are not the canonical serialization of the signed record".into(),
        });
    }
    Ok(stored.record)
}

fn check_pubkey(encoded: &str, expected: &VerifyingKey, line: usize) -> Result<(), LedgerError> {
    let bytes = decode_pubkey(encoded).ok_or(LedgerError::WrongPubkey { line })?;
    if bytes == expected.to_bytes() {
        Ok(())
    } else {
        Err(LedgerError::WrongPubkey { line })
    }
}

fn decode_pubkey(encoded: &str) -> Option<[u8; 32]> {
    hex::decode(encoded).ok()?.try_into().ok()
}

fn parse_signature(encoded: &str) -> Option<Signature> {
    let bytes: [u8; 64] = hex::decode(encoded).ok()?.try_into().ok()?;
    Some(Signature::from_bytes(&bytes))
}
