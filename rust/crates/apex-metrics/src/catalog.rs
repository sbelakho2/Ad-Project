//! The metric catalog (section 06 pages 111 and 112).
//!
//! Every metric the design names is declared here as a `static`
//! [`MetricSpec`]; producers write through the [`Registry`](crate::Registry)
//! with declared label values only. The request-level series
//! (`apex_http_*`) are declared for the console/API workstream; the existing
//! hand-rolled counters in `apex-api` are kept during migration (page 111:
//! "Keep existing counters during migration").

use crate::spec::{LabelSpec, MetricKind, MetricSpec, Vocab};
use crate::vocab::{
    ActionClass, CapabilityClass, Channel, ConnectorOperation, Decision, Environment, Guardrail,
    GuardrailAction, JobType, LedgerResult, Method, NotificationOutcome, Outcome, OutcomeClass,
    PaymentEventClass, PaymentProvider, PaymentResult, Platform, ProjectionName, ReadinessClass,
    ReasonClass, RestoreOperation, RestoreOutcome, RuleVersion, Service, StatusClass,
    TemplateClass, VerifierVersion,
};

/// Server response latency buckets (page 111: "buckets appropriate to latency
/// objectives; derive quantiles across instances").
pub const HTTP_LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// External platform-call latency buckets.
pub const CONNECTOR_LATENCY_BUCKETS: &[f64] =
    &[0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0];

/// Whole-job duration buckets, from seconds to a quarter hour.
pub const JOB_LATENCY_BUCKETS: &[f64] = &[0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0, 900.0];

/// Schedule lag buckets: on time, minutes late, hours late, a day late.
pub const SCHEDULE_LAG_BUCKETS: &[f64] = &[0.0, 60.0, 300.0, 900.0, 3600.0, 21_600.0, 86_400.0];

macro_rules! labels {
    ($($name:literal => $vocab:expr),+ $(,)?) => {
        &[$(LabelSpec { name: $name, vocab: $vocab }),+]
    };
}

/// Total HTTP requests by status class, including redirects (page 111).
pub static HTTP_REQUESTS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_http_requests_total",
    help: "Total HTTP requests by status class.",
    kind: MetricKind::Counter,
    labels: labels!(
        "service" => Vocab::Values(Service::VALUES),
        "route_template" => Vocab::RouteTemplate,
        "method" => Vocab::Values(Method::VALUES),
        "status_class" => Vocab::Values(StatusClass::VALUES),
    ),
};

/// Server response duration (page 111).
pub static HTTP_REQUEST_DURATION_SECONDS: MetricSpec = MetricSpec {
    name: "apex_http_request_duration_seconds",
    help: "HTTP response duration in seconds by route template.",
    kind: MetricKind::Histogram {
        buckets: HTTP_LATENCY_BUCKETS,
    },
    labels: labels!(
        "service" => Vocab::Values(Service::VALUES),
        "route_template" => Vocab::RouteTemplate,
        "method" => Vocab::Values(Method::VALUES),
    ),
};

/// Requests in progress (page 111); the in-flight guard keeps this honest
/// under cancellation and errors.
pub static HTTP_REQUESTS_IN_FLIGHT: MetricSpec = MetricSpec {
    name: "apex_http_requests_in_flight",
    help: "HTTP requests currently being served.",
    kind: MetricKind::Gauge,
    labels: labels!(
        "service" => Vocab::Values(Service::VALUES),
    ),
};

/// Connector call outcomes (page 111).
pub static CONNECTOR_REQUESTS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_connector_requests_total",
    help: "External platform calls by outcome class; retries are separate attempts.",
    kind: MetricKind::Counter,
    labels: labels!(
        "platform" => Vocab::Values(Platform::VALUES),
        "operation" => Vocab::Values(ConnectorOperation::VALUES),
        "outcome_class" => Vocab::Values(OutcomeClass::VALUES),
    ),
};

/// Connector call duration (page 111).
pub static CONNECTOR_DURATION_SECONDS: MetricSpec = MetricSpec {
    name: "apex_connector_duration_seconds",
    help: "External platform call duration in seconds; retries are separate attempts.",
    kind: MetricKind::Histogram {
        buckets: CONNECTOR_LATENCY_BUCKETS,
    },
    labels: labels!(
        "platform" => Vocab::Values(Platform::VALUES),
        "operation" => Vocab::Values(ConnectorOperation::VALUES),
    ),
};

/// Job runs (page 111); job IDs belong in durable events, not labels.
pub static JOB_RUNS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_job_runs_total",
    help: "Scheduled runner job runs by outcome.",
    kind: MetricKind::Counter,
    labels: labels!(
        "job_type" => Vocab::Values(JobType::VALUES),
        "platform" => Vocab::Values(Platform::VALUES),
        "outcome" => Vocab::Values(Outcome::VALUES),
    ),
};

