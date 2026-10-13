//! The write cycle: propose -> policy gates -> apply -> mandatory experiment.
//! Daily accounting reads the signed ledger, never caller state.

use apex_audit::{Finding, Severity};
use apex_proof::{ChangeKind, InMemoryLedger, LedgerRecord};

use crate::keys::{experiment_name, idempotency_key, operation_of, platform_of};
use crate::model::{Action, ActionOutcome, AutopilotPolicy, CycleOutcome, PlanError, Proposal};
use crate::ops::PlatformOps;

const DAY_MS: i64 = 86_400_000;

fn utc_day(at_unix_ms: i64) -> i64 {
    at_unix_ms.div_euclid(DAY_MS)
}

fn change_kind_of(action: &Action) -> ChangeKind {
    match action {
        Action::SetCountryFloor {
            country,
            floor_ecpm,
            ..
        } => ChangeKind::CountryFloorSet {
            country: country.clone(),
            floor_ecpm: *floor_ecpm,
        },
        Action::SetMappingEcpm {
            mediation_group_id,
            ad_unit_id,
            ecpm,
        } => ChangeKind::EcpmSet {
            mediation_group_id: mediation_group_id.clone(),
            ad_unit_id: ad_unit_id.clone(),
            ecpm: *ecpm,
        },
        Action::SetWaterfallPrice {
            ad_unit_id,
            entry_id,
            price_ecpm,
        } => ChangeKind::WaterfallEntryPriceSet {
            ad_unit_id: ad_unit_id.clone(),
            entry_id: entry_id.clone(),
            price_ecpm: *price_ecpm,
        },
        Action::CreateWaterfallEntry {
            ad_unit_id,
            network,
            instance_name,
            price_ecpm,
        } => ChangeKind::WaterfallEntryCreated {
            ad_unit_id: ad_unit_id.clone(),
            network: network.clone(),
            instance_name: instance_name.clone(),
            price_ecpm: *price_ecpm,
        },
        Action::RemoveWaterfallEntry {
            ad_unit_id,
            entry_id,
        } => ChangeKind::WaterfallEntryRemoved {
            ad_unit_id: ad_unit_id.clone(),
            entry_id: entry_id.clone(),
        },
    }
}

/// Bound the action's own values before any side effect: the step cap bounds
/// the caller's estimate, and this bounds the actual write.
fn validate_action_values(action: &Action) -> Result<(), PlanError> {
    match action {
        Action::SetCountryFloor {
            country,
            floor_ecpm,
            ..
        } => {
            apex_core::normalize_country(country)
                .map_err(|e| PlanError::Validation(e.to_string()))?;
            apex_core::validate_floor(*floor_ecpm)
                .map_err(|e| PlanError::Validation(e.to_string()))?;
        }
        Action::SetMappingEcpm { ecpm, .. } => {
            apex_core::validate_ecpm(*ecpm).map_err(|e| PlanError::Validation(e.to_string()))?;
        }
        Action::SetWaterfallPrice { price_ecpm, .. } => {
            apex_core::validate_ecpm(*price_ecpm)
                .map_err(|e| PlanError::Validation(e.to_string()))?;
        }
        Action::CreateWaterfallEntry {
            network,
            instance_name,
            price_ecpm,
            ..
        } => {
            if network.trim().is_empty() || instance_name.trim().is_empty() {
                return Err(PlanError::Validation(
                    "network and instanceName are required".into(),
                ));
            }
            apex_core::validate_ecpm(*price_ecpm)
                .map_err(|e| PlanError::Validation(e.to_string()))?;
        }
        Action::RemoveWaterfallEntry { .. } => {}
    }
    Ok(())
}

/// True when the same change (equal idempotency key and equal payload)
/// is already in the ledger; a rerun must never re-apply it, and a rerun with
/// a different value under a colliding key must never be mistaken for it.
pub fn already_applied(ledger: &InMemoryLedger, key: &str, action: &Action) -> bool {
    let expected = change_kind_of(action);
    let rolled_back: std::collections::HashSet<u64> = ledger
        .records()
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::RollbackPerformed {
                rolled_back_seq, ..
            } => Some(*rolled_back_seq),
            _ => None,
        })
        .collect();
    ledger.records().iter().any(|record| match record {
        LedgerRecord::ChangeApplied {
            seq,
            idempotency_key,
            change,
            ..
        } => idempotency_key == key && *change == expected && !rolled_back.contains(seq),
        _ => false,
    })
}

