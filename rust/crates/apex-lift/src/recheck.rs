//! Month-6 re-checks: at month six, or earlier when the scope's eCPM has
//! moved more than 30% from the test period, run a reversal test that can
//! only lower the billed lift.

/// The eCPM move that triggers an early re-check.
pub const ECPM_MOVE_TRIGGER: f64 = 0.30;
/// The scheduled re-check month.
pub const RECHECK_MONTH: u32 = 6;
/// The reversal test uses a 25% share of users.
pub const REVERSAL_SPLIT: f64 = 0.25;
/// The reversal test runs 14 analysis days.
pub const REVERSAL_ANALYSIS_DAYS: usize = 14;

/// Whether a re-check is due: the scheduled month-6 look, or earlier when
/// the scope's eCPM moved more than 30% from the test period.
pub fn recheck_due(months_since_keep: u32, ecpm_move_pct: f64) -> bool {
    months_since_keep >= RECHECK_MONTH
        || (ecpm_move_pct.is_finite() && ecpm_move_pct.abs() > ECPM_MOVE_TRIGGER)
}

/// The reversal test plan: 25% of users on the old setting for 14 analysis
/// days.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReversalPlan {
    pub old_setting_share: f64,
    pub analysis_days: usize,
}

pub fn reversal_plan() -> ReversalPlan {
    ReversalPlan {
        old_setting_share: REVERSAL_SPLIT,
        analysis_days: REVERSAL_ANALYSIS_DAYS,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn triggers_are_month_six_or_a_thirty_percent_move() {
        assert!(!recheck_due(5, 0.10));
        assert!(recheck_due(6, 0.0));
        assert!(recheck_due(2, 0.31));
        assert!(recheck_due(2, -0.31));
        assert!(!recheck_due(2, 0.30));
        assert!(!recheck_due(2, f64::NAN));
    }

    #[test]
    fn the_reversal_plan_matches_the_spec() {
        let plan = reversal_plan();
        assert_eq!(plan.old_setting_share, 0.25);
        assert_eq!(plan.analysis_days, 14);
    }
}
