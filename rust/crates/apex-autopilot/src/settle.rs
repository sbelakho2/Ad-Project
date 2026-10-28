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
use crate::ops::{HoldoutRequest, PlatformOps};

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
        ChangeKind::FrequencyCapSet {
            ad_unit_id,
            cap_type,
            countries,
            session_limit,
            day_limit,
            minute_frequency,
        } => Some(Action::SetMaxFrequencyCap {
            ad_unit_id: ad_unit_id.clone(),
            cap_type: match cap_type.as_str() {
                "session" => crate::model::MaxCapType::Session,
                "time" => crate::model::MaxCapType::Time,
                _ => return None,
            },
            countries: countries.clone(),
            session_limit: *session_limit,
            day_limit: *day_limit,
            minute_frequency: *minute_frequency,
        }),
        ChangeKind::LevelplayGroupCapping {
            group_id,
            session_limit,
            interstitial_limit,
            day_limit,
        } => Some(Action::SetLevelplayGroupCapping {
            group_id: group_id.clone(),
            session_limit: *session_limit,
            interstitial_limit: *interstitial_limit,
            day_limit: *day_limit,
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

/// The engine never makes a keep/rollback/void decision without an estimate at
/// a completed look, but a void can be reached from a fatal window before the
/// estimate exists. The signed terminal record still needs a well-formed
/// estimate; zero is the honest "no measurement" value.
fn zero_estimate() -> apex_lift::stats::Estimate {
    apex_lift::stats::Estimate {
        days: 0,
        mean_log: 0.0,
        lift: 0.0,
        sd: 0.0,
        lower_90: 0.0,
        lower_99: 0.0,
        upper_90: 0.0,
        upper_99: 0.0,
        lower_alpha: 0.0,
        lower_interim: 0.0,
    }
}

/// Append the signed terminal `Result` for a decision. This is the durable
/// proof that must exist before any remote promote/deprecate/rollback, so a
/// crash can never leave platform state ahead of the signed record.
#[allow(clippy::too_many_arguments)]
fn append_signed_result(
    ledger: &mut dyn LedgerLog,
    index: &mut LedgerIndex,
    prereg: &PreRegistration,
    decision: &Decision,
    estimate: &apex_lift::stats::Estimate,
    ratios: Vec<f64>,
    bootstrap_lower: f64,
    now_unix_ms: i64,
) -> Result<u64, PlanError> {
    let signed = apex_lift::emit::result(prereg, decision, estimate, ratios, bootstrap_lower)
        .map_err(|error| PlanError::Validation(format!("result record: {error}")))?;
    let payload = signed
        .record
        .canonical_json()
        .map_err(|error| PlanError::Validation(format!("result record: {error}")))?;
    let record = ledger
        .append(LedgerRecord::lift_record(
            now_unix_ms,
            &signed.sha256,
            &payload,
        ))
        .map_err(PlanError::Ledger)?;
    index.observe(&record);
    Ok(record.seq())
}

/// A Requested state still needs the provider confirmation append.
fn holdout_needs_confirmation(status: apex_lift::holdout::HoldoutStatus) -> bool {
    status == apex_lift::holdout::HoldoutStatus::Requested
}

/// Open the durable 90/10 holdout for a kept change. The signed state is
/// written before the observation marker, so a crash after the keep is
/// replayed into the same instance's holdout rather than a second experiment.
async fn append_holdout_state(
    ops: &dyn PlatformOps,
    ledger: &mut dyn LedgerLog,
    index: &mut LedgerIndex,
    prereg: &PreRegistration,
    change_seq: u64,
    kept_day: &str,
    now_unix_ms: i64,
) -> Result<(), PlanError> {
    if !ops.holdout_supported() {
        return Ok(());
    }
    let change = index
        .change(change_seq)
        .ok_or_else(|| PlanError::NoBaseline {
            reason: format!("change sequence {change_seq} is not in the ledger"),
        })?
        .change
        .clone();
    let platform = ops.platform();
    let holdout_name = apex_lift::holdout::holdout_experiment_id(&prereg.instance_id);
    let spec = apex_core::ExperimentSpec {
        name: holdout_name.clone(),
        instance_id: format!("{}::holdout", prereg.instance_id),
        ad_unit_id: prereg.scope.clone(),
        metric: apex_core::ExperimentMetric::Revenue,
        variants: 2,
        change_description: format!("holdout: old setting for {}", prereg.change_description),
        test_group_allocation: 10,
    };
    let request = HoldoutRequest {
        experiment_id: holdout_name,
        spec,
        change,
        prev_value: prereg.before_value.clone(),
    };
    let mut state = match index.holdout_state(&prereg.instance_id) {
        Some(existing) if !holdout_needs_confirmation(existing.status) => {
            return Ok(());
        }
        Some(existing) => existing.clone(),
        None => {
            let exposed_is_test = matches!(platform, apex_core::PlatformId::LevelPlay);
            apex_lift::holdout::HoldoutState::open(
                &prereg.instance_id,
                &prereg.experiment_id,
                &prereg.scope,
                platform.as_str(),
                kept_day,
                exposed_is_test,
                index
                    .pre_registration_hash(&prereg.instance_id)
                    .unwrap_or_default(),
                index.result_hash(&prereg.instance_id).unwrap_or_default(),
            )
        }
    };
    if !holdout_needs_confirmation(state.status) {
        state.request();
        let signed = apex_lift::emit::holdout_state(&state)
            .map_err(|error| PlanError::Validation(format!("holdout state: {error}")))?;
        let payload = signed
            .record
            .canonical_json()
            .map_err(|error| PlanError::Validation(format!("holdout state: {error}")))?;
        let record = ledger
            .append(LedgerRecord::lift_record(
                now_unix_ms,
                &signed.sha256,
                &payload,
            ))
            .map_err(PlanError::Ledger)?;
        index.observe(&record);
    }
    // The request record is durable before this call. If the provider succeeds
    // and the process dies before the confirmation append, the next settlement
    // retries the deterministic request and then writes the confirmed state.
    let holdout_experiment_id = ops
        .start_holdout(&request)
        .await
        .map_err(|source| PlanError::ApplyFailed { source })?;
    state.confirm(&holdout_experiment_id);
    let signed = apex_lift::emit::holdout_state(&state)
        .map_err(|error| PlanError::Validation(format!("holdout state: {error}")))?;
    let payload = signed
        .record
        .canonical_json()
        .map_err(|error| PlanError::Validation(format!("holdout state: {error}")))?;
    let record = ledger
        .append(LedgerRecord::lift_record(
            now_unix_ms,
            &signed.sha256,
            &payload,
        ))
        .map_err(PlanError::Ledger)?;
    index.observe(&record);
    Ok(())
}

/// One full decision pass over a window of `analysis_days`.
#[allow(clippy::too_many_arguments)]
fn decide_window(
    prereg: &PreRegistration,
    days: &crate::ops::ExperimentDays,
    players: Option<&crate::ops::PlayerEvidence>,
    player_feed_error: Option<&str>,
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
    // L1/L3: when the platform supplied persisted per-player rows, the unique
    // players are the split denominator and the Horvitz–Thompson estimate is
    // the decision estimate. A player in both arms voids rather than being
    // folded into one arm or counted twice.
    let mut player_problem: Option<String> = player_feed_error.map(str::to_string);
    let mut unique_split = None;
    let mut player_estimate = None;
    if let Some(players) = players.filter(|players| !players.rows.is_empty()) {
        let cuped = !players.pretest.is_empty();
        // The estimator's unit is the analysis window. A 45/14-day feed can
        // carry days outside it; those rows must not enter the split or the
        // per-player totals (a pre-window user would otherwise be counted).
        let analysis_days: std::collections::BTreeSet<&str> =
            window.days.iter().map(|day| day.day.as_str()).collect();
        let rows: Vec<apex_lift::player_store::PlayerDay> = players
            .rows
            .iter()
            .filter(|row| analysis_days.contains(row.day.as_str()))
            .cloned()
            .collect();
        if rows.is_empty() {
            // The feed answered but carried nothing from the analysis window:
            // the shortest shortened feed of all. It must not silently fall
            // back to the daily estimate while partial feeds void.
            player_problem = Some(format!(
                "player evidence covers 0 of {} analysis days; a shortened feed cannot decide",
                analysis_days.len()
            ));
        } else {
            let row_days: std::collections::BTreeSet<&str> =
                rows.iter().map(|row| row.day.as_str()).collect();
            // Evidence integrity runs first: a player under both arms voids
            // regardless of how many days the feed carried.
            match apex_lift::player_store::player_window(&rows, Some(&players.pretest)) {
                Ok(window) => {
                    if row_days.len() != analysis_days.len() {
                        // A feed that silently skipped days would shrink the
                        // window the decision statistic is built from.
                        // Incomplete player evidence never replaces the daily
                        // estimate; it is reported instead.
                        player_problem = Some(format!(
                            "player evidence covers only {} of {} analysis days; a shortened feed cannot decide",
                            row_days.len(),
                            analysis_days.len()
                        ));
                    } else {
                        unique_split = Some((window.test_players, window.control_players));
                        player_estimate = apex_lift::player_store::estimate_window(
                            &window,
                            prereg.test_share,
                            cuped,
                        );
                    }
                }
                Err(error) => {
                    player_problem = Some(format!("player evidence is unusable: {error}"))
                }
            }
        }
    }

    // A missing reporting day is the stale evidence the reporting-gap
    // guardrail must see; passing zero made the advertised void unreachable.
    let stale_days = u32::try_from(
        window
            .problems
            .iter()
            .filter(|problem| {
                matches!(problem, apex_lift::window::WindowProblem::MissingDay { .. })
            })
            .count(),
    )
    .unwrap_or(u32::MAX);
    let guardrails = apex_lift::guardrails::evaluate_with_unique_players(
        prereg,
        &window.days,
        unique_split,
        &arm_counts,
        fill_format,
        settings_changed,
        stale_days,
        &incidents,
    );
    let ratios: Vec<f64> = window.days.iter().map(|d| d.log_ratio).collect();
    let used_player_estimate = player_estimate.is_some();
    let estimate = player_estimate
        .map(|player| player.estimate)
        .or_else(|| apex_lift::stats::estimate(&ratios));
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
    let decision = if let Some(reason) = player_problem {
        Decision::Void { reason }
    } else if used_player_estimate {
        match estimate {
            Some(estimate) => apex_lift::decision::decide_with_estimate(prereg, &input, estimate),
            None => decide(prereg, &input),
        }
    } else {
        decide(prereg, &input)
    };
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

    // Durable intent before the remote stop: a crash after the stop or the
    // observation marker but before the signed result can be reconciled on
    // the next pass because the marker says this experiment was mid-terminal.
    if !index.has_pending_promotion(experiment_id, change_seq) {
        let pending = ledger
            .append(LedgerRecord::promotion_pending(
                now_unix_ms,
                platform,
                experiment_id,
                ad_unit_id,
                change_seq,
                now_unix_ms.saturating_add(DAY_MS),
                "concluding an unmeasured experiment: signed terminal result pending",
            ))
            .map_err(PlanError::Ledger)?;
        index.observe(&pending);
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
        lower_alpha: 0.0,
        lower_interim: 0.0,
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

/// The signed decision a pending terminal result pinned, with the values the
/// settle outcome needs to report it again after a crash.
struct RecoveredResult {
    decision: String,
    lambda: f64,
    lower_90: f64,
    bootstrap_lower: f64,
    estimate: apex_lift::stats::Estimate,
    ratios: Vec<f64>,
    seq: u64,
}

fn recovered_result(record: &apex_lift::records::LiftRecord, seq: u64) -> Option<RecoveredResult> {
    let apex_lift::records::LiftRecord::Result {
        daily_log_ratios,
        mean_log,
        lower_90,
        lower_99,
        bootstrap_lower,
        decision,
        lambda,
        ..
    } = record
    else {
        return None;
    };
    let estimate = apex_lift::stats::Estimate {
        days: daily_log_ratios.len(),
        mean_log: *mean_log,
        lift: mean_log.exp() - 1.0,
        sd: 0.0,
        lower_90: *lower_90,
        lower_99: *lower_99,
        upper_90: 0.0,
        upper_99: 0.0,
        lower_alpha: *lower_99,
        lower_interim: *lower_99,
    };
    Some(RecoveredResult {
        decision: decision.clone(),
        lambda: *lambda,
        lower_90: *lower_90,
        bootstrap_lower: *bootstrap_lower,
        estimate,
        ratios: daily_log_ratios.clone(),
        seq,
    })
}

/// Complete the remote side effect a signed terminal result promised. This is
/// the crash-recovery half of "proof before side effect": the result is
/// already durable, so the only work left is to apply the platform state and
/// append the completion marker the daily scan keys on.
#[allow(clippy::too_many_arguments)]
async fn reconcile_terminal(
    ops: &dyn PlatformOps,
    ledger: &mut dyn LedgerLog,
    index: &mut LedgerIndex,
    experiment_id: &str,
    change_seq: u64,
    now_unix_ms: i64,
) -> Result<SettleOutcome, PlanError> {
    let start = index
        .start(experiment_id, change_seq)
        .cloned()
        .ok_or_else(|| PlanError::NoBaseline {
            reason: format!("experiment {experiment_id} was never started in this ledger"),
        })?;
    let (result, seq) = index
        .result_record(&start.instance_id)
        .map(|(record, seq)| (record.clone(), seq))
        .ok_or_else(|| {
            PlanError::Validation(format!(
                "experiment {experiment_id} has no signed terminal result to reconcile"
            ))
        })?;
    let recovered = recovered_result(&result, seq).ok_or_else(|| {
        PlanError::Validation(format!(
            "experiment {experiment_id} carries a non-result lift record"
        ))
    })?;
    let (action, platform, ad_unit_id) = {
        let applied = index
            .change(change_seq)
            .ok_or_else(|| PlanError::NoBaseline {
                reason: format!("change sequence {change_seq} is not in the ledger"),
            })?;
        let action = change_to_action(&applied.change)
            .ok_or_else(|| PlanError::Validation("unknown change kind".into()))?;
        (action, applied.platform, applied.ad_unit_id.clone())
    };

    match recovered.decision.as_str() {
        "keep" => {
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
                    crate::ops::PromotionStatus::Pending => {
                        return Ok(SettleOutcome::Pending {
                            reason: "the signed keep is waiting for the publisher's promotion"
                                .into(),
                        })
                    }
                    crate::ops::PromotionStatus::Declined => {
                        return Err(PlanError::Validation(format!(
                            "experiment {experiment_id} has a signed keep but the publisher declined the promotion"
                        )))
                    }
                }
            } else {
                ops.promote_experiment(experiment_id)
                    .await
                    .map_err(|source| PlanError::ApplyFailed { source })?;
            }
            if let Some(prereg) = index.pre_registration(&start.instance_id).cloned() {
                append_holdout_state(
                    ops,
                    ledger,
                    index,
                    &prereg,
                    change_seq,
                    &start.started_day,
                    now_unix_ms,
                )
                .await?;
            }
            if !index.is_observed(experiment_id) {
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
                            winner_variant_id: None,
                        },
                    ))
                    .map_err(PlanError::Ledger)?;
                index.observe(&observed);
            }
            Ok(SettleOutcome::Kept {
                experiment_id: experiment_id.to_string(),
                lift: recovered.estimate.lift,
                lambda: recovered.lambda,
                lower_90: recovered.lower_90,
                bootstrap_lower: recovered.bootstrap_lower,
                result_seq: recovered.seq,
                daily_log_ratios: recovered.ratios,
                unmeasured: false,
                variant_leader: None,
            })
        }
        "rollback" => {
            ops.deprecate_experiment(experiment_id)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?;
            if !ops.change_in_test_arm() {
                ops.rollback(&action)
                    .await
                    .map_err(|source| PlanError::ApplyFailed { source })?;
            }
            let rollback = if index.is_rolled_back(change_seq) {
                None
            } else {
                Some(
                    ledger
                        .append_rollback(
                            now_unix_ms,
                            platform,
                            &ad_unit_id,
                            change_seq,
                            "reconciled signed rollback result",
                        )
                        .map_err(PlanError::Ledger)?,
                )
            };
            if let Some(rollback) = &rollback {
                index.observe(rollback);
            }
            Ok(SettleOutcome::RolledBack {
                lift: recovered.estimate.lift,
                reason: "reconciled signed rollback result".into(),
                rollback_seq: rollback
                    .as_ref()
                    .map(LedgerRecord::seq)
                    .unwrap_or(recovered.seq),
                guardrails: Vec::new(),
                daily_log_ratios: recovered.ratios,
                unmeasured: false,
                variant_leader: None,
            })
        }
        "void" => {
            ops.deprecate_experiment(experiment_id)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?;
            if !index.is_observed(experiment_id) {
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
                            winner_variant_id: None,
                        },
                    ))
                    .map_err(PlanError::Ledger)?;
                index.observe(&observed);
            }
            Ok(SettleOutcome::Voided {
                reason: "reconciled signed void result".into(),
                guardrails: Vec::new(),
            })
        }
        other => Err(PlanError::Validation(format!(
            "experiment {experiment_id} has a non-terminal result '{other}' to reconcile"
        ))),
    }
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
    // A signed terminal result with no completion marker is a crash between
    // proof and the remote side effect. Finish that side effect from the
    // signed decision instead of recomputing (or refusing) the experiment.
    if let Some(start) = index.start(experiment_id, change_seq) {
        if index.pending_terminal(experiment_id, &start.instance_id, change_seq) {
            return reconcile_terminal(ops, ledger, index, experiment_id, change_seq, now_unix_ms)
                .await;
        }
    }
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

    let pending_unmeasured = !ops.measures_variants()
        && index.has_pending_promotion(experiment_id, change_seq)
        && index
            .start(experiment_id, change_seq)
            .and_then(|start| index.result_record(&start.instance_id))
            .is_none();
    if !pending_unmeasured {
        if let Some(refusal) = index.settled_refusal(experiment_id, change_seq) {
            return Err(refusal);
        }
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
    let (players, player_feed_error) = {
        let instance_key = index
            .start(experiment_id, change_seq)
            .map(|start| start.instance_id.clone())
            .unwrap_or_else(|| instance_identity(&prereg));
        let fetched = ops.experiment_players(experiment_id, &instance_key).await;
        match fetched {
            Ok(evidence) => {
                if ops.supports_player_feed() && evidence.rows.is_empty() {
                    (
                        None,
                        Some("the registered per-player feed returned no evidence".to_string()),
                    )
                } else {
                    (Some(evidence), None)
                }
            }
            // A platform that documents no per-variant metric legitimately has
            // no player feed. One that registers a per-user statistic must not
            // silently fall back to the daily ratios when its feed fails.
            Err(_) if !ops.supports_player_feed() => (None, None),
            Err(error) => (
                None,
                Some(format!(
                    "the registered per-player feed is unavailable: {error}"
                )),
            ),
        }
    };

    let bootstrap_seed = deterministic_bootstrap_seed(&prereg);
    let (window, guardrails, ratios, estimate, decision) = decide_window(
        &prereg,
        &days,
        players.as_ref(),
        player_feed_error.as_deref(),
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
                players.as_ref(),
                player_feed_error.as_deref(),
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
            // The signed terminal proof lands before the platform end call:
            // a crash can leave the experiment running, never the reverse.
            let terminal_estimate = estimate.unwrap_or_else(zero_estimate);
            append_guardrails(ledger, index, &guardrails, now_unix_ms)?;
            append_signed_result(
                ledger,
                index,
                &prereg,
                &decision,
                &terminal_estimate,
                ratios.clone(),
                0.0,
                now_unix_ms,
            )?;
            ops.deprecate_experiment(experiment_id)
                .await
                .map_err(|source| PlanError::ApplyFailed { source })?;
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
                        winner_variant_id: None,
                    },
                ))
                .map_err(PlanError::Ledger)?;
            index.observe(&observed);
            return Ok(SettleOutcome::Voided {
                reason: reason.clone(),
                guardrails: guardrails.clone(),
            });
        }
        Decision::Rollback { reason } => {
            // Proof first, then the irreversible platform/billing-side
            // rollback. The guardrails are part of that proof.
            let terminal_estimate = estimate.unwrap_or_else(zero_estimate);
            append_guardrails(ledger, index, &guardrails, now_unix_ms)?;
            append_signed_result(
                ledger,
                index,
                &prereg,
                &decision,
                &terminal_estimate,
                ratios.clone(),
                0.0,
                now_unix_ms,
            )?;
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

    let keep_decision = decision.clone();
    let Decision::Keep {
        lambda,
        estimate: keep_estimate,
        bootstrap_lower,
        ..
    } = keep_decision
    else {
        return Err(PlanError::Validation(
            "decide returned a non-keep decision after the keep match".into(),
        ));
    };
    let keep_result_seq: u64;

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
            crate::ops::PromotionStatus::Confirmed => {
                // The publisher promoted the change in the dashboard. Record
                // the signed keep before the observation that completes it.
                append_guardrails(ledger, index, &guardrails, now_unix_ms)?;
                keep_result_seq = append_signed_result(
                    ledger,
                    index,
                    &prereg,
                    &decision,
                    &keep_estimate,
                    ratios.clone(),
                    bootstrap_lower,
                    now_unix_ms,
                )?;
            }
            crate::ops::PromotionStatus::Declined => {
                let reason =
                    "the publisher kept the control arm instead of promoting the test; no fee"
                        .to_string();
                append_guardrails(ledger, index, &guardrails, now_unix_ms)?;
                append_signed_result(
                    ledger,
                    index,
                    &prereg,
                    &Decision::Rollback {
                        reason: reason.clone(),
                    },
                    &keep_estimate,
                    ratios.clone(),
                    0.0,
                    now_unix_ms,
                )?;
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
                        let reason = "the promotion deadline passed without the change serving everyone; the variant arm was ended and no fee applies".to_string();
                        append_guardrails(ledger, index, &guardrails, now_unix_ms)?;
                        append_signed_result(
                            ledger,
                            index,
                            &prereg,
                            &Decision::Rollback {
                                reason: reason.clone(),
                            },
                            &keep_estimate,
                            ratios.clone(),
                            0.0,
                            now_unix_ms,
                        )?;
                        ops.deprecate_experiment(experiment_id)
                            .await
                            .map_err(|source| PlanError::ApplyFailed { source })?;
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
        // Proof before the irreversible promotion: the signed keep is durable
        // before the platform serves the change to all users.
        append_guardrails(ledger, index, &guardrails, now_unix_ms)?;
        keep_result_seq = append_signed_result(
            ledger,
            index,
            &prereg,
            &decision,
            &keep_estimate,
            ratios.clone(),
            bootstrap_lower,
            now_unix_ms,
        )?;
        ops.promote_experiment(experiment_id)
            .await
            .map_err(|source| PlanError::ApplyFailed { source })?;
    }

    let kept_day = crate::calendar::format_day(now_unix_ms.div_euclid(DAY_MS));
    append_holdout_state(
        ops,
        ledger,
        index,
        &prereg,
        change_seq,
        &kept_day,
        now_unix_ms,
    )
    .await?;

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

    let result_seq = keep_result_seq;

    Ok(SettleOutcome::Kept {
        experiment_id: experiment_id.to_string(),
        lift: keep_estimate.lift,
        lambda,
        lower_90: keep_estimate.lower_90,
        bootstrap_lower,
        result_seq,
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
    fn only_a_requested_state_needs_the_provider_confirmation() {
        use apex_lift::holdout::HoldoutStatus;
        assert!(holdout_needs_confirmation(HoldoutStatus::Requested));
        assert!(!holdout_needs_confirmation(HoldoutStatus::Active));
        assert!(!holdout_needs_confirmation(HoldoutStatus::RolledBack));
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
