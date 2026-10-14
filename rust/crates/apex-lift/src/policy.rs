//! Planning policy and account-level caps. These refuse tests that cannot
//! conclude or that the contract forbids; the engine never starts what it
//! cannot bill or must roll back unfairly.

use crate::candidates::candidate_split;
use crate::candidates::FloorCandidate;
use crate::error::{LiftError, LiftResult};
use crate::stats::Plan;

/// At most three new changes a day.
pub const MAX_CHANGES_PER_DAY: u32 = 3;
/// At most a 20% step in any floor or price per test.
pub const MAX_STEP_PCT: f64 = 20.0;

/// Whether a planned test may start, with the planned delta and the fixed
/// 3% eligibility rule already applied by [`Plan`].
pub fn can_start(
    plan: &Plan,
    candidate: &FloorCandidate,
    active_test_on_unit: bool,
) -> LiftResult<()> {
    if active_test_on_unit {
        return Err(LiftError::Malformed(
            "one test per ad unit: an experiment is already active".into(),
        ));
    }
    if plan.analysis_days > crate::stats::MAX_ANALYSIS_DAYS {
        return Err(LiftError::Malformed(format!(
            "the planned {} analysis days exceed the 28-day rule",
            plan.analysis_days
        )));
    }
    if !plan.eligible {
        return Err(LiftError::Malformed(
            "the unit cannot detect the fixed 3% target within 28 analysis days".into(),
        ));
    }
    validate_step_pct(candidate.move_pct)?;
    let Some(split) = candidate_split(candidate) else {
        return Err(LiftError::Malformed(
            "the candidate's worst-case loss exceeds 5%".into(),
        ));
    };
    if split == 0.5 {
        let loss = candidate.worst_case_loss.unwrap_or(1.0);
        if loss >= 0.01 {
            return Err(LiftError::Malformed(
                "50/50 is allowed only under 1% worst-case loss".into(),
            ));
        }
    }
    if split == 0.10 {
        return Err(LiftError::Malformed(
            "10% splits are exploratory only and never billed".into(),
        ));
    }
    Ok(())
}

/// Validate a planned test share for a billing test: 25% is the default, 50%
/// needs the cheap worst case, 10% is refused.
pub fn billing_split(candidate: &FloorCandidate) -> LiftResult<f64> {
    match candidate_split(candidate) {
        Some(0.5) => Ok(0.5),
        Some(0.25) => Ok(0.25),
        Some(_) => Err(LiftError::Malformed(
            "10% splits are exploratory only and never billed".into(),
        )),
        None => Err(LiftError::Malformed(
            "the candidate's worst-case loss exceeds 5%".into(),
        )),
    }
}

/// The daily write cap.
pub fn validate_daily_count(changes_today: u32) -> LiftResult<()> {
    if changes_today >= MAX_CHANGES_PER_DAY {
        return Err(LiftError::Malformed(format!(
            "the daily cap of {MAX_CHANGES_PER_DAY} new changes is reached"
        )));
    }
    Ok(())
}

/// The per-test step cap.
pub fn validate_step_pct(step_pct: f64) -> LiftResult<()> {
    if !step_pct.is_finite() || step_pct.abs() > MAX_STEP_PCT {
        return Err(LiftError::Malformed(format!(
            "a {step_pct}% step exceeds the {MAX_STEP_PCT}% cap"
        )));
    }
    Ok(())
}

/// No writes while a critical issue from the audit is open.
pub fn validate_no_open_critical(open_critical_findings: u32) -> LiftResult<()> {
    if open_critical_findings > 0 {
        return Err(LiftError::Malformed(format!(
            "{open_critical_findings} critical issues are open; writes are frozen"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::candidates::FloorCandidate;

    fn plan(days: usize, eligible: bool) -> Plan {
        Plan {
            sigma_z: 0.03,
            raw_days: days as f64,
            analysis_days: days,
            planned_length_days: days + 3,
            eligible,
        }
    }

    fn candidate(loss: Option<f64>) -> FloorCandidate {
        FloorCandidate {
            group: "t1".into(),
            current_ecpm: 1_000_000,
            proposed_ecpm: 1_100_000,
            move_pct: 10.0,
            worst_case_loss: loss,
        }
    }

    #[test]
    fn one_test_per_unit_and_length_rules() {
        assert!(can_start(&plan(14, true), &candidate(Some(0.03)), true).is_err());
        assert!(can_start(&plan(35, true), &candidate(Some(0.03)), false).is_err());
        assert!(can_start(&plan(14, false), &candidate(Some(0.03)), false).is_err());
        assert!(can_start(&plan(14, true), &candidate(Some(0.03)), false).is_ok());
    }

    #[test]
    fn oversized_steps_are_refused() {
        let mut huge = candidate(Some(0.03));
        huge.move_pct = 45.0;
        assert!(can_start(&plan(14, true), &huge, false).is_err());
        assert!(can_start(&plan(14, true), &candidate(Some(0.03)), false).is_ok());
    }

    #[test]
    fn risky_candidates_are_refused_and_ten_percent_never_bills() {
        assert!(can_start(&plan(14, true), &candidate(Some(0.06)), false).is_err());
        assert!(billing_split(&candidate(Some(0.06))).is_err());
        let cheap = candidate(Some(0.005));
        assert_eq!(billing_split(&cheap).unwrap(), 0.5);
        let mid = candidate(Some(0.03));
        assert_eq!(billing_split(&mid).unwrap(), 0.25);
        let unknown = candidate(None);
        assert_eq!(billing_split(&unknown).unwrap(), 0.25);
    }

    #[test]
    fn cheap_candidates_may_use_fifty_fifty_and_risky_ones_refuse() {
        let cheap = candidate(Some(0.005));
        assert!(can_start(&plan(14, true), &cheap, false).is_ok());
        assert!(billing_split(&cheap).is_ok());
        assert!(can_start(&plan(14, true), &candidate(Some(0.06)), false).is_err());
        assert!(billing_split(&candidate(Some(0.06))).is_err());
        assert!(billing_split(&candidate(None)).is_ok());
    }

    #[test]
    fn account_caps_are_pinned() {
        assert!(validate_daily_count(2).is_ok());
        assert!(validate_daily_count(3).is_err());
        assert!(validate_step_pct(20.0).is_ok());
        assert!(validate_step_pct(20.1).is_err());
        assert!(validate_step_pct(f64::NAN).is_err());
        assert!(validate_no_open_critical(0).is_ok());
        assert!(validate_no_open_critical(1).is_err());
    }
}
