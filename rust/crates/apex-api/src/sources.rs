//! Snapshot and ledger sources, plus the publisher/platform parsing rules.
//!
//! Two real file-backed implementations ship here:
//!
//! - [`JsonFileAuditSource`] reads `<root>/<publisher>/<platform>.json` into
//!   an `apex_audit::Snapshot`: one snapshot per publisher and platform, the
//!   same layout the runner's `apex_autopilot::registry::snapshot_path`
//!   writes. A missing file is `404`; an unparseable or mismatched file is a
//!   logged internal error (an operator problem, not a client one). A snapshot
//!   is only ever served for the publisher whose own directory holds it, so one
//!   publisher can never read another's data.
//! - [`FileLedgerSource`] reads `<root>/<publisher>.jsonl` through
//!   [`apex_proof::Ledger::read_verified`], the single read that both verifies
//!   and returns the records. There is deliberately no second whole-file read:
//!   records are only ever parsed from bytes that passed verification. Records
//!   that passed verification are kept in a small in-process cache keyed by
//!   the file's size and modification time, so repeated reads (e.g. the
//!   overview totals walking every publisher on every request) do not re-run
//!   the full chain verification until the file changes. A verification
//!   failure is `422 ledger_unverified` with the 1-based first offending line
//!   quoted from the verifier; a ledger above [`MAX_LEDGER_BYTES`] is refused
//!   before any read.
//!
//! All filesystem work is synchronous and runs on `tokio`'s blocking pool via
//! [`tokio::task::spawn_blocking`], so handlers never block a runtime worker.
//!
//! ## Path traversal is structurally impossible, and symlinks cannot escape
//!
//! Publisher names are the only URL-derived file names, and
//! [`is_valid_publisher`] restricts them to `^[A-Za-z0-9_-]{1,64}$` (ASCII
//! alphanumerics, `_`, `-`). `.`, `/`, `\`, NUL, whitespace, unicode and
//! over-long names are refused with a `400 invalid_publisher` before any path
//! is built; the sources re-check the name so they are safe even when called
//! directly (pinned by tests). Platform names are matched against the closed
//! set `applovin_max | levelplay | admob`.
//!
//! As defense in depth over that allow-list, each source canonicalizes its
//! configured root once at construction and requires every file path to
//! canonicalize inside that root before reading: a symlink planted inside the
//! root that points elsewhere resolves outside and is reported as `404`, and a
//! dangling path is a `404` as well.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use apex_audit::{AuditReport, Snapshot};
use apex_core::PlatformId;
use apex_proof::{Ledger, LedgerError, LedgerRecord, VerifyingKey};
use async_trait::async_trait;

use crate::error::ApiError;

/// The three supported platforms and their console labels, in display order.
pub const PLATFORM_NAMES: [(&str, &str); 3] = [
    ("applovin_max", "AppLovin MAX"),
    ("levelplay", "LevelPlay"),
    ("admob", "AdMob"),
];

/// Every accepted spelling of a platform name, as `(spelling, canonical
/// name)`. `applovin_max` is the canonical file/route name and the one the
/// console uses; `max` is the spelling the free-audit form stores (see
/// `crate::leads::PLATFORM_CHOICES`). Both parsing directions derive from this
/// one table: submitted values normalize through [`canonical_platform_name`]
/// (to the lead wire spelling in `crate::leads::canonical_platform`, to the
/// canonical name for [`parse_platform`]), so the two vocabularies cannot
/// drift apart silently.
pub const PLATFORM_SPELLINGS: [(&str, &str); 4] = [
    ("applovin_max", "applovin_max"),
    ("max", "applovin_max"),
    ("levelplay", "levelplay"),
    ("admob", "admob"),
];

/// Read cap for one ledger file: a ledger larger than this is refused before
/// any bytes are read, so a hostile or runaway file cannot exhaust memory.
pub const MAX_LEDGER_BYTES: u64 = 32 * 1024 * 1024;

/// The canonical platform name for any accepted spelling (case-insensitive,
/// trimmed), or `None` for unknown input. This is the one normalization both
/// directions of platform parsing share: the lead surfaces canonicalize
/// submitted values through it, and [`parse_platform`] accepts exactly the
/// canonical names it returns.
pub fn canonical_platform_name(value: &str) -> Option<&'static str> {
    let value = value.trim();
    PLATFORM_SPELLINGS
        .iter()
        .find(|(spelling, _)| spelling.eq_ignore_ascii_case(value))
        .map(|(_, canonical)| *canonical)
}

/// Parse a URL/platform-file name into a [`PlatformId`]. The accepted set
/// stays the three canonical names; a submitted spelling (e.g. the lead
/// form's `max`) normalizes first through [`canonical_platform_name`].
pub fn parse_platform(name: &str) -> Option<PlatformId> {
    match name {
        "applovin_max" => Some(PlatformId::AppLovinMax),
        "levelplay" => Some(PlatformId::LevelPlay),
        "admob" => Some(PlatformId::Admob),
        _ => None,
    }
}

