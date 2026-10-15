//! Ledger verification for the runner's output line and the
//! `ledger_verified` / `ledger_verification_failed` events.
//!
//! `apex-proof` verifies a whole on-disk chain against the expected public
//! key ([`apex_proof::Ledger::read_verified`]); the runner reports the result
//! honestly: "Ledger: appended and verified" appears only when the verifier
//! actually verified the chain, and a failed verification is printed with the
//! failing line, never softened.

use std::path::PathBuf;

use apex_proof::{Ledger, LedgerError, LedgerLog, VerifyingKey};

/// The verifier version recorded on verification events and metrics.
pub const VERIFIER_VERSION: &str = "v0.1.0";

/// What one verification pass found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerCheck {
    /// The whole chain verified; `highest_sequence` is the last record's
    /// sequence (0 for an empty ledger).
    Verified {
        /// The highest verified sequence.
        highest_sequence: u64,
        /// How long verification took.
        duration_ms: u64,
    },
    /// The chain did not verify.
    Failed {
        /// The bounded result class for the metric.
        result: apex_metrics::vocab::LedgerResult,
        /// A bounded reason naming the first offending line when known.
        reason: String,
        /// The 1-based offending line, when the error names one.
        failure_line: Option<u64>,
    },
}

impl LedgerCheck {
    /// Whether the check passed.
    pub fn is_verified(&self) -> bool {
        matches!(self, LedgerCheck::Verified { .. })
    }

    /// The highest sequence the check established (the last record for a
    /// verified chain, the last read line otherwise).
    pub fn highest_sequence(&self) -> u64 {
        match self {
            LedgerCheck::Verified {
                highest_sequence, ..
            } => *highest_sequence,
            LedgerCheck::Failed { failure_line, .. } => failure_line.unwrap_or(0),
        }
    }
}

/// How the runner verifies the ledger it just wrote.
pub trait LedgerVerifier: Send + Sync + std::fmt::Debug {
    /// Verify the chain behind `ledger`.
    fn verify(&self, ledger: &dyn LedgerLog) -> LedgerCheck;
}

/// The file-backed verifier: re-reads and verifies the whole chain against
/// the expected public key in one pass.
#[derive(Debug)]
pub struct FileLedgerVerifier {
    path: PathBuf,
    key: VerifyingKey,
}

impl FileLedgerVerifier {
    /// Verify `path` against `key`.
    pub fn new(path: impl Into<PathBuf>, key: VerifyingKey) -> Self {
        Self {
            path: path.into(),
            key,
        }
    }
}

impl LedgerVerifier for FileLedgerVerifier {
    fn verify(&self, _ledger: &dyn LedgerLog) -> LedgerCheck {
        let started = std::time::Instant::now();
        match Ledger::read_verified(&self.path, &self.key) {
            Ok(records) => {
                let highest_sequence = records.last().map(|record| record.seq()).unwrap_or(0);
                LedgerCheck::Verified {
                    highest_sequence,
                    duration_ms: started.elapsed().as_millis() as u64,
                }
            }
            Err(error) => LedgerCheck::Failed {
                result: result_class_of(&error),
                reason: bounded_reason(&error.to_string()),
                failure_line: failure_line_of(&error),
            },
        }
    }
}

/// The 1-based line a typed ledger error names, when it names one.
fn failure_line_of(error: &LedgerError) -> Option<u64> {
    let line = match error {
        LedgerError::Malformed { line, .. }
        | LedgerError::HashMismatch { line }
        | LedgerError::ChainBreak { line }
        | LedgerError::BadSignature { line }
        | LedgerError::WrongPubkey { line }
        | LedgerError::SequenceGap { line, .. }
        | LedgerError::Duplicate { line, .. }
        | LedgerError::Genesis { line }
        | LedgerError::Truncated { line } => *line,
        _ => return None,
    };
    u64::try_from(line).ok()
}

/// The bounded metric class of a ledger verification failure.
fn result_class_of(error: &LedgerError) -> apex_metrics::vocab::LedgerResult {
    use apex_metrics::vocab::LedgerResult;
    match error {
        LedgerError::BadSignature { .. } | LedgerError::WrongPubkey { .. } => {
            LedgerResult::InvalidSignature
        }
        LedgerError::Malformed { .. } => LedgerResult::Malformed,
        LedgerError::Io(_) => LedgerResult::Unavailable,
        _ => LedgerResult::InvalidChain,
    }
}

fn bounded_reason(reason: &str) -> String {
    crate::text::bounded_chars(reason, 200)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use apex_proof::{InMemoryLedger, Keypair};

    #[test]
    fn a_verified_file_chain_reports_its_highest_sequence() {
        let dir = std::env::temp_dir().join(format!("apex-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let keypair = Keypair::from_bytes(&[7u8; 32]);
        let path = dir.join("ledger.jsonl");
        let mut ledger = Ledger::open(&path, keypair.clone()).expect("open");
        ledger
            .append(apex_proof::LedgerRecord::change_applied(
                1,
                apex_core::PlatformId::AppLovinMax,
                "u1",
                apex_proof::ChangeKind::CountryFloorSet {
                    country: "EE".into(),
                    floor_ecpm: 1,
                },
                "k",
                "{}",
            ))
            .expect("append");
        let verifier = FileLedgerVerifier::new(&path, keypair.verifying_key());
        let empty = InMemoryLedger::new(Keypair::from_bytes(&[1u8; 32]));
        let check = verifier.verify(&empty);
        assert!(check.is_verified(), "{check:?}");
        assert_eq!(check.highest_sequence(), 0, "one record has seq 0");

        // A tampered payload is a failed verification naming the line.
        let raw = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            raw.replace("\"country\":\"EE\"", "\"country\":\"FR\""),
        )
        .unwrap();
        let check = verifier.verify(&empty);
        match check {
            LedgerCheck::Failed {
                reason,
                failure_line,
                ..
            } => {
                assert!(reason.contains("line 1"), "{reason}");
                assert_eq!(failure_line, Some(1));
            }
            other => panic!("expected failure, got {other:?}"),
        }
        // A missing file is a failure, never a silent pass.
        let verifier = FileLedgerVerifier::new(dir.join("missing.jsonl"), keypair.verifying_key());
        assert!(!verifier.verify(&empty).is_verified());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failure_line_mapping_is_total() {
        assert_eq!(
            failure_line_of(&LedgerError::HashMismatch { line: 3 }),
            Some(3)
        );
        assert_eq!(
            failure_line_of(&LedgerError::Io(std::io::Error::other("x"))),
            None
        );
        assert_eq!(VERIFIER_VERSION, "v0.1.0");
        // Reason bounding counts characters: the classic byte-boundary case
        // (one ASCII byte, then two-byte characters) must not panic and must
        // not split a character.
        let reason = format!("a{}", "é".repeat(500));
        let bounded = bounded_reason(&reason);
        assert_eq!(bounded.chars().count(), 200);
        assert_eq!(bounded, format!("a{}", "é".repeat(199)));
    }
}
