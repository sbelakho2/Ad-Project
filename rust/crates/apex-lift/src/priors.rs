//! Hierarchical lift priors and the Thompson-sampling picker. Buckets shrink
//! toward their parent (lever plus platform), parents shrink toward the
//! skeptical global prior (mean 0, standard deviation 3%), and stored results
//! carry no publisher names.

use std::collections::BTreeMap;

use apex_core::Micros;

use crate::rng::SplitMix64;
use crate::stats::estimate;

/// The bucket a test belongs to. No publisher identifiers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub lever: String,
    pub ad_format: String,
    pub country_tier: String,
    pub platform: String,
}

/// A normal posterior over lift for one bucket.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Posterior {
    pub mean: f64,
    pub sd: f64,
    pub observations: u64,
}

/// The skeptical global prior: mean 0, sd 3%.
pub const SKEPTICAL_SD: f64 = 0.03;
/// Observation standard errors below this are treated as this value, so a
/// single degenerate run cannot pin a bucket.
const MIN_SE: f64 = 1e-3;

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct BucketData {
    /// Number of observations.
    n: u64,
    /// Sum of observation precisions `1 / se^2`.
    precision: f64,
    /// Sum of `lift / se^2`.
    weighted: f64,
}

/// The model: bucket-level data plus the hierarchical lookup.
#[derive(Debug, Clone, Default)]
pub struct LiftPriors {
    buckets: BTreeMap<Bucket, BucketData>,
}

/// The wire form: a list of bucket/data pairs, because JSON object keys must
/// be strings and a bucket is a struct.
#[derive(serde::Serialize, serde::Deserialize)]
struct PriorsWire {
    buckets: Vec<(Bucket, BucketData)>,
}

impl serde::Serialize for LiftPriors {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PriorsWire {
            buckets: self
                .buckets
                .iter()
                .map(|(bucket, data)| (bucket.clone(), *data))
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for LiftPriors {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = PriorsWire::deserialize(deserializer)?;
        Ok(Self {
            buckets: wire.buckets.into_iter().collect(),
        })
    }
}

impl LiftPriors {
    /// The posterior for a bucket, pooled through its parent (the same lever
    /// and platform across formats and tiers) and the skeptical global prior.
    pub fn posterior(&self, bucket: &Bucket) -> Posterior {
        let data = self.buckets.get(bucket).copied().unwrap_or_default();
        // Parent: every sibling bucket's data, no publisher names, plus the
        // global skeptical prior.
        let mut parent_precision = 1.0 / (SKEPTICAL_SD * SKEPTICAL_SD);
        let mut parent_weighted = 0.0;
        let mut parent_observations = 0u64;
        for (sibling, sibling_data) in &self.buckets {
            if sibling.lever == bucket.lever
                && sibling.platform == bucket.platform
                && sibling != bucket
            {
                parent_precision += sibling_data.precision;
                parent_weighted += sibling_data.weighted;
                parent_observations += sibling_data.n;
            }
        }
        let parent_mean = parent_weighted / parent_precision;
        // Bucket posterior: the skeptical spread around the parent mean,
        // combined with the bucket's own observations.
        let prior_precision = 1.0 / (SKEPTICAL_SD * SKEPTICAL_SD);
        let precision = prior_precision + data.precision;
        Posterior {
            mean: (parent_mean * prior_precision + data.weighted) / precision,
            sd: (1.0 / precision).sqrt(),
            observations: data.n + parent_observations,
        }
    }

    /// Record one finished test (lift and its standard error) in its bucket.
    /// The observation's standard error is floored so degenerate runs cannot
    /// dominate the posterior.
    pub fn observe(&mut self, bucket: &Bucket, lift: f64, standard_error: f64) {
        if !lift.is_finite() {
            return;
        }
        let se = if standard_error.is_finite() {
            standard_error.max(MIN_SE)
        } else {
            MIN_SE
        };
        let precision = 1.0 / (se * se);
        let entry = self.buckets.entry(bucket.clone()).or_default();
        entry.n += 1;
        entry.precision += precision;
        entry.weighted += lift * precision;
    }

    /// Record a decision outcome directly from its daily ratios, which is the
    /// pipeline from the engine's result record into the priors.
    pub fn observe_ratios(&mut self, bucket: &Bucket, daily_log_ratios: &[f64]) {
        if let Some(estimate) = estimate(daily_log_ratios) {
            let se = estimate.sd / (estimate.days as f64).sqrt();
            self.observe(bucket, estimate.lift, se);
        }
    }

    /// How many publisher-name-free observations the model holds.
    pub fn total_observations(&self) -> u64 {
        self.buckets.values().map(|d| d.n).sum()
    }
}

/// One candidate change for the picker.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub change_id: String,
    pub bucket: Bucket,
    /// Monthly revenue the change's scope touches, in micros.
    pub scope_revenue: Micros,
    /// Probability the change passes the guardrails, 0..=1.
    pub pass_probability: f64,
    /// Whether the change is eligible for the autopilot at all.
    pub eligible: bool,
}

/// Score a sampled lift: `L * P(passes guardrails) * R_scope * 12 * rho`.
/// The sampled lift is used exactly as drawn; an ineligible probability or a
/// non-finite input scores zero rather than poisoning the comparison.
pub fn score(sampled_lift: f64, candidate: &Candidate, rate_bp: u16) -> f64 {
    if !sampled_lift.is_finite() || !candidate.pass_probability.is_finite() {
        return 0.0;
    }
    let probability = candidate.pass_probability.clamp(0.0, 1.0);
    let rate = rate_bp as f64 / 10_000.0;
    sampled_lift * probability * candidate.scope_revenue as f64 * 12.0 * rate
}

