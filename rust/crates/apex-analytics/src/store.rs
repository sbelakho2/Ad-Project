//! The event store: a ClickHouse HTTP store and an in-memory store sharing
//! one trait.
//!
//! [`ClickHouseStore`] talks to the server over the HTTP interface with
//! `JSONEachRow` bodies; [`MemoryStore`] implements the same contract for
//! unit tests. Deduplication uses the spec's stable source/event key:
//!
//! - The ClickHouse table is a `ReplacingMergeTree(version)` ordered by
//!   `(source, event_id)`. Replay of one event lands the same key twice;
//!   merges collapse the key, keeping the greatest `version` (milliseconds
//!   of `received_at_utc`). Reads use `FINAL` for deterministic
//!   read-your-writes dedup. Because merges are asynchronous,
//!   `duplicates_observed` on the HTTP outcome is `None` ("unknown"); the
//!   `data_quality` read model counts physical duplicate rows instead.
//! - [`MemoryStore`] deduplicates immediately by `(source, event_id)`: a
//!   replay with a strictly older `received_at_utc` is dropped and counted,
//!   while an equal or newer received time replaces the row. The equal case
//!   mirrors the server: `ReplacingMergeTree(version)` keeps the LAST
//!   inserted row on an equal version, so the last insert wins on both
//!   backends. Tests pin both behaviours.
//!
//! Projections are STAGED: [`ProjectionStore::write_projection`] records an
//! unpublished version, readers ([`ProjectionStore::load_projection`],
//! [`ProjectionStore::projection_version`],
//! [`ProjectionStore::projection_versions`]) resolve only versions marked
//! `published`, and [`ProjectionStore::publish_batch`] makes one batch's
//! versions reader-visible after a clean compare.
//!
//! Queries never interpolate user input: values travel as ClickHouse
//! `{name:Type}` bound parameters over HTTP `param_*` keys.
//!
//! The store reads the environment contract `APEX_CLICKHOUSE_HTTP`,
//! `APEX_CLICKHOUSE_DB`, `APEX_CLICKHOUSE_USER` and
//! `APEX_CLICKHOUSE_PASSWORD` (through [`ClickHouseConfig::from_lookup`]).
//! When a user or password is configured, every request carries the standard
//! ClickHouse HTTP credentials (`X-ClickHouse-User` / `X-ClickHouse-Key`);
//! when neither is set nothing is attached and the request is byte-identical
//! to the unauthenticated form. TLS is the URL scheme: an `https://` base URL
//! is served over rustls with the default `reqwest` certificate verification
//! (never disabled here), so a server whose chain the client does not trust
//! is refused at the handshake. The password is never rendered: the config's
//! `Debug` redacts it, and it is never interpolated into a URL, body or
//! error.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::envelope::DomainEvent;
use crate::projections::{ProjectionName, ProjectionRow};
use crate::schema::{all_ddl, validate_identifier, SchemaError};
use crate::timestamp::UtcMillis;

/// Store failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// Configuration was refused before any request.
    #[error("store configuration is invalid: {0}")]
    Config(String),
    /// The schema has not been created; call `ensure_schema` first.
    #[error("schema is not ready; ensure_schema has not run")]
    SchemaNotReady,
    /// A database identifier was refused.
    #[error("schema error: {0}")]
    Schema(String),
    /// The HTTP request itself failed (connect, timeout, TLS).
    #[error("http transport failed: {0}")]
    Http(String),
    /// The server answered with a non-success status.
    #[error("clickhouse answered with status {status}: {body}")]
    Status {
        /// HTTP status code.
        status: u16,
        /// Truncated response body carrying the server's error text.
        body: String,
    },
    /// A response body could not be parsed.
    #[error("unexpected response payload: {0}")]
    Json(String),
    /// A query was refused before it could return a misleading answer (for
    /// example a read that would exceed its stated bound).
    #[error("query refused: {0}")]
    Query(String),
}

impl From<SchemaError> for StoreError {
    fn from(error: SchemaError) -> Self {
        StoreError::Schema(error.to_string())
    }
}

/// How many characters of a provider response body an error carries.
const ERROR_BODY_CHARS: usize = 512;

/// Bound a response body for an error message by CHARACTER count.
/// `String::truncate` takes a byte index and panics when it lands inside a
/// multi-byte character, so provider text is never cut that way.
fn truncate_body(text: &str) -> String {
    text.chars().take(ERROR_BODY_CHARS).collect()
}

/// Read an unsigned integer JSON field, tolerating plain numbers and quoted
/// strings (ClickHouse quotes 64-bit integers in JSON under some settings).
pub(crate) fn json_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    value.and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    })
}

/// The `(name, sorting_key)` pairs of a `system.tables` answer, skipping rows
/// without a name and defaulting a missing or non-string key to the empty
/// string, which never matches a staged key (the planner leaves that table
/// alone and the next boot re-reads it).
fn observed_sorting_keys(rows: &[serde_json::Value]) -> Vec<(String, String)> {
    rows.iter()
        .filter_map(|row| {
            let name = row.get("name")?.as_str()?.to_string();
            let key = row
                .get("sorting_key")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some((name, key))
        })
        .collect()
}

/// How many events a store accepted and how many duplicates it observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InsertOutcome {
    /// Rows (events or source rows) sent to the store.
    pub inserted: u64,
    /// Rows recognized as replays at insert time: for events a strictly
    /// older `received_at_utc` replay of the `(source, event_id)` key, which
    /// is dropped. An equal or newer replay replaces the stored row (the
    /// last insert wins, like the server) and is not counted here. `None`
    /// means the store removes duplicates at merge time and cannot know
    /// (ClickHouse).
    pub duplicates_observed: Option<u64>,
}

/// A store reachability and identity check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// Database the store is bound to.
    pub database: String,
    /// Server version when the backend reports one.
    pub server_version: Option<String>,
    /// Server time when the backend reports one.
    pub server_time: Option<UtcMillis>,
}

/// One finalized platform report day: the revenue source fact.
///
/// Money is integer micros with an explicit currency; a missing amount is
/// `None`, never zero. The breakdown dimensions are optional because the
/// spec breaks revenue down "only where source fields exist" (page 51).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReportDayRow {
    /// Tenant scope.
    pub publisher_id: String,
    /// Platform the report came from.
    pub platform: String,
    /// Platform account identity.
    pub platform_account_id: String,
    /// Reporting day, `YYYY-MM-DD`.
    pub day: String,
    /// Finality of the row (`final`, `partial`, `provisional`).
    pub finality: String,
    /// Revenue in integer micros when the platform supplied it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue_micros: Option<i64>,
    /// Impressions when the platform supplied them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impressions: Option<u64>,
    /// ISO-4217 currency of the amount.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Country when the source reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// Ad unit when the source reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_unit_id: Option<String>,
    /// Ad format when the source reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Digest of the snapshot the row came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<String>,
}

/// One account row for lifecycle projections.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AccountRow {
    /// Tenant scope.
    pub publisher_id: String,
    /// Account identity.
    pub account_id: String,
    /// Current lifecycle stage.
    pub stage: String,
    /// Owner when assigned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Platforms the account is connected to (platform names).
    pub platforms: Vec<String>,
    /// First day the account appeared (`YYYY-MM-DD`).
    pub first_seen: String,
    /// Last stage change day (`YYYY-MM-DD`).
    pub stage_updated: String,
}

/// One sanitized HTTP request log row, matching the `request_logs` table
/// columns exactly. Times are UTC milliseconds; [`RequestLogRow::to_storage_value`]
/// renders them the way `DateTime64(3)` expects.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RequestLogRow {
    /// Stable identity derived from the parsed line (replay keeps it).
    pub event_id: String,
    /// When the request happened, UTC.
    pub occurred_at_utc: UtcMillis,
    /// When the ingester received the line, UTC.
    pub received_at_utc: UtcMillis,
    /// Normalized route template (never the raw URL).
    pub route_template: String,
    /// HTTP status code of the request.
    pub status: u16,
    /// Bounded status class (`2xx`).
    pub status_class: String,
    /// Bounded bot classification, never a claim about humans.
    pub bot_class: String,
    /// Referrer host only, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referrer_host: Option<String>,
    /// Sanitized campaign code, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_code: Option<String>,
    /// Allow-listed query parameter keys only; values are never stored.
    pub query_keys: Vec<String>,
    /// Deployment environment label.
    pub environment: String,
}

impl RequestLogRow {
    /// The `JSONEachRow` shape of the `request_logs` table: the times render
    /// as ClickHouse `DateTime64(3)` text and absent fields stay absent.
    pub fn to_storage_value(&self) -> serde_json::Value {
        serde_json::json!({
            "event_id": self.event_id,
            "occurred_at_utc": self.occurred_at_utc.to_clickhouse(),
            "received_at_utc": self.received_at_utc.to_clickhouse(),
            "route_template": self.route_template,
            "status": self.status,
            "status_class": self.status_class,
            "bot_class": self.bot_class,
            "referrer_host": self.referrer_host,
            "campaign_code": self.campaign_code,
            "query_keys": self.query_keys,
            "environment": self.environment,
        })
    }
}

/// The event store contract every backend implements.
#[async_trait]
pub trait EventStore: Send + Sync + std::fmt::Debug {
    /// Create the database and every table, idempotently.
    async fn ensure_schema(&self) -> Result<(), StoreError>;
    /// Insert events under the `(source, event_id)` dedup key.
    async fn insert(&self, events: &[DomainEvent]) -> Result<InsertOutcome, StoreError>;
    /// Insert finalized platform report rows (the revenue source facts).
    async fn insert_report_days(&self, rows: &[ReportDayRow]) -> Result<InsertOutcome, StoreError>;
    /// Insert account lifecycle rows.
    async fn insert_accounts(&self, rows: &[AccountRow]) -> Result<InsertOutcome, StoreError>;
    /// Insert sanitized HTTP request log rows into the 30-day TTL table.
    async fn insert_request_logs(
        &self,
        rows: &[RequestLogRow],
    ) -> Result<InsertOutcome, StoreError>;
    /// Reachability and identity check.
    async fn health(&self) -> Result<Health, StoreError>;
}

