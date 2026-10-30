//! The keep rule: six conditions at the pre-registered final look, a single
//! stricter interim look at the whole-week midpoint, and one pre-registered
//! extension offered only at the final look. Anything else rolls back or
//! voids, and only a Keep produces a billable lambda.

use crate::error::{LiftError, LiftResult};
use crate::guardrails::GuardrailEvent;
use crate::records::LiftRecord;
use crate::stats::{bootstrap_lower_bound, estimate, Estimate};
use crate::window::{AnalysisWindow, WindowProblem};

/// The pre-registered metric. The engine bills revenue per user only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    RevenuePerUser,
}

/// Contract-tightenable guardrail thresholds. The defaults below are the
/// loosest values the spec allows; a publisher may only tighten them.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailThresholds {
    /// Fast-harm point-lift stop. Default -0.05; a publisher may stop earlier,
    /// never later: the value must be in (-0.05, 0].
    pub harm_point: f64,
    /// Rewarded and interstitial fill drop. Default 0.02; tighten downward.
    pub fill_rewarded: f64,
    /// Banner fill drop. Default 0.05; tighten downward.
    pub fill_banner: f64,
    /// Split-mismatch chi-square threshold. Default 10.8276 (p = 0.001);
    /// tighten downward (stop more easily), never upward.
    pub split_chi_square: f64,
}

impl Default for GuardrailThresholds {
    fn default() -> Self {
        Self {
            harm_point: -0.05,
            fill_rewarded: 0.02,
            fill_banner: 0.05,
            split_chi_square: 10.8276,
        }
    }
}

impl GuardrailThresholds {
    /// Refuse anything looser than the spec defaults.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.harm_point <= 0.0 && self.harm_point >= -0.05) {
            return Err(format!(
                "harm_point {} must be in (-0.05, 0]: tighter is allowed, looser is not",
                self.harm_point
            ));
        }
        if !(self.fill_rewarded >= 0.0 && self.fill_rewarded <= 0.02) {
            return Err(format!(
                "fill_rewarded {} must be in [0, 0.02]",
                self.fill_rewarded
            ));
        }
        if !(self.fill_banner >= 0.0 && self.fill_banner <= 0.05) {
            return Err(format!(
                "fill_banner {} must be in [0, 0.05]",
                self.fill_banner
            ));
        }
        if !(self.split_chi_square > 0.0 && self.split_chi_square <= 10.8276) {
            return Err(format!(
                "split_chi_square {} must be in (0, 10.8276]",
                self.split_chi_square
            ));
        }
        Ok(())
    }
}

/// What was written to the signed ledger before the first platform call.
#[derive(Debug, Clone, PartialEq)]
pub struct PreRegistration {
    pub experiment_id: String,
    /// The immutable experiment instance this registration pins. The platform
    /// experiment name (`experiment_id`) may be reused by a restart; this id
    /// never is.
    pub instance_id: String,
    pub scope: String,
    /// Human-auditable description of the single change under test.
    pub change_description: String,
    /// The setting's value before the change, as read from the platform
    /// before the first write. Signed in the pre-registration record.
    pub before_value: String,
    /// Planned test-arm share of users: 0.5, 0.25 or 0.10.
    pub test_share: f64,
    pub metric: Metric,
    pub burn_in_days: u32,
    /// 7, 14, 21 or 28.
    pub window_days: usize,
    /// Whether the single interim look is allowed.
    pub interim_look: bool,
    /// Whether the one-week extension is allowed.
    pub extension_allowed: bool,
    /// Hash of the analysis-code version that will run.
    pub code_version_hash: String,
    /// Contract rate in basis points, 1500..=2000.
    pub rate_bp: u16,
    /// Tightenable guardrail thresholds.
    pub thresholds: GuardrailThresholds,
    /// The minimum point lift this registration pinned for the keep rule.
    /// Both the interim and final looks use this value; there is no engine
    /// constant.
    pub min_point_lift: f64,
    /// The unit randomization happens on ("user").
    pub randomization_unit: String,
    /// Who assigns a unit to an arm ("provider" / "apex" / "publisher").
    pub assignment_source: String,
    /// The assignment epoch the split belongs to (string-encoded u64).
    pub assignment_epoch: String,
    /// The estimator this registration pins
    /// ("planned_share_total_v2"); a different id is a different contract.
    pub estimator_id: String,
    /// What the estimator measures outcomes from
    /// ("user_revenue" / "daily_ratio").
    pub outcome_source: String,
    /// The measurement scope; registered equal to [`Self::scope`].
    pub measurement_scope: String,
    /// Who counts as measurable ("eligible_users").
    pub eligibility: String,
    /// The identity keys the measurement joins on
    /// ("idfv_idfa_user").
    pub identity_contract: String,
}

