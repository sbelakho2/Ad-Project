//! Calibration: the first test on each ad unit runs identical settings in
//! both arms for seven days at the planned split. It measures the unit's real
//! daily log-ratio spread and checks for bias; a biased or too-noisy unit is
//! audit-only.

use crate::stats::{estimate, sigma_z};
use crate::window::{build_window, ArmDay, WindowProblem};

/// What the A/A run decided for the unit.
#[derive(Debug, Clone, PartialEq)]
pub enum AaConclusion {
    /// The unit is usable; `sigma_50` is the daily spread normalized to a
    /// 50/50 split, ready for the duration planner.
    Calibrated { sigma_50: f64, sigma_at_split: f64 },
    /// The unit may only be audited, never billed.
    AuditOnly { reason: String },
}

/// Seven analysis days is the spec's A/A length.
pub const AA_DAYS: usize = 7;
/// The A/A run uses the same calendar-position burn-in as production
/// measurement.
pub const AA_BURN_IN_DAYS: usize = 3;
/// A mean log ratio beyond this absolute value is bias, not noise.
const BIAS_SE_MULTIPLE: f64 = 2.0;

/// Evaluate an A/A run through the same day alignment, finality and clean
/// window path as production measurement: burn-in by calendar position, both
/// pulls at least 24 hours apart, incident exclusion and whole-week
/// analysis. A window that is not complete (or carries a fatal sequence or
/// schema problem) is audit-only; it never calibrates a unit from incomplete
/// evidence.
pub fn evaluate_aa_days(
    control: &[ArmDay],
    test: &[ArmDay],
    planned_split: f64,
    burn_in_days: usize,
) -> AaConclusion {
    let window = build_window(control, test, AA_DAYS, planned_split, burn_in_days);
    if window.fatal {
        let detail = window
            .problems
            .iter()
            .find(|problem| problem.is_fatal())
            .map(|problem| format!("{problem:?}"))
            .unwrap_or_else(|| "fatal window problem".to_string());
        return AaConclusion::AuditOnly {
            reason: format!("the A/A window is unusable: {detail}"),
        };
    }
    if !window.complete {
        let reasons: Vec<&WindowProblem> = window
            .problems
            .iter()
            .filter(|problem| !problem.is_informational())
            .collect();
        return AaConclusion::AuditOnly {
            reason: format!(
                "the A/A window is not complete: {} accepted day(s), problems {reasons:?}",
                window.days.len()
            ),
        };
    }
    let ratios: Vec<f64> = window.days.iter().map(|day| day.log_ratio).collect();
    evaluate_aa(&ratios, planned_split)
}

