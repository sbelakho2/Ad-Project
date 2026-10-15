//! Analytics wiring: the ClickHouse event sink and the tenant-scoped reader.
//!
//! Two env-driven halves:
//!
//! - the **reader** behind `/console/analytics`, built from
//!   `APEX_CLICKHOUSE_HTTP` and `APEX_CLICKHOUSE_DB`; when either is unset the
//!   console renders the honest not-available state with the deployment
//!   reason, and a query failure renders "the analytics store did not answer"
//!   instead of a fabricated number;
//! - the **emitter** ([`apex_analytics::AnalyticsClient`]) that records the
//!   catalog events this service originates: free-audit and API lead
//!   acceptance/rejection, audit completion/failure, connection
//!   validation/expiry/revocation, proposal approval/decline, ledger
//!   verification and verification failure. Events are built through the
//!   client so every envelope carries its deployment labels; a disabled sink
//!   logs and emits nothing.
//!
//! Stable event ids make at-least-once retries deduplicate at the store's
//! `(source, event_id)` key: the lead request id for request events, the
//! snapshot digest for audit completions, the ledger's highest verified
//! sequence for verifications.

use std::sync::Arc;

use apex_analytics::{
    AnalyticsClient, AnalyticsReader, ClickHouseConfig, ClickHouseReader, ClickHouseStore,
    DomainEvent, EventPayload, EventSource, EventType, UtcMillis,
};

/// Deployment label stamped on every emitted event.
pub const ANALYTICS_ENV_ENV: &str = "APEX_ANALYTICS_ENV";
/// Default deployment label.
pub const DEFAULT_ANALYTICS_ENV: &str = "production";
/// ClickHouse base URL (shared convention with `apex-analytics`).
pub const CLICKHOUSE_HTTP_ENV: &str = "APEX_CLICKHOUSE_HTTP";
/// ClickHouse database (shared convention with `apex-analytics`).
pub const CLICKHOUSE_DB_ENV: &str = "APEX_CLICKHOUSE_DB";
/// Read timeout for console queries: a page must fail fast, never hang.
pub const CONSOLE_QUERY_TIMEOUT_MS: u64 = 5_000;

/// The reason the console shows when no store is configured in this
/// deployment (the frozen 503 family phrasing applies to the panel).
pub const NOT_CONFIGURED_REASON: &str = "the analytics store is not configured in this deployment";
/// The reason the console shows when a configured store did not answer.
pub const UNREACHABLE_REASON: &str = "the analytics store did not answer";

/// The reader plus emitter of this deployment.
#[derive(Debug, Clone)]
pub struct AnalyticsAccess {
    reader: Option<Arc<dyn AnalyticsReader>>,
    client: Option<AnalyticsClient>,
    disabled_reason: Option<String>,
}

impl AnalyticsAccess {
    /// A deployment without an analytics store: every emit is logged and
    /// skipped, and the console renders `reason` as the not-available state.
    pub fn disabled(reason: impl Into<String>) -> Self {
        Self {
            reader: None,
            client: None,
            disabled_reason: Some(reason.into()),
        }
    }

    /// Build from the process environment. Unset ClickHouse settings disable
    /// the sink with a logged notice; an invalid configuration is refused the
    /// same way (the API never fails to boot over telemetry).
    pub fn from_env() -> Self {
        Self::from_lookup(&|key| crate::config::env_value(key))
    }

