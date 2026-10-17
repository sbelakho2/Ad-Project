//! The publisher registry: which publishers the runner serves, built from the
//! vault root the console writes.
//!
//! The console (`apex-api`) stores one vaulted account bundle per publisher at
//! `APEX_CREDENTIAL_ROOT/<publisher>.blob` (AES-256-GCM, see `apex-vault`) and
//! one non-secret observation record at
//! `<root>/<publisher>.connections.json` whose per-platform `state` says
//! whether a live read validated the credential. The runner reads the same
//! root (named `APEX_CONNECTIONS_ROOT` for the runner; `APEX_CREDENTIAL_ROOT`
//! is the apex-api spelling and stays readable) and builds its registry from
//! it:
//!
//! - a publisher whose meta records at least one `validated` platform and
//!   whose blob decrypts into usable keys is **runnable** — the runner builds
//!   the platform adapter from those vaulted keys;
//! - every other publisher is **skipped with a typed reason** that is surfaced
//!   in the run's human output and JSON report; a publisher that fails
//!   registration never touches a ledger and never bills.
//!
//! The blob is the same JSON `apex-api::live::LiveAccount` writes (snake_case
//! field names, `platforms` keyed by wire name); this module deserializes that
//! shared shape without depending on apex-api (which depends on this crate).
//! Only the fields the runner adapter needs are read; unknown fields are
//! ignored, so the console can extend the stored account without breaking the
//! runner.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use apex_vault::{Vault, VaultError};
use serde::Deserialize;

/// The runner's name for the vault root the console writes.
pub const VAULT_ROOT_ENV: &str = "APEX_CONNECTIONS_ROOT";
/// The apex-api spelling of the same vault root; read as a fallback so one
/// deployment variable serves both processes.
pub const VAULT_ROOT_APEX_API_ENV: &str = "APEX_CREDENTIAL_ROOT";
/// The vault key variable (64 lowercase hex characters).
pub const VAULT_KEY_ENV: &str = "APEX_VAULT_KEY_HEX";
/// The per-publisher ledger root in registry mode:
/// `<APEX_RUNNER_LEDGER_ROOT>/<publisher>.jsonl`, the same naming apex-api and
/// apex-control read.
pub const LEDGER_ROOT_ENV: &str = "APEX_RUNNER_LEDGER_ROOT";
/// The runner's name for the snapshot root in fleet mode: each snapshot lands
/// at `<root>/<publisher>/<platform>.json`, the path rule the console's
/// `JsonFileAuditSource` reads.
pub const SNAPSHOT_ROOT_ENV: &str = "APEX_RUNNER_SNAPSHOT";
/// The console's spelling of the same snapshot root (`apex-api`'s
/// `DEFAULT_SNAPSHOT_ROOT` default), read as a fallback so one deployment
/// variable serves both processes.
pub const SNAPSHOT_ROOT_APEX_API_ENV: &str = "APEX_SNAPSHOT_ROOT";
/// The relative snapshot root the console defaults to; the runner falls back
/// to it when neither spelling is configured.
pub const DEFAULT_SNAPSHOT_ROOT: &str = ".apex/snapshots";
/// The plain connection-metadata file suffix (written by apex-api).
pub const META_SUFFIX: &str = ".connections.json";
/// The encrypted account-bundle file suffix (written by apex-api).
pub const BLOB_SUFFIX: &str = ".blob";
/// The per-publisher customer billing profile the monthly pass reads:
/// `<root>/<publisher>.billing.json`, a `CustomerProfile` JSON document.
pub const CUSTOMER_PROFILE_SUFFIX: &str = ".billing.json";
/// The wire name of the platform with a full runner lifecycle adapter.
pub const MAX_WIRE: &str = "applovin_max";
/// The wire name of the LevelPlay runner adapter (groups + reporting).
pub const LEVELPLAY_WIRE: &str = "levelplay";
/// The wire name of the AdMob runner adapter (mediation A/B lifecycle; no
/// lift-fee eligibility).
pub const ADMOB_WIRE: &str = "admob";
/// The platform wire names a runner adapter exists for. A validated
/// connection for any other platform is skipped with the typed reason
/// instead of silently falling back to a different platform's keys.
pub const SUPPORTED_WIRES: [&str; 3] = [MAX_WIRE, LEVELPLAY_WIRE, ADMOB_WIRE];

