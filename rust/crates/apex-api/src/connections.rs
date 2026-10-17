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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use apex_core::PlatformId;
use apex_vault::{entry_meta, Vault, VaultEntryMeta};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::live::LiveAccount;
use crate::sources::{contained_in, is_valid_publisher};

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

/// Shared inner state so blocking file work can move to the blocking pool.
#[derive(Debug)]
struct Inner {
    root: PathBuf,
    canonical_root: PathBuf,
    vault: Vault,
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
            }),
        })
    }

    /// The root directory.
    pub fn root(&self) -> &std::path::Path {
        &self.inner.root
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
            let blob = std::fs::read_to_string(&resolved)
                .map_err(|error| ApiError::Internal(format!("credential read: {error}")))?;
            let plaintext = inner
                .vault
                .decrypt(&blob)
                .map_err(|error| ApiError::Internal(format!("credential decrypt: {error}")))?;
            let account: LiveAccount = serde_json::from_str(&plaintext)
                .map_err(|error| ApiError::Internal(format!("credential parse: {error}")))?;
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
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&inner.root)
                .map_err(|error| ApiError::Internal(format!("credential root: {error}")))?;
            let root = inner
                .root
                .canonicalize()
                .map_err(|error| ApiError::Internal(format!("credential root: {error}")))?;
            let blob = inner.vault.encrypt(&plaintext);
            let target = root.join(format!("{publisher}.blob"));
            let tmp = root.join(format!("{publisher}.blob.tmp"));
            std::fs::write(&tmp, &blob)
                .map_err(|error| ApiError::Internal(format!("credential write: {error}")))?;
            std::fs::rename(&tmp, &target)
                .map_err(|error| ApiError::Internal(format!("credential rename: {error}")))?;
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
            let blob = std::fs::read_to_string(&resolved)
                .map_err(|error| ApiError::Internal(format!("credential read: {error}")))?;
            Ok(Some(entry_meta(&blob)))
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
            let raw = std::fs::read_to_string(&resolved)
                .map_err(|error| ApiError::Internal(format!("connection meta read: {error}")))?;
            serde_json::from_str(&raw)
                .map_err(|error| ApiError::Internal(format!("connection meta parse: {error}")))
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
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&inner.root)
                .map_err(|error| ApiError::Internal(format!("connection meta root: {error}")))?;
            let root = inner
                .root
                .canonicalize()
                .map_err(|error| ApiError::Internal(format!("connection meta root: {error}")))?;
            let target = root.join(format!("{publisher}{META_SUFFIX}"));
            let tmp = root.join(format!("{publisher}{META_SUFFIX}.tmp"));
            std::fs::write(&tmp, &raw)
                .map_err(|error| ApiError::Internal(format!("connection meta write: {error}")))?;
            std::fs::rename(&tmp, &target)
                .map_err(|error| ApiError::Internal(format!("connection meta rename: {error}")))?;
            Ok(())
        })
        .await
        .map_err(|join| ApiError::Internal(format!("connection meta task failed: {join}")))?
    }

    /// Remove one platform's credentials. Returns whether anything changed.
    /// An account with no remaining platforms keeps its blob (the entry is
    /// marked `unvalidated` and its fields cleared) so the removal is an
    /// observation, not a silent file deletion.
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
        }
        Ok(removed)
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
        let loaded = store
            .load_account("pub1")
            .await
            .expect("load")
            .expect("some");
        assert!(loaded.platforms.is_empty());
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