/// Projection sources and sinks a rebuild, compare and publish need.
///
/// Versions are staged: [`ProjectionStore::write_projection`] records an
/// unpublished version, readers resolve only published versions, and
/// [`ProjectionStore::publish_batch`] makes one batch's versions
/// reader-visible after a clean compare.
#[async_trait]
pub trait ProjectionStore: Send + Sync + std::fmt::Debug {
    /// All events currently stored (post-dedup).
    async fn load_events(&self) -> Result<Vec<DomainEvent>, StoreError>;
    /// All report day rows currently stored.
    async fn load_report_days(&self) -> Result<Vec<ReportDayRow>, StoreError>;
    /// All account rows currently stored.
    async fn load_accounts(&self) -> Result<Vec<AccountRow>, StoreError>;
    /// Record one projection under `as_of_version` as a NEW UNPUBLISHED
    /// version: readers do not see it until
    /// [`ProjectionStore::publish_batch`] marks it published.
    async fn write_projection(
        &self,
        name: ProjectionName,
        as_of_version: u64,
        batch_id: &str,
        rows: &[ProjectionRow],
    ) -> Result<(), StoreError>;
    /// Read the current PUBLISHED rows of one projection; empty until a
    /// first publish.
    async fn load_projection(&self, name: ProjectionName)
        -> Result<Vec<ProjectionRow>, StoreError>;
    /// The newest as-of version readers currently see (published), when any.
    async fn projection_version(&self, name: ProjectionName) -> Result<Option<u64>, StoreError>;
    /// One row per projection with a published version: the recorded
    /// version, the batch that produced it, the newest source event time it
    /// covers and when it was rebuilt. This is the read the daily
    /// projection-lag gauge is computed from.
    async fn projection_versions(&self) -> Result<Vec<ProjectionVersionRow>, StoreError>;
    /// The newest version recorded for a projection, published or not.
    /// Rebuilds allocate `latest + 1` from this so a staged version is never
    /// overwritten.
    async fn latest_projection_version(
        &self,
        name: ProjectionName,
    ) -> Result<Option<u64>, StoreError>;
    /// The newest stored rows of one projection with their version and
    /// batch, published or not: the compare read. Readers themselves use
    /// [`ProjectionStore::load_projection`].
    async fn latest_projection(
        &self,
        name: ProjectionName,
    ) -> Result<Option<StagedProjection>, StoreError>;
    /// Mark every recorded version row of `batch_id` published; returns the
    /// highest version the batch recorded, or `None` when the batch has no
    /// rows at all. Readers always resolve the highest PUBLISHED version, so
    /// publishing an older batch can never roll readers back.
    async fn publish_batch(&self, batch_id: &str) -> Result<Option<u64>, StoreError>;
}

/// The newest stored rows of one projection, published or not.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedProjection {
    /// The as-of version the rows were written under.
    pub as_of_version: u64,
    /// The batch identity that produced them.
    pub batch_id: String,
    /// The stored rows.
    pub rows: Vec<ProjectionRow>,
}

/// The recorded state of one projection version row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectionVersionRow {
    /// The projection table name.
    pub projection_name: String,
    /// The as-of version the readers currently see.
    pub as_of_version: u64,
    /// The batch id that produced this version.
    pub batch_id: String,
    /// The newest source event time covered, in unix milliseconds.
    pub source_event_max_unix_ms: Option<i64>,
    /// When the projection was rebuilt, in unix milliseconds.
    pub rebuilt_at_unix_ms: i64,
}

/// ClickHouse HTTP connection settings.
///
/// `Debug` redacts the password; the value itself is compared normally.
#[derive(Clone, PartialEq, Eq)]
pub struct ClickHouseConfig {
    /// Base HTTP URL, e.g. `http://127.0.0.1:8123`. `https://` enables TLS
    /// with the default `reqwest` certificate verification.
    pub http_url: String,
    /// Database name; a plain identifier, validated before use.
    pub database: String,
    /// Request timeout in milliseconds.
    pub request_timeout_ms: u64,
    /// Optional ClickHouse user; sent as `X-ClickHouse-User` when set.
    pub user: Option<String>,
    /// Optional ClickHouse password; sent as `X-ClickHouse-Key` when set.
    pub password: Option<String>,
}

impl std::fmt::Debug for ClickHouseConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClickHouseConfig")
            .field("http_url", &self.http_url)
            .field("database", &self.database)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

impl Default for ClickHouseConfig {
    fn default() -> Self {
        Self {
            http_url: "http://127.0.0.1:8123".to_string(),
            database: "apex_analytics".to_string(),
            request_timeout_ms: 30_000,
            user: None,
            password: None,
        }
    }
}

impl ClickHouseConfig {
    /// Build from environment-like lookups: `APEX_CLICKHOUSE_HTTP`,
    /// `APEX_CLICKHOUSE_DB`, `APEX_CLICKHOUSE_USER` and
    /// `APEX_CLICKHOUSE_PASSWORD`; everything else keeps the default. A
    /// blank/whitespace credential reads as unset.
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, StoreError> {
        let mut config = Self::default();
        if let Some(url) = lookup("APEX_CLICKHOUSE_HTTP") {
            if !url.trim().is_empty() {
                config.http_url = url;
            }
        }
        if let Some(database) = lookup("APEX_CLICKHOUSE_DB") {
            if !database.trim().is_empty() {
                config.database = database;
            }
        }
        if let Some(user) = lookup("APEX_CLICKHOUSE_USER") {
            if !user.trim().is_empty() {
                config.user = Some(user);
            }
        }
        if let Some(password) = lookup("APEX_CLICKHOUSE_PASSWORD") {
            if !password.trim().is_empty() {
                config.password = Some(password);
            }
        }
        config.validate()?;
        Ok(config)
    }

    /// Build from the process environment.
    pub fn from_env() -> Result<Self, StoreError> {
        Self::from_lookup(&|key| std::env::var(key).ok())
    }

    /// Validate the URL scheme, the database identifier and the credential
    /// header values (they travel as HTTP header values, so control
    /// characters are refused before a request could smuggle them).
    pub fn validate(&self) -> Result<(), StoreError> {
        let url = reqwest::Url::parse(&self.http_url)
            .map_err(|error| StoreError::Config(format!("http_url: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(StoreError::Config(
                "http_url must use http or https".to_string(),
            ));
        }
        validate_identifier(&self.database)
            .map_err(|error| StoreError::Config(error.to_string()))?;
        if self.request_timeout_ms == 0 {
            return Err(StoreError::Config(
                "request_timeout_ms must be positive".to_string(),
            ));
        }
        for (variable, value) in [
            ("APEX_CLICKHOUSE_USER", &self.user),
            ("APEX_CLICKHOUSE_PASSWORD", &self.password),
        ] {
            if let Some(value) = value {
                if !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
                    return Err(StoreError::Config(format!(
                        "{variable} must be printable ASCII without control characters"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// One transport failure with its full cause chain on one line, so a TLS or
/// connection refusal names its cause (`invalid peer certificate:
/// UnknownIssuer`) instead of only the outer `reqwest` message. No URL query
/// or body is ever included beyond what `reqwest` itself renders.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut rendered = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        rendered.push_str(": ");
        rendered.push_str(&cause.to_string());
        source = cause.source();
    }
    rendered
}

/// An event store backed by the ClickHouse HTTP interface.
#[derive(Debug, Clone)]
pub struct ClickHouseStore {
    config: ClickHouseConfig,
    client: reqwest::Client,
}

impl ClickHouseStore {
    /// Build a store; validates the configuration and creates the HTTP client
    /// with the configured timeout.
    pub fn new(config: ClickHouseConfig) -> Result<Self, StoreError> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.request_timeout_ms))
            .build()
            .map_err(|error| StoreError::Http(error.to_string()))?;
        Ok(Self { config, client })
    }

    /// The configured database name.
    pub fn database(&self) -> &str {
        &self.config.database
    }

    /// The configured base URL.
    pub fn http_url(&self) -> &str {
        &self.config.http_url
    }

    fn url(&self, params: &[(&str, &str)]) -> Result<reqwest::Url, StoreError> {
        let mut url = reqwest::Url::parse(&self.config.http_url)
            .map_err(|error| StoreError::Config(format!("http_url: {error}")))?;
        {
            // No `database=` connection default: every statement names its
            // database explicitly, and CREATE DATABASE itself must run
            // before the name exists.
            let mut pairs = url.query_pairs_mut();
            for (name, value) in params {
                pairs.append_pair(&format!("param_{name}"), value);
            }
        }
        Ok(url)
    }

    /// Send one request whose body is the full query text. ClickHouse wants
    /// a body on POST (a bare POST is refused with HTTP 411), and the body
    /// spelling keeps long DDL out of the URL.
    async fn post_text(&self, body: &str, params: &[(&str, &str)]) -> Result<String, StoreError> {
        let url = self.url(params)?;
        let mut request = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "text/plain");
        // The standard ClickHouse HTTP credentials; absent configuration
        // sends exactly the unauthenticated request.
        if let Some(user) = &self.config.user {
            request = request.header("X-ClickHouse-User", user);
        }
        if let Some(password) = &self.config.password {
            request = request.header("X-ClickHouse-Key", password);
        }
        let response = request
            .body(body.to_string())
            .send()
            .await
            .map_err(|error| StoreError::Http(error_chain(&error)))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| StoreError::Http(error_chain(&error)))?;
        if !status.is_success() {
            return Err(StoreError::Status {
                status: status.as_u16(),
                body: truncate_body(&text),
            });
        }
        Ok(text)
    }

    /// Run one admin statement (DDL, `OPTIMIZE`, teardown) and return the
    /// server's text answer. Statements are program-built; never interpolate
    /// user input here.
    pub async fn execute_statement(&self, statement: &str) -> Result<String, StoreError> {
        self.execute(statement).await
    }

    async fn execute(&self, query: &str) -> Result<String, StoreError> {
        self.post_text(query, &[]).await
    }

