//! The operator-jobs consumer: the frozen job-store contract.
//!
//! The control plane writes one JSON file per operator job into
//! `APEX_OPERATOR_JOBS_DIR` (default `.apex/control/jobs`); the runner's
//! daily pass consumes the kinds `retry_read`, `recheck_request`,
//! `restore_reconcile` (the legacy spelling `restore_reconcile_request` is
//! still read), `reconnection_request` and `job_retry`. The file format is
//! frozen:
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
    /// The job kind; the runner consumes `retry_read`, `recheck_request`,
    /// `restore_reconcile`, `reconnection_request` and `job_retry`.
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
    /// The runner pass that currently owns the job while it is `running`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// When the owner's claim expires. An expired running job is reclaimed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at_ms: Option<i64>,
    /// How many times a consumer has claimed the job (stale-running recovery
    /// increments it).
    #[serde(default)]
    pub attempts: u32,
}

/// How long one runner pass owns a claimed job before another pass may
/// recover it as stale.
pub const JOB_LEASE_MS: i64 = 15 * 60 * 1_000;
/// A queued job older than this has expired without a consumer.
pub const JOB_EXPIRY_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

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

/// Consume every queued job in `dir` that `belongs` to this pass (in the
/// legacy single-publisher mode `belongs` accepts everything; the fleet daily
/// pass claims only jobs whose target its own ledger owns, so one publisher
/// never re-reads another publisher's experiment and an orphaned job stays
/// visible as skipped instead of being consumed by the wrong adapter).
///
/// A corrupt file is reported on stderr and counted as failed; it is never
/// rewritten (the control plane's bytes are not ours to guess at). The rest
/// of the directory is still consumed: one bad file must not starve the
/// queue.
#[allow(clippy::too_many_arguments)]
pub async fn consume_operator_jobs(
    dir: &Path,
    ops: &dyn PlatformOps,
    due_rechecks: &dyn Fn(&str) -> bool,
    belongs: &dyn Fn(&OperatorJob) -> bool,
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
        if !matches!(state, JobState::Queued | JobState::Running) {
            // Deduplication: a terminal job is a no-op forever.
            report.skipped += 1;
            continue;
        }
        if !belongs(&job) {
            // Not this pass's job: leave it queued for its owner (or for an
            // operator when no owner exists), and say so in the counts.
            report.skipped += 1;
            continue;
        }
        if state == JobState::Queued
            && job.requested_at_ms.saturating_add(JOB_EXPIRY_MS) < now_unix_ms
        {
            // A queued job no consumer ever claimed has expired. Record the
            // terminal fact instead of leaving it queued forever.
            job.state = JobState::Failed.as_str().to_string();
            job.owner = None;
            job.lease_expires_at_ms = None;
            let note = "the job expired before any runner claimed it".to_string();
            job.state_history.push(StateChange {
                state: JobState::Failed.as_str().to_string(),
                at_ms: now_unix_ms,
                note: note.clone(),
            });
            job.outcome = Value::String(note);
            write_job(&path, &job)?;
            report.failed += 1;
            continue;
        }
        let stale_running = state == JobState::Running;
        if stale_running {
            // A live owner keeps its lease. An expired (or legacy ownerless)
            // running job is recovered instead of being skipped forever.
            let lease_live = job
                .lease_expires_at_ms
                .is_some_and(|expiry| now_unix_ms < expiry);
            if lease_live {
                report.skipped += 1;
                continue;
            }
        }

        job.state = JobState::Running.as_str().to_string();
        job.attempts = job.attempts.saturating_add(1);
        job.owner = Some(format!("{publisher}@{}", std::process::id()));
        job.lease_expires_at_ms = Some(now_unix_ms.saturating_add(JOB_LEASE_MS));
        job.state_history.push(StateChange {
            state: JobState::Running.as_str().to_string(),
            at_ms: now_unix_ms,
            note: if stale_running {
                format!("recovered an expired running job for the {publisher} runner pass")
            } else {
                format!("claimed by the {publisher} runner pass")
            },
        });
        write_job(&path, &job)?;

        let (terminal, note) = if job.kind == "reconnection_request" {
            execute_reconnection(notifier, publisher, &job, now_unix_ms).await
        } else {
            execute(&job, ops, due_rechecks, dir, now_unix_ms).await
        };
        job.state = terminal.as_str().to_string();
        job.owner = None;
        job.lease_expires_at_ms = None;
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
    dir: &Path,
    now_unix_ms: i64,
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
        // Canonical spelling, plus the control plane's older
        // `restore_reconcile_request` files, which must keep working.
        "restore_reconcile" | "restore_reconcile_request" => {
            match ops.experiment(&job.target).await {
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
            }
        }
        "job_retry" => retry_referenced_job(dir, job, now_unix_ms),
        other => (JobState::Failed, format!("unsupported job kind '{other}'")),
    }
}

