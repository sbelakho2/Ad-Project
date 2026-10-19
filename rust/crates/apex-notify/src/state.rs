//! The delivery state store: append-only JSONL rows.
//!
//! Page 94 (delivery contract): "Record requested, accepted, delivered or
//! failed as distinct states when the provider supplies them. Deduplicate
//! retries". Every send therefore appends a `requested` row before the sink
//! is called and exactly one outcome row after; a `(template_class,
//! dedup_key, to)` triple that already has an `accepted` or `delivered` row
//! never sends again inside the dedup window.
//!
//! The store is opened on `APEX_NOTIFY_STATE`; without a path it still
//! deduplicates within the process, and that limitation is recorded in
//! [`DeliveryLog::path`]. A missing file is a first run; a file that exists
//! but cannot be read (permissions, I/O) is a typed error, never an empty
//! deduplication index.
//!
//! ## Cost and retention
//!
//! Opening folds the whole log once into an in-memory lookup index. A send
//! then costs one `stat` (the stamp check below), one hash lookup and one
//! constant-size append per recorded row; it never re-reads or re-folds the
//! log. The stamp is the file's length, modification time and (on Unix) ctime
//! and inode, so an external append is detected and folded exactly once.
//!
//! The dedup index is bounded by [`DEDUP_WINDOW_MS`]: an `accepted` or
//! `delivered` row stops blocking a resend once it is older than that window,
//! and [`RETENTION_MS`] is never shorter than the window, so no resend can
//! happen while the durable row still says it is inside the window. Rows
//! older than the retention are dropped from the in-memory index as time
//! advances and are physically compacted out of the file on the next open
//! (an atomic rewrite under the documented single-writer ownership of the
//! state path). That bounds both memory and the on-disk log instead of letting
//! dedup state grow without limit.

use std::collections::{hash_map::Entry, HashMap};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The environment variable naming the state file.
pub const STATE_ENV: &str = "APEX_NOTIFY_STATE";

/// How long an `accepted` or `delivered` row blocks a resend of the same
/// `(template_class, dedup_key)` to the same recipient: 30 days. A retry
/// inside this window is a duplicate; after it, the message may be sent again.
pub const DEDUP_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

/// How long delivery rows are retained in the index and on disk: the dedup
/// window. Keeping retention at least as long as the window is what makes
/// "never resend inside the window" hold across process restarts.
pub const RETENTION_MS: i64 = DEDUP_WINDOW_MS;

const _: () = assert!(RETENTION_MS >= DEDUP_WINDOW_MS);

/// How often the in-memory index is swept for expired entries. A sweep is a
/// full index walk, so it is amortised over an hour rather than done per send.
const PRUNE_INTERVAL_MS: i64 = 60 * 60 * 1_000;
/// Distinguishes concurrent compaction temp names within one process.
static COMPACT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

/// Observable counters of [`DeliveryLog`]: file reads, folds, stat probes,
/// appends, retention work and the size of the live dedup index. Tests and the
/// performance harness read them to prove that sends do not rescan the log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliveryLogStats {
    /// Whole-file reads (one per open or detected external change).
    pub reads: u64,
    /// Whole-file folds into the index (one per read).
    pub folds: u64,
    /// File identity probes (`stat`) made by sends and appends.
    pub stat_calls: u64,
    /// State rows appended by this process.
    pub appends: u64,
    /// Rows dropped as older than [`RETENTION_MS`].
    pub pruned_rows: u64,
    /// Atomic compactions that removed expired rows from the file.
    pub compactions: u64,
    /// Probes that could not inspect a changed file (blocked fail-closed).
    pub refresh_errors: u64,
    /// Live dedup triples in the index.
    pub dedup_entries: usize,
    /// Approximate bytes held by the dedup keys (template class + dedup key +
    /// recipient), excluding map overhead.
    pub dedup_key_bytes: usize,
}

/// One file's cache identity: byte length, modification time (`None` when the
/// filesystem does not expose one) and, on Unix, the ctime and inode. A
/// same-length rewrite that puts the mtime back still changes the ctime, so
/// the cached fold is invalidated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
    #[cfg(unix)]
    ino: u64,
}

