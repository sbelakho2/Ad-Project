//! The write cycle: propose -> policy gates -> apply -> mandatory experiment.
//! Daily accounting reads the signed ledger, never caller state.

use apex_audit::{Finding, Severity};
use apex_proof::{ChangeKind, LedgerLog, LedgerRecord};

use crate::keys::{experiment_name, idempotency_key, operation_of, platform_of};
use crate::ledger_index::LedgerIndex;
use crate::model::{Action, ActionOutcome, AutopilotPolicy, CycleOutcome, PlanError, Proposal};
use crate::ops::PlatformOps;

const DAY_MS: i64 = 86_400_000;

fn utc_day(at_unix_ms: i64) -> i64 {
    at_unix_ms.div_euclid(DAY_MS)
}

pub(crate) fn change_kind_of(action: &Action) -> ChangeKind {
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
        // The frozen instance/mediation actions ride the platform's existing
        // ledger change kinds: LevelPlay's instance price is a waterfall-entry
        // price write and AdMob's group floor is a mediation-group eCPM write.
        // The action names only the platform resource id, so that id is both
        // the scope and the existing kind's target id.
        Action::SetLevelplayInstancePrice {
            instance_id,
            price_micros,
        } => ChangeKind::WaterfallEntryPriceSet {
            ad_unit_id: instance_id.clone(),
            entry_id: instance_id.clone(),
            price_ecpm: *price_micros,
        },
        Action::SetAdmobMediationFloor {
            group_id,
            floor_micros,
        } => ChangeKind::EcpmSet {
            mediation_group_id: group_id.clone(),
            ad_unit_id: group_id.clone(),
            ecpm: *floor_micros,
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
        Action::SetLevelplayInstancePrice {
            instance_id,
            price_micros,
        } => {
            if instance_id.trim().is_empty() {
                return Err(PlanError::Validation(
                    "instanceId is required to set an instance price".into(),
                ));
            }
            apex_core::validate_ecpm(*price_micros)
                .map_err(|e| PlanError::Validation(e.to_string()))?;
        }
        Action::SetAdmobMediationFloor {
            group_id,
            floor_micros,
        } => {
            if group_id.trim().is_empty() {
                return Err(PlanError::Validation(
                    "mediationGroupId is required to set a mediation floor".into(),
                ));
            }
            apex_core::validate_ecpm(*floor_micros)
                .map_err(|e| PlanError::Validation(e.to_string()))?;
        }
    }
    Ok(())
}

/// The registered metric's stable name, as the `experiment_registered`
/// payload records it. The mapping is total over the engine's metric set.
fn registered_metric(metric: &apex_lift::decision::Metric) -> &'static str {
    match metric {
        apex_lift::decision::Metric::RevenuePerUser => "revenue_per_user",
    }
}

/// True when the same change (equal idempotency key and equal payload)
/// is already in the ledger; a rerun must never re-apply it, and a rerun with
/// a different value under a colliding key must never be mistaken for it.
pub fn already_applied(ledger: &dyn LedgerLog, key: &str, action: &Action) -> bool {
    LedgerIndex::build(ledger).already_applied(key, action)
}

/// True when the ledger already carries the signed pre-registration for
/// `experiment_id` (a rerun must not stack duplicate registrations).
pub fn pre_registration_exists(ledger: &dyn LedgerLog, experiment_id: &str) -> bool {
    LedgerIndex::build(ledger).pre_registration_exists(experiment_id)
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
    run_cycle_observed(ops, ledger, policy, findings, &proposals, None, now_unix_ms).await
}

/// Run one write cycle with the analytics emitter attached: the
/// pre-registration step emits `experiment_registered` and the confirmed
/// platform start emits `experiment_started`, each at its own sub-step so a
/// registered-but-unstarted attempt still leaves the registration fact.
pub async fn run_cycle_observed(
    ops: &dyn PlatformOps,
    ledger: &mut dyn LedgerLog,
    policy: &AutopilotPolicy,
    findings: &[Finding],
    proposals: &[Proposal],
    events: Option<&crate::events::RunnerEvents>,
    now_unix_ms: i64,
) -> Result<CycleOutcome, PlanError> {
    let none = std::collections::HashSet::new();
    run_cycle_observed_with(
        ops,
        ledger,
        policy,
        findings,
        proposals,
        events,
        &none,
        &none,
        now_unix_ms,
    )
    .await
}

