//! The delivery state store: append-only JSONL rows.
//!
//! Page 94 (delivery contract): "Record requested, accepted, delivered or
//! failed as distinct states when the provider supplies them. Deduplicate
//! retries". Every send therefore appends a `requested` row before the sink
//! is called and exactly one outcome row after; a `(template_class,
//! dedup_key, to)` triple that already has an `accepted` or `delivered` row
//! never sends again.
//!
//! The store is opened on `APEX_NOTIFY_STATE`; without a path it still
//! deduplicates within the process, and that limitation is recorded in
//! [`DeliveryLog::path`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// The environment variable naming the state file.
pub const STATE_ENV: &str = "APEX_NOTIFY_STATE";

/// One recorded delivery state row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryStateRow {
    /// When the row was written, unix milliseconds.
    pub at_unix_ms: i64,
    /// The template that produced the message.
    pub template_class: String,
    /// The deduplication identity.
    pub dedup_key: String,
    /// The recipient.
    pub to: String,
    /// One of `requested`, `accepted`, `delivered`, `failed`.
    pub state: String,
    /// The provider's reference, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reference: Option<String>,
    /// A bounded detail line (a failure reason).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The four distinct states of the delivery contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    /// The send was requested of the provider.
    Requested,
    /// The provider accepted the message.
    Accepted,
    /// The provider confirmed delivery.
    Delivered,
    /// The send failed; a retry may still send.
    Failed,
}

impl StateKind {
    /// The stable row state name.
    pub fn as_str(self) -> &'static str {
        match self {
            StateKind::Requested => "requested",
            StateKind::Accepted => "accepted",
            StateKind::Delivered => "delivered",
            StateKind::Failed => "failed",
        }
    }

    /// Parse a row state name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "requested" => Some(StateKind::Requested),
            "accepted" => Some(StateKind::Accepted),
            "delivered" => Some(StateKind::Delivered),
            "failed" => Some(StateKind::Failed),
            _ => None,
        }
    }

    /// Whether a row in this state blocks a resend of the same
    /// `(template_class, dedup_key)` to the same recipient.
    fn blocks_resend(self) -> bool {
        matches!(self, StateKind::Accepted | StateKind::Delivered)
    }
}

/// Why the state store failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// The state file could not be read or written.
    #[error("notification state at {path}: {reason}")]
    Io {
        /// The file involved.
        path: String,
        /// The reason.
        reason: String,
    },
    /// A stored row is corrupt; the store fails closed rather than dropping
    /// deduplication state.
    #[error("notification state at {path} line {line} is corrupt: {reason}")]
    Corrupt {
        /// The file involved.
        path: String,
        /// The 1-based line.
        line: usize,
        /// The parse failure.
        reason: String,
    },
    /// A row carries an unknown state name.
    #[error("notification state row has unknown state '{state}'")]
    UnknownState {
        /// The refused state.
        state: String,
    },
}

/// The append-only delivery log.
#[derive(Debug)]
pub struct DeliveryLog {
    path: Option<PathBuf>,
    sent: Mutex<BTreeSet<(String, String, String)>>,
    write_lock: Mutex<()>,
}

