//! Platform connections: the vault-encrypted credential store and the honest
//! capability model behind `/console/connections`.
//!
//! What the console can honestly do (the capability release matrix of the
//! design document):
//!
//! - **AppLovin MAX** has live audit reads and a MAX-specific runner, so a
//!   submitted connection is validated with a real read and its writes are
//!   gated by the approval policy (`Approval required`).
//! - **LevelPlay and AdMob** have live audit reads and connector operations in
//!   `apex-connectors`, but the current production runner wires no equivalent
//!   full lifecycle adapters, so their write readiness is `Not enabled`.
//! - The shipped console remains read-only; a validation never writes to a
//!   platform.
//!
//! Secret material is written only through [`Vault`] (AES-256-GCM) into
//! `<root>/<publisher>.blob`, the same format `live.rs` reads. The console
//! never renders a secret: the page shows the masked credential metadata
//! (scheme, ciphertext length, stored field names), the validation time and
//! the scope. Validation states are records of real observations only:
//! `validated` when the platform answered the read, `rejected` when it
//! refused the credential, `expired` when a previously validated credential
//! stopped working, `unvalidated` when this deployment has no way to validate
//! (no live read path, or no verified platform host).
//!
//! ## Read cache
//!
//! The credential blob (after decrypt) and the observation records are cached
//! in process memory, keyed by the file's hardened stamp (length, mtime and on
//! Unix ctime and inode), the same discipline the snapshot and ledger caches
//! use. Repeated console renders are therefore a stat plus a clone instead of
//! another read, decrypt and JSON parse. Every write updates the entry under
//! the stamp of the file it just renamed into place, so the next render sees
//! the write immediately; an external rewrite changes the stamp and forces a
//! fresh read. The decrypted credential exists only in that in-memory cache:
//! it is never written anywhere, serialized into a response or included in
//! `Debug` output, and it is dropped with the store.

use std::collections::{BTreeMap, HashMap};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use apex_core::PlatformId;
use apex_vault::{entry_meta, Vault, VaultEntryMeta};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::live::LiveAccount;
use crate::sources::{contained_in, is_valid_publisher, FileStamp};

/// Documented MAX management host.
pub const MAX_MANAGEMENT_BASE: &str = apex_connectors::max::MAX_MANAGEMENT_BASE;
/// Documented MAX reporting host.
pub const MAX_REPORTING_BASE: &str = apex_connectors::max::MAX_REPORTING_BASE;
/// Deployment-configured LevelPlay platform host (no documented constant
/// exists in `apex-connectors`; without it a LevelPlay connection cannot be
/// validated and is stored `unvalidated`).
pub const LEVELPLAY_BASE_ENV: &str = "APEX_LEVELPLAY_BASE";
/// Deployment-configured AdMob platform host, same rule as LevelPlay.
pub const ADMOB_BASE_ENV: &str = "APEX_ADMOB_BASE";
/// Read cap for one connection metadata file.
pub const MAX_META_BYTES: u64 = 256 * 1024;
/// Suffix of the plain (non-secret) connection metadata file.
pub const META_SUFFIX: &str = ".connections.json";

/// How many unique temp names one credential write may try before reporting a
/// typed conflict. A collision needs two writers to draw the same 64 random
/// bits, so this bound is only a guard against a broken RNG or filesystem.
const MAX_TEMP_ATTEMPTS: usize = 16;

/// Write readiness per platform, straight from the capability matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteReadiness {
    /// Live writes exist and wait for the approval policy.
    ApprovalRequired,
    /// The production runner wires no full lifecycle adapter.
    NotEnabled,
}

impl WriteReadiness {
    /// The exact label the list column shows.
    pub fn label(&self) -> &'static str {
        match self {
            WriteReadiness::ApprovalRequired => "Approval required",
            WriteReadiness::NotEnabled => "Not enabled",
        }
    }
}

/// One platform's honest capabilities in this product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCapability {
    /// The wire name (`applovin_max`).
    pub wire: &'static str,
    /// The console label (`AppLovin MAX`).
    pub label: &'static str,
    /// Write readiness.
    pub write: WriteReadiness,
}

/// The three platforms, in the console's display order.
pub const CAPABILITIES: [PlatformCapability; 3] = [
    PlatformCapability {
        wire: "applovin_max",
        label: "AppLovin MAX",
        write: WriteReadiness::ApprovalRequired,
    },
    PlatformCapability {
        wire: "levelplay",
        label: "LevelPlay",
        write: WriteReadiness::NotEnabled,
    },
    PlatformCapability {
        wire: "admob",
        label: "AdMob",
        write: WriteReadiness::NotEnabled,
    },
];

/// The capability release matrix note for MAX (frozen copy).
pub const MATRIX_MAX_NOTE: &str =
    "MAX: live audit reads and a MAX-specific runner exist; writes need a verified allow-list.";
/// The matrix note for LevelPlay and AdMob (frozen copy).
pub const MATRIX_OTHER_NOTE: &str = "LevelPlay and AdMob: live audit reads and connector \
                                    operations exist; the current production runner does not \
                                    wire equivalent full lifecycle adapters.";
/// The matrix note for every platform (frozen copy).
pub const MATRIX_ALL_NOTE: &str = "All: the shipped console remains read-only.";

/// The recorded validation state of one platform credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialState {
    /// The platform answered a read with these credentials.
    Validated,
    /// The platform refused the credential.
    Rejected,
    /// A previously validated credential stopped working.
    Expired,
    /// Stored, but this deployment has no way to validate it yet.
    Unvalidated,
}

/// What is recorded about one platform's stored credential. Never a secret:
/// the field names, the observation times and the reporting outcome only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlatformState {
    /// The recorded validation state.
    pub state: Option<CredentialState>,
    /// When validation last ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validated_at_unix_ms: Option<i64>,
    /// Whether reporting reads were confirmed (MAX without a reporting key is
    /// `false`; the audit groups that need reporting cannot run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporting: Option<bool>,
    /// The stored credential field names (presence only, never values).
    #[serde(default)]
    pub credential_fields: Vec<String>,
    /// Bounded failure class of the last refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
}

