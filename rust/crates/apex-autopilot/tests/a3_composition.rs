//! A3 composition parity: the signed holdout fee survives the whole chain.
//!
//! One fixture (shared with the Python twin, `tools/lift-validation/
//! a3_fee_fixture.json`) flows through the exact production composition:
//! `PlayerEvidence` -> `measure_from_feed` -> `run_holdout_month` -> the
//! emitted `FeeLine` -> the `BillingBasisRecord` `runner.rs` builds ->
//! `apex_billing::invoice::compute_invoice_from_basis`. The invoice's fee
//! micros must equal the `FeeLine`'s fee micros to the micro, and the whole
//! chain must reproduce the fixture's golden fee. The fixture's measured
//! lift is strictly above its registered `lower_alpha`, so a
//! point-estimate fee would bill a different amount and fail here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use apex_autopilot::ops::PlayerEvidence;
use apex_lift::player_store::PlayerDay;
use apex_lift::run::{run_holdout_month, HoldoutMonthChange};

/// The golden vector both implementations must reproduce, to the micro.
const FIXTURE: &str = include_str!("../../../../tools/lift-validation/a3_fee_fixture.json");

const EXPOSED_IS_TEST: bool = false;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    month: String,
    month_start: String,
    month_end: String,
    month_days: u32,
    window_start_day: u32,
    window_days: u32,
    exposed_players: usize,
    control_players: usize,
    exposed_daily_micros_base: i64,
    exposed_daily_micros_stride: i64,
    exposed_daily_micros_mod: i64,
    control_daily_micros_base: i64,
    control_daily_micros_stride: i64,
    control_daily_micros_mod: i64,
    treated_revenue_micros: i64,
    scope_revenue_micros: i64,
    measured_lift: f64,
    lower_alpha: f64,
    fee_micros: i64,
    fee_micros_cent_rounded: i64,
}

fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE).expect("fixture parses")
}

/// The same per-player-per-day feed the Python twin builds: heterogeneous
/// revenue so the standard error is positive and `lift != lower_alpha`.
fn fixture_evidence(fx: &Fixture) -> PlayerEvidence {
    let mut rows = Vec::new();
    for offset in 0..fx.window_days {
        let day = format!(
            "{}-{:02}",
            &fx.month_start[..7],
            fx.window_start_day + offset
        );
        for index in 0..fx.exposed_players {
            rows.push(PlayerDay {
                day: day.clone(),
                user_id: Some(format!("e{index}")),
                idfv: None,
                idfa: None,
                test: EXPOSED_IS_TEST,
                revenue_micros: fx.exposed_daily_micros_base
                    + (index as i64 % fx.exposed_daily_micros_mod) * fx.exposed_daily_micros_stride,
            });
        }
        for index in 0..fx.control_players {
            rows.push(PlayerDay {
                day: day.clone(),
                user_id: Some(format!("c{index}")),
                idfv: None,
                idfa: None,
                test: !EXPOSED_IS_TEST,
                revenue_micros: fx.control_daily_micros_base
                    + (index as i64 % fx.control_daily_micros_mod) * fx.control_daily_micros_stride,
            });
        }
    }
    PlayerEvidence {
        rows,
        pretest: BTreeMap::new(),
        measurement_scope: Some("u1".to_string()),
        roster: Some(apex_lift::holdout::AssignedRoster {
            exposed: fx.exposed_players as u64,
            control: fx.control_players as u64,
        }),
        eligible: None,
        certified_gain_lower: Some(900_000_000),
        certified_delta_lower: Some(0.04),
        assigned_test_control: None,
    }
}

/// The no-roster golden: the same chain on the planned-share estimator, with
/// asymmetric zero-revenue players, unequal observed arm counts, anonymous
/// revenue in both arms and a control refund block.
const FIXTURE_NO_ROSTER: &str =
    include_str!("../../../../tools/lift-validation/a3_fee_fixture_noroster.json");