    /// Run a SELECT with `{name:Type}` bound parameters and parse the
    /// `JSONEachRow` answer. Scope values travel as `param_*` keys only;
    /// never interpolate user input into `select`.
    pub async fn fetch_rows(
        &self,
        select: &str,
        params: &[(&str, &str)],
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let body = self
            .post_text(&format!("{select}\nFORMAT JSONEachRow"), params)
            .await?;
        body.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line)
                    .map_err(|error| StoreError::Json(error.to_string()))
            })
            .collect()
    }

    async fn insert_json_rows(
        &self,
        table: &str,
        rows: &[serde_json::Value],
    ) -> Result<(), StoreError> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut body = String::with_capacity(rows.len() * 256);
        body.push_str(&format!(
            "INSERT INTO {}.{table} FORMAT JSONEachRow\n",
            self.config.database
        ));
        for row in rows {
            body.push_str(
                &serde_json::to_string(row).map_err(|error| StoreError::Json(error.to_string()))?,
            );
            body.push('\n');
        }
        self.post_text(&body, &[]).await.map(|_| ())
    }

    /// Drop this store's database. Used by integration tests and by an
    /// explicit operator teardown; never called from normal ingestion.
    pub async fn drop_database(&self) -> Result<(), StoreError> {
        self.execute(&format!(
            "DROP DATABASE IF EXISTS {}",
            validate_identifier(&self.config.database)?
        ))
        .await
        .map(|_| ())
    }

    /// Delete projection rows of superseded as-of versions (all below
    /// `published_version`) so staged and replaced versions cannot
    /// accumulate; runs synchronously so a failure is reported before the
    /// publish is called done.
    async fn prune_superseded_versions(
        &self,
        projection_name: &str,
        published_version: u64,
    ) -> Result<(), StoreError> {
        let Some(name) = crate::projections::ProjectionName::ALL
            .into_iter()
            .find(|name| name.as_str() == projection_name)
        else {
            return Err(StoreError::Schema(format!(
                "unknown projection '{projection_name}'"
            )));
        };
        let statement = format!(
            "ALTER TABLE {}.{} DELETE WHERE as_of_version < {published_version} \
             SETTINGS mutations_sync = 1",
            self.config.database,
            name.table_name()
        );
        self.execute_statement(&statement).await.map(|_| ())
    }

    /// Migrate legacy staged-table shapes in place.
    ///
    /// Databases created before the staging change have `projection_versions`
    /// ordered by `projection_name` and projection tables ordered by their
    /// business keys only, so a staged rebuild collapses the published rows
    /// away at merge time and readers resolve no version at all. The live
    /// sorting keys are read from `system.tables` and the plan from
    /// [`crate::schema::staged_migration_plan`] is executed statement by
    /// statement; the plan first drops any `<table>__migration` leftover from
    /// an interrupted boot and only rebuilds tables whose key is not the
    /// staged one, so every boot is idempotent and a crash anywhere in the
    /// sequence is repaired by the next boot.
    async fn migrate_legacy_staged_tables(&self) -> Result<(), StoreError> {
        let rows = self
            .fetch_rows(
                "SELECT name, sorting_key FROM system.tables WHERE database = {db:String}",
                &[("db", self.config.database.as_str())],
            )
            .await?;
        let observed = observed_sorting_keys(&rows);
        for statement in crate::schema::staged_migration_plan(&self.config.database, &observed)? {
            self.execute_statement(&statement).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl EventStore for ClickHouseStore {
    async fn ensure_schema(&self) -> Result<(), StoreError> {
        for statement in all_ddl(&self.config.database)? {
            self.execute(&statement).await?;
        }
        // A database created before staged projections existed keeps its
        // legacy sorting keys: migrate every legacy shape before any rebuild
        // can collapse a published version away.
        self.migrate_legacy_staged_tables().await
    }

    async fn insert(&self, events: &[DomainEvent]) -> Result<InsertOutcome, StoreError> {
        let rows: Vec<serde_json::Value> = events
            .iter()
            .map(|event| {
                serde_json::to_value(event.to_row())
                    .map_err(|error| StoreError::Json(error.to_string()))
            })
            .collect::<Result<_, _>>()?;
        self.insert_json_rows("domain_events", &rows).await?;
        Ok(InsertOutcome {
            inserted: events.len() as u64,
            duplicates_observed: None,
        })
    }

    async fn insert_report_days(&self, rows: &[ReportDayRow]) -> Result<InsertOutcome, StoreError> {
        let rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| {
                serde_json::to_value(row).map_err(|error| StoreError::Json(error.to_string()))
            })
            .collect::<Result<_, _>>()?;
        self.insert_json_rows("report_days", &rows).await?;
        Ok(InsertOutcome {
            inserted: rows.len() as u64,
            duplicates_observed: None,
        })
    }

    async fn insert_accounts(&self, rows: &[AccountRow]) -> Result<InsertOutcome, StoreError> {
        let rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| {
                serde_json::to_value(row).map_err(|error| StoreError::Json(error.to_string()))
            })
            .collect::<Result<_, _>>()?;
        self.insert_json_rows("accounts", &rows).await?;
        Ok(InsertOutcome {
            inserted: rows.len() as u64,
            duplicates_observed: None,
        })
    }

    async fn insert_request_logs(
        &self,
        rows: &[RequestLogRow],
    ) -> Result<InsertOutcome, StoreError> {
        let values: Vec<serde_json::Value> =
            rows.iter().map(RequestLogRow::to_storage_value).collect();
        self.insert_json_rows("request_logs", &values).await?;
        Ok(InsertOutcome {
            inserted: rows.len() as u64,
            duplicates_observed: None,
        })
    }

    async fn health(&self) -> Result<Health, StoreError> {
        let rows = self
            .fetch_rows(
                "SELECT version() AS version, toString(toUnixTimestamp64Milli(now64(3))) AS now_millis",
                &[],
            )
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| StoreError::Json("health check returned no rows".to_string()))?;
        let version = row
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let server_time = row
            .get("now_millis")
            .and_then(serde_json::Value::as_str)
            .and_then(|text| text.parse::<i64>().ok())
            .map(UtcMillis::from_unix_millis);
        Ok(Health {
            database: self.config.database.clone(),
            server_version: version,
            server_time,
        })
    }
}

/// One stored projection version snapshot.
#[derive(Debug, Clone, PartialEq)]
struct ProjectionSnapshot {
    batch_id: String,
    published: bool,
    rows: Vec<ProjectionRow>,
}

#[derive(Debug, Default)]
struct MemoryState {
    schema_ready: bool,
    events: BTreeMap<(String, String), DomainEvent>,
    event_inserts: u64,
    event_duplicates: u64,
    report_days: BTreeMap<ReportDayKey, ReportDayRow>,
    accounts: BTreeMap<(String, String), AccountRow>,
    request_logs: Vec<RequestLogRow>,
    projections: BTreeMap<&'static str, BTreeMap<u64, ProjectionSnapshot>>,
}

/// The report-day dedup key, mirroring the `report_days` sorting key of the
/// schema exactly: `(publisher_id, platform, platform_account_id, day,
/// country, ad_unit_id, format)`. Rows that differ only in a breakdown
/// dimension are separate source facts and must never collapse into one.
type ReportDayKey = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The newest PUBLISHED snapshot of a projection, if any.
fn published_snapshot<'a>(state: &'a MemoryState, name: &str) -> Option<&'a ProjectionSnapshot> {
    state
        .projections
        .get(name)?
        .iter()
        .filter(|(_, snapshot)| snapshot.published)
        .max_by_key(|(version, _)| **version)
        .map(|(_, snapshot)| snapshot)
}

/// An in-memory store implementing the same contract as ClickHouse.
///
/// Dedup semantics are immediate and exact: `(source, event_id)` is the key;
/// a replay with a strictly older `received_at_utc` is dropped and counted,
/// while an equal or newer received time replaces the stored row (the last
/// insert wins on equal versions, exactly like `ReplacingMergeTree(version)`
/// with `version = received_at_utc` collapsed to its fixed point).
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    state: Arc<Mutex<MemoryState>>,
}

impl MemoryStore {
    /// A fresh, empty store. `ensure_schema` is still required before writes.
    pub fn new() -> Self {
        Self::default()
    }

    fn with_state<T>(&self, action: impl FnOnce(&mut MemoryState) -> T) -> T {
        let mut guard = self.state.lock().unwrap_or_else(|poisoned| {
            // A poisoned lock means a test panicked mid-mutation; the state
            // itself is plain data, so recovering it keeps later assertions
            // meaningful instead of cascading panics.
            poisoned.into_inner()
        });
        action(&mut guard)
    }

    /// Total events accepted and duplicates dropped across inserts.
    pub fn insert_stats(&self) -> (u64, u64) {
        self.with_state(|state| (state.event_inserts, state.event_duplicates))
    }

    /// Stored report day count (post-dedup).
    pub fn report_day_count(&self) -> usize {
        self.with_state(|state| state.report_days.len())
    }

    /// Sanitized request log rows stored so far, in insert order (test
    /// read-back of the 30-day retention table).
    pub fn request_logs(&self) -> Vec<RequestLogRow> {
        self.with_state(|state| state.request_logs.clone())
    }
}

#[async_trait]
impl EventStore for MemoryStore {
    async fn ensure_schema(&self) -> Result<(), StoreError> {
        self.with_state(|state| state.schema_ready = true);
        Ok(())
    }