impl PreRegistration {
    /// Refuse a registration the engine could not honor; decide() voids with
    /// this reason so an invalid registration never reaches billing.
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.test_share, s if s == 0.5 || s == 0.25 || s == 0.10) {
            return Err(format!(
                "test_share {} must be one of 0.5, 0.25, 0.10",
                self.test_share
            ));
        }
        if !crate::window::ALLOWED_WINDOW_DAYS.contains(&self.window_days) {
            return Err(format!(
                "window_days {} must be one of 7, 14, 21, 28",
                self.window_days
            ));
        }
        if !(1500..=2000).contains(&self.rate_bp) {
            return Err(format!(
                "rate_bp {} must be in the contracted 1500..=2000 band",
                self.rate_bp
            ));
        }
        if self.code_version_hash.trim().is_empty() {
            return Err("code_version_hash must not be empty".into());
        }
        if self.before_value.trim().is_empty() {
            return Err("before_value must carry the prior platform value".into());
        }
        if !self.min_point_lift.is_finite()
            || self.min_point_lift < 0.0
            || self.min_point_lift > 1.0
        {
            return Err(format!(
                "min_point_lift {} must be a finite number in [0, 1]",
                self.min_point_lift
            ));
        }
        self.thresholds.validate()
    }

    /// Reconstruct the registration from the signed record that was written
    /// before the test started. Settlement is driven by this, never by a
    /// registration rebuilt at settle time: a test can only be judged against
    /// what was pinned before its first platform call.
    pub fn from_record(record: &LiftRecord) -> LiftResult<Self> {
        let LiftRecord::PreRegistration {
            experiment_id,
            instance_id,
            scope,
            before_value,
            after_value,
            split,
            metric,
            burn_in_days,
            window_days,
            looks,
            extension_allowed,
            guardrail_thresholds,
            min_point_lift,
            code_version_hash,
            rate_bp,
            randomization_unit,
            assignment_source,
            assignment_epoch,
            estimator_id,
            outcome_source,
            measurement_scope,
            eligibility,
            identity_contract,
        } = record
        else {
            return Err(LiftError::Malformed(
                "record is not a pre-registration".into(),
            ));
        };
        if metric != "revenue_per_user" {
            return Err(LiftError::Malformed(format!(
                "unknown pre-registered metric '{metric}'"
            )));
        }
        let change_description = after_value
            .get("change")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                LiftError::Malformed("pre-registration after_value lost its change".into())
            })?;
        let before_value = match before_value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        let f64_field = |name: &str| -> LiftResult<f64> {
            guardrail_thresholds
                .get(name)
                .and_then(|value| value.as_f64())
                .ok_or_else(|| {
                    LiftError::Malformed(format!(
                        "guardrail threshold '{name}' missing or non-numeric"
                    ))
                })
        };
        let thresholds = GuardrailThresholds {
            harm_point: f64_field("harmPoint")?,
            fill_rewarded: f64_field("fillRewarded")?,
            fill_banner: f64_field("fillBanner")?,
            split_chi_square: f64_field("splitChiSquare")?,
        };
        let prereg = PreRegistration {
            experiment_id: experiment_id.clone(),
            instance_id: instance_id.clone(),
            scope: scope.clone(),
            change_description: change_description.to_string(),
            before_value,
            test_share: *split,
            metric: Metric::RevenuePerUser,
            burn_in_days: *burn_in_days,
            window_days: *window_days as usize,
            interim_look: *looks >= 2,
            extension_allowed: *extension_allowed,
            code_version_hash: code_version_hash.clone(),
            rate_bp: *rate_bp,
            thresholds,
            min_point_lift: *min_point_lift,
            randomization_unit: randomization_unit.clone(),
            assignment_source: assignment_source.clone(),
            assignment_epoch: assignment_epoch.clone(),
            estimator_id: estimator_id.clone(),
            outcome_source: outcome_source.clone(),
            // A legacy record without a measurement scope measures the
            // registered scope itself.
            measurement_scope: if measurement_scope.is_empty() {
                scope.clone()
            } else {
                measurement_scope.clone()
            },
            eligibility: eligibility.clone(),
            identity_contract: identity_contract.clone(),
        };
        prereg.validate().map_err(LiftError::Malformed)?;
        Ok(prereg)
    }
}

/// The inputs to the decision: the analyzed window, the guardrail events, the
/// live scope identity (condition six) and the cross-check parameters.
#[derive(Debug, Clone)]
pub struct DecisionInput<'a> {
    pub window: &'a AnalysisWindow,
    pub guardrails: &'a [GuardrailEvent],
    /// The scope as it exists on the platform right now; must equal the
    /// pre-registered scope.
    pub observed_scope: String,
    /// The change values as they exist on the platform right now; must equal
    /// the pre-registered change.
    pub observed_change_description: String,
    /// Whether the test has reached its planned final look.
    pub at_final_look: bool,
    pub extension_used: bool,
    pub bootstrap_iterations: usize,
    pub bootstrap_block_len: usize,
    pub bootstrap_seed: u64,
}

