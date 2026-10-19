//! The event envelope: the required fields, the full event catalog and the
//! builder that enforces the spec's per-family minimum fields.
//!
//! Section 05 states the envelope as: `event_id`, `event_type`,
//! `schema_version`, `occurred_at_utc`, `received_at_utc`, `environment`,
//! `service_version`, `source`, `publisher_id` where applicable, `platform`,
//! `object_type`, `object_id`, `correlation_id`, `causation_id`, `outcome`
//! and `reason_code`; effective reporting dates and currency ride on the
//! relevant events. The builder below refuses token-looking values and
//! complete query strings in every string field, because the spec forbids
//! platform secrets, raw authorization headers and complete query strings in
//! telemetry (page 86, Data minimization).

use crate::timestamp::{TimeError, UtcMillis};

#[cfg(test)]
thread_local! {
    /// Canonical payload serializations. Building an event used to serialize
    /// the payload once for the derived id and again for the stored row; the
    /// counter pins that the canonical bytes are produced exactly once.
    static PAYLOAD_SERIALIZATIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_payload_serializations() {
    PAYLOAD_SERIALIZATIONS.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn payload_serializations() -> u64 {
    PAYLOAD_SERIALIZATIONS.with(std::cell::Cell::get)
}

/// One event family from the catalog in section 05 (pages 74 and 75).
///
/// The catalog names families with a shared prefix (for example
/// `connection_validated / expired / revoked`); [`EventType`] spells every
/// variant with the full family prefix so `as_str` matches the spec spelling
/// and stays globally unique in one column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventType {
    /// Edge/server request at the delivery/access-log boundary.
    PublicPageRequested,
    /// Axum validation accepted and durably persisted a lead.
    AuditRequestAccepted,
    /// Axum validation rejected a submission; the reason is recorded.
    AuditRequestRejected,
    /// Optional webhook delivery attempt to the follow-up system.
    LeadForwardAttempted,
    /// The follow-up system acknowledged the lead.
    LeadForwardAcknowledged,
    /// The delivery attempt failed; response class recorded.
    LeadForwardFailed,
    /// Durable lifecycle transition between account stages.
    AccountStageChanged,
    /// Credential scope and connection state validated.
    ConnectionValidated,
    /// A previously valid connection expired.
    ConnectionExpired,
    /// A connection was revoked.
    ConnectionRevoked,
    /// Worker report pull started.
    ReportPullStarted,
    /// Worker report pull completed; rows, bounds and finality recorded.
    ReportPullCompleted,
    /// Worker report pull failed; error class recorded.
    ReportPullFailed,
    /// Audit execution completed on a snapshot.
    AuditCompleted,
    /// Audit execution failed.
    AuditFailed,
    /// A change proposal was created.
    ProposalCreated,
    /// A change proposal was approved by an actor.
    ProposalApproved,
    /// A change proposal was declined.
    ProposalDeclined,
    /// A change proposal expired without a decision.
    ProposalExpired,
    /// An experiment was registered with metric, window and source digests.
    ExperimentRegistered,
    /// The platform experiment started.
    ExperimentStarted,
    /// A finalized experiment pull arrived.
    ExperimentPull,
    /// The experiment received its decision.
    ExperimentDecided,
    /// A guardrail fired on observed inputs beyond its threshold.
    GuardrailFired,
    /// A restore or stop was requested after a guardrail.
    GuardrailRestoreRequested,
    /// The restore was confirmed by a platform read.
    GuardrailRestoreConfirmed,
    /// The restore failed; the known state stays unresolved.
    GuardrailRestoreFailed,
    /// Ledger history verified through a sequence.
    LedgerVerified,
    /// Ledger verification failed at a line.
    LedgerVerificationFailed,
    /// A fee was computed from a qualifying decision.
    FeeComputed,
    /// A fee was held (proof, recheck or dispute).
    FeeHeld,
    /// A fee line was issued on an invoice.
    FeeInvoiceIssued,
    /// A fee line was credited.
    FeeCredited,
    /// A verified payment event arrived.
    PaymentReceived,
    /// A payment was reconciled against an invoice.
    PaymentReconciled,
    /// A payment was refunded.
    PaymentRefunded,
    /// A payment was disputed.
    PaymentDisputed,
    /// A scheduled job ran (schedule and outcome recorded).
    JobRun,
    /// An incident changed state (owner, impact).
    IncidentChanged,
    /// An operator action was recorded (actor, reason, target, result).
    OperatorAction,
}

/// The event families of the catalog, used by validation and projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventFamily {
    /// Public page requests from the delivery/access-log boundary.
    PageRequest,
    /// Audit request validation outcomes.
    AuditRequest,
    /// Lead forwarding to the follow-up system.
    LeadForward,
    /// Account lifecycle transitions.
    AccountLifecycle,
    /// Platform connection state.
    Connection,
    /// Report pulls.
    ReportPull,
    /// Audit executions.
    AuditRun,
    /// Change proposals.
    Proposal,
    /// Experiments.
    Experiment,
    /// Guardrails and restores.
    Guardrail,
    /// Ledger verification.
    Ledger,
    /// Fee computation and invoicing.
    Fee,
    /// Payments and disputes.
    Payment,
    /// Jobs, incidents and operator actions.
    Operations,
}

impl EventFamily {
    /// Stable family name for grouping and metrics.
    pub fn as_str(self) -> &'static str {
        match self {
            EventFamily::PageRequest => "page_request",
            EventFamily::AuditRequest => "audit_request",
            EventFamily::LeadForward => "lead_forward",
            EventFamily::AccountLifecycle => "account_lifecycle",
            EventFamily::Connection => "connection",
            EventFamily::ReportPull => "report_pull",
            EventFamily::AuditRun => "audit_run",
            EventFamily::Proposal => "proposal",
            EventFamily::Experiment => "experiment",
            EventFamily::Guardrail => "guardrail",
            EventFamily::Ledger => "ledger",
            EventFamily::Fee => "fee",
            EventFamily::Payment => "payment",
            EventFamily::Operations => "operations",
        }
    }
}

impl EventType {
    /// Every catalog entry, in spec order.
    pub const ALL: [EventType; 40] = [
        EventType::PublicPageRequested,
        EventType::AuditRequestAccepted,
        EventType::AuditRequestRejected,
        EventType::LeadForwardAttempted,
        EventType::LeadForwardAcknowledged,
        EventType::LeadForwardFailed,
        EventType::AccountStageChanged,
        EventType::ConnectionValidated,
        EventType::ConnectionExpired,
        EventType::ConnectionRevoked,
        EventType::ReportPullStarted,
        EventType::ReportPullCompleted,
        EventType::ReportPullFailed,
        EventType::AuditCompleted,
        EventType::AuditFailed,
        EventType::ProposalCreated,
        EventType::ProposalApproved,
        EventType::ProposalDeclined,
        EventType::ProposalExpired,
        EventType::ExperimentRegistered,
        EventType::ExperimentStarted,
        EventType::ExperimentPull,
        EventType::ExperimentDecided,
        EventType::GuardrailFired,
        EventType::GuardrailRestoreRequested,
        EventType::GuardrailRestoreConfirmed,
        EventType::GuardrailRestoreFailed,
        EventType::LedgerVerified,
        EventType::LedgerVerificationFailed,
        EventType::FeeComputed,
        EventType::FeeHeld,
        EventType::FeeInvoiceIssued,
        EventType::FeeCredited,
        EventType::PaymentReceived,
        EventType::PaymentReconciled,
        EventType::PaymentRefunded,
        EventType::PaymentDisputed,
        EventType::JobRun,
        EventType::IncidentChanged,
        EventType::OperatorAction,
    ];

    /// The spec spelling of this event type.
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::PublicPageRequested => "public_page_requested",
            EventType::AuditRequestAccepted => "audit_request_accepted",
            EventType::AuditRequestRejected => "audit_request_rejected",
            EventType::LeadForwardAttempted => "lead_forward_attempted",
            EventType::LeadForwardAcknowledged => "lead_forward_acknowledged",
            EventType::LeadForwardFailed => "lead_forward_failed",
            EventType::AccountStageChanged => "account_stage_changed",
            EventType::ConnectionValidated => "connection_validated",
            EventType::ConnectionExpired => "connection_expired",
            EventType::ConnectionRevoked => "connection_revoked",
            EventType::ReportPullStarted => "report_pull_started",
            EventType::ReportPullCompleted => "report_pull_completed",
            EventType::ReportPullFailed => "report_pull_failed",
            EventType::AuditCompleted => "audit_completed",
            EventType::AuditFailed => "audit_failed",
            EventType::ProposalCreated => "proposal_created",
            EventType::ProposalApproved => "proposal_approved",
            EventType::ProposalDeclined => "proposal_declined",
            EventType::ProposalExpired => "proposal_expired",
            EventType::ExperimentRegistered => "experiment_registered",
            EventType::ExperimentStarted => "experiment_started",
            EventType::ExperimentPull => "experiment_pull",
            EventType::ExperimentDecided => "experiment_decided",
            EventType::GuardrailFired => "guardrail_fired",
            EventType::GuardrailRestoreRequested => "guardrail_restore_requested",
            EventType::GuardrailRestoreConfirmed => "guardrail_restore_confirmed",
            EventType::GuardrailRestoreFailed => "guardrail_restore_failed",
            EventType::LedgerVerified => "ledger_verified",
            EventType::LedgerVerificationFailed => "ledger_verification_failed",
            EventType::FeeComputed => "fee_computed",
            EventType::FeeHeld => "fee_held",
            EventType::FeeInvoiceIssued => "fee_invoice_issued",
            EventType::FeeCredited => "fee_credited",
            EventType::PaymentReceived => "payment_received",
            EventType::PaymentReconciled => "payment_reconciled",
            EventType::PaymentRefunded => "payment_refunded",
            EventType::PaymentDisputed => "payment_disputed",
            EventType::JobRun => "job_run",
            EventType::IncidentChanged => "incident_changed",
            EventType::OperatorAction => "operator_action",
        }
    }

    /// Parse the spec spelling back into the enum.
    pub fn parse(text: &str) -> Option<Self> {
        EventType::ALL
            .iter()
            .copied()
            .find(|event_type| event_type.as_str() == text)
    }

    /// The catalog family this event type belongs to.
    pub fn family(self) -> EventFamily {
        match self {
            EventType::PublicPageRequested => EventFamily::PageRequest,
            EventType::AuditRequestAccepted | EventType::AuditRequestRejected => {
                EventFamily::AuditRequest
            }
            EventType::LeadForwardAttempted
            | EventType::LeadForwardAcknowledged
            | EventType::LeadForwardFailed => EventFamily::LeadForward,
            EventType::AccountStageChanged => EventFamily::AccountLifecycle,
            EventType::ConnectionValidated
            | EventType::ConnectionExpired
            | EventType::ConnectionRevoked => EventFamily::Connection,
            EventType::ReportPullStarted
            | EventType::ReportPullCompleted
            | EventType::ReportPullFailed => EventFamily::ReportPull,
            EventType::AuditCompleted | EventType::AuditFailed => EventFamily::AuditRun,
            EventType::ProposalCreated
            | EventType::ProposalApproved
            | EventType::ProposalDeclined
            | EventType::ProposalExpired => EventFamily::Proposal,
            EventType::ExperimentRegistered
            | EventType::ExperimentStarted
            | EventType::ExperimentPull
            | EventType::ExperimentDecided => EventFamily::Experiment,
            EventType::GuardrailFired
            | EventType::GuardrailRestoreRequested
            | EventType::GuardrailRestoreConfirmed
            | EventType::GuardrailRestoreFailed => EventFamily::Guardrail,
            EventType::LedgerVerified | EventType::LedgerVerificationFailed => EventFamily::Ledger,
            EventType::FeeComputed
            | EventType::FeeHeld
            | EventType::FeeInvoiceIssued
            | EventType::FeeCredited => EventFamily::Fee,
            EventType::PaymentReceived
            | EventType::PaymentReconciled
            | EventType::PaymentRefunded
            | EventType::PaymentDisputed => EventFamily::Payment,
            EventType::JobRun | EventType::IncidentChanged | EventType::OperatorAction => {
                EventFamily::Operations
            }
        }
    }

    /// Does this event require the envelope `outcome` field?
    fn requires_outcome(self) -> bool {
        matches!(
            self,
            EventType::JobRun
                | EventType::OperatorAction
                | EventType::LeadForwardAcknowledged
                | EventType::LeadForwardFailed
                | EventType::ReportPullFailed
                | EventType::AuditFailed
        )
    }

    /// Does this event require the envelope `reason_code` field?
    fn requires_reason_code(self) -> bool {
        matches!(
            self,
            EventType::AccountStageChanged
                | EventType::FeeHeld
                | EventType::AuditRequestRejected
                | EventType::GuardrailRestoreFailed
        )
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl serde::Serialize for EventType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for EventType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = EventType;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a catalog event type")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<EventType, E> {
                EventType::parse(value)
                    .ok_or_else(|| E::custom(format!("unknown event type '{value}'")))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

/// The producer boundary an event originates from (spec page 73).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventSource {
    /// The delivery/access-log boundary (Caddy JSON access logs).
    AccessLog,
    /// The Axum application (validated submissions and response outcomes).
    Axum,
    /// Background workers (ingestion, lifecycle, finance).
    Worker,
    /// Platform connectors and the verified runner.
    Connector,
    /// The operator control plane.
    Operator,
    /// The finance boundary (payments, invoices).
    Finance,
}

