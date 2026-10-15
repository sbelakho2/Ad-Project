//! Runner-side event emission into the analytics store.
//!
//! Section 05 (pages 73 to 75) defines the event envelope and the catalog.
//! Every event family whose source is the runner/autopilot is emitted here:
//! report pulls, job runs, experiment lifecycle observations, guardrail
//! events, restore operations, fee computation/holds/invoices, payment events
//! and audit runs. High-cardinality identities ride the event payload, never
//! a metric label.
//!
//! `APEX_CLICKHOUSE_HTTP` selects the deployment: when set, events go through
//! [`apex_analytics::AnalyticsClient`] wrapped in a [`apex_analytics::SpoolSink`]
//! (a failed insert is spooled, never lost); when unset, emission is disabled
//! and the caller logs that state. Tests build the same type over
//! [`apex_analytics::MemoryStore`].
//!
//! ## Spool durability
//!
//! A failed batch is appended to a JSONL spool and stays there until a later
//! replay delivers it. [`RunnerEvents::replay`] re-emits the spool through
//! the primary sink, truncating only delivered records; the CLI dispatch path
//! calls it once at runner start and logs the re-delivery, so a spool that
//! accumulated while ClickHouse was unreachable is drained on the next run
//! instead of sitting beside it forever.
//!
//! The spool path is `APEX_ANALYTICS_SPOOL` when set; otherwise it defaults
//! under the runner's data directory (`APEX_RUNNER_DATA_DIR`, default
//! `.apex`), at `<data dir>/analytics/spool.jsonl`, so production units that
//! set `WorkingDirectory=/var/lib/apex` (or the data-dir variable) write the
//! spool into their durable state, never an arbitrary working directory.

use std::path::PathBuf;
use std::sync::Arc;

use apex_analytics::sink::ReplayOutcome;
use apex_analytics::{
    AnalyticsClient, ClickHouseConfig, ClickHouseStore, DomainEvent, DomainEventBuilder,
    EventPayload, EventSink, EventSource, EventType, SinkError, SpoolSink, UtcMillis,
};

/// The environment variable selecting the ClickHouse HTTP endpoint.
pub const CLICKHOUSE_ENV: &str = "APEX_CLICKHOUSE_HTTP";
/// The environment variable choosing the spool path for failed batches.
pub const SPOOL_ENV: &str = "APEX_ANALYTICS_SPOOL";
/// The environment variable naming the runner's data directory, under which
/// the default spool path lives.
pub const DATA_DIR_ENV: &str = "APEX_RUNNER_DATA_DIR";
/// The environment variable naming the deployment environment label. This is
/// the name `apex-api` reads (`APEX_ANALYTICS_ENV`); the historical
/// runner-only spelling is a deprecated fallback announced once.
pub const ENVIRONMENT_ENV: &str = "APEX_ANALYTICS_ENV";
/// The historical runner-only environment variable name; still read as the
/// deprecated fallback for [`ENVIRONMENT_ENV`].
pub const DEPRECATED_ENVIRONMENT_ENV: &str = "APEX_ANALYTICS_ENVIRONMENT";
/// The default data directory (matching the runner's other state defaults).
pub const DEFAULT_DATA_DIR: &str = ".apex";
/// The spool path under the data directory.
pub const DEFAULT_SPOOL: &str = "analytics/spool.jsonl";

/// A sink handle shared behind the spool wrapper. [`SpoolSink`] is generic
/// over its primary; this erases the primary type so one concrete
/// `SpoolSink<SharedSink>` serves both the ClickHouse deployment and tests
/// while keeping the spool handle available for [`RunnerEvents::replay`].
#[derive(Debug, Clone)]
struct SharedSink(Arc<dyn EventSink>);

#[async_trait::async_trait]
impl EventSink for SharedSink {
    async fn emit(&self, events: &[DomainEvent]) -> Result<apex_analytics::EmitOutcome, SinkError> {
        self.0.emit(events).await
    }
}

/// The runner's event emitter.
#[derive(Debug, Clone)]
pub struct RunnerEvents {
    client: Arc<dyn EventSink>,
    /// The concrete spool handle, present when the sink was built with a
    /// spool; `None` for direct sinks (tests) where replay is a no-op.
    spool: Option<Arc<SpoolSink<SharedSink>>>,
    publisher: String,
    platform: String,
    environment: String,
    service_version: String,
}

impl RunnerEvents {
    /// Build over any sink (tests use `MemoryStore`, deployments the spooled
    /// ClickHouse client). Failed batches of a direct sink are not spooled;
    /// use [`RunnerEvents::with_spool`] for a replayable emitter.
    pub fn new(client: Arc<dyn EventSink>, publisher: impl Into<String>) -> Self {
        Self {
            client,
            spool: None,
            publisher: publisher.into(),
            platform: "applovin_max".to_string(),
            environment: "production".to_string(),
            service_version: format!("apex-runner-{}", env!("CARGO_PKG_VERSION")),
        }
    }

