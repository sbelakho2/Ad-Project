//! The estimator from the spec: lift is the mean daily log ratio of the arms'
//! revenue per user, bounded with a one-sided t quantile, cross-checked with a
//! block bootstrap, and planned with the duration formula.

use crate::rng::SplitMix64;

/// The registered one-sided error for one final initial-keep look. The
/// planned family is interim 0.005 + planned-final 0.0225 + extension-final
/// 0.0225 = 0.05; a normal final look spends only the first two.
pub const INITIAL_ALPHA: f64 = 0.0225;
/// The registered one-sided error for the single interim look.
pub const INTERIM_ALPHA: f64 = 0.005;
/// The estimator identity pinned in pre-registration and reports.
pub const ESTIMATOR_ID: &str = "planned_share_total_v2";

/// Which one-sided bound a report shows. Decisions use the registered
/// explicit-alpha bounds in [`Estimate`] (`lower_alpha` / `lower_interim`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Lower90,
    Lower99,
    Upper90,
}

/// One-sided Student-t quantile, computed from the regularized incomplete
/// beta function with a bounded bisection. Exact for every positive df, so no
/// table can be wrong at the edges and public `estimate` calls beyond the
/// window sizes stay correct.
pub fn t_quantile(df: usize, level_99: bool) -> f64 {
    if df == 0 {
        return f64::INFINITY;
    }
    let target = if level_99 { 0.99 } else { 0.90 };
    let (mut low, mut high) = (0.0f64, 64.0f64);
    for _ in 0..80 {
        let mid = (low + high) / 2.0;
        if t_cdf(mid, df) < target {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

/// One-sided Student-t quantile at an explicit tail probability.
pub fn t_quantile_alpha(df: usize, alpha: f64) -> f64 {
    if df == 0 {
        return f64::INFINITY;
    }
    if !(0.0..0.5).contains(&alpha) {
        return f64::NAN;
    }
    let target = 1.0 - alpha;
    let (mut low, mut high) = (0.0f64, 128.0f64);
    for _ in 0..96 {
        let mid = (low + high) / 2.0;
        if t_cdf(mid, df) < target {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

/// One-sided normal quantile for an explicit tail probability: the
/// large-sample bound the per-player estimator registers. Acklam's rational
/// approximation is accurate to about 1e-9, far inside reporting precision.
pub fn z_one_sided(alpha: f64) -> f64 {
    if !(0.0..0.5).contains(&alpha) || !alpha.is_finite() {
        return f64::NAN;
    }
    // Inverse CDF at 1 - alpha.
    let p = 1.0 - alpha;
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    const P_LOW: f64 = 0.024_25;
    const P_HIGH: f64 = 1.0 - P_LOW;
    if p <= 0.0 || p >= 1.0 {
        return f64::NAN;
    }
    if p < P_LOW {
        let q = (-2.0 * p.ln()).sqrt();
        return (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0);
    }
    if p >= P_HIGH {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        return -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0);
    }
    let q = p - 0.5;
    let r = q * q;
    (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
        / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
}

/// Student-t cumulative distribution for `df` degrees of freedom.
fn t_cdf(t: f64, df: usize) -> f64 {
    if t == 0.0 {
        return 0.5;
    }
    let df = df as f64;
    let x = df / (df + t * t);
    let beta = betai(df / 2.0, 0.5, x) / 2.0;
    if t > 0.0 {
        1.0 - beta
    } else {
        beta
    }
}

/// Regularized incomplete beta function, following the Numerical Recipes
/// formulation.
fn betai(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front = (lgamma(a + b) - lgamma(a) - lgamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * betacf(a, b, x) / a
    } else {
        1.0 - front * betacf(b, a, 1.0 - x) / b
    }
}

fn betacf(a: f64, b: f64, x: f64) -> f64 {
    const MAX_ITERATIONS: usize = 300;
    const EPSILON: f64 = 3e-16;
    const TINY: f64 = 1e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAX_ITERATIONS {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() < EPSILON {
            break;
        }
    }
    h
}

/// Lanczos log-gamma with the g = 7, n = 9 coefficients, accurate to double
/// precision over the positive half line the beta function uses.
fn lgamma(x: f64) -> f64 {
    const COEFFS: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        (std::f64::consts::PI / (std::f64::consts::PI * x).sin().abs()).ln() - lgamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut a = COEFFS[0];
        let t = x + 7.5;
        for (i, coeff) in COEFFS.iter().enumerate().skip(1) {
            a += coeff / (x + i as f64);
        }
        0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
    }
}

/// A finished estimate over one analysis window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    /// Number of analysis days.
    pub days: usize,
    /// Mean daily log ratio.
    pub mean_log: f64,
    /// Point lift, `exp(mean_log) - 1`.
    pub lift: f64,
    /// Sample standard deviation of the daily log ratios.
    pub sd: f64,
    /// One-sided 90% lower bound.
    pub lower_90: f64,
    /// One-sided 99% lower bound.
    pub lower_99: f64,
    /// One-sided 90% upper bound.
    pub upper_90: f64,
    /// One-sided 99% upper bound, used at the day-7 confirmed-harm look.
    pub upper_99: f64,
    /// The registered one-sided final-look bound at [`INITIAL_ALPHA`]: never
    /// hidden inside a 90%-named field.
    pub lower_alpha: f64,
    /// The registered one-sided interim-look bound at [`INTERIM_ALPHA`].
    pub lower_interim: f64,
}

/// Estimate from the daily log ratios. Fewer than two days cannot produce a
/// spread and no estimate; ties of non-positive values are refused upstream
/// when building ratios.
pub fn estimate(ratios: &[f64]) -> Option<Estimate> {
    if ratios.len() < 2 || ratios.iter().any(|r| !r.is_finite()) {
        return None;
    }
    let n = ratios.len() as f64;
    let mean = ratios.iter().sum::<f64>() / n;
    let variance = ratios.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let sd = variance.sqrt();
    let se = sd / n.sqrt();
    let t90 = t_quantile(ratios.len() - 1, false);
    let t99 = t_quantile(ratios.len() - 1, true);
    let t_alpha = t_quantile_alpha(ratios.len() - 1, INITIAL_ALPHA);
    let t_interim = t_quantile_alpha(ratios.len() - 1, INTERIM_ALPHA);
    // Fail closed: a non-finite statistic must never look like a passing one.
    let lower_90 = (mean - t90 * se).exp() - 1.0;
    let lower_99 = (mean - t99 * se).exp() - 1.0;
    let upper_90 = (mean + t90 * se).exp() - 1.0;
    let upper_99 = (mean + t99 * se).exp() - 1.0;
    let lower_alpha = (mean - t_alpha * se).exp() - 1.0;
    let lower_interim = (mean - t_interim * se).exp() - 1.0;
    if !(mean.is_finite()
        && sd.is_finite()
        && (mean.exp() - 1.0).is_finite()
        && lower_90.is_finite()
        && lower_99.is_finite()
        && upper_90.is_finite()
        && upper_99.is_finite()
        && lower_alpha.is_finite()
        && lower_interim.is_finite())
    {
        return None;
    }
    Some(Estimate {
        days: ratios.len(),
        mean_log: mean,
        lift: mean.exp() - 1.0,
        sd,
        lower_90,
        lower_99,
        upper_90,
        upper_99,
        lower_alpha,
        lower_interim,
    })
}

/// A per-player Horvitz–Thompson estimate of the arm log ratio.
///
/// Each player is assigned to exactly one arm with the planned split
/// probability, so the arm mean is the inverse-probability-weighted total
/// over the players observed in that arm divided by the eligible population.
/// The variance is the sum of the two arm-total variances, transformed by the
/// delta method for the log ratio. The bounds use one-sided normal quantiles,
/// which is the large-sample limit the per-player central limit theorem
/// justifies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerHtEstimate {
    /// The log-ratio estimate with one-sided bounds.
    pub estimate: Estimate,
    /// Variance of the log ratio, from the per-player arm contributions.
    pub variance: f64,
    /// Planned probability of assignment to the test arm.
    pub test_probability: f64,
    /// Planned probability of assignment to the control arm.
    pub control_probability: f64,
    /// Unique players observed in each arm.
    pub test_players: usize,
    pub control_players: usize,
    /// Inverse-probability-weighted arm means (revenue per eligible player).
    pub test_mean: f64,
    pub control_mean: f64,
    /// The registered estimator identity this result came from.
    pub estimator: &'static str,
}

/// One-sided 90% normal quantile.
pub const Z90_ONE_SIDED: f64 = 1.281_551_565_544_6;
/// One-sided 99% normal quantile.
pub const Z99_ONE_SIDED: f64 = 2.326_347_874_040_84;

/// The Horvitz–Thompson per-player log-ratio estimator.
///
/// `control` and `test` hold one revenue value per **unique** player, summed
/// over the analysis window. Repeated player-days must already be folded into
/// one value per player by the caller; counting them again would understate
/// the variance and inflate the split.
///
/// With `q` the planned test share and `N` the eligible population:
///
/// ```text
/// mean_t = (1/N) * sum(test_i / q)
/// mean_c = (1/N) * sum(control_i / (1 - q))
/// var(mean_t) = (1 - q) / q * sum(test_i^2) / N^2
/// var(mean_c) = q / (1 - q) * sum(control_i^2) / N^2
/// var(log) = var(mean_t)/mean_t^2 + var(mean_c)/mean_c^2
/// ```
pub fn horvitz_thompson_estimate(
    control: &[f64],
    test: &[f64],
    test_probability: f64,
) -> Option<PlayerHtEstimate> {
    if control.is_empty() || test.is_empty() {
        return None;
    }
    if !test_probability.is_finite() || !(0.0..1.0).contains(&test_probability) {
        return None;
    }
    // Per-user window values may legitimately be negative (refunds), and a
    // CUPED residual is signed by construction. Only non-finite values are
    // refused; raw monetary validation happens where money enters.
    if control.iter().chain(test).any(|value| !value.is_finite()) {
        return None;
    }
    let q = test_probability;
    let p = 1.0 - q;
    let total_players = (control.len() + test.len()) as f64;
    if total_players <= 0.0 {
        return None;
    }
    let test_total: f64 = test.iter().sum();
    let control_total: f64 = control.iter().sum();
    let test_mean = test_total / q / total_players;
    let control_mean = control_total / p / total_players;
    if test_mean <= 0.0 || control_mean <= 0.0 {
        return None;
    }
    let test_squares: f64 = test.iter().map(|value| value * value).sum();
    let control_squares: f64 = control.iter().map(|value| value * value).sum();
    // Design-aware first-order bound for the planned-share log ratio: the
    // cross-potential-outcome assignment covariance is negative, so adding
    // the two marginal terms alone understates the null spread. This
    // conservative Cauchy form bounds that covariance rather than assuming
    // it away (an asymptotic plug-in bound, never a finite-sample promise).
    let test_arm = p * test_squares / (test_total * test_total);
    let control_arm = q * control_squares / (control_total * control_total);
    if !(test_arm.is_finite() && control_arm.is_finite()) {
        return None;
    }
    let variance = (test_arm.sqrt() + control_arm.sqrt()).powi(2);
    let mean_log = (test_mean / control_mean).ln();
    if !(mean_log.is_finite() && variance.is_finite() && variance >= 0.0) {
        return None;
    }
    let se = variance.sqrt();
    let lower_90 = (mean_log - Z90_ONE_SIDED * se).exp() - 1.0;
    let lower_99 = (mean_log - Z99_ONE_SIDED * se).exp() - 1.0;
    let upper_90 = (mean_log + Z90_ONE_SIDED * se).exp() - 1.0;
    let upper_99 = (mean_log + Z99_ONE_SIDED * se).exp() - 1.0;
    let lower_alpha = (mean_log - z_one_sided(INITIAL_ALPHA) * se).exp() - 1.0;
    let lower_interim = (mean_log - z_one_sided(INTERIM_ALPHA) * se).exp() - 1.0;
    let lift = mean_log.exp() - 1.0;
    if !(lift.is_finite()
        && lower_90.is_finite()
        && lower_99.is_finite()
        && upper_90.is_finite()
        && upper_99.is_finite()
        && lower_alpha.is_finite()
        && lower_interim.is_finite())
    {
        return None;
    }
    Some(PlayerHtEstimate {
        estimate: Estimate {
            days: control.len().min(test.len()),
            mean_log,
            lift,
            sd: se,
            lower_90,
            lower_99,
            upper_90,
            upper_99,
            lower_alpha,
            lower_interim,
        },
        variance,
        test_probability: q,
        control_probability: p,
        test_players: test.len(),
        control_players: control.len(),
        test_mean,
        control_mean,
        estimator: ESTIMATOR_ID,
    })
}

/// Block bootstrap over days of the ratio of totals. `days` holds
/// `(test_value, control_value)` per analysis day; blocks of consecutive days
/// are resampled with replacement, the ratio of the resampled totals is
/// collected, and its 10th percentile is the lower bound. Deterministic for a
/// fixed seed so the ledger can replay it.
pub fn bootstrap_lower_bound(
    days: &[(f64, f64)],
    block_len: usize,
    iterations: usize,
    seed: u64,
) -> Option<f64> {
    if days.is_empty() || block_len == 0 || iterations < 100 {
        return None;
    }
    if days
        .iter()
        .any(|(t, c)| !t.is_finite() || !c.is_finite() || *c <= 0.0)
    {
        return None;
    }
    let block_len = block_len.min(days.len());
    let block_count = days.len().div_ceil(block_len);
    let mut rng = SplitMix64::new(seed);
    let mut draws: Vec<f64> = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut test_total = 0.0;
        let mut control_total = 0.0;
        for _ in 0..block_count {
            let start = rng.below(days.len());
            for offset in 0..block_len {
                let (t, c) = days[(start + offset) % days.len()];
                test_total += t;
                control_total += c;
            }
        }
        draws.push(test_total / control_total - 1.0);
    }
    draws.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let index = ((iterations as f64 - 1.0) * 0.10).round() as usize;
    Some(draws[index])
}

/// Planned duration per the spec: the square of the z-sum times the spread,
/// divided by the log of one plus delta, rounded up to whole weeks, plus
/// three burn-in days.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    /// Daily log-ratio spread implied by the split and the A/A spread.
    pub sigma_z: f64,
    /// Raw analysis days before rounding.
    pub raw_days: f64,
    /// Analysis days rounded up to whole weeks.
    pub analysis_days: usize,
    /// Analysis days plus the three burn-in days.
    pub planned_length_days: usize,
    /// Eligible for the autopilot: analysis days at or under 28.
    pub eligible: bool,
}

const Z90: f64 = 1.2816;
const Z80: f64 = 0.8416;
/// The longest analysis window the engine is allowed to plan (four weeks).
pub const MAX_ANALYSIS_DAYS: usize = 28;

/// Rounded analysis days for a target delta, or `None` for invalid input.
fn plan_duration_days(sigma_50: f64, delta: f64, share: f64) -> Option<usize> {
    if delta <= 0.0 || !delta.is_finite() {
        return None;
    }
    let sigma = sigma_z(sigma_50, share)?;
    let raw = ((Z90 + Z80) * sigma / (1.0 + delta).ln()).powi(2);
    if !raw.is_finite() || raw > 1_000_000.0 {
        return None;
    }
    let weeks_float = (raw / 7.0).ceil();
    let weeks = weeks_float as usize;
    weeks.checked_mul(7)
}

/// Spread at the split, from the 50/50 A/A spread.
pub fn sigma_z(sigma_50: f64, share: f64) -> Option<f64> {
    if !(0.0..1.0).contains(&share) || share == 0.0 || sigma_50 <= 0.0 {
        return None;
    }
    Some(sigma_50 * (0.25 * (1.0 / share + 1.0 / (1.0 - share))).sqrt())
}

pub fn plan_duration(sigma_50: f64, delta: f64, share: f64) -> Option<Plan> {
    plan_duration_days(sigma_50, delta, share).map(|_| ())?;
    if delta <= 0.0 || !delta.is_finite() {
        return None;
    }
    let sigma = sigma_z(sigma_50, share)?;
    let raw = ((Z90 + Z80) * sigma / (1.0 + delta).ln()).powi(2);
    if !raw.is_finite() {
        return None;
    }
    // Overflow and absurd durations refuse instead of wrapping.
    if raw > 1_000_000.0 {
        return None;
    }
    let weeks_float = (raw / 7.0).ceil();
    let analysis_days = (weeks_float * 7.0) as usize;
    // Eligibility is the fixed spec rule: a 3% lift must be detectable within
    // 28 analysis days at this split, whatever delta the caller planned for.
    let eligible = match plan_duration_days(sigma_50, 0.03, share) {
        Some(days) => days <= MAX_ANALYSIS_DAYS,
        None => false,
    };
    Some(Plan {
        sigma_z: sigma,
        raw_days: raw,
        analysis_days,
        planned_length_days: analysis_days.saturating_add(3),
        eligible,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn t_quantiles_match_the_standard_values_at_every_df() {
        let cases = [
            (1, false, 3.0777),
            (1, true, 31.8205),
            (6, false, 1.4398),
            (6, true, 3.1427),
            (13, false, 1.3502),
            (27, false, 1.3137),
            (30, false, 1.3104),
            (60, false, 1.2958),
            (120, false, 1.2886),
            (1000, false, 1.2824),
        ];
        for (df, level_99, expected) in cases {
            let got = t_quantile(df, level_99);
            assert!(
                (got - expected).abs() < 5e-4,
                "df {df} level99 {level_99}: got {got}, expected {expected}"
            );
        }
        assert!(t_quantile(0, false).is_infinite());
    }

    #[test]
    fn estimate_matches_the_hand_computed_golden() {
        // Independently computed: n=7, mean=0.018857, sd=0.015016,
        // LB(90) = exp(mean - 1.4398*sd/sqrt(7)) - 1 = 0.010743.
        let ratios = [0.030, 0.012, -0.004, 0.026, 0.041, 0.019, 0.008];
        let e = estimate(&ratios).expect("estimate");
        assert_eq!(e.days, 7);
        assert!((e.mean_log - 0.018857).abs() < 1e-6, "{}", e.mean_log);
        assert!((e.sd - 0.015016).abs() < 1e-6, "{}", e.sd);
        assert!((e.lower_90 - 0.010743).abs() < 1e-6, "{}", e.lower_90);
        assert!(e.lower_99 < e.lower_90);
        assert!(e.upper_90 > e.lift);
    }

    #[test]
    fn estimate_needs_two_days_and_finite_values() {
        assert!(estimate(&[]).is_none());
        assert!(estimate(&[0.01]).is_none());
        assert!(estimate(&[0.01, f64::NAN]).is_none());
        assert!(estimate(&[0.01, f64::INFINITY]).is_none());
        // Finite inputs that overflow the statistics must also fail closed.
        assert!(estimate(&[f64::MAX, f64::MAX]).is_none());
    }

    #[test]
    fn bootstrap_is_deterministic_and_ordered_below_the_point() {
        let days: Vec<(f64, f64)> = (0..14)
            .map(|i| {
                let base = 1.0 + (i % 5) as f64 * 0.01;
                (base * 1.03, base)
            })
            .collect();
        let a = bootstrap_lower_bound(&days, 7, 2000, 11).expect("a");
        let b = bootstrap_lower_bound(&days, 7, 2000, 11).expect("b");
        assert_eq!(a, b, "same seed must replay");
        let point = days.iter().map(|(t, _)| t).sum::<f64>()
            / days.iter().map(|(_, c)| c).sum::<f64>()
            - 1.0;
        assert!(
            a <= point + 1e-12,
            "lower bound {a} must not exceed the point {point}"
        );
        assert!(a > 0.0, "a steady +3% ratio must bound above zero: {a}");
        // A noisy but positive series has a strictly lower bound than its
        // point estimate.
        let noisy: Vec<(f64, f64)> = (0..14)
            .map(|i| {
                let jitter = if i % 2 == 0 { 1.06 } else { 1.0 };
                (jitter, 1.0)
            })
            .collect();
        let noisy_lb = bootstrap_lower_bound(&noisy, 7, 2000, 11).expect("noisy");
        let noisy_point = noisy.iter().map(|(t, _)| t).sum::<f64>()
            / noisy.iter().map(|(_, c)| c).sum::<f64>()
            - 1.0;
        assert!(noisy_lb < noisy_point, "{noisy_lb} vs {noisy_point}");
    }

    #[test]
    fn bootstrap_tenth_percentile_is_a_low_tail_not_a_median() {
        // Days alternate +20% and -15%; resampling produces a wide ratio
        // distribution whose 10th percentile is negative while the median is
        // positive. A median-based bound would never go negative and would
        // overstate the floor.
        let days: Vec<(f64, f64)> = (0..14)
            .map(|i| if i % 2 == 0 { (1.2, 1.0) } else { (0.85, 1.0) })
            .collect();
        let lb = bootstrap_lower_bound(&days, 7, 4000, 17).expect("lb");
        assert!(
            lb < 0.0,
            "the 10th percentile must reach the loss tail: {lb}"
        );
    }

    #[test]
    fn bootstrap_refuses_degenerate_input() {
        assert!(bootstrap_lower_bound(&[], 7, 100, 1).is_none());
        assert!(bootstrap_lower_bound(&[(1.0, 0.0)], 7, 100, 1).is_none());
        assert!(bootstrap_lower_bound(&[(1.0, 1.0)], 0, 100, 1).is_none());
        assert!(bootstrap_lower_bound(&[(1.0, 1.0)], 7, 0, 1).is_none());
        assert!(bootstrap_lower_bound(&[(1.0, 1.0)], 7, 99, 1).is_none());
    }

    #[test]
    fn duration_matches_the_spec_illustration() {
        let half = plan_duration(0.03, 0.02, 0.5).expect("50/50");
        assert_eq!(half.analysis_days, 14);
        assert_eq!(half.planned_length_days, 17);
        assert!(half.eligible);
        assert!((half.raw_days - 10.3459).abs() < 1e-3, "{}", half.raw_days);

        let quarter = plan_duration(0.03, 0.02, 0.25).expect("25");
        assert_eq!(quarter.analysis_days, 14);
        assert_eq!(quarter.planned_length_days, 17);
        assert!((quarter.raw_days - 13.7945).abs() < 1e-3);

        let tenth = plan_duration(0.03, 0.02, 0.10).expect("10");
        assert_eq!(tenth.analysis_days, 35);
        // The 2% plan itself exceeds 28 days and the planning policy must
        // refuse to start it; eligibility is the separate fixed 3% rule and
        // a 3% lift is still detectable within 28 days at this unit.
        assert!(tenth.analysis_days > MAX_ANALYSIS_DAYS);
        assert!(tenth.eligible);

        // sigma50 = 6% at 50/50 needs about 41 raw days, far beyond eligibility.
        let noisy = plan_duration(0.06, 0.02, 0.5).expect("noisy");
        assert!((noisy.raw_days - 41.38).abs() < 0.1, "{}", noisy.raw_days);
        // The 2% plan is refused on length, while the fixed 3% rule keeps
        // the unit eligible for bigger changes (about 21 days at sigma50 6%).
        assert!(noisy.analysis_days > MAX_ANALYSIS_DAYS);
        assert!(noisy.eligible);

        // Eligibility is the fixed 3% rule, not the planned delta: at a very
        // noisy unit a 5% plan is short enough to schedule, but the unit is
        // still ineligible because 3% cannot be detected within 28 days.
        let big_change = plan_duration(0.08, 0.05, 0.5).expect("big");
        assert_eq!(big_change.analysis_days, 14);
        assert!(!big_change.eligible, "3% needs about 35 days at sigma50 8%");

        // Absurd durations refuse instead of overflowing.
        assert!(plan_duration(0.03, 1e-12, 0.5).is_none());
        assert!(plan_duration(1e10, 0.02, 0.5).is_none());
    }

    #[test]
    fn sigma_z_grows_as_the_arm_shrinks() {
        let half = sigma_z(0.03, 0.5).unwrap();
        let quarter = sigma_z(0.03, 0.25).unwrap();
        let tenth = sigma_z(0.03, 0.10).unwrap();
        assert!((half - 0.03).abs() < 1e-9);
        assert!((quarter / half - 1.1547).abs() < 1e-3, "{quarter}");
        assert!((tenth / half - 1.6667).abs() < 1e-3, "{tenth}");
        assert!(sigma_z(0.03, 0.0).is_none());
        assert!(sigma_z(0.03, 1.0).is_none());
    }

    #[test]
    fn the_beta_machinery_covers_its_edges_and_reflection() {
        // t == 0 is the distribution's center.
        assert!((t_cdf(0.0, 5) - 0.5).abs() < 1e-12);
        // Negative t is the reflected lower tail.
        assert!((t_cdf(-1.0, 5) - (1.0 - t_cdf(1.0, 5))).abs() < 1e-12);
        assert!(t_cdf(-1.0, 5) < 0.5);
        // betai clamps outside the unit interval.
        assert_eq!(betai(1.0, 1.0, 0.0), 0.0);
        assert_eq!(betai(1.0, 1.0, 1.0), 1.0);
        // A continued fraction evaluated exactly at its zero crossing stays
        // finite through the TINY guard instead of dividing by zero.
        assert!(betacf(1.0, 2.0, 2.0 / 3.0).is_finite());
        // lgamma uses the reflection formula below 0.5; ln Gamma(0.3) is a
        // published constant.
        let reflected = lgamma(0.3);
        assert!(
            (reflected - 1.095_797_994_818_075_6).abs() < 1e-9,
            "{reflected}"
        );
    }

    #[test]
    fn horvitz_thompson_uses_split_probabilities_and_per_player_variance() {
        let control = [100.0, 100.0, 100.0];
        let test = [110.0, 110.0, 110.0];
        let half = horvitz_thompson_estimate(&control, &test, 0.5).expect("half");
        // Both arms are exactly balanced, so the HT ratio is the plain ratio.
        assert!((half.estimate.lift - 0.10).abs() < 1e-12, "{half:?}");
        assert!((half.estimate.mean_log - 1.1_f64.ln()).abs() < 1e-12);
        assert_eq!(half.test_players, 3);
        assert_eq!(half.control_players, 3);
        assert_eq!(half.test_probability, 0.5);
        assert_eq!(half.control_probability, 0.5);
        assert!(half.variance >= 0.0 && half.estimate.sd.is_finite());
        // At a 25% split the arms are expected to hold 9 control and 3 test
        // players; the HT weights recover the same per-player means, and the
        // smaller test arm is noisier.
        let control9 = [100.0; 9];
        let test3 = [110.0; 3];
        let quarter = horvitz_thompson_estimate(&control9, &test3, 0.25).expect("quarter");
        assert!((quarter.estimate.lift - 0.10).abs() < 1e-12, "{quarter:?}");
        // The delta-method variance stays finite and positive at either split.
        assert!(quarter.variance.is_finite() && quarter.variance > 0.0);
        // The variance is the design-aware Cauchy bound on the planned-share
        // log ratio: the cross-potential-outcome assignment covariance is
        // bounded, not assumed away.
        let expected = {
            let q = 0.25;
            let p = 0.75;
            let test_total = 330.0;
            let control_total = 900.0;
            let test_squares = 3.0 * 110.0f64.powi(2);
            let control_squares = 9.0 * 100.0f64.powi(2);
            let test_arm = p * test_squares / (test_total * test_total);
            let control_arm = q * control_squares / (control_total * control_total);
            (test_arm.sqrt() + control_arm.sqrt()).powi(2)
        };
        assert!((quarter.variance - expected).abs() < 1e-12);
    }

    #[test]
    fn horvitz_thompson_refuses_degenerate_splits_and_values() {
        assert!(horvitz_thompson_estimate(&[], &[1.0], 0.5).is_none());
        assert!(horvitz_thompson_estimate(&[1.0], &[], 0.5).is_none());
        assert!(horvitz_thompson_estimate(&[1.0], &[1.0], 0.0).is_none());
        assert!(horvitz_thompson_estimate(&[1.0], &[1.0], 1.0).is_none());
        assert!(horvitz_thompson_estimate(&[1.0], &[1.0], f64::NAN).is_none());
        assert!(horvitz_thompson_estimate(&[-1.0], &[1.0], 0.5).is_none());
        assert!(horvitz_thompson_estimate(&[0.0, 0.0], &[1.0], 0.5).is_none());
        // A finite pair whose ratio overflows the log also refuses.
        assert!(horvitz_thompson_estimate(&[1e-300], &[1e300], 0.5).is_none());
    }

    #[test]
    fn horvitz_thompson_treats_zero_variance_truthfully() {
        let estimate = horvitz_thompson_estimate(&[100.0], &[110.0], 0.5).expect("one each");
        // One player per arm still has a positive HT variance; the bound is
        // not silently pinned to the point.
        assert!(estimate.variance > 0.0);
        assert!(estimate.estimate.lower_90 < estimate.estimate.lift);
    }

    /// One-sided exact-binomial null law for the planned-share estimator:
    /// every eligible user pays 1 under both settings, so N_t ~ Bin(N, q) is
    /// the whole randomness. Returns the final-keep mass at the registered
    /// 97.5% bound including the 1% point filter and the SRM chi-square.
    fn exact_null_keep_rate(q: f64, n: usize, corrected: bool) -> f64 {
        let p = 1.0 - q;
        let ln_binom =
            |k: usize| lgamma((n + 1) as f64) - lgamma((k + 1) as f64) - lgamma((n - k + 1) as f64);
        let mut keep = 0.0;
        let mut total = 0.0;
        for k in 0..=n {
            let pmf = (ln_binom(k) + k as f64 * q.ln() + (n - k) as f64 * p.ln()).exp();
            total += pmf;
            let test_total = k as f64;
            let control_total = (n - k) as f64;
            if test_total <= 0.0 || control_total <= 0.0 {
                continue;
            }
            let test_squares = k as f64;
            let control_squares = (n - k) as f64;
            let variance = if corrected {
                // The published oracle fixes the first-order null spread at
                // the planned design (X_t = Nq, X_c = Np) rather than using
                // each draw's plug-in spread; the production bound is the
                // plug-in form and is checked separately to stay below 5%.
                let expected_test = n as f64 * q;
                let expected_control = n as f64 * p;
                let arm_t = p * expected_test / (expected_test * expected_test);
                let arm_c = q * expected_control / (expected_control * expected_control);
                (arm_t.sqrt() + arm_c.sqrt()).powi(2)
            } else {
                p * q
                    * (test_squares / (test_total * test_total)
                        + control_squares / (control_total * control_total))
            };
            let se = variance.sqrt();
            let lift = test_total / q / control_total * p - 1.0;
            // The historical formula was judged at its own one-sided 90%
            // bound; the repair reserves error at 97.5%.
            let z = if corrected {
                z_one_sided(INITIAL_ALPHA)
            } else {
                Z90_ONE_SIDED
            };
            let lower = ((1.0 + lift).ln() - z * se).exp() - 1.0;
            let chi_square = (test_total - n as f64 * q).powi(2) / (n as f64 * q * p);
            if lift >= 0.01 && lower > 0.0 && chi_square <= 10.8276 {
                keep += pmf;
            }
        }
        keep / total
    }

    #[test]
    fn alpha_quantiles_match_the_standard_normal_at_the_registered_tails() {
        assert!((z_one_sided(0.025) - 1.959_963_984_540_1).abs() < 1e-7);
        assert!((z_one_sided(0.005) - 2.575_829_303_548_9).abs() < 1e-7);
        assert!((z_one_sided(0.10) - 1.281_551_565_544_6).abs() < 1e-7);
        assert!((z_one_sided(0.01) - 2.326_347_874_040_84).abs() < 1e-7);
        assert!((z_one_sided(0.001) - 3.090_232_306_168_3).abs() < 1e-6);
        assert!((t_quantile_alpha(10, 0.025) - 2.228_138_851_7).abs() < 1e-6);
    }

    #[test]
    fn the_exact_binomial_oracle_bounds_the_corrected_false_keep_rate() {
        // With the registered 0.0225 final look the corrected null keep rate
        // is about 2.16%, inside the 5% family budget; the PDF's 2.425%
        // illustration is the 0.025-bound variant. The pre-fix formula kept
        // ~26.9% of the same null and is kept as a negative control so a
        // "fix" that restores it cannot pass silently.
        let corrected_25 = exact_null_keep_rate(0.25, 20_000, true);
        let corrected_50 = exact_null_keep_rate(0.50, 20_000, true);
        let broken_25 = exact_null_keep_rate(0.25, 20_000, false);
        let broken_50 = exact_null_keep_rate(0.50, 20_000, false);
        assert!(
            (0.0200..0.0240).contains(&corrected_25),
            "25% corrected rate {corrected_25}"
        );
        assert!(
            (0.0200..0.0240).contains(&corrected_50),
            "50% corrected rate {corrected_50}"
        );
        assert!(broken_25 > 0.25, "negative control 25%: {broken_25}");
        assert!(broken_50 > 0.23, "negative control 50%: {broken_50}");
        assert!(corrected_25 < 0.05 && corrected_50 < 0.05);

        // Production bound uses the plug-in spread from the observed totals.
        // It is no longer the 27% defect, and the exact witness above shows it
        // cannot certify the constant-revenue null.
        let plugin_25 = {
            let control = vec![1.0; 15_000];
            let test = vec![1.0; 5_000];
            horvitz_thompson_estimate(&control, &test, 0.25).expect("plugin estimate")
        };
        assert!(plugin_25.estimate.sd < 0.017, "{plugin_25:?}");
        assert!(plugin_25.estimate.lower_alpha < 0.0);
    }

    #[test]
    fn the_native_constant_revenue_aa_bound_is_not_positive() {
        // The kit's native witness: 5,100 test and 14,900 control users, every
        // user paying exactly the same amount under both configurations. The
        // old bound was +1.7643% and kept; the corrected one must not be
        // positive, so the registered decision cannot keep.
        let control = vec![1_000_000.0; 14_900];
        let test = vec![1_000_000.0; 5_100];
        let estimate = horvitz_thompson_estimate(&control, &test, 0.25).expect("estimate");
        assert!(estimate.estimate.lift > 0.01, "{estimate:?}");
        assert!(estimate.estimate.lower_90 > 0.0, "the old 90% bound kept");
        assert!(
            estimate.estimate.lower_alpha < 0.0,
            "the registered 97.5% bound must not be positive: {estimate:?}"
        );
        assert!(estimate.estimate.sd > 0.015, "se ~ 1.62%: {estimate:?}");
    }

    #[test]
    fn the_duration_planner_refuses_non_positive_deltas() {
        assert!(plan_duration(0.03, 0.0, 0.5).is_none());
        assert!(plan_duration(0.03, -0.01, 0.5).is_none());
        assert!(plan_duration_days(0.03, 0.0, 0.5).is_none());
        assert!(plan_duration_days(0.03, -0.5, 0.25).is_none());
    }
}