/// Re-enqueue the durable job named by a `job_retry` job.
fn retry_referenced_job(dir: &Path, job: &OperatorJob, now_unix_ms: i64) -> (JobState, String) {
    let target_id = job
        .target
        .trim()
        .strip_suffix(".json")
        .unwrap_or(job.target.trim());
    if !is_job_id(target_id) {
        return (
            JobState::Failed,
            format!(
                "job_retry target '{}' is not an opj_<12hex> job id",
                job.target
            ),
        );
    }
    if target_id == job.job_id {
        return (
            JobState::Failed,
            "job_retry cannot retry its own job".to_string(),
        );
    }
    let path = dir.join(format!("{target_id}.json"));
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            return (
                JobState::Failed,
                format!("no job record for {target_id}: {error}"),
            )
        }
    };
    let mut referenced: OperatorJob = match serde_json::from_str(&raw) {
        Ok(referenced) => referenced,
        Err(error) => {
            return (
                JobState::Failed,
                format!("job {target_id} is not a readable job file: {error}"),
            )
        }
    };
    let previous = referenced.state.clone();
    let previous_outcome = referenced.outcome.clone();
    match previous.as_str() {
        "queued" => {
            return (
                JobState::Completed,
                format!("job {target_id} is already queued; nothing was re-enqueued"),
            )
        }
        "running" => {
            return (
                JobState::Unknown,
                format!("job {target_id} is running; it was not re-enqueued"),
            )
        }
        _ => {}
    }
    referenced.state = JobState::Queued.as_str().to_string();
    referenced.state_history.push(StateChange {
        state: JobState::Queued.as_str().to_string(),
        at_ms: now_unix_ms,
        note: format!("re-enqueued by operator job {}", job.job_id),
    });
    referenced.outcome = Value::Null;
    if let Err(error) = write_job(&path, &referenced) {
        return (
            JobState::Failed,
            format!("job {target_id} could not be re-enqueued: {error}"),
        );
    }
    (
        JobState::Completed,
        format!(
            "re-enqueued {target_id} (previous state {previous}, previous outcome {previous_outcome})"
        ),
    )
}

/// A reconnection request is a customer notification.
async fn execute_reconnection(
    notifier: Option<&NotifyPlan>,
    publisher: &str,
    job: &OperatorJob,
    now_unix_ms: i64,
) -> (JobState, String) {
    let Some(notifier) = notifier else {
        return (
            JobState::Failed,
            "no notification channel is configured for reconnection requests".to_string(),
        );
    };
    let platform = job
        .payload
        .get("platform")
        .and_then(Value::as_str)
        .map(platform_label)
        .unwrap_or("the account");
    let access = access_label(&job.payload);
    let day = crate::calendar::format_day(now_unix_ms.div_euclid(86_400_000));
    let publisher = if publisher.is_empty() {
        job.target.as_str()
    } else {
        publisher
    };
    let report = notifier
        .reconnection_requested(publisher, platform, &access, &day, now_unix_ms)
        .await;
    if report.attempted == 0 && report.duplicates == 0 {
        return (
            JobState::Unknown,
            format!(
                "reconnection for {platform} was recorded but no publisher contact is configured"
            ),
        );
    }
    if report.failed > 0 && report.accepted == 0 && report.delivered == 0 {
        return (
            JobState::Failed,
            format!(
                "reconnection notification for {platform} failed ({} send(s))",
                report.failed
            ),
        );
    }
    (
        JobState::Completed,
        format!(
            "reconnection notification for {platform} recorded (attempted {}, delivered {}, duplicates {})",
            report.attempted, report.delivered, report.duplicates
        ),
    )
}

