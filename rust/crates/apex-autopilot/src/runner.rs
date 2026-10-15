//! The scheduled job runner: the daily measurement pass and the monthly
//! billing pass over a durable ledger.
//!
//! Nothing here is a daemon; each method is one idempotent job an operator (or
//! a systemd timer) invokes on a schedule. The daily pass records the raw
//! platform pull, settles every experiment whose window is complete, and
//! leaves running experiments untouched. The monthly pass bills each kept
//! change for the month from the same signed ledger.

use std::sync::Arc;

use apex_lift::guardrails::FillFormat;
use apex_metrics::vocab::{Decision, RestoreOperation, RestoreOutcome};
use apex_proof::{ChangeKind, LedgerLog, LedgerRecord};

use crate::alerts::{Alert, AlertSink, NoopAlertSink};
use crate::model::{AutopilotPolicy, PlanError, SettleOutcome};
use crate::ops::PlatformOps;
use crate::settle::settle_experiment;

/// What one daily pass did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DailyReport {
    /// Experiments whose reporting was pulled (one data-pull record each).
    pub pulled: u32,
    /// Experiments settled this pass.
    pub settled: u32,
    /// Samples observed as still pending this pass.
    pub pending: u32,
    /// Reporting pulls that failed this pass.
    pub pull_failures: u32,
    /// Settle outcomes in ledger order (experiment id, status label).
    pub outcomes: Vec<(String, String)>,
    /// Experiments that could not be processed (id, reason). The pass
    /// continues past a failure so one bad test never starves the rest.
    pub failures: Vec<(String, String)>,
    /// Operator jobs consumed this pass, when the jobs directory is
    /// configured.
    pub operator_jobs: Option<crate::operator_jobs::OperatorJobsReport>,
    /// Payment events processed this pass, when the collection file is
    /// configured.
    pub payments: Option<crate::payment_events::PaymentsReport>,
    /// Projection refresh outcome, when the analytics store is configured.
    pub projections: Option<crate::projections_job::ProjectionRefreshReport>,
    /// Why the projection refresh failed, when it did; the failure stays
    /// visible and the pass reports it instead of hiding it.
    pub projection_failure: Option<String>,
}

/// The account-readiness counts one daily pull pass observed, mapped to the
/// bounded readiness vocabulary. Every account the pass attempted lands in
/// exactly one class:
///
/// - final: the pull returned data covering the last closed day,
/// - partial: the pull returned data, but only up to an earlier day (the
///   window is not fully reported yet),
/// - disconnected: the pull was refused on auth (the credential cannot read),
/// - stale: the pull failed for any other reason, so the account was not
///   refreshed this pass.
///
/// Accounts the pass never attempted are unknown to it and are not counted;
/// the gauges are only written when at least one account was attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PullReadiness {
    final_accounts: u32,
    partial_accounts: u32,
    stale_accounts: u32,
    disconnected_accounts: u32,
}

impl PullReadiness {
    /// How many accounts the pass attempted.
    fn attempted(&self) -> u32 {
        self.final_accounts
            .saturating_add(self.partial_accounts)
            .saturating_add(self.stale_accounts)
            .saturating_add(self.disconnected_accounts)
    }
}

/// The runner holds the platform adapter and the policy; the ledger is passed
/// per job so the same runner serves any publisher's ledger.
#[derive(Clone)]
pub struct Runner {
    ops: Arc<dyn PlatformOps>,
    policy: AutopilotPolicy,
    fill_format: FillFormat,
    alerts: Arc<dyn AlertSink>,
    /// The publisher this runner bills; required for the monthly pass.
    publisher_id: String,
    /// The vat rate applied to invoice lines (basis points, 0 = none).
    vat_rate_bp: u16,
    /// The optional flat monitoring plan, billed as its own invoice line.
    monitoring: Option<apex_billing::MonitoringPlan>,
    /// The Adyen collection path: client plus transport. Absent in dry-run
    /// deployments, which record billing but never collect.
    adyen: Option<(
        Arc<apex_billing::AdyenClient>,
        Arc<dyn apex_connectors::transport::Transport>,
    )>,
    /// The bucket model and where it persists; learning follows results.
    priors: Option<apex_lift::priors::LiftPriors>,
    priors_path: Option<std::path::PathBuf>,
    /// The bounded telemetry registry handle, when configured.
    telemetry: Option<crate::telemetry::RunnerTelemetry>,
    /// The analytics emitter, when configured.
    events: Option<Arc<crate::events::RunnerEvents>>,
    /// The notification plan, when contacts are configured.
    notify: Option<Arc<crate::notify::NotifyPlan>>,
    /// The operator-jobs directory consumed by the daily pass.
    operator_jobs_dir: Option<std::path::PathBuf>,
    /// The verified provider-event collection file.
    payment_events_path: Option<std::path::PathBuf>,
    /// The consumed-offset cursor beside the collection file; `None` means the
    /// default `<collection>.cursor` next to it.
    payment_cursor_path: Option<std::path::PathBuf>,
    /// The collected-reference ledger that payment messages read balances
    /// from; rows are appended when collection runs.
    payment_ledger_path: Option<std::path::PathBuf>,
    /// The scheduled-retry file written by the finance boundary.
    payment_retries_path: Option<std::path::PathBuf>,
    /// The ledger verifier used for the run's verification line and events.
    verifier: Option<Arc<dyn crate::verify::LedgerVerifier>>,
    /// The daily projection refresh, when the analytics store is configured.
    projections: Option<crate::projections_job::RunnerProjections>,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner")
            .field("policy", &self.policy)
            .finish()
    }
}

impl Runner {
    pub fn new(
        ops: Arc<dyn PlatformOps>,
        policy: AutopilotPolicy,
        fill_format: FillFormat,
    ) -> Self {
        Self {
            ops,
            policy,
            fill_format,
            alerts: Arc::new(NoopAlertSink),
            publisher_id: String::new(),
            vat_rate_bp: 0,
            monitoring: None,
            adyen: None,
            priors: None,
            priors_path: None,
            telemetry: None,
            events: None,
            projections: None,
            notify: None,
            operator_jobs_dir: None,
            payment_events_path: None,
            payment_cursor_path: None,
            payment_ledger_path: None,
            payment_retries_path: None,
            verifier: None,
        }
    }

    /// The publisher this runner bills (required for the monthly pass).
    #[must_use]
    pub fn with_publisher(mut self, publisher_id: impl Into<String>) -> Self {
        self.publisher_id = publisher_id.into();
        self
    }

    /// The vat rate applied to invoice lines, in basis points.
    #[must_use]
    pub fn with_vat_rate(mut self, vat_rate_bp: u16) -> Self {
        self.vat_rate_bp = vat_rate_bp;
        self
    }

    /// Bill the flat monitoring plan as its own invoice line.
    #[must_use]
    pub fn with_monitoring(mut self, plan: apex_billing::MonitoringPlan) -> Self {
        self.monitoring = Some(plan);
        self
    }

    /// Attach the Adyen collection path (client + transport).
    #[must_use]
    pub fn with_adyen(
        mut self,
        client: Arc<apex_billing::AdyenClient>,
        transport: Arc<dyn apex_connectors::transport::Transport>,
    ) -> Self {
        self.adyen = Some((client, transport));
        self
    }

    /// Load the bucket model and persist updates to `path` after every
    /// settled test, so the next cycle's proposals learn from results.
    #[must_use]
    pub fn with_priors(
        mut self,
        priors: apex_lift::priors::LiftPriors,
        path: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.priors = Some(priors);
        self.priors_path = Some(path.into());
        self
    }

    /// Attach the alert sink that receives customer-impact alerts.
    #[must_use]
    pub fn with_alerts(mut self, alerts: Arc<dyn AlertSink>) -> Self {
        self.alerts = alerts;
        self
    }

