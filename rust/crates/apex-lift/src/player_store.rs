//! A3 D4: the player store for per-player revenue and test group.
//!
//! Ids are resolved in one order: User ID, IDFV, IDFA. A zeroed IDFA is
//! missing, never a player called "00000000-0000-0000-0000-000000000000".

use std::collections::BTreeMap;

use apex_core::Micros;

/// One player-day observation from an impression or revenue feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerDay {
    pub day: String,
    pub user_id: Option<String>,
    pub idfv: Option<String>,
    pub idfa: Option<String>,
    pub test: bool,
    pub revenue_micros: Micros,
}

/// The id chosen for a row, or `None` when every id is missing.
#[must_use]
pub fn player_key(row: &PlayerDay) -> Option<String> {
    fn valid(id: &Option<String>) -> Option<&str> {
        id.as_deref().filter(|value| {
            !value.is_empty()
                && *value != "00000000-0000-0000-0000-000000000000"
                && *value != "00000000000000000000000000000000"
        })
    }
    valid(&row.user_id)
        .or_else(|| valid(&row.idfv))
        .or_else(|| valid(&row.idfa))
        .map(str::to_string)
}

/// Sum revenue per resolved player. Rows with no usable id are returned
/// separately so the caller can report the uncovered revenue share.
#[must_use]
pub fn player_totals(rows: &[PlayerDay]) -> (BTreeMap<String, Micros>, Micros) {
    let mut players = BTreeMap::new();
    let mut unidentified = 0;
    for row in rows {
        match player_key(row) {
            Some(key) => *players.entry(key).or_default() += row.revenue_micros,
            None => unidentified += row.revenue_micros,
        }
    }
    (players, unidentified)
}

/// The share of revenue with no id in basis points.
#[must_use]
pub fn unidentified_share_bp(unidentified: Micros, total: Micros) -> i64 {
    if total <= 0 || unidentified <= 0 {
        return 0;
    }
    ((i128::from(unidentified) * 10_000) / i128::from(total)).min(10_000) as i64
}

/// The MAX user-revenue history window: 45 days per player.
pub const MAX_PLAYER_HISTORY_DAYS: usize = 45;
/// The LevelPlay impression feed window.
pub const LEVELPLAY_IMPRESSION_HISTORY_DAYS: usize = 14;

/// An in-memory player feed that de-duplicates a day's re-pull and retains the
/// documented history windows. Keys are `(day, arm, player)`; a later pull of
/// the same key replaces the earlier row (the source is authoritative), while
/// different days stay separate so repeated users fold in the estimator.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerFeed {
    rows: BTreeMap<(String, bool, String), PlayerDay>,
}

impl PlayerFeed {
    /// Ingest one daily pull. Unresolved rows use a stable raw-id key so two
    /// anonymous players are not merged.
    pub fn ingest(&mut self, rows: &[PlayerDay]) {
        for row in rows {
            self.rows
                .insert((row.day.clone(), row.test, feed_key(row)), row.clone());
        }
    }

    /// Keep only the most recent `days` distinct UTC days (the feed's retention
    /// window). Returns the number of rows dropped.
    pub fn retain_last_days(&mut self, days: usize) -> usize {
        let mut distinct: Vec<String> = self.rows.keys().map(|(day, _, _)| day.clone()).collect();
        distinct.sort();
        distinct.dedup();
        if distinct.len() <= days {
            return 0;
        }
        let keep_from = distinct.len() - days;
        let cutoff = distinct[keep_from].clone();
        let before = self.rows.len();
        self.rows.retain(|(day, _, _), _| *day >= cutoff);
        before - self.rows.len()
    }

    /// The retained player-days in day/arm/player order.
    #[must_use]
    pub fn rows(&self) -> Vec<PlayerDay> {
        self.rows.values().cloned().collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

fn feed_key(row: &PlayerDay) -> String {
    player_key(row).unwrap_or_else(|| {
        format!(
            "anonymous:{}:{}",
            row.idfv.as_deref().unwrap_or(""),
            row.idfa.as_deref().unwrap_or("")
        )
    })
}

/// A player appearing in both arms is contaminated evidence: the same person
/// cannot be randomized into two arms. Analysis refuses rather than choosing
/// one arm (which would silently change the split) or counting the player
/// twice (which would understate the variance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerStoreError {
    /// The same resolved player id appeared under both arm labels.
    CrossArm { player: String },
}

impl std::fmt::Display for PlayerStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CrossArm { player } => write!(
                f,
                "player '{player}' appears in both the control and test arms"
            ),
        }
    }
}