/// Evaluate the A/A run's daily log ratios at the planned split.
pub fn evaluate_aa(ratios: &[f64], planned_split: f64) -> AaConclusion {
    if ratios.len() != AA_DAYS {
        return AaConclusion::AuditOnly {
            reason: format!(
                "the A/A run has {} days; exactly {AA_DAYS} are required",
                ratios.len()
            ),
        };
    }
    let Some(estimate) = estimate(ratios) else {
        return AaConclusion::AuditOnly {
            reason: "the A/A run has no usable spread".into(),
        };
    };
    let se = estimate.sd / (estimate.days as f64).sqrt();
    if estimate.mean_log.abs() > BIAS_SE_MULTIPLE * se {
        return AaConclusion::AuditOnly {
            reason: format!(
                "A/A bias: mean log ratio {:.4} is more than two standard errors from zero",
                estimate.mean_log
            ),
        };
    }
    // Normalize the split's spread to the 50/50 spread the planner expects.
    let factor = match sigma_z(1.0, planned_split) {
        Some(factor) => factor,
        None => {
            return AaConclusion::AuditOnly {
                reason: format!("planned split {planned_split} is invalid"),
            }
        }
    };
    let sigma_at_split = estimate.sd;
    AaConclusion::Calibrated {
        sigma_50: sigma_at_split / factor,
        sigma_at_split,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn a_clean_aa_run_calibrates_the_unit() {
        let ratios = [0.01, -0.005, 0.002, 0.004, -0.002, 0.001, -0.003];
        match evaluate_aa(&ratios, 0.5) {
            AaConclusion::Calibrated { sigma_50, .. } => {
                assert!(sigma_50 > 0.0 && sigma_50 < 0.02, "{sigma_50}");
            }
            other => panic!("expected calibrated, got {other:?}"),
        }
        // At a 25% split the measured spread is bigger than the 50/50 spread
        // it normalizes down to.
        match evaluate_aa(&ratios, 0.25) {
            AaConclusion::Calibrated {
                sigma_50,
                sigma_at_split,
            } => {
                assert!(sigma_50 < sigma_at_split, "{sigma_50} vs {sigma_at_split}");
            }
            other => panic!("expected calibrated, got {other:?}"),
        }
        // At a 50/50 split the two figures are identical.
        match evaluate_aa(&ratios, 0.5) {
            AaConclusion::Calibrated {
                sigma_50,
                sigma_at_split,
            } => assert!((sigma_50 - sigma_at_split).abs() < 1e-9),
            other => panic!("expected calibrated, got {other:?}"),
        }
    }

    #[test]
    fn a_biased_aa_run_is_audit_only() {
        let biased = [0.03, 0.02, 0.04, 0.03, 0.02, 0.03, 0.03];
        match evaluate_aa(&biased, 0.5) {
            AaConclusion::AuditOnly { reason } => assert!(reason.contains("bias"), "{reason}"),
            other => panic!("expected audit-only, got {other:?}"),
        }
        // A small but statistically clear mean is also bias.
        let subtle = [0.005, 0.005, 0.005, 0.005, 0.005, 0.005, 0.005];
        match evaluate_aa(&subtle, 0.5) {
            AaConclusion::AuditOnly { reason } => assert!(reason.contains("bias"), "{reason}"),
            other => panic!("expected audit-only, got {other:?}"),
        }
    }

    #[test]
    fn invalid_splits_and_unusable_runs_are_audit_only() {
        let ratios = [0.01, -0.005, 0.002, 0.004, -0.002, 0.001, -0.003];
        assert!(matches!(
            evaluate_aa(&ratios, 0.0),
            AaConclusion::AuditOnly { .. }
        ));
        assert!(matches!(
            evaluate_aa(&ratios, 1.0),
            AaConclusion::AuditOnly { .. }
        ));
        let mut broken = ratios;
        broken[3] = f64::NAN;
        assert!(matches!(
            evaluate_aa(&broken, 0.5),
            AaConclusion::AuditOnly { .. }
        ));
    }

    #[test]
    fn wrong_length_runs_are_audit_only() {
        assert!(matches!(
            evaluate_aa(&[0.0; 6], 0.5),
            AaConclusion::AuditOnly { .. }
        ));
        assert!(matches!(
            evaluate_aa(&[0.0; 8], 0.5),
            AaConclusion::AuditOnly { .. }
        ));
    }

    fn arm_day(day: &str, revenue: i64, users: u64) -> ArmDay {
        ArmDay {
            day: day.to_string(),
            revenue,
            users: Some(users),
            second_pull_revenue: Some(revenue),
            second_pull_users: Some(users),
            fill: None,
            second_pull_fill: None,
            first_observed_at_unix_s: Some(1_700_000_000),
            second_observed_at_unix_s: Some(1_700_086_400),
            first_source_sha256: Some("a".repeat(64)),
            second_source_sha256: Some("b".repeat(64)),
        }
    }

    fn ten_days(ratio: f64) -> (Vec<ArmDay>, Vec<ArmDay>) {
        let control: Vec<ArmDay> = (1..=10)
            .map(|i| arm_day(&format!("2026-10-{i:02}"), 1_000_000, 1_000))
            .collect();
        let test: Vec<ArmDay> = (1..=10)
            .map(|i| {
                let signed = if i % 2 == 0 { ratio } else { -ratio };
                arm_day(
                    &format!("2026-10-{i:02}"),
                    (1_000_000.0 * signed.exp()).round() as i64,
                    1_000,
                )
            })
            .collect();
        (control, test)
    }

    #[test]
    fn aa_calibration_uses_the_production_finality_window() {
        let (control, test) = ten_days(0.01);
        match evaluate_aa_days(&control, &test, 0.5, AA_BURN_IN_DAYS) {
            AaConclusion::Calibrated { .. } => {}
            other => panic!("expected calibrated, got {other:?}"),
        }
        // A day without an accepted second pull is not final, so the A/A run
        // is not calibratable yet even with ten days of first observations.
        let mut pending = control.clone();
        let mut pending_test = test.clone();
        pending[5].second_pull_revenue = None;
        pending[5].second_observed_at_unix_s = None;
        pending[5].second_source_sha256 = None;
        pending_test[5].second_pull_revenue = None;
        pending_test[5].second_observed_at_unix_s = None;
        pending_test[5].second_source_sha256 = None;
        match evaluate_aa_days(&pending, &pending_test, 0.5, AA_BURN_IN_DAYS) {
            AaConclusion::AuditOnly { reason } => {
                assert!(reason.contains("not complete"), "{reason}");
            }
            other => panic!("expected audit-only, got {other:?}"),
        }
    }

    #[test]
    fn aa_calibration_does_not_synthesize_a_second_pull_from_one_read() {
        let (mut control, mut test) = ten_days(0.01);
        for day in control.iter_mut().chain(test.iter_mut()) {
            day.second_pull_revenue = None;
            day.second_pull_users = None;
            day.second_observed_at_unix_s = None;
            day.second_source_sha256 = None;
        }
        match evaluate_aa_days(&control, &test, 0.5, AA_BURN_IN_DAYS) {
            AaConclusion::AuditOnly { reason } => {
                assert!(reason.contains("not complete"), "{reason}");
            }
            other => panic!("expected audit-only, got {other:?}"),
        }
    }
}