    /// Attach the bounded telemetry registry handle.
    #[must_use]
    pub fn with_telemetry(mut self, telemetry: crate::telemetry::RunnerTelemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    /// Attach the analytics emitter.
    #[must_use]
    pub fn with_events(mut self, events: Arc<crate::events::RunnerEvents>) -> Self {
        self.events = Some(events);
        self
    }

    /// Attach the notification plan.
    #[must_use]
    pub fn with_notifications(mut self, notify: Arc<crate::notify::NotifyPlan>) -> Self {
        self.notify = Some(notify);
        self
    }

    /// Refresh the analytics projections at the end of the daily pass.
    #[must_use]
    pub fn with_projections(
        mut self,
        projections: crate::projections_job::RunnerProjections,
    ) -> Self {
        self.projections = Some(projections);
        self
    }

    /// Consume operator jobs from `dir` in the daily pass.
    #[must_use]
    pub fn with_operator_jobs(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.operator_jobs_dir = Some(dir.into());
        self
    }

    /// Process verified payment events from `events` (the collection file),
    /// resolving received-message balances from `ledger` and reading
    /// scheduled retries from `retries`.
    #[must_use]
    pub fn with_payment_events(
        mut self,
        events: impl Into<std::path::PathBuf>,
        ledger: Option<std::path::PathBuf>,
        retries: Option<std::path::PathBuf>,
    ) -> Self {
        self.payment_events_path = Some(events.into());
        self.payment_ledger_path = ledger;
        self.payment_retries_path = retries;
        self
    }

    /// Override the consumed-offset cursor file for the payment collection
    /// (`APEX_PAYMENT_CURSOR`); the default is `<collection>.cursor` beside
    /// the collection file.
    #[must_use]
    pub fn with_payment_cursor(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.payment_cursor_path = Some(path.into());
        self
    }

    /// Attach the ledger verifier used for the run's verification line and
    /// the `ledger_verified` events.
    #[must_use]
    pub fn with_verifier(mut self, verifier: Arc<dyn crate::verify::LedgerVerifier>) -> Self {
        self.verifier = Some(verifier);
        self
    }

    /// The telemetry handle, when configured.
    pub fn telemetry(&self) -> Option<&crate::telemetry::RunnerTelemetry> {
        self.telemetry.as_ref()
    }

    /// The analytics emitter, when configured.
    pub fn events(&self) -> Option<&Arc<crate::events::RunnerEvents>> {
        self.events.as_ref()
    }

    /// The notification plan, when configured.
    pub fn notify(&self) -> Option<&Arc<crate::notify::NotifyPlan>> {
        self.notify.as_ref()
    }

    /// The ledger verifier, when configured.
    pub fn verifier(&self) -> Option<&Arc<dyn crate::verify::LedgerVerifier>> {
        self.verifier.as_ref()
    }

    /// One daily pass: pull reporting for every running experiment, record the
    /// pull, and settle any whose window has completed. A failure on one
    /// experiment is recorded (and alerted) without starving the rest.
    ///
    /// The pass also consumes operator jobs and verified payment events when
    /// they are configured, and every pull, decision, guardrail and restore
    /// leaves a durable analytics event beside its ledger record.
    pub async fn run_daily(
        &self,
        ledger: &mut dyn LedgerLog,
        now_unix_ms: i64,
    ) -> Result<DailyReport, PlanError> {
        let mut report = DailyReport::default();
        let job_id = crate::cli::run_reference(now_unix_ms);
        let today = now_unix_ms.div_euclid(86_400_000);
        let range_end = crate::calendar::format_day(today - 1);

        // Operator jobs first: a requested re-read or reconcile is a
        // read-only recovery step and never blocks the measurement pass.
        if let Some(dir) = &self.operator_jobs_dir {
            // The due set is computed once per pass and reused for every
            // recheck_request job: the ledger scan must not repeat per job.
            let due_set = self.due_rechecks(ledger, now_unix_ms);
            let due = |change_id: &str| due_set.iter().any(|due| due.change_id == change_id);
            let jobs = crate::operator_jobs::consume_operator_jobs(
                dir,
                self.ops.as_ref(),
                &due,
                self.notify.as_deref(),
                self.telemetry.as_ref(),
                &self.publisher_id,
                now_unix_ms,
            )
            .await
            .map_err(|error| PlanError::Validation(error.to_string()))?;
            report.operator_jobs = Some(jobs);
        }

        // Verified provider events react into messages and analytics; the
        // ledger is untouched by payment processing. The pass consumes only
        // the collection file's new lines, through the consumed-offset
        // cursor, and advances the cursor atomically after processing: a
        // crash re-processes the last batch, which the dedup-keyed emits and
        // the delivery log absorb.
        if let Some(path) = &self.payment_events_path {
            let cursor_path = self
                .payment_cursor_path
                .clone()
                .unwrap_or_else(|| crate::payment_events::default_cursor_path(path));
            let offset = crate::payment_events::read_consumed_offset(&cursor_path)
                .map_err(|error| PlanError::Validation(error.to_string()))?;
            let (events, consumed) =
                crate::payment_events::read_new_collection_events(path, offset)
                    .map_err(|error| PlanError::Validation(error.to_string()))?;
            let payment_ledger = match &self.payment_ledger_path {
                Some(path) => crate::payment_events::read_payment_ledger(path)
                    .map_err(|error| PlanError::Validation(error.to_string()))?,
                None => Vec::new(),
            };
            let retries = match &self.payment_retries_path {
                Some(path) => crate::payment_events::read_scheduled_retries(path)
                    .map_err(|error| PlanError::Validation(error.to_string()))?,
                None => Vec::new(),
            };
            report.payments = Some(
                crate::payment_events::process_payment_events(
                    &events,
                    &payment_ledger,
                    &retries,
                    self.notify.as_deref(),
                    self.events.as_deref(),
                    self.telemetry.as_ref(),
                    &self.publisher_id,
                    now_unix_ms,
                )
                .await
                .map_err(|error| PlanError::Validation(error.to_string()))?,
            );
            // Only a pass that read new lines advances the cursor; a read
            // failure above left it untouched so the batch is retried.
            if consumed != offset {
                crate::payment_events::write_consumed_offset(&cursor_path, consumed)
                    .map_err(|error| PlanError::Validation(error.to_string()))?;
            }
        }

        let mut readiness = PullReadiness::default();
        for experiment in running_experiments(ledger) {
            let range_start =
                started_day(ledger, &experiment.experiment_id).unwrap_or_else(|| range_end.clone());
            let attempt = 1u32;
            if let Some(events) = self.events.as_ref() {
                events
                    .report_pull_started(
                        &job_id,
                        "experiment_days",
                        &experiment.experiment_id,
                        &range_start,
                        &range_end,
                        attempt,
                        now_unix_ms,
                    )
                    .await;
            }
            let pull_started = std::time::Instant::now();
            let days = match self.ops.experiment_days(&experiment.experiment_id).await {
                Ok(days) => days,
                Err(source) => {
                    if let Some(events) = self.events.as_ref() {
                        events
                            .report_pull_failed(
                                &job_id,
                                "experiment_days",
                                &experiment.experiment_id,
                                &range_start,
                                &range_end,
                                attempt,
                                error_class_of(&source),
                                now_unix_ms,
                            )
                            .await;
                    }
                    if let Some(notify) = self.notify.as_ref() {
                        if matches!(source, apex_core::PlatformError::Auth { .. }) {
                            // The design's reporting-access message exists for
                            // exactly this state: the credential was refused.
                            notify
                                .reporting_access(
                                    &self.publisher_id,
                                    &crate::calendar::format_day(today),
                                    now_unix_ms,
                                )
                                .await;
                        }
                    }
                    self.alert_pull_failure(&experiment.experiment_id, &source)
                        .await;
                    // The pull outcome is the readiness observation: an
                    // auth refusal means the credential is disconnected,
                    // any other failure leaves the account stale for now.
                    match source {
                        apex_core::PlatformError::Auth { .. } => {
                            readiness.disconnected_accounts += 1;
                        }
                        _ => readiness.stale_accounts += 1,
                    }
                    report.pull_failures += 1;
                    report
                        .failures
                        .push((experiment.experiment_id.clone(), source.to_string()));
                    continue;
                }
            };
            let rows = apex_lift::emit::day_rows(&days.control, &days.test);
            let day = days
                .control
                .last()
                .map(|d| d.day.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let actual_start = days
                .control
                .first()
                .or_else(|| days.test.first())
                .map(|d| d.day.clone())
                .unwrap_or_else(|| range_start.clone());
            let actual_end = days
                .control
                .last()
                .or_else(|| days.test.last())
                .map(|d| d.day.clone())
                .unwrap_or_else(|| range_end.clone());
            // Coverage readiness: the pull is final when it returned data
            // covering the last closed day; anything less (earlier coverage
            // or no reported rows) is partial.
            let reported_days = days.control.len() + days.test.len();
            if reported_days > 0 && actual_end.as_str() >= range_end.as_str() {
                readiness.final_accounts += 1;
            } else {
                readiness.partial_accounts += 1;
            }
            if let Some(events) = self.events.as_ref() {
                events
                    .report_pull_completed(
                        &job_id,
                        "experiment_days",
                        &experiment.experiment_id,
                        &actual_start,
                        &actual_end,
                        rows.as_array().map(Vec::len).unwrap_or(0) as u64,
                        "final",
                        pull_started.elapsed().as_millis() as u64,
                        now_unix_ms,
                    )
                    .await;
            }
            let pull = apex_lift::emit::data_pull(&day, &rows)
                .map_err(|error| PlanError::Validation(format!("data pull: {error}")))?;
            let payload = pull
                .record
                .canonical_json()
                .map_err(|error| PlanError::Validation(format!("data pull: {error}")))?;
            ledger
                .append(LedgerRecord::lift_record(
                    now_unix_ms,
                    &pull.sha256,
                    &payload,
                ))
                .map_err(PlanError::Ledger)?;
            report.pulled += 1;
            if let Some(events) = self.events.as_ref() {
                events
                    .experiment_pull(
                        &experiment.experiment_id,
                        &pull.sha256,
                        &actual_start,
                        &actual_end,
                        now_unix_ms,
                    )
                    .await;
            }

            let settings_changed = self.settings_changed(&experiment).await;
            let outcome = match settle_experiment(
                self.ops.as_ref(),
                ledger,
                &experiment.experiment_id,
                experiment.change_seq,
                self.fill_format,
                &settings_changed,
                now_unix_ms,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(PlanError::RollbackFailed { reason }) => {
                    self.alerts
                        .emit(Alert::FailedRollback {
                            experiment_id: experiment.experiment_id.clone(),
                            detail: reason.clone(),
                        })
                        .await;
                    report
                        .failures
                        .push((experiment.experiment_id.clone(), reason));
                    continue;
                }
                Err(other) => {
                    report
                        .failures
                        .push((experiment.experiment_id.clone(), other.to_string()));
                    continue;
                }
            };
            // A pending window is not a settlement; everything else is.
            if let SettleOutcome::Pending { .. } = &outcome {
                report.pending += 1;
                continue;
            }
            match &outcome {
                SettleOutcome::Kept { lift, lambda, .. } => {
                    self.emit_decision(
                        &experiment.experiment_id,
                        Decision::Keep,
                        &[],
                        Some((lift * 10_000.0).round() as i64),
                        Some((lambda * 10_000.0).round() as i64),
                        now_unix_ms,
                    )
                    .await;
                }
                SettleOutcome::RolledBack {
                    reason, guardrails, ..
                } => {
                    self.emit_decision(
                        &experiment.experiment_id,
                        Decision::Rollback,
                        guardrails,
                        None,
                        None,
                        now_unix_ms,
                    )
                    .await;
                    self.confirm_restore(&experiment.experiment_id, guardrails, now_unix_ms)
                        .await;
                    self.alerts
                        .emit(Alert::GuardrailStop {
                            experiment_id: experiment.experiment_id.clone(),
                            detail: reason.clone(),
                        })
                        .await;
                    if let Some(notify) = self.notify.as_ref() {
                        notify
                            .guardrail_stop(
                                &self.publisher_id,
                                &experiment.experiment_id,
                                now_unix_ms,
                            )
                            .await;
                    }
                }
                SettleOutcome::Voided { reason, guardrails } => {
                    self.emit_decision(
                        &experiment.experiment_id,
                        Decision::Void,
                        guardrails,
                        None,
                        None,
                        now_unix_ms,
                    )
                    .await;
                    self.confirm_restore(&experiment.experiment_id, guardrails, now_unix_ms)
                        .await;
                    self.alerts
                        .emit(Alert::GuardrailStop {
                            experiment_id: experiment.experiment_id.clone(),
                            detail: reason.clone(),
                        })
                        .await;
                    if let Some(notify) = self.notify.as_ref() {
                        notify
                            .guardrail_stop(
                                &self.publisher_id,
                                &experiment.experiment_id,
                                now_unix_ms,
                            )
                            .await;
                    }
                }
                SettleOutcome::Pending { .. } => {}
            }
            self.learn(&experiment, ledger, &outcome).await;
            report.settled += 1;
            report
                .outcomes
                .push((experiment.experiment_id.clone(), status_label(&outcome)));
        }

        // The readiness gauges carry exactly what the pass observed. When no
        // account was attempted, no class is known (and none is written):
        // a gauge is never fabricated from an absent observation.
        if readiness.attempted() > 0 {
            if let Some(telemetry) = self.telemetry.as_ref() {
                use apex_metrics::vocab::ReadinessClass;
                telemetry
                    .report_accounts(ReadinessClass::Final, f64::from(readiness.final_accounts));
                telemetry.report_accounts(
                    ReadinessClass::Partial,
                    f64::from(readiness.partial_accounts),
                );
                telemetry
                    .report_accounts(ReadinessClass::Stale, f64::from(readiness.stale_accounts));
                telemetry.report_accounts(
                    ReadinessClass::Disconnected,
                    f64::from(readiness.disconnected_accounts),
                );
            }
        }

        // The projection refresh runs last so the day's settle work is part
        // of the facts the projections are rebuilt from. A failure is
        // recorded and reported, never silently skipped.
        if let Some(projections) = &self.projections {
            let batch_id = crate::projections_job::daily_batch_id(now_unix_ms);
            match projections
                .refresh(&batch_id, self.telemetry.as_ref(), now_unix_ms)
                .await
            {
                Ok(refresh) => report.projections = Some(refresh),
                Err(error) => {
                    eprintln!("apex-runner: projection refresh failed: {error}");
                    report.projection_failure = Some(error);
                }
            }
        }

        Ok(report)
    }

    /// Record one terminal decision: the bounded metric, the
    /// `experiment_decided` event, and one `guardrail_fired` event per fired
    /// guardrail (with its bounded action).
    async fn emit_decision(
        &self,
        experiment_id: &str,
        decision: Decision,
        guardrails: &[apex_lift::guardrails::GuardrailEvent],
        point_estimate_bp: Option<i64>,
        decision_bound_bp: Option<i64>,
        now_unix_ms: i64,
    ) {
        let reason = crate::telemetry::decision_reason_class(decision, guardrails);
        if let Some(telemetry) = self.telemetry.as_ref() {
            telemetry.decision(decision, reason);
        }
        if let Some(events) = self.events.as_ref() {
            events
                .experiment_decided(
                    experiment_id,
                    decision.as_str(),
                    point_estimate_bp,
                    decision_bound_bp,
                    reason.as_str(),
                    now_unix_ms,
                )
                .await;
            for guardrail in guardrails {
                let bounded = crate::telemetry::guardrail_of(guardrail);
                events
                    .guardrail_fired(
                        bounded.as_str(),
                        guardrail_input(guardrail),
                        "registered threshold (see pre-registration)",
                        now_unix_ms,
                    )
                    .await;
            }
        }
        for guardrail in guardrails {
            if let Some(telemetry) = self.telemetry.as_ref() {
                telemetry.guardrail(
                    crate::telemetry::guardrail_of(guardrail),
                    crate::telemetry::guardrail_action(guardrail),
                );
            }
        }
    }

    /// The restore contract after a stop request: the request is recorded,
    /// then the platform state is read back. An unconfirmed restore is a
    /// failed restore with a stable incident reference, never an assumed
    /// success.
    async fn confirm_restore(
        &self,
        experiment_id: &str,
        guardrails: &[apex_lift::guardrails::GuardrailEvent],
        now_unix_ms: i64,
    ) {
        let operation_id = format!("op_{experiment_id}");
        let guard_type = guardrails
            .first()
            .map(crate::telemetry::guardrail_of)
            .map(|guardrail| guardrail.as_str())
            .unwrap_or("stop_requested");
        if let Some(telemetry) = self.telemetry.as_ref() {
            telemetry.restore(RestoreOperation::Restore, RestoreOutcome::Requested);
        }
        if let Some(events) = self.events.as_ref() {
            events
                .restore_requested(guard_type, &operation_id, "stopped", now_unix_ms)
                .await;
        }
        let read = self.ops.experiment(experiment_id).await;
        let (confirmed, error_class) = match &read {
            Ok(result) if result.state == apex_core::ExperimentState::Stopped => (true, None),
            Ok(_) => (false, Some("not_stopped")),
            Err(_) => (false, Some("read_failed")),
        };
        if confirmed {
            if let Some(telemetry) = self.telemetry.as_ref() {
                telemetry.restore(RestoreOperation::Restore, RestoreOutcome::Confirmed);
            }
            if let Some(events) = self.events.as_ref() {
                events
                    .restore_confirmed(
                        guard_type,
                        &operation_id,
                        "stopped",
                        "platform read reports stopped",
                        now_unix_ms,
                    )
                    .await;
            }
            return;
        }
        let error_class = error_class.unwrap_or("unconfirmed");
        if let Some(telemetry) = self.telemetry.as_ref() {
            telemetry.restore(RestoreOperation::Restore, RestoreOutcome::Failed);
        }
        if let Some(events) = self.events.as_ref() {
            events
                .restore_failed(
                    guard_type,
                    &operation_id,
                    "stopped",
                    error_class,
                    now_unix_ms,
                )
                .await;
        }
        if let Some(notify) = self.notify.as_ref() {
            notify
                .restore_failed(
                    &self.publisher_id,
                    experiment_id,
                    &crate::notify::incident_id_for(experiment_id),
                    now_unix_ms,
                )
                .await;
        }
    }

    /// Feed a settled test back into the bucket model and persist it, so the
    /// next cycle's proposals learn from results (keeps and rollbacks alike,
    /// or the model conditions on the decision).
    async fn learn(
        &self,
        experiment: &RunningExperiment,
        ledger: &dyn LedgerLog,
        outcome: &SettleOutcome,
    ) {
        let (Some(priors), Some(path)) = (&self.priors, &self.priors_path) else {
            return;
        };
        let label = status_label(outcome);
        if !matches!(label.as_str(), "kept" | "rolled_back") {
            return;
        }
        let Some(bucket) = change_bucket(ledger, experiment.change_seq) else {
            return;
        };
        let ratios = match outcome {
            SettleOutcome::Kept {
                daily_log_ratios, ..
            }
            | SettleOutcome::RolledBack {
                daily_log_ratios, ..
            } => daily_log_ratios.clone(),
            _ => return,
        };
        let mut priors = priors.clone();
        apex_lift::emit::apply_decision_to_priors(&mut priors, &bucket, &label, &ratios);
        if let Ok(json) = serde_json::to_string_pretty(&priors) {
            let _ = std::fs::write(path, json);
        }
    }

    /// The policy this runner applies (exposed so callers can pin it in logs).
    pub fn policy(&self) -> &AutopilotPolicy {
        &self.policy
    }

    /// The publisher this runner bills.
    pub fn publisher_id(&self) -> &str {
        &self.publisher_id
    }

    /// The bucket model this runner learns into, when configured.
    pub fn runner_priors(&self) -> Option<&apex_lift::priors::LiftPriors> {
        self.priors.as_ref()
    }

    /// The platform adapter behind the write loop.
    pub fn platform_ops(&self) -> &dyn PlatformOps {
        self.ops.as_ref()
    }

    /// Emit the right alert for a failed daily pull: an expired token when the
    /// platform refused the call on auth, otherwise a reporting failure.
    async fn alert_pull_failure(&self, experiment_id: &str, source: &apex_core::PlatformError) {
        let alert = match source {
            apex_core::PlatformError::Auth { platform, .. } => Alert::ExpiredToken {
                platform: platform.as_str().to_string(),
                detail: format!("daily pull for {experiment_id} was refused on auth"),
            },
            other => Alert::PullFailure {
                experiment_id: experiment_id.to_string(),
                detail: other.to_string(),
            },
        };
        self.alerts.emit(alert).await;
    }

    /// The scopes whose current settings differ from what the test
    /// pre-registered. An adapter that cannot read settings is skipped, not
    /// treated as an edit: a failure to look never voids a test.
    async fn settings_changed(&self, experiment: &RunningExperiment) -> Vec<String> {
        if experiment.expected_settings.trim().is_empty() {
            return Vec::new();
        }
        match self.ops.current_settings(&experiment.ad_unit_id).await {
            // Compare canonical JSON, not raw strings: two encodings of the
            // same settings are not a publisher edit and must not void a test.
            Ok(current) if !same_settings(&current, &experiment.expected_settings) => {
                vec![experiment.ad_unit_id.clone()]
            }
            _ => Vec::new(),
        }
    }
}

/// What one monthly billing pass did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MonthlyReport {
    pub month: String,
    /// Scopes (ad units) that had at least one kept change in force.
    pub billed_scopes: u32,
    /// One entry per billed change: its id and the fee in micros.
    pub fees: Vec<(String, i64)>,
    /// The invoice's gross total (net + vat), in micros.
    pub invoice_total_micros: i64,
    /// The invoice's display reference (`INV-<month>-<publisher>`), derived
    /// deterministically because the ledger carries no invoice registry.
    pub invoice_reference: String,
    /// The Adyen payment links created for this invoice, when collection is
    /// wired.
    pub payment_links: Vec<String>,
    /// Whether collection ran (an Adyen client was attached).
    pub collected: bool,
}

impl Runner {
    /// One monthly billing pass: bill every kept change still in force for
    /// `month`, from the platform-reported revenue of its pre-registered
    /// scope. A change past its term or covered by a publisher override is
    /// skipped by [`apex_lift::run::run_month`].
    pub async fn run_monthly(
        &self,
        ledger: &mut dyn LedgerLog,
        publisher_id: &str,
        month: &str,
        month_start: &str,
        month_days: u32,
        _now_unix_ms: i64,
    ) -> Result<MonthlyReport, PlanError> {
        if publisher_id.trim().is_empty() {
            return Err(PlanError::Validation(
                "the monthly pass needs a publisher id to bill".into(),
            ));
        }
        let month_end = month_end(month_start, month_days)?;
        // The fee accrues over the closed month: date every billing record at
        // the month's last day so the invoice's period check holds.
        let accrual_at = crate::calendar::day_number(&month_end)
            .ok_or_else(|| PlanError::Validation("bad month end".into()))?
            * 86_400_000;
        let period = apex_billing::BillingPeriod {
            year_month: month.to_string(),
            start_unix_ms: crate::calendar::day_number(month_start)
                .ok_or_else(|| PlanError::Validation("bad month start".into()))?
                * 86_400_000,
            end_unix_ms: (crate::calendar::day_number(&month_end)
                .ok_or_else(|| PlanError::Validation("bad month end".into()))?
                + 1)
                * 86_400_000,
        };
        let mut by_scope: std::collections::BTreeMap<String, Vec<apex_lift::billing::KeptChange>> =
            std::collections::BTreeMap::new();
        for (scope, change) in kept_changes(ledger) {
            by_scope.entry(scope).or_default().push(change);
        }
        let mut report = MonthlyReport {
            month: month.to_string(),
            ..MonthlyReport::default()
        };
        let mut basis: Vec<apex_billing::BillingBasisRecord> = Vec::new();
        let mut fees_by_change: std::collections::BTreeMap<String, apex_lift::billing::FeeLine> =
            std::collections::BTreeMap::new();
        for (scope, changes) in by_scope {
            let revenue = self
                .ops
                .scope_revenue(&scope, month_start, &month_end)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?;
            let run = apex_lift::run::run_month(month, month_start, month_days, revenue, &changes)
                .map_err(|error| PlanError::Validation(format!("monthly billing: {error}")))?;
            for fee in &run.fees {
                // Adyen collects whole minor units: the signed fee is the
                // cent-exact one, and the basis re-derives it the same way.
                let mut fee = fee.clone();
                fee.fee_micros = apex_billing::round_to_cent(fee.fee_micros);
                fees_by_change.insert(fee.change_id.clone(), fee.clone());
                let fee = &fee;
                let record = apex_lift::emit::monthly_billing(fee, month)
                    .map_err(|error| PlanError::Validation(format!("monthly billing: {error}")))?;
                let payload = record
                    .record
                    .canonical_json()
                    .map_err(|error| PlanError::Validation(format!("monthly billing: {error}")))?;
                let entry = ledger
                    .append(LedgerRecord::lift_record(
                        accrual_at,
                        &record.sha256,
                        &payload,
                    ))
                    .map_err(PlanError::Ledger)?;
                report.fees.push((fee.change_id.clone(), fee.fee_micros));
                basis.push(apex_billing::BillingBasisRecord {
                    seq: entry.seq(),
                    at_unix_ms: accrual_at,
                    change_id: fee.change_id.clone(),
                    scope_revenue: fee.scope_revenue,
                    incremental: fee.incremental,
                    log_share: fee.log_share,
                    rate_bp: fee.rate_bp,
                    fee_micros: fee.fee_micros,
                    billed_days: fee.billed_days,
                    month_days: fee.month_days,
                });
            }
            // Term-ended changes stop billing: record the term end so the
            // billing-change pipeline is complete, not just skipped.
            for (change_id, stop) in &run.skipped {
                if matches!(stop, apex_lift::billing::BillingStop::TermEnd) {
                    let record =
                        apex_lift::emit::billing_change(change_id, &month_end, "termEnded", 0.0)
                            .map_err(|error| {
                                PlanError::Validation(format!("billing change: {error}"))
                            })?;
                    let payload = record.record.canonical_json().map_err(|error| {
                        PlanError::Validation(format!("billing change: {error}"))
                    })?;
                    ledger
                        .append(LedgerRecord::lift_record(
                            accrual_at,
                            &record.sha256,
                            &payload,
                        ))
                        .map_err(PlanError::Ledger)?;
                }
            }
            report.billed_scopes += 1;
        }

        let invoice_reference = invoice_reference(publisher_id, month);
        report.invoice_reference = invoice_reference.clone();

        // The fee preview goes out before the invoice is issued, and every
        // fee leaves a `fee_computed` event beside its ledger record.
        if !report.fees.is_empty() {
            let net_micros = report
                .fees
                .iter()
                .map(|(_, fee)| *fee)
                .fold(0i64, |total, fee| total.saturating_add(fee));
            if let Some(notify) = self.notify.as_ref() {
                notify
                    .fee_preview(
                        publisher_id,
                        month,
                        net_micros,
                        self.monitoring.is_some(),
                        _now_unix_ms,
                    )
                    .await;
            }
        }
        if let Some(events) = self.events.as_ref() {
            for (change_id, _) in &report.fees {
                if let Some(fee) = fees_by_change.get(change_id) {
                    events
                        .fee_computed(
                            change_id,
                            month,
                            apex_billing::ADYEN_CURRENCY,
                            fee.scope_revenue,
                            (fee.lambda * 10_000.0).round() as i64,
                            (fee.factor * 10_000.0).round() as i64,
                            fee.rate_bp,
                            fee.billed_days,
                            vec![fee.pre_registration_hash.clone(), fee.result_hash.clone()],
                            accrual_at,
                        )
                        .await;
                }
            }
        }

        // The invoice is re-derived from the signed billing records; the flat
        // monitoring plan is billed as its own line on the same invoice.
        let mut invoice =
            match apex_billing::compute_invoice_from_basis(publisher_id, period.clone(), &basis) {
                Ok(invoice) => invoice,
                Err(error) => {
                    // A refused proof is a hold, not a charge: the affected draft
                    // is flagged, the customer is told, and nothing is issued.
                    let draft_reference = billing_error_seq(&error)
                        .and_then(|seq| {
                            basis
                                .iter()
                                .find(|record| record.seq == seq)
                                .map(|record| record.change_id.clone())
                        })
                        .unwrap_or_else(|| crate::notify::draft_reference(month));
                    if let Some(events) = self.events.as_ref() {
                        events
                            .fee_held(
                                &draft_reference,
                                month,
                                apex_billing::ADYEN_CURRENCY,
                                "proof_hold",
                                accrual_at,
                            )
                            .await;
                    }
                    if let Some(notify) = self.notify.as_ref() {
                        notify
                            .proof_hold(publisher_id, &draft_reference, _now_unix_ms)
                            .await;
                    }
                    return Err(PlanError::Validation(format!("invoice: {error}")));
                }
            };
        if let Some(plan) = &self.monitoring {
            let next_seq = invoice
                .line_items
                .iter()
                .map(|line| line.seq)
                .max()
                .unwrap_or(0)
                + 1;
            invoice.line_items.push(plan.monthly_line(next_seq, month));
            invoice.total_micros = invoice
                .total_micros
                .checked_add(plan.monthly_fee_micros)
                .ok_or(PlanError::Validation("invoice overflow".into()))?;
        }
        invoice = invoice.with_vat(self.vat_rate_bp);
        report.invoice_total_micros = invoice.gross_micros;
        if let Some(events) = self.events.as_ref() {
            for line in &invoice.line_items {
                events
                    .fee_invoice_issued(
                        &line.experiment_id,
                        month,
                        apex_billing::ADYEN_CURRENCY,
                        &invoice_reference,
                        line.gross_micros.max(line.fee_micros),
                        accrual_at,
                    )
                    .await;
            }
        }

        if let Some((client, transport)) = &self.adyen {
            let receipt = apex_billing::create_payment_links(client, transport.as_ref(), &invoice)
                .await
                .map_err(|error| PlanError::Validation(format!("collection: {error}")))?;
            report.payment_links = receipt.links.iter().map(|link| link.url.clone()).collect();
            report.collected = true;
            // The collected references are recorded so a later received
            // payment can state a real remaining balance.
            if let Some(ledger_path) = &self.payment_ledger_path {
                for line in &invoice.line_items {
                    let Ok(item) =
                        apex_billing::InvoiceLineItem::from_invoice_line(line, publisher_id, month)
                    else {
                        continue;
                    };
                    if item.amount_minor_units <= 0 {
                        continue;
                    }
                    let gross_micros = line.gross_micros.max(line.fee_micros);
                    let row = crate::payment_events::PaymentLedgerRow {
                        reference: item.reference.clone(),
                        amount_micros: gross_micros,
                        currency: item.currency.clone(),
                        at_unix_ms: _now_unix_ms,
                    };
                    crate::payment_events::append_payment_ledger_row(ledger_path, &row)
                        .map_err(|error| PlanError::Validation(error.to_string()))?;
                }
            }
        }
        Ok(report)
    }
}

impl Runner {
    /// Start an A/A (null) experiment and record it: the same configuration on
    /// both arms, so the split and pipeline can be checked before any real
    /// change rides on them. Returns the platform experiment id.
    pub async fn start_aa(
        &self,
        ledger: &mut dyn LedgerLog,
        platform: apex_core::PlatformId,
        spec: &apex_core::ExperimentSpec,
        now_unix_ms: i64,
    ) -> Result<String, PlanError> {
        let experiment_id = self
            .ops
            .start_null_experiment(spec)
            .await
            .map_err(|source| PlanError::ApplyFailed { source })?;
        ledger
            .append(LedgerRecord::experiment_started(
                now_unix_ms,
                platform,
                &experiment_id,
                spec.clone(),
            ))
            .map_err(PlanError::Ledger)?;
        // The platform start is confirmed: the experiment is running. The
        // A/A path has no pre-registration, so only the started event exists.
        if let Some(events) = self.events.as_ref() {
            events
                .experiment_started(&experiment_id, "running", now_unix_ms)
                .await;
        }
        Ok(experiment_id)
    }

