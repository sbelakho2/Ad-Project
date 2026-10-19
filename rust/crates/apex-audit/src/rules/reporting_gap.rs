//! Rule `reporting_gap` — platform-side totals and mediation-side totals disagree.
//!
//! Exact semantics (thresholds fire AT the boundary):
//!
//! - `delta = |platform_revenue - mediation_revenue|` (i256-safe: computed in
//!   `i128`).
//! - `pct_bp = delta * 10_000 / max(platform_revenue, mediation_revenue)`
//!   (truncating). The denominator is the larger of the two totals, so the gap
//!   is always a fraction of the best-case basis.
//! - Denominator `<= 0` (both totals zero, or both non-positive) => no finding:
//!   there is no basis for a percentage, and this must never divide by zero.
//!   One total zero and the other positive is a 100% gap and fires Critical.
//! - Fire at `>= 200` bp (Warning), `>= 1000` bp (Critical).
//!
//! Recovery: **0**. This is a data-quality/reporting finding — a mismatch does
//! not directly prove lost revenue, so the message is explicitly informational
//! and must not claim any recovery.

use crate::model::{Finding, RuleId, Severity, Snapshot};
use crate::rules::{clamp_i64, finding};

const WARN_GAP_BP: i128 = 200; // 2.00%
const CRITICAL_GAP_BP: i128 = 1000; // 10.00%

pub fn check(snapshot: &Snapshot) -> Vec<Finding> {
    // A missing mediation report is not a zero total: comparing a real
    // platform total against an unmeasured zero would report a permanent
    // 100% gap. The provider must actually report the second side.
    let platform = snapshot.platform_totals_30d.revenue_30d;
    let mediation = snapshot.mediation_totals_30d.revenue_30d;
    // Unknown is not zero: skip when the provider never reported a mediation
    // total (the flag is transient and absent on stored snapshots) and the
    // value is the default zero. A nonzero total is itself evidence it was
    // measured; an explicitly reported zero (flag set) still compares.
    if !snapshot.mediation_totals_reported && mediation <= 0 && platform > 0 {
        return Vec::new();
    }
    let denominator = platform.max(mediation);
    if denominator <= 0 {
        return Vec::new();
    }
    let delta = (platform as i128 - mediation as i128).abs();
    let pct_bp = delta * 10_000 / denominator as i128;
    let severity = if pct_bp >= CRITICAL_GAP_BP {
        Severity::Critical
    } else if pct_bp >= WARN_GAP_BP {
        Severity::Warning
    } else {
        return Vec::new();
    };
    let delta_i64 = clamp_i64(delta);
    let pct_bp_i64 = clamp_i64(pct_bp);
    let message = format!(
        "reporting gap between platform totals {platform} micros and mediation totals \
         {mediation} micros: absolute delta {delta_i64} micros ({pct_bp_i64} bp); informational \
         only, no recovery estimated"
    );
    vec![finding(
        snapshot,
        RuleId::ReportingGap,
        severity,
        "platform_totals_30d".to_string(),
        serde_json::json!({
            "platform_revenue_30d_micros": platform,
            "mediation_revenue_30d_micros": mediation,
            "absolute_delta_micros": delta_i64,
            "pct_bp": pct_bp_i64,
            "denominator_basis": "max_of_totals",
            "recovery_model": "not_applicable",
            "estimated_monthly_recovery_micros": 0,
        }),
        message,
        0,
    )]
}

#[cfg(test)]
mod tests {
    use crate::testkit::{empty_snapshot, totals};

    use super::*;

    fn snapshot_with(platform: i64, mediation: i64) -> Snapshot {
        let mut snapshot = empty_snapshot();
        snapshot.platform_totals_30d = totals(platform, 100_000);
        snapshot.mediation_totals_30d = totals(mediation, 100_000);
        snapshot.mediation_totals_reported = true;
        snapshot
    }

    fn only(snapshot: &Snapshot) -> Finding {
        let mut findings = check(snapshot);
        assert_eq!(findings.len(), 1, "expected exactly one finding");
        findings.remove(0)
    }

    #[test]
    fn warning_at_exact_2_percent_boundary() {
        let snapshot = snapshot_with(1_000_000, 980_000);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.evidence["pct_bp"], 200);
        assert_eq!(finding.evidence["absolute_delta_micros"], 20_000);
    }

    #[test]
    fn does_not_fire_just_below_2_percent() {
        let snapshot = snapshot_with(1_000_000, 981_000);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn critical_at_exact_10_percent_boundary() {
        let snapshot = snapshot_with(1_000_000, 900_000);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.evidence["pct_bp"], 1_000);
    }

    #[test]
    fn warning_just_below_10_percent() {
        let snapshot = snapshot_with(1_000_000, 901_000);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.evidence["pct_bp"], 990);
    }

    #[test]
    fn gap_is_symmetric() {
        let below = snapshot_with(980_000, 1_000_000);
        let finding = only(&below);
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.evidence["pct_bp"], 200);
    }

    #[test]
    fn both_totals_zero_does_not_fire() {
        let snapshot = snapshot_with(0, 0);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn both_totals_non_positive_does_not_fire() {
        let snapshot = snapshot_with(-5, -10);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn one_zero_one_positive_is_a_full_gap() {
        let snapshot = snapshot_with(1_000_000, 0);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.evidence["pct_bp"], 10_000);
    }

    #[test]
    fn identical_totals_do_not_fire() {
        let snapshot = snapshot_with(1_000_000, 1_000_000);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn recovery_is_zero_and_message_is_informational() {
        let snapshot = snapshot_with(1_000_000, 900_000);
        let finding = only(&snapshot);
        assert_eq!(finding.rule, RuleId::ReportingGap);
        assert_eq!(finding.subject, "platform_totals_30d");
        assert_eq!(finding.estimated_monthly_recovery, 0);
        assert_eq!(finding.evidence["recovery_model"], "not_applicable");
        assert!(finding.message.contains("no recovery estimated"));
        assert!(finding.message.contains("informational"));
    }
}
