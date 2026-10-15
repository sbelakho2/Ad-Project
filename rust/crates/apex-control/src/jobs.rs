//! Durable operator jobs in the frozen format the autopilot workstream
//! consumes.
//!
//! The control plane never performs a platform operation itself. `retry_read`,
//! `recheck_request` and `restore_reconcile_request` write one job file into
//! `APEX_OPERATOR_JOBS_DIR` (default `.apex/control/jobs`) and display its
//! state. The schema is frozen:
//!
//! ```json
//! {"job_id": "opj_<12hex>", "kind": "...", "requested_by": "...",
//!  "requested_at_ms": 0, "target": "...", "payload": {},
//!  "idempotency_key": "...", "payload_fingerprint": "sha256:<hex>",
//!  "state": "queued",
//!  "state_history": [{"state": "queued", "at_ms": 0, "note": "..."}],
//!  "outcome": null, "correlation_id": "..."}
//! ```
//!
//! Writes are atomic (temp file plus rename), so a reader never observes a
//! half-written job. A job file that cannot be parsed, or whose state is not
//! one of the known values, is displayed as `State unknown`, never guessed.
//!
//! A write is also idempotent by `idempotency_key` *and* payload fingerprint:
//! the key claim (scan) and the file write happen under one process-wide
//! lock, so a submission that already produced a durable job for the same
//! action reuses that job instead of writing a second file, while the same
//! key reused for a different target or payload reports a conflict and
//! changes nothing.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::signing::random_hex_32;

/// The job kinds the control plane writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Retry one safe platform read.
    RetryRead,
    /// Request a month-6 or triggered re-check.
    RecheckRequest,
    /// Ask the runner to reconcile a restore state.
    RestoreReconcileRequest,
    /// Retry a durable runner job.
    JobRetry,
}

impl JobKind {
    /// Stable name used inside the file.
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::RetryRead => "retry_read",
            JobKind::RecheckRequest => "recheck_request",
            JobKind::RestoreReconcileRequest => "restore_reconcile_request",
            JobKind::JobRetry => "job_retry",
        }
    }
}

/// One state transition of a job.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JobStateEntry {
    /// State name.
    pub state: String,
    /// When the transition was recorded (Unix milliseconds).
    pub at_ms: i64,
    /// A short note about the transition.
    pub note: String,
}

/// One operator job in the frozen file format.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperatorJob {
    /// `opj_<12 hex>`.
    pub job_id: String,
    /// Job kind.
    pub kind: JobKind,
    /// Operator identity that requested it.
    pub requested_by: String,
    /// Request time (Unix milliseconds).
    pub requested_at_ms: i64,
    /// The object the job targets.
    pub target: String,
    /// Job-specific payload (bounded, no secrets).
    pub payload: serde_json::Value,
    /// The originating idempotency key (raw; this file is a restricted
    /// durable record consumed by the runner, not an analytics event).
    pub idempotency_key: String,
    /// The fingerprint of the submitted action (kind, target and payload);
    /// a reused key with a different fingerprint is a conflict. Absent on
    /// job files written before fingerprints existed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub payload_fingerprint: String,
    /// Current state: `queued`, `running`, `completed` or `failed`.
    pub state: String,
    /// The full state history.
    pub state_history: Vec<JobStateEntry>,
    /// Terminal outcome, when one is recorded.
    pub outcome: Option<String>,
    /// Correlation id shared with the action journal.
    pub correlation_id: String,
}

impl OperatorJob {
    /// Build a fresh queued job with a generated `opj_` id.
    pub fn queued(
        kind: JobKind,
        requested_by: &str,
        requested_at_ms: i64,
        target: &str,
        payload: serde_json::Value,
        idempotency_key: &str,
        correlation_id: &str,
    ) -> Self {
        let note = format!("requested by {requested_by} from the operator control plane");
        let payload_fingerprint =
            crate::signing::payload_fingerprint(kind.as_str(), target, &payload);
        Self {
            job_id: format!("opj_{}", &random_hex_32()[..12]),
            kind,
            requested_by: requested_by.to_string(),
            requested_at_ms,
            target: target.to_string(),
            payload,
            idempotency_key: idempotency_key.to_string(),
            payload_fingerprint,
            state: "queued".to_string(),
            state_history: vec![JobStateEntry {
                state: "queued".to_string(),
                at_ms: requested_at_ms,
                note,
            }],
            outcome: None,
            correlation_id: correlation_id.to_string(),
        }
    }