/// The decision. `Keep` is the only variant that carries a billable lambda.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Keep the change; bill on `lambda` (the lower bound used).
    Keep {
        lambda: f64,
        estimate: Estimate,
        used_interim_99: bool,
        /// The bootstrap cross-check lower bound, or the interim's analytic
        /// 99% bound when the interim keep ran before a full window.
        bootstrap_lower: f64,
    },
    /// Roll back; no fee.
    Rollback { reason: String },
    /// Void; no decision, no fee.
    Void { reason: String },
    /// The window is not complete; wait.
    Pending { reason: String },
    /// One pre-registered extra week, offered only at the final look.
    Extend { reason: String },
}

const BOOTSTRAP_AGREEMENT: f64 = 0.01;

fn fatal_problem(window: &AnalysisWindow) -> Option<&WindowProblem> {
    window.problems.iter().find(|problem| problem.is_fatal())
}

/// Validate registration and apply the void/rollback/pause guardrails before
/// any estimator runs. Returns the terminal decision when one is already
/// determined.
fn pre_decision(prereg: &PreRegistration, input: &DecisionInput) -> Option<Decision> {
    if let Err(reason) = prereg.validate() {
        return Some(Decision::Void {
            reason: format!("invalid pre-registration: {reason}"),
        });
    }
    // A 10% split is exploratory only and can never be billed, at any look.
    if prereg.test_share == 0.10 {
        return Some(Decision::Rollback {
            reason: "10% splits are exploratory and never billed".into(),
        });
    }
    // A sequence or schema problem is fatal: waiting cannot repair it, so the
    // test voids instead of waiting forever for a window that can never form.
    if let Some(problem) = fatal_problem(input.window) {
        return Some(Decision::Void {
            reason: format!("fatal window problem: {problem:?}"),
        });
    }
    if let Some(event) = input.guardrails.iter().find(|e| e.voids()) {
        return Some(Decision::Void {
            reason: format!("{event:?}"),
        });
    }
    if let Some(event) = input.guardrails.iter().find(|e| e.rolls_back()) {
        return Some(Decision::Rollback {
            reason: format!("{event:?}"),
        });
    }
    // A reporting gap pauses the window until it resolves, and three stale
    // days void it: the gap event carries the count.
    if let Some(event) = input.guardrails.iter().find(|e| e.pauses()) {
        return Some(Decision::Pending {
            reason: format!("{event:?}"),
        });
    }

    None
}

pub fn decide(prereg: &PreRegistration, input: &DecisionInput) -> Decision {
    if let Some(decision) = pre_decision(prereg, input) {
        return decision;
    }
    let ratios: Vec<f64> = input.window.days.iter().map(|d| d.log_ratio).collect();
    let Some(current) = estimate(&ratios) else {
        return Decision::Pending {
            reason: "fewer than two analysis days have accumulated".into(),
        };
    };
    decide_completed(prereg, input, current, true)
}

/// Decide from a caller-supplied estimate. The per-player Horvitz–Thompson
/// estimate replaces the daily-ratio estimate when player evidence exists;
/// every registration, guardrail, interim, extension, keep and cross-check
/// rule stays identical.
pub fn decide_with_estimate(
    prereg: &PreRegistration,
    input: &DecisionInput,
    current: Estimate,
) -> Decision {
    if let Some(decision) = pre_decision(prereg, input) {
        return decision;
    }
    // The per-player Horvitz–Thompson bound is the registered decision
    // statistic. The daily ratio bootstrap describes a different estimator
    // (arm-days), so requiring the two to agree would roll back an honest
    // per-player result whenever the noisy daily series disagrees.
    decide_completed(prereg, input, current, false)
}

