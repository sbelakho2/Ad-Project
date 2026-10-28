//! The monthly billing run: split the month into intervals with a stable
//! active change set, attribute each interval's revenue to the changes active
//! throughout it, and sum every change's interval contributions.
//!
//! The decomposition is the point: a change that was active for one day only
//! participates in that day's interval, so it can never re-price another
//! change for the rest of the month. Term ends, publisher overrides and
//! rollbacks all cut the interval at the day they land.

use apex_core::Micros;

use crate::billing::{
    attribute_interval, parse_billing_day, revenue_through, round_micros, BillingStop, FeeLine,
    KeptChange,
};
use crate::error::{LiftError, LiftResult};

/// One change's billing stop reason in a month.
type SkippedChanges = Vec<(String, BillingStop)>;

/// One month of billing for one scope.
#[derive(Debug, Clone, PartialEq)]
pub struct MonthRun {
    pub month: String,
    pub scope_revenue: Micros,
    /// Changes billed this month. Each line already reflects every interval
    /// the change was active in; nothing is pro-rated a second time.
    pub fees: Vec<FeeLine>,
    /// Changes skipped because the term ended, the publisher overrode them or
    /// they were rolled back before this month.
    pub skipped: SkippedChanges,
}

/// A change's billable day range inside the month, inclusive.
struct BillableRange {
    index: usize,
    first: u32,
    last: u32,
}

/// Run one month.
///
/// `month_start` is the first day of the billing month (`YYYY-MM-DD`) and
/// `month_days` its length. The month is decomposed at every day on which the
/// active set changes (a keep, an override, a term anniversary or a
/// rollback). Each interval's revenue is attributed only to the changes that
/// were active for the whole interval.
pub fn run_month(
    month: &str,
    month_start: &str,
    month_days: u32,
    scope_revenue: Micros,
    changes: &[KeptChange],
) -> LiftResult<MonthRun> {
    let (ranges, skipped) = billable_ranges(month_start, month_days, changes)?;

    let mut fees = Vec::new();
    if scope_revenue > 0 && !ranges.is_empty() {
        let attributed = attributed_by_change(scope_revenue, month_days, changes, &ranges, 1.0)?;
        for range in &ranges {
            let Some(total) = attributed.get(range.index) else {
                continue;
            };
            if total.billed_days == 0 {
                continue;
            }
            let change = &changes[range.index];
            let rate = change.rate_bp as f64 / 10_000.0;
            let fee_micros = round_micros(rate * total.attributed as f64)?;
            fees.push(FeeLine {
                change_id: change.change_id.clone(),
                scope_revenue,
                treated_revenue: total.revenue,
                holdout: None,
                lambda: change.lambda,
                factor: total.factor,
                incremental: total.attributed,
                // The line's `incremental` is already this change's share of
                // every interval, so its own share of that number is one.
                log_share: 1.0,
                rate_bp: change.rate_bp,
                fee_micros,
                billed_days: total.billed_days,
                month_days,
                pre_registration_hash: change.pre_registration_hash.clone(),
                result_hash: change.result_hash.clone(),
            });
        }
    }

    Ok(MonthRun {
        month: month.to_string(),
        scope_revenue,
        fees,
        skipped,
    })
}

/// One kept change with the measured holdout reading for a month.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldoutMonthChange {
    pub change: KeptChange,
    /// The trailing measurement, when the feed carried enough players.
    pub estimate: Option<crate::holdout::HoldoutEstimate>,
    /// Signed ledger references behind the reading (pre-registration, keep
    /// result, holdout state).
    pub ledger_references: Vec<String>,
    /// Signed raw rows behind the estimate, when the feed carried them.
    pub evidence: Option<crate::holdout::HoldoutEvidence>,
}

