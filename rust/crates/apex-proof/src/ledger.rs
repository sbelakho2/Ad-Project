//! File-backed ledger: an append-only, one-record-per-line JSONL file where
//! every line is signed with ed25519 and chained by SHA-256.
//!
//! ## Persisted line (byte-exact)
//!
//! ```json
//! {"record":{...},"signature":"<128 lowercase hex>","pubkey":"<64 lowercase hex>"}
//! ```
//!
//! Parsing the line alone is not enough: verification re-serializes the parsed
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
//! ## Verification pipeline
//!
//! Reading and verifying is split into two phases. Parsing, public-key
//! matching, canonical-encoding checks and the hash-chain/link rules run
//! sequentially in line order, so structural errors keep their deterministic
//! first-failure line. The ed25519 checks (the expensive part) are collected
//! and verified on a bounded pool of scoped std threads; the pool defaults to
//! the available parallelism capped at `MAX_VERIFY_THREADS` and can be
//! overridden with `APEX_LEDGER_VERIFY_THREADS`. Failures are reduced by line
//! number across workers, so the reported error is always the same
//! [`LedgerError::BadSignature`] for the smallest failing line that the
//! sequential check order would have reported, independent of scheduling.
//!
//! ## Fail-closed appends
//!
//! [`Ledger::open`] verifies the whole existing file before the handle is
//! usable; every [`Ledger::append`] verifies the records **not yet verified**
//! (a tampered history is a typed error, never a silent append).
//!
//! The append fast path is identity-gated. After every successful append the
//! handle records the file identity it left behind — `(len, inode, ctime,
//! ctime_nsec)` from `fstat` — and only an exact match on the next append
//! proves the prefix is byte-identical to what this handle wrote. The kernel
//! updates ctime on every write and metadata change and an unprivileged writer
//! cannot restore it, so an external append, truncation, replacement (inode)
//! or same-length rewrite with a restored mtime all change the identity. On a
//! match the handle seeks to `verified_offset` and reads only the tail bytes,
//! which are parsed, signature-checked and chain-checked exactly as on a full
//! verification; the prefix is neither read nor re-hashed.
//!
//! On any identity mismatch the handle falls back to the content-digest gate:
//! it re-reads the file, hashes the verified prefix and takes the tail path
//! only when that digest matches the running prefix digest, otherwise it
//! performs full verification. The tamper taxonomy is unchanged for every
//! case: the same [`LedgerError`] and 1-based line are reported as before. The
//! identity recorded by [`Ledger::open`] is accepted only when it is identical
//! before and after the verifying read, so an external write racing the open
//! leaves the fast path disabled and the next append falls back to reading and
//! verifying.
//!
//! Writes are `sync_data`-durable; a failed write is rolled back by truncation,
//! and a failed rollback is reported loudly instead of being discarded. The
//! durability point is **one record per append**: every [`Ledger::append`]
//! seals, writes and syncs that record before returning, so a caller that
//! performs a platform side effect between two records keeps the ledger fact
//! durable at the same point in the flow. There is deliberately no batch
//! append: the in-repo write flows (pre-registration before a platform call,
//! the change record before an experiment start, the rollback after a failed
//! start) interleave records with side effects, so deferring the sync across
//! them would weaken crash recovery rather than improve it.
//!
//! The ledger is single-writer by contract: concurrent handles must be
//! externally coordinated. An out-of-contract interleaved append is still
//! handled safely: the next append on a handle detects the other writer's
//! identity change, re-verifies (tail-only when the content digest still
//! matches), and either continues the valid chain linked to the externally
//! written records or refuses with the same typed tamper error (a shorter but
//! valid chain is [`LedgerError::StateDiverged`]). Ordering between concurrent
//! writers is not guaranteed (the JSONL format itself stays append-safe against
//! torn single writes, and any torn write is detected as
//! [`LedgerError::Truncated`]).
//!
//! On unix the file is created with mode `0600` (read/write only for the
//! owner) using [`std::os::unix::fs::OpenOptionsExt`]; existing files keep
//! their current mode.