/// Resolve the vault root from a configuration lookup: the runner spelling
/// first, then the apex-api spelling.
pub fn vault_root_from_env(lookup: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    for name in [VAULT_ROOT_ENV, VAULT_ROOT_APEX_API_ENV] {
        if let Some(root) = lookup(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            return Some(PathBuf::from(root));
        }
    }
    None
}

/// Resolve the snapshot root from a configuration lookup: the runner spelling
/// first, then the console spelling, then the console's relative default. The
/// fleet audit writes every snapshot under it and the fleet cycle reads each
/// publisher's own files back.
pub fn snapshot_root_from_env(lookup: &impl Fn(&str) -> Option<String>) -> PathBuf {
    for name in [SNAPSHOT_ROOT_ENV, SNAPSHOT_ROOT_APEX_API_ENV] {
        if let Some(root) = lookup(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            return PathBuf::from(root);
        }
    }
    PathBuf::from(DEFAULT_SNAPSHOT_ROOT)
}

/// The path one platform's snapshot lives at:
/// `<root>/<publisher>/<platform>.json`, the layout the console's
/// `JsonFileAuditSource` reads (one snapshot per publisher and platform).
pub fn snapshot_path(root: &Path, publisher: &str, platform_wire: &str) -> PathBuf {
    root.join(publisher).join(format!("{platform_wire}.json"))
}

/// Why one publisher is not served by this runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// No `<publisher>.connections.json` record exists, so nothing says the
    /// connection was ever validated.
    NoConnectionMetadata,
    /// The metadata record parses as JSON but not as a connection record.
    ConnectionMetaMalformed(String),
    /// The metadata records no platform in the `validated` state.
    NoValidatedConnection,
    /// The metadata says validated, but no `<publisher>.blob` exists.
    NoVaultedCredentials,
    /// The blob is present but the vault refused to decrypt it (wrong key or
    /// tampered ciphertext).
    CredentialDecryptFailed(VaultError),
    /// The decrypted bundle is not the stored account JSON shape, or it is
    /// missing the fields the adapter needs.
    CredentialMalformed(String),
    /// The validated platform has no full lifecycle adapter in the runner
    /// (AppLovin MAX, LevelPlay and AdMob have one).
    NoRunnerAdapter { platform: String },
    /// The validated MAX connection carries no reporting key, so the daily
    /// reporting pulls could never run.
    MissingReportKey,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::NoConnectionMetadata => f.write_str("no connection metadata recorded"),
            SkipReason::ConnectionMetaMalformed(detail) => {
                write!(f, "connection metadata is malformed: {detail}")
            }
            SkipReason::NoValidatedConnection => f.write_str("no validated platform connection"),
            SkipReason::NoVaultedCredentials => f.write_str("no vaulted credentials stored"),
            SkipReason::CredentialDecryptFailed(error) => {
                write!(f, "credential decrypt failed: {error}")
            }
            SkipReason::CredentialMalformed(detail) => {
                write!(f, "credential bundle malformed: {detail}")
            }
            SkipReason::NoRunnerAdapter { platform } => {
                write!(f, "no runner adapter for platform '{platform}'")
            }
            SkipReason::MissingReportKey => {
                f.write_str("the validated MAX connection has no reporting key")
            }
        }
    }
}

/// One platform key set as stored in the vaulted account bundle; the serde
/// shape mirrors `apex-api::live::LiveCredentials` field for field.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VaultedCredentials {
    /// MAX management key (or the platform's key/token).
    pub api_key: String,
    /// MAX account id (or the platform's account identifier).
    #[serde(default)]
    pub account_id: String,
    /// The platform API base url the console validated against.
    pub base_url: String,
    /// MAX Revenue Reporting key, when the connection carries one.
    #[serde(default)]
    pub report_key: Option<String>,
    /// MAX reporting base url; the documented host when absent.
    #[serde(default)]
    pub reporting_base_url: Option<String>,
    /// LevelPlay refresh token (unused by the MAX adapter).
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// The stored account bundle shape (the fields the runner reads).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VaultedAccount {
    /// The validated developer site (informational for the runner).
    #[serde(default)]
    pub developer_site: String,
    /// Networks the publisher should authorize in app-ads.txt.
    #[serde(default)]
    pub expected_networks: Vec<String>,
    /// Ad unit ids to audit on platforms with a per-unit waterfall.
    #[serde(default)]
    pub ad_unit_ids: Vec<String>,
    /// Bidder accounts as apex knows them, copied into every snapshot.
    #[serde(default)]
    pub bidder_accounts: Vec<apex_audit::BidderAccount>,
    /// Key sets keyed by platform wire name.
    #[serde(default)]
    pub platforms: BTreeMap<String, VaultedCredentials>,
}