    async fn insert(&self, events: &[DomainEvent]) -> Result<InsertOutcome, StoreError> {
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            let mut duplicates = 0;
            for event in events {
                let key = (event.source.as_str().to_string(), event.event_id.clone());
                match state.events.get(&key) {
                    Some(existing) if existing.received_at_utc > event.received_at_utc => {
                        duplicates += 1;
                    }
                    _ => {
                        state.events.insert(key, event.clone());
                    }
                }
            }
            state.event_inserts += events.len() as u64;
            state.event_duplicates += duplicates;
            Ok(InsertOutcome {
                inserted: events.len() as u64,
                duplicates_observed: Some(duplicates),
            })
        })
    }

    async fn insert_report_days(&self, rows: &[ReportDayRow]) -> Result<InsertOutcome, StoreError> {
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            let mut duplicates = 0;
            for row in rows {
                let key = (
                    row.publisher_id.clone(),
                    row.platform.clone(),
                    row.platform_account_id.clone(),
                    row.day.clone(),
                    row.country.clone(),
                    row.ad_unit_id.clone(),
                    row.format.clone(),
                );
                if state.report_days.contains_key(&key) {
                    duplicates += 1;
                }
                state.report_days.insert(key, row.clone());
            }
            Ok(InsertOutcome {
                inserted: rows.len() as u64,
                duplicates_observed: Some(duplicates),
            })
        })
    }

    async fn insert_accounts(&self, rows: &[AccountRow]) -> Result<InsertOutcome, StoreError> {
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            let mut duplicates = 0;
            for row in rows {
                let key = (row.publisher_id.clone(), row.account_id.clone());
                if state.accounts.contains_key(&key) {
                    duplicates += 1;
                }
                state.accounts.insert(key, row.clone());
            }
            Ok(InsertOutcome {
                inserted: rows.len() as u64,
                duplicates_observed: Some(duplicates),
            })
        })
    }

    async fn insert_request_logs(
        &self,
        rows: &[RequestLogRow],
    ) -> Result<InsertOutcome, StoreError> {
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            state.request_logs.extend_from_slice(rows);
            Ok(InsertOutcome {
                inserted: rows.len() as u64,
                duplicates_observed: Some(0),
            })
        })
    }

    async fn health(&self) -> Result<Health, StoreError> {
        let ready = self.with_state(|state| state.schema_ready);
        if !ready {
            return Err(StoreError::SchemaNotReady);
        }
        Ok(Health {
            database: "memory".to_string(),
            server_version: Some("memory".to_string()),
            server_time: UtcMillis::system_now().ok(),
        })
    }
}

#[async_trait]
impl ProjectionStore for ClickHouseStore {
    async fn load_events(&self) -> Result<Vec<DomainEvent>, StoreError> {
        // Rebuilds read the raw facts; dedup is applied per key so a replay
        // cannot double-count even before merges run.
        let database = self.config.database.as_str();
        let rows = self
            .fetch_rows(
                &format!(
                    "SELECT * FROM {database}.domain_events FINAL \
                     ORDER BY occurred_at_utc, source, event_id"
                ),
                &[],
            )
            .await?;
        rows.iter().map(domain_event_from_row).collect()
    }

    async fn load_report_days(&self) -> Result<Vec<ReportDayRow>, StoreError> {
        let database = self.config.database.as_str();
        let rows = self
            .fetch_rows(
                &format!(
                    "SELECT publisher_id, platform, platform_account_id, toString(day) AS day, \
                     finality, revenue_micros, impressions, currency, country, ad_unit_id, \
                     format, source_digest \
                     FROM {database}.report_days FINAL \
                     ORDER BY publisher_id, platform, platform_account_id, day"
                ),
                &[],
            )
            .await?;
        rows.iter()
            .map(|row| {
                serde_json::from_value(row.clone())
                    .map_err(|error| StoreError::Json(error.to_string()))
            })
            .collect()
    }

    async fn load_accounts(&self) -> Result<Vec<AccountRow>, StoreError> {
        let database = self.config.database.as_str();
        let rows = self
            .fetch_rows(
                &format!(
                    // `first_seen` and `stage_updated` are day fields in the
                    // `AccountRow` contract; `toDate` keeps the read shape
                    // identical to what the memory store stores ("YYYY-MM-DD")
                    // instead of leaking the stored DateTime64 rendering.
                    "SELECT publisher_id, account_id, stage, owner, platforms, \
                     toString(toDate(first_seen)) AS first_seen, \
                     toString(toDate(stage_updated)) AS stage_updated \
                     FROM {database}.accounts FINAL ORDER BY publisher_id, account_id"
                ),
                &[],
            )
            .await?;
        rows.iter()
            .map(|row| {
                serde_json::from_value(row.clone())
                    .map_err(|error| StoreError::Json(error.to_string()))
            })
            .collect()
    }

    async fn write_projection(
        &self,
        name: ProjectionName,
        as_of_version: u64,
        batch_id: &str,
        rows: &[ProjectionRow],
    ) -> Result<(), StoreError> {
        let data_rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| row.to_table_json(name, as_of_version))
            .collect::<Result<_, _>>()?;
        let source_event_max: Option<String> = self
            .fetch_rows(
                &format!(
                    "SELECT toString(max(occurred_at_utc)) AS max_occurred \
                     FROM {}.domain_events",
                    self.config.database
                ),
                &[],
            )
            .await?
            .first()
            .and_then(|row| row.get("max_occurred"))
            .and_then(serde_json::Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string);
        let version_row = serde_json::json!({
            "projection_name": name.as_str(),
            "as_of_version": as_of_version,
            "batch_id": batch_id,
            "source_event_max": source_event_max,
            "rebuilt_at": UtcMillis::system_now()
                .map_err(|_| StoreError::Config("system clock unavailable".to_string()))?
                .to_clickhouse(),
            // Staged: readers resolve only published versions.
            "published": 0,
        });
        self.insert_json_rows(name.table_name(), &data_rows).await?;
        self.insert_json_rows("projection_versions", &[version_row])
            .await?;
        Ok(())
    }

    async fn load_projection(
        &self,
        name: ProjectionName,
    ) -> Result<Vec<ProjectionRow>, StoreError> {
        let select = crate::projections::projection_select_sql(self.config.database.as_str(), name)
            .ok_or_else(|| StoreError::Schema(format!("unknown projection {name:?}")))?;
        let rows = self
            .fetch_rows(&select, &[("projection", name.as_str())])
            .await?;
        rows.iter()
            .map(|row| ProjectionRow::from_table_json(name, row))
            .collect()
    }

    async fn projection_version(&self, name: ProjectionName) -> Result<Option<u64>, StoreError> {
        // No FINAL: FINAL collapses the key to the newest row, which may be
        // an unpublished staged version; the reader-visible version is the
        // highest PUBLISHED one.
        let rows = self
            .fetch_rows(
                &format!(
                    "SELECT toString(as_of_version) AS as_of_version \
                     FROM {}.projection_versions \
                     WHERE projection_name = {{projection_name:String}} AND published = 1 \
                     ORDER BY as_of_version DESC LIMIT 1",
                    self.config.database
                ),
                &[("projection_name", name.as_str())],
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|row| json_u64(row.get("as_of_version"))))
    }

    async fn projection_versions(&self) -> Result<Vec<ProjectionVersionRow>, StoreError> {
        let rows = self
            .fetch_rows(
                &format!(
                    "SELECT projection_name, toString(as_of_version) AS as_of_version, batch_id, \
                     toString(source_event_max) AS source_event_max, \
                     toString(rebuilt_at) AS rebuilt_at \
                     FROM {}.projection_versions \
                     WHERE published = 1 \
                     ORDER BY projection_name, as_of_version DESC \
                     LIMIT 1 BY projection_name",
                    self.config.database
                ),
                &[],
            )
            .await?;
        rows.iter()
            .map(|row| {
                let text = |key: &str| {
                    row.get(key)
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| StoreError::Json(format!("missing column '{key}'")))
                };
                let parse_clickhouse_millis = |value: &str| -> Result<i64, StoreError> {
                    let normalized = value.replacen(' ', "T", 1);
                    UtcMillis::parse_rfc3339(&format!("{normalized}Z"))
                        .map(UtcMillis::unix_millis)
                        .map_err(|error| StoreError::Json(error.to_string()))
                };
                let source_event_max_unix_ms = row
                    .get("source_event_max")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(parse_clickhouse_millis)
                    .transpose()?;
                Ok(ProjectionVersionRow {
                    projection_name: text("projection_name")?.to_string(),
                    as_of_version: json_u64(row.get("as_of_version")).ok_or_else(|| {
                        StoreError::Json("missing column 'as_of_version'".to_string())
                    })?,
                    batch_id: text("batch_id")?.to_string(),
                    source_event_max_unix_ms,
                    rebuilt_at_unix_ms: parse_clickhouse_millis(text("rebuilt_at")?)?,
                })
            })
            .collect()
    }

    async fn latest_projection_version(
        &self,
        name: ProjectionName,
    ) -> Result<Option<u64>, StoreError> {
        let rows = self
            .fetch_rows(
                &format!(
                    "SELECT toString(count()) AS recorded, \
                     toString(max(as_of_version)) AS latest \
                     FROM {}.projection_versions \
                     WHERE projection_name = {{projection_name:String}}",
                    self.config.database
                ),
                &[("projection_name", name.as_str())],
            )
            .await?;
        let recorded = rows
            .first()
            .and_then(|row| json_u64(row.get("recorded")))
            .unwrap_or(0);
        if recorded == 0 {
            return Ok(None);
        }
        rows.first()
            .and_then(|row| json_u64(row.get("latest")))
            .ok_or_else(|| StoreError::Json("missing column 'latest'".to_string()))
            .map(Some)
    }

    async fn latest_projection(
        &self,
        name: ProjectionName,
    ) -> Result<Option<StagedProjection>, StoreError> {
        let version_rows = self
            .fetch_rows(
                &format!(
                    "SELECT toString(count()) AS recorded, \
                     toString(max(as_of_version)) AS latest, \
                     argMax(batch_id, as_of_version) AS batch_id \
                     FROM {}.projection_versions \
                     WHERE projection_name = {{projection_name:String}}",
                    self.config.database
                ),
                &[("projection_name", name.as_str())],
            )
            .await?;
        let recorded = version_rows
            .first()
            .and_then(|row| json_u64(row.get("recorded")))
            .unwrap_or(0);
        if recorded == 0 {
            return Ok(None);
        }
        let row = version_rows
            .first()
            .ok_or_else(|| StoreError::Json("version lookup returned no rows".to_string()))?;
        let as_of_version = json_u64(row.get("latest"))
            .ok_or_else(|| StoreError::Json("missing column 'latest'".to_string()))?;
        let batch_id = row
            .get("batch_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| StoreError::Json("missing column 'batch_id'".to_string()))?
            .to_string();
        let select = crate::projections::projection_version_select_sql(
            self.config.database.as_str(),
            name,
            as_of_version,
        )
        .ok_or_else(|| StoreError::Schema(format!("unknown projection {name:?}")))?;
        let rows = self.fetch_rows(&select, &[]).await?;
        let rows = rows
            .iter()
            .map(|row| ProjectionRow::from_table_json(name, row))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(StagedProjection {
            as_of_version,
            batch_id,
            rows,
        }))
    }

    async fn publish_batch(&self, batch_id: &str) -> Result<Option<u64>, StoreError> {
        // FINAL collapses the replacements of one (projection, version) key
        // to the newest row, so every recorded version appears exactly once.
        // Versions of other batches stay visible: publishing an older batch
        // is harmless because readers resolve the highest published version.
        let rows = self
            .fetch_rows(
                &format!(
                    "SELECT projection_name, toString(as_of_version) AS as_of_version, \
                     batch_id, toString(source_event_max) AS source_event_max, \
                     toString(rebuilt_at) AS rebuilt_at, toString(published) AS published \
                     FROM {}.projection_versions FINAL",
                    self.config.database
                ),
                &[],
            )
            .await?;
        let mut newest_rebuilt: Option<UtcMillis> = None;
        let mut batch_rows: Vec<(String, u64, Option<String>, bool)> = Vec::new();
        // The highest version readers see per projection before this publish.
        let mut published_max: BTreeMap<String, u64> = BTreeMap::new();
        for row in &rows {
            let text = |key: &str| row.get(key).and_then(serde_json::Value::as_str);
            if let Some(rebuilt) = text("rebuilt_at").and_then(|value| {
                // ClickHouse renders DateTime64 as `YYYY-MM-DD hh:mm:ss.fff`.
                UtcMillis::parse_rfc3339(value).ok()
            }) {
                newest_rebuilt =
                    Some(newest_rebuilt.map_or(rebuilt, |current| current.max(rebuilt)));
            }
            let projection_name = text("projection_name")
                .ok_or_else(|| StoreError::Json("missing column 'projection_name'".to_string()))?
                .to_string();
            let as_of_version = json_u64(row.get("as_of_version"))
                .ok_or_else(|| StoreError::Json("missing column 'as_of_version'".to_string()))?;
            let published = json_u64(row.get("published")) == Some(1);
            if published {
                published_max
                    .entry(projection_name.clone())
                    .and_modify(|current| *current = (*current).max(as_of_version))
                    .or_insert(as_of_version);
            }
            if text("batch_id") != Some(batch_id) {
                continue;
            }
            let source_event_max = text("source_event_max")
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            batch_rows.push((projection_name, as_of_version, source_event_max, published));
        }
        let Some(published_version) = batch_rows.iter().map(|(_, version, _, _)| *version).max()
        else {
            return Ok(None);
        };
        // Replace the batch's still-unpublished version rows with published
        // ones. The ReplacingMergeTree keeps the greatest rebuilt_at per
        // (projection, version), so the marker must be strictly newer than
        // every stored one.
        let unpublished: Vec<&(String, u64, Option<String>, bool)> = batch_rows
            .iter()
            .filter(|(_, _, _, published)| !*published)
            .collect();
        if !unpublished.is_empty() {
            let now = UtcMillis::system_now().map_err(|error| {
                StoreError::Config(format!("system clock unavailable: {error}"))
            })?;
            let base = newest_rebuilt.map_or(now, |newest| newest.max(now));
            let published_at = UtcMillis::from_unix_millis(base.unix_millis().saturating_add(1));
            let published_rows: Vec<serde_json::Value> = unpublished
                .iter()
                .map(|(projection_name, as_of_version, source_event_max, _)| {
                    serde_json::json!({
                        "projection_name": projection_name,
                        "as_of_version": as_of_version,
                        "batch_id": batch_id,
                        "source_event_max": source_event_max,
                        "rebuilt_at": published_at.to_clickhouse(),
                        "published": 1,
                    })
                })
                .collect();
            self.insert_json_rows("projection_versions", &published_rows)
                .await?;
        }
        // Readers now resolve `max(published, this batch)` per projection;
        // prune everything below it so staged and superseded versions cannot
        // accumulate in the projection tables. Idempotent: a retry prunes
        // again after an interrupted publish.
        for (projection_name, version, _, _) in &batch_rows {
            published_max
                .entry(projection_name.clone())
                .and_modify(|current| *current = (*current).max(*version))
                .or_insert(*version);
        }
        for (projection_name, max_published) in published_max {
            self.prune_superseded_versions(&projection_name, max_published)
                .await?;
        }
        Ok(Some(published_version))
    }
}

