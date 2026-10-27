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
//!
//! The one pre-registered extension week is taken when its data exists: if
//! the first decision is an extension offer, the window is re-analyzed at
//! `window_days + 7` with `extension_used = true` as soon as that window is
//! complete, so a promising test is never left running forever.
//!
//! A platform that reports no per-variant measurement (AdMob) is settled from
//! its own lifecycle instead: once the registered window has ended the
//! documented end call supplies the state and `variantLeader` verdict, a
//! `TREATMENT` leader keeps the change, every other verdict leaves the
//! original lines, and the conclusion is recorded with zero lift and no fee.
//! Before that day the test stays running and is retried.

use apex_lift::decision::{decide, Decision, DecisionInput, PreRegistration};
use apex_lift::guardrails::FillFormat;
use apex_lift::window::{build_window, incident_runs, AnalysisWindow};
use apex_proof::{ChangeKind, LedgerLog, LedgerRecord};

use crate::keys::platform_of;
use crate::ledger_index::LedgerIndex;
use crate::model::{Action, PlanError, SettleOutcome};
use crate::ops::PlatformOps;

const DAY_MS: i64 = 86_400_000;

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

/// The full outcome of one decision pass: the window, its guardrails, the
/// log ratios, the estimate and the decision.
type DecisionPass = (
    AnalysisWindow,
    Vec<apex_lift::guardrails::GuardrailEvent>,
    Vec<f64>,
    Option<apex_lift::stats::Estimate>,
    Decision,
);

/// One full decision pass over a window of `analysis_days`.
#[allow(clippy::too_many_arguments)]
fn decide_window(
    prereg: &PreRegistration,
    days: &crate::ops::ExperimentDays,
    analysis_days: usize,
    extension_used: bool,
    fill_format: FillFormat,
    settings_changed: &[String],
    bootstrap_seed: u64,
) -> Result<DecisionPass, PlanError> {
    let window = build_window(
        &days.control,
        &days.test,
        analysis_days,
        prereg.test_share,
        prereg.burn_in_days as usize,
    );
    let incidents = incident_runs(&days.control, &days.test);
    let arm_counts: Vec<(u64, u64)> = window
        .days
        .iter()
        .filter_map(|day| Some((day.test_users?, day.control_users?)))
        .collect();
    let guardrails = apex_lift::guardrails::evaluate(
        prereg,
        &window.days,
        &arm_counts,
        fill_format,
        settings_changed,
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
        extension_used,
        bootstrap_iterations: 2000,
        bootstrap_block_len: 7,
        bootstrap_seed,
    };
    let decision = decide(prereg, &input);
    Ok((window, guardrails, ratios, estimate, decision))
}

/// The last day of an unmeasured experiment's registered window.
///
/// The engine's window is three calendar burn-in days followed by the
/// pre-registered analysis days, counted from the day the experiment started
/// (the daily pull for a start day is the first burn-in day). The window is
/// complete once the last closed day is past this date.
fn registered_window_end(started_day: &str, burn_in_days: u32, window_days: usize) -> Option<i64> {
    let start = crate::calendar::day_number(started_day)?;
    Some(start + i64::from(burn_in_days) + i64::try_from(window_days).ok()? - 1)
}