fn platform_label(wire: &str) -> &'static str {
    match wire.trim().to_ascii_lowercase().as_str() {
        "applovin_max" | "max" => "AppLovin MAX",
        "levelplay" => "LevelPlay",
        "admob" => "AdMob",
        _ => "the account",
    }
}

/// The access the operator asked to restore. The payload is operator
/// supplied, so only a short, plain label is rendered; anything else falls
/// back to the generic reporting access rather than putting arbitrary text
/// (or control characters) into a mail header.
fn access_label(payload: &Value) -> String {
    payload
        .get("access")
        .or_else(|| payload.get("access_label"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value.chars().all(|c| {
                    c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '-' || c == '/'
                })
        })
        .map(str::to_string)
        .unwrap_or_else(|| "reporting access".to_string())
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
        "restore_reconcile" | "restore_reconcile_request" => {
            Some((CapabilityClass::Restore, ActionClass::Reconcile))
        }
        "reconnection_request" => {
            Some((CapabilityClass::Ingestion, ActionClass::RequestReconnection))
        }
        "job_retry" => Some((CapabilityClass::Jobs, ActionClass::JobRetry)),
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
    {
        let mut file = std::fs::File::create(&tmp).map_err(|error| OperatorJobError::Io {
            path: tmp.display().to_string(),
            reason: error.to_string(),
        })?;
        use std::io::Write as _;
        file.write_all(json.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| OperatorJobError::Io {
                path: tmp.display().to_string(),
                reason: error.to_string(),
            })?;
    }
    std::fs::rename(&tmp, path).map_err(|error| OperatorJobError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    // A renamed job file is only durable with its directory entry behind it:
    // a losing write must survive a power loss or the queue loses the job.
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        let parent = if parent.as_os_str().is_empty() {
            std::path::Path::new(".")
        } else {
            parent
        };
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| OperatorJobError::Io {
                path: path.display().to_string(),
                reason: format!("directory sync failed: {error}"),
            })?;
    }
    Ok(())
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
            owner: None,
            lease_expires_at_ms: None,
            attempts: 0,
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
            &|_| true,
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
            &|_| true,
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
    async fn a_queued_job_past_its_expiry_fails_instead_of_waiting_forever() {
        let dir = std::env::temp_dir().join(format!(
            "apex-opsjobs-expiry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = job_file(&dir, "opj_0000000000bb", "retry_read", "exp_0247", "queued");
        let mut expired: OperatorJob =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        expired.requested_at_ms = 1_700_000_001_000 - JOB_EXPIRY_MS - 1;
        write_job(&path, &expired).expect("write expired");

        let ops = ops_with(apex_core::ExperimentState::Running);
        let report = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &|_| false,
            &|_| true,
            None,
            None,
            "northstar",
            1_700_000_001_000,
        )
        .await
        .expect("consume");
        assert_eq!(report.failed, 1);
        let failed: OperatorJob =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(failed.state, "failed");
        assert!(failed.outcome.as_str().unwrap().contains("expired"));
        assert!(
            ops.call_log().is_empty(),
            "an expired queued job is never executed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_expired_running_job_is_reclaimed_and_finished() {
        let dir = std::env::temp_dir().join(format!(
            "apex-opsjobs-stale-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = job_file(&dir, "opj_0000000000aa", "retry_read", "exp_0247", "queued");
        let mut stale: OperatorJob =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        stale.state = "running".to_string();
        stale.owner = Some("crashed-runner@999".to_string());
        stale.lease_expires_at_ms = Some(500);
        stale.attempts = 1;
        stale.state_history.push(StateChange {
            state: "running".to_string(),
            at_ms: 100,
            note: "claimed by the crashed runner".to_string(),
        });
        write_job(&path, &stale).expect("write stale");

        let ops = ops_with(apex_core::ExperimentState::Running);
        let report = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &|_| false,
            &|_| true,
            None,
            None,
            "northstar",
            1_700_000_001_000,
        )
        .await
        .expect("consume");
        assert_eq!(report.completed, 1, "the stale job is recovered and run");
        let finished: OperatorJob =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(finished.state, "completed");
        assert_eq!(finished.attempts, 2);
        assert!(finished.owner.is_none(), "the claim is released");
        assert!(finished.lease_expires_at_ms.is_none());
        assert!(
            finished
                .state_history
                .iter()
                .any(|entry| entry.note.contains("recovered an expired running job")),
            "{:?}",
            finished.state_history
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn jobs_that_do_not_belong_are_left_queued_for_their_owner() {
        let dir = std::env::temp_dir().join(format!("apex-opsjobs-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        job_file(&dir, "opj_000000000011", "retry_read", "exp_0247", "queued");
        job_file(
            &dir,
            "opj_000000000012",
            "retry_read",
            "exp_other",
            "queued",
        );
        let ops = ops_with(apex_core::ExperimentState::Running);
        let belongs = |job: &OperatorJob| job.target == "exp_0247";
        let report = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &|_| false,
            &belongs,
            None,
            None,
            "northstar",
            1,
        )
        .await
        .expect("consume");
        assert_eq!(report.completed, 1);
        assert_eq!(report.skipped, 1);
        let mine: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000011.json")).unwrap(),
        )
        .unwrap();
        let other: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000012.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(mine.state, "completed");
        assert_eq!(other.state, "queued", "another owner's job is untouched");
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
        let report = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &|_| false,
            &|_| true,
            None,
            None,
            "northstar",
            1,
        )
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
    async fn job_retry_re_enqueues_the_referenced_terminal_job() {
        let dir = std::env::temp_dir().join(format!("apex-opsjobs-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        job_file(&dir, "opj_000000000020", "retry_read", "exp_0247", "failed");
        job_file(
            &dir,
            "opj_000000000021",
            "job_retry",
            "opj_000000000020",
            "queued",
        );
        job_file(
            &dir,
            "opj_000000000022",
            "job_retry",
            "opj_0000000000ff",
            "queued",
        );
        let ops = ops_with(apex_core::ExperimentState::Running);
        let report = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &|_| false,
            &|_| true,
            None,
            None,
            "northstar",
            1_700_000_001_000,
        )
        .await
        .expect("consume");
        assert_eq!(report.completed, 1, "the valid retry completed");
        assert_eq!(report.failed, 1, "the missing target failed honestly");
        assert_eq!(report.skipped, 1, "the terminal target was skipped first");

        let referenced: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000020.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            referenced.state, "queued",
            "the referenced job is re-enqueued"
        );
        assert!(referenced
            .state_history
            .last()
            .unwrap()
            .note
            .contains("re-enqueued by operator job opj_000000000021"));
        let retry: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000021.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(retry.state, "completed");
        assert!(retry
            .outcome
            .as_str()
            .unwrap()
            .contains("re-enqueued opj_000000000020"));
        let missing: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000022.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(missing.state, "failed");
        assert!(missing.outcome.as_str().unwrap().contains("no job record"));

        let second = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &|_| false,
            &|_| true,
            None,
            None,
            "northstar",
            1_700_000_002_000,
        )
        .await
        .expect("consume again");
        assert_eq!(second.completed, 1);
        assert_eq!(
            serde_json::from_str::<OperatorJob>(
                &std::fs::read_to_string(dir.join("opj_000000000020.json")).unwrap()
            )
            .unwrap()
            .state,
            "completed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_reconnection_request_notifies_the_publisher_or_reports_the_missing_channel() {
        let dir =
            std::env::temp_dir().join(format!("apex-opsjobs-reconnect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        job_file(
            &dir,
            "opj_000000000031",
            "reconnection_request",
            "northstar",
            "queued",
        );
        let ops = ops_with(apex_core::ExperimentState::Running);
        let report = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &|_| false,
            &|_| true,
            None,
            None,
            "northstar",
            1_700_000_003_000,
        )
        .await
        .expect("consume");
        assert_eq!(report.failed, 1);
        let failed: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000031.json")).unwrap(),
        )
        .unwrap();
        assert!(failed
            .outcome
            .as_str()
            .unwrap()
            .contains("no notification channel"));

        job_file(
            &dir,
            "opj_000000000032",
            "reconnection_request",
            "northstar",
            "queued",
        );
        // The control plane names the platform and the access to restore.
        let job_path = dir.join("opj_000000000032.json");
        let mut requested: OperatorJob =
            serde_json::from_str(&std::fs::read_to_string(&job_path).unwrap()).unwrap();
        requested.payload = serde_json::json!({
            "platform": "applovin_max",
            "access": "reporting access"
        });
        write_job(&job_path, &requested).unwrap();
        let outbox = dir.join("outbox.jsonl");
        let state = dir.join("notify-state.jsonl");
        let (plan, _logs) = NotifyPlan::from_lookup(
            |name| match name {
                "APEX_NOTIFY_CONTACTS" => Some(r#"{"northstar":["ops@example.com"]}"#.to_string()),
                "APEX_CONSOLE_URL" => Some("https://console.example".to_string()),
                "APEX_NOTIFY_SINK" => Some("file".to_string()),
                "APEX_NOTIFY_FILE" => Some(outbox.display().to_string()),
                "APEX_NOTIFY_STATE" => Some(state.display().to_string()),
                _ => None,
            },
            std::sync::Arc::new(apex_metrics::Registry::new()),
        )
        .expect("notify plan");
        let plan = plan.expect("configured plan");
        let report = consume_operator_jobs(
            &dir,
            ops.as_ref(),
            &|_| false,
            &|_| true,
            Some(plan.as_ref()),
            None,
            "northstar",
            1_700_000_004_000,
        )
        .await
        .expect("consume");
        assert_eq!(report.completed, 1);
        let completed: OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000032.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(completed.state, "completed");
        assert!(completed
            .outcome
            .as_str()
            .unwrap()
            .contains("reconnection notification"));
        assert!(outbox.exists(), "the file sink received the message");
        let delivered = std::fs::read_to_string(&outbox).unwrap();
        assert_eq!(
            delivered.lines().count(),
            1,
            "the new template is dispatched exactly once"
        );
        let row: Value = serde_json::from_str(delivered.lines().next().unwrap()).unwrap();
        assert_eq!(row["template_class"], "reporting_access");
        assert!(
            row["dedup_key"]
                .as_str()
                .unwrap()
                .starts_with("reconnection_requested:"),
            "{}",
            row["dedup_key"]
        );
        assert_eq!(row["subject"], "Restore AppLovin MAX access for Northstar");
        assert!(row["text"]
            .as_str()
            .unwrap()
            .contains("restore reporting access for Northstar on AppLovin MAX"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_missing_directory_is_an_empty_pass_and_shared_states_are_skipped() {
        let dir = std::env::temp_dir().join(format!("apex-opsjobs-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ops = RecordingOps::new();
        let report = consume_operator_jobs(
            &dir,
            &ops,
            &|_| false,
            &|_| true,
            None,
            None,
            "northstar",
            1,
        )
        .await
        .expect("missing dir");
        assert_eq!(report, OperatorJobsReport::default());

        std::fs::create_dir_all(&dir).unwrap();
        let running = job_file(
            &dir,
            "opj_00000000000b",
            "retry_read",
            "exp_0247",
            "running",
        );
        let mut live: OperatorJob =
            serde_json::from_str(&std::fs::read_to_string(&running).unwrap()).unwrap();
        live.owner = Some("another-live-runner".to_string());
        live.lease_expires_at_ms = Some(60_000);
        write_job(&running, &live).expect("write live lease");
        job_file(
            &dir,
            "opj_00000000000c",
            "retry_read",
            "exp_0247",
            "completed",
        );
        std::fs::write(dir.join("corrupt.json"), "not json").unwrap();
        std::fs::write(dir.join("ignored.txt"), "not a job").unwrap();
        let report = consume_operator_jobs(
            &dir,
            &ops,
            &|_| false,
            &|_| true,
            None,
            None,
            "northstar",
            1,
        )
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
