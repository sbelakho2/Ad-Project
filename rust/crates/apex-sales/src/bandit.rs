//! Message experiments: which email variant to send.
//!
//! Each variant (an [`Arm`]) holds two Beta posteriors:
//!
//! * **value** — per send, the imputed probability of eventually paying
//!   `P(paid | stage reached)` from [`crate::funnel`], so a variant is
//!   judged on revenue rather than opens or raw replies. Rewards are
//!   fractional; a reward `r ∈ [0, 1]` adds `r` to α and `1 − r` to β.
//! * **complaints** — per send, the spam-complaint rate.
//!
//! Selection is Thompson sampling restricted to *safe* arms: an arm whose
//! posterior probability of exceeding the complaint ceiling is at least the
//! safety confidence (after a minimum number of sends) is never chosen. If no
//! arm is safe, nothing is sent: the gate fails closed.
//!
//! In experiment mode, top-two Thompson sampling (Russo, 2016) spends a fixed
//! share of sends on the strongest challenger, which identifies the best arm
//! with far fewer sends than plain Thompson sampling.
//!
//! Posteriors are discounted toward the prior by `γ^days` between updates
//! (Raj and Kalyani, 2017), so a variant that worked last spring cannot
//! coast on stale wins.

use crate::rng::Rng;
use crate::stats::beta_tail_above;

/// Selection behaviour and safety limits.
#[derive(Debug, Clone, PartialEq)]
pub struct BanditConfig {
    /// Value prior `(alpha, beta)`: mean 0.3% per send.
    pub value_prior: (f64, f64),
    /// Complaint prior `(alpha, beta)`: mean 0.05% per send.
    pub complaint_prior: (f64, f64),
    /// Daily discount factor γ.
    pub discount_per_day: f64,
    /// Complaint-rate ceiling.
    pub max_complaint_rate: f64,
    /// Posterior probability above the ceiling that makes an arm unsafe.
    pub safety_confidence: f64,
    /// Sends before the safety test applies.
    pub min_sends_for_safety: f64,
    /// Share of sends that go to the challenger in experiment mode.
    pub top_two_beta: f64,
}

impl Default for BanditConfig {
    fn default() -> Self {
        Self {
            value_prior: (0.3, 100.0),
            complaint_prior: (0.5, 999.5),
            discount_per_day: 0.995,
            max_complaint_rate: 0.001,
            safety_confidence: 0.95,
            min_sends_for_safety: 50.0,
            top_two_beta: 0.5,
        }
    }
}

/// One message variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    /// Template id.
    pub id: String,
    /// Value posterior `(alpha, beta)`.
    pub value: (f64, f64),
    /// Complaint posterior `(alpha, beta)`.
    pub complaints: (f64, f64),
    /// Sends counted (discounted).
    pub sends: f64,
    /// Day of the last update (days since epoch).
    pub last_day: i64,
}

impl Arm {
    /// A fresh arm at the prior.
    pub fn new(id: impl Into<String>, config: &BanditConfig, day: i64) -> Self {
        Self {
            id: id.into(),
            value: config.value_prior,
            complaints: config.complaint_prior,
            sends: 0.0,
            last_day: day,
        }
    }

    fn decay_to(&mut self, day: i64, config: &BanditConfig) {
        let days = (day - self.last_day).max(0);
        if days == 0 {
            return;
        }
        let g = config
            .discount_per_day
            .clamp(0.0, 1.0)
            .powi(i32::try_from(days).unwrap_or(i32::MAX));
        let shrink = |post: (f64, f64), prior: (f64, f64)| {
            (
                prior.0 + g * (post.0 - prior.0),
                prior.1 + g * (post.1 - prior.1),
            )
        };
        self.value = shrink(self.value, config.value_prior);
        self.complaints = shrink(self.complaints, config.complaint_prior);
        self.sends *= g;
        self.last_day = day;
    }

    /// Record one settled send with its imputed reward and whether it drew a
    /// complaint.
    pub fn record(&mut self, reward: f64, complaint: bool, day: i64, config: &BanditConfig) {
        self.decay_to(day, config);
        let r = if reward.is_finite() {
            reward.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.value.0 += r;
        self.value.1 += 1.0 - r;
        if complaint {
            self.complaints.0 += 1.0;
        } else {
            self.complaints.1 += 1.0;
        }
        self.sends += 1.0;
    }

    /// Posterior mean value per send.
    pub fn mean_value(&self) -> f64 {
        self.value.0 / (self.value.0 + self.value.1)
    }

    /// Whether the arm may be sent under the safety rule.
    pub fn is_safe(&self, config: &BanditConfig) -> bool {
        if self.sends < config.min_sends_for_safety {
            return true;
        }
        beta_tail_above(
            config.max_complaint_rate,
            self.complaints.0,
            self.complaints.1,
        ) < config.safety_confidence
    }
}

/// Why an arm was picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    /// Highest Thompson draw.
    Leader,
    /// Top-two challenger.
    Challenger,
}

/// Choose an arm index. `experiment` turns on top-two sampling. `None` when
/// no arm is safe.
pub fn choose(
    arms: &[Arm],
    config: &BanditConfig,
    experiment: bool,
    rng: &mut Rng,
) -> Option<(usize, PickKind)> {
    let safe: Vec<usize> = (0..arms.len())
        .filter(|&i| arms[i].is_safe(config))
        .collect();
    if safe.is_empty() {
        return None;
    }
    let draw = |rng: &mut Rng| -> usize {
        let mut best = safe[0];
        let mut best_v = f64::NEG_INFINITY;
        for &i in &safe {
            let v = rng.beta(arms[i].value.0, arms[i].value.1);
            if v > best_v {
                best_v = v;
                best = i;
            }
        }
        best
    };
    let leader = draw(rng);
    if !experiment || safe.len() < 2 || rng.uniform() >= config.top_two_beta {
        return Some((leader, PickKind::Leader));
    }
    for _ in 0..200 {
        let challenger = draw(rng);
        if challenger != leader {
            return Some((challenger, PickKind::Challenger));
        }
    }
    Some((leader, PickKind::Leader))
}