impl std::error::Error for PlayerStoreError {}

/// One arm's unique players and their window totals.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerWindow {
    /// Post-period revenue per unique test player, summed over the window.
    pub test: Vec<f64>,
    /// Post-period revenue per unique control player.
    pub control: Vec<f64>,
    /// The same values after per-player CUPED, when a pre-period covariate
    /// was supplied; otherwise identical to `test`/`control`.
    pub cuped_test: Vec<f64>,
    pub cuped_control: Vec<f64>,
    /// Number of unique players in each arm (the split check's denominator).
    pub test_players: u64,
    pub control_players: u64,
    /// Revenue that had no usable id, and its share in basis points.
    pub unidentified_micros: Micros,
    pub unidentified_share_bp: i64,
    /// The CUPED coefficient actually applied (0 when no covariate).
    pub cuped_theta: f64,
}

/// Fold repeated player-days into one value per unique player and compute the
/// split counts the guardrail uses.
///
/// Repeated rows for one player **in the same arm** are summed (the whole-user
/// value), never counted as separate players. A player under both arm labels is
/// a typed refusal. Rows with no usable id are excluded from the player
/// analysis and reported separately.
pub fn player_window(
    rows: &[PlayerDay],
    pretest: Option<&BTreeMap<String, Micros>>,
) -> Result<PlayerWindow, PlayerStoreError> {
    let mut arms: BTreeMap<String, (bool, Micros)> = BTreeMap::new();
    let mut unidentified = 0;
    for row in rows {
        let Some(key) = player_key(row) else {
            unidentified += row.revenue_micros;
            continue;
        };
        match arms.get(&key) {
            Some((existing_test, _)) if *existing_test != row.test => {
                return Err(PlayerStoreError::CrossArm { player: key });
            }
            Some((_, total)) => {
                let updated = total.saturating_add(row.revenue_micros);
                if let Some(value) = arms.get_mut(&key) {
                    value.1 = updated;
                }
            }
            None => {
                arms.insert(key, (row.test, row.revenue_micros));
            }
        }
    }

    let mut test = Vec::new();
    let mut control = Vec::new();
    let mut pretest_by_player: BTreeMap<String, f64> = BTreeMap::new();
    for (player, (is_test, revenue)) in &arms {
        let value = *revenue as f64;
        if *is_test {
            test.push(value);
        } else {
            control.push(value);
        }
        if let Some(pretest) = pretest {
            if let Some(pre) = pretest.get(player) {
                pretest_by_player.insert(player.clone(), *pre as f64);
            }
        }
    }

    // CUPED: theta is the pooled covariance / variance over paired players,
    // then applied to each arm's post value. Players without a pre value pass
    // through unchanged, so no arm silently loses players.
    let cuped_theta = if pretest.is_some() {
        cuped_theta(&arms, &pretest_by_player)
    } else {
        0.0
    };
    let adjust = |values: &[(String, bool, Micros)]| -> Vec<f64> {
        values
            .iter()
            .map(|(player, _, revenue)| {
                let post = *revenue as f64;
                match pretest_by_player.get(player) {
                    Some(pre) if cuped_theta != 0.0 => {
                        post - cuped_theta * (pre - cuped_mean(&pretest_by_player))
                    }
                    _ => post,
                }
            })
            .collect()
    };
    let ordered: Vec<(String, bool, Micros)> = arms
        .iter()
        .map(|(player, (is_test, revenue))| (player.clone(), *is_test, *revenue))
        .collect();
    let cuped_values = adjust(&ordered);
    let mut cuped_test = Vec::new();
    let mut cuped_control = Vec::new();
    for ((_, is_test, _), value) in ordered.iter().zip(cuped_values) {
        if *is_test {
            cuped_test.push(value);
        } else {
            cuped_control.push(value);
        }
    }
    let test_players = test.len() as u64;
    let control_players = control.len() as u64;
    let total: Micros = arms.values().map(|(_, revenue)| *revenue).sum();
    let all_revenue = total.saturating_add(unidentified);
    Ok(PlayerWindow {
        test,
        control,
        cuped_test,
        cuped_control,
        test_players,
        control_players,
        unidentified_micros: unidentified,
        unidentified_share_bp: unidentified_share_bp(unidentified, all_revenue),
        cuped_theta,
    })
}

