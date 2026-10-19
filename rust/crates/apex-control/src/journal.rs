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
    /// The fingerprint of the submitted action (capability, target and
    /// payload). A reused key with a different fingerprint is a conflict.
    /// Absent on journal lines written before fingerprints existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_fingerprint: Option<String>,
    /// The result page id, when a result page exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    /// The durable job id this action wrote, when it wrote one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Correlation id shared with any emitted analytics event.
    pub correlation_id: String,
}

impl ActionJournalEntry {
    /// The stable key the access page uses to identify one action.
    pub fn action_key(&self) -> String {
        format!("{}:{}:{}", self.at_ms, self.capability, self.actor)
    }
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
    #[cfg(test)]
    reads: std::sync::Arc<std::sync::atomic::AtomicU64>,
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
        Self {
            path: path.into(),
            #[cfg(test)]
            reads: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// The number of file reads this handle performed (test seam).
    #[cfg(test)]
    pub(crate) fn read_count(&self) -> u64 {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
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
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
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
        #[cfg(test)]
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

/// One remembered submission: the result page id plus the fingerprint of the
/// action that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberedResult {
    /// The result page id (`res_<12 hex>`).
    pub result_id: String,
    /// The payload fingerprint of the remembered action; `None` on records
    /// rebuilt from journal lines written before fingerprints existed.
    pub payload_fingerprint: Option<String>,
}

impl RememberedResult {
    /// Whether a submission carrying `payload_fingerprint` may reuse this
    /// result. A recorded fingerprint must match exactly; a legacy record
    /// without one cannot establish a difference, so it keeps the original
    /// duplicate behavior.
    pub fn matches(&self, payload_fingerprint: &str) -> bool {
        match &self.payload_fingerprint {
            Some(recorded) => recorded == payload_fingerprint,
            None => true,
        }
    }
}

/// The minimal result record the result page needs: outcome, durable job and
/// note. The page never reads the journal again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedResult {
    /// Result page id (`res_<12 hex>`).
    pub result_id: String,
    /// Journal outcome.
    pub outcome: String,
    /// Durable job id, when one exists.
    pub job_id: Option<String>,
    /// Redacted operator note.
    pub reason: String,
}

/// The minimal action record the access-page review form needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedAction {
    /// When the action was processed.
    pub at_ms: i64,
    /// Operator identity.
    pub actor: String,
    /// Capability name.
    pub capability: String,
    /// Journal outcome.
    pub outcome: String,
    /// Target identity.
    pub target: String,
}

/// The in-memory index built from the journal: idempotency fingerprints to
/// remembered results, result ids to the page fields, action keys to the
/// review fields, and the journal entries themselves for the access history.
/// It is updated on every successful append, so pages read memory instead of
/// re-parsing the file.
#[derive(Debug, Clone, Default)]
pub struct JournalIndex {
    results_by_fingerprint: HashMap<String, RememberedResult>,
    results_by_id: HashMap<String, IndexedResult>,
    actions_by_key: HashMap<String, IndexedAction>,
    entries: Vec<ActionJournalEntry>,
}

impl JournalIndex {
    /// Build the index from journal entries. The first record for a result
    /// id or an action key wins, matching the historical linear search.
    pub fn from_entries(entries: &[ActionJournalEntry]) -> Self {
        let mut index = Self::default();
        for entry in entries {
            index.observe(entry.clone());
        }
        index
    }

    /// Add one successfully appended entry to every index.
    pub fn observe(&mut self, entry: ActionJournalEntry) {
        if let Some(result_id) = &entry.result_id {
            self.results_by_fingerprint
                .entry(entry.idempotency_key.clone())
                .or_insert_with(|| RememberedResult {
                    result_id: result_id.clone(),
                    payload_fingerprint: entry.payload_fingerprint.clone(),
                });
            self.results_by_id
                .entry(result_id.clone())
                .or_insert_with(|| IndexedResult {
                    result_id: result_id.clone(),
                    outcome: entry.outcome.clone(),
                    job_id: entry.job_id.clone(),
                    reason: entry.reason.clone(),
                });
        }
        self.actions_by_key
            .entry(entry.action_key())
            .or_insert_with(|| IndexedAction {
                at_ms: entry.at_ms,
                actor: entry.actor.clone(),
                capability: entry.capability.clone(),
                outcome: entry.outcome.clone(),
                target: entry.target.clone(),
            });
        self.entries.push(entry);
    }

