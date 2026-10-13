//! Snapshot and ledger sources, plus the publisher/platform parsing rules.
//!
//! Two real file-backed implementations ship here:
//!
//! - [`JsonFileAuditSource`] reads `<root>/<platform>.json` into an
//!   `apex_audit::Snapshot`. A missing file is `404`; an unparseable or
//!   mismatched file is a logged internal error (an operator problem, not a
//!   client one).
//! - [`FileLedgerSource`] reads `<root>/<publisher>.jsonl` and re-verifies the
//!   whole chain with `apex_proof::Ledger::verify` before serving any record.
//!   A verification failure is `422 ledger_unverified` with the 1-based first
//!   offending line quoted from the verifier.
//!
//! ## Path traversal is structurally impossible
//!
//! Publisher names are the only URL-derived file names, and
//! [`is_valid_publisher`] restricts them to `^[A-Za-z0-9_-]{1,64}$` (ASCII
//! alphanumerics, `_`, `-`). `.`, `/`, `\`, NUL, whitespace, unicode and
//! over-long names are refused with a `400 invalid_publisher` before any path
//! is built; the sources re-check the name so they are safe even when called
//! directly (pinned by tests). Platform names are matched against the closed
//! set `applovin_max | levelplay | admob`.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use apex_audit::Snapshot;
use apex_core::PlatformId;
use apex_proof::{Ledger, LedgerError, LedgerRecord, VerifyingKey};
use async_trait::async_trait;
use serde::Deserialize;

use crate::error::ApiError;

/// The three supported platforms and their console labels, in display order.
pub const PLATFORM_NAMES: [(&str, &str); 3] = [
    ("applovin_max", "AppLovin MAX"),
    ("levelplay", "LevelPlay"),
    ("admob", "AdMob"),
];

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
}

impl JsonFileAuditSource {
    /// A source rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
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
        let path = self.path_for(platform);
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
        self.load(platform)
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

/// Reads and re-verifies `<root>/<publisher>.jsonl` against the pinned key.
///
/// The key map is the allow-list of onboarded publishers: an unknown (but
/// well-formed) publisher is a `404`, never an attempt at some other file.
#[derive(Debug, Clone)]
pub struct FileLedgerSource {
    keys: HashMap<String, VerifyingKey>,
    root: PathBuf,
}

impl FileLedgerSource {
    /// Build a source from the publisher -> public-key allow-list and the
    /// ledger root.
    pub fn new(root: impl Into<PathBuf>, keys: HashMap<String, VerifyingKey>) -> Self {
        Self {
            root: root.into(),
            keys,
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
        let path = self.path_for(publisher);
        match std::fs::metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(ApiError::NotFound),
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "stat ledger {}: {error}",
                    path.display()
                )));
            }
        }
        Ledger::verify(&path, expected_key).map_err(|error| match ledger_error_line(&error) {
            Some(line) => ApiError::LedgerUnverified { line },
            None => ApiError::Internal(format!("verify ledger {}: {error}", path.display())),
        })?;
        let bytes = std::fs::read(&path).map_err(|error| {
            ApiError::Internal(format!("read ledger {}: {error}", path.display()))
        })?;
        parse_verified_records(&bytes)
    }
}

#[async_trait]
impl LedgerSource for FileLedgerSource {
    async fn records(&self, publisher: &str) -> Result<Vec<LedgerRecord>, ApiError> {
        self.load(publisher)
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

/// One persisted ledger line, mirroring the byte-exact format documented and
/// enforced by `apex_proof` (`{"record":...,"signature":...,"pubkey":...}`).
/// Only `record` is read here: `Ledger::verify` has already re-derived the
/// canonical bytes and checked the signature and chain, so the extra fields
/// need no second look.
#[derive(Debug, Deserialize)]
struct PersistedLine {
    record: LedgerRecord,
}

/// Parse the records out of a file that `Ledger::verify` has already accepted.
///
/// The verified file always ends with a newline and every chunk is a canonical
/// `PersistedLine`; a parse failure here therefore means the file changed
/// between verification and this read, which is reported as an internal error.
fn parse_verified_records(bytes: &[u8]) -> Result<Vec<LedgerRecord>, ApiError> {
    let mut records = Vec::new();
    if bytes.is_empty() {
        return Ok(records);
    }
    let Some((complete, _)) = bytes.split_at_checked(bytes.len() - 1) else {
        return Err(ApiError::Internal("ledger ended unexpectedly".into()));
    };
    for (index, chunk) in complete.split(|byte| *byte == b'\n').enumerate() {
        let line: PersistedLine = serde_json::from_slice(chunk).map_err(|error| {
            ApiError::Internal(format!("parse verified ledger line {}: {error}", index + 1))
        })?;
        records.push(line.record);
    }
    Ok(records)
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
