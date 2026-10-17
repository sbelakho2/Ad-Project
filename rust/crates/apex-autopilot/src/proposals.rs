//! Proposal generation: turn an audit snapshot plus the lift priors into the
//! candidate changes a cycle can test.
//!
//! The generator is deliberately narrow: it proposes country-floor moves from
//! the audit snapshot, scores each candidate with one Thompson draw from the
//! hierarchical posterior of its bucket, and keeps only draws inside the
//! policy's `(0, max_step_bp]` window -- the same window the write loop
//! enforces -- ranked by the draw. The draw is deterministic in the cycle's
//! seed, so a replayed cycle produces the same plan, and it is what lets a
//! fresh account propose: the skeptical global prior has mean zero, and a
//! gate on the mean would keep yielding an empty list.

use apex_audit::Snapshot;
use apex_lift::candidates::floor_candidates;
use apex_lift::priors::{Bucket, LiftPriors};
use apex_lift::rng::SplitMix64;

use crate::model::{Action, AutopilotPolicy, Proposal};

/// Generate floor-move proposals from `snapshot`, scored and ranked by one
/// Thompson draw per candidate from its bucket's posterior. `seed` is the
/// cycle's identity (the runner's `now_unix_ms as u64`, the same seam the
/// settle path seeds its bootstrap with), so replaying a cycle reproduces
/// the plan.
pub fn floor_proposals(
    snapshot: &Snapshot,
    priors: &LiftPriors,
    policy: &AutopilotPolicy,
    seed: u64,
) -> Vec<Proposal> {
    let platform = snapshot.platform.as_str().to_string();
    let cap_pct = (policy.max_step_bp as f64) / 100.0;
    let mut rng = SplitMix64::new(seed);
    let mut out = Vec::new();
    for unit in &snapshot.ad_units {
        if !unit.enabled {
            continue;
        }
        let Some(floors) = &unit.country_floors else {
            continue;
        };
        let prev_value = serde_json::to_string(floors).unwrap_or_default();
        for floor in floors {
            let bucket = Bucket {
                lever: "floor".to_string(),
                ad_format: format!("{:?}", unit.format).to_lowercase(),
                country_tier: floor.country.clone(),
                platform: platform.clone(),
            };
            let posterior = priors.posterior(&bucket);
            for candidate in floor_candidates(&floor.country, floor.floor_ecpm, None) {
                // The policy's magnitude cap is unchanged: a candidate that
                // moves the floor further than the cap never reaches the draw.
                if candidate.move_pct.abs() > cap_pct {
                    continue;
                }
                // One Thompson draw per candidate: `lift ~ N(mean, sd)`. The
                // draw -- not the posterior mean -- decides whether and where
                // the candidate is proposed: the skeptical prior has mean 0,
                // so a mean gate could never propose for a fresh account,
                // while the draw's spread keeps exploring. Even an
                // all-negative posterior keeps a live chance of proposing,
                // because `P(draw > 0) = Phi(-mean/sd) > 0` for any finite
                // mean; the exploration never freezes on bad news alone.
                let sampled = posterior.mean + posterior.sd * rng.next_normal();
                let expected_effect_bp = (sampled * 10_000.0).round() as i64;
                // The write loop refuses an effect outside `(0, max_step_bp]`
                // (it would abort the whole cycle), so a draw outside that
                // window is dropped here rather than seeded downstream.
                if expected_effect_bp <= 0 || expected_effect_bp > policy.max_step_bp {
                    continue;
                }
                out.push(Proposal {
                    action: Action::SetCountryFloor {
                        ad_unit_id: unit.platform_unit_id.clone(),
                        country: floor.country.clone(),
                        floor_ecpm: candidate.proposed_ecpm,
                    },
                    rationale: format!(
                        "move the {} floor {:+.0}% (prior mean {:+.2}%, sampled draw {:+.2}%)",
                        floor.country,
                        candidate.move_pct,
                        posterior.mean * 100.0,
                        sampled * 100.0
                    ),
                    expected_effect_bp,
                    prev_value: prev_value.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| {
        b.expected_effect_bp
            .cmp(&a.expected_effect_bp)
            .then_with(|| order_key(a).cmp(&order_key(b)))
    });
    out
}

fn order_key(proposal: &Proposal) -> (String, String, i64) {
    match &proposal.action {
        Action::SetCountryFloor {
            ad_unit_id,
            country,
            floor_ecpm,
        } => (ad_unit_id.clone(), country.clone(), *floor_ecpm),
        other => (other.ad_unit_id().to_string(), String::new(), 0),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_audit::{AppAdsTxt, RevenueTotals, Snapshot};
    use apex_core::{AdFormat, AdUnit, CountryFloor, PlatformId};
    use apex_lift::priors::Bucket;

    use super::*;

    fn snapshot(units: Vec<AdUnit>) -> Snapshot {
        Snapshot {
            platform: PlatformId::AppLovinMax,
            fetched_at_unix_ms: 1_700_000_000_000,
            ad_units: units,
            waterfall: vec![],
            mediation_groups: vec![],
            bidder_accounts: vec![],
            country_revenue: vec![],
            app_ads_txt: AppAdsTxt {
                present: true,
                lines: vec![],
                expected_networks: vec![],
            },
            platform_totals_30d: RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_30d: RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
        }
    }

    fn unit(enabled: bool, floors: Vec<CountryFloor>) -> AdUnit {
        AdUnit {
            platform_unit_id: "u1".into(),
            name: "Rewarded".into(),
            format: AdFormat::Rewarded,
            enabled,
            country_floors: Some(floors),
            platform: Some("android".into()),
            package_name: Some("com.example".into()),
        }
    }

    fn seeded_priors() -> LiftPriors {
        let mut priors = LiftPriors::default();
        priors.observe(&ee_bucket(), 0.05, 0.01);
        priors
    }

    const CYCLE_SEED: u64 = 1_700_000_000_000;

    fn single_floor_snapshot() -> Snapshot {
        snapshot(vec![unit(
            true,
            vec![CountryFloor {
                country: "EE".into(),
                floor_ecpm: 2_500_000,
                group: None,
            }],
        )])
    }

    fn ee_bucket() -> Bucket {
        Bucket {
            lever: "floor".into(),
            ad_format: "rewarded".into(),
            country_tier: "EE".into(),
            platform: "applovin_max".into(),
        }
    }

    #[test]
    fn a_fresh_account_proposes_from_the_skeptical_prior() {
        let snap = single_floor_snapshot();
        let policy = AutopilotPolicy::conservative();
        // The skeptical prior has mean zero; the Thompson draw is what turns
        // it into a plan (one draw per candidate, deterministic in the seed).
        let proposals = floor_proposals(&snap, &LiftPriors::default(), &policy, CYCLE_SEED);
        assert!(!proposals.is_empty(), "the skeptical prior still proposes");
        assert!(proposals
            .iter()
            .all(|p| p.expected_effect_bp > 0 && p.expected_effect_bp <= policy.max_step_bp));
    }

    #[test]
    fn the_same_cycle_seed_replays_the_same_plan() {
        let snap = single_floor_snapshot();
        let policy = AutopilotPolicy::conservative();
        let first = floor_proposals(&snap, &seeded_priors(), &policy, CYCLE_SEED);
        let replay = floor_proposals(&snap, &seeded_priors(), &policy, CYCLE_SEED);
        assert!(!first.is_empty());
        assert_eq!(first, replay, "a replay with the same seed is identical");
    }

    #[test]
    fn seeds_change_the_draws_but_never_the_gates() {
        let snap = single_floor_snapshot();
        let policy = AutopilotPolicy::conservative();
        let mut plans = Vec::new();
        for seed in [3u64, 5, 8, 13] {
            let proposals = floor_proposals(&snap, &LiftPriors::default(), &policy, seed);
            for proposal in &proposals {
                assert!(
                    proposal.expected_effect_bp > 0
                        && proposal.expected_effect_bp <= policy.max_step_bp,
                    "the (0, cap] gate holds: {proposal:?}"
                );
                match &proposal.action {
                    Action::SetCountryFloor { floor_ecpm, .. } => {
                        // The 20% step cap around the 2.5 floor: 2.0..=3.0.
                        assert!((2_000_000..=3_000_000).contains(floor_ecpm), "{floor_ecpm}");
                    }
                    other => panic!("floor proposals only, got {other:?}"),
                }
            }
            plans.push(proposals);
        }
        assert!(
            plans.windows(2).any(|pair| pair[0] != pair[1]),
            "different seeds draw different samples: {plans:?}"
        );
    }

    #[test]
    fn an_all_negative_posterior_still_proposes_per_the_draw() {
        // Every observation in the bucket lost 2% with a 5% standard error:
        // the posterior mean is negative, but its spread keeps
        // `P(draw > 0) = Phi(-mean/sd)` well above zero, so a draw can still
        // propose instead of freezing the account on bad news alone.
        let mut priors = LiftPriors::default();
        for _ in 0..3 {
            priors.observe(&ee_bucket(), -0.02, 0.05);
        }
        let posterior = priors.posterior(&ee_bucket());
        assert!(
            posterior.mean < 0.0,
            "all-negative observations: {posterior:?}"
        );
        // Seed 42 is chosen to make the point concrete and deterministic:
        // its draws are [-18, -289, +256, +10] bp, so the negative-mean
        // posterior still proposes the two positive draws (+10% and +20%).
        let proposals = floor_proposals(
            &single_floor_snapshot(),
            &priors,
            &AutopilotPolicy::conservative(),
            42,
        );
        let pinned: Vec<(i64, i64)> = proposals
            .iter()
            .map(|p| match &p.action {
                Action::SetCountryFloor { floor_ecpm, .. } => (p.expected_effect_bp, *floor_ecpm),
                other => panic!("floor proposals only, got {other:?}"),
            })
            .collect();
        assert_eq!(pinned, vec![(256, 2_750_000), (10, 3_000_000)]);
    }

    #[test]
    fn the_default_prior_plan_matches_the_pinned_draws() {
        // The golden for a fresh account at the pinned cycle seed: the exact
        // draws the deterministic SplitMix64 stream produces, in rank order.
        let proposals = floor_proposals(
            &single_floor_snapshot(),
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        let pinned: Vec<(i64, i64)> = proposals
            .iter()
            .map(|p| match &p.action {
                Action::SetCountryFloor { floor_ecpm, .. } => (p.expected_effect_bp, *floor_ecpm),
                other => panic!("floor proposals only, got {other:?}"),
            })
            .collect();
        assert_eq!(pinned, vec![(24, 3_000_000)]);
        assert_eq!(
            proposals[0].rationale,
            "move the EE floor +20% (prior mean +0.00%, sampled draw +0.24%)"
        );
    }

    #[test]
    fn a_seeded_prior_yields_ranked_candidates_within_the_cap() {
        let snap = single_floor_snapshot();
        let policy = AutopilotPolicy::conservative();
        let proposals = floor_proposals(&snap, &seeded_priors(), &policy, CYCLE_SEED);
        assert!(!proposals.is_empty());
        assert!(proposals
            .iter()
            .all(|p| p.expected_effect_bp > 0 && p.expected_effect_bp <= policy.max_step_bp));
        assert!(proposals
            .windows(2)
            .all(|w| w[0].expected_effect_bp >= w[1].expected_effect_bp));
        assert!(proposals
            .iter()
            .all(|p| matches!(p.action, Action::SetCountryFloor { .. })));
        assert!(
            proposals[0].rationale.contains("sampled draw"),
            "the draw is on the record: {}",
            proposals[0].rationale
        );
    }

    #[test]
    fn order_key_handles_non_floor_actions() {
        let proposal = Proposal {
            action: Action::RemoveWaterfallEntry {
                ad_unit_id: "u1".into(),
                entry_id: "e1".into(),
            },
            rationale: "r".into(),
            expected_effect_bp: 1,
            prev_value: "{}".into(),
        };
        assert_eq!(order_key(&proposal), ("u1".to_string(), String::new(), 0));
    }

    #[test]
    fn disabled_units_are_never_proposed_against() {
        let snap = snapshot(vec![unit(
            false,
            vec![CountryFloor {
                country: "EE".into(),
                floor_ecpm: 2_500_000,
                group: None,
            }],
        )]);
        assert!(floor_proposals(
            &snap,
            &seeded_priors(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED
        )
        .is_empty());
    }
}