    /// The remembered result a duplicate POST with this idempotency key must
    /// consider, if the original is known.
    pub fn result_for(&self, idempotency_key: &str) -> Option<&RememberedResult> {
        self.results_by_fingerprint
            .get(&idempotency_fingerprint(idempotency_key))
    }

    /// Remember a result id for an idempotency key and payload fingerprint
    /// (the first write wins, so a duplicate read later still resolves to the
    /// same page).
    pub fn remember(&mut self, idempotency_key: &str, payload_fingerprint: &str, result_id: &str) {
        self.results_by_fingerprint
            .entry(idempotency_fingerprint(idempotency_key))
            .or_insert_with(|| RememberedResult {
                result_id: result_id.to_string(),
                payload_fingerprint: Some(payload_fingerprint.to_string()),
            });
    }

    /// The result page fields for `result_id`, when indexed.
    pub fn result_by_id(&self, result_id: &str) -> Option<&IndexedResult> {
        self.results_by_id.get(result_id)
    }

    /// The action record for one `action_target_key`, when indexed.
    pub fn action_by_key(&self, key: &str) -> Option<&IndexedAction> {
        self.actions_by_key.get(key)
    }

    /// The journal entries in file order (the access history).
    pub fn entries(&self) -> &[ActionJournalEntry] {
        &self.entries
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
            payload_fingerprint: Some(crate::signing::payload_fingerprint(
                "incident_resolve",
                "inc_018",
                &serde_json::json!({}),
            )),
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

    #[cfg(unix)]
    #[test]
    fn the_journal_is_created_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir =
            std::env::temp_dir().join(format!("apex-control-journal-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = ActionJournal::new(dir.join("journal.jsonl"));
        journal
            .append(&entry("0123456789abcdef0123456789abcdef", None))
            .expect("append");
        let mode = std::fs::metadata(journal.path())
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "journal file");
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
        let remembered = index.result_for(key).expect("remembered");
        assert_eq!(remembered.result_id, "res_1");
        assert!(remembered.matches(&crate::signing::payload_fingerprint(
            "incident_resolve",
            "inc_018",
            &serde_json::json!({})
        )));
        assert!(!remembered.matches("sha256:different"));
        assert_eq!(index.result_for(other), None);
        assert_eq!(index.len(), 1);
        let mut index = index;
        index.remember(other, "sha256:fresh", "res_3");
        assert_eq!(
            index.result_for(other).expect("remembered").result_id,
            "res_3"
        );
        // Remembering a duplicate does not overwrite the first result.
        index.remember(key, "sha256:fresh", "res_9");
        assert_eq!(
            index.result_for(key).expect("remembered").result_id,
            "res_1"
        );
    }

    #[test]
    fn the_index_keeps_the_first_result_id_and_ignores_corrupt_lines() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-index-duplicate-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("journal.jsonl");
        let first = entry("0123456789abcdef0123456789abcdef", Some("res_first"));
        let duplicate = entry("fedcba9876543210fedcba9876543210", Some("res_first"));
        let mut raw = serde_json::to_string(&first).expect("json");
        raw.push_str("\nnot json\n");
        raw.push_str(&serde_json::to_string(&duplicate).expect("json"));
        raw.push('\n');
        std::fs::write(&path, raw).expect("write");
        let read = ActionJournal::new(&path).read().expect("read");
        assert_eq!(read.corrupt_lines, vec![2]);
        let index = JournalIndex::from_entries(&read.entries);
        let result = index.result_by_id("res_first").expect("indexed");
        assert_eq!(result.outcome, "requested");
        assert_eq!(result.reason, first.reason);
        assert!(index.result_by_id("res_missing").is_none());
        assert_eq!(index.entries().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_legacy_record_without_a_fingerprint_matches_any_payload() {
        let key = "0123456789abcdef0123456789abcdef";
        let mut legacy = entry(key, Some("res_1"));
        legacy.payload_fingerprint = None;
        let index = JournalIndex::from_entries(&[legacy]);
        let remembered = index.result_for(key).expect("remembered");
        assert_eq!(remembered.result_id, "res_1");
        assert!(remembered.matches("sha256:anything"));
    }

    #[test]
    fn journal_entries_never_carry_a_raw_idempotency_key() {
        let raw_key = "0123456789abcdef0123456789abcdef";
        let line = serde_json::to_string(&entry(raw_key, None)).expect("json");
        assert!(!line.contains(raw_key), "raw key leaked into the journal");
        assert!(line.contains("sha256:"));
    }
}
