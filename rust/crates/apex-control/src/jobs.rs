//! Durable operator jobs in the frozen format the autopilot workstream
//! consumes.
//!
//! The control plane never performs a platform operation itself. `retry_read`,
//! `recheck_request`, `restore_reconcile`, `reconnection_request` and
//! `job_retry` write one job file into
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
//!
//! Two transports share one interface ([`JobTransport`]):
//!
//! - [`JobTransport::Local`] is the original single-host mode: the write and
//!   the read go straight to `APEX_OPERATOR_JOBS_DIR`.
//! - [`JobTransport::Remote`] is the split-host mode: with `APEX_API_JOBS_URL`
//!   and `APEX_OPERATOR_JOBS_TOKEN` set, the control plane sends the job to
//!   apex-api (`POST /v1/operator/jobs`) instead of stat-ing a folder it no
//!   longer shares, and reads a job's current state back through
//!   `GET /v1/operator/jobs/{job_id}`. The bytes are the same pretty-printed
//!   frozen JSON the local writer produces, and the endpoint persists them
//!   verbatim, so the runner-host folder sees byte-identical files in both
//!   modes. The endpoint deduplicates by `job_id` (a duplicate POST returns
//!   the existing job), which is the remote counterpart of the local
//!   `idempotency_key` scan for the ordinary replay flow.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::data::FileStamp;
use crate::signing::random_hex_32;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

/// Environment variable naming the apex-api jobs endpoint base URL; together
/// with [`JOBS_TOKEN_ENV`] it selects the split-host mode.
pub const JOBS_URL_ENV: &str = "APEX_API_JOBS_URL";
/// Environment variable naming the bearer token for the jobs endpoint; the
/// same token apex-api's receiver reads.
pub const JOBS_TOKEN_ENV: &str = "APEX_OPERATOR_JOBS_TOKEN";
/// The frozen endpoint path appended to the configured base URL.
pub const JOBS_ENDPOINT_PATH: &str = "/v1/operator/jobs";
/// Request timeout of the remote jobs client.
pub const REMOTE_JOBS_TIMEOUT_MS: u64 = 10_000;

/// The job kinds the control plane writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Retry one safe platform read.
    RetryRead,
    /// Request a month-6 or triggered re-check.
    RecheckRequest,
    /// Ask the runner to reconcile a restore state. The wire name is the
    /// runner's canonical `restore_reconcile`; the legacy
    /// `restore_reconcile_request` spelling is still accepted on read.
    #[serde(rename = "restore_reconcile", alias = "restore_reconcile_request")]
    RestoreReconcileRequest,
    /// Ask the runner to notify the publisher to reconnect credentials.
    ReconnectionRequest,
    /// Retry a durable runner job.
    JobRetry,
}

