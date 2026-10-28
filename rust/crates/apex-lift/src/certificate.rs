//! Finite-population causal certificates (revision-2 validation).
//!
//! Fixed closed cohort, independent Bernoulli assignment, one whole-window
//! outcome per eligible randomization unit, known pre-assignment envelopes and
//! a fixed prior predictor. Only assignment is random; outcomes need not be
//! identically distributed and no normal approximation is used.
//!
//! The causal contrast is the augmented inverse-probability estimate
//! `Z_i = A_i (Y_i - m_i)/q - (1-A_i)(Y_i - m_i)/p`; the cohort baseline is
//! `M0 = mean(m) + mean((1-A)(Y-m)/p)`. A deterministic Hoeffding radius and a
//! Maurer-Pontil empirical-Bernstein radius are each granted half of the
//! bound's error and the smaller is used, so the data-dependent choice is
//! covered by a union bound. The same construction bounds the baseline from
//! above, which yields an **absolute** lower bound on the revenue the treated
//! subset actually added: `L_actual = observed_total - N * U0`.
//!
//! The envelopes and priors are assumptions this module cannot verify; a
//! caller without them must refuse a financial certificate.

/// One eligible randomization unit and its observed whole-window outcome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CertifiedUser {
    /// Assigned to the treated configuration.
    pub test: bool,
    /// Observed whole-window revenue in micros.
    pub outcome: f64,
    /// Fixed pre-assignment predictor of the old-setting outcome, clipped by
    /// the caller into `[0, envelope]`.
    pub prior: f64,
    /// Hard pre-assignment envelope covering BOTH potential outcomes.
    pub envelope: f64,
}

/// A certified reading for one cohort.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Certificate {
    /// Population incremental revenue per eligible user.
    pub delta: f64,
    /// One-sided lower bound on `delta` at the requested error.
    pub delta_lower: f64,
    /// One-sided upper bound on the all-old baseline mean.
    pub baseline_upper: f64,
    /// `delta_lower / baseline_upper` when both parts are positive.
    pub relative_lift_lower: Option<f64>,
    /// One-sided lower bound on the revenue the treated subset actually added.
    pub actual_gain_lower: f64,
    /// Which deterministic radius was selected before seeing outcomes.
    pub method: &'static str,
}

/// Conservative radius for a bounded sum; the choice uses only fixed design
/// quantities, never the observed outcomes.
fn radius(n: f64, q: f64, sum_h2: f64, max_h: f64, alpha: f64, baseline: bool) -> f64 {
    let p = 1.0 - q;
    let x = (1.0 / alpha).ln();
    let (widths2, variance, max_centered) = if baseline {
        (sum_h2 / (p * p), q / p * sum_h2, q.max(p) * max_h / p)
    } else {
        (
            sum_h2 / (q * q * p * p),
            sum_h2 / (q * p),
            q.max(p) * max_h / (q * p),
        )
    };
    let hoeffding = (widths2 * x / 2.0).sqrt() / n;
    let z = max_centered * x / 3.0;
    let bernstein = (z + (z * z + 2.0 * variance * x).sqrt()) / n;
    bernstein.min(hoeffding)
}

/// Maurer-Pontil Theorem 11 empirical-Bernstein radius for independent
/// bounded, not necessarily identically distributed variables.
fn empirical_radius(variance: f64, range: f64, n: f64, alpha: f64) -> f64 {
    if n < 2.0 || !(alpha > 0.0 && alpha < 1.0) {
        return f64::INFINITY;
    }
    let x = (4.0 / alpha).ln();
    (2.0 * variance * x / n).sqrt() + 7.0 * range * x / (3.0 * (n - 1.0))
}