/// A publisher the runner serves: its platform keys and the adapter platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnablePublisher {
    /// The publisher name (`^[A-Za-z0-9_-]{1,64}$`).
    pub publisher: String,
    /// The validated platform wire name the adapter is built for.
    pub platform: String,
    /// The vaulted key set for that platform.
    pub credentials: VaultedCredentials,
    /// Every supported platform whose key set the vault holds, keyed by wire
    /// name. The audit builds one snapshot per entry; the primary `platform`
    /// is always present.
    pub vaulted_platforms: BTreeMap<String, VaultedCredentials>,
    /// The developer site the app-ads.txt check reads.
    pub developer_site: String,
    /// Networks the publisher should authorize in app-ads.txt.
    pub expected_networks: Vec<String>,
    /// Ad unit ids the LevelPlay builder enumerates.
    pub ad_unit_ids: Vec<String>,
    /// Bidder accounts copied into every snapshot the audit builds.
    pub bidder_accounts: Vec<apex_audit::BidderAccount>,
}

impl RunnablePublisher {
    /// The platform wire names this publisher has vaulted snapshots for: the
    /// primary adapter platform first (so monitoring activation reads the
    /// adapter's own snapshot), then the rest in sorted order.
    pub fn snapshot_platforms(&self) -> Vec<String> {
        let mut wires: Vec<String> = self.vaulted_platforms.keys().cloned().collect();
        if let Some(index) = wires.iter().position(|wire| wire == &self.platform) {
            let primary = wires.remove(index);
            wires.insert(0, primary);
        }
        wires
    }
}

/// A publisher that failed registration, with its typed reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPublisher {
    /// The publisher name.
    pub publisher: String,
    /// Why this runner will not serve it.
    pub reason: SkipReason,
}

/// One registry entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryEntry {
    /// Runnable: validated connection plus decryptable keys. Boxed because a
    /// runnable publisher carries every vaulted key set and the account
    /// context, well beyond the skipped variant's size.
    Runnable(Box<RunnablePublisher>),
    /// Skipped, with a typed reason.
    Skipped(SkippedPublisher),
}

/// Why the registry root could not be read at all (a deployment problem, not
/// a per-publisher state).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// The root directory could not be listed.
    #[error("publisher registry root {path}: {reason}")]
    Io {
        /// The path involved.
        path: String,
        /// The OS report.
        reason: String,
    },
}

/// The publishers one runner serves, in stable (sorted) order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PublisherRegistry {
    entries: Vec<RegistryEntry>,
}

impl PublisherRegistry {
    /// Scan `root` for candidate publishers: every `*.blob` and every
    /// `*.connections.json` names one. A missing directory is an empty
    /// registry (a fresh deployment serves nobody yet); any other read
    /// failure is a typed error, never a silently empty fleet.
    pub fn load(root: &Path, vault: &Vault) -> Result<Self, RegistryError> {
        let candidates = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => {
                return Err(RegistryError::Io {
                    path: root.display().to_string(),
                    reason: error.to_string(),
                })
            }
        };
        let (mut names, _probes) = collect_publisher_names(candidates);
        names.sort();
        let entries = names
            .into_iter()
            .map(|publisher| register_one(root, &publisher, vault))
            .collect();
        Ok(Self { entries })
    }

    /// The registry entries in sorted publisher order.
    pub fn entries(&self) -> &[RegistryEntry] {
        &self.entries
    }

    /// The runnable publishers in sorted order.
    pub fn runnable(&self) -> impl Iterator<Item = &RunnablePublisher> {
        self.entries.iter().filter_map(|entry| match entry {
            RegistryEntry::Runnable(publisher) => Some(publisher.as_ref()),
            RegistryEntry::Skipped(_) => None,
        })
    }

    /// The skipped publishers in sorted order.
    pub fn skipped(&self) -> impl Iterator<Item = &SkippedPublisher> {
        self.entries.iter().filter_map(|entry| match entry {
            RegistryEntry::Runnable(_) => None,
            RegistryEntry::Skipped(publisher) => Some(publisher),
        })
    }

    /// Whether the registry holds exactly one runnable publisher and no
    /// skips: the single-publisher mode whose reports keep the committed
    /// shape.
    pub fn is_singleton(&self) -> bool {
        self.entries.len() == 1 && matches!(self.entries[0], RegistryEntry::Runnable(_))
    }
}