impl StateStamp {
    fn of(metadata: &std::fs::Metadata) -> Self {
        let len = metadata.len();
        let modified = metadata.modified().ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Self {
                len,
                modified,
                ctime: metadata.ctime(),
                ctime_nsec: metadata.ctime_nsec(),
                ino: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            Self { len, modified }
        }
    }
}

/// The folded delivery log: the stamp it was folded at plus a three-level map
/// `template_class -> dedup_key -> recipient -> last blocking time`.
///
/// Nested maps make a probe allocation-free: each level looks up a borrowed
/// `&str`. A flat set of owned triples would allocate three `String`s on every
/// send just to ask whether the triple is present.
#[derive(Debug, Default)]
struct DeliveryIndex {
    stamp: Option<StateStamp>,
    sent: HashMap<String, HashMap<String, HashMap<String, i64>>>,
    indexed: usize,
    last_prune_ms: i64,
}

impl DeliveryIndex {
    fn last_blocking_at(&self, template_class: &str, dedup_key: &str, to: &str) -> Option<i64> {
        self.sent
            .get(template_class)
            .and_then(|by_key| by_key.get(dedup_key))
            .and_then(|by_to| by_to.get(to))
            .copied()
    }

    /// Whether the newest blocking row is still inside the dedup window as of
    /// `now_unix_ms`. Evaluated at query time so a row that just left the
    /// window stops blocking immediately, independent of the hourly sweep;
    /// the sweep only bounds memory and the file.
    fn blocks_at(&self, template_class: &str, dedup_key: &str, to: &str, now_unix_ms: i64) -> bool {
        self.last_blocking_at(template_class, dedup_key, to)
            .is_some_and(|at| now_unix_ms.saturating_sub(at) <= DEDUP_WINDOW_MS)
    }

    /// Record one blocking row, keeping the newest time for a triple.
    fn insert_row(&mut self, row: &DeliveryStateRow) {
        let by_to = self
            .sent
            .entry(row.template_class.clone())
            .or_default()
            .entry(row.dedup_key.clone())
            .or_default();
        match by_to.entry(row.to.clone()) {
            Entry::Occupied(mut existing) => {
                if row.at_unix_ms > *existing.get() {
                    existing.insert(row.at_unix_ms);
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(row.at_unix_ms);
                self.indexed += 1;
            }
        }
    }

    /// Drop entries older than the window, at most once per
    /// [`PRUNE_INTERVAL_MS`]. Expired triples may be resent, exactly as the
    /// documented dedup window allows.
    fn prune(&mut self, now_unix_ms: i64) {
        if now_unix_ms.saturating_sub(self.last_prune_ms) < PRUNE_INTERVAL_MS {
            return;
        }
        self.last_prune_ms = now_unix_ms;
        let cutoff = now_unix_ms.saturating_sub(RETENTION_MS);
        self.sent.retain(|_, by_key| {
            by_key.retain(|_, by_to| {
                by_to.retain(|_, at| *at >= cutoff);
                !by_to.is_empty()
            });
            !by_key.is_empty()
        });
        self.indexed = self
            .sent
            .values()
            .map(|by_key| by_key.values().map(HashMap::len).sum::<usize>())
            .sum();
    }

    fn measure(&self) -> (usize, usize) {
        let mut bytes = 0usize;
        for (template_class, by_key) in &self.sent {
            bytes += template_class.len();
            for (dedup_key, by_to) in by_key {
                bytes += dedup_key.len();
                for to in by_to.keys() {
                    bytes += to.len();
                }
            }
        }
        (self.indexed, bytes)
    }
}

/// The append-only delivery log.
#[derive(Debug)]
pub struct DeliveryLog {
    path: Option<PathBuf>,
    index: Mutex<DeliveryIndex>,
    write_lock: Mutex<()>,
    reads: AtomicU64,
    folds: AtomicU64,
    stat_calls: AtomicU64,
    appends: AtomicU64,
    pruned_rows: AtomicU64,
    compactions: AtomicU64,
    refresh_errors: AtomicU64,
}

impl DeliveryLog {
    /// Open the log at `path` (creating parent directories). A missing file
    /// is a first run (an empty log); any other read failure (permissions,
    /// I/O) is a typed error, so a deduplication index is never silently
    /// emptied and messages never silently re-send. A corrupt line is a
    /// typed error, never a silent reset.
    pub fn open(path: Option<&Path>) -> Result<Self, StateError> {
        Self::open_at(path, now_unix_ms())
    }

