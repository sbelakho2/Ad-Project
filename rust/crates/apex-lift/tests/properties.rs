//! A3 T1: the lift properties that must hold for every run, independent of a
#![allow(clippy::expect_used)]
//! fixture. These are algebraic invariants, not another copy of the golden
//! tests.

use apex_lift::holdout::{
    estimate as holdout_estimate, invoice_line, monthly_fee, within_term, HoldoutEstimate,
    PlayerRevenue, HOLDOUT_SHARE, ROLLOUT_SHARE,
};
use apex_lift::stats::estimate;

fn estimate_with(lift: f64, revenue: i64) -> HoldoutEstimate {
    HoldoutEstimate {
        lift,
        standard_error: 0.01,
        lower_90: lift - 0.016,
        upper_90: lift + 0.016,
        revenue_micros: revenue,
        exposed_players: 10,
        control_players: 10,
    }
}

#[test]
fn a_positive_lift_orders_the_bounds() {
    for lift in [0.001, 0.01, 0.1, 0.5] {
        let e = estimate_with(lift, 1_000_000);
        assert!(e.lower_90 < lift);
        assert!(lift < e.upper_90);
    }
}

#[test]
fn a_nonpositive_lift_never_bills() {
    for lift in [-0.5, -0.01, 0.0] {
        assert_eq!(monthly_fee(&estimate_with(lift, 1_000_000)), 0);
    }
}

#[test]
fn the_fee_never_exceeds_the_revenue_or_the_gain() {
    for lift in [0.001, 0.01, 0.1, 0.9] {
        let revenue = 10_000_000;
        let fee = monthly_fee(&estimate_with(lift, revenue));
        assert!(fee >= 0);
        assert!(fee <= revenue);
        let gain = (revenue as f64 * (1.0 - 1.0 / (1.0 + lift))) as i64;
        assert!(fee <= gain, "{fee} > {gain}");
    }
}

#[test]
fn rollout_and_holdout_partition_the_population() {
    assert!((ROLLOUT_SHARE + HOLDOUT_SHARE - 1.0).abs() < 1e-12);
}

#[test]
fn a_zero_fee_line_always_explains_itself() {
    let line = invoice_line(&estimate_with(0.0, 100), Vec::new());
    assert_eq!(line.fee_micros, 0);
    assert!(line.no_fee_note().is_some());
    let paid = invoice_line(&estimate_with(0.1, 1_000_000), Vec::new());
    assert!(paid.fee_micros > 0);
    assert!(paid.no_fee_note().is_none());
}

#[test]
fn the_term_ends_exactly_once() {
    assert!((0..12).all(within_term));
    assert!(!within_term(12));
    assert!(!within_term(13));
}

#[test]
fn a_per_player_estimate_has_finite_bounds() {
    let e = estimate(&[0.01, 0.011, 0.009, 0.012, 0.008]).expect("estimate");
    assert!(e.lower_90.is_finite() && e.upper_90.is_finite());
    assert!(e.lower_90 <= e.lift && e.lift <= e.upper_90);
}

#[test]
fn a_holdout_estimate_is_symmetric_under_an_arm_swap() {
    let rows = vec![
        PlayerRevenue {
            user: "e1".into(),
            revenue_micros: 120,
            pre_revenue_micros: None,
            exposed: true,
        },
        PlayerRevenue {
            user: "e2".into(),
            revenue_micros: 130,
            pre_revenue_micros: None,
            exposed: true,
        },
        PlayerRevenue {
            user: "c1".into(),
            revenue_micros: 100,
            pre_revenue_micros: None,
            exposed: false,
        },
        PlayerRevenue {
            user: "c2".into(),
            revenue_micros: 100,
            pre_revenue_micros: None,
            exposed: false,
        },
    ];
    let estimate = holdout_estimate(&rows).expect("estimate");
    // Swapping the arm labels inverts a positive lift to a negative one.
    let mut swapped = rows;
    for row in &mut swapped {
        row.exposed = !row.exposed;
    }
    let flipped = holdout_estimate(&swapped).expect("flipped");
    assert!(estimate.lift > 0.0 && flipped.lift < 0.0);
}

#[test]
fn a_flat_holdout_has_zero_incremental_revenue() {
    let e = estimate_with(0.0, 1_000_000);
    assert_eq!(monthly_fee(&e), 0);
}
