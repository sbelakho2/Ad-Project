//! Typed ledger errors. Every rejection that happens while reading a ledger
//! names the 1-based line of the *first* offending record; callers can quote
//! that line number in operator-facing reports.

use std::io;

/// Everything that can go wrong while writing or verifying a ledger.
///
/// The error variants are the tamper taxonomy: a verifier can distinguish a
/// mutated payload ([`LedgerError::HashMismatch`]) from a mutated hash chain
/// ([`LedgerError::ChainBreak`], [`LedgerError::Genesis`]) from a forged line
/// ([`LedgerError::BadSignature`], [`LedgerError::WrongPubkey`]) from a
/// structural problem ([`LedgerError::Truncated`], [`LedgerError::Malformed`],
/// [`LedgerError::SequenceGap`], [`LedgerError::Duplicate`]).
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    /// Underlying filesystem failure.
    #[error("ledger io error: {0}")]
    Io(#[from] io::Error),

    /// A line could not be parsed as a persisted ledger line at all.
    #[error("line {line}: malformed ledger line: {detail}")]
    Malformed { line: usize, detail: String },

    /// The record's payload was changed after it was sealed: the recomputed
    /// hash does not match the stored one (covers payload, seq and prevHash
    /// field tampering, since all of them are part of the hashed bytes).
    #[error("line {line}: record payload does not match its hash")]
    HashMismatch { line: usize },

    /// `prevHash` does not link to the previous record (swapped, reordered or
    /// spliced lines).
    #[error("line {line}: prevHash does not link to the previous record")]
    ChainBreak { line: usize },

    /// The ed25519 signature does not verify over the record hash (covers
    /// signature and hash-field tampering).
    #[error("line {line}: signature verification failed")]
    BadSignature { line: usize },

    /// The line is signed by a different key than the expected one.
    #[error("line {line}: unexpected public key")]
    WrongPubkey { line: usize },

    /// `seq` is neither the expected successor nor a duplicate.
    #[error("line {line}: expected sequence {expected}, found {found}")]
    SequenceGap {
        line: usize,
        expected: u64,
        found: u64,
    },

    /// `seq` repeats the previous record's sequence.
    #[error("line {line}: duplicate sequence {seq}")]
    Duplicate { line: usize, seq: u64 },

    /// The first record's `prevHash` is not all zeros.
    #[error("line {line}: genesis record must have a zero prevHash")]
    Genesis { line: usize },

    /// The final line has no trailing newline: the file was truncated
    /// (possibly mid-write) and cannot be trusted.
    #[error("line {line}: truncated line (missing trailing newline)")]
    Truncated { line: usize },

    /// The file on disk is a valid chain, but it is not the chain this
    /// in-memory handle has been building (someone rewrote or replaced it).
    #[error("line {line}: on-disk ledger diverged from the in-memory chain")]
    StateDiverged { line: usize },

    /// `append_rollback` was pointed at a sequence that is not in the ledger.
    #[error("rollback target sequence {seq} is unknown")]
    UnknownRollbackTarget { seq: u64 },

    /// `append_rollback` was pointed at a record that is itself a rollback.
    #[error("sequence {seq} is a rollback and cannot be rolled back")]
    RollbackOfRollback { seq: u64 },

    /// The rollback target exists but is not a `ChangeApplied` record of the same
    /// platform and ad unit as the rollback being appended.
    #[error("sequence {seq} is not a change applied to the same platform and ad unit")]
    RollbackTargetMismatch { seq: u64 },

    /// A `BillingProof` record violates the money invariants (rate outside
    /// 15..=20%, non-positive proven lift, or a fee that is not the exact
    /// recomputation of proven_lift * rate). The ledger refuses to persist it.
    #[error("line {line}: invalid billing proof: {reason}")]
    InvalidBillingProof { line: usize, reason: String },

    /// This sequence was already rolled back once.
    #[error("sequence {seq} was already rolled back")]
    DuplicateRollback { seq: u64 },

    /// A record could not be serialized (impossible for the current record
    /// types; kept so no code path has to panic).
    #[error("cannot serialize ledger record: {0}")]
    Serialize(#[from] serde_json::Error),

    /// Other invariant violations.
    #[error("ledger validation error: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_name_the_offending_line() {
        assert_eq!(
            LedgerError::HashMismatch { line: 7 }.to_string(),
            "line 7: record payload does not match its hash"
        );
        assert_eq!(
            LedgerError::SequenceGap {
                line: 3,
                expected: 2,
                found: 5
            }
            .to_string(),
            "line 3: expected sequence 2, found 5"
        );
    }
}