/// Run one month of A3 holdout billing.
///
/// The month is decomposed exactly like [`run_month`], but each interval's
/// revenue is first reduced to the 90% exposed share, then the measured lift
/// is applied and the 30% results rate is billed. A flat or negative
/// measurement still produces a zero-fee line so the invoice can state "no
/// fee this month". A change with no usable reading bills nothing (the old
/// 14-day lift path is never used for an A3 holdout).
pub fn run_holdout_month(
    month: &str,
    month_start: &str,
    month_days: u32,
    scope_revenue: Micros,
    changes: &[HoldoutMonthChange],
) -> LiftResult<MonthRun> {
    if changes.len() > 1 {
        return Err(LiftError::Malformed(
            "multiple billable holdouts target one causal scope".into(),
        ));
    }
    let billing: Vec<KeptChange> = changes
        .iter()
        .map(|holdout| {
            let mut change = holdout.change.clone();
            change.rate_bp = crate::holdout::HOLDOUT_RATE_BP;
            change.lambda = holdout.estimate.as_ref().map_or(0.0, |estimate| {
                if crate::holdout::split_is_90_10(estimate) {
                    crate::holdout::billing_lift(estimate)
                } else {
                    0.0
                }
            });
            change
        })
        .collect();
    let (ranges, skipped) = billable_ranges(month_start, month_days, &billing)?;

    let treated_share = changes
        .first()
        .and_then(|holdout| holdout.estimate.as_ref())
        .and_then(crate::holdout::exposed_share)
        .filter(|_| {
            changes
                .first()
                .and_then(|holdout| holdout.estimate.as_ref())
                .is_some_and(crate::holdout::split_is_90_10)
        })
        .unwrap_or(0.0);
    let mut fees = Vec::new();
    if scope_revenue > 0 && !ranges.is_empty() {
        let attributed =
            attributed_by_change(scope_revenue, month_days, &billing, &ranges, treated_share)?;
        for range in &ranges {
            let Some(total) = attributed.get(range.index) else {
                continue;
            };
            if total.billed_days == 0 {
                continue;
            }
            let holdout = &changes[range.index];
            let change = &billing[range.index];
            let (measured_lift, lower_90, upper_90) = match &holdout.estimate {
                Some(estimate) => (estimate.lift, estimate.lower_90, estimate.upper_90),
                None => (0.0, 0.0, 0.0),
            };
            let fee_micros = if holdout.estimate.is_some() {
                // `total.attributed` is already the exposed incremental
                // revenue; the 30% rate is all that remains.
                round_micros(
                    f64::from(crate::holdout::HOLDOUT_RATE_BP) / 10_000.0 * total.attributed as f64,
                )?
            } else {
                0
            };
            let split_ok = holdout
                .estimate
                .as_ref()
                .is_none_or(crate::holdout::split_is_90_10);
            let no_fee_reason = (fee_micros == 0).then(|| {
                if holdout.estimate.is_none() {
                    crate::holdout::NO_MEASUREMENT_REASON.to_string()
                } else if !split_ok {
                    crate::holdout::INVALID_SPLIT_REASON.to_string()
                } else if holdout
                    .estimate
                    .as_ref()
                    .is_some_and(|estimate| crate::holdout::billing_lift(estimate) <= 0.0)
                {
                    crate::holdout::NO_PROVEN_GAIN_REASON.to_string()
                } else {
                    crate::holdout::GAIN_ROUNDED_TO_ZERO_REASON.to_string()
                }
            });
            let line = crate::holdout::HoldoutInvoiceLine {
                measured_lift,
                lower_90,
                upper_90,
                revenue_micros: total.revenue,
                fee_micros,
                ledger_references: holdout.ledger_references.clone(),
                evidence: holdout.evidence.clone(),
                no_fee_reason,
            };
            fees.push(FeeLine {
                change_id: change.change_id.clone(),
                scope_revenue,
                treated_revenue: total.revenue,
                holdout: Some(line),
                lambda: change.lambda,
                factor: total.factor,
                incremental: total.attributed,
                log_share: 1.0,
                rate_bp: crate::holdout::HOLDOUT_RATE_BP,
                fee_micros,
                billed_days: total.billed_days,
                month_days,
                pre_registration_hash: change.pre_registration_hash.clone(),
                result_hash: change.result_hash.clone(),
            });
        }
    }

    Ok(MonthRun {
        month: month.to_string(),
        scope_revenue,
        fees,
        skipped,
    })
}

/// The ordinal of a stop date, or `None` when it does not parse.
fn stop_ordinal(day: &str) -> Option<i64> {
    let (year, month, day_of_month) = parse_billing_day(day).ok()?;
    Some(ordinal(year, month, day_of_month))
}

