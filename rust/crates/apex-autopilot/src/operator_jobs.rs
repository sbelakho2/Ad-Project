//! The operator-jobs consumer: the frozen job-store contract.
//!
//! The control plane writes one JSON file per operator job into
//! `APEX_OPERATOR_JOBS_DIR` (default `.apex/control/jobs`); the runner's
//! daily pass consumes the kinds `retry_read`, `recheck_request` and
//! `restore_reconcile`. The file format is frozen:
//!
//! ```json
//! {"job_id": "opj_<12hex>", "kind": "...", "requested_by": "...",
//!  "requested_at_ms": 0, "target": "...", "payload": {},
//!  "idempotency_key": "...", "state": "queued",
//!  "state_history": [{"state": "queued", "at_ms": 0, "note": "..."}],
//!  "outcome": null, "correlation_id": "..."}
//! ```
//!
//! Transitions are atomic (temp file plus rename) and append to
//! `state_history`. A non-`queued` job is skipped, so re-running a completed
//! job is a no-op: the file's terminal state is the deduplication record.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::notify::NotifyPlan;
use crate::ops::PlatformOps;
use crate::telemetry::RunnerTelemetry;

/// The environment variable naming the jobs directory.
pub const JOBS_DIR_ENV: &str = "APEX_OPERATOR_JOBS_DIR";
/// The default jobs directory (`.apex/control/jobs`).
pub const DEFAULT_JOBS_DIR: &str = ".apex/control/jobs";

/// One recorded state transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateChange {
    /// The state after the transition.
    pub state: String,
    /// When the transition happened, unix milliseconds.
    pub at_ms: i64,
    /// A short human note.
    #[serde(default)]
    pub note: String,
}

/// One operator job exactly as the control plane stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperatorJob {
    /// The durable job identity, `opj_<12hex>`.
    pub job_id: String,
    /// The job kind; the runner consumes `retry_read`, `recheck_request` and
    /// `restore_reconcile`.
    pub kind: String,
    /// Who requested it.
    pub requested_by: String,
    /// When it was requested, unix milliseconds.
    pub requested_at_ms: i64,
    /// What object the job targets.
    pub target: String,
    /// The job's parameters.
    #[serde(default)]
    pub payload: Value,
    /// The requester's idempotency key.
    pub idempotency_key: String,
    /// `queued`, `running`, `completed`, `failed` or `unknown`.
    pub state: String,
    /// The transition history, oldest first.
    #[serde(default)]
    pub state_history: Vec<StateChange>,
    /// The terminal outcome note.
    #[serde(default)]
    pub outcome: Value,
    /// The correlation id shared with the requesting workflow.
    pub correlation_id: String,
}

/// The job states of the frozen contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Waiting for a consumer.
    Queued,
    /// Claimed by a consumer.
    Running,
    /// Finished successfully.
    Completed,
    /// Finished with a failure.
    Failed,
    /// The platform state could not be established.
    Unknown,
}

impl JobState {
    /// The stable state name.
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Completed => "completed",
            JobState::Failed => "failed",
            JobState::Unknown => "unknown",
        }
    }

    /// Parse a stored state; an unrecognized state is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "queued" => Some(JobState::Queued),
            "running" => Some(JobState::Running),
            "completed" => Some(JobState::Completed),
            "failed" => Some(JobState::Failed),
            "unknown" => Some(JobState::Unknown),
            _ => None,
        }
    }
}

/// Why the jobs directory could not be consumed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OperatorJobError {
    /// The directory could not be read.
    #[error("operator jobs directory {path}: {reason}")]
    Io {
        /// The directory or file involved.
        path: String,
        /// The reason.
        reason: String,
    },
}

/// What one daily pass consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OperatorJobsReport {
    /// Jobs the pass completed.
    pub completed: u32,
    /// Jobs that ended failed.
    pub failed: u32,
    /// Jobs whose platform state is unknown.
    pub unknown: u32,
    /// Jobs left alone (not queued, or a different kind).
    pub skipped: u32,
}

impl OperatorJobsReport {
    /// Whether the pass saw any job file at all.
    pub fn any(&self) -> bool {
        !(self.completed == 0 && self.failed == 0 && self.unknown == 0 && self.skipped == 0)
    }
}