/// Certify one cohort at a one-sided error `alpha` per statement.
///
/// `alpha` is the error of ONE statement; the caller allocates the family
/// budget across statements before calling.
#[must_use]
pub fn certify(users: &[CertifiedUser], q: f64, alpha: f64) -> Option<Certificate> {
    if users.len() < 2 || !q.is_finite() || !(0.0..1.0).contains(&q) {
        return None;
    }
    if !alpha.is_finite() || !(0.0..1.0).contains(&alpha) {
        return None;
    }
    let p = 1.0 - q;
    let n = users.len() as f64;
    for user in users {
        if !(user.outcome.is_finite()
            && user.prior.is_finite()
            && user.envelope.is_finite()
            && user.prior >= 0.0
            && user.prior <= user.envelope
            && user.outcome >= 0.0
            && user.outcome <= user.envelope)
        {
            return None;
        }
    }
    let mut delta_sum = 0.0;
    let mut base_sum = 0.0;
    let mut sum_h2 = 0.0;
    let mut max_h: f64 = 0.0;
    let mut sum_z2 = 0.0;
    let mut sum_w2 = 0.0;
    let mut prior_sum = 0.0;
    let mut envelope_sum = 0.0;
    let mut observed_total = 0.0;
    let mut control_revenue = 0.0;
    let mut max_m: f64 = 0.0;
    let mut max_u_minus_m: f64 = 0.0;
    for user in users {
        let m = user.prior;
        let u = user.envelope;
        let h = m.max(u - m);
        let z = if user.test {
            (user.outcome - m) / q
        } else {
            -(user.outcome - m) / p
        };
        let w = if user.test {
            0.0
        } else {
            (user.outcome - m) / p
        };
        delta_sum += z;
        base_sum += w;
        sum_h2 += h * h;
        max_h = max_h.max(h);
        sum_z2 += z * z;
        sum_w2 += w * w;
        prior_sum += m;
        envelope_sum += u;
        observed_total += user.outcome;
        if !user.test {
            control_revenue += user.outcome;
        }
        max_m = max_m.max(m);
        max_u_minus_m = max_u_minus_m.max(u - m);
    }
    let delta = delta_sum / n;
    let mean_prior = prior_sum / n;
    let baseline = mean_prior + base_sum / n;
    let radius_det = radius(n, q, sum_h2, max_h, alpha / 2.0, false);
    let radius_base_det = radius(n, q, sum_h2, max_h, alpha / 2.0, true);
    let variance_z = ((sum_z2 - n * delta * delta) / (n - 1.0)).max(0.0);
    let residual_mean = baseline - mean_prior;
    let variance_w = ((sum_w2 - n * residual_mean * residual_mean) / (n - 1.0)).max(0.0);
    let range_z = (max_m / q).max(max_u_minus_m / p) + (max_u_minus_m / q).max(max_m / p);
    let range_w = (max_m + max_u_minus_m) / p;
    let radius_eb = empirical_radius(variance_z, range_z, n, alpha / 2.0);
    let radius_base_eb = empirical_radius(variance_w, range_w, n, alpha / 2.0);
    let (radius_delta, method) = if radius_eb < radius_det {
        (radius_eb, "empirical Bernstein")
    } else {
        (radius_det, "deterministic")
    };
    let radius_base = radius_base_det.min(radius_base_eb);
    let delta_lower = delta - radius_delta;
    let baseline_upper = (envelope_sum / n).min(baseline + radius_base);
    let relative_lift_lower = (delta_lower > 0.0 && control_revenue > 0.0)
        .then(|| delta_lower / baseline_upper.max(1e-300));
    let actual_gain_lower = observed_total - n * baseline_upper;
    if !(delta.is_finite()
        && delta_lower.is_finite()
        && baseline_upper.is_finite()
        && actual_gain_lower.is_finite())
    {
        return None;
    }
    Some(Certificate {
        delta,
        delta_lower,
        baseline_upper,
        relative_lift_lower,
        actual_gain_lower,
        method,
    })
}

/// The fee an absolute lower gain justifies: the same floor rule as
/// [`certified_fee_micros`], for callers that already hold the bound.
#[must_use]
pub fn certified_fee_from_gain(actual_gain_lower: f64, rate: f64) -> i64 {
    if actual_gain_lower <= 0.0 || !rate.is_finite() || rate <= 0.0 {
        return 0;
    }
    let fee = rate * actual_gain_lower;
    if !fee.is_finite() || fee <= 0.0 {
        return 0;
    }
    (fee + 1e-9).floor() as i64
}

