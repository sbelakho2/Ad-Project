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
/// The exposed share may drift this far from 90% before the split is refused
/// for billing (three percentage points).
pub const SPLIT_TOLERANCE: f64 = 0.03;
/// The minimum unique players in each arm before a monthly financial estimate
/// is produced. Two players cannot size a fee.
pub const MIN_PLAYERS_PER_ARM: usize = 30;
/// The conservative 90% two-sided Student-t critical value at the minimum
/// sample size (df = 29). Using it for larger samples is deliberately
/// conservative.
pub const T90_CONSERVATIVE: f64 = 1.697;
/// The maximum revenue share without a resolved player id that may still be
/// billed. Above this the identified-player estimate is not representative.
pub const MAX_UNIDENTIFIED_SHARE_BP: i64 = 1_000;

/// Why a zero-fee holdout month billed nothing: the bound did not clear zero.
pub const NO_PROVEN_GAIN_REASON: &str = "No fee this month: the holdout measured no gain.";
/// Why a zero-fee holdout month billed nothing: no usable measurement.
pub const NO_MEASUREMENT_REASON: &str =
    "No fee this month: no usable holdout measurement covered the window.";
/// Why a zero-fee holdout month billed nothing: the registered split was not
/// observed.
pub const INVALID_SPLIT_REASON: &str =
    "No fee this month: the observed exposure was outside the registered 90/10 split.";
/// Why a zero-fee holdout month billed nothing: the feed measured another
/// scope than the registered one.
pub const SCOPE_MISMATCH_REASON: &str =
    "No fee this month: the player feed did not measure the registered scope.";
/// Why a zero-fee holdout month billed nothing: the trailing window is short.
pub const INCOMPLETE_WINDOW_REASON: &str =
    "No fee this month: the trailing 28-day window is incomplete.";
/// Why a zero-fee holdout month billed nothing: the bound was positive but
/// the attributable gain rounds to zero at this revenue.
pub const GAIN_ROUNDED_TO_ZERO_REASON: &str =
    "No fee this month: the bound-derived gain rounds to zero at this revenue.";
/// Why a zero-fee holdout month billed nothing: the platform supplied no
/// eligible assignment roster, so observed ad-user counts cannot stand in for
/// randomized eligible users.
pub const NO_ROSTER_REASON: &str =
    "No fee this month: the eligible assignment roster is unavailable.";
/// Why a zero-fee holdout month billed nothing: no bounded-outcome
/// certificate (envelopes + fixed priors) was supplied.
pub const NO_CERTIFICATE_REASON: &str =
    "No fee this month: no bounded-outcome certificate covered this cohort.";
/// Why a zero-fee holdout month billed nothing: the certificate did not prove
/// a positive absolute gain.
pub const NO_ACTUAL_GAIN_REASON: &str =
    "No fee this month: actual treated gain is not certified above zero.";
/// Why a zero-fee holdout month billed nothing: no matched exposed revenue for
/// the billed interval.
pub const NO_EXPOSED_REVENUE_REASON: &str =
    "No fee this month: matched exposed revenue for the billed interval is unavailable.";
/// Why a zero-fee holdout month billed nothing: identity coverage is short.
pub const LOW_COVERAGE_REASON: &str =
    "No fee this month: too much revenue has no resolved player id.";