/// Decompose the month into each change's billable day range.
fn billable_ranges(
    month_start: &str,
    month_days: u32,
    changes: &[KeptChange],
) -> LiftResult<(Vec<BillableRange>, SkippedChanges)> {
    let (start_year, start_month, _) = parse_billing_day(month_start)?;
    if month_days == 0 {
        return Err(LiftError::Malformed(
            "a billing month must have at least one day".into(),
        ));
    }
    let month_start_ord = ordinal(start_year, start_month, 1);
    let month_end_ord = ordinal(start_year, start_month, month_days);
    let mut skipped: Vec<(String, BillingStop)> = Vec::new();
    let mut ranges: Vec<BillableRange> = Vec::new();

    for (index, change) in changes.iter().enumerate() {
        let (kept_year, kept_month, kept_day) = parse_billing_day(&change.kept_day)?;
        let kept = ordinal(kept_year, kept_month, kept_day);
        if kept > month_end_ord {
            skipped.push((change.change_id.clone(), BillingStop::NotStarted));
            continue;
        }
        let (anniversary_year, anniversary_month, anniversary_day) =
            anniversary(kept_year, kept_month, kept_day);
        let anniversary_ord = ordinal(anniversary_year, anniversary_month, anniversary_day);
        match change.billing_stop(month_start)? {
            BillingStop::No => {}
            stop => {
                // A stop strictly before this month ends it entirely; a stop
                // landing ON the first day must fall through to the range
                // math below, where the override and anniversary days are
                // billable and a rollback day is not. Short-circuiting here
                // lost that first day.
                let stop_ord = match stop {
                    BillingStop::Override => {
                        change.overridden_day.as_deref().and_then(stop_ordinal)
                    }
                    BillingStop::RolledBack => {
                        change.rolled_back_day.as_deref().and_then(stop_ordinal)
                    }
                    BillingStop::TermEnd => Some(anniversary_ord),
                    BillingStop::No | BillingStop::NotStarted => None,
                };
                if stop_ord.is_none_or(|at| at < month_start_ord) {
                    skipped.push((change.change_id.clone(), stop));
                    continue;
                }
            }
        }
        // Day-of-month of the first billable day: a keep inside the month
        // starts at its own day, an earlier keep starts at day one. The
        // clamped difference avoids an equivalent day-one comparison.
        let first = 1 + u32::try_from((kept - month_start_ord).max(0)).map_err(|_| {
            LiftError::Malformed("kept day is outside the billing month range".into())
        })?;
        let mut last = month_days;
        let mut override_ord: Option<i64> = None;
        let mut rollback_ord: Option<i64> = None;

        if let Some(day) = &change.overridden_day {
            let (year, month_number, day_of_month) = parse_billing_day(day)?;
            let at = ordinal(year, month_number, day_of_month);
            if (month_start_ord..=month_end_ord).contains(&at) {
                // The override day itself is still billable (the historical
                // contract pro-rates through the override day).
                last = last.min(day_of_month);
                override_ord = Some(at);
            } else if at < month_start_ord {
                skipped.push((change.change_id.clone(), BillingStop::Override));
                continue;
            }
        }
        if let Some(day) = &change.rolled_back_day {
            let (year, month_number, day_of_month) = parse_billing_day(day)?;
            let at = ordinal(year, month_number, day_of_month);
            if (month_start_ord..=month_end_ord).contains(&at) {
                // The rollback day is the first day the change no longer
                // serves users; it is not billable.
                last = last.min(day_of_month.saturating_sub(1));
                rollback_ord = Some(at);
            } else if at < month_start_ord {
                skipped.push((change.change_id.clone(), BillingStop::RolledBack));
                continue;
            }
        }

        if (month_start_ord..=month_end_ord).contains(&anniversary_ord) {
            // The anniversary day closes the term and is still billable.
            last = last.min(anniversary_day);
        }

        if first > last {
            let first_ord = ordinal(start_year, start_month, first);
            let stop = if rollback_ord.is_some_and(|at| at <= first_ord) {
                BillingStop::RolledBack
            } else if override_ord.is_some_and(|at| at <= first_ord) {
                BillingStop::Override
            } else if anniversary_ord <= first_ord {
                BillingStop::TermEnd
            } else {
                BillingStop::NotStarted
            };
            skipped.push((change.change_id.clone(), stop));
            continue;
        }
        ranges.push(BillableRange { index, first, last });
    }
    Ok((ranges, skipped))
}

/// One change's accumulated interval contribution.
struct Attributed {
    attributed: Micros,
    /// Interval scope revenue in which the change was active. The holdout
    /// fee applies its 90% exposed share to this amount.
    revenue: Micros,
    billed_days: u32,
    factor: f64,
    factor_days: u32,
}

/// Decompose the month into stable-active-set intervals and accumulate each
/// change's attributed incremental revenue.
fn attributed_by_change(
    scope_revenue: Micros,
    month_days: u32,
    changes: &[KeptChange],
    ranges: &[BillableRange],
    treated_share: f64,
) -> LiftResult<Vec<Attributed>> {
    let mut boundaries: Vec<u32> = vec![1, month_days + 1];
    for range in ranges {
        boundaries.push(range.first);
        boundaries.push(range.last.saturating_add(1).min(month_days + 1));
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut totals: Vec<Attributed> = changes
        .iter()
        .map(|_| Attributed {
            attributed: 0,
            revenue: 0,
            billed_days: 0,
            factor: 1.0,
            factor_days: 0,
        })
        .collect();

    for window in boundaries.windows(2) {
        let (start, end) = (window[0], window[1]);
        if start >= end {
            continue;
        }
        let active_ranges: Vec<&BillableRange> = ranges
            .iter()
            .filter(|range| range.first <= start && range.last >= start)
            .collect();
        if active_ranges.is_empty() {
            continue;
        }
        let active: Vec<KeptChange> = active_ranges
            .iter()
            .map(|range| changes[range.index].clone())
            .collect();
        let through_end = revenue_through(scope_revenue, end - 1, month_days)?;
        let through_start = revenue_through(scope_revenue, start - 1, month_days)?;
        let interval_revenue = through_end.checked_sub(through_start).ok_or_else(|| {
            LiftError::Malformed("interval revenue allocation went negative".into())
        })?;
        // The holdout bills the exposed share of the interval; the legacy
        // lift path uses the whole scope revenue (share 1.0).
        let treated_revenue = if treated_share == 1.0 {
            interval_revenue
        } else {
            round_micros(interval_revenue as f64 * treated_share)?
        };
        let attribution = attribute_interval(treated_revenue, &active)?;
        let interval_days = end - start;
        for (position, range) in active_ranges.iter().enumerate() {
            let total = totals.get_mut(range.index).ok_or_else(|| {
                LiftError::Malformed("interval attribution lost its change".into())
            })?;
            total.attributed = total
                .attributed
                .checked_add(attribution.attributed[position])
                .ok_or_else(|| {
                    LiftError::Malformed("attributed revenue overflowed the micro range".into())
                })?;
            total.revenue = total
                .revenue
                .checked_add(treated_revenue)
                .ok_or_else(|| LiftError::Malformed("interval revenue overflowed".into()))?;
            total.billed_days = total
                .billed_days
                .checked_add(interval_days)
                .ok_or_else(|| LiftError::Malformed("billed day count overflowed".into()))?;
            if interval_days > total.factor_days {
                total.factor = attribution.factor;
                total.factor_days = interval_days;
            }
        }
    }
    Ok(totals)
}

/// The kept date plus twelve months, clamping the day to the target month's
/// length.
fn anniversary(year: i32, month: u32, day: u32) -> (i32, u32, u32) {
    let year = year + 1;
    let max_day = days_in_month(year, month);
    (year, month, day.min(max_day))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 30,
    }
}

