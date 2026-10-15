//! Experiment settlement through the Apex Lift Engine.
//!
//! The engine measures lift as the mean daily log ratio of revenue per user,
//! bounds it with a one-sided 90% (or 99% interim) t quantile, enforces the
//! eight guardrails, and applies the six keep conditions. Billable lift is
//! the lower bound, never the point estimate. The platform's own reporting is
//! the only data source.
//!
//! The pre-registration is **not** rebuilt here: settlement reads the signed
//! record that was written before the test's first platform call, so a test
//! can only ever be judged against what it pre-registered. A kept change is
//! promoted to all users; a rollback or a void deprecates the experiment.

use apex_lift::decision::{decide, Decision, DecisionInput, PreRegistration};
use apex_lift::guardrails::FillFormat;
use apex_lift::window::{build_window, incident_runs};
use apex_proof::{ChangeKind, LedgerLog, LedgerRecord};

use crate::keys::platform_of;
use crate::model::{Action, PlanError, SettleOutcome};
use crate::ops::PlatformOps;

/// Rebuild the action a change record applied (used to roll back the live unit
/// on the adapters that apply a change directly).
fn change_to_action(change: &ChangeKind) -> Option<Action> {
    match change {
        ChangeKind::CountryFloorSet {
            country,
            floor_ecpm,
        } => Some(Action::SetCountryFloor {
            ad_unit_id: String::new(),
            country: country.clone(),
            floor_ecpm: *floor_ecpm,
        }),
        ChangeKind::EcpmSet {
            mediation_group_id,
            ad_unit_id,
            ecpm,
        } => Some(Action::SetMappingEcpm {
            mediation_group_id: mediation_group_id.clone(),
            ad_unit_id: ad_unit_id.clone(),
            ecpm: *ecpm,
        }),
        ChangeKind::WaterfallEntryPriceSet {
            ad_unit_id,
            entry_id,
            price_ecpm,
        } => Some(Action::SetWaterfallPrice {
            ad_unit_id: ad_unit_id.clone(),
            entry_id: entry_id.clone(),
            price_ecpm: *price_ecpm,
        }),
        ChangeKind::WaterfallEntryCreated {
            ad_unit_id,
            network,
            instance_name,
            price_ecpm,
        } => Some(Action::CreateWaterfallEntry {
            ad_unit_id: ad_unit_id.clone(),
            network: network.clone(),
            instance_name: instance_name.clone(),
            price_ecpm: *price_ecpm,
        }),
        ChangeKind::WaterfallEntryRemoved {
            ad_unit_id,
            entry_id,
        } => Some(Action::RemoveWaterfallEntry {
            ad_unit_id: ad_unit_id.clone(),
            entry_id: entry_id.clone(),
        }),
    }
}

fn parse_lift_record(payload: &str) -> Option<apex_lift::records::LiftRecord> {
    serde_json::from_str(payload).ok()
}

/// The experiment name pinned by the `experimentStarted` record.
fn started_experiment_name(ledger: &dyn LedgerLog, experiment_id: &str) -> Option<String> {
    ledger.records().iter().find_map(|record| match record {
        LedgerRecord::ExperimentStarted {
            experiment_id: e,
            spec,
            ..
        } if e == experiment_id => Some(spec.name.clone()),
        _ => None,
    })
}

/// The signed pre-registration for `experiment_id` (the platform id), read
/// back from the ledger.
fn signed_pre_registration(
    ledger: &dyn LedgerLog,
    experiment_id: &str,
) -> Result<PreRegistration, PlanError> {
    let name =
        started_experiment_name(ledger, experiment_id).ok_or_else(|| PlanError::NoBaseline {
            reason: format!("experiment {experiment_id} was never started in this ledger"),
        })?;
    ledger
        .records()
        .iter()
        .find_map(|record| match record {
            LedgerRecord::LiftRecord { payload, .. } => {
                let parsed = parse_lift_record(payload)?;
                let prereg = PreRegistration::from_record(&parsed).ok()?;
                (prereg.experiment_id == name).then_some(prereg)
            }
            _ => None,
        })
        .ok_or(PlanError::NoBaseline {
            reason: format!("no signed pre-registration for experiment {name}"),
        })
}