/// The fee one certified cohort can justify: at most 30% of the absolute
/// lower gain, rounded down to a whole micro, and nothing without a positive
/// certificate.
#[must_use]
pub fn certified_fee_micros(certificate: &Certificate, rate: f64) -> i64 {
    if certificate.actual_gain_lower <= 0.0 {
        return 0;
    }
    let fee = rate * certificate.actual_gain_lower;
    if !fee.is_finite() || fee <= 0.0 {
        return 0;
    }
    // Floor: the publisher is never charged more than the certificate allows.
    (fee + 1e-9).floor() as i64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// The validation kit's selection worlds at 100,000 eligible users with a
    /// stratified 90/10 assignment: 40% (1, 1.25), 10% (1, 0), 50% (0, 0).
    fn selection_world(new_first: f64) -> Vec<CertifiedUser> {
        let mut users = Vec::new();
        for index in 0..100_000 {
            let (y0, y1) = if index < 40_000 {
                (1.0, new_first)
            } else if index < 50_000 {
                (1.0, 0.0)
            } else {
                (0.0, 0.0)
            };
            let test = index % 10 == 0;
            let envelope = 1.4;
            let prior: f64 = 1.0;
            users.push(CertifiedUser {
                test,
                outcome: if test { y1 } else { y0 },
                prior: prior.min(envelope),
                envelope,
            });
        }
        users
    }

    #[test]
    fn the_zero_gain_selection_world_certifies_no_positive_gain() {
        let users = selection_world(1.25);
        let certificate = certify(&users, 0.1, 0.01).expect("certificate");
        assert!(certificate.delta_lower <= 0.0, "{certificate:?}");
        assert!(certificate.actual_gain_lower <= 0.0, "{certificate:?}");
        assert_eq!(certified_fee_micros(&certificate, 0.30), 0);
        // The naive positive-ad-user means show +25%; the certificate must
        // not reproduce that population bias.
        assert!(certificate.relative_lift_lower.is_none());
    }

    #[test]
    fn the_harmful_selection_world_certifies_no_positive_gain() {
        let users = selection_world(1.20);
        let certificate = certify(&users, 0.1, 0.01).expect("certificate");
        assert!(certificate.actual_gain_lower <= 0.0, "{certificate:?}");
        assert_eq!(certified_fee_micros(&certificate, 0.30), 0);
    }

    #[test]
    fn a_clear_positive_gain_certifies_and_caps_the_fee() {
        // 600k users, half assigned, +3% with tight envelopes.
        let mut users = Vec::with_capacity(600_000);
        for index in 0..600_000 {
            let y0 = 0.5 + (index % 7) as f64 * 0.01;
            let y1 = y0 * 1.03;
            let test = index % 2 == 0;
            users.push(CertifiedUser {
                test,
                outcome: if test { y1 } else { y0 },
                prior: y0,
                envelope: y0 * 1.4,
            });
        }
        let certificate = certify(&users, 0.5, 0.01).expect("certificate");
        assert!(certificate.delta_lower > 0.0, "{certificate:?}");
        assert!(certificate.actual_gain_lower > 0.0, "{certificate:?}");
        assert_eq!(certificate.method, "empirical Bernstein", "{certificate:?}");
        let fee = certified_fee_micros(&certificate, 0.30);
        assert!(fee > 0);
        let cap = 0.30 * certificate.actual_gain_lower;
        assert!((fee as f64) <= cap + 1.0, "{fee} vs {cap}");
    }

    #[test]
    fn a_missing_or_invalid_envelope_refuses_the_certificate() {
        let mut users = selection_world(1.25);
        users[0].envelope = 0.0; // outcome 1.0 now exceeds the envelope
        assert!(certify(&users, 0.1, 0.01).is_none());
        assert!(certify(&[], 0.1, 0.01).is_none());
        assert!(certify(&selection_world(1.25), 0.0, 0.01).is_none());
        assert!(certify(&selection_world(1.25), 0.1, 0.0).is_none());
    }

    #[test]
    fn monetary_scale_does_not_change_the_relative_certificate() {
        let base = selection_world(1.25);
        let scaled: Vec<CertifiedUser> = base
            .iter()
            .map(|user| CertifiedUser {
                outcome: user.outcome * 7.0,
                prior: user.prior * 7.0,
                envelope: user.envelope * 7.0,
                ..*user
            })
            .collect();
        let a = certify(&base, 0.1, 0.01).expect("a");
        let b = certify(&scaled, 0.1, 0.01).expect("b");
        assert!((a.delta * 7.0 - b.delta).abs() < 1e-9);
        assert!((a.actual_gain_lower * 7.0 - b.actual_gain_lower).abs() < 1e-3);
    }
}
