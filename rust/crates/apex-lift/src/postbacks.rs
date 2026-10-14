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
    let lift = mean_log.exp() - 1.0;
    if !(lift.is_finite() && lower_90.is_finite() && lower_99.is_finite() && upper_90.is_finite()) {
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
    })
}

/// The whole precise path in one call: impressions plus optional pre-test
/// user revenue, giving a clustered, CUPED-adjusted estimate.
pub fn lift_from_postbacks(
    impressions: &[Impression],
    pretest: Option<&BTreeMap<String, f64>>,
) -> Option<Estimate> {
    let (control, test) = user_day_values(impressions);
    match pretest {
        Some(pretest) => {
            // Aggregate post values per user for the covariate pairing.
            let mut post_by_user: BTreeMap<String, f64> = BTreeMap::new();
            for impression in impressions {
                *post_by_user.entry(impression.user_id.clone()).or_default() +=
                    impression.revenue as f64;
            }
            let adjusted = cuped_adjust(pretest, &post_by_user);
            let control_values: Vec<f64> = adjusted
                .iter()
                .filter(|(user, _)| impressions.iter().any(|i| i.user_id == **user && !i.test))
                .map(|(_, value)| *value)
                .collect();
            let test_values: Vec<f64> = adjusted
                .iter()
                .filter(|(user, _)| impressions.iter().any(|i| i.user_id == **user && i.test))
                .map(|(_, value)| *value)
                .collect();
            estimate_user_lift(&control_values, &test_values)
        }
        None => estimate_user_lift(&control, &test),
    }
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