impl EventSource {
    /// Stable source name; it is half of the dedup key `(source, event_id)`.
    pub fn as_str(self) -> &'static str {
        match self {
            EventSource::AccessLog => "access_log",
            EventSource::Axum => "axum",
            EventSource::Worker => "worker",
            EventSource::Connector => "connector",
            EventSource::Operator => "operator",
            EventSource::Finance => "finance",
        }
    }

    /// Parse a stored source name.
    pub fn parse(text: &str) -> Option<Self> {
        [
            EventSource::AccessLog,
            EventSource::Axum,
            EventSource::Worker,
            EventSource::Connector,
            EventSource::Operator,
            EventSource::Finance,
        ]
        .into_iter()
        .find(|source| source.as_str() == text)
    }
}

impl std::fmt::Display for EventSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl serde::Serialize for EventSource {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for EventSource {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = EventSource;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a producing boundary source")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<EventSource, E> {
                EventSource::parse(value)
                    .ok_or_else(|| E::custom(format!("unknown source '{value}'")))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

/// Builder and validation errors. The spec requires refusing secrets and
/// query strings instead of storing them, so those are distinct variants.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    /// A field the catalog lists as a per-family minimum is absent.
    #[error("{event_type} requires field '{field}'")]
    MissingField {
        /// The event type being built.
        event_type: EventType,
        /// The missing field name.
        field: &'static str,
    },
    /// A present field carries an unusable value.
    #[error("{event_type}.{field} is not valid: {reason}")]
    InvalidValue {
        /// The event type being built.
        event_type: EventType,
        /// The offending field name.
        field: String,
        /// Why the value was refused.
        reason: String,
    },
    /// A string field looks like a secret, credential or raw query string.
    #[error("{event_type}.{field} is refused: {reason}")]
    ForbiddenValue {
        /// The event type being built.
        event_type: EventType,
        /// The offending field name.
        field: String,
        /// The class of forbidden content.
        reason: &'static str,
    },
    /// The system clock is unusable; pass explicit timestamps.
    #[error("system clock unavailable before the Unix epoch")]
    Clock,
}

impl From<TimeError> for ValidationError {
    fn from(_: TimeError) -> Self {
        ValidationError::Clock
    }
}

/// The typed payload carried beside the envelope.
///
/// Every field is optional at the type level; the builder enforces the
/// spec's per-family minimums at `build()` time. All money amounts are
/// integer micros and always accompany an explicit ISO-4217 currency.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventPayload {
    /// Normalized route template (never the raw URL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_template: Option<String>,
    /// HTTP status code of the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Referrer host only (never the full referrer URL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer_host: Option<String>,
    /// Allowed campaign code from designated campaign parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_code: Option<String>,
    /// Documented bot classification, never a claim about humans.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_class: Option<String>,
    /// Allow-listed query parameter keys only; values are never stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_keys: Option<Vec<String>>,
    /// Durable lead identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_id: Option<String>,
    /// Account identity, when the request resolved to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Platforms a lead or check covers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    /// First reporting day of a range (`YYYY-MM-DD`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_start: Option<String>,
    /// Last reporting day of a range (`YYYY-MM-DD`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_end: Option<String>,
    /// Acquisition source code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_code: Option<String>,
    /// Why a submission or operation was rejected or held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_reason: Option<String>,
    /// Follow-up destination type (`webhook`, `email` relay, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_type: Option<String>,
    /// Attempt number, starting at 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// Bounded response class of a delivery attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_class: Option<String>,
    /// Account stage before a lifecycle transition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_stage: Option<String>,
    /// Account stage after a lifecycle transition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_stage: Option<String>,
    /// Account owner at transition time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Effective time of a transition (UTC timestamp or day).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_at: Option<String>,
    /// Platform account identity for a connection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_account_id: Option<String>,
    /// Whether the credential can read configuration and reports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_capability: Option<bool>,
    /// Whether a verified write operation is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_capability: Option<bool>,
    /// Validator version used for the connection check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validator_version: Option<String>,
    /// Bounded failure class; never the secret itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
    /// Durable job identity of a worker run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Platform operation the worker performed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_operation: Option<String>,
    /// Duration of the operation in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Rows read or written by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    /// Finality of the pulled range (`final`, `partial`, `provisional`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finality: Option<String>,
    /// Bounded error class of a failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    /// Digest of the source snapshot the audit ran on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    /// Rule version of the audit or decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_version: Option<String>,
    /// Rule groups that were eligible and evaluated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eligible_rule_groups: Option<u32>,
    /// Rule groups the snapshot could evaluate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_rule_groups: Option<u32>,
    /// Count of critical findings (a real zero is a fact).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings_critical: Option<u32>,
    /// Count of warning findings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings_warning: Option<u32>,
    /// Count of info findings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings_info: Option<u32>,
    /// Rule groups that could not run and their missing inputs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_inputs: Option<Vec<String>>,
    /// Change proposal identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    /// Snapshot the proposal is bound to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    /// Hash of the exact proposed change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_hash: Option<String>,
    /// Policy version governing the proposal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
    /// Actor who approved (or is named in) a proposal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_actor: Option<String>,
    /// Expiry of a proposal or approval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<String>,
    /// Platform or internal experiment identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    /// Digest of the source rows an experiment result was computed from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<String>,
    /// Registered metric of an experiment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_metric: Option<String>,
    /// Registered analysis window in days.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    /// Experiment or lifecycle state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Decision outcome; the closed vocabulary is
    /// [`DECISION_VOCABULARY`] (`keep`, `rollback`, `void`, `extend`,
    /// `pending`), case-normalized to lower case and stored that way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// Point estimate in basis points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_estimate_bp: Option<i64>,
    /// Billable decision bound in basis points; a bound of -100 percent or
    /// lower has no reproducible basis and is refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_bound_bp: Option<i64>,
    /// Guardrail or guard type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard_type: Option<String>,
    /// Observed input that triggered or was evaluated by a guard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_input: Option<String>,
    /// Threshold the guard compares against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<String>,
    /// Target state a restore should reach.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_state: Option<String>,
    /// Platform operation identity of a restore.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    /// Confirming platform read after a restore.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirming_read: Option<String>,
    /// Highest ledger sequence verified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highest_sequence: Option<u64>,
    /// Verifier version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_version: Option<String>,
    /// Ledger line at which verification failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_line: Option<u64>,
    /// Change identity a fee line is based on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
    /// Billing period (`YYYY-MM`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
    /// ISO-4217 currency of the amounts on this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Eligible scope revenue the fee is computed from, integer micros.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_amount_micros: Option<i64>,
    /// Billable lower bound in basis points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_bp: Option<i64>,
    /// Combined factor in basis points of one (10300 = 1.03) when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor_bp: Option<i64>,
    /// Agreed fee rate in basis points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_bp: Option<u16>,
    /// Active billed days in the period.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_days: Option<u32>,
    /// Proof hashes linking the fee to its signed decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_hashes: Option<Vec<String>>,
    /// Invoice identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice_id: Option<String>,
    /// PSP event key used for deduplication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub psp_event_key: Option<String>,
    /// Payment amount in integer micros.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_micros: Option<i64>,
    /// Incident identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incident_id: Option<String>,
    /// Customer impact summary of an incident (no personal data).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<String>,
    /// Operator actor identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_actor: Option<String>,
    /// Action target (object class and identity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Authorization basis for an operator action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization: Option<String>,
    /// Result of an operator action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

impl EventPayload {
    /// Empty payload; start here and set the fields the family requires.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder shortcut for the common one-field payloads.
    pub fn with_route_template(mut self, template: impl Into<String>) -> Self {
        self.route_template = Some(template.into());
        self
    }