impl DeliveryLog {
    /// Open the log at `path` (creating parent directories). An existing file
    /// is loaded; a corrupt line is a typed error, never a silent reset.
    pub fn open(path: Option<&Path>) -> Result<Self, StateError> {
        let mut sent = BTreeSet::new();
        if let Some(path) = path {
            if let Ok(bytes) = std::fs::read(path) {
                let text = String::from_utf8_lossy(&bytes);
                for (index, line) in text.lines().enumerate() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let row: DeliveryStateRow =
                        serde_json::from_str(line).map_err(|error| StateError::Corrupt {
                            path: path.display().to_string(),
                            line: index + 1,
                            reason: error.to_string(),
                        })?;
                    let kind =
                        StateKind::parse(&row.state).ok_or_else(|| StateError::UnknownState {
                            state: row.state.clone(),
                        })?;
                    if kind.blocks_resend() {
                        sent.insert((row.template_class, row.dedup_key, row.to));
                    }
                }
            }
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|error| StateError::Io {
                        path: parent.display().to_string(),
                        reason: error.to_string(),
                    })?;
                }
            }
        }
        Ok(Self {
            path: path.map(Path::to_path_buf),
            sent: Mutex::new(sent),
            write_lock: Mutex::new(()),
        })
    }

    /// The state file, when one is configured.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Whether `(template_class, dedup_key)` was already accepted or
    /// delivered to `to`.
    pub fn has_sent(&self, template_class: &str, dedup_key: &str, to: &str) -> bool {
        self.sent
            .lock()
            .map(|sent| {
                sent.contains(&(
                    template_class.to_string(),
                    dedup_key.to_string(),
                    to.to_string(),
                ))
            })
            .unwrap_or(false)
    }

    /// Append one row and update the deduplication index. A write failure to
    /// a configured file is a typed error: deduplication state must not be
    /// lost silently.
    pub fn record(&self, row: &DeliveryStateRow) -> Result<(), StateError> {
        let kind = StateKind::parse(&row.state).ok_or_else(|| StateError::UnknownState {
            state: row.state.clone(),
        })?;
        {
            let _guard = self.write_lock.lock().map_err(|_| StateError::Io {
                path: self
                    .path
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<memory>".to_string()),
                reason: "state store write lock is poisoned".to_string(),
            })?;
            if let Some(path) = &self.path {
                let line = serde_json::to_string(row).map_err(|error| StateError::Io {
                    path: path.display().to_string(),
                    reason: error.to_string(),
                })?;
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|error| StateError::Io {
                        path: path.display().to_string(),
                        reason: error.to_string(),
                    })?;
                writeln!(file, "{line}").map_err(|error| StateError::Io {
                    path: path.display().to_string(),
                    reason: error.to_string(),
                })?;
            }
        }
        if kind.blocks_resend() {
            if let Ok(mut sent) = self.sent.lock() {
                sent.insert((
                    row.template_class.clone(),
                    row.dedup_key.clone(),
                    row.to.clone(),
                ));
            }
        }
        Ok(())
    }

    /// Every row of a state file, for tests and operators.
    pub fn read_rows(path: &Path) -> Result<Vec<DeliveryStateRow>, StateError> {
        let bytes = std::fs::read(path).map_err(|error| StateError::Io {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
        let text = String::from_utf8_lossy(&bytes);
        let mut rows = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            rows.push(
                serde_json::from_str(line).map_err(|error| StateError::Corrupt {
                    path: path.display().to_string(),
                    line: index + 1,
                    reason: error.to_string(),
                })?,
            );
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn row(state: &str, dedup_key: &str) -> DeliveryStateRow {
        DeliveryStateRow {
            at_unix_ms: 1,
            template_class: "payment_failed".into(),
            dedup_key: dedup_key.into(),
            to: "ops@example.com".into(),
            state: state.into(),
            provider_reference: None,
            detail: None,
        }
    }

    #[test]
    fn state_names_round_trip_and_resend_blocking_is_exact() {
        for kind in [
            StateKind::Requested,
            StateKind::Accepted,
            StateKind::Delivered,
            StateKind::Failed,
        ] {
            assert_eq!(StateKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(StateKind::parse("queued"), None);
        assert!(!StateKind::Requested.blocks_resend());
        assert!(StateKind::Accepted.blocks_resend());
        assert!(StateKind::Delivered.blocks_resend());
        assert!(!StateKind::Failed.blocks_resend());
    }

    #[test]
    fn an_in_memory_log_still_deduplicates_and_records_four_states() {
        let log = DeliveryLog::open(None).expect("memory log");
        assert!(log.path().is_none());
        assert!(!log.has_sent("payment_failed", "k1", "ops@example.com"));
        log.record(&row("requested", "k1")).unwrap();
        assert!(!log.has_sent("payment_failed", "k1", "ops@example.com"));
        log.record(&row("accepted", "k1")).unwrap();
        assert!(log.has_sent("payment_failed", "k1", "ops@example.com"));
        // A different recipient is a different dedup identity.
        assert!(!log.has_sent("payment_failed", "k1", "fin@example.com"));
        // A failed attempt does not block a later retry.
        log.record(&row("requested", "k2")).unwrap();
        log.record(&row("failed", "k2")).unwrap();
        assert!(!log.has_sent("payment_failed", "k2", "ops@example.com"));
        assert!(matches!(
            log.record(&row("queued", "k3")),
            Err(StateError::UnknownState { .. })
        ));
    }

    #[test]
    fn a_file_backed_log_survives_reopen_and_keeps_four_distinct_rows() {
        let dir = std::env::temp_dir().join(format!("apex-notify-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open(Some(&path)).expect("open");
        assert_eq!(log.path(), Some(path.as_path()));
        log.record(&row("requested", "k1")).unwrap();
        log.record(&row("accepted", "k1")).unwrap();
        drop(log);

        let reopened = DeliveryLog::open(Some(&path)).expect("reopen");
        assert!(reopened.has_sent("payment_failed", "k1", "ops@example.com"));
        reopened.record(&row("requested", "k2")).unwrap();
        reopened
            .record(&DeliveryStateRow {
                state: "delivered".into(),
                provider_reference: Some("msg_9".into()),
                ..row("requested", "k2")
            })
            .unwrap();
        let rows = DeliveryLog::read_rows(&path).expect("rows");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].state, "requested");
        assert_eq!(rows[1].state, "accepted");
        assert_eq!(rows[3].state, "delivered");
        assert_eq!(rows[3].provider_reference.as_deref(), Some("msg_9"));

        // A corrupt line is a typed refusal on open.
        std::fs::write(&path, "not json\n").unwrap();
        assert!(matches!(
            DeliveryLog::open(Some(&path)),
            Err(StateError::Corrupt { line: 1, .. })
        ));
        std::fs::write(
            &path,
            r#"{"at_unix_ms":1,"template_class":"payment_failed","dedup_key":"k","to":"o@example.com","state":"queued"}"#,
        )
        .unwrap();
        assert!(matches!(
            DeliveryLog::open(Some(&path)),
            Err(StateError::UnknownState { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_failure_to_a_configured_file_is_typed() {
        let dir =
            std::env::temp_dir().join(format!("apex-notify-state-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The path is a directory: appending the first row fails typed.
        let log = DeliveryLog::open(Some(&dir)).expect("open tolerates a missing file");
        let error = log.record(&row("requested", "k1")).expect_err("dir path");
        assert!(matches!(error, StateError::Io { .. }), "{error:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn error_displays_are_actionable() {
        let error = StateError::UnknownState {
            state: "queued".into(),
        };
        assert!(error.to_string().contains("queued"));
        let error = StateError::Corrupt {
            path: "x.jsonl".into(),
            line: 3,
            reason: "bad".into(),
        };
        assert!(error.to_string().contains("line 3"));
    }
}