impl JobKind {
    /// Stable name used inside the file.
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::RetryRead => "retry_read",
            JobKind::RecheckRequest => "recheck_request",
            JobKind::RestoreReconcileRequest => "restore_reconcile",
            JobKind::ReconnectionRequest => "reconnection_request",
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
    /// The remote jobs endpoint could not be used.
    #[error("remote jobs endpoint {url}: {reason}")]
    Remote {
        /// Endpoint or request URL involved.
        url: String,
        /// Underlying reason (transport, HTTP status, malformed answer).
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
///
/// Every read path folds the directory through one parse cache: a file whose
/// identity (length, mtime and on Unix ctime and inode) is unchanged reuses
/// its previously parsed row. Writes never list the directory: an
/// [`Self::key_index`] answers the idempotency lookup, updated in place after
/// each local write and refreshed from disk only when the directory's own
/// identity (mtime, ctime and inode) changed — that is, when an external
/// writer added or replaced a file or the store starts cold. A rewrite of an
/// indexed job file is picked up by stat-ing that one file before serving it,
/// so an external state transition is never masked by the index.
#[derive(Debug, Clone)]
pub struct JobStore {
    dir: PathBuf,
    write_lock: Arc<Mutex<()>>,
    cache: Arc<Mutex<HashMap<PathBuf, CachedJobRow>>>,
    key_index: Arc<Mutex<KeyIndex>>,
    #[cfg(test)]
    parses: Arc<AtomicU64>,
    #[cfg(test)]
    scans: Arc<AtomicU64>,
}

/// One parsed job file kept while its file identity is unchanged.
#[derive(Debug, Clone)]
struct CachedJobRow {
    stamp: FileStamp,
    row: JobRead,
}

/// One idempotency key as last indexed: the file that supplied it, that
/// file's identity when indexed, and the parsed job.
#[derive(Debug, Clone)]
struct IndexedJob {
    path: PathBuf,
    stamp: Option<FileStamp>,
    job: OperatorJob,
}

/// The in-memory idempotency index. It is rebuilt from the directory only on
/// cold start or when the directory identity changes; local writes update it
/// in place, so N writes cost one cold scan and no rescans.
#[derive(Debug, Default)]
struct KeyIndex {
    initialized: bool,
    dir_stamp: Option<FileStamp>,
    by_key: HashMap<String, IndexedJob>,
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl JobStore {
    /// A store rooted at `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            write_lock: Arc::new(Mutex::new(())),
            cache: Arc::new(Mutex::new(HashMap::new())),
            key_index: Arc::new(Mutex::new(KeyIndex::default())),
            #[cfg(test)]
            parses: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            scans: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The number of job file parses this store performed (test seam proving
    /// the cache is hit; a fresh store counts from zero).
    #[cfg(test)]
    pub(crate) fn parse_count(&self) -> u64 {
        self.parses.load(Ordering::SeqCst)
    }

    /// The number of directory scans this store performed (test seam proving
    /// writes do not re-list the folder; a fresh store counts from zero).
    #[cfg(test)]
    pub(crate) fn scan_count(&self) -> u64 {
        self.scans.load(Ordering::SeqCst)
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
        self.record_written(job, &final_path);
        Ok(JobWrite::Created(job.clone()))
    }

    /// The first durable job written under `idempotency_key`, when any.
    ///
    /// The key index answers the lookup. It is refreshed from disk only on
    /// cold start or when the directory identity changed (an external writer
    /// added or renamed a file); the one indexed file is stat-ed before use,
    /// so an external rewrite of that job's state is reflected too.
    fn existing_for_key(&self, idempotency_key: &str) -> Result<Option<OperatorJob>, JobError> {
        let mut index = lock_or_recover(&self.key_index);
        let dir_stamp = self.dir_stamp();
        if !index.initialized || index.dir_stamp != dir_stamp {
            self.refresh_key_index(&mut index)?;
        }
        let Some(entry) = index.by_key.get(idempotency_key).cloned() else {
            return Ok(None);
        };
        let stamp = self.file_stamp(&entry.path);
        if stamp == entry.stamp {
            return Ok(Some(entry.job));
        }
        // The indexed file moved (a runner state rewrite, an external
        // replacement, or a removal): re-read that one file instead of
        // scanning the directory.
        let name = entry
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        match self.row_for(&entry.path, &name, stamp) {
            JobRead::Parsed(job) => {
                index.by_key.insert(
                    idempotency_key.to_string(),
                    IndexedJob {
                        path: entry.path,
                        stamp,
                        job: (*job).clone(),
                    },
                );
                Ok(Some(*job))
            }
            JobRead::StateUnknown { .. } => {
                // Gone or malformed: the old directory scan ignored both, so
                // the key is no longer claimed.
                index.by_key.remove(idempotency_key);
                Ok(None)
            }
        }
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
    /// a file that cannot be parsed appears as `State unknown`. Unchanged
    /// files are served from the parse cache. A listing also refreshes the
    /// key index from the files it discovered.
    pub fn list(&self) -> Result<Vec<JobRead>, JobError> {
        // Serialize with writes so the index rebuilt from this snapshot is
        // never overwritten by a write that raced the scan.
        let _guard = lock_or_recover(&self.write_lock);
        let dir_stamp = self.dir_stamp();
        let scanned = self.scan_directory()?;
        self.apply_key_index(dir_stamp, &scanned);
        Ok(scanned.into_iter().map(|(_, _, row)| row).collect())
    }

    /// The directory's own identity (mtime, ctime and inode on Unix).
    fn dir_stamp(&self) -> Option<FileStamp> {
        std::fs::metadata(&self.dir)
            .ok()
            .map(|metadata| FileStamp::of(&metadata))
    }

    /// One file's identity.
    fn file_stamp(&self, path: &Path) -> Option<FileStamp> {
        std::fs::metadata(path)
            .ok()
            .map(|metadata| FileStamp::of(&metadata))
    }

    /// One parsed row, from the cache when the file identity is unchanged.
    fn row_for(&self, path: &Path, name: &str, stamp: Option<FileStamp>) -> JobRead {
        let cached = stamp.and_then(|stamp| {
            self.cache.lock().ok().and_then(|cache| {
                cache
                    .get(path)
                    .filter(|cached| cached.stamp == stamp)
                    .map(|cached| cached.row.clone())
            })
        });
        if let Some(row) = cached {
            return row;
        }
        #[cfg(test)]
        self.parses.fetch_add(1, Ordering::SeqCst);
        let row = match std::fs::read_to_string(path) {
            Ok(raw) => parse_job(name, &raw),
            Err(error) => JobRead::StateUnknown {
                file: name.to_string(),
                reason: error.to_string(),
            },
        };
        if let Some(stamp) = stamp {
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert(
                    path.to_path_buf(),
                    CachedJobRow {
                        stamp,
                        row: row.clone(),
                    },
                );
            }
        }
        row
    }

    /// Every `*.json` job file with its identity, sorted by job id. Temporary
    /// files are ignored.
    fn scan_directory(&self) -> Result<Vec<(PathBuf, Option<FileStamp>, JobRead)>, JobError> {
        #[cfg(test)]
        self.scans.fetch_add(1, Ordering::SeqCst);
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
            let path = entry.path();
            // A stat that fails mid-race falls through to the read, which
            // reports the file as `State unknown` exactly like before.
            let stamp = self.file_stamp(&path);
            let row = self.row_for(&path, &name, stamp);
            rows.push((path, stamp, row));
        }
        rows.sort_by(|left, right| left.2.sort_key().cmp(right.2.sort_key()));
        Ok(rows)
    }

    /// Rebuild the key index from a directory scan.
    fn refresh_key_index(&self, index: &mut KeyIndex) -> Result<(), JobError> {
        // Capture the stamp before the scan: a change during the scan then
        // shows up as a difference on the next lookup and triggers another
        // refresh instead of being masked.
        let dir_stamp = self.dir_stamp();
        let scanned = self.scan_directory()?;
        *index = KeyIndex {
            initialized: true,
            dir_stamp,
            by_key: key_map(&scanned),
        };
        Ok(())
    }

    /// Replace the key index with one built from a scan already performed by
    /// [`Self::list`], whose row order is the same stable order.
    fn apply_key_index(
        &self,
        dir_stamp: Option<FileStamp>,
        scanned: &[(PathBuf, Option<FileStamp>, JobRead)],
    ) {
        let mut index = lock_or_recover(&self.key_index);
        *index = KeyIndex {
            initialized: true,
            dir_stamp,
            by_key: key_map(scanned),
        };
    }

    /// Record a job this store just wrote, so the next write needs no scan.
    fn record_written(&self, job: &OperatorJob, path: &Path) {
        let stamp = self.file_stamp(path);
        let dir_stamp = self.dir_stamp();
        let mut index = lock_or_recover(&self.key_index);
        index.initialized = true;
        index.by_key.insert(
            job.idempotency_key.clone(),
            IndexedJob {
                path: path.to_path_buf(),
                stamp,
                job: job.clone(),
            },
        );
        index.dir_stamp = dir_stamp;
    }
}

/// The first parsed job per key, in scan order (callers pass rows sorted by
/// job id, so a duplicated key resolves exactly like the old sorted scan).
fn key_map(scanned: &[(PathBuf, Option<FileStamp>, JobRead)]) -> HashMap<String, IndexedJob> {
    let mut by_key = HashMap::with_capacity(scanned.len());
    for (path, stamp, row) in scanned {
        if let JobRead::Parsed(job) = row {
            by_key
                .entry(job.idempotency_key.clone())
                .or_insert_with(|| IndexedJob {
                    path: path.clone(),
                    stamp: *stamp,
                    job: (**job).clone(),
                });
        }
    }
    by_key
}

/// The split-host endpoint configuration (`APEX_API_JOBS_URL` plus
/// `APEX_OPERATOR_JOBS_TOKEN`); the token is redacted in `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteJobsConfig {
    /// Base URL of the apex-api deployment (or the full jobs endpoint URL).
    pub url: String,
    /// Bearer token presented on every request.
    pub token: String,
}

impl std::fmt::Debug for RemoteJobsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteJobsConfig")
            .field("url", &self.url)
            .field("token", &"[redacted]")
            .finish()
    }
}

