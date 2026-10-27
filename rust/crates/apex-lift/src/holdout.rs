//! The new A3 holdout: a kept change runs to 90% of players while 10% stay
//! on the old setting. The monthly fee is 30% of the gain the holdout still
//! measures, never a 14-day snapshot. A confident harm rolls the change back
//! and a change bills for at most twelve months.

use std::collections::BTreeMap;

use apex_core::Micros;
use serde::{Deserialize, Serialize};

use crate::player_store::{player_key, PlayerDay, PlayerStoreError};

/// The share of players that receive a kept change.
pub const ROLLOUT_SHARE: f64 = 0.90;
/// The share left on the old setting as the holdout.
pub const HOLDOUT_SHARE: f64 = 0.10;
/// The results-fee rate in basis points (30%).
pub const HOLDOUT_RATE_BP: u16 = apex_core::RESULTS_RATE_BP;
/// The maximum billing term of one kept change.
pub const HOLDOUT_TERM_MONTHS: u32 = 12;
/// The 90% two-sided normal quantile used by the decision rule.
pub const Z90: f64 = 1.644_853_626_951_472_2;
/// The 99% two-sided normal quantile used by the day-7 harm check.
pub const Z99: f64 = 2.326_347_874_040_841;
/// The trailing measurement window in days.
pub const MEASUREMENT_DAYS: usize = 28;
/// The exposed share may drift this far from 90% before the split is called
/// out (ten percentage points).
pub const SPLIT_TOLERANCE: f64 = 0.10;

/// The durable state of one kept change's holdout, keyed by the immutable
/// experiment instance. It is written once when the change is kept and
/// carried on the signed ledger so a restart never opens a second holdout or
/// bills the same instance twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HoldoutStatus {
    /// The 90/10 holdout is live.
    Active,
    /// A confident harm (or a publisher stop) ended it; no later month bills.
    RolledBack,
}

/// One kept change's immutable holdout identity and state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldoutState {
    /// The immutable experiment instance this holdout belongs to.
    pub change_id: String,
    /// The platform experiment name the change was tested under.
    pub experiment_id: String,
    /// The pre-registered scope (ad unit).
    pub scope: String,
    /// The platform name (`applovin_max`, `levelplay`).
    pub platform: String,
    /// The experiment id the holdout feed is read under.
    pub holdout_experiment_id: String,
    /// The day the change was kept and the holdout opened.
    pub kept_day: String,
    /// The exposed share in basis points (9000).
    pub exposure_bp: u16,
    /// The old-setting share in basis points (1000).
    pub holdout_bp: u16,
    /// The results-fee rate in basis points (3000).
    pub rate_bp: u16,
    /// Whether the feed's `test` label is the exposed (kept-change) arm. On
    /// MAX the holdout experiment's test arm carries the old setting, so the
    /// exposed arm is the control side; on LevelPlay group B is exposed.
    pub exposed_is_test: bool,
    /// Current state.
    pub status: HoldoutStatus,
    /// The first day the holdout no longer bills, when it was rolled back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_day: Option<String>,
    /// The typed reason the holdout ended, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Signed ledger reference of the pre-registration.
    pub pre_registration_hash: String,
    /// Signed ledger reference of the keep result.
    pub result_hash: String,
}

impl HoldoutState {
    /// Open the 90/10 holdout for a kept change. `exposed_is_test` records
    /// which feed arm carries the kept change.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        change_id: &str,
        experiment_id: &str,
        scope: &str,
        platform: &str,
        kept_day: &str,
        exposed_is_test: bool,
        pre_registration_hash: &str,
        result_hash: &str,
    ) -> Self {
        Self {
            change_id: change_id.to_string(),
            experiment_id: experiment_id.to_string(),
            scope: scope.to_string(),
            platform: platform.to_string(),
            holdout_experiment_id: holdout_experiment_id(change_id),
            kept_day: kept_day.to_string(),
            exposure_bp: 9_000,
            holdout_bp: 1_000,
            rate_bp: HOLDOUT_RATE_BP,
            exposed_is_test,
            status: HoldoutStatus::Active,
            rollback_day: None,
            reason: None,
            pre_registration_hash: pre_registration_hash.to_string(),
            result_hash: result_hash.to_string(),
        }
    }

    /// End billing from `day` onward.
    pub fn roll_back(&mut self, day: &str, reason: &str) {
        if self.status == HoldoutStatus::RolledBack {
            // Keep the first rollback day: a replay must not move the stop
            // later and re-open a month that was already removed.
            return;
        }
        self.status = HoldoutStatus::RolledBack;
        self.rollback_day = Some(day.to_string());
        self.reason = Some(reason.to_string());
    }

    /// Whether the state is still billing.
    #[must_use]
    pub fn active(&self) -> bool {
        self.status == HoldoutStatus::Active
    }
}

/// The deterministic holdout experiment id for an immutable instance.
#[must_use]
pub fn holdout_experiment_id(change_id: &str) -> String {
    format!("{change_id}::holdout")
}