impl PlatformState {
    /// The state, defaulting to `unvalidated`.
    pub fn credential_state(&self) -> CredentialState {
        self.state.unwrap_or(CredentialState::Unvalidated)
    }
}

/// The plain connection metadata file per publisher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionMeta {
    /// Per-platform state keyed by wire name.
    #[serde(default)]
    pub platforms: BTreeMap<String, PlatformState>,
}

impl ConnectionMeta {
    /// The state of one platform, defaulting to `unvalidated`.
    pub fn platform(&self, wire: &str) -> PlatformState {
        self.platforms.get(wire).cloned().unwrap_or_default()
    }

    /// Record one platform's state.
    pub fn set_platform(&mut self, wire: &str, state: PlatformState) {
        self.platforms.insert(wire.to_string(), state);
    }

    /// Drop one platform's state (disconnect).
    pub fn remove_platform(&mut self, wire: &str) {
        self.platforms.remove(wire);
    }
}

/// Observable counters of the connection-store caches. Tests and the
/// performance harness read them to prove that repeated console renders are a
/// stat plus a clone, not another read, decrypt and parse.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectionCacheStats {
    /// Decrypted blob file reads (one per account cache miss).
    pub account_reads: u64,
    /// Vault decryptions (one per account cache miss).
    pub account_decrypts: u64,
    /// Credential JSON parses (one per account cache miss).
    pub account_parses: u64,
    /// Account reads served from the cache (a stat plus a clone).
    pub account_hits: u64,
    /// Observation-record file reads (one per meta cache miss).
    pub meta_reads: u64,
    /// Observation-record JSON parses (one per meta cache miss).
    pub meta_parses: u64,
    /// Observation-record reads served from the cache.
    pub meta_hits: u64,
    /// Blob metadata reads (one per blob-metadata cache miss). This path
    /// never decrypts.
    pub blob_reads: u64,
    /// Blob metadata reads served from the cache.
    pub blob_hits: u64,
}

/// One decrypted account held in memory, keyed by the blob's hardened file
/// stamp. The plaintext credential never leaves this process: it is not
/// serialized, logged or exposed through `Debug` (see the manual
/// [`ConnectionCache`] `Debug` implementation).
#[derive(Debug)]
struct CachedAccount {
    stamp: FileStamp,
    account: LiveAccount,
}

/// One parsed observation record set, keyed by its file stamp.
#[derive(Debug)]
struct CachedMeta {
    stamp: FileStamp,
    meta: ConnectionMeta,
}

/// The masked evidence of one blob, keyed by its file stamp.
#[derive(Debug)]
struct CachedBlob {
    stamp: FileStamp,
    ciphertext_len: usize,
}

/// The in-process connection-store cache.
///
/// Entries are served only while the file's length, modification time and (on
/// Unix) ctime and inode are unchanged, the same hardened identity the
/// snapshot and ledger caches use, so an external same-length rewrite with a
/// restored mtime still invalidates. Every write path updates the entry under
/// the new stamp (or drops it when the stamp cannot be read), so a write is
/// visible to the next read immediately. The decrypted account is memory-only;
/// `Debug` prints entry counts and counters, never a value.
#[derive(Default)]
struct ConnectionCache {
    accounts: Mutex<HashMap<PathBuf, CachedAccount>>,
    metas: Mutex<HashMap<PathBuf, CachedMeta>>,
    blobs: Mutex<HashMap<PathBuf, CachedBlob>>,
    account_reads: AtomicU64,
    account_decrypts: AtomicU64,
    account_parses: AtomicU64,
    account_hits: AtomicU64,
    meta_reads: AtomicU64,
    meta_parses: AtomicU64,
    meta_hits: AtomicU64,
    blob_reads: AtomicU64,
    blob_hits: AtomicU64,
}

impl std::fmt::Debug for ConnectionCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = |entries: &Mutex<HashMap<PathBuf, CachedAccount>>| {
            entries.lock().map(|entries| entries.len()).unwrap_or(0)
        };
        formatter
            .debug_struct("ConnectionCache")
            .field("accounts", &len(&self.accounts))
            .field(
                "metas",
                &self.metas.lock().map(|entries| entries.len()).unwrap_or(0),
            )
            .field(
                "blobs",
                &self.blobs.lock().map(|entries| entries.len()).unwrap_or(0),
            )
            .field("account_reads", &self.account_reads.load(Ordering::Relaxed))
            .field(
                "account_decrypts",
                &self.account_decrypts.load(Ordering::Relaxed),
            )
            .field(
                "account_parses",
                &self.account_parses.load(Ordering::Relaxed),
            )
            .field("account_hits", &self.account_hits.load(Ordering::Relaxed))
            .field("meta_reads", &self.meta_reads.load(Ordering::Relaxed))
            .field("meta_parses", &self.meta_parses.load(Ordering::Relaxed))
            .field("meta_hits", &self.meta_hits.load(Ordering::Relaxed))
            .field("blob_reads", &self.blob_reads.load(Ordering::Relaxed))
            .field("blob_hits", &self.blob_hits.load(Ordering::Relaxed))
            .finish()
    }
}

impl ConnectionCache {
    fn get_account(&self, path: &Path, stamp: FileStamp) -> Option<LiveAccount> {
        let entries = self.accounts.lock().ok()?;
        entries.get(path).and_then(|cached| {
            (cached.stamp == stamp).then(|| {
                self.account_hits.fetch_add(1, Ordering::Relaxed);
                cached.account.clone()
            })
        })
    }