/// Collect the distinct valid publisher names from directory entries. The
/// hash set makes the duplicate check constant time per file; the returned
/// probe count is one per candidate name and backs the scan-equivalence test
/// (a growing-vector `contains` would add a comparison per prior name).
fn collect_publisher_names(
    entries: impl Iterator<Item = std::io::Result<std::fs::DirEntry>>,
) -> (Vec<String>, u64) {
    let mut names: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut probes = 0u64;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let publisher = if let Some(publisher) = name.strip_suffix(BLOB_SUFFIX) {
            Some(publisher.to_string())
        } else {
            name.strip_suffix(META_SUFFIX).map(str::to_string)
        };
        if let Some(publisher) = publisher {
            if is_valid_publisher(&publisher) {
                probes += 1;
                if seen.insert(publisher.clone()) {
                    names.push(publisher);
                }
            }
        }
    }
    (names, probes)
}

/// The publisher-name charset the whole product uses
/// (`^[A-Za-z0-9_-]{1,64}$`): the only file names derived from a registry.
fn is_valid_publisher(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Register one candidate: meta first (validation state), then the blob
/// (keys). The first refusal wins and is typed.
fn register_one(root: &Path, publisher: &str, vault: &Vault) -> RegistryEntry {
    let meta_path = root.join(format!("{publisher}{META_SUFFIX}"));
    let meta = match std::fs::read_to_string(&meta_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return skip(publisher, SkipReason::NoConnectionMetadata)
        }
        Err(error) => {
            // An unreadable meta is a data-state refusal, not a fleet failure:
            // this publisher is skipped with the visible reason.
            return skip(
                publisher,
                SkipReason::ConnectionMetaMalformed(format!("{meta_path:?}: {error}")),
            );
        }
    };
    let meta: ConnectionMetaFile = match serde_json::from_str(&meta) {
        Ok(meta) => meta,
        Err(error) => {
            return skip(
                publisher,
                SkipReason::ConnectionMetaMalformed(error.to_string()),
            )
        }
    };
    let validated: Vec<String> = meta
        .platforms
        .iter()
        .filter(|(_, state)| state.state.as_deref() == Some("validated"))
        .map(|(platform, _)| platform.clone())
        .collect();
    if validated.is_empty() {
        return skip(publisher, SkipReason::NoValidatedConnection);
    }
    let blob_path = root.join(format!("{publisher}{BLOB_SUFFIX}"));
    let blob = match std::fs::read_to_string(&blob_path) {
        Ok(blob) => blob,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return skip(publisher, SkipReason::NoVaultedCredentials)
        }
        Err(error) => {
            return skip(
                publisher,
                SkipReason::CredentialMalformed(format!("{blob_path:?}: {error}")),
            )
        }
    };
    let plaintext = match vault.decrypt(&blob) {
        Ok(plaintext) => plaintext,
        Err(error) => return skip(publisher, SkipReason::CredentialDecryptFailed(error)),
    };
    let account: VaultedAccount = match serde_json::from_str(&plaintext) {
        Ok(account) => account,
        Err(error) => {
            return skip(
                publisher,
                SkipReason::CredentialMalformed(error.to_string()),
            )
        }
    };
    // The platform with a full lifecycle adapter: AppLovin MAX, LevelPlay and
    // AdMob. The validated wire names are in sorted order, so the first
    // supported one is deterministic. A validated connection for any other
    // platform is skipped with its typed reason instead of silently falling
    // back to a different platform's keys.
    let Some(platform) = validated
        .iter()
        .find(|platform| SUPPORTED_WIRES.contains(&platform.as_str()))
    else {
        return skip(
            publisher,
            SkipReason::NoRunnerAdapter {
                platform: validated.first().cloned().unwrap_or_default(),
            },
        );
    };
    let Some(credentials) = account.platforms.get(platform.as_str()) else {
        return skip(publisher, SkipReason::NoVaultedCredentials);
    };
    if let Err(reason) = validate_platform_keys(platform, credentials) {
        return skip(publisher, reason);
    }
    // Every supported platform whose key set the vault holds gets its own
    // audit snapshot; the primary adapter platform is always among them. A
    // secondary key set that fails its own validation is simply not served a
    // snapshot rather than refusing the whole publisher.
    let mut vaulted_platforms: BTreeMap<String, VaultedCredentials> = BTreeMap::new();
    for wire in SUPPORTED_WIRES {
        if wire != platform.as_str() && !validated.iter().any(|validated| validated == wire) {
            continue;
        }
        let Some(key_set) = account.platforms.get(wire) else {
            continue;
        };
        if validate_platform_keys(wire, key_set).is_ok() {
            vaulted_platforms.insert(wire.to_string(), key_set.clone());
        }
    }
    // The primary key set is validated above, so it is always present; keeping
    // the insert unconditional makes that invariant explicit.
    vaulted_platforms.insert(platform.clone(), credentials.clone());
    RegistryEntry::Runnable(Box::new(RunnablePublisher {
        publisher: publisher.to_string(),
        platform: platform.clone(),
        credentials: credentials.clone(),
        vaulted_platforms,
        developer_site: account.developer_site.clone(),
        expected_networks: account.expected_networks.clone(),
        ad_unit_ids: account.ad_unit_ids.clone(),
        bidder_accounts: account.bidder_accounts.clone(),
    }))
}