/// `^[A-Za-z0-9_-]{1,64}$` over bytes: publisher names become file names, so
/// only unreserved ASCII is allowed.
pub fn is_valid_publisher(publisher: &str) -> bool {
    !publisher.is_empty()
        && publisher.len() <= 64
        && publisher
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Resolve `path` and require the result to stay under `canonical_root`.
///
/// A missing (or dangling) path is [`ApiError::NotFound`], an unresolvable
/// path is a logged internal error, and a path that resolves outside the
/// canonical root — e.g. through a symlink planted in the root — is reported
/// as `NotFound` rather than served.
pub(crate) fn contained_in(canonical_root: &Path, path: PathBuf) -> Result<PathBuf, ApiError> {
    let resolved = path.canonicalize().map_err(|error| match error.kind() {
        ErrorKind::NotFound => ApiError::NotFound,
        _ => ApiError::Internal(format!("resolve {}: {error}", path.display())),
    })?;
    if resolved.starts_with(canonical_root) {
        Ok(resolved)
    } else {
        Err(ApiError::NotFound)
    }
}

/// Refuse a file whose on-disk size exceeds `cap` before it is read.
pub(crate) fn ensure_within_cap(size: u64, cap: u64) -> Result<(), ApiError> {
    if size > cap {
        return Err(ApiError::Internal(format!(
            "ledger is {size} bytes, above the {cap}-byte read cap"
        )));
    }
    Ok(())
}

/// Canonicalize a configured root once, falling back to the configured path
/// when it does not resolve yet (every later containment check then fails
/// closed until the root exists).
pub(crate) fn canonical_root(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

/// An audit snapshot provider (one snapshot per publisher and platform).
#[async_trait]
pub trait AuditSource: Send + Sync + std::fmt::Debug {
    /// Fetch `publisher`'s current snapshot for `platform`, or the typed error
    /// that prevents serving one. The publisher is validated before any path
    /// is built, so a source is safe even when called directly.
    async fn snapshot(&self, publisher: &str, platform: PlatformId) -> Result<Snapshot, ApiError>;

    /// The snapshot and the audit report computed from it. The default runs
    /// the engine on every call; the file source caches the report beside its
    /// parsed snapshot, keyed by the same hardened file stamp, so the
    /// overview, report, finding and CSV pages do not recompute it per render.
    async fn snapshot_report(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<(Snapshot, AuditReport), ApiError> {
        let snapshot = self.snapshot(publisher, platform).await?;
        let report = apex_audit::run(&snapshot);
        Ok((snapshot, report))
    }

    /// Whether this source holds at least one snapshot for `publisher`.
    /// Used to refuse a self-serve claim of an operator-provisioned
    /// workspace that already holds runner data.
    fn has_snapshot(&self, _publisher: &str) -> bool {
        false
    }

    /// The snapshot behind a shared handle. The file source hands out its
    /// cached `Arc`, so a render does not clone the whole snapshot; the
    /// default boxes whatever [`Self::snapshot`] returns.
    async fn snapshot_arc(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<Arc<Snapshot>, ApiError> {
        self.snapshot(publisher, platform).await.map(Arc::new)
    }

    /// The snapshot and report behind shared handles. The file source returns
    /// the cached `Arc`s (one report per unchanged snapshot), so the overview,
    /// audit, finding and CSV pages borrow them instead of cloning a whole
    /// snapshot and report per request.
    async fn snapshot_report_arc(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<(Arc<Snapshot>, Arc<AuditReport>), ApiError> {
        let (snapshot, report) = self.snapshot_report(publisher, platform).await?;
        Ok((Arc::new(snapshot), Arc::new(report)))
    }

    /// Persist a freshly built snapshot for `publisher`. The file source
    /// writes it beside the reads (atomically); a static source refuses.
    async fn write(
        &self,
        publisher: &str,
        platform: PlatformId,
        _snapshot: &Snapshot,
    ) -> Result<(), ApiError> {
        Err(ApiError::Internal(format!(
            "snapshot writing for publisher {publisher} on {platform} is not supported"
        )))
    }
}

/// One page of a publisher's verified records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerPage {
    /// Every verified record in the publisher's ledger (the page's
    /// denominator, not the number of records returned here).
    pub total: u64,
    /// Only the records in the requested range, in chain order.
    pub records: Vec<LedgerRecord>,
}

/// A verified ledger provider (one append-only chain per publisher).
#[async_trait]
pub trait LedgerSource: Send + Sync + std::fmt::Debug {
    /// Return the publisher's records after full chain verification.
    async fn records(&self, publisher: &str) -> Result<Vec<LedgerRecord>, ApiError>;

    /// Return the publisher's verified records behind a shared handle.
    ///
    /// Read-only pages (the record detail and the experiment/billing
    /// projections) borrow this slice instead of cloning the whole chain. The
    /// file source returns its cached `Arc`, so a cache hit allocates nothing;
    /// the default boxes whatever [`Self::records`] returns.
    async fn records_shared(&self, publisher: &str) -> Result<Arc<[LedgerRecord]>, ApiError> {
        self.records(publisher)
            .await
            .map(|records| Arc::from(records.into_boxed_slice()))
    }

    /// Return one page of the publisher's verified records.
    ///
    /// The default materializes the whole chain and slices the requested
    /// range; sources that keep verified records behind a shared handle (the
    /// file source's cache) override this so a page request never clones the
    /// full vector. `offset` past the end is an empty page, not an error; the
    /// caller uses `total` to decide whether the requested page exists.
    async fn records_page(
        &self,
        publisher: &str,
        offset: usize,
        limit: usize,
    ) -> Result<LedgerPage, ApiError> {
        let records = self.records(publisher).await?;
        let total = records.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        Ok(LedgerPage {
            total: total as u64,
            records: records[start..end].to_vec(),
        })
    }

    /// The number of verified records for `publisher`, with the same
    /// verification and error semantics as [`Self::records`]. Sources that
    /// can answer from verified state without materializing the records
    /// (the file source's cache) override this, so aggregate readers (the
    /// overview totals) do not copy every record just to count them. The
    /// default materializes and counts.
    async fn record_count(&self, publisher: &str) -> Result<u64, ApiError> {
        self.records(publisher)
            .await
            .map(|records| records.len() as u64)
    }

    /// The publishers this source can serve, sorted. The overview totals read
    /// every one of them; a source that cannot enumerate returns nothing.
    fn publishers(&self) -> Vec<String> {
        Vec::new()
    }

    /// Whether this source can serve at least one publisher.
    ///
    /// The overview only asks whether *any* ledger is configured; sources
    /// override this so the check does not clone and sort every name just to
    /// test emptiness.
    fn has_publishers(&self) -> bool {
        !self.publishers().is_empty()
    }

    /// Whether this source has no publishers at all.
    fn is_empty(&self) -> bool {
        !self.has_publishers()
    }
}

/// One parsed snapshot held in memory: the file identity it was parsed at and
/// (once requested) the audit report computed from it. The report is cached
/// beside the snapshot because the console renders it on overview, report,
/// finding and CSV pages; recomputing the engine per page is the dominant cost
/// at large snapshots.
#[derive(Debug)]
struct CachedSnapshot {
    stamp: FileStamp,
    snapshot: Arc<Snapshot>,
    report: std::sync::OnceLock<Arc<AuditReport>>,
}

/// Observable counters of [`JsonFileAuditSource`]'s parse cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapshotCacheStats {
    /// File reads (one per cache miss).
    pub reads: u64,
    /// JSON parses (one per cache miss with a usable file stamp).
    pub parses: u64,
    /// Cache hits (a stat plus a lookup, no read or parse).
    pub hits: u64,
    /// Audit-report computations (one per snapshot per file identity).
    pub report_builds: u64,
}

/// The in-process parsed-snapshot cache of [`JsonFileAuditSource`]. A read is
/// served from the cache only while the file's size, modification time and
/// (on Unix) ctime and inode are unchanged, the same hardened identity the
/// verified-ledger cache uses. Console pages load one publisher's snapshots
/// on every request, so the unchanged-file path is a stat plus a clone
/// instead of a re-read and re-parse of the JSON.
#[derive(Debug, Default)]
struct SnapshotCache {
    entries: Mutex<HashMap<PathBuf, CachedSnapshot>>,
    reads: AtomicU64,
    parses: AtomicU64,
    hits: AtomicU64,
    report_builds: AtomicU64,
}

/// One `read` result: the snapshot, its report when already built, the
/// resolved path and the stamp the pair may be cached under.
type CachedRead = (
    Arc<Snapshot>,
    Option<Arc<AuditReport>>,
    PathBuf,
    Option<FileStamp>,
);

impl SnapshotCache {
    fn get(
        &self,
        path: &Path,
        stamp: FileStamp,
    ) -> Option<(Arc<Snapshot>, Option<Arc<AuditReport>>)> {
        let entries = self.entries.lock().ok()?;
        entries.get(path).and_then(|cached| {
            (cached.stamp == stamp).then(|| {
                self.hits.fetch_add(1, Ordering::Relaxed);
                (
                    Arc::clone(&cached.snapshot),
                    cached.report.get().map(Arc::clone),
                )
            })
        })
    }

    fn put(&self, path: &Path, stamp: FileStamp, snapshot: &Arc<Snapshot>) {
        if let Ok(mut entries) = self.entries.lock() {
            // A concurrent request may have completed this entry (including
            // its report) while this one was reading; never replace it.
            if entries
                .get(path)
                .is_some_and(|cached| cached.stamp == stamp)
            {
                return;
            }
            entries.insert(
                path.to_path_buf(),
                CachedSnapshot {
                    stamp,
                    snapshot: Arc::clone(snapshot),
                    report: std::sync::OnceLock::new(),
                },
            );
        }
    }

    /// Attach `report` to the entry at `stamp`, unless another request already
    /// did or the file changed underneath this one.
    fn put_report(&self, path: &Path, stamp: FileStamp, report: &Arc<AuditReport>) {
        if let Ok(entries) = self.entries.lock() {
            if let Some(cached) = entries.get(path) {
                if cached.stamp == stamp {
                    let _ = cached.report.set(Arc::clone(report));
                }
            }
        }
    }

    fn stats(&self) -> SnapshotCacheStats {
        SnapshotCacheStats {
            reads: self.reads.load(Ordering::Relaxed),
            parses: self.parses.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            report_builds: self.report_builds.load(Ordering::Relaxed),
        }
    }
}

/// Reads `<root>/<publisher>/<platform>.json` as an `apex_audit::Snapshot`.
#[derive(Debug, Clone)]
pub struct JsonFileAuditSource {
    root: PathBuf,
    canonical_root: PathBuf,
    cache: Arc<SnapshotCache>,
}

impl JsonFileAuditSource {
    /// A source rooted at `root`; the root is canonicalized here, once.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let canonical_root = canonical_root(&root);
        Self {
            root,
            canonical_root,
            cache: Arc::new(SnapshotCache::default()),
        }
    }

    /// This source's parse-cache counters (tests and the performance harness).
    pub fn cache_stats(&self) -> SnapshotCacheStats {
        self.cache.stats()
    }

    /// The directory snapshots are read from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether `<root>/<publisher>/` exists (the runner's write layout).
    pub fn publisher_dir_exists(&self, publisher: &str) -> bool {
        if !is_valid_publisher(publisher) {
            return false;
        }
        contained_in(&self.canonical_root, self.root.join(publisher))
            .map(|path| path.is_dir())
            .unwrap_or(false)
    }

    /// The file one publisher's platform snapshot lives in:
    /// `<root>/<publisher>/<platform>.json`. The rule is the runner's own
    /// [`apex_autopilot::registry::snapshot_path`], the single writer of this
    /// layout, so the reader and the writer can never drift apart. Only call
    /// with a valid publisher name; [`Self::load`] re-checks it before
    /// building any path.
    pub fn path_for(&self, publisher: &str, platform: PlatformId) -> PathBuf {
        apex_autopilot::registry::snapshot_path(&self.root, publisher, platform.as_str())
    }

    /// The snapshot plus (when already computed for this file identity) its
    /// cached report, the resolved path and the stamp under which it may be
    /// cached. A cache miss reads and parses once and stores the snapshot with
    /// no report yet. Cached values are handed out as `Arc`s so a render never
    /// materializes a copy of the whole snapshot.
    fn read(&self, publisher: &str, platform: PlatformId) -> Result<CachedRead, ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let path = contained_in(&self.canonical_root, self.path_for(publisher, platform))?;
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(ApiError::NotFound),
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "stat snapshot {}: {error}",
                    path.display()
                )));
            }
        };
        let stamp = FileStamp::read(&metadata);
        if let Some(stamp) = stamp {
            if let Some((snapshot, report)) = self.cache.get(&path, stamp) {
                return Ok((snapshot, report, path, Some(stamp)));
            }
        }
        self.cache.reads.fetch_add(1, Ordering::Relaxed);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(ApiError::NotFound),
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "read snapshot {}: {error}",
                    path.display()
                )));
            }
        };
        self.cache.parses.fetch_add(1, Ordering::Relaxed);
        let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|error| {
            ApiError::Internal(format!("parse snapshot {}: {error}", path.display()))
        })?;
        if snapshot.platform != platform {
            return Err(ApiError::Internal(format!(
                "snapshot {} declares platform {} but was requested as {}",
                path.display(),
                snapshot.platform,
                platform
            )));
        }
        let snapshot = Arc::new(snapshot);
        if let Some(stamp) = stamp {
            self.cache.put(&path, stamp, &snapshot);
        }
        Ok((snapshot, None, path, stamp))
    }

    fn load(&self, publisher: &str, platform: PlatformId) -> Result<Snapshot, ApiError> {
        self.read(publisher, platform)
            .map(|(snapshot, _, _, _)| (*snapshot).clone())
    }

    fn load_arc(&self, publisher: &str, platform: PlatformId) -> Result<Arc<Snapshot>, ApiError> {
        self.read(publisher, platform)
            .map(|(snapshot, _, _, _)| snapshot)
    }

    fn load_report(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<(Snapshot, AuditReport), ApiError> {
        self.load_report_arc(publisher, platform)
            .map(|(snapshot, report)| ((*snapshot).clone(), (*report).clone()))
    }

    fn load_report_arc(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<(Arc<Snapshot>, Arc<AuditReport>), ApiError> {
        let (snapshot, report, path, stamp) = self.read(publisher, platform)?;
        if let Some(report) = report {
            return Ok((snapshot, report));
        }
        let report = Arc::new(apex_audit::run(&snapshot));
        self.cache.report_builds.fetch_add(1, Ordering::Relaxed);
        if let Some(stamp) = stamp {
            self.cache.put_report(&path, stamp, &report);
        }
        Ok((snapshot, report))
    }
}

#[async_trait]
impl AuditSource for JsonFileAuditSource {
    fn has_snapshot(&self, publisher: &str) -> bool {
        self.publisher_dir_exists(publisher)
    }

    async fn write(
        &self,
        publisher: &str,
        platform: PlatformId,
        snapshot: &Snapshot,
    ) -> Result<(), ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let path = self.path_for(publisher, platform);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ApiError::Internal(format!("snapshot directory: {error}")))?;
        }
        let bytes = serde_json::to_vec_pretty(snapshot)
            .map_err(|error| ApiError::Internal(format!("snapshot serialization: {error}")))?;
        // Atomic replace so a reader never sees a half-written snapshot.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes)
            .map_err(|error| ApiError::Internal(format!("snapshot write: {error}")))?;
        std::fs::rename(&tmp, &path)
            .map_err(|error| ApiError::Internal(format!("snapshot rename: {error}")))?;
        Ok(())
    }

    async fn snapshot(&self, publisher: &str, platform: PlatformId) -> Result<Snapshot, ApiError> {
        let publisher = publisher.to_string();
        let source = self.clone();
        tokio::task::spawn_blocking(move || source.load(&publisher, platform))
            .await
            .map_err(|error| ApiError::Internal(format!("snapshot read task failed: {error}")))?
    }

    async fn snapshot_arc(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<Arc<Snapshot>, ApiError> {
        let publisher = publisher.to_string();
        let source = self.clone();
        tokio::task::spawn_blocking(move || source.load_arc(&publisher, platform))
            .await
            .map_err(|error| ApiError::Internal(format!("snapshot read task failed: {error}")))?
    }

    async fn snapshot_report(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<(Snapshot, AuditReport), ApiError> {
        let publisher = publisher.to_string();
        let source = self.clone();
        tokio::task::spawn_blocking(move || source.load_report(&publisher, platform))
            .await
            .map_err(|error| ApiError::Internal(format!("snapshot read task failed: {error}")))?
    }

    async fn snapshot_report_arc(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<(Arc<Snapshot>, Arc<AuditReport>), ApiError> {
        let publisher = publisher.to_string();
        let source = self.clone();
        tokio::task::spawn_blocking(move || source.load_report_arc(&publisher, platform))
            .await
            .map_err(|error| ApiError::Internal(format!("snapshot read task failed: {error}")))?
    }
}

/// An in-process snapshot map keyed by `(publisher, platform)` (used by
/// embedders, tests and benchmarks).
#[derive(Debug, Clone, Default)]
pub struct StaticAuditSource {
    snapshots: HashMap<(String, PlatformId), Snapshot>,
}

impl StaticAuditSource {
    /// Build from a `(publisher, platform) -> snapshot` map.
    pub fn new(snapshots: HashMap<(String, PlatformId), Snapshot>) -> Self {
        Self { snapshots }
    }

    /// Add or replace one publisher's platform snapshot.
    pub fn insert(
        &mut self,
        publisher: impl Into<String>,
        platform: PlatformId,
        snapshot: Snapshot,
    ) {
        self.snapshots
            .insert((publisher.into(), platform), snapshot);
    }
}

#[async_trait]
impl AuditSource for StaticAuditSource {
    fn has_snapshot(&self, publisher: &str) -> bool {
        self.snapshots.keys().any(|(key, _)| key == publisher)
    }

    async fn snapshot(&self, publisher: &str, platform: PlatformId) -> Result<Snapshot, ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        self.snapshots
            .get(&(publisher.to_string(), platform))
            .cloned()
            .ok_or(ApiError::NotFound)
    }
}

/// A ledger file's cache identity.
///
/// The size and modification time are the original key. On Unix the ctime and
/// inode are included too: an unprivileged writer cannot set the ctime, so a
/// rewrite that restores the mtime (`touch -t`) still invalidates the entry.
/// The fallback keeps the original pair where those fields do not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileStamp {
    size: u64,
    modified: SystemTime,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
    #[cfg(unix)]
    ino: u64,
}

