//! Configuration: the `APEX_CONTROL_*` environment namespace, the operator
//! file and the deployment environment chip.
//!
//! The control plane reads only its own namespace plus the analytics store
//! address; it never reads publisher-console variables. Every value is parsed
//! once here so the binary stays a thin `main`.
//!
//! Variables (all optional except that signing in requires operators):
//!
//! - `APEX_CONTROL_BIND` — listen address, default `127.0.0.1:8790`;
//! - `APEX_CONTROL_OPERATORS` — path to the operator configuration JSON file
//!   (`[{id, name, role, password_argon2id_phc, totp_secret_base32?}]`).
//!   When the file is absent the service starts in a clearly logged
//!   unconfigured state where `/control/login` explains that configuration is
//!   required and no session can be minted; there is never a default
//!   password;
//! - `APEX_ENVIRONMENT` — the environment chip, `PRODUCTION` or `TEST`
//!   (default `PRODUCTION`; unknown labels are upper-cased and bounded);
//! - `APEX_CONTROL_JOURNAL` — append-only action journal, default
//!   `.apex/control/journal.jsonl`;
//! - `APEX_OPERATOR_JOBS_DIR` — durable operator job directory (the local,
//!   single-host mode), default `.apex/control/jobs`;
//! - `APEX_API_JOBS_URL` + `APEX_OPERATOR_JOBS_TOKEN` — the split-host
//!   operator-jobs endpoint: when both are set the control plane POSTs each
//!   job to the apex-api receiver and reads job state back through
//!   `GET /v1/operator/jobs/{job_id}` instead of writing the local folder;
//!   when the URL is unset (or only the token is set, which single-host
//!   deployments share with the API's own environment) the local folder
//!   write is used exactly as before. A URL without the token refuses
//!   startup, so the control plane can never POST without authenticating.
//! - `APEX_CONTROL_SYNC_MARKER` — the sync-timer marker file the system page
//!   reads for the copied-ledger age; unset or absent is the honest "sync age
//!   not recorded" state;
//! - `APEX_CONTROL_LEDGER_ROOT` — read-only verified ledger root, default
//!   `.apex/ledgers` (`<publisher>.jsonl`), the durable file the runner
//!   keeps;
//! - `APEX_CONTROL_LEDGER_KEYS` (or the shared `APEX_LEDGER_KEYS`) — the
//!   comma-separated `publisher=<64-hex-ed25519-pubkey>` allow-list the
//!   verified reader checks each chain against; a publisher without a key is
//!   never served;
//! - `APEX_RUNNER_SNAPSHOT` (or `APEX_SNAPSHOT_ROOT`) — the runner snapshot
//!   root (`<publisher>/<platform>.json`), default `.apex/snapshots`;
//! - `APEX_NOTIFY_SINK` — the notification sink label shown as a system
//!   component when configured;
//! - `APEX_METRICS_TEXTFILE` — the textfile metrics export path shown as a
//!   system component;
//! - `APEX_CONTROL_INVOICE_REGISTER` — read-only invoice register JSONL,
//!   default `.apex/billing/invoices.jsonl` (one serialized invoice per
//!   line, camelCase, the shape `apex-billing` produces);
//! - `APEX_CONTROL_TOTP_STATE` — the authenticator replay-protection state
//!   file, default `.apex/control/totp-steps.json`. The highest accepted time
//!   step per operator is loaded at startup and rewritten atomically (mode
//!   0600, temp file + rename) on every claim, so a restart cannot accept a
//!   spent code again. An unreadable or malformed file fails closed: every
//!   code is refused until the file is fixed or removed;
//! - `APEX_CONTROL_INTERVENTIONS` — the append-only fleet pause/resume
//!   register, default `.apex/control/interventions.jsonl`; the runner reads
//!   the same file before each platform's cycle;
//! - `APEX_EFFECTIVE_POLICY` — the effective `AutopilotPolicy` file the
//!   `policy_set` capability writes, default
//!   `.apex/control/effective-policy.json`; the runner and console read the
//!   same path;
//! - `APEX_CONTROL_CONNECTIONS_ROOT` — the vault/connections root holding
//!   publisher plan state (`<publisher>.<plan>.json`), falling back to
//!   `APEX_CONNECTIONS_ROOT` then `APEX_CREDENTIAL_ROOT`; default
//!   `.apex/credentials`;
//! - `APEX_CONTROL_OPERATIONS` — the verified-operation evidence file,
//!   default `.apex/control/operations.json`; without it the operation
//!   matrix shows every action as unavailable ("Missing evidence means the
//!   action remains unavailable", page 72);
//! - `APEX_CLICKHOUSE_HTTP`, `APEX_CLICKHOUSE_DB` — the analytics store the
//!   read models come from; `APEX_CLICKHOUSE_USER` / `APEX_CLICKHOUSE_PASSWORD`
//!   are the optional HTTP credentials, and `https://` in the store URL
//!   enables TLS. Without the HTTP/DB pair every queue reports the honest
//!   not-available state with the reason.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use apex_analytics::ClickHouseConfig;