    fn put_account(&self, path: &Path, stamp: FileStamp, account: &LiveAccount) {
        if let Ok(mut entries) = self.accounts.lock() {
            entries.insert(
                path.to_path_buf(),
                CachedAccount {
                    stamp,
                    account: account.clone(),
                },
            );
        }
    }

    fn get_meta(&self, path: &Path, stamp: FileStamp) -> Option<ConnectionMeta> {
        let entries = self.metas.lock().ok()?;
        entries.get(path).and_then(|cached| {
            (cached.stamp == stamp).then(|| {
                self.meta_hits.fetch_add(1, Ordering::Relaxed);
                cached.meta.clone()
            })
        })
    }

    fn put_meta(&self, path: &Path, stamp: FileStamp, meta: &ConnectionMeta) {
        if let Ok(mut entries) = self.metas.lock() {
            entries.insert(
                path.to_path_buf(),
                CachedMeta {
                    stamp,
                    meta: meta.clone(),
                },
            );
        }
    }

    fn get_blob(&self, path: &Path, stamp: FileStamp) -> Option<usize> {
        let entries = self.blobs.lock().ok()?;
        entries.get(path).and_then(|cached| {
            (cached.stamp == stamp).then(|| {
                self.blob_hits.fetch_add(1, Ordering::Relaxed);
                cached.ciphertext_len
            })
        })
    }

    fn put_blob(&self, path: &Path, stamp: FileStamp, ciphertext_len: usize) {
        if let Ok(mut entries) = self.blobs.lock() {
            entries.insert(
                path.to_path_buf(),
                CachedBlob {
                    stamp,
                    ciphertext_len,
                },
            );
        }
    }

    fn stats(&self) -> ConnectionCacheStats {
        ConnectionCacheStats {
            account_reads: self.account_reads.load(Ordering::Relaxed),
            account_decrypts: self.account_decrypts.load(Ordering::Relaxed),
            account_parses: self.account_parses.load(Ordering::Relaxed),
            account_hits: self.account_hits.load(Ordering::Relaxed),
            meta_reads: self.meta_reads.load(Ordering::Relaxed),
            meta_parses: self.meta_parses.load(Ordering::Relaxed),
            meta_hits: self.meta_hits.load(Ordering::Relaxed),
            blob_reads: self.blob_reads.load(Ordering::Relaxed),
            blob_hits: self.blob_hits.load(Ordering::Relaxed),
        }
    }
}

/// Shared inner state so blocking file work can move to the blocking pool.
#[derive(Debug)]
struct Inner {
    root: PathBuf,
    canonical_root: PathBuf,
    vault: Vault,
    cache: ConnectionCache,
}

/// One per-call random 64-bit suffix for a credential temp file.
fn temp_suffix() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Whether two metadata reads describe the same file after a rename. On Unix
/// the inode is stable across `rename` and unique per temp file, so it
/// identifies this call's write even when another writer replaced the target
/// immediately afterwards. The fallback compares the size and modification
/// time (the fields every platform exposes).
fn same_written_file(written: &std::fs::Metadata, current: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        written.ino() == current.ino()
            && written.len() == current.len()
            && written.modified().ok() == current.modified().ok()
    }
    #[cfg(not(unix))]
    {
        written.len() == current.len() && written.modified().ok() == current.modified().ok()
    }
}

/// Publish `bytes` through a per-call unique temp file and an atomic rename.
///
/// `create_new` refuses a name another writer already owns, so concurrent
/// saves never share a temp file; the rename is the atomic publish. The
/// returned stamp is `Some` only when this call's file is still the one at
/// `target`, which makes the write-through cache priming reflect the winner
/// rather than whichever writer happened to stat last. If another writer
/// published after us, the winner primes the cache itself.
fn publish_unique(target: &Path, bytes: &[u8], label: &str) -> Result<Option<FileStamp>, ApiError> {
    let parent = target
        .parent()
        .ok_or_else(|| ApiError::Internal(format!("{label} path has no parent")))?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ApiError::Internal(format!("{label} path has no file name")))?;
    let mut temp: Option<PathBuf> = None;
    for _ in 0..MAX_TEMP_ATTEMPTS {
        let candidate = parent.join(format!("{name}.tmp-{}", temp_suffix()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            // The encrypted credential bundle and the observation record are
            // both per-publisher secrets-adjacent state. Create the temp file
            // with 0600 so the atomic rename publishes the final file with
            // the same restrictive mode (instead of the process umask's
            // world-readable default).
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(mut file) => {
                let written = file.write_all(bytes).and_then(|()| file.sync_all());
                drop(file);
                match written {
                    Ok(()) => {
                        temp = Some(candidate);
                        break;
                    }
                    Err(error) => {
                        let _ = std::fs::remove_file(&candidate);
                        return Err(ApiError::Internal(format!("{label} write: {error}")));
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(ApiError::Internal(format!("{label} write: {error}")));
            }
        }
    }
    let Some(temp) = temp else {
        return Err(ApiError::Conflict("credential_write_conflict"));
    };
    let written = match std::fs::metadata(&temp) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            return Err(ApiError::Internal(format!("{label} stat: {error}")));
        }
    };
    if let Err(error) = std::fs::rename(&temp, target) {
        let _ = std::fs::remove_file(&temp);
        return Err(ApiError::Internal(format!("{label} rename: {error}")));
    }
    if let Err(error) = crate::config::sync_parent(target) {
        return Err(ApiError::Internal(format!(
            "{label} directory sync: {error}"
        )));
    }
    let Ok(current) = std::fs::metadata(target) else {
        // The publish succeeded; a stat failure only means this call cannot
        // prime the cache and the next read will.
        return Ok(None);
    };
    if same_written_file(&written, &current) {
        Ok(FileStamp::read(&current))
    } else {
        Ok(None)
    }
}

