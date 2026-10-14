//! Experiment settlement through the Apex Lift Engine.
//!
//! The engine measures lift as the mean daily log ratio of revenue per user,
//! bounds it with a one-sided 90% (or 99% interim) t quantile, enforces the
//! eight guardrails, and applies the six keep conditions. Billable lift is
//! the lower bound, never the point estimate. The platform's own reporting is
//! the only data source.

use apex_lift::decision::{decide, Decision, DecisionInput, PreRegistration};
use apex_lift::error::LiftError;
use apex_lift::guardrails::{evaluate, FillFormat};
use apex_lift::window::{build_window, incident_runs};
use apex_proof::{ChangeKind, InMemoryLedger, LedgerRecord};

use crate::keys::platform_of;
use crate::model::{Action, PlanError, SettleOutcome};
use crate::ops::PlatformOps;

/// Rebuild the action a change record applied (used to roll back on settle).
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

/// Settle one experiment through the Apex Lift Engine.
///
/// The engine measures the daily log ratio of revenue per user from the
/// platform's reporting pulls, enforces the guardrails, applies the six keep
/// conditions and returns the decision. Only a `Keep` appends records that
/// carry a billable lambda; rollbacks and voids never bill.
#[allow(clippy::too_many_arguments)]
pub async fn settle_experiment(
    ops: &dyn PlatformOps,
    ledger: &mut InMemoryLedger,
    prereg: &PreRegistration,
    fill_format: FillFormat,
    experiment_id: &str,
    change_seq: u64,
    now_unix_ms: i64,
) -> Result<SettleOutcome, PlanError> {
    let result = ops
        .experiment(experiment_id)
        .await
        .map_err(|source| PlanError::ApplyFailed { source })?;

    if result.state != apex_core::ExperimentState::Completed {
        return Err(PlanError::PolicyViolation {
            reason: format!(
                "experiment {experiment_id} is still {:?}; never assume completion",
                result.state
            ),
        });
    }

    let (action, platform, ad_unit_id, prev_value) = {
        let records = ledger.records();
        let record = records
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
                prev_value,
                ..
            } => {
                let mut action = change_to_action(change)
                    .ok_or_else(|| PlanError::Validation("unknown change kind".into()))?;
                if let Action::SetCountryFloor { ad_unit_id: a, .. } = &mut action {
                    *a = ad_unit_id.clone();
                }
                (action, *platform, ad_unit_id.clone(), prev_value.clone())
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

    let already_settled = ledger.records().iter().any(|record| {
        matches!(
            record,
            LedgerRecord::ExperimentObserved { experiment_id: e, .. } if e == experiment_id
        ) || matches!(
            record,
            LedgerRecord::BillingProof { experiment_id: e, .. } if e == experiment_id
        ) || matches!(
            record,
            LedgerRecord::LiftRecord { payload, .. } if payload.contains(experiment_id)
        )
    });
    if already_settled {
        return Err(PlanError::Validation(format!(
            "experiment {experiment_id} is already settled"
        )));
    }

    let days = ops
        .experiment_days(experiment_id)
        .await
        .map_err(|source| PlanError::ApplyFailed { source })?;

    // Build the pre-registration from the policy, the applied change's prior
    // value, and the engine's analysis version. In production this record is
    // written before the first platform call; at settle it is re-emitted so
    // the ledger carries the full sequence.
    let prereg = PreRegistration {
        experiment_id: experiment_id.to_string(),
        scope: ad_unit_id.clone(),
        change_description: prereg.change_description.clone(),
        before_value: prev_value,
        test_share: prereg.test_share,
        metric: apex_lift::decision::Metric::RevenuePerUser,
        burn_in_days: prereg.burn_in_days,
        window_days: prereg.window_days,
        interim_look: prereg.interim_look,
        extension_allowed: prereg.extension_allowed,
        code_version_hash: prereg.code_version_hash.clone(),
        rate_bp: prereg.rate_bp,
        thresholds: prereg.thresholds.clone(),
    };

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
    let guardrails = evaluate(
        &prereg,
        &window.days,
        &arm_counts,
        fill_format,
        &[],
        0,
        &incidents,
    );

    let ratios: Vec<f64> = window.days.iter().map(|d| d.log_ratio).collect();
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
        Decision::Rollback { reason } => {
            if ops.rollback(&action).await.is_err() {
                return Err(PlanError::RollbackFailed {
                    reason: format!("rollback failed: {reason}"),
                });
            }
            let rollback = ledger
                .append_rollback(
                    now_unix_ms,
                    platform,
                    &ad_unit_id,
                    change_seq,
                    &format!("lift engine: {reason}"),
                )
                .map_err(PlanError::Ledger)?;
            return Ok(SettleOutcome::RolledBack {
                lift: estimate_lift(&ratios).unwrap_or(0.0),
                reason: reason.clone(),
                rollback_seq: rollback.seq(),
            });
        }
        Decision::Void { reason } => {
            return Ok(SettleOutcome::Voided {
                reason: reason.clone(),
            });
        }
        Decision::Pending { reason } => {
            return Ok(SettleOutcome::Pending {
                reason: reason.clone(),
            });
        }
        _ => {}
    }

    let Decision::Keep {
        lambda,
        estimate,
        bootstrap_lower,
        ..
    } = decision
    else {
        // Extend was already handled above as a Pending outcome via the
        // final-look path; any other non-keep decision is an engine bug.
        return Err(PlanError::Validation(
            "decide returned a non-keep decision after the keep match".into(),
        ));
    };

    ledger
        .append(LedgerRecord::experiment_observed(
            now_unix_ms,
            platform,
            experiment_id,
            result.clone(),
        ))
        .map_err(PlanError::Ledger)?;

    // Emit the signed lift-engine result record into the ledger.
    let result_record =
        apex_lift::emit::result(&prereg, &decision, &estimate, ratios, bootstrap_lower)
            .map_err(lift_to_plan)?;
    let payload = result_record
        .record
        .canonical_json()
        .map_err(lift_to_plan)?;
    let sha = result_record.sha256.clone();
    let lift_entry = ledger
        .append(LedgerRecord::lift_record(now_unix_ms, &sha, &payload))
        .map_err(PlanError::Ledger)?;

    Ok(SettleOutcome::Kept {
        experiment_id: experiment_id.to_string(),
        lift: estimate.lift,
        lambda,
        lower_90: estimate.lower_90,
        bootstrap_lower,
        result_seq: lift_entry.seq(),
    })
}

fn estimate_lift(ratios: &[f64]) -> Option<f64> {
    apex_lift::stats::estimate(ratios).map(|e| e.lift)
}

fn lift_to_plan(error: LiftError) -> PlanError {
    PlanError::Validation(format!("lift engine: {error}"))
}