/// Conclude a test whose platform reports no per-variant measurement.
///
/// The platform's own end call provides the state and `variantLeader`
/// verdict. `TREATMENT` keeps the change; anything else restores the original
/// control lines. No estimate exists, so the signed result carries zero lift,
/// and a kept AdMob conclusion is recorded as fee-ineligible: no fee record
/// can ever be built from it.
#[allow(clippy::too_many_arguments)]
async fn settle_unmeasured(
    ops: &dyn PlatformOps,
    ledger: &mut dyn LedgerLog,
    index: &mut LedgerIndex,
    experiment_id: &str,
    change_seq: u64,
    platform: apex_core::PlatformId,
    ad_unit_id: &str,
    prereg: &PreRegistration,
    now_unix_ms: i64,
) -> Result<SettleOutcome, PlanError> {
    let Some(started_day) = index
        .start(experiment_id, change_seq)
        .map(|start| start.started_day.as_str())
    else {
        return Ok(SettleOutcome::Pending {
            reason: format!(
                "the registered window cannot be dated: experiment {experiment_id} has no start record"
            ),
        });
    };
    let Some(end_day) = registered_window_end(started_day, prereg.burn_in_days, prereg.window_days)
    else {
        return Err(PlanError::Validation(format!(
            "experiment {experiment_id} has an unparseable start day '{started_day}'"
        )));
    };
    let today = now_unix_ms.div_euclid(DAY_MS);
    if today <= end_day {
        return Ok(SettleOutcome::Pending {
            reason: format!(
                "the registered window ends after {}; this platform reports no per-variant measurement",
                crate::calendar::format_day(end_day)
            ),
        });
    }

    let conclusion = ops
        .conclude_unmeasured_experiment(experiment_id)
        .await
        .map_err(|source| PlanError::ApplyFailed { source })?;
    let variant_leader = conclusion.variant_leader.clone();
    let leader = variant_leader
        .as_deref()
        .unwrap_or("no variantLeader verdict");
    let reason = if conclusion.keep_treatment {
        format!(
            "variantLeader {leader}: treatment kept after the registered window; no per-variant measurement (fee ineligible)"
        )
    } else {
        format!(
            "variantLeader {leader}: original lines restored after the registered window; no per-variant measurement (fee ineligible)"
        )
    };

    let observed = ledger
        .append(LedgerRecord::experiment_observed(
            now_unix_ms,
            platform,
            experiment_id,
            apex_core::ExperimentResult {
                experiment_id: experiment_id.to_string(),
                state: apex_core::ExperimentState::Stopped,
                results: Vec::new(),
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: variant_leader.clone(),
            },
        ))
        .map_err(PlanError::Ledger)?;
    index.observe(&observed);

    // No measurement exists: the signed result is explicitly zero-lift with
    // an empty ratio series, so nothing downstream can mistake a verdict for
    // an estimate.
    let estimate = apex_lift::stats::Estimate {
        days: 0,
        mean_log: 0.0,
        lift: 0.0,
        sd: 0.0,
        lower_90: 0.0,
        lower_99: 0.0,
        upper_90: 0.0,
        upper_99: 0.0,
    };
    let decision = if conclusion.keep_treatment {
        Decision::Keep {
            lambda: 0.0,
            estimate,
            used_interim_99: false,
            bootstrap_lower: 0.0,
        }
    } else {
        Decision::Rollback {
            reason: reason.clone(),
        }
    };
    let signed = apex_lift::emit::result(prereg, &decision, &estimate, Vec::new(), 0.0)
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
    index.observe(&result_entry);

    if conclusion.keep_treatment {
        // The durable fee decision: the kept change is excluded with the
        // engine's typed no-measurement reason. This is a billing *change*
        // (billed_lambda 0), never a KeptChange and never a fee.
        let ineligible = apex_lift::emit::billing_change(
            &prereg.experiment_id,
            &crate::calendar::format_day(today),
            apex_lift::billing::ADMOB_NO_MEASUREMENT_REASON,
            0.0,
        )
        .map_err(|error| PlanError::Validation(format!("billing change: {error}")))?;
        let payload = ineligible
            .record
            .canonical_json()
            .map_err(|error| PlanError::Validation(format!("billing change: {error}")))?;
        let entry = ledger
            .append(LedgerRecord::lift_record(
                now_unix_ms,
                &ineligible.sha256,
                &payload,
            ))
            .map_err(PlanError::Ledger)?;
        index.observe(&entry);
        return Ok(SettleOutcome::Kept {
            experiment_id: experiment_id.to_string(),
            lift: 0.0,
            lambda: 0.0,
            lower_90: 0.0,
            bootstrap_lower: 0.0,
            result_seq: result_entry.seq(),
            daily_log_ratios: Vec::new(),
            unmeasured: true,
            variant_leader,
        });
    }

    let rollback = ledger
        .append_rollback(now_unix_ms, platform, ad_unit_id, change_seq, &reason)
        .map_err(PlanError::Ledger)?;
    index.observe(&rollback);
    Ok(SettleOutcome::RolledBack {
        lift: 0.0,
        reason,
        rollback_seq: rollback.seq(),
        guardrails: Vec::new(),
        daily_log_ratios: Vec::new(),
        unmeasured: true,
        variant_leader,
    })
}