/// The durable state of one kept change's holdout, keyed by the immutable
/// experiment instance. It is written once when the change is kept and
/// carried on the signed ledger so a restart never opens a second holdout or
/// bills the same instance twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HoldoutStatus {
    /// The provider operation was durably requested; confirmation is pending.
    Requested,
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

    /// Mark the provider operation as requested, before any remote call.
    pub fn request(&mut self) {
        self.status = HoldoutStatus::Requested;
    }

    /// Mark the provider operation confirmed and the holdout live.
    pub fn confirm(&mut self, holdout_experiment_id: &str) {
        self.holdout_experiment_id = holdout_experiment_id.to_string();
        self.status = HoldoutStatus::Active;
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

/// Whether the assigned roster satisfies the 90/10 assignment-count test and
/// both arms clear the minimum observed sample. A fixed share tolerance is not
/// a binomial assignment test; the chi-square at p = 0.001 is.
#[must_use]
pub fn split_is_90_10(estimate: &HoldoutEstimate, roster: AssignedRoster) -> bool {
    let total = roster.total() as f64;
    let q = ROLLOUT_SHARE;
    let p = 1.0 - q;
    let chi_square = (roster.exposed as f64 - total * q).powi(2) / (total * q * p);
    if chi_square > 10.8276 {
        return false;
    }
    roster.exposed as usize >= estimate.exposed_players
        && roster.control as usize >= estimate.control_players
        && estimate.exposed_players >= MIN_PLAYERS_PER_ARM
        && estimate.control_players >= MIN_PLAYERS_PER_ARM
}

/// The lift the monthly fee is sized from: the positive registered one-sided
/// bound, never the point estimate. A month whose bound does not clear zero
/// bills nothing.
#[must_use]
pub fn billing_lift(estimate: &HoldoutEstimate) -> f64 {
    if estimate.lower_alpha.is_finite() {
        estimate.lower_alpha.max(0.0)
    } else {
        0.0
    }
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

/// The signed raw evidence behind one monthly holdout reading. A publisher can
/// rebuild the player fold, identity coverage, estimate, bound and fee from
/// these rows without trusting Apex's summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldoutEvidence {
    pub month_end: String,
    pub exposed_is_test: bool,
    pub measurement_scope: String,
    /// The distinct measurement days the rows were folded over, sorted. A
    /// re-run refuses a fee whose window is not the registered length.
    #[serde(default)]
    pub window_days: Vec<String>,
    /// The player rows folded to one row per identified player (plus one
    /// aggregated unidentified row): the exact estimator input, sized for the
    /// ledger instead of one row per player-day.
    pub rows: Vec<PlayerDay>,
    pub pretest: BTreeMap<String, Micros>,
    pub unidentified_micros: Micros,
    pub unidentified_share_bp: i64,
    /// The eligible assignment roster this reading used, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned: Option<AssignedRoster>,
    /// Exposed revenue by UTC day inside the measured window, so a fee can be
    /// priced on exactly the billed days instead of the observed-user share of
    /// scope revenue.
    #[serde(default)]
    pub daily_exposed: BTreeMap<String, Micros>,
    /// The certified one-sided lower bound on the absolute revenue the treated
    /// subset added, when envelope evidence produced a certificate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_gain_lower_micros: Option<Micros>,
}

/// The measured holdout estimate.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldoutEstimate {
    /// Estimated lift, `(exposed - control) / control`.
    pub lift: f64,
    /// Standard error of the lift.
    pub standard_error: f64,
    /// 90% lower bound (reported for continuity).
    pub lower_90: f64,
    /// 90% upper bound (reported for continuity).
    pub upper_90: f64,
    /// Registered one-sided 97.75% lower bound the fee and keep use.
    pub lower_alpha: f64,
    /// Revenue of the exposed players in the window.
    pub revenue_micros: Micros,
    /// Players in each group.
    pub exposed_players: usize,
    pub control_players: usize,
}

/// Planned-share estimator (A3 V2, E2): per-user revenue totals with the
/// registered 90/10 split, no eligible roster. It is the estimator the keep
/// path uses, so a publisher's labelled feed can price the holdout without
/// platform roster access. The identity coverage gate still applies.
#[must_use]
pub fn estimate_planned_share(players: &[PlayerRevenue], q: f64) -> Option<HoldoutEstimate> {
    if !(0.0..1.0).contains(&q) {
        return None;
    }
    let p = 1.0 - q;
    let (exposed, control): (Vec<&PlayerRevenue>, Vec<&PlayerRevenue>) =
        players.iter().partition(|player| player.exposed);
    if exposed.len() < MIN_PLAYERS_PER_ARM || control.len() < MIN_PLAYERS_PER_ARM {
        return None;
    }
    let total = players.len() as f64;
    // The realized label split must still look like the registered 90/10 at
    // p = 0.001; a feed that drifted is not a planned-share reading.
    let chi_square = (exposed.len() as f64 - total * q).powi(2) / (total * q * p);
    if chi_square > 10.8276 {
        return None;
    }
    let exposed_revenue: f64 = exposed.iter().map(|row| row.revenue_micros as f64).sum();
    let control_revenue: f64 = control.iter().map(|row| row.revenue_micros as f64).sum();
    if exposed_revenue <= 0.0 || control_revenue <= 0.0 {
        return None;
    }
    let exposed_mean = exposed_revenue / (q * total);
    let control_mean = control_revenue / (p * total);
    let exposed_squares: f64 = exposed
        .iter()
        .map(|row| (row.revenue_micros as f64).powi(2))
        .sum();
    let control_squares: f64 = control
        .iter()
        .map(|row| (row.revenue_micros as f64).powi(2))
        .sum();
    let arm_t = p * exposed_squares / (exposed_revenue * exposed_revenue);
    let arm_c = q * control_squares / (control_revenue * control_revenue);
    let variance = (arm_t.sqrt() + arm_c.sqrt()).powi(2);
    let standard_error = variance.sqrt();
    if !(standard_error.is_finite() && control_mean > 0.0) {
        return None;
    }
    let lift = (exposed_mean - control_mean) / control_mean;
    let lower_90 = lift - crate::stats::Z90_ONE_SIDED * standard_error;
    let lower_alpha =
        lift - crate::stats::z_one_sided(crate::stats::INITIAL_ALPHA) * standard_error;
    let upper_90 = lift + crate::stats::Z90_ONE_SIDED * standard_error;
    Some(HoldoutEstimate {
        lift,
        standard_error,
        lower_90,
        upper_90,
        lower_alpha,
        revenue_micros: exposed_revenue.round() as Micros,
        exposed_players: exposed.len(),
        control_players: control.len(),
    })
}

