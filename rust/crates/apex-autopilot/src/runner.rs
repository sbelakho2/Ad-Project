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
use apex_proof::{LedgerLog, LedgerRecord};

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
}

/// The runner holds the platform adapter and the policy; the ledger is passed
/// per job so the same runner serves any publisher's ledger.
#[derive(Clone)]
pub struct Runner {
    ops: Arc<dyn PlatformOps>,
    policy: AutopilotPolicy,
    fill_format: FillFormat,
    alerts: Arc<dyn AlertSink>,
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
        }
    }

    /// Attach the alert sink that receives customer-impact alerts.
    #[must_use]
    pub fn with_alerts(mut self, alerts: Arc<dyn AlertSink>) -> Self {
        self.alerts = alerts;
        self
    }

    /// One daily pass: pull reporting for every running experiment, record the
    /// pull, and settle any whose window has completed.
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
                    return Err(PlanError::ApplyFailed { source });
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
                    return Err(PlanError::RollbackFailed { reason });
                }
                Err(other) => return Err(other),
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
            report.settled += 1;
            report
                .outcomes
                .push((experiment.experiment_id.clone(), status_label(&outcome)));
        }
        Ok(report)
    }

    /// The policy this runner applies (exposed so callers can pin it in logs).
    pub fn policy(&self) -> &AutopilotPolicy {
        &self.policy
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
}

impl Runner {
    /// One monthly billing pass: bill every kept change still in force for
    /// `month`, from the platform-reported revenue of its pre-registered
    /// scope. A change past its term or covered by a publisher override is
    /// skipped by [`apex_lift::run::run_month`].
    pub async fn run_monthly(
        &self,
        ledger: &mut dyn LedgerLog,
        month: &str,
        month_start: &str,
        month_days: u32,
        now_unix_ms: i64,
    ) -> Result<MonthlyReport, PlanError> {
        let month_end = month_end(month_start, month_days)?;
        let mut by_scope: std::collections::BTreeMap<String, Vec<apex_lift::billing::KeptChange>> =
            std::collections::BTreeMap::new();
        for (scope, change) in kept_changes(ledger) {
            by_scope.entry(scope).or_default().push(change);
        }
        let mut report = MonthlyReport {
            month: month.to_string(),
            ..MonthlyReport::default()
        };
        for (scope, changes) in by_scope {
            let revenue = self
                .ops
                .scope_revenue(&scope, month_start, &month_end)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?;
            let run = apex_lift::run::run_month(month, month_start, month_days, revenue, &changes)
                .map_err(|error| PlanError::Validation(format!("monthly billing: {error}")))?;
            for fee in &run.fees {
                let record = apex_lift::emit::monthly_billing(fee, month)
                    .map_err(|error| PlanError::Validation(format!("monthly billing: {error}")))?;
                let payload = record
                    .record
                    .canonical_json()
                    .map_err(|error| PlanError::Validation(format!("monthly billing: {error}")))?;
                ledger
                    .append(LedgerRecord::lift_record(
                        now_unix_ms,
                        &record.sha256,
                        &payload,
                    ))
                    .map_err(PlanError::Ledger)?;
                report.fees.push((fee.change_id.clone(), fee.fee_micros));
            }
            report.billed_scopes += 1;
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
        assert!(runner.run_daily(&mut l, NOW + 1).await.is_err());
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
            .run_monthly(&mut l, "2023-11", "2023-11-01", 30, NOW + 2)
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
            .run_monthly(&mut l, "2023-11", "2023-11-01", 30, NOW + 2)
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