/// The remote jobs client: the frozen endpoint over the crate's `reqwest`
/// conventions (rustls, certificate verification on by default).
#[derive(Clone)]
pub struct RemoteJobs {
    client: reqwest::Client,
    endpoint: String,
    token: String,
}

impl std::fmt::Debug for RemoteJobs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteJobs")
            .field("endpoint", &self.endpoint)
            .field("token", &"[redacted]")
            .finish()
    }
}

impl RemoteJobs {
    /// Build the client; the URL is normalized once (a base URL gets the
    /// frozen path appended, a URL that already names the endpoint is used
    /// as-is).
    pub fn new(config: &RemoteJobsConfig) -> Result<Self, JobError> {
        let endpoint = normalize_jobs_endpoint(&config.url).map_err(|reason| JobError::Remote {
            url: config.url.clone(),
            reason,
        })?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(REMOTE_JOBS_TIMEOUT_MS))
            .build()
            .map_err(|error| JobError::Remote {
                url: endpoint.clone(),
                reason: format!("the HTTP client could not be built: {error}"),
            })?;
        Ok(Self {
            client,
            endpoint,
            token: config.token.clone(),
        })
    }

    /// The absolute endpoint URL.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The governance endpoint on the same service: the jobs URL's sibling.
    fn governance_endpoint(&self) -> Result<String, JobError> {
        self.endpoint
            .strip_suffix("/v1/operator/jobs")
            .map(|base| format!("{base}/v1/operator/governance"))
            .ok_or_else(|| JobError::Remote {
                url: self.endpoint.clone(),
                reason: "the jobs URL does not end with /v1/operator/jobs, so the governance sibling cannot be derived".to_string(),
            })
    }

    /// Deliver one governance mutation to the console host. The runner and
    /// the console read those files there, so in split-host mode this call is
    /// what makes a pause or a policy change take effect.
    pub async fn write_governance(&self, body: &serde_json::Value) -> Result<(), JobError> {
        let url = self.governance_endpoint()?;
        let payload = serde_json::to_string(body).map_err(|error| JobError::Remote {
            url: url.clone(),
            reason: format!("serializing the governance request failed: {error}"),
        })?;
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload)
            .send()
            .await
            .map_err(|error| self.transport_error(&error))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(JobError::Remote {
                url,
                reason: format!(
                    "the governance endpoint answered HTTP {} ({})",
                    status.as_u16(),
                    bounded_reason(&text)
                ),
            });
        }
        Ok(())
    }

    /// POST the job's exact pretty-printed bytes. The endpoint answers the
    /// durable record; identical bytes echo back (`Created`), a duplicate
    /// `job_id` answers the existing job (`Duplicate`), so the caller sees
    /// the same [`JobWrite`] shapes as the local writer.
    pub async fn write(&self, job: &OperatorJob) -> Result<JobWrite, JobError> {
        let body = serde_json::to_string_pretty(job).map_err(|error| JobError::Remote {
            url: self.endpoint.clone(),
            reason: format!("serializing the job failed: {error}"),
        })?;
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| self.transport_error(&error))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| self.transport_error(&error))?;
        if !status.is_success() {
            return Err(JobError::Remote {
                url: self.endpoint.clone(),
                reason: format!(
                    "the endpoint answered HTTP {} ({})",
                    status.as_u16(),
                    bounded_reason(&text)
                ),
            });
        }
        let stored: OperatorJob =
            serde_json::from_str(&text).map_err(|error| JobError::Remote {
                url: self.endpoint.clone(),
                reason: format!("the endpoint answered a job that did not parse: {error}"),
            })?;
        if stored == *job {
            Ok(JobWrite::Created(stored))
        } else {
            Ok(JobWrite::Duplicate(stored))
        }
    }

    /// GET one job's current bytes; `404` is the endpoint's honest absence
    /// (the local reader's missing file), a malformed body is the local
    /// reader's `State unknown`.
    pub async fn read(&self, job_id: &str) -> Result<JobRead, JobError> {
        let url = format!("{}/{job_id}", self.endpoint);
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|error| self.transport_error(&error))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(JobError::Remote {
                url,
                reason: "the endpoint has no record of the job".to_string(),
            });
        }
        let text = response
            .text()
            .await
            .map_err(|error| self.transport_error(&error))?;
        if !status.is_success() {
            return Err(JobError::Remote {
                url,
                reason: format!(
                    "the endpoint answered HTTP {} ({})",
                    status.as_u16(),
                    bounded_reason(&text)
                ),
            });
        }
        match serde_json::from_str::<OperatorJob>(&text) {
            Ok(job) => {
                if job.state_history.is_empty() {
                    Ok(JobRead::StateUnknown {
                        file: format!("{job_id}.json"),
                        reason: "the job carries no state history".to_string(),
                    })
                } else {
                    Ok(JobRead::Parsed(Box::new(job)))
                }
            }
            Err(error) => Ok(JobRead::StateUnknown {
                file: format!("{job_id}.json"),
                reason: format!("the job file did not parse: {error}"),
            }),
        }
    }

    /// The endpoint deliberately lists nothing: one job is read by id.
    fn list_unsupported(&self) -> JobError {
        JobError::Remote {
            url: self.endpoint.clone(),
            reason: "the endpoint lists nothing; read one job by id".to_string(),
        }
    }

    fn transport_error(&self, error: &reqwest::Error) -> JobError {
        JobError::Remote {
            url: self.endpoint.clone(),
            reason: error.to_string(),
        }
    }
}