    /// Every named string-bearing field, for the secret and query-string
    /// checks. List fields yield one entry per element.
    fn string_fields(&self) -> Vec<(&'static str, &str)> {
        let mut out: Vec<(&'static str, &str)> = Vec::new();
        macro_rules! one {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(value) = &self.$field {
                    out.push((stringify!($field), value.as_str()));
                })+
            };
        }
        macro_rules! many {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(values) = &self.$field {
                    for value in values {
                        out.push((stringify!($field), value.as_str()));
                    }
                })+
            };
        }
        one!(
            route_template,
            referrer_host,
            campaign_code,
            bot_class,
            lead_id,
            account_id,
            range_start,
            range_end,
            source_code,
            validation_reason,
            destination_type,
            response_class,
            previous_stage,
            next_stage,
            owner,
            effective_at,
            platform_account_id,
            validator_version,
            failure_class,
            job_id,
            platform_operation,
            finality,
            error_class,
            snapshot_digest,
            rule_version,
            proposal_id,
            snapshot,
            change_hash,
            policy_version,
            approval_actor,
            expiry,
            experiment_id,
            source_digest,
            registered_metric,
            state,
            decision,
            guard_type,
            observed_input,
            threshold,
            target_state,
            operation_id,
            confirming_read,
            verification_version,
            change_id,
            period,
            currency,
            invoice_id,
            psp_event_key,
            incident_id,
            impact,
            action_actor,
            target,
            authorization,
            result,
        );
        many!(platforms, missing_inputs, proof_hashes, query_keys);
        out
    }

    /// Serialize the payload the way `payload_json` stores it: compact JSON
    /// with absent fields omitted (missing is not zero).
    ///
    /// Values are written straight into the output buffer. The previous
    /// implementation formatted each field into a temporary `String` first,
    /// so one payload cost one allocation per present field.
    pub fn to_json_string(&self) -> String {
        #[cfg(test)]
        PAYLOAD_SERIALIZATIONS.with(|count| count.set(count.get() + 1));
        let mut writer = JsonWriter::new();
        writer.begin_object();
        macro_rules! field {
            ($($name:literal => $value:expr),+ $(,)?) => {
                $(
                    if let Some(value) = &$value {
                        writer.field_json($name, value);
                    }
                )+
            };
        }
        field!(
            "route_template" => self.route_template,
            "status" => self.status,
            "referrer_host" => self.referrer_host,
            "campaign_code" => self.campaign_code,
            "bot_class" => self.bot_class,
            "query_keys" => self.query_keys,
            "lead_id" => self.lead_id,
            "account_id" => self.account_id,
            "platforms" => self.platforms,
            "range_start" => self.range_start,
            "range_end" => self.range_end,
            "source_code" => self.source_code,
            "validation_reason" => self.validation_reason,
            "destination_type" => self.destination_type,
            "attempt" => self.attempt,
            "response_class" => self.response_class,
            "previous_stage" => self.previous_stage,
            "next_stage" => self.next_stage,
            "owner" => self.owner,
            "effective_at" => self.effective_at,
            "platform_account_id" => self.platform_account_id,
            "read_capability" => self.read_capability,
            "write_capability" => self.write_capability,
            "validator_version" => self.validator_version,
            "failure_class" => self.failure_class,
            "job_id" => self.job_id,
            "platform_operation" => self.platform_operation,
            "duration_ms" => self.duration_ms,
            "rows" => self.rows,
            "finality" => self.finality,
            "error_class" => self.error_class,
            "snapshot_digest" => self.snapshot_digest,
            "rule_version" => self.rule_version,
            "eligible_rule_groups" => self.eligible_rule_groups,
            "expected_rule_groups" => self.expected_rule_groups,
            "findings_critical" => self.findings_critical,
            "findings_warning" => self.findings_warning,
            "findings_info" => self.findings_info,
            "missing_inputs" => self.missing_inputs,
            "proposal_id" => self.proposal_id,
            "snapshot" => self.snapshot,
            "change_hash" => self.change_hash,
            "policy_version" => self.policy_version,
            "approval_actor" => self.approval_actor,
            "expiry" => self.expiry,
            "experiment_id" => self.experiment_id,
            "source_digest" => self.source_digest,
            "registered_metric" => self.registered_metric,
            "window_days" => self.window_days,
            "state" => self.state,
            "decision" => self.decision,
            "point_estimate_bp" => self.point_estimate_bp,
            "decision_bound_bp" => self.decision_bound_bp,
            "guard_type" => self.guard_type,
            "observed_input" => self.observed_input,
            "threshold" => self.threshold,
            "target_state" => self.target_state,
            "operation_id" => self.operation_id,
            "confirming_read" => self.confirming_read,
            "highest_sequence" => self.highest_sequence,
            "verification_version" => self.verification_version,
            "failure_line" => self.failure_line,
            "change_id" => self.change_id,
            "period" => self.period,
            "currency" => self.currency,
            "source_amount_micros" => self.source_amount_micros,
            "bound_bp" => self.bound_bp,
            "factor_bp" => self.factor_bp,
            "rate_bp" => self.rate_bp,
            "active_days" => self.active_days,
            "proof_hashes" => self.proof_hashes,
            "invoice_id" => self.invoice_id,
            "psp_event_key" => self.psp_event_key,
            "amount_micros" => self.amount_micros,
            "incident_id" => self.incident_id,
            "impact" => self.impact,
            "action_actor" => self.action_actor,
            "target" => self.target,
            "authorization" => self.authorization,
            "result" => self.result,
        );
        writer.end_object();
        writer.finish()
    }
}

/// Minimal JSON object writer used for `payload_json`. All values are
/// pre-serialized JSON fragments produced by `serde_json::to_string` on
/// primitives and string lists, which cannot fail.
struct JsonWriter {
    out: String,
    any: bool,
}

/// `serde_json::to_writer` needs `io::Write`; this adapter appends to a
/// `String` with no intermediate buffer. serde_json only emits valid UTF-8
/// (escaping everything else), and the types written here are primitives and
/// string lists, so the UTF-8 check never fails in practice.
struct JsonSink<'a>(&'a mut String);

impl std::io::Write for JsonSink<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let text = std::str::from_utf8(buffer)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        self.0.push_str(text);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl JsonWriter {
    fn new() -> Self {
        Self {
            out: String::with_capacity(256),
            any: false,
        }
    }

    fn begin_object(&mut self) {
        self.out.push('{');
    }

    fn begin_field(&mut self, name: &str) {
        if self.any {
            self.out.push(',');
        }
        self.any = true;
        self.out.push('"');
        self.out.push_str(name);
        self.out.push_str("\":");
    }

    /// Serialize one value directly into the output buffer. The supported
    /// field types (primitives and string lists) cannot fail on a `String`
    /// writer, but `null` keeps the method total if that ever changes.
    fn field_json<T: serde::Serialize + ?Sized>(&mut self, name: &str, value: &T) {
        self.begin_field(name);
        if serde_json::to_writer(JsonSink(&mut self.out), value).is_err() {
            self.out.push_str("null");
        }
    }

    fn end_object(&mut self) {
        self.out.push('}');
    }

    fn finish(self) -> String {
        self.out
    }
}

/// The stored envelope plus its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainEvent {
    /// Stable identity, unique per `(source, event_id)`.
    pub event_id: String,
    /// Catalog event type.
    pub event_type: EventType,
    /// Envelope schema version; starts at 1.
    pub schema_version: u16,
    /// When the fact happened, UTC.
    pub occurred_at_utc: UtcMillis,
    /// When the store received it, UTC.
    pub received_at_utc: UtcMillis,
    /// Deployment environment (`production`, `staging`, `dev`).
    pub environment: String,
    /// Version of the service that emitted the event.
    pub service_version: String,
    /// Producing boundary.
    pub source: EventSource,
    /// Tenant scope, where applicable.
    pub publisher_id: Option<String>,
    /// Platform scope, where applicable.
    pub platform: Option<String>,
    /// Object class the event is about.
    pub object_type: Option<String>,
    /// Object identity the event is about.
    pub object_id: Option<String>,
    /// Correlation across a workflow.
    pub correlation_id: Option<String>,
    /// The direct cause of this event.
    pub causation_id: Option<String>,
    /// Bounded outcome label.
    pub outcome: Option<String>,
    /// Bounded reason code.
    pub reason_code: Option<String>,
    /// Typed payload.
    pub payload: EventPayload,
    /// Canonical compact JSON of `payload`, computed once when the event is
    /// built (or read back from the store) and reused for the derived event
    /// id and the stored row. The payload is immutable after `build()`; a
    /// caller that mutates the public `payload` field must rebuild the event.
    pub(crate) payload_json: String,
}

/// Builder for [`DomainEvent`], enforcing the catalog minimums at `build()`.
#[derive(Debug, Clone)]
pub struct DomainEventBuilder {
    event_type: EventType,
    source: EventSource,
    event_id: Option<String>,
    schema_version: u16,
    occurred_at_utc: Option<UtcMillis>,
    received_at_utc: Option<UtcMillis>,
    environment: Option<String>,
    service_version: Option<String>,
    publisher_id: Option<String>,
    platform: Option<String>,
    object_type: Option<String>,
    object_id: Option<String>,
    correlation_id: Option<String>,
    causation_id: Option<String>,
    outcome: Option<String>,
    reason_code: Option<String>,
    payload: EventPayload,
}

impl DomainEventBuilder {
    /// Set the dedup identity explicitly; when omitted, `build()` derives a
    /// deterministic id from the event content so at-least-once producers
    /// still deduplicate on replay.
    pub fn with_event_id(mut self, event_id: impl Into<String>) -> Self {
        self.event_id = Some(event_id.into());
        self
    }

    /// Override the envelope schema version (default 1).
    pub fn with_schema_version(mut self, version: u16) -> Self {
        self.schema_version = version;
        self
    }

    /// When the fact happened.
    pub fn with_occurred_at(mut self, occurred_at_utc: UtcMillis) -> Self {
        self.occurred_at_utc = Some(occurred_at_utc);
        self
    }

    /// When the store received the event.
    pub fn with_received_at(mut self, received_at_utc: UtcMillis) -> Self {
        self.received_at_utc = Some(received_at_utc);
        self
    }

    /// Deployment environment.
    pub fn with_environment(mut self, environment: impl Into<String>) -> Self {
        self.environment = Some(environment.into());
        self
    }

    /// Emitting service version.
    pub fn with_service_version(mut self, service_version: impl Into<String>) -> Self {
        self.service_version = Some(service_version.into());
        self
    }

    /// Tenant scope.
    pub fn with_publisher_id(mut self, publisher_id: impl Into<String>) -> Self {
        self.publisher_id = Some(publisher_id.into());
        self
    }

    /// Platform scope.
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }

    /// Object class and identity.
    pub fn with_object(
        mut self,
        object_type: impl Into<String>,
        object_id: impl Into<String>,
    ) -> Self {
        self.object_type = Some(object_type.into());
        self.object_id = Some(object_id.into());
        self
    }

    /// Workflow correlation identity.
    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }

    /// Direct cause identity.
    pub fn with_causation_id(mut self, causation_id: impl Into<String>) -> Self {
        self.causation_id = Some(causation_id.into());
        self
    }

    /// Bounded outcome label.
    pub fn with_outcome(mut self, outcome: impl Into<String>) -> Self {
        self.outcome = Some(outcome.into());
        self
    }

    /// Bounded reason code.
    pub fn with_reason_code(mut self, reason_code: impl Into<String>) -> Self {
        self.reason_code = Some(reason_code.into());
        self
    }

    /// Replace the payload.
    pub fn with_payload(mut self, payload: EventPayload) -> Self {
        self.payload = payload;
        self
    }

    /// Validate and freeze the event.
    pub fn build(self) -> Result<DomainEvent, ValidationError> {
        let event_type = self.event_type;
        if self.schema_version == 0 {
            return Err(ValidationError::InvalidValue {
                event_type,
                field: "schema_version".into(),
                reason: "must be at least 1".into(),
            });
        }
        let environment = required(self.environment, event_type, "environment")?;
        let service_version = required(self.service_version, event_type, "service_version")?;
        let now = if self.occurred_at_utc.is_none() || self.received_at_utc.is_none() {
            UtcMillis::system_now()?
        } else {
            UtcMillis::from_unix_millis(0)
        };
        let occurred_at_utc = self.occurred_at_utc.unwrap_or(now);
        let received_at_utc = self.received_at_utc.unwrap_or(now);
        if received_at_utc < occurred_at_utc {
            return Err(ValidationError::InvalidValue {
                event_type,
                field: "received_at_utc".into(),
                reason: "received time is before the occurred time".into(),
            });
        }

        let event = DomainEvent {
            event_id: self.event_id.unwrap_or_default(),
            event_type,
            schema_version: self.schema_version,
            occurred_at_utc,
            received_at_utc,
            environment,
            service_version,
            source: self.source,
            publisher_id: self.publisher_id,
            platform: self.platform,
            object_type: self.object_type,
            object_id: self.object_id,
            correlation_id: self.correlation_id,
            causation_id: self.causation_id,
            outcome: self.outcome,
            reason_code: self.reason_code,
            payload: self.payload,
            payload_json: String::new(),
        };
        let mut event = event;
        event.normalize_empty_strings();
        event.validate()?;
        // Serialize the canonical payload exactly once. The derived id and
        // every `to_row` use these bytes verbatim, so the two can never
        // disagree and repeated inserts do not reserialize the payload.
        event.payload_json = event.payload.to_json_string();
        Ok(if event.event_id.is_empty() {
            let event_id = event.derive_event_id();
            DomainEvent { event_id, ..event }
        } else {
            event
        })
    }
}

