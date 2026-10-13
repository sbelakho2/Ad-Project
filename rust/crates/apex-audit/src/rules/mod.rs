//! The five audit rules. Each rule is a pure function `&Snapshot -> Vec<Finding>`
//! and returns findings in an order that does not depend on the order of the
//! input vectors (rules aggregate through sorted maps; the engine sorts the
//! final list).
//!
//! Shared arithmetic lives here; the recovery model is:
//!
//! ```text
//! revenue_for(impressions) = ecpm_micros * impressions / 1000      // eCPM is per 1000 impressions
//! recovery                 = revenue_for(affected) * fraction_bp / 10_000
//! ```
//!
//! computed in `i128` and saturated into `i64` (never negative, never wraps).
//! Integer division truncates toward zero, which is pinned by golden tests.

pub mod app_ads_txt;
pub mod disabled_bidder;
pub mod missing_floor;
pub mod reporting_gap;
pub mod stale_floor;

use apex_core::Micros;

use crate::model::{Finding, RuleId, Severity, Snapshot};

/// Trim + uppercase; snapshot country codes are compared normalized. Validation
/// is the connector's job (fail-closed parsing); the engine must not reject.
pub(crate) fn normalize_country(input: &str) -> String {
    input.trim().to_uppercase()
}

/// `apex_core::ecpm_micros` semantics (revenue micros per 1000 impressions,
/// `None` on zero impressions or i64 overflow) generalized to i128 sums so a
/// rule can aggregate many rows before computing the eCPM.
pub(crate) fn ecpm_i128(revenue_micros: i128, impressions: u64) -> Option<Micros> {
    if impressions == 0 {
        return None;
    }
    let value = revenue_micros.checked_mul(1000)? / impressions as i128;
    Micros::try_from(value).ok()
}

/// Clamp into `[i64::MIN, i64::MAX]` (evidence numbers and sums).
pub(crate) fn clamp_i64(value: i128) -> i64 {
    if value > i64::MAX as i128 {
        i64::MAX
    } else if value < i64::MIN as i128 {
        i64::MIN
    } else {
        value as i64
    }
}

/// Clamp a recovery into `[0, i64::MAX]`: recovery is never a negative claim.
pub(crate) fn clamp_recovery(value: i128) -> Micros {
    if value <= 0 {
        0
    } else if value > i64::MAX as i128 {
        i64::MAX
    } else {
        value as i64
    }
}

/// Recovery model (see module docs): `ecpm * impressions / 1000 * fraction_bp / 10_000`.
/// Returns 0 when there is no money basis (non-positive eCPM, no impressions,
/// non-positive fraction); saturates to `i64::MAX` on an (unreachable in
/// practice) `i128` overflow.
pub(crate) fn recovery_from_ecpm(ecpm: Micros, impressions: u64, fraction_bp: i64) -> Micros {
    if ecpm <= 0 || impressions == 0 || fraction_bp <= 0 {
        return 0;
    }
    let revenue = (ecpm as i128) * (impressions as i128) / 1000;
    let scaled = revenue
        .checked_mul(fraction_bp as i128)
        .map_or(i64::MAX as i128, |value| value / 10_000);
    clamp_recovery(scaled)
}

/// Drop one severity level; `Info` is the floor. Used by `stale_floor` when no
/// eCPM basis exists (documented per-rule rule).
pub(crate) fn downgrade(severity: Severity) -> Severity {
    match severity {
        Severity::Critical => Severity::Warning,
        Severity::Warning => Severity::Info,
        Severity::Info => Severity::Info,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn finding(
    snapshot: &Snapshot,
    rule: RuleId,
    severity: Severity,
    subject: String,
    evidence: serde_json::Value,
    message: String,
    estimated_monthly_recovery: Micros,
) -> Finding {
    Finding {
        rule,
        severity,
        platform: snapshot.platform,
        subject,
        evidence,
        message,
        estimated_monthly_recovery,
    }
}