/// Validate one platform's vaulted key set. The refusals are the typed skip
/// reasons the runner has always published; they are computed per platform so
/// a publisher with several key sets is judged on each of them.
fn validate_platform_keys(
    platform: &str,
    credentials: &VaultedCredentials,
) -> Result<(), SkipReason> {
    match platform {
        MAX_WIRE => {
            if credentials.api_key.trim().is_empty() || credentials.base_url.trim().is_empty() {
                return Err(SkipReason::CredentialMalformed(
                    "the MAX key set misses api_key or base_url".to_string(),
                ));
            }
            if credentials
                .report_key
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
            {
                return Err(SkipReason::MissingReportKey);
            }
        }
        LEVELPLAY_WIRE => {
            // The secret key travels in `api_key`, the app key in
            // `account_id` and the refresh token in `refresh_token`; all
            // three plus the validated base url are required.
            if credentials.api_key.trim().is_empty()
                || credentials.account_id.trim().is_empty()
                || credentials.base_url.trim().is_empty()
                || credentials
                    .refresh_token
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
            {
                return Err(SkipReason::CredentialMalformed(
                    "the LevelPlay key set misses the secret key, refresh token, app key or base url"
                        .to_string(),
                ));
            }
        }
        ADMOB_WIRE => {
            if credentials.api_key.trim().is_empty() || credentials.base_url.trim().is_empty() {
                return Err(SkipReason::CredentialMalformed(
                    "the AdMob key set misses the access token or base url".to_string(),
                ));
            }
            if let Err(error) =
                apex_connectors::admob::normalize_account_id(&credentials.account_id)
            {
                return Err(SkipReason::CredentialMalformed(format!(
                    "the AdMob key set {error}"
                )));
            }
        }
        other => {
            return Err(SkipReason::NoRunnerAdapter {
                platform: other.to_string(),
            })
        }
    }
    Ok(())
}

fn skip(publisher: &str, reason: SkipReason) -> RegistryEntry {
    RegistryEntry::Skipped(SkippedPublisher {
        publisher: publisher.to_string(),
        reason,
    })
}