/// Vault-encrypted connection storage: `<root>/<publisher>.blob` holds the
/// encrypted [`LiveAccount`] (the same file the live read path consumes) and
/// `<root>/<publisher>.connections.json` holds the non-secret observation
/// records. The UI never depends on the file layout; this type is the only
/// reader/writer of it.
#[derive(Debug, Clone)]
pub struct VaultedConnectionStore {
    inner: Arc<Inner>,
}

impl VaultedConnectionStore {
    /// Build from the credential root and vault key (64 hex characters).
    pub fn with_vault_key_hex(
        root: impl Into<PathBuf>,
        key_hex: &str,
    ) -> Result<Self, apex_vault::VaultError> {
        let root = root.into();
        // The canonical root is only meaningful once the directory exists;
        // creating it here keeps escapes closed on the first write too.
        let _ = std::fs::create_dir_all(&root);
        let canonical_root = crate::sources::canonical_root(&root);
        Ok(Self {
            inner: Arc::new(Inner {
                root,
                canonical_root,
                vault: Vault::from_hex_key(key_hex)?,
                cache: ConnectionCache::default(),
            }),
        })
    }

    /// The root directory.
    pub fn root(&self) -> &std::path::Path {
        &self.inner.root
    }

    /// This store's cache counters (tests and the performance harness).
    pub fn cache_stats(&self) -> ConnectionCacheStats {
        self.inner.cache.stats()
    }