/// The normalized endpoint: the frozen path is appended when the configured
/// URL is the deployment base, kept when it already names the endpoint.
fn normalize_jobs_endpoint(url: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|error| format!("not a URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("must use http or https".to_string());
    }
    let trimmed = parsed.as_str().trim_end_matches('/');
    if trimmed.ends_with(JOBS_ENDPOINT_PATH) {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{trimmed}{JOBS_ENDPOINT_PATH}"))
    }
}

/// One bounded line of a refusal body, so an error string cannot grow with a
/// server response.
fn bounded_reason(text: &str) -> String {
    text.trim().chars().take(120).collect()
}

/// The operator-jobs transport: the local folder (single-host mode) or the
/// apex-api endpoint (split-host mode). Both implements share the frozen
/// [`OperatorJob`] bytes and the caller-visible [`JobWrite`]/[`JobRead`]
/// shapes.
#[derive(Debug, Clone)]
pub enum JobTransport {
    /// Files in `APEX_OPERATOR_JOBS_DIR`, exactly as before the split.
    Local(JobStore),
    /// The same folder through `POST /v1/operator/jobs`.
    Remote(RemoteJobs),
}

impl JobTransport {
    /// The single-host transport over `dir`.
    pub fn local(dir: impl Into<PathBuf>) -> Self {
        Self::Local(JobStore::new(dir))
    }