    /// Build over any primary sink with failed batches spooled to
    /// `spool_path`, exposing the concrete spool through
    /// [`RunnerEvents::replay`].
    pub fn with_spool(primary: Arc<dyn EventSink>, spool_path: impl Into<PathBuf>) -> Self {
        let sink = Arc::new(SpoolSink::new(SharedSink(primary), spool_path));
        Self {
            client: sink.clone(),
            spool: Some(sink),
            publisher: String::new(),
            platform: "applovin_max".to_string(),
            environment: "production".to_string(),
            service_version: format!("apex-runner-{}", env!("CARGO_PKG_VERSION")),
        }
    }

    /// Build from the environment: `Ok(None)` when `APEX_CLICKHOUSE_HTTP` is
    /// unset (emission disabled; the caller logs the clear line).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let Some(http) = lookup(CLICKHOUSE_ENV).filter(|value| !value.trim().is_empty()) else {
            return Ok(None);
        };
        let environment = match lookup(ENVIRONMENT_ENV).filter(|value| !value.trim().is_empty()) {
            Some(value) => value,
            None => {
                match lookup(DEPRECATED_ENVIRONMENT_ENV).filter(|value| !value.trim().is_empty()) {
                    Some(value) => {
                        eprintln!(
                        "apex-runner: {DEPRECATED_ENVIRONMENT_ENV} is deprecated; use {ENVIRONMENT_ENV} (the apex-api name)"
                    );
                        value
                    }
                    None => "production".to_string(),
                }
            }
        };
        let spool = lookup(SPOOL_ENV)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                let data_dir = lookup(DATA_DIR_ENV)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_DATA_DIR.to_string());
                format!("{}/{}", data_dir.trim_end_matches('/'), DEFAULT_SPOOL)
            });
        let mut config = ClickHouseConfig::from_lookup(&lookup)
            .map_err(|error| format!("analytics configuration: {error}"))?;
        config.http_url = http.clone();
        let store =
            ClickHouseStore::new(config).map_err(|error| format!("analytics store: {error}"))?;
        let client = AnalyticsClient::new(
            Arc::new(store),
            environment.clone(),
            format!("apex-runner-{}", env!("CARGO_PKG_VERSION")),
        );
        Ok(Some(Self {
            environment,
            ..Self::with_spool(Arc::new(client), spool)
        }))
    }

    /// Re-deliver every spooled batch through the primary sink once, for
    /// example at runner start. Returns the number of records re-delivered;
    /// a failed replay keeps the undelivered records spooled and is reported
    /// on stderr, never silently dropped. A sink without a spool replays
    /// nothing.
    pub async fn replay(&self) -> usize {
        let Some(spool) = &self.spool else {
            return 0;
        };
        match spool.replay_spool().await {
            Ok(ReplayOutcome { replayed, .. }) => replayed,
            Err(error) => {
                eprintln!("apex-analytics: spool replay failed: {error}");
                0
            }
        }
    }

    /// The spool file path when one is attached (tests and operations read
    /// it; production units configure it through `APEX_ANALYTICS_SPOOL`).
    pub fn spool_path(&self) -> Option<&std::path::Path> {
        self.spool.as_ref().map(|spool| spool.spool_path())
    }

    /// The publisher every tenant-scoped event carries.
    #[must_use]
    pub fn with_publisher(mut self, publisher: impl Into<String>) -> Self {
        self.publisher = publisher.into();
        self
    }

    /// Override the deployment environment label (tests use `dev`).
    #[must_use]
    pub fn with_environment(mut self, environment: impl Into<String>) -> Self {
        self.environment = environment.into();
        self
    }

    /// Emit one event; a refusal or delivery failure is reported on stderr
    /// and never aborts the job.
    async fn emit(&self, builder: DomainEventBuilder) {
        let builder = if self.publisher.is_empty() {
            builder
        } else {
            builder.with_publisher_id(self.publisher.clone())
        };
        let event = match builder.build() {
            Ok(event) => event,
            Err(error) => {
                eprintln!("apex-analytics: refusing to emit a malformed event: {error}");
                return;
            }
        };
        if let Err(error) = self.client.emit(std::slice::from_ref(&event)).await {
            eprintln!(
                "apex-analytics: event {} was not delivered: {error}",
                event.event_id
            );
        }
    }

    fn base(
        &self,
        event_type: EventType,
        event_id: String,
        occurred_ms: i64,
    ) -> DomainEventBuilder {
        let at = UtcMillis::from_unix_millis(occurred_ms);
        DomainEvent::builder(event_type, EventSource::Worker)
            .with_event_id(event_id)
            .with_occurred_at(at)
            .with_received_at(at)
            .with_environment(self.environment.clone())
            .with_service_version(self.service_version.clone())
            .with_platform(self.platform.clone())
    }

    /// `job_run` (page 75): schedule and outcome. The event id carries the
    /// job type, so a failed daily and a same-minute monthly are two
    /// distinct events with distinct merge keys (the run reference alone is
    /// minute-granular and would collapse them in the store).
    pub async fn job_run(
        &self,
        job_type: &str,
        job_id: &str,
        occurred_ms: i64,
        outcome: &str,
        duration_ms: u64,
    ) {
        let mut payload = EventPayload::new();
        payload.job_id = Some(job_id.to_string());
        payload.duration_ms = Some(duration_ms);
        let builder = self
            .base(
                EventType::JobRun,
                format!("job_run:{job_type}:{job_id}"),
                occurred_ms,
            )
            .with_object("job", job_id)
            .with_outcome(outcome)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `report_pull_started` (page 74).
    #[allow(clippy::too_many_arguments)]
    pub async fn report_pull_started(
        &self,
        job_id: &str,
        operation: &str,
        experiment_id: &str,
        range_start: &str,
        range_end: &str,
        attempt: u32,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.job_id = Some(job_id.to_string());
        payload.platform_operation = Some(operation.to_string());
        payload.attempt = Some(attempt);
        payload.range_start = Some(range_start.to_string());
        payload.range_end = Some(range_end.to_string());
        payload.experiment_id = Some(experiment_id.to_string());
        let builder = self
            .base(
                EventType::ReportPullStarted,
                format!("report_pull_started:{job_id}:{experiment_id}:{attempt}"),
                occurred_ms,
            )
            .with_object("experiment", experiment_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `report_pull_completed` (page 74): rows, bounds and finality.
    #[allow(clippy::too_many_arguments)]
    pub async fn report_pull_completed(
        &self,
        job_id: &str,
        operation: &str,
        experiment_id: &str,
        range_start: &str,
        range_end: &str,
        rows: u64,
        finality: &str,
        duration_ms: u64,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.job_id = Some(job_id.to_string());
        payload.platform_operation = Some(operation.to_string());
        payload.rows = Some(rows);
        payload.finality = Some(finality.to_string());
        payload.duration_ms = Some(duration_ms);
        payload.range_start = Some(range_start.to_string());
        payload.range_end = Some(range_end.to_string());
        payload.experiment_id = Some(experiment_id.to_string());
        let builder = self
            .base(
                EventType::ReportPullCompleted,
                format!("report_pull_completed:{job_id}:{experiment_id}:{range_end}"),
                occurred_ms,
            )
            .with_object("experiment", experiment_id)
            .with_outcome("completed")
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `report_pull_failed` (page 74): the error class is recorded.
    #[allow(clippy::too_many_arguments)]
    pub async fn report_pull_failed(
        &self,
        job_id: &str,
        operation: &str,
        experiment_id: &str,
        range_start: &str,
        range_end: &str,
        attempt: u32,
        error_class: &str,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.job_id = Some(job_id.to_string());
        payload.platform_operation = Some(operation.to_string());
        payload.attempt = Some(attempt);
        payload.error_class = Some(error_class.to_string());
        payload.range_start = Some(range_start.to_string());
        payload.range_end = Some(range_end.to_string());
        payload.experiment_id = Some(experiment_id.to_string());
        let builder = self
            .base(
                EventType::ReportPullFailed,
                format!("report_pull_failed:{job_id}:{experiment_id}:{attempt}:{occurred_ms}"),
                occurred_ms,
            )
            .with_object("experiment", experiment_id)
            .with_outcome("failed")
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `experiment_registered` (page 75): metric, window and source digest.
    pub async fn experiment_registered(
        &self,
        experiment_id: &str,
        source_digest: &str,
        registered_metric: &str,
        window_days: u32,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.experiment_id = Some(experiment_id.to_string());
        payload.source_digest = Some(source_digest.to_string());
        payload.registered_metric = Some(registered_metric.to_string());
        payload.window_days = Some(window_days);
        let builder = self
            .base(
                EventType::ExperimentRegistered,
                format!("experiment_registered:{experiment_id}"),
                occurred_ms,
            )
            .with_object("experiment", experiment_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `experiment_started` (page 75).
    pub async fn experiment_started(&self, experiment_id: &str, state: &str, occurred_ms: i64) {
        let mut payload = EventPayload::new();
        payload.experiment_id = Some(experiment_id.to_string());
        payload.state = Some(state.to_string());
        let builder = self
            .base(
                EventType::ExperimentStarted,
                format!("experiment_started:{experiment_id}"),
                occurred_ms,
            )
            .with_object("experiment", experiment_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `experiment_pull` (page 75): a finalized pull arrived.
    pub async fn experiment_pull(
        &self,
        experiment_id: &str,
        source_digest: &str,
        range_start: &str,
        range_end: &str,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.experiment_id = Some(experiment_id.to_string());
        payload.source_digest = Some(source_digest.to_string());
        payload.range_start = Some(range_start.to_string());
        payload.range_end = Some(range_end.to_string());
        let builder = self
            .base(
                EventType::ExperimentPull,
                format!("experiment_pull:{experiment_id}:{range_end}"),
                occurred_ms,
            )
            .with_object("experiment", experiment_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `experiment_decided` (page 75): the decision, with the billable bound
    /// on a Keep.
    #[allow(clippy::too_many_arguments)]
    pub async fn experiment_decided(
        &self,
        experiment_id: &str,
        decision: &str,
        point_estimate_bp: Option<i64>,
        decision_bound_bp: Option<i64>,
        reason_code: &str,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.experiment_id = Some(experiment_id.to_string());
        payload.decision = Some(decision.to_string());
        payload.point_estimate_bp = point_estimate_bp;
        payload.decision_bound_bp = decision_bound_bp;
        let builder = self
            .base(
                EventType::ExperimentDecided,
                format!("experiment_decided:{experiment_id}:{occurred_ms}"),
                occurred_ms,
            )
            .with_object("experiment", experiment_id)
            .with_reason_code(reason_code)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `guardrail_fired` (page 75).
    pub async fn guardrail_fired(
        &self,
        guard_type: &str,
        observed_input: &str,
        threshold: &str,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.guard_type = Some(guard_type.to_string());
        payload.observed_input = Some(observed_input.to_string());
        payload.threshold = Some(threshold.to_string());
        let builder = self
            .base(
                EventType::GuardrailFired,
                format!("guardrail_fired:{guard_type}:{occurred_ms}"),
                occurred_ms,
            )
            .with_object("guardrail", guard_type)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `guardrail_restore_requested` (page 75).
    pub async fn restore_requested(
        &self,
        guard_type: &str,
        operation_id: &str,
        target_state: &str,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.guard_type = Some(guard_type.to_string());
        payload.operation_id = Some(operation_id.to_string());
        payload.target_state = Some(target_state.to_string());
        let builder = self
            .base(
                EventType::GuardrailRestoreRequested,
                format!("restore_requested:{operation_id}"),
                occurred_ms,
            )
            .with_object("operation", operation_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `guardrail_restore_confirmed` (page 75): the confirming platform read.
    pub async fn restore_confirmed(
        &self,
        guard_type: &str,
        operation_id: &str,
        target_state: &str,
        confirming_read: &str,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.guard_type = Some(guard_type.to_string());
        payload.operation_id = Some(operation_id.to_string());
        payload.target_state = Some(target_state.to_string());
        payload.confirming_read = Some(confirming_read.to_string());
        let builder = self
            .base(
                EventType::GuardrailRestoreConfirmed,
                format!("restore_confirmed:{operation_id}"),
                occurred_ms,
            )
            .with_object("operation", operation_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `guardrail_restore_failed` (page 75): the state stays unresolved.
    pub async fn restore_failed(
        &self,
        guard_type: &str,
        operation_id: &str,
        target_state: &str,
        error_class: &str,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.guard_type = Some(guard_type.to_string());
        payload.operation_id = Some(operation_id.to_string());
        payload.target_state = Some(target_state.to_string());
        payload.error_class = Some(error_class.to_string());
        let builder = self
            .base(
                EventType::GuardrailRestoreFailed,
                format!("restore_failed:{operation_id}"),
                occurred_ms,
            )
            .with_object("operation", operation_id)
            .with_reason_code(error_class)
            .with_outcome("failed")
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `ledger_verified` (page 75).
    pub async fn ledger_verified(
        &self,
        highest_sequence: u64,
        verification_version: &str,
        duration_ms: u64,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.highest_sequence = Some(highest_sequence);
        payload.verification_version = Some(verification_version.to_string());
        payload.duration_ms = Some(duration_ms);
        let builder = self
            .base(
                EventType::LedgerVerified,
                format!("ledger_verified:{highest_sequence}:{occurred_ms}"),
                occurred_ms,
            )
            .with_object("ledger", "publisher")
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `ledger_verification_failed` (page 75): the failing line is recorded.
    pub async fn ledger_verification_failed(
        &self,
        highest_sequence: u64,
        verification_version: &str,
        failure_line: Option<u64>,
        duration_ms: u64,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.highest_sequence = Some(highest_sequence);
        payload.verification_version = Some(verification_version.to_string());
        payload.duration_ms = Some(duration_ms);
        let failure_line = failure_line.unwrap_or(highest_sequence.saturating_add(1));
        payload.failure_line = Some(failure_line);
        let builder = self
            .base(
                EventType::LedgerVerificationFailed,
                format!("ledger_verification_failed:{failure_line}:{occurred_ms}"),
                occurred_ms,
            )
            .with_object("ledger", "publisher")
            .with_reason_code("verification_failed")
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `fee_computed` (page 75): the full fee basis with proof hashes.
    #[allow(clippy::too_many_arguments)]
    pub async fn fee_computed(
        &self,
        change_id: &str,
        period: &str,
        currency: &str,
        source_amount_micros: i64,
        bound_bp: i64,
        factor_bp: i64,
        rate_bp: u16,
        active_days: u32,
        proof_hashes: Vec<String>,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.change_id = Some(change_id.to_string());
        payload.period = Some(period.to_string());
        payload.currency = Some(currency.to_string());
        payload.source_amount_micros = Some(source_amount_micros);
        payload.bound_bp = Some(bound_bp);
        payload.factor_bp = Some(factor_bp);
        payload.rate_bp = Some(rate_bp);
        payload.active_days = Some(active_days);
        payload.proof_hashes = Some(proof_hashes);
        let builder = self
            .base(
                EventType::FeeComputed,
                format!("fee_computed:{change_id}:{period}"),
                occurred_ms,
            )
            .with_object("change", change_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `fee_held` (page 75): the fee is not issued or collected.
    pub async fn fee_held(
        &self,
        change_id: &str,
        period: &str,
        currency: &str,
        reason_code: &str,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.change_id = Some(change_id.to_string());
        payload.period = Some(period.to_string());
        payload.currency = Some(currency.to_string());
        let builder = self
            .base(
                EventType::FeeHeld,
                format!("fee_held:{change_id}:{period}"),
                occurred_ms,
            )
            .with_object("change", change_id)
            .with_reason_code(reason_code)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `fee_invoice_issued` (page 75): one issued invoice line.
    pub async fn fee_invoice_issued(
        &self,
        change_id: &str,
        period: &str,
        currency: &str,
        invoice_id: &str,
        amount_micros: i64,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.change_id = Some(change_id.to_string());
        payload.period = Some(period.to_string());
        payload.currency = Some(currency.to_string());
        payload.invoice_id = Some(invoice_id.to_string());
        payload.amount_micros = Some(amount_micros);
        let builder = self
            .base(
                EventType::FeeInvoiceIssued,
                format!("fee_invoice_issued:{invoice_id}:{change_id}"),
                occurred_ms,
            )
            .with_object("invoice", invoice_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `audit_completed` (page 74): digest, counts and coverage.
    #[allow(clippy::too_many_arguments)]
    pub async fn audit_completed(
        &self,
        snapshot_digest: &str,
        rule_version: &str,
        eligible_rule_groups: u32,
        expected_rule_groups: u32,
        findings_critical: u32,
        findings_warning: u32,
        findings_info: u32,
        missing_inputs: Vec<String>,
        duration_ms: u64,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.snapshot_digest = Some(snapshot_digest.to_string());
        payload.rule_version = Some(rule_version.to_string());
        payload.eligible_rule_groups = Some(eligible_rule_groups);
        payload.expected_rule_groups = Some(expected_rule_groups);
        payload.findings_critical = Some(findings_critical);
        payload.findings_warning = Some(findings_warning);
        payload.findings_info = Some(findings_info);
        payload.missing_inputs = Some(missing_inputs);
        payload.duration_ms = Some(duration_ms);
        let builder = self
            .base(
                EventType::AuditCompleted,
                format!("audit_completed:{snapshot_digest}"),
                occurred_ms,
            )
            .with_object("snapshot", snapshot_digest)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `audit_failed` (page 74): the error class is recorded.
    pub async fn audit_failed(&self, rule_version: &str, error_class: &str, occurred_ms: i64) {
        let mut payload = EventPayload::new();
        payload.rule_version = Some(rule_version.to_string());
        payload.error_class = Some(error_class.to_string());
        let builder = self
            .base(
                EventType::AuditFailed,
                format!("audit_failed:{rule_version}:{occurred_ms}"),
                occurred_ms,
            )
            .with_object("audit", rule_version)
            .with_outcome("failed")
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `payment_received` (page 75): a durable verified provider event.
    pub async fn payment_received(
        &self,
        psp_event_key: &str,
        invoice_id: &str,
        currency: &str,
        amount_micros: i64,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.psp_event_key = Some(psp_event_key.to_string());
        payload.invoice_id = Some(invoice_id.to_string());
        payload.currency = Some(currency.to_string());
        payload.amount_micros = Some(amount_micros);
        let builder = self
            .base(
                EventType::PaymentReceived,
                format!("payment_received:{psp_event_key}"),
                occurred_ms,
            )
            .with_object("invoice", invoice_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `payment_reconciled` / `payment_refunded` / `payment_disputed`.
    pub async fn payment_event(
        &self,
        event_type: EventType,
        psp_event_key: &str,
        invoice_id: &str,
        currency: &str,
        amount_micros: i64,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.invoice_id = Some(invoice_id.to_string());
        payload.currency = Some(currency.to_string());
        payload.amount_micros = Some(amount_micros);
        payload.psp_event_key = Some(psp_event_key.to_string());
        let builder = self
            .base(
                event_type,
                format!("{}:{psp_event_key}", event_type.as_str()),
                occurred_ms,
            )
            .with_object("invoice", invoice_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// `proposal_created` (page 75): proposal identity, snapshot and change
    /// hash.
    pub async fn proposal_created(
        &self,
        proposal_id: &str,
        snapshot: &str,
        change_hash: &str,
        policy_version: &str,
        occurred_ms: i64,
    ) {
        let mut payload = EventPayload::new();
        payload.proposal_id = Some(proposal_id.to_string());
        payload.snapshot = Some(snapshot.to_string());
        payload.change_hash = Some(change_hash.to_string());
        payload.policy_version = Some(policy_version.to_string());
        let builder = self
            .base(
                EventType::ProposalCreated,
                format!("proposal_created:{proposal_id}"),
                occurred_ms,
            )
            .with_object("proposal", proposal_id)
            .with_payload(payload);
        self.emit(builder).await;
    }

    /// The current occurrence time for events emitted in the same pass.
    pub fn at_millis(occurred_ms: i64) -> i64 {
        occurred_ms
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_analytics::store::ProjectionStore;
    use apex_analytics::{EventStore, MemoryStore};

    use super::*;

    async fn runner_events() -> (RunnerEvents, Arc<MemoryStore>) {
        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = AnalyticsClient::new(store.clone(), "dev", "apex-runner-test");
        (
            RunnerEvents::new(Arc::new(client), "northstar").with_environment("dev"),
            store,
        )
    }

    async fn emitted(store: &MemoryStore) -> Vec<DomainEvent> {
        store.load_events().await.expect("events")
    }

    #[tokio::test]
    async fn every_runner_family_builds_and_is_stored() {
        let (events, store) = runner_events().await;
        let now = 1_700_000_000_000;
        events
            .job_run("daily", "job_20261015_0810", now, "completed", 4200)
            .await;
        events
            .report_pull_started(
                "job_20261015_0810",
                "experiment_days",
                "exp_0242",
                "2026-10-01",
                "2026-10-14",
                1,
                now,
            )
            .await;
        events
            .report_pull_completed(
                "job_20261015_0810",
                "experiment_days",
                "exp_0242",
                "2026-10-01",
                "2026-10-14",
                14,
                "final",
                1200,
                now,
            )
            .await;
        events
            .report_pull_failed(
                "job_20261015_0810",
                "experiment_days",
                "exp_0243",
                "2026-10-01",
                "2026-10-14",
                1,
                "auth_refusal",
                now,
            )
            .await;
        events
            .experiment_registered("exp_0242", &"a".repeat(64), "revenue_per_user", 14, now)
            .await;
        events.experiment_started("exp_0242", "running", now).await;
        events
            .experiment_pull("exp_0242", &"b".repeat(64), "2026-10-01", "2026-10-14", now)
            .await;
        events
            .experiment_decided(
                "exp_0242",
                "keep",
                Some(540),
                Some(300),
                "bound_positive",
                now,
            )
            .await;
        events
            .guardrail_fired("fast_harm", "-8.0%", "-5.0%", now)
            .await;
        events
            .restore_requested("fast_harm", "op_31", "floor_restored", now)
            .await;
        events
            .restore_confirmed(
                "fast_harm",
                "op_31",
                "floor_restored",
                "platform read at 09:05",
                now,
            )
            .await;
        events
            .restore_failed(
                "fill_banner",
                "op_32",
                "floor_restored",
                "platform_refused",
                now + 1,
            )
            .await;
        events.ledger_verified(24, "v0.1.0", 150, now).await;
        events
            .ledger_verification_failed(24, "v0.1.0", Some(18), 150, now)
            .await;
        events
            .fee_computed(
                "ch_0246",
                "2026-10",
                "USD",
                51_500_000_000,
                300,
                10_300,
                2_000,
                31,
                vec![format!("{:064x}", 246)],
                now,
            )
            .await;
        events
            .fee_held("ch_0246", "2026-10", "USD", "proof_hold", now)
            .await;
        events
            .fee_invoice_issued(
                "ch_0246",
                "2026-10",
                "USD",
                "INV-2026-104",
                300_000_000,
                now,
            )
            .await;
        events
            .audit_completed(&"c".repeat(64), "v0.1.0", 5, 5, 1, 3, 1, vec![], 800, now)
            .await;
        events
            .audit_failed("v0.1.0", "snapshot_unreadable", now)
            .await;
        events
            .payment_received("psp_evt_88", "INV-2026-104", "USD", 300_000_000, now)
            .await;
        events
            .payment_event(
                EventType::PaymentReconciled,
                "psp_evt_88",
                "INV-2026-104",
                "USD",
                300_000_000,
                now,
            )
            .await;
        events
            .payment_event(
                EventType::PaymentRefunded,
                "psp_evt_89",
                "INV-2026-104",
                "USD",
                50_000_000,
                now,
            )
            .await;
        events
            .payment_event(
                EventType::PaymentDisputed,
                "psp_evt_90",
                "INV-2026-104",
                "USD",
                10_000_000,
                now,
            )
            .await;
        events
            .proposal_created(
                "prop_00000001",
                "snap_09",
                &"d".repeat(64),
                "apex-lift-0.1.0",
                now,
            )
            .await;
        let stored = emitted(&store).await;
        assert_eq!(stored.len(), 24, "every family landed");
        let types: std::collections::BTreeSet<&str> = stored
            .iter()
            .map(|event| event.event_type.as_str())
            .collect();
        for expected in [
            "job_run",
            "report_pull_started",
            "report_pull_completed",
            "report_pull_failed",
            "experiment_registered",
            "experiment_started",
            "experiment_pull",
            "experiment_decided",
            "guardrail_fired",
            "guardrail_restore_requested",
            "guardrail_restore_confirmed",
            "guardrail_restore_failed",
            "ledger_verified",
            "ledger_verification_failed",
            "fee_computed",
            "fee_held",
            "fee_invoice_issued",
            "audit_completed",
            "audit_failed",
            "payment_received",
            "payment_reconciled",
            "payment_refunded",
            "payment_disputed",
            "proposal_created",
        ] {
            assert!(types.contains(expected), "{expected} was not emitted");
        }
        assert!(stored
            .iter()
            .all(|event| event.publisher_id.as_deref() == Some("northstar")));
    }

    #[tokio::test]
    async fn event_ids_are_stable_across_reruns() {
        let (events, store) = runner_events().await;
        let now = 1_700_000_000_000;
        for _ in 0..2 {
            events
                .fee_computed(
                    "ch_0246",
                    "2026-10",
                    "USD",
                    1_000_000,
                    300,
                    10_300,
                    2_000,
                    31,
                    vec![format!("{:064x}", 1)],
                    now,
                )
                .await;
            events.job_run("daily", "job_x", now, "completed", 1).await;
        }
        // The store's (source, event_id) dedup absorbs the replay.
        assert_eq!(emitted(&store).await.len(), 2);
    }

    #[tokio::test]
    async fn a_daily_and_a_monthly_run_in_the_same_minute_keep_distinct_ids() {
        let (events, store) = runner_events().await;
        let now = 1_700_000_000_000;
        // The run reference is minute-granular; without the job type a
        // failed daily and a same-minute monthly would collapse into one
        // merge key and hide the failure.
        let same_minute = "job_20261015_0810";
        events.job_run("daily", same_minute, now, "failed", 1).await;
        events
            .job_run("monthly", same_minute, now, "completed", 1)
            .await;
        let stored = emitted(&store).await;
        assert_eq!(
            stored.len(),
            2,
            "both runs are visible after the store's exact dedup (FINAL)"
        );
        let ids: std::collections::BTreeSet<&str> =
            stored.iter().map(|event| event.event_id.as_str()).collect();
        assert_eq!(
            ids,
            std::collections::BTreeSet::from([
                "job_run:daily:job_20261015_0810",
                "job_run:monthly:job_20261015_0810",
            ])
        );
        // A replay of the identical type+minute still deduplicates.
        events.job_run("daily", same_minute, now, "failed", 1).await;
        assert_eq!(emitted(&store).await.len(), 2);
    }

    #[tokio::test]
    async fn no_secrets_or_query_strings_reach_the_payload() {
        let (events, store) = runner_events().await;
        // A token-looking error class is refused by the envelope builder and
        // reported on stderr, never stored.
        events
            .audit_failed(
                "v0.1.0",
                "token=abcdefghijklmnopqrstuvwxyz0123456789",
                1_700_000_000_000,
            )
            .await;
        assert!(emitted(&store).await.is_empty());
    }

    #[test]
    fn from_lookup_is_disabled_without_clickhouse() {
        assert!(RunnerEvents::from_lookup(|_| None)
            .expect("unset")
            .is_none());
        assert!(RunnerEvents::from_lookup(|_| Some("  ".into()))
            .expect("blank")
            .is_none());
        // A configured endpoint builds a spooled client without performing I/O.
        let configured = RunnerEvents::from_lookup(|name| match name {
            CLICKHOUSE_ENV => Some("http://127.0.0.1:8123".into()),
            _ => None,
        })
        .expect("configured");
        assert!(configured.is_some());
        assert_eq!(RunnerEvents::at_millis(7), 7);
    }

    #[test]
    fn the_environment_reads_the_apex_api_name_first() {
        // The apex-api name is primary.
        let configured = RunnerEvents::from_lookup(|name| match name {
            CLICKHOUSE_ENV => Some("http://127.0.0.1:8123".into()),
            ENVIRONMENT_ENV => Some("staging".into()),
            _ => None,
        })
        .expect("configured")
        .expect("some");
        assert_eq!(configured.environment, "staging");

        // The historical name is the deprecated fallback.
        let configured = RunnerEvents::from_lookup(|name| match name {
            CLICKHOUSE_ENV => Some("http://127.0.0.1:8123".into()),
            DEPRECATED_ENVIRONMENT_ENV => Some("dev".into()),
            _ => None,
        })
        .expect("configured")
        .expect("some");
        assert_eq!(configured.environment, "dev");

        // The primary name wins when both are set; neither means
        // production.
        let configured = RunnerEvents::from_lookup(|name| match name {
            CLICKHOUSE_ENV => Some("http://127.0.0.1:8123".into()),
            ENVIRONMENT_ENV => Some("staging".into()),
            DEPRECATED_ENVIRONMENT_ENV => Some("dev".into()),
            _ => None,
        })
        .expect("configured")
        .expect("some");
        assert_eq!(configured.environment, "staging");
        let configured = RunnerEvents::from_lookup(|name| match name {
            CLICKHOUSE_ENV => Some("http://127.0.0.1:8123".into()),
            _ => None,
        })
        .expect("configured")
        .expect("some");
        assert_eq!(configured.environment, "production");
    }

    #[test]
    fn the_spool_path_defaults_under_the_data_dir() {
        let configured = RunnerEvents::from_lookup(|name| match name {
            CLICKHOUSE_ENV => Some("http://127.0.0.1:8123".into()),
            _ => None,
        })
        .expect("configured")
        .expect("some");
        assert_eq!(
            configured.spool_path(),
            Some(std::path::Path::new(".apex/analytics/spool.jsonl"))
        );

        let configured = RunnerEvents::from_lookup(|name| match name {
            CLICKHOUSE_ENV => Some("http://127.0.0.1:8123".into()),
            DATA_DIR_ENV => Some("/var/lib/apex/".into()),
            _ => None,
        })
        .expect("configured")
        .expect("some");
        assert_eq!(
            configured.spool_path(),
            Some(std::path::Path::new("/var/lib/apex/analytics/spool.jsonl"))
        );

        // The explicit spool path always wins.
        let configured = RunnerEvents::from_lookup(|name| match name {
            CLICKHOUSE_ENV => Some("http://127.0.0.1:8123".into()),
            DATA_DIR_ENV => Some("/var/lib/apex".into()),
            SPOOL_ENV => Some("/tmp/explicit-spool.jsonl".into()),
            _ => None,
        })
        .expect("configured")
        .expect("some");
        assert_eq!(
            configured.spool_path(),
            Some(std::path::Path::new("/tmp/explicit-spool.jsonl"))
        );
    }

    #[tokio::test]
    async fn failed_batches_spool_and_replay_through_a_swapped_client() {
        /// A primary sink that refuses every batch deterministically.
        #[derive(Debug)]
        struct RefusingSink;

        #[async_trait::async_trait]
        impl EventSink for RefusingSink {
            async fn emit(
                &self,
                _events: &[DomainEvent],
            ) -> Result<apex_analytics::EmitOutcome, SinkError> {
                Err(SinkError::Store("refusing".into()))
            }
        }

        let directory = std::env::temp_dir().join(format!(
            "apex-runner-spool-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let spool = directory.join("spool.jsonl");
        std::fs::create_dir_all(&directory).expect("spool directory");

        // The primary sink refuses: every batch lands in the spool instead
        // of being lost.
        let events = RunnerEvents::with_spool(Arc::new(RefusingSink), spool.clone())
            .with_environment("dev")
            .with_publisher("northstar");
        assert_eq!(events.spool_path(), Some(spool.as_path()));
        events
            .job_run("daily", "job_1", 1_700_000_000_000, "completed", 5)
            .await;
        let spooled = tokio::fs::read_to_string(&spool).await.expect("spool");
        assert_eq!(spooled.lines().count(), 1, "the failed batch spooled");
        assert!(spooled.contains("\"event_type\":\"job_run\""));

        // Replaying through the same failing primary keeps the spool intact.
        assert_eq!(events.replay().await, 0);
        let kept = tokio::fs::read_to_string(&spool).await.expect("spool kept");
        assert_eq!(kept.lines().count(), 1);

        // Swap in a working MemoryStore-backed client over the same spool:
        // the replay delivers the batch and truncates the spool.
        let warm = Arc::new(MemoryStore::new());
        warm.ensure_schema().await.expect("schema");
        let working = RunnerEvents::with_spool(
            Arc::new(AnalyticsClient::new(
                warm.clone(),
                "dev",
                "apex-runner-test",
            )),
            spool.clone(),
        )
        .with_environment("dev")
        .with_publisher("northstar");
        assert_eq!(working.replay().await, 1);
        let delivered = warm.load_events().await.expect("events");
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].event_type.as_str(), "job_run");
        assert!(tokio::fs::read_to_string(&spool)
            .await
            .expect("spool")
            .is_empty());
        // The second replay delivers nothing.
        assert_eq!(working.replay().await, 0);

        // A direct sink has no spool and replays nothing.
        let direct = RunnerEvents::new(
            Arc::new(AnalyticsClient::new(warm, "dev", "apex-runner-test")),
            "northstar",
        );
        assert_eq!(direct.replay().await, 0);
        assert!(direct.spool_path().is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