impl FileStamp {
    /// The stamp of `metadata`, or `None` when the filesystem exposes no
    /// modification time (the cache is then bypassed, exactly as before).
    pub(crate) fn read(metadata: &std::fs::Metadata) -> Option<Self> {
        let modified = metadata.modified().ok()?;
        let size = metadata.len();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Some(Self {
                size,
                modified,
                ctime: metadata.ctime(),
                ctime_nsec: metadata.ctime_nsec(),
                ino: metadata.ino(),
            })
        }
        #[cfg(not(unix))]
        {
            Some(Self { size, modified })
        }
    }
}

/// The identity of a JSONL log file: present with a hardened stamp, or absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogStamp {
    /// The file does not exist; repeated reads are an empty log until it is
    /// created.
    Missing,
    /// The file exists and can be keyed by this stamp.
    Present(FileStamp),
}

/// A parse-once cache for the runner's alert log and the Mollie collection log.
///
/// The key is the file's hardened [`FileStamp`] (size, mtime, ctime and inode
/// on Unix), so a changed file reparses once and an unchanged file is parsed
/// exactly once no matter how many requests read it. The parsed rows are
/// shared behind an `Arc<[T]>`, so a reader borrows the cache instead of
/// copying it. A missing file is cached as an empty log; creating it changes
/// the stamp to `Present` and invalidates the entry.
///
/// [`Self::get_or_parse`] is synchronous on purpose: the async readers call it
/// through [`tokio::task::spawn_blocking`], keeping a 16 MiB collection log
/// off the async worker. A separate parse lock serialises cache misses, so
/// concurrent readers of a changed file still parse it once.
#[derive(Debug)]
pub(crate) struct JsonlCache<T> {
    entries: Mutex<HashMap<PathBuf, CachedJsonl<T>>>,
    parse_lock: Mutex<()>,
    parses: AtomicU64,
    hits: AtomicU64,
}

#[derive(Debug, Clone)]
struct CachedJsonl<T> {
    stamp: LogStamp,
    rows: Arc<[T]>,
}

/// Observable counters of a [`JsonlCache`] (tests and the performance harness).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JsonlCacheStats {
    /// Full file parses (one per cache miss with a cacheable stamp).
    pub parses: u64,
    /// Cache hits served without a parse.
    pub hits: u64,
}

impl<T> Default for JsonlCache<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            parse_lock: Mutex::new(()),
            parses: AtomicU64::new(0),
            hits: AtomicU64::new(0),
        }
    }
}

impl<T> JsonlCache<T> {
    /// This cache's counters.
    pub(crate) fn stats(&self) -> JsonlCacheStats {
        JsonlCacheStats {
            parses: self.parses.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
        }
    }

    /// The stamp of `path`, or `None` when the filesystem exposes no
    /// modification time (the cache is then bypassed, exactly like the ledger
    /// cache). Files above `cap` are refused before any parse.
    fn stamp(path: &Path, cap: u64) -> Result<Option<LogStamp>, ApiError> {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(Some(LogStamp::Missing));
            }
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "stat {}: {error}",
                    path.display()
                )));
            }
        };
        ensure_within_cap(metadata.len(), cap)?;
        Ok(FileStamp::read(&metadata).map(LogStamp::Present))
    }

    fn lookup(&self, path: &Path, stamp: LogStamp) -> Option<Arc<[T]>> {
        let entries = self.entries.lock().ok()?;
        entries
            .get(path)
            .filter(|cached| cached.stamp == stamp)
            .map(|cached| Arc::clone(&cached.rows))
    }

    /// Parse `path` once per stamp and return the shared rows.
    ///
    /// `parse` runs only on a miss. Missing files are cached as an empty log,
    /// so repeated reads of an unconfigured log do not pay a read+parse; a
    /// file that later appears changes the stamp and reparses.
    pub(crate) fn get_or_parse<F>(
        &self,
        path: &Path,
        cap: u64,
        parse: F,
    ) -> Result<Arc<[T]>, ApiError>
    where
        F: FnOnce(&Path) -> Result<Vec<T>, ApiError>,
        T: Send + Sync + 'static,
    {
        let Some(stamp) = Self::stamp(path, cap)? else {
            // No modification time: bypass the cache and parse each time.
            self.parses.fetch_add(1, Ordering::Relaxed);
            return parse(path).map(|rows| Arc::from(rows.into_boxed_slice()));
        };
        if let Some(rows) = self.lookup(path, stamp) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(rows);
        }
        // Serialise misses so concurrent readers of one changed file parse it
        // once; the fast hit path above never takes this lock.
        let _guard = self
            .parse_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stamp) = Self::stamp(path, cap)? else {
            self.parses.fetch_add(1, Ordering::Relaxed);
            return parse(path).map(|rows| Arc::from(rows.into_boxed_slice()));
        };
        if let Some(rows) = self.lookup(path, stamp) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(rows);
        }
        let rows: Arc<[T]> = Arc::from(parse(path)?.into_boxed_slice());
        self.parses.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                path.to_path_buf(),
                CachedJsonl {
                    stamp,
                    rows: Arc::clone(&rows),
                },
            );
        }
        Ok(rows)
    }
}

