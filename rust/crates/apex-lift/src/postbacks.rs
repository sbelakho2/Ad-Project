//! The precise path: MAX S2S impression postbacks with user ids and revenue.
//! User-level values allow clustered variance and covariate variance reduction,
//! so the same bound needs fewer days. Everything here is deterministic.

use std::collections::BTreeMap;

use apex_core::Micros;

use crate::stats::{estimate, t_quantile, Estimate};

/// One impression-level record from the postback stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Impression {
    pub user_id: String,
    pub day: String,
    /// Whether the impression came from the test arm.
    pub test: bool,
    pub revenue: Micros,
}

/// Daily revenue per user, per arm: the unit of analysis for postbacks.
pub fn user_day_values(impressions: &[Impression]) -> (Vec<f64>, Vec<f64>) {
    let mut control: BTreeMap<(String, String), f64> = BTreeMap::new();
    let mut test: BTreeMap<(String, String), f64> = BTreeMap::new();
    for impression in impressions {
        let key = (impression.user_id.clone(), impression.day.clone());
        let bucket = if impression.test {
            test.entry(key).or_default()
        } else {
            control.entry(key).or_default()
        };
        *bucket += impression.revenue as f64;
    }
    (
        control.into_values().collect(),
        test.into_values().collect(),
    )
}

/// Sum each player over the whole window. A3 L2 replaced the player-day
/// unit with one value per player; the variance and CUPED covariate then
/// describe players, not repeated player-days.
pub fn user_window_values(impressions: &[Impression]) -> (Vec<f64>, Vec<f64>) {
    let mut control: BTreeMap<String, f64> = BTreeMap::new();
    let mut test: BTreeMap<String, f64> = BTreeMap::new();
    for impression in impressions {
        let bucket = if impression.test {
            &mut test
        } else {
            &mut control
        };
        *bucket.entry(impression.user_id.clone()).or_default() += impression.revenue as f64;
    }
    (
        control.into_values().collect(),
        test.into_values().collect(),
    )
}

/// Covariate adjustment: subtract the fraction of the pre-test covariate that
/// predicts the post value, with `theta = cov(pre, post) / var(pre)` over
/// users present on both sides. Users without a pre value pass through.
pub fn cuped_adjust(
    pretest: &BTreeMap<String, f64>,
    posttest: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    let pairs: Vec<(f64, f64)> = posttest
        .iter()
        .filter_map(|(user, post)| pretest.get(user).map(|pre| (*pre, *post)))
        .collect();
    if pairs.len() < 2 {
        return posttest.clone();
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
    if variance <= 0.0 {
        return posttest.clone();
    }
    let theta = covariance / variance;
    posttest
        .iter()
        .map(|(user, post)| {
            let adjusted = match pretest.get(user) {
                Some(pre) => post - theta * (pre - mean_pre),
                None => *post,
            };
            (user.clone(), adjusted)
        })
        .collect()
}

/// Lift from user-level arm values with the delta-method log-ratio standard
/// error, clustered by construction because each value is one user. The
/// degrees of freedom use the Welch-Satterthwaite approximation, so the
/// bounds stay honest when the arms have different sizes or spreads.
pub fn estimate_user_lift(control: &[f64], test: &[f64]) -> Option<Estimate> {
    if control.len() < 2 || test.len() < 2 {
        return None;
    }
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len() as f64;
    let variance = |values: &[f64], mean: f64| {
        values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() as f64 - 1.0)
    };
    let (mean_c, mean_t) = (mean(control), mean(test));
    if !(mean_c > 0.0 && mean_t > 0.0) {
        return None;
    }
    let (var_c, var_t) = (variance(control, mean_c), variance(test, mean_t));
    let term_c = var_c / (control.len() as f64 * mean_c * mean_c);
    let term_t = var_t / (test.len() as f64 * mean_t * mean_t);
    let se = (term_c + term_t).sqrt();
    if !se.is_finite() {
        return None;
    }
    let df = if term_c + term_t > 0.0 {
        ((term_c + term_t).powi(2)
            / (term_c.powi(2) / (control.len() as f64 - 1.0)
                + term_t.powi(2) / (test.len() as f64 - 1.0)))
            .floor() as usize
    } else {
        control.len().min(test.len()) - 1
    };
    let df = df.max(1);
    let mean_log = (mean_t / mean_c).ln();
    let t90 = t_quantile(df, false);
    let t99 = t_quantile(df, true);
    let lower_90 = (mean_log - t90 * se).exp() - 1.0;
    let lower_99 = (mean_log - t99 * se).exp() - 1.0;
    let upper_90 = (mean_log + t90 * se).exp() - 1.0;
    let lower_alpha =
        (mean_log - crate::stats::t_quantile_alpha(df, crate::stats::INITIAL_ALPHA) * se).exp()
            - 1.0;
    let lower_interim =
        (mean_log - crate::stats::t_quantile_alpha(df, crate::stats::INTERIM_ALPHA) * se).exp()
            - 1.0;
    let lift = mean_log.exp() - 1.0;
    if ![
        lift,
        lower_90,
        lower_99,
        upper_90,
        lower_alpha,
        lower_interim,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        return None;
    }
    Some(Estimate {
        days: control.len().min(test.len()),
        mean_log,
        lift,
        sd: se,
        lower_90,
        lower_99,
        upper_90,
        upper_99: (mean_log + t99 * se).exp() - 1.0,
        lower_alpha,
        lower_interim,
    })
}

