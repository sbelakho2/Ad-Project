//! The append-only operator action journal.
//!
//! Every action attempt (permitted, denied, failed), every step-up
//! requirement and every session revocation is appended as one JSON line to
//! `APEX_CONTROL_JOURNAL`. The journal is the restricted record the design
//! document asks for (page 71): actor, reason, target, before/after digests
//! and outcome. It never carries raw tokens, secrets or unrestricted query
//! bodies; the human-readable fields pass [`crate::redact::redact_text`]
//! before they are written.
//!
//! The journal is append-only by construction: the file is opened with the
//! append flag, and no code path rewrites or truncates it. Loading it back
//! builds the in-memory idempotency index, so a restarted process still
//! answers a duplicate POST with the original result page.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::signing::idempotency_fingerprint;

/// One journal line.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionJournalEntry {
    /// When the action was processed (Unix milliseconds).
    pub at_ms: i64,
    /// Operator identity.
    pub actor: String,
    /// Role effective at the time.
    pub role: String,
    /// Capability name (`incident_resolve`, ...).
    pub capability: String,
    /// Bounded capability family for telemetry.
    pub capability_class: String,
    /// Target object identity (`inc_018`, `INV-104`, `session:<prefix>`, ...).
    pub target: String,
    /// Outcome (`permitted`, `denied`, `requested`, `confirmed`, `failed`,
    /// `state_unknown`, `step_up_required`).
    pub outcome: String,
    /// Operator reason, redacted.
    pub reason: String,
    /// Digest of the object state the form was prepared from.
    pub before_digest: String,
    /// Digest of the intended or resulting state.
    pub after_digest: String,
    /// The idempotency key fingerprint (`sha256:<hex>`), never the raw key.
    pub idempotency_key: String,
    /// The result page id, when a result page exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    /// The durable job id this action wrote, when it wrote one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Correlation id shared with any emitted analytics event.
    pub correlation_id: String,
}

/// A journal write failure. A failed journal write refuses the action: an
/// action that cannot be recorded does not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    /// The journal file could not be created, opened or written.
    #[error("journal write failed at {path}: {reason}")]
    Write {
        /// Journal path.
        path: String,
        /// Underlying reason.
        reason: String,
    },
    /// The journal could not be read back.
    #[error("journal read failed at {path}: {reason}")]
    Read {
        /// Journal path.
        path: String,
        /// Underlying reason.
        reason: String,
    },
}

/// The append-only journal handle.
#[derive(Debug, Clone)]
pub struct ActionJournal {
    path: PathBuf,
}

/// The journal read back, with corrupt lines named rather than dropped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JournalRead {
    /// Parsed entries, in file order.
    pub entries: Vec<ActionJournalEntry>,
    /// 1-based line numbers that did not parse.
    pub corrupt_lines: Vec<usize>,
}

impl ActionJournal {
    /// A journal at `path`. The file is created on the first append.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The journal path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one entry as a JSON line. The directory is created when
    /// missing; the write is a single `write_all` + flush of one line.
    pub fn append(&self, entry: &ActionJournalEntry) -> Result<(), JournalError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| JournalError::Write {
                path: self.path.display().to_string(),
                reason: format!("creating the journal directory failed: {error}"),
            })?;
        }
        let mut line = serde_json::to_string(entry).map_err(|error| JournalError::Write {
            path: self.path.display().to_string(),
            reason: format!("serializing the entry failed: {error}"),
        })?;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| JournalError::Write {
                path: self.path.display().to_string(),
                reason: error.to_string(),
            })?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.flush())
            .map_err(|error| JournalError::Write {
                path: self.path.display().to_string(),
                reason: error.to_string(),
            })
    }

    /// Read every entry back. A missing file reads as an empty journal (a
    /// fresh deployment has no history); an unreadable file is an error.
    pub fn read(&self) -> Result<JournalRead, JournalError> {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(JournalRead::default())
            }
            Err(error) => {
                return Err(JournalError::Read {
                    path: self.path.display().to_string(),
                    reason: error.to_string(),
                })
            }
        };
        let mut read = JournalRead::default();
        for (index, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ActionJournalEntry>(line) {
                Ok(entry) => read.entries.push(entry),
                Err(_) => read.corrupt_lines.push(index + 1),
            }
        }
        Ok(read)
    }
}

/// The in-memory index built from the journal: idempotency fingerprints to
/// result page ids, and object versions to their highest observed value.
#[derive(Debug, Clone, Default)]
pub struct JournalIndex {
    results_by_fingerprint: HashMap<String, String>,
}