/// A monotone ordinal for comparison across months and years.
fn ordinal(year: i32, month: u32, day: u32) -> i64 {
    (year as i64) * 10_000 + (month as i64) * 100 + day as i64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn change(id: &str, lambda: f64, override_day: Option<&str>) -> KeptChange {
        KeptChange {
            change_id: id.into(),
            lambda,
            rate_bp: 2000,
            kept_day: "2026-01-15".into(),
            overridden_day: override_day.map(str::to_string),
            rolled_back_day: None,
            pre_registration_hash: format!("pre-{id}"),
            result_hash: format!("result-{id}"),
        }
    }

    fn one_interval_fee(revenue: Micros, lambda: f64) -> Micros {
        let c = change("x", lambda, None);
        let attribution = attribute_interval(revenue, std::slice::from_ref(&c)).expect("attr");
        round_micros(0.2 * attribution.attributed[0] as f64).expect("fee")
    }

    #[test]
    fn a_clean_month_bills_every_active_change() {
        let run = run_month(
            "2026-03",
            "2026-03-01",
            31,
            18_000_000_000,
            &[change("c1", 0.03, None), change("c2", 0.02, None)],
        )
        .expect("run");
        assert_eq!(run.fees.len(), 2);
        assert!(run.skipped.is_empty());
        for fee in &run.fees {
            assert_eq!(fee.billed_days, 31);
            assert_eq!(fee.month_days, 31);
            assert_eq!(fee.result_hash, format!("result-{}", fee.change_id));
        }
        // The whole month is one stable interval, so the combined attribution
        // is the same model the single-interval formula uses.
        let attribution = attribute_interval(
            18_000_000_000,
            &[change("c1", 0.03, None), change("c2", 0.02, None)],
        )
        .expect("attr");
        assert_eq!(run.fees[0].incremental, attribution.attributed[0]);
        assert_eq!(run.fees[1].incremental, attribution.attributed[1]);
        assert_eq!(
            run.fees.iter().map(|f| f.fee_micros).sum::<Micros>(),
            round_micros(0.2 * attribution.attributed[0] as f64).expect("fee")
                + round_micros(0.2 * attribution.attributed[1] as f64).expect("fee")
        );
    }

    #[test]
    fn overrides_cut_the_interval_and_stop_after() {
        let run = run_month(
            "2026-03",
            "2026-03-01",
            31,
            10_000_000_000,
            &[change("c1", 0.05, Some("2026-03-11"))],
        )
        .expect("run");
        assert_eq!(run.fees.len(), 1);
        // Days 1..=11 belong to the change; day 12 onward is overridden.
        let expected_revenue = revenue_through(10_000_000_000, 11, 31).expect("revenue");
        assert_eq!(run.fees[0].billed_days, 11);
        assert_eq!(
            run.fees[0].fee_micros,
            one_interval_fee(expected_revenue, 0.05)
        );

        // The next month skips it entirely.
        let after = run_month(
            "2026-04",
            "2026-04-01",
            30,
            10_000_000_000,
            &[change("c1", 0.05, Some("2026-03-11"))],
        )
        .expect("run");
        assert!(after.fees.is_empty());
        assert_eq!(after.skipped.len(), 1);
        assert_eq!(after.skipped[0].1, BillingStop::Override);
    }

    #[test]
    fn an_override_on_the_first_day_still_bills_that_day() {
        let run = run_month(
            "2026-03",
            "2026-03-01",
            31,
            10_000_000_000,
            &[change("c1", 0.05, Some("2026-03-01"))],
        )
        .expect("run");
        assert_eq!(run.fees.len(), 1, "the override day itself is billable");
        assert_eq!(run.fees[0].billed_days, 1);
        let day_one = revenue_through(10_000_000_000, 1, 31).expect("revenue");
        assert_eq!(run.fees[0].fee_micros, one_interval_fee(day_one, 0.05));
        // The month after an override on day one bills nothing.
        let after = run_month(
            "2026-04",
            "2026-04-01",
            30,
            10_000_000_000,
            &[change("c1", 0.05, Some("2026-03-01"))],
        )
        .expect("run");
        assert!(after.fees.is_empty());
        assert_eq!(after.skipped[0].1, BillingStop::Override);
    }

    #[test]
    fn an_anniversary_on_the_first_day_still_bills_that_day() {
        let mut c = change("c1", 0.05, None);
        c.kept_day = "2026-03-01".into();
        // Twelve months later the term closes on 2027-03-01, which is the
        // next month's first day: the term's last day is still billable.
        let run =
            run_month("2027-03", "2027-03-01", 31, 10_000_000_000, &[c.clone()]).expect("run");
        assert_eq!(run.fees.len(), 1, "the term's final day is billable");
        assert_eq!(run.fees[0].billed_days, 1);
        let after = run_month("2027-04", "2027-04-01", 30, 10_000_000_000, &[c]).expect("run");
        assert!(after.fees.is_empty());
        assert_eq!(after.skipped[0].1, BillingStop::TermEnd);
    }

    #[test]
    fn a_rollback_on_the_first_day_bills_nothing() {
        let mut c = change("c1", 0.05, None);
        c.kept_day = "2026-01-15".into();
        c.rolled_back_day = Some("2026-03-01".to_string());
        let run = run_month("2026-03", "2026-03-01", 31, 10_000_000_000, &[c]).expect("run");
        assert!(run.fees.is_empty(), "a rollback day never bills");
        assert_eq!(run.skipped[0].1, BillingStop::RolledBack);
    }

    #[test]
    fn a_change_kept_mid_month_bills_from_its_keep_day() {
        let mut c = change("c1", 0.05, None);
        c.kept_day = "2026-03-15".into();
        let run =
            run_month("2026-03", "2026-03-01", 31, 10_000_000_000, &[c.clone()]).expect("run");
        // 17 billable days: the 15th through the 31st.
        assert_eq!(run.fees[0].billed_days, 17);
        let before_keep = revenue_through(10_000_000_000, 14, 31).expect("revenue");
        let expected_revenue = 10_000_000_000 - before_keep;
        assert_eq!(
            run.fees[0].fee_micros,
            one_interval_fee(expected_revenue, 0.05)
        );
        // The month before the keep skips it as not started.
        let before = run_month("2026-02", "2026-02-01", 28, 10_000_000_000, &[c]).expect("run");
        assert!(before.fees.is_empty());
        assert_eq!(before.skipped[0].1, BillingStop::NotStarted);
    }

    #[test]
    fn the_anniversary_month_bills_up_to_the_anniversary_day() {
        let mut c = change("c1", 0.05, None);
        c.kept_day = "2026-01-15".into();
        let run =
            run_month("2027-01", "2027-01-01", 31, 10_000_000_000, &[c.clone()]).expect("run");
        // The 1st through the 15th: fifteen days.
        assert_eq!(run.fees[0].billed_days, 15);
        let expected_revenue = revenue_through(10_000_000_000, 15, 31).expect("revenue");
        assert_eq!(
            run.fees[0].fee_micros,
            one_interval_fee(expected_revenue, 0.05)
        );
        // The month after the anniversary skips the change as term-ended.
        let after = run_month("2027-02", "2027-02-01", 28, 10_000_000_000, &[c]).expect("run");
        assert!(after.fees.is_empty());
        assert_eq!(after.skipped[0].1, BillingStop::TermEnd);
    }

    #[test]
    fn a_one_day_change_only_reprices_its_own_day() {
        // A is active all month; B is active only on day 20. The rest of the
        // month (days 1..=19 and 21..=30) must attribute exactly as if B had
        // never been kept, and B must bill only its one day.
        let a = change("A", 0.03, None);
        let mut b = change("B", 0.02, None);
        b.kept_day = "2026-04-20".into();
        b.overridden_day = Some("2026-04-20".into());
        let revenue = 30_000_000_000;
        let with_b = run_month(
            "2026-04",
            "2026-04-01",
            30,
            revenue,
            &[a.clone(), b.clone()],
        )
        .expect("run");
        assert_eq!(with_b.fees.len(), 2);
        let a_line = with_b
            .fees
            .iter()
            .find(|fee| fee.change_id == "A")
            .expect("A");
        let b_line = with_b
            .fees
            .iter()
            .find(|fee| fee.change_id == "B")
            .expect("B");
        assert_eq!(b_line.billed_days, 1);

        // A's contribution outside day 20 is exactly the standalone A run on
        // those same two intervals.
        let day20_revenue = revenue_through(revenue, 20, 30).expect("through 20")
            - revenue_through(revenue, 19, 30).expect("through 19");
        let a_only_day20 = attribute_interval(day20_revenue, std::slice::from_ref(&a))
            .expect("attr")
            .attributed[0];
        let a_with_b_day20 = attribute_interval(day20_revenue, &[a.clone(), b])
            .expect("attr")
            .attributed[0];
        let through_19 = revenue_through(revenue, 19, 30).expect("through 19");
        let after_20 = revenue - revenue_through(revenue, 20, 30).expect("through 20");
        let a_outside = through_19 + after_20;
        let a_only = attribute_interval(a_outside, std::slice::from_ref(&a))
            .expect("attr")
            .attributed[0];
        assert_eq!(
            a_line.incremental,
            a_only + a_with_b_day20,
            "only day 20 is shared, so only day 20 can change A"
        );
        assert!(
            a_with_b_day20 < a_only_day20,
            "B takes a share of day 20 only"
        );
        // B is not attributed revenue from any day other than the 20th.
        let b_attr = attribute_interval(
            day20_revenue,
            &[change("A", 0.03, None), change("B", 0.02, None)],
        )
        .expect("attr")
        .attributed[1];
        assert_eq!(b_line.incremental, b_attr);
        assert_eq!(b_line.billed_days, 1);
    }

    #[test]
    fn b_active_days_20_to_30_prices_only_that_window() {
        // A is active all month; B is active days 20..=30. A's attribution
        // for days 1..=19 and 31 is exactly its standalone attribution; only
        // days 20..=30 are shared.
        let a = change("A", 0.03, None);
        let mut b = change("B", 0.02, None);
        b.kept_day = "2026-04-20".into();
        b.overridden_day = Some("2026-04-30".into());
        let revenue = 30_000_000_000;
        let run = run_month(
            "2026-04",
            "2026-04-01",
            30,
            revenue,
            &[a.clone(), b.clone()],
        )
        .expect("run");
        let a_line = run.fees.iter().find(|fee| fee.change_id == "A").expect("A");
        let b_line = run.fees.iter().find(|fee| fee.change_id == "B").expect("B");
        assert_eq!(b_line.billed_days, 11);
        let shared_revenue = revenue_through(revenue, 30, 30).expect("through 30")
            - revenue_through(revenue, 19, 30).expect("through 19");
        let shared = attribute_interval(shared_revenue, &[a.clone(), b]).expect("attr");
        let outside_revenue = revenue_through(revenue, 19, 30).expect("through 19");
        let outside = attribute_interval(outside_revenue, std::slice::from_ref(&a))
            .expect("attr")
            .attributed[0];
        assert_eq!(
            a_line.incremental,
            outside + shared.attributed[0],
            "A's days outside B's window are untouched"
        );
        assert_eq!(b_line.incremental, shared.attributed[1]);
    }

    #[test]
    fn a_zero_day_change_never_reaches_billing() {
        // B is kept after the month closes: it has no billable day, bills
        // nothing, and A's whole-month attribution is exactly the same as if
        // B had never been proposed.
        let a = change("A", 0.03, None);
        let mut b = change("B", 0.02, None);
        b.kept_day = "2026-05-02".into();
        let revenue = 30_000_000_000;
        let with_b = run_month("2026-04", "2026-04-01", 30, revenue, &[a.clone(), b]).expect("run");
        let a_only = run_month("2026-04", "2026-04-01", 30, revenue, &[a]).expect("run");
        assert_eq!(with_b.fees.len(), 1);
        assert_eq!(with_b.fees[0].incremental, a_only.fees[0].incremental);
        assert_eq!(with_b.fees[0].fee_micros, a_only.fees[0].fee_micros);
        assert_eq!(with_b.skipped[0].1, BillingStop::NotStarted);
    }

    #[test]
    fn overlapping_and_non_overlapping_changes_sum_their_own_intervals() {
        // A runs days 1..=10, B runs days 20..=30: the intervals do not
        // overlap and each change bills only its own days.
        let mut a = change("A", 0.03, None);
        a.kept_day = "2026-04-01".into();
        a.overridden_day = Some("2026-04-10".into());
        let mut b = change("B", 0.02, None);
        b.kept_day = "2026-04-20".into();
        let run = run_month(
            "2026-04",
            "2026-04-01",
            30,
            30_000_000_000,
            &[a.clone(), b.clone()],
        )
        .expect("run");
        assert_eq!(run.fees.len(), 2);
        let a_line = run.fees.iter().find(|fee| fee.change_id == "A").expect("A");
        let b_line = run.fees.iter().find(|fee| fee.change_id == "B").expect("B");
        let a_revenue = revenue_through(30_000_000_000, 10, 30).expect("through 10");
        let b_revenue =
            30_000_000_000 - revenue_through(30_000_000_000, 19, 30).expect("through 19");
        assert_eq!(a_line.billed_days, 10);
        assert_eq!(b_line.billed_days, 11);
        assert_eq!(a_line.fee_micros, one_interval_fee(a_revenue, 0.03));
        assert_eq!(b_line.fee_micros, one_interval_fee(b_revenue, 0.02));
    }

    #[test]
    fn a_rollback_stops_the_interval_on_its_own_day() {
        let mut c = change("c1", 0.05, None);
        c.kept_day = "2026-04-01".into();
        c.rolled_back_day = Some("2026-04-11".into());
        let run =
            run_month("2026-04", "2026-04-01", 30, 30_000_000_000, &[c.clone()]).expect("run");
        assert_eq!(
            run.fees[0].billed_days, 10,
            "the rollback day is not billed"
        );
        let expected_revenue = revenue_through(30_000_000_000, 10, 30).expect("through 10");
        assert_eq!(
            run.fees[0].fee_micros,
            one_interval_fee(expected_revenue, 0.05)
        );
        let after = run_month("2026-05", "2026-05-01", 31, 30_000_000_000, &[c]).expect("run");
        assert!(after.fees.is_empty());
        assert_eq!(after.skipped[0].1, BillingStop::RolledBack);

        // A rollback on the keep day leaves no billable day at all: the
        // change is reported as rolled back, never as merely not started.
        let mut same_day = change("c2", 0.05, None);
        same_day.kept_day = "2026-06-10".into();
        same_day.rolled_back_day = Some("2026-06-10".into());
        let same =
            run_month("2026-06", "2026-06-01", 30, 30_000_000_000, &[same_day]).expect("run");
        assert!(same.fees.is_empty());
        assert_eq!(same.skipped[0].1, BillingStop::RolledBack);
    }

    fn holdout_estimate(lift: f64) -> crate::holdout::HoldoutEstimate {
        crate::holdout::HoldoutEstimate {
            lift,
            standard_error: 0.005,
            lower_90: lift - 0.01,
            upper_90: lift + 0.01,
            revenue_micros: 1_000_000,
            exposed_players: 270,
            control_players: 30,
        }
    }

    fn holdout_change(
        id: &str,
        kept_day: &str,
        estimate: Option<f64>,
        rolled_back_day: Option<&str>,
    ) -> HoldoutMonthChange {
        HoldoutMonthChange {
            change: KeptChange {
                change_id: id.into(),
                lambda: 0.0,
                rate_bp: 2_000,
                kept_day: kept_day.into(),
                overridden_day: None,
                rolled_back_day: rolled_back_day.map(str::to_string),
                pre_registration_hash: format!("pre-{id}"),
                result_hash: format!("result-{id}"),
            },
            estimate: estimate.map(holdout_estimate),
            ledger_references: vec![format!("pre-{id}"), format!("result-{id}")],
            evidence: None,
        }
    }

    #[test]
    fn zero_scope_revenue_never_emits_a_fee_line() {
        let run =
            run_month("2026-03", "2026-03-01", 31, 0, &[change("c1", 0.05, None)]).expect("run");
        assert!(run.fees.is_empty(), "no revenue, no line");
        let holdout = run_holdout_month(
            "2026-03",
            "2026-03-01",
            31,
            0,
            &[holdout_change("h1", "2026-01-10", Some(0.05), None)],
        )
        .expect("holdout run");
        assert!(holdout.fees.is_empty(), "no revenue, no holdout line");
    }

    #[test]
    fn a_holdout_month_bills_thirty_percent_of_the_exposed_gain() {
        let run = run_holdout_month(
            "2026-03",
            "2026-03-01",
            31,
            1_000_000_000,
            &[holdout_change("h1", "2026-01-10", Some(0.04), None)],
        )
        .expect("run");
        assert_eq!(run.fees.len(), 1);
        let fee = &run.fees[0];
        // The treated revenue is the 90% exposed share of the month.
        assert_eq!(fee.treated_revenue, 900_000_000);
        let bound = crate::holdout::billing_lift(&holdout_estimate(0.04));
        let gain = (fee.treated_revenue as f64 * (1.0 - 1.0 / (1.0 + bound))).round() as Micros;
        assert_eq!(fee.incremental, gain);
        assert_eq!(
            fee.fee_micros,
            (0.30 * gain as f64).round() as Micros,
            "30% of the measured gain"
        );
        assert!(fee.fee_micros > 0);
        assert_eq!(fee.rate_bp, crate::holdout::HOLDOUT_RATE_BP);
        let line = fee.holdout.as_ref().expect("holdout line");
        assert_eq!(line.measured_lift, 0.04);
        assert_eq!(line.ledger_references.len(), 2);
        assert!(line.no_fee_note().is_none());
        assert!(
            line.no_fee_reason.is_none(),
            "a billed line carries no no-fee reason"
        );
    }

    #[test]
    fn an_invalid_split_on_an_existing_estimate_bills_nothing() {
        let mut estimate = holdout_estimate(0.05);
        estimate.exposed_players = 90;
        estimate.control_players = 90;
        let run = run_holdout_month(
            "2026-03",
            "2026-03-01",
            31,
            1_000_000_000,
            &[HoldoutMonthChange {
                change: holdout_change("h1", "2026-01-10", None, None).change,
                estimate: Some(estimate),
                ledger_references: Vec::new(),
                evidence: None,
            }],
        )
        .expect("run");
        assert_eq!(run.fees.len(), 1);
        assert_eq!(run.fees[0].fee_micros, 0, "50/50 is not billable");
        assert_eq!(run.fees[0].treated_revenue, 0);
        assert!(run.fees[0].holdout.as_ref().is_some_and(|line| {
            line.no_fee_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("90/10"))
        }));
    }

    #[test]
    fn a_positive_bound_whose_gain_rounds_to_zero_explains_itself() {
        let mut estimate = holdout_estimate(1.0e-8);
        estimate.lower_90 = 1.0e-9;
        estimate.upper_90 = 2.0e-8;
        let run = run_holdout_month(
            "2026-03",
            "2026-03-01",
            31,
            1_000_000,
            &[HoldoutMonthChange {
                change: holdout_change("h1", "2026-01-10", None, None).change,
                estimate: Some(estimate),
                ledger_references: Vec::new(),
                evidence: None,
            }],
        )
        .expect("run");
        assert_eq!(run.fees.len(), 1);
        assert_eq!(run.fees[0].fee_micros, 0);
        assert_eq!(run.fees[0].incremental, 0, "the gain rounds to nothing");
        let line = run.fees[0].holdout.as_ref().expect("line");
        assert_eq!(
            line.no_fee_reason.as_deref(),
            Some(crate::holdout::GAIN_ROUNDED_TO_ZERO_REASON)
        );
    }

    #[test]
    fn a_flat_or_harmful_holdout_emits_a_zero_fee_line() {
        for lift in [0.0, -0.04] {
            let run = run_holdout_month(
                "2026-03",
                "2026-03-01",
                31,
                1_000_000_000,
                &[holdout_change("h1", "2026-01-10", Some(lift), None)],
            )
            .expect("run");
            assert_eq!(run.fees.len(), 1, "{lift}");
            assert_eq!(run.fees[0].fee_micros, 0, "{lift}");
            let line = run.fees[0].holdout.as_ref().expect("line");
            assert!(line.no_fee_note().is_some());
            assert_eq!(
                line.no_fee_reason.as_deref(),
                Some(crate::holdout::NO_PROVEN_GAIN_REASON),
                "a zero-fee line carries its typed reason on the line itself"
            );
        }
    }

    #[test]
    fn multiple_billable_holdouts_in_one_scope_are_refused() {
        let a = holdout_change("A", "2026-04-01", Some(0.03), None);
        let b = holdout_change("B", "2026-04-20", Some(0.02), Some("2026-04-30"));
        let error = run_holdout_month("2026-04", "2026-04-01", 30, 30_000_000_000, &[a, b])
            .expect_err("overlapping holdouts are not an identified design");
        assert!(
            error.to_string().contains("multiple billable holdouts"),
            "{error}"
        );
    }

    #[test]
    fn a_mid_month_stop_bills_only_the_interval_before_it() {
        let run = run_holdout_month(
            "2026-04",
            "2026-04-01",
            30,
            30_000_000_000,
            &[holdout_change(
                "h1",
                "2026-01-10",
                Some(0.05),
                Some("2026-04-11"),
            )],
        )
        .expect("run");
        assert_eq!(run.fees.len(), 1);
        let fee = &run.fees[0];
        assert_eq!(fee.billed_days, 10, "the rollback day is not billed");
        let treated =
            (revenue_through(30_000_000_000, 10, 30).expect("10") as f64 * 0.9).round() as Micros;
        assert_eq!(fee.treated_revenue, treated);
        let bound = crate::holdout::billing_lift(&holdout_estimate(0.05));
        let gain = (treated as f64 * (1.0 - 1.0 / (1.0 + bound))).round() as Micros;
        assert_eq!(fee.fee_micros, (0.30 * gain as f64).round() as Micros);
    }

    #[test]
    fn a_holdout_bills_for_at_most_twelve_months() {
        let change = holdout_change("h1", "2026-01-15", Some(0.05), None);
        let anniversary = run_holdout_month(
            "2027-01",
            "2027-01-01",
            31,
            10_000_000_000,
            std::slice::from_ref(&change),
        )
        .expect("anniversary month");
        assert_eq!(anniversary.fees.len(), 1);
        assert_eq!(
            anniversary.fees[0].billed_days, 15,
            "the anniversary day closes the term and is billed"
        );
        let after = run_holdout_month("2027-02", "2027-02-01", 28, 10_000_000_000, &[change])
            .expect("after term");
        assert!(after.fees.is_empty());
        assert_eq!(after.skipped[0].1, BillingStop::TermEnd);
    }

    #[test]
    fn term_end_and_bad_days_behave() {
        let mut expired = change("c1", 0.03, None);
        expired.kept_day = "2024-01-01".into();
        let run = run_month("2026-03", "2026-03-01", 31, 10_000_000_000, &[expired]).expect("run");
        assert!(run.fees.is_empty());
        assert_eq!(run.skipped[0].1, BillingStop::TermEnd);
        assert!(run_month("2026-03", "not-a-day", 31, 10_000_000_000, &[]).is_err());
        assert!(run_month("2026-03", "2026-03-01", 0, 10_000_000_000, &[]).is_err());
    }
}