fn required<T>(
    value: Option<T>,
    event_type: EventType,
    field: &'static str,
) -> Result<T, ValidationError> {
    value.ok_or(ValidationError::MissingField { event_type, field })
}

/// Max length of any single string field. Long free text belongs in its own
/// restricted store, not in telemetry (spec page 86).
const MAX_STRING_BYTES: usize = 1024;

/// Why a string value was refused by [`check_string_value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForbiddenClass {
    /// The value is empty where a value is required.
    Empty,
    /// The value is longer than the envelope allows.
    TooLong,
    /// The value carries a control or zero-width format character.
    Control,
    /// The value looks like a credential (bearer token, key assignment, JWT).
    Credential,
    /// The value contains a complete query string.
    QueryString,
    /// The value looks like an opaque token or key.
    TokenLike,
}

impl ForbiddenClass {
    /// Stable label stored in the validation error.
    pub fn as_str(self) -> &'static str {
        match self {
            ForbiddenClass::Empty => "empty value",
            ForbiddenClass::TooLong => "longer than the envelope allows",
            ForbiddenClass::Control => "control character",
            ForbiddenClass::Credential => "credential-looking value",
            ForbiddenClass::QueryString => "complete query string",
            ForbiddenClass::TokenLike => "token-looking value",
        }
    }
}

impl std::fmt::Display for ForbiddenClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Invisible zero-width format characters that `char::is_control` (Unicode
/// general category Cc) does not cover but that must never enter a stored
/// label: once present they cannot be seen or reviewed. U+200B..U+200D
/// (zero width space, zero width non-joiner, zero width joiner), U+2060
/// (word joiner) and U+FEFF (zero width no-break space) are refused.
fn is_invisible_format_char(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}')
}

/// Check one string value against the spec's data-minimization rules:
/// no credentials, no token-looking blobs, no complete query strings.
///
/// The token rule flags any single whitespace-delimited word of 40 or more
/// characters made only of `[A-Za-z0-9_-]` that is not pure hex. Hex digests
/// are allowed because the spec stores source snapshot digests, change
/// hashes and proof hashes by design.
pub fn check_string_value(value: &str) -> Result<(), ForbiddenClass> {
    if value.is_empty() {
        return Err(ForbiddenClass::Empty);
    }
    if value.len() > MAX_STRING_BYTES {
        return Err(ForbiddenClass::TooLong);
    }
    if value
        .chars()
        .any(|c| c.is_control() || is_invisible_format_char(c))
    {
        return Err(ForbiddenClass::Control);
    }
    let lower = value.to_lowercase();
    if lower.contains("bearer ") {
        return Err(ForbiddenClass::Credential);
    }
    for needle in [
        "password=",
        "passwd=",
        "secret=",
        "token=",
        "api_key=",
        "apikey=",
        "access_key=",
        "private_key=",
    ] {
        if lower.contains(needle) {
            return Err(ForbiddenClass::Credential);
        }
    }
    if looks_like_jwt(value) {
        return Err(ForbiddenClass::Credential);
    }
    if value.contains('?') && (value.contains('=') || value.contains('&')) {
        return Err(ForbiddenClass::QueryString);
    }
    for word in value.split_whitespace() {
        if word.len() >= 40
            && word
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            && !word.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(ForbiddenClass::TokenLike);
        }
    }
    Ok(())
}

fn looks_like_jwt(value: &str) -> bool {
    let mut parts = value.split('.');
    let (Some(header), Some(_payload), Some(_signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    header.starts_with("eyJ") && header.len() > 8
}

fn valid_currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|b| b.is_ascii_uppercase())
}

fn valid_date(value: &str) -> bool {
    UtcMillis::parse_rfc3339(&format!("{value}T00:00:00Z")).is_ok()
}

fn valid_timestamp_or_date(value: &str) -> bool {
    if value.len() == 10 {
        valid_date(value)
    } else {
        UtcMillis::parse_rfc3339(value).is_ok()
    }
}

fn valid_period(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 7
        || bytes[4] != b'-'
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    value[5..]
        .parse::<u32>()
        .map(|month| (1..=12).contains(&month))
        .unwrap_or(false)
}

/// The exact experiment decision vocabulary (spec page 75). Stored decision
/// labels are case-normalized to lower case first; anything outside this
/// closed set is refused.
pub const DECISION_VOCABULARY: [&str; 5] = ["keep", "rollback", "void", "extend", "pending"];

impl DomainEvent {
    /// Start building an event of `event_type` from `source`.
    pub fn builder(event_type: EventType, source: EventSource) -> DomainEventBuilder {
        DomainEventBuilder {
            event_type,
            source,
            event_id: None,
            schema_version: 1,
            occurred_at_utc: None,
            received_at_utc: None,
            environment: None,
            service_version: None,
            publisher_id: None,
            platform: None,
            object_type: None,
            object_id: None,
            correlation_id: None,
            causation_id: None,
            outcome: None,
            reason_code: None,
            payload: EventPayload::default(),
        }
    }

    /// Treat empty strings as absent everywhere (missing is `None`, and an
    /// empty label is a missing label). Surrounding whitespace is not data:
    /// every string value is stored trimmed, and a whitespace-only value
    /// counts as absent, so it can never satisfy a required field. The
    /// `decision` vocabulary is lower-case in the catalog, so the decision
    /// label is case-normalized here and then validated against the exact
    /// vocabulary. List fields keep their shape: an empty `missing_inputs`
    /// list is a real statement.
    fn normalize_empty_strings(&mut self) {
        fn blank_to_none(value: &mut Option<String>) {
            let Some(current) = value.take() else {
                return;
            };
            let trimmed = current.trim();
            if trimmed.is_empty() {
                *value = None;
            } else if trimmed.len() == current.len() {
                *value = Some(current);
            } else {
                *value = Some(trimmed.to_string());
            }
        }
        blank_to_none(&mut self.publisher_id);
        blank_to_none(&mut self.platform);
        blank_to_none(&mut self.object_type);
        blank_to_none(&mut self.object_id);
        blank_to_none(&mut self.correlation_id);
        blank_to_none(&mut self.causation_id);
        blank_to_none(&mut self.outcome);
        blank_to_none(&mut self.reason_code);
        let payload = &mut self.payload;
        macro_rules! clear {
            ($($field:ident),+ $(,)?) => {
                $(blank_to_none(&mut payload.$field);)+
            };
        }
        clear!(
            route_template,
            referrer_host,
            campaign_code,
            bot_class,
            lead_id,
            account_id,
            range_start,
            range_end,
            source_code,
            validation_reason,
            destination_type,
            response_class,
            previous_stage,
            next_stage,
            owner,
            effective_at,
            platform_account_id,
            validator_version,
            failure_class,
            job_id,
            platform_operation,
            finality,
            error_class,
            snapshot_digest,
            rule_version,
            proposal_id,
            snapshot,
            change_hash,
            policy_version,
            approval_actor,
            expiry,
            experiment_id,
            source_digest,
            registered_metric,
            state,
            decision,
            guard_type,
            observed_input,
            threshold,
            target_state,
            operation_id,
            confirming_read,
            verification_version,
            change_id,
            period,
            currency,
            invoice_id,
            psp_event_key,
            incident_id,
            impact,
            action_actor,
            target,
            authorization,
            result,
        );
        // The catalog spells the decision vocabulary in lower case; a
        // producer's `Keep` is the same decision and must be stored and
        // validated as `keep`.
        if let Some(decision) = self.payload.decision.take() {
            self.payload.decision = Some(decision.to_lowercase());
        }
    }