use crate::capabilities::Role;
use crate::jobs::RemoteJobsConfig;

/// Default listen address of the operator origin.
pub const DEFAULT_BIND: &str = "127.0.0.1:8790";
/// Default action journal path.
pub const DEFAULT_JOURNAL: &str = ".apex/control/journal.jsonl";
/// Default operator job directory.
pub const DEFAULT_JOBS_DIR: &str = ".apex/control/jobs";
/// Default read-only ledger root.
pub const DEFAULT_LEDGER_ROOT: &str = ".apex/ledgers";
/// Default runner snapshot root (`<publisher>/<platform>.json`).
pub const DEFAULT_SNAPSHOT_ROOT: &str = ".apex/snapshots";
/// The verified public-key allow-list for the ledger root, shared with
/// `apex-api`: comma-separated `publisher=<64-hex-ed25519-pubkey>`.
pub const LEDGER_KEYS_ENV: &str = "APEX_LEDGER_KEYS";
/// The control-plane spelling of the same allow-list, preferred when set.
pub const CONTROL_LEDGER_KEYS_ENV: &str = "APEX_CONTROL_LEDGER_KEYS";
/// Notification sink label (`email`, `webhook`, ...), shown as a component.
pub const NOTIFY_SINK_ENV: &str = "APEX_NOTIFY_SINK";
/// The textfile metrics export path, shown as a component.
pub const METRICS_TEXTFILE_ENV: &str = "APEX_METRICS_TEXTFILE";
/// Default read-only invoice register path.
pub const DEFAULT_INVOICE_REGISTER: &str = ".apex/billing/invoices.jsonl";
/// Default TOTP replay-protection state file.
pub const DEFAULT_TOTP_STATE: &str = ".apex/control/totp-steps.json";
/// Default verified-operation evidence path.
pub const DEFAULT_OPERATIONS: &str = ".apex/control/operations.json";
/// Default policy drafts register.
pub const DEFAULT_POLICIES: &str = ".apex/control/policies.jsonl";
/// Default fleet intervention register.
pub const DEFAULT_INTERVENTIONS: &str = ".apex/control/interventions.jsonl";
/// Environment variable naming the effective policy file the runner reads.
pub const EFFECTIVE_POLICY_ENV: &str = "APEX_EFFECTIVE_POLICY";
/// Default effective policy file (the same relative path the runner sample
/// documents).
pub const DEFAULT_EFFECTIVE_POLICY: &str = ".apex/control/effective-policy.json";
/// Default vault/connections root holding publisher plan state.
pub const DEFAULT_CONNECTIONS_ROOT: &str = ".apex/credentials";
/// Longest accepted environment label.
pub const MAX_ENVIRONMENT_LEN: usize = 32;

/// Configuration failures that stop the binary before it serves anything.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The bind address is not a socket address.
    #[error("APEX_CONTROL_BIND is not a socket address: {0}")]
    Bind(String),
    /// The operator file could not be read.
    #[error("operator configuration could not be read: {0}")]
    OperatorsRead(String),
    /// The operator file is not valid JSON of the expected shape.
    #[error("operator configuration is not valid: {0}")]
    OperatorsInvalid(String),
    /// The analytics store configuration is invalid.
    #[error("analytics store configuration is invalid: {0}")]
    Analytics(String),
    /// The ledger public-key allow-list is invalid.
    #[error("APEX_LEDGER_KEYS is invalid: {0}")]
    LedgerKeys(String),
    /// The remote operator-jobs base URL is not usable.
    #[error("APEX_API_JOBS_URL is not a valid http/https URL: {0}")]
    JobsUrl(String),
    /// The remote operator-jobs endpoint needs its bearer token.
    #[error("APEX_API_JOBS_URL requires APEX_OPERATOR_JOBS_TOKEN to be set")]
    JobsPair,
}

