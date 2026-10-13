//! The chain engine shared by the file ledger and the in-memory ledger.
//!
//! ## Canonical hashing
//!
//! `hash` = SHA-256 of the *canonical JSON* of the record with the `hash`
//! field removed. Canonical JSON here means: compact (`serde_json`)
//! serialization with every object's keys sorted lexicographically,
//! recursively. The sort makes the byte stream independent of serde's map
//! implementation (`BTreeMap` vs `preserve_order`) and of struct declaration
//! order; the value types used in records (ints, bools, strings, arrays) have
//! exactly one JSON encoding each, so the canonical form is fully determined
//! by the record's field values. The pinned test in `record.rs` locks the
//! exact bytes for one worked record.
//!
//! ## Chain rules (checked in this order, per line)
//!
//! 1. the stored `hash` equals the recomputed canonical hash,
//! 2. line 1: `prevHash` is all zeros (genesis) and `seq` is 0,
//! 3. line n > 1: `prevHash` equals the previous record's `hash`,
//! 4. `seq` follows the previous record (duplicate and gap are distinct
//!    typed errors).
//!
//! Signature and public-key checks live in the ledgers, which own the keys;
//! this module deliberately does not know about ed25519.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::errors::LedgerError;
use crate::record::LedgerRecord;

/// The all-zero hash: the `prevHash` of the genesis record.
pub(crate) const ZERO_HASH: [u8; 32] = [0u8; 32];

/// Recursively rebuild a JSON value with object keys in sorted order.
fn sort_nested(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .into_iter()
                .map(|(key, value)| (key, sort_nested(value)))
                .collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = serde_json::Map::new();
            for (key, value) in entries {
                sorted.insert(key, value);
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_nested).collect()),
        scalar => scalar,
    }
}

/// Canonical bytes hashed for the chain: compact JSON of the record without
/// its `hash` field, keys sorted recursively.
pub(crate) fn canonical_bytes(record: &LedgerRecord) -> Result<Vec<u8>, LedgerError> {
    let value = serde_json::to_value(record)?;
    let Value::Object(mut map) = value else {
        return Err(LedgerError::Validation(
            "ledger record serialized to a non-object JSON value".into(),
        ));
    };
    map.remove("hash");
    serde_json::to_vec(&sort_nested(Value::Object(map))).map_err(LedgerError::Serialize)
}

/// SHA-256 over [`canonical_bytes`].
pub(crate) fn compute_hash(record: &LedgerRecord) -> Result<[u8; 32], LedgerError> {
    let bytes = canonical_bytes(record)?;
    Ok(Sha256::digest(bytes).into())
}

/// Streaming verifier for the chain rules above. Callers check signatures and
/// public keys before feeding each record in.
#[derive(Debug, Default)]
pub(crate) struct ChainValidator {
    prev_hash: [u8; 32],
    prev_seq: Option<u64>,
}

impl ChainValidator {
    pub(crate) fn new() -> Self {
        Self {
            prev_hash: ZERO_HASH,
            prev_seq: None,
        }
    }

    /// Resume validation from a previously verified tip (used by the append path
    /// so an append only re-verifies the file tail, not the whole history).
    pub(crate) fn resume(last_hash: [u8; 32], next_seq: u64) -> Self {
        Self {
            prev_hash: last_hash,
            prev_seq: next_seq.checked_sub(1),
        }
    }

    /// Verify one record at 1-based `line`, advancing the chain state.
    pub(crate) fn check(&mut self, record: &LedgerRecord, line: usize) -> Result<(), LedgerError> {
        let recomputed = compute_hash(record)?;
        if recomputed != record.hash() {
            return Err(LedgerError::HashMismatch { line });
        }
        // Money invariant: the ledger itself refuses invalid BillingProof records,
        // so `build_billing_proof` is not the only line of defense.
        if let LedgerRecord::BillingProof {
            proven_lift,
            rate_bp,
            fee_micros,
            ..
        } = record
        {
            if *proven_lift <= 0 {
                return Err(LedgerError::InvalidBillingProof {
                    line,
                    reason: format!("proven lift {proven_lift} must be positive"),
                });
            }
            let expected_fee = apex_core::lift_fee_micros(*proven_lift, *rate_bp)
                .map_err(|e| LedgerError::InvalidBillingProof { line, reason: e })?;
            if *fee_micros != expected_fee {
                return Err(LedgerError::InvalidBillingProof {
                    line,
                    reason: format!(
                        "fee {fee_micros} is not proven_lift({proven_lift}) * rate({rate_bp}bp) = {expected_fee}"
                    ),
                });
            }
        }
        match self.prev_seq {
            None => {
                if record.prev_hash() != ZERO_HASH {
                    return Err(LedgerError::Genesis { line });
                }
                if record.seq() != 0 {
                    return Err(LedgerError::SequenceGap {
                        line,
                        expected: 0,
                        found: record.seq(),
                    });
                }
            }
            Some(prev_seq) => {
                if record.prev_hash() != self.prev_hash {
                    return Err(LedgerError::ChainBreak { line });
                }
                if record.seq() == prev_seq {
                    return Err(LedgerError::Duplicate {
                        line,
                        seq: prev_seq,
                    });
                }
                let expected = prev_seq.saturating_add(1);
                if record.seq() != expected {
                    return Err(LedgerError::SequenceGap {
                        line,
                        expected,
                        found: record.seq(),
                    });
                }
            }
        }
        self.prev_hash = record.hash();
        self.prev_seq = Some(record.seq());
        Ok(())
    }
}

/// Rollback rules, shared by both ledgers:
/// - the target sequence must exist,
/// - the target must be a `ChangeApplied` record **for the same platform and ad
///   unit** as the rollback (a LevelPlay rollback cannot "undo" an AdMob change),
/// - the target must not itself be a rollback (rollback-of-rollback),
/// - the target must not have been rolled back before (double rollback).
pub(crate) fn check_rollback_target<'a, I>(
    records: I,
    platform: apex_core::PlatformId,
    ad_unit_id: &str,
    rolled_back_seq: u64,
) -> Result<(), LedgerError>
where
    I: IntoIterator<Item = &'a LedgerRecord> + Clone,
{
    let target = records
        .clone()
        .into_iter()
        .find(|record| record.seq() == rolled_back_seq)
        .ok_or(LedgerError::UnknownRollbackTarget {
            seq: rolled_back_seq,
        })?;
    match target {
        LedgerRecord::ChangeApplied {
            platform: target_platform,
            ad_unit_id: target_unit,
            ..
        } if *target_platform == platform && target_unit == ad_unit_id => {}
        LedgerRecord::RollbackPerformed { .. } => {
            return Err(LedgerError::RollbackOfRollback {
                seq: rolled_back_seq,
            });
        }
        _ => {
            return Err(LedgerError::RollbackTargetMismatch {
                seq: rolled_back_seq,
            });
        }
    }
    let already_rolled_back = records.into_iter().any(|record| {
        matches!(
            record,
            LedgerRecord::RollbackPerformed { rolled_back_seq: target, .. } if *target == rolled_back_seq
        )
    });
    if already_rolled_back {
        return Err(LedgerError::DuplicateRollback {
            seq: rolled_back_seq,
        });
    }
    Ok(())
}