    /// The canonical payload bytes: the build-time cache when present,
    /// otherwise (an event assembled inside the crate without `build`) the
    /// same serializer's output.
    fn canonical_payload_json(&self) -> std::borrow::Cow<'_, str> {
        if self.payload_json.is_empty() {
            std::borrow::Cow::Owned(self.payload.to_json_string())
        } else {
            std::borrow::Cow::Borrowed(&self.payload_json)
        }
    }

    /// A deterministic identity derived from the event content. Producers
    /// with a durable identity (form id, job id, PSP key) pass it explicitly;
    /// this derivation still makes at-least-once replay deduplicate.
    pub fn derive_event_id(&self) -> String {
        let mut hash = Fnv1a::new();
        hash.write_str(self.source.as_str());
        hash.write_str(self.event_type.as_str());
        hash.write_str(&self.occurred_at_utc.unix_millis().to_string());
        hash.write_str(self.object_id.as_deref().unwrap_or(""));
        hash.write_str(self.canonical_payload_json().as_ref());
        hash.finish_hex()
    }

    /// Category-level validation: field shapes, forbidden values, and the
    /// spec's per-family minimum fields.
    fn validate(&self) -> Result<(), ValidationError> {
        let event_type = self.event_type;
        let invalid = |field: &str, reason: &str| ValidationError::InvalidValue {
            event_type,
            field: field.to_string(),
            reason: reason.to_string(),
        };
        let forbidden = |field: &str, class: ForbiddenClass| ValidationError::ForbiddenValue {
            event_type,
            field: field.to_string(),
            reason: class.as_str(),
        };

        if self.event_id.len() > MAX_STRING_BYTES {
            return Err(invalid("event_id", "longer than the envelope allows"));
        }
        for (name, value) in self.envelope_string_fields() {
            if let Err(class) = check_string_value(value) {
                return Err(forbidden(name, class));
            }
        }
        for (name, value) in self.payload.string_fields() {
            if let Err(class) = check_string_value(value) {
                return Err(forbidden(name, class));
            }
        }

        // Field shapes.
        if let Some(status) = self.payload.status {
            if !(100..=599).contains(&status) {
                return Err(invalid("status", "outside 100 to 599"));
            }
        }
        if let Some(attempt) = self.payload.attempt {
            if attempt == 0 {
                return Err(invalid("attempt", "attempt numbering starts at 1"));
            }
        }
        if let Some(window_days) = self.payload.window_days {
            if window_days == 0 {
                return Err(invalid(
                    "window_days",
                    "a registered window needs at least one day",
                ));
            }
        }
        if let Some(rate_bp) = self.payload.rate_bp {
            if rate_bp > 10_000 {
                return Err(invalid("rate_bp", "rate above 100 percent"));
            }
        }
        if let Some(bound_bp) = self.payload.bound_bp {
            if bound_bp <= -10_000 {
                return Err(invalid("bound_bp", "bound of -100 percent or lower"));
            }
        }
        if let Some(bound_bp) = self.payload.decision_bound_bp {
            if bound_bp <= -10_000 {
                return Err(invalid(
                    "decision_bound_bp",
                    "bound of -100 percent or lower",
                ));
            }
        }
        if let Some(currency) = &self.payload.currency {
            if !valid_currency(currency) {
                return Err(invalid("currency", "not an ISO-4217 code"));
            }
        }
        if let Some(period) = &self.payload.period {
            if !valid_period(period) {
                return Err(invalid("period", "not a YYYY-MM month"));
            }
        }
        for field in ["range_start", "range_end"] {
            let value = match field {
                "range_start" => self.payload.range_start.as_deref(),
                "range_end" => self.payload.range_end.as_deref(),
                _ => None,
            };
            if let Some(value) = value {
                if !valid_date(value) {
                    return Err(invalid(field, "not a YYYY-MM-DD day"));
                }
            }
        }
        for field in ["effective_at", "expiry"] {
            let value = match field {
                "effective_at" => self.payload.effective_at.as_deref(),
                "expiry" => self.payload.expiry.as_deref(),
                _ => None,
            };
            if let Some(value) = value {
                if !valid_timestamp_or_date(value) {
                    return Err(invalid(field, "not a UTC timestamp or day"));
                }
            }
        }
        if let (Some(start_text), Some(end_text)) = (
            self.payload.range_start.as_deref(),
            self.payload.range_end.as_deref(),
        ) {
            let parse = |text: &str| UtcMillis::parse_rfc3339(&format!("{text}T00:00:00Z"));
            let start = parse(start_text)
                .map_err(|_| invalid("range_start", "range start does not parse"))?;
            let end =
                parse(end_text).map_err(|_| invalid("range_end", "range end does not parse"))?;
            if end < start {
                return Err(invalid("range_end", "range end is before range start"));
            }
        }
        self.validate_family()
    }

    fn envelope_string_fields(&self) -> Vec<(&'static str, &str)> {
        let mut out: Vec<(&'static str, &str)> = Vec::new();
        if !self.event_id.is_empty() {
            out.push(("event_id", self.event_id.as_str()));
        }
        out.push(("environment", self.environment.as_str()));
        out.push(("service_version", self.service_version.as_str()));
        macro_rules! one {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(value) = &self.$field {
                    out.push((stringify!($field), value.as_str()));
                })+
            };
        }
        one!(
            publisher_id,
            platform,
            object_type,
            object_id,
            correlation_id,
            causation_id,
            outcome,
            reason_code,
        );
        out
    }

    /// The spec's per-family minimum fields (section 05, event catalog).
    ///
    /// Where the catalog lists one union across a family of variants, the
    /// assignment below is the narrowest honest one: fields that cannot exist
    /// for a variant (a finding count on a failed audit, a decision bound on
    /// a Void decision) are not required for it, and the report documents the
    /// split. Failure variants require their failure class because the spec
    /// asks for the reason a boundary failed, and `Keep` decisions require
    /// the billable bound because a keep without a bound is not billable.
    fn validate_family(&self) -> Result<(), ValidationError> {
        let event_type = self.event_type;
        let payload = &self.payload;
        let missing = |field: &'static str| ValidationError::MissingField { event_type, field };
        let need = |value: &Option<String>, field: &'static str| -> Result<(), ValidationError> {
            if value.as_deref().unwrap_or("").is_empty() {
                Err(missing(field))
            } else {
                Ok(())
            }
        };

        if event_type.requires_outcome() && self.outcome.as_deref().unwrap_or("").is_empty() {
            return Err(missing("outcome"));
        }
        if event_type.requires_reason_code() && self.reason_code.as_deref().unwrap_or("").is_empty()
        {
            return Err(missing("reason_code"));
        }
        macro_rules! require_platform {
            () => {
                if self.platform.as_deref().unwrap_or("").is_empty() {
                    return Err(missing("platform"));
                }
            };
        }
        macro_rules! require_publisher {
            () => {
                if self.publisher_id.as_deref().unwrap_or("").is_empty() {
                    return Err(missing("publisher_id"));
                }
            };
        }

        match event_type {
            EventType::PublicPageRequested => {
                need(&payload.route_template, "route_template")?;
                if payload.status.is_none() {
                    return Err(missing("status"));
                }
                need(&payload.bot_class, "bot_class")?;
            }
            EventType::AuditRequestAccepted => {
                need(&payload.lead_id, "lead_id")?;
                if payload
                    .platforms
                    .as_ref()
                    .map(Vec::is_empty)
                    .unwrap_or(true)
                {
                    return Err(missing("platforms"));
                }
                need(&payload.range_start, "range_start")?;
                need(&payload.range_end, "range_end")?;
                need(&payload.source_code, "source_code")?;
            }
            EventType::AuditRequestRejected => {
                need(&payload.lead_id, "lead_id")?;
                need(&payload.source_code, "source_code")?;
                need(&payload.validation_reason, "validation_reason")?;
            }
            EventType::LeadForwardAttempted
            | EventType::LeadForwardAcknowledged
            | EventType::LeadForwardFailed => {
                need(&payload.lead_id, "lead_id")?;
                need(&payload.destination_type, "destination_type")?;
                if payload.attempt.is_none() {
                    return Err(missing("attempt"));
                }
                if matches!(
                    event_type,
                    EventType::LeadForwardAcknowledged | EventType::LeadForwardFailed
                ) {
                    need(&payload.response_class, "response_class")?;
                }
            }
            EventType::AccountStageChanged => {
                require_publisher!();
                need(&payload.previous_stage, "previous_stage")?;
                need(&payload.next_stage, "next_stage")?;
                need(&payload.owner, "owner")?;
                need(&payload.effective_at, "effective_at")?;
            }
            EventType::ConnectionValidated
            | EventType::ConnectionExpired
            | EventType::ConnectionRevoked => {
                require_publisher!();
                require_platform!();
                need(&payload.platform_account_id, "platform_account_id")?;
                if payload.read_capability.is_none() {
                    return Err(missing("read_capability"));
                }
                if payload.write_capability.is_none() {
                    return Err(missing("write_capability"));
                }
                need(&payload.validator_version, "validator_version")?;
                if matches!(
                    event_type,
                    EventType::ConnectionExpired | EventType::ConnectionRevoked
                ) {
                    need(&payload.failure_class, "failure_class")?;
                }
            }
            EventType::ReportPullStarted
            | EventType::ReportPullCompleted
            | EventType::ReportPullFailed => {
                require_publisher!();
                require_platform!();
                need(&payload.job_id, "job_id")?;
                need(&payload.platform_operation, "platform_operation")?;
                need(&payload.range_start, "range_start")?;
                need(&payload.range_end, "range_end")?;
                if matches!(
                    event_type,
                    EventType::ReportPullStarted | EventType::ReportPullFailed
                ) && payload.attempt.is_none()
                {
                    return Err(missing("attempt"));
                }
                if event_type == EventType::ReportPullCompleted {
                    if payload.rows.is_none() {
                        return Err(missing("rows"));
                    }
                    need(&payload.finality, "finality")?;
                    if payload.duration_ms.is_none() {
                        return Err(missing("duration_ms"));
                    }
                }
                if event_type == EventType::ReportPullFailed {
                    need(&payload.error_class, "error_class")?;
                }
            }
            EventType::AuditCompleted => {
                require_publisher!();
                require_platform!();
                need(&payload.snapshot_digest, "snapshot_digest")?;
                need(&payload.rule_version, "rule_version")?;
                if payload.eligible_rule_groups.is_none() {
                    return Err(missing("eligible_rule_groups"));
                }
                if payload.expected_rule_groups.is_none() {
                    return Err(missing("expected_rule_groups"));
                }
                if payload.findings_critical.is_none()
                    || payload.findings_warning.is_none()
                    || payload.findings_info.is_none()
                {
                    return Err(missing("finding_counts"));
                }
                if payload.missing_inputs.is_none() {
                    return Err(missing("missing_inputs"));
                }
                if payload.duration_ms.is_none() {
                    return Err(missing("duration_ms"));
                }
            }
            EventType::AuditFailed => {
                require_publisher!();
                require_platform!();
                need(&payload.rule_version, "rule_version")?;
                need(&payload.error_class, "error_class")?;
            }
            EventType::ProposalCreated
            | EventType::ProposalDeclined
            | EventType::ProposalExpired => {
                require_publisher!();
                need(&payload.proposal_id, "proposal_id")?;
                need(&payload.snapshot, "snapshot")?;
                need(&payload.change_hash, "change_hash")?;
                need(&payload.policy_version, "policy_version")?;
                if event_type == EventType::ProposalExpired {
                    need(&payload.expiry, "expiry")?;
                }
            }
            EventType::ProposalApproved => {
                require_publisher!();
                need(&payload.proposal_id, "proposal_id")?;
                need(&payload.snapshot, "snapshot")?;
                need(&payload.change_hash, "change_hash")?;
                need(&payload.policy_version, "policy_version")?;
                need(&payload.approval_actor, "approval_actor")?;
            }
            EventType::ExperimentRegistered => {
                require_publisher!();
                require_platform!();
                need(&payload.experiment_id, "experiment_id")?;
                need(&payload.source_digest, "source_digest")?;
                need(&payload.registered_metric, "registered_metric")?;
                if payload.window_days.is_none() {
                    return Err(missing("window_days"));
                }
            }
            EventType::ExperimentStarted => {
                require_publisher!();
                require_platform!();
                need(&payload.experiment_id, "experiment_id")?;
                need(&payload.state, "state")?;
            }
            EventType::ExperimentPull => {
                require_publisher!();
                require_platform!();
                need(&payload.experiment_id, "experiment_id")?;
                need(&payload.source_digest, "source_digest")?;
                need(&payload.range_start, "range_start")?;
                need(&payload.range_end, "range_end")?;
            }
            EventType::ExperimentDecided => {
                require_publisher!();
                require_platform!();
                need(&payload.experiment_id, "experiment_id")?;
                need(&payload.decision, "decision")?;
                let decision = payload.decision.as_deref().unwrap_or("");
                if !DECISION_VOCABULARY.contains(&decision) {
                    return Err(ValidationError::InvalidValue {
                        event_type,
                        field: "decision".to_string(),
                        reason: format!(
                            "'{decision}' is outside the decision vocabulary ({})",
                            DECISION_VOCABULARY.join(", ")
                        ),
                    });
                }
                // The bound rule applies to the normalized value: a `Keep`
                // decision (however it was cased) is billable only with a
                // point estimate and a billable lower bound.
                if decision == "keep" {
                    if payload.point_estimate_bp.is_none() {
                        return Err(missing("point_estimate_bp"));
                    }
                    if payload.decision_bound_bp.is_none() {
                        return Err(missing("decision_bound_bp"));
                    }
                }
            }
            EventType::GuardrailFired => {
                require_publisher!();
                require_platform!();
                need(&payload.guard_type, "guard_type")?;
                need(&payload.observed_input, "observed_input")?;
                need(&payload.threshold, "threshold")?;
            }
            EventType::GuardrailRestoreRequested => {
                require_publisher!();
                require_platform!();
                need(&payload.guard_type, "guard_type")?;
                need(&payload.operation_id, "operation_id")?;
                need(&payload.target_state, "target_state")?;
            }
            EventType::GuardrailRestoreConfirmed => {
                require_publisher!();
                require_platform!();
                need(&payload.guard_type, "guard_type")?;
                need(&payload.operation_id, "operation_id")?;
                need(&payload.target_state, "target_state")?;
                need(&payload.confirming_read, "confirming_read")?;
            }
            EventType::GuardrailRestoreFailed => {
                require_publisher!();
                require_platform!();
                need(&payload.guard_type, "guard_type")?;
                need(&payload.operation_id, "operation_id")?;
                need(&payload.target_state, "target_state")?;
                need(&payload.error_class, "error_class")?;
            }
            EventType::LedgerVerified => {
                require_publisher!();
                if payload.highest_sequence.is_none() {
                    return Err(missing("highest_sequence"));
                }
                need(&payload.verification_version, "verification_version")?;
                if payload.duration_ms.is_none() {
                    return Err(missing("duration_ms"));
                }
            }
            EventType::LedgerVerificationFailed => {
                require_publisher!();
                if payload.highest_sequence.is_none() {
                    return Err(missing("highest_sequence"));
                }
                need(&payload.verification_version, "verification_version")?;
                if payload.failure_line.is_none() {
                    return Err(missing("failure_line"));
                }
                if payload.duration_ms.is_none() {
                    return Err(missing("duration_ms"));
                }
            }
            EventType::FeeComputed => {
                require_publisher!();
                require_platform!();
                need(&payload.change_id, "change_id")?;
                need(&payload.period, "period")?;
                need(&payload.currency, "currency")?;
                if payload.source_amount_micros.is_none() {
                    return Err(missing("source_amount_micros"));
                }
                if payload.bound_bp.is_none() {
                    return Err(missing("bound_bp"));
                }
                if payload.rate_bp.is_none() {
                    return Err(missing("rate_bp"));
                }
                if payload.active_days.is_none() {
                    return Err(missing("active_days"));
                }
                if payload
                    .proof_hashes
                    .as_ref()
                    .map(Vec::is_empty)
                    .unwrap_or(true)
                {
                    return Err(missing("proof_hashes"));
                }
            }
            EventType::FeeHeld => {
                require_publisher!();
                need(&payload.change_id, "change_id")?;
                need(&payload.period, "period")?;
                need(&payload.currency, "currency")?;
            }
            EventType::FeeInvoiceIssued | EventType::FeeCredited => {
                require_publisher!();
                need(&payload.change_id, "change_id")?;
                need(&payload.period, "period")?;
                need(&payload.currency, "currency")?;
                need(&payload.invoice_id, "invoice_id")?;
                if payload.amount_micros.is_none() {
                    return Err(missing("amount_micros"));
                }
            }
            EventType::PaymentReceived => {
                require_publisher!();
                need(&payload.psp_event_key, "psp_event_key")?;
                need(&payload.invoice_id, "invoice_id")?;
                need(&payload.currency, "currency")?;
                if payload.amount_micros.is_none() {
                    return Err(missing("amount_micros"));
                }
            }
            EventType::PaymentReconciled
            | EventType::PaymentRefunded
            | EventType::PaymentDisputed => {
                require_publisher!();
                need(&payload.invoice_id, "invoice_id")?;
                need(&payload.currency, "currency")?;
                if payload.amount_micros.is_none() {
                    return Err(missing("amount_micros"));
                }
            }
            EventType::JobRun => {
                need(&payload.job_id, "job_id")?;
            }
            EventType::IncidentChanged => {
                require_publisher!();
                need(&payload.incident_id, "incident_id")?;
                need(&payload.owner, "owner")?;
                need(&payload.impact, "impact")?;
            }
            EventType::OperatorAction => {
                need(&payload.action_actor, "action_actor")?;
                need(&payload.target, "target")?;
                need(&payload.authorization, "authorization")?;
            }
        }
        Ok(())
    }

    /// Flatten to the row shape of the `domain_events` table
    /// (JSONEachRow-friendly primitives; the full typed payload rides in
    /// `payload_json`, the frequently queried fields are promoted columns).
    pub fn to_row(&self) -> EventRow {
        EventRow {
            event_id: self.event_id.clone(),
            event_type: self.event_type.as_str().to_string(),
            schema_version: self.schema_version,
            occurred_at_utc: self.occurred_at_utc.to_clickhouse(),
            received_at_utc: self.received_at_utc.to_clickhouse(),
            environment: self.environment.clone(),
            service_version: self.service_version.clone(),
            source: self.source.as_str().to_string(),
            publisher_id: self.publisher_id.clone(),
            platform: self.platform.clone(),
            object_type: self.object_type.clone(),
            object_id: self.object_id.clone(),
            correlation_id: self.correlation_id.clone(),
            causation_id: self.causation_id.clone(),
            outcome: self.outcome.clone(),
            reason_code: self.reason_code.clone(),
            currency: self.payload.currency.clone(),
            amount_micros: self
                .payload
                .amount_micros
                .or(self.payload.source_amount_micros),
            period: self.payload.period.clone(),
            lead_id: self.payload.lead_id.clone(),
            account_id: self.payload.account_id.clone(),
            job_id: self.payload.job_id.clone(),
            experiment_id: self.payload.experiment_id.clone(),
            change_id: self.payload.change_id.clone(),
            proposal_id: self.payload.proposal_id.clone(),
            invoice_id: self.payload.invoice_id.clone(),
            incident_id: self.payload.incident_id.clone(),
            platform_operation: self.payload.platform_operation.clone(),
            finality: self.payload.finality.clone(),
            rule_version: self.payload.rule_version.clone(),
            snapshot_digest: self.payload.snapshot_digest.clone(),
            guard_type: self.payload.guard_type.clone(),
            payload_json: self.canonical_payload_json().into_owned(),
        }
    }
}