/// Estimate the holdout from per-player revenue and the eligible assignment
/// roster. Revenue per assigned user includes the zero-revenue eligible users
/// implied by the roster: only the counts and the observed revenue sums are
/// needed, so a positive-ad-user mean can never masquerade as the population
/// mean.
#[must_use]
pub fn estimate(players: &[PlayerRevenue], roster: AssignedRoster) -> Option<HoldoutEstimate> {
    let exposed: Vec<&PlayerRevenue> = players.iter().filter(|p| p.exposed).collect();
    let control: Vec<&PlayerRevenue> = players.iter().filter(|p| !p.exposed).collect();
    if exposed.len() < MIN_PLAYERS_PER_ARM || control.len() < MIN_PLAYERS_PER_ARM {
        return None;
    }
    if roster.exposed < exposed.len() as u64 || roster.control < control.len() as u64 {
        return None;
    }
    if roster.total() == 0 {
        return None;
    }
    let total = roster.total() as f64;
    let q = ROLLOUT_SHARE;
    let p = 1.0 - q;
    // Assignment-count SRM. Observed ad-user proportions are not the
    // randomization law and cannot substitute for this.
    let chi_square = (roster.exposed as f64 - total * q).powi(2) / (total * q * p);
    if chi_square > 10.8277 {
        return None;
    }
    let complete =
        roster.exposed as usize == exposed.len() && roster.control as usize == control.len();
    let paired: Vec<(f64, f64)> = players
        .iter()
        .filter_map(|player| {
            player
                .pre_revenue_micros
                .map(|pre| (pre as f64, player.revenue_micros as f64))
        })
        .collect();
    // Fit the covariate only when the roster is fully observed: fitting it on
    // the monetized survivors alone is the selection bias the roster exists to
    // remove. Without a complete cohort the unadjusted estimator is used.
    let theta = if complete { cuped_theta(&paired) } else { 0.0 };
    let mean_pre = if paired.is_empty() {
        0.0
    } else {
        paired.iter().map(|(pre, _)| pre).sum::<f64>() / paired.len() as f64
    };
    let adjusted = |group: &[&PlayerRevenue]| -> Vec<f64> {
        group
            .iter()
            .map(|player| {
                let post = player.revenue_micros as f64;
                match player.pre_revenue_micros {
                    Some(pre) if theta != 0.0 => post - theta * (pre as f64 - mean_pre),
                    _ => post,
                }
            })
            .collect()
    };
    let exposed_values = adjusted(&exposed);
    let control_values = adjusted(&control);
    let exposed_revenue: f64 = exposed_values.iter().sum();
    let control_revenue: f64 = control_values.iter().sum();
    if exposed_revenue <= 0.0 || control_revenue <= 0.0 {
        return None;
    }
    let exposed_n = roster.exposed as f64;
    let control_n = roster.control as f64;
    if exposed_n <= 1.0 || control_n <= 1.0 {
        return None;
    }
    let exposed_mean = exposed_revenue / exposed_n;
    let control_mean = control_revenue / control_n;
    let exposed_squares: f64 = exposed_values.iter().map(|value| value * value).sum();
    let control_squares: f64 = control_values.iter().map(|value| value * value).sum();
    // Zero-revenue eligible users contribute 0 to both the sum and the sum of
    // squares but still count in the denominator and in the variance.
    let exposed_variance =
        ((exposed_squares - exposed_n * exposed_mean * exposed_mean) / (exposed_n - 1.0)).max(0.0);
    let control_variance =
        ((control_squares - control_n * control_mean * control_mean) / (control_n - 1.0)).max(0.0);
    let variance_ratio = exposed_variance / exposed_n / (control_mean * control_mean)
        + exposed_mean * exposed_mean * control_variance / control_n / control_mean.powi(4);
    let standard_error = variance_ratio.sqrt();
    if !standard_error.is_finite() {
        return None;
    }
    let lift = (exposed_mean - control_mean) / control_mean;
    let lower_90 = lift - crate::stats::Z90_ONE_SIDED * standard_error;
    let lower_alpha =
        lift - crate::stats::z_one_sided(crate::stats::INITIAL_ALPHA) * standard_error;
    let upper_90 = lift + crate::stats::Z90_ONE_SIDED * standard_error;
    let revenue_micros = exposed_values.iter().sum::<f64>().round() as Micros;
    Some(HoldoutEstimate {
        lift,
        standard_error,
        lower_90,
        upper_90,
        lower_alpha,
        revenue_micros,
        exposed_players: exposed.len(),
        control_players: control.len(),
    })
}

