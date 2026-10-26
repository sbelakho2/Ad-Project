//! Runner-side telemetry through the bounded `apex-metrics` registry.
//!
//! Section 06 page 111: `apex_job_runs_total` / `apex_job_duration_seconds`
//! carry `(job_type, platform, outcome)` and "job IDs belong in durable
//! events", never in labels. Page 112 names the domain series this module
//! writes: report-account readiness, audit runs, experiment decisions,
//! guardrail events, restore operations, ledger verifications and payment
//! events.
//!
//! `apex_job_schedule_lag_seconds` is written when the invocation knows its
//! expected schedule: `--scheduled-at <unix-ms>` on the command line, or the
//! `APEX_SCHEDULED_AT_MS` fallback the timer exports. The CLI dispatch
//! computes the lag (actual start minus scheduled, clamped at zero) for the
//! job it is about to run; a timer that passes neither records no sample
//! rather than a fabricated one. No in-repo scheduler exists, so
//! `apex_jobs_overdue` is declared in the catalog and never written here
//! (documented for the scheduler workstream).
//!
//! `apex_report_accounts` carries the readiness classes the daily pull pass
//! observed: `final` (data covering the last closed day), `partial` (data
//! stopping earlier or no rows), `stale` (the pull failed for a non-auth
//! reason) and `disconnected` (the pull was refused on auth). Each attempted
//! account lands in exactly one class; accounts the pass never attempted are
//! unknown to it and no class is written when nothing was attempted.

use std::sync::Arc;
use std::time::Instant;

use apex_metrics::catalog;
use apex_metrics::spec::MetricSpec;
use apex_metrics::vocab::{
    Decision, Guardrail, GuardrailAction, JobType, LedgerResult, Outcome, PaymentEventClass,
    PaymentResult, ProjectionName, ReadinessClass, ReasonClass, RestoreOperation, RestoreOutcome,
};
use apex_metrics::Registry;

/// The platform label every runner series carries (the runner serves the
/// AppLovin MAX adapter today).
pub const RUNNER_PLATFORM: &str = "applovin_max";

/// Writes the runner's domain series into one registry.
#[derive(Debug, Clone)]
pub struct RunnerTelemetry {
    registry: Arc<Registry>,
}

impl RunnerTelemetry {
    /// Wrap a registry.
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// The registry behind this handle.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Record a finished job: the runs counter and the duration histogram.
    pub fn job_finished(&self, job: JobType, outcome: Outcome, started: Instant) {
        self.count(
            &catalog::JOB_RUNS_TOTAL,
            &[
                ("job_type", job.as_str()),
                ("platform", RUNNER_PLATFORM),
                ("outcome", outcome.as_str()),
            ],
        );
        self.observe(
            &catalog::JOB_DURATION_SECONDS,
            &[
                ("job_type", job.as_str()),
                ("platform", RUNNER_PLATFORM),
                ("outcome", outcome.as_str()),
            ],
            started.elapsed().as_secs_f64(),
        );
    }

    /// Record how late a job started against its expected schedule.
    pub fn schedule_lag(&self, job: JobType, lag_seconds: f64) {
        self.observe(
            &catalog::JOB_SCHEDULE_LAG_SECONDS,
            &[("job_type", job.as_str()), ("platform", RUNNER_PLATFORM)],
            lag_seconds,
        );
    }

    /// Set the projection lag for one projection: the newest source event
    /// time minus the projected watermark, in seconds (page 111).
    pub fn projection_lag(&self, projection: ProjectionName, environment: &str, lag_seconds: f64) {
        let Some(environment) = apex_metrics::vocab::Environment::parse(environment) else {
            return;
        };
        let _ = self.registry.gauge_set(
            &catalog::PROJECTION_LAG_SECONDS,
            &[
                ("projection_name", projection.as_str()),
                ("environment", environment.as_str()),
            ],
            lag_seconds,
        );
    }