/// The flat row shape inserted into the ClickHouse `domain_events` table.
/// `payload_json` is the typed payload; every other promoted column mirrors
/// a field readers filter on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventRow {
    /// Stable identity, half of the dedup key.
    pub event_id: String,
    /// Spec spelling of the event type (owned so rows deserialize).
    pub event_type: String,
    /// Envelope schema version.
    pub schema_version: u16,
    /// Occurred time, ClickHouse `DateTime64(3)` spelling.
    pub occurred_at_utc: String,
    /// Received time, ClickHouse `DateTime64(3)` spelling.
    pub received_at_utc: String,
    /// Deployment environment.
    pub environment: String,
    /// Emitting service version.
    pub service_version: String,
    /// Producer boundary, half of the dedup key (owned so rows deserialize).
    pub source: String,
    /// Tenant scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher_id: Option<String>,
    /// Platform scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Object class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    /// Object identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    /// Workflow correlation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Direct cause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    /// Bounded outcome label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Bounded reason code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    /// Currency of the promoted amount, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Money amount in integer micros, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_micros: Option<i64>,
    /// Billing period `YYYY-MM`, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
    /// Promoted lead identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_id: Option<String>,
    /// Promoted account identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Promoted job identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Promoted experiment identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    /// Promoted change identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
    /// Promoted proposal identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    /// Promoted invoice identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice_id: Option<String>,
    /// Promoted incident identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incident_id: Option<String>,
    /// Promoted platform operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_operation: Option<String>,
    /// Promoted finality label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finality: Option<String>,
    /// Promoted rule version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_version: Option<String>,
    /// Promoted snapshot digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    /// Promoted guard type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard_type: Option<String>,
    /// The full typed payload as compact JSON.
    pub payload_json: String,
}

/// FNV-1a 64-bit hasher; two passes with different seeds form the 128-bit
/// derived event identity. No external dependency is warranted for this.
struct Fnv1a {
    state: u64,
}