/// The deployment configuration of one control-plane process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlConfig {
    /// Listen address of the operator origin.
    pub bind: SocketAddr,
    /// Operator configuration file; `None` means the unconfigured state.
    pub operators_path: Option<PathBuf>,
    /// Append-only action journal path.
    pub journal_path: PathBuf,
    /// Durable operator job directory (single-host mode).
    pub jobs_dir: PathBuf,
    /// The split-host jobs endpoint, when `APEX_API_JOBS_URL` and
    /// `APEX_OPERATOR_JOBS_TOKEN` are both set.
    pub jobs_remote: Option<RemoteJobsConfig>,
    /// The sync-timer marker file behind the system page's copied-ledger
    /// age, when `APEX_CONTROL_SYNC_MARKER` is set.
    pub sync_marker: Option<PathBuf>,
    /// The environment chip label (`PRODUCTION`, `TEST`, or a bounded
    /// deployment-specific label).
    pub environment: String,
    /// Read-only ledger root.
    pub ledger_root: PathBuf,
    /// Verified public keys for the publisher ledgers. A publisher without a
    /// key is never served: unverified lines are not trusted.
    pub ledger_keys: std::collections::HashMap<String, apex_proof::VerifyingKey>,
    /// The runner snapshot root (`<publisher>/<platform>.json`).
    pub snapshot_root: PathBuf,
    /// The notification sink label, when configured.
    pub notify_sink: Option<String>,
    /// The metrics textfile export path, when configured.
    pub metrics_textfile: Option<PathBuf>,
    /// Read-only invoice register path.
    pub invoice_register: PathBuf,
    /// TOTP replay-protection state file (`APEX_CONTROL_TOTP_STATE`).
    pub totp_state_path: PathBuf,
    /// Verified-operation evidence path.
    pub operations_path: PathBuf,
    /// Policy drafts register path.
    pub policies_path: PathBuf,
    /// Fleet intervention register path.
    pub interventions_path: PathBuf,
    /// Effective policy file written by the `policy_set` capability and read
    /// by the runner in preference to its inline policy.
    pub effective_policy_path: PathBuf,
    /// The vault/connections root holding `<publisher>.<plan>.json` state
    /// files (the same contract the runner and console read).
    pub connections_root: PathBuf,
    /// Analytics store settings; `None` means no read models configured.
    pub clickhouse: Option<ClickHouseConfig>,
}

