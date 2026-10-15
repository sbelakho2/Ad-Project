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
    /// Settle outcomes in ledger order (experiment id, status label).
    pub outcomes: Vec<(String, String)>,
    /// Experiments that could not be processed (id, reason). The pass
    /// continues past a failure so one bad test never starves the rest.
    pub failures: Vec<(String, String)>,
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

    /// One daily pass: pull reporting for every running experiment, record the
    /// pull, and settle any whose window has completed. A failure on one
    /// experiment is recorded (and alerted) without starving the rest.
    pub async fn run_daily(
        &self,
        ledger: &mut dyn LedgerLog,
        now_unix_ms: i64,
    ) -> Result<DailyReport, PlanError> {
        let mut report = DailyReport::default();
        for experiment in running_experiments(ledger) {
            let days = match self.ops.experiment_days(&experiment.experiment_id).await {
                Ok(days) => days,
                Err(source) => {
                    self.alert_pull_failure(&experiment.experiment_id, &source)
                        .await;
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
                continue;
            }
            match &outcome {
                SettleOutcome::RolledBack { reason, .. } | SettleOutcome::Voided { reason } => {
                    self.alerts
                        .emit(Alert::GuardrailStop {
                            experiment_id: experiment.experiment_id.clone(),
                            detail: reason.clone(),
                        })
                        .await;
                }
                _ => {}
            }
            self.learn(&experiment, ledger, &outcome).await;
            report.settled += 1;
            report
                .outcomes
                .push((experiment.experiment_id.clone(), status_label(&outcome)));
        }
        Ok(report)
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

        // The invoice is re-derived from the signed billing records; the flat
        // monitoring plan is billed as its own line on the same invoice.
        let mut invoice =
            apex_billing::compute_invoice_from_basis(publisher_id, period.clone(), &basis)
                .map_err(|error| PlanError::Validation(format!("invoice: {error}")))?;
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

        if let Some((client, transport)) = &self.adyen {
            let receipt = apex_billing::create_payment_links(client, transport.as_ref(), &invoice)
                .await
                .map_err(|error| PlanError::Validation(format!("collection: {error}")))?;
            report.payment_links = receipt.links.iter().map(|link| link.url.clone()).collect();
            report.collected = true;
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
}