impl Fnv1a {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self {
            state: Self::OFFSET,
        }
    }

    fn write_str(&mut self, text: &str) {
        for byte in text.as_bytes() {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    fn finish_hex(self) -> String {
        let mut second = Fnv1a {
            state: self.state ^ 0x9e37_79b9_7f4a_7c15,
        };
        second.state ^= self.state.rotate_left(31);
        second.state = second.state.wrapping_mul(Self::PRIME);
        format!("{:016x}{:016x}", self.state, second.state)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    use crate::test_support::{occurred, payload_for, valid_event_builder};

    fn base(event_type: EventType) -> DomainEventBuilder {
        DomainEvent::builder(event_type, EventSource::Worker)
            .with_occurred_at(occurred())
            .with_received_at(occurred())
            .with_environment("production")
            .with_service_version("apex-0.1.0")
    }

    fn builder_for(event_type: EventType) -> DomainEventBuilder {
        valid_event_builder(event_type)
    }

    #[test]
    fn every_catalog_entry_builds_with_its_minimum_fields() {
        for event_type in EventType::ALL {
            let event = builder_for(event_type)
                .with_event_id(format!("evt_{}", event_type.as_str()))
                .build()
                .unwrap_or_else(|error| panic!("{event_type} must build: {error}"));
            assert_eq!(event.event_type, event_type);
            assert_eq!(event.schema_version, 1);
            assert_eq!(event.environment, "production");
            assert_eq!(event.service_version, "apex-0.1.0");
        }
    }

    #[test]
    fn event_type_names_match_the_catalog_spelling() {
        let expected = [
            "public_page_requested",
            "audit_request_accepted",
            "audit_request_rejected",
            "lead_forward_attempted",
            "lead_forward_acknowledged",
            "lead_forward_failed",
            "account_stage_changed",
            "connection_validated",
            "connection_expired",
            "connection_revoked",
            "report_pull_started",
            "report_pull_completed",
            "report_pull_failed",
            "audit_completed",
            "audit_failed",
            "proposal_created",
            "proposal_approved",
            "proposal_declined",
            "proposal_expired",
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
            "fee_credited",
            "payment_received",
            "payment_reconciled",
            "payment_refunded",
            "payment_disputed",
            "job_run",
            "incident_changed",
            "operator_action",
        ];
        assert_eq!(EventType::ALL.len(), expected.len());
        for (event_type, name) in EventType::ALL.into_iter().zip(expected) {
            assert_eq!(event_type.as_str(), name);
            assert_eq!(EventType::parse(name), Some(event_type));
            assert_eq!(event_type.to_string(), name);
        }
        assert_eq!(EventType::parse("not_a_real_event"), None);
    }

    #[test]
    fn families_group_the_catalog() {
        assert_eq!(
            EventType::PublicPageRequested.family(),
            EventFamily::PageRequest
        );
        assert_eq!(
            EventType::GuardrailRestoreFailed.family(),
            EventFamily::Guardrail
        );
        assert_eq!(EventType::PaymentDisputed.family(), EventFamily::Payment);
        assert_eq!(EventType::OperatorAction.family(), EventFamily::Operations);
        assert_eq!(EventFamily::Fee.as_str(), "fee");
    }

    #[test]
    fn missing_minimum_fields_are_rejected_field_by_field() {
        // One probe per family: the catalog's minimums are enforced, and the
        // error names the missing field.
        let probes: [(EventType, &'static str); 12] = [
            (EventType::PublicPageRequested, "route_template"),
            (EventType::AuditRequestAccepted, "lead_id"),
            (EventType::LeadForwardAttempted, "attempt"),
            (EventType::AccountStageChanged, "owner"),
            (EventType::ConnectionValidated, "validator_version"),
            (EventType::ReportPullCompleted, "rows"),
            (EventType::AuditCompleted, "snapshot_digest"),
            (EventType::ProposalApproved, "approval_actor"),
            (EventType::ExperimentDecided, "decision_bound_bp"),
            (EventType::GuardrailRestoreConfirmed, "confirming_read"),
            (EventType::FeeComputed, "proof_hashes"),
            (EventType::PaymentReceived, "psp_event_key"),
        ];
        for (event_type, field) in probes {
            let mut payload = payload_for(event_type);
            match field {
                "route_template" => payload.route_template = None,
                "lead_id" => payload.lead_id = None,
                "attempt" => payload.attempt = None,
                "owner" => payload.owner = None,
                "validator_version" => payload.validator_version = None,
                "rows" => payload.rows = None,
                "snapshot_digest" => payload.snapshot_digest = None,
                "approval_actor" => payload.approval_actor = None,
                "decision_bound_bp" => payload.decision_bound_bp = None,
                "confirming_read" => payload.confirming_read = None,
                "proof_hashes" => payload.proof_hashes = None,
                "psp_event_key" => payload.psp_event_key = None,
                _ => panic!("unhandled probe {field}"),
            }
            let error = builder_for(event_type)
                .with_payload(payload)
                .build()
                .expect_err("must be refused");
            assert!(
                matches!(error, ValidationError::MissingField { field: got, .. } if got == field),
                "{event_type}: expected MissingField({field}), got {error:?}"
            );
        }
    }

    #[test]
    fn rejected_families_require_their_envelope_labels() {
        let no_reason = builder_for(EventType::AccountStageChanged)
            .with_reason_code("")
            .build()
            .expect_err("account_stage_changed needs a reason");
        assert!(matches!(
            no_reason,
            ValidationError::MissingField {
                field: "reason_code",
                ..
            }
        ));

        let no_outcome = builder_for(EventType::JobRun)
            .with_outcome("")
            .build()
            .expect_err("job_run needs an outcome");
        assert!(matches!(
            no_outcome,
            ValidationError::MissingField {
                field: "outcome",
                ..
            }
        ));

        let no_publisher = DomainEvent::builder(EventType::FeeComputed, EventSource::Finance)
            .with_occurred_at(occurred())
            .with_received_at(occurred())
            .with_environment("production")
            .with_service_version("v")
            .with_platform("applovin_max")
            .with_payload(payload_for(EventType::FeeComputed))
            .build()
            .expect_err("fees are tenant-scoped");
        assert!(matches!(
            no_publisher,
            ValidationError::MissingField {
                field: "publisher_id",
                ..
            }
        ));
    }

    #[test]
    fn a_keep_decision_needs_its_billable_bound() {
        let mut payload = payload_for(EventType::ExperimentDecided);
        payload.decision_bound_bp = None;
        let error = builder_for(EventType::ExperimentDecided)
            .with_payload(payload)
            .build()
            .expect_err("keep without a bound is not billable");
        assert!(matches!(
            error,
            ValidationError::MissingField {
                field: "decision_bound_bp",
                ..
            }
        ));

        // A Void decision has no bound to give and must still be honest.
        let mut void = payload_for(EventType::ExperimentDecided);
        void.decision = Some("void".into());
        void.point_estimate_bp = None;
        void.decision_bound_bp = None;
        builder_for(EventType::ExperimentDecided)
            .with_payload(void)
            .build()
            .expect("void decisions carry no bound");
    }

    #[test]
    fn token_looking_values_are_refused_everywhere() {
        let mut payload = payload_for(EventType::AccountStageChanged);
        // Composed at runtime so the source carries no key-shaped literal;
        // the validator must still refuse the same value.
        let stripe_like = format!(
            "{}_{}",
            "sk", "live_51H8xyzABCDEFGHIJKLMNOPQRSTUVWXY0123456789"
        );
        payload.owner = Some(stripe_like);
        let error = builder_for(EventType::AccountStageChanged)
            .with_payload(payload)
            .build()
            .expect_err("token-looking owner");
        assert!(matches!(error, ValidationError::ForbiddenValue { .. }));

        let jwt = builder_for(EventType::GuardrailFired)
            .with_payload(EventPayload {
                guard_type: Some("fill".into()),
                observed_input: Some(
                    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxIn0.sig".into(),
                ),
                threshold: Some("-2%".into()),
                ..EventPayload::default()
            })
            .build()
            .expect_err("JWT payload");
        assert!(matches!(jwt, ValidationError::ForbiddenValue { .. }));

        let bearer = builder_for(EventType::IncidentChanged)
            .with_causation_id("Bearer abcdef")
            .build()
            .expect_err("bearer credential");
        assert!(matches!(bearer, ValidationError::ForbiddenValue { .. }));

        let env_secret = DomainEvent::builder(EventType::JobRun, EventSource::Worker)
            .with_environment("production")
            .with_service_version("v")
            .with_occurred_at(occurred())
            .with_received_at(occurred())
            .with_reason_code("api_key=abcd1234")
            .with_payload(payload_for(EventType::JobRun))
            .with_outcome("completed")
            .build()
            .expect_err("key assignment");
        assert!(matches!(env_secret, ValidationError::ForbiddenValue { .. }));
    }

    #[test]
    fn long_hex_digests_are_allowed_but_long_opaque_words_are_not() {
        assert!(check_string_value(&"a".repeat(64)).is_ok());
        assert!(check_string_value("short_value_1").is_ok());
        assert!(check_string_value(&"AbCdEf0123456789xyz".repeat(3)).is_err());
        // 40+ characters that are letters beyond hex are refused.
        assert!(check_string_value(&"z".repeat(41)).is_err());
    }

    #[test]
    fn complete_query_strings_are_refused() {
        let mut payload = payload_for(EventType::PublicPageRequested);
        payload.referrer_host = Some("example.com?utm_source=newsletter".into());
        let error = builder_for(EventType::PublicPageRequested)
            .with_payload(payload)
            .build()
            .expect_err("query string");
        assert!(matches!(error, ValidationError::ForbiddenValue { .. }));

        // A bare question mark is not a query string.
        let mut payload = payload_for(EventType::PublicPageRequested);
        payload.campaign_code = Some("launch?".into());
        builder_for(EventType::PublicPageRequested)
            .with_payload(payload)
            .build()
            .expect("a bare question mark is not a query string");

        // Query keys are keys only and are allowed.
        let mut payload = payload_for(EventType::PublicPageRequested);
        payload.query_keys = Some(vec!["utm_source".into(), "utm_campaign".into()]);
        builder_for(EventType::PublicPageRequested)
            .with_payload(payload)
            .build()
            .expect("allow-listed keys");
    }

    #[test]
    fn field_shapes_are_validated() {
        let mut payload = payload_for(EventType::PublicPageRequested);
        payload.status = Some(99);
        assert!(builder_for(EventType::PublicPageRequested)
            .with_payload(payload)
            .build()
            .is_err());

        let mut payload = payload_for(EventType::LeadForwardAttempted);
        payload.attempt = Some(0);
        assert!(builder_for(EventType::LeadForwardAttempted)
            .with_payload(payload)
            .build()
            .is_err());

        let mut payload = payload_for(EventType::FeeComputed);
        payload.currency = Some("usd".into());
        assert!(builder_for(EventType::FeeComputed)
            .with_payload(payload)
            .build()
            .is_err());

        let mut payload = payload_for(EventType::FeeComputed);
        payload.period = Some("2026-13".into());
        assert!(builder_for(EventType::FeeComputed)
            .with_payload(payload)
            .build()
            .is_err());

        let mut payload = payload_for(EventType::FeeComputed);
        payload.bound_bp = Some(-10_000);
        assert!(builder_for(EventType::FeeComputed)
            .with_payload(payload)
            .build()
            .is_err());

        let mut payload = payload_for(EventType::ReportPullCompleted);
        payload.range_start = Some("2026-10-14".into());
        payload.range_end = Some("2026-10-01".into());
        assert!(builder_for(EventType::ReportPullCompleted)
            .with_payload(payload)
            .build()
            .is_err());

        let reversed = base(EventType::JobRun)
            .with_outcome("completed")
            .with_payload(payload_for(EventType::JobRun))
            .with_occurred_at(UtcMillis::from_unix_millis(1_000))
            .with_received_at(UtcMillis::from_unix_millis(500))
            .build();
        assert!(matches!(
            reversed,
            Err(ValidationError::InvalidValue { .. })
        ));
    }

    #[test]
    fn required_envelope_fields_are_enforced() {
        let no_environment = DomainEvent::builder(EventType::JobRun, EventSource::Worker)
            .with_service_version("v")
            .with_occurred_at(occurred())
            .with_received_at(occurred())
            .with_outcome("completed")
            .with_payload(payload_for(EventType::JobRun))
            .build();
        assert!(matches!(
            no_environment,
            Err(ValidationError::MissingField {
                field: "environment",
                ..
            })
        ));

        let no_service = DomainEvent::builder(EventType::JobRun, EventSource::Worker)
            .with_environment("production")
            .with_occurred_at(occurred())
            .with_received_at(occurred())
            .with_outcome("completed")
            .with_payload(payload_for(EventType::JobRun))
            .build();
        assert!(matches!(
            no_service,
            Err(ValidationError::MissingField {
                field: "service_version",
                ..
            })
        ));
    }

    #[test]
    fn payload_is_serialized_once_and_the_row_reuses_the_canonical_bytes() {
        reset_payload_serializations();
        let event = builder_for(EventType::FeeComputed).build().expect("event");
        assert_eq!(
            payload_serializations(),
            1,
            "build serializes the canonical payload exactly once"
        );

        // The derived identity is a hash of the bytes computed at build time;
        // deriving it again must not reserialize the payload.
        let derived = event.derive_event_id();
        assert_eq!(derived, event.event_id);
        assert_eq!(payload_serializations(), 1);

        // The stored row carries those exact bytes, not a second rendering.
        let row = event.to_row();
        assert_eq!(row.payload_json, event.payload_json);
        assert_eq!(payload_serializations(), 1);

        // A direct serializer call (the equivalence reference) is the only
        // way the counter moves again.
        assert_eq!(row.payload_json, event.payload.to_json_string());
        assert_eq!(payload_serializations(), 2);
    }

    #[test]
    fn derived_identities_are_stable_and_content_sensitive() {
        let first = builder_for(EventType::JobRun).build().expect("first");
        let again = builder_for(EventType::JobRun).build().expect("again");
        assert_eq!(first.event_id, again.event_id);
        assert_eq!(first.event_id.len(), 32);

        let mut changed = payload_for(EventType::JobRun);
        changed.job_id = Some("job_other".into());
        let other = builder_for(EventType::JobRun)
            .with_payload(changed)
            .build()
            .expect("other");
        assert_ne!(first.event_id, other.event_id);

        // Explicit identities win over derivation.
        let explicit = builder_for(EventType::JobRun)
            .with_event_id("job_daily_2026-10-15")
            .build()
            .expect("explicit");
        assert_eq!(explicit.event_id, "job_daily_2026-10-15");
    }

    #[test]
    fn payload_json_matches_the_serde_shape() {
        let payload = payload_for(EventType::FeeComputed);
        let text = payload.to_json_string();
        let back: EventPayload = serde_json::from_str(&text).expect("round trip");
        assert_eq!(back, payload);
        assert!(text.contains("\"bound_bp\":300"));
        assert!(text.contains("\"currency\":\"USD\""));
        // Absent fields are omitted, not nulled and never zeroed.
        assert!(!text.contains("\"status\""));
        assert!(!text.contains("null"));
    }

    #[test]
    fn to_row_promotes_the_queryable_fields() {
        let event = builder_for(EventType::FeeComputed)
            .with_event_id("fee_evt_1")
            .build()
            .expect("fee event");
        let row = event.to_row();
        assert_eq!(row.event_type, "fee_computed");
        assert_eq!(row.source, "finance");
        assert_eq!(row.currency.as_deref(), Some("USD"));
        assert_eq!(row.amount_micros, Some(51_500_000_000));
        assert_eq!(row.change_id.as_deref(), Some("ch_0246"));
        assert_eq!(row.period.as_deref(), Some("2026-10"));
        assert_eq!(row.occurred_at_utc, "2026-10-15 09:10:00.000");
        let json = serde_json::to_string(&row).expect("row json");
        assert!(json.contains("\"event_type\":\"fee_computed\""));
        assert!(json.contains("\"amount_micros\":51500000000"));

        let page = builder_for(EventType::PublicPageRequested)
            .with_object("document", "/pricing")
            .build()
            .expect("page event");
        let row = page.to_row();
        assert_eq!(row.object_id.as_deref(), Some("/pricing"));
        assert!(row.amount_micros.is_none());
        assert!(row.currency.is_none());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.payload_json).expect("payload json")
                ["route_template"],
            "/pricing"
        );
    }

    #[test]
    fn sources_name_their_boundaries() {
        assert_eq!(EventSource::AccessLog.as_str(), "access_log");
        assert_eq!(EventSource::Axum.as_str(), "axum");
        assert_eq!(EventSource::Worker.as_str(), "worker");
        assert_eq!(EventSource::Connector.as_str(), "connector");
        assert_eq!(EventSource::Operator.as_str(), "operator");
        assert_eq!(EventSource::Finance.as_str(), "finance");
        assert_eq!(EventSource::parse("worker"), Some(EventSource::Worker));
        assert_eq!(EventSource::parse("unknown"), None);
    }

    #[test]
    fn event_types_and_sources_round_trip_through_serde() {
        for event_type in EventType::ALL {
            let json = serde_json::to_string(&event_type).expect("serialize");
            assert_eq!(json, format!("\"{}\"", event_type.as_str()));
            let back: EventType = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, event_type);
        }
        assert!(serde_json::from_str::<EventType>("\"nope\"").is_err());
        for source in [
            EventSource::AccessLog,
            EventSource::Axum,
            EventSource::Worker,
            EventSource::Connector,
            EventSource::Operator,
            EventSource::Finance,
        ] {
            let json = serde_json::to_string(&source).expect("serialize");
            let back: EventSource = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, source);
        }
        assert!(serde_json::from_str::<EventSource>("\"nope\"").is_err());
    }

    #[test]
    fn families_have_stable_names() {
        for family in [
            EventFamily::PageRequest,
            EventFamily::AuditRequest,
            EventFamily::LeadForward,
            EventFamily::AccountLifecycle,
            EventFamily::Connection,
            EventFamily::ReportPull,
            EventFamily::AuditRun,
            EventFamily::Proposal,
            EventFamily::Experiment,
            EventFamily::Guardrail,
            EventFamily::Ledger,
            EventFamily::Fee,
            EventFamily::Payment,
            EventFamily::Operations,
        ] {
            assert!(!family.as_str().is_empty());
            assert!(family
                .as_str()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'));
        }
        assert_eq!(EventFamily::Operations.as_str(), "operations");
    }

    #[test]
    fn rows_deserialize_back_into_the_row_shape() {
        let event = builder_for(EventType::JobRun)
            .with_event_id("job_row")
            .build()
            .expect("event");
        let row = event.to_row();
        let json = serde_json::to_string(&row).expect("json");
        let back: EventRow = serde_json::from_str(&json).expect("row back");
        assert_eq!(back, row);
        assert_eq!(back.payload_json, event.payload.to_json_string());
    }

    #[test]
    fn forbidden_classes_name_their_reason() {
        assert_eq!(ForbiddenClass::Empty.as_str(), "empty value");
        assert_eq!(
            ForbiddenClass::TooLong.to_string(),
            "longer than the envelope allows"
        );
        assert_eq!(ForbiddenClass::Control.as_str(), "control character");
        assert_eq!(
            ForbiddenClass::Credential.as_str(),
            "credential-looking value"
        );
        assert_eq!(
            ForbiddenClass::QueryString.as_str(),
            "complete query string"
        );
        assert_eq!(ForbiddenClass::TokenLike.as_str(), "token-looking value");
        assert_eq!(check_string_value(""), Err(ForbiddenClass::Empty));
        assert_eq!(
            check_string_value(&"x".repeat(2_000)),
            Err(ForbiddenClass::TooLong)
        );
        assert_eq!(
            check_string_value("line\nbreak"),
            Err(ForbiddenClass::Control)
        );
    }

    #[test]
    fn empty_strings_normalize_to_missing() {
        let mut payload = payload_for(EventType::PublicPageRequested);
        payload.referrer_host = Some(String::new());
        let event = builder_for(EventType::PublicPageRequested)
            .with_payload(payload)
            .with_correlation_id("")
            .build()
            .expect("empty optional strings normalize");
        assert_eq!(event.payload.referrer_host, None);
        assert_eq!(event.correlation_id, None);
    }

    #[test]
    fn schema_version_zero_is_refused() {
        let error = builder_for(EventType::JobRun)
            .with_schema_version(0)
            .build()
            .expect_err("zero version");
        assert!(matches!(error, ValidationError::InvalidValue { .. }));
    }

    #[test]
    fn whitespace_only_required_strings_are_refused_and_values_are_stored_trimmed() {
        // route_template: whitespace-only is missing, exactly like "".
        let mut payload = payload_for(EventType::PublicPageRequested);
        payload.route_template = Some("   ".into());
        let error = builder_for(EventType::PublicPageRequested)
            .with_payload(payload)
            .build()
            .expect_err("whitespace-only route template");
        assert!(matches!(
            error,
            ValidationError::MissingField {
                field: "route_template",
                ..
            }
        ));

        // change_id: the same rule.
        let mut payload = payload_for(EventType::FeeComputed);
        payload.change_id = Some("\t ".into());
        let error = builder_for(EventType::FeeComputed)
            .with_payload(payload)
            .build()
            .expect_err("whitespace-only change id");
        assert!(matches!(
            error,
            ValidationError::MissingField {
                field: "change_id",
                ..
            }
        ));

        // owner: the same rule.
        let mut payload = payload_for(EventType::AccountStageChanged);
        payload.owner = Some("  ".into());
        let error = builder_for(EventType::AccountStageChanged)
            .with_payload(payload)
            .build()
            .expect_err("whitespace-only owner");
        assert!(matches!(
            error,
            ValidationError::MissingField { field: "owner", .. }
        ));

        // The envelope scope field follows the same rule.
        let error = DomainEvent::builder(EventType::FeeComputed, EventSource::Finance)
            .with_occurred_at(occurred())
            .with_received_at(occurred())
            .with_environment("production")
            .with_service_version("v")
            .with_platform("applovin_max")
            .with_publisher_id("   ")
            .with_payload(payload_for(EventType::FeeComputed))
            .build()
            .expect_err("whitespace-only publisher");
        assert!(matches!(
            error,
            ValidationError::MissingField {
                field: "publisher_id",
                ..
            }
        ));

        // A padded value passes and is stored trimmed.
        let mut payload = payload_for(EventType::PublicPageRequested);
        payload.route_template = Some("  /pricing  ".into());
        let event = builder_for(EventType::PublicPageRequested)
            .with_payload(payload)
            .build()
            .expect("padded values are stored trimmed");
        assert_eq!(event.payload.route_template.as_deref(), Some("/pricing"));
        assert_eq!(event.payload.bot_class.as_deref(), Some("unclassified"));
    }

    #[test]
    fn zero_width_format_characters_are_refused() {
        for invisible in ['\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}'] {
            let value = format!("/pricing{invisible}");
            assert_eq!(
                check_string_value(&value),
                Err(ForbiddenClass::Control),
                "U+{:04X} must be refused",
                invisible as u32
            );
        }
        let mut payload = payload_for(EventType::PublicPageRequested);
        payload.route_template = Some("\u{200B}".into());
        let error = builder_for(EventType::PublicPageRequested)
            .with_payload(payload)
            .build()
            .expect_err("zero-width route template");
        assert!(matches!(
            error,
            ValidationError::ForbiddenValue {
                reason: "control character",
                ..
            }
        ));
        // Ordinary values still pass.
        assert!(check_string_value("/pricing").is_ok());
    }

    #[test]
    fn experiment_decisions_are_normalized_and_vocabulary_bound() {
        // Case is normalized before validation and storage: `Keep` is the
        // same decision as `keep`.
        let mut payload = payload_for(EventType::ExperimentDecided);
        payload.decision = Some("Keep".into());
        let event = builder_for(EventType::ExperimentDecided)
            .with_payload(payload)
            .build()
            .expect("Keep is keep");
        assert_eq!(event.payload.decision.as_deref(), Some("keep"));
        assert!(event
            .payload
            .to_json_string()
            .contains("\"decision\":\"keep\""));

        let mut payload = payload_for(EventType::ExperimentDecided);
        payload.decision = Some(" RoLLbAcK ".into());
        let event = builder_for(EventType::ExperimentDecided)
            .with_payload(payload)
            .build()
            .expect("mixed case is normalized");
        assert_eq!(event.payload.decision.as_deref(), Some("rollback"));

        // The bound rule applies to the normalized value: a capitalized
        // Keep without its billable bound is refused.
        let mut payload = payload_for(EventType::ExperimentDecided);
        payload.decision = Some("Keep".into());
        payload.decision_bound_bp = None;
        let error = builder_for(EventType::ExperimentDecided)
            .with_payload(payload)
            .build()
            .expect_err("Keep needs its billable bound");
        assert!(matches!(
            error,
            ValidationError::MissingField {
                field: "decision_bound_bp",
                ..
            }
        ));

        // An out-of-band bound is refused too, capitalized or not.
        let mut payload = payload_for(EventType::ExperimentDecided);
        payload.decision = Some("Keep".into());
        payload.decision_bound_bp = Some(-10_000);
        let error = builder_for(EventType::ExperimentDecided)
            .with_payload(payload)
            .build()
            .expect_err("out-of-band decision bound");
        assert!(matches!(
            error,
            ValidationError::InvalidValue { field, .. } if field == "decision_bound_bp"
        ));

        // The vocabulary is closed.
        let mut payload = payload_for(EventType::ExperimentDecided);
        payload.decision = Some("banana".into());
        let error = builder_for(EventType::ExperimentDecided)
            .with_payload(payload)
            .build()
            .expect_err("banana is not a decision");
        assert!(matches!(
            error,
            ValidationError::InvalidValue { field, .. } if field == "decision"
        ));

        // Every vocabulary value builds; only keep carries the bound rule.
        for decision in DECISION_VOCABULARY {
            let mut payload = payload_for(EventType::ExperimentDecided);
            payload.decision = Some(decision.to_string());
            if decision != "keep" {
                payload.point_estimate_bp = None;
                payload.decision_bound_bp = None;
            }
            let event = builder_for(EventType::ExperimentDecided)
                .with_payload(payload)
                .build()
                .unwrap_or_else(|error| panic!("{decision} must build: {error}"));
            assert_eq!(event.payload.decision.as_deref(), Some(decision));
        }
    }
}
