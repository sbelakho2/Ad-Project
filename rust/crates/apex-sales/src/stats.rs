//! Numerical statistics the engine's decisions rest on.
//!
//! Everything here is a pure function with a documented method:
//!
//! * `ln Γ` by the Lanczos approximation (g = 7, n = 9);
//! * the regularized incomplete beta `I_x(a, b)` by Lentz's continued
//!   fraction (the Beta CDF), and its inverse by bisection;
//! * the normal CDF through `erfc` (Numerical Recipes' Chebyshev fit, error
//!   below 1.2e-7) and the normal quantile by Acklam's rational
//!   approximation;
//! * interval estimates for proportions: Jeffreys (quantiles of
//!   `Beta[k + ½, n − k + ½]`), Wilson score, and Newcombe's hybrid score interval for a
//!   difference of two proportions (method 10 of Newcombe, 1998);
//! * proper scoring rules (Brier, log loss), expected calibration error and
//!   isotonic regression by pool-adjacent-violators.

/// `ln Γ(x)` for `x > 0` (Lanczos, g = 7). Non-positive or non-finite input
/// yields `f64::INFINITY`.
pub fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const COEF: [f64; 9] = [
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
    if !x.is_finite() || x <= 0.0 {
        return f64::INFINITY;
    }
    if x < 0.5 {
        // Reflection: Γ(x)Γ(1−x) = π / sin(πx).
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = COEF[0];
    let t = x + G + 0.5;
    for (i, c) in COEF.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// `ln B(a, b)`.
pub fn ln_beta(a: f64, b: f64) -> f64 {
    ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b)
}

fn beta_continued_fraction(a: f64, b: f64, x: f64) -> f64 {
    const MAX_ITER: usize = 300;
    const EPS: f64 = 1e-14;
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
    for m in 1..=MAX_ITER {
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
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// The Beta(`a`, `b`) CDF at `x`: the regularized incomplete beta
/// `I_x(a, b)`. Out-of-range `x` is clamped to `[0, 1]`.
pub fn beta_cdf(x: f64, a: f64, b: f64) -> f64 {
    if !(a > 0.0 && b > 0.0) || x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_front = a * x.ln() + b * (1.0 - x).ln() - ln_beta(a, b);
    let front = ln_front.exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        (front * beta_continued_fraction(a, b, x) / a).clamp(0.0, 1.0)
    } else {
        (1.0 - front * beta_continued_fraction(b, a, 1.0 - x) / b).clamp(0.0, 1.0)
    }
}

/// The `q` quantile of Beta(`a`, `b`) by bisection on [`beta_cdf`] (60
/// halvings: below 1e-17 of the unit interval).
pub fn beta_quantile(q: f64, a: f64, b: f64) -> f64 {
    let q = q.clamp(0.0, 1.0);
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if beta_cdf(mid, a, b) < q {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The probability that a Beta(`a`, `b`) quantity exceeds `threshold`.
pub fn beta_tail_above(threshold: f64, a: f64, b: f64) -> f64 {
    1.0 - beta_cdf(threshold, a, b)
}

/// Complementary error function (Numerical Recipes `erfcc`, |error| < 1.2e-7).
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let ans = t * poly.exp();
    if x >= 0.0 {
        ans
    } else {
        2.0 - ans
    }
}

/// Standard normal CDF Φ(z).
pub fn normal_cdf(z: f64) -> f64 {
    0.5 * erfc(-z / std::f64::consts::SQRT_2)
}

/// Standard normal quantile Φ⁻¹(p) (Acklam's algorithm, relative error below
/// 1.15e-9). `p` outside (0, 1) maps to ±infinity.
pub fn normal_quantile(p: f64) -> f64 {
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
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    let p_low = 0.024_25;
    if p < p_low {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - p_low {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

/// Numerically stable logistic function.
pub fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

/// Log-odds of `p`, clamped away from 0 and 1 so the result is finite.
pub fn logit(p: f64) -> f64 {
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    (p / (1.0 - p)).ln()
}

/// A two-sided interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    /// Lower end.
    pub low: f64,
    /// Upper end.
    pub high: f64,
}

/// Jeffreys interval for `k` successes in `n` trials at `level` (e.g. 0.95).
/// The end at 0 or 1 successes is pinned to the boundary, as Brown, Cai and
/// DasGupta (2001) recommend.
pub fn jeffreys_interval(k: f64, n: f64, level: f64) -> Interval {
    if n <= 0.0 {
        return Interval {
            low: 0.0,
            high: 1.0,
        };
    }
    let k = k.clamp(0.0, n);
    let tail = (1.0 - level) / 2.0;
    let a = k + 0.5;
    let b = n - k + 0.5;
    let low = if k <= 0.0 {
        0.0
    } else {
        beta_quantile(tail, a, b)
    };
    let high = if k >= n {
        1.0
    } else {
        beta_quantile(1.0 - tail, a, b)
    };
    Interval { low, high }
}

/// Wilson score interval for `k` successes in `n` trials at `level`.
pub fn wilson_interval(k: f64, n: f64, level: f64) -> Interval {
    if n <= 0.0 {
        return Interval {
            low: 0.0,
            high: 1.0,
        };
    }
    let z = normal_quantile(1.0 - (1.0 - level) / 2.0);
    let p = (k / n).clamp(0.0, 1.0);
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denom;
    Interval {
        low: (centre - half).max(0.0),
        high: (centre + half).min(1.0),
    }
}

/// Newcombe's hybrid score interval for `p1 − p2` (method 10, 1998).
pub fn newcombe_difference(k1: f64, n1: f64, k2: f64, n2: f64, level: f64) -> Interval {
    let p1 = if n1 > 0.0 { k1 / n1 } else { 0.0 };
    let p2 = if n2 > 0.0 { k2 / n2 } else { 0.0 };
    let w1 = wilson_interval(k1, n1, level);
    let w2 = wilson_interval(k2, n2, level);
    let d = p1 - p2;
    let low = d - ((p1 - w1.low).powi(2) + (w2.high - p2).powi(2)).sqrt();
    let high = d + ((w1.high - p1).powi(2) + (p2 - w2.low).powi(2)).sqrt();
    Interval { low, high }
}

/// Mean Brier score `mean[(p − y)²]` over `(prediction, outcome)` pairs.
pub fn brier(pairs: &[(f64, bool)]) -> Option<f64> {
    if pairs.is_empty() {
        return None;
    }
    let total: f64 = pairs
        .iter()
        .map(|&(p, y)| {
            let y = if y { 1.0 } else { 0.0 };
            (p - y).powi(2)
        })
        .sum();
    Some(total / pairs.len() as f64)
}

/// Mean log loss with predictions clamped to `[1e-6, 1 − 1e-6]`, so one
/// confident miss is enormous but finite.
pub fn log_loss(pairs: &[(f64, bool)]) -> Option<f64> {
    if pairs.is_empty() {
        return None;
    }
    let total: f64 = pairs
        .iter()
        .map(|&(p, y)| {
            let p = p.clamp(1e-6, 1.0 - 1e-6);
            if y {
                -p.ln()
            } else {
                -(1.0 - p).ln()
            }
        })
        .sum();
    Some(total / pairs.len() as f64)
}

/// One reliability bin.
#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityBin {
    /// Lower edge of the predicted-probability bin.
    pub lower: f64,
    /// Upper edge.
    pub upper: f64,
    /// Pairs in the bin.
    pub n: usize,
    /// Mean prediction in the bin.
    pub mean_predicted: f64,
    /// Observed event rate in the bin.
    pub observed: f64,
    /// 95% Jeffreys interval of the observed rate.
    pub interval: Interval,
}

/// Equal-width reliability bins (`bins ≥ 1`) and the expected calibration
/// error `Σ (n_b / N) · |observed_b − predicted_b|`.
pub fn reliability(pairs: &[(f64, bool)], bins: usize) -> (Vec<ReliabilityBin>, Option<f64>) {
    let bins = bins.max(1);
    let mut acc = vec![(0usize, 0.0f64, 0.0f64); bins];
    for &(p, y) in pairs {
        let p = p.clamp(0.0, 1.0);
        let idx = ((p * bins as f64) as usize).min(bins - 1);
        if let Some(slot) = acc.get_mut(idx) {
            slot.0 += 1;
            slot.1 += p;
            if y {
                slot.2 += 1.0;
            }
        }
    }
    let mut out = Vec::new();
    let mut ece = 0.0;
    for (i, (n, sum_p, hits)) in acc.into_iter().enumerate() {
        if n == 0 {
            continue;
        }
        let mean_predicted = sum_p / n as f64;
        let observed = hits / n as f64;
        ece += n as f64 / pairs.len() as f64 * (observed - mean_predicted).abs();
        out.push(ReliabilityBin {
            lower: i as f64 / bins as f64,
            upper: (i + 1) as f64 / bins as f64,
            n,
            mean_predicted,
            observed,
            interval: jeffreys_interval(hits, n as f64, 0.95),
        });
    }
    let ece = if pairs.is_empty() { None } else { Some(ece) };
    (out, ece)
}

/// A monotone non-decreasing step function fitted by isotonic regression.
#[derive(Debug, Clone, PartialEq)]
pub struct Isotonic {
    /// Block upper-score boundaries, ascending.
    knots: Vec<f64>,
    /// Fitted value of each block.
    values: Vec<f64>,
}

impl Isotonic {
    /// Fit by pool-adjacent-violators to `(score, outcome, weight)` triples.
    /// Returns `None` when no triple has positive weight.
    pub fn fit(points: &[(f64, f64, f64)]) -> Option<Self> {
        let mut pts: Vec<(f64, f64, f64)> = points
            .iter()
            .copied()
            .filter(|&(s, y, w)| s.is_finite() && y.is_finite() && w > 0.0)
            .collect();
        if pts.is_empty() {
            return None;
        }
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Each block: (max score, weighted mean, total weight).
        let mut blocks: Vec<(f64, f64, f64)> = Vec::with_capacity(pts.len());
        for (s, y, w) in pts {
            blocks.push((s, y, w));
            while blocks.len() >= 2 {
                let n = blocks.len();
                let (prev, last) = (blocks[n - 2], blocks[n - 1]);
                if prev.1 <= last.1 {
                    break;
                }
                let w = prev.2 + last.2;
                let merged = (last.0, (prev.1 * prev.2 + last.1 * last.2) / w, w);
                blocks.truncate(n - 2);
                blocks.push(merged);
            }
        }
        Some(Self {
            knots: blocks.iter().map(|b| b.0).collect(),
            values: blocks.iter().map(|b| b.1).collect(),
        })
    }

    /// The calibrated value at `score`: the value of the first block whose
    /// upper boundary is at or above it (the last block beyond the range).
    pub fn predict(&self, score: f64) -> f64 {
        let idx = self.knots.partition_point(|&k| k < score);
        let idx = idx.min(self.values.len().saturating_sub(1));
        self.values.get(idx).copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn ln_gamma_matches_known_values() {
        assert!(close(ln_gamma(1.0), 0.0, 1e-12));
        assert!(close(ln_gamma(5.0), 24f64.ln(), 1e-12));
        assert!(close(
            ln_gamma(0.5),
            std::f64::consts::PI.sqrt().ln(),
            1e-12
        ));
        assert!(close(ln_gamma(0.1), 2.252_712_651_734_206, 1e-10));
        assert!(ln_gamma(0.0).is_infinite());
    }

    #[test]
    fn beta_cdf_matches_closed_forms() {
        // Beta(1,1) is uniform; Beta(2,1) has CDF x²; Beta(1,3) has 1−(1−x)³.
        for x in [0.1, 0.37, 0.5, 0.9] {
            assert!(close(beta_cdf(x, 1.0, 1.0), x, 1e-12));
            assert!(close(beta_cdf(x, 2.0, 1.0), x * x, 1e-12));
            assert!(close(beta_cdf(x, 1.0, 3.0), 1.0 - (1.0 - x).powi(3), 1e-12));
        }
        assert!(close(beta_cdf(0.5, 7.0, 7.0), 0.5, 1e-12));
        assert_eq!(beta_cdf(-1.0, 2.0, 2.0), 0.0);
        assert_eq!(beta_cdf(2.0, 2.0, 2.0), 1.0);
        assert!(beta_cdf(0.5, 0.0, 1.0).is_nan());
    }

    #[test]
    fn beta_quantile_inverts_the_cdf() {
        for (a, b) in [(0.5, 0.5), (2.0, 30.0), (40.0, 9000.0)] {
            for q in [0.025, 0.5, 0.975] {
                let x = beta_quantile(q, a, b);
                assert!(close(beta_cdf(x, a, b), q, 1e-9), "{a} {b} {q}");
            }
        }
        assert!(beta_tail_above(0.5, 50.0, 1.0) > 0.99);
    }

    #[test]
    fn normal_functions_match_tables() {
        assert!(close(normal_cdf(0.0), 0.5, 1e-7));
        assert!(close(normal_cdf(1.96), 0.975_002_105, 5e-7));
        assert!(close(normal_quantile(0.975), 1.959_963_985, 1e-8));
        assert!(close(normal_quantile(0.01), -2.326_347_874, 1e-8));
        assert!(close(normal_quantile(0.999), 3.090_232_306, 1e-8));
        assert!(normal_quantile(0.0).is_infinite());
        assert!(close(erfc(0.0), 1.0, 1e-7));
    }

    #[test]
    fn logistic_helpers_are_stable_and_inverse() {
        assert!(close(sigmoid(0.0), 0.5, 1e-15));
        assert!(sigmoid(-800.0) >= 0.0 && sigmoid(800.0) <= 1.0);
        for p in [0.01, 0.3, 0.8] {
            assert!(close(sigmoid(logit(p)), p, 1e-12));
        }
        assert!(logit(0.0).is_finite() && logit(1.0).is_finite());
    }

    #[test]
    fn intervals_cover_and_order() {
        let j = jeffreys_interval(3.0, 100.0, 0.95);
        assert!(j.low > 0.0 && j.low < 0.03 && j.high > 0.03 && j.high < 0.1);
        assert_eq!(jeffreys_interval(0.0, 50.0, 0.95).low, 0.0);
        assert_eq!(jeffreys_interval(50.0, 50.0, 0.95).high, 1.0);
        assert_eq!(jeffreys_interval(0.0, 0.0, 0.95).high, 1.0);
        // Wilson (k=81, n=263) from Newcombe's paper: 0.2553 to 0.3662.
        let w = wilson_interval(81.0, 263.0, 0.95);
        assert!(close(w.low, 0.2553, 5e-4) && close(w.high, 0.3662, 5e-4));
        // Newcombe (1998), 56/70 against 48/80: d = 0.2, method 10 gives
        // 0.0524 to 0.3339.
        let d = newcombe_difference(56.0, 70.0, 48.0, 80.0, 0.95);
        assert!(
            close(d.low, 0.0524, 1e-3) && close(d.high, 0.3339, 1e-3),
            "{d:?}"
        );
    }

    #[test]
    fn scoring_rules_reward_calibration() {
        let good = vec![(0.9, true), (0.1, false), (0.8, true), (0.2, false)];
        let bad = vec![(0.1, true), (0.9, false), (0.2, true), (0.8, false)];
        assert!(brier(&good) < brier(&bad));
        assert!(log_loss(&good) < log_loss(&bad));
        assert!(brier(&[]).is_none() && log_loss(&[]).is_none());
        let confident_miss = log_loss(&[(1.0, false)]).unwrap_or(0.0);
        assert!(confident_miss.is_finite() && confident_miss > 10.0);
    }

    #[test]
    fn reliability_bins_and_ece() {
        // Perfectly calibrated within bins: predictions 0.2 with 20% hits.
        let mut pairs = Vec::new();
        for i in 0..100 {
            pairs.push((0.2, i % 5 == 0));
        }
        let (bins, ece) = reliability(&pairs, 10);
        assert_eq!(bins.len(), 1);
        assert!(close(ece.unwrap_or(1.0), 0.0, 1e-12));
        assert!(bins[0].interval.low < 0.2 && bins[0].interval.high > 0.2);
        let (_, ece) = reliability(&[(0.9, false), (0.9, false)], 10);
        assert!(close(ece.unwrap_or(0.0), 0.9, 1e-12));
        assert!(reliability(&[], 5).1.is_none());
    }

    #[test]
    fn isotonic_fit_is_monotone_and_pools_violators() {
        let pts = vec![
            (0.1, 0.0, 1.0),
            (0.2, 1.0, 1.0),
            (0.3, 0.0, 1.0),
            (0.4, 1.0, 1.0),
            (0.5, 1.0, 1.0),
        ];
        let iso = Isotonic::fit(&pts).unwrap_or(Isotonic {
            knots: vec![],
            values: vec![],
        });
        let preds: Vec<f64> = [0.05, 0.1, 0.2, 0.3, 0.45, 0.9]
            .iter()
            .map(|&s| iso.predict(s))
            .collect();
        for w in preds.windows(2) {
            assert!(w[0] <= w[1] + 1e-12, "{preds:?}");
        }
        assert!(close(iso.predict(0.25), 0.5, 1e-12));
        assert!(close(iso.predict(0.9), 1.0, 1e-12));
        assert!(Isotonic::fit(&[(0.1, 1.0, 0.0)]).is_none());
    }
}
