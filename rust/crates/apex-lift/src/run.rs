//! The monthly billing run: iterate the active kept changes for a scope,
//! applying term ends, publisher overrides with day-level pro-rating, and the
//! chain multiplication of every change still in force.

use apex_core::Micros;

use crate::billing::{monthly_fees, parse_billing_day, pro_rate, BillingStop, FeeLine, KeptChange};
use crate::error::LiftResult;

/// One month of billing for one scope.
#[derive(Debug, Clone, PartialEq)]
pub struct MonthRun {
    pub month: String,
    pub scope_revenue: Micros,
    /// Changes billed this month, already pro-rated where an override landed
    /// mid-month.
    pub fees: Vec<FeeLine>,
    /// Changes skipped because the term ended or the publisher overrode them
    /// before this month.
    pub skipped: Vec<(String, BillingStop)>,
}

/// Run one month.
///
/// `month_start` is the first day of the billing month (`YYYY-MM-DD`) and
/// `month_days` its length. A change kept inside the month bills only for the
/// days it existed; an override inside the month pro-rates up to that day; a
/// term anniversary inside the month pro-rates up to the anniversary day.
pub fn run_month(
    month: &str,
    month_start: &str,
    month_days: u32,
    scope_revenue: Micros,
    changes: &[KeptChange],
) -> LiftResult<MonthRun> {
    let (start_year, start_month, start_day) = parse_billing_day(month_start)?;
    let month_end_ordinal = ordinal(start_year, start_month, month_days);
    let mut active: Vec<KeptChange> = Vec::new();
    let mut skipped: Vec<(String, BillingStop)> = Vec::new();
    let mut active_days: Vec<(String, u32)> = Vec::new();
    for change in changes {
        let (kept_year, kept_month, kept_day) = parse_billing_day(&change.kept_day)?;
        if ordinal(kept_year, kept_month, kept_day) > month_end_ordinal {
            skipped.push((change.change_id.clone(), BillingStop::NotStarted));
            continue;
        }
        match change.billing_stop(month_start)? {
            BillingStop::No => {
                let _ = start_day;
                let days = billed_days(change, start_year, start_month, month_days)?;
                active.push(change.clone());
                active_days.push((change.change_id.clone(), days));
            }
            stop => skipped.push((change.change_id.clone(), stop)),
        }
    }
    let mut fees = monthly_fees(scope_revenue, &active)?;
    for fee in &mut fees {
        if let Some((_, days)) = active_days.iter().find(|(id, _)| *id == fee.change_id) {
            fee.billed_days = *days;
            fee.month_days = month_days;
            if *days < month_days {
                fee.fee_micros = pro_rate(fee.fee_micros, *days, month_days);
            }
        }
    }
    Ok(MonthRun {
        month: month.to_string(),
        scope_revenue,
        fees,
        skipped,
    })
}

/// Whole days a change is billable inside this month: from the keep day, to
/// the override day or the term anniversary when either lands inside it.
fn billed_days(
    change: &KeptChange,
    start_year: i32,
    start_month: u32,
    month_days: u32,
) -> LiftResult<u32> {
    let (kept_year, kept_month, kept_day) = parse_billing_day(&change.kept_day)?;
    let month_start = ordinal(start_year, start_month, 1);
    let month_end = ordinal(start_year, start_month, month_days);
    let kept = ordinal(kept_year, kept_month, kept_day);
    // A change kept inside the month bills from its keep day; one kept in an
    // earlier month bills from the first.
    let from = if (month_start..=month_end).contains(&kept) {
        kept_day
    } else {
        1
    };
    let mut to = month_days;
    if let Some(day) = &change.overridden_day {
        let (override_year, override_month, override_day) = parse_billing_day(day)?;
        let overridden = ordinal(override_year, override_month, override_day);
        if (month_start..=month_end).contains(&overridden) {
            to = to.min(override_day);
        }
    }
    let anniversary = ordinal(kept_year + 1, kept_month, kept_day);
    if (month_start..=month_end).contains(&anniversary) {
        to = to.min(kept_day.min(month_days));
    }
    Ok(to.saturating_sub(from) + 1)
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
            pre_registration_hash: format!("pre-{id}"),
            result_hash: format!("result-{id}"),
        }
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
        assert_eq!(
            run.fees.iter().map(|f| f.fee_micros).sum::<Micros>(),
            173_386_636
        );
    }

    #[test]
    fn overrides_pro_rate_inside_the_month_and_stop_after() {
        let run = run_month(
            "2026-03",
            "2026-03-01",
            31,
            10_000_000_000,
            &[change("c1", 0.05, Some("2026-03-11"))],
        )
        .expect("run");
        assert_eq!(run.fees.len(), 1);
        let full = crate::billing::monthly_fees(10_000_000_000, &[change("c1", 0.05, None)])
            .expect("full")[0]
            .fee_micros;
        // Overridden on day 11: eleven active days of a 31-day month.
        let expected = crate::billing::pro_rate(full, 11, 31);
        assert_eq!(run.fees[0].fee_micros, expected);
        assert!(expected < full);

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
    fn a_change_kept_mid_month_bills_from_its_keep_day() {
        let mut c = change("c1", 0.05, None);
        c.kept_day = "2026-03-15".into();
        let run =
            run_month("2026-03", "2026-03-01", 31, 10_000_000_000, &[c.clone()]).expect("run");
        let full = crate::billing::monthly_fees(10_000_000_000, &[change("c1", 0.05, None)])
            .expect("full")[0]
            .fee_micros;
        // 17 billable days: the 15th through the 31st.
        assert_eq!(
            run.fees[0].fee_micros,
            crate::billing::pro_rate(full, 17, 31)
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
        let full = crate::billing::monthly_fees(10_000_000_000, &[change("c1", 0.05, None)])
            .expect("full")[0]
            .fee_micros;
        // The 1st through the 15th: fifteen days.
        assert_eq!(
            run.fees[0].fee_micros,
            crate::billing::pro_rate(full, 15, 31)
        );
        // The month after the anniversary skips the change as term-ended.
        let after = run_month("2027-02", "2027-02-01", 28, 10_000_000_000, &[c]).expect("run");
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
    }
}
