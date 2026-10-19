//! Hierarchical lift priors and the Thompson-sampling picker. Buckets shrink
//! toward their parent (lever plus platform), parents shrink toward the
//! skeptical global prior (mean 0, standard deviation 3%), and stored results
//! carry no publisher names.

use std::collections::{BTreeMap, HashMap};

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

// Sibling scans in the uncached posterior. The cache test asserts zero visits:
// once the parent aggregates are built, no query walks the bucket map.
#[cfg(test)]
thread_local! {
    static SIBLING_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_sibling_visits() {
    SIBLING_VISITS.with(|count| count.set(0));
}

#[cfg(test)]
fn sibling_visits() -> u64 {
    SIBLING_VISITS.with(std::cell::Cell::get)
}

#[derive(Debug, Clone, Copy, Default)]
struct ParentAggregate {
    precision: f64,
    weighted: f64,
    observations: u64,
}

/// Parent `(lever, platform)` aggregates built in one pass, so a proposal run
/// can answer every bucket posterior without rescanning the model.
///
/// [`PosteriorCache::posterior`] reproduces the sibling-exclusive semantics of
/// [`LiftPriors::posterior`] by subtracting the queried bucket's own
/// contribution from its parent total. The cache borrows the model it was
/// built from; build it again after mutating the priors.
#[derive(Debug, Clone, Default)]
pub struct PosteriorCache<'a> {
    parents: HashMap<(&'a str, &'a str), ParentAggregate>,
}

impl<'a> PosteriorCache<'a> {
    /// Build the parent aggregates once: O(buckets).
    pub fn new(priors: &'a LiftPriors) -> Self {
        let mut parents: HashMap<(&'a str, &'a str), ParentAggregate> = HashMap::new();
        for (bucket, data) in &priors.buckets {
            let aggregate = parents
                .entry((bucket.lever.as_str(), bucket.platform.as_str()))
                .or_default();
            aggregate.precision += data.precision;
            aggregate.weighted += data.weighted;
            aggregate.observations += data.n;
        }
        Self { parents }
    }

    /// The posterior for one bucket, using the cached parent aggregates.
    pub fn posterior(&self, priors: &LiftPriors, bucket: &Bucket) -> Posterior {
        let data = priors.buckets.get(bucket).copied().unwrap_or_default();
        let parent = self
            .parents
            .get(&(bucket.lever.as_str(), bucket.platform.as_str()))
            .copied()
            .unwrap_or_default();
        // The parent total includes this bucket; subtract its contribution so
        // the result is sibling-exclusive, exactly as the uncached lookup.
        let parent_precision =
            1.0 / (SKEPTICAL_SD * SKEPTICAL_SD) + (parent.precision - data.precision);
        let parent_weighted = parent.weighted - data.weighted;
        let parent_observations = parent.observations.saturating_sub(data.n);
        let parent_mean = parent_weighted / parent_precision;
        let prior_precision = 1.0 / (SKEPTICAL_SD * SKEPTICAL_SD);
        let precision = prior_precision + data.precision;
        Posterior {
            mean: (parent_mean * prior_precision + data.weighted) / precision,
            sd: (1.0 / precision).sqrt(),
            observations: data.n + parent_observations,
        }
    }
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
            #[cfg(test)]
            SIBLING_VISITS.with(|count| count.set(count.get() + 1));
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
    // One parent-aggregate pass for the whole candidate list: scoring is no
    // longer O(candidates x buckets).
    let cache = PosteriorCache::new(priors);
    for candidate in candidates.iter().filter(|c| c.eligible) {
        let posterior = cache.posterior(priors, &candidate.bucket);
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
    fn cached_posterior_matches_sibling_exclusive_priors_on_randomized_buckets() {
        // Dyadic observations keep every aggregate exactly representable, so
        // "precompute the parent total, subtract this bucket" is bit-for-bit
        // the same as the uncached scan that skips the bucket.
        let mut state = 0x5eed_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            state >> 11
        };
        let mut priors = LiftPriors::default();
        let levers = ["floor", "waterfall", "mediation", "format"];
        let platforms = ["applovin_max", "levelplay", "admob", "other"];
        for i in 0..1_200usize {
            let bucket = Bucket {
                lever: levers[i % levers.len()].to_string(),
                ad_format: format!("format{}", i % 13),
                country_tier: format!("tier{i}"),
                platform: platforms[(i / 3) % platforms.len()].to_string(),
            };
            let se = 0.5 / f64::from(1u32 << (next() % 4));
            let lift = ((next() % 4_001) as f64 - 2_000.0) / 1_024.0;
            priors.observe(&bucket, lift, se);
        }
        let cache = PosteriorCache::new(&priors);
        assert_eq!(priors.buckets.len(), 1_200);
        for bucket in priors.buckets.keys() {
            assert_eq!(
                cache.posterior(&priors, bucket),
                priors.posterior(bucket),
                "{bucket:?}"
            );
        }
        // A bucket that has no data of its own still pools its siblings.
        let unseen = Bucket {
            lever: "floor".into(),
            ad_format: "unseen".into(),
            country_tier: "unseen".into(),
            platform: "applovin_max".into(),
        };
        assert_eq!(cache.posterior(&priors, &unseen), priors.posterior(&unseen));
    }

    #[test]
    fn ten_thousand_cached_posteriors_never_scan_siblings() {
        let mut priors = LiftPriors::default();
        for i in 0..10_000usize {
            let bucket = Bucket {
                lever: if i % 2 == 0 { "floor" } else { "other" }.to_string(),
                ad_format: format!("format{}", i % 25),
                country_tier: format!("tier{i}"),
                platform: if i % 3 == 0 { "applovin_max" } else { "admob" }.to_string(),
            };
            priors.observe(&bucket, 0.01, 0.5);
        }
        let cache = PosteriorCache::new(&priors);
        assert_eq!(priors.buckets.len(), 10_000);
        reset_sibling_visits();
        let started = std::time::Instant::now();
        for bucket in priors.buckets.keys() {
            let posterior = cache.posterior(&priors, bucket);
            assert!(posterior.sd.is_finite() && posterior.sd > 0.0);
        }
        let elapsed = started.elapsed();
        println!(
            "cached_posteriors buckets={} cached_queries={} sibling_visits={} elapsed={elapsed:?}",
            priors.buckets.len(),
            priors.buckets.len(),
            sibling_visits()
        );
        assert_eq!(
            sibling_visits(),
            0,
            "a cached query must not walk the bucket map"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "10k cached queries should stay well under the scan budget: {elapsed:?}"
        );
    }

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

    #[test]
    fn non_finite_lifts_teach_nothing_and_non_finite_errors_floor() {
        let mut priors = LiftPriors::default();
        priors.observe(&bucket(), f64::NAN, 0.01);
        priors.observe(&bucket(), f64::INFINITY, 0.01);
        assert_eq!(
            priors.total_observations(),
            0,
            "non-finite lifts are refused"
        );
        // A non-finite standard error floors to MIN_SE instead of poisoning
        // the precision sums.
        priors.observe(&bucket(), 0.02, f64::NAN);
        assert_eq!(priors.total_observations(), 1);
        let posterior = priors.posterior(&bucket());
        assert!(
            posterior.mean > 0.0 && posterior.mean < 0.02,
            "{}",
            posterior.mean
        );
        assert!(posterior.sd.is_finite() && posterior.sd > 0.0);
    }
}