/// One verified ledger held in memory: the records and the file identity they
/// were verified at. The records are shared behind an `Arc` so a paginated
/// reader can borrow the cached chain and clone only the requested page.
#[derive(Debug, Clone)]
struct CachedLedger {
    stamp: FileStamp,
    records: Arc<[LedgerRecord]>,
}

/// The in-process verified-record cache of [`FileLedgerSource`]. A read is
/// served from the cache only while the file's size, modification time and
/// (on Unix) ctime and inode are unchanged; a changed file (or one whose
/// mtime cannot be read) goes back through
/// [`apex_proof::Ledger::read_verified`].
#[derive(Debug, Default)]
struct LedgerCache {
    entries: Mutex<HashMap<PathBuf, CachedLedger>>,
    verifications: AtomicU64,
    materializations: AtomicU64,
    page_slices: AtomicU64,
}

/// Observable counters of [`FileLedgerSource`]'s verified-record cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LedgerCacheStats {
    /// Full chain verifications (one per cache miss).
    pub verifications: u64,
    /// Cached record clones handed to a reader.
    pub materializations: u64,
    /// Pages served by slicing the shared cached records without cloning the
    /// whole chain (the paginated console list).
    pub page_slices: u64,
}

impl LedgerCache {
    /// The whole cached chain, cloned into a fresh vector. This is the
    /// non-paginated reader (the JSON API and record detail); the paginated
    /// console list uses [`Self::get_shared`] instead.
    fn get_cloned(&self, path: &Path, stamp: FileStamp) -> Option<Vec<LedgerRecord>> {
        let entries = self.entries.lock().ok()?;
        entries.get(path).and_then(|cached| {
            (cached.stamp == stamp).then(|| {
                self.materializations.fetch_add(1, Ordering::Relaxed);
                cached.records.to_vec()
            })
        })
    }

    /// The cached chain behind its shared handle, with no record clone.
    fn get_shared(&self, path: &Path, stamp: FileStamp) -> Option<Arc<[LedgerRecord]>> {
        let entries = self.entries.lock().ok()?;
        entries
            .get(path)
            .and_then(|cached| (cached.stamp == stamp).then(|| Arc::clone(&cached.records)))
    }

    /// The cached verified record count, without materializing the records.
    fn count(&self, path: &Path, stamp: FileStamp) -> Option<u64> {
        let entries = self.entries.lock().ok()?;
        entries
            .get(path)
            .and_then(|cached| (cached.stamp == stamp).then_some(cached.records.len() as u64))
    }

    fn put(&self, path: &Path, stamp: FileStamp, records: &[LedgerRecord]) {
        self.put_shared(path, stamp, Arc::from(records.to_vec()));
    }

    fn put_shared(&self, path: &Path, stamp: FileStamp, records: Arc<[LedgerRecord]>) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(path.to_path_buf(), CachedLedger { stamp, records });
        }
    }
}

/// Reads and verifies `<root>/<publisher>.jsonl` against the pinned key.
///
/// The key map is the allow-list of onboarded publishers: an unknown (but
/// well-formed) publisher is a `404`, never an attempt at some other file.
/// Records are parsed from the one read that verification accepted, and that
/// verification is reused only while the file's size, modification time and
/// (on Unix) ctime and inode are unchanged: any change re-verifies before
/// serving, including a same-length rewrite whose mtime was restored.
#[derive(Debug, Clone)]
pub struct FileLedgerSource {
    keys: Arc<HashMap<String, VerifyingKey>>,
    root: PathBuf,
    canonical_root: PathBuf,
    cache: Arc<LedgerCache>,
}

impl FileLedgerSource {
    /// Build a source from the publisher -> public-key allow-list and the
    /// ledger root; the root is canonicalized here, once.
    pub fn new(root: impl Into<PathBuf>, keys: HashMap<String, VerifyingKey>) -> Self {
        let root = root.into();
        let canonical_root = canonical_root(&root);
        Self {
            keys: Arc::new(keys),
            root,
            canonical_root,
            cache: Arc::new(LedgerCache::default()),
        }
    }

    /// The directory ledgers are read from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file a publisher's ledger lives in (only call with a valid name).
    pub fn path_for(&self, publisher: &str) -> PathBuf {
        self.root.join(format!("{publisher}.jsonl"))
    }

    /// This source's verified-record cache counters (tests and the
    /// performance harness).
    pub fn cache_stats(&self) -> LedgerCacheStats {
        LedgerCacheStats {
            verifications: self.cache.verifications.load(Ordering::Relaxed),
            materializations: self.cache.materializations.load(Ordering::Relaxed),
            page_slices: self.cache.page_slices.load(Ordering::Relaxed),
        }
    }

    /// Validate the publisher and resolve its ledger path, metadata and file
    /// stamp. A missing file is [`ApiError::NotFound`]; a file above the read
    /// cap is refused before any bytes are read.
    fn resolve(&self, publisher: &str) -> Result<(PathBuf, Option<FileStamp>), ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let _expected_key = self.keys.get(publisher).ok_or(ApiError::NotFound)?;
        let path = contained_in(&self.canonical_root, self.path_for(publisher))?;
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(ApiError::NotFound),
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "stat ledger {}: {error}",
                    path.display()
                )));
            }
        };
        ensure_within_cap(metadata.len(), MAX_LEDGER_BYTES)?;
        Ok((path, FileStamp::read(&metadata)))
    }

    fn load(&self, publisher: &str) -> Result<Vec<LedgerRecord>, ApiError> {
        let (path, stamp) = self.resolve(publisher)?;
        if let Some(stamp) = stamp {
            if let Some(records) = self.cache.get_cloned(&path, stamp) {
                return Ok(records);
            }
        }
        let expected_key = self.keys.get(publisher).ok_or(ApiError::NotFound)?;
        self.cache.verifications.fetch_add(1, Ordering::Relaxed);
        let records =
            Ledger::read_verified(&path, expected_key).map_err(|error| match ledger_error_line(
                &error,
            ) {
                Some(line) => ApiError::LedgerUnverified { line },
                None => ApiError::Internal(format!("verify ledger {}: {error}", path.display())),
            })?;
        if let Some(stamp) = stamp {
            self.cache.put(&path, stamp, &records);
        }
        Ok(records)
    }

    /// The whole verified chain behind its shared cache handle. A cache hit
    /// clones the `Arc`, not the records; a miss verifies once and caches the
    /// shared handle.
    fn load_shared(&self, publisher: &str) -> Result<Arc<[LedgerRecord]>, ApiError> {
        let (path, stamp) = self.resolve(publisher)?;
        if let Some(stamp) = stamp {
            if let Some(records) = self.cache.get_shared(&path, stamp) {
                return Ok(records);
            }
        }
        let expected_key = self.keys.get(publisher).ok_or(ApiError::NotFound)?;
        self.cache.verifications.fetch_add(1, Ordering::Relaxed);
        let records =
            Ledger::read_verified(&path, expected_key).map_err(|error| match ledger_error_line(
                &error,
            ) {
                Some(line) => ApiError::LedgerUnverified { line },
                None => ApiError::Internal(format!("verify ledger {}: {error}", path.display())),
            })?;
        let records: Arc<[LedgerRecord]> = Arc::from(records.into_boxed_slice());
        if let Some(stamp) = stamp {
            self.cache.put_shared(&path, stamp, Arc::clone(&records));
        }
        Ok(records)
    }

    /// One page of the shared verified chain. Only the requested range is
    /// cloned; the full vector never is.
    fn load_page(
        &self,
        publisher: &str,
        offset: usize,
        limit: usize,
    ) -> Result<LedgerPage, ApiError> {
        let records = self.load_shared(publisher)?;
        self.cache.page_slices.fetch_add(1, Ordering::Relaxed);
        let total = records.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        Ok(LedgerPage {
            total: total as u64,
            records: records[start..end].to_vec(),
        })
    }

    /// The verified record count without materializing cached records.
    ///
    /// A cache hit answers from the entry's length alone; a miss runs the
    /// same verified read as [`Self::load`] (warming the cache) and counts
    /// what that read returned, so the count is always a count of records
    /// that passed verification.
    fn load_count_cached(&self, publisher: &str) -> Result<Option<u64>, ApiError> {
        let (path, stamp) = self.resolve(publisher)?;
        if let Some(stamp) = stamp {
            if let Some(count) = self.cache.count(&path, stamp) {
                return Ok(Some(count));
            }
        }
        Ok(None)
    }
}

#[async_trait]
impl LedgerSource for FileLedgerSource {
    async fn records(&self, publisher: &str) -> Result<Vec<LedgerRecord>, ApiError> {
        let publisher = publisher.to_string();
        let source = self.clone();
        tokio::task::spawn_blocking(move || source.load(&publisher))
            .await
            .map_err(|error| ApiError::Internal(format!("ledger read task failed: {error}")))?
    }

