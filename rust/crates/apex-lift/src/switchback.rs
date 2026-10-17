//! The randomized switchback fallback: alternate the old and new setting in
//! randomized two-day blocks over four weeks, balanced so each weekday sees
//! both arms. Blocks, not days, are the unit, and billing needs at least six
//! blocks per arm with a positive 90% lower bound.

use crate::stats::{estimate, Estimate};

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

/// Evaluate a finished switchback. `block_values` holds `(test_value,
/// control_value)` per block; the first six hours after each switch are
/// already dropped when the caller builds the block values. Billing is
/// allowed only with six or more blocks per arm and a positive 90% bound.
#[derive(Debug, Clone, PartialEq)]
pub enum SwitchbackDecision {
    Billable { lambda: f64, estimate: Estimate },
    NotBillable { reason: String },
}

pub fn decide(
    plan: &SwitchbackPlan,
    block_values: &[(f64, f64)],
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
        .any(|(t, c)| !t.is_finite() || !c.is_finite() || *t <= 0.0 || *c <= 0.0)
    {
        return SwitchbackDecision::NotBillable {
            reason: "a block value is non-positive or non-finite".into(),
        };
    }
    let ratios: Vec<f64> = block_values.iter().map(|(t, c)| (t / c).ln()).collect();
    let Some(estimate) = estimate(&ratios) else {
        return SwitchbackDecision::NotBillable {
            reason: "fewer than two usable blocks".into(),
        };
    };
    if estimate.lower_90 <= 0.0 {
        return SwitchbackDecision::NotBillable {
            reason: "the 90% lower bound is not above zero".into(),
        };
    }
    SwitchbackDecision::Billable {
        lambda: estimate.lower_90,
        estimate,
    }
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

    #[test]
    fn billing_needs_six_blocks_and_a_positive_bound() {
        let planned = plan(14, 7).expect("plan");
        let strong: Vec<(f64, f64)> = (0..14).map(|i| (1.04 + i as f64 * 0.0001, 1.0)).collect();
        // Partial coverage is never billable, whatever the used count says.
        assert!(matches!(
            decide(&planned, &[(1.05, 1.0), (1.05, 1.0)], 7),
            SwitchbackDecision::NotBillable { .. }
        ));
        let mut dropped = strong.clone();
        dropped[3] = (0.0, 1.0);
        assert!(matches!(
            decide(&planned, &dropped, 7),
            SwitchbackDecision::NotBillable { .. }
        ));
        match decide(&planned, &strong, 7) {
            SwitchbackDecision::Billable { lambda, .. } => assert!(lambda > 0.0),
            other => panic!("expected billable, got {other:?}"),
        }
        assert!(matches!(
            decide(&planned, &strong, 5),
            SwitchbackDecision::NotBillable { .. }
        ));
        // A genuinely lossy alternation (+1% and -2%) bounds below zero.
        let weak: Vec<(f64, f64)> = (0..14)
            .map(|i| if i % 2 == 0 { (1.01, 1.0) } else { (0.98, 1.0) })
            .collect();
        assert!(matches!(
            decide(&planned, &weak, 7),
            SwitchbackDecision::NotBillable { .. }
        ));
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
