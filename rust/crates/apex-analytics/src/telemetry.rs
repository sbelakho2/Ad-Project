//! Telemetry names from the implementation chapter (section 06, pages 111
//! and 112): every proposed metric name as a constant, so instrumentation
//! and dashboards reference one spelling.
//!
//! This module carries no metrics dependency on purpose: the crate only
//! publishes the names, the emitting service wires its own registry. Names
//! and bounded label sets are the spec's; per-customer facts stay in the
//! analytics store, not in metric labels.

/// Requests by service, route template, method and status class.
pub const HTTP_REQUESTS_TOTAL: &str = "apex_http_requests_total";
/// Server response duration histogram.
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "apex_http_request_duration_seconds";
/// Requests currently in progress.
pub const HTTP_REQUESTS_IN_FLIGHT: &str = "apex_http_requests_in_flight";
/// Platform connector requests by platform, operation and outcome class.
pub const CONNECTOR_REQUESTS_TOTAL: &str = "apex_connector_requests_total";
/// External connector call duration histogram.
pub const CONNECTOR_DURATION_SECONDS: &str = "apex_connector_duration_seconds";
/// Worker job runs by job type, platform and outcome.
pub const JOB_RUNS_TOTAL: &str = "apex_job_runs_total";
/// Worker job duration histogram.
pub const JOB_DURATION_SECONDS: &str = "apex_job_duration_seconds";
/// Actual start minus expected schedule, per job type and platform.
pub const JOB_SCHEDULE_LAG_SECONDS: &str = "apex_job_schedule_lag_seconds";
/// Latest source event time minus projected watermark, per projection and
/// environment.
pub const PROJECTION_LAG_SECONDS: &str = "apex_projection_lag_seconds";
/// Account counts by platform and readiness class (Final, Partial, Stale,
/// Disconnected).
pub const REPORT_ACCOUNTS: &str = "apex_report_accounts";
/// Audit execution outcome by platform, retained rule version and outcome.
pub const AUDIT_RUNS_TOTAL: &str = "apex_audit_runs_total";
/// Experiment decisions by platform, decision and reason class.
pub const EXPERIMENT_DECISIONS_TOTAL: &str = "apex_experiment_decisions_total";
/// Fired guardrails by platform, guardrail and action.
pub const GUARDRAIL_EVENTS_TOTAL: &str = "apex_guardrail_events_total";
/// Restore operations by platform, operation and outcome.
pub const RESTORE_OPERATIONS_TOTAL: &str = "apex_restore_operations_total";
/// Ledger verifications by result and verifier version.
pub const LEDGER_VERIFICATIONS_TOTAL: &str = "apex_ledger_verifications_total";
/// Payment provider events by provider, event class and result.
pub const PAYMENT_EVENTS_TOTAL: &str = "apex_payment_events_total";
/// Notification attempts by channel, template class and outcome.
pub const NOTIFICATION_ATTEMPTS_TOTAL: &str = "apex_notification_attempts_total";
/// Operator actions by capability class, action class and outcome.
pub const OPERATOR_ACTIONS_TOTAL: &str = "apex_operator_actions_total";

/// Every telemetry name, for registration and dashboard wiring.
pub const ALL: [&str; 18] = [
    HTTP_REQUESTS_TOTAL,
    HTTP_REQUEST_DURATION_SECONDS,
    HTTP_REQUESTS_IN_FLIGHT,
    CONNECTOR_REQUESTS_TOTAL,
    CONNECTOR_DURATION_SECONDS,
    JOB_RUNS_TOTAL,
    JOB_DURATION_SECONDS,
    JOB_SCHEDULE_LAG_SECONDS,
    PROJECTION_LAG_SECONDS,
    REPORT_ACCOUNTS,
    AUDIT_RUNS_TOTAL,
    EXPERIMENT_DECISIONS_TOTAL,
    GUARDRAIL_EVENTS_TOTAL,
    RESTORE_OPERATIONS_TOTAL,
    LEDGER_VERIFICATIONS_TOTAL,
    PAYMENT_EVENTS_TOTAL,
    NOTIFICATION_ATTEMPTS_TOTAL,
    OPERATOR_ACTIONS_TOTAL,
];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_spec_names_are_exact() {
        // Pinned char-for-char against the telemetry tables of section 06.
        let expected = [
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
        ];
        assert_eq!(ALL, expected);
        assert_eq!(HTTP_REQUESTS_TOTAL, expected[0]);
        assert_eq!(OPERATOR_ACTIONS_TOTAL, expected[17]);
    }

    #[test]
    fn every_name_is_prefixed_and_unique() {
        for name in ALL {
            assert!(name.starts_with("apex_"), "{name}");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{name} must be a Prometheus-safe name"
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in ALL {
            assert!(seen.insert(name), "duplicate telemetry name {name}");
        }
    }
}