    /// Set the account count for one readiness class (final, partial, stale,
    /// disconnected).
    pub fn report_accounts(&self, readiness: ReadinessClass, count: f64) {
        let _ = self.registry.gauge_set(
            &catalog::REPORT_ACCOUNTS,
            &[
                ("platform", RUNNER_PLATFORM),
                ("readiness_class", readiness.as_str()),
            ],
            count,
        );
    }

    /// An audit execution outcome.
    pub fn audit_run(&self, rule_version: &str, outcome: Outcome) {
        let Some(rule_version) = apex_metrics::vocab::RuleVersion::parse(rule_version) else {
            return;
        };
        self.count(
            &catalog::AUDIT_RUNS_TOTAL,
            &[
                ("platform", RUNNER_PLATFORM),
                ("rule_version", rule_version.as_str()),
                ("outcome", outcome.as_str()),
            ],
        );
    }

    /// One experiment decision.
    pub fn decision(&self, decision: Decision, reason: ReasonClass) {
        self.count(
            &catalog::EXPERIMENT_DECISIONS_TOTAL,
            &[
                ("platform", RUNNER_PLATFORM),
                ("decision", decision.as_str()),
                ("reason_class", reason.as_str()),
            ],
        );
    }

    /// One fired guardrail and what it did.
    pub fn guardrail(&self, guardrail: Guardrail, action: GuardrailAction) {
        self.count(
            &catalog::GUARDRAIL_EVENTS_TOTAL,
            &[
                ("platform", RUNNER_PLATFORM),
                ("guardrail", guardrail.as_str()),
                ("action", action.as_str()),
            ],
        );
    }

    /// One restore operation outcome.
    pub fn restore(&self, operation: RestoreOperation, outcome: RestoreOutcome) {
        self.count(
            &catalog::RESTORE_OPERATIONS_TOTAL,
            &[
                ("platform", RUNNER_PLATFORM),
                ("operation", operation.as_str()),
                ("outcome", outcome.as_str()),
            ],
        );
    }

    /// One ledger verification result.
    pub fn ledger(&self, result: LedgerResult, verifier_version: &str) {
        let Some(verifier_version) = apex_metrics::vocab::VerifierVersion::parse(verifier_version)
        else {
            return;
        };
        self.count(
            &catalog::LEDGER_VERIFICATIONS_TOTAL,
            &[
                ("result", result.as_str()),
                ("verifier_version", verifier_version.as_str()),
            ],
        );
    }

    /// One processed provider payment event.
    pub fn payment(&self, class: PaymentEventClass, result: PaymentResult) {
        self.count(
            &catalog::PAYMENT_EVENTS_TOTAL,
            &[
                ("provider", "mollie"),
                ("event_class", class.as_str()),
                ("result", result.as_str()),
            ],
        );
    }

    /// One consumed operator job, by capability and action class.
    pub fn operator_action(
        &self,
        capability: apex_metrics::vocab::CapabilityClass,
        action: apex_metrics::vocab::ActionClass,
        outcome: Outcome,
    ) {
        self.count(
            &catalog::OPERATOR_ACTIONS_TOTAL,
            &[
                ("capability_class", capability.as_str()),
                ("action_class", action.as_str()),
                ("outcome", outcome.as_str()),
            ],
        );
    }

    fn count(&self, metric: &'static MetricSpec, labels: &[(&'static str, &str)]) {
        // Declared values cannot be refused by the registry; a failed write is
        // impossible by construction and the analytics store holds the facts.
        let _ = self.registry.increment(metric, labels);
    }

    fn observe(&self, metric: &'static MetricSpec, labels: &[(&'static str, &str)], value: f64) {
        let _ = self.registry.observe(metric, labels, value);
    }
}