/// The exposed arm's share of the unique players, or `None` for no players.
#[must_use]
pub fn exposed_share(estimate: &HoldoutEstimate) -> Option<f64> {
    let total = estimate.exposed_players + estimate.control_players;
    (total > 0).then(|| estimate.exposed_players as f64 / total as f64)
}

/// Whether the observed unique-player split is within ten points of 90/10.
#[must_use]
pub fn split_is_90_10(estimate: &HoldoutEstimate) -> bool {
    exposed_share(estimate).is_some_and(|share| (share - ROLLOUT_SHARE).abs() <= SPLIT_TOLERANCE)
}

/// The 99% upper bound of the lift.
#[must_use]
pub fn upper_99(estimate: &HoldoutEstimate) -> f64 {
    estimate.lift + Z99 * estimate.standard_error
}

/// Whether the holdout is confidently harmful at 99% (the day-7 rule).
#[must_use]
pub fn confidently_harmful_99(estimate: &HoldoutEstimate) -> bool {
    upper_99(estimate) < 0.0
}

/// The harm check for an elapsed holdout day: day 7 uses 99%, day 14 and
/// later use 90%. Before day 7 there is no harm verdict.
#[must_use]
pub fn confidently_harmful_at(estimate: &HoldoutEstimate, elapsed_days: u32) -> bool {
    if elapsed_days < 7 {
        false
    } else if elapsed_days < 14 {
        confidently_harmful_99(estimate)
    } else {
        confidently_harmful(estimate)
    }
}

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

/// Fold a platform player feed into the per-player exposed/control revenue
/// the holdout estimator consumes. Repeated player-days are summed per
/// player; a player under both arms is a typed refusal. `exposed_is_test`
/// maps the feed's `test` label onto the exposed arm.
pub fn from_player_feed(
    rows: &[PlayerDay],
    exposed_is_test: bool,
    pretest: Option<&BTreeMap<String, Micros>>,
) -> Result<Vec<PlayerRevenue>, PlayerStoreError> {
    let mut players: BTreeMap<String, (bool, Micros)> = BTreeMap::new();
    for row in rows {
        let Some(key) = player_key(row) else {
            continue;
        };
        let exposed = row.test == exposed_is_test;
        match players.get_mut(&key) {
            Some((existing_exposed, revenue)) if *existing_exposed != exposed => {
                return Err(PlayerStoreError::CrossArm { player: key });
            }
            Some((_, revenue)) => *revenue = revenue.saturating_add(row.revenue_micros),
            None => {
                players.insert(key, (exposed, row.revenue_micros));
            }
        }
    }
    Ok(players
        .into_iter()
        .map(|(user, (exposed, revenue_micros))| PlayerRevenue {
            pre_revenue_micros: pretest.and_then(|values| values.get(&user).copied()),
            user,
            revenue_micros,
            exposed,
        })
        .collect())
}

/// Estimate the holdout directly from the platform's per-player feed.
pub fn estimate_from_feed(
    rows: &[PlayerDay],
    exposed_is_test: bool,
    pretest: Option<&BTreeMap<String, Micros>>,
) -> Result<Option<HoldoutEstimate>, PlayerStoreError> {
    let players = from_player_feed(rows, exposed_is_test, pretest)?;
    Ok(estimate(&players))
}

/// The monthly results fee on treated revenue. The incremental revenue is
/// `treated_revenue * (1 - 1/(1 + lift))`, the gain the exposed players still
/// show against the holdout; the fee is the 30% rate times that. Zero when
/// the measured lift is not positive.
#[must_use]
pub fn monthly_fee_on(treated_revenue: Micros, estimate: &HoldoutEstimate) -> Micros {
    if estimate.lift <= 0.0 || treated_revenue <= 0 {
        return 0;
    }
    let incremental = treated_revenue as f64 * (1.0 - 1.0 / (1.0 + estimate.lift));
    let fee = incremental * f64::from(HOLDOUT_RATE_BP) / 10_000.0;
    fee.round() as Micros
}

/// The monthly results fee on the exposed players' revenue carried by the
/// estimate. No fee when the measured lift is zero or below.
#[must_use]
pub fn monthly_fee(estimate: &HoldoutEstimate) -> Micros {
    monthly_fee_on(estimate.revenue_micros, estimate)
}

/// The scope-level fee when `scope_revenue` is the whole ad unit's revenue
/// (both the 90% exposed and the 10% holdout). The exposed share is applied
/// first, then the results rate.
#[must_use]
pub fn monthly_fee_on_scope(scope_revenue: Micros, estimate: &HoldoutEstimate) -> Micros {
    let treated = (scope_revenue as f64 * ROLLOUT_SHARE).round() as Micros;
    monthly_fee_on(treated, estimate)
}

/// One invoice line for a kept change's holdout month.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldoutInvoiceLine {
    pub measured_lift: f64,
    pub lower_90: f64,
    pub upper_90: f64,
    /// The treated (exposed) revenue the fee was applied to.
    pub revenue_micros: Micros,
    pub fee_micros: Micros,
    /// Signed ledger records behind the reading: pre-registration, keep
    /// result and holdout state.
    #[serde(default)]
    pub ledger_references: Vec<String>,
}