use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use apex_core::PlatformId;
use ed25519_dalek::{Signature, Signer, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::chain::{self, ChainValidator, ZERO_HASH};
use crate::errors::LedgerError;
use crate::record::LedgerRecord;
use crate::Keypair;

#[cfg(test)]
thread_local! {
    /// Stored lines parsed and signature-checked since the last reset; the
    /// append tests count verification work through this seam.
    static VERIFIED_LINES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Whole-file reads performed by append verification (the old O(file)
    /// per-append cost), and prefix bytes hashed by the content-digest gate.
    /// Consecutive self-appends must leave both at zero.
    static APPEND_PREFIX_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static APPEND_PREFIX_HASHED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Logical durable write and `sync_data` calls issued by appends. Every
    /// append is one of each: the per-record durability point is the contract.
    static WRITE_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SYNC_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Verification threads spawned by the parallel signature path. The
    /// calling thread participates as one worker, so a call spawns at most
    /// `workers - 1` threads.
    static WORKER_THREADS_SPAWNED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_verified_lines() {
    VERIFIED_LINES.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn verified_lines() -> u64 {
    VERIFIED_LINES.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_append_prefix_metrics() {
    APPEND_PREFIX_READS.with(|count| count.set(0));
    APPEND_PREFIX_HASHED.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn append_prefix_reads() -> u64 {
    APPEND_PREFIX_READS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn append_prefix_hashed() -> u64 {
    APPEND_PREFIX_HASHED.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_write_metrics() {
    WRITE_CALLS.with(|count| count.set(0));
    SYNC_CALLS.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn write_calls() -> u64 {
    WRITE_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn sync_calls() -> u64 {
    SYNC_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_worker_spawns() {
    WORKER_THREADS_SPAWNED.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn worker_spawns() -> u64 {
    WORKER_THREADS_SPAWNED.with(std::cell::Cell::get)
}

/// Identity of the file a handle left behind after its last successful write.
///
/// The length detects growth and truncation, the inode detects replacement by
/// rename, and the change time detects same-length rewrites and external
/// appends: the kernel updates ctime on every write or metadata change, and an
/// unprivileged writer cannot set it back (mtime is writable, ctime is not).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
struct FileIdentity {
    len: u64,
    inode: u64,
    ctime_sec: i64,
    ctime_nsec: i64,
}

/// The identity fields this gate can trust on this platform. Off unix there is
/// no inode/ctime pair in `std`, so the gate is disabled and append
/// verification always follows the content-digest path.
#[cfg(unix)]
fn identity_of(metadata: &Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Some(FileIdentity {
        len: metadata.len(),
        inode: metadata.ino(),
        ctime_sec: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec(),
    })
}

#[cfg(not(unix))]
fn identity_of(_metadata: &Metadata) -> Option<FileIdentity> {
    None
}

/// The handle's identity when it still names the file at `path`, so a path
/// replacement between the verifying read and the `fstat` cannot make
/// unverified bytes pass as the verified prefix.
fn capture_identity(file: &File, path: &Path) -> Option<FileIdentity> {
    let handle = identity_of(&file.metadata().ok()?)?;
    let on_disk = identity_of(&std::fs::metadata(path).ok()?)?;
    (handle == on_disk).then_some(handle)
}

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

/// Environment override for the number of signature-verification workers.
pub(crate) const VERIFY_THREADS_ENV: &str = "APEX_LEDGER_VERIFY_THREADS";

/// Upper bound on signature-verification workers. A cold read must not
/// monopolize every core of a server that is still serving requests.
pub(crate) const MAX_VERIFY_THREADS: usize = 8;

/// Below this many records, spawning workers costs more than it saves.
const MIN_PARALLEL_RECORDS: usize = 8;

/// One line that passed every structural check and now only needs its
/// signature verified.
#[derive(Debug)]
struct ParsedLine {
    line: usize,
    record: LedgerRecord,
    signature: Signature,
}

/// Result of verifying a full file: the records, the chain tip, the byte
/// length that was verified and the running SHA-256 state over those bytes
/// (the content-digest fallback gate).
struct Verified {
    records: Vec<LedgerRecord>,
    last_hash: [u8; 32],
    verified_offset: u64,
    prefix_hasher: Sha256,
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
    /// SHA-256 state over exactly the file bytes up to `verified_offset`; the
    /// fallback gate when the file identity does not match.
    prefix_hasher: Sha256,
    /// Number of records covered by the verified prefix at `verified_offset`.
    verified_records: usize,
    /// Identity (len, inode, ctime) of the file this handle left behind after
    /// its last successful append. `None` disables the identity fast path.
    identity: Option<FileIdentity>,
}

impl Ledger {
    /// Open (or create) the ledger at `path` and verify every existing line
    /// against `keypair`'s public key before returning.
    pub fn open(path: impl AsRef<Path>, keypair: Keypair) -> Result<Self, LedgerError> {
        let path = path.as_ref().to_path_buf();
        let verifying_key = keypair.verifying_key();
        let file = open_append(&path)?;
        // Only an identity that is stable across the verifying read describes
        // the bytes that were verified. Any write racing the open changes the
        // ctime (or length/inode) and leaves the fast path disabled.
        let before = identity_of(&file.metadata()?);
        let verified = read_and_verify(&path, &verifying_key)?;
        let identity = match (before, capture_identity(&file, &path)) {
            (Some(before), Some(after))
                if before == after && after.len == verified.verified_offset =>
            {
                Some(after)
            }
            _ => None,
        };
        let verified_records = verified.records.len();
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
            prefix_hasher: verified.prefix_hasher,
            verified_records,
            identity,
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
        Self::read_verified(path, expected_pubkey).map(|_| ())
    }

    /// Read a ledger file and return its records from **the same single read
    /// that verified them**.
    ///
    /// This is the read path for consumers that display or forward verified
    /// records: `Ledger::verify` plus a separate file read would leave a
    /// window in which the file can be replaced between verification and the
    /// read, and the records actually served would then never have been
    /// verified. Here the bytes that pass every check are the bytes the
    /// returned records are parsed from, so no such window exists.
    ///
    /// The verification contract is exactly [`Ledger::verify`]'s: same
    /// signature, public-key, canonical-encoding and chain checks, reported
    /// with the 1-based line of the first offender. An empty file is a valid
    /// empty ledger.
    pub fn read_verified(
        path: impl AsRef<Path>,
        expected_pubkey: &VerifyingKey,
    ) -> Result<Vec<LedgerRecord>, LedgerError> {
        read_and_verify(path.as_ref(), expected_pubkey).map(|verified| verified.records)
    }

    /// Seal, sign and append one record after verifying the bytes not yet
    /// verified (identity-gated tail, with the content-digest fallback on any
    /// identity mismatch). Returns the sealed record. This record is the
    /// durability point: one write and one `sync_data`, exactly as the
    /// documented contract states. On any error the file and the handle state
    /// are unchanged (or the partial write is rolled back and reported).
    pub fn append(&mut self, draft: LedgerRecord) -> Result<LedgerRecord, LedgerError> {
        self.refresh_from_disk()?;
        // Validate and encode the record before touching the file: a refused
        // record leaves the on-disk prefix and the handle unchanged.
        let mut record = draft;
        record.seal(self.next_seq, self.last_hash)?;
        // Same-record validation as the read path: chain rules AND money
        // invariants (a forged BillingProof must never reach the disk).
        ChainValidator::resume(self.last_hash, self.next_seq)
            .check(&record, self.records.len() + 1)?;
        let next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or_else(|| LedgerError::Validation("ledger sequence space exhausted".into()))?;
        let signature = self.keypair.sign(&record.hash());
        let bytes = canonical_line(&record, &signature, &self.verifying_key)?;
        let end = self.file.metadata()?.len();
        #[cfg(test)]
        WRITE_CALLS.with(|count| count.set(count.get() + 1));
        let result = self.file.write_all(&bytes).and_then(|()| {
            #[cfg(test)]
            SYNC_CALLS.with(|count| count.set(count.get() + 1));
            self.file.sync_data()
        });
        if let Err(error) = result {
            self.identity = None;
            return Err(undo_partial_write(&self.file, end, error));
        }
        // Record the identity this handle just left behind. A path that no
        // longer names the handle's inode gets `None`, which forces the next
        // append through full verification instead of trusting this prefix.
        self.identity = capture_identity(&self.file, &self.path);
        self.last_hash = record.hash();
        self.next_seq = next_seq;
        self.records.push(record.clone());
        // The verified prefix deliberately stays at the pre-append file: the
        // next refresh re-checks this line as tail instead of the handle
        // blindly trusting its own write.
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

    /// Fail closed: verify the bytes beyond the verified prefix before any
    /// append.
    ///
    /// The fast path is gated on the file identity recorded by the last
    /// successful append: when `(len, inode, ctime, ctime_nsec)` still match,
    /// the prefix is byte-identical to what this handle wrote, so only the
    /// tail beyond `verified_offset` is read (a seek plus one `read_to_end`)
    /// and verified. Any mismatch — an external append, truncation, same-length
    /// rewrite, restored mtime, replaced inode or another handle's write —
    /// falls back to the content-digest gate: the whole file is re-read, the
    /// verified prefix hashed, and the tail path is taken only when the digest
    /// still matches; otherwise the file is fully re-verified (and the existing
    /// tamper taxonomy applies).
    fn refresh_from_disk(&mut self) -> Result<(), LedgerError> {
        if self.try_identity_tail()? {
            return Ok(());
        }
        let bytes = std::fs::read(&self.path)?;
        #[cfg(test)]
        APPEND_PREFIX_READS.with(|count| count.set(count.get() + 1));
        let len = bytes.len() as u64;
        let complete_lines = bytes.is_empty() || bytes.last() == Some(&b'\n');
        let prefix_unchanged = complete_lines && len >= self.verified_offset && {
            #[cfg(test)]
            APPEND_PREFIX_HASHED.with(|count| count.set(count.get() + self.verified_offset));
            let digest: [u8; 32] = Sha256::digest(&bytes[..self.verified_offset as usize]).into();
            let expected: [u8; 32] = self.prefix_hasher.clone().finalize().into();
            digest == expected
        };
        if prefix_unchanged {
            let tail = &bytes[self.verified_offset as usize..];
            self.verify_tail(tail)?;
            self.prefix_hasher.update(tail);
        } else {
            let verified = read_and_verify_bytes(&bytes, &self.verifying_key)?;
            if verified.records.len() < self.records.len() {
                // The on-disk chain is valid but shorter than the history this
                // handle already verified: never append on top of a swapped file.
                return Err(LedgerError::StateDiverged {
                    line: verified.records.len() + 1,
                });
            }
            self.records = verified.records;
            self.last_hash = verified.last_hash;
            self.prefix_hasher = verified.prefix_hasher;
        }
        self.next_seq = u64::try_from(self.records.len())
            .map_err(|_| LedgerError::Validation("ledger has too many records".into()))?;
        self.verified_offset = len;
        self.verified_records = self.records.len();
        Ok(())
    }

    /// The identity-gated tail path: return `true` when the file still has
    /// exactly the identity this handle wrote and the tail was read and
    /// verified, `false` when the caller must fall back.
    ///
    /// The identity (handle and path must agree) is checked before and after
    /// the tail read; a concurrent external write changes it between the two
    /// calls and sends the append through the content-digest fallback, so a
    /// match means the bytes read are exactly the bytes this handle left
    /// behind.
    fn try_identity_tail(&mut self) -> Result<bool, LedgerError> {
        let Some(recorded) = self.identity else {
            return Ok(false);
        };
        // `capture_identity` requires the handle to still name the file at
        // `path`, so a rename replacement disables the fast path too.
        let Some(before) = capture_identity(&self.file, &self.path) else {
            return Ok(false);
        };
        if before != recorded || before.len < self.verified_offset {
            return Ok(false);
        }
        let Some(tail) = self.read_tail(before.len)? else {
            return Ok(false);
        };
        let Some(after) = capture_identity(&self.file, &self.path) else {
            return Ok(false);
        };
        if after != before {
            return Ok(false);
        }
        if !tail.is_empty() && tail.last() != Some(&b'\n') {
            // The identity matched but the tail is not a complete line: fall
            // back so the content path reports the same Truncated taxonomy.
            return Ok(false);
        }
        self.verify_tail(&tail)?;
        self.prefix_hasher.update(&tail);
        self.verified_offset = before.len;
        self.verified_records = self.records.len();
        self.next_seq = u64::try_from(self.records.len())
            .map_err(|_| LedgerError::Validation("ledger has too many records".into()))?;
        Ok(true)
    }

    /// Read only the bytes after `verified_offset`. `None` means the file
    /// changed between the `fstat` and the read (or is not the expected
    /// length), so the caller must use the full-read fallback.
    fn read_tail(&mut self, len: u64) -> Result<Option<Vec<u8>>, LedgerError> {
        let expected = len - self.verified_offset;
        let mut tail = Vec::new();
        self.file.seek(SeekFrom::Start(self.verified_offset))?;
        self.file.read_to_end(&mut tail)?;
        if u64::try_from(tail.len()).ok() != Some(expected) {
            return Ok(None);
        }
        Ok(Some(tail))
    }

    /// Verify the bytes beyond the verified prefix, resuming the chain from
    /// the prefix tip. Line numbers and error taxonomy match full
    /// verification; on any error the handle state is untouched.
    fn verify_tail(&mut self, tail: &[u8]) -> Result<(), LedgerError> {
        let mut tail_records = Vec::new();
        if !tail.is_empty() {
            let prev = self
                .verified_records
                .checked_sub(1)
                .and_then(|index| self.records.get(index))
                .map_or(ZERO_HASH, LedgerRecord::hash);
            let mut validator = ChainValidator::resume(
                prev,
                u64::try_from(self.verified_records)
                    .map_err(|_| LedgerError::Validation("ledger has too many records".into()))?,
            );
            for (index, chunk) in tail[..tail.len() - 1]
                .split(|byte| *byte == b'\n')
                .enumerate()
            {
                let line = self.verified_records + index + 1;
                let record = verify_stored_chunk(chunk, line, &self.verifying_key)?;
                validator.check(&record, line)?;
                tail_records.push(record);
            }
        }
        self.records.truncate(self.verified_records);
        self.records.extend(tail_records);
        self.last_hash = self.records.last().map_or(ZERO_HASH, LedgerRecord::hash);
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
    read_and_verify_bytes(&bytes, expected_pubkey)
}

fn read_and_verify_bytes(
    bytes: &[u8],
    expected_pubkey: &VerifyingKey,
) -> Result<Verified, LedgerError> {
    read_and_verify_bytes_with_threads(bytes, expected_pubkey, resolve_verify_threads(), None)
}

/// The full verification pipeline: structural checks run sequentially in line
/// order, signature checks run on `threads` scoped workers (bounded by the
/// record count). `probe`, when present, is incremented once per verification
/// worker (a test seam for the thread-count contract).
fn read_and_verify_bytes_with_threads(
    bytes: &[u8],
    expected_pubkey: &VerifyingKey,
    threads: usize,
    probe: Option<&AtomicU64>,
) -> Result<Verified, LedgerError> {
    let mut parsed = Vec::new();
    // The first structural error, the line it belongs to, and whether the
    // sequential order would have checked that line's signature before the
    // failing check (canonical and chain checks run after the signature;
    // parsing and public-key matching run before it).
    let mut structural: Option<(LedgerError, usize, bool)> = None;
    // A signature field that is not even decodable is a signature failure at
    // its line; it stops parsing exactly like the sequential path does.
    let mut undecodable_signature = None;
    if !bytes.is_empty() {
        if bytes.last() != Some(&b'\n') {
            let complete_lines = bytes.iter().filter(|byte| **byte == b'\n').count();
            return Err(LedgerError::Truncated {
                line: complete_lines + 1,
            });
        }
        let mut validator = ChainValidator::new();
        for (index, chunk) in bytes[..bytes.len() - 1]
            .split(|byte| *byte == b'\n')
            .enumerate()
        {
            let line = index + 1;
            let stored = match parse_stored_chunk(chunk, line) {
                Ok(stored) => stored,
                Err(error) => {
                    structural = Some((error, line, false));
                    break;
                }
            };
            if let Err(error) = check_pubkey(&stored.pubkey, expected_pubkey, line) {
                structural = Some((error, line, false));
                break;
            }
            let signature = match parse_signature(&stored.signature) {
                Some(signature) => signature,
                None => {
                    undecodable_signature = Some(line);
                    break;
                }
            };
            // Deferred ordering: the signature check for this line runs on
            // the pool, but a canonical or chain failure is only reported if
            // this line's signature verifies (matching the sequential order).
            let canonical =
                canonical_check(chunk, &stored.record, &signature, expected_pubkey, line);
            let chain = if canonical.is_ok() {
                validator.check(&stored.record, line)
            } else {
                Ok(())
            };
            parsed.push(ParsedLine {
                line,
                record: stored.record,
                signature,
            });
            if let Err(error) = canonical {
                structural = Some((error, line, true));
                break;
            }
            if let Err(error) = chain {
                structural = Some((error, line, true));
                break;
            }
        }
    }

    let bad_signature = match (
        first_bad_signature(&parsed, expected_pubkey, threads, probe)?,
        undecodable_signature,
    ) {
        (Some(from_pool), Some(undecodable)) => Some(from_pool.min(undecodable)),
        (Some(line), None) | (None, Some(line)) => Some(line),
        (None, None) => None,
    };
    if let Some(bad_line) = bad_signature {
        let signature_wins = match &structural {
            Some((_, structural_line, checked_at_line)) => {
                bad_line < *structural_line || (*checked_at_line && bad_line == *structural_line)
            }
            None => true,
        };
        if signature_wins {
            return Err(LedgerError::BadSignature { line: bad_line });
        }
    }
    if let Some((error, _, _)) = structural {
        return Err(error);
    }

    let records: Vec<LedgerRecord> = parsed
        .into_iter()
        .map(|parsed_line| parsed_line.record)
        .collect();
    let last_hash = records.last().map_or(ZERO_HASH, |record| record.hash());
    let mut prefix_hasher = Sha256::new();
    prefix_hasher.update(bytes);
    Ok(Verified {
        records,
        last_hash,
        verified_offset: bytes.len() as u64,
        prefix_hasher,
    })
}

/// The worker count for one read: the environment override when it parses,
/// otherwise the available parallelism capped at `MAX_VERIFY_THREADS`.
fn resolve_verify_threads() -> usize {
    std::env::var(VERIFY_THREADS_ENV)
        .ok()
        .and_then(|raw| parse_verify_threads(&raw))
        .unwrap_or_else(default_verify_threads)
}

fn default_verify_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .clamp(1, MAX_VERIFY_THREADS)
}

/// Parse `APEX_LEDGER_VERIFY_THREADS`: whitespace is ignored, non-numbers are
/// rejected (caller falls back to the default) and the value is clamped to
/// `1..=MAX_VERIFY_THREADS`.
pub(crate) fn parse_verify_threads(raw: &str) -> Option<usize> {
    raw.trim()
        .parse::<usize>()
        .ok()
        .map(|threads| threads.clamp(1, MAX_VERIFY_THREADS))
}

/// Verify the collected lines on at most `threads` scoped workers and return
/// the smallest failing line, independent of thread scheduling.
fn first_bad_signature(
    parsed: &[ParsedLine],
    expected_pubkey: &VerifyingKey,
    threads: usize,
    probe: Option<&AtomicU64>,
) -> Result<Option<usize>, LedgerError> {
    if parsed.is_empty() {
        return Ok(None);
    }
    let workers = threads.clamp(1, parsed.len());
    if workers == 1 || parsed.len() < MIN_PARALLEL_RECORDS {
        if let Some(probe) = probe {
            probe.fetch_add(1, Ordering::Relaxed);
        }
        return Ok(first_bad_in(parsed, expected_pubkey));
    }
    let chunk_size = parsed.len().div_ceil(workers);
    let chunks: Vec<&[ParsedLine]> = parsed.chunks(chunk_size).collect();
    let mut first_bad = None;
    std::thread::scope(|scope| -> Result<(), LedgerError> {
        // The calling thread is one of the workers: a call with `k` chunks
        // spawns only `k - 1` threads. Verification is CPU-bound and the
        // caller must wait for the result anyway, so making it work costs
        // nothing and removes one thread creation per call.
        let mut handles = Vec::with_capacity(chunks.len().saturating_sub(1));
        for chunk in &chunks[1..] {
            let chunk = *chunk;
            #[cfg(test)]
            WORKER_THREADS_SPAWNED.with(|count| count.set(count.get() + 1));
            handles.push(scope.spawn(move || {
                if let Some(probe) = probe {
                    probe.fetch_add(1, Ordering::Relaxed);
                }
                first_bad_in(chunk, expected_pubkey)
            }));
        }
        if let Some(probe) = probe {
            probe.fetch_add(1, Ordering::Relaxed);
        }
        first_bad = first_bad_in(chunks[0], expected_pubkey);
        let mut panicked = false;
        for handle in handles {
            match handle.join() {
                Ok(Some(line)) => {
                    first_bad = Some(first_bad.map_or(line, |current: usize| current.min(line)));
                }
                Ok(None) => {}
                Err(_) => panicked = true,
            }
        }
        if panicked {
            return Err(LedgerError::Validation(
                "ledger signature verification worker panicked".into(),
            ));
        }
        Ok(())
    })?;
    Ok(first_bad)
}

/// First signature failure within one contiguous slice (line order).
fn first_bad_in(parsed: &[ParsedLine], expected_pubkey: &VerifyingKey) -> Option<usize> {
    parsed
        .iter()
        .find(|parsed_line| {
            expected_pubkey
                .verify_strict(&parsed_line.record.hash(), &parsed_line.signature)
                .is_err()
        })
        .map(|parsed_line| parsed_line.line)
}

/// Verify one raw line (without its newline): parse, public key, signature,
/// and byte-exact canonical re-serialization. This is the sequential path used
/// by the append tail fast path.
fn verify_stored_chunk(
    chunk: &[u8],
    line: usize,
    expected_pubkey: &VerifyingKey,
) -> Result<LedgerRecord, LedgerError> {
    let stored = parse_stored_chunk(chunk, line)?;
    check_pubkey(&stored.pubkey, expected_pubkey, line)?;
    let signature = parse_signature(&stored.signature).ok_or(LedgerError::BadSignature { line })?;
    expected_pubkey
        .verify_strict(&stored.record.hash(), &signature)
        .map_err(|_| LedgerError::BadSignature { line })?;
    canonical_check(chunk, &stored.record, &signature, expected_pubkey, line)?;
    Ok(stored.record)
}

/// Parse one line and count it through the test seam.
fn parse_stored_chunk(chunk: &[u8], line: usize) -> Result<StoredLine, LedgerError> {
    #[cfg(test)]
    VERIFIED_LINES.with(|count| count.set(count.get() + 1));
    serde_json::from_slice(chunk).map_err(|error| LedgerError::Malformed {
        line,
        detail: error.to_string(),
    })
}

/// Byte-exact canonical encoding: rejects injected unknown fields, whitespace
/// edits, reordered keys and hex-case aliases.
fn canonical_check(
    chunk: &[u8],
    record: &LedgerRecord,
    signature: &Signature,
    expected_pubkey: &VerifyingKey,
    line: usize,
) -> Result<(), LedgerError> {
    let canonical = canonical_line(record, signature, expected_pubkey)?;
    if canonical[..canonical.len() - 1] != *chunk {
        return Err(LedgerError::Malformed {
            line,
            detail: "line bytes are not the canonical serialization of the signed record".into(),
        });
    }
    Ok(())
}

/// Test-only entry point for the parallel pipeline.
#[cfg(test)]
pub(crate) fn read_verified_bytes_with_threads(
    bytes: &[u8],
    expected_pubkey: &VerifyingKey,
    threads: usize,
    probe: Option<&AtomicU64>,
) -> Result<Vec<LedgerRecord>, LedgerError> {
    read_and_verify_bytes_with_threads(bytes, expected_pubkey, threads, probe)
        .map(|verified| verified.records)
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
