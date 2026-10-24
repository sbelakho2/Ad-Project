//! Money and lift math. Integer micros only; every boundary is unit-tested.

/// 1e-6 of a currency unit.
pub type Micros = i64;

pub const MICROS_PER_UNIT: i64 = 1_000_000;

/// The autopilot bills a share of platform-proven lift; the share is fixed by the
/// business model to 15%..20% and enforced in code, not in a spreadsheet.
pub const MIN_LIFT_RATE_BP: u16 = 1_500; // legacy proven-lift contracts
pub const MAX_LIFT_RATE_BP: u16 = 2_000;
/// A3's single holdout results rate: 30% of the monthly measured gain.
pub const RESULTS_RATE_BP: u16 = 3_000;

/// eCPM in micros from revenue micros and an impression count.
/// `None` when impressions is zero (division by zero must never produce a number)
/// or when the i128 intermediate does not fit back into i64.
pub fn ecpm_micros(revenue_micros: Micros, impressions: u64) -> Option<Micros> {
    if impressions == 0 {
        return None;
    }
    let value = (revenue_micros as i128).checked_mul(1000)? / impressions as i128;
    Micros::try_from(value).ok()
}

/// Percent lift of `variant` over `control`, in basis points (1 bp = 0.01%).
/// `None` when control is not positive (no baseline, no claim) or on overflow.
pub fn lift_basis_points(control: Micros, variant: Micros) -> Option<i64> {
    if control <= 0 {
        return None;
    }
    let delta = (variant as i128) - (control as i128);
    let bp = delta.checked_mul(10_000)? / control as i128;
    i64::try_from(bp).ok()
}

/// Fee micros for a *platform-proven* lift.
/// Anchored business rules, unit-tested:
/// - zero or negative proven lift -> zero fee (no lift, no fee)
/// - rate outside 15%..=20% -> typed validation error
/// - integer division truncates toward zero; overflow is an error, never a wrap
pub fn lift_fee_micros(proven_lift_micros: Micros, rate_bp: u16) -> Result<Micros, String> {
    if !(MIN_LIFT_RATE_BP..=MAX_LIFT_RATE_BP).contains(&rate_bp) {
        return Err(format!(
            "lift rate {rate_bp} bp outside [{MIN_LIFT_RATE_BP}, {MAX_LIFT_RATE_BP}]"
        ));
    }
    if proven_lift_micros <= 0 {
        return Ok(0);
    }
    let fee = (proven_lift_micros as i128)
        .checked_mul(rate_bp as i128)
        .ok_or_else(|| "lift fee overflow".to_string())?
        / 10_000;
    Micros::try_from(fee).map_err(|_| "lift fee overflow".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecpm_needs_impressions() {
        assert_eq!(ecpm_micros(5 * MICROS_PER_UNIT, 0), None);
        assert_eq!(
            ecpm_micros(5 * MICROS_PER_UNIT, 1000),
            Some(5 * MICROS_PER_UNIT)
        );
        assert_eq!(ecpm_micros(500_000, 1000), Some(500_000));
    }

    #[test]
    fn ecpm_overflow_is_none() {
        assert_eq!(ecpm_micros(i64::MAX, 1), None);
    }

    #[test]
    fn lift_requires_positive_control() {
        assert_eq!(lift_basis_points(0, 100), None);
        assert_eq!(lift_basis_points(-5, 100), None);
        assert_eq!(lift_basis_points(100, 100), Some(0));
        assert_eq!(lift_basis_points(100, 105), Some(500)); // +5.00%
        assert_eq!(lift_basis_points(100, 95), Some(-500));
        assert_eq!(
            lift_basis_points(50_000 * MICROS_PER_UNIT, 55_000 * MICROS_PER_UNIT),
            Some(1_000)
        );
    }

    #[test]
    fn fee_only_on_proven_positive_lift() {
        assert_eq!(lift_fee_micros(0, 1_800), Ok(0));
        assert_eq!(lift_fee_micros(-1_000_000, 1_800), Ok(0));
        // $2.50 proven lift at 20% -> $0.50
        assert_eq!(lift_fee_micros(2_500_000, 2_000), Ok(500_000));
        // truncation is explicit: 3 micros at 15% -> 0 micros (documented integer rule)
        assert_eq!(lift_fee_micros(3, 1_500), Ok(0));
    }

    #[test]
    fn fee_rate_bounds_are_typed_errors() {
        assert!(lift_fee_micros(1_000_000, 1_499).is_err());
        assert!(lift_fee_micros(1_000_000, 2_001).is_err());
        assert!(lift_fee_micros(1_000_000, 1_500).is_ok());
        assert!(lift_fee_micros(1_000_000, 2_000).is_ok());
    }

    #[test]
    fn fee_overflow_is_error_not_wrap() {
        let huge = i64::MAX / 2;
        assert!(lift_fee_micros(huge, 2_000).is_ok()); // fits
                                                       // Exact integer arithmetic, pinned: i64::MAX * 2000 / 10000 floor-divides.
        assert_eq!(
            lift_fee_micros(i64::MAX, 2_000),
            Ok(1_844_674_407_370_955_161)
        );
    }
}
