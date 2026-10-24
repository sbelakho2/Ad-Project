//! The new A3 holdout: a kept change runs to 90% of players while 10% stay
//! on the old setting. The monthly fee is 30% of the gain the holdout still
//! measures, never a 14-day snapshot. A confident harm rolls the change back
//! and a change bills for at most twelve months.

use apex_core::Micros;

/// The share of players that receive a kept change.
pub const ROLLOUT_SHARE: f64 = 0.90;
/// The share left on the old setting as the holdout.
pub const HOLDOUT_SHARE: f64 = 0.10;
/// The results-fee rate in basis points (30%).
pub const HOLDOUT_RATE_BP: u16 = 3_000;
/// The maximum billing term of one kept change.
pub const HOLDOUT_TERM_MONTHS: u32 = 12;
/// The 90% two-sided normal quantile used by the decision rule.
pub const Z90: f64 = 1.644_853_626_951_472_2;

/// One player's revenue over the trailing holdout window.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerRevenue {
    pub user: String,
    /// Revenue while the kept change was live.
    pub revenue_micros: Micros,
    /// Pre-change revenue, the CUPED covariate when it is available.
    pub pre_revenue_micros: Option<Micros>,
    /// `true` for the exposed players, `false` for the old-setting holdout.
    pub exposed: bool,
}

/// The measured holdout estimate.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldoutEstimate {
    /// Estimated lift, `(exposed - control) / control`.
    pub lift: f64,
    /// Standard error of the lift.
    pub standard_error: f64,
    /// 90% lower bound.
    pub lower_90: f64,
    /// 90% upper bound.
    pub upper_90: f64,
    /// Revenue of the exposed players in the window.
    pub revenue_micros: Micros,
    /// Players in each group.
    pub exposed_players: usize,
    pub control_players: usize,
}

/// Estimate the holdout from per-player revenue. CUPED is applied when every
/// player in a group carries a pre-period value: the covariate's group gap is
/// removed before the exposed/control comparison.
#[must_use]
pub fn estimate(players: &[PlayerRevenue]) -> Option<HoldoutEstimate> {
    let exposed: Vec<&PlayerRevenue> = players.iter().filter(|p| p.exposed).collect();
    let control: Vec<&PlayerRevenue> = players.iter().filter(|p| !p.exposed).collect();
    if exposed.len() < 2 || control.len() < 2 {
        return None;
    }
    let common_pre = players
        .iter()
        .map(|p| p.pre_revenue_micros)
        .collect::<Option<Vec<_>>>()
        .map(|values| values.iter().map(|v| *v as f64).sum::<f64>() / values.len() as f64);
    let adjusted = |group: &[&PlayerRevenue]| -> (f64, Vec<f64>) {
        let mean_rev =
            group.iter().map(|p| p.revenue_micros as f64).sum::<f64>() / group.len() as f64;
        let values: Vec<f64> = group
            .iter()
            .map(|p| match (common_pre, p.pre_revenue_micros) {
                (Some(common), Some(pre)) => p.revenue_micros as f64 - (pre as f64 - common),
                _ => p.revenue_micros as f64,
            })
            .collect();
        (mean_rev, values)
    };
    let (_exposed_mean, exposed_values) = adjusted(&exposed);
    let (_control_mean, control_values) = adjusted(&control);
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len() as f64;
    let variance = |values: &[f64], mean: f64| {
        values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64
    };
    let exposed_mean = mean(&exposed_values);
    let control_mean = mean(&control_values);
    if control_mean <= 0.0 {
        return None;
    }
    let lift = (exposed_mean - control_mean) / control_mean;
    let exposed_var = variance(&exposed_values, exposed_mean);
    let control_var = variance(&control_values, control_mean);
    let standard_error =
        ((exposed_var / exposed.len() as f64 + control_var / control.len() as f64).sqrt())
            / control_mean;
    Some(HoldoutEstimate {
        lift,
        standard_error,
        lower_90: lift - Z90 * standard_error,
        upper_90: lift + Z90 * standard_error,
        revenue_micros: exposed.iter().map(|p| p.revenue_micros).sum::<Micros>(),
        exposed_players: exposed.len(),
        control_players: control.len(),
    })
}

/// Whether the holdout says the change is confidently harmful.
#[must_use]
pub fn confidently_harmful(estimate: &HoldoutEstimate) -> bool {
    estimate.upper_90 < 0.0
}

/// The monthly results fee. Incremental revenue is treated as
/// `revenue * (1 - 1/(1 + lift))`, the exposed players' incremental share,
/// then multiplied by the 90% rollout and the 30% rate. Zero when the
/// measured lift is not positive.
#[must_use]
pub fn monthly_fee(estimate: &HoldoutEstimate) -> Micros {
    if estimate.lift <= 0.0 || estimate.revenue_micros <= 0 {
        return 0;
    }
    let incremental = estimate.revenue_micros as f64 * (1.0 - 1.0 / (1.0 + estimate.lift));
    let fee = incremental * ROLLOUT_SHARE * f64::from(HOLDOUT_RATE_BP) / 10_000.0;
    fee.round() as Micros
}