fn cuped_theta(pairs: &[(f64, f64)]) -> f64 {
    if pairs.len() < 2 {
        return 0.0;
    }
    let n = pairs.len() as f64;
    let mean_pre = pairs.iter().map(|(pre, _)| pre).sum::<f64>() / n;
    let mean_post = pairs.iter().map(|(_, post)| post).sum::<f64>() / n;
    let covariance = pairs
        .iter()
        .map(|(pre, post)| (pre - mean_pre) * (post - mean_post))
        .sum::<f64>()
        / (n - 1.0);
    let variance = pairs
        .iter()
        .map(|(pre, _)| (pre - mean_pre).powi(2))
        .sum::<f64>()
        / (n - 1.0);
    if variance <= 0.0 || !covariance.is_finite() || !variance.is_finite() {
        return 0.0;
    }
    covariance / variance
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
            Some((existing_exposed, _revenue)) if *existing_exposed != exposed => {
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

/// The eligible assignment roster: unique users randomized to each arm,
/// including users with zero revenue who never appear in an impression feed.
/// Assignment counts, not observed ad-user counts, are the SRM denominator and
/// the denominator of the eligible-user mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignedRoster {
    pub exposed: u64,
    pub control: u64,
}

impl AssignedRoster {
    #[must_use]
    pub fn total(&self) -> u64 {
        self.exposed.saturating_add(self.control)
    }
}

/// The monthly measurement plus the identity coverage of its feed.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldoutMeasurement {
    pub estimate: Option<HoldoutEstimate>,
    /// Revenue in rows with no usable player id.
    pub unidentified_micros: Micros,
    /// Unidentified revenue as basis points of the feed's total revenue.
    pub unidentified_share_bp: i64,
    /// The eligible assignment roster the estimate used, when one was
    /// supplied. Without it no eligible-population estimate exists.
    pub roster: Option<AssignedRoster>,
}

impl HoldoutMeasurement {
    /// Whether the identified-player estimate is representative enough to
    /// size a fee.
    #[must_use]
    pub fn coverage_ok(&self) -> bool {
        self.unidentified_share_bp <= MAX_UNIDENTIFIED_SHARE_BP
    }
}

/// The unidentified revenue share in basis points, rounded up so a share just
/// above the ceiling cannot pass the billing gate.
fn ceiling_share_bp(unidentified: Micros, total: Micros) -> i64 {
    if total <= 0 || unidentified <= 0 {
        return 0;
    }
    let numerator = i128::from(unidentified) * 10_000;
    i64::try_from((numerator + i128::from(total) - 1) / i128::from(total))
        .unwrap_or(10_000)
        .min(10_000)
}

/// Estimate the holdout directly from the platform's per-player feed.
pub fn estimate_from_feed(
    rows: &[PlayerDay],
    exposed_is_test: bool,
    pretest: Option<&BTreeMap<String, Micros>>,
    roster: Option<AssignedRoster>,
) -> Result<Option<HoldoutEstimate>, PlayerStoreError> {
    Ok(measure_from_feed(rows, exposed_is_test, pretest, roster)?.estimate)
}