    async fn records_shared(&self, publisher: &str) -> Result<Arc<[LedgerRecord]>, ApiError> {
        let publisher = publisher.to_string();
        let source = self.clone();
        tokio::task::spawn_blocking(move || source.load_shared(&publisher))
            .await
            .map_err(|error| ApiError::Internal(format!("ledger read task failed: {error}")))?
    }

    async fn records_page(
        &self,
        publisher: &str,
        offset: usize,
        limit: usize,
    ) -> Result<LedgerPage, ApiError> {
        let publisher = publisher.to_string();
        let source = self.clone();
        tokio::task::spawn_blocking(move || source.load_page(&publisher, offset, limit))
            .await
            .map_err(|error| ApiError::Internal(format!("ledger read task failed: {error}")))?
    }

    async fn record_count(&self, publisher: &str) -> Result<u64, ApiError> {
        // The cache-hit path is a stat plus a cache lookup; only a miss pays
        // for the blocking verified read (which also warms the cache).
        if let Some(count) = self.load_count_cached(publisher)? {
            return Ok(count);
        }
        let publisher = publisher.to_string();
        let source = self.clone();
        tokio::task::spawn_blocking(move || {
            source.load(&publisher).map(|records| records.len() as u64)
        })
        .await
        .map_err(|error| ApiError::Internal(format!("ledger read task failed: {error}")))?
    }

    fn publishers(&self) -> Vec<String> {
        let mut names: Vec<String> = self.keys.keys().cloned().collect();
        names.sort();
        names
    }

    fn has_publishers(&self) -> bool {
        !self.keys.is_empty()
    }
}

/// An in-process ledger map (used by embedders, tests and benchmarks).
#[derive(Debug, Clone, Default)]
pub struct StaticLedgerSource {
    ledgers: HashMap<String, Vec<LedgerRecord>>,
}

impl StaticLedgerSource {
    /// Build from a publisher -> records map.
    pub fn new(ledgers: HashMap<String, Vec<LedgerRecord>>) -> Self {
        Self { ledgers }
    }

    /// Add or replace one publisher's records.
    pub fn insert(&mut self, publisher: impl Into<String>, records: Vec<LedgerRecord>) {
        self.ledgers.insert(publisher.into(), records);
    }
}

#[async_trait]
impl LedgerSource for StaticLedgerSource {
    async fn records(&self, publisher: &str) -> Result<Vec<LedgerRecord>, ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        self.ledgers
            .get(publisher)
            .cloned()
            .ok_or(ApiError::NotFound)
    }

    fn publishers(&self) -> Vec<String> {
        let mut names: Vec<String> = self.ledgers.keys().cloned().collect();
        names.sort();
        names
    }

    fn has_publishers(&self) -> bool {
        !self.ledgers.is_empty()
    }
}