fn decide_completed(
    prereg: &PreRegistration,
    input: &DecisionInput,
    current: Estimate,
    cross_check: bool,
) -> Decision {
    // The midpoint interim look is the largest whole week at or below half
    // the window: none for a 7-day window, day 7 for 14 and 21, day 14 for
    // 28.
    let interim_days = (prereg.window_days / 2 / 7) * 7;
    if prereg.interim_look
        && interim_days > 0
        && input.window.days.len() == interim_days
        && !input.window.incomplete
        && current.lower_interim > 0.0
        && current.lift >= prereg.min_point_lift
    {
        let bootstrap = if cross_check {
            let totals: Vec<(f64, f64)> = input
                .window
                .days
                .iter()
                .map(|d| (d.test_value, d.control_value))
                .collect();
            let Some(bootstrap) = bootstrap_lower_bound(
                &totals,
                input.bootstrap_block_len,
                input.bootstrap_iterations,
                input.bootstrap_seed,
            ) else {
                return Decision::Pending {
                    reason: "the bootstrap cross-check could not run".into(),
                };
            };
            Some(bootstrap)
        } else {
            None
        };
        // The interim look is held to the same cross-check as the final look
        // when the daily estimator is the decision statistic; the per-player
        // path carries its own registered bound.
        if bootstrap.is_none_or(|bootstrap| {
            bootstrap > 0.0
                && (bootstrap - current.lower_interim).abs() <= BOOTSTRAP_AGREEMENT + 1e-9
        }) {
            return Decision::Keep {
                // The interim look registers its own 0.5% one-sided error;
                // the historical field name is kept for record compatibility.
                lambda: current.lower_interim,
                estimate: current,
                used_interim_99: true,
                bootstrap_lower: bootstrap.unwrap_or(current.lower_interim),
            };
        }
    }

    // The extension is a final-look decision only: a positive point with a
    // non-positive bound, allowed and unused, buys exactly one extra week of
    // data before the rollback.
    if input.window.complete
        && input.at_final_look
        && current.lift > 0.0
        && current.lower_alpha <= 0.0
        && prereg.extension_allowed
        && !input.extension_used
        && !input.window.incomplete
    {
        return Decision::Extend {
            reason: "positive estimate, bound not above zero: one extra week".into(),
        };
    }

    if !input.window.complete || !input.at_final_look {
        return Decision::Pending {
            reason: format!(
                "{} of {} analysis days are final",
                input.window.days.len(),
                prereg.window_days
            ),
        };
    }

    // The six keep conditions.
    let mut failed: Vec<String> = Vec::new();
    if input.window.incomplete {
        failed.push("the window has non-final, missing or non-consecutive days".into());
    }
    if input.observed_scope != prereg.scope {
        failed.push(format!(
            "the scope changed since pre-registration: {} vs {}",
            input.observed_scope, prereg.scope
        ));
    }
    if input.observed_change_description != prereg.change_description {
        failed.push(format!(
            "the change values changed since pre-registration: {} vs {}",
            input.observed_change_description, prereg.change_description
        ));
    }
    if current.lift < prereg.min_point_lift {
        failed.push(format!(
            "point lift is below the registered minimum {}",
            prereg.min_point_lift
        ));
    }
    if current.lower_alpha <= 0.0 {
        failed.push("the registered 97.75% lower bound is not above zero".into());
    }
    let bootstrap = if cross_check {
        let totals: Vec<(f64, f64)> = input
            .window
            .days
            .iter()
            .map(|d| (d.test_value, d.control_value))
            .collect();
        let Some(bootstrap) = bootstrap_lower_bound(
            &totals,
            input.bootstrap_block_len,
            input.bootstrap_iterations,
            input.bootstrap_seed,
        ) else {
            return Decision::Pending {
                reason: "the bootstrap cross-check could not run".into(),
            };
        };
        if (bootstrap - current.lower_90).abs() > BOOTSTRAP_AGREEMENT + 1e-9 {
            if prereg.extension_allowed
                && !input.extension_used
                && current.lift > 0.0
                && !input.window.incomplete
            {
                return Decision::Extend {
                    reason: "the bootstrap and analytic bounds differ by more than one point"
                        .into(),
                };
            }
            failed.push("the bootstrap cross-check disagrees by more than one point".into());
        }
        bootstrap
    } else {
        current.lower_90
    };
    if !failed.is_empty() {
        return Decision::Rollback {
            reason: failed.join("; "),
        };
    }
    Decision::Keep {
        lambda: current.lower_alpha.max(0.0),
        estimate: current,
        used_interim_99: false,
        bootstrap_lower: bootstrap,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::window::{AnalysisDay, AnalysisWindow, WindowProblem};

    fn prereg() -> PreRegistration {
        PreRegistration {
            experiment_id: "exp-1".into(),
            instance_id: "exp-1@1".into(),
            scope: "u1".into(),
            change_description: "raise t1 floor by 10%".into(),
            before_value: "{\"floor\": 1000000}".into(),
            test_share: 0.25,
            metric: Metric::RevenuePerUser,
            burn_in_days: 3,
            window_days: 14,
            interim_look: true,
            extension_allowed: true,
            code_version_hash: "abc123".into(),
            rate_bp: 2000,
            thresholds: GuardrailThresholds::default(),
            min_point_lift: 0.01,
            randomization_unit: crate::records::default_randomization_unit(),
            assignment_source: crate::records::default_assignment_source(),
            assignment_epoch: crate::records::default_assignment_epoch(),
            estimator_id: crate::records::default_estimator_id(),
            outcome_source: crate::records::default_outcome_source(),
            measurement_scope: "u1".into(),
            eligibility: crate::records::default_eligibility(),
            identity_contract: crate::records::default_identity_contract(),
        }
    }

    fn window(days: usize, ratio: f64, complete: bool) -> AnalysisWindow {
        AnalysisWindow {
            days: (0..days)
                .map(|i| AnalysisDay {
                    day: format!("2026-10-{i:02}"),
                    test_value: ratio.exp(),
                    control_value: 1.0,
                    log_ratio: ratio,
                    fill_delta: None,
                    calendar_index: i,
                    test_users: None,
                    control_users: None,
                })
                .collect(),
            problems: Vec::new(),
            complete,
            incomplete: false,
            fatal: false,
        }
    }

    fn input<'a>(
        window: &'a AnalysisWindow,
        guardrails: &'a [GuardrailEvent],
        at_final_look: bool,
    ) -> DecisionInput<'a> {
        DecisionInput {
            window,
            guardrails,
            observed_scope: "u1".into(),
            observed_change_description: "raise t1 floor by 10%".into(),
            at_final_look,
            extension_used: false,
            bootstrap_iterations: 2000,
            bootstrap_block_len: 7,
            bootstrap_seed: 99,
        }
    }

    #[test]
    fn a_clean_positive_test_is_kept_on_the_lower_bound() {
        let days = window(14, 0.02, true);
        match decide(&prereg(), &input(&days, &[], true)) {
            Decision::Keep {
                lambda,
                used_interim_99,
                ..
            } => {
                let expected = 0.02f64.exp() - 1.0;
                assert!((lambda - expected).abs() < 1e-9, "{lambda}");
                assert!(!used_interim_99);
            }
            other => panic!("expected keep, got {other:?}"),
        }
        let mut noisy = window(14, 0.0, true);
        noisy.days.clear();
        for i in 0..14 {
            let ratio: f64 = if i % 2 == 0 { 0.05 } else { -0.01 };
            noisy.days.push(AnalysisDay {
                day: format!("d{i}"),
                test_value: ratio.exp(),
                control_value: 1.0,
                log_ratio: ratio,
                fill_delta: None,
                calendar_index: i,
                test_users: None,
                control_users: None,
            });
        }
        match decide(&prereg(), &input(&noisy, &[], true)) {
            Decision::Keep { lambda, .. } => {
                let point = 0.02f64.exp() - 1.0;
                assert!(lambda > 0.0 && lambda < point, "{lambda} vs {point}");
            }
            other => panic!("expected noisy keep, got {other:?}"),
        }
    }

    #[test]
    fn a_flat_test_rolls_back_with_no_fee() {
        let days = window(14, 0.0, true);
        let decision = decide(&prereg(), &input(&days, &[], true));
        assert!(
            matches!(decision, Decision::Rollback { .. }),
            "{decision:?}"
        );
    }

    #[test]
    fn the_extension_is_offered_only_at_the_final_look_and_only_once() {
        // A positive point with a straddling bound at the completed final look.
        let mut days = window(14, 0.0, true);
        days.days.clear();
        for i in 0..14 {
            let ratio: f64 = if i % 2 == 0 { 0.10 } else { -0.092 };
            days.days.push(AnalysisDay {
                day: format!("d{i}"),
                test_value: ratio.exp(),
                control_value: 1.0,
                log_ratio: ratio,
                fill_delta: None,
                calendar_index: i,
                test_users: None,
                control_users: None,
            });
        }
        assert!(matches!(
            decide(&prereg(), &input(&days, &[], true)),
            Decision::Extend { .. }
        ));
        let mut used = input(&days, &[], true);
        used.extension_used = true;
        assert!(matches!(
            decide(&prereg(), &used),
            Decision::Rollback { .. }
        ));
        // Mid-test the same numbers stay pending, never extending.
        assert!(matches!(
            decide(&prereg(), &input(&days, &[], false)),
            Decision::Pending { .. }
        ));
    }

    #[test]
    fn the_interim_look_uses_the_whole_week_midpoint() {
        let days = window(7, 0.02, false);
        match decide(&prereg(), &input(&days, &[], false)) {
            Decision::Keep {
                used_interim_99: true,
                bootstrap_lower,
                ..
            } => {
                assert!(bootstrap_lower.is_finite());
            }
            other => panic!("expected interim keep, got {other:?}"),
        }
        // 21-day windows take their interim at day 7, not day 10.
        let mut twenty_one = prereg();
        twenty_one.window_days = 21;
        assert!(matches!(
            decide(&twenty_one, &input(&days, &[], false)),
            Decision::Keep { .. }
        ));
        // 7-day windows have no whole-week midpoint.
        let mut seven = prereg();
        seven.window_days = 7;
        assert!(matches!(
            decide(&seven, &input(&days, &[], false)),
            Decision::Pending { .. }
        ));
    }

    #[test]
    fn an_incomplete_day_blocks_the_interim_keep() {
        let mut days = window(7, 0.02, false);
        days.problems
            .push(WindowProblem::MissingDay { day: "d".into() });
        days.incomplete = true;
        assert!(matches!(
            decide(&prereg(), &input(&days, &[], false)),
            Decision::Pending { .. }
        ));
    }

    #[test]
    fn guardrails_beat_every_keep_condition() {
        let days = window(14, 0.02, true);
        let rollback = [GuardrailEvent::FastHarm {
            day: "d".into(),
            point_lift: -0.06,
        }];
        assert!(matches!(
            decide(&prereg(), &input(&days, &rollback, true)),
            Decision::Rollback { .. }
        ));
        let void = [GuardrailEvent::PublisherEdit { day: "d".into() }];
        assert!(matches!(
            decide(&prereg(), &input(&days, &void, true)),
            Decision::Void { .. }
        ));
        // A stale day pauses; three stale days void.
        let pause = [GuardrailEvent::ReportingGap {
            day: "d".into(),
            stale_days: 1,
        }];
        assert!(matches!(
            decide(&prereg(), &input(&days, &pause, true)),
            Decision::Pending { .. }
        ));
        let void_gap = [GuardrailEvent::ReportingGap {
            day: "d".into(),
            stale_days: 3,
        }];
        assert!(matches!(
            decide(&prereg(), &input(&days, &void_gap, true)),
            Decision::Void { .. }
        ));
    }

    #[test]
    fn a_fatal_sequence_or_schema_problem_voids_instead_of_waiting() {
        let days = window(14, 0.02, true);
        for problem in [
            WindowProblem::Sequence { day: "d".into() },
            WindowProblem::Schema { day: "d".into() },
        ] {
            let mut broken = days.clone();
            broken.problems.push(problem.clone());
            broken.complete = false;
            broken.fatal = true;
            assert!(matches!(
                decide(&prereg(), &input(&broken, &[], true)),
                Decision::Void { .. }
            ));
        }
    }

    #[test]
    fn a_window_with_missing_days_is_never_kept() {
        let mut days = window(14, 0.02, true);
        days.problems
            .push(WindowProblem::MissingDay { day: "d".into() });
        days.incomplete = true;
        assert!(matches!(
            decide(&prereg(), &input(&days, &[], true)),
            Decision::Rollback { .. }
        ));
    }

    #[test]
    fn a_changed_scope_or_value_fails_condition_six() {
        let days = window(14, 0.02, true);
        let mut moved_scope = input(&days, &[], true);
        moved_scope.observed_scope = "u2".into();
        assert!(matches!(
            decide(&prereg(), &moved_scope),
            Decision::Rollback { .. }
        ));
        let mut moved_value = input(&days, &[], true);
        moved_value.observed_change_description = "raise t1 floor by 20%".into();
        assert!(matches!(
            decide(&prereg(), &moved_value),
            Decision::Rollback { .. }
        ));
    }

    #[test]
    fn a_ten_percent_split_is_never_billed_at_any_look() {
        let mut ten = prereg();
        ten.test_share = 0.10;
        let final_look = window(14, 0.02, true);
        match decide(&ten, &input(&final_look, &[], true)) {
            Decision::Rollback { reason } => assert!(reason.contains("exploratory"), "{reason}"),
            other => panic!("expected rollback, got {other:?}"),
        }
        // The interim look must refuse it too: it runs before any final-look
        // gate.
        let interim = window(7, 0.02, false);
        match decide(&ten, &input(&interim, &[], false)) {
            Decision::Rollback { reason } => assert!(reason.contains("exploratory"), "{reason}"),
            other => panic!("expected interim rollback, got {other:?}"),
        }
    }

    #[test]
    fn an_invalid_registration_voids_before_anything_else() {
        let days = window(14, 0.02, true);
        for mutate in [
            |p: &mut PreRegistration| p.test_share = 0.33,
            |p: &mut PreRegistration| p.window_days = 10,
            |p: &mut PreRegistration| p.rate_bp = 1000,
            |p: &mut PreRegistration| p.code_version_hash = " ".into(),
            |p: &mut PreRegistration| p.thresholds.harm_point = -0.10,
            |p: &mut PreRegistration| p.before_value = "  ".into(),
            |p: &mut PreRegistration| p.thresholds.split_chi_square = 12.0,
        ] {
            let mut bad = prereg();
            mutate(&mut bad);
            assert!(
                matches!(
                    decide(&bad, &input(&days, &[], true)),
                    Decision::Void { .. }
                ),
                "invalid registration must void: {bad:?}"
            );
        }
        // Tighter thresholds are allowed and keep working.
        let mut tighter = prereg();
        tighter.thresholds.harm_point = -0.02;
        tighter.thresholds.split_chi_square = 8.0;
        assert!(matches!(
            decide(&tighter, &input(&days, &[], true)),
            Decision::Keep { .. }
        ));
    }

    #[test]
    fn bootstrap_disagreement_is_gated_by_the_extension_flags() {
        let mut days = window(14, 0.0, true);
        days.days.clear();
        for i in 0..14 {
            let ratio: f64 = if i == 0 { 1.0 } else { 0.0 };
            days.days.push(AnalysisDay {
                day: format!("d{i}"),
                test_value: ratio.exp(),
                control_value: 1.0,
                log_ratio: ratio,
                fill_delta: None,
                calendar_index: i,
                test_users: None,
                control_users: None,
            });
        }
        let decision = decide(&prereg(), &input(&days, &[], true));
        assert!(matches!(decision, Decision::Extend { .. }), "{decision:?}");
        let mut used = input(&days, &[], true);
        used.extension_used = true;
        assert!(matches!(
            decide(&prereg(), &used),
            Decision::Rollback { .. }
        ));
        let mut forbidden = prereg();
        forbidden.extension_allowed = false;
        assert!(matches!(
            decide(&forbidden, &input(&days, &[], true)),
            Decision::Rollback { .. }
        ));
    }

    #[test]
    fn threshold_validation_bounds_the_fill_targets() {
        let loose_rewarded = GuardrailThresholds {
            fill_rewarded: 0.021,
            ..GuardrailThresholds::default()
        };
        let error = loose_rewarded.validate().expect_err("loose rewarded fill");
        assert!(error.contains("fill_rewarded"), "{error}");
        let loose_banner = GuardrailThresholds {
            fill_banner: 0.051,
            ..GuardrailThresholds::default()
        };
        let error = loose_banner.validate().expect_err("loose banner fill");
        assert!(error.contains("fill_banner"), "{error}");
        // The defaults and a fully tightened registration stay valid.
        assert!(GuardrailThresholds::default().validate().is_ok());
        let tightened = GuardrailThresholds {
            harm_point: 0.0,
            fill_rewarded: 0.0,
            fill_banner: 0.0,
            split_chi_square: f64::EPSILON,
        };
        assert!(tightened.validate().is_ok());
    }

    #[test]
    fn the_registered_minimum_point_lift_must_be_finite_and_in_range() {
        for invalid in [f64::NAN, -0.1, 2.0] {
            let mut registration = prereg();
            registration.min_point_lift = invalid;
            assert!(
                registration.validate().is_err(),
                "min_point_lift {invalid} must be refused"
            );
        }
        let mut registration = prereg();
        registration.min_point_lift = 0.0;
        registration.validate().expect("the endpoints are valid");
        registration.min_point_lift = 1.0;
        registration.validate().expect("the endpoints are valid");
    }

    #[test]
    fn from_record_rebuilds_non_string_before_values_verbatim() {
        let record = LiftRecord::PreRegistration {
            experiment_id: "exp-1".into(),
            instance_id: "exp-1@1".into(),
            scope: "u1".into(),
            before_value: serde_json::json!(42),
            after_value: serde_json::json!({"change": "raise t1 floor by 10%"}),
            split: 0.25,
            metric: "revenue_per_user".into(),
            burn_in_days: 3,
            window_days: 14,
            looks: 2,
            extension_allowed: true,
            guardrail_thresholds: serde_json::json!({
                "harmPoint": -0.05,
                "fillRewarded": 0.02,
                "fillBanner": 0.05,
                "splitChiSquare": 10.8276,
            }),
            min_point_lift: 0.02,
            code_version_hash: "abc123".into(),
            rate_bp: 2000,
            randomization_unit: crate::records::default_randomization_unit(),
            assignment_source: crate::records::default_assignment_source(),
            assignment_epoch: crate::records::default_assignment_epoch(),
            estimator_id: crate::records::default_estimator_id(),
            outcome_source: crate::records::default_outcome_source(),
            measurement_scope: "u1".into(),
            eligibility: crate::records::default_eligibility(),
            identity_contract: crate::records::default_identity_contract(),
        };
        let prereg = PreRegistration::from_record(&record).expect("record rebuilds");
        // A non-string prior value is rendered through its JSON form.
        assert_eq!(prereg.before_value, "42");
        assert_eq!(prereg.change_description, "raise t1 floor by 10%");
        assert!(prereg.interim_look, "two looks mean the interim look is on");
        assert_eq!(prereg.min_point_lift, 0.02, "the signed floor is rebuilt");
    }

    #[test]
    fn a_missing_min_point_lift_reads_the_legacy_default() {
        let record = LiftRecord::PreRegistration {
            experiment_id: "exp-1".into(),
            instance_id: "exp-1@1".into(),
            scope: "u1".into(),
            before_value: serde_json::json!(42),
            after_value: serde_json::json!({"change": "raise t1 floor by 10%"}),
            split: 0.25,
            metric: "revenue_per_user".into(),
            burn_in_days: 3,
            window_days: 14,
            looks: 2,
            extension_allowed: true,
            guardrail_thresholds: serde_json::json!({
                "harmPoint": -0.05,
                "fillRewarded": 0.02,
                "fillBanner": 0.05,
                "splitChiSquare": 10.8276,
            }),
            code_version_hash: "abc123".into(),
            rate_bp: 2000,
            min_point_lift: crate::records::default_min_point_lift(),
            randomization_unit: crate::records::default_randomization_unit(),
            assignment_source: crate::records::default_assignment_source(),
            assignment_epoch: crate::records::default_assignment_epoch(),
            estimator_id: crate::records::default_estimator_id(),
            outcome_source: crate::records::default_outcome_source(),
            measurement_scope: "u1".into(),
            eligibility: crate::records::default_eligibility(),
            identity_contract: crate::records::default_identity_contract(),
        };
        let json = serde_json::to_string(&record).expect("json");
        let without = json.replace(",\"minPointLift\":0.01", "");
        let parsed: LiftRecord = serde_json::from_str(&without).expect("legacy record");
        let prereg = PreRegistration::from_record(&parsed).expect("legacy rebuilds");
        assert_eq!(prereg.min_point_lift, 0.01);
    }

    #[test]
    fn the_registered_minimum_point_lift_drives_both_looks() {
        // A 1.5% point lift clears the legacy 1% floor but fails a 2% floor.
        let days = window(14, 0.015, true);
        let mut strict = prereg();
        strict.min_point_lift = 0.02;
        assert!(matches!(
            decide(&strict, &input(&days, &[], true)),
            Decision::Rollback { .. }
        ));
        assert!(matches!(
            decide(&prereg(), &input(&days, &[], true)),
            Decision::Keep { .. }
        ));
        // The interim look uses the same registered floor.
        let interim = window(7, 0.015, false);
        assert!(matches!(
            decide(&strict, &input(&interim, &[], false)),
            Decision::Pending { .. }
        ));
    }

    #[test]
    fn fewer_than_two_days_is_pending_not_a_verdict() {
        let one = window(1, 0.02, false);
        match decide(&prereg(), &input(&one, &[], false)) {
            Decision::Pending { reason } => {
                assert!(reason.contains("fewer than two analysis days"), "{reason}");
            }
            other => panic!("expected pending, got {other:?}"),
        }
    }

    #[test]
    fn an_unrunnable_bootstrap_is_pending_at_every_look() {
        // Fewer than 100 iterations cannot run: the interim and the final
        // look must wait instead of keeping on an unverified bound.
        let interim = window(7, 0.02, false);
        let mut interim_input = input(&interim, &[], false);
        interim_input.bootstrap_iterations = 99;
        match decide(&prereg(), &interim_input) {
            Decision::Pending { reason } => {
                assert!(
                    reason.contains("bootstrap cross-check could not run"),
                    "{reason}"
                );
            }
            other => panic!("expected pending, got {other:?}"),
        }
        let final_look = window(14, 0.02, true);
        let mut final_input = input(&final_look, &[], true);
        final_input.bootstrap_iterations = 99;
        match decide(&prereg(), &final_input) {
            Decision::Pending { reason } => {
                assert!(
                    reason.contains("bootstrap cross-check could not run"),
                    "{reason}"
                );
            }
            other => panic!("expected pending, got {other:?}"),
        }
    }

    #[test]
    fn a_bootstrap_disagreement_offers_the_pre_registered_extension() {
        // Alternating +5%/-1% days: the analytic lower bound is just above
        // zero, while whole-window blocks make the bootstrap replay the point
        // estimate about 1.2 points higher. The cross-check disagrees and the
        // one pre-registered extra week is offered at the final look.
        let mut days = window(14, 0.0, true);
        days.days.clear();
        for i in 0..14 {
            let ratio: f64 = if i % 2 == 0 { 0.05 } else { -0.01 };
            days.days.push(AnalysisDay {
                day: format!("d{i}"),
                test_value: ratio.exp(),
                control_value: 1.0,
                log_ratio: ratio,
                fill_delta: None,
                calendar_index: i,
                test_users: None,
                control_users: None,
            });
        }
        let mut final_input = input(&days, &[], true);
        final_input.bootstrap_block_len = days.days.len();
        match decide(&prereg(), &final_input) {
            Decision::Extend { reason } => {
                assert!(reason.contains("bootstrap"), "{reason}");
            }
            other => panic!("expected extend, got {other:?}"),
        }
    }
}