/// The apex-api `ConnectionMeta` shape, reduced to the field the registry
/// reads: the per-platform `state`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionMetaFile {
    #[serde(default)]
    platforms: BTreeMap<String, PlatformStateFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformStateFile {
    #[serde(default)]
    state: Option<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const OTHER_KEY: &str = "1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100";

    fn dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("apex-registry-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn account_json(report_key: Option<&str>) -> String {
        serde_json::json!({
            "developer_site": "studio.example.com",
            "expected_networks": [],
            "ad_unit_ids": [],
            "bidder_accounts": [],
            "platforms": {
                "applovin_max": {
                    "api_key": "management-key",
                    "account_id": "acct-1",
                    "base_url": "https://api.example.test",
                    "report_key": report_key,
                    "reporting_base_url": "https://r.example.test",
                    "refresh_token": null
                }
            }
        })
        .to_string()
    }

    /// An account carrying one non-MAX platform key set.
    fn platform_account_json(platform: &str, credentials: serde_json::Value) -> String {
        serde_json::json!({
            "developer_site": "studio.example.com",
            "expected_networks": [],
            "ad_unit_ids": [],
            "bidder_accounts": [],
            "platforms": { platform: credentials }
        })
        .to_string()
    }

    fn write_blob(root: &Path, publisher: &str, key: &str, plaintext: &str) {
        let vault = Vault::from_hex_key(key).expect("key");
        std::fs::write(
            root.join(format!("{publisher}.blob")),
            vault.encrypt(plaintext),
        )
        .expect("blob");
    }

    fn write_meta(root: &Path, publisher: &str, platform: &str, state: &str) {
        let meta = serde_json::json!({
            "platforms": { platform: { "state": state, "credentialFields": ["api_key"] } }
        });
        std::fs::write(
            root.join(format!("{publisher}.connections.json")),
            serde_json::to_string_pretty(&meta).expect("meta json"),
        )
        .expect("meta");
    }

    #[test]
    fn a_publisher_with_two_validated_platforms_carries_both_key_sets() {
        let root = dir("multi-platform");
        let vault = Vault::from_hex_key(KEY).expect("key");
        let account = serde_json::json!({
            "developer_site": "studio.example.com",
            "expected_networks": ["google.com"],
            "ad_unit_ids": ["u1"],
            "bidder_accounts": [],
            "platforms": {
                "applovin_max": {
                    "api_key": "management-key",
                    "account_id": "acct-1",
                    "base_url": "https://api.example.test",
                    "report_key": "report-key",
                    "reporting_base_url": null,
                    "refresh_token": null
                },
                "levelplay": {
                    "api_key": "secret-key",
                    "account_id": "app-key-1",
                    "base_url": "https://platform.ironsrc.com",
                    "refresh_token": "refresh-token"
                }
            }
        });
        write_blob(&root, "studio", KEY, &account.to_string());
        let meta = serde_json::json!({
            "platforms": {
                "applovin_max": { "state": "validated" },
                "levelplay": { "state": "validated" }
            }
        });
        std::fs::write(root.join("studio.connections.json"), meta.to_string()).expect("meta");

        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        let runnable: Vec<&RunnablePublisher> = registry.runnable().collect();
        assert_eq!(runnable.len(), 1);
        // Alphabetical validated order picks applovin_max first.
        assert_eq!(runnable[0].platform, MAX_WIRE);
        let platforms: Vec<String> = runnable[0].snapshot_platforms();
        assert_eq!(
            platforms,
            vec![MAX_WIRE.to_string(), LEVELPLAY_WIRE.to_string()],
            "the primary platform leads"
        );
        assert_eq!(runnable[0].vaulted_platforms.len(), 2);
        assert_eq!(runnable[0].developer_site, "studio.example.com");
        assert_eq!(
            runnable[0].expected_networks,
            vec!["google.com".to_string()]
        );
        assert_eq!(runnable[0].ad_unit_ids, vec!["u1".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn validated_publishers_load_with_their_keys() {
        let root = dir("ok");
        let vault = Vault::from_hex_key(KEY).expect("key");
        write_blob(&root, "northstar", KEY, &account_json(Some("report-key")));
        write_meta(&root, "northstar", "applovin_max", "validated");

        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        assert!(registry.is_singleton());
        let runnable: Vec<&RunnablePublisher> = registry.runnable().collect();
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].publisher, "northstar");
        assert_eq!(runnable[0].platform, MAX_WIRE);
        assert_eq!(runnable[0].credentials.api_key, "management-key");
        assert_eq!(
            runnable[0].credentials.report_key.as_deref(),
            Some("report-key")
        );
        assert_eq!(registry.skipped().count(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_registration_refusal_is_typed() {
        let root = dir("refusals");
        let vault = Vault::from_hex_key(KEY).expect("key");

        // A blob with no meta at all.
        write_blob(&root, "nometa", KEY, &account_json(Some("r")));
        // A meta with no validated platform.
        write_blob(&root, "unvalidated", KEY, &account_json(Some("r")));
        write_meta(&root, "unvalidated", "applovin_max", "unvalidated");
        // A validated meta with no blob.
        write_meta(&root, "noblob", "applovin_max", "validated");
        // A validated meta, a blob under a different vault key.
        write_blob(&root, "wrongkey", OTHER_KEY, &account_json(Some("r")));
        write_meta(&root, "wrongkey", "applovin_max", "validated");
        // A validated meta, a blob that is not the account shape.
        write_blob(&root, "garbage", KEY, "not json");
        write_meta(&root, "garbage", "applovin_max", "validated");
        // A validated platform with no runner adapter.
        write_blob(
            &root,
            "unsupported",
            KEY,
            &platform_account_json(
                "unity_ads",
                serde_json::json!({
                    "api_key": "k",
                    "account_id": "a",
                    "base_url": "https://api.example.test"
                }),
            ),
        );
        write_meta(&root, "unsupported", "unity_ads", "validated");
        // A validated MAX connection without a reporting key.
        write_blob(&root, "noreport", KEY, &account_json(None));
        write_meta(&root, "noreport", "applovin_max", "validated");

        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        assert!(!registry.is_singleton());
        assert_eq!(registry.runnable().count(), 0);
        let reasons: Vec<(String, String)> = registry
            .skipped()
            .map(|skip| (skip.publisher.clone(), skip.reason.to_string()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                (
                    "garbage".to_string(),
                    "credential bundle malformed: expected ident at line 1 column 2".to_string()
                ),
                (
                    "noblob".to_string(),
                    "no vaulted credentials stored".to_string()
                ),
                (
                    "nometa".to_string(),
                    "no connection metadata recorded".to_string()
                ),
                (
                    "noreport".to_string(),
                    "the validated MAX connection has no reporting key".to_string()
                ),
                (
                    "unsupported".to_string(),
                    "no runner adapter for platform 'unity_ads'".to_string()
                ),
                (
                    "unvalidated".to_string(),
                    "no validated platform connection".to_string()
                ),
                (
                    "wrongkey".to_string(),
                    "credential decrypt failed: decryption failed (wrong key or tampered ciphertext)"
                        .to_string()
                ),
            ],
            "{reasons:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_validated_levelplay_connection_yields_the_levelplay_adapter() {
        let root = dir("levelplay");
        let vault = Vault::from_hex_key(KEY).expect("key");
        write_blob(
            &root,
            "studio",
            KEY,
            &platform_account_json(
                "levelplay",
                serde_json::json!({
                    "api_key": "secret-key",
                    "account_id": "app-key-1",
                    "base_url": "https://platform.ironsrc.com",
                    "refresh_token": "refresh-token"
                }),
            ),
        );
        write_meta(&root, "studio", "levelplay", "validated");

        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        assert!(registry.is_singleton());
        let runnable: Vec<&RunnablePublisher> = registry.runnable().collect();
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].publisher, "studio");
        assert_eq!(runnable[0].platform, LEVELPLAY_WIRE);
        assert_eq!(runnable[0].credentials.api_key, "secret-key");
        assert_eq!(runnable[0].credentials.account_id, "app-key-1");
        assert_eq!(
            runnable[0].credentials.refresh_token.as_deref(),
            Some("refresh-token")
        );
        assert_eq!(registry.skipped().count(), 0);

        // A LevelPlay connection without the refresh token is refused typed.
        let root = dir("levelplay-norefresh");
        write_blob(
            &root,
            "studio",
            KEY,
            &platform_account_json(
                "levelplay",
                serde_json::json!({
                    "api_key": "secret-key",
                    "account_id": "app-key-1",
                    "base_url": "https://platform.ironsrc.com",
                    "refresh_token": null
                }),
            ),
        );
        write_meta(&root, "studio", "levelplay", "validated");
        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        let skipped: Vec<&SkippedPublisher> = registry.skipped().collect();
        assert_eq!(skipped.len(), 1);
        match &skipped[0].reason {
            SkipReason::CredentialMalformed(detail) => {
                assert!(detail.contains("refresh token"), "{detail}");
            }
            other => panic!("expected a typed credential refusal, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_validated_admob_connection_yields_the_admob_adapter() {
        let root = dir("admob");
        let vault = Vault::from_hex_key(KEY).expect("key");
        write_blob(
            &root,
            "studio",
            KEY,
            &platform_account_json(
                "admob",
                serde_json::json!({
                    "api_key": "access-token",
                    "account_id": "accounts/pub-1234567890",
                    "base_url": "https://admob.googleapis.com"
                }),
            ),
        );
        write_meta(&root, "studio", "admob", "validated");

        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        let runnable: Vec<&RunnablePublisher> = registry.runnable().collect();
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].platform, ADMOB_WIRE);
        assert_eq!(runnable[0].credentials.api_key, "access-token");

        // An invalid AdMob account id is a typed credential refusal.
        let root = dir("admob-badaccount");
        write_blob(
            &root,
            "studio",
            KEY,
            &platform_account_json(
                "admob",
                serde_json::json!({
                    "api_key": "access-token",
                    "account_id": "pub-12",
                    "base_url": "https://admob.googleapis.com"
                }),
            ),
        );
        write_meta(&root, "studio", "admob", "validated");
        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        let skipped: Vec<&SkippedPublisher> = registry.skipped().collect();
        assert_eq!(skipped.len(), 1);
        match &skipped[0].reason {
            SkipReason::CredentialMalformed(detail) => {
                assert!(detail.contains("invalid AdMob account id"), "{detail}");
            }
            other => panic!("expected a typed credential refusal, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn malformed_metadata_is_a_typed_skip_and_a_missing_root_is_empty() {
        let root = dir("meta");
        let vault = Vault::from_hex_key(KEY).expect("key");
        std::fs::write(root.join("broken.connections.json"), "{ not json").expect("write");
        write_blob(&root, "broken", KEY, &account_json(Some("r")));

        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        let skipped: Vec<&SkippedPublisher> = registry.skipped().collect();
        assert_eq!(skipped.len(), 1);
        assert!(matches!(
            skipped[0].reason,
            SkipReason::ConnectionMetaMalformed(_)
        ));

        let missing = PublisherRegistry::load(&root.join("absent"), &vault).expect("missing dir");
        assert!(missing.entries().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The pre-fix scan, verbatim: a growing vector and a linear `contains`
    /// per candidate. Returns the registry and the name comparisons it made.
    fn naive_load(root: &Path, vault: &Vault) -> (PublisherRegistry, u64) {
        let mut names: Vec<String> = Vec::new();
        let mut comparisons = 0u64;
        for entry in std::fs::read_dir(root).expect("read").flatten() {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            let publisher = if let Some(publisher) = name.strip_suffix(BLOB_SUFFIX) {
                Some(publisher.to_string())
            } else {
                name.strip_suffix(META_SUFFIX).map(str::to_string)
            };
            if let Some(publisher) = publisher {
                if is_valid_publisher(&publisher) {
                    comparisons += names.len() as u64;
                    if !names.contains(&publisher) {
                        names.push(publisher);
                    }
                }
            }
        }
        names.sort();
        let entries = names
            .into_iter()
            .map(|publisher| register_one(root, &publisher, vault))
            .collect();
        (PublisherRegistry { entries }, comparisons)
    }

    #[test]
    fn ten_thousand_files_scan_with_a_hash_index_and_match_the_naive_reference() {
        let root = dir("ten-thousand");
        let vault = Vault::from_hex_key(KEY).expect("key");
        for index in 0..10_000 {
            std::fs::write(
                root.join(format!("pub{index:05}.connections.json")),
                r#"{"platforms":{}}"#,
            )
            .expect("meta");
        }

        let (names, probes) = collect_publisher_names(std::fs::read_dir(&root).expect("read"));
        assert_eq!(probes, 10_000, "one hash-index probe per candidate file");
        assert_eq!(names.len(), 10_000, "every publisher is distinct");

        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        let (naive, comparisons) = naive_load(&root, &vault);
        assert_eq!(registry, naive, "the scan must keep the same entries");
        assert_eq!(registry.entries().len(), 10_000);
        assert_eq!(registry.skipped().count(), 10_000);
        assert_eq!(
            comparisons,
            10_000 * 9_999 / 2,
            "the pre-fix scan compares every prior name"
        );
        println!(
            "files=10000 index_probes={probes} naive_comparisons={comparisons} entries={}",
            registry.entries().len()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hostile_file_names_never_enter_the_registry() {
        let root = dir("hostile");
        let vault = Vault::from_hex_key(KEY).expect("key");
        let long = "x".repeat(65);
        for name in ["a b", "näme", long.as_str(), ".hidden"] {
            std::fs::write(root.join(format!("{name}.blob")), "x").expect("write");
        }
        let registry = PublisherRegistry::load(&root, &vault).expect("load");
        assert!(registry.entries().is_empty(), "{:?}", registry.entries());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn both_vault_root_spellings_resolve() {
        let lookup = |name: &str| match name {
            VAULT_ROOT_APEX_API_ENV => Some(" /apex/credentials ".to_string()),
            _ => None,
        };
        assert_eq!(
            vault_root_from_env(&lookup),
            Some(PathBuf::from("/apex/credentials"))
        );
        let both = |name: &str| match name {
            VAULT_ROOT_ENV => Some("/connections".to_string()),
            VAULT_ROOT_APEX_API_ENV => Some("/credentials".to_string()),
            _ => None,
        };
        assert_eq!(
            vault_root_from_env(&both),
            Some(PathBuf::from("/connections"))
        );
        assert_eq!(vault_root_from_env(&|_| None), None);
        assert_eq!(vault_root_from_env(&|_| Some("  ".to_string())), None);
    }
}
