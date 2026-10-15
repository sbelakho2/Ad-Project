//! Lift billing over time: the monthly fee formulas, the 12-month term,
//! publisher-override pro-rating and the re-check rule that can only lower
//! the billed lift.

use apex_core::Micros;

use crate::error::{LiftError, LiftResult};

/// Why a change is or is not billed on a given day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingStop {
    /// Still billing.
    No,
    /// The 12-month term ended.
    TermEnd,
    /// The publisher overrode the setting.
    Override,
    /// The change was kept after the billed period began.
    NotStarted,
}

/// Parse `YYYY-MM-DD` into the day tuple used by the billing date math.
pub(crate) fn parse_billing_day(day: &str) -> LiftResult<(i32, u32, u32)> {
    parse_day(day)
        .filter(|parsed| valid_date(*parsed))
        .ok_or_else(|| LiftError::Malformed(format!("billing day '{day}' is not a valid date")))
}

/// One kept change that is still being billed.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptChange {
    pub change_id: String,
    /// Billable lift from the decision: `max(0, lower bound)`.
    pub lambda: f64,
    /// Contract rate in basis points (1500..=2000).
    pub rate_bp: u16,
    /// The day the change was kept, `YYYY-MM-DD`.
    pub kept_day: String,
    /// The day the publisher overrode the setting, when they did.
    pub overridden_day: Option<String>,
    /// Signed ledger reference of the pre-registration record.
    pub pre_registration_hash: String,
    /// Signed ledger reference of the result record that billed this lift.
    pub result_hash: String,
}

impl KeptChange {
    /// Billing stop state on `today`. Malformed dates are an error: corrupt
    /// ledger data must stop the billing run with an alert, never bill on.
    pub fn billing_stop(&self, today: &str) -> LiftResult<BillingStop> {
        let today_parsed = parse_day(today)
            .filter(|day| valid_date(*day))
            .ok_or_else(|| {
                LiftError::Malformed(format!("billing day '{today}' is not a valid date"))
            })?;
        if let Some(overridden) = &self.overridden_day {
            let overridden = parse_day(overridden)
                .filter(|day| valid_date(*day))
                .ok_or_else(|| {
                    LiftError::Malformed(format!("override day '{overridden}' is not a valid date"))
                })?;
            if today_parsed >= overridden {
                return Ok(BillingStop::Override);
            }
        }
        let kept = parse_day(&self.kept_day)
            .filter(|day| valid_date(*day))
            .ok_or_else(|| {
                LiftError::Malformed(format!("kept day '{}' is not a valid date", self.kept_day))
            })?;
        if today_parsed >= anniversary(kept) {
            return Ok(BillingStop::TermEnd);
        }
        Ok(BillingStop::No)
    }
}

/// Whether the parsed day exists in the real calendar (month lengths and leap
/// years included).
fn valid_date((year, month, day): Day) -> bool {
    (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)
}

type Day = (i32, u32, u32);