/// The observed cycle with the console's recorded decisions: `approved`
/// idempotency keys pass the approval gate; `declined` keys are skipped and
/// never applied. Every other gate still runs.
#[allow(clippy::too_many_arguments)]
pub async fn run_cycle_observed_with(
    ops: &dyn PlatformOps,
    ledger: &mut dyn LedgerLog,
    policy: &AutopilotPolicy,
    findings: &[Finding],
    proposals: &[Proposal],
    events: Option<&crate::events::RunnerEvents>,
    approved_keys: &std::collections::HashSet<String>,
    declined_keys: &std::collections::HashSet<String>,
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
    let mut index = LedgerIndex::build(ledger);
    // Clock skew must not consume the day: only records at or before `now`
    // count, and a change that was rolled back frees its slot for a retry.
    let mut applied_today: u32 = index
        .applied_on_day(day, now_unix_ms)
        .try_into()
        .map_err(|_| PlanError::Validation("applied count overflow".into()))?;

    if applied_today >= policy.max_actions_per_day {
        return Err(PlanError::DailyCapReached { n: applied_today });
    }

    let mut outcomes = Vec::with_capacity(proposals.len());
    let mut applied_this_cycle: u32 = 0;
    let mut skipped_this_cycle: u32 = 0;
    let mut awaiting_approval: u32 = 0;
    let mut awaiting_verification: Vec<String> = Vec::new();

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
        if index.already_applied(&key, &proposal.action) {
            skipped_this_cycle += 1;
            outcomes.push(ActionOutcome::Skipped {
                reason: "already_applied".into(),
            });
            continue;
        }

        let operation = operation_of(&proposal.action);
        if !ops.writable(operation).await {
            // Platform-capability-aware hold: the adapter's verified write
            // set does not (yet) include this operation, so the change is
            // never applied. The typed awaiting state rides the same
            // `Skipped` decision record the other gates use and names the
            // operation and platform; the pass continues to the remaining
            // proposals instead of aborting on one unverified endpoint.
            skipped_this_cycle += 1;
            let reason = format!(
                "awaiting live verification for {:?}: {operation}",
                ops.platform()
            );
            awaiting_verification.push(reason.clone());
            outcomes.push(ActionOutcome::Skipped { reason });
            continue;
        }

        // A declined proposal is final for this exact change: it is never
        // applied, even once the time-based approval window has passed.
        if declined_keys.contains(&key) {
            skipped_this_cycle += 1;
            outcomes.push(ActionOutcome::Skipped {
                reason: "approval_declined".into(),
            });
            continue;
        }

        // Approval mode: report the proposal and stop before any write or
        // pre-registration. On for a customer's first 30 days, unless the
        // workspace approved this exact change in the console.
        if policy.approval_required(now_unix_ms) && !approved_keys.contains(&key) {
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
        if index.pre_registration_exists(&spec.name) && !index.prior_change_exists(&key) {
            skipped_this_cycle += 1;
            outcomes.push(ActionOutcome::Skipped {
                reason: "experiment_in_flight".into(),
            });
            continue;
        }

        let (source_digest, prereg_seq) = {
            let signed = apex_lift::emit::pre_registration(&prereg)
                .map_err(|error| PlanError::Validation(format!("pre-registration: {error}")))?;
            let payload = signed
                .record
                .canonical_json()
                .map_err(|error| PlanError::Validation(format!("pre-registration: {error}")))?;
            let record = ledger
                .append(LedgerRecord::lift_record(
                    now_unix_ms,
                    &signed.sha256,
                    &payload,
                ))
                .map_err(PlanError::Ledger)?;
            index.observe(&record);
            // A retry may append a duplicate registration; the old scan
            // reported the first one, so the index's first-wins entry is the
            // sequence the rollback outcome names.
            let seq = index.pre_registration_seq(&spec.name).unwrap_or(0);
            (signed.sha256.clone(), seq)
        };
        // Step 1's registration is complete; the event lands before the
        // platform start so a registered-but-unstarted attempt stays visible.
        if let Some(events) = events {
            let registered_window_days = u32::try_from(prereg.window_days)
                .map_err(|_| PlanError::Validation("registered window is out of range".into()))?;
            events
                .experiment_registered(
                    &spec.name,
                    &source_digest,
                    registered_metric(&prereg.metric),
                    registered_window_days,
                    now_unix_ms,
                )
                .await;
        }
        let change = change_kind_of(&proposal.action);
        let mut record_change = |ledger: &mut dyn LedgerLog| -> Result<u64, PlanError> {
            let record = ledger
                .append(LedgerRecord::change_applied(
                    now_unix_ms,
                    platform_of(&proposal.action),
                    proposal.action.ad_unit_id(),
                    change.clone(),
                    &key,
                    &proposal.prev_value,
                ))
                .map_err(PlanError::Ledger)?;
            index.observe(&record);
            Ok(record.seq())
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
                let started = ledger
                    .append(LedgerRecord::experiment_started(
                        now_unix_ms,
                        platform_of(&proposal.action),
                        &experiment_id,
                        spec,
                    ))
                    .map_err(PlanError::Ledger)?;
                index.observe(&started);
                // The platform start is confirmed (the adapter returned the
                // running experiment): record the lifecycle fact.
                if let Some(events) = events {
                    events
                        .experiment_started(&experiment_id, "running", now_unix_ms)
                        .await;
                }
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
                    let rollback = ledger
                        .append_rollback(
                            now_unix_ms,
                            platform_of(&proposal.action),
                            proposal.action.ad_unit_id(),
                            change_seq,
                            "experiment_start_failed",
                        )
                        .map_err(PlanError::Ledger)?;
                    index.observe(&rollback);
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
        awaiting_verification,
    })
}