    /// Evaluate a completed A/A run: the daily log ratios of the two arms,
    /// judged by the engine's calibration rule at the planned split.
    pub async fn aa_conclusion(
        &self,
        experiment_id: &str,
        planned_split: f64,
    ) -> Result<apex_lift::calibration::AaConclusion, PlanError> {
        let days = self
            .ops
            .experiment_days(experiment_id)
            .await
            .map_err(|source| PlanError::ApplyFailed { source })?;
        let ratios = aa_ratios(&days.control, &days.test, planned_split);
        Ok(apex_lift::calibration::evaluate_aa(&ratios, planned_split))
    }
}

/// The daily log ratios between the two arms, using the same arm-value rule as
/// the analysis window: per-user when users are reported, otherwise revenue
/// divided by the arm's allocated share.
fn aa_ratios(
    control: &[apex_lift::window::ArmDay],
    test: &[apex_lift::window::ArmDay],
    planned_split: f64,
) -> Vec<f64> {
    let split = planned_split.clamp(0.01, 0.99);
    let value = |day: &apex_lift::window::ArmDay, share: f64| -> Option<f64> {
        match day.users {
            Some(users) if users > 0 => Some(day.revenue as f64 / users as f64),
            Some(_) => None,
            None => Some(day.revenue as f64 / share),
        }
    };
    control
        .iter()
        .zip(test.iter())
        .filter_map(|(c, t)| {
            let control = value(c, 1.0 - split)?;
            let test = value(t, split)?;
            (control > 0.0 && test > 0.0).then(|| (test / control).ln())
        })
        .collect()
}

/// A kept change whose month-6 (or 30%-eCPM-move) re-check is due.
#[derive(Debug, Clone, PartialEq)]
pub struct RecheckDue {
    pub change_id: String,
    pub scope: String,
    pub months_since_keep: u32,
    pub plan: apex_lift::recheck::ReversalPlan,
}

impl Runner {
    /// The kept changes whose re-check is due at `now_unix_ms` by the scheduled
    /// month-6 trigger. The reversal test itself is a normal cycle; this
    /// planner tells the operator which one to start.
    pub fn due_rechecks(&self, ledger: &dyn LedgerLog, now_unix_ms: i64) -> Vec<RecheckDue> {
        let today = now_unix_ms.div_euclid(86_400_000);
        kept_changes(ledger)
            .into_iter()
            .filter_map(|(scope, change)| {
                let months = months_between(&change.kept_day, today)?;
                apex_lift::recheck::recheck_due(months, 0.0).then_some(RecheckDue {
                    change_id: change.change_id.clone(),
                    scope,
                    months_since_keep: months,
                    plan: apex_lift::recheck::reversal_plan(),
                })
            })
            .collect()
    }
}

/// Whole months from a `YYYY-MM-DD` day to a day number since the epoch.
fn months_between(kept_day: &str, today_day: i64) -> Option<u32> {
    let kept = crate::calendar::day_number(kept_day)?;
    let today = crate::calendar::format_day(today_day);
    let (ky, km, kd) = parse_ymd(kept_day)?;
    let (ty, tm, td) = parse_ymd(&today)?;
    if today_day < kept {
        return Some(0);
    }
    let mut months = (ty - ky) * 12 + (tm - km);
    if td < kd {
        months -= 1;
    }
    u32::try_from(months.max(0)).ok()
}

fn parse_ymd(day: &str) -> Option<(i64, i64, i64)> {
    let mut parts = day.split('-');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((y, m, d))
}

/// The last calendar day of a month, `YYYY-MM-DD`.
fn month_end(month_start: &str, month_days: u32) -> Result<String, PlanError> {
    let start = crate::calendar::day_number(month_start).ok_or_else(|| {
        PlanError::Validation(format!("month start '{month_start}' is not a valid day"))
    })?;
    Ok(crate::calendar::format_day(
        start + i64::from(month_days.max(1)) - 1,
    ))
}

/// Every kept change in the ledger, paired with its pre-registered scope.
fn kept_changes(ledger: &dyn LedgerLog) -> Vec<(String, apex_lift::billing::KeptChange)> {
    let records = ledger.records();
    let mut out = Vec::new();
    for record in &records {
        let LedgerRecord::ExperimentStarted { spec, .. } = record else {
            continue;
        };
        let name = &spec.name;
        let pre = records.iter().find_map(|candidate| match candidate {
            LedgerRecord::LiftRecord {
                payload_sha256,
                payload,
                ..
            } => match parse_lift_record(payload) {
                Some(apex_lift::records::LiftRecord::PreRegistration {
                    experiment_id,
                    rate_bp,
                    ..
                }) if &experiment_id == name => Some((payload_sha256.clone(), rate_bp)),
                _ => None,
            },
            _ => None,
        });
        let result = records.iter().find_map(|candidate| match candidate {
            LedgerRecord::LiftRecord {
                at_unix_ms,
                payload_sha256,
                payload,
                ..
            } => match parse_lift_record(payload) {
                Some(apex_lift::records::LiftRecord::Result {
                    experiment_id,
                    decision,
                    lambda,
                    ..
                }) if &experiment_id == name && decision == "keep" && lambda > 0.0 => {
                    Some((*at_unix_ms, payload_sha256.clone(), lambda))
                }
                _ => None,
            },
            _ => None,
        });
        if let (Some((pre_hash, rate_bp)), Some((at_unix_ms, result_hash, lambda))) = (pre, result)
        {
            out.push((
                spec.ad_unit_id.clone(),
                apex_lift::billing::KeptChange {
                    change_id: name.clone(),
                    lambda,
                    rate_bp,
                    kept_day: crate::calendar::format_day(at_unix_ms.div_euclid(86_400_000)),
                    overridden_day: None,
                    pre_registration_hash: pre_hash,
                    result_hash,
                },
            ));
        }
    }
    out
}

fn parse_lift_record(payload: &str) -> Option<apex_lift::records::LiftRecord> {
    serde_json::from_str(payload).ok()
}

/// Whether two settings encodings describe the same value, comparing parsed
/// JSON when both parse (so key order or spacing never reads as an edit).
fn same_settings(a: &str, b: &str) -> bool {
    match (
        serde_json::from_str::<serde_json::Value>(a),
        serde_json::from_str::<serde_json::Value>(b),
    ) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The day an experiment started, from its signed `experimentStarted`
/// record; `None` when the record is absent.
fn started_day(ledger: &dyn LedgerLog, experiment_id: &str) -> Option<String> {
    ledger.records().iter().find_map(|record| match record {
        LedgerRecord::ExperimentStarted {
            experiment_id: id,
            at_unix_ms,
            ..
        } if id == experiment_id => Some(crate::calendar::format_day(
            at_unix_ms.div_euclid(86_400_000),
        )),
        _ => None,
    })
}

/// The invoice display reference the events and messages quote:
/// `INV-<YYYY-MM>-<publisher>` (the ledger has no invoice registry, so the
/// reference is derived from the invoicing facts).
pub fn invoice_reference(publisher_id: &str, month: &str) -> String {
    format!("INV-{month}-{publisher_id}")
}

/// The ledger sequence a billing refusal names, when it names one.
fn billing_error_seq(error: &apex_billing::BillingError) -> Option<u64> {
    match error {
        apex_billing::BillingError::OutOfPeriod { seq }
        | apex_billing::BillingError::WrongPublisher { seq }
        | apex_billing::BillingError::TamperedProof { seq, .. }
        | apex_billing::BillingError::Duplicate { seq, .. }
        | apex_billing::BillingError::SubCentAmount { seq }
        | apex_billing::BillingError::UnsafeIdentifier { seq, .. } => Some(*seq),
        _ => None,
    }
}

/// The bounded error class of a platform failure, for pull events.
fn error_class_of(source: &apex_core::PlatformError) -> &'static str {
    match source {
        apex_core::PlatformError::Auth { .. } => "auth_refusal",
        apex_core::PlatformError::RateLimited { .. } => "rate_limit",
        apex_core::PlatformError::Schema { .. } => "schema_failure",
        apex_core::PlatformError::Network { .. } => "transport_failure",
        apex_core::PlatformError::Http { .. } => "transport_failure",
        apex_core::PlatformError::Validation(_) => "validation_failure",
    }
}

/// How a fired guardrail's observed input reads in the event payload.
fn guardrail_input(event: &apex_lift::guardrails::GuardrailEvent) -> &'static str {
    use apex_lift::guardrails::GuardrailEvent;
    match event {
        GuardrailEvent::FastHarm { .. } => "point lift below the registered harm threshold",
        GuardrailEvent::ConfirmedHarm { .. } => "upper bound below zero",
        GuardrailEvent::FillRewardedInterstitial { .. } => {
            "fill below the registered threshold on two consecutive days"
        }
        GuardrailEvent::FillBanner { .. } => {
            "banner fill below the registered threshold on two consecutive days"
        }
        GuardrailEvent::PublisherEdit { .. } => "settings changed outside Apex",
        GuardrailEvent::SplitMismatch { .. } => {
            "arm split past the registered chi-square threshold"
        }
        GuardrailEvent::ReportingGap { .. } => "days still not final past 72 hours",
        GuardrailEvent::PlatformIncident { .. } => {
            "both arms' revenue fell past the incident threshold"
        }
    }
}