/// The whole precise path in one call: impressions plus optional pre-test
/// user revenue, giving a clustered, CUPED-adjusted estimate.
pub fn lift_from_postbacks(
    impressions: &[Impression],
    pretest: Option<&BTreeMap<String, f64>>,
) -> Option<Estimate> {
    // L2/L1: the analysis unit is the whole user over the window, never a
    // player-day. Fold each arm's post revenue per user first; a user that
    // appears in both arms is contaminated and refuses (the arm cannot be
    // chosen silently or counted twice).
    let (control, test) = user_window_values(impressions);
    let post_by_user = per_user_arm_totals(impressions)?;
    let Some(pretest) = pretest else {
        return estimate_user_lift(&control, &test);
    };
    let post_values: BTreeMap<String, f64> = post_by_user
        .iter()
        .map(|(user, (_, value))| (user.clone(), *value))
        .collect();
    let adjusted = cuped_adjust(pretest, &post_values);
    let control_values: Vec<f64> = post_by_user
        .iter()
        .filter(|(_, (is_test, _))| !*is_test)
        .map(|(user, _)| adjusted.get(user).copied().unwrap_or(0.0))
        .collect();
    let test_values: Vec<f64> = post_by_user
        .iter()
        .filter(|(_, (is_test, _))| *is_test)
        .map(|(user, _)| adjusted.get(user).copied().unwrap_or(0.0))
        .collect();
    estimate_user_lift(&control_values, &test_values)
}

/// Fold impressions into one post value per user, carrying the arm. A user
/// observed in both arms returns `None`: the assignment is not trustworthy.
fn per_user_arm_totals(impressions: &[Impression]) -> Option<BTreeMap<String, (bool, f64)>> {
    let mut users: BTreeMap<String, (bool, f64)> = BTreeMap::new();
    for impression in impressions {
        match users.get_mut(&impression.user_id) {
            Some((test, total)) if *test == impression.test => {
                *total += impression.revenue as f64;
            }
            Some(_) => return None,
            None => {
                users.insert(
                    impression.user_id.clone(),
                    (impression.test, impression.revenue as f64),
                );
            }
        }
    }
    Some(users)
}