/// The bounded reason class of a fired guardrail.
pub fn reason_class_of(guardrail: Guardrail) -> ReasonClass {
    match guardrail {
        Guardrail::FastHarm | Guardrail::ConfirmedHarm => ReasonClass::Harm,
        Guardrail::FillRewardedInterstitial | Guardrail::FillBanner => ReasonClass::Fill,
        Guardrail::PublisherEdit => ReasonClass::PublisherEdit,
        Guardrail::SplitMismatch => ReasonClass::SplitMismatch,
        Guardrail::PlatformIncident => ReasonClass::PlatformIncident,
        Guardrail::ReportingGap => ReasonClass::ReportingGap,
    }
}

/// Map one engine guardrail event to the bounded guardrail vocabulary.
pub fn guardrail_of(event: &apex_lift::guardrails::GuardrailEvent) -> Guardrail {
    use apex_lift::guardrails::GuardrailEvent;
    match event {
        GuardrailEvent::FastHarm { .. } => Guardrail::FastHarm,
        GuardrailEvent::ConfirmedHarm { .. } => Guardrail::ConfirmedHarm,
        GuardrailEvent::FillRewardedInterstitial { .. } => Guardrail::FillRewardedInterstitial,
        GuardrailEvent::FillBanner { .. } => Guardrail::FillBanner,
        GuardrailEvent::PublisherEdit { .. } => Guardrail::PublisherEdit,
        GuardrailEvent::SplitMismatch { .. } => Guardrail::SplitMismatch,
        GuardrailEvent::ReportingGap { .. } => Guardrail::ReportingGap,
        GuardrailEvent::PlatformIncident { .. } => Guardrail::PlatformIncident,
    }
}

/// What a fired guardrail did, for the metric's bounded `action` label.
pub fn guardrail_action(event: &apex_lift::guardrails::GuardrailEvent) -> GuardrailAction {
    if event.voids() {
        GuardrailAction::Void
    } else if event.rolls_back() {
        GuardrailAction::Rollback
    } else {
        GuardrailAction::Pause
    }
}