/// Settle one experiment through the Apex Lift Engine.
///
/// This rebuilds the ledger index once for the single experiment. A pass that
/// settles several experiments builds [`LedgerIndex`] once and calls
/// [`settle_experiment_indexed`] for each, so the ledger is never rescanned
/// per experiment.
#[allow(clippy::too_many_arguments)]
pub async fn settle_experiment(
    ops: &dyn PlatformOps,
    ledger: &mut dyn LedgerLog,
    experiment_id: &str,
    change_seq: u64,
    fill_format: FillFormat,
    settings_changed: &[String],
    now_unix_ms: i64,
) -> Result<SettleOutcome, PlanError> {
    let mut index = LedgerIndex::build(ledger);
    settle_experiment_indexed(
        ops,
        ledger,
        &mut index,
        experiment_id,
        change_seq,
        fill_format,
        settings_changed,
        None,
        now_unix_ms,
    )
    .await
}

/// Settle one experiment against an index the caller built once and keeps
/// current. All ledger facts are O(1) lookups; appends made here are folded
/// back into the index so the next experiment sees them.
///
/// `prefetched_days` is the reporting read the daily pass already performed
/// for the data-pull record. Passing it avoids a second identical reporting
/// call; `None` keeps the standalone behavior and fetches the days here.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn settle_experiment_indexed(
    ops: &dyn PlatformOps,
    ledger: &mut dyn LedgerLog,
    index: &mut LedgerIndex,
    experiment_id: &str,
    change_seq: u64,
    fill_format: FillFormat,
    settings_changed: &[String],
    prefetched_days: Option<&crate::ops::ExperimentDays>,
    now_unix_ms: i64,
) -> Result<SettleOutcome, PlanError> {
    let measured = ops.measures_variants();
    let platform_result = if measured {
        let result = ops
            .experiment(experiment_id)
            .await
            .map_err(|source| PlanError::ApplyFailed { source })?;
        // A manually promoted test ends by the publisher's own action: the
        // variant group disappearing is the confirmation signal, not an
        // already-ended refusal. Without a pending promotion this stays a
        // typed refusal.
        // Manual adapters decide a stopped test through promotion_status:
        // the dashboard may have ended it early, promoted it, or declined it.
        let awaiting_promotion = ops.promotion_requires_manual_action();
        if result.state == apex_core::ExperimentState::Stopped && !awaiting_promotion {
            return Err(PlanError::PolicyViolation {
                reason: format!("experiment {experiment_id} has already ended"),
            });
        }
        Some(result)
    } else {
        None
    };

    let (action, platform, ad_unit_id) = {
        let applied = index.change(change_seq).ok_or_else(|| {
            if index.contains_seq(change_seq) {
                PlanError::NoBaseline {
                    reason: format!("sequence {change_seq} is not a ChangeApplied record"),
                }
            } else {
                PlanError::NoBaseline {
                    reason: format!("change sequence {change_seq} is not in the ledger"),
                }
            }
        })?;
        let mut action = change_to_action(&applied.change)
            .ok_or_else(|| PlanError::Validation("unknown change kind".into()))?;
        if let Action::SetCountryFloor { ad_unit_id: a, .. } = &mut action {
            *a = applied.ad_unit_id.clone();
        }
        (action, applied.platform, applied.ad_unit_id.clone())
    };
    if platform != platform_of(&action) {
        return Err(PlanError::Validation(
            "ledger platform does not match the action's platform".into(),
        ));
    }

    if let Some(refusal) = index.settled_refusal(experiment_id, change_seq) {
        return Err(refusal);
    }

    let prereg = index.signed_pre_registration(experiment_id, change_seq)?;

    if !measured {
        return settle_unmeasured(
            ops,
            ledger,
            index,
            experiment_id,
            change_seq,
            platform,
            &ad_unit_id,
            &prereg,
            now_unix_ms,
        )
        .await;
    }

    let mut days = match prefetched_days {
        Some(days) => days.clone(),
        None => {
            // The evidence key is the pre-registered experiment instance plus
            // its start day, not the reusable platform id (MAX returns the ad
            // unit id for every experiment on that unit).
            let instance_key = index
                .start(experiment_id, change_seq)
                .map(|start| start.instance_id.clone())
                .unwrap_or_else(|| instance_identity(&prereg));
            ops.experiment_days_for(experiment_id, &instance_key)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?
        }
    };
    // Some adapters return the platform's full report window, including days
    // before the test existed. Analysing those days would fail the split
    // check on traffic that was never in an experiment. The registered start
    // day is the earliest analysable day.
    if let Some(start_day) = index
        .start(experiment_id, change_seq)
        .map(|start| start.started_day.as_str())
    {
        retain_from_start_day(&mut days, start_day);
    }

    let bootstrap_seed = deterministic_bootstrap_seed(&prereg);
    let (window, guardrails, ratios, estimate, decision) = decide_window(
        &prereg,
        &days,
        prereg.window_days,
        false,
        fill_format,
        settings_changed,
        bootstrap_seed,
    )?;

    // The one pre-registered extension week is taken exactly when its data
    // exists: re-analyze at window + 7 with the extension marked used.
    let (_window, guardrails, ratios, estimate, decision) =
        if matches!(decision, Decision::Extend { .. }) {
            let extended = prereg.window_days + 7;
            if !apex_lift::window::ALLOWED_WINDOW_DAYS.contains(&extended) {
                return Ok(SettleOutcome::Pending {
                    reason: "the extension week cannot extend this window; re-register".into(),
                });
            }
            let (window2, guardrails2, ratios2, estimate2, decision2) = decide_window(
                &prereg,
                &days,
                extended,
                true,
                fill_format,
                settings_changed,
                bootstrap_seed,
            )?;
            if !window2.complete {
                return Ok(SettleOutcome::Pending {
                    reason: "the extension week is still accruing final days".into(),
                });
            }
            (window2, guardrails2, ratios2, estimate2, decision2)
        } else {
            (window, guardrails, ratios, estimate, decision)
        };

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
            eprintln!("apex-runner: experiment {experiment_id} voided: {reason}");
            ops.deprecate_experiment(experiment_id)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?;
            return Ok(SettleOutcome::Voided {
                reason: reason.clone(),
                guardrails: guardrails.clone(),
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
            // The guardrails that fired are part of the record.
            append_guardrails(ledger, index, &guardrails, now_unix_ms)?;
            let rollback = ledger
                .append_rollback(now_unix_ms, platform, &ad_unit_id, change_seq, reason)
                .map_err(PlanError::Ledger)?;
            index.observe(&rollback);
            return Ok(SettleOutcome::RolledBack {
                lift: estimate.as_ref().map(|e| e.lift).unwrap_or(0.0),
                reason: reason.clone(),
                rollback_seq: rollback.seq(),
                guardrails: guardrails.clone(),
                daily_log_ratios: ratios,
                unmeasured: false,
                variant_leader: None,
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

    if ops.promotion_requires_manual_action() {
        let change = index
            .change(change_seq)
            .ok_or_else(|| PlanError::NoBaseline {
                reason: format!("change sequence {change_seq} is not in the ledger"),
            })?
            .change
            .clone();
        match ops
            .promotion_status(experiment_id, &change)
            .await
            .map_err(|source| PlanError::ApplyFailed { source })?
        {
            crate::ops::PromotionStatus::Confirmed => {}
            crate::ops::PromotionStatus::Declined => {
                let reason =
                    "the publisher kept the control arm instead of promoting the test; no fee"
                        .to_string();
                append_guardrails(ledger, index, &guardrails, now_unix_ms)?;
                let rollback = ledger
                    .append_rollback(now_unix_ms, platform, &ad_unit_id, change_seq, &reason)
                    .map_err(PlanError::Ledger)?;
                index.observe(&rollback);
                return Ok(SettleOutcome::RolledBack {
                    lift: keep_estimate.lift,
                    reason,
                    rollback_seq: rollback.seq(),
                    guardrails: guardrails.clone(),
                    daily_log_ratios: ratios,
                    unmeasured: false,
                    variant_leader: None,
                });
            }
            crate::ops::PromotionStatus::Pending => {
                if let Some(deadline) = index.promotion_deadline(experiment_id, change_seq) {
                    if now_unix_ms > deadline {
                        ops.deprecate_experiment(experiment_id)
                            .await
                            .map_err(|source| PlanError::ApplyFailed { source })?;
                        let reason = "the promotion deadline passed without the change serving everyone; the variant arm was ended and no fee applies".to_string();
                        append_guardrails(ledger, index, &guardrails, now_unix_ms)?;
                        let rollback = ledger
                            .append_rollback(
                                now_unix_ms,
                                platform,
                                &ad_unit_id,
                                change_seq,
                                &reason,
                            )
                            .map_err(PlanError::Ledger)?;
                        index.observe(&rollback);
                        return Ok(SettleOutcome::RolledBack {
                            lift: keep_estimate.lift,
                            reason,
                            rollback_seq: rollback.seq(),
                            guardrails: guardrails.clone(),
                            daily_log_ratios: ratios,
                            unmeasured: false,
                            variant_leader: None,
                        });
                    }
                    return Ok(SettleOutcome::Pending {
                        reason: format!(
                            "waiting for the publisher to promote the test in the platform dashboard (deadline {})",
                            crate::calendar::format_day(deadline.div_euclid(DAY_MS))
                        ),
                    });
                }
                let deadline = now_unix_ms.saturating_add(14 * DAY_MS);
                let instructions = "Open the platform dashboard and promote group B (the treatment arm) so its settings serve all traffic. The change is recorded but is not billed until a later daily read confirms the promotion.".to_string();
                let pending = ledger
                    .append(LedgerRecord::promotion_pending(
                        now_unix_ms,
                        platform,
                        experiment_id,
                        &ad_unit_id,
                        change_seq,
                        deadline,
                        &instructions,
                    ))
                    .map_err(PlanError::Ledger)?;
                index.observe(&pending);
                return Ok(SettleOutcome::PromotionPending {
                    experiment_id: experiment_id.to_string(),
                    instructions,
                    deadline_unix_ms: deadline,
                });
            }
        }
    } else {
        ops.promote_experiment(experiment_id)
            .await
            .map_err(|source| PlanError::ApplyFailed { source })?;
    }

    let observed_result = platform_result.ok_or_else(|| {
        PlanError::Validation(format!(
            "experiment {experiment_id} has no readable platform state on a measured adapter"
        ))
    })?;
    let observed = ledger
        .append(LedgerRecord::experiment_observed(
            now_unix_ms,
            platform,
            experiment_id,
            observed_result,
        ))
        .map_err(PlanError::Ledger)?;
    index.observe(&observed);
    append_guardrails(ledger, index, &guardrails, now_unix_ms)?;

    let signed = apex_lift::emit::result(
        &prereg,
        &decision,
        &keep_estimate,
        ratios.clone(),
        bootstrap_lower,
    )
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
    index.observe(&result_entry);

    Ok(SettleOutcome::Kept {
        experiment_id: experiment_id.to_string(),
        lift: keep_estimate.lift,
        lambda,
        lower_90: keep_estimate.lower_90,
        bootstrap_lower,
        result_seq: result_entry.seq(),
        daily_log_ratios: ratios,
        unmeasured: false,
        variant_leader: None,
    })
}

/// A stable seed from immutable experiment identity and the signed
/// pre-registration, never from wall-clock settlement time. Re-evaluating the
/// same evidence therefore draws the same bootstrap sample.
fn deterministic_bootstrap_seed(prereg: &PreRegistration) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let instance = instance_identity(prereg);
    for field in [
        instance.as_str(),
        prereg.experiment_id.as_str(),
        prereg.scope.as_str(),
        prereg.change_description.as_str(),
        prereg.before_value.as_str(),
        prereg.code_version_hash.as_str(),
    ] {
        for byte in field.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^ u64::from(prereg.rate_bp)
}

/// The instance id a pre-registration pins, falling back to the legacy
/// experiment name only when reading a record written before instance ids.
fn instance_identity(prereg: &PreRegistration) -> String {
    if prereg.instance_id.trim().is_empty() {
        prereg.experiment_id.clone()
    } else {
        prereg.instance_id.clone()
    }
}

/// Append the signed guardrail records for the events that fired.
fn append_guardrails(
    ledger: &mut dyn LedgerLog,
    index: &mut LedgerIndex,
    guardrails: &[apex_lift::guardrails::GuardrailEvent],
    now_unix_ms: i64,
) -> Result<(), PlanError> {
    for event in guardrails {
        let emitted = apex_lift::emit::guardrail_event(event)
            .map_err(|error| PlanError::Validation(format!("guardrail record: {error}")))?;
        let payload = emitted
            .record
            .canonical_json()
            .map_err(|error| PlanError::Validation(format!("guardrail record: {error}")))?;
        let record = ledger
            .append(LedgerRecord::lift_record(
                now_unix_ms,
                &emitted.sha256,
                &payload,
            ))
            .map_err(PlanError::Ledger)?;
        index.observe(&record);
    }
    Ok(())
}

/// Drop platform history before the experiment's own start day. The start
/// day itself is part of the experiment, so the comparison is inclusive.
fn retain_from_start_day(days: &mut crate::ops::ExperimentDays, start_day: &str) {
    days.control.retain(|day| day.day.as_str() >= start_day);
    days.test.retain(|day| day.day.as_str() >= start_day);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::ops::ExperimentDays;

    fn day(name: &str) -> apex_lift::window::ArmDay {
        apex_lift::window::ArmDay {
            day: name.to_string(),
            revenue: 1,
            users: None,
            second_pull_revenue: Some(1),
            second_pull_users: None,
            fill: None,
            second_pull_fill: None,
            first_observed_at_unix_s: Some(1_700_000_000),
            second_observed_at_unix_s: Some(
                1_700_000_000 + apex_lift::window::SECOND_PULL_MIN_SECS,
            ),
            first_source_sha256: Some("a".repeat(64)),
            second_source_sha256: Some("b".repeat(64)),
        }
    }

    #[test]
    fn the_start_day_itself_is_analyzed() {
        let mut days = ExperimentDays {
            control: vec![day("2026-10-09"), day("2026-10-10"), day("2026-10-11")],
            test: vec![day("2026-10-09"), day("2026-10-10"), day("2026-10-11")],
        };
        retain_from_start_day(&mut days, "2026-10-10");
        assert_eq!(
            days.control
                .iter()
                .map(|day| day.day.as_str())
                .collect::<Vec<_>>(),
            ["2026-10-10", "2026-10-11"]
        );
        assert_eq!(
            days.test
                .iter()
                .map(|day| day.day.as_str())
                .collect::<Vec<_>>(),
            ["2026-10-10", "2026-10-11"]
        );
    }
}