    /// Build from an environment-like lookup (tests pin explicit values).
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Self {
        let http = lookup(CLICKHOUSE_HTTP_ENV).filter(|value| !value.trim().is_empty());
        let db = lookup(CLICKHOUSE_DB_ENV).filter(|value| !value.trim().is_empty());
        if http.is_none() && db.is_none() {
            tracing::warn!(
                "analytics sink disabled: {CLICKHOUSE_HTTP_ENV} / {CLICKHOUSE_DB_ENV} are not set"
            );
            return Self::disabled(NOT_CONFIGURED_REASON);
        }
        let mut config = match ClickHouseConfig::from_lookup(lookup) {
            Ok(config) => config,
            Err(error) => {
                tracing::error!(%error, "analytics configuration is invalid; sink disabled");
                return Self::disabled(NOT_CONFIGURED_REASON);
            }
        };
        config.request_timeout_ms = CONSOLE_QUERY_TIMEOUT_MS;
        let store = match ClickHouseStore::new(config) {
            Ok(store) => Arc::new(store),
            Err(error) => {
                tracing::error!(%error, "analytics store could not be built; sink disabled");
                return Self::disabled(NOT_CONFIGURED_REASON);
            }
        };
        let reader: Arc<dyn AnalyticsReader> = Arc::new(ClickHouseReader::new((*store).clone()));
        let environment = lookup(ANALYTICS_ENV_ENV)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ANALYTICS_ENV.to_string());
        let client = AnalyticsClient::new(
            store,
            environment,
            format!("apex-api-{}", env!("CARGO_PKG_VERSION")),
        );
        Self {
            reader: Some(reader),
            client: Some(client),
            disabled_reason: None,
        }
    }

    /// An enabled access over an explicit reader and client (tests).
    pub fn enabled(reader: Arc<dyn AnalyticsReader>, client: AnalyticsClient) -> Self {
        Self {
            reader: Some(reader),
            client: Some(client),
            disabled_reason: None,
        }
    }

    /// The reader, when this deployment has one.
    pub fn reader(&self) -> Option<&Arc<dyn AnalyticsReader>> {
        self.reader.as_ref()
    }

    /// The emitter, when this deployment has one.
    pub fn client(&self) -> Option<&AnalyticsClient> {
        self.client.as_ref()
    }

    /// True when reads and emits are enabled.
    pub fn is_enabled(&self) -> bool {
        self.reader.is_some()
    }

    /// The console-facing reason reads are unavailable, when they are.
    pub fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }

    /// Build one event through the client and emit it. Disabled sinks log at
    /// debug level and emit nothing; a store failure is logged, never
    /// propagated into a request path.
    async fn emit_one(
        &self,
        event_type: &'static str,
        build: impl FnOnce(&AnalyticsClient) -> Option<DomainEvent>,
    ) {
        let Some(client) = &self.client else {
            tracing::debug!(event_type, "analytics sink disabled; event not emitted");
            return;
        };
        let Some(event) = build(client) else {
            tracing::warn!(
                event_type,
                "analytics event could not be built; not emitted"
            );
            return;
        };
        match client.emit(&event).await {
            Ok(outcome) => {
                tracing::debug!(event_type, delivered = outcome.delivered, "analytics event");
            }
            Err(error) => {
                tracing::warn!(event_type, %error, "analytics event delivery failed");
            }
        }
    }

    /// Free-audit or API lead accepted: `audit_request_accepted`.
    pub async fn emit_audit_request_accepted(
        &self,
        occurred_unix_ms: i64,
        lead_id: &str,
        platforms: Vec<String>,
        range_start: &str,
        range_end: &str,
        source_code: &str,
    ) {
        let lead_id = lead_id.to_string();
        let source_code = source_code.to_string();
        let (range_start, range_end) = (range_start.to_string(), range_end.to_string());
        self.emit_one("audit_request_accepted", move |client| {
            built(
                client,
                EventType::AuditRequestAccepted,
                EventSource::Axum,
                &lead_id,
                occurred_unix_ms,
                EventPayload {
                    lead_id: Some(lead_id.clone()),
                    platforms: Some(platforms),
                    range_start: Some(range_start),
                    range_end: Some(range_end),
                    source_code: Some(source_code),
                    ..EventPayload::default()
                },
                Extras::default(),
            )
        })
        .await;
    }

    /// Free-audit or API lead rejected: `audit_request_rejected`.
    pub async fn emit_audit_request_rejected(
        &self,
        occurred_unix_ms: i64,
        lead_id: &str,
        source_code: &str,
        validation_reason: &str,
        reason_code: &str,
    ) {
        let lead_id = lead_id.to_string();
        let source_code = source_code.to_string();
        let validation_reason = validation_reason.to_string();
        let reason_code = reason_code.to_string();
        self.emit_one("audit_request_rejected", move |client| {
            built(
                client,
                EventType::AuditRequestRejected,
                EventSource::Axum,
                &lead_id,
                occurred_unix_ms,
                EventPayload {
                    lead_id: Some(lead_id.clone()),
                    source_code: Some(source_code),
                    validation_reason: Some(validation_reason),
                    ..EventPayload::default()
                },
                Extras {
                    reason_code: Some(&reason_code),
                    ..Extras::default()
                },
            )
        })
        .await;
    }

    /// Audit completion with the snapshot digest and rule counts.
    #[allow(clippy::too_many_arguments)]
    pub async fn emit_audit_completed(
        &self,
        occurred_unix_ms: i64,
        publisher: &str,
        platform: &str,
        snapshot_digest: &str,
        rule_version: &str,
        eligible_rule_groups: u32,
        expected_rule_groups: u32,
        findings_critical: u32,
        findings_warning: u32,
        findings_info: u32,
        missing_inputs: Vec<String>,
        duration_ms: u64,
    ) {
        let publisher = publisher.to_string();
        let platform = platform.to_string();
        let snapshot_digest = snapshot_digest.to_string();
        let rule_version = rule_version.to_string();
        let event_id = short_id(
            "audit-",
            &format!("{publisher}|{platform}|{snapshot_digest}"),
        );
        self.emit_one("audit_completed", move |client| {
            built(
                client,
                EventType::AuditCompleted,
                EventSource::Axum,
                &event_id,
                occurred_unix_ms,
                EventPayload {
                    snapshot_digest: Some(snapshot_digest),
                    rule_version: Some(rule_version),
                    eligible_rule_groups: Some(eligible_rule_groups),
                    expected_rule_groups: Some(expected_rule_groups),
                    findings_critical: Some(findings_critical),
                    findings_warning: Some(findings_warning),
                    findings_info: Some(findings_info),
                    missing_inputs: Some(missing_inputs),
                    duration_ms: Some(duration_ms),
                    ..EventPayload::default()
                },
                Extras {
                    publisher: Some(&publisher),
                    platform: Some(&platform),
                    ..Extras::default()
                },
            )
        })
        .await;
    }

    /// Audit failure with a bounded error class.
    pub async fn emit_audit_failed(
        &self,
        occurred_unix_ms: i64,
        publisher: &str,
        platform: &str,
        rule_version: &str,
        error_class: &str,
    ) {
        let publisher = publisher.to_string();
        let platform = platform.to_string();
        let rule_version = rule_version.to_string();
        let error_class = error_class.to_string();
        let event_id = short_id(
            "afailed-",
            &format!("{publisher}|{platform}|{occurred_unix_ms}"),
        );
        self.emit_one("audit_failed", move |client| {
            built(
                client,
                EventType::AuditFailed,
                EventSource::Axum,
                &event_id,
                occurred_unix_ms,
                EventPayload {
                    rule_version: Some(rule_version),
                    error_class: Some(error_class),
                    ..EventPayload::default()
                },
                Extras {
                    publisher: Some(&publisher),
                    platform: Some(&platform),
                    outcome: Some("failed"),
                    ..Extras::default()
                },
            )
        })
        .await;
    }

    /// A connection's credential and scope validated.
    #[allow(clippy::too_many_arguments)]
    pub async fn emit_connection_validated(
        &self,
        occurred_unix_ms: i64,
        publisher: &str,
        platform: &str,
        platform_account_id: &str,
        read_capability: bool,
        write_capability: bool,
        validator_version: &str,
    ) {
        self.emit_connection(
            EventType::ConnectionValidated,
            occurred_unix_ms,
            publisher,
            platform,
            platform_account_id,
            read_capability,
            write_capability,
            validator_version,
            None,
        )
        .await;
    }

    /// A previously valid connection expired (auth refusal on re-validation).
    #[allow(clippy::too_many_arguments)]
    pub async fn emit_connection_expired(
        &self,
        occurred_unix_ms: i64,
        publisher: &str,
        platform: &str,
        platform_account_id: &str,
        read_capability: bool,
        write_capability: bool,
        validator_version: &str,
        failure_class: &str,
    ) {
        self.emit_connection(
            EventType::ConnectionExpired,
            occurred_unix_ms,
            publisher,
            platform,
            platform_account_id,
            read_capability,
            write_capability,
            validator_version,
            Some(failure_class),
        )
        .await;
    }

    /// A connection was revoked by the customer.
    #[allow(clippy::too_many_arguments)]
    pub async fn emit_connection_revoked(
        &self,
        occurred_unix_ms: i64,
        publisher: &str,
        platform: &str,
        platform_account_id: &str,
        read_capability: bool,
        write_capability: bool,
        validator_version: &str,
        failure_class: &str,
    ) {
        self.emit_connection(
            EventType::ConnectionRevoked,
            occurred_unix_ms,
            publisher,
            platform,
            platform_account_id,
            read_capability,
            write_capability,
            validator_version,
            Some(failure_class),
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn emit_connection(
        &self,
        event_type: EventType,
        occurred_unix_ms: i64,
        publisher: &str,
        platform: &str,
        platform_account_id: &str,
        read_capability: bool,
        write_capability: bool,
        validator_version: &str,
        failure_class: Option<&str>,
    ) {
        let publisher = publisher.to_string();
        let platform = platform.to_string();
        let platform_account_id = platform_account_id.to_string();
        let validator_version = validator_version.to_string();
        let failure_class = failure_class.map(str::to_string);
        let event_id = short_id(
            "conn-",
            &format!(
                "{}-{publisher}-{platform}-{occurred_unix_ms}",
                event_type.as_str()
            ),
        );
        self.emit_one(event_type.as_str(), move |client| {
            built(
                client,
                event_type,
                EventSource::Axum,
                &event_id,
                occurred_unix_ms,
                EventPayload {
                    platform_account_id: Some(platform_account_id),
                    read_capability: Some(read_capability),
                    write_capability: Some(write_capability),
                    validator_version: Some(validator_version),
                    failure_class,
                    ..EventPayload::default()
                },
                Extras {
                    publisher: Some(&publisher),
                    platform: Some(&platform),
                    ..Extras::default()
                },
            )
        })
        .await;
    }

    /// A proposal was approved by the workspace.
    #[allow(clippy::too_many_arguments)]
    pub async fn emit_proposal_approved(
        &self,
        occurred_unix_ms: i64,
        publisher: &str,
        proposal_id: &str,
        snapshot: &str,
        change_hash: &str,
        policy_version: &str,
        approval_actor: &str,
    ) {
        self.emit_proposal(
            EventType::ProposalApproved,
            occurred_unix_ms,
            publisher,
            proposal_id,
            snapshot,
            change_hash,
            policy_version,
            Some(approval_actor),
        )
        .await;
    }

    /// A proposal was declined by the workspace.
    pub async fn emit_proposal_declined(
        &self,
        occurred_unix_ms: i64,
        publisher: &str,
        proposal_id: &str,
        snapshot: &str,
        change_hash: &str,
        policy_version: &str,
    ) {
        self.emit_proposal(
            EventType::ProposalDeclined,
            occurred_unix_ms,
            publisher,
            proposal_id,
            snapshot,
            change_hash,
            policy_version,
            None,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn emit_proposal(
        &self,
        event_type: EventType,
        occurred_unix_ms: i64,
        publisher: &str,
        proposal_id: &str,
        snapshot: &str,
        change_hash: &str,
        policy_version: &str,
        approval_actor: Option<&str>,
    ) {
        let publisher = publisher.to_string();
        let proposal_id = proposal_id.to_string();
        let snapshot = snapshot.to_string();
        let change_hash = change_hash.to_string();
        let policy_version = policy_version.to_string();
        let approval_actor = approval_actor.map(str::to_string);
        let event_id = short_id("prop-", &format!("{proposal_id}-{}", event_type.as_str()));
        self.emit_one(event_type.as_str(), move |client| {
            built(
                client,
                event_type,
                EventSource::Axum,
                &event_id,
                occurred_unix_ms,
                EventPayload {
                    proposal_id: Some(proposal_id),
                    snapshot: Some(snapshot),
                    change_hash: Some(change_hash),
                    policy_version: Some(policy_version),
                    approval_actor,
                    ..EventPayload::default()
                },
                Extras {
                    publisher: Some(&publisher),
                    ..Extras::default()
                },
            )
        })
        .await;
    }

    /// Ledger history verified through `highest_sequence`.
    pub async fn emit_ledger_verified(
        &self,
        occurred_unix_ms: i64,
        publisher: &str,
        highest_sequence: u64,
        verification_version: &str,
        duration_ms: u64,
    ) {
        let publisher = publisher.to_string();
        let verification_version = verification_version.to_string();
        let event_id = short_id("lgr-", &format!("{publisher}|{highest_sequence}"));
        self.emit_one("ledger_verified", move |client| {
            built(
                client,
                EventType::LedgerVerified,
                EventSource::Axum,
                &event_id,
                occurred_unix_ms,
                EventPayload {
                    highest_sequence: Some(highest_sequence),
                    verification_version: Some(verification_version),
                    duration_ms: Some(duration_ms),
                    ..EventPayload::default()
                },
                Extras {
                    publisher: Some(&publisher),
                    ..Extras::default()
                },
            )
        })
        .await;
    }

    /// Ledger verification failed at a line.
    pub async fn emit_ledger_verification_failed(
        &self,
        occurred_unix_ms: i64,
        publisher: &str,
        highest_sequence: u64,
        verification_version: &str,
        failure_line: u64,
        duration_ms: u64,
    ) {
        let publisher = publisher.to_string();
        let verification_version = verification_version.to_string();
        let event_id = short_id(
            "lgf-",
            &format!("{publisher}|{failure_line}|{occurred_unix_ms}"),
        );
        self.emit_one("ledger_verification_failed", move |client| {
            built(
                client,
                EventType::LedgerVerificationFailed,
                EventSource::Axum,
                &event_id,
                occurred_unix_ms,
                EventPayload {
                    highest_sequence: Some(highest_sequence),
                    verification_version: Some(verification_version),
                    failure_line: Some(failure_line),
                    duration_ms: Some(duration_ms),
                    ..EventPayload::default()
                },
                Extras {
                    publisher: Some(&publisher),
                    ..Extras::default()
                },
            )
        })
        .await;
    }
}

impl Default for AnalyticsAccess {
    fn default() -> Self {
        Self::disabled(NOT_CONFIGURED_REASON)
    }
}

/// A compact stable id: `prefix` plus 12 hex characters of SHA-256 over the
/// material. Short enough to pass the envelope's token heuristic and stable
/// across retries, so the store's `(source, event_id)` key deduplicates.
fn short_id(prefix: &str, material: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(material.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("{prefix}{}", &digest[..12])
}

/// The optional envelope fields an emitter needs beyond the payload: the
/// catalog validates them per family, so they are set before `build`.
#[derive(Debug, Clone, Copy, Default)]
struct Extras<'a> {
    publisher: Option<&'a str>,
    platform: Option<&'a str>,
    outcome: Option<&'a str>,
    reason_code: Option<&'a str>,
}

/// Build one envelope through the client; `None` when the catalog refuses the
/// values (logged by the caller, never emitted half-built).
fn built(
    client: &AnalyticsClient,
    event_type: EventType,
    source: EventSource,
    event_id: &str,
    occurred_unix_ms: i64,
    payload: EventPayload,
    extras: Extras<'_>,
) -> Option<DomainEvent> {
    let at = UtcMillis::from_unix_millis(occurred_unix_ms);
    let mut builder = client
        .builder(event_type, source)
        .with_event_id(event_id)
        .with_occurred_at(at)
        .with_received_at(at)
        .with_payload(payload);
    if let Some(publisher) = extras.publisher {
        builder = builder.with_publisher_id(publisher);
    }
    if let Some(platform) = extras.platform {
        builder = builder.with_platform(platform);
    }
    if let Some(outcome) = extras.outcome {
        builder = builder.with_outcome(outcome);
    }
    if let Some(reason_code) = extras.reason_code {
        builder = builder.with_reason_code(reason_code);
    }
    match builder.build() {
        Ok(event) => Some(event),
        Err(error) => {
            tracing::warn!(event_type = event_type.as_str(), %error, "analytics event refused");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_analytics::store::{EventStore, ProjectionStore};
    use apex_analytics::{MemoryReader, MemoryStore};

    use super::*;

    async fn enabled_access() -> (AnalyticsAccess, Arc<MemoryStore>) {
        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = AnalyticsClient::new(store.clone(), "production", "apex-api-test");
        let reader: Arc<dyn AnalyticsReader> = Arc::new(MemoryReader::new((*store).clone()));
        (AnalyticsAccess::enabled(reader, client), store)
    }

    #[tokio::test]
    async fn from_lookup_disables_the_sink_when_clickhouse_is_unset() {
        let access = AnalyticsAccess::from_lookup(&|_| None);
        assert!(!access.is_enabled());
        assert_eq!(access.disabled_reason(), Some(NOT_CONFIGURED_REASON));
        assert!(access.client().is_none());
        // Disabled emits are safe no-ops.
        access
            .emit_ledger_verified(0, "northstar", 3, "apex-proof-0.1.0", 1)
            .await;
    }

    #[test]
    fn from_lookup_enables_when_the_url_is_set() {
        let lookup = |key: &str| match key {
            CLICKHOUSE_HTTP_ENV => Some("http://127.0.0.1:8123".to_string()),
            CLICKHOUSE_DB_ENV => Some("apex_console_test".to_string()),
            ANALYTICS_ENV_ENV => Some("staging".to_string()),
            _ => None,
        };
        let access = AnalyticsAccess::from_lookup(&lookup);
        assert!(access.is_enabled());
        assert_eq!(access.disabled_reason(), None);
        assert_eq!(
            access.client().expect("client").environment(),
            "staging",
            "the deployment label is read from the environment"
        );
    }

    #[test]
    fn invalid_configuration_disables_instead_of_failing_the_process() {
        let lookup = |key: &str| match key {
            CLICKHOUSE_HTTP_ENV => Some("ftp://not-http".to_string()),
            _ => None,
        };
        let access = AnalyticsAccess::from_lookup(&lookup);
        assert!(!access.is_enabled());
        assert_eq!(access.disabled_reason(), Some(NOT_CONFIGURED_REASON));
    }

    #[tokio::test]
    async fn accepted_lead_events_carry_the_catalog_minimums() {
        let (access, store) = enabled_access().await;
        access
            .emit_audit_request_accepted(
                1_792_055_400_000,
                "req_0123456789",
                vec!["applovin_max".to_string()],
                "2026-09-15",
                "2026-10-14",
                "free-audit",
            )
            .await;
        let events = store.load_events().await.expect("events");
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.event_type, EventType::AuditRequestAccepted);
        assert_eq!(event.source, EventSource::Axum);
        assert_eq!(event.event_id, "req_0123456789");
        assert_eq!(event.environment, "production");
        assert_eq!(
            event.payload.lead_id.as_deref(),
            Some("req_0123456789"),
            "payload minimum: lead_id"
        );
        assert_eq!(
            event.payload.platforms,
            Some(vec!["applovin_max".to_string()])
        );
        assert_eq!(event.payload.source_code.as_deref(), Some("free-audit"));
        assert_eq!(event.payload.range_start.as_deref(), Some("2026-09-15"));
        assert_eq!(event.payload.range_end.as_deref(), Some("2026-10-14"));
    }

    #[tokio::test]
    async fn rejected_lead_events_carry_the_reason_code() {
        let (access, store) = enabled_access().await;
        access
            .emit_audit_request_rejected(
                1,
                "req_abcdefabcd",
                "free-audit",
                "invalid_email",
                "validation_failed",
            )
            .await;
        let events = store.load_events().await.expect("events");
        assert_eq!(events[0].event_type, EventType::AuditRequestRejected);
        assert_eq!(events[0].reason_code.as_deref(), Some("validation_failed"));
        assert_eq!(
            events[0].payload.validation_reason.as_deref(),
            Some("invalid_email")
        );
    }

    #[tokio::test]
    async fn audit_completion_events_carry_digest_counts_and_missing_inputs() {
        let (access, store) = enabled_access().await;
        access
            .emit_audit_completed(
                1_792_055_400_000,
                "northstar",
                "applovin_max",
                &"ab".repeat(32),
                "apex-audit-0.1.0",
                4,
                5,
                1,
                3,
                1,
                vec!["disabled bidder".to_string()],
                120,
            )
            .await;
        let events = store.load_events().await.expect("events");
        let event = &events[0];
        assert_eq!(event.event_type, EventType::AuditCompleted);
        assert_eq!(event.publisher_id.as_deref(), Some("northstar"));
        assert_eq!(event.platform.as_deref(), Some("applovin_max"));
        assert_eq!(event.payload.findings_critical, Some(1));
        assert_eq!(event.payload.eligible_rule_groups, Some(4));
        assert_eq!(event.payload.expected_rule_groups, Some(5));
        assert_eq!(event.payload.duration_ms, Some(120));
        assert_eq!(
            event.payload.missing_inputs,
            Some(vec!["disabled bidder".to_string()])
        );
    }

    #[tokio::test]
    async fn audit_failure_events_carry_outcome_and_error_class() {
        let (access, store) = enabled_access().await;
        access
            .emit_audit_failed(42, "northstar", "admob", "apex-audit-0.1.0", "auth_refused")
            .await;
        let events = store.load_events().await.expect("events");
        let event = &events[0];
        assert_eq!(event.event_type, EventType::AuditFailed);
        assert_eq!(event.outcome.as_deref(), Some("failed"));
        assert_eq!(event.payload.error_class.as_deref(), Some("auth_refused"));
        assert_eq!(
            event.payload.rule_version.as_deref(),
            Some("apex-audit-0.1.0")
        );
    }

    #[tokio::test]
    async fn connection_events_carry_capability_and_failure_class() {
        let (access, store) = enabled_access().await;
        access
            .emit_connection_validated(
                1_792_055_400_000,
                "northstar",
                "applovin_max",
                "acct-1",
                true,
                false,
                "apex-connectors-0.1.0",
            )
            .await;
        access
            .emit_connection_expired(
                1_792_055_400_001,
                "northstar",
                "levelplay",
                "app-key",
                false,
                false,
                "apex-connectors-0.1.0",
                "auth_refused",
            )
            .await;
        access
            .emit_connection_revoked(
                1_792_055_400_002,
                "northstar",
                "admob",
                "accounts/pub-1",
                false,
                false,
                "apex-connectors-0.1.0",
                "customer_revoked",
            )
            .await;
        let events = store.load_events().await.expect("events");
        assert_eq!(events.len(), 3);
        let by_type = |event_type: EventType| {
            events
                .iter()
                .find(|event| event.event_type == event_type)
                .expect("event present")
        };
        let validated = by_type(EventType::ConnectionValidated);
        assert_eq!(validated.payload.read_capability, Some(true));
        assert_eq!(validated.payload.write_capability, Some(false));
        assert_eq!(
            validated.payload.platform_account_id.as_deref(),
            Some("acct-1")
        );
        let expired = by_type(EventType::ConnectionExpired);
        assert_eq!(
            expired.payload.failure_class.as_deref(),
            Some("auth_refused")
        );
        let revoked = by_type(EventType::ConnectionRevoked);
        assert_eq!(
            revoked.payload.failure_class.as_deref(),
            Some("customer_revoked")
        );
    }

    #[tokio::test]
    async fn proposal_and_ledger_events_match_the_catalog() {
        let (access, store) = enabled_access().await;
        access
            .emit_proposal_approved(
                1,
                "northstar",
                "p_0247",
                "digest",
                "change-hash",
                "policy-v1",
                "workspace",
            )
            .await;
        access
            .emit_proposal_declined(
                2,
                "northstar",
                "p_0248",
                "digest",
                "change-hash",
                "policy-v1",
            )
            .await;
        access
            .emit_ledger_verified(3, "northstar", 24, "apex-proof-0.1.0", 5)
            .await;
        access
            .emit_ledger_verification_failed(4, "northstar", 17, "apex-proof-0.1.0", 18, 5)
            .await;
        let events = store.load_events().await.expect("events");
        assert_eq!(events.len(), 4);
        let by_type = |event_type: EventType| {
            events
                .iter()
                .find(|event| event.event_type == event_type)
                .expect("event present")
        };
        let approved = by_type(EventType::ProposalApproved);
        assert_eq!(
            approved.payload.approval_actor.as_deref(),
            Some("workspace")
        );
        let declined = by_type(EventType::ProposalDeclined);
        assert_eq!(declined.payload.approval_actor, None);
        let verified = by_type(EventType::LedgerVerified);
        assert_eq!(verified.payload.highest_sequence, Some(24));
        assert!(verified.event_id.starts_with("lgr-"));
        let failed = by_type(EventType::LedgerVerificationFailed);
        assert_eq!(failed.payload.failure_line, Some(18));
    }

    #[tokio::test]
    async fn repeated_ledger_reads_deduplicate_on_the_event_id_key() {
        let (access, store) = enabled_access().await;
        for _ in 0..3 {
            access
                .emit_ledger_verified(3, "northstar", 24, "apex-proof-0.1.0", 5)
                .await;
        }
        let events = store.load_events().await.expect("events");
        assert_eq!(events.len(), 1, "the same verified sequence is one fact");
    }

    #[tokio::test]
    async fn hostile_payload_values_are_refused_not_emitted() {
        let (access, store) = enabled_access().await;
        // A credential-looking validation reason is refused by the envelope.
        access
            .emit_audit_request_rejected(
                1,
                "req_0123456789",
                "free-audit",
                "Bearer sk_live_secret",
                "validation_failed",
            )
            .await;
        assert!(store.load_events().await.expect("events").is_empty());
    }
}