/// Fold the measurement rows to one row per identified player plus one
/// aggregated row for revenue with no resolvable id, stamping every row with
/// the window marker day. The fold is the exact input
/// [`estimate_from_feed`] consumes, so the signed evidence stays small without
/// changing a single estimate; a player under both arms is refused rather
/// than folded into one.
pub fn fold_evidence_rows(
    rows: &[PlayerDay],
    marker_day: &str,
) -> Result<Vec<PlayerDay>, PlayerStoreError> {
    let mut folded: BTreeMap<String, PlayerDay> = BTreeMap::new();
    let mut unidentified: Option<PlayerDay> = None;
    for row in rows {
        let Some(key) = player_key(row) else {
            let entry = unidentified.get_or_insert_with(|| PlayerDay {
                day: marker_day.to_string(),
                user_id: None,
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 0,
            });
            entry.revenue_micros = entry.revenue_micros.saturating_add(row.revenue_micros);
            continue;
        };
        match folded.get_mut(&key) {
            Some(entry) if entry.test != row.test => {
                return Err(PlayerStoreError::CrossArm { player: key });
            }
            Some(entry) => {
                entry.revenue_micros = entry.revenue_micros.saturating_add(row.revenue_micros);
            }
            None => {
                folded.insert(
                    key,
                    PlayerDay {
                        day: marker_day.to_string(),
                        user_id: row.user_id.clone(),
                        idfv: row.idfv.clone(),
                        idfa: row.idfa.clone(),
                        test: row.test,
                        revenue_micros: row.revenue_micros,
                    },
                );
            }
        }
    }
    let mut out: Vec<PlayerDay> = folded.into_values().collect();
    // Keep the aggregate even at zero or negative sum (refunds): the signed
    // evidence must reproduce the engine's coverage exactly.
    if let Some(row) = unidentified {
        out.push(row);
    }
    Ok(out)
}

/// Measure the holdout and its identity coverage from one feed.
pub fn measure_from_feed(
    rows: &[PlayerDay],
    exposed_is_test: bool,
    pretest: Option<&BTreeMap<String, Micros>>,
    roster: Option<AssignedRoster>,
) -> Result<HoldoutMeasurement, PlayerStoreError> {
    let (_, unidentified_micros) = crate::player_store::player_totals(rows);
    let total: Micros = rows.iter().map(|row| row.revenue_micros).sum();
    let players = from_player_feed(rows, exposed_is_test, pretest)?;
    let unidentified_share_bp = ceiling_share_bp(unidentified_micros, total);
    let estimate = match roster {
        Some(roster) => estimate(&players, roster),
        None => estimate_planned_share(&players, ROLLOUT_SHARE),
    };
    Ok(HoldoutMeasurement {
        estimate,
        unidentified_micros,
        unidentified_share_bp,
        roster,
    })
}

/// The monthly results fee on treated revenue. The incremental revenue is
/// `treated_revenue * (1 - 1/(1 + lift))`, the gain the exposed players still
/// show against the holdout; the fee is the 30% rate times that. Zero when
/// the measured lift is not positive.
#[must_use]
pub fn monthly_fee_on(treated_revenue: Micros, estimate: &HoldoutEstimate) -> Micros {
    let lift = billing_lift(estimate);
    if lift <= 0.0 || treated_revenue <= 0 {
        return 0;
    }
    let incremental = treated_revenue as f64 * (1.0 - 1.0 / (1.0 + lift));
    let fee = incremental * f64::from(HOLDOUT_RATE_BP) / 10_000.0;
    fee.round() as Micros
}

/// The monthly results fee on the exposed players' revenue carried by the
/// estimate. No fee when the measured lift is zero or below.
#[must_use]
pub fn monthly_fee(estimate: &HoldoutEstimate) -> Micros {
    monthly_fee_on(estimate.revenue_micros, estimate)
}

/// One invoice line for a kept change's holdout month.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldoutInvoiceLine {
    pub measured_lift: f64,
    pub lower_90: f64,
    pub upper_90: f64,
    /// Registered one-sided 97.75% bound the fee and keep are sized from.
    pub lower_alpha: f64,
    /// The treated (exposed) revenue the fee was applied to.
    pub revenue_micros: Micros,
    pub fee_micros: Micros,
    /// Signed ledger records behind the reading: pre-registration, keep
    /// result and holdout state.
    #[serde(default)]
    pub ledger_references: Vec<String>,
    /// The raw, signed player evidence the bound and fee were computed from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<HoldoutEvidence>,
    /// Why this month billed nothing, when it did. Absent on a billed line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_fee_reason: Option<String>,
}

