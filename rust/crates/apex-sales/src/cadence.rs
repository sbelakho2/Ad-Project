//! When to send, and when to stop following up.
//!
//! # Send time
//!
//! Each local business hour (8:00 to 18:00, Monday to Friday) has a Beta
//! posterior over the interest rate of sends made then, partially pooled
//! toward the all-hours rate with strength [`SendTimeModel::pooling`]. The
//! slot is chosen by Thompson sampling, so hours with little data are still
//! explored and hours that clearly underperform stop being used.
//!
//! # Follow-ups: an optimal-stopping rule
//!
//! For step *k* of a sequence, `h_k` is the probability of an interested
//! reply to that touch given no reply so far (a discrete-time hazard) and
//! `c_k` the probability of a spam complaint. Both are Beta posteriors per
//! step. The next touch is sent only if its expected value is positive:
//!
//! ```text
//! E[h_k] · V_interest − cost_per_touch − E[c_k] · C_complaint > 0
//! ```
//!
//! where `V_interest` comes from [`crate::funnel`] for the publisher's band.
//! Hazards decline with each touch, so the rule stops sequences early for
//! low-value publishers and lets high-value ones run to the hard cap.

use crate::rng::Rng;

/// First and last local sending hour (inclusive).
pub const HOURS: std::ops::RangeInclusive<u32> = 8..=18;

/// Send-time model over local business hours.
#[derive(Debug, Clone, PartialEq)]
pub struct SendTimeModel {
    /// `(interested, sends)` per hour, index `hour − 8`.
    pub counts: [(f64, f64); 11],
    /// Prior for the all-hours rate `(alpha, beta)`.
    pub prior: (f64, f64),
    /// Pooling strength toward the all-hours rate.
    pub pooling: f64,
    /// Hour used when nothing has been learned yet.
    pub default_hour: u32,
}

impl Default for SendTimeModel {
    fn default() -> Self {
        Self {
            counts: [(0.0, 0.0); 11],
            prior: (1.0, 99.0),
            pooling: 200.0,
            default_hour: 10,
        }
    }
}

impl SendTimeModel {
    /// Record a settled send at local `hour`.
    pub fn observe(&mut self, hour: u32, interested: bool) {
        if let Some(c) = self
            .counts
            .get_mut(usize::try_from(hour.saturating_sub(8)).unwrap_or(usize::MAX))
            .filter(|_| HOURS.contains(&hour))
        {
            c.1 += 1.0;
            if interested {
                c.0 += 1.0;
            }
        }
    }

    fn pooled_rate(&self) -> f64 {
        let (k, n) = self
            .counts
            .iter()
            .fold((0.0, 0.0), |(k, n), (a, b)| (k + a, n + b));
        (self.prior.0 + k) / (self.prior.0 + self.prior.1 + n)
    }

    /// Posterior `(alpha, beta)` for an hour slot index.
    pub fn slot_posterior(&self, slot: usize) -> (f64, f64) {
        let mu = self.pooled_rate();
        let (k, n) = self.counts.get(slot).copied().unwrap_or((0.0, 0.0));
        (self.pooling * mu + k, self.pooling * (1.0 - mu) + n - k)
    }

    /// Choose a local hour by Thompson sampling; the default hour while no
    /// send has been observed.
    pub fn choose_hour(&self, rng: &mut Rng) -> u32 {
        let total: f64 = self.counts.iter().map(|c| c.1).sum();
        if total == 0.0 {
            return self.default_hour;
        }
        let mut best = self.default_hour;
        let mut best_v = f64::NEG_INFINITY;
        for (slot, hour) in HOURS.enumerate() {
            let (a, b) = self.slot_posterior(slot);
            let v = rng.beta(a, b);
            if v > best_v {
                best_v = v;
                best = hour;
            }
        }
        best
    }
}

/// Per-step reply and complaint hazards.
#[derive(Debug, Clone, PartialEq)]
pub struct StepModel {
    /// Interest hazard posterior `(alpha, beta)` per step (index 0 = first
    /// touch).
    pub interest: Vec<(f64, f64)>,
    /// Complaint posterior per step.
    pub complaint: Vec<(f64, f64)>,
}

impl StepModel {
    /// Documented priors for `steps` touches: interest hazards of 1.2%,
    /// 0.6%, 0.4% and 0.3% (each worth 300 sends), complaints of 0.05%.
    pub fn with_steps(steps: usize) -> Self {
        let base = [0.012, 0.006, 0.004, 0.003];
        let interest = (0..steps)
            .map(|k| {
                let h = base.get(k).copied().unwrap_or(0.002);
                (h * 300.0, (1.0 - h) * 300.0)
            })
            .collect();
        let complaint = (0..steps).map(|_| (1.0, 1999.0)).collect();
        Self {
            interest,
            complaint,
        }
    }

    /// Record a settled touch at step `k`.
    pub fn observe(&mut self, k: usize, interested: bool, complaint: bool) {
        if let Some(p) = self.interest.get_mut(k) {
            if interested {
                p.0 += 1.0;
            } else {
                p.1 += 1.0;
            }
        }
        if let Some(c) = self.complaint.get_mut(k) {
            if complaint {
                c.0 += 1.0;
            } else {
                c.1 += 1.0;
            }
        }
    }

    /// Expected value of sending touch `k`. `None` past the last step.
    pub fn touch_value(
        &self,
        k: usize,
        value_interest: f64,
        cost: f64,
        complaint_cost: f64,
    ) -> Option<f64> {
        let (a, b) = *self.interest.get(k)?;
        let (ca, cb) = *self.complaint.get(k)?;
        Some(a / (a + b) * value_interest - cost - ca / (ca + cb) * complaint_cost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_time_learns_the_best_hour() {
        let mut m = SendTimeModel::default();
        let mut rng = Rng::seeded(6);
        assert_eq!(m.choose_hour(&mut rng), 10);
        for hour in HOURS {
            let rate = if hour == 15 { 0.04 } else { 0.01 };
            for i in 0..3000 {
                m.observe(hour, (i as f64) < rate * 3000.0);
            }
        }
        m.observe(22, true);
        m.observe(3, true);
        let mut picks = 0;
        for _ in 0..500 {
            if m.choose_hour(&mut rng) == 15 {
                picks += 1;
            }
        }
        assert!(picks > 450, "{picks}");
        let (a, b) = m.slot_posterior(99);
        assert!(a > 0.0 && b > 0.0);
    }

    #[test]
    fn stopping_rule_depends_on_lead_value() {
        let m = StepModel::with_steps(5);
        let cost = 0.5;
        let complaint_cost = 200.0;
        // A $720 interested reply (core band) justifies every touch...
        for k in 0..4 {
            assert!(
                m.touch_value(k, 720.0, cost, complaint_cost).unwrap() > 0.0,
                "{k}"
            );
        }
        // ...a $144 one (small band) stops after the second touch.
        assert!(m.touch_value(0, 144.0, cost, complaint_cost).unwrap() > 0.0);
        assert!(m.touch_value(3, 144.0, cost, complaint_cost).unwrap() < 0.0);
        assert!(m.touch_value(5, 720.0, cost, complaint_cost).is_none());
    }

    #[test]
    fn complaints_on_a_step_shut_it_down() {
        let mut m = StepModel::with_steps(3);
        let before = m.touch_value(2, 720.0, 0.5, 200.0).unwrap();
        for i in 0..500 {
            m.observe(2, false, i % 20 == 0);
        }
        let after = m.touch_value(2, 720.0, 0.5, 200.0).unwrap();
        assert!(before > 0.0 && after < 0.0, "{before} {after}");
        m.observe(9, true, true);
    }
}