/// Chronological order of events as both readers feed them to the read
/// models: occurrence time, then producing boundary, then identity. The
/// server orders by the stored `source` string, so the comparison uses
/// [`EventSource::as_str`] rather than the enum's declaration order; the two
/// readers must see the identical order or a tie between two facts could
/// resolve differently per backend.
pub(crate) fn chronological_order(left: &DomainEvent, right: &DomainEvent) -> std::cmp::Ordering {
    left.occurred_at_utc
        .cmp(&right.occurred_at_utc)
        .then_with(|| left.source.as_str().cmp(right.source.as_str()))
        .then_with(|| left.event_id.cmp(&right.event_id))
}

/// Rebuild a [`DomainEvent`] from a `domain_events` row (FINAL-read shape).
pub(crate) fn domain_event_from_row(row: &serde_json::Value) -> Result<DomainEvent, StoreError> {
    use crate::envelope::{EventSource, EventType};
    let text = |key: &str| row.get(key).and_then(serde_json::Value::as_str);
    let string = |key: &str| text(key).map(str::to_string);
    let required = |key: &str| {
        text(key)
            .map(str::to_string)
            .ok_or_else(|| StoreError::Json(format!("row is missing '{key}'")))
    };
    let parse_time = |key: &str| {
        text(key)
            .and_then(|value| UtcMillis::parse_rfc3339(value).ok())
            .ok_or_else(|| StoreError::Json(format!("row has an unreadable '{key}'")))
    };
    let event_type_text = required("event_type")?;
    let event_type = EventType::parse(&event_type_text)
        .ok_or_else(|| StoreError::Json(format!("unknown event_type '{event_type_text}'")))?;
    let source_text = required("source")?;
    let source = EventSource::parse(&source_text)
        .ok_or_else(|| StoreError::Json(format!("unknown source '{source_text}'")))?;
    let payload_json = required("payload_json")?;
    let payload = serde_json::from_str(&payload_json)
        .map_err(|error| StoreError::Json(format!("payload_json: {error}")))?;
    Ok(DomainEvent {
        event_id: required("event_id")?,
        event_type,
        schema_version: row
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(1),
        occurred_at_utc: parse_time("occurred_at_utc")?,
        received_at_utc: parse_time("received_at_utc")?,
        environment: required("environment")?,
        service_version: required("service_version")?,
        source,
        publisher_id: string("publisher_id"),
        platform: string("platform"),
        object_type: string("object_type"),
        object_id: string("object_id"),
        correlation_id: string("correlation_id"),
        causation_id: string("causation_id"),
        outcome: string("outcome"),
        reason_code: string("reason_code"),
        payload,
    })
}

