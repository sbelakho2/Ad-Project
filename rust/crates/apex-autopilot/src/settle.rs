//! Experiment settlement: keep only platform-proven lift; otherwise roll back.
//! The comparison is computed from the platform's own variant results; the
//! autopilot never trusts its own expected_effect_bp (it is a planning input,
//! never evidence).

use apex_core::{lift_basis_points, ExperimentResult, ExperimentState};
use apex_proof::{ChangeKind, InMemoryLedger, LedgerRecord};

use crate::keys::platform_of;
use crate::model::{Action, AutopilotPolicy, PlanError, SettleOutcome};
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

/// Keep iff the platform measured lift >= policy.min_lift_bp AND the platform's
/// confidence >= policy.min_confidence_bp; otherwise roll the change back and
/// record it. A kept change with positive lift also emits the billing proof.
pub async fn settle_experiment(
    ops: &dyn PlatformOps,
    ledger: &mut InMemoryLedger,
    policy: &AutopilotPolicy,
    publisher_id: &str,
    experiment_id: &str,
    change_seq: u64,
    now_unix_ms: i64,
) -> Result<SettleOutcome, PlanError> {
    let result = ops
        .experiment(experiment_id)
        .await
        .map_err(|source| PlanError::ApplyFailed { source })?;

    if result.state != ExperimentState::Completed {
        return Err(PlanError::PolicyViolation {
            reason: format!(
                "experiment {experiment_id} is still {:?}; never assume completion",
                result.state
            ),
        });
    }

    let (action, platform, ad_unit_id) = {
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
                ..
            } => {
                let mut action = change_to_action(change)
                    .ok_or_else(|| PlanError::Validation("unknown change kind".into()))?;
                // CountryFloorSet records do not carry the ad unit in the change
                // itself; take it from the record so rollback targets the right unit.
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

    // One settlement per experiment, ever: a repeat would append a second
    // ExperimentObserved and a second billing proof, double-billing the same
    // proven lift. Refuse before touching the platform, so a retried settle
    // cannot issue a second rollback either.
    let already_settled = ledger.records().iter().any(|record| match record {
        LedgerRecord::ExperimentObserved {
            experiment_id: e, ..
        }
        | LedgerRecord::BillingProof {
            experiment_id: e, ..
        } => e == experiment_id,
        _ => false,
    });
    if already_settled {
        return Err(PlanError::Validation(format!(
            "experiment {experiment_id} is already settled"
        )));
    }

    let (lift_bp, confidence_bp, winner_variant_id, control, winner) = evaluate_result(&result)?;

    let keep = lift_bp >= policy.min_lift_bp && confidence_bp >= policy.min_confidence_bp;
    if !keep {
        if ops.rollback(&action).await.is_err() {
            return Err(PlanError::RollbackFailed {
                reason: format!(
                    "lift {lift_bp} bp / confidence {confidence_bp} below policy and rollback failed"
                ),
            });
        }
        let rollback = ledger
            .append_rollback(
                now_unix_ms,
                platform,
                &ad_unit_id,
                change_seq,
                "below_policy",
            )
            .map_err(PlanError::Ledger)?;
        return Ok(SettleOutcome::RolledBack {
            lift_bp,
            confidence_bp,
            reason: "below_policy".into(),
            rollback_seq: rollback.seq(),
        });
    }

    ledger
        .append(LedgerRecord::experiment_observed(
            now_unix_ms,
            platform,
            experiment_id,
            result.clone(),
        ))
        .map_err(PlanError::Ledger)?;

    let (fee_micros, billing_seq) = if lift_bp > 0 && winner.revenue > control.revenue {
        let proof = apex_proof::build_billing_proof(
            publisher_id,
            experiment_id,
            control.revenue,
            winner.revenue,
            2_000,
            now_unix_ms,
        )
        .map_err(|source| PlanError::ApplyFailed { source })?;
        let fee = match &proof {
            LedgerRecord::BillingProof { fee_micros, .. } => *fee_micros,
            _ => 0,
        };
        let record = ledger.append(proof).map_err(PlanError::Ledger)?;
        (Some(fee), Some(record.seq()))
    } else {
        (None, None)
    };

    Ok(SettleOutcome::Kept {
        lift_bp,
        confidence_bp,
        winner_variant_id,
        fee_micros,
        billing_seq,
    })
}

fn evaluate_result(
    result: &ExperimentResult,
) -> Result<
    (
        i64,
        i32,
        String,
        apex_core::ExperimentVariantResult,
        apex_core::ExperimentVariantResult,
    ),
    PlanError,
> {
    let control = result
        .results
        .first()
        .cloned()
        .ok_or_else(|| PlanError::NoBaseline {
            reason: "experiment has no control (first) variant".into(),
        })?;
    if control.ecpm <= 0 {
        return Err(PlanError::NoBaseline {
            reason: format!("control ecpm {} is not positive", control.ecpm),
        });
    }
    let winner = match &result.winner_variant_id {
        Some(id) => result
            .results
            .iter()
            .find(|variant| &variant.variant_id == id)
            .cloned()
            .ok_or_else(|| PlanError::NoBaseline {
                reason: format!("winner variant '{id}' is not among the results"),
            })?,
        None => result
            .results
            .iter()
            .filter(|variant| variant.variant_id != control.variant_id)
            .max_by(|a, b| {
                a.ecpm
                    .cmp(&b.ecpm)
                    .then_with(|| b.variant_id.cmp(&a.variant_id))
            })
            .cloned()
            .ok_or_else(|| PlanError::NoBaseline {
                reason: "experiment has only the control variant".into(),
            })?,
    };
    let lift_bp =
        lift_basis_points(control.ecpm, winner.ecpm).ok_or_else(|| PlanError::NoBaseline {
            reason: "lift is undefined for a non-positive control".into(),
        })?;
    let confidence_bp = result.confidence.unwrap_or(0);
    Ok((
        lift_bp,
        confidence_bp,
        winner.variant_id.clone(),
        control,
        winner,
    ))
}
