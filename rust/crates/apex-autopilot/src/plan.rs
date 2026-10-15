//! The write cycle: propose -> policy gates -> apply -> mandatory experiment.
//! Daily accounting reads the signed ledger, never caller state.

use apex_audit::{Finding, Severity};
use apex_proof::{ChangeKind, LedgerLog, LedgerRecord};

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
    // Every action must name a real ad unit; a blank scope would mint an
    // idempotency key and a platform path that address nothing.
    if action.ad_unit_id().trim().is_empty() {
        return Err(PlanError::Validation(
            "the action must name an ad unit".into(),
        ));
    }
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
        Action::RemoveWaterfallEntry { entry_id, .. } => {
            if entry_id.trim().is_empty() {
                return Err(PlanError::Validation(
                    "entryId is required to remove a waterfall entry".into(),
                ));
            }
        }
    }
    Ok(())
}

/// True when the same change (equal idempotency key and equal payload)
/// is already in the ledger; a rerun must never re-apply it, and a rerun with
/// a different value under a colliding key must never be mistaken for it.
pub fn already_applied(ledger: &dyn LedgerLog, key: &str, action: &Action) -> bool {
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

/// The experiment id inside a signed pre-registration payload, when the
/// payload is in fact a pre-registration.
fn prereg_experiment_id(payload: &str) -> Option<String> {
    match serde_json::from_str::<apex_lift::records::LiftRecord>(payload) {
        Ok(apex_lift::records::LiftRecord::PreRegistration { experiment_id, .. }) => {
            Some(experiment_id)
        }
        _ => None,
    }
}

/// True when the ledger already carries the signed pre-registration for
/// `experiment_id` (a rerun must not stack duplicate registrations).
pub fn pre_registration_exists(ledger: &dyn LedgerLog, experiment_id: &str) -> bool {
    ledger.records().iter().any(|record| match record {
        LedgerRecord::LiftRecord { payload, .. } => {
            prereg_experiment_id(payload).as_deref() == Some(experiment_id)
        }
        _ => false,
    })
}

/// The ledger sequence of the signed pre-registration for `experiment_id`.
fn pre_registration_seq(ledger: &dyn LedgerLog, experiment_id: &str) -> u64 {
    ledger
        .records()
        .iter()
        .find_map(|record| match record {
            LedgerRecord::LiftRecord { seq, payload, .. }
                if prereg_experiment_id(payload).as_deref() == Some(experiment_id) =>
            {
                Some(*seq)
            }
            _ => None,
        })
        .unwrap_or(0)
}

/// Run one write cycle. All gates run before any side effect of the gated step:
/// a refused proposal costs zero platform calls.
pub async fn run_cycle(
    ops: &dyn PlatformOps,
    ledger: &mut dyn LedgerLog,
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
    let mut awaiting_approval: u32 = 0;

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

        // Approval mode: report the proposal and stop before any write or
        // pre-registration. On for a customer's first 30 days.
        if policy.approval_required(now_unix_ms) {
            awaiting_approval += 1;
            outcomes.push(ActionOutcome::AwaitingApproval {
                idempotency_key: key,
            });
            continue;
        }

        // 1. Pre-registration, signed into the ledger before the first
        //    platform call. The test is judged only against what was pinned
        //    before it started; the split comes from policy, never a fixed 50.
        let spec = apex_core::ExperimentSpec {
            test_group_allocation: policy.max_allocation(),
            name: experiment_name(&proposal.action),
            ad_unit_id: proposal.action.ad_unit_id().to_string(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: proposal.rationale.clone(),
        };
        let prereg = apex_lift::decision::PreRegistration {
            experiment_id: spec.name.clone(),
            scope: proposal.action.ad_unit_id().to_string(),
            change_description: proposal.rationale.clone(),
            before_value: proposal.prev_value.clone(),
            test_share: policy.test_share,
            metric: apex_lift::decision::Metric::RevenuePerUser,
            burn_in_days: policy.burn_in_days,
            window_days: policy.window_days,
            interim_look: policy.interim_look,
            extension_allowed: policy.extension_allowed,
            code_version_hash: policy.code_version_hash.clone(),
            rate_bp: policy.rate_bp,
            thresholds: policy.thresholds.clone(),
        };
        // A pre-registration already on the ledger with no change record for
        // this key means a prior attempt may have started the experiment
        // before its bookkeeping was written. Refuse to start a second one;
        // the operator investigates. A resolved attempt (a change record
        // exists, whether live or rolled back) does not block a retry.
        let prior_change_exists = ledger.records().iter().any(|record| {
            matches!(
                record,
                LedgerRecord::ChangeApplied { idempotency_key, .. } if idempotency_key == &key
            )
        });
        if pre_registration_exists(ledger, &spec.name) && !prior_change_exists {
            skipped_this_cycle += 1;
            outcomes.push(ActionOutcome::Skipped {
                reason: "experiment_in_flight".into(),
            });
            continue;
        }

        {
            let signed = apex_lift::emit::pre_registration(&prereg)
                .map_err(|error| PlanError::Validation(format!("pre-registration: {error}")))?;
            let payload = signed
                .record
                .canonical_json()
                .map_err(|error| PlanError::Validation(format!("pre-registration: {error}")))?;
            ledger
                .append(LedgerRecord::lift_record(
                    now_unix_ms,
                    &signed.sha256,
                    &payload,
                ))
                .map_err(PlanError::Ledger)?;
        }
        let prereg_seq = pre_registration_seq(ledger, &spec.name);

        let record_change = |ledger: &mut dyn LedgerLog| -> Result<u64, PlanError> {
            Ok(ledger
                .append(LedgerRecord::change_applied(
                    now_unix_ms,
                    platform_of(&proposal.action),
                    proposal.action.ad_unit_id(),
                    change_kind_of(&proposal.action),
                    &key,
                    &proposal.prev_value,
                ))
                .map_err(PlanError::Ledger)?
                .seq())
        };

        // 2. An adapter that cannot carry the change in a test arm mutates the
        //    live unit first; the MAX path (test-arm change) skips this.
        let in_test_arm = ops.change_in_test_arm();
        let mut applied_change_seq: Option<u64> = None;
        if !in_test_arm {
            if let Err(source) = ops.apply(&proposal.action, &key).await {
                return Err(PlanError::ApplyFailed { source });
            }
            applied_change_seq = Some(record_change(ledger)?);
        }

        // 3. Start the experiment whose test arm carries the change.
        let request = crate::ops::ExperimentRequest {
            experiment_id: spec.name.clone(),
            spec: spec.clone(),
            change: change_kind_of(&proposal.action),
            prev_value: proposal.prev_value.clone(),
        };
        match ops.start_experiment(&request).await {
            Ok(experiment_id) => {
                // On the test-arm path the change is only enacted now, under
                // test; on the apply path it was recorded before the start.
                let change_seq = match applied_change_seq {
                    Some(seq) => seq,
                    None => record_change(ledger)?,
                };
                ledger
                    .append(LedgerRecord::experiment_started(
                        now_unix_ms,
                        platform_of(&proposal.action),
                        &experiment_id,
                        spec,
                    ))
                    .map_err(PlanError::Ledger)?;
                applied_today += 1;
                applied_this_cycle += 1;
                outcomes.push(ActionOutcome::Applied {
                    change_seq,
                    experiment_id,
                    idempotency_key: key,
                });
            }
            Err(source) => match applied_change_seq {
                Some(change_seq) => {
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
                None => {
                    // Test-arm path: the live unit was never touched, so the
                    // failed start rolls nothing back; the pre-registration
                    // simply stands and is visible in the ledger.
                    outcomes.push(ActionOutcome::RolledBack {
                        change_seq: prereg_seq,
                        reason: "experiment_start_failed".into(),
                    });
                }
            },
        }
    }

    Ok(CycleOutcome {
        outcomes,
        applied: applied_this_cycle,
        skipped: skipped_this_cycle,
        awaiting_approval,
    })
}