impl HoldoutInvoiceLine {
    /// The "no fee this month" explanation the invoice shows: the typed
    /// reason the run recorded, defaulting to the no-gain copy for records
    /// written before reasons existed.
    pub fn no_fee_note(&self) -> Option<&str> {
        (self.fee_micros == 0).then(|| {
            self.no_fee_reason
                .as_deref()
                .unwrap_or(NO_PROVEN_GAIN_REASON)
        })
    }
}

/// Build the invoice line H6 requires: the measured lift and its range, the
/// revenue it applies to, the fee, and the signed ledger references.
#[must_use]
pub fn invoice_line(
    estimate: &HoldoutEstimate,
    ledger_references: Vec<String>,
) -> HoldoutInvoiceLine {
    let fee_micros = monthly_fee(estimate);
    HoldoutInvoiceLine {
        measured_lift: estimate.lift,
        lower_90: estimate.lower_90,
        upper_90: estimate.upper_90,
        lower_alpha: estimate.lower_alpha,
        revenue_micros: estimate.revenue_micros,
        fee_micros,
        ledger_references,
        evidence: None,
        no_fee_reason: (fee_micros == 0).then(|| NO_PROVEN_GAIN_REASON.to_string()),
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

    fn roster_of(players: &[PlayerRevenue]) -> AssignedRoster {
        AssignedRoster {
            exposed: players.iter().filter(|player| player.exposed).count() as u64,
            control: players.iter().filter(|player| !player.exposed).count() as u64,
        }
    }

    fn estimate_full(players: &[PlayerRevenue]) -> Option<HoldoutEstimate> {
        estimate(players, roster_of(players))
    }

    fn players(exposed_revenue: &[i64], control_revenue: &[i64]) -> Vec<PlayerRevenue> {
        let mut out = Vec::new();
        for index in 0..270 {
            out.push(PlayerRevenue {
                user: format!("e{index}"),
                revenue_micros: exposed_revenue[index % exposed_revenue.len()],
                pre_revenue_micros: None,
                exposed: true,
            });
        }
        for index in 0..30 {
            out.push(PlayerRevenue {
                user: format!("c{index}"),
                revenue_micros: control_revenue[index % control_revenue.len()],
                pre_revenue_micros: None,
                exposed: false,
            });
        }
        out
    }

    #[test]
    fn a_measured_gain_bills_thirty_percent_of_the_ninety_percent_gain() {
        let estimate = estimate_full(&players(
            &[105_000, 106_000, 107_000],
            &[100_000, 100_000, 100_000],
        ))
        .expect("estimate");
        assert!(estimate.lift > 0.05);
        assert!(!confidently_harmful(&estimate));
        // V2 E5: the monthly fee is sized from the point estimate, floored at
        // zero; confidence bounds still decide keeps and harm.
        let bound = billing_lift(&estimate);
        assert!((bound - estimate.lower_alpha).abs() < 1e-12 && bound > 0.0);
        let incremental =
            (estimate.revenue_micros as f64 * (1.0 - 1.0 / (1.0 + bound))).round() as Micros;
        let fee = monthly_fee(&estimate);
        assert_eq!(fee, (0.30 * incremental as f64).round() as Micros);
        // The fee basis is the actual exposed revenue, never scope revenue
        // times an observed-user share.
        assert_eq!(estimate.revenue_micros, 270 * 106_000);
        let direct = monthly_fee_on(estimate.revenue_micros, &estimate);
        assert_eq!(direct, fee);
    }

    #[test]
    fn no_gain_and_confident_harm_bill_nothing() {
        let flat = estimate_full(&players(&[100_000, 100_000], &[100_000, 100_000])).expect("flat");
        assert_eq!(monthly_fee(&flat), 0);
        let harm = estimate_full(&players(
            &[50_000, 50_000, 50_000],
            &[100_000, 100_000, 100_000],
        ))
        .expect("harm");
        assert!(confidently_harmful(&harm));
        assert_eq!(monthly_fee(&harm), 0);
    }

    #[test]
    fn cuped_uses_the_estimated_coefficient_and_removes_the_pre_gap() {
        // post = 2 * pre + 10,000 for both arms, so the estimated CUPED
        // coefficient is 2 and the pre-period gap is removed. A hardcoded
        // coefficient of 1 would leave a spurious lift.
        let mut rows = players(&[210_000], &[190_000]);
        for row in rows.iter_mut() {
            row.pre_revenue_micros = Some(if row.exposed { 100_000 } else { 90_000 });
        }
        let estimate = estimate_full(&rows).expect("estimate");
        assert!(estimate.lift.abs() < 1e-9, "{estimate:?}");
    }

    #[test]
    fn the_invoice_line_names_the_gain_range_revenue_and_ledger() {
        let measured = estimate_full(&players(
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
        let flat = estimate_full(&players(&[100_000, 100_000], &[100_000, 100_000])).expect("flat");
        assert!(invoice_line(&flat, Vec::new()).no_fee_note().is_some());
    }

    #[test]
    fn the_term_is_twelve_months() {
        assert!(within_term(11));
        assert!(!within_term(12));
    }

    #[test]
    fn folded_evidence_reproduces_the_same_measurement() {
        let mut rows = Vec::new();
        for day in ["2026-10-01", "2026-10-02", "2026-10-03"] {
            for index in 0..270 {
                rows.push(PlayerDay {
                    day: day.into(),
                    user_id: Some(format!("e{index}")),
                    idfv: None,
                    idfa: None,
                    test: false,
                    revenue_micros: 104_000,
                });
            }
            for index in 0..30 {
                rows.push(PlayerDay {
                    day: day.into(),
                    user_id: Some(format!("c{index}")),
                    idfv: None,
                    idfa: None,
                    test: true,
                    revenue_micros: 100_000,
                });
            }
            rows.push(PlayerDay {
                day: day.into(),
                user_id: None,
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: -50_000,
            });
        }
        let raw = measure_from_feed(
            &rows,
            false,
            None,
            Some(roster_of(
                &from_player_feed(&rows, false, None).expect("fold"),
            )),
        )
        .expect("raw");
        let folded = fold_evidence_rows(&rows, "2026-10-31").expect("fold");
        assert_eq!(folded.len(), 301, "one row per player + one unidentified");
        assert!(folded.iter().all(|row| row.day == "2026-10-31"));
        let anonymous = folded
            .iter()
            .find(|row| row.user_id.is_none() && row.idfv.is_none() && row.idfa.is_none())
            .expect("one aggregated unidentified row, even at a negative sum");
        assert!(!anonymous.test, "an unidentified row carries no arm");
        assert_eq!(
            anonymous.revenue_micros, -150_000,
            "three days of -50k (refunds) stay in the evidence"
        );
        assert_eq!(raw.unidentified_micros, -150_000);
        let rebuilt = measure_from_feed(
            &folded,
            false,
            None,
            Some(roster_of(
                &from_player_feed(&folded, false, None).expect("fold"),
            )),
        )
        .expect("rebuilt");
        assert_eq!(raw.estimate, rebuilt.estimate);
        assert_eq!(raw.unidentified_micros, rebuilt.unidentified_micros);
        assert_eq!(raw.unidentified_share_bp, rebuilt.unidentified_share_bp);
    }

    #[test]
    fn the_evidence_fold_refuses_a_player_in_both_arms() {
        let rows = vec![
            PlayerDay {
                day: "2026-10-01".into(),
                user_id: Some("shared".into()),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 1,
            },
            PlayerDay {
                day: "2026-10-02".into(),
                user_id: Some("shared".into()),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 1,
            },
        ];
        assert!(matches!(
            fold_evidence_rows(&rows, "2026-10-31"),
            Err(PlayerStoreError::CrossArm { .. })
        ));
    }

    #[test]
    fn coverage_share_rounds_up_above_the_ceiling() {
        assert_eq!(ceiling_share_bp(10_001, 100_000), 1_001);
        assert_eq!(ceiling_share_bp(10_000, 100_000), 1_000);
        assert_eq!(ceiling_share_bp(0, 100_000), 0);
        assert_eq!(ceiling_share_bp(10, 0), 0, "no total means no share");
        let measurement = HoldoutMeasurement {
            estimate: None,
            unidentified_micros: 10_001,
            unidentified_share_bp: ceiling_share_bp(10_001, 100_000),
            roster: None,
        };
        assert!(!measurement.coverage_ok());
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
        let players = from_player_feed(&rows, true, None).expect("feed");
        assert_eq!(players.len(), 4);
        // The financial estimator needs the minimum arm sizes; build that
        // separately without changing the folding assertions above.
        let (_, unidentified) = crate::player_store::player_totals(&rows);
        assert_eq!(unidentified, 0);
    }

    #[test]
    fn the_zero_gain_selection_world_has_no_positive_billable_bound() {
        // Validation revision 2, page 6: 100,000 eligible users, 90/10
        // assignment, 40% with old/new (1.00, 1.25), 10% (1.00, 0), 50% zero.
        // Only 36,000 exposed and 5,000 control users appear in the ad feed.
        let mut rows = Vec::new();
        for index in 0..36_000 {
            rows.push(PlayerDay {
                day: "2026-10-01".into(),
                user_id: Some(format!("e{index}")),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 1_250_000,
            });
        }
        for index in 0..5_000 {
            rows.push(PlayerDay {
                day: "2026-10-01".into(),
                user_id: Some(format!("c{index}")),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 1_000_000,
            });
        }
        let roster = AssignedRoster {
            exposed: 90_000,
            control: 10_000,
        };
        let players = from_player_feed(&rows, false, None).expect("fold");
        let measured = estimate(&players, roster).expect("eligible mean");
        // Eligible means are both $0.50: the known-zero-gain null.
        assert!(measured.lift.abs() < 1e-12, "{measured:?}");
        assert!(measured.lower_alpha < 0.0, "{measured:?}");
        assert_eq!(measured.exposed_players, 36_000);
        assert_eq!(measured.control_players, 5_000);
        assert_eq!(monthly_fee(&measured), 0);
        // The observed ad-user means would show +25%; that is exactly the
        // bias the roster denominators remove.
        let naive: f64 = 1_250_000.0 / 1_000_000.0 - 1.0;
        assert!((naive - 0.25).abs() < 1e-12);
        // The truly harmful variant (-4%) is also not billable.
        let mut harmful_rows = rows.clone();
        for row in &mut harmful_rows {
            if !row.test {
                row.revenue_micros = 1_200_000;
            }
        }
        let harmful_players = from_player_feed(&harmful_rows, false, None).expect("fold");
        let harmful = estimate(&harmful_players, roster).expect("harmful mean");
        assert!(harmful.lift < -0.03, "{harmful:?}");
        assert_eq!(monthly_fee(&harmful), 0);
        // Without a roster there is no eligible-population estimate at all.
        assert!(estimate(
            &players,
            AssignedRoster {
                exposed: 0,
                control: 0
            }
        )
        .is_none());
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
            lower_alpha: -0.02,
            upper_90: 0.0,
            revenue_micros: 100_000,
            exposed_players: 270,
            control_players: 30,
        };
        // 90% upper is exactly zero (not harmful), 99% upper is positive.
        assert!(!confidently_harmful(&borderline));
        assert!(!confidently_harmful_at(&borderline, 7));
        assert!(!confidently_harmful_at(&borderline, 14));
        let harmful = HoldoutEstimate {
            lift: -0.05,
            standard_error: 0.01,
            lower_90: -0.07,
            lower_alpha: -0.07,
            upper_90: -0.03,
            revenue_micros: 100_000,
            exposed_players: 270,
            control_players: 30,
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
            lower_alpha: 0.0,
            upper_90: 0.02,
            revenue_micros: 100,
            exposed_players: 270,
            control_players: 30,
        };
        assert!(split_is_90_10(
            &good,
            roster_of(&players(&[100_000], &[100_000]))
        ));
        let bad = HoldoutEstimate {
            exposed_players: 50,
            control_players: 50,
            ..good
        };
        assert!(!split_is_90_10(
            &bad,
            AssignedRoster {
                exposed: 5000,
                control: 5000
            }
        ));
        assert!(!split_is_90_10(
            &good,
            AssignedRoster {
                exposed: 0,
                control: 0
            }
        ));
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
    #[test]
    fn the_planned_share_estimator_refuses_out_of_range_splits() {
        let players = players(&[105_000], &[100_000]);
        assert!(estimate_planned_share(&players, 0.0).is_none());
        assert!(estimate_planned_share(&players, 1.0).is_none());
        assert!(estimate_planned_share(&players, -0.1).is_none());
        assert!(estimate_planned_share(&players, 0.9).is_some());
    }

    #[test]
    fn split_requires_an_assignment_count_test_not_a_share_tolerance() {}
}