/// The 1-based line of the first offending record, for every `LedgerError`
/// variant that names one. The remaining variants are ledger-lifecycle errors
/// (I/O, append-target rules) that cannot occur on the read path.
fn ledger_error_line(error: &LedgerError) -> Option<usize> {
    match error {
        LedgerError::Malformed { line, .. }
        | LedgerError::HashMismatch { line }
        | LedgerError::ChainBreak { line }
        | LedgerError::BadSignature { line }
        | LedgerError::WrongPubkey { line }
        | LedgerError::SequenceGap { line, .. }
        | LedgerError::Duplicate { line, .. }
        | LedgerError::Genesis { line }
        | LedgerError::Truncated { line }
        | LedgerError::StateDiverged { line }
        | LedgerError::InvalidBillingProof { line, .. } => Some(*line),
        LedgerError::Io(_)
        | LedgerError::UnknownRollbackTarget { .. }
        | LedgerError::RollbackOfRollback { .. }
        | LedgerError::RollbackTargetMismatch { .. }
        | LedgerError::DuplicateRollback { .. }
        | LedgerError::Serialize(_)
        | LedgerError::Validation(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use sha2::Digest;

    use super::*;

    #[test]
    fn platform_parsing_and_source_paths_are_pinned() {
        assert_eq!(
            parse_platform("applovin_max"),
            Some(PlatformId::AppLovinMax)
        );
        assert_eq!(parse_platform("levelplay"), Some(PlatformId::LevelPlay));
        assert_eq!(parse_platform("admob"), Some(PlatformId::Admob));
        assert_eq!(parse_platform("tiktok"), None);

        let audit = JsonFileAuditSource::new("/tmp/apex");
        assert_eq!(audit.root(), std::path::Path::new("/tmp/apex"));
        assert!(audit
            .path_for("pub1", PlatformId::Admob)
            .ends_with("pub1/admob.json"));
        assert_eq!(
            audit.path_for("pub1", PlatformId::Admob),
            std::path::Path::new("/tmp/apex/pub1/admob.json")
        );

        let mut keys = HashMap::new();
        keys.insert(
            "pub".to_string(),
            apex_proof::Keypair::from_bytes(&[1u8; 32]).verifying_key(),
        );
        let ledger = FileLedgerSource::new("/tmp/apex-ledgers", keys);
        assert!(ledger.path_for("pub").ends_with("pub.jsonl"));
    }

    #[tokio::test]
    async fn the_file_audit_source_writes_and_reads_back_a_snapshot() {
        use apex_audit::{AppAdsTxt, Snapshot};
        let dir = std::env::temp_dir().join(format!("apex-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let snapshot = Snapshot {
            platform: PlatformId::AppLovinMax,
            fetched_at_unix_ms: 7,
            ad_units: vec![],
            waterfall: vec![],
            mediation_groups: vec![],
            bidder_accounts: vec![],
            country_revenue: vec![],
            daily_revenue: vec![],
            app_ads_txt: AppAdsTxt {
                present: false,
                lines: vec![],
                expected_networks: vec![],
            },
            platform_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_reported: false,
        };
        std::fs::create_dir_all(&dir).unwrap();
        let source = JsonFileAuditSource::new(&dir);
        assert!(
            source
                .snapshot("pub1", PlatformId::AppLovinMax)
                .await
                .is_err(),
            "nothing yet"
        );
        source
            .write("pub1", PlatformId::AppLovinMax, &snapshot)
            .await
            .expect("write");
        assert!(
            source.path_for("pub1", PlatformId::AppLovinMax).exists(),
            "written"
        );
        let read = source
            .snapshot("pub1", PlatformId::AppLovinMax)
            .await
            .expect("read");
        assert_eq!(read.fetched_at_unix_ms, 7);
        // A second publisher has no file even though one exists for pub1:
        // snapshots never leak across publishers.
        assert!(matches!(
            source.snapshot("pub2", PlatformId::AppLovinMax).await,
            Err(ApiError::NotFound)
        ));
        // A hostile publisher name is refused before any path is built.
        assert!(matches!(
            source.snapshot("../etc", PlatformId::AppLovinMax).await,
            Err(ApiError::BadRequest("invalid_publisher"))
        ));
        assert!(matches!(
            source
                .write("a/b", PlatformId::AppLovinMax, &snapshot)
                .await,
            Err(ApiError::BadRequest("invalid_publisher"))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn empty_snapshot(platform: PlatformId, fetched_at_unix_ms: i64) -> Snapshot {
        Snapshot {
            platform,
            fetched_at_unix_ms,
            ad_units: vec![],
            waterfall: vec![],
            mediation_groups: vec![],
            bidder_accounts: vec![],
            country_revenue: vec![],
            daily_revenue: vec![],
            app_ads_txt: apex_audit::AppAdsTxt {
                present: false,
                lines: vec![],
                expected_networks: vec![],
            },
            platform_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_reported: false,
        }
    }

    /// A console page loads one publisher's snapshot on every request. The
    /// unchanged file must be read and parsed once, then answered from the
    /// cache (including across clones of the source, which share the cache).
    #[tokio::test]
    async fn snapshots_are_parsed_once_per_unchanged_file() {
        let dir = std::env::temp_dir().join(format!("apex-snapshot-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let source = JsonFileAuditSource::new(&dir);
        source
            .write(
                "pub1",
                PlatformId::Admob,
                &empty_snapshot(PlatformId::Admob, 7),
            )
            .await
            .expect("write");

        let first = source
            .snapshot("pub1", PlatformId::Admob)
            .await
            .expect("first read");
        assert_eq!(first.fetched_at_unix_ms, 7);
        let stats = source.cache_stats();
        assert_eq!(stats.reads, 1, "the first read reads and parses");
        assert_eq!(stats.parses, 1);
        assert_eq!(stats.hits, 0);

        // Repeated loads (and a clone, as every request handler holds one) are
        // cache hits: no read, no parse.
        let clone = source.clone();
        for _ in 0..3 {
            let again = clone
                .snapshot("pub1", PlatformId::Admob)
                .await
                .expect("cached read");
            assert_eq!(again.fetched_at_unix_ms, 7);
        }
        let stats = source.cache_stats();
        assert_eq!(stats.reads, 1, "no file read on a cache hit");
        assert_eq!(stats.parses, 1, "no JSON parse on a cache hit");
        assert_eq!(stats.hits, 3);

        // A rewrite changes the hardened stamp, so the next read parses again.
        source
            .write(
                "pub1",
                PlatformId::Admob,
                &empty_snapshot(PlatformId::Admob, 8),
            )
            .await
            .expect("rewrite");
        let changed = source
            .snapshot("pub1", PlatformId::Admob)
            .await
            .expect("changed read");
        assert_eq!(changed.fetched_at_unix_ms, 8);
        let stats = source.cache_stats();
        assert_eq!(stats.reads, 2);
        assert_eq!(stats.parses, 2);
        assert_eq!(stats.hits, 3, "only a stamp change re-parses");

        // A missing snapshot and an invalid publisher never touch the cache.
        assert!(matches!(
            source.snapshot("pub2", PlatformId::Admob).await,
            Err(ApiError::NotFound)
        ));
        assert!(matches!(
            source.snapshot("../etc", PlatformId::Admob).await,
            Err(ApiError::BadRequest("invalid_publisher"))
        ));
        let unchanged = source.cache_stats();
        assert_eq!(unchanged.reads, 2);
        assert_eq!(unchanged.parses, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shared-handle reads are byte-for-byte equivalent to the by-value
    /// reads and hand out the same cached value, so the console pages that
    /// borrow (`snapshot_arc`/`snapshot_report_arc`) render exactly what the
    /// cloning path used to render without paying a whole-snapshot copy.
    #[tokio::test]
    async fn arc_reads_share_the_cached_snapshot_and_report() {
        let dir = std::env::temp_dir().join(format!("apex-arc-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let source = JsonFileAuditSource::new(&dir);
        source
            .write(
                "pub1",
                PlatformId::Admob,
                &empty_snapshot(PlatformId::Admob, 7),
            )
            .await
            .expect("write");

        let (snapshot_arc, report_arc) = source
            .snapshot_report_arc("pub1", PlatformId::Admob)
            .await
            .expect("arc report");
        let (snapshot_value, report_value) = source
            .snapshot_report("pub1", PlatformId::Admob)
            .await
            .expect("value report");
        assert_eq!((*snapshot_arc).clone(), snapshot_value);
        assert_eq!((*report_arc).clone(), report_value);

        // The second shared read is the same allocation, not a copy.
        let (snapshot_again, report_again) = source
            .snapshot_report_arc("pub1", PlatformId::Admob)
            .await
            .expect("cached arc report");
        assert!(Arc::ptr_eq(&snapshot_arc, &snapshot_again));
        assert!(Arc::ptr_eq(&report_arc, &report_again));
        let plain = source
            .snapshot_arc("pub1", PlatformId::Admob)
            .await
            .expect("cached arc snapshot");
        assert!(Arc::ptr_eq(&snapshot_arc, &plain));
        let stats = source.cache_stats();
        assert_eq!(stats.parses, 1, "one parse for the whole sequence");
        assert_eq!(stats.report_builds, 1, "one report build");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The console renders the report on the overview, report, finding and
    /// CSV pages. The engine runs once per file identity, not once per page.
    #[tokio::test]
    async fn audit_reports_are_computed_once_per_unchanged_snapshot() {
        let dir = std::env::temp_dir().join(format!("apex-report-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let source = JsonFileAuditSource::new(&dir);
        source
            .write(
                "pub1",
                PlatformId::Admob,
                &empty_snapshot(PlatformId::Admob, 7),
            )
            .await
            .expect("write");

        let (snapshot, report) = source
            .snapshot_report("pub1", PlatformId::Admob)
            .await
            .expect("first report");
        assert_eq!(snapshot.fetched_at_unix_ms, 7);
        assert_eq!(source.cache_stats().parses, 1);
        assert_eq!(source.cache_stats().report_builds, 1);

        // A second page (and a clone, as the handlers hold) reuses both the
        // parsed snapshot and the computed report.
        for _ in 0..3 {
            let (_, again) = source
                .clone()
                .snapshot_report("pub1", PlatformId::Admob)
                .await
                .expect("cached report");
            assert_eq!(again.findings.len(), report.findings.len());
        }
        let stats = source.cache_stats();
        assert_eq!(stats.parses, 1, "no re-parse for the report");
        assert_eq!(stats.report_builds, 1, "no re-run of the audit engine");
        assert_eq!(stats.hits, 3);

        // A plain snapshot read shares the same entry.
        source
            .snapshot("pub1", PlatformId::Admob)
            .await
            .expect("snapshot");
        assert_eq!(source.cache_stats().hits, 4);
        assert_eq!(source.cache_stats().report_builds, 1);

        // A changed file invalidates both.
        source
            .write(
                "pub1",
                PlatformId::Admob,
                &empty_snapshot(PlatformId::Admob, 8),
            )
            .await
            .expect("rewrite");
        source
            .snapshot_report("pub1", PlatformId::Admob)
            .await
            .expect("changed report");
        let stats = source.cache_stats();
        assert_eq!(stats.parses, 2, "the changed file re-parses");
        assert_eq!(stats.report_builds, 2, "and the report is rebuilt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A same-length rewrite whose mtime is restored must still re-parse:
    /// the Unix ctime is part of the cache key. A size+mtime pair cache would
    /// serve the stale snapshot here.
    #[tokio::test]
    async fn snapshot_cache_detects_a_restored_mtime_rewrite() {
        let dir = std::env::temp_dir().join(format!("apex-snapshot-ctime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let source = JsonFileAuditSource::new(&dir);
        let first = empty_snapshot(PlatformId::Admob, 7);
        let second = empty_snapshot(PlatformId::Admob, 8);
        source
            .write("pub1", PlatformId::Admob, &first)
            .await
            .expect("write");
        source
            .snapshot("pub1", PlatformId::Admob)
            .await
            .expect("first read");

        let path = source.path_for("pub1", PlatformId::Admob);
        let metadata = std::fs::metadata(&path).expect("stat");
        let modified = metadata.modified().expect("mtime");
        let replacement = serde_json::to_vec_pretty(&second).expect("serialize");
        assert_eq!(
            metadata.len(),
            replacement.len() as u64,
            "the rewrite keeps the length"
        );
        std::fs::write(&path, &replacement).expect("rewrite");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(modified)
            .expect("restore mtime");

        let rewritten = source
            .snapshot("pub1", PlatformId::Admob)
            .await
            .expect("second read");
        assert_eq!(rewritten.fetched_at_unix_ms, 8, "the new bytes are served");
        assert_eq!(source.cache_stats().parses, 2, "the rewrite re-parses");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_static_ledger_source_answers_known_and_unknown_publishers() {
        let mut source = StaticLedgerSource::new(HashMap::new());
        assert!(matches!(
            source.records("pub").await,
            Err(ApiError::NotFound)
        ));
        source.insert("pub", vec![]);
        assert_eq!(source.records("pub").await.expect("records").len(), 0);
        assert!(matches!(
            source.records("a/b").await,
            Err(ApiError::BadRequest("invalid_publisher"))
        ));
    }

    #[test]
    fn random_publisher_strings_are_classified_without_panicking() {
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..500 {
            let len = (next() % 90) as usize;
            let value: String = (0..len).map(|_| (next() % 95 + 32) as u8 as char).collect();
            let valid = is_valid_publisher(&value);
            assert!(!valid || value.len() <= 64);
            assert!(
                !valid
                    || value
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            );
        }
    }

    #[test]
    fn publisher_charset_boundaries_are_pinned() {
        for valid in ["a", "A9", "publisher_1", "p-2", &"a".repeat(64)] {
            assert!(is_valid_publisher(valid), "{valid:?} must be valid");
        }
        for invalid in [
            "",
            "..",
            "a/../b",
            "a/b",
            "a\\b",
            "a b",
            "a.b",
            "\u{00e9}",
            "\u{4e16}",
            "<script>",
            "pub;rm",
            &"a".repeat(65),
        ] {
            assert!(!is_valid_publisher(invalid), "{invalid:?} must be refused");
        }
        // Exactly 64 multibyte characters are 256 bytes and must be refused
        // (the limit is over the name's bytes).
        assert!(!is_valid_publisher(&"\u{4e16}".repeat(64)));
    }

    #[test]
    fn platform_parsing_is_strict_over_the_canonical_names() {
        assert_eq!(
            parse_platform("applovin_max"),
            Some(PlatformId::AppLovinMax)
        );
        assert_eq!(parse_platform("levelplay"), Some(PlatformId::LevelPlay));
        assert_eq!(parse_platform("admob"), Some(PlatformId::Admob));
        for refused in ["", "Admob", "applovin", "max", " meta", "admob ", "tiktok"] {
            assert_eq!(parse_platform(refused), None, "{refused:?}");
        }
        // The console labels and platform names stay aligned.
        for (name, _label) in PLATFORM_NAMES {
            assert!(parse_platform(name).is_some(), "{name}");
        }
    }

    #[test]
    fn the_platform_vocabulary_feeds_both_parse_directions() {
        // Every spelling normalizes to the table's canonical name, and every
        // canonical name parses to a platform id: the free-audit choices and
        // the shared parser can never drift apart silently.
        for (spelling, canonical) in PLATFORM_SPELLINGS {
            assert_eq!(canonical_platform_name(spelling), Some(canonical));
            assert_eq!(
                parse_platform(canonical_platform_name(spelling).expect("spelling")),
                parse_platform(canonical),
                "{spelling} must normalize into the parseable set"
            );
        }
        assert_eq!(
            canonical_platform_name("  AppLovin_MAX  "),
            Some("applovin_max")
        );
        assert_eq!(canonical_platform_name("nope"), None);
        // The lead surfaces' wire values are accepted spellings of this one
        // vocabulary and map to valid platform ids after normalization.
        for (wire, _label) in crate::leads::PLATFORM_CHOICES {
            assert_eq!(crate::leads::canonical_platform(wire), wire);
            let canonical = canonical_platform_name(wire).expect("lead wire value in vocabulary");
            assert!(
                parse_platform(canonical).is_some(),
                "lead wire value {wire}"
            );
        }
    }

    #[test]
    fn ledger_error_lines_are_extracted_for_every_tamper_class() {
        assert_eq!(
            ledger_error_line(&LedgerError::HashMismatch { line: 4 }),
            Some(4)
        );
        assert_eq!(
            ledger_error_line(&LedgerError::BadSignature { line: 2 }),
            Some(2)
        );
        assert_eq!(
            ledger_error_line(&LedgerError::Truncated { line: 9 }),
            Some(9)
        );
        assert_eq!(
            ledger_error_line(&LedgerError::SequenceGap {
                line: 3,
                expected: 2,
                found: 7,
            }),
            Some(3)
        );
        assert_eq!(
            ledger_error_line(&LedgerError::Validation("x".into())),
            None
        );
    }

    #[test]
    fn ledger_size_cap_boundaries_are_pinned() {
        // Synthetic sizes: the guard is pure, so no file is needed.
        assert!(ensure_within_cap(0, MAX_LEDGER_BYTES).is_ok());
        assert!(ensure_within_cap(MAX_LEDGER_BYTES - 1, MAX_LEDGER_BYTES).is_ok());
        assert!(ensure_within_cap(MAX_LEDGER_BYTES, MAX_LEDGER_BYTES).is_ok());
        let error = ensure_within_cap(MAX_LEDGER_BYTES + 1, MAX_LEDGER_BYTES)
            .expect_err("one byte above the cap is refused");
        assert!(matches!(error, ApiError::Internal(_)), "{error}");
        assert!(error.to_string().contains("read cap"), "{error}");
        assert_eq!(MAX_LEDGER_BYTES, 32 * 1024 * 1024);
    }

    #[test]
    fn containment_requires_the_canonical_root_prefix() {
        let root = std::env::temp_dir().join(format!(
            "apex-api-contained-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).expect("create root");
        let inside = root.join("pub1.jsonl");
        std::fs::write(&inside, b"").expect("write inside");
        let canonical = canonical_root(&root);
        assert_eq!(
            contained_in(&canonical, inside).expect("inside"),
            canonical.join("pub1.jsonl")
        );
        // A sibling path that merely shares a textual prefix must not pass.
        let sibling = root.with_file_name(format!(
            "{}-sibling",
            root.file_name()
                .and_then(|name| name.to_str())
                .expect("root name")
        ));
        std::fs::create_dir_all(&sibling).expect("create sibling");
        let escaped = sibling.join("pub1.jsonl");
        std::fs::write(&escaped, b"").expect("write sibling");
        assert!(matches!(
            contained_in(&canonical, escaped),
            Err(ApiError::NotFound)
        ));
        assert!(matches!(
            contained_in(&canonical, root.join("missing.jsonl")),
            Err(ApiError::NotFound)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
    }

    #[tokio::test]
    async fn static_ledger_source_refuses_invalid_publishers_defensively() {
        let source = StaticLedgerSource::default();
        let error = source.records("../etc/passwd").await.expect_err("refused");
        assert!(matches!(error, ApiError::BadRequest("invalid_publisher")));
    }

    #[tokio::test]
    async fn static_sources_report_missing_entries_as_not_found() {
        let audit = StaticAuditSource::default();
        assert!(matches!(
            audit.snapshot("pub1", PlatformId::Admob).await,
            Err(ApiError::NotFound)
        ));
        assert!(matches!(
            audit.snapshot("a/b", PlatformId::Admob).await,
            Err(ApiError::BadRequest("invalid_publisher"))
        ));
        let ledger = StaticLedgerSource::default();
        assert!(matches!(
            ledger.records("known_name").await,
            Err(ApiError::NotFound)
        ));
    }

    fn lift_draft(payload: &str) -> LedgerRecord {
        let digest = hex::encode(sha2::Sha256::digest(payload.as_bytes()));
        LedgerRecord::lift_record(1_792_000_000_000, &digest, payload)
    }

    #[tokio::test]
    async fn verified_ledgers_are_cached_until_the_file_changes() {
        let dir = std::env::temp_dir().join(format!("apex-ledger-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("pub1.jsonl");
        let seed = [5u8; 32];
        {
            let mut ledger =
                apex_proof::Ledger::open(&path, apex_proof::Keypair::from_bytes(&seed))
                    .expect("open ledger");
            ledger
                .append(lift_draft(r#"{"type":"dataPull","day":"2026-10-01"}"#))
                .expect("append");
        }
        let mut keys = HashMap::new();
        keys.insert(
            "pub1".to_string(),
            apex_proof::Keypair::from_bytes(&seed).verifying_key(),
        );
        let source = FileLedgerSource::new(&dir, keys);

        // The first read verifies; the second is a cache hit.
        let first = source.records("pub1").await.expect("first read");
        assert_eq!(first.len(), 1);
        assert_eq!(source.cache.verifications.load(Ordering::Relaxed), 1);
        let second = source.records("pub1").await.expect("second read");
        assert_eq!(second, first);
        assert_eq!(
            source.cache.verifications.load(Ordering::Relaxed),
            1,
            "the repeated read is served from the cache"
        );

        // A modified file (another appended record) re-verifies.
        {
            let mut ledger =
                apex_proof::Ledger::open(&path, apex_proof::Keypair::from_bytes(&seed))
                    .expect("reopen ledger");
            ledger
                .append(lift_draft(r#"{"type":"dataPull","day":"2026-10-02"}"#))
                .expect("append");
        }
        let third = source.records("pub1").await.expect("third read");
        assert_eq!(third.len(), 2);
        assert_eq!(
            source.cache.verifications.load(Ordering::Relaxed),
            2,
            "a changed file re-verifies"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A same-length rewrite whose mtime is restored (`touch -t`) must still
    /// be detected and re-verified: the Unix ctime is part of the cache key,
    /// and an unprivileged writer cannot restore it. A pair-keyed cache
    /// serves the stale records here; this test fails on it.
    #[tokio::test]
    async fn restored_mtime_same_length_rewrite_is_detected() {
        let dir = std::env::temp_dir().join(format!("apex-ledger-ctime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let seed = [9u8; 32];
        let path = dir.join("pub1.jsonl");
        {
            let mut ledger =
                apex_proof::Ledger::open(&path, apex_proof::Keypair::from_bytes(&seed))
                    .expect("open ledger");
            ledger
                .append(lift_draft(r#"{"type":"dataPull","day":"2026-10-01"}"#))
                .expect("append");
        }
        // A second, equally long valid ledger signed by the same key.
        let replacement = dir.join("replacement.jsonl");
        {
            let mut ledger =
                apex_proof::Ledger::open(&replacement, apex_proof::Keypair::from_bytes(&seed))
                    .expect("open replacement");
            ledger
                .append(lift_draft(r#"{"type":"dataPull","day":"2026-10-02"}"#))
                .expect("append");
        }
        let replacement_bytes = std::fs::read_to_string(&replacement).expect("replacement bytes");
        let mut keys = HashMap::new();
        keys.insert(
            "pub1".to_string(),
            apex_proof::Keypair::from_bytes(&seed).verifying_key(),
        );
        let source = FileLedgerSource::new(&dir, keys);

        let original = source.records("pub1").await.expect("first read");
        assert_eq!(source.cache.verifications.load(Ordering::Relaxed), 1);
        let original_json = serde_json::to_string(&original[0]).expect("record json");
        assert!(original_json.contains("2026-10-01"), "{original_json}");

        let metadata = std::fs::metadata(&path).expect("stat");
        let modified = metadata.modified().expect("mtime");
        assert_eq!(
            metadata.len(),
            replacement_bytes.len() as u64,
            "the rewrites keep the length"
        );
        std::fs::write(&path, &replacement_bytes).expect("rewrite");
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

        let rewritten = source.records("pub1").await.expect("second read");
        assert_eq!(
            source.cache.verifications.load(Ordering::Relaxed),
            2,
            "the restored-mtime rewrite re-verifies"
        );
        let rewritten_json = serde_json::to_string(&rewritten[0]).expect("record json");
        assert!(
            rewritten_json.contains("2026-10-02"),
            "the new bytes are served: {rewritten_json}"
        );
        assert_ne!(rewritten, original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A paginated read borrows the cached chain and clones only the page.
    /// The [materializations](LedgerCacheStats::materializations) counter
    /// stays at zero while [page_slices](LedgerCacheStats::page_slices)
    /// advances, which is the seam proving the console list never clones the
    /// full vector.
    #[tokio::test]
    async fn paginated_reads_slice_the_shared_cache_without_materializing_it() {
        let dir = std::env::temp_dir().join(format!("apex-ledger-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("pub1.jsonl");
        let seed = [11u8; 32];
        {
            let mut ledger =
                apex_proof::Ledger::open(&path, apex_proof::Keypair::from_bytes(&seed))
                    .expect("open ledger");
            for index in 0..250 {
                ledger
                    .append(lift_draft(&format!(r#"{{"type":"dataPull","n":{index}}}"#)))
                    .expect("append");
            }
        }
        let mut keys = HashMap::new();
        keys.insert(
            "pub1".to_string(),
            apex_proof::Keypair::from_bytes(&seed).verifying_key(),
        );
        let source = FileLedgerSource::new(&dir, keys);

        let first = source.records_page("pub1", 0, 100).await.expect("page 1");
        assert_eq!(first.total, 250);
        assert_eq!(first.records.len(), 100);
        assert_eq!(first.records[0].seq(), 0);
        assert_eq!(first.records[99].seq(), 99);
        let stats = source.cache_stats();
        assert_eq!(stats.verifications, 1, "one verified read warms the page");
        assert_eq!(
            stats.materializations, 0,
            "a page never clones the whole chain"
        );
        assert_eq!(stats.page_slices, 1);

        // The read-only shared reader borrows the same cached chain. It does
        // not clone the records (materializations stays at zero).
        let shared = source.records_shared("pub1").await.expect("shared chain");
        assert_eq!(shared.len(), 250);
        assert_eq!(source.cache_stats().materializations, 0);
        assert_eq!(source.cache_stats().verifications, 1);

        let remainder = source.records_page("pub1", 200, 100).await.expect("page 3");
        assert_eq!(remainder.total, 250);
        assert_eq!(
            remainder.records.len(),
            50,
            "the final page is the remainder"
        );
        assert_eq!(remainder.records[0].seq(), 200);
        assert_eq!(remainder.records[49].seq(), 249);
        let stats = source.cache_stats();
        assert_eq!(stats.verifications, 1, "the cached chain is reused");
        assert_eq!(stats.materializations, 0);
        assert_eq!(stats.page_slices, 2);

        let beyond = source
            .records_page("pub1", 1_000, 100)
            .await
            .expect("past the end");
        assert_eq!(beyond.total, 250);
        assert!(beyond.records.is_empty(), "past the end is an empty page");

        // The non-paginated reader still returns the whole chain and is what
        // the materializations counter measures; it agrees byte-for-byte with
        // the shared chain the read-only pages borrow.
        let owned = source.records("pub1").await.expect("all");
        assert_eq!(owned.len(), 250);
        assert_eq!(&*shared, owned.as_slice());
        assert_eq!(source.cache_stats().materializations, 1);

        assert!(matches!(
            source.records_page("a/b", 0, 100).await,
            Err(ApiError::BadRequest("invalid_publisher"))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The overview totals read every publisher every request. The count path
    /// must answer from the verified cache without re-verifying and without
    /// materializing (cloning) records, until a file actually changes. The
    /// pre-fix overview loop (`records()` per publisher, summed lengths)
    /// materialized every record on every request; this test guards that the
    /// count path never regresses to that shape (implementing `record_count`
    /// as `records().len()` makes the materialization assertion fail).
    #[tokio::test]
    async fn record_count_avoids_verification_and_materialization_until_a_file_changes() {
        let dir = std::env::temp_dir().join(format!("apex-ledger-count-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");

        let mut keys = HashMap::new();
        let mut ledger_paths = Vec::new();
        for (publisher, records) in [("pub1", 3usize), ("pub2", 2)] {
            let seed = [publisher.len() as u8 + 10; 32];
            let path = dir.join(format!("{publisher}.jsonl"));
            {
                let mut ledger =
                    apex_proof::Ledger::open(&path, apex_proof::Keypair::from_bytes(&seed))
                        .expect("open ledger");
                for i in 0..records {
                    ledger
                        .append(lift_draft(&format!(
                            r#"{{"type":"dataPull","day":"2026-10-{:02}"}}"#,
                            i + 1
                        )))
                        .expect("append");
                }
            }
            keys.insert(
                publisher.to_string(),
                apex_proof::Keypair::from_bytes(&seed).verifying_key(),
            );
            ledger_paths.push((publisher, seed, path));
        }
        let source = FileLedgerSource::new(&dir, keys);
        let publishers = source.publishers();
        assert_eq!(publishers, vec!["pub1".to_string(), "pub2".to_string()]);

        // Three overview sweeps: one verification per publisher in total and
        // zero record materializations on the count path.
        for sweep in 0..3 {
            let mut total = 0;
            for publisher in &publishers {
                total += source.record_count(publisher).await.expect("count");
            }
            assert_eq!(total, 5, "sweep {sweep}");
        }
        assert_eq!(
            source.cache.verifications.load(Ordering::Relaxed),
            publishers.len() as u64,
            "one verification per publisher, no re-verification across sweeps"
        );
        assert_eq!(
            source.cache.materializations.load(Ordering::Relaxed),
            0,
            "counting never clones records"
        );

        // The rendered path still materializes, and both paths agree.
        let records = source.records("pub1").await.expect("records");
        assert_eq!(records.len(), 3);
        assert_eq!(
            source.cache.materializations.load(Ordering::Relaxed),
            1,
            "reading records is what materializes them"
        );
        assert_eq!(
            source.record_count("pub1").await.expect("count"),
            records.len() as u64
        );

        // A changed file re-verifies once and the new count is served.
        let (publisher, seed, path) = &ledger_paths[0];
        {
            let mut ledger = apex_proof::Ledger::open(path, apex_proof::Keypair::from_bytes(seed))
                .expect("reopen ledger");
            ledger
                .append(lift_draft(r#"{"type":"dataPull","day":"2026-11-01"}"#))
                .expect("append");
        }
        assert_eq!(
            source.record_count(publisher).await.expect("count"),
            4,
            "the appended record is counted"
        );
        assert_eq!(
            source.cache.verifications.load(Ordering::Relaxed),
            publishers.len() as u64 + 1,
            "exactly one re-verification after the change"
        );
        assert_eq!(
            source.records(publisher).await.expect("records").len(),
            4,
            "the read path agrees with the count path"
        );

        // Refusals agree across both paths.
        for bad in ["pub3", "a/b"] {
            let count_error = source.record_count(bad).await.expect_err("refused");
            let read_error = source.records(bad).await.expect_err("refused");
            assert_eq!(count_error.to_string(), read_error.to_string(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn static_source_record_count_agrees_with_records() {
        let mut source = StaticLedgerSource::new(HashMap::new());
        assert!(source.record_count("pub").await.is_err());
        source.insert("pub", vec![]);
        assert_eq!(source.record_count("pub").await.expect("count"), 0);
        assert!(matches!(
            source.record_count("a/b").await,
            Err(ApiError::BadRequest("invalid_publisher"))
        ));
        assert!(source.has_publishers());
        assert!(!source.is_empty());
        let empty = StaticLedgerSource::default();
        assert!(!empty.has_publishers());
        assert!(empty.is_empty());
    }

    fn parse_lines(path: &Path) -> Result<Vec<String>, ApiError> {
        match std::fs::read_to_string(path) {
            Ok(raw) => Ok(raw.lines().map(str::to_string).collect()),
            // The log readers treat a missing file as an empty log.
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(ApiError::Internal(format!(
                "read {}: {error}",
                path.display()
            ))),
        }
    }

    /// A repeated read parses once; a changed file invalidates that one entry.
    /// A missing file is cached as empty and creation invalidates it.
    #[test]
    fn jsonl_cache_parses_once_and_invalidates_on_change() {
        let dir = std::env::temp_dir().join(format!("apex-jsonl-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("log.jsonl");
        let cache: JsonlCache<String> = JsonlCache::default();

        std::fs::write(&path, "one\ntwo\n").expect("write");
        let first = cache
            .get_or_parse(&path, 1 << 20, parse_lines)
            .expect("first parse");
        assert_eq!(&*first, &["one".to_string(), "two".to_string()]);
        let second = cache
            .get_or_parse(&path, 1 << 20, parse_lines)
            .expect("cache hit");
        assert_eq!(first, second);
        assert_eq!(cache.stats().parses, 1, "the repeated read parses once");
        assert_eq!(cache.stats().hits, 1);

        std::fs::write(&path, "one\ntwo\nthree\n").expect("rewrite");
        let changed = cache
            .get_or_parse(&path, 1 << 20, parse_lines)
            .expect("reparse");
        assert_eq!(changed.len(), 3, "the changed file is reparsed");
        assert_eq!(cache.stats().parses, 2, "exactly one reparse");

        let missing = dir.join("missing.jsonl");
        let empty = cache
            .get_or_parse(&missing, 1 << 20, parse_lines)
            .expect("missing is empty");
        assert!(empty.is_empty());
        let _ = cache
            .get_or_parse(&missing, 1 << 20, parse_lines)
            .expect("missing cache hit");
        assert_eq!(cache.stats().parses, 3, "a missing file parses once");
        std::fs::write(&missing, "created\n").expect("create");
        let created = cache
            .get_or_parse(&missing, 1 << 20, parse_lines)
            .expect("created");
        assert_eq!(&*created, &["created".to_string()]);
        assert_eq!(cache.stats().parses, 4, "creation invalidates the entry");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Concurrent readers of one unchanged file parse it once: the fast hit
    /// path and the serialised miss path are both safe under contention.
    #[test]
    fn jsonl_cache_concurrent_readers_parse_once() {
        let dir =
            std::env::temp_dir().join(format!("apex-jsonl-concurrent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("log.jsonl");
        std::fs::write(&path, "a\nb\nc\n").expect("write");
        let cache = Arc::new(JsonlCache::<String>::default());

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        let rows = cache
                            .get_or_parse(&path, 1 << 20, parse_lines)
                            .expect("concurrent read");
                        assert_eq!(rows.len(), 3);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("reader thread");
        }
        assert_eq!(
            cache.stats().parses,
            1,
            "concurrent readers parse the file once"
        );
        assert_eq!(cache.stats().hits, 199);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
