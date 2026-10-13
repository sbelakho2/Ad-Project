//! In-memory ledger: the same chain engine as the file ledger (seal, sign,
//! verify) without touching the filesystem. Used by tests and by callers that
//! need to stage records before persisting them.
//!
//! Equivalence with the file ledger is the contract: given the same key and
//! the same scripted sequence of records, both produce identical
//! `seq`/`prevHash`/`hash` values.

use apex_core::PlatformId;
use ed25519_dalek::{Signature, Signer, VerifyingKey};

use crate::chain::{self, ChainValidator, ZERO_HASH};
use crate::errors::LedgerError;
use crate::record::LedgerRecord;
use crate::Keypair;

/// One stored entry: the sealed record plus its signature over the record hash.
#[derive(Debug)]
pub(crate) struct MemoryEntry {
    pub(crate) record: LedgerRecord,
    pub(crate) signature: Signature,
}

/// An append-only, signed and hash-chained ledger held in memory.
#[derive(Debug)]
pub struct InMemoryLedger {
    keypair: Keypair,
    verifying_key: VerifyingKey,
    entries: Vec<MemoryEntry>,
    last_hash: [u8; 32],
    next_seq: u64,
}

impl InMemoryLedger {
    /// Create an empty ledger signing with `keypair`.
    pub fn new(keypair: Keypair) -> Self {
        let verifying_key = keypair.verifying_key();
        Self {
            keypair,
            verifying_key,
            entries: Vec::new(),
            last_hash: ZERO_HASH,
            next_seq: 0,
        }
    }

    /// Seal, sign and append one record. The full in-memory chain is verified
    /// first (fail closed: a tampered history is never appended to); on error
    /// nothing is appended. (In-memory ledgers stage small sequences; the file
    /// ledger keeps its append fast-path.)
    pub fn append(&mut self, draft: LedgerRecord) -> Result<LedgerRecord, LedgerError> {
        self.verify()?;
        let mut record = draft;
        record.seal(self.next_seq, self.last_hash)?;
        let mut validator = ChainValidator::resume(self.last_hash, self.next_seq);
        validator.check(&record, self.entries.len() + 1)?;
        let next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or_else(|| LedgerError::Validation("ledger sequence space exhausted".into()))?;
        let signature = self.keypair.sign(&record.hash());
        self.entries.push(MemoryEntry {
            record: record.clone(),
            signature,
        });
        self.last_hash = record.hash();
        self.next_seq = next_seq;
        Ok(record)
    }

    /// Append a `rollbackPerformed` record with the same refusal rules as
    /// [`crate::Ledger::append_rollback`].
    pub fn append_rollback(
        &mut self,
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: &str,
        rolled_back_seq: u64,
        reason: &str,
    ) -> Result<LedgerRecord, LedgerError> {
        chain::check_rollback_target(
            self.entries.iter().map(|entry| &entry.record),
            platform,
            ad_unit_id,
            rolled_back_seq,
        )?;
        self.append(LedgerRecord::rollback_performed(
            at_unix_ms,
            platform,
            ad_unit_id,
            rolled_back_seq,
            reason,
        ))
    }

    /// Verify the whole in-memory chain: every signature, every recomputed
    /// hash, genesis, linkage and sequence continuity.
    pub fn verify(&self) -> Result<(), LedgerError> {
        let mut validator = ChainValidator::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let line = index + 1;
            self.verifying_key
                .verify_strict(&entry.record.hash(), &entry.signature)
                .map_err(|_| LedgerError::BadSignature { line })?;
            validator.check(&entry.record, line)?;
        }
        Ok(())
    }

    /// Number of records in the ledger.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True for an empty (fresh) ledger.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record at `index`, in append order.
    pub fn get(&self, index: usize) -> Option<&LedgerRecord> {
        self.entries.get(index).map(|entry| &entry.record)
    }

    /// All records, in append order.
    pub fn records(&self) -> Vec<&LedgerRecord> {
        self.entries.iter().map(|entry| &entry.record).collect()
    }

    /// The public key this ledger signs with.
    pub fn public_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    /// Test-only access for tamper tests.
    #[cfg(test)]
    pub(crate) fn entry_mut(&mut self, index: usize) -> Option<&mut MemoryEntry> {
        self.entries.get_mut(index)
    }
}