fn cuped_mean(values: &BTreeMap<String, f64>) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.values().sum::<f64>() / values.len() as f64
}

fn cuped_theta(arms: &BTreeMap<String, (bool, Micros)>, pretest: &BTreeMap<String, f64>) -> f64 {
    let pairs: Vec<(f64, f64)> = arms
        .iter()
        .filter_map(|(player, (_, revenue))| pretest.get(player).map(|pre| (*pre, *revenue as f64)))
        .collect();
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

/// The per-player Horvitz–Thompson estimate over a folded window.
pub fn estimate_window(
    window: &PlayerWindow,
    test_probability: f64,
    cuped: bool,
) -> Option<crate::stats::PlayerHtEstimate> {
    if cuped {
        crate::stats::horvitz_thompson_estimate(
            &window.cuped_control,
            &window.cuped_test,
            test_probability,
        )
    } else {
        crate::stats::horvitz_thompson_estimate(&window.control, &window.test, test_probability)
    }
}

/// Unique-player split counts from persisted player rows: the L3 input, never
/// summed player-days.
pub fn unique_arm_counts(rows: &[PlayerDay]) -> Result<(u64, u64), PlayerStoreError> {
    let window = player_window(rows, None)?;
    Ok((window.test_players, window.control_players))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn row(
        user_id: Option<&str>,
        idfv: Option<&str>,
        idfa: Option<&str>,
        revenue: i64,
    ) -> PlayerDay {
        PlayerDay {
            day: "2026-10-01".into(),
            user_id: user_id.map(str::to_string),
            idfv: idfv.map(str::to_string),
            idfa: idfa.map(str::to_string),
            test: true,
            revenue_micros: revenue,
        }
    }

    #[test]
    fn ids_fall_back_in_the_documented_order() {
        assert_eq!(
            player_key(&row(Some("u"), Some("v"), Some("a"), 1)),
            Some("u".into())
        );
        assert_eq!(
            player_key(&row(None, Some("v"), Some("a"), 1)),
            Some("v".into())
        );
        assert_eq!(player_key(&row(None, None, Some("a"), 1)), Some("a".into()));
        assert_eq!(
            player_key(&row(
                None,
                None,
                Some("00000000-0000-0000-0000-000000000000"),
                1
            )),
            None
        );
    }

    #[test]
    fn revenue_sums_per_player_and_reports_the_uncovered_share() {
        let rows = vec![
            row(Some("u1"), None, None, 100),
            row(Some("u1"), None, None, 50),
            row(None, None, None, 50),
        ];
        let (players, unidentified) = player_totals(&rows);
        assert_eq!(players.get("u1"), Some(&150));
        assert_eq!(unidentified, 50);
        assert_eq!(unidentified_share_bp(50, 200), 2_500);
    }
    fn window_row(day: &str, user: &str, test: bool, revenue: Micros) -> PlayerDay {
        PlayerDay {
            day: day.into(),
            user_id: Some(user.into()),
            idfv: None,
            idfa: None,
            test,
            revenue_micros: revenue,
        }
    }

    #[test]
    fn repeated_players_across_days_are_one_player_and_one_summed_value() {
        let rows = vec![
            window_row("2026-10-01", "u1", true, 100),
            window_row("2026-10-02", "u1", true, 50),
            window_row("2026-10-03", "u1", true, 25),
            window_row("2026-10-01", "u2", true, 10),
            window_row("2026-10-01", "c1", false, 90),
            window_row("2026-10-02", "c1", false, 10),
            window_row("2026-10-03", "c2", false, 50),
        ];
        let window = player_window(&rows, None).expect("window");
        assert_eq!(window.test_players, 2, "three u1 days are one player");
        assert_eq!(window.control_players, 2);
        assert_eq!(window.test, vec![175.0, 10.0]);
        assert_eq!(window.control, vec![100.0, 50.0]);
        // The split check sees unique players, not seven player-days.
        assert_eq!(unique_arm_counts(&rows).expect("counts"), (2, 2));
    }

    #[test]
    fn a_player_in_both_arms_is_refused_not_double_counted() {
        let rows = vec![
            window_row("2026-10-01", "u1", true, 100),
            window_row("2026-10-02", "u1", false, 100),
        ];
        assert_eq!(
            player_window(&rows, None),
            Err(PlayerStoreError::CrossArm {
                player: "u1".into()
            })
        );
        assert!(unique_arm_counts(&rows).is_err());
    }

    #[test]
    fn cuped_uses_each_players_pretest_revenue() {
        let rows = vec![
            window_row("2026-10-01", "t1", true, 120),
            window_row("2026-10-01", "t2", true, 140),
            window_row("2026-10-01", "c1", false, 100),
            window_row("2026-10-01", "c2", false, 120),
        ];
        let mut pretest = BTreeMap::new();
        // Every player's post revenue is exactly pre + 20, so CUPED should
        // remove the pre-period spread completely.
        for (player, pre) in [("t1", 100), ("t2", 120), ("c1", 80), ("c2", 100)] {
            pretest.insert(player.to_string(), pre);
        }
        let window = player_window(&rows, Some(&pretest)).expect("window");
        assert!(
            (window.cuped_theta - 1.0).abs() < 1e-12,
            "{}",
            window.cuped_theta
        );
        let estimate = estimate_window(&window, 0.5, true).expect("estimate");
        assert!(estimate.estimate.lift.abs() < 1e-12, "{estimate:?}");
        // Without CUPED the observed arms differ, so the raw estimate is not
        // silently replaced by the adjusted one.
        let raw = estimate_window(&window, 0.5, false).expect("raw");
        assert!((raw.estimate.lift - (260.0 / 220.0 - 1.0)).abs() < 1e-12);
    }

    #[test]
    fn unidentified_revenue_is_reported_outside_the_player_analysis() {
        let rows = vec![
            window_row("2026-10-01", "u1", true, 100),
            PlayerDay {
                day: "2026-10-01".into(),
                user_id: None,
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 25,
            },
        ];
        let window = player_window(&rows, None).expect("window");
        assert_eq!(window.test_players, 1);
        assert_eq!(window.unidentified_micros, 25);
        assert_eq!(window.unidentified_share_bp, 2_000);
    }

    #[test]
    fn player_feed_dedupes_a_reread_and_retains_the_documented_windows() {
        let mut feed = PlayerFeed::default();
        let mut rows = Vec::new();
        // 51 distinct days exercise the 45-day retention boundary.
        for day in 1..=31 {
            rows.push(window_row(&format!("2026-01-{day:02}"), "u1", true, 10));
        }
        for day in 1..=20 {
            rows.push(window_row(&format!("2026-02-{day:02}"), "u1", true, 10));
        }
        feed.ingest(&rows);
        assert_eq!(feed.len(), 51);
        // A re-pull of one day replaces, never duplicates.
        feed.ingest(&[window_row("2026-02-20", "u1", true, 99)]);
        assert_eq!(feed.len(), 51);
        assert_eq!(
            feed.rows()
                .iter()
                .find(|row| row.day == "2026-02-20")
                .map(|row| row.revenue_micros),
            Some(99)
        );
        assert_eq!(feed.retain_last_days(MAX_PLAYER_HISTORY_DAYS), 6);
        assert_eq!(feed.len(), 45);
        assert!(feed
            .rows()
            .iter()
            .all(|row| row.day.as_str() >= "2026-01-07"));
        let mut short = PlayerFeed::default();
        short.ingest(&rows[..3]);
        assert_eq!(short.retain_last_days(LEVELPLAY_IMPRESSION_HISTORY_DAYS), 0);
    }
}