/// One invoice line for a kept change's holdout month.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldoutInvoiceLine {
    pub measured_lift: f64,
    pub lower_90: f64,
    pub upper_90: f64,
    pub revenue_micros: Micros,
    pub fee_micros: Micros,
    pub ledger_references: Vec<String>,
}

impl HoldoutInvoiceLine {
    pub fn no_fee_note(&self) -> Option<&'static str> {
        (self.fee_micros == 0).then_some("No fee this month: the holdout measured no gain.")
    }
}

/// Build the invoice line H6 requires: the measured lift and its range, the
/// revenue it applies to, the fee, and the signed ledger references.
#[must_use]
pub fn invoice_line(
    estimate: &HoldoutEstimate,
    ledger_references: Vec<String>,
) -> HoldoutInvoiceLine {
    HoldoutInvoiceLine {
        measured_lift: estimate.lift,
        lower_90: estimate.lower_90,
        upper_90: estimate.upper_90,
        revenue_micros: estimate.revenue_micros,
        fee_micros: monthly_fee(estimate),
        ledger_references,
    }
}

/// Whether a kept change is still inside its twelve-month term.
#[must_use]
pub fn within_term(months_billed: u32) -> bool {
    months_billed < HOLDOUT_TERM_MONTHS
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    fn players(exposed_revenue: &[i64], control_revenue: &[i64]) -> Vec<PlayerRevenue> {
        let mut out = Vec::new();
        for (index, revenue) in exposed_revenue.iter().enumerate() {
            out.push(PlayerRevenue {
                user: format!("e{index}"),
                revenue_micros: *revenue,
                pre_revenue_micros: None,
                exposed: true,
            });
        }
        for (index, revenue) in control_revenue.iter().enumerate() {
            out.push(PlayerRevenue {
                user: format!("c{index}"),
                revenue_micros: *revenue,
                pre_revenue_micros: None,
                exposed: false,
            });
        }
        out
    }

    #[test]
    fn a_measured_gain_bills_thirty_percent_of_the_ninety_percent_gain() {
        let estimate = estimate(&players(
            &[105_000, 106_000, 107_000],
            &[100_000, 100_000, 100_000],
        ))
        .expect("estimate");
        assert!(estimate.lift > 0.05);
        assert!(!confidently_harmful(&estimate));
        // 6% lift on 318,000 exposed micros -> ~17,700 incremental;
        // 90% rollout * 30% rate -> ~4,779 micros.
        let fee = monthly_fee(&estimate);
        assert!((4_000..5_500).contains(&fee), "{fee}");
    }

    #[test]
    fn no_gain_and_confident_harm_bill_nothing() {
        let flat = estimate(&players(&[100_000, 100_000], &[100_000, 100_000])).expect("flat");
        assert_eq!(monthly_fee(&flat), 0);
        let harm = estimate(&players(
            &[50_000, 50_000, 50_000],
            &[100_000, 100_000, 100_000],
        ))
        .expect("harm");
        assert!(confidently_harmful(&harm));
        assert_eq!(monthly_fee(&harm), 0);
    }

    #[test]
    fn cuped_removes_a_pre_period_gap() {
        let mut rows = players(&[110_000, 110_000, 110_000], &[100_000, 100_000, 100_000]);
        for (index, row) in rows.iter_mut().enumerate() {
            row.pre_revenue_micros = Some(if index < 3 { 200_000 } else { 100_000 });
        }
        let estimate = estimate(&rows).expect("estimate");
        assert!(estimate.lift < 0.05, "{estimate:?}");
    }

    #[test]
    fn the_invoice_line_names_the_gain_range_revenue_and_ledger() {
        let measured = estimate(&players(
            &[105_000, 106_000, 107_000],
            &[100_000, 100_000, 100_000],
        ))
        .expect("estimate");
        let line = invoice_line(&measured, vec!["pre:aa".into(), "result:bb".into()]);
        assert!(line.measured_lift > 0.0);
        assert!(line.lower_90 < line.upper_90);
        assert_eq!(line.revenue_micros, measured.revenue_micros);
        assert!(line.fee_micros > 0);
        assert_eq!(line.ledger_references.len(), 2);
        assert!(line.no_fee_note().is_none());
        let flat = estimate(&players(&[100_000, 100_000], &[100_000, 100_000])).expect("flat");
        assert!(invoice_line(&flat, Vec::new()).no_fee_note().is_some());
    }

    #[test]
    fn the_term_is_twelve_months() {
        assert!(within_term(11));
        assert!(!within_term(12));
    }
}