/// Settle one experiment through the Apex Lift Engine.
#[allow(clippy::too_many_arguments)]
pub async fn settle_experiment(
    ops: &dyn PlatformOps,
    ledger: &mut dyn LedgerLog,
    experiment_id: &str,
    change_seq: u64,
    fill_format: FillFormat,
    now_unix_ms: i64,
) -> Result<SettleOutcome, PlanError> {
    let result = ops
        .experiment(experiment_id)
        .await
        .map_err(|source| PlanError::ApplyFailed { source })?;
    if result.state == apex_core::ExperimentState::Stopped {
        return Err(PlanError::PolicyViolation {
            reason: format!("experiment {experiment_id} has already ended"),
        });
    }

    let (action, platform, ad_unit_id) = {
        let record = ledger
            .records()
            .into_iter()
            .find(|record| record.seq() == change_seq)
            .ok_or_else(|| PlanError::NoBaseline {
                reason: format!("change sequence {change_seq} is not in the ledger"),
            })?;
        match record {
            LedgerRecord::ChangeApplied {
                change,
                platform,
                ad_unit_id,
                ..
            } => {
                let mut action = change_to_action(change)
                    .ok_or_else(|| PlanError::Validation("unknown change kind".into()))?;
                if let Action::SetCountryFloor { ad_unit_id: a, .. } = &mut action {
                    *a = ad_unit_id.clone();
                }
                (action, *platform, ad_unit_id.clone())
            }
            _ => {
                return Err(PlanError::NoBaseline {
                    reason: format!("sequence {change_seq} is not a ChangeApplied record"),
                })
            }
        }
    };
    if platform != platform_of(&action) {
        return Err(PlanError::Validation(
            "ledger platform does not match the action's platform".into(),
        ));
    }

    let already_settled = ledger.records().iter().any(|record| match record {
        LedgerRecord::ExperimentObserved {
            experiment_id: e, ..
        } => e == experiment_id,
        LedgerRecord::RollbackPerformed { rolled_back_seq, .. } => *rolled_back_seq == change_seq,
        LedgerRecord::LiftRecord { payload, .. } => parse_lift_record(payload)
            .is_some_and(|r| matches!(r, apex_lift::records::LiftRecord::Result { experiment_id: e, .. } if e == started_experiment_name(ledger, experiment_id).as_deref().unwrap_or_default())),
        _ => false,
    });
    if already_settled {
        return Err(PlanError::Validation(format!(
            "experiment {experiment_id} is already settled"
        )));
    }

    let prereg = signed_pre_registration(ledger, experiment_id)?;

    let days = ops
        .experiment_days(experiment_id)
        .await
        .map_err(|source| PlanError::ApplyFailed { source })?;

    let window = build_window(
        &days.control,
        &days.test,
        prereg.window_days,
        prereg.test_share,
    );
    let incidents = incident_runs(&days.control, &days.test);
    let arm_counts: Vec<(u64, u64)> = window
        .days
        .iter()
        .enumerate()
        .filter_map(|(i, _)| {
            let c = days.control.get(i + 3)?;
            let t = days.test.get(i + 3)?;
            Some((t.users?, c.users?))
        })
        .collect();
    let guardrails = apex_lift::guardrails::evaluate(
        &prereg,
        &window.days,
        &arm_counts,
        fill_format,
        &[],
        0,
        &incidents,
    );

    let ratios: Vec<f64> = window.days.iter().map(|d| d.log_ratio).collect();
    let estimate = apex_lift::stats::estimate(&ratios);
    let input = DecisionInput {
        window: &window,
        guardrails: &guardrails,
        observed_scope: prereg.scope.clone(),
        observed_change_description: prereg.change_description.clone(),
        at_final_look: window.complete,
        extension_used: false,
        bootstrap_iterations: 2000,
        bootstrap_block_len: 7,
        bootstrap_seed: now_unix_ms as u64,
    };
    let decision = decide(&prereg, &input);

    match &decision {
        Decision::Pending { reason } => {
            return Ok(SettleOutcome::Pending {
                reason: reason.clone(),
            });
        }
        Decision::Extend { reason } => {
            return Ok(SettleOutcome::Pending {
                reason: format!("extension offered: {reason}"),
            });
        }
        Decision::Void { reason } => {
            ops.deprecate_experiment(experiment_id)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?;
            return Ok(SettleOutcome::Voided {
                reason: reason.clone(),
            });
        }
        Decision::Rollback { reason } => {
            ops.deprecate_experiment(experiment_id)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?;
            if !ops.change_in_test_arm() {
                ops.rollback(&action)
                    .await
                    .map_err(|source| PlanError::ApplyFailed { source })?;
            }
            let rollback = ledger
                .append_rollback(now_unix_ms, platform, &ad_unit_id, change_seq, reason)
                .map_err(PlanError::Ledger)?;
            return Ok(SettleOutcome::RolledBack {
                lift: estimate.as_ref().map(|e| e.lift).unwrap_or(0.0),
                reason: reason.clone(),
                rollback_seq: rollback.seq(),
            });
        }
        Decision::Keep { .. } => {}
    }

    let Decision::Keep {
        lambda,
        estimate: keep_estimate,
        bootstrap_lower,
        ..
    } = decision
    else {
        return Err(PlanError::Validation(
            "decide returned a non-keep decision after the keep match".into(),
        ));
    };

    ops.promote_experiment(experiment_id)
        .await
        .map_err(|source| PlanError::ApplyFailed { source })?;

    ledger
        .append(LedgerRecord::experiment_observed(
            now_unix_ms,
            platform,
            experiment_id,
            result.clone(),
        ))
        .map_err(PlanError::Ledger)?;

    let signed =
        apex_lift::emit::result(&prereg, &decision, &keep_estimate, ratios, bootstrap_lower)
            .map_err(|error| PlanError::Validation(format!("result record: {error}")))?;
    let payload = signed
        .record
        .canonical_json()
        .map_err(|error| PlanError::Validation(format!("result record: {error}")))?;
    let result_entry = ledger
        .append(LedgerRecord::lift_record(
            now_unix_ms,
            &signed.sha256,
            &payload,
        ))
        .map_err(PlanError::Ledger)?;

    Ok(SettleOutcome::Kept {
        experiment_id: experiment_id.to_string(),
        lift: keep_estimate.lift,
        lambda,
        lower_90: keep_estimate.lower_90,
        bootstrap_lower,
        result_seq: result_entry.seq(),
    })
}