    /// The split-host transport from the environment pair.
    pub fn remote(config: &RemoteJobsConfig) -> Result<Self, JobError> {
        Ok(Self::Remote(RemoteJobs::new(config)?))
    }

    /// Write one job (local atomic file, or remote POST).
    pub async fn write(&self, job: &OperatorJob) -> Result<JobWrite, JobError> {
        match self {
            Self::Local(store) => store.write(job),
            Self::Remote(remote) => remote.write(job).await,
        }
    }

    /// Read one job back (local file, or remote GET).
    pub async fn read(&self, job_id: &str) -> Result<JobRead, JobError> {
        match self {
            Self::Local(store) => store.read(job_id),
            Self::Remote(remote) => remote.read(job_id).await,
        }
    }

    /// List the local folder; the remote endpoint answers a typed
    /// unsupported error (there is no listing across the split).
    pub async fn list(&self) -> Result<Vec<JobRead>, JobError> {
        match self {
            Self::Local(store) => store.list(),
            Self::Remote(remote) => Err(remote.list_unsupported()),
        }
    }

    /// The local folder, when this transport is the single-host one.
    pub fn dir(&self) -> Option<&Path> {
        match self {
            Self::Local(store) => Some(store.dir()),
            Self::Remote(_) => None,
        }
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
    fn the_wire_kinds_are_the_ones_the_runner_reads() {
        let expected = [
            (JobKind::RetryRead, "retry_read"),
            (JobKind::RecheckRequest, "recheck_request"),
            (JobKind::RestoreReconcileRequest, "restore_reconcile"),
            (JobKind::ReconnectionRequest, "reconnection_request"),
            (JobKind::JobRetry, "job_retry"),
        ];
        for (kind, wire) in expected {
            assert_eq!(kind.as_str(), wire);
            let json = serde_json::to_string(&kind).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""));
            assert_eq!(
                serde_json::from_str::<JobKind>(&json).expect("deserialize"),
                kind
            );
        }
        assert_eq!(
            serde_json::from_str::<JobKind>("\"restore_reconcile_request\"").expect("legacy"),
            JobKind::RestoreReconcileRequest
        );
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