    /// Open as of `now_unix_ms`: rows older than [`RETENTION_MS`] are dropped
    /// from the index and compacted out of the file. This is the deterministic
    /// seam the retention tests use; [`Self::open`] passes the wall clock.
    pub fn open_at(path: Option<&Path>, now_unix_ms: i64) -> Result<Self, StateError> {
        let mut index = DeliveryIndex {
            last_prune_ms: now_unix_ms,
            ..DeliveryIndex::default()
        };
        let mut reads = 0u64;
        let mut folds = 0u64;
        let mut pruned = 0u64;
        let mut compactions = 0u64;
        if let Some(path) = path {
            let (rows, stamp) = Self::read_file(path)?;
            if stamp.is_some() {
                reads = 1;
                folds = 1;
            }
            index.stamp = stamp;
            let cutoff = now_unix_ms.saturating_sub(RETENTION_MS);
            let mut kept = Vec::with_capacity(rows.len());
            for row in rows {
                if row.at_unix_ms < cutoff {
                    pruned += 1;
                    continue;
                }
                if StateKind::parse(&row.state).is_some_and(StateKind::blocks_resend) {
                    index.insert_row(&row);
                }
                kept.push(row);
            }
            if pruned > 0 {
                // The index is bounded immediately; the file is bounded too,
                // at the next open, by an atomic rewrite of the retained rows.
                Self::compact(path, &kept)?;
                compactions = 1;
                index.stamp = Self::stat(path)?;
            }
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| Self::io(Some(parent), error))?;
                }
            }
        }
        Ok(Self {
            path: path.map(Path::to_path_buf),
            index: Mutex::new(index),
            write_lock: Mutex::new(()),
            reads: AtomicU64::new(reads),
            folds: AtomicU64::new(folds),
            stat_calls: AtomicU64::new(0),
            appends: AtomicU64::new(0),
            pruned_rows: AtomicU64::new(pruned),
            compactions: AtomicU64::new(compactions),
            refresh_errors: AtomicU64::new(0),
        })
    }

    /// The state file, when one is configured.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// This log's counters and index size (tests and the performance harness).
    pub fn stats(&self) -> DeliveryLogStats {
        let (dedup_entries, dedup_key_bytes) = self
            .index
            .lock()
            .map(|index| index.measure())
            .unwrap_or((0, 0));
        DeliveryLogStats {
            reads: self.reads.load(Ordering::Relaxed),
            folds: self.folds.load(Ordering::Relaxed),
            stat_calls: self.stat_calls.load(Ordering::Relaxed),
            appends: self.appends.load(Ordering::Relaxed),
            pruned_rows: self.pruned_rows.load(Ordering::Relaxed),
            compactions: self.compactions.load(Ordering::Relaxed),
            refresh_errors: self.refresh_errors.load(Ordering::Relaxed),
            dedup_entries,
            dedup_key_bytes,
        }
    }

    /// Whether `(template_class, dedup_key)` was already accepted or
    /// delivered to `to` and is still inside the retained dedup window.
    ///
    /// This probe refreshes from the file when its stamp moved (one read and
    /// fold per external change) and applies the window against the wall
    /// clock, so an expired triple does not block even when the hourly sweep
    /// has not run. It does not sweep the index.
    pub fn has_sent(&self, template_class: &str, dedup_key: &str, to: &str) -> bool {
        let now_unix_ms = now_unix_ms();
        let Ok(mut index) = self.index.lock() else {
            // A poisoned index cannot prove the message is new: fail closed.
            self.refresh_errors.fetch_add(1, Ordering::Relaxed);
            return true;
        };
        if self.refresh(&mut index, now_unix_ms).is_err() {
            self.refresh_errors.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        index.blocks_at(template_class, dedup_key, to, now_unix_ms)
    }

    /// Whether the triple is blocked as of `now_unix_ms`; also sweeps expired
    /// entries at most once per [`PRUNE_INTERVAL_MS`]. This is the notifier's
    /// per-send probe: one stat, no rescan of the log while the stamp is
    /// unchanged. The window is evaluated against `now_unix_ms` directly, so
    /// a row that just expired stops blocking even when the sweep is not due.
    pub fn has_sent_at(
        &self,
        template_class: &str,
        dedup_key: &str,
        to: &str,
        now_unix_ms: i64,
    ) -> bool {
        let Ok(mut index) = self.index.lock() else {
            self.refresh_errors.fetch_add(1, Ordering::Relaxed);
            return true;
        };
        if self.refresh(&mut index, now_unix_ms).is_err() {
            self.refresh_errors.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        index.prune(now_unix_ms);
        index.blocks_at(template_class, dedup_key, to, now_unix_ms)
    }

    /// Append one row and update the deduplication index. A write failure to
    /// a configured file is a typed error: deduplication state must not be
    /// lost silently.
    pub fn record(&self, row: &DeliveryStateRow) -> Result<(), StateError> {
        let kind = StateKind::parse(&row.state).ok_or_else(|| StateError::UnknownState {
            state: row.state.clone(),
        })?;
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| Self::io(self.path.as_deref(), "state store write lock is poisoned"))?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| Self::io(self.path.as_deref(), "state store index lock is poisoned"))?;
        let observed = match &self.path {
            Some(path) => {
                self.stat_calls.fetch_add(1, Ordering::Relaxed);
                Self::stat(path)?
            }
            None => None,
        };
        // Fold an external change before appending, so a row another process
        // wrote is never lost from the index just because ours is newer.
        if self.path.is_some() && index.stamp != observed {
            self.reload(&mut index, row.at_unix_ms)?;
        }
        if let Some(path) = &self.path {
            let line = serde_json::to_string(row).map_err(|error| Self::io(Some(path), error))?;
            use std::fs::OpenOptions;
            let mut options = OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(path)
                .map_err(|error| Self::io(Some(path), error))?;
            writeln!(file, "{line}").map_err(|error| Self::io(Some(path), error))?;
            drop(file);
            self.appends.fetch_add(1, Ordering::Relaxed);
            let written = line.len() as u64 + 1;
            self.stat_calls.fetch_add(1, Ordering::Relaxed);
            let after = Self::stat(path)?;
            index.stamp = match after {
                Some(after) if appended_alone(observed.as_ref(), &after, written) => Some(after),
                // A concurrent external append makes our cached stamp stale:
                // drop it so the next probe folds the file exactly once.
                _ => None,
            };
        }
        if kind.blocks_resend() {
            index.insert_row(row);
        }
        index.prune(row.at_unix_ms);
        Ok(())
    }

    /// Every row of a state file, for tests and operators.
    pub fn read_rows(path: &Path) -> Result<Vec<DeliveryStateRow>, StateError> {
        Self::read_file(path).map(|(rows, _)| rows)
    }

    /// The file's current identity, `None` when it is missing.
    fn stat(path: &Path) -> Result<Option<StateStamp>, StateError> {
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(Some(StateStamp::of(&metadata))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Self::io(Some(path), error)),
        }
    }

    /// Read and validate the whole log. The returned stamp is the identity
    /// observed before the read, so a file that changed during the read is
    /// re-read on the next probe instead of being trusted.
    fn read_file(path: &Path) -> Result<(Vec<DeliveryStateRow>, Option<StateStamp>), StateError> {
        let before = Self::stat(path)?;
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), None))
            }
            Err(error) => return Err(Self::io(Some(path), error)),
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut rows = Vec::new();
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
            StateKind::parse(&row.state).ok_or_else(|| StateError::UnknownState {
                state: row.state.clone(),
            })?;
            rows.push(row);
        }
        Ok((rows, before))
    }

    /// Stat the file and re-fold it only when its identity moved. The caller
    /// holds the index guard, so one change is one read and one fold.
    fn refresh(&self, index: &mut DeliveryIndex, now_unix_ms: i64) -> Result<(), StateError> {
        let Some(path) = &self.path else {
            // An in-process log has no file to observe.
            return Ok(());
        };
        let stamp = Self::stat(path)?;
        self.stat_calls.fetch_add(1, Ordering::Relaxed);
        if stamp.is_none() {
            // A deletion after a fold must read as the empty store it is now.
            if index.stamp.is_some() || !index.sent.is_empty() {
                *index = DeliveryIndex {
                    last_prune_ms: now_unix_ms,
                    ..DeliveryIndex::default()
                };
            }
            return Ok(());
        }
        if index.stamp == stamp {
            return Ok(());
        }
        self.reload(index, now_unix_ms)
    }

    /// Re-fold the file into `index`. Rows outside the retention window are
    /// counted and dropped rather than indexed.
    fn reload(&self, index: &mut DeliveryIndex, now_unix_ms: i64) -> Result<(), StateError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let (rows, stamp) = Self::read_file(path)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.folds.fetch_add(1, Ordering::Relaxed);
        let cutoff = now_unix_ms.saturating_sub(RETENTION_MS);
        let mut next = DeliveryIndex {
            stamp,
            last_prune_ms: now_unix_ms,
            ..DeliveryIndex::default()
        };
        for row in rows {
            if row.at_unix_ms < cutoff {
                self.pruned_rows.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if StateKind::parse(&row.state).is_some_and(StateKind::blocks_resend) {
                next.insert_row(&row);
            }
        }
        *index = next;
        Ok(())
    }

    /// Atomically rewrite `path` with only `rows`. This runs at open, under
    /// the documented single-writer ownership of the state path, so it cannot
    /// race a live append from this process; a second process sharing the path
    /// is not a supported configuration for the delivery state.
    fn compact(path: &Path, rows: &[DeliveryStateRow]) -> Result<(), StateError> {
        use std::fs::OpenOptions;
        let (tmp, file) = loop {
            let sequence = COMPACT_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            let candidate = path.with_extension(format!(
                "jsonl.{}.{}.{}.tmp",
                std::process::id(),
                nanos,
                sequence
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&candidate) {
                Ok(file) => break (candidate, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(Self::io(Some(&candidate), error)),
            }
        };
        {
            let mut writer = std::io::BufWriter::new(file);
            for row in rows {
                let line =
                    serde_json::to_string(row).map_err(|error| Self::io(Some(&tmp), error))?;
                writeln!(writer, "{line}").map_err(|error| Self::io(Some(&tmp), error))?;
            }
            writer
                .flush()
                .map_err(|error| Self::io(Some(&tmp), error))?;
        }
        std::fs::rename(&tmp, path).map_err(|error| {
            let _ = std::fs::remove_file(&tmp);
            Self::io(Some(path), error)
        })
    }

    fn io(path: Option<&Path>, reason: impl std::fmt::Display) -> StateError {
        StateError::Io {
            path: path
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<memory>".to_string()),
            reason: reason.to_string(),
        }
    }
}

/// Whether the file after our append is exactly the file before it plus the
/// bytes we wrote. A concurrent external append between the stamp and our
/// write makes the length (or the inode) disagree, so the index is dropped and
/// rebuilt once instead of hiding the external row.
fn appended_alone(before: Option<&StateStamp>, after: &StateStamp, written: u64) -> bool {
    match before {
        Some(before) => after.len == before.len.saturating_add(written) && same_file(before, after),
        None => after.len == written,
    }
}

#[cfg(unix)]
fn same_file(before: &StateStamp, after: &StateStamp) -> bool {
    before.ino == after.ino
}

#[cfg(not(unix))]
fn same_file(_before: &StateStamp, _after: &StateStamp) -> bool {
    true
}

/// The wall clock in unix milliseconds.
fn now_unix_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    fn row_at(state: &str, dedup_key: &str, at_unix_ms: i64) -> DeliveryStateRow {
        DeliveryStateRow {
            at_unix_ms,
            template_class: "payment_failed".into(),
            dedup_key: dedup_key.into(),
            to: "ops@example.com".into(),
            state: state.into(),
            provider_reference: None,
            detail: None,
        }
    }

    fn row(state: &str, dedup_key: &str) -> DeliveryStateRow {
        // `has_sent` applies the wall clock; a row stamped at the epoch would
        // be outside the window before the probe even runs.
        row_at(state, dedup_key, now_unix_ms())
    }

    #[cfg(unix)]
    #[test]
    fn the_delivery_state_file_is_created_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = std::env::temp_dir().join(format!(
            "apex-notify-mode-{}-{}",
            std::process::id(),
            now_unix_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open_at(Some(&path), NOW).expect("open");
        log.record(&row_at("accepted", "k1", NOW)).unwrap();
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "delivery state file");
        let _ = std::fs::remove_dir_all(&dir);
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
        let log = DeliveryLog::open_at(Some(&path), 1).expect("open");
        assert_eq!(log.path(), Some(path.as_path()));
        log.record(&row("requested", "k1")).unwrap();
        log.record(&row("accepted", "k1")).unwrap();
        drop(log);

        let reopened = DeliveryLog::open_at(Some(&path), 1).expect("reopen");
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
    fn sends_do_not_rescan_the_log_per_message() {
        let dir = std::env::temp_dir().join(format!("apex-notify-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open_at(Some(&path), NOW).expect("open");
        assert_eq!(log.stats().reads, 0, "a missing file is not a read");
        for index in 0..100 {
            let key = format!("k{index}");
            log.record(&row_at("accepted", &key, NOW + i64::from(index)))
                .unwrap();
            assert!(log.has_sent_at(
                "payment_failed",
                &key,
                "ops@example.com",
                NOW + i64::from(index)
            ));
        }
        let stats = log.stats();
        assert_eq!(stats.reads, 0, "sends never re-read the log");
        assert_eq!(stats.folds, 0, "sends never re-fold the index");
        assert!(stats.stat_calls >= 100, "one identity probe per send");
        assert_eq!(stats.dedup_entries, 100);

        // An external append moves the stamp: the next probe folds once.
        let external = row_at("accepted", "external", NOW + 500);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", serde_json::to_string(&external).unwrap()).unwrap();
        drop(file);
        assert!(log.has_sent_at("payment_failed", "external", "ops@example.com", NOW + 500));
        let stats = log.stats();
        assert_eq!(stats.reads, 1, "exactly one read for the external change");
        assert_eq!(stats.folds, 1, "exactly one fold for the external change");
        assert!(
            log.has_sent_at("payment_failed", "k0", "ops@example.com", NOW + 500),
            "the earlier accepted rows are still indexed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retention_is_bounded_and_never_forgets_inside_the_window() {
        let dir = std::env::temp_dir().join(format!("apex-notify-retain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("delivery.jsonl");
        let mut body = String::new();
        for row in [
            row_at("accepted", "old", NOW),
            row_at("accepted", "edge", NOW + 1),
            row_at("accepted", "fresh", NOW + DEDUP_WINDOW_MS + 1),
        ] {
            body.push_str(&serde_json::to_string(&row).unwrap());
            body.push('\n');
        }
        std::fs::write(&path, body).unwrap();

        // One millisecond past a full window after `old`, that row is outside
        // the window and is compacted away; `edge` sits exactly on the
        // boundary (age == window) and must still block a resend.
        let now = NOW + DEDUP_WINDOW_MS + 1;
        let log = DeliveryLog::open_at(Some(&path), now).expect("open");
        assert!(!log.has_sent_at("payment_failed", "old", "ops@example.com", now));
        assert!(log.has_sent_at("payment_failed", "edge", "ops@example.com", now));
        assert!(log.has_sent_at("payment_failed", "fresh", "ops@example.com", now));
        let stats = log.stats();
        assert_eq!(stats.pruned_rows, 1);
        assert_eq!(stats.compactions, 1);
        let rows = DeliveryLog::read_rows(&path).expect("rows");
        assert_eq!(rows.len(), 2, "the expired row is gone from the file");
        assert!(rows.iter().all(|row| row.dedup_key != "old"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_window_is_evaluated_at_query_time_before_the_sweep() {
        let log = DeliveryLog::open_at(None, NOW).expect("memory log");
        // Three rows, all recorded before `NOW` while the process clock is
        // `NOW`; no hourly sweep is due at `NOW`, so the probe itself must
        // apply the window instead of waiting for the sweep.
        log.record(&row_at("accepted", "window-1", NOW - DEDUP_WINDOW_MS + 1))
            .unwrap();
        log.record(&row_at("accepted", "exactly", NOW - DEDUP_WINDOW_MS))
            .unwrap();
        log.record(&row_at("accepted", "window+1ms", NOW - DEDUP_WINDOW_MS - 1))
            .unwrap();
        assert!(
            log.has_sent_at("payment_failed", "window-1", "ops@example.com", NOW),
            "age window-1 is inside the window"
        );
        assert!(
            log.has_sent_at("payment_failed", "exactly", "ops@example.com", NOW),
            "age exactly window is inside the window"
        );
        assert!(
            !log.has_sent_at("payment_failed", "window+1ms", "ops@example.com", NOW),
            "age window+1ms is outside the window even before a sweep"
        );
        assert_eq!(
            log.stats().dedup_entries,
            3,
            "expiry is decided by the query, not by the sweep: the row is still indexed"
        );
    }

    #[test]
    fn the_sweep_compacts_and_never_unblocks_inside_the_window() {
        let log = DeliveryLog::open_at(None, NOW).expect("memory log");
        log.record(&row_at("accepted", "k", NOW)).unwrap();
        // A sweep runs one prune interval later, still well inside the
        // window: the row is retained and keeps blocking.
        let swept = NOW + PRUNE_INTERVAL_MS;
        assert!(log.has_sent_at("payment_failed", "k", "ops@example.com", swept));
        assert_eq!(log.stats().dedup_entries, 1);
        // At exactly the window boundary the row is retained and blocks.
        let edge = NOW + DEDUP_WINDOW_MS;
        assert!(log.has_sent_at("payment_failed", "k", "ops@example.com", edge));
        assert_eq!(log.stats().dedup_entries, 1);
        // One millisecond later it stops blocking immediately; the row stays
        // in the index until the next sweep is due.
        let expired = edge + 1;
        assert!(!log.has_sent_at("payment_failed", "k", "ops@example.com", expired));
        assert_eq!(log.stats().dedup_entries, 1);
        // The next due sweep physically drops it.
        assert!(!log.has_sent_at(
            "payment_failed",
            "k",
            "ops@example.com",
            expired + PRUNE_INTERVAL_MS
        ));
        assert_eq!(log.stats().dedup_entries, 0);
    }

    #[test]
    fn an_in_memory_index_prunes_as_the_clock_advances() {
        let log = DeliveryLog::open_at(None, NOW).expect("memory log");
        log.record(&row_at("accepted", "k", NOW)).unwrap();
        assert!(log.has_sent_at("payment_failed", "k", "ops@example.com", NOW + 1));
        assert_eq!(log.stats().dedup_entries, 1);
        // One millisecond past the window the triple may be resent; the
        // in-memory index is swept without any file work.
        assert!(!log.has_sent_at(
            "payment_failed",
            "k",
            "ops@example.com",
            NOW + DEDUP_WINDOW_MS + 1
        ));
        let stats = log.stats();
        assert_eq!(stats.dedup_entries, 0);
        assert_eq!(stats.pruned_rows, 0, "no file, so no file pruning");
        assert_eq!(stats.reads, 0);
    }

    #[test]
    fn ten_thousand_blocking_rows_are_indexed_and_near_misses_stay_clear() {
        let dir = std::env::temp_dir().join(format!("apex-notify-scale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("delivery.jsonl");
        let mut body = String::with_capacity(10_000 * 180);
        for index in 0..10_000 {
            body.push_str(
                &serde_json::to_string(&row_at(
                    "accepted",
                    &format!("payment_failed:INV-{index}:psp_evt_{index}"),
                    NOW + index,
                ))
                .unwrap(),
            );
            body.push('\n');
        }
        std::fs::write(&path, body).unwrap();
        let now = NOW + 10_000;
        let log = DeliveryLog::open_at(Some(&path), now).expect("open");
        assert_eq!(log.stats().dedup_entries, 10_000);
        assert_eq!(log.stats().reads, 1);
        assert_eq!(log.stats().folds, 1);
        for index in (0..10_000).step_by(97) {
            assert!(
                log.has_sent_at(
                    "payment_failed",
                    &format!("payment_failed:INV-{index}:psp_evt_{index}"),
                    "ops@example.com",
                    now
                ),
                "row {index} is indexed"
            );
        }
        for index in [10_000i64, 10_001, 20_000] {
            assert!(
                !log.has_sent_at(
                    "payment_failed",
                    &format!("payment_failed:INV-{index}:psp_evt_{index}"),
                    "ops@example.com",
                    now
                ),
                "unknown key {index} does not block"
            );
        }
        assert_eq!(
            log.stats().reads,
            1,
            "the sweep of 104 probes never re-reads the log"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_failure_to_a_configured_file_is_typed() {
        let dir =
            std::env::temp_dir().join(format!("apex-notify-state-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Open with a missing file (a first run), then make the configured
        // path a directory: appending the first row fails typed.
        let path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open(Some(&path)).expect("open tolerates a missing file");
        std::fs::create_dir(&path).unwrap();
        let error = log.record(&row("requested", "k1")).expect_err("dir path");
        assert!(matches!(error, StateError::Io { .. }), "{error:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_that_cannot_be_read_is_a_typed_open_error_not_an_empty_log() {
        // A directory as the state path is a non-NotFound read error on
        // every platform: opening must fail loud, never start from empty.
        let dir =
            std::env::temp_dir().join(format!("apex-notify-state-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let error = DeliveryLog::open(Some(&dir)).expect_err("directory path");
        assert!(matches!(error, StateError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("notification state"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_state_file_fails_open_instead_of_emptying_the_index() {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("apex-notify-state-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open_at(Some(&path), 1).expect("open");
        log.record(&row("accepted", "k1")).unwrap();
        drop(log);
        let readable_mode = std::fs::metadata(&path).unwrap().permissions().mode();
        let mut locked = std::fs::metadata(&path).unwrap().permissions();
        locked.set_mode(0o000);
        std::fs::set_permissions(&path, locked).unwrap();

        if std::fs::read(&path).is_ok() {
            // Running as root (or on a filesystem that ignores the mode):
            // the permission bit did not take effect, so assert the same
            // fail-loud contract with a read error that cannot be bypassed.
            let mut restore = std::fs::metadata(&path).unwrap().permissions();
            restore.set_mode(readable_mode);
            std::fs::set_permissions(&path, restore).unwrap();
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
            let error = DeliveryLog::open(Some(&path)).expect_err("directory path");
            assert!(matches!(error, StateError::Io { .. }), "{error:?}");
        } else {
            let error = DeliveryLog::open(Some(&path)).expect_err("unreadable file");
            assert!(matches!(error, StateError::Io { .. }), "{error:?}");
            // Restore the mode and reopen: the dedup index loads again,
            // because a permission problem never trades the index away.
            let mut restore = std::fs::metadata(&path).unwrap().permissions();
            restore.set_mode(readable_mode);
            std::fs::set_permissions(&path, restore).unwrap();
            let reopened = DeliveryLog::open_at(Some(&path), 1).expect("reopen");
            assert!(reopened.has_sent("payment_failed", "k1", "ops@example.com"));
        }
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