/// Consume every queued job in `dir`.
///
/// A corrupt file is reported on stderr and counted as failed; it is never
/// rewritten (the control plane's bytes are not ours to guess at). The rest
/// of the directory is still consumed: one bad file must not starve the
/// queue.
pub async fn consume_operator_jobs(
    dir: &Path,
    ops: &dyn PlatformOps,
    due_rechecks: &dyn Fn(&str) -> bool,
    notifier: Option<&NotifyPlan>,
    telemetry: Option<&RunnerTelemetry>,
    publisher: &str,
    now_unix_ms: i64,
) -> Result<OperatorJobsReport, OperatorJobError> {
    let mut report = OperatorJobsReport::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => {
            return Err(OperatorJobError::Io {
                path: dir.display().to_string(),
                reason: error.to_string(),
            })
        }
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            files.push(path);
        }
    }
    files.sort();

    for path in files {
        let raw = std::fs::read_to_string(&path).map_err(|error| OperatorJobError::Io {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
        let mut job: OperatorJob = match serde_json::from_str(&raw) {
            Ok(job) => job,
            Err(error) => {
                eprintln!(
                    "apex-operator-jobs: {} is not a readable job file: {error}",
                    path.display()
                );
                report.failed += 1;
                continue;
            }
        };
        if !is_job_id(&job.job_id) {
            eprintln!(
                "apex-operator-jobs: {} carries job id '{}' which is not opj_<12hex>; leaving it alone",
                path.display(),
                job.job_id
            );
            report.failed += 1;
            continue;
        }
        let Some(state) = JobState::parse(&job.state) else {
            eprintln!(
                "apex-operator-jobs: job {} has unknown state '{}'; leaving it alone",
                job.job_id, job.state
            );
            report.skipped += 1;
            continue;
        };
        if state != JobState::Queued {
            // Deduplication: a terminal or claimed job is a no-op forever.
            report.skipped += 1;
            continue;
        }

        job.state = JobState::Running.as_str().to_string();
        job.state_history.push(StateChange {
            state: JobState::Running.as_str().to_string(),
            at_ms: now_unix_ms,
            note: format!("claimed by the {} runner pass", publisher),
        });
        write_job(&path, &job)?;

        let (terminal, note) = execute(&job, ops, due_rechecks).await;
        job.state = terminal.as_str().to_string();
        job.state_history.push(StateChange {
            state: terminal.as_str().to_string(),
            at_ms: now_unix_ms,
            note: note.clone(),
        });
        job.outcome = Value::String(note);
        write_job(&path, &job)?;

        match terminal {
            JobState::Completed => report.completed += 1,
            JobState::Failed => report.failed += 1,
            JobState::Unknown => report.unknown += 1,
            JobState::Queued | JobState::Running => report.skipped += 1,
        }
        if let Some(telemetry) = telemetry {
            if let Some((capability, action)) = capability_of(&job.kind) {
                telemetry.operator_action(capability, action, outcome_of(terminal));
            }
        }
        if let Some(notifier) = notifier {
            // A reconcile that cannot establish the platform state is the one
            // consumer outcome that reaches the customer: the restore is not
            // confirmed, so the incident makes the state honest.
            if job.kind == "restore_reconcile" && terminal == JobState::Unknown {
                notifier
                    .restore_failed(
                        publisher,
                        &job.target,
                        &crate::notify::incident_id_for(&job.target),
                        now_unix_ms,
                    )
                    .await;
            }
        }
    }
    Ok(report)
}

/// The frozen job id shape: `opj_` plus twelve lowercase hex digits.
fn is_job_id(job_id: &str) -> bool {
    let Some(hex) = job_id.strip_prefix("opj_") else {
        return false;
    };
    hex.len() == 12
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Execute one claimed job and return its terminal state with a note.
async fn execute(
    job: &OperatorJob,
    ops: &dyn PlatformOps,
    due_rechecks: &dyn Fn(&str) -> bool,
) -> (JobState, String) {
    match job.kind.as_str() {
        "retry_read" => match ops.experiment(&job.target).await {
            Ok(result) => (
                JobState::Completed,
                format!("state re-read: {:?}", result.state),
            ),
            Err(error) => (JobState::Failed, bounded(error.to_string())),
        },
        "recheck_request" => {
            if due_rechecks(&job.target) {
                (
                    JobState::Completed,
                    format!("recheck is due for {}", job.target),
                )
            } else {
                (
                    JobState::Completed,
                    format!("no recheck is due for {}", job.target),
                )
            }
        }
        "restore_reconcile" => match ops.experiment(&job.target).await {
            Ok(result) if result.state == apex_core::ExperimentState::Stopped => (
                JobState::Completed,
                format!("the platform confirms {} is stopped", job.target),
            ),
            Ok(result) => (
                JobState::Unknown,
                format!(
                    "the platform still reports {} for {}",
                    state_label(result.state),
                    job.target
                ),
            ),
            Err(error) => (JobState::Failed, bounded(error.to_string())),
        },
        other => (JobState::Failed, format!("unsupported job kind '{other}'")),
    }
}

fn state_label(state: apex_core::ExperimentState) -> &'static str {
    match state {
        apex_core::ExperimentState::Running => "running",
        apex_core::ExperimentState::Stopped => "stopped",
        apex_core::ExperimentState::Completed => "completed",
    }
}

fn capability_of(
    kind: &str,
) -> Option<(
    apex_metrics::vocab::CapabilityClass,
    apex_metrics::vocab::ActionClass,
)> {
    use apex_metrics::vocab::{ActionClass, CapabilityClass};
    match kind {
        "retry_read" => Some((CapabilityClass::Reporting, ActionClass::RetryRead)),
        "recheck_request" => Some((CapabilityClass::Experiment, ActionClass::Recheck)),
        "restore_reconcile" => Some((CapabilityClass::Restore, ActionClass::Reconcile)),
        _ => None,
    }
}

fn bounded(text: String) -> String {
    crate::text::bounded_chars(&text, 200)
}

/// The bounded metric outcome of a terminal job state.
fn outcome_of(state: JobState) -> apex_metrics::vocab::Outcome {
    use apex_metrics::vocab::Outcome;
    match state {
        JobState::Completed => Outcome::Completed,
        JobState::Failed => Outcome::Failed,
        JobState::Unknown => Outcome::Unknown,
        JobState::Queued | JobState::Running => Outcome::Skipped,
    }
}

/// Write the job atomically (temp file plus rename).
fn write_job(path: &Path, job: &OperatorJob) -> Result<(), OperatorJobError> {
    let json = serde_json::to_string_pretty(job).map_err(|error| OperatorJobError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, json).map_err(|error| OperatorJobError::Io {
        path: tmp.display().to_string(),
        reason: error.to_string(),
    })?;
    std::fs::rename(&tmp, path).map_err(|error| OperatorJobError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use super::*;
    use crate::ops::RecordingOps;

    fn job_file(dir: &Path, id: &str, kind: &str, target: &str, state: &str) -> PathBuf {
        let job = OperatorJob {
            job_id: id.to_string(),
            kind: kind.to_string(),
            requested_by: "maya".to_string(),
            requested_at_ms: 1_700_000_000_000,
            target: target.to_string(),
            payload: serde_json::json!({}),
            idempotency_key: format!("k-{id}"),
            state: state.to_string(),
            state_history: vec![StateChange {
                state: state.to_string(),
                at_ms: 1_700_000_000_000,
                note: "requested".to_string(),
            }],
            outcome: Value::Null,
            correlation_id: "corr_1".to_string(),
        };
        let path = dir.join(format!("{id}.json"));
        write_job(&path, &job).expect("write");
        path
    }

    fn ops_with(state: apex_core::ExperimentState) -> Arc<RecordingOps> {
        let ops = Arc::new(RecordingOps::new());
        ops.put_experiment(
            "exp_0247",
            apex_core::ExperimentResult {
                experiment_id: "exp_0247".into(),
                state,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops
    }

    #[tokio::test]
    async fn queued_jobs_transition_atomically_and_reruns_are_no_ops() {
        let dir = std::env::temp_dir().join(format!("apex-opsjobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        job_file(&dir, "opj_000000000001", "retry_read", "exp_0247", "queued");
        job_file(
            &dir,
            "opj_000000000002",
            "recheck_request",
            "foot",
            "queued",
        );
        job_file(
            &dir,
            "opj_000000000003",
            "restore_reconcile",
            "exp_0247",
            "queued",
        );
        job_file(&dir, "opj_000000000004", "mystery_kind", "x", "queued");
        let ops = ops_with(apex_core::ExperimentState::Running);
        let due = |change: &str| change == "foot";
        let report = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &due,
            None,
            None,
            "northstar",
            1_700_000_001_000,
        )
        .await
        .expect("consume");
        assert_eq!(report.completed, 2);
        assert_eq!(report.unknown, 1, "running restore is unknown");
        assert_eq!(report.failed, 1, "unsupported kind fails");
        assert_eq!(report.skipped, 0);
        assert!(report.any());

        let first: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000001.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(first.state, "completed");
        assert_eq!(first.state_history.len(), 3, "queued + running + completed");
        assert_eq!(first.state_history[1].state, "running");
        assert!(first.state_history[1].note.contains("northstar"));
        assert!(first.outcome.as_str().unwrap().contains("state re-read"));
        let third: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000003.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(third.state, "unknown");
        let fourth: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000004.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(fourth.state, "failed");
        assert!(!dir.join("opj_000000000001.json.tmp").exists());

        // Re-running the pass is a no-op: nothing is queued any more.
        let second = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &due,
            None,
            None,
            "northstar",
            1_700_000_002_000,
        )
        .await
        .expect("consume again");
        assert_eq!(
            second,
            OperatorJobsReport {
                completed: 0,
                failed: 0,
                unknown: 0,
                skipped: 4
            }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_restore_reconcile_completes_when_the_platform_confirms_the_stop() {
        let dir = std::env::temp_dir().join(format!("apex-opsjobs-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        job_file(
            &dir,
            "opj_00000000000a",
            "restore_reconcile",
            "exp_0247",
            "queued",
        );
        let ops = ops_with(apex_core::ExperimentState::Stopped);
        let report =
            consume_operator_jobs(&dir, ops.as_ref(), &|_| false, None, None, "northstar", 1)
                .await
                .expect("consume");
        assert_eq!(report.completed, 1);
        let job: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_00000000000a.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(job.state, "completed");
        assert!(job.outcome.as_str().unwrap().contains("confirms"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_missing_directory_is_an_empty_pass_and_shared_states_are_skipped() {
        let dir = std::env::temp_dir().join(format!("apex-opsjobs-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ops = RecordingOps::new();
        let report = consume_operator_jobs(&dir, &ops, &|_| false, None, None, "northstar", 1)
            .await
            .expect("missing dir");
        assert_eq!(report, OperatorJobsReport::default());

        std::fs::create_dir_all(&dir).unwrap();
        job_file(
            &dir,
            "opj_00000000000b",
            "retry_read",
            "exp_0247",
            "running",
        );
        job_file(
            &dir,
            "opj_00000000000c",
            "retry_read",
            "exp_0247",
            "completed",
        );
        std::fs::write(dir.join("corrupt.json"), "not json").unwrap();
        std::fs::write(dir.join("ignored.txt"), "not a job").unwrap();
        let report = consume_operator_jobs(&dir, &ops, &|_| false, None, None, "northstar", 1)
            .await
            .expect("consume");
        assert_eq!(report.skipped, 2, "running and completed are left alone");
        assert_eq!(report.failed, 1, "corrupt file is counted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn job_states_and_helpers_are_bounded() {
        for state in [
            JobState::Queued,
            JobState::Running,
            JobState::Completed,
            JobState::Failed,
            JobState::Unknown,
        ] {
            assert_eq!(JobState::parse(state.as_str()), Some(state));
        }
        assert_eq!(JobState::parse("wibble"), None);
        assert!(capability_of("retry_read").is_some());
        assert!(capability_of("other").is_none());
        assert_eq!(DEFAULT_JOBS_DIR, ".apex/control/jobs");
        assert!(is_job_id("opj_000000000001"));
        assert!(!is_job_id("opj_1"));
        assert!(!is_job_id("opj_00000000000Z"));
        assert!(!is_job_id("job_000000000001"));
        let long = "x".repeat(500);
        assert_eq!(bounded(long).len(), 200);
        // A multibyte error at the bound is cut by characters, never bytes
        // (the old `String::truncate` panicked here).
        let multibyte = "é".repeat(500);
        let bounded_multibyte = bounded(multibyte);
        assert_eq!(bounded_multibyte.chars().count(), 200);
        assert_eq!(bounded_multibyte, "é".repeat(200));
    }
}
