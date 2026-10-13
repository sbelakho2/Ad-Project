//! Billing proofs: the only records that carry money.
//!
//! A proof is refused unless it is *provable*: a positive control revenue, a
//! strictly positive proven lift and a rate inside the business model's
//! 15.00%..=20.00% band. Zero lift needs no proof (a zero-fee proof would be
//! ledger noise), and a loss never bills.
//!
//! Arithmetic is delegated to `apex-core`:
//! [`apex_core::lift_basis_points`] validates the lift (and its sign) and
//! [`apex_core::lift_fee_micros`] computes the fee with truncation toward zero
//! and overflow as an error, never a wrap.

use apex_core::{
    lift_basis_points, lift_fee_micros, Micros, PlatformError, MAX_LIFT_RATE_BP, MIN_LIFT_RATE_BP,
};

use crate::record::LedgerRecord;

/// Build the `billingProof` record for an experiment's proven lift.
///
/// The stored `provenLift` is the absolute revenue delta
/// `variant_revenue - control_revenue` in micros; the stored `feeMicros` is
/// recomputed here (never taken from the caller) as
/// [`apex_core::lift_fee_micros(proven_lift, rate_bp)`]. The returned record is
/// unsealed; append it to a [`crate::Ledger`] or
/// [`crate::InMemoryLedger`] to chain and sign it.
///
/// Refusals ([`PlatformError::Validation`]):
/// - `control_revenue <= 0` (no baseline, no claim),
/// - proven lift is zero or negative (nothing to bill),
/// - `rate_bp` outside `[1500, 2000]` (15.00%..=20.00%).
pub fn build_billing_proof(
    publisher_id: &str,
    experiment_id: &str,
    control_revenue: Micros,
    variant_revenue: Micros,
    rate_bp: u16,
    at_unix_ms: i64,
) -> Result<LedgerRecord, PlatformError> {
    if control_revenue <= 0 {
        return Err(PlatformError::Validation(format!(
            "billing proof needs a positive control revenue, got {control_revenue} micros"
        )));
    }
    let lift_bp = lift_basis_points(control_revenue, variant_revenue).ok_or_else(|| {
        PlatformError::Validation("billing proof lift is not representable".to_string())
    })?;
    if lift_bp <= 0 {
        return Err(PlatformError::Validation(format!(
            "billing proof needs a positive proven lift, got {lift_bp} bp"
        )));
    }
    if !(MIN_LIFT_RATE_BP..=MAX_LIFT_RATE_BP).contains(&rate_bp) {
        return Err(PlatformError::Validation(format!(
            "lift rate {rate_bp} bp outside [{MIN_LIFT_RATE_BP}, {MAX_LIFT_RATE_BP}]"
        )));
    }
    // Positive basis points imply variant > control, so the absolute delta is
    // positive; `checked_sub` is kept so no arithmetic path can wrap.
    let proven_lift = variant_revenue
        .checked_sub(control_revenue)
        .ok_or_else(|| {
            PlatformError::Validation("billing proof lift overflows i64 micros".to_string())
        })?;
    let fee_micros = lift_fee_micros(proven_lift, rate_bp).map_err(PlatformError::Validation)?;
    Ok(LedgerRecord::billing_proof(
        at_unix_ms,
        publisher_id,
        experiment_id,
        proven_lift,
        rate_bp,
        fee_micros,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(record: &LedgerRecord) -> (i64, u16, i64) {
        match record {
            LedgerRecord::BillingProof {
                proven_lift,
                rate_bp,
                fee_micros,
                ..
            } => (*proven_lift, *rate_bp, *fee_micros),
            other => panic!("expected billing proof, got {other:?}"),
        }
    }

    fn error(control: i64, variant: i64, rate_bp: u16) -> PlatformError {
        build_billing_proof(
            "pub-1",
            "exp-1",
            control,
            variant,
            rate_bp,
            1_700_000_000_000,
        )
        .expect_err("must be refused")
    }

    #[test]
    fn billing_arithmetic_is_pinned() {
        // $2.50 proven lift at 20% -> $0.50 fee (500000 micros).
        let proof = build_billing_proof(
            "pub-1",
            "exp-1",
            10_000_000,
            12_500_000,
            2_000,
            1_700_000_000_000,
        )
        .expect("valid proof");
        assert_eq!(fields(&proof), (2_500_000, 2_000, 500_000));

        // Same lift at the 15% floor -> $0.375 fee.
        let proof = build_billing_proof(
            "pub-1",
            "exp-1",
            10_000_000,
            12_500_000,
            1_500,
            1_700_000_000_000,
        )
        .expect("valid proof");
        assert_eq!(fields(&proof), (2_500_000, 1_500, 375_000));

        // Rate bounds themselves are inside the band.
        for rate in [1_500u16, 2_000] {
            assert!(build_billing_proof("p", "e", 1_000_000, 2_000_000, rate, 0).is_ok());
        }
    }

    #[test]
    fn billing_refuses_non_positive_control() {
        for control in [0, -1, -1_000_000] {
            let err = error(control, control + 1_000_000, 2_000);
            assert!(
                matches!(err, PlatformError::Validation(_)),
                "control {control} gave {err}"
            );
        }
    }

    #[test]
    fn billing_refuses_zero_and_negative_lift() {
        // zero lift: variant == control
        assert!(matches!(
            error(5_000_000, 5_000_000, 1_800),
            PlatformError::Validation(_)
        ));
        // negative lift: variant < control
        assert!(matches!(
            error(5_000_000, 4_999_999, 1_800),
            PlatformError::Validation(_)
        ));
        assert!(matches!(
            error(5_000_000, -5_000_000, 1_800),
            PlatformError::Validation(_)
        ));
        // extreme negative variant is also refused, not wrapped
        assert!(matches!(
            error(1, i64::MIN, 1_800),
            PlatformError::Validation(_)
        ));
        // unrepresentable basis points are refused
        assert!(matches!(
            error(1, i64::MAX, 1_800),
            PlatformError::Validation(_)
        ));
    }

    #[test]
    fn billing_refuses_rate_outside_band() {
        // 14.99% and 20.01% are both refused.
        assert!(matches!(
            error(10_000_000, 12_500_000, 1_499),
            PlatformError::Validation(_)
        ));
        assert!(matches!(
            error(10_000_000, 12_500_000, 2_001),
            PlatformError::Validation(_)
        ));
        // 0% and 100% too.
        assert!(matches!(
            error(10_000_000, 12_500_000, 0),
            PlatformError::Validation(_)
        ));
        assert!(matches!(
            error(10_000_000, 12_500_000, 10_000),
            PlatformError::Validation(_)
        ));
    }

    #[test]
    fn fee_is_recomputed_not_taken_from_caller() {
        // Same inputs always produce the same stored fee, and it matches
        // apex-core's arithmetic exactly.
        let proof = build_billing_proof("p", "e", 2_000_000, 3_000_000, 1_700, 0).expect("ok");
        let (lift, rate, fee) = fields(&proof);
        assert_eq!(fee, lift_fee_micros(lift, rate).expect("core fee"));
        assert_eq!(lift, 1_000_000);
    }
}