    /// Load the account, or `None` when the publisher has no blob yet.
    pub async fn load_account(&self, publisher: &str) -> Result<Option<LiveAccount>, ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let inner = Arc::clone(&self.inner);
        let publisher = publisher.to_string();
        tokio::task::spawn_blocking(move || {
            let path = inner.root.join(format!("{publisher}.blob"));
            let resolved = match contained_in(&inner.canonical_root, path) {
                Ok(resolved) => resolved,
                Err(ApiError::NotFound) => return Ok(None),
                Err(error) => return Err(error),
            };
            let metadata = std::fs::metadata(&resolved)
                .map_err(|error| ApiError::Internal(format!("credential stat: {error}")))?;
            crate::sources::ensure_within_cap(metadata.len(), crate::live::MAX_CREDENTIAL_BYTES)?;
            let stamp = FileStamp::read(&metadata);
            if let Some(stamp) = stamp {
                if let Some(account) = inner.cache.get_account(&resolved, stamp) {
                    return Ok(Some(account));
                }
            }
            inner.cache.account_reads.fetch_add(1, Ordering::Relaxed);
            let blob = std::fs::read_to_string(&resolved)
                .map_err(|error| ApiError::Internal(format!("credential read: {error}")))?;
            inner.cache.account_decrypts.fetch_add(1, Ordering::Relaxed);
            let plaintext = inner
                .vault
                .decrypt(&blob)
                .map_err(|error| ApiError::Internal(format!("credential decrypt: {error}")))?;
            inner.cache.account_parses.fetch_add(1, Ordering::Relaxed);
            let account: LiveAccount = serde_json::from_str(&plaintext)
                .map_err(|error| ApiError::Internal(format!("credential parse: {error}")))?;
            if let Some(stamp) = stamp {
                inner.cache.put_account(&resolved, stamp, &account);
            }
            Ok(Some(account))
        })
        .await
        .map_err(|join| ApiError::Internal(format!("credential task failed: {join}")))?
    }

    /// Write the account back, encrypted, atomically.
    pub async fn save_account(
        &self,
        publisher: &str,
        account: &LiveAccount,
    ) -> Result<(), ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let inner = Arc::clone(&self.inner);
        let publisher = publisher.to_string();
        let plaintext = serde_json::to_string(account)
            .map_err(|error| ApiError::Internal(format!("credential serialization: {error}")))?;
        let account = account.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&inner.root)
                .map_err(|error| ApiError::Internal(format!("credential root: {error}")))?;
            let root = inner
                .root
                .canonicalize()
                .map_err(|error| ApiError::Internal(format!("credential root: {error}")))?;
            let blob = inner.vault.encrypt(&plaintext);
            let ciphertext_len = blob.len();
            let target = root.join(format!("{publisher}.blob"));
            // The publish is atomic and uses a per-call temp name, so
            // concurrent saves never race on one temp file. The returned
            // stamp identifies this write as the current file; a loser does
            // not prime the cache and cannot overwrite the winner's entry.
            let stamp = publish_unique(&target, blob.as_bytes(), "credential")?;
            // The write is the cache update: the next render sees the account
            // that was just stored without another read and decrypt. A lost
            // publish race or a stat failure only means the entry is served
            // on the next read.
            if let Some(stamp) = stamp {
                inner.cache.put_account(&target, stamp, &account);
                inner.cache.put_blob(&target, stamp, ciphertext_len);
            }
            Ok(())
        })
        .await
        .map_err(|join| ApiError::Internal(format!("credential task failed: {join}")))?
    }

    /// The masked evidence of the stored bundle: scheme and ciphertext length.
    pub async fn blob_meta(&self, publisher: &str) -> Result<Option<VaultEntryMeta>, ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let inner = Arc::clone(&self.inner);
        let publisher = publisher.to_string();
        tokio::task::spawn_blocking(move || {
            let path = inner.root.join(format!("{publisher}.blob"));
            let resolved = match contained_in(&inner.canonical_root, path) {
                Ok(resolved) => resolved,
                Err(ApiError::NotFound) => return Ok(None),
                Err(error) => return Err(error),
            };
            // The metadata read never loads an oversized blob into memory:
            // the same stat-and-cap check as `load_account`, so an
            // over-cap file is the same typed error class on both paths.
            let metadata = std::fs::metadata(&resolved)
                .map_err(|error| ApiError::Internal(format!("credential stat: {error}")))?;
            crate::sources::ensure_within_cap(metadata.len(), crate::live::MAX_CREDENTIAL_BYTES)?;
            let stamp = FileStamp::read(&metadata);
            if let Some(stamp) = stamp {
                if let Some(ciphertext_len) = inner.cache.get_blob(&resolved, stamp) {
                    return Ok(Some(VaultEntryMeta {
                        ciphertext_len,
                        scheme: "aes-256-gcm",
                    }));
                }
            }
            inner.cache.blob_reads.fetch_add(1, Ordering::Relaxed);
            let blob = std::fs::read_to_string(&resolved)
                .map_err(|error| ApiError::Internal(format!("credential read: {error}")))?;
            let meta = entry_meta(&blob);
            if let Some(stamp) = stamp {
                inner.cache.put_blob(&resolved, stamp, meta.ciphertext_len);
            }
            Ok(Some(meta))
        })
        .await
        .map_err(|join| ApiError::Internal(format!("credential task failed: {join}")))?
    }

    /// Load the observation records; a missing file is an empty default.
    pub async fn load_meta(&self, publisher: &str) -> Result<ConnectionMeta, ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let inner = Arc::clone(&self.inner);
        let publisher = publisher.to_string();
        tokio::task::spawn_blocking(move || {
            let path = inner.root.join(format!("{publisher}{META_SUFFIX}"));
            let resolved = match contained_in(&inner.canonical_root, path) {
                Ok(resolved) => resolved,
                Err(ApiError::NotFound) => return Ok(ConnectionMeta::default()),
                Err(error) => return Err(error),
            };
            let metadata = std::fs::metadata(&resolved)
                .map_err(|error| ApiError::Internal(format!("connection meta stat: {error}")))?;
            crate::sources::ensure_within_cap(metadata.len(), MAX_META_BYTES)?;
            let stamp = FileStamp::read(&metadata);
            if let Some(stamp) = stamp {
                if let Some(meta) = inner.cache.get_meta(&resolved, stamp) {
                    return Ok(meta);
                }
            }
            inner.cache.meta_reads.fetch_add(1, Ordering::Relaxed);
            let raw = std::fs::read_to_string(&resolved)
                .map_err(|error| ApiError::Internal(format!("connection meta read: {error}")))?;
            inner.cache.meta_parses.fetch_add(1, Ordering::Relaxed);
            let meta = serde_json::from_str(&raw)
                .map_err(|error| ApiError::Internal(format!("connection meta parse: {error}")))?;
            if let Some(stamp) = stamp {
                inner.cache.put_meta(&resolved, stamp, &meta);
            }
            Ok(meta)
        })
        .await
        .map_err(|join| ApiError::Internal(format!("connection meta task failed: {join}")))?
    }

    /// Write the observation records atomically.
    pub async fn save_meta(&self, publisher: &str, meta: &ConnectionMeta) -> Result<(), ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let inner = Arc::clone(&self.inner);
        let publisher = publisher.to_string();
        let raw = serde_json::to_vec_pretty(meta).map_err(|error| {
            ApiError::Internal(format!("connection meta serialization: {error}"))
        })?;
        let meta = meta.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&inner.root)
                .map_err(|error| ApiError::Internal(format!("connection meta root: {error}")))?;
            let root = inner
                .root
                .canonicalize()
                .map_err(|error| ApiError::Internal(format!("connection meta root: {error}")))?;
            let target = root.join(format!("{publisher}{META_SUFFIX}"));
            let stamp = publish_unique(&target, &raw, "connection meta")?;
            if let Some(stamp) = stamp {
                inner.cache.put_meta(&target, stamp, &meta);
            }
            Ok(())
        })
        .await
        .map_err(|join| ApiError::Internal(format!("connection meta task failed: {join}")))?
    }

    /// Remove one platform's credentials. Returns whether anything changed.
    /// Removing the last platform also removes the encrypted bundle: keeping
    /// ciphertext after the customer's revocation would contradict the page.
    pub async fn disconnect_platform(
        &self,
        publisher: &str,
        platform: PlatformId,
    ) -> Result<bool, ApiError> {
        let Some(mut account) = self.load_account(publisher).await? else {
            return Ok(false);
        };
        let removed = account.platforms.remove(platform.as_str()).is_some();
        if removed {
            self.save_account(publisher, &account).await?;
            let mut meta = self.load_meta(publisher).await?;
            meta.remove_platform(platform.as_str());
            self.save_meta(publisher, &meta).await?;
            // The encrypted bundle belongs to the account, not to one
            // platform. Once the last platform is gone the bundle must go
            // with it; leaving ciphertext after a disconnect would contradict
            // the page and the customer's revocation.
            if account.platforms.is_empty() {
                self.remove_blob(publisher).await?;
            }
        }
        Ok(removed)
    }

    /// Remove the encrypted credential bundle for one publisher.
    async fn remove_blob(&self, publisher: &str) -> Result<(), ApiError> {
        if !is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let inner = Arc::clone(&self.inner);
        let publisher = publisher.to_string();
        tokio::task::spawn_blocking(move || {
            let path = inner.root.join(format!("{publisher}.blob"));
            let resolved = match contained_in(&inner.canonical_root, path) {
                Ok(resolved) => resolved,
                Err(ApiError::NotFound) => return Ok(()),
                Err(error) => return Err(error),
            };
            match std::fs::remove_file(resolved) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(ApiError::Internal(format!("credential remove: {error}"))),
            }
        })
        .await
        .map_err(|join| ApiError::Internal(format!("credential task failed: {join}")))?
    }
}