    /// The display state. A state outside the known set (a hand-edited or
    /// newer file) displays as `State unknown` rather than a guess.
    pub fn display_state(&self) -> &str {
        match self.state.as_str() {
            "queued" => "Queued",
            "running" => "Running",
            "completed" => "Completed",
            "failed" => "Failed",
            _ => "State unknown",
        }
    }
}

/// A job directory failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobError {
    /// The job file could not be written.
    #[error("job write failed at {path}: {reason}")]
    Write {
        /// File involved.
        path: String,
        /// Underlying reason.
        reason: String,
    },
    /// The job directory could not be listed.
    #[error("job directory read failed at {path}: {reason}")]
    Read {
        /// Directory involved.
        path: String,
        /// Underlying reason.
        reason: String,
    },
}

/// One row of the job listing: a parsed job or an unreadable file. The job
/// is boxed so a listing row stays small next to the unreadable-file variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobRead {
    /// A parsed job.
    Parsed(Box<OperatorJob>),
    /// A file that exists but cannot be trusted.
    StateUnknown {
        /// File name.
        file: String,
        /// Why it cannot be trusted.
        reason: String,
    },
}

impl JobRead {
    /// The stable sort key (job id or file name).
    pub fn sort_key(&self) -> &str {
        match self {
            JobRead::Parsed(job) => &job.job_id,
            JobRead::StateUnknown { file, .. } => file,
        }
    }

    /// The display state.
    pub fn display_state(&self) -> &str {
        match self {
            JobRead::Parsed(job) => job.display_state(),
            JobRead::StateUnknown { .. } => "State unknown",
        }
    }

    /// The target, when known.
    pub fn target(&self) -> &str {
        match self {
            JobRead::Parsed(job) => &job.target,
            JobRead::StateUnknown { file, .. } => file,
        }
    }

    /// The kind name, when known.
    pub fn kind_str(&self) -> &str {
        match self {
            JobRead::Parsed(job) => job.kind.as_str(),
            JobRead::StateUnknown { .. } => "unknown",
        }
    }
}

/// The outcome of an idempotent job write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobWrite {
    /// The job file was written.
    Created(OperatorJob),
    /// The idempotency key already claimed an identical action (same payload
    /// fingerprint): the existing job is returned, nothing new is written.
    Duplicate(OperatorJob),
    /// The idempotency key already claimed a different action: the caller
    /// must answer with the conflict page and change nothing.
    KeyConflict(OperatorJob),
}

impl JobWrite {
    /// The durable job involved: the created or existing one.
    pub fn job(&self) -> &OperatorJob {
        match self {
            JobWrite::Created(job) | JobWrite::Duplicate(job) | JobWrite::KeyConflict(job) => job,
        }
    }

    /// Whether this call created the file.
    pub fn created(&self) -> bool {
        matches!(self, JobWrite::Created(_))
    }
}

/// The durable job directory.
#[derive(Debug, Clone)]
pub struct JobStore {
    dir: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl JobStore {
    /// A store rooted at `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// The directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write one job atomically (temp file plus rename), idempotent by
    /// `idempotency_key` and payload fingerprint: the key claim and the
    /// write share one process-wide lock; a job whose key is already durable
    /// is returned unchanged when the fingerprint matches, and reported as a
    /// conflict when it does not.
    pub fn write(&self, job: &OperatorJob) -> Result<JobWrite, JobError> {
        // The claim and the write are one step: two concurrent submissions
        // of the same form can never produce two durable jobs.
        let _guard = match self.write_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(existing) = self.existing_for_key(&job.idempotency_key)? {
            // A legacy job without a recorded fingerprint cannot establish a
            // difference and keeps the duplicate behavior.
            let same_action = existing.payload_fingerprint.is_empty()
                || existing.payload_fingerprint == job.payload_fingerprint;
            return Ok(if same_action {
                JobWrite::Duplicate(existing)
            } else {
                JobWrite::KeyConflict(existing)
            });
        }
        std::fs::create_dir_all(&self.dir).map_err(|error| JobError::Write {
            path: self.dir.display().to_string(),
            reason: format!("creating the job directory failed: {error}"),
        })?;
        let final_path = self.dir.join(format!("{}.json", job.job_id));
        let temp_path = self.dir.join(format!(".{}.json.tmp", job.job_id));
        let body = serde_json::to_string_pretty(job).map_err(|error| JobError::Write {
            path: final_path.display().to_string(),
            reason: format!("serializing the job failed: {error}"),
        })?;
        std::fs::write(&temp_path, body).map_err(|error| JobError::Write {
            path: temp_path.display().to_string(),
            reason: error.to_string(),
        })?;
        std::fs::rename(&temp_path, &final_path).map_err(|error| JobError::Write {
            path: final_path.display().to_string(),
            reason: format!("atomic rename failed: {error}"),
        })?;
        Ok(JobWrite::Created(job.clone()))
    }

    /// The first durable job written under `idempotency_key`, when any.
    fn existing_for_key(&self, idempotency_key: &str) -> Result<Option<OperatorJob>, JobError> {
        for row in self.list()? {
            if let JobRead::Parsed(job) = row {
                if job.idempotency_key == idempotency_key {
                    return Ok(Some(*job));
                }
            }
        }
        Ok(None)
    }

    /// Read one job file back.
    pub fn read(&self, job_id: &str) -> Result<JobRead, JobError> {
        let path = self.dir.join(format!("{job_id}.json"));
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                return Err(JobError::Read {
                    path: path.display().to_string(),
                    reason: error.to_string(),
                })
            }
        };
        Ok(parse_job(&format!("{job_id}.json"), &raw))
    }

    /// List every job file, sorted by job id. Temporary files are ignored;
    /// a file that cannot be parsed appears as `State unknown`.
    pub fn list(&self) -> Result<Vec<JobRead>, JobError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(JobError::Read {
                    path: self.dir.display().to_string(),
                    reason: error.to_string(),
                })
            }
        };
        let mut rows = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| JobError::Read {
                path: self.dir.display().to_string(),
                reason: error.to_string(),
            })?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") || name.starts_with('.') {
                continue;
            }
            match std::fs::read_to_string(entry.path()) {
                Ok(raw) => rows.push(parse_job(&name, &raw)),
                Err(error) => rows.push(JobRead::StateUnknown {
                    file: name,
                    reason: error.to_string(),
                }),
            }
        }
        rows.sort_by(|left, right| left.sort_key().cmp(right.sort_key()));
        Ok(rows)
    }
}