fn parse_day(day: &str) -> Option<Day> {
    let mut parts = day.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// The kept date plus twelve months, clamping the day to the target month's
/// length.
fn anniversary((year, month, day): Day) -> Day {
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

/// One invoice line for one change in one month. Every number the spec asks
/// an invoice line to show is present, including the rate and the signed
/// record references behind the lift.
#[derive(Debug, Clone, PartialEq)]
pub struct FeeLine {
    pub change_id: String,
    /// The pre-registered scope's platform-reported revenue for the month.
    pub scope_revenue: Micros,
    pub lambda: f64,
    /// Combined factor over every active change.
    pub factor: f64,
    /// Incremental revenue attributed to the scope this month.
    pub incremental: Micros,
    /// This change's log share of the combined factor.
    pub log_share: f64,
    /// Contract rate in basis points.
    pub rate_bp: u16,
    /// The fee in micros, rounded to the nearest micro (before pro-rating).
    pub fee_micros: Micros,
    /// Whole days the change was billable this month.
    pub billed_days: u32,
    /// Days in the month, for the pro-rata re-derivation.
    pub month_days: u32,
    /// Signed ledger reference of the pre-registration record.
    pub pre_registration_hash: String,
    /// Signed ledger reference of the result record.
    pub result_hash: String,
}

/// The monthly fees for one ad unit's pre-registered scope.
///
/// `scope_revenue` is the platform-reported revenue of the scope in the
/// month, `active` the changes still billed. Changes whose term ended or that
/// the publisher overrode are excluded by the caller.
pub fn monthly_fees(scope_revenue: Micros, active: &[KeptChange]) -> LiftResult<Vec<FeeLine>> {
    if active.is_empty() || scope_revenue <= 0 {
        return Ok(Vec::new());
    }
    if active
        .iter()
        .any(|c| c.lambda <= -1.0 || !c.lambda.is_finite())
    {
        return Err(LiftError::Malformed(
            "an active change carries a lambda at or below -100%".into(),
        ));
    }
    let factor: f64 = active.iter().map(|c| 1.0 + c.lambda).product();
    if factor <= 1.0 || !factor.is_finite() {
        return Err(LiftError::Malformed(
            "the combined factor is not above one".into(),
        ));
    }
    let incremental = (scope_revenue as f64 * (1.0 - 1.0 / factor)).round() as Micros;
    let log_factor = factor.ln();
    Ok(active
        .iter()
        .map(|change| {
            let share = (1.0 + change.lambda).ln() / log_factor;
            let rate = change.rate_bp as f64 / 10_000.0;
            let fee_micros = (rate * incremental as f64 * share).round() as Micros;
            FeeLine {
                change_id: change.change_id.clone(),
                scope_revenue,
                lambda: change.lambda,
                factor,
                incremental,
                log_share: share,
                rate_bp: change.rate_bp,
                fee_micros,
                billed_days: 0,
                month_days: 0,
                pre_registration_hash: change.pre_registration_hash.clone(),
                result_hash: change.result_hash.clone(),
            }
        })
        .collect())
}

/// Pro-rate one month's fee for a change the publisher overrode mid-month.
pub fn pro_rate(fee_micros: Micros, days_active: u32, days_in_month: u32) -> Micros {
    if days_in_month == 0 || days_active == 0 {
        return 0;
    }
    let days_active = days_active.min(days_in_month);
    (fee_micros as f64 * days_active as f64 / days_in_month as f64).round() as Micros
}

/// The month-6 re-check outcome. Billing continues at the smaller of the
/// original and the re-check lower bound; a zero or negative re-check bound
/// rolls the change back and stops billing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Recheck {
    Continue { billed_lambda: f64 },
    Rollback,
}

pub fn recheck(original_lambda: f64, recheck_lower_bound: f64) -> Recheck {
    // A missing or non-finite re-check result rolls back: fail closed, never
    // keep billing on data that could not be measured.
    if !recheck_lower_bound.is_finite() || !original_lambda.is_finite() {
        return Recheck::Rollback;
    }
    let bounded = recheck_lower_bound.min(original_lambda);
    if bounded <= 0.0 {
        Recheck::Rollback
    } else {
        Recheck::Continue {
            billed_lambda: bounded,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn change(id: &str, lambda: f64) -> KeptChange {
        KeptChange {
            change_id: id.into(),
            lambda,
            rate_bp: 2000,
            kept_day: "2026-01-15".into(),
            overridden_day: None,
            pre_registration_hash: format!("pre-{id}"),
            result_hash: format!("result-{id}"),
        }
    }

    #[test]
    fn the_worked_example_matches_the_spec_table() {
        // $18,000 scope, lambda1 = 3%, lambda2 = 2%, rho = 20%:
        // F = 1.0506, R/F = $17,133.07, delta = $866.93,
        // log shares 59.9% / 40.1%, fees $103.83 / $69.56, total $173.39.
        let fees =
            monthly_fees(18_000_000_000, &[change("c1", 0.03), change("c2", 0.02)]).expect("fees");
        assert_eq!(fees.len(), 2);
        assert!(
            (fees[0].factor - 1.0506).abs() < 1e-12,
            "{}",
            fees[0].factor
        );
        assert_eq!(fees[0].incremental, 866_933_181);
        assert!(
            (fees[0].log_share - 0.598_823_87).abs() < 1e-9,
            "{}",
            fees[0].log_share
        );
        assert!(
            (fees[1].log_share - 0.401_176_13).abs() < 1e-9,
            "{}",
            fees[1].log_share
        );
        assert_eq!(fees[0].fee_micros, 103_828_056);
        assert_eq!(fees[1].fee_micros, 69_558_580);
        assert_eq!(
            fees.iter().map(|f| f.fee_micros).sum::<Micros>(),
            173_386_636
        );
        assert_eq!(fees[0].rate_bp, 2000);
        assert_eq!(fees[0].pre_registration_hash, "pre-c1");
        assert_eq!(fees[1].result_hash, "result-c2");
    }

    #[test]
    fn no_active_changes_means_no_fees_and_bad_lambdas_error() {
        assert!(monthly_fees(18_000_000_000, &[]).expect("empty").is_empty());
        assert!(monthly_fees(0, &[change("c1", 0.03)])
            .expect("zero")
            .is_empty());
        assert!(monthly_fees(-5, &[change("c1", 0.03)])
            .expect("negative")
            .is_empty());
        assert!(monthly_fees(18_000_000_000, &[change("c1", -1.5)]).is_err());
        assert!(monthly_fees(18_000_000_000, &[change("c1", f64::NAN)]).is_err());
    }

    #[test]
    fn term_ends_after_twelve_months_and_overrides_stop_billing() {
        let c = change("c1", 0.03);
        assert_eq!(c.billing_stop("2026-12-31").expect("day"), BillingStop::No);
        assert_eq!(c.billing_stop("2027-01-14").expect("day"), BillingStop::No);
        assert_eq!(
            c.billing_stop("2027-01-15").expect("day"),
            BillingStop::TermEnd,
            "the 12-month anniversary ends the term"
        );
        // Leap-day kept dates clamp to the target month's length.
        let mut leap = change("c2", 0.02);
        leap.kept_day = "2024-02-29".into();
        assert_eq!(
            leap.billing_stop("2025-02-27").expect("day"),
            BillingStop::No
        );
        assert_eq!(
            leap.billing_stop("2025-02-28").expect("day"),
            BillingStop::TermEnd
        );
        // An override stops billing on the override day, not before it.
        let mut overridden = c.clone();
        overridden.overridden_day = Some("2026-06-01".into());
        assert_eq!(
            overridden.billing_stop("2026-05-31").expect("day"),
            BillingStop::No
        );
        assert_eq!(
            overridden.billing_stop("2026-06-01").expect("day"),
            BillingStop::Override
        );
        // Malformed or impossible dates error instead of billing on.
        let mut broken = c.clone();
        broken.kept_day = "2026-13-05".into();
        assert!(broken.billing_stop("2099-01-01").is_err());
        let mut impossible = c.clone();
        impossible.kept_day = "2026-02-29".into();
        assert!(impossible.billing_stop("2099-01-01").is_err());
        assert!(c.billing_stop("garbage").is_err());
    }

    #[test]
    fn pro_rating_is_exact_and_clamped() {
        assert_eq!(pro_rate(1_000_000, 15, 30), 500_000);
        assert_eq!(pro_rate(1_000_000, 31, 30), 1_000_000);
        assert_eq!(pro_rate(1_000_000, 0, 30), 0);
        assert_eq!(pro_rate(1_000_000, 5, 0), 0);
    }

    #[test]
    fn rechecks_can_only_lower_and_zero_rolls_back() {
        assert_eq!(
            recheck(0.03, 0.05),
            Recheck::Continue {
                billed_lambda: 0.03
            },
            "a re-check never raises the billed lift"
        );
        assert_eq!(
            recheck(0.03, 0.012),
            Recheck::Continue {
                billed_lambda: 0.012
            }
        );
        assert_eq!(recheck(0.03, 0.0), Recheck::Rollback);
        assert_eq!(recheck(0.03, -0.01), Recheck::Rollback);
        assert_eq!(recheck(0.03, f64::NAN), Recheck::Rollback);
        assert_eq!(recheck(f64::NAN, 0.02), Recheck::Rollback);
    }
}