impl HoldoutInvoiceLine {
    /// The "no fee this month" note the invoice shows when the holdout
    /// measured no gain.
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
        // 6% lift on 318,000 exposed micros -> ~18,000 gain; 30% of it.
        let fee = monthly_fee(&estimate);
        assert!((4_000..5_500).contains(&fee), "{fee}");
        // The whole-scope helper applies the 90% exposed share exactly once.
        let scope_fee = monthly_fee_on_scope(353_333, &estimate);
        assert!((scope_fee - fee).abs() <= 1, "{scope_fee} vs {fee}");
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

    #[test]
    fn the_feed_maps_the_exposed_arm_and_sums_repeated_players() {
        let rows = vec![
            PlayerDay {
                day: "2026-10-01".into(),
                user_id: Some("e1".into()),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 60_000,
            },
            PlayerDay {
                day: "2026-10-02".into(),
                user_id: Some("e1".into()),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 50_000,
            },
            PlayerDay {
                day: "2026-10-01".into(),
                user_id: Some("e2".into()),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 110_000,
            },
            PlayerDay {
                day: "2026-10-01".into(),
                user_id: Some("c1".into()),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 100_000,
            },
            PlayerDay {
                day: "2026-10-01".into(),
                user_id: Some("c2".into()),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 100_000,
            },
        ];
        let players = from_player_feed(&rows, true, None).expect("feed");
        assert_eq!(players.len(), 4);
        assert_eq!(
            players
                .iter()
                .find(|player| player.user == "e1")
                .map(|player| player.revenue_micros),
            Some(110_000)
        );
        let estimate = estimate_from_feed(&rows, true, None)
            .expect("feed")
            .expect("estimate");
        assert!(estimate.lift > 0.0);
    }

    #[test]
    fn a_player_in_both_feed_arms_is_refused() {
        let rows = vec![
            PlayerDay {
                day: "2026-10-01".into(),
                user_id: Some("u1".into()),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 100,
            },
            PlayerDay {
                day: "2026-10-02".into(),
                user_id: Some("u1".into()),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 100,
            },
        ];
        assert_eq!(
            from_player_feed(&rows, true, None),
            Err(PlayerStoreError::CrossArm {
                player: "u1".into()
            })
        );
    }

    #[test]
    fn day_seven_uses_the_stricter_harm_bound() {
        let borderline = HoldoutEstimate {
            lift: -0.01,
            standard_error: 0.006,
            lower_90: -0.02,
            upper_90: 0.0,
            revenue_micros: 100_000,
            exposed_players: 90,
            control_players: 10,
        };
        // 90% upper is exactly zero (not harmful), 99% upper is positive.
        assert!(!confidently_harmful(&borderline));
        assert!(!confidently_harmful_at(&borderline, 7));
        assert!(!confidently_harmful_at(&borderline, 14));
        let harmful = HoldoutEstimate {
            lift: -0.05,
            standard_error: 0.01,
            lower_90: -0.07,
            upper_90: -0.03,
            revenue_micros: 100_000,
            exposed_players: 90,
            control_players: 10,
        };
        assert!(confidently_harmful_at(&harmful, 7));
        assert!(confidently_harmful_at(&harmful, 14));
        assert!(!confidently_harmful_at(&harmful, 6));
    }

    #[test]
    fn the_split_check_reports_ninety_ten() {
        let good = HoldoutEstimate {
            lift: 0.01,
            standard_error: 0.01,
            lower_90: 0.0,
            upper_90: 0.02,
            revenue_micros: 100,
            exposed_players: 90,
            control_players: 10,
        };
        assert!(split_is_90_10(&good));
        let bad = HoldoutEstimate {
            exposed_players: 50,
            control_players: 50,
            ..good
        };
        assert!(!split_is_90_10(&bad));
    }

    #[test]
    fn a_holdout_state_is_per_instance_and_rolls_back_once() {
        let mut state = HoldoutState::open(
            "inst-1",
            "floors-u1",
            "u1",
            "applovin_max",
            "2026-10-01",
            false,
            "pre",
            "result",
        );
        assert_eq!(state.holdout_experiment_id, "inst-1::holdout");
        assert_eq!(state.exposure_bp, 9_000);
        assert_eq!(state.holdout_bp, 1_000);
        assert_eq!(state.rate_bp, 3_000);
        assert!(state.active());
        state.roll_back("2026-11-01", "holdoutHarm");
        assert!(!state.active());
        assert_eq!(state.rollback_day.as_deref(), Some("2026-11-01"));
        state.roll_back("2026-12-01", "later");
        assert_eq!(
            state.rollback_day.as_deref(),
            Some("2026-11-01"),
            "a replay never moves the stop later"
        );
        let json = serde_json::to_string(&state).expect("json");
        let back: HoldoutState = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back, state);
    }
}