/// The base URLs a console submission uses per platform, or `None` when this
/// deployment has no verified host for the platform.
pub fn platform_bases(
    platform: PlatformId,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<(String, Option<String>)> {
    match platform {
        PlatformId::AppLovinMax => Some((
            MAX_MANAGEMENT_BASE.to_string(),
            Some(MAX_REPORTING_BASE.to_string()),
        )),
        PlatformId::LevelPlay => lookup(LEVELPLAY_BASE_ENV)
            .filter(|value| !value.trim().is_empty())
            .map(|base| (base, None)),
        PlatformId::Admob => lookup(ADMOB_BASE_ENV)
            .filter(|value| !value.trim().is_empty())
            .map(|base| (base, None)),
        // The billing domain is not a mediation platform: it has no
        // management/reporting base to validate credentials against.
        PlatformId::Billing => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn account() -> LiveAccount {
        let mut platforms = BTreeMap::new();
        platforms.insert(
            "applovin_max".to_string(),
            crate::live::LiveCredentials {
                api_key: "management-key".to_string(),
                account_id: "acct-1".to_string(),
                base_url: MAX_MANAGEMENT_BASE.to_string(),
                report_key: Some("reporting-key".to_string()),
                reporting_base_url: None,
                refresh_token: None,
            },
        );
        LiveAccount {
            developer_site: "studio.example.com".to_string(),
            expected_networks: vec![],
            ad_unit_ids: vec![],
            bidder_accounts: vec![],
            platforms,
        }
    }

    fn temp_store(name: &str) -> (VaultedConnectionStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("apex-conn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = VaultedConnectionStore::with_vault_key_hex(&dir, KEY).expect("store");
        (store, dir)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn published_credential_and_meta_files_are_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let (store, dir) = temp_store("modes");
        store.save_account("pub1", &account()).await.expect("save");
        let mut meta = ConnectionMeta::default();
        meta.set_platform(
            "applovin_max",
            PlatformState {
                state: Some(CredentialState::Unvalidated),
                validated_at_unix_ms: None,
                reporting: None,
                credential_fields: vec!["account_id".to_string()],
                failure_class: None,
            },
        );
        store.save_meta("pub1", &meta).await.expect("save meta");
        for name in ["pub1.blob", "pub1.connections.json"] {
            let mode = std::fs::metadata(dir.join(name))
                .expect("published file")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{name} must be 0600, got {mode:o}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn account_roundtrips_encrypted_and_never_leaks_plaintext() {
        let (store, dir) = temp_store("roundtrip");
        assert!(store.load_account("pub1").await.expect("load").is_none());
        store.save_account("pub1", &account()).await.expect("save");
        let raw = std::fs::read_to_string(dir.join("pub1.blob")).expect("blob exists");
        assert!(
            !raw.contains("management-key") && !raw.contains("reporting-key"),
            "the blob is ciphertext only"
        );
        let loaded = store
            .load_account("pub1")
            .await
            .expect("load")
            .expect("some");
        assert_eq!(
            loaded.credentials(PlatformId::AppLovinMax).unwrap().api_key,
            "management-key"
        );
        let meta = store.blob_meta("pub1").await.expect("meta").expect("some");
        assert_eq!(meta.scheme, "aes-256-gcm");
        assert!(meta.ciphertext_len > 0);
        // A blob written under another key fails closed.
        let other = VaultedConnectionStore::with_vault_key_hex(
            &dir,
            "1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100",
        )
        .expect("store");
        assert!(matches!(
            other.load_account("pub1").await,
            Err(ApiError::Internal(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn blob_meta_refuses_an_oversized_blob_before_reading() {
        let (store, dir) = temp_store("blob-cap");
        std::fs::create_dir_all(&dir).expect("dir");
        let oversized =
            vec![b'a'; usize::try_from(crate::live::MAX_CREDENTIAL_BYTES).expect("fits") + 1];
        std::fs::write(dir.join("pub1.blob"), oversized).expect("write oversized blob");
        let error = store
            .blob_meta("pub1")
            .await
            .expect_err("an over-cap blob is refused");
        assert!(matches!(error, ApiError::Internal(_)), "{error}");
        assert!(error.to_string().contains("read cap"), "{error}");
        assert!(
            matches!(store.load_account("pub1").await, Err(ApiError::Internal(_))),
            "both reads refuse the same over-cap file the same way"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn meta_roundtrips_and_defaults_to_empty() {
        let (store, dir) = temp_store("meta");
        let meta = store.load_meta("pub1").await.expect("default");
        assert!(meta.platforms.is_empty());
        assert_eq!(
            meta.platform("admob").credential_state(),
            CredentialState::Unvalidated
        );
        let mut meta = ConnectionMeta::default();
        meta.set_platform(
            "applovin_max",
            PlatformState {
                state: Some(CredentialState::Validated),
                validated_at_unix_ms: Some(7),
                reporting: Some(true),
                credential_fields: vec!["api_key".to_string(), "report_key".to_string()],
                failure_class: None,
            },
        );
        store.save_meta("pub1", &meta).await.expect("save");
        let loaded = store.load_meta("pub1").await.expect("load");
        assert_eq!(
            loaded.platform("applovin_max").state,
            Some(CredentialState::Validated)
        );
        assert_eq!(
            loaded.platform("applovin_max").validated_at_unix_ms,
            Some(7)
        );
        let raw = std::fs::read_to_string(dir.join("pub1.connections.json")).expect("file");
        assert!(raw.contains("\"validated\""), "{raw}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn disconnect_removes_the_platform_entry_and_its_observation() {
        let (store, dir) = temp_store("disconnect");
        store.save_account("pub1", &account()).await.expect("save");
        let mut meta = ConnectionMeta::default();
        meta.set_platform(
            "applovin_max",
            PlatformState {
                state: Some(CredentialState::Validated),
                ..PlatformState::default()
            },
        );
        store.save_meta("pub1", &meta).await.expect("save meta");
        assert!(store
            .disconnect_platform("pub1", PlatformId::AppLovinMax)
            .await
            .expect("disconnect"));
        assert!(!store
            .disconnect_platform("pub1", PlatformId::AppLovinMax)
            .await
            .expect("second disconnect is a no-op"));
        assert!(
            store.load_account("pub1").await.expect("load").is_none(),
            "the last platform's disconnect removes the encrypted bundle"
        );
        let meta = store.load_meta("pub1").await.expect("meta");
        assert!(meta.platforms.is_empty());
        // Unknown publishers are a clean no-op.
        assert!(!store
            .disconnect_platform("nobody", PlatformId::Admob)
            .await
            .expect("no account"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn invalid_publishers_and_escapes_are_refused() {
        let (store, dir) = temp_store("escape");
        std::fs::create_dir_all(&dir).expect("root");
        let outside = dir
            .parent()
            .expect("parent")
            .join(format!("apex-conn-escape-out-{}", std::process::id()));
        std::fs::write(&outside, "not encrypted").expect("outside");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, dir.join("pub1.blob")).expect("symlink");
        for publisher in ["a/b", "..", ""] {
            assert!(matches!(
                store.load_account(publisher).await,
                Err(ApiError::BadRequest("invalid_publisher"))
            ));
        }
        #[cfg(unix)]
        assert!(store
            .load_account("pub1")
            .await
            .expect("escape is hidden")
            .is_none());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn capability_matrix_matches_the_published_matrix() {
        let max = CAPABILITIES[0];
        assert_eq!(max.wire, "applovin_max");
        assert_eq!(max.write, WriteReadiness::ApprovalRequired);
        assert_eq!(CAPABILITIES[1].write, WriteReadiness::NotEnabled);
        assert_eq!(CAPABILITIES[2].write, WriteReadiness::NotEnabled);
        assert_eq!(
            WriteReadiness::ApprovalRequired.label(),
            "Approval required"
        );
        assert_eq!(WriteReadiness::NotEnabled.label(), "Not enabled");
        assert!(MATRIX_MAX_NOTE.contains("writes need a verified allow-list"));
        assert!(MATRIX_OTHER_NOTE.contains("does not wire equivalent full lifecycle adapters"));
        assert!(MATRIX_ALL_NOTE.contains("remains read-only"));
    }

    #[tokio::test]
    async fn repeated_account_renders_are_one_read_and_decrypt_until_the_file_changes() {
        let (store, dir) = temp_store("account-cache");
        store.save_account("pub1", &account()).await.expect("save");
        // A fresh store over the same directory models the next process: the
        // first read fills the cache, the rest are hits.
        let reader = VaultedConnectionStore::with_vault_key_hex(&dir, KEY).expect("reader");
        for _ in 0..3 {
            let loaded = reader
                .load_account("pub1")
                .await
                .expect("load")
                .expect("some");
            assert_eq!(
                loaded
                    .credentials(PlatformId::AppLovinMax)
                    .expect("max")
                    .api_key,
                "management-key"
            );
        }
        let stats = reader.cache_stats();
        assert_eq!(stats.account_reads, 1, "one file read");
        assert_eq!(stats.account_decrypts, 1, "one decryption");
        assert_eq!(stats.account_parses, 1, "one JSON parse");
        assert_eq!(stats.account_hits, 2, "the other two renders are hits");

        // An external rewrite with a different account changes the stamp.
        let vault = Vault::from_hex_key(KEY).expect("vault");
        let mut changed = account();
        changed
            .platforms
            .get_mut("applovin_max")
            .expect("max")
            .api_key = "rotated-key".to_string();
        std::fs::write(
            dir.join("pub1.blob"),
            vault.encrypt(&serde_json::to_string(&changed).expect("json")),
        )
        .expect("external write");
        let loaded = reader
            .load_account("pub1")
            .await
            .expect("load")
            .expect("some");
        assert_eq!(
            loaded
                .credentials(PlatformId::AppLovinMax)
                .expect("max")
                .api_key,
            "rotated-key"
        );
        let stats = reader.cache_stats();
        assert_eq!(stats.account_reads, 2, "the changed file is read again");
        assert_eq!(stats.account_decrypts, 2, "and decrypted again");
        assert_eq!(stats.account_parses, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_write_is_visible_to_the_next_read_without_a_reload() {
        let (store, dir) = temp_store("write-cache");
        store.save_account("pub1", &account()).await.expect("save");
        assert_eq!(
            store
                .load_account("pub1")
                .await
                .expect("load")
                .expect("some")
                .credentials(PlatformId::AppLovinMax)
                .expect("max")
                .api_key,
            "management-key"
        );
        let mut next = account();
        next.platforms.get_mut("applovin_max").expect("max").api_key = "second-key".to_string();
        store.save_account("pub1", &next).await.expect("save");
        let before = store.cache_stats();
        let loaded = store
            .load_account("pub1")
            .await
            .expect("load")
            .expect("some");
        assert_eq!(
            loaded
                .credentials(PlatformId::AppLovinMax)
                .expect("max")
                .api_key,
            "second-key"
        );
        let after = store.cache_stats();
        assert_eq!(
            after.account_reads, before.account_reads,
            "the write primed the cache, so the next render does not reload"
        );
        assert_eq!(after.account_hits, before.account_hits + 1);
        // The masked evidence is refreshed by the same write.
        let meta = store.blob_meta("pub1").await.expect("meta").expect("some");
        assert!(meta.ciphertext_len > 0);
        assert_eq!(
            store.cache_stats().blob_reads,
            before.blob_reads,
            "the write primed the blob-metadata entry too"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn concurrent_saves_use_unique_temps_and_the_cache_reflects_the_winner() {
        let (store, dir) = temp_store("concurrent-saves");
        let mut tasks = Vec::new();
        for index in 0..32 {
            let store = store.clone();
            tasks.push(tokio::spawn(async move {
                let mut account = account();
                account.developer_site = format!("winner-{index}.example.com");
                match store.save_account("pub1", &account).await {
                    Ok(()) => Ok(account.developer_site),
                    Err(error) => {
                        assert!(
                            matches!(error, ApiError::Conflict(_)),
                            "a concurrent save either succeeds or reports a typed conflict: {error}"
                        );
                        Err(error.to_string())
                    }
                }
            }));
        }
        let mut succeeded = 0usize;
        for task in tasks {
            let outcome = task.await.expect("join");
            if outcome.is_ok() {
                succeeded += 1;
            }
        }
        assert!(succeeded > 0, "at least one concurrent save wins");

        // Exactly one blob is published, and it decrypts to one complete
        // account. Reading it directly is the independent truth the cache
        // must agree with.
        let blobs: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "blob")
            })
            .collect();
        assert_eq!(blobs.len(), 1, "exactly one final blob per publisher");
        let vault = Vault::from_hex_key(KEY).expect("vault");
        let raw = std::fs::read_to_string(&blobs[0]).expect("blob");
        let plaintext = vault.decrypt(&raw).expect("decrypt");
        let stored: LiveAccount = serde_json::from_str(&plaintext).expect("parse");
        assert!(
            stored.developer_site.starts_with("winner-"),
            "the final blob is one writer's complete account: {}",
            stored.developer_site
        );

        // The winning save primed the cache, so the next render is a cache
        // hit and reports exactly the account in the final blob.
        let before = store.cache_stats();
        let loaded = store
            .load_account("pub1")
            .await
            .expect("load")
            .expect("some");
        assert_eq!(loaded.developer_site, stored.developer_site);
        let after = store.cache_stats();
        let hits_after_first = after.account_hits;
        if hits_after_first == before.account_hits {
            // A lost publish race is allowed to leave the cache cold: another
            // writer can rename between this writer's stamp and its cache
            // put. The next read must then converge on the same final blob
            // and prime the cache.
            let loaded = store
                .load_account("pub1")
                .await
                .expect("load")
                .expect("some");
            assert_eq!(loaded.developer_site, stored.developer_site);
            let converged = store.cache_stats();
            assert_eq!(
                converged.account_hits,
                hits_after_first + 1,
                "the cache converges after a cold read"
            );
        }

        // Every temp file was renamed or removed.
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp leftovers: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn repeated_meta_renders_are_one_parse_and_a_write_invalidates() {
        let (_store, dir) = temp_store("meta-cache");
        let mut meta = ConnectionMeta::default();
        meta.set_platform(
            "admob",
            PlatformState {
                state: Some(CredentialState::Rejected),
                ..PlatformState::default()
            },
        );
        std::fs::write(
            dir.join(format!("pub1{META_SUFFIX}")),
            serde_json::to_vec(&meta).expect("json"),
        )
        .expect("external meta");
        let reader = VaultedConnectionStore::with_vault_key_hex(&dir, KEY).expect("reader");
        for _ in 0..3 {
            assert_eq!(
                reader
                    .load_meta("pub1")
                    .await
                    .expect("load")
                    .platform("admob")
                    .state,
                Some(CredentialState::Rejected)
            );
        }
        let stats = reader.cache_stats();
        assert_eq!(stats.meta_reads, 1);
        assert_eq!(stats.meta_parses, 1);
        assert_eq!(stats.meta_hits, 2);

        meta.set_platform(
            "admob",
            PlatformState {
                state: Some(CredentialState::Validated),
                ..PlatformState::default()
            },
        );
        reader.save_meta("pub1", &meta).await.expect("save");
        assert_eq!(
            reader
                .load_meta("pub1")
                .await
                .expect("load")
                .platform("admob")
                .state,
            Some(CredentialState::Validated)
        );
        assert_eq!(
            reader.cache_stats().meta_reads,
            1,
            "the write primed the entry, so the next render does not reload"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn repeated_blob_metadata_reads_are_one_file_read() {
        let (store, dir) = temp_store("blob-cache");
        store.save_account("pub1", &account()).await.expect("save");
        let reader = VaultedConnectionStore::with_vault_key_hex(&dir, KEY).expect("reader");
        for _ in 0..3 {
            assert!(reader.blob_meta("pub1").await.expect("meta").is_some());
        }
        let stats = reader.cache_stats();
        assert_eq!(stats.blob_reads, 1);
        assert_eq!(stats.blob_hits, 2);
        let before = reader.cache_stats().blob_reads;
        reader.save_account("pub1", &account()).await.expect("save");
        assert!(reader.blob_meta("pub1").await.expect("meta").is_some());
        assert_eq!(
            reader.cache_stats().blob_reads,
            before,
            "a write refreshes the blob-metadata entry"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_store_debug_output_never_contains_a_secret() {
        let (store, dir) = temp_store("debug-secret");
        store.save_account("pub1", &account()).await.expect("save");
        assert!(store.load_account("pub1").await.expect("load").is_some());
        let rendered = format!("{store:?}");
        assert!(!rendered.contains("management-key"), "{rendered}");
        assert!(!rendered.contains("reporting-key"), "{rendered}");
        assert!(rendered.contains("ConnectionCache"), "{rendered}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn platform_bases_use_documented_hosts_and_env_hosts() {
        let lookup = |key: &str| match key {
            LEVELPLAY_BASE_ENV => Some("https://levelplay.example.test".to_string()),
            _ => None,
        };
        let (max_base, max_report) =
            platform_bases(PlatformId::AppLovinMax, &lookup).expect("max has documented hosts");
        assert_eq!(max_base, MAX_MANAGEMENT_BASE);
        assert_eq!(max_report.as_deref(), Some(MAX_REPORTING_BASE));
        let (levelplay, _) =
            platform_bases(PlatformId::LevelPlay, &lookup).expect("configured host");
        assert_eq!(levelplay, "https://levelplay.example.test");
        assert!(platform_bases(PlatformId::Admob, &lookup).is_none());
        assert!(platform_bases(PlatformId::Admob, &|_| Some("  ".to_string())).is_none());
    }
}