/// Job duration (page 111).
pub static JOB_DURATION_SECONDS: MetricSpec = MetricSpec {
    name: "apex_job_duration_seconds",
    help: "Scheduled runner job duration in seconds by outcome.",
    kind: MetricKind::Histogram {
        buckets: JOB_LATENCY_BUCKETS,
    },
    labels: labels!(
        "job_type" => Vocab::Values(JobType::VALUES),
        "platform" => Vocab::Values(Platform::VALUES),
        "outcome" => Vocab::Values(Outcome::VALUES),
    ),
};

/// Actual start minus expected schedule (page 111).
pub static JOB_SCHEDULE_LAG_SECONDS: MetricSpec = MetricSpec {
    name: "apex_job_schedule_lag_seconds",
    help: "Actual job start minus the expected schedule.",
    kind: MetricKind::Histogram {
        buckets: SCHEDULE_LAG_BUCKETS,
    },
    labels: labels!(
        "job_type" => Vocab::Values(JobType::VALUES),
        "platform" => Vocab::Values(Platform::VALUES),
    ),
};

/// The overdue-job gauge the scheduler pairs with schedule lag (page 111).
///
/// No in-repo scheduler exists: systemd timers invoke `apex-runner` directly,
/// so nothing sets this gauge today. The declaration is the contract for the
/// scheduler workstream; the runner never writes a value it cannot observe.
pub static JOBS_OVERDUE: MetricSpec = MetricSpec {
    name: "apex_jobs_overdue",
    help: "Scheduled jobs past their expected start plus grace.",
    kind: MetricKind::Gauge,
    labels: labels!(
        "job_type" => Vocab::Values(JobType::VALUES),
        "platform" => Vocab::Values(Platform::VALUES),
    ),
};

/// Projection lag (page 111).
pub static PROJECTION_LAG_SECONDS: MetricSpec = MetricSpec {
    name: "apex_projection_lag_seconds",
    help: "Latest source event time minus the projected watermark, in seconds.",
    kind: MetricKind::Gauge,
    labels: labels!(
        "projection_name" => Vocab::Values(ProjectionName::VALUES),
        "environment" => Vocab::Values(Environment::VALUES),
    ),
};

/// Account counts by readiness class (page 112). Per-account freshness stays
/// in the read models, never in a label.
pub static REPORT_ACCOUNTS: MetricSpec = MetricSpec {
    name: "apex_report_accounts",
    help: "Reporting account counts by readiness class.",
    kind: MetricKind::Gauge,
    labels: labels!(
        "platform" => Vocab::Values(Platform::VALUES),
        "readiness_class" => Vocab::Values(ReadinessClass::VALUES),
    ),
};

/// Audit execution outcomes (page 112) with a bounded rule-version release
/// set.
pub static AUDIT_RUNS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_audit_runs_total",
    help: "Audit execution outcomes by rule release.",
    kind: MetricKind::Counter,
    labels: labels!(
        "platform" => Vocab::Values(Platform::VALUES),
        "rule_version" => Vocab::Values(RuleVersion::VALUES),
        "outcome" => Vocab::Values(Outcome::VALUES),
    ),
};

/// Experiment decisions (page 112); cohort queries deduplicate repeated
/// observations in the analytics store.
pub static EXPERIMENT_DECISIONS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_experiment_decisions_total",
    help: "Experiment decision outcomes by bounded reason class.",
    kind: MetricKind::Counter,
    labels: labels!(
        "platform" => Vocab::Values(Platform::VALUES),
        "decision" => Vocab::Values(Decision::VALUES),
        "reason_class" => Vocab::Values(ReasonClass::VALUES),
    ),
};

/// Fired guardrails (page 112).
pub static GUARDRAIL_EVENTS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_guardrail_events_total",
    help: "Guardrails that fired, by what the guard did.",
    kind: MetricKind::Counter,
    labels: labels!(
        "platform" => Vocab::Values(Platform::VALUES),
        "guardrail" => Vocab::Values(Guardrail::VALUES),
        "action" => Vocab::Values(GuardrailAction::VALUES),
    ),
};

/// Restore operations (page 112): requested, confirmed, failed, reconciled.
pub static RESTORE_OPERATIONS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_restore_operations_total",
    help: "Restore and reconcile operations by outcome.",
    kind: MetricKind::Counter,
    labels: labels!(
        "platform" => Vocab::Values(Platform::VALUES),
        "operation" => Vocab::Values(RestoreOperation::VALUES),
        "outcome" => Vocab::Values(RestoreOutcome::VALUES),
    ),
};

