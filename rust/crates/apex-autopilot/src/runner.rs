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
        }
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
            let days = self
                .ops
                .experiment_days(&experiment.experiment_id)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?;
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

            let outcome = settle_experiment(
                self.ops.as_ref(),
                ledger,
                &experiment.experiment_id,
                experiment.change_seq,
                self.fill_format,
                now_unix_ms,
            )
            .await?;
            // A pending window is not a settlement; everything else is.
            if let SettleOutcome::Pending { .. } = &outcome {
                continue;
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

/// The last calendar day of a month, `YYYY-MM-DD`.
fn month_end(month_start: &str, month_days: u32) -> Result<String, PlanError> {
    let start = crate::live_max::day_number(month_start).ok_or_else(|| {
        PlanError::Validation(format!("month start '{month_start}' is not a valid day"))
    })?;
    Ok(crate::live_max::format_day(
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
                    kept_day: crate::live_max::format_day(at_unix_ms.div_euclid(86_400_000)),
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
                seq, experiment_id, ..
            } => {
                let change_seq = change_before(&records, *seq)?;
                if settled.contains(experiment_id) || rolled_back.contains(&change_seq) {
                    None
                } else {
                    Some(RunningExperiment {
                        experiment_id: experiment_id.clone(),
                        change_seq,
                    })
                }
            }
            _ => None,
        })
        .collect()
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

    #[tokio::test]
    async fn an_empty_ledger_is_a_no_op() {
        let runner = Runner::new(Arc::new(NullOps), policy(), FillFormat::Rewarded);
        let mut l = ledger();
        let report = runner.run_daily(&mut l, NOW).await.expect("daily");
        assert_eq!(report, DailyReport::default());
    }
}