fn parse_job(file: &str, raw: &str) -> JobRead {
    match serde_json::from_str::<OperatorJob>(raw) {
        Ok(job) => {
            if job.state_history.is_empty() {
                return JobRead::StateUnknown {
                    file: file.to_string(),
                    reason: "the job carries no state history".to_string(),
                };
            }
            JobRead::Parsed(Box::new(job))
        }
        Err(error) => JobRead::StateUnknown {
            file: file.to_string(),
            reason: format!("the job file did not parse: {error}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (JobStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-jobs-{}-{}",
            std::process::id(),
            crate::signing::random_hex_32()
        ));
        (JobStore::new(dir.clone()), dir)
    }

    #[test]
    fn the_frozen_shape_round_trips() {
        let (store, dir) = store();
        let job = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1_792_000_000_000,
            "acct_northstar_max_01",
            serde_json::json!({"platform": "applovin_max", "account": "acct_northstar_max_01"}),
            "0123456789abcdef0123456789abcdef",
            "corr-1234",
        );
        assert!(job.job_id.starts_with("opj_"));
        assert_eq!(job.job_id.len(), "opj_".len() + 12);
        assert_eq!(job.state, "queued");
        assert_eq!(job.state_history.len(), 1);
        assert_eq!(job.state_history[0].state, "queued");
        let written = store.write(&job).expect("write");
        assert!(written.created());
        assert_eq!(written.job(), &job);
        let path = store.dir().join(format!("{}.json", job.job_id));
        assert!(path.exists());
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some(format!("{}.json", job.job_id).as_str())
        );
        let read = store.read(&job.job_id).expect("read");
        match read {
            JobRead::Parsed(back) => assert_eq!(*back, job),
            other => panic!("expected a parsed job, got {other:?}"),
        }
        // The frozen field names are exactly these.
        let raw = std::fs::read_to_string(&path).expect("raw");
        for field in [
            "\"job_id\"",
            "\"kind\"",
            "\"requested_by\"",
            "\"requested_at_ms\"",
            "\"target\"",
            "\"payload\"",
            "\"idempotency_key\"",
            "\"payload_fingerprint\"",
            "\"state\"",
            "\"state_history\"",
            "\"outcome\"",
            "\"correlation_id\"",
        ] {
            assert!(raw.contains(field), "missing field {field}");
        }
        // No temp file is left behind.
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn listing_sorts_and_reports_unknown_files_as_state_unknown() {
        let (store, dir) = store();
        let mut first = OperatorJob::queued(
            JobKind::RecheckRequest,
            "sam",
            1,
            "ch_0246:2026-10",
            serde_json::json!({}),
            "a",
            "c1",
        );
        first.job_id = "opj_000000000001".to_string();
        let mut second = OperatorJob::queued(
            JobKind::RestoreReconcileRequest,
            "maya",
            2,
            "exp_0247",
            serde_json::json!({}),
            "b",
            "c2",
        );
        second.job_id = "opj_000000000002".to_string();
        store.write(&second).expect("write");
        store.write(&first).expect("write");
        std::fs::write(dir.join("opj_broken.json"), "{ not json").expect("broken");
        let rows = store.list().expect("list");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].sort_key(), "opj_000000000001");
        assert_eq!(rows[1].sort_key(), "opj_000000000002");
        assert_eq!(rows[0].display_state(), "Queued");
        assert_eq!(rows[2].display_state(), "State unknown");
        assert_eq!(rows[2].kind_str(), "unknown");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_write_with_the_same_key_returns_the_first_job() {
        let (store, dir) = store();
        let first = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1,
            "acct_northstar_max_01",
            serde_json::json!({"platform": "applovin_max"}),
            "the-same-key",
            "corr-1",
        );
        let second = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            2,
            "acct_northstar_max_01",
            serde_json::json!({"platform": "applovin_max"}),
            "the-same-key",
            "corr-2",
        );
        assert_ne!(first.job_id, second.job_id);
        let written = store.write(&first).expect("first write");
        assert!(written.created());
        assert_eq!(written.job().job_id, first.job_id);
        let again = store.write(&second).expect("second write");
        assert_eq!(again, JobWrite::Duplicate(first.clone()));
        // Exactly one durable file exists, under the first job id; the
        // second submission's id was never written.
        assert!(dir.join(format!("{}.json", first.job_id)).exists());
        assert!(!dir.join(format!("{}.json", second.job_id)).exists());
        let rows = store.list().expect("list");
        assert_eq!(rows.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_writes_of_one_key_leave_exactly_one_job() {
        let (store, dir) = store();
        let jobs: Vec<OperatorJob> = (0..8)
            .map(|index| {
                OperatorJob::queued(
                    JobKind::RecheckRequest,
                    "maya",
                    index,
                    "ch_0246",
                    serde_json::json!({}),
                    "one-shared-key",
                    "corr",
                )
            })
            .collect();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(jobs.len()));
        let race = std::sync::Arc::clone(&barrier);
        let writers: Vec<std::thread::JoinHandle<JobWrite>> = jobs
            .iter()
            .cloned()
            .map(|job| {
                let store = store.clone();
                let race = std::sync::Arc::clone(&race);
                std::thread::spawn(move || {
                    race.wait();
                    store.write(&job).expect("write")
                })
            })
            .collect();
        let outcomes: Vec<JobWrite> = writers
            .into_iter()
            .map(|writer| writer.join().expect("join"))
            .collect();
        assert_eq!(
            outcomes.iter().filter(|outcome| outcome.created()).count(),
            1
        );
        let winner = outcomes
            .iter()
            .find(|outcome| outcome.created())
            .expect("one creator");
        for outcome in &outcomes {
            assert_eq!(outcome.job().job_id, winner.job().job_id);
            assert!(!matches!(outcome, JobWrite::KeyConflict(_)));
        }
        assert_eq!(store.list().expect("list").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_same_key_with_a_different_action_is_a_conflict() {
        let (store, dir) = store();
        let first = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1,
            "acct_northstar_max_01",
            serde_json::json!({"platform": "applovin_max"}),
            "one-key",
            "corr-1",
        );
        // Same key and kind but a different target and payload.
        let other = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            2,
            "acct_northstar_max_02",
            serde_json::json!({"platform": "applovin_max"}),
            "one-key",
            "corr-2",
        );
        let written = store.write(&first).expect("first write");
        assert!(written.created());
        let conflict = store.write(&other).expect("conflicting write");
        match &conflict {
            JobWrite::KeyConflict(existing) => assert_eq!(existing.job_id, first.job_id),
            other => panic!("expected a key conflict, got {other:?}"),
        }
        assert!(!conflict.created());
        // Nothing changed: one file, under the first job id.
        assert!(dir.join(format!("{}.json", first.job_id)).exists());
        assert!(!dir.join(format!("{}.json", other.job_id)).exists());
        assert_eq!(store.list().expect("list").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unrecognized_state_displays_state_unknown() {
        let mut job = OperatorJob::queued(
            JobKind::JobRetry,
            "lina",
            3,
            "job_daily",
            serde_json::json!({}),
            "c",
            "c3",
        );
        job.state = "reconciling".to_string();
        assert_eq!(job.display_state(), "State unknown");
    }

    #[test]
    fn missing_directories_read_as_empty_and_missing_files_error() {
        let (store, dir) = store();
        assert!(store.list().expect("empty").is_empty());
        assert!(store.read("opj_missing").is_err());
        assert!(!dir.exists());
    }
}
