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
//!   records are only ever parsed from bytes that passed verification. A
//!   verification failure is `422 ledger_unverified` with the 1-based first
//!   offending line quoted from the verifier; a ledger above
//!   [`MAX_LEDGER_BYTES`] is refused before any read.
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
use std::sync::Arc;

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

/// Read cap for one ledger file: a ledger larger than this is refused before
/// any bytes are read, so a hostile or runaway file cannot exhaust memory.
pub const MAX_LEDGER_BYTES: u64 = 32 * 1024 * 1024;

/// Parse a URL/platform-file name into a [`PlatformId`].
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
}

/// A verified ledger provider (one append-only chain per publisher).
#[async_trait]
pub trait LedgerSource: Send + Sync + std::fmt::Debug {
    /// Return the publisher's records after full chain verification.
    async fn records(&self, publisher: &str) -> Result<Vec<LedgerRecord>, ApiError>;
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

/// Reads and verifies `<root>/<publisher>.jsonl` against the pinned key.
///
/// The key map is the allow-list of onboarded publishers: an unknown (but
/// well-formed) publisher is a `404`, never an attempt at some other file.
/// Records are parsed from the one read that verification accepted, so a file
/// swapped after an earlier check can never be served from a stale
/// verification.
#[derive(Debug, Clone)]
pub struct FileLedgerSource {
    keys: Arc<HashMap<String, VerifyingKey>>,
    root: PathBuf,
    canonical_root: PathBuf,
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
        let size = match std::fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(ApiError::NotFound),
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "stat ledger {}: {error}",
                    path.display()
                )));
            }
        };
        ensure_within_cap(size, MAX_LEDGER_BYTES)?;
        Ledger::read_verified(&path, expected_key).map_err(|error| {
            match ledger_error_line(&error) {
                Some(line) => ApiError::LedgerUnverified { line },
                None => ApiError::Internal(format!("verify ledger {}: {error}", path.display())),
            }
        })
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
    fn platform_parsing_is_a_closed_set() {
        assert_eq!(
            parse_platform("applovin_max"),
            Some(PlatformId::AppLovinMax)
        );
        assert_eq!(parse_platform("levelplay"), Some(PlatformId::LevelPlay));
        assert_eq!(parse_platform("admob"), Some(PlatformId::Admob));
        for unknown in ["", "Admob", "applovin", "max", " meta", "admob "] {
            assert_eq!(parse_platform(unknown), None, "{unknown:?}");
        }
        // The console labels and platform names stay aligned.
        for (name, _label) in PLATFORM_NAMES {
            assert!(parse_platform(name).is_some(), "{name}");
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
}