/// Posterior summary of an experiment.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmReport {
    /// Template id.
    pub id: String,
    /// Posterior mean value per send.
    pub mean_value: f64,
    /// Probability this arm is the best (Monte Carlo).
    pub p_best: f64,
    /// Whether it passes the safety rule.
    pub safe: bool,
}

/// Monte Carlo summary with `draws` joint posterior samples, plus the
/// expected loss of committing to the current leader:
/// `E[max_k θ_k − θ_leader]`. An experiment can stop when the loss is small
/// relative to the leader's value (Scott, 2015).
pub fn report(
    arms: &[Arm],
    config: &BanditConfig,
    draws: usize,
    rng: &mut Rng,
) -> (Vec<ArmReport>, f64) {
    let mut wins = vec![0usize; arms.len()];
    let leader =
        (0..arms.len()).max_by(|&a, &b| arms[a].mean_value().total_cmp(&arms[b].mean_value()));
    let mut loss = 0.0;
    let draws = draws.max(1);
    for _ in 0..draws {
        let samples: Vec<f64> = arms
            .iter()
            .map(|a| rng.beta(a.value.0, a.value.1))
            .collect();
        let best = (0..samples.len()).max_by(|&a, &b| samples[a].total_cmp(&samples[b]));
        if let Some(b) = best {
            wins[b] += 1;
            if let Some(l) = leader {
                loss += samples[b] - samples[l];
            }
        }
    }
    let reports = arms
        .iter()
        .zip(&wins)
        .map(|(a, w)| ArmReport {
            id: a.id.clone(),
            mean_value: a.mean_value(),
            p_best: *w as f64 / draws as f64,
            safe: a.is_safe(config),
        })
        .collect();
    (reports, loss / draws as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arms(config: &BanditConfig) -> Vec<Arm> {
        vec![
            Arm::new("a", config, 0),
            Arm::new("b", config, 0),
            Arm::new("c", config, 0),
        ]
    }

    #[test]
    fn thompson_concentrates_on_the_best_variant() {
        let config = BanditConfig::default();
        let mut arms = arms(&config);
        let truth = [0.002, 0.006, 0.003];
        let mut rng = Rng::seeded(4);
        let mut picks = [0usize; 3];
        for t in 0..30_000 {
            let (i, _) = choose(&arms, &config, false, &mut rng).unwrap();
            picks[i] += 1;
            let reward = if rng.uniform() < truth[i] { 1.0 } else { 0.0 };
            arms[i].record(reward, false, t / 1000, &config);
        }
        assert!(picks[1] > picks[0] + picks[2], "{picks:?}");
        let (rep, loss) = report(&arms, &config, 2000, &mut rng);
        assert!(rep[1].p_best > 0.8, "{rep:?}");
        assert!((0.0..0.001).contains(&loss));
    }

    #[test]
    fn top_two_keeps_testing_the_runner_up() {
        let config = BanditConfig::default();
        let mut arms = arms(&config);
        for _ in 0..2000 {
            arms[0].record(0.004, false, 0, &config);
            arms[1].record(0.003, false, 0, &config);
            arms[2].record(0.0, false, 0, &config);
        }
        let mut rng = Rng::seeded(8);
        let mut challengers = 0;
        let n = 4000;
        for _ in 0..n {
            if let Some((_, PickKind::Challenger)) = choose(&arms, &config, true, &mut rng) {
                challengers += 1;
            }
        }
        let share = challengers as f64 / n as f64;
        assert!((share - 0.5).abs() < 0.05, "{share}");
    }

    #[test]
    fn complaint_heavy_arms_are_excluded_and_all_unsafe_fails_closed() {
        let config = BanditConfig::default();
        let mut arms = arms(&config);
        for i in 0..400 {
            arms[0].record(1.0, i % 40 == 0, 0, &config); // 2.5% complaints
        }
        assert!(!arms[0].is_safe(&config));
        assert!(arms[1].is_safe(&config));
        let mut rng = Rng::seeded(2);
        for _ in 0..200 {
            let (i, _) = choose(&arms, &config, false, &mut rng).unwrap();
            assert_ne!(i, 0);
        }
        let unsafe_only = vec![arms[0].clone()];
        assert!(choose(&unsafe_only, &config, false, &mut rng).is_none());
        assert!(choose(&[], &config, false, &mut rng).is_none());
    }

    #[test]
    fn discounting_forgets_stale_wins() {
        let config = BanditConfig::default();
        let mut arm = Arm::new("a", &config, 0);
        for _ in 0..1000 {
            arm.record(0.05, false, 0, &config);
        }
        let fresh = arm.mean_value();
        arm.record(0.0, false, 1000, &config);
        let prior_mean = config.value_prior.0 / (config.value_prior.0 + config.value_prior.1);
        assert!((arm.mean_value() - prior_mean).abs() < (fresh - prior_mean).abs() / 10.0);
        assert!(arm.sends < 10.0);
        arm.record(f64::NAN, false, 1000, &config);
        assert!(arm.mean_value().is_finite());
    }
}