/// Pick the next test per ad unit with Thompson sampling: draw a lift from
/// each eligible candidate's posterior and take the highest score. The draw
/// keeps the engine learning instead of retesting the same safe change.
pub fn pick_next(
    priors: &LiftPriors,
    candidates: &[Candidate],
    rate_bp: u16,
    seed: u64,
) -> Option<(String, f64)> {
    let mut rng = SplitMix64::new(seed);
    let mut best: Option<(String, f64)> = None;
    for candidate in candidates.iter().filter(|c| c.eligible) {
        let posterior = priors.posterior(&candidate.bucket);
        let sampled = posterior.mean + posterior.sd * rng.next_normal();
        let value = score(sampled, candidate, rate_bp);
        if best
            .as_ref()
            .map(|(_, previous)| value.total_cmp(previous).is_gt())
            .unwrap_or(true)
        {
            best = Some((candidate.change_id.clone(), value));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn priors_round_trip_through_json() {
        let target = bucket();
        let mut priors = LiftPriors::default();
        priors.observe(&target, 0.04, 0.01);
        let json = serde_json::to_string(&priors).expect("serialize");
        // A bucket key is a struct, so the wire form lists pairs.
        assert!(json.contains("\"buckets\""), "{json}");
        let back: LiftPriors = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.posterior(&target), priors.posterior(&target));

        let empty: LiftPriors = serde_json::from_str("{\"buckets\":[]}").expect("empty");
        assert_eq!(empty.posterior(&target).mean, 0.0);
    }

    fn bucket() -> Bucket {
        Bucket {
            lever: "floor".into(),
            ad_format: "rewarded".into(),
            country_tier: "t1".into(),
            platform: "applovin_max".into(),
        }
    }

    fn sibling(tier: &str) -> Bucket {
        Bucket {
            country_tier: tier.into(),
            ..bucket()
        }
    }

    fn candidate(id: &str, scope: Micros) -> Candidate {
        Candidate {
            change_id: id.into(),
            bucket: bucket(),
            scope_revenue: scope,
            pass_probability: 1.0,
            eligible: true,
        }
    }

    #[test]
    fn the_prior_is_skeptical_and_pools_through_siblings() {
        let mut priors = LiftPriors::default();
        let prior = priors.posterior(&bucket());
        assert_eq!(prior.mean, 0.0);
        assert_eq!(prior.sd, SKEPTICAL_SD);
        // A sibling's results move an unseen bucket's posterior: that is the
        // hierarchical pooling.
        priors.observe(&sibling("t2"), 0.04, 0.005);
        let pooled = priors.posterior(&bucket());
        assert!(pooled.mean > 0.0 && pooled.mean < 0.04, "{}", pooled.mean);
        assert_eq!(pooled.observations, 1);
        assert_eq!(priors.total_observations(), 1);
    }

    #[test]
    fn observations_shrink_the_bucket_sd_without_pinning_it() {
        let mut priors = LiftPriors::default();
        for _ in 0..3 {
            priors.observe(&bucket(), 0.02, 0.0);
        }
        let posterior = priors.posterior(&bucket());
        assert!(
            posterior.mean > 0.0 && posterior.mean < 0.02,
            "{}",
            posterior.mean
        );
        assert!(posterior.sd < SKEPTICAL_SD - 0.02, "{}", posterior.sd);
        assert!(posterior.sd > 0.0);
    }

    #[test]
    fn ratio_pipeline_feeds_the_priors() {
        let mut priors = LiftPriors::default();
        priors.observe_ratios(
            &bucket(),
            &[0.03, 0.012, -0.004, 0.026, 0.041, 0.019, 0.008],
        );
        assert_eq!(priors.total_observations(), 1);
        let posterior = priors.posterior(&bucket());
        assert!(posterior.mean > 0.0);
    }

    #[test]
    fn the_score_matches_the_formula_and_clamps_bad_probabilities() {
        let value = score(0.02, &candidate("c", 10_000_000_000), 2000);
        assert!((value - 0.02 * 10_000_000_000.0 * 12.0 * 0.20).abs() < 1e-3);
        // A negative draw scores negative, exactly as drawn.
        assert!(score(-0.05, &candidate("c", 10_000_000_000), 2000) < 0.0);
        let mut bad = candidate("c", 10_000_000_000);
        bad.pass_probability = 5.0;
        assert!((score(0.02, &bad, 2000) - value).abs() < 1e-3);
        bad.pass_probability = f64::NAN;
        assert_eq!(score(0.02, &bad, 2000), 0.0);
    }

    #[test]
    fn thompson_ignores_nan_candidates_and_stays_deterministic() {
        // With a learned positive belief the sampled lifts are positive and
        // the larger scope wins; a NaN candidate can never take the pick.
        let mut priors = LiftPriors::default();
        priors.observe(&bucket(), 0.03, 0.001);
        let mut nan = candidate("nan", 9_000_000_000_000);
        nan.pass_probability = f64::NAN;
        let candidates = vec![
            nan,
            candidate("small", 1_000_000_000),
            candidate("big", 9_000_000_000),
        ];
        let picked = pick_next(&priors, &candidates, 2000, 5).expect("pick");
        assert_eq!(picked.0, "big");
        assert_eq!(pick_next(&priors, &candidates, 2000, 5), Some(picked));

        let mut ineligible = candidates;
        ineligible[2].eligible = false;
        assert_eq!(
            pick_next(&priors, &ineligible, 2000, 5).expect("pick").0,
            "small"
        );
        assert!(pick_next(&priors, &[], 2000, 5).is_none());
    }
}
