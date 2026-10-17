//! Snapshot and ledger sources, plus the publisher/platform parsing rules.
//!
//! Two real file-backed implementations ship here:
//!
//! - [`JsonFileAuditSource`] reads `<root>/<platform>.json` into an
//!   `apex_audit::Snapshot`. A missing file is `404`; an unparseable or
//!   mismatched file is a logged internal error (an operator problem, not a
//!   client one).
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

use apex_audit::Snapshot;
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

/// An audit snapshot provider (one snapshot per platform).
#[async_trait]
pub trait AuditSource: Send + Sync + std::fmt::Debug {
    /// Fetch the current snapshot for `platform`, or the typed error that
    /// prevents serving one.
    async fn snapshot(&self, platform: PlatformId) -> Result<Snapshot, ApiError>;

    /// Persist a freshly built snapshot. The file source writes it beside the
    /// reads (atomically); a static source refuses.
    async fn write(&self, platform: PlatformId, _snapshot: &Snapshot) -> Result<(), ApiError> {
        Err(ApiError::Internal(format!(
            "snapshot writing is not supported for {platform}"
        )))
    }
}

/// A verified ledger provider (one append-only chain per publisher).
#[async_trait]
pub trait LedgerSource: Send + Sync + std::fmt::Debug {
    /// Return the publisher's records after full chain verification.
    async fn records(&self, publisher: &str) -> Result<Vec<LedgerRecord>, ApiError>;

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
}

/// Reads `<root>/<platform>.json` as an `apex_audit::Snapshot`.
#[derive(Debug, Clone)]
pub struct JsonFileAuditSource {
    root: PathBuf,
    canonical_root: PathBuf,
}

impl JsonFileAuditSource {
    /// A source rooted at `root`; the root is canonicalized here, once.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let canonical_root = canonical_root(&root);
        Self {
            root,
            canonical_root,
        }
    }

    /// The directory snapshots are read from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file a platform's snapshot lives in.
    pub fn path_for(&self, platform: PlatformId) -> PathBuf {
        self.root.join(format!("{}.json", platform.as_str()))
    }

    fn load(&self, platform: PlatformId) -> Result<Snapshot, ApiError> {
        let path = contained_in(&self.canonical_root, self.path_for(platform))?;
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
        Ok(snapshot)
    }
}

#[async_trait]
impl AuditSource for JsonFileAuditSource {
    async fn write(&self, platform: PlatformId, snapshot: &Snapshot) -> Result<(), ApiError> {
        let path = self.path_for(platform);
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

    async fn snapshot(&self, platform: PlatformId) -> Result<Snapshot, ApiError> {
        let source = self.clone();
        tokio::task::spawn_blocking(move || source.load(platform))
            .await
            .map_err(|error| ApiError::Internal(format!("snapshot read task failed: {error}")))?
    }
}

/// An in-process snapshot map (used by embedders, tests and benchmarks).
#[derive(Debug, Clone, Default)]
pub struct StaticAuditSource {
    snapshots: HashMap<PlatformId, Snapshot>,
}

impl StaticAuditSource {
    /// Build from a platform -> snapshot map.
    pub fn new(snapshots: HashMap<PlatformId, Snapshot>) -> Self {
        Self { snapshots }
    }

    /// Add or replace one platform's snapshot.
    pub fn insert(&mut self, platform: PlatformId, snapshot: Snapshot) {
        self.snapshots.insert(platform, snapshot);
    }
}

#[async_trait]
impl AuditSource for StaticAuditSource {
    async fn snapshot(&self, platform: PlatformId) -> Result<Snapshot, ApiError> {
        self.snapshots
            .get(&platform)
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
struct FileStamp {
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
    fn read(metadata: &std::fs::Metadata) -> Option<Self> {
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

/// One verified ledger held in memory: the records and the file identity they
/// were verified at.
#[derive(Debug, Clone)]
struct CachedLedger {
    stamp: FileStamp,
    records: Vec<LedgerRecord>,
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
}

impl LedgerCache {
    fn get(&self, path: &Path, stamp: FileStamp) -> Option<Vec<LedgerRecord>> {
        let entries = self.entries.lock().ok()?;
        entries.get(path).and_then(|cached| {
            (cached.stamp == stamp).then(|| {
                self.materializations.fetch_add(1, Ordering::Relaxed);
                cached.records.clone()
            })
        })
    }

    /// The cached verified record count, without materializing the records.
    fn count(&self, path: &Path, stamp: FileStamp) -> Option<u64> {
        let entries = self.entries.lock().ok()?;
        entries
            .get(path)
            .and_then(|cached| (cached.stamp == stamp).then_some(cached.records.len() as u64))
    }

    fn put(&self, path: &Path, stamp: FileStamp, records: &[LedgerRecord]) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                path.to_path_buf(),
                CachedLedger {
                    stamp,
                    records: records.to_vec(),
                },
            );
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

    fn load(&self, publisher: &str) -> Result<Vec<LedgerRecord>, ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let expected_key = self.keys.get(publisher).ok_or(ApiError::NotFound)?;
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
        let size = metadata.len();
        ensure_within_cap(size, MAX_LEDGER_BYTES)?;
        let stamp = FileStamp::read(&metadata);
        if let Some(stamp) = stamp {
            if let Some(records) = self.cache.get(&path, stamp) {
                return Ok(records);
            }
        }
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

    /// The verified record count without materializing cached records.
    ///
    /// A cache hit answers from the entry's length alone; a miss runs the
    /// same verified read as [`Self::load`] (warming the cache) and counts
    /// what that read returned, so the count is always a count of records
    /// that passed verification.
    fn load_count_cached(&self, publisher: &str) -> Result<Option<u64>, ApiError> {
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
        let size = metadata.len();
        ensure_within_cap(size, MAX_LEDGER_BYTES)?;
        let stamp = FileStamp::read(&metadata);
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
        assert!(audit.path_for(PlatformId::Admob).ends_with("admob.json"));

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
        };
        std::fs::create_dir_all(&dir).unwrap();
        let source = JsonFileAuditSource::new(&dir);
        assert!(
            source.snapshot(PlatformId::AppLovinMax).await.is_err(),
            "nothing yet"
        );
        source
            .write(PlatformId::AppLovinMax, &snapshot)
            .await
            .expect("write");
        assert!(source.path_for(PlatformId::AppLovinMax).exists(), "written");
        let read = source
            .snapshot(PlatformId::AppLovinMax)
            .await
            .expect("read");
        assert_eq!(read.fetched_at_unix_ms, 7);
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
            audit.snapshot(PlatformId::Admob).await,
            Err(ApiError::NotFound)
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
    }
}