    #[test]
    fn the_remote_endpoint_normalizes_base_and_full_urls() {
        assert_eq!(
            normalize_jobs_endpoint("https://api.example.test").expect("base"),
            "https://api.example.test/v1/operator/jobs"
        );
        assert_eq!(
            normalize_jobs_endpoint("https://api.example.test/").expect("trailing slash"),
            "https://api.example.test/v1/operator/jobs"
        );
        assert_eq!(
            normalize_jobs_endpoint("http://127.0.0.1:8790").expect("loopback"),
            "http://127.0.0.1:8790/v1/operator/jobs"
        );
        // A URL that already names the endpoint is kept as-is.
        assert_eq!(
            normalize_jobs_endpoint("https://api.example.test/v1/operator/jobs")
                .expect("full endpoint"),
            "https://api.example.test/v1/operator/jobs"
        );
        assert!(normalize_jobs_endpoint("not a url").is_err());
        assert!(normalize_jobs_endpoint("ftp://api.example.test").is_err());
    }

    #[test]
    fn the_remote_transport_redacts_the_token_and_is_honest_about_listing() {
        let config = RemoteJobsConfig {
            url: "http://127.0.0.1:9".to_string(),
            token: "jobs-token-secret".to_string(),
        };
        assert!(!format!("{config:?}").contains("jobs-token-secret"));
        let transport = JobTransport::remote(&config).expect("remote transport");
        assert!(transport.dir().is_none());
        let rendered = format!("{transport:?}");
        assert!(rendered.contains("/v1/operator/jobs"), "{rendered}");
        assert!(!rendered.contains("jobs-token-secret"), "{rendered}");
        let local = JobTransport::local(std::env::temp_dir().join("apex-control-jobs-local"));
        assert_eq!(
            local.dir(),
            Some(std::path::Path::new(
                &std::env::temp_dir().join("apex-control-jobs-local")
            ))
        );
        // The remote endpoint lists nothing; the typed error says to read by
        // id instead of silently answering an empty list.
        let error = block_on(transport.list()).expect_err("no listing");
        assert!(error.to_string().contains("read one job by id"), "{error}");
    }