fn status_label(outcome: &SettleOutcome) -> String {
    match outcome {
        SettleOutcome::Kept { .. } => "kept".into(),
        SettleOutcome::RolledBack { .. } => "rolled_back".into(),
        SettleOutcome::Voided { .. } => "voided".into(),
        SettleOutcome::Pending { .. } => "pending".into(),
    }
}

/// One running experiment: its platform id and the ledger sequence of the
/// change it tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningExperiment {
    pub experiment_id: String,
    pub change_seq: u64,
    pub ad_unit_id: String,
    /// The scope's value as pre-registered; the check flags a publisher edit
    /// when the scope's current settings differ.
    pub expected_settings: String,
}

/// Experiments that have started but have no terminal record yet. A settled
/// experiment leaves an `experimentObserved` record (keep) or a
/// `rollbackPerformed` record (rollback); a voided one leaves neither, so a
/// void is re-checked until the window closes and the engine voids again.
fn running_experiments(ledger: &dyn LedgerLog) -> Vec<RunningExperiment> {
    let records = ledger.records();
    // Only experiments with a signed pre-registration are settled here; an
    // A/A (null) run has none and is evaluated through `aa_conclusion`
    // instead, so it never gets mis-attributed to another change's sequence.
    let pre_registered: std::collections::HashSet<String> = records
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::LiftRecord { payload, .. } => {
                parse_lift_record(payload).and_then(|parsed| match parsed {
                    apex_lift::records::LiftRecord::PreRegistration { experiment_id, .. } => {
                        Some(experiment_id)
                    }
                    _ => None,
                })
            }
            _ => None,
        })
        .collect();
    let settled: std::collections::HashSet<String> = records
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::ExperimentObserved { experiment_id, .. } => Some(experiment_id.clone()),
            _ => None,
        })
        .collect();
    let rolled_back: std::collections::HashSet<u64> = records
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::RollbackPerformed {
                rolled_back_seq, ..
            } => Some(*rolled_back_seq),
            _ => None,
        })
        .collect();
    records
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::ExperimentStarted {
                seq,
                experiment_id,
                spec,
                ..
            } => {
                if !pre_registered.contains(&spec.name) {
                    return None;
                }
                let change_seq = change_before(&records, *seq)?;
                if settled.contains(experiment_id) || rolled_back.contains(&change_seq) {
                    return None;
                }
                let expected_settings = pre_registration_before_value(&records, &spec.name);
                Some(RunningExperiment {
                    experiment_id: experiment_id.clone(),
                    change_seq,
                    ad_unit_id: spec.ad_unit_id.clone(),
                    expected_settings,
                })
            }
            _ => None,
        })
        .collect()
}