fn fixture_no_roster() -> FixtureNoRoster {
    serde_json::from_str(FIXTURE_NO_ROSTER).expect("fixture parses")
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureNoRoster {
    month: String,
    month_start: String,
    month_end: String,
    month_days: u32,
    window_start_day: u32,
    window_days: u32,
    exposed_players: usize,
    control_players: usize,
    exposed_daily_micros_base: i64,
    exposed_daily_micros_stride: i64,
    exposed_daily_micros_mod: i64,
    control_daily_micros_base: i64,
    control_daily_micros_stride: i64,
    control_daily_micros_mod: i64,
    exposed_zero_players: usize,
    control_zero_players: usize,
    anonymous_exposed_daily_micros: i64,
    anonymous_control_daily_micros: i64,
    treated_revenue_micros: i64,
    scope_revenue_micros: i64,
    measured_lift: f64,
    lower_alpha: f64,
    fee_micros: i64,
    fee_micros_cent_rounded: i64,
}

fn no_roster_rows(fx: &FixtureNoRoster) -> Vec<PlayerDay> {
    let mut rows = Vec::new();
    for offset in 0..fx.window_days {
        let day = format!(
            "{}-{:02}",
            &fx.month_start[..7],
            fx.window_start_day + offset
        );
        for index in 0..fx.exposed_players {
            rows.push(PlayerDay {
                day: day.clone(),
                user_id: Some(format!("e{index}")),
                idfv: None,
                idfa: None,
                test: EXPOSED_IS_TEST,
                revenue_micros: if index < fx.exposed_zero_players {
                    0
                } else {
                    fx.exposed_daily_micros_base
                        + (index as i64 % fx.exposed_daily_micros_mod)
                            * fx.exposed_daily_micros_stride
                },
            });
        }
        for index in 0..fx.control_players {
            rows.push(PlayerDay {
                day: day.clone(),
                user_id: Some(format!("c{index}")),
                idfv: None,
                idfa: None,
                test: !EXPOSED_IS_TEST,
                revenue_micros: if index < fx.control_zero_players {
                    0
                } else {
                    fx.control_daily_micros_base
                        + (index as i64 % fx.control_daily_micros_mod)
                            * fx.control_daily_micros_stride
                },
            });
        }
        rows.push(PlayerDay {
            day: day.clone(),
            user_id: None,
            idfv: None,
            idfa: None,
            test: EXPOSED_IS_TEST,
            revenue_micros: fx.anonymous_exposed_daily_micros,
        });
        rows.push(PlayerDay {
            day: day.clone(),
            user_id: None,
            idfv: None,
            idfa: None,
            test: !EXPOSED_IS_TEST,
            revenue_micros: fx.anonymous_control_daily_micros,
        });
    }
    rows
}

#[test]
fn invoice_fee_matches_the_planned_share_holdout_without_a_roster() {
    let fx = fixture_no_roster();
    let rows = no_roster_rows(&fx);

    let folded = apex_lift::holdout::fold_evidence_rows(&rows, &fx.month_end).expect("fold");
    let measurement = apex_lift::holdout::measure_from_feed(&rows, EXPOSED_IS_TEST, None, None)
        .expect("measurement");
    let estimate = measurement
        .estimate
        .clone()
        .expect("planned-share estimate");
    assert!(
        estimate.lower_alpha > 0.0,
        "a positive bound must bill: {estimate:?}"
    );

    let mut daily_exposed: BTreeMap<String, i64> = BTreeMap::new();
    for row in &rows {
        if row.test == EXPOSED_IS_TEST {
            *daily_exposed.entry(row.day.clone()).or_default() += row.revenue_micros;
        }
    }
    let exposed_revenue: i64 = daily_exposed.values().sum();

    let signed_evidence = apex_lift::holdout::HoldoutEvidence {
        month_end: fx.month_end.clone(),
        exposed_is_test: EXPOSED_IS_TEST,
        measurement_scope: "application:dogfood".to_string(),
        window_days: daily_exposed.keys().cloned().collect(),
        rows: folded,
        pretest: BTreeMap::new(),
        unidentified_micros: measurement.unidentified_micros,
        unidentified_exposed_micros: measurement.unidentified.exposed_micros,
        unidentified_control_micros: measurement.unidentified.control_micros,
        unidentified_share_bp: measurement.unidentified_share_bp,
        assigned: None,
        daily_exposed: daily_exposed.clone(),
        actual_gain_lower_micros: None,
        delta_lower: None,
    };
    let change = apex_lift::billing::KeptChange {
        change_id: "chg-a3-noroster".to_string(),
        lambda: 0.0,
        rate_bp: apex_core::RESULTS_RATE_BP,
        kept_day: "2026-09-15".to_string(),
        overridden_day: None,
        rolled_back_day: None,
        pre_registration_hash: "prereg-hash".to_string(),
        result_hash: "result-hash".to_string(),
    };
    let run = run_holdout_month(
        &fx.month,
        &fx.month_start,
        fx.month_days,
        fx.scope_revenue_micros,
        &[HoldoutMonthChange {
            change,
            estimate: Some(estimate.clone()),
            ledger_references: vec![
                "prereg-hash".to_string(),
                "result-hash".to_string(),
                "holdout-hash".to_string(),
            ],
            evidence: Some(signed_evidence),
            roster: None,
            exposed_revenue: Some(exposed_revenue),
            actual_gain_lower: None,
            delta_lower: None,
            exposed_daily: daily_exposed,
        }],
    )
    .expect("monthly holdout run");
    let fee = run.fees.first().expect("one fee line");

    assert_eq!(fee.fee_micros, fx.fee_micros, "fixture fee");
    assert_eq!(
        exposed_revenue, fx.treated_revenue_micros,
        "treated revenue includes the exposed anonymous block"
    );
    assert!(
        (estimate.lift - fx.measured_lift).abs() < 1e-12,
        "lift {} != {}",
        estimate.lift,
        fx.measured_lift
    );
    assert!(
        (estimate.lower_alpha - fx.lower_alpha).abs() < 1e-12,
        "lower_alpha {} != {}",
        estimate.lower_alpha,
        fx.lower_alpha
    );
    assert_eq!(
        measurement.unidentified.exposed_micros,
        fx.anonymous_exposed_daily_micros * fx.window_days as i64
    );
    assert_eq!(
        measurement.unidentified.control_micros,
        fx.anonymous_control_daily_micros * fx.window_days as i64
    );

    let mut fee = fee.clone();
    fee.fee_micros = apex_billing::round_to_cent(fee.fee_micros);
    if let Some(line) = &mut fee.holdout {
        line.fee_micros = fee.fee_micros;
    }
    assert_eq!(fee.fee_micros, fx.fee_micros_cent_rounded);
    let basis = apex_billing::BillingBasisRecord {
        seq: 1,
        at_unix_ms: 1_791_000_000_000,
        change_id: fee.change_id.clone(),
        scope_revenue: fee.scope_revenue,
        incremental: fee.incremental,
        log_share: fee.log_share,
        rate_bp: fee.rate_bp,
        fee_micros: fee.fee_micros,
        billed_days: fee.month_days,
        month_days: fee.month_days,
        holdout: fee.holdout.as_ref().map(|line| apex_billing::HoldoutLine {
            measured_lift: line.measured_lift,
            lower_90: line.lower_90,
            upper_90: line.upper_90,
            lower_alpha: line.lower_alpha,
            treated_revenue: line.revenue_micros,
            ledger_references: line.ledger_references.clone(),
            no_fee_reason: line.no_fee_note().map(str::to_string),
        }),
    };
    let period = apex_billing::BillingPeriod {
        year_month: fx.month.clone(),
        start_unix_ms: 1_790_812_800_000,
        end_unix_ms: 1_793_491_200_000,
    };
    let invoice =
        apex_billing::compute_invoice_from_basis("pub-a3", period, std::slice::from_ref(&basis))
            .expect("the signed basis verifies");
    let line = invoice.line_items.first().expect("one invoice line");
    assert_eq!(line.fee_micros, fee.fee_micros);
    assert!(line.fee_micros > 0);
}

#[test]
fn invoice_fee_matches_the_holdout_fee_line_to_the_micro() {
    let fx = fixture();
    let evidence = fixture_evidence(&fx);

    // The real estimator path: fold, measure against the assigned roster,
    // and take the adapter-certified certificate the runner falls back to
    // when no per-user envelopes are present (runner.rs evidence chain).
    let folded =
        apex_lift::holdout::fold_evidence_rows(&evidence.rows, &fx.month_end).expect("fold");
    let measurement = apex_lift::holdout::measure_from_feed(
        &evidence.rows,
        EXPOSED_IS_TEST,
        Some(&evidence.pretest),
        evidence.roster,
    )
    .expect("measurement");
    let estimate = measurement.estimate.expect("eligible estimate");
    assert!(
        (estimate.lift - estimate.lower_alpha).abs() > 1e-9,
        "the fixture must measure lift {} distinct from lower_alpha {}",
        estimate.lift,
        estimate.lower_alpha
    );
    assert!(estimate.lower_alpha > 0.0, "a positive bound must bill");

    let mut daily_exposed: BTreeMap<String, i64> = BTreeMap::new();
    for row in &evidence.rows {
        if row.test == EXPOSED_IS_TEST {
            *daily_exposed.entry(row.day.clone()).or_default() += row.revenue_micros;
        }
    }
    let exposed_revenue: i64 = daily_exposed.values().sum();
    assert_eq!(exposed_revenue, fx.treated_revenue_micros);

    let certificate = evidence
        .certified_gain_lower
        .zip(evidence.certified_delta_lower)
        .expect("adapter certificate");
    let change = apex_lift::billing::KeptChange {
        change_id: "chg-a3-composition".to_string(),
        lambda: 0.0,
        rate_bp: apex_core::RESULTS_RATE_BP,
        kept_day: "2026-09-15".to_string(),
        overridden_day: None,
        rolled_back_day: None,
        pre_registration_hash: "prereg-hash".to_string(),
        result_hash: "result-hash".to_string(),
    };
    let signed_evidence = apex_lift::holdout::HoldoutEvidence {
        month_end: fx.month_end.clone(),
        exposed_is_test: EXPOSED_IS_TEST,
        measurement_scope: "u1".to_string(),
        window_days: daily_exposed.keys().cloned().collect(),
        rows: folded,
        pretest: evidence.pretest.clone(),
        unidentified_micros: measurement.unidentified_micros,
        unidentified_exposed_micros: measurement.unidentified.exposed_micros,
        unidentified_control_micros: measurement.unidentified.control_micros,
        unidentified_share_bp: measurement.unidentified_share_bp,
        assigned: measurement.roster,
        daily_exposed: daily_exposed.clone(),
        actual_gain_lower_micros: Some(certificate.0),
        delta_lower: Some(certificate.1),
    };
    let run = run_holdout_month(
        &fx.month,
        &fx.month_start,
        fx.month_days,
        fx.scope_revenue_micros,
        &[HoldoutMonthChange {
            change,
            estimate: Some(estimate.clone()),
            ledger_references: vec![
                "prereg-hash".to_string(),
                "result-hash".to_string(),
                "holdout-hash".to_string(),
            ],
            evidence: Some(signed_evidence),
            roster: evidence.roster,
            exposed_revenue: Some(exposed_revenue),
            actual_gain_lower: Some(certificate.0),
            delta_lower: Some(certificate.1),
            exposed_daily: daily_exposed,
        }],
    )
    .expect("monthly holdout run");
    let fee = run.fees.first().expect("one fee line");

    // The bound-derived fee, pinned by the shared golden vector and equal
    // to the Python twin's value for the same fixture.
    assert_eq!(fee.fee_micros, fx.fee_micros);
    assert!((estimate.lift - fx.measured_lift).abs() < 1e-12);
    assert!((estimate.lower_alpha - fx.lower_alpha).abs() < 1e-12);
    let bound = estimate.lower_alpha.max(0.0);
    let expected = (fx.treated_revenue_micros as f64
        * (1.0 - 1.0 / (1.0 + bound))
        * f64::from(apex_core::RESULTS_RATE_BP)
        / 10_000.0)
        .round() as i64;
    assert_eq!(fee.fee_micros, expected);

    // The runner's exact basis construction (runner.rs monthly pass): the
    // signed fee is cent-exact for Mollie, the invoice must re-derive it.
    let mut fee = fee.clone();
    fee.fee_micros = apex_billing::round_to_cent(fee.fee_micros);
    if let Some(line) = &mut fee.holdout {
        line.fee_micros = fee.fee_micros;
    }
    assert_eq!(fee.fee_micros, fx.fee_micros_cent_rounded);
    let basis = apex_billing::BillingBasisRecord {
        seq: 1,
        at_unix_ms: 1_791_000_000_000,
        change_id: fee.change_id.clone(),
        scope_revenue: fee.scope_revenue,
        incremental: fee.incremental,
        log_share: fee.log_share,
        rate_bp: fee.rate_bp,
        fee_micros: fee.fee_micros,
        billed_days: fee.month_days,
        month_days: fee.month_days,
        holdout: fee.holdout.as_ref().map(|line| apex_billing::HoldoutLine {
            measured_lift: line.measured_lift,
            lower_90: line.lower_90,
            upper_90: line.upper_90,
            lower_alpha: line.lower_alpha,
            treated_revenue: line.revenue_micros,
            ledger_references: line.ledger_references.clone(),
            no_fee_reason: line.no_fee_note().map(str::to_string),
        }),
    };
    let period = apex_billing::BillingPeriod {
        year_month: fx.month.clone(),
        start_unix_ms: 1_790_812_800_000,
        end_unix_ms: 1_793_491_200_000,
    };
    let invoice =
        apex_billing::compute_invoice_from_basis("pub-a3", period, std::slice::from_ref(&basis))
            .expect("the signed basis verifies");
    let line = invoice.line_items.first().expect("one invoice line");
    // The composition parity this test exists for: to the micro.
    assert_eq!(line.fee_micros, fee.fee_micros);
    assert_eq!(invoice.total_micros, fee.fee_micros);
    assert!(line.fee_micros > 0);
    assert_eq!(
        line.holdout.as_ref().expect("holdout line").no_fee_reason,
        None
    );
}