impl JournalIndex {
    /// Build the index from journal entries.
    pub fn from_entries(entries: &[ActionJournalEntry]) -> Self {
        let mut results_by_fingerprint = HashMap::new();
        for entry in entries {
            if let Some(result_id) = &entry.result_id {
                results_by_fingerprint
                    .entry(entry.idempotency_key.clone())
                    .or_insert_with(|| result_id.clone());
            }
        }
        Self {
            results_by_fingerprint,
        }
    }

    /// The result page id a duplicate POST with this idempotency key must
    /// return, if the original is known.
    pub fn result_for(&self, idempotency_key: &str) -> Option<&str> {
        self.results_by_fingerprint
            .get(&idempotency_fingerprint(idempotency_key))
            .map(String::as_str)
    }

    /// Remember a result id for an idempotency key (the first write wins, so
    /// a duplicate read later still resolves to the same page).
    pub fn remember(&mut self, idempotency_key: &str, result_id: &str) {
        self.results_by_fingerprint
            .entry(idempotency_fingerprint(idempotency_key))
            .or_insert_with(|| result_id.to_string());
    }

    /// Number of indexed results.
    pub fn len(&self) -> usize {
        self.results_by_fingerprint.len()
    }

    /// True when no result is indexed.
    pub fn is_empty(&self) -> bool {
        self.results_by_fingerprint.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(idempotency_key: &str, result_id: Option<&str>) -> ActionJournalEntry {
        ActionJournalEntry {
            at_ms: 1_792_000_000_000,
            actor: "maya".to_string(),
            role: "operator".to_string(),
            capability: "incident_resolve".to_string(),
            capability_class: "incident".to_string(),
            target: "inc_018".to_string(),
            outcome: "requested".to_string(),
            reason: "recovery evidence: confirming read shows the floor restored".to_string(),
            before_digest: "aa".repeat(32),
            after_digest: "bb".repeat(32),
            idempotency_key: idempotency_fingerprint(idempotency_key),
            result_id: result_id.map(str::to_string),
            job_id: None,
            correlation_id: "corr-1".to_string(),
        }
    }

    #[test]
    fn append_and_read_round_trip_one_journal_line() {
        let dir = std::env::temp_dir().join(format!("apex-control-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = ActionJournal::new(dir.join("nested/journal.jsonl"));
        assert!(journal
            .read()
            .expect("missing reads empty")
            .entries
            .is_empty());
        let first = entry("0123456789abcdef0123456789abcdef", Some("res_1"));
        journal.append(&first).expect("append");
        journal
            .append(&entry("fedcba9876543210fedcba9876543210", None))
            .expect("append");
        let read = journal.read().expect("read");
        assert_eq!(read.entries.len(), 2);
        assert!(read.corrupt_lines.is_empty());
        assert_eq!(read.entries[0], first);
        // The file is one JSON object per line.
        let raw = std::fs::read_to_string(journal.path()).expect("raw");
        assert_eq!(raw.lines().count(), 2);
        assert!(raw.contains("\"capability\":\"incident_resolve\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_lines_are_named_not_dropped() {
        let dir = std::env::temp_dir().join(format!("apex-control-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, "{\"atMs\":1}\nnot json\n\n{\"also\":\"bad\"}\n").expect("write");
        let read = ActionJournal::new(&path).read().expect("read");
        assert!(read.entries.is_empty());
        assert_eq!(read.corrupt_lines, vec![1, 2, 4]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_index_answers_duplicates_with_the_original_result() {
        let key = "0123456789abcdef0123456789abcdef";
        let other = "00000000000000000000000000000000";
        let index = JournalIndex::from_entries(&[
            entry(key, Some("res_1")),
            entry(key, Some("res_2")),
            entry(other, None),
        ]);
        assert_eq!(index.result_for(key), Some("res_1"));
        assert_eq!(index.result_for(other), None);
        assert_eq!(index.len(), 1);
        let mut index = index;
        index.remember(other, "res_3");
        assert_eq!(index.result_for(other), Some("res_3"));
        // Remembering a duplicate does not overwrite the first result.
        index.remember(key, "res_9");
        assert_eq!(index.result_for(key), Some("res_1"));
    }

    #[test]
    fn journal_entries_never_carry_a_raw_idempotency_key() {
        let raw_key = "0123456789abcdef0123456789abcdef";
        let line = serde_json::to_string(&entry(raw_key, None)).expect("json");
        assert!(!line.contains(raw_key), "raw key leaked into the journal");
        assert!(line.contains("sha256:"));
    }
}
