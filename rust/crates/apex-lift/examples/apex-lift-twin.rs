//! The Rust half of the lift twin: read one JSON case, run the production
//! `apex-lift` engine, and print the canonical result the Python reference
//! kit must reproduce.
//!
//! Usage:
//!     cargo run -q -p apex-lift --example apex-lift-twin -- case.json
//!     cat case.json | cargo run -q -p apex-lift --example apex-lift-twin
//!
//! The example intentionally keeps orchestration here and calls the public
//! engine modules directly, so the twin compares datasets rather than a
//! second copy of the Rust rules.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::Read;

use apex_core::Micros;
use apex_lift::billing::{monthly_fees, KeptChange};
use apex_lift::decision::{
    decide, decide_with_estimate, Decision, DecisionInput, GuardrailThresholds, Metric,
    PreRegistration,
};
use apex_lift::guardrails::{
    evaluate_with_unique_players, split_chi_square, FillFormat, GuardrailEvent,
};
use apex_lift::player_store::{estimate_window, player_window, PlayerDay, PlayerWindow};
use apex_lift::rerun::{parse_csv, rerun_with_burn_in, RERUN_BURN_IN_DAYS};
use apex_lift::stats::{Estimate, PlayerHtEstimate};
use apex_lift::switchback::{decide as switchback_decide, plan as switchback_plan};
use apex_lift::window::{build_window, AnalysisDay, AnalysisWindow, ArmDay, WindowProblem};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    id: String,
    prereg: PreregIn,
    window: WindowIn,
    #[serde(default)]
    guardrails: GuardrailsIn,
    #[serde(default)]
    players: Option<PlayersIn>,
    #[serde(default)]
    switchback: Option<SwitchbackIn>,
    #[serde(default)]
    billing: Option<BillingIn>,
    #[serde(default)]
    rerun: Option<RerunIn>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreregIn {
    experiment_id: String,
    instance_id: String,
    scope: String,
    change_description: String,
    before_value: String,
    test_share: f64,
    burn_in_days: u32,
    window_days: usize,
    interim_look: bool,
    extension_allowed: bool,
    code_version_hash: String,
    rate_bp: u16,
    harm_point: f64,
    fill_rewarded: f64,
    fill_banner: f64,
    split_chi_square: f64,
    min_point_lift: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowIn {
    control: Vec<ArmDayIn>,
    test: Vec<ArmDayIn>,
    test_allocated_share: f64,
    at_final_look: bool,
    extension_used: bool,
    observed_scope: String,
    observed_change_description: String,
    bootstrap_iterations: usize,
    bootstrap_block_len: usize,
    bootstrap_seed: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArmDayIn {
    day: String,
    revenue: Micros,
    users: Option<u64>,
    second_pull_revenue: Option<Micros>,
    second_pull_users: Option<u64>,
    fill: Option<f64>,
    second_pull_fill: Option<f64>,
    first_observed_at_unix_s: Option<i64>,
    second_observed_at_unix_s: Option<i64>,
    first_source_sha256: Option<String>,
    second_source_sha256: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct GuardrailsIn {
    arm_counts: Vec<(u64, u64)>,
    unique_players: Option<(u64, u64)>,
    fill_format: Option<String>,
    settings_changed: Vec<String>,
    stale_days: u32,
    incident_runs: Vec<(String, u32)>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayersIn {
    rows: Vec<PlayerDayIn>,
    #[serde(default)]
    pretest: BTreeMap<String, Micros>,
    #[serde(default)]
    cuped: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerDayIn {
    day: String,
    user_id: Option<String>,
    idfv: Option<String>,
    idfa: Option<String>,
    test: bool,
    revenue_micros: Micros,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwitchbackIn {
    block_count: usize,
    seed: u64,
    #[serde(default)]
    values: Vec<f64>,
    #[serde(default)]
    used_blocks_per_arm: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BillingIn {
    scope_revenue_micros: Micros,
    #[serde(default = "default_kept_day")]
    kept_day: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RerunIn {
    csv: String,
    window_days: usize,
    allocated_share: f64,
    #[serde(default = "default_burn_in")]
    burn_in_days: usize,
}

fn default_kept_day() -> String {
    "2026-01-01".to_string()
}

fn default_burn_in() -> usize {
    RERUN_BURN_IN_DAYS
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    id: String,
    window: WindowOut,
    daily_estimate: Option<EstimateOut>,
    player_estimate: Option<PlayerEstimateOut>,
    guardrails: Vec<GuardrailOut>,
    decision: DecisionOut,
    split: SplitOut,
    switchback: Option<SwitchbackOut>,
    rerun: Option<RerunOut>,
    fee: Option<FeeOut>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WindowOut {
    complete: bool,
    incomplete: bool,
    fatal: bool,
    days: Vec<DayOut>,
    problems: Vec<ProblemOut>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DayOut {
    day: String,
    test_value: f64,
    control_value: f64,
    log_ratio: f64,
    fill_delta: Option<f64>,
    calendar_index: usize,
    test_users: Option<u64>,
    control_users: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProblemOut {
    kind: String,
    day: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EstimateOut {
    days: usize,
    mean_log: f64,
    lift: f64,
    sd: f64,
    lower_90: f64,
    lower_99: f64,
    upper_90: f64,
    upper_99: f64,
    lower_alpha: f64,
    lower_interim: f64,
}

impl From<&Estimate> for EstimateOut {
    fn from(value: &Estimate) -> Self {
        Self {
            days: value.days,
            mean_log: value.mean_log,
            lift: value.lift,
            sd: value.sd,
            lower_90: value.lower_90,
            lower_99: value.lower_99,
            upper_90: value.upper_90,
            upper_99: value.upper_99,
            lower_alpha: value.lower_alpha,
            lower_interim: value.lower_interim,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayerEstimateOut {
    estimate: EstimateOut,
    variance: f64,
    test_probability: f64,
    control_probability: f64,
    test_players: usize,
    control_players: usize,
    test_mean: f64,
    control_mean: f64,
}

impl From<&PlayerHtEstimate> for PlayerEstimateOut {
    fn from(value: &PlayerHtEstimate) -> Self {
        Self {
            estimate: EstimateOut::from(&value.estimate),
            variance: value.variance,
            test_probability: value.test_probability,
            control_probability: value.control_probability,
            test_players: value.test_players,
            control_players: value.control_players,
            test_mean: value.test_mean,
            control_mean: value.control_mean,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GuardrailOut {
    kind: String,
    day: String,
    value: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DecisionOut {
    verdict: String,
    lambda: Option<f64>,
    used_interim99: bool,
    bootstrap_lower: Option<f64>,
    reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SplitOut {
    chi_square: Option<f64>,
    test_players: Option<u64>,
    control_players: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SwitchbackOut {
    plan: Vec<bool>,
    blocks_per_arm: usize,
    billable: bool,
    lambda: Option<f64>,
    estimate: Option<EstimateOut>,
    reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RerunOut {
    days: usize,
    mean_log: f64,
    lift: f64,
    lower_90: f64,
    upper_90: f64,
    daily_log_ratios: Vec<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeeOut {
    lambda: f64,
    factor: f64,
    incremental: Micros,
    log_share: f64,
    rate_bp: u16,
    fee_micros: Micros,
}

fn arm_day(input: &ArmDayIn) -> ArmDay {
    ArmDay {
        day: input.day.clone(),
        revenue: input.revenue,
        users: input.users,
        second_pull_revenue: input.second_pull_revenue,
        second_pull_users: input.second_pull_users,
        fill: input.fill,
        second_pull_fill: input.second_pull_fill,
        first_observed_at_unix_s: input.first_observed_at_unix_s,
        second_observed_at_unix_s: input.second_observed_at_unix_s,
        first_source_sha256: input.first_source_sha256.clone(),
        second_source_sha256: input.second_source_sha256.clone(),
    }
}

fn player_day(input: &PlayerDayIn) -> PlayerDay {
    PlayerDay {
        day: input.day.clone(),
        user_id: input.user_id.clone(),
        idfv: input.idfv.clone(),
        idfa: input.idfa.clone(),
        test: input.test,
        revenue_micros: input.revenue_micros,
    }
}

fn problem_out(problem: &WindowProblem) -> ProblemOut {
    let (kind, day) = match problem {
        WindowProblem::MissingDay { day } => ("missingDay", day),
        WindowProblem::NotFinal { day } => ("notFinal", day),
        WindowProblem::BurnIn { day } => ("burnIn", day),
        WindowProblem::Incident { day } => ("incident", day),
        WindowProblem::Sequence { day } => ("sequence", day),
        WindowProblem::Schema { day } => ("schema", day),
    };
    ProblemOut {
        kind: kind.to_string(),
        day: day.clone(),
    }
}

fn day_out(day: &AnalysisDay) -> DayOut {
    DayOut {
        day: day.day.clone(),
        test_value: day.test_value,
        control_value: day.control_value,
        log_ratio: day.log_ratio,
        fill_delta: day.fill_delta,
        calendar_index: day.calendar_index,
        test_users: day.test_users,
        control_users: day.control_users,
    }
}

fn window_out(window: &AnalysisWindow) -> WindowOut {
    WindowOut {
        complete: window.complete,
        incomplete: window.incomplete,
        fatal: window.fatal,
        days: window.days.iter().map(day_out).collect(),
        problems: window.problems.iter().map(problem_out).collect(),
    }
}

fn guardrail_out(event: &GuardrailEvent) -> GuardrailOut {
    let (kind, day, value) = match event {
        GuardrailEvent::FastHarm { day, point_lift } => ("fastHarm", day, Some(*point_lift)),
        GuardrailEvent::ConfirmedHarm { day, upper_bound } => {
            ("confirmedHarm", day, Some(*upper_bound))
        }
        GuardrailEvent::FillRewardedInterstitial { day, delta } => {
            ("fillRewardedInterstitial", day, Some(*delta))
        }
        GuardrailEvent::FillBanner { day, delta } => ("fillBanner", day, Some(*delta)),
        GuardrailEvent::PublisherEdit { day } => ("publisherEdit", day, None),
        GuardrailEvent::SplitMismatch { day, chi_square } => {
            ("splitMismatch", day, Some(*chi_square))
        }
        GuardrailEvent::ReportingGap { day, stale_days } => {
            ("reportingGap", day, Some(f64::from(*stale_days)))
        }
        GuardrailEvent::PlatformIncident { day } => ("platformIncident", day, None),
    };
    GuardrailOut {
        kind: kind.to_string(),
        day: day.clone(),
        value,
    }
}

fn decision_out(decision: &Decision) -> DecisionOut {
    match decision {
        Decision::Keep {
            lambda,
            used_interim_99,
            bootstrap_lower,
            ..
        } => DecisionOut {
            verdict: "keep".to_string(),
            lambda: Some(*lambda),
            used_interim99: *used_interim_99,
            bootstrap_lower: Some(*bootstrap_lower),
            reason: String::new(),
        },
        Decision::Rollback { reason } => DecisionOut {
            verdict: "rollback".to_string(),
            lambda: None,
            used_interim99: false,
            bootstrap_lower: None,
            reason: reason.clone(),
        },
        Decision::Void { reason } => DecisionOut {
            verdict: "void".to_string(),
            lambda: None,
            used_interim99: false,
            bootstrap_lower: None,
            reason: reason.clone(),
        },
        Decision::Pending { reason } => DecisionOut {
            verdict: "pending".to_string(),
            lambda: None,
            used_interim99: false,
            bootstrap_lower: None,
            reason: reason.clone(),
        },
        Decision::Extend { reason } => DecisionOut {
            verdict: "extend".to_string(),
            lambda: None,
            used_interim99: false,
            bootstrap_lower: None,
            reason: reason.clone(),
        },
    }
}

fn split_out(guardrails: &GuardrailsIn, prereg: &PreRegistration) -> SplitOut {
    let split = match guardrails.unique_players {
        Some(unique) => Some(unique),
        None => {
            let pooled = guardrails
                .arm_counts
                .iter()
                .fold((0u64, 0u64), |(test, control), (dt, dc)| {
                    (test + dt, control + dc)
                });
            (pooled.0 + pooled.1 > 0).then_some(pooled)
        }
    };
    match split {
        Some((test, control)) => SplitOut {
            chi_square: split_chi_square(test, control, prereg.test_share),
            test_players: Some(test),
            control_players: Some(control),
        },
        None => SplitOut {
            chi_square: None,
            test_players: None,
            control_players: None,
        },
    }
}

fn fee_out(billing: &BillingIn, prereg: &PreRegistration, decision: &Decision) -> Option<FeeOut> {
    let lambda = match decision {
        Decision::Keep { lambda, .. } => *lambda,
        _ => 0.0,
    };
    if lambda <= 0.0 {
        return Some(FeeOut {
            lambda,
            factor: 1.0,
            incremental: 0,
            log_share: 1.0,
            rate_bp: prereg.rate_bp,
            fee_micros: 0,
        });
    }
    let change = KeptChange {
        change_id: prereg.experiment_id.clone(),
        lambda,
        rate_bp: prereg.rate_bp,
        kept_day: billing.kept_day.clone(),
        overridden_day: None,
        rolled_back_day: None,
        pre_registration_hash: String::new(),
        result_hash: String::new(),
    };
    let lines = monthly_fees(billing.scope_revenue_micros, &[change]).ok()?;
    let line = lines.first()?;
    Some(FeeOut {
        lambda: line.lambda,
        factor: line.factor,
        incremental: line.incremental,
        log_share: line.log_share,
        rate_bp: line.rate_bp,
        fee_micros: line.fee_micros,
    })
}

fn switchback_out(input: &SwitchbackIn) -> SwitchbackOut {
    let Some(planned) = switchback_plan(input.block_count, input.seed) else {
        return SwitchbackOut {
            plan: Vec::new(),
            blocks_per_arm: 0,
            billable: false,
            lambda: None,
            estimate: None,
            reason: "no weekday-balanced plan for this block count".to_string(),
        };
    };
    if input.values.is_empty() {
        return SwitchbackOut {
            plan: planned.blocks.clone(),
            blocks_per_arm: planned.blocks_per_arm,
            billable: false,
            lambda: None,
            estimate: None,
            reason: "no observations".to_string(),
        };
    }
    match switchback_decide(&planned, &input.values, input.used_blocks_per_arm) {
        apex_lift::switchback::SwitchbackDecision::Billable { lambda, estimate } => SwitchbackOut {
            plan: planned.blocks,
            blocks_per_arm: planned.blocks_per_arm,
            billable: true,
            lambda: Some(lambda),
            estimate: Some(EstimateOut::from(&estimate)),
            reason: String::new(),
        },
        apex_lift::switchback::SwitchbackDecision::NotBillable { reason } => SwitchbackOut {
            plan: planned.blocks,
            blocks_per_arm: planned.blocks_per_arm,
            billable: false,
            lambda: None,
            estimate: None,
            reason,
        },
    }
}

fn run(case: &Case) -> Result<Output, String> {
    let thresholds = GuardrailThresholds {
        harm_point: case.prereg.harm_point,
        fill_rewarded: case.prereg.fill_rewarded,
        fill_banner: case.prereg.fill_banner,
        split_chi_square: case.prereg.split_chi_square,
    };
    let prereg = PreRegistration {
        experiment_id: case.prereg.experiment_id.clone(),
        instance_id: case.prereg.instance_id.clone(),
        scope: case.prereg.scope.clone(),
        change_description: case.prereg.change_description.clone(),
        before_value: case.prereg.before_value.clone(),
        test_share: case.prereg.test_share,
        metric: Metric::RevenuePerUser,
        burn_in_days: case.prereg.burn_in_days,
        window_days: case.prereg.window_days,
        interim_look: case.prereg.interim_look,
        extension_allowed: case.prereg.extension_allowed,
        code_version_hash: case.prereg.code_version_hash.clone(),
        rate_bp: case.prereg.rate_bp,
        thresholds,
        min_point_lift: case.prereg.min_point_lift,
        randomization_unit: apex_lift::records::default_randomization_unit(),
        assignment_source: "provider".to_string(),
        assignment_epoch: apex_lift::records::default_assignment_epoch(),
        estimator_id: apex_lift::records::default_estimator_id(),
        outcome_source: apex_lift::records::PLAYER_REVENUE_OUTCOME_SOURCE.to_string(),
        measurement_scope: case.prereg.scope.clone(),
        eligibility: apex_lift::records::default_eligibility(),
        identity_contract: apex_lift::records::default_identity_contract(),
    };

    let control: Vec<ArmDay> = case.window.control.iter().map(arm_day).collect();
    let test: Vec<ArmDay> = case.window.test.iter().map(arm_day).collect();
    let window = build_window(
        &control,
        &test,
        case.prereg.window_days,
        case.window.test_allocated_share,
        case.prereg.burn_in_days as usize,
    );

    let fill_format = match case.guardrails.fill_format.as_deref() {
        Some("banner") => FillFormat::Banner,
        Some("interstitial") => FillFormat::Interstitial,
        _ => FillFormat::Rewarded,
    };
    let guardrails = evaluate_with_unique_players(
        &prereg,
        &window.days,
        case.guardrails.unique_players,
        &case.guardrails.arm_counts,
        fill_format,
        &case.guardrails.settings_changed,
        case.guardrails.stale_days,
        &case.guardrails.incident_runs,
    );

    let input = DecisionInput {
        window: &window,
        guardrails: &guardrails,
        observed_scope: case.window.observed_scope.clone(),
        observed_change_description: case.window.observed_change_description.clone(),
        at_final_look: case.window.at_final_look,
        extension_used: case.window.extension_used,
        bootstrap_iterations: case.window.bootstrap_iterations,
        bootstrap_block_len: case.window.bootstrap_block_len,
        bootstrap_seed: case.window.bootstrap_seed,
    };

    let daily_ratios: Vec<f64> = window.days.iter().map(|day| day.log_ratio).collect();
    let daily_estimate = apex_lift::stats::estimate(&daily_ratios);
    let mut player_estimate = None;
    let decision = match &case.players {
        Some(players) => {
            let rows: Vec<PlayerDay> = players.rows.iter().map(player_day).collect();
            let folded: PlayerWindow =
                player_window(&rows, Some(&players.pretest)).map_err(|error| error.to_string())?;
            let ht = estimate_window(&folded, prereg.test_share, players.cuped);
            player_estimate = ht.as_ref().map(PlayerEstimateOut::from);
            match ht {
                Some(ht) => decide_with_estimate(&prereg, &input, ht.estimate),
                None => decide(&prereg, &input),
            }
        }
        None => decide(&prereg, &input),
    };

    let fee = case
        .billing
        .as_ref()
        .and_then(|billing| fee_out(billing, &prereg, &decision));

    let rerun = match &case.rerun {
        Some(rerun) => {
            let rows = parse_csv(&rerun.csv).map_err(|error| error.to_string())?;
            let report = rerun_with_burn_in(
                &rows,
                rerun.window_days,
                rerun.allocated_share,
                rerun.burn_in_days,
            )
            .map_err(|error| error.to_string())?;
            Some(RerunOut {
                days: report.days,
                mean_log: report.mean_log,
                lift: report.lift,
                lower_90: report.lower_90,
                upper_90: report.upper_90,
                daily_log_ratios: report.daily_log_ratios,
            })
        }
        None => None,
    };

    Ok(Output {
        id: case.id.clone(),
        window: window_out(&window),
        daily_estimate: daily_estimate.as_ref().map(EstimateOut::from),
        player_estimate,
        guardrails: guardrails.iter().map(guardrail_out).collect(),
        decision: decision_out(&decision),
        split: split_out(&case.guardrails, &prereg),
        switchback: case.switchback.as_ref().map(switchback_out),
        rerun,
        fee,
    })
}

fn read_input() -> Result<String, String> {
    let path = std::env::args().nth(1);
    match path {
        Some(path) => {
            std::fs::read_to_string(&path).map_err(|error| format!("read {path}: {error}"))
        }
        None => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|error| format!("read stdin: {error}"))?;
            Ok(text)
        }
    }
}

fn main() {
    let result = (|| -> Result<(), String> {
        let text = read_input()?;
        let case: Case = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        let output = run(&case)?;
        let json = serde_json::to_string_pretty(&output).map_err(|error| error.to_string())?;
        println!("{json}");
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("apex-lift-twin: {error}");
        std::process::exit(1);
    }
}
