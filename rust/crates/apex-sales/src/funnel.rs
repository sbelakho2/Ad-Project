//! The value of a lead: what an interested reply is worth in expected paid
//! autopilot revenue.
//!
//! The funnel after interest has three transitions — interest → audit
//! requested → platform connected → paid — each a Beta–Binomial with a
//! documented prior. Segments (size bands) are partially pooled toward the
//! global rate by empirical-Bayes shrinkage: a band's posterior is
//! `Beta(m·μ + k, m·[1 − μ] + n − k)`, where `μ` is the global posterior mean
//! and `m` the pooling strength, so a band with three observations borrows
//! almost everything from the global rate and a band with three hundred
//! speaks for itself.
//!
//! The same quantities impute delayed rewards for the message bandit: a
//! contact who has reached stage *s* is credited `P(paid | s)` — the
//! posterior expected value of the eventual binary outcome — instead of
//! waiting months for the invoice (a value-to-go surrogate).

use crate::signals::SizeBand;

/// Funnel stages after a send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// Sent; no interest yet.
    Contacted,
    /// Replied with interest.
    Interested,
    /// Requested the free audit.
    AuditRequested,
    /// Connected a platform to the autopilot.
    Connected,
    /// Paid.
    Paid,
}

/// Beta–Binomial counts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Counts {
    /// Successes.
    pub k: f64,
    /// Trials.
    pub n: f64,
}

/// The post-interest funnel.
#[derive(Debug, Clone, PartialEq)]
pub struct FunnelModel {
    /// Prior `(alpha, beta)` per transition: interest→audit,
    /// audit→connected, connected→paid.
    pub prior: [(f64, f64); 3],
    /// Global counts per transition.
    pub global: [Counts; 3],
    /// Counts per size band, in [`SizeBand::ALL`] order.
    pub by_band: [[Counts; 3]; 4],
    /// Pooling strength `m` in pseudo-observations.
    pub pooling: f64,
    /// Lifetime value of a paying customer per band (USD), in
    /// [`SizeBand::ALL`] order.
    pub ltv: [f64; 4],
}

impl Default for FunnelModel {
    /// Priors worth 10 observations each: half of interested publishers
    /// request the audit, 40% of those connect, 60% of connected accounts
    /// pay. Lifetime values are 12 months of the lift fee at 80% retention
    /// for each band's typical revenue.
    fn default() -> Self {
        Self {
            prior: [(5.0, 5.0), (4.0, 6.0), (6.0, 4.0)],
            global: [Counts::default(); 3],
            by_band: [[Counts::default(); 3]; 4],
            pooling: 20.0,
            ltv: [2_400.0, 1_200.0, 6_000.0, 18_000.0],
        }
    }
}

fn band_index(band: SizeBand) -> usize {
    match band {
        SizeBand::Unknown => 0,
        SizeBand::Small => 1,
        SizeBand::Core => 2,
        SizeBand::Large => 3,
    }
}

impl FunnelModel {
    /// Record one settled transition (`t` in 0..3) for a band.
    pub fn observe(&mut self, band: SizeBand, t: usize, success: bool) {
        if t >= 3 {
            return;
        }
        let s = if success { 1.0 } else { 0.0 };
        self.global[t].k += s;
        self.global[t].n += 1.0;
        let b = &mut self.by_band[band_index(band)][t];
        b.k += s;
        b.n += 1.0;
    }

    /// Global posterior mean of transition `t`.
    pub fn global_rate(&self, t: usize) -> f64 {
        let t = t.min(2);
        let (a, b) = self.prior[t];
        (a + self.global[t].k) / (a + b + self.global[t].n)
    }

    /// Partially pooled posterior `(alpha, beta)` of transition `t` for a band.
    pub fn band_posterior(&self, band: SizeBand, t: usize) -> (f64, f64) {
        let t = t.min(2);
        let mu = self.global_rate(t);
        let c = self.by_band[band_index(band)][t];
        (
            self.pooling * mu + c.k,
            self.pooling * (1.0 - mu) + c.n - c.k,
        )
    }

    /// Posterior mean of transition `t` for a band.
    pub fn band_rate(&self, band: SizeBand, t: usize) -> f64 {
        let (a, b) = self.band_posterior(band, t);
        a / (a + b)
    }

    /// `P(paid | reached stage)` for a band.
    pub fn p_paid_given(&self, band: SizeBand, stage: Stage) -> f64 {
        let from = match stage {
            Stage::Contacted => return 0.0,
            Stage::Interested => 0,
            Stage::AuditRequested => 1,
            Stage::Connected => 2,
            Stage::Paid => return 1.0,
        };
        (from..3).map(|t| self.band_rate(band, t)).product()
    }

    /// Lifetime value for a band.
    pub fn ltv(&self, band: SizeBand) -> f64 {
        self.ltv[band_index(band)]
    }

    /// Expected revenue of an interested reply from a band.
    pub fn value_of_interest(&self, band: SizeBand) -> f64 {
        self.p_paid_given(band, Stage::Interested) * self.ltv(band)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priors_give_the_documented_chain() {
        let f = FunnelModel::default();
        let p = f.p_paid_given(SizeBand::Core, Stage::Interested);
        assert!((p - 0.5 * 0.4 * 0.6).abs() < 1e-12);
        assert_eq!(f.p_paid_given(SizeBand::Core, Stage::Contacted), 0.0);
        assert_eq!(f.p_paid_given(SizeBand::Core, Stage::Paid), 1.0);
        assert!((f.p_paid_given(SizeBand::Core, Stage::Connected) - 0.6).abs() < 1e-12);
        assert!((f.value_of_interest(SizeBand::Core) - 0.12 * 6_000.0).abs() < 1e-9);
        assert!(f.value_of_interest(SizeBand::Large) > f.value_of_interest(SizeBand::Small));
    }

    #[test]
    fn small_bands_borrow_from_the_global_rate_large_ones_speak() {
        let mut f = FunnelModel::default();
        // Globally, audits convert to connections at 30%.
        for i in 0..1000 {
            f.observe(SizeBand::Core, 1, i % 10 < 3);
        }
        // Three Large audits, all connected.
        for _ in 0..3 {
            f.observe(SizeBand::Large, 1, true);
        }
        let large = f.band_rate(SizeBand::Large, 1);
        assert!(large > 0.3 && large < 0.45, "shrunk toward 30%: {large}");
        // Three hundred Small audits at 60%.
        for i in 0..300 {
            f.observe(SizeBand::Small, 1, i % 10 < 6);
        }
        let small = f.band_rate(SizeBand::Small, 1);
        assert!(small > 0.55, "{small}");
        f.observe(SizeBand::Small, 9, true);
        assert!((f.global_rate(5) - f.global_rate(2)).abs() < 1e-12);
    }
}
