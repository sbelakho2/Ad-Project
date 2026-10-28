//! The randomized switchback fallback: alternate the old and new setting in
//! randomized two-day blocks over four weeks, balanced so each weekday sees
//! both arms. Blocks, not days, are the unit, and billing needs at least six
//! blocks per arm with a positive 90% lower bound.
//!
//! The plan is not decoration: [`decide`] reads [`SwitchbackPlan::blocks`] to
//! split the observed block series into the test and control arms. A block
//! carries one arm's outcome, never a synthetic (test, control) pair, so the
//! analysis cannot silently compare two readings that never coexisted.

use crate::stats::t_quantile;
use crate::stats::Estimate;

/// The six hours after a switch are dropped to let bidders settle; a sample
/// counts only once it is past this horizon.
pub const SETTLE_HOURS: f64 = 6.0;

/// Whether time since the switch is past the settle horizon.
pub fn settled(hours_since_switch: f64) -> bool {
    hours_since_switch.is_finite() && hours_since_switch >= SETTLE_HOURS
}

/// A planned switchback.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchbackPlan {
    /// One entry per two-day block: `true` means the test setting runs.
    pub blocks: Vec<bool>,
    /// How many blocks per arm (equal by construction).
    pub blocks_per_arm: usize,
}

/// Plan the four-week switchback: 14 two-day blocks (28 days) balanced so
/// every weekday sees both arms. With 28 days each weekday occurs exactly
/// four times; the test arm takes each weekday exactly twice. Plans are
/// enumerated, filtered to never run three blocks on one arm, and chosen
/// deterministically from the seed. Any other block count is refused: only
/// four whole weeks can be weekday-symmetric.
pub fn plan(block_count: usize, seed: u64) -> Option<SwitchbackPlan> {
    if block_count != 14 {
        return None;
    }
    let mut feasible: Vec<Vec<bool>> = Vec::new();
    for mask in 0u32..(1 << block_count) {
        if mask.count_ones() != (block_count / 2) as u32 {
            continue;
        }
        let blocks: Vec<bool> = (0..block_count).map(|k| mask & (1 << k) != 0).collect();
        if has_triple(&blocks) {
            continue;
        }
        // Weekday balance: each weekday appears in exactly two test blocks
        // (and therefore two control blocks).
        let mut per_weekday = [0u32; 7];
        for (k, is_test) in blocks.iter().enumerate() {
            if !is_test {
                continue;
            }
            per_weekday[(2 * k) % 7] += 1;
            per_weekday[(2 * k + 1) % 7] += 1;
        }
        if per_weekday.iter().all(|count| *count == 2) {
            feasible.push(blocks);
        }
    }
    if feasible.is_empty() {
        return None;
    }
    let chosen = feasible[(seed % feasible.len() as u64) as usize].clone();
    Some(SwitchbackPlan {
        blocks: chosen,
        blocks_per_arm: block_count / 2,
    })
}

fn has_triple(blocks: &[bool]) -> bool {
    blocks.windows(3).any(|w| w[0] == w[1] && w[1] == w[2])
}

/// Evaluate a finished switchback. `block_values` holds one observed value
/// per planned block, in plan order. The first six hours after each switch
/// must already be dropped by the caller: a block only contributes after it
/// has [`settled`]. Billing is allowed only with six or more blocks per arm
/// and a positive 90% lower bound.
#[derive(Debug, Clone, PartialEq)]
pub enum SwitchbackDecision {
    Billable { lambda: f64, estimate: Estimate },
    NotBillable { reason: String },
}