/// A ratio-series cross-check: the same impressions reduced to arm-day
/// values feed the standard estimator, when day-level arms are preferred.
pub fn arm_day_estimate(control: &[f64], test: &[f64]) -> Option<Estimate> {
    if control.len() != test.len() || control.len() < 2 {
        return None;
    }
    let ratios: Vec<f64> = control
        .iter()
        .zip(test)
        .filter_map(|(c, t)| {
            if *c > 0.0 && *t > 0.0 {
                Some((t / c).ln())
            } else {
                None
            }
        })
        .collect();
    estimate(&ratios)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn impressions() -> Vec<Impression> {
        let mut out = Vec::new();
        for day in 0..7 {
            for user in 0..20 {
                let test = user % 2 == 0;
                let revenue = if test { 1_060 } else { 1_000 };
                out.push(Impression {
                    user_id: format!("user-{user}"),
                    day: format!("2026-10-{day:02}"),
                    test,
                    revenue,
                });
            }
        }
        out
    }

    #[test]
    fn user_day_values_cluster_per_user_and_day() {
        let (control, test) = user_day_values(&impressions());
        // 20 users times 7 days, split by arm.
        assert_eq!(control.len(), 70);
        assert_eq!(test.len(), 70);
        assert!(control.iter().all(|v| (*v - 1_000.0).abs() < 1e-9));
        assert!(test.iter().all(|v| (*v - 1_060.0).abs() < 1e-9));
    }

    #[test]
    fn cuped_removes_the_covariate_signal() {
        let mut pretest = BTreeMap::new();
        let mut posttest = BTreeMap::new();
        for user in 0..50 {
            let pre = 100.0 + user as f64 * 2.0;
            pretest.insert(format!("u{user}"), pre);
            // Post is dominated by the pre value plus a flat effect.
            posttest.insert(format!("u{user}"), pre * 2.0 + 5.0);
        }
        let adjusted = cuped_adjust(&pretest, &posttest);
        // After adjustment every user sits near the same flat effect.
        let values: Vec<f64> = adjusted.values().copied().collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let spread = values
            .iter()
            .map(|v| (v - mean).abs())
            .fold(0.0f64, f64::max);
        assert!(spread < 1.0, "adjusted values still spread by {spread}");
    }

    #[test]
    fn the_clustered_estimator_bounds_a_real_effect() {
        let (control, test) = user_day_values(&impressions());
        let estimate = estimate_user_lift(&control, &test).expect("estimate");
        assert!(
            estimate.lift > 0.05 && estimate.lift < 0.07,
            "{}",
            estimate.lift
        );
        // A noiseless series bounds exactly at its point; a noisy one sits
        // strictly under it.
        assert!(estimate.lower_90 > 0.0 && estimate.lower_90 <= estimate.lift + 1e-12);
        assert_eq!(estimate.days, 70);

        let mut noisy_control = Vec::new();
        let mut noisy_test = Vec::new();
        for user in 0..40 {
            let jitter = if user % 2 == 0 { 40.0 } else { -40.0 };
            noisy_control.push(1_000.0 + jitter);
            noisy_test.push(1_060.0 + jitter);
        }
        let noisy = estimate_user_lift(&noisy_control, &noisy_test).expect("noisy");
        assert!(noisy.lower_90 > 0.0 && noisy.lower_90 < noisy.lift);
    }

    #[test]
    fn constant_pretests_and_missing_users_take_their_paths() {
        let mut flat_pre: BTreeMap<String, f64> = BTreeMap::new();
        let mut post: BTreeMap<String, f64> = BTreeMap::new();
        for user in 0..4 {
            flat_pre.insert(format!("u{user}"), 5.0);
            post.insert(format!("u{user}"), 10.0 + user as f64);
        }
        // Zero covariate variance: the adjustment is the identity.
        let adjusted = cuped_adjust(&flat_pre, &post);
        assert_eq!(adjusted.len(), 4);
        assert!((adjusted["u0"] - 10.0).abs() < 1e-9);
        // A single paired user cannot estimate theta and passes through.
        let mut one_pre = BTreeMap::new();
        one_pre.insert("u0".to_string(), 5.0);
        assert_eq!(cuped_adjust(&one_pre, &post)["u0"], post["u0"]);
        // Users absent from the pretest keep their raw value.
        let mut partial = flat_pre.clone();
        partial.remove("u3");
        assert_eq!(cuped_adjust(&partial, &post)["u3"], post["u3"]);
    }

    #[test]
    fn raw_day_series_cross_checks_through_the_standard_estimator() {
        let control = [100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0];
        let test = [106.0; 7];
        let estimate = arm_day_estimate(&control, &test).expect("estimate");
        assert!((estimate.lift - 0.06).abs() < 1e-9);
        assert!(arm_day_estimate(&control[..1], &test[..1]).is_none());
        assert!(arm_day_estimate(&control, &[0.0; 7]).is_none());
    }

    #[test]
    fn degenerate_inputs_refuse() {
        assert!(estimate_user_lift(&[1.0], &[1.0, 2.0]).is_none());
        assert!(estimate_user_lift(&[0.0, 0.0], &[1.0, 1.0]).is_none());
        assert!(lift_from_postbacks(&[], None).is_none());
    }

    #[test]
    fn cuped_adjusts_matched_users_and_passes_unmatched_ones_through() {
        let pretest: BTreeMap<String, f64> = [("a", 1.0), ("b", 2.0)]
            .into_iter()
            .map(|(user, value)| (user.to_string(), value))
            .collect();
        let posttest: BTreeMap<String, f64> = [("a", 2.0), ("b", 4.0), ("c", 99.0)]
            .into_iter()
            .map(|(user, value)| (user.to_string(), value))
            .collect();
        let adjusted = cuped_adjust(&pretest, &posttest);
        // theta = 2 over this pair set: a -> 2 - 2*(1 - 1.5) = 3.
        assert!((adjusted["a"] - 3.0).abs() < 1e-12, "{}", adjusted["a"]);
        assert!((adjusted["b"] - 3.0).abs() < 1e-12, "{}", adjusted["b"]);
        // The user with no pre value has no theta term and passes through.
        assert_eq!(adjusted["c"], 99.0);
    }

    #[test]
    fn overflowing_arm_statistics_fail_closed() {
        // The Welch standard error overflows to NaN: no estimate is emitted.
        assert!(estimate_user_lift(&[1e308, 0.0], &[1.0, 1.0]).is_none());
        // A finite pair whose ratio overflows the log-ratio also refuses.
        assert!(estimate_user_lift(&[1e-300, 1e-300], &[1e300, 1e300]).is_none());
    }

    #[test]
    fn repeated_users_across_days_fold_to_whole_user_values() {
        // Two test users and two control users, each seen on several days.
        // The decision path must use four whole-user values, not six
        // player-days, and the resulting estimates differ.
        let impressions = vec![
            Impression {
                user_id: "t1".into(),
                day: "2026-10-01".into(),
                test: true,
                revenue: 10,
            },
            Impression {
                user_id: "t1".into(),
                day: "2026-10-02".into(),
                test: true,
                revenue: 10,
            },
            Impression {
                user_id: "t1".into(),
                day: "2026-10-03".into(),
                test: true,
                revenue: 10,
            },
            Impression {
                user_id: "t1".into(),
                day: "2026-10-04".into(),
                test: true,
                revenue: 10,
            },
            Impression {
                user_id: "t1".into(),
                day: "2026-10-05".into(),
                test: true,
                revenue: 10,
            },
            Impression {
                user_id: "t2".into(),
                day: "2026-10-01".into(),
                test: true,
                revenue: 100,
            },
            Impression {
                user_id: "c1".into(),
                day: "2026-10-01".into(),
                test: false,
                revenue: 100,
            },
            Impression {
                user_id: "c2".into(),
                day: "2026-10-02".into(),
                test: false,
                revenue: 100,
            },
        ];
        let (control, test) = user_window_values(&impressions);
        assert_eq!(control.len(), 2);
        assert_eq!(test.len(), 2);
        let whole = estimate_user_lift(&control, &test).expect("whole user");
        let (control_days, test_days) = user_day_values(&impressions);
        let player_days = estimate_user_lift(&control_days, &test_days).expect("player days");
        assert_ne!(
            whole.lift, player_days.lift,
            "the units must not be conflated"
        );
        // Whole-user means: (50 + 100) / 2 = 75 against 100. Player-day
        // means: (5*10 + 100) / 6 = 25 against 100. Folding first matters.
        assert!(
            (whole.lift - (75.0 / 100.0 - 1.0)).abs() < 1e-12,
            "{}",
            whole.lift
        );
        assert!((player_days.lift - (25.0 / 100.0 - 1.0)).abs() < 1e-12);
    }

    #[test]
    fn a_user_in_both_arms_refuses_the_whole_user_path() {
        let impressions = vec![
            Impression {
                user_id: "u1".into(),
                day: "2026-10-01".into(),
                test: true,
                revenue: 10,
            },
            Impression {
                user_id: "u1".into(),
                day: "2026-10-02".into(),
                test: false,
                revenue: 10,
            },
            Impression {
                user_id: "u2".into(),
                day: "2026-10-01".into(),
                test: true,
                revenue: 10,
            },
            Impression {
                user_id: "u3".into(),
                day: "2026-10-01".into(),
                test: false,
                revenue: 10,
            },
        ];
        assert!(lift_from_postbacks(&impressions, None).is_none());
        assert!(per_user_arm_totals(&impressions).is_none());
    }

    #[test]
    fn the_precise_path_runs_end_to_end() {
        let estimate = lift_from_postbacks(&impressions(), None).expect("estimate");
        assert!(estimate.lower_90 > 0.0);
        // With a covariate pretest the estimate still lands near the effect.
        let mut pretest = BTreeMap::new();
        for user in 0..20 {
            pretest.insert(format!("user-{user}"), 1_000.0);
        }
        let adjusted = lift_from_postbacks(&impressions(), Some(&pretest)).expect("cuped");
        assert!(adjusted.lift > 0.0);
    }
}