/// The pre-registered `beforeValue` for `experiment_name`, from the signed
/// pre-registration record. Empty when the registration is not on the ledger.
fn pre_registration_before_value(records: &[&LedgerRecord], experiment_name: &str) -> String {
    for record in records {
        let LedgerRecord::LiftRecord { payload, .. } = record else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<apex_lift::records::LiftRecord>(payload) else {
            continue;
        };
        let Ok(prereg) = apex_lift::decision::PreRegistration::from_record(&parsed) else {
            continue;
        };
        if prereg.experiment_id == experiment_name {
            return prereg.before_value;
        }
    }
    String::new()
}

/// The prior bucket a settled change belongs to: a country-floor move on its
/// platform. The ad format is not carried by the change record, so the bucket
/// pools across formats for the platform and tier.
fn change_bucket(ledger: &dyn LedgerLog, change_seq: u64) -> Option<apex_lift::priors::Bucket> {
    let record = ledger
        .records()
        .into_iter()
        .find(|r| r.seq() == change_seq)?;
    match record {
        LedgerRecord::ChangeApplied {
            platform,
            change: ChangeKind::CountryFloorSet { country, .. },
            ..
        } => Some(apex_lift::priors::Bucket {
            lever: "floor".into(),
            ad_format: "unknown".into(),
            country_tier: country.clone(),
            platform: platform.as_str().to_string(),
        }),
        _ => None,
    }
}