impl ControlConfig {
    /// Tighten the control plane's private files to mode `0600`.
    ///
    /// The append paths create new files owner-only, but a file provisioned
    /// by an older build (or by an operator with a permissive umask) keeps
    /// its old mode. The journal, operator file (TOTP secrets), replay state,
    /// policy/intervention registers, effective policy, job files and plan
    /// state are all restricted records. Startup tightens their mode and
    /// refuses to run if a file cannot be inspected or restricted.
    pub fn restrict_private_files(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut paths: Vec<PathBuf> = vec![
                self.journal_path.clone(),
                self.totp_state_path.clone(),
                self.operations_path.clone(),
                self.policies_path.clone(),
                self.interventions_path.clone(),
                self.effective_policy_path.clone(),
            ];
            if let Some(path) = &self.operators_path {
                paths.push(path.clone());
            }
            if let Ok(entries) = std::fs::read_dir(&self.jobs_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|value| value.to_str()) == Some("json") {
                        paths.push(path);
                    }
                }
            }
            if let Ok(entries) = std::fs::read_dir(&self.connections_root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|value| value.to_str()) == Some("json") {
                        paths.push(path);
                    }
                }
            }
            for path in paths {
                match std::fs::metadata(&path) {
                    Ok(metadata) => {
                        let mode = metadata.permissions().mode() & 0o777;
                        if mode & 0o077 != 0 {
                            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                                .map_err(|error| {
                                    format!(
                                    "the private file {} could not be restricted to 0600: {error}",
                                    path.display()
                                )
                                })?;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(format!(
                            "the private file {} could not be inspected: {error}",
                            path.display()
                        ))
                    }
                }
            }
        }
        Ok(())
    }

    /// Parse from environment-like lookups.
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let bind = lookup("APEX_CONTROL_BIND")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BIND.to_string())
            .parse::<SocketAddr>()
            .map_err(|error| ConfigError::Bind(error.to_string()))?;
        let operators_path = lookup("APEX_CONTROL_OPERATORS")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let journal_path = lookup("APEX_CONTROL_JOURNAL")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_JOURNAL));
        let jobs_dir = lookup("APEX_OPERATOR_JOBS_DIR")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_JOBS_DIR));
        let jobs_remote = match (
            lookup(crate::jobs::JOBS_URL_ENV).filter(|value| !value.trim().is_empty()),
            lookup(crate::jobs::JOBS_TOKEN_ENV).filter(|value| !value.trim().is_empty()),
        ) {
            // Both unset (or only the token, which single-host deployments
            // share with the API's own environment): the local folder.
            (None, _) => None,
            // Half the pair is a misconfiguration: never POST without auth.
            (Some(_), None) => return Err(ConfigError::JobsPair),
            (Some(url), Some(token)) => {
                let url = url.trim();
                let parsed = reqwest::Url::parse(url)
                    .map_err(|error| ConfigError::JobsUrl(error.to_string()))?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    return Err(ConfigError::JobsUrl(
                        "the scheme must be http or https".to_string(),
                    ));
                }
                Some(RemoteJobsConfig {
                    url: url.to_string(),
                    token,
                })
            }
        };
        let sync_marker = lookup("APEX_CONTROL_SYNC_MARKER")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let environment = normalize_environment(
            lookup("APEX_ENVIRONMENT")
                .filter(|value| !value.trim().is_empty())
                .as_deref(),
        );
        let ledger_root = lookup("APEX_CONTROL_LEDGER_ROOT")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_LEDGER_ROOT));
        let ledger_keys_raw = lookup(CONTROL_LEDGER_KEYS_ENV)
            .filter(|value| !value.trim().is_empty())
            .or_else(|| lookup(LEDGER_KEYS_ENV).filter(|value| !value.trim().is_empty()));
        let ledger_keys = match ledger_keys_raw {
            Some(raw) => parse_ledger_keys(&raw).map_err(ConfigError::LedgerKeys)?,
            None => std::collections::HashMap::new(),
        };
        let snapshot_root = lookup("APEX_RUNNER_SNAPSHOT")
            .filter(|value| !value.trim().is_empty())
            .or_else(|| lookup("APEX_SNAPSHOT_ROOT").filter(|value| !value.trim().is_empty()))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_SNAPSHOT_ROOT));
        let notify_sink = lookup(NOTIFY_SINK_ENV)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let metrics_textfile = lookup(METRICS_TEXTFILE_ENV)
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let invoice_register = lookup("APEX_CONTROL_INVOICE_REGISTER")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_INVOICE_REGISTER));
        let totp_state_path = lookup("APEX_CONTROL_TOTP_STATE")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TOTP_STATE));
        let operations_path = lookup("APEX_CONTROL_OPERATIONS")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_OPERATIONS));
        let policies_path = lookup("APEX_CONTROL_POLICIES")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_POLICIES));
        let interventions_path = lookup("APEX_CONTROL_INTERVENTIONS")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_INTERVENTIONS));
        let effective_policy_path = lookup(EFFECTIVE_POLICY_ENV)
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_EFFECTIVE_POLICY));
        // The control plane writes the same plan-state files the runner and
        // console read. Prefer its own name, then the two deployment
        // spellings for the vault root.
        let connections_root = lookup("APEX_CONTROL_CONNECTIONS_ROOT")
            .filter(|value| !value.trim().is_empty())
            .or_else(|| lookup("APEX_CONNECTIONS_ROOT").filter(|value| !value.trim().is_empty()))
            .or_else(|| lookup("APEX_CREDENTIAL_ROOT").filter(|value| !value.trim().is_empty()))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONNECTIONS_ROOT));
        let clickhouse = match (
            lookup("APEX_CLICKHOUSE_HTTP").filter(|value| !value.trim().is_empty()),
            lookup("APEX_CLICKHOUSE_DB").filter(|value| !value.trim().is_empty()),
        ) {
            (None, None) => None,
            (http, database) => {
                let mut config = ClickHouseConfig::default();
                if let Some(http) = http {
                    config.http_url = http;
                }
                if let Some(database) = database {
                    config.database = database;
                }
                config
                    .validate()
                    .map_err(|error| ConfigError::Analytics(error.to_string()))?;
                Some(config)
            }
        };
        Ok(Self {
            bind,
            operators_path,
            journal_path,
            jobs_dir,
            jobs_remote,
            sync_marker,
            environment,
            ledger_root,
            ledger_keys,
            snapshot_root,
            notify_sink,
            metrics_textfile,
            invoice_register,
            totp_state_path,
            operations_path,
            policies_path,
            interventions_path,
            effective_policy_path,
            connections_root,
            clickhouse,
        })
    }

    /// Parse from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(&|key| std::env::var(key).ok())
    }

    /// Whether the operator file is configured at all.
    pub fn is_configured(&self) -> bool {
        self.operators_path.is_some()
    }
}

/// Normalize the environment chip: `production` and `test` map to their
/// upper-case canonical labels, anything else is upper-cased and bounded.
fn normalize_environment(raw: Option<&str>) -> String {
    let value = raw.unwrap_or("PRODUCTION").trim();
    if value.eq_ignore_ascii_case("production") {
        return "PRODUCTION".to_string();
    }
    if value.eq_ignore_ascii_case("test") {
        return "TEST".to_string();
    }
    let upper: String = value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_ENVIRONMENT_LEN)
        .collect::<String>()
        .to_ascii_uppercase();
    if upper.trim().is_empty() {
        "PRODUCTION".to_string()
    } else {
        upper
    }
}