/// Run one write cycle. All gates run before any side effect of the gated step:
/// a refused proposal costs zero platform calls.
pub async fn run_cycle(
    ops: &dyn PlatformOps,
    ledger: &mut InMemoryLedger,
    policy: &AutopilotPolicy,
    findings: &[Finding],
    proposals: Vec<Proposal>,
    now_unix_ms: i64,
) -> Result<CycleOutcome, PlanError> {
    if policy.blackout_on_critical_findings {
        let criticals: Vec<&str> = findings
            .iter()
            .filter(|finding| finding.severity == Severity::Critical)
            .map(|finding| finding.subject.as_str())
            .collect();
        if !criticals.is_empty() {
            return Err(PlanError::PolicyViolation {
                reason: format!(
                    "blackout: {} critical audit result(s): {}",
                    criticals.len(),
                    criticals.join(", ")
                ),
            });
        }
    }

    let day = utc_day(now_unix_ms);
    let rolled_back: std::collections::HashSet<u64> = ledger
        .records()
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::RollbackPerformed {
                rolled_back_seq, ..
            } => Some(*rolled_back_seq),
            _ => None,
        })
        .collect();
    // Clock skew must not consume the day: only records at or before `now`
    // count, and a change that was rolled back frees its slot for a retry.
    let mut applied_today: u32 = ledger
        .records()
        .iter()
        .filter(|record| match record {
            LedgerRecord::ChangeApplied { at_unix_ms, .. } => {
                utc_day(*at_unix_ms) == day
                    && *at_unix_ms <= now_unix_ms
                    && !rolled_back.contains(&record.seq())
            }
            _ => false,
        })
        .count()
        .try_into()
        .map_err(|_| PlanError::Validation("applied count overflow".into()))?;

    if applied_today >= policy.max_actions_per_day {
        return Err(PlanError::DailyCapReached { n: applied_today });
    }

    let mut outcomes = Vec::with_capacity(proposals.len());
    let mut applied_this_cycle: u32 = 0;
    let mut skipped_this_cycle: u32 = 0;

    for proposal in proposals {
        if applied_today >= policy.max_actions_per_day {
            skipped_this_cycle += 1;
            outcomes.push(ActionOutcome::Skipped {
                reason: "daily_cap_reached".into(),
            });
            continue;
        }

        if proposal.expected_effect_bp <= 0 || proposal.expected_effect_bp > policy.max_step_bp {
            return Err(PlanError::PolicyViolation {
                reason: format!(
                    "expected effect {} bp outside (0, {}]",
                    proposal.expected_effect_bp, policy.max_step_bp
                ),
            });
        }

        validate_action_values(&proposal.action)?;
        let key = idempotency_key(&proposal.action)
            .map_err(|source| PlanError::Validation(source.to_string()))?;
        if already_applied(ledger, &key, &proposal.action) {
            skipped_this_cycle += 1;
            outcomes.push(ActionOutcome::Skipped {
                reason: "already_applied".into(),
            });
            continue;
        }

        let operation = operation_of(&proposal.action);
        if !ops.writable(operation).await {
            return Err(PlanError::EndpointNotWritable {
                operation: operation.to_string(),
            });
        }

        if let Err(source) = ops.apply(&proposal.action, &key).await {
            // The failure surfaces with the typed platform error; nothing was
            // recorded in the ledger for this action.
            return Err(PlanError::ApplyFailed { source });
        }

        let change = ledger
            .append(LedgerRecord::change_applied(
                now_unix_ms,
                platform_of(&proposal.action),
                proposal.action.ad_unit_id(),
                change_kind_of(&proposal.action),
                &key,
            ))
            .map_err(PlanError::Ledger)?;
        let change_seq = change.seq();
        applied_today += 1;
        applied_this_cycle += 1;

        // A change never ships without its platform-native experiment.
        let spec = apex_core::ExperimentSpec {
            name: experiment_name(&proposal.action),
            ad_unit_id: proposal.action.ad_unit_id().to_string(),
            metric: apex_core::ExperimentMetric::Ecpm,
            variants: 2,
            change_description: proposal.rationale.clone(),
        };
        match ops.start_experiment(&spec).await {
            Ok(experiment_id) => {
                ledger
                    .append(LedgerRecord::experiment_started(
                        now_unix_ms,
                        platform_of(&proposal.action),
                        &experiment_id,
                        spec,
                    ))
                    .map_err(PlanError::Ledger)?;
                outcomes.push(ActionOutcome::Applied {
                    change_seq,
                    experiment_id,
                    idempotency_key: key,
                });
            }
            Err(source) => {
                if ops.rollback(&proposal.action).await.is_err() {
                    return Err(PlanError::RollbackFailed {
                        reason: format!(
                            "experiment start failed ({source}) and rollback also failed for {key}"
                        ),
                    });
                }
                ledger
                    .append_rollback(
                        now_unix_ms,
                        platform_of(&proposal.action),
                        proposal.action.ad_unit_id(),
                        change_seq,
                        "experiment_start_failed",
                    )
                    .map_err(PlanError::Ledger)?;
                outcomes.push(ActionOutcome::RolledBack {
                    change_seq,
                    reason: "experiment_start_failed".into(),
                });
            }
        }
    }

    Ok(CycleOutcome {
        outcomes,
        applied: applied_this_cycle,
        skipped: skipped_this_cycle,
    })
}