pub fn decide(
    plan: &SwitchbackPlan,
    block_values: &[f64],
    used_blocks_per_arm: usize,
) -> SwitchbackDecision {
    if used_blocks_per_arm < 6 {
        return SwitchbackDecision::NotBillable {
            reason: format!("{used_blocks_per_arm} blocks per arm: fewer than the required six"),
        };
    }
    if plan.blocks_per_arm < 6 {
        return SwitchbackDecision::NotBillable {
            reason: "the plan itself has fewer than six blocks per arm".into(),
        };
    }
    let planned_test = plan.blocks.iter().filter(|is_test| **is_test).count();
    if planned_test != plan.blocks_per_arm
        || plan.blocks.len() - planned_test != plan.blocks_per_arm
    {
        return SwitchbackDecision::NotBillable {
            reason: "the plan's block labels disagree with blocks_per_arm".into(),
        };
    }
    // Every planned block must carry its value, all of them usable: a
    // dropped or non-positive block is missing data, not a smaller test.
    if block_values.len() != plan.blocks.len() || used_blocks_per_arm != plan.blocks_per_arm {
        return SwitchbackDecision::NotBillable {
            reason: format!(
                "{} block values for {} planned blocks ({used_blocks_per_arm} of {} blocks per arm)",
                block_values.len(),
                plan.blocks.len(),
                plan.blocks_per_arm
            ),
        };
    }
    if block_values
        .iter()
        .any(|value| !(value.is_finite() && *value > 0.0))
    {
        return SwitchbackDecision::NotBillable {
            reason: "a block value is non-positive or non-finite".into(),
        };
    }
    let mut test_logs = Vec::new();
    let mut control_logs = Vec::new();
    for (is_test, value) in plan.blocks.iter().zip(block_values) {
        if *is_test {
            test_logs.push(value.ln());
        } else {
            control_logs.push(value.ln());
        }
    }
    let Some(estimate) = difference_estimate(&test_logs, &control_logs) else {
        return SwitchbackDecision::NotBillable {
            reason: "fewer than two usable blocks per arm".into(),
        };
    };
    if estimate.lower_alpha > 0.0 {
        return SwitchbackDecision::Billable {
            lambda: estimate.lower_alpha,
            estimate,
        };
    }
    SwitchbackDecision::NotBillable {
        reason: "the registered 97.5% lower bound is not above zero".into(),
    }
}