#[async_trait]
impl ProjectionStore for MemoryStore {
    async fn load_events(&self) -> Result<Vec<DomainEvent>, StoreError> {
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            let mut events: Vec<DomainEvent> = state.events.values().cloned().collect();
            // The ClickHouse reader reads `ORDER BY occurred_at_utc, source,
            // event_id`; the fold-based read models must see the identical
            // order so ties between two facts resolve the same way.
            events.sort_by(chronological_order);
            Ok(events)
        })
    }

    async fn load_report_days(&self) -> Result<Vec<ReportDayRow>, StoreError> {
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            Ok(state.report_days.values().cloned().collect())
        })
    }

    async fn load_accounts(&self) -> Result<Vec<AccountRow>, StoreError> {
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            Ok(state.accounts.values().cloned().collect())
        })
    }

    async fn write_projection(
        &self,
        name: ProjectionName,
        as_of_version: u64,
        batch_id: &str,
        rows: &[ProjectionRow],
    ) -> Result<(), StoreError> {
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            // Staged: a new version starts unpublished and only
            // `publish_batch` makes it reader-visible.
            state.projections.entry(name.as_str()).or_default().insert(
                as_of_version,
                ProjectionSnapshot {
                    batch_id: batch_id.to_string(),
                    published: false,
                    rows: rows.to_vec(),
                },
            );
            Ok(())
        })
    }

    async fn load_projection(
        &self,
        name: ProjectionName,
    ) -> Result<Vec<ProjectionRow>, StoreError> {
        self.with_state(|state| {
            Ok(published_snapshot(state, name.as_str())
                .map(|snapshot| snapshot.rows.clone())
                .unwrap_or_default())
        })
    }

    async fn projection_version(&self, name: ProjectionName) -> Result<Option<u64>, StoreError> {
        self.with_state(|state| {
            Ok(state.projections.get(name.as_str()).and_then(|versions| {
                versions
                    .iter()
                    .filter(|(_, snapshot)| snapshot.published)
                    .map(|(version, _)| *version)
                    .max()
            }))
        })
    }

    async fn projection_versions(&self) -> Result<Vec<ProjectionVersionRow>, StoreError> {
        // The memory store keeps the same observable shape as the server:
        // one row per projection with a published version, with the newest
        // source event time the stored events admit and a rebuild time of
        // "now".
        let rebuilt_at_unix_ms = UtcMillis::system_now()
            .map_err(|error| StoreError::Config(error.to_string()))?
            .unix_millis();
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            let source_event_max_unix_ms = state
                .events
                .values()
                .map(|event| event.occurred_at_utc.unix_millis())
                .max();
            let mut names: Vec<&str> = state.projections.keys().copied().collect();
            names.sort_unstable();
            Ok(names
                .into_iter()
                .filter_map(|name| {
                    let snapshot = published_snapshot(state, name)?;
                    let version = state
                        .projections
                        .get(name)?
                        .iter()
                        .filter(|(_, candidate)| candidate.published)
                        .map(|(version, _)| *version)
                        .max()?;
                    Some(ProjectionVersionRow {
                        projection_name: name.to_string(),
                        as_of_version: version,
                        batch_id: snapshot.batch_id.clone(),
                        source_event_max_unix_ms,
                        rebuilt_at_unix_ms,
                    })
                })
                .collect())
        })
    }

    async fn latest_projection_version(
        &self,
        name: ProjectionName,
    ) -> Result<Option<u64>, StoreError> {
        self.with_state(|state| {
            Ok(state
                .projections
                .get(name.as_str())
                .and_then(|versions| versions.keys().max().copied()))
        })
    }

    async fn latest_projection(
        &self,
        name: ProjectionName,
    ) -> Result<Option<StagedProjection>, StoreError> {
        self.with_state(|state| {
            Ok(state.projections.get(name.as_str()).and_then(|versions| {
                versions
                    .iter()
                    .max_by_key(|(version, _)| **version)
                    .map(|(version, snapshot)| StagedProjection {
                        as_of_version: *version,
                        batch_id: snapshot.batch_id.clone(),
                        rows: snapshot.rows.clone(),
                    })
            }))
        })
    }

    async fn publish_batch(&self, batch_id: &str) -> Result<Option<u64>, StoreError> {
        self.with_state(|state| {
            if !state.schema_ready {
                return Err(StoreError::SchemaNotReady);
            }
            let mut published_version: Option<u64> = None;
            for versions in state.projections.values_mut() {
                for (version, snapshot) in versions.iter_mut() {
                    if snapshot.batch_id == batch_id {
                        snapshot.published = true;
                        published_version = Some(
                            published_version.map_or(*version, |current| current.max(*version)),
                        );
                    }
                }
            }
            Ok(published_version)
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::envelope::{EventSource, EventType};
    use crate::test_support::{valid_event, valid_event_builder};

    #[test]
    fn observed_sorting_keys_reads_system_tables_rows() {
        let rows = vec![
            serde_json::json!({"name": "customer_daily", "sorting_key": "publisher_id, day, platform, currency, as_of_version"}),
            serde_json::json!({"name": "request_logs", "sorting_key": ""}),
            serde_json::json!({"sorting_key": "x"}),
            serde_json::json!({"name": "no_key"}),
            serde_json::json!({"name": "projection_versions", "sorting_key": "projection_name"}),
        ];
        let observed = observed_sorting_keys(&rows);
        assert_eq!(
            observed,
            vec![
                (
                    "customer_daily".to_string(),
                    "publisher_id, day, platform, currency, as_of_version".to_string()
                ),
                ("request_logs".to_string(), String::new()),
                ("no_key".to_string(), String::new()),
                (
                    "projection_versions".to_string(),
                    "projection_name".to_string()
                ),
            ]
        );
        // The staged planner sees the legacy projection_versions shape and
        // plans its migration.
        let plan = crate::schema::staged_migration_plan("db", &observed).expect("plan");
        assert!(plan
            .iter()
            .any(|sql| sql.contains("EXCHANGE TABLES db.projection_versions")));
        assert!(!plan
            .iter()
            .any(|sql| sql.contains("EXCHANGE TABLES db.customer_daily")));
    }

    #[test]
    fn error_bodies_truncate_on_character_boundaries() {
        // A multibyte character spans byte 512: a byte-index truncate would
        // panic here (and `panic = abort` in release).
        let mut text = "a".repeat(511);
        text.push('é'); // two bytes at 511..513, spanning the boundary
        text.push_str(&"b".repeat(100));
        let bounded = truncate_body(&text);
        assert_eq!(bounded.chars().count(), 512);
        assert!(bounded.ends_with('é'), "{bounded}");
        // A long ASCII body bounds at 512 characters unchanged.
        let long = "x".repeat(2_000);
        assert_eq!(truncate_body(&long).chars().count(), 512);
        // Short bodies pass through untouched.
        assert_eq!(
            truncate_body("short provider error"),
            "short provider error"
        );
    }

    #[test]
    fn request_log_rows_match_the_table_columns() {
        let row = RequestLogRow {
            event_id: "req_1".into(),
            occurred_at_utc: UtcMillis::from_unix_millis(1_792_055_400_125),
            received_at_utc: UtcMillis::from_unix_millis(1_792_055_401_000),
            route_template: "/pricing".into(),
            status: 200,
            status_class: "2xx".into(),
            bot_class: "unclassified".into(),
            referrer_host: Some("example.com".into()),
            campaign_code: None,
            query_keys: vec!["utm_source".into()],
            environment: "production".into(),
        };
        let value = row.to_storage_value();
        assert_eq!(value["occurred_at_utc"], "2026-10-15 09:10:00.125");
        assert_eq!(value["received_at_utc"], "2026-10-15 09:10:01.000");
        assert_eq!(value["route_template"], "/pricing");
        assert_eq!(value["status"], 200);
        assert_eq!(value["status_class"], "2xx");
        assert_eq!(value["query_keys"][0], "utm_source");
        assert!(value["campaign_code"].is_null());
        // The spool shape round-trips through serde for the replay path.
        let json = serde_json::to_string(&row).expect("spool json");
        let back: RequestLogRow = serde_json::from_str(&json).expect("back");
        assert_eq!(back, row);
    }

    #[tokio::test]
    async fn the_http_store_maps_transport_failures() {
        // Port 1 is closed: every network path must surface a typed error
        // instead of panicking, and empty batches must not touch the network.
        let config = ClickHouseConfig {
            http_url: "http://127.0.0.1:1".into(),
            database: "apex_probe".into(),
            request_timeout_ms: 1_500,
            ..ClickHouseConfig::default()
        };
        let store = ClickHouseStore::new(config).expect("store");
        assert_eq!(store.database(), "apex_probe");
        assert_eq!(store.http_url(), "http://127.0.0.1:1");
        assert!(matches!(store.health().await, Err(StoreError::Http(_))));
        assert!(store.ensure_schema().await.is_err());
        // The staged-shape migration runs after the DDL: on an unreachable
        // server it surfaces its own read failure instead of panicking.
        assert!(matches!(
            store.migrate_legacy_staged_tables().await,
            Err(StoreError::Http(_))
        ));
        assert!(store
            .insert(&[valid_event(EventType::JobRun)])
            .await
            .is_err());
        assert!(store.insert_report_days(&[]).await.is_ok());
        assert!(store.insert_accounts(&[]).await.is_ok());
        assert!(store.load_events().await.is_err());
        assert!(store.load_report_days().await.is_err());
        assert!(store
            .projection_version(crate::projections::ProjectionName::CustomerDaily)
            .await
            .is_err());
        assert!(store.drop_database().await.is_err());
        assert!(store.execute_statement("SELECT 1").await.is_err());
    }

    #[test]
    fn rows_round_trip_into_events() {
        let event = valid_event(EventType::FeeComputed);
        let row = serde_json::to_value(event.to_row()).expect("row");
        let back = domain_event_from_row(&row).expect("back");
        assert_eq!(back.event_id, event.event_id);
        assert_eq!(back.event_type, event.event_type);
        assert_eq!(back.source, event.source);
        assert_eq!(back.occurred_at_utc, event.occurred_at_utc);
        assert_eq!(back.received_at_utc, event.received_at_utc);
        assert_eq!(back.payload, event.payload);
        assert_eq!(back.publisher_id, event.publisher_id);
    }

    #[test]
    fn malformed_rows_are_refused_field_by_field() {
        let event = valid_event(EventType::JobRun);
        let row = serde_json::to_value(event.to_row()).expect("row");

        let mut bad = row.clone();
        bad["event_type"] = "not_a_real_event".into();
        assert!(matches!(
            domain_event_from_row(&bad),
            Err(StoreError::Json(_))
        ));

        let mut bad = row.clone();
        bad["source"] = "smoke_signal".into();
        assert!(matches!(
            domain_event_from_row(&bad),
            Err(StoreError::Json(_))
        ));

        let mut bad = row.clone();
        bad.as_object_mut().expect("object").remove("payload_json");
        assert!(matches!(
            domain_event_from_row(&bad),
            Err(StoreError::Json(_))
        ));

        let mut bad = row.clone();
        bad["occurred_at_utc"] = "yesterday".into();
        assert!(matches!(
            domain_event_from_row(&bad),
            Err(StoreError::Json(_))
        ));

        let mut bad = row;
        bad["payload_json"] = "{not json".into();
        assert!(matches!(
            domain_event_from_row(&bad),
            Err(StoreError::Json(_))
        ));
    }

    #[test]
    fn configure_from_lookup_applies_defaults_and_overrides() {
        let config = ClickHouseConfig::from_lookup(&|_| None).expect("defaults");
        assert_eq!(config.http_url, "http://127.0.0.1:8123");
        assert_eq!(config.database, "apex_analytics");

        let overrides: BTreeMap<&str, &str> = [
            ("APEX_CLICKHOUSE_HTTP", "http://127.0.0.1:18123"),
            ("APEX_CLICKHOUSE_DB", "apex_test"),
        ]
        .into_iter()
        .collect();
        let config =
            ClickHouseConfig::from_lookup(&|key| overrides.get(key).map(|v| v.to_string()))
                .expect("overrides");
        assert_eq!(config.http_url, "http://127.0.0.1:18123");
        assert_eq!(config.database, "apex_test");

        // Empty overrides fall back to defaults; a bad db name is refused.
        let empty = ClickHouseConfig::from_lookup(&|key| {
            Some(if key == "APEX_CLICKHOUSE_HTTP" {
                "  ".into()
            } else {
                String::new()
            })
        })
        .expect("empty means default");
        assert_eq!(empty.http_url, "http://127.0.0.1:8123");

        let bad = ClickHouseConfig::from_lookup(&|key| {
            Some(if key == "APEX_CLICKHOUSE_DB" {
                "bad name".into()
            } else {
                String::new()
            })
        });
        assert!(matches!(bad, Err(StoreError::Config(_))));
    }

    #[test]
    fn config_from_env_yields_a_usable_default() {
        // Whatever the host environment holds, the result must be a usable
        // configuration; with the conventional variables unset it is the
        // documented default.
        let config = ClickHouseConfig::from_env().expect("environment config");
        assert!(config.http_url.starts_with("http"), "{}", config.http_url);
        assert!(config.request_timeout_ms > 0);
        assert!(validate_identifier(&config.database).is_ok());
    }

    #[test]
    fn credentials_come_from_the_environment_pair_and_blank_reads_as_unset() {
        let overrides: BTreeMap<&str, &str> = [
            ("APEX_CLICKHOUSE_USER", "report_user"),
            ("APEX_CLICKHOUSE_PASSWORD", "top-secret-value"),
        ]
        .into_iter()
        .collect();
        let config =
            ClickHouseConfig::from_lookup(&|key| overrides.get(key).map(|v| v.to_string()))
                .expect("credential overrides");
        assert_eq!(config.user.as_deref(), Some("report_user"));
        assert_eq!(config.password.as_deref(), Some("top-secret-value"));
        // Whitespace-only values read as unset, exactly like the URL pair.
        let blank = ClickHouseConfig::from_lookup(&|key| {
            (key == "APEX_CLICKHOUSE_PASSWORD").then(|| "   ".to_string())
        })
        .expect("blank credentials");
        assert!(blank.user.is_none());
        assert!(blank.password.is_none());
    }

    #[test]
    fn https_base_urls_and_credential_shapes_are_validated() {
        let https = ClickHouseConfig {
            http_url: "https://clickhouse.example.test:8443".to_string(),
            ..ClickHouseConfig::default()
        };
        https.validate().expect("https is a valid scheme");
        let store = ClickHouseStore::new(https).expect("https store");
        assert_eq!(store.http_url(), "https://clickhouse.example.test:8443");
        // Credentials travel as HTTP header values: control characters that
        // could smuggle a header are refused before any request is built.
        for bad_value in ["report\r\nuser", "user\n"] {
            let bad = ClickHouseConfig {
                user: Some(bad_value.to_string()),
                ..ClickHouseConfig::default()
            };
            assert!(matches!(bad.validate(), Err(StoreError::Config(_))));
        }
    }

    #[test]
    fn the_password_is_redacted_in_debug_output() {
        let config = ClickHouseConfig {
            user: Some("report_user".to_string()),
            password: Some("top-secret-value".to_string()),
            ..ClickHouseConfig::default()
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("top-secret-value"), "{rendered}");
        assert!(rendered.contains("[redacted]"), "{rendered}");
        // The store derives Debug over the config, so it inherits the
        // redaction.
        let store = ClickHouseStore::new(config).expect("store");
        let rendered = format!("{store:?}");
        assert!(!rendered.contains("top-secret-value"), "{rendered}");
        assert!(rendered.contains("[redacted]"), "{rendered}");
    }

    /// A one-shot HTTP/1.1 capture server: accepts one connection, reads the
    /// request head and body, answers `200` with an empty body, and hands the
    /// raw request text to the test through the returned channel.
    fn capture_one_request() -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let port = listener.local_addr().expect("address").port();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut raw = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                raw.extend_from_slice(&buffer[..read]);
                let Some(head_end) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
                let content_length = head.lines().find_map(|line| {
                    let line = line.to_ascii_lowercase();
                    line.strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                });
                if raw.len() >= head_end + 4 + content_length.unwrap_or(0) {
                    break;
                }
            }
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = sender.send(String::from_utf8_lossy(&raw).to_string());
        });
        (port, receiver)
    }

    #[tokio::test]
    async fn credentials_attach_the_clickhouse_headers_only_when_configured() {
        // With credentials: the standard headers carry them and the body is
        // unchanged.
        let (port, capture) = capture_one_request();
        let store = ClickHouseStore::new(ClickHouseConfig {
            http_url: format!("http://127.0.0.1:{port}"),
            database: "apex_test".to_string(),
            request_timeout_ms: 5_000,
            user: Some("report_user".to_string()),
            password: Some("top-secret-value".to_string()),
        })
        .expect("store");
        let answer = store.execute_statement("SELECT 1").await.expect("200");
        assert_eq!(answer, "");
        let raw = capture
            .recv_timeout(Duration::from_secs(5))
            .expect("captured request");
        let head = raw.to_ascii_lowercase();
        assert!(
            head.contains("x-clickhouse-user: report_user"),
            "missing user header: {raw}"
        );
        assert!(
            head.contains("x-clickhouse-key: top-secret-value"),
            "missing key header: {raw}"
        );
        assert!(raw.contains("SELECT 1"), "body changed: {raw}");

        // Without credentials: neither header is present (the unauthenticated
        // request shape is byte-for-byte the legacy one).
        let (port, capture) = capture_one_request();
        let store = ClickHouseStore::new(ClickHouseConfig {
            http_url: format!("http://127.0.0.1:{port}"),
            database: "apex_test".to_string(),
            request_timeout_ms: 5_000,
            user: None,
            password: None,
        })
        .expect("store");
        store.execute_statement("SELECT 1").await.expect("200");
        let raw = capture
            .recv_timeout(Duration::from_secs(5))
            .expect("captured request");
        let head = raw.to_ascii_lowercase();
        assert!(
            !head.contains("x-clickhouse"),
            "credential header without configuration: {raw}"
        );
    }

    #[tokio::test]
    async fn https_uses_default_certificate_verification() {
        use std::io::Write as _;

        // A local TLS server presenting a self-signed certificate the client
        // has no reason to trust. `https` must be attempted (the connection
        // reaches the handshake) and the untrusted chain must be refused by
        // the default verification, never accepted.
        let cert = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tls/self-signed-cert.der"
        ))
        .expect("certificate fixture");
        let key = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tls/self-signed-key.der"
        ))
        .expect("key fixture");
        let server_config = std::sync::Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert)],
                    rustls::pki_types::PrivateKeyDer::Pkcs8(
                        rustls::pki_types::PrivatePkcs8KeyDer::from(key),
                    ),
                )
                .expect("server config"),
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let port = listener.local_addr().expect("address").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let config = std::sync::Arc::clone(&server_config);
                std::thread::spawn(move || {
                    let Ok(mut connection) = rustls::ServerConnection::new(config) else {
                        return;
                    };
                    // The client aborts after rejecting the certificate; any
                    // I/O outcome is fine here, the client-side assertion is
                    // what the test pins.
                    let _ = connection.complete_io(&mut stream);
                });
            }
        });
        let config = ClickHouseConfig {
            http_url: format!("https://127.0.0.1:{port}"),
            database: "apex_test".to_string(),
            request_timeout_ms: 5_000,
            ..ClickHouseConfig::default()
        };
        config.validate().expect("https base URL is valid");
        let store = ClickHouseStore::new(config).expect("https store");
        let error = store
            .execute_statement("SELECT 1")
            .await
            .expect_err("the untrusted self-signed certificate must be refused");
        let message = error.to_string();
        assert!(
            message.to_ascii_lowercase().contains("certificate")
                || message.contains("UnknownIssuer"),
            "expected a certificate-verification failure, got: {message}"
        );
        // The server really was a TLS listener: it saw an incoming handshake.
        // (If the client had fallen back to plaintext the request would have
        // hung or failed differently; the certificate error above proves the
        // handshake happened and verification ran.)
        let _ = std::io::stderr().flush();
    }

    fn queue_row(object_id: &str) -> ProjectionRow {
        ProjectionRow::OperatorQueues(crate::projections::OperatorQueueRow {
            queue: "incidents".into(),
            object_id: object_id.into(),
            publisher_id: "northstar".into(),
            severity: "critical".into(),
            age_seconds: 60,
            owner: None,
            next_action: "review_incident".into(),
        })
    }

    #[tokio::test]
    async fn memory_store_keeps_staged_versions_invisible_until_published() {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        // An unpublished write is recorded but no reader sees it.
        store
            .write_projection(
                crate::projections::ProjectionName::OperatorQueues,
                1,
                "batch-1",
                &[queue_row("inc_1")],
            )
            .await
            .expect("write 1");
        assert_eq!(
            store
                .projection_version(crate::projections::ProjectionName::OperatorQueues)
                .await
                .expect("version"),
            None
        );
        assert!(store
            .load_projection(crate::projections::ProjectionName::OperatorQueues)
            .await
            .expect("read")
            .is_empty());
        assert!(store
            .projection_versions()
            .await
            .expect("versions")
            .is_empty());
        let staged = store
            .latest_projection(crate::projections::ProjectionName::OperatorQueues)
            .await
            .expect("staged")
            .expect("recorded");
        assert_eq!(staged.as_of_version, 1);
        assert_eq!(staged.batch_id, "batch-1");

        // A newer staged version does not replace the published v1.
        store
            .write_projection(
                crate::projections::ProjectionName::OperatorQueues,
                2,
                "batch-2",
                &[queue_row("inc_2")],
            )
            .await
            .expect("write 2");
        assert_eq!(
            store
                .latest_projection_version(crate::projections::ProjectionName::OperatorQueues)
                .await
                .expect("latest"),
            Some(2)
        );
        assert_eq!(
            store.publish_batch("batch-1").await.expect("publish"),
            Some(1)
        );
        assert_eq!(
            store
                .projection_version(crate::projections::ProjectionName::OperatorQueues)
                .await
                .expect("version"),
            Some(1)
        );
        assert_eq!(
            store
                .load_projection(crate::projections::ProjectionName::OperatorQueues)
                .await
                .expect("read"),
            vec![queue_row("inc_1")]
        );
        // Publishing batch-2 switches readers; publish is idempotent and an
        // unknown batch reports nothing to publish.
        assert_eq!(
            store.publish_batch("batch-2").await.expect("publish"),
            Some(2)
        );
        assert_eq!(
            store.publish_batch("batch-2").await.expect("publish"),
            Some(2)
        );
        assert_eq!(store.publish_batch("batch-3").await.expect("publish"), None);
        assert_eq!(
            store
                .load_projection(crate::projections::ProjectionName::OperatorQueues)
                .await
                .expect("read"),
            vec![queue_row("inc_2")]
        );
        let versions = store.projection_versions().await.expect("versions");
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].as_of_version, 2);
        assert_eq!(versions[0].batch_id, "batch-2");
    }

    #[tokio::test]
    async fn memory_store_request_logs_require_the_schema_and_store_rows() {
        let store = MemoryStore::new();
        let row = RequestLogRow {
            event_id: "req_1".into(),
            occurred_at_utc: UtcMillis::from_unix_millis(1_792_055_400_125),
            received_at_utc: UtcMillis::from_unix_millis(1_792_055_401_000),
            route_template: "/pricing".into(),
            status: 200,
            status_class: "2xx".into(),
            bot_class: "unclassified".into(),
            referrer_host: None,
            campaign_code: None,
            query_keys: Vec::new(),
            environment: "production".into(),
        };
        assert!(matches!(
            store.insert_request_logs(std::slice::from_ref(&row)).await,
            Err(StoreError::SchemaNotReady)
        ));
        store.ensure_schema().await.expect("schema");
        let outcome = store
            .insert_request_logs(std::slice::from_ref(&row))
            .await
            .expect("insert");
        assert_eq!(outcome.inserted, 1);
        assert_eq!(store.request_logs(), vec![row]);
    }

    #[test]
    fn config_validation_refuses_odd_urls_and_timeouts() {
        let bad_scheme = ClickHouseConfig {
            http_url: "ftp://host".into(),
            ..ClickHouseConfig::default()
        };
        assert!(bad_scheme.validate().is_err());
        let bad_timeout = ClickHouseConfig {
            http_url: "http://host:8123".into(),
            request_timeout_ms: 0,
            ..ClickHouseConfig::default()
        };
        assert!(bad_timeout.validate().is_err());
    }

    #[tokio::test]
    async fn memory_store_lifecycle_and_dedup_semantics() {
        let store = MemoryStore::new();
        let event = valid_event(EventType::ReportPullCompleted);

        // Writes require the schema, like ClickHouse.
        assert!(matches!(
            store.insert(std::slice::from_ref(&event)).await,
            Err(StoreError::SchemaNotReady)
        ));
        store.ensure_schema().await.expect("schema");
        let health = store.health().await.expect("health");
        assert_eq!(health.database, "memory");

        // First insert lands.
        let outcome = store
            .insert(std::slice::from_ref(&event))
            .await
            .expect("insert");
        assert_eq!(outcome.inserted, 1);
        assert_eq!(outcome.duplicates_observed, Some(0));

        // A replay with an EQUAL received time is not a duplicate: the last
        // insert wins, exactly like ReplacingMergeTree with an equal
        // version, so the replay replaces the stored row.
        let replay = store
            .insert(std::slice::from_ref(&event))
            .await
            .expect("replay");
        assert_eq!(replay.duplicates_observed, Some(0));
        let events = store.load_events().await.expect("load");
        assert_eq!(events.len(), 1);
        assert_eq!(store.insert_stats(), (2, 0));

        // A corrected re-emit with a newer received time wins.
        let mut corrected = event.clone();
        corrected.received_at_utc =
            UtcMillis::from_unix_millis(event.received_at_utc.unix_millis() + 1_000);
        corrected.reason_code = Some("corrected".into());
        let outcome = store.insert(&[corrected.clone()]).await.expect("correct");
        assert_eq!(outcome.duplicates_observed, Some(0));
        let events = store.load_events().await.expect("load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason_code.as_deref(), Some("corrected"));

        // An older replay cannot overwrite the newer fact.
        let outcome = store
            .insert(std::slice::from_ref(&event))
            .await
            .expect("older");
        assert_eq!(outcome.duplicates_observed, Some(1));
        let events = store.load_events().await.expect("load");
        assert_eq!(events[0].reason_code.as_deref(), Some("corrected"));

        // The same event_id from another source is a different key.
        let mut other_source = valid_event_builder(EventType::ReportPullCompleted)
            .with_event_id(event.event_id.clone())
            .build()
            .expect("other source built");
        other_source.source = crate::envelope::EventSource::Connector;
        store.insert(&[other_source]).await.expect("other source");
        assert_eq!(store.load_events().await.expect("load").len(), 2);
    }

    #[tokio::test]
    async fn memory_store_report_days_and_accounts_deduplicate_by_key() {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        let day = ReportDayRow {
            publisher_id: "northstar".into(),
            platform: "applovin_max".into(),
            platform_account_id: "max_acct_1".into(),
            day: "2026-10-01".into(),
            finality: "final".into(),
            revenue_micros: Some(5_150_000_000),
            impressions: Some(2_500_000),
            currency: Some("USD".into()),
            country: None,
            ad_unit_id: None,
            format: None,
            source_digest: None,
        };
        let outcome = store
            .insert_report_days(std::slice::from_ref(&day))
            .await
            .expect("insert");
        assert_eq!(outcome.duplicates_observed, Some(0));
        let outcome = store.insert_report_days(&[day]).await.expect("replay");
        assert_eq!(outcome.duplicates_observed, Some(1));
        assert_eq!(store.report_day_count(), 1);

        let account = AccountRow {
            publisher_id: "northstar".into(),
            account_id: "acct_1".into(),
            stage: "audit_delivered".into(),
            owner: Some("maya".into()),
            platforms: vec!["applovin_max".into()],
            first_seen: "2026-09-01".into(),
            stage_updated: "2026-10-15".into(),
        };
        store
            .insert_accounts(std::slice::from_ref(&account))
            .await
            .expect("account");
        assert_eq!(store.load_accounts().await.expect("load").len(), 1);
    }

    #[tokio::test]
    async fn memory_store_replay_with_an_equal_received_time_keeps_the_last_row() {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        let first = valid_event(EventType::ReportPullCompleted);
        let outcome = store
            .insert(std::slice::from_ref(&first))
            .await
            .expect("first");
        assert_eq!(outcome.duplicates_observed, Some(0));

        // ReplacingMergeTree keeps the LAST inserted row when the version
        // (received time) is equal; the memory store must not drop the new
        // row, so the LAST content is what every reader sees.
        let mut last = first.clone();
        last.reason_code = Some("last-insert-wins".into());
        let outcome = store
            .insert(std::slice::from_ref(&last))
            .await
            .expect("equal received replay");
        assert_eq!(outcome.duplicates_observed, Some(0));
        let events = store.load_events().await.expect("load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason_code.as_deref(), Some("last-insert-wins"));

        // A strictly older replay is still dropped and counted.
        let mut older = first;
        older.received_at_utc =
            UtcMillis::from_unix_millis(older.received_at_utc.unix_millis() - 1);
        older.reason_code = Some("stale".into());
        let outcome = store.insert(&[older]).await.expect("older replay");
        assert_eq!(outcome.duplicates_observed, Some(1));
        let events = store.load_events().await.expect("load");
        assert_eq!(events[0].reason_code.as_deref(), Some("last-insert-wins"));
    }

    #[tokio::test]
    async fn memory_store_load_events_orders_chronologically() {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        let base = valid_event(EventType::JobRun);
        let at = |offset_ms: i64| {
            UtcMillis::from_unix_millis(base.occurred_at_utc.unix_millis() + offset_ms)
        };
        let later = DomainEvent {
            event_id: "z_later".into(),
            source: EventSource::AccessLog,
            occurred_at_utc: at(60_000),
            received_at_utc: at(60_000),
            ..base.clone()
        };
        let same_m = DomainEvent {
            event_id: "m_same".into(),
            ..base.clone()
        };
        let same_n = DomainEvent {
            event_id: "n_same".into(),
            ..base.clone()
        };
        // `finance` sorts before `worker`, so this event must come first even
        // though it shares the occurrence time of the later-inserted pair.
        let same_finance = DomainEvent {
            event_id: "a_same".into(),
            source: EventSource::Finance,
            ..base.clone()
        };
        // Deliberately out of order: identity order would be a, m, n, z.
        store
            .insert(&[later, same_n, same_finance, same_m])
            .await
            .expect("insert");
        let events = store.load_events().await.expect("load");
        let ids: Vec<&str> = events.iter().map(|event| event.event_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["a_same", "m_same", "n_same", "z_later"],
            "chronological order (occurred, source, event_id) like the server read"
        );
    }

    #[tokio::test]
    async fn report_day_rows_with_distinct_breakdown_dimensions_are_kept() {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        let base = ReportDayRow {
            publisher_id: "northstar".into(),
            platform: "applovin_max".into(),
            platform_account_id: "max_acct_1".into(),
            day: "2026-10-01".into(),
            finality: "final".into(),
            revenue_micros: Some(1_000_000),
            impressions: Some(100),
            currency: Some("USD".into()),
            country: Some("DE".into()),
            ad_unit_id: Some("rw_1".into()),
            format: Some("rewarded".into()),
            source_digest: Some("d".repeat(64)),
        };
        let gb = ReportDayRow {
            country: Some("GB".into()),
            revenue_micros: Some(2_000_000),
            ..base.clone()
        };
        let outcome = store
            .insert_report_days(&[base.clone(), gb])
            .await
            .expect("insert");
        assert_eq!(outcome.duplicates_observed, Some(0));
        // The dedup key mirrors the `report_days` sorting key, which includes
        // country, ad unit and format: both rows are separate facts.
        assert_eq!(store.report_day_count(), 2);
        let rows = store.load_report_days().await.expect("load");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.country.as_deref() == Some("DE")));
        assert!(rows.iter().any(|row| row.country.as_deref() == Some("GB")));

        // Only the identical full key is a replay duplicate.
        let outcome = store
            .insert_report_days(std::slice::from_ref(&base))
            .await
            .expect("replay");
        assert_eq!(outcome.duplicates_observed, Some(1));
        assert_eq!(store.report_day_count(), 2);
    }
}