/// The bounded reason class of a decision's free reason text, derived from
/// the guardrails that fired (never by parsing free text).
pub fn decision_reason_class(
    decision: Decision,
    guardrails: &[apex_lift::guardrails::GuardrailEvent],
) -> ReasonClass {
    match decision {
        Decision::Keep => ReasonClass::BoundPositive,
        Decision::Pending | Decision::Extend => ReasonClass::WindowIncomplete,
        Decision::Rollback | Decision::Void => guardrails
            .first()
            .map(|event| reason_class_of(guardrail_of(event)))
            .unwrap_or(ReasonClass::Harm),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use apex_metrics::vocab::VerifierVersion;

    #[test]
    fn job_runs_and_durations_are_recorded_with_bounded_labels() {
        let registry = Arc::new(Registry::new());
        let telemetry = RunnerTelemetry::new(registry.clone());
        let started = Instant::now();
        telemetry.job_finished(JobType::Daily, Outcome::Completed, started);
        telemetry.schedule_lag(JobType::Daily, 12.0);
        let runs = registry
            .counter_value(
                &catalog::JOB_RUNS_TOTAL,
                &[
                    ("job_type", "daily"),
                    ("platform", "applovin_max"),
                    ("outcome", "completed"),
                ],
            )
            .expect("counter");
        assert_eq!(runs, 1);
        let family = registry
            .family_snapshot("apex_job_duration_seconds")
            .expect("duration family");
        let histogram = family.series[0].histogram.as_ref().expect("histogram");
        assert_eq!(histogram.count, 1);
        assert!(registry
            .family_snapshot("apex_job_schedule_lag_seconds")
            .is_some());
    }

    #[test]
    fn domain_series_are_written_with_bounded_labels() {
        let registry = Arc::new(Registry::new());
        let telemetry = RunnerTelemetry::new(registry.clone());
        telemetry.decision(Decision::Keep, ReasonClass::BoundPositive);
        telemetry.guardrail(Guardrail::FastHarm, GuardrailAction::Rollback);
        telemetry.restore(RestoreOperation::Restore, RestoreOutcome::Requested);
        telemetry.restore(RestoreOperation::Restore, RestoreOutcome::Failed);
        telemetry.ledger(LedgerResult::Verified, VerifierVersion::V0_1_0.as_str());
        telemetry.ledger(LedgerResult::InvalidChain, VerifierVersion::V0_1_0.as_str());
        telemetry.payment(PaymentEventClass::Authorisation, PaymentResult::Accepted);
        telemetry.audit_run("v0.1.0", Outcome::Completed);
        telemetry.report_accounts(ReadinessClass::Final, 3.0);
        // An unknown rule version and verifier version are refused, not
        // minted as new labels.
        telemetry.audit_run("v9.9.9", Outcome::Completed);
        telemetry.ledger(LedgerResult::Verified, "v9");
        assert_eq!(
            registry
                .counter_value(
                    &catalog::AUDIT_RUNS_TOTAL,
                    &[
                        ("platform", "applovin_max"),
                        ("rule_version", "v0.1.0"),
                        ("outcome", "completed"),
                    ],
                )
                .unwrap(),
            1
        );
        assert_eq!(
            registry
                .gauge_value(
                    &catalog::REPORT_ACCOUNTS,
                    &[("platform", "applovin_max"), ("readiness_class", "final")],
                )
                .unwrap(),
            3.0
        );
        assert_eq!(
            registry
                .counter_value(
                    &catalog::LEDGER_VERIFICATIONS_TOTAL,
                    &[("result", "verified"), ("verifier_version", "v0.1.0")],
                )
                .unwrap(),
            1
        );
        let family = registry
            .family_snapshot("apex_ledger_verifications_total")
            .unwrap();
        assert_eq!(family.series.len(), 2, "no v9 series was minted");
    }

    #[test]
    fn guardrail_mapping_is_total_and_specific() {
        use apex_lift::guardrails::GuardrailEvent;
        let events = [
            GuardrailEvent::FastHarm {
                day: "2026-10-01".into(),
                point_lift: -0.1,
            },
            GuardrailEvent::ConfirmedHarm {
                day: "2026-10-01".into(),
                upper_bound: -0.02,
            },
            GuardrailEvent::FillRewardedInterstitial {
                day: "2026-10-01".into(),
                delta: -0.1,
            },
            GuardrailEvent::FillBanner {
                day: "2026-10-01".into(),
                delta: -0.1,
            },
            GuardrailEvent::PublisherEdit {
                day: "2026-10-01".into(),
            },
            GuardrailEvent::SplitMismatch {
                day: "2026-10-01".into(),
                chi_square: 20.0,
            },
            GuardrailEvent::ReportingGap {
                day: "2026-10-01".into(),
                stale_days: 1,
            },
            GuardrailEvent::PlatformIncident {
                day: "2026-10-01".into(),
            },
        ];
        for event in &events {
            let guardrail = guardrail_of(event);
            assert_eq!(guardrail_of(event), guardrail);
            assert!(!guardrail.as_str().is_empty());
            assert!(!reason_class_of(guardrail).as_str().is_empty());
        }
        assert_eq!(guardrail_action(&events[0]), GuardrailAction::Rollback);
        assert_eq!(guardrail_action(&events[4]), GuardrailAction::Void);
        assert_eq!(guardrail_action(&events[6]), GuardrailAction::Pause);
        assert_eq!(
            decision_reason_class(Decision::Keep, &[]),
            ReasonClass::BoundPositive
        );
        assert_eq!(
            decision_reason_class(Decision::Pending, &[]),
            ReasonClass::WindowIncomplete
        );
        assert_eq!(
            decision_reason_class(Decision::Rollback, &events),
            ReasonClass::Harm
        );
        assert_eq!(
            decision_reason_class(Decision::Void, &[]),
            ReasonClass::Harm
        );
    }
}