/// Difference of arm mean log values with a Welch-Satterthwaite t bound.
/// Blocks are the independent unit, so the variance is the sum of the two
/// arm mean variances divided by their block counts.
fn difference_estimate(test_logs: &[f64], control_logs: &[f64]) -> Option<Estimate> {
    if test_logs.len().min(control_logs.len()) < 2 {
        return None;
    }
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len() as f64;
    let variance = |values: &[f64], mean: f64| {
        values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (values.len() as f64 - 1.0)
    };
    let (test_mean, control_mean) = (mean(test_logs), mean(control_logs));
    let (test_variance, control_variance) = (
        variance(test_logs, test_mean),
        variance(control_logs, control_mean),
    );
    let test_term = test_variance / test_logs.len() as f64;
    let control_term = control_variance / control_logs.len() as f64;
    let se = (test_term + control_term).sqrt();
    if !se.is_finite() {
        return None;
    }
    let df = if test_term + control_term > 0.0 {
        ((test_term + control_term).powi(2)
            / (test_term.powi(2) / (test_logs.len() as f64 - 1.0)
                + control_term.powi(2) / (control_logs.len() as f64 - 1.0)))
            .floor() as usize
    } else {
        test_logs.len().min(control_logs.len()) - 1
    };
    let df = df.max(1);
    let mean_log = test_mean - control_mean;
    let t90 = t_quantile(df, false);
    let t99 = t_quantile(df, true);
    let lift = mean_log.exp() - 1.0;
    let lower_90 = (mean_log - t90 * se).exp() - 1.0;
    let lower_99 = (mean_log - t99 * se).exp() - 1.0;
    let upper_90 = (mean_log + t90 * se).exp() - 1.0;
    let upper_99 = (mean_log + t99 * se).exp() - 1.0;
    let lower_alpha =
        (mean_log - crate::stats::t_quantile_alpha(df, crate::stats::INITIAL_ALPHA) * se).exp()
            - 1.0;
    let lower_interim =
        (mean_log - crate::stats::t_quantile_alpha(df, crate::stats::INTERIM_ALPHA) * se).exp()
            - 1.0;
    if ![
        lift,
        lower_90,
        lower_99,
        upper_90,
        upper_99,
        lower_alpha,
        lower_interim,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        return None;
    }
    Some(Estimate {
        days: test_logs.len() + control_logs.len(),
        mean_log,
        lift,
        sd: se,
        lower_90,
        lower_99,
        upper_90,
        upper_99,
        lower_alpha,
        lower_interim,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_settle_horizon_drops_the_first_six_hours() {
        assert!(!settled(0.0));
        assert!(!settled(5.99));
        assert!(settled(6.0));
        assert!(settled(48.0));
        assert!(!settled(f64::NAN));
    }

    #[test]
    fn plans_are_balanced_per_weekday_and_never_triple() {
        for seed in 0..25u64 {
            let planned = plan(14, seed).expect("plan");
            assert_eq!(planned.blocks_per_arm, 7);
            assert!(!has_triple(&planned.blocks), "seed {seed}");
            assert_eq!(planned.blocks.iter().filter(|b| **b).count(), 7);
            let mut test_days_per_weekday = [0u32; 7];
            let mut control_days_per_weekday = [0u32; 7];
            for (k, is_test) in planned.blocks.iter().enumerate() {
                let bucket = if *is_test {
                    &mut test_days_per_weekday
                } else {
                    &mut control_days_per_weekday
                };
                bucket[(2 * k) % 7] += 1;
                bucket[(2 * k + 1) % 7] += 1;
            }
            for weekday in 0..7 {
                assert_eq!(
                    test_days_per_weekday[weekday], 2,
                    "seed {seed} weekday {weekday}"
                );
                assert_eq!(
                    control_days_per_weekday[weekday], 2,
                    "seed {seed} weekday {weekday}"
                );
            }
        }
        assert_eq!(plan(14, 3), plan(14, 3), "deterministic per seed");
        assert!(plan(12, 7).is_none(), "only four whole weeks are symmetric");
        assert!(plan(11, 7).is_none());
        assert!(plan(10, 7).is_none());
    }

    fn values_for(plan: &SwitchbackPlan, test_value: f64, control_value: f64) -> Vec<f64> {
        plan.blocks
            .iter()
            .map(|is_test| if *is_test { test_value } else { control_value })
            .collect()
    }

    #[test]
    fn analysis_uses_the_plan_blocks_to_split_the_observed_series() {
        let planned = plan(14, 7).expect("plan");
        let strong = values_for(&planned, 1.04, 1.0);
        // The same series reads as a loss when the plan labels flip.
        let flipped = SwitchbackPlan {
            blocks: planned.blocks.iter().map(|is_test| !is_test).collect(),
            blocks_per_arm: planned.blocks_per_arm,
        };
        match decide(&planned, &strong, 7) {
            SwitchbackDecision::Billable {
                lambda, estimate, ..
            } => {
                assert!(lambda > 0.0);
                assert!((estimate.lift - 0.04).abs() < 1e-12, "{estimate:?}");
            }
            other => panic!("expected billable, got {other:?}"),
        }
        match decide(&flipped, &strong, 7) {
            SwitchbackDecision::NotBillable { .. } => {}
            other => panic!("the flipped plan must not bill, got {other:?}"),
        }
        // With real block-to-block spread the billed bound is the 90% bound,
        // strictly below the 99% bound (a 99% mutation must not survive).
        let mut spread = values_for(&planned, 1.04, 1.0);
        let test_indexes: Vec<usize> = planned
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, is_test)| is_test.then_some(index))
            .collect();
        for (position, index) in test_indexes.iter().enumerate() {
            spread[*index] = if position % 2 == 0 { 1.06 } else { 1.04 };
        }
        match decide(&planned, &spread, 7) {
            SwitchbackDecision::Billable { lambda, estimate } => {
                assert!(
                    estimate.lower_alpha > estimate.lower_interim,
                    "{estimate:?}"
                );
                assert!((lambda - estimate.lower_alpha).abs() < 1e-12);
            }
            other => panic!("expected a billable spread, got {other:?}"),
        }
    }

    #[test]
    fn the_admissible_assignment_law_has_exactly_thirty_schedules() {
        use std::collections::BTreeSet;
        let mut schedules: BTreeSet<Vec<bool>> = BTreeSet::new();
        for seed in 0..4001u64 {
            if let Some(plan) = plan(14, seed) {
                schedules.insert(plan.blocks);
            }
        }
        assert_eq!(
            schedules.len(),
            30,
            "the finite sharp-null law is 30 admissible 14-block schedules"
        );
        // A one-sided exact randomization p-value cannot go below 1/30, so a
        // 2.5% exact gate can never reject with this design.
        let minimum_p = 1.0 / schedules.len() as f64;
        assert!(minimum_p > 0.025, "{minimum_p}");
    }

    #[test]
    fn billing_needs_six_blocks_and_a_positive_bound() {
        let planned = plan(14, 7).expect("plan");
        // Partial coverage is never billable, whatever the used count says.
        assert!(matches!(
            decide(&planned, &[1.05, 1.05], 7),
            SwitchbackDecision::NotBillable { .. }
        ));
        let mut dropped = values_for(&planned, 1.04, 1.0);
        dropped[3] = 0.0;
        assert!(matches!(
            decide(&planned, &dropped, 7),
            SwitchbackDecision::NotBillable { .. }
        ));
        assert!(matches!(
            decide(&planned, &values_for(&planned, 1.04, 1.0), 5),
            SwitchbackDecision::NotBillable { .. }
        ));
        // A noisy arm pair whose mean gap straddles zero bounds below zero.
        let mut noisy = values_for(&planned, 1.04, 1.0);
        let test_indexes: Vec<usize> = planned
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, is_test)| is_test.then_some(index))
            .collect();
        for (position, index) in test_indexes.iter().enumerate() {
            noisy[*index] = if position % 2 == 0 { 1.04 } else { 0.97 };
        }
        assert!(matches!(
            decide(&planned, &noisy, 7),
            SwitchbackDecision::NotBillable { .. }
        ));
    }

    #[test]
    fn a_non_positive_block_or_short_arm_is_not_billable() {
        let planned = plan(14, 7).expect("plan");
        let mut non_positive = values_for(&planned, 1.04, 1.0);
        non_positive[0] = f64::NAN;
        match decide(&planned, &non_positive, 7) {
            SwitchbackDecision::NotBillable { reason } => {
                assert!(reason.contains("non-positive or non-finite"), "{reason}");
            }
            other => panic!("expected not billable, got {other:?}"),
        }
        // A negative value must be refused by the value guard itself.
        let mut negative = values_for(&planned, 1.04, 1.0);
        negative[0] = -1.0;
        match decide(&planned, &negative, 7) {
            SwitchbackDecision::NotBillable { reason } => {
                assert!(reason.contains("non-positive or non-finite"), "{reason}");
            }
            other => panic!("expected not billable, got {other:?}"),
        }
        let tiny = SwitchbackPlan {
            blocks: vec![true, false],
            blocks_per_arm: 1,
        };
        assert!(matches!(
            decide(&tiny, &[1.04, 1.0], 1),
            SwitchbackDecision::NotBillable { .. }
        ));
    }

    #[test]
    fn difference_estimate_needs_two_blocks_per_arm() {
        assert!(difference_estimate(&[0.1, 0.2], &[0.0, 0.1]).is_some());
        assert!(difference_estimate(&[0.1], &[0.0, 0.1]).is_none());
        assert!(difference_estimate(&[0.1, 0.2], &[0.0]).is_none());
    }

    #[test]
    fn an_inconsistent_plan_is_never_billable() {
        let planned = SwitchbackPlan {
            blocks: vec![
                true, false, true, false, true, false, true, false, true, false, true, false,
            ],
            blocks_per_arm: 6,
        };
        // Twelve labels, six test, but the plan claims six per arm (twelve
        // total): the labels and the declared arm counts must agree.
        assert!(matches!(
            decide(&planned, &values_for(&planned, 1.04, 1.0), 6),
            SwitchbackDecision::Billable { .. }
        ));
        let inconsistent = SwitchbackPlan {
            blocks: vec![
                true, false, true, false, true, false, true, false, true, false, true, false, true,
            ],
            blocks_per_arm: 6,
        };
        match decide(&inconsistent, &values_for(&inconsistent, 1.04, 1.0), 6) {
            SwitchbackDecision::NotBillable { reason } => {
                assert!(reason.contains("labels disagree"), "{reason}");
            }
            other => panic!("expected not billable, got {other:?}"),
        }
    }

    #[test]
    fn a_plan_with_too_few_blocks_per_arm_is_never_billable() {
        let plan = SwitchbackPlan {
            blocks: vec![
                true, false, true, false, true, false, true, false, true, false,
            ],
            blocks_per_arm: 5,
        };
        match decide(&plan, &[], 6) {
            SwitchbackDecision::NotBillable { reason } => {
                assert!(reason.contains("fewer than six blocks per arm"), "{reason}");
            }
            other => panic!("expected not billable, got {other:?}"),
        }
    }
}