/// One operator entry from `APEX_CONTROL_OPERATORS`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperatorConfig {
    /// Stable operator identity (`^[a-z0-9][a-z0-9_-]{0,31}$`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Role name (`observer`, `support`, `operator`, `finance`, `security`).
    pub role: String,
    /// Argon2id PHC string; the only stored form of the password.
    pub password_argon2id_phc: String,
    /// Optional RFC 4648 base32 TOTP secret. An operator without one cannot
    /// pass the second factor, so no session can be issued for them.
    #[serde(default)]
    pub totp_secret_base32: Option<String>,
}

/// The validated operator directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operators {
    entries: Vec<ValidatedOperator>,
    by_id: HashMap<String, usize>,
}

/// One validated operator: the configuration entry plus its decoded secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedOperator {
    /// Configuration entry.
    pub config: OperatorConfig,
    /// Effective role.
    pub role: Role,
    /// Decoded TOTP secret bytes; `None` when the entry has no secret.
    pub totp_secret: Option<Vec<u8>>,
}

impl Operators {
    /// The empty directory (the unconfigured state).
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            by_id: HashMap::new(),
        }
    }

    /// Load and validate the operator file. Every entry is checked: identity
    /// shape and uniqueness, role name, argon2id PHC shape and, when present,
    /// a decodable base32 secret. A single invalid entry refuses the load,
    /// so a half-valid directory never serves.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| ConfigError::OperatorsRead(format!("{}: {error}", path.display())))?;
        let entries: Vec<OperatorConfig> = serde_json::from_str(&raw)
            .map_err(|error| ConfigError::OperatorsInvalid(error.to_string()))?;
        Self::from_entries(entries)
    }

    /// Validate a parsed entry list.
    pub fn from_entries(entries: Vec<OperatorConfig>) -> Result<Self, ConfigError> {
        let mut validated = Vec::with_capacity(entries.len());
        let mut by_id = HashMap::new();
        for entry in entries {
            let id = entry.id.trim().to_string();
            if !valid_operator_id(&id) {
                return Err(ConfigError::OperatorsInvalid(format!(
                    "operator id {:?} must match ^[a-z0-9][a-z0-9_-]{{0,31}}$",
                    entry.id
                )));
            }
            if by_id.contains_key(&id) {
                return Err(ConfigError::OperatorsInvalid(format!(
                    "operator {id:?} is listed twice"
                )));
            }
            let name = entry.name.trim().to_string();
            if name.is_empty() || name.chars().count() > 80 {
                return Err(ConfigError::OperatorsInvalid(format!(
                    "operator {id:?} needs a display name of 1 to 80 characters"
                )));
            }
            let role = Role::parse(&entry.role).ok_or_else(|| {
                ConfigError::OperatorsInvalid(format!(
                    "operator {id:?} has unknown role {:?}",
                    entry.role
                ))
            })?;
            crate::passwords::phc_is_argon2id(&entry.password_argon2id_phc).map_err(|error| {
                ConfigError::OperatorsInvalid(format!("operator {id:?}: {error}"))
            })?;
            let totp_secret = match entry.totp_secret_base32.as_deref() {
                Some(secret) if !secret.trim().is_empty() => {
                    let bytes = crate::base32::decode(secret).map_err(|error| {
                        ConfigError::OperatorsInvalid(format!(
                            "operator {id:?} totp_secret_base32: {error}"
                        ))
                    })?;
                    if bytes.is_empty() {
                        return Err(ConfigError::OperatorsInvalid(format!(
                            "operator {id:?} totp_secret_base32 decodes to no bytes"
                        )));
                    }
                    Some(bytes)
                }
                _ => None,
            };
            by_id.insert(id.clone(), validated.len());
            validated.push(ValidatedOperator {
                config: OperatorConfig {
                    id,
                    name,
                    role: role.as_str().to_string(),
                    password_argon2id_phc: entry.password_argon2id_phc,
                    totp_secret_base32: entry.totp_secret_base32,
                },
                role,
                totp_secret,
            });
        }
        Ok(Self {
            entries: validated,
            by_id,
        })
    }

    /// Find an operator by id.
    pub fn find(&self, id: &str) -> Option<&ValidatedOperator> {
        self.by_id
            .get(id)
            .and_then(|index| self.entries.get(*index))
    }

    /// Every operator, in file order.
    pub fn all(&self) -> &[ValidatedOperator] {
        &self.entries
    }

    /// Number of operators.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no operator is configured.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Parse `publisher=<64 hex>` entries into the verified-ledger allow-list.
pub fn parse_ledger_keys(
    raw: &str,
) -> Result<std::collections::HashMap<String, apex_proof::VerifyingKey>, String> {
    let mut keys = std::collections::HashMap::new();
    for entry in raw.split(',').filter(|entry| !entry.trim().is_empty()) {
        let (publisher, hex_key) = entry
            .split_once('=')
            .ok_or_else(|| format!("entry {entry:?} is not publisher=<64-hex-ed25519-pubkey>"))?;
        let publisher = publisher.trim();
        if publisher.is_empty()
            || publisher.len() > 64
            || !publisher
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(format!(
                "publisher {publisher:?} must match ^[A-Za-z0-9_-]{{1,64}}$"
            ));
        }
        if keys.contains_key(publisher) {
            return Err(format!("publisher {publisher:?} is listed twice"));
        }
        keys.insert(publisher.to_string(), parse_verifying_key(hex_key)?);
    }
    Ok(keys)
}