/// Ledger verification results (page 112).
pub static LEDGER_VERIFICATIONS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_ledger_verifications_total",
    help: "Ledger verification results by retained verifier release.",
    kind: MetricKind::Counter,
    labels: labels!(
        "result" => Vocab::Values(LedgerResult::VALUES),
        "verifier_version" => Vocab::Values(VerifierVersion::VALUES),
    ),
};

/// Payment provider events (page 112). No PSP reference label.
pub static PAYMENT_EVENTS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_payment_events_total",
    help: "Provider payment events by class and processing result.",
    kind: MetricKind::Counter,
    labels: labels!(
        "provider" => Vocab::Values(PaymentProvider::VALUES),
        "event_class" => Vocab::Values(PaymentEventClass::VALUES),
        "result" => Vocab::Values(PaymentResult::VALUES),
    ),
};

/// Notification delivery attempts (page 112).
pub static NOTIFICATION_ATTEMPTS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_notification_attempts_total",
    help: "Notification delivery attempts, provider acceptance and delivery.",
    kind: MetricKind::Counter,
    labels: labels!(
        "channel" => Vocab::Values(Channel::VALUES),
        "template_class" => Vocab::Values(TemplateClass::VALUES),
        "outcome" => Vocab::Values(NotificationOutcome::VALUES),
    ),
};

/// Operator actions (page 112); actor and target IDs stay in the restricted
/// action journal.
pub static OPERATOR_ACTIONS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_operator_actions_total",
    help: "Operator capabilities exercised by action class and outcome.",
    kind: MetricKind::Counter,
    labels: labels!(
        "capability_class" => Vocab::Values(CapabilityClass::VALUES),
        "action_class" => Vocab::Values(ActionClass::VALUES),
        "outcome" => Vocab::Values(Outcome::VALUES),
    ),
};

/// Every declared metric, for the monitoring workstream and for tests that
/// pin name/label shape.
pub const CATALOG: &[&MetricSpec] = &[
    &HTTP_REQUESTS_TOTAL,
    &HTTP_REQUEST_DURATION_SECONDS,
    &HTTP_REQUESTS_IN_FLIGHT,
    &CONNECTOR_REQUESTS_TOTAL,
    &CONNECTOR_DURATION_SECONDS,
    &JOB_RUNS_TOTAL,
    &JOB_DURATION_SECONDS,
    &JOB_SCHEDULE_LAG_SECONDS,
    &JOBS_OVERDUE,
    &PROJECTION_LAG_SECONDS,
    &REPORT_ACCOUNTS,
    &AUDIT_RUNS_TOTAL,
    &EXPERIMENT_DECISIONS_TOTAL,
    &GUARDRAIL_EVENTS_TOTAL,
    &RESTORE_OPERATIONS_TOTAL,
    &LEDGER_VERIFICATIONS_TOTAL,
    &PAYMENT_EVENTS_TOTAL,
    &NOTIFICATION_ATTEMPTS_TOTAL,
    &OPERATOR_ACTIONS_TOTAL,
];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_names_are_unique_and_spec_shaped() {
        let mut names = BTreeSet::new();
        for metric in CATALOG {
            assert!(metric.name.starts_with("apex_"), "{}", metric.name);
            assert!(names.insert(metric.name), "duplicate {}", metric.name);
            assert!(!metric.help.is_empty(), "{} has no help", metric.name);
            let mut labels = BTreeSet::new();
            for label in metric.labels {
                assert!(labels.insert(label.name), "duplicate label {}", label.name);
            }
            if let Some(buckets) = metric.buckets() {
                assert!(!buckets.is_empty(), "{} has no buckets", metric.name);
                for pair in buckets.windows(2) {
                    assert!(pair[0] < pair[1], "{} buckets unsorted", metric.name);
                }
            }
        }
    }

    #[test]
    fn the_design_named_metrics_are_all_declared() {
        let names: BTreeSet<&str> = CATALOG.iter().map(|m| m.name).collect();
        for expected in [
            "apex_http_requests_total",
            "apex_http_request_duration_seconds",
            "apex_http_requests_in_flight",
            "apex_connector_requests_total",
            "apex_connector_duration_seconds",
            "apex_job_runs_total",
            "apex_job_duration_seconds",
            "apex_job_schedule_lag_seconds",
            "apex_projection_lag_seconds",
            "apex_report_accounts",
            "apex_audit_runs_total",
            "apex_experiment_decisions_total",
            "apex_guardrail_events_total",
            "apex_restore_operations_total",
            "apex_ledger_verifications_total",
            "apex_payment_events_total",
            "apex_notification_attempts_total",
            "apex_operator_actions_total",
        ] {
            assert!(names.contains(expected), "{expected} missing");
        }
    }
}