/// The nearest `changeApplied` record before `started_seq`.
fn change_before(records: &[&LedgerRecord], started_seq: u64) -> Option<u64> {
    records
        .iter()
        .filter(|record| record.seq() < started_seq)
        .filter_map(|record| match record {
            LedgerRecord::ChangeApplied { seq, .. } => Some(*seq),
            _ => None,
        })
        .max()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_core::{ExperimentResult, ExperimentState};
    use apex_proof::{InMemoryLedger, Keypair};
    use async_trait::async_trait;

    use crate::model::Action;
    use crate::ops::{ExperimentDays, PlatformOps, RecordingOps};
    use crate::plan::run_cycle;

    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    fn ledger() -> InMemoryLedger {
        InMemoryLedger::new(Keypair::from_bytes(&[9u8; 32]))
    }

    fn floor_action() -> Action {
        Action::SetCountryFloor {
            ad_unit_id: "u1".into(),
            country: "ee".into(),
            floor_ecpm: 2_500_000,
        }
    }

    fn arm_day(day: &str, revenue: i64) -> apex_lift::window::ArmDay {
        apex_lift::window::ArmDay {
            day: day.to_string(),
            revenue,
            users: None,
            second_pull_revenue: Some(revenue),
            second_pull_users: None,
            fill: None,
        }
    }

    fn constant_days(control: i64, test: i64) -> ExperimentDays {
        ExperimentDays {
            control: (0..10)
                .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), control / 4 * 3))
                .collect(),
            test: (0..10)
                .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), test / 4))
                .collect(),
        }
    }

    fn policy() -> AutopilotPolicy {
        AutopilotPolicy {
            window_days: 7,
            interim_look: false,
            extension_allowed: false,
            ..AutopilotPolicy::conservative()
        }
    }

    #[tokio::test]
    async fn daily_pass_pulls_records_and_settles_a_complete_window() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));

        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{\"floor\":1000000}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");

        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(report.pulled, 1);
        assert_eq!(report.settled, 1);
        assert_eq!(
            report.outcomes,
            vec![("exp-1".to_string(), "kept".to_string())]
        );
        // A data-pull record precedes the signed result.
        let pulls = l
            .records()
            .iter()
            .filter(|r| match r {
                LedgerRecord::LiftRecord { payload, .. } => payload.contains("dataPull"),
                _ => false,
            })
            .count();
        assert_eq!(pulls, 1);
    }

    #[tokio::test]
    async fn a_publisher_edit_voids_the_test() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        // The scope's settings no longer match the pre-registered value.
        ops.put_current_settings("u1", "[{\"country\":\"ee\",\"floorEcpm\":9}]");
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "[{\"country\":\"ee\",\"floorEcpm\":1}]".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(
            report.outcomes,
            vec![("exp-1".to_string(), "voided".to_string())]
        );
        let deprecated = ops
            .call_log()
            .iter()
            .filter(|entry| entry.starts_with("deprecate:"))
            .count();
        assert_eq!(deprecated, 1, "a publisher edit deprecates the test");
    }

    #[tokio::test]
    async fn an_aa_run_starts_and_is_recorded() {
        let ops = Arc::new(RecordingOps::new());
        let mut l = ledger();
        let spec = apex_core::ExperimentSpec {
            name: "apex-aa-u1".into(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "A/A calibration".into(),
            test_group_allocation: 50,
        };
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let id = runner
            .start_aa(&mut l, apex_core::PlatformId::AppLovinMax, &spec, NOW)
            .await
            .expect("aa");
        assert_eq!(id, "apex-aa-u1");
        assert!(
            l.records().iter().any(|r| matches!(
                r,
                LedgerRecord::ExperimentStarted { experiment_id, .. } if experiment_id == "apex-aa-u1"
            )),
            "the A/A start is recorded"
        );
    }

    #[tokio::test]
    async fn a_short_aa_run_is_audit_only() {
        use apex_lift::calibration::AaConclusion;
        let ops = Arc::new(RecordingOps::new());
        ops.put_experiment_days(
            "apex-aa-u1",
            ExperimentDays {
                control: (0..3)
                    .map(|i| arm_day(&format!("2026-10-0{}", i + 1), 1_000_000))
                    .collect(),
                test: (0..3)
                    .map(|i| arm_day(&format!("2026-10-0{}", i + 1), 1_000_000))
                    .collect(),
            },
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let conclusion = runner
            .aa_conclusion("apex-aa-u1", 0.5)
            .await
            .expect("conclusion");
        assert!(
            matches!(conclusion, AaConclusion::AuditOnly { .. }),
            "{conclusion:?}"
        );
    }

    /// A transport that answers the Adyen payment-link call and records it.
    #[derive(Default)]
    struct AdyenFake {
        seen: std::sync::Mutex<Vec<apex_connectors::transport::HttpRequest>>,
    }

    #[async_trait]
    impl apex_connectors::transport::Transport for AdyenFake {
        async fn request(
            &self,
            req: apex_connectors::transport::HttpRequest,
        ) -> Result<
            apex_connectors::transport::HttpResponse,
            apex_connectors::transport::TransportError,
        > {
            self.seen.lock().unwrap().push(req);
            Ok(apex_connectors::transport::HttpResponse {
                status: 200,
                headers: vec![],
                body: br#"{"id":"PL1","url":"https://pay.adyen.com/PL1"}"#.to_vec(),
            })
        }
    }

    async fn kept_ledger(ops: &Arc<RecordingOps>, at: i64) -> InMemoryLedger {
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            at,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, at + 1).await.expect("daily");
        l
    }

    fn kept_ops() -> Arc<RecordingOps> {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        ops.put_scope_revenue("u1", 1_000_000_000);
        ops
    }

    #[tokio::test]
    async fn the_monthly_pass_collects_through_adyen_and_bills_monitoring() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let adyen = Arc::new(AdyenFake::default());
        let client = Arc::new(
            apex_billing::AdyenClient::new(
                "key",
                "ApexMerchant",
                b"hmac".to_vec(),
                "https://checkout-test.adyen.com",
            )
            .expect("client"),
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_vat_rate(2_000)
            .with_monitoring(apex_billing::MonitoringPlan::STANDARD)
            .with_adyen(client, adyen.clone());
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert!(report.collected, "the Adyen client collects");
        // One link per invoice line: the lift fee and the flat monitoring fee.
        assert_eq!(report.payment_links.len(), 2, "{:?}", report.payment_links);
        assert!(report
            .payment_links
            .iter()
            .all(|url| url == "https://pay.adyen.com/PL1"));
        assert!(report.invoice_total_micros > 0);
        let calls = adyen.seen.lock().unwrap();
        assert_eq!(calls.len(), 2, "one payment link per invoice line");
        assert!(calls[0].url.contains("/v71/paymentLinks"));
    }

    #[tokio::test]
    async fn the_monthly_pass_without_adyen_records_but_does_not_collect() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let runner =
            Runner::new(ops.clone(), policy(), FillFormat::Rewarded).with_publisher("pub1");
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert!(!report.collected);
        assert!(report.payment_links.is_empty());
        assert!(
            report.invoice_total_micros > 0,
            "the invoice is still computed"
        );
    }

    #[tokio::test]
    async fn the_monthly_pass_requires_a_publisher() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let err = runner
            .run_monthly(&mut l, "", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect_err("publisher required");
        assert!(matches!(err, PlanError::Validation(_)), "{err}");
    }

    #[tokio::test]
    async fn a_daily_pass_learns_and_persists_the_bucket_model() {
        let ops = kept_ops();
        let dir = std::env::temp_dir().join(format!("apex-priors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let priors_path = dir.join("priors.json");
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_priors(apex_lift::priors::LiftPriors::default(), &priors_path);
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        let saved = std::fs::read_to_string(&priors_path).expect("saved");
        assert!(saved.contains("floor"), "the bucket is learned: {saved}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_aa_run_is_not_swept_into_the_daily_settle_pass() {
        // An A/A run has no pre-registration; the daily pass must leave it
        // alone rather than mis-attribute it to another change's sequence.
        let ops = Arc::new(RecordingOps::new());
        let mut l = ledger();
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let spec = apex_core::ExperimentSpec {
            name: "apex-aa-u1".into(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "A/A calibration".into(),
            test_group_allocation: 50,
        };
        runner
            .start_aa(&mut l, apex_core::PlatformId::AppLovinMax, &spec, NOW)
            .await
            .expect("aa");
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(report.pulled, 0, "the A/A run is not pulled for settling");
        assert!(report.failures.is_empty());
    }

    #[tokio::test]
    async fn a_first_look_that_offers_an_extension_waits_for_its_week() {
        // A positive point lift with a wide interval offers an extension; with
        // only ten days of pulls the extended window is incomplete, so the
        // experiment waits rather than rolling back.
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        // Alternating strong/weak days: a positive mean with a wide spread.
        let days = ExperimentDays {
            control: (0..10)
                .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), 4_000_000))
                .collect(),
            test: (0..10)
                .map(|i| {
                    let value = if i % 2 == 0 { 4_900_000 } else { 4_020_000 };
                    arm_day(&format!("2026-10-{:02}", i + 1), value)
                })
                .collect(),
        };
        ops.put_experiment_days("exp-1", days);
        let extension_policy = AutopilotPolicy {
            extension_allowed: true,
            ..policy()
        };
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &extension_policy,
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), extension_policy, FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(report.settled, 0, "the first look does not settle");
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(
            !l.records()
                .iter()
                .any(|r| matches!(r, LedgerRecord::RollbackPerformed { .. })),
            "an offered extension never rolls the test back"
        );
    }

    #[tokio::test]
    async fn a_guardrail_stop_alerts_the_customer() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        // Negative lift trips the fast-harm guardrail and rolls the test back.
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 3_600_000));
        let alerts = Arc::new(crate::alerts::RecordingAlertSink::new());
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner =
            Runner::new(ops.clone(), policy(), FillFormat::Rewarded).with_alerts(alerts.clone());
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        let emitted = alerts.alerts();
        assert_eq!(emitted.len(), 1, "{emitted:?}");
        assert!(
            matches!(emitted[0], Alert::GuardrailStop { .. }),
            "{emitted:?}"
        );
    }

    #[tokio::test]
    async fn a_failed_pull_alerts_the_customer() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        // No scripted days: the pull fails.
        let alerts = Arc::new(crate::alerts::RecordingAlertSink::new());
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner =
            Runner::new(ops.clone(), policy(), FillFormat::Rewarded).with_alerts(alerts.clone());
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("pass");
        assert_eq!(
            report.failures.len(),
            1,
            "the failure is recorded, not fatal"
        );
        assert!(matches!(
            alerts.alerts().as_slice(),
            [Alert::PullFailure { .. }]
        ));
    }

    #[tokio::test]
    async fn the_same_settings_in_a_different_encoding_do_not_void() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        // The same floors, keys in a different order: not a publisher edit.
        ops.put_current_settings("u1", r#"[{"floorEcpm":1,"country":"ee"}]"#);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: r#"[{"country":"ee","floorEcpm":1}]"#.into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(
            report.outcomes,
            vec![("exp-1".to_string(), "kept".to_string())],
            "a re-ordered encoding must not read as an edit"
        );
    }

    #[test]
    fn same_settings_compares_parsed_json_then_falls_back_to_strings() {
        assert!(same_settings("[1, 2]", "[1,2]"));
        assert!(!same_settings("[1, 2]", "[1, 3]"));
        assert!(same_settings("not json", "not json"));
        assert!(!same_settings("not json", "other"));
    }

    #[tokio::test]
    async fn a_second_daily_pass_does_not_re_settle() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, NOW + 1).await.expect("first");
        let second = runner.run_daily(&mut l, NOW + 2).await.expect("second");
        assert_eq!(
            second.pulled, 0,
            "a settled experiment is no longer running"
        );
        assert_eq!(second.settled, 0);
    }

    #[tokio::test]
    async fn a_settled_pending_experiment_stays_running_until_the_window_closes() {
        // Nine days is short of the ten a seven-day window needs: the engine
        // returns Pending and the experiment is retried next pass.
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        let mut short = constant_days(4_000_000, 4_400_000);
        short.control.truncate(9);
        short.test.truncate(9);
        ops.put_experiment_days("exp-1", short);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(report.pulled, 1);
        assert_eq!(report.settled, 0, "the window is not complete yet");
    }

    #[tokio::test]
    async fn monthly_pass_bills_a_kept_change_for_its_scope() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        ops.put_scope_revenue("u1", 1_000_000_000);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(
            kept_changes(&l).len(),
            1,
            "one kept change is on the ledger"
        );

        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.billed_scopes, 1);
        assert_eq!(report.fees.len(), 1);
        assert!(report.fees[0].1 > 0, "a proven lift bills a positive fee");
        let billing = l
            .records()
            .iter()
            .filter(|r| match r {
                LedgerRecord::LiftRecord { payload, .. } => payload.contains("monthlyBilling"),
                _ => false,
            })
            .count();
        assert_eq!(billing, 1);
    }

    #[tokio::test]
    async fn a_rolled_back_change_is_never_billed() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 3_600_000));
        ops.put_scope_revenue("u1", 1_000_000_000);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");

        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.billed_scopes, 0, "no kept change, no scope");
        assert!(report.fees.is_empty());
    }

    #[tokio::test]
    async fn due_rechecks_finds_a_kept_change_after_six_months() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");

        // The kept day is late 2023; six months on, the re-check is due.
        let later = crate::calendar::day_number("2024-06-01").expect("day") * 86_400_000;
        let due = runner.due_rechecks(&l, later);
        assert_eq!(due.len(), 1, "{due:?}");
        assert!(due[0].months_since_keep >= 6);
    }

    #[tokio::test]
    async fn with_priors_accepts_owned_and_borrowed_paths() {
        let ops = Arc::new(RecordingOps::new());
        let owned = Runner::new(ops.clone(), policy(), FillFormat::Rewarded).with_priors(
            apex_lift::priors::LiftPriors::default(),
            String::from("/tmp/apex-priors-owned.json"),
        );
        let borrowed_path = std::path::PathBuf::from("/tmp/apex-priors-borrowed.json");
        let borrowed = Runner::new(ops, policy(), FillFormat::Rewarded)
            .with_priors(apex_lift::priors::LiftPriors::default(), &borrowed_path);
        assert!(owned.runner_priors().is_some());
        assert!(borrowed.runner_priors().is_some());
    }

    #[test]
    fn change_bucket_is_none_for_non_floor_changes() {
        let mut l = ledger();
        l.append(LedgerRecord::change_applied(
            1,
            apex_core::PlatformId::Admob,
            "u1",
            ChangeKind::EcpmSet {
                mediation_group_id: "g".into(),
                ad_unit_id: "u1".into(),
                ecpm: 1,
            },
            "k",
            "p",
        ))
        .expect("append");
        assert!(change_bucket(&l, 0).is_none());
    }

    #[tokio::test]
    async fn policy_rechecks_and_debug_are_all_reachable() {
        let ops = Arc::new(RecordingOps::new());
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        assert_eq!(runner.policy().window_days, 7);
        assert!(format!("{runner:?}").contains("Runner"));

        let mut l = ledger();
        // A months-later clock makes the month-6 re-check due for a kept change.
        let later = crate::calendar::day_number("2024-06-01").expect("day") * 86_400_000;
        assert!(
            runner.due_rechecks(&l, later).is_empty(),
            "nothing kept yet"
        );

        // Exercise the null adapter so its trait surface is covered too.
        let null = NullOps;
        assert!(!null.writable("op").await);
        assert!(null.experiment("x").await.is_err());
        assert!(null.apply(&floor_action(), "k").await.is_ok());
        assert!(null.rollback(&floor_action()).await.is_ok());
        assert!(null
            .start_experiment(&crate::ops::ExperimentRequest {
                experiment_id: "e".into(),
                spec: apex_core::ExperimentSpec {
                    name: "e".into(),
                    ad_unit_id: "u".into(),
                    metric: apex_core::ExperimentMetric::Revenue,
                    variants: 2,
                    change_description: "c".into(),
                    test_group_allocation: 50,
                },
                change: apex_proof::ChangeKind::CountryFloorSet {
                    country: "EE".into(),
                    floor_ecpm: 1,
                },
                prev_value: "{}".into(),
            })
            .await
            .is_ok());
        assert!(null.promote_experiment("x").await.is_ok());
        assert!(null.deprecate_experiment("x").await.is_ok());
        assert!(null.call_log().is_empty());
        assert!(!null.change_in_test_arm());
        assert!(null.scope_revenue("u", "a", "b").await.is_err());
        assert!(null.experiment_days("x").await.is_err());
        assert!(null.current_settings("u").await.is_err());
        assert!(null
            .start_null_experiment(&apex_core::ExperimentSpec {
                name: "e".into(),
                ad_unit_id: "u".into(),
                metric: apex_core::ExperimentMetric::Revenue,
                variants: 2,
                change_description: "c".into(),
                test_group_allocation: 50,
            })
            .await
            .is_err());
        let _ = &mut l;
    }

    /// A minimal adapter used to prove the runner is generic over PlatformOps.
    struct NullOps;

    #[async_trait]
    impl PlatformOps for NullOps {
        async fn writable(&self, _operation: &str) -> bool {
            false
        }
        async fn start_experiment(
            &self,
            _request: &crate::ops::ExperimentRequest,
        ) -> Result<String, apex_core::PlatformError> {
            Ok(String::new())
        }
        async fn experiment(
            &self,
            _id: &str,
        ) -> Result<ExperimentResult, apex_core::PlatformError> {
            Err(apex_core::PlatformError::Validation("none".into()))
        }
        async fn promote_experiment(&self, _id: &str) -> Result<(), apex_core::PlatformError> {
            Ok(())
        }
        async fn deprecate_experiment(&self, _id: &str) -> Result<(), apex_core::PlatformError> {
            Ok(())
        }
        async fn apply(
            &self,
            _action: &Action,
            _key: &str,
        ) -> Result<(), apex_core::PlatformError> {
            Ok(())
        }
        async fn rollback(&self, _action: &Action) -> Result<(), apex_core::PlatformError> {
            Ok(())
        }
        fn call_log(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn months_between_counts_whole_months() {
        let day = |text: &str| crate::calendar::day_number(text).expect("valid day");
        assert_eq!(months_between("2023-11-14", day("2023-11-20")), Some(0));
        assert_eq!(months_between("2023-11-01", day("2023-12-01")), Some(1));
        assert_eq!(months_between("2023-05-01", day("2023-11-01")), Some(6));
        assert_eq!(months_between("2023-05-20", day("2023-11-01")), Some(5));
        assert_eq!(months_between("bad", day("2023-11-01")), None);
    }

    #[tokio::test]
    async fn an_empty_ledger_is_a_no_op() {
        let runner = Runner::new(Arc::new(NullOps), policy(), FillFormat::Rewarded);
        let mut l = ledger();
        let report = runner.run_daily(&mut l, NOW).await.expect("daily");
        assert_eq!(report, DailyReport::default());
    }

    /// The full instrumented runner for one test: file sink outbox, delivery
    /// log, registry, analytics store and the recording ops.
    struct Rig {
        runner: Runner,
        dir: std::path::PathBuf,
        store: Arc<apex_analytics::MemoryStore>,
        registry: Arc<apex_metrics::Registry>,
    }

    async fn rig(ops: Arc<RecordingOps>, label: &str) -> Rig {
        let dir =
            std::env::temp_dir().join(format!("apex-runner-wire-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let registry = Arc::new(apex_metrics::Registry::new());
        let sink = Arc::new(apex_notify::sink::FileSink::new(dir.join("outbox.jsonl")));
        let log =
            apex_notify::state::DeliveryLog::open(Some(&dir.join("delivery.jsonl"))).expect("log");
        let notifier = Arc::new(apex_notify::notifier::Notifier::new(
            sink,
            log,
            registry.clone(),
        ));
        let book =
            apex_notify::contacts::ContactBook::from_json(r#"{"northstar": ["ops@example.com"]}"#)
                .expect("book");
        let notify = Arc::new(crate::notify::NotifyPlan::new(
            notifier,
            book,
            "https://console.apexmediation.ee",
        ));
        let store = Arc::new(apex_analytics::MemoryStore::new());
        {
            use apex_analytics::EventStore as _;
            store.ensure_schema().await.expect("schema");
        }
        let client = apex_analytics::AnalyticsClient::new(store.clone(), "dev", "apex-runner-test");
        let events = Arc::new(
            crate::events::RunnerEvents::new(Arc::new(client), "northstar").with_environment("dev"),
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_publisher("northstar")
            .with_notifications(notify)
            .with_events(events)
            .with_telemetry(crate::telemetry::RunnerTelemetry::new(registry.clone()));
        Rig {
            runner,
            dir,
            store,
            registry,
        }
    }

    fn outbox(rig: &Rig) -> Vec<serde_json::Value> {
        std::fs::read_to_string(rig.dir.join("outbox.jsonl"))
            .map(|raw| {
                raw.lines()
                    .map(|line| serde_json::from_str(line).expect("row"))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn stored_types(rig: &Rig) -> Vec<String> {
        use apex_analytics::store::ProjectionStore as _;
        rig.store
            .load_events()
            .await
            .expect("events")
            .into_iter()
            .map(|event| event.event_type.as_str().to_string())
            .collect()
    }

    fn metric_counter(
        rig: &Rig,
        metric: &'static apex_metrics::MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> u64 {
        rig.registry.counter_value(metric, labels).expect("metric")
    }

    #[tokio::test]
    async fn an_auth_refused_pull_notifies_reporting_access_and_records_failed_events() {
        use crate::ops::RecordingOps as Ops;
        let ops = Arc::new(Ops::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.script_experiment_days(vec![Err(apex_core::PlatformError::Auth {
            platform: apex_core::PlatformId::AppLovinMax,
            operation: "experimentDays".into(),
            status: 401,
        })]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        let rig = rig(ops.clone(), "auth").await;
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{\"floor\":1000000}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let report = rig
            .runner
            .run_daily(&mut l, NOW + 8 * 86_400_000)
            .await
            .expect("daily");
        assert_eq!(report.pull_failures, 1);
        assert_eq!(report.failures.len(), 1);
        let rows = outbox(&rig);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["template_class"], "reporting_access");
        let text = rows[0]["text"].as_str().expect("text");
        assert!(text.contains("Reconnect MAX reporting for Northstar"));
        assert!(text.contains("The latest reporting request was refused."));
        let types = stored_types(&rig).await;
        assert!(types.contains(&"report_pull_started".to_string()));
        assert!(types.contains(&"report_pull_failed".to_string()));
        assert!(!types.contains(&"report_pull_completed".to_string()));
        let _ = std::fs::remove_dir_all(&rig.dir);
    }

    #[tokio::test]
    async fn a_rollback_notifies_the_stop_and_records_the_restore_state() {
        use crate::ops::RecordingOps as Ops;
        let ops = Arc::new(Ops::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 3_600_000));
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        let rig = rig(ops.clone(), "stop").await;
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{\"floor\":1000000}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let report = rig
            .runner
            .run_daily(&mut l, NOW + 8 * 86_400_000)
            .await
            .expect("daily");
        assert!(report
            .outcomes
            .iter()
            .any(|(_, status)| status == "rolled_back"));
        assert!(report.failures.is_empty());
        // The stop message and the unconfirmed restore both reached the
        // contacts, with a stable incident reference.
        let rows = outbox(&rig);
        let classes: Vec<&str> = rows
            .iter()
            .map(|row| row["template_class"].as_str().unwrap_or_default())
            .collect();
        assert!(classes.contains(&"guardrail_stop"), "{classes:?}");
        assert!(classes.contains(&"restore_failed"), "{classes:?}");
        let restore = rows
            .iter()
            .find(|row| row["template_class"] == "restore_failed")
            .expect("restore row");
        let text = restore["text"].as_str().expect("text");
        assert!(text.contains("incident inc_"));
        assert!(text.contains("What is known:"));
        let incident = crate::notify::incident_id_for("exp-1");
        assert!(text.contains(&incident));
        let types = stored_types(&rig).await;
        for expected in [
            "experiment_decided",
            "guardrail_fired",
            "guardrail_restore_requested",
            "guardrail_restore_failed",
        ] {
            assert!(types.contains(&expected.to_string()), "missing {expected}");
        }
        assert_eq!(
            metric_counter(
                &rig,
                &apex_metrics::catalog::RESTORE_OPERATIONS_TOTAL,
                &[
                    ("platform", "applovin_max"),
                    ("operation", "restore"),
                    ("outcome", "failed"),
                ],
            ),
            1
        );
        assert_eq!(
            metric_counter(
                &rig,
                &apex_metrics::catalog::EXPERIMENT_DECISIONS_TOTAL,
                &[
                    ("platform", "applovin_max"),
                    ("decision", "rollback"),
                    ("reason_class", "harm"),
                ],
            ),
            1
        );
        let _ = std::fs::remove_dir_all(&rig.dir);
    }

    #[tokio::test]
    async fn pull_outcomes_set_the_accounts_gauge() {
        use crate::ops::RecordingOps as Ops;
        let ops = Arc::new(Ops::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![
            Ok("exp-1".into()),
            Ok("exp-2".into()),
            Ok("exp-3".into()),
            Ok("exp-4".into()),
        ]);
        for id in ["exp-1", "exp-2", "exp-3", "exp-4"] {
            ops.put_experiment(
                id,
                ExperimentResult {
                    experiment_id: id.into(),
                    state: ExperimentState::Running,
                    results: vec![],
                    lift_percent_bp: None,
                    confidence: None,
                    winner_variant_id: None,
                },
            );
        }
        let now = NOW + 1;
        let range_end = crate::calendar::format_day(now.div_euclid(86_400_000) - 1);
        let earlier =
            crate::calendar::format_day(crate::calendar::day_number(&range_end).expect("day") - 1);
        let partial_days = ExperimentDays {
            control: vec![arm_day(&earlier, 4_000_000)],
            test: vec![arm_day(&earlier, 4_400_000)],
        };
        // Pull order is ledger order: final coverage, an auth refusal (the
        // credential is disconnected), a schema failure (the account stays
        // stale), and coverage that stops before the last closed day. The
        // settle pass of exp-1 re-reads its days between the pulls, so the
        // script repeats exp-1's days before the next experiment's pull.
        ops.script_experiment_days(vec![
            Ok(constant_days(4_000_000, 4_400_000)),
            Ok(constant_days(4_000_000, 4_400_000)),
            Err(apex_core::PlatformError::Auth {
                platform: apex_core::PlatformId::AppLovinMax,
                operation: "experimentDays".into(),
                status: 401,
            }),
            Err(apex_core::PlatformError::Schema {
                platform: apex_core::PlatformId::AppLovinMax,
                operation: "experimentDays".into(),
                detail: "malformed".into(),
            }),
            Ok(partial_days.clone()),
            Ok(partial_days.clone()),
        ]);
        // The settle path re-reads days after the script is exhausted.
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        ops.put_experiment_days("exp-4", partial_days);

        let proposals = ["ee", "fr", "de", "se"]
            .into_iter()
            .map(|country| crate::model::Proposal {
                action: crate::model::Action::SetCountryFloor {
                    ad_unit_id: "u1".into(),
                    country: country.into(),
                    floor_ecpm: 2_500_000,
                },
                rationale: format!("raise {country}"),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            })
            .collect();
        let cycle_policy = AutopilotPolicy {
            max_actions_per_day: 8,
            ..policy()
        };
        let mut l = ledger();
        run_cycle(ops.as_ref(), &mut l, &cycle_policy, &[], proposals, NOW)
            .await
            .expect("cycle");

        let rig = rig(ops.clone(), "readiness").await;
        let report = rig.runner.run_daily(&mut l, now).await.expect("daily");
        assert!(report.pulled >= 1);
        let gauge = |class: &str| {
            rig.registry
                .gauge_value(
                    &apex_metrics::catalog::REPORT_ACCOUNTS,
                    &[("platform", "applovin_max"), ("readiness_class", class)],
                )
                .expect("gauge")
        };
        assert_eq!(gauge("final"), 1.0, "coverage through the last closed day");
        assert_eq!(gauge("partial"), 1.0, "coverage stops earlier");
        assert_eq!(
            gauge("stale"),
            1.0,
            "schema failure leaves the account stale"
        );
        assert_eq!(gauge("disconnected"), 1.0, "auth refusal disconnects");
        let _ = std::fs::remove_dir_all(&rig.dir);
    }

    #[tokio::test]
    async fn the_aa_start_emits_experiment_started() {
        use apex_analytics::store::{EventStore as _, ProjectionStore as _};
        let ops = Arc::new(RecordingOps::new());
        let store = Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = apex_analytics::AnalyticsClient::new(store.clone(), "dev", "apex-runner-test");
        let events = Arc::new(
            crate::events::RunnerEvents::new(Arc::new(client), "northstar").with_environment("dev"),
        );
        let runner = Runner::new(ops, policy(), FillFormat::Rewarded).with_events(events);
        let mut l = ledger();
        let spec = apex_core::ExperimentSpec {
            name: "apex-aa-u1".into(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "A/A calibration".into(),
            test_group_allocation: 50,
        };
        runner
            .start_aa(&mut l, apex_core::PlatformId::AppLovinMax, &spec, NOW)
            .await
            .expect("aa");
        let stored = store.load_events().await.expect("events");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].event_type.as_str(), "experiment_started");
        assert_eq!(stored[0].publisher_id.as_deref(), Some("northstar"));
        assert_eq!(
            stored[0].payload.experiment_id.as_deref(),
            Some("apex-aa-u1")
        );
        assert_eq!(stored[0].payload.state.as_deref(), Some("running"));
    }

    #[tokio::test]
    async fn the_daily_payment_pass_consumes_only_new_events() {
        let dir = std::env::temp_dir().join(format!("apex-runner-payments-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let collection = dir.join("collection.jsonl");
        let event = |psp: &str| apex_billing::CollectionEvent {
            at_unix_ms: NOW,
            event_code: "AUTHORISATION".into(),
            psp_reference: psp.into(),
            merchant_reference: "apex-fee:northstar:ch_0246:2026-10".into(),
            success: "true".into(),
            amount_value: 300,
            amount_currency: "USD".into(),
        };
        let append = |events: &[apex_billing::CollectionEvent]| {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&collection)
                .unwrap();
            for event in events {
                writeln!(file, "{}", serde_json::to_string(event).unwrap()).unwrap();
            }
        };
        append(&[event("psp_1"), event("psp_2")]);
        let ops = Arc::new(RecordingOps::new());
        let runner = Runner::new(ops, policy(), FillFormat::Rewarded)
            .with_publisher("northstar")
            .with_payment_events(collection.clone(), None, None);
        let mut l = ledger();
        let first = runner.run_daily(&mut l, NOW).await.expect("first");
        assert_eq!(first.payments.expect("payments").received, 2);
        let cursor = crate::payment_events::default_cursor_path(&collection);
        assert!(cursor.exists(), "the offset cursor advanced");
        assert_eq!(
            crate::payment_events::read_consumed_offset(&cursor).expect("cursor"),
            std::fs::metadata(&collection).expect("metadata").len()
        );

        append(&[event("psp_3")]);
        let second = runner.run_daily(&mut l, NOW + 1).await.expect("second");
        assert_eq!(
            second.payments.expect("payments").received,
            1,
            "only the new event was processed"
        );
        let third = runner.run_daily(&mut l, NOW + 2).await.expect("third");
        assert_eq!(third.payments.expect("payments").received, 0);

        // A rewound cursor re-processes the file safely (the emits are
        // dedup-keyed; consuming nothing would be the real bug).
        crate::payment_events::write_consumed_offset(&cursor, 0).expect("rewind");
        let rewound = runner.run_daily(&mut l, NOW + 3).await.expect("rewound");
        assert_eq!(rewound.payments.expect("payments").received, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A ledger wrapper that counts `records()` scans, for structural
    /// assertions that a pass does not rescan the ledger per job.
    #[derive(Debug)]
    struct CountingLedger {
        inner: InMemoryLedger,
        scans: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl apex_proof::LedgerLog for CountingLedger {
        fn append(&mut self, draft: LedgerRecord) -> Result<LedgerRecord, apex_proof::LedgerError> {
            self.inner.append(draft)
        }

        fn append_rollback(
            &mut self,
            at_unix_ms: i64,
            platform: apex_core::PlatformId,
            ad_unit_id: &str,
            rolled_back_seq: u64,
            reason: &str,
        ) -> Result<LedgerRecord, apex_proof::LedgerError> {
            self.inner
                .append_rollback(at_unix_ms, platform, ad_unit_id, rolled_back_seq, reason)
        }

        fn records(&self) -> Vec<&LedgerRecord> {
            self.scans.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.records()
        }

        fn len(&self) -> usize {
            self.inner.len()
        }

        fn get(&self, index: usize) -> Option<&LedgerRecord> {
            self.inner.get(index)
        }

        fn is_empty(&self) -> bool {
            self.inner.is_empty()
        }
    }

    #[tokio::test]
    async fn the_due_recheck_scan_runs_once_per_pass() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = std::env::temp_dir().join(format!("apex-runner-due-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (id, target) in [("opj_00000000000d", "foot"), ("opj_00000000000e", "bar")] {
            let job = crate::operator_jobs::OperatorJob {
                job_id: id.to_string(),
                kind: "recheck_request".to_string(),
                requested_by: "maya".to_string(),
                requested_at_ms: NOW,
                target: target.to_string(),
                payload: serde_json::json!({}),
                idempotency_key: format!("k-{id}"),
                state: "queued".to_string(),
                state_history: vec![],
                outcome: serde_json::Value::Null,
                correlation_id: "corr_1".to_string(),
            };
            std::fs::write(
                dir.join(format!("{id}.json")),
                serde_json::to_string_pretty(&job).unwrap(),
            )
            .unwrap();
        }

        // Baseline: a pass without jobs scans the ledger once (running
        // experiments only).
        let baseline_scans = std::sync::Arc::new(AtomicUsize::new(0));
        let mut baseline = CountingLedger {
            inner: ledger(),
            scans: baseline_scans.clone(),
        };
        let ops = Arc::new(RecordingOps::new());
        Runner::new(ops, policy(), FillFormat::Rewarded)
            .run_daily(&mut baseline, NOW)
            .await
            .expect("baseline");
        assert_eq!(baseline_scans.load(Ordering::SeqCst), 1);

        // Two recheck jobs add exactly one scan (the shared due set), never
        // one per job.
        let scans = std::sync::Arc::new(AtomicUsize::new(0));
        let mut with_jobs = CountingLedger {
            inner: ledger(),
            scans: scans.clone(),
        };
        let ops = Arc::new(RecordingOps::new());
        let runner =
            Runner::new(ops, policy(), FillFormat::Rewarded).with_operator_jobs(dir.clone());
        let report = runner.run_daily(&mut with_jobs, NOW).await.expect("daily");
        let jobs = report.operator_jobs.expect("jobs");
        assert_eq!(jobs.completed, 2, "both recheck jobs ran");
        assert_eq!(
            scans.load(Ordering::SeqCst),
            baseline_scans.load(Ordering::SeqCst) + 1,
            "the due set is computed once and reused for every job"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