    /// A tiny blocking executor for the one list() assertion above; the same
    /// shape the test runtime would provide, without pulling a runtime into
    /// the unit test module.
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }
    /// Regression: the idempotency scan used to re-read and re-parse every
    /// job file on every write (quadratic over a growing directory). The
    /// per-store parse counter proves the cache is used; this test fails on
    /// the pre-fix implementation where each call parses every file again.
    #[test]
    fn unchanged_job_files_are_parsed_once_and_a_changed_file_reparses() {
        let (store, dir) = store();
        for i in 0..12 {
            let job = OperatorJob::queued(
                JobKind::RetryRead,
                "maya",
                1_792_000_000_000 + i,
                &format!("acct_{i}"),
                serde_json::json!({"row": i}),
                &format!("key_{i:02}"),
                &format!("corr_{i}"),
            );
            assert!(store.write(&job).expect("write").created());
        }
        let first = store.list().expect("first listing");
        assert_eq!(first.len(), 12);
        assert_eq!(store.parse_count(), 12, "each file parsed once");
        let second = store.list().expect("second listing");
        assert_eq!(second, first, "the cached rows are identical");
        assert_eq!(
            store.parse_count(),
            12,
            "the second listing re-parses nothing"
        );
        // A replayed write finds its existing job without re-parsing the
        // directory: the cached rows answer the idempotency scan.
        let replay = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1_792_000_000_000,
            "acct_0",
            serde_json::json!({"row": 0}),
            "key_00",
            "corr_0",
        );
        match store.write(&replay).expect("replay") {
            JobWrite::Duplicate(existing) => {
                assert_eq!(existing.requested_at_ms, 1_792_000_000_000)
            }
            other => panic!("expected a duplicate, got {other:?}"),
        }
        assert_eq!(store.parse_count(), 12, "the replay re-parses nothing");

        // A rewritten file (different length and mtime) re-parses exactly
        // that one file and the listing reflects the new state.
        let job_id = match &first[0] {
            JobRead::Parsed(job) => job.job_id.clone(),
            other => panic!("expected a parsed job, got {other:?}"),
        };
        let path = dir.join(format!("{job_id}.json"));
        let mut updated: OperatorJob =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("raw")).expect("parse");
        updated.state = "completed".to_string();
        updated.state_history.push(JobStateEntry {
            state: "completed".to_string(),
            at_ms: 1_792_000_100_000,
            note: "runner finished".to_string(),
        });
        std::fs::write(&path, serde_json::to_string_pretty(&updated).expect("json"))
            .expect("rewrite");
        let third = store.list().expect("third listing");
        assert_eq!(third.len(), 12);
        assert_eq!(store.parse_count(), 13, "only the changed file re-parses");
        let row = third
            .iter()
            .find(|row| row.sort_key() == job_id)
            .expect("the rewritten row");
        assert_eq!(row.display_state(), "Completed");
    }

    /// A same-length rewrite whose mtime is restored (`touch -t`) must still
    /// re-parse: the Unix ctime is part of the cache key. A pair-keyed cache
    /// serves the stale row here; this test fails on it.
    #[test]
    fn job_cache_detects_a_same_length_rewrite_with_restored_mtime() {
        let (store, dir) = store();
        let job = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1_792_000_000_000,
            "acct_0",
            serde_json::json!({"row": 0}),
            "key_00",
            "corr_0",
        );
        assert!(store.write(&job).expect("write").created());
        let first = store.list().expect("first listing");
        assert_eq!(store.parse_count(), 1);
        let job_id = match &first[0] {
            JobRead::Parsed(job) => job.job_id.clone(),
            other => panic!("expected a parsed job, got {other:?}"),
        };
        let path = dir.join(format!("{job_id}.json"));
        let before = std::fs::read_to_string(&path).expect("raw");
        let mutated = before.replacen("\"state\": \"queued\"", "\"state\": \"failed\"", 1);
        assert_eq!(before.len(), mutated.len(), "the rewrite keeps the length");
        assert_ne!(before, mutated);
        let modified = std::fs::metadata(&path)
            .expect("stat")
            .modified()
            .expect("mtime");
        std::fs::write(&path, &mutated).expect("rewrite");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(modified)
            .expect("restore mtime");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("stat")
                .modified()
                .expect("mtime"),
            modified,
            "the mtime is restored to the pair-keyed value"
        );

        let second = store.list().expect("second listing");
        assert_eq!(
            store.parse_count(),
            2,
            "the restored-mtime rewrite re-parses exactly that file"
        );
        assert_eq!(second[0].display_state(), "Failed", "the new row is served");
    }

    #[test]
    fn job_writes_do_not_rescan_the_whole_directory_each_time() {
        let (store, _dir) = store();
        let job = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1_792_000_000_000,
            "acct_0",
            serde_json::json!({}),
            "stable_key",
            "corr",
        );
        assert!(store.write(&job).expect("first").created());
        let after_first = store.parse_count();
        assert!(after_first <= 1, "at most the one written file is parsed");
        // The first scan after the write parses the one file; every later
        // replay hits the cache.
        match store.write(&job).expect("replay") {
            JobWrite::Duplicate(_) => {}
            other => panic!("expected duplicate, got {other:?}"),
        }
        let cached = store.parse_count();
        assert!(cached <= after_first + 1, "one cold-list parse at most");
        for _ in 0..4 {
            match store.write(&job).expect("replay") {
                JobWrite::Duplicate(_) => {}
                other => panic!("expected duplicate, got {other:?}"),
            }
        }
        assert_eq!(
            store.parse_count(),
            cached,
            "replays must not re-parse existing files"
        );
        let conflicting = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1_792_000_000_000,
            "acct_0",
            serde_json::json!({"changed": true}),
            "stable_key",
            "corr",
        );
        match store.write(&conflicting).expect("conflict") {
            JobWrite::KeyConflict(_) => {}
            other => panic!("expected a conflict, got {other:?}"),
        }
        assert_eq!(store.parse_count(), cached);
    }

    /// Regression: every write used to run `existing_for_key`, which listed
    /// the directory. The key index makes N consecutive writes cost one cold
    /// scan and zero rescans. Pre-fix the scan counter grows with N.
    #[test]
    fn consecutive_writes_scan_the_directory_once() {
        let (store, dir) = store();
        for i in 0..64 {
            let job = OperatorJob::queued(
                JobKind::RetryRead,
                "maya",
                1_792_000_000_000 + i,
                &format!("acct_{i}"),
                serde_json::json!({"row": i}),
                &format!("key_{i:02}"),
                &format!("corr_{i}"),
            );
            assert!(store.write(&job).expect("write").created());
        }
        assert_eq!(store.scan_count(), 1, "one cold scan for 64 writes");
        assert_eq!(store.parse_count(), 0, "writes parse no existing file");

        let replay = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1_792_000_000_000,
            "acct_0",
            serde_json::json!({"row": 0}),
            "key_00",
            "corr_0",
        );
        match store.write(&replay).expect("replay") {
            JobWrite::Duplicate(existing) => {
                assert_eq!(existing.requested_at_ms, 1_792_000_000_000)
            }
            other => panic!("expected a duplicate, got {other:?}"),
        }
        assert_eq!(store.scan_count(), 1, "a replay does not rescan either");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An external writer (the API endpoint, the runner) changes the
    /// directory's identity, so the next write refreshes the index once and
    /// dedupes against the file it discovers.
    #[test]
    fn an_externally_added_job_file_is_discovered_through_the_directory_stamp() {
        let (store, dir) = store();
        let first = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1,
            "acct_0",
            serde_json::json!({}),
            "first-key",
            "corr-1",
        );
        assert!(store.write(&first).expect("first").created());
        assert_eq!(store.scan_count(), 1);

        // Written directly, exactly like the endpoint/runner persisting a
        // job without this store's knowledge.
        let external = OperatorJob::queued(
            JobKind::RecheckRequest,
            "sam",
            2,
            "ch_0246",
            serde_json::json!({"external": true}),
            "external-key",
            "corr-ext",
        );
        std::fs::write(
            dir.join(format!("{}.json", external.job_id)),
            serde_json::to_string_pretty(&external).expect("json"),
        )
        .expect("external write");

        let replay = OperatorJob::queued(
            JobKind::RecheckRequest,
            "sam",
            3,
            "ch_0246",
            serde_json::json!({"external": true}),
            "external-key",
            "corr-replay",
        );
        assert_ne!(replay.job_id, external.job_id);
        match store.write(&replay).expect("dedupe") {
            JobWrite::Duplicate(existing) => assert_eq!(existing.job_id, external.job_id),
            other => panic!("expected a duplicate, got {other:?}"),
        }
        assert_eq!(
            store.scan_count(),
            2,
            "the changed directory stamp triggers exactly one refresh"
        );
        assert!(
            !dir.join(format!("{}.json", replay.job_id)).exists(),
            "the externally claimed key is not written again"
        );
        assert_eq!(store.list().expect("list").len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A runner state rewrite (temp file plus rename) is visible to a read,
    /// to an idempotent replay (the indexed file is stat-ed before use), and
    /// to a listing.
    #[test]
    fn an_external_state_rewrite_is_reflected_in_read_list_and_replay() {
        let (store, dir) = store();
        let job = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1,
            "acct_0",
            serde_json::json!({}),
            "state-key",
            "corr-1",
        );
        assert!(store.write(&job).expect("write").created());
        let path = dir.join(format!("{}.json", job.job_id));
        let mut updated: OperatorJob =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("raw")).expect("parse");
        updated.state = "completed".to_string();
        updated.state_history.push(JobStateEntry {
            state: "completed".to_string(),
            at_ms: 2,
            note: "runner finished".to_string(),
        });
        let temp = dir.join(format!(".{}.json.tmp", job.job_id));
        std::fs::write(&temp, serde_json::to_string_pretty(&updated).expect("json")).expect("temp");
        std::fs::rename(&temp, &path).expect("atomic rewrite");

        match store.read(&job.job_id).expect("read") {
            JobRead::Parsed(back) => assert_eq!(back.state, "completed"),
            other => panic!("expected a parsed job, got {other:?}"),
        }

        let replay = OperatorJob::queued(
            JobKind::RetryRead,
            "maya",
            1,
            "acct_0",
            serde_json::json!({}),
            "state-key",
            "corr-1",
        );
        match store.write(&replay).expect("replay") {
            JobWrite::Duplicate(existing) => {
                assert_eq!(existing.state, "completed");
                assert_eq!(existing.state_history.len(), 2);
            }
            other => panic!("expected a duplicate, got {other:?}"),
        }

        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].display_state(), "Completed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