/// Decode a 64-hex ed25519 verifying key.
pub fn parse_verifying_key(raw: &str) -> Result<apex_proof::VerifyingKey, String> {
    let raw = raw.trim();
    if raw.len() != 64 {
        return Err(format!(
            "ed25519 public key must be 64 hex characters, got {}",
            raw.len()
        ));
    }
    let bytes = hex::decode(raw).map_err(|_| "public key is not hex".to_string())?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "public key is not 32 bytes".to_string())?;
    apex_proof::VerifyingKey::from_bytes(&bytes)
        .map_err(|error| format!("invalid ed25519 public key: {error}"))
}

/// `^[a-z0-9][a-z0-9_-]{0,31}$` without pulling in a regex engine.
fn valid_operator_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    id.len() <= 32
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }
    }

    #[test]
    fn defaults_are_the_documented_values() {
        let config = ControlConfig::from_lookup(&lookup(&[])).expect("defaults");
        assert_eq!(config.bind.to_string(), DEFAULT_BIND);
        assert_eq!(config.operators_path, None);
        assert!(!config.is_configured());
        assert_eq!(config.journal_path, PathBuf::from(DEFAULT_JOURNAL));
        assert_eq!(config.jobs_dir, PathBuf::from(DEFAULT_JOBS_DIR));
        assert_eq!(config.environment, "PRODUCTION");
        assert_eq!(config.ledger_root, PathBuf::from(DEFAULT_LEDGER_ROOT));
        assert_eq!(
            config.invoice_register,
            PathBuf::from(DEFAULT_INVOICE_REGISTER)
        );
        assert_eq!(config.totp_state_path, PathBuf::from(DEFAULT_TOTP_STATE));
        assert_eq!(config.operations_path, PathBuf::from(DEFAULT_OPERATIONS));
        assert_eq!(
            config.effective_policy_path,
            PathBuf::from(DEFAULT_EFFECTIVE_POLICY)
        );
        assert_eq!(
            config.connections_root,
            PathBuf::from(DEFAULT_CONNECTIONS_ROOT)
        );
        assert!(config.clickhouse.is_none());
        assert!(config.jobs_remote.is_none());
        assert!(config.sync_marker.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn startup_restricts_pre_existing_private_files_to_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!(
            "apex-control-modes-{}-{}",
            std::process::id(),
            crate::signing::random_hex_32()
        ));
        let jobs = dir.join("jobs");
        let connections = dir.join("connections");
        std::fs::create_dir_all(&jobs).expect("jobs");
        std::fs::create_dir_all(&connections).expect("connections");
        let paths = [
            dir.join("operators.json"),
            dir.join("journal.jsonl"),
            dir.join("totp.json"),
            dir.join("operations.json"),
            dir.join("policies.jsonl"),
            dir.join("interventions.jsonl"),
            dir.join("effective.json"),
            jobs.join("job.json"),
            connections.join("publisher.managed.json"),
        ];
        for path in &paths {
            std::fs::write(path, "{}").expect("write");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        }
        let config = ControlConfig::from_lookup(&lookup(&[
            ("APEX_CONTROL_OPERATORS", paths[0].to_str().expect("path")),
            ("APEX_CONTROL_JOURNAL", paths[1].to_str().expect("path")),
            ("APEX_CONTROL_TOTP_STATE", paths[2].to_str().expect("path")),
            ("APEX_CONTROL_OPERATIONS", paths[3].to_str().expect("path")),
            ("APEX_CONTROL_POLICIES", paths[4].to_str().expect("path")),
            (
                "APEX_CONTROL_INTERVENTIONS",
                paths[5].to_str().expect("path"),
            ),
            ("APEX_EFFECTIVE_POLICY", paths[6].to_str().expect("path")),
            ("APEX_OPERATOR_JOBS_DIR", jobs.to_str().expect("path")),
            (
                "APEX_CONTROL_CONNECTIONS_ROOT",
                connections.to_str().expect("path"),
            ),
        ]))
        .expect("config");
        config.restrict_private_files().expect("restrict");
        for path in &paths {
            let mode = std::fs::metadata(path).expect("stat").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", path.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_ledger_key_allow_list_is_validated_and_configured() {
        let key = "11".repeat(32);
        let config = ControlConfig::from_lookup(&lookup(&[
            ("APEX_CONTROL_LEDGER_KEYS", &format!("northstar={key}")),
            ("APEX_RUNNER_SNAPSHOT", "/tmp/snapshots"),
            ("APEX_NOTIFY_SINK", "email"),
            ("APEX_METRICS_TEXTFILE", "/tmp/control.prom"),
        ]))
        .expect("configured");
        assert!(config.ledger_keys.contains_key("northstar"));
        assert_eq!(config.snapshot_root, PathBuf::from("/tmp/snapshots"));
        assert_eq!(config.notify_sink.as_deref(), Some("email"));
        assert_eq!(
            config.metrics_textfile,
            Some(PathBuf::from("/tmp/control.prom"))
        );
        assert!(matches!(
            ControlConfig::from_lookup(&lookup(&[("APEX_LEDGER_KEYS", "northstar=zz")])),
            Err(ConfigError::LedgerKeys(_))
        ));
        assert!(matches!(
            ControlConfig::from_lookup(&lookup(&[("APEX_LEDGER_KEYS", "../escape=00")])),
            Err(ConfigError::LedgerKeys(_))
        ));
    }

    #[test]
    fn the_split_host_jobs_pair_must_be_complete_and_is_debug_redacted() {
        // Both set: the remote transport, with the URL and token kept.
        let config = ControlConfig::from_lookup(&lookup(&[
            ("APEX_API_JOBS_URL", "https://api.example.test"),
            ("APEX_OPERATOR_JOBS_TOKEN", "jobs-token"),
        ]))
        .expect("remote pair");
        let remote = config.jobs_remote.as_ref().expect("remote mode");
        assert_eq!(remote.url, "https://api.example.test");
        assert_eq!(remote.token, "jobs-token");
        let rendered = format!("{remote:?}");
        assert!(!rendered.contains("jobs-token"), "{rendered}");
        assert!(rendered.contains("[redacted]"), "{rendered}");

        // URL without token: refused at startup, never an unauthenticated POST.
        let half = ControlConfig::from_lookup(&lookup(&[(
            "APEX_API_JOBS_URL",
            "https://api.example.test",
        )]));
        assert!(matches!(half, Err(ConfigError::JobsPair)));

        // Token without URL: the local folder stays (single-host deployments
        // share the API's token environment without splitting the hosts).
        let token_only =
            ControlConfig::from_lookup(&lookup(&[("APEX_OPERATOR_JOBS_TOKEN", "jobs-token")]))
                .expect("token only");
        assert!(token_only.jobs_remote.is_none());
        assert_eq!(token_only.jobs_dir, PathBuf::from(DEFAULT_JOBS_DIR));

        // A malformed or non-http URL is refused.
        for bad in ["not a url", "ftp://api.example.test"] {
            let config = ControlConfig::from_lookup(&lookup(&[
                ("APEX_API_JOBS_URL", bad),
                ("APEX_OPERATOR_JOBS_TOKEN", "jobs-token"),
            ]));
            assert!(matches!(config, Err(ConfigError::JobsUrl(_))), "{bad}");
        }

        // A blank token reads as unset, so the pair is incomplete.
        let blank = ControlConfig::from_lookup(&lookup(&[
            ("APEX_API_JOBS_URL", "https://api.example.test"),
            ("APEX_OPERATOR_JOBS_TOKEN", "   "),
        ]));
        assert!(matches!(blank, Err(ConfigError::JobsPair)));
    }

    #[test]
    fn the_sync_marker_path_is_optional_and_blank_reads_as_unset() {
        let config = ControlConfig::from_lookup(&lookup(&[(
            "APEX_CONTROL_SYNC_MARKER",
            "/var/lib/apex/sync-marker.json",
        )]))
        .expect("marker path");
        assert_eq!(
            config.sync_marker,
            Some(PathBuf::from("/var/lib/apex/sync-marker.json"))
        );
        let blank = ControlConfig::from_lookup(&lookup(&[("APEX_CONTROL_SYNC_MARKER", "  ")]))
            .expect("blank marker");
        assert!(blank.sync_marker.is_none());
    }

    #[test]
    fn environment_labels_are_canonicalized_and_bounded() {
        assert_eq!(normalize_environment(None), "PRODUCTION");
        assert_eq!(normalize_environment(Some("test")), "TEST");
        assert_eq!(normalize_environment(Some("Production")), "PRODUCTION");
        assert_eq!(normalize_environment(Some(" staging ")), "STAGING");
        assert_eq!(normalize_environment(Some("")), "PRODUCTION");
        let long = "x".repeat(100);
        assert_eq!(
            normalize_environment(Some(&long)).len(),
            MAX_ENVIRONMENT_LEN
        );
    }

    #[test]
    fn explicit_values_parse_and_clickhouse_requires_a_valid_url() {
        let config = ControlConfig::from_lookup(&lookup(&[
            ("APEX_CONTROL_BIND", "0.0.0.0:9000"),
            ("APEX_CONTROL_OPERATORS", "/etc/apex/operators.json"),
            ("APEX_ENVIRONMENT", "TEST"),
            ("APEX_CLICKHOUSE_HTTP", "http://127.0.0.1:8123"),
            ("APEX_CLICKHOUSE_DB", "apex_analytics_test"),
        ]))
        .expect("explicit");
        assert_eq!(config.bind.to_string(), "0.0.0.0:9000");
        assert!(config.is_configured());
        assert_eq!(config.environment, "TEST");
        assert_eq!(
            config.clickhouse.as_ref().map(|c| c.database.as_str()),
            Some("apex_analytics_test")
        );
        let bad = ControlConfig::from_lookup(&lookup(&[("APEX_CONTROL_BIND", "not an address")]));
        assert!(matches!(bad, Err(ConfigError::Bind(_))));
        let bad_url =
            ControlConfig::from_lookup(&lookup(&[("APEX_CLICKHOUSE_HTTP", "ftp://example.test")]));
        assert!(matches!(bad_url, Err(ConfigError::Analytics(_))));
    }

    fn entry(id: &str, role: &str, with_totp: bool) -> OperatorConfig {
        // Argon2id over a low-cost parameter set is still an argon2id PHC
        // string; the fixture uses the library defaults via hash_with_salt.
        let phc = crate::passwords::hash_with_salt("fixture-password", b"apex-control-fixture")
            .expect("hash");
        OperatorConfig {
            id: id.to_string(),
            name: format!("Operator {id}"),
            role: role.to_string(),
            password_argon2id_phc: phc,
            totp_secret_base32: with_totp.then(|| "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".to_string()),
        }
    }

    #[test]
    fn operators_validate_roles_phc_and_unique_ids() {
        let operators = Operators::from_entries(vec![
            entry("maya", "operator", true),
            entry("sam", "finance", false),
        ])
        .expect("valid");
        assert_eq!(operators.len(), 2);
        assert_eq!(operators.find("maya").expect("maya").role, Role::Operator);
        assert!(operators.find("sam").expect("sam").totp_secret.is_none());
        assert!(operators.find("maya").expect("maya").totp_secret.is_some());
        assert!(operators.find("nobody").is_none());

        let duplicate = Operators::from_entries(vec![
            entry("maya", "operator", true),
            entry("maya", "support", true),
        ]);
        assert!(matches!(duplicate, Err(ConfigError::OperatorsInvalid(m)) if m.contains("twice")));
        let bad_role = Operators::from_entries(vec![entry("maya", "admin", true)]);
        assert!(bad_role.is_err());
        let bad_id = Operators::from_entries(vec![entry("Maya!", "operator", true)]);
        assert!(bad_id.is_err());
        let mut bad_phc_entry = entry("maya", "operator", true);
        bad_phc_entry.password_argon2id_phc = "$2b$10$notargon2".to_string();
        assert!(Operators::from_entries(vec![bad_phc_entry]).is_err());
        let mut bad_secret_entry = entry("maya", "operator", true);
        bad_secret_entry.totp_secret_base32 = Some("not-base32!".to_string());
        assert!(Operators::from_entries(vec![bad_secret_entry]).is_err());
    }

    #[test]
    fn operator_ids_are_strict_and_bounded() {
        assert!(valid_operator_id("maya"));
        assert!(valid_operator_id("m"));
        assert!(valid_operator_id("op-1_x"));
        assert!(valid_operator_id(&"a".repeat(32)));
        assert!(!valid_operator_id(""));
        assert!(!valid_operator_id("Maya"));
        assert!(!valid_operator_id("-maya"));
        assert!(!valid_operator_id("_maya"));
        assert!(!valid_operator_id("ma ya"));
        assert!(!valid_operator_id(&"a".repeat(33)));
    }

    #[test]
    fn load_reads_and_validates_a_file() {
        let dir = std::env::temp_dir().join(format!("apex-control-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("operators.json");
        let entries = vec![
            entry("maya", "operator", true),
            entry("sam", "security", true),
        ];
        std::fs::write(&path, serde_json::to_string(&entries).expect("json")).expect("write");
        let operators = Operators::load(&path).expect("load");
        assert_eq!(operators.len(), 2);
        assert_eq!(operators.find("sam").expect("sam").role, Role::Security);
        // A missing file is a read error; the binary turns it into the
        // unconfigured state only for an absent path, never for a broken one.
        assert!(Operators::load(&dir.join("absent.json")).is_err());
        std::fs::write(&path, "{not json}").expect("write");
        assert!(matches!(
            Operators::load(&path),
            Err(ConfigError::OperatorsInvalid(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
