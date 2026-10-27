//! Proposal generation: turn an audit snapshot plus the lift priors into the
//! candidate changes a cycle can test.
//!
//! MAX candidates are country-floor moves. LevelPlay and AdMob snapshots carry
//! no ad units, so their candidates come from the reads those platforms do
//! fill: one waterfall instance-price move per LevelPlay instance and one
//! mediation-group floor move per AdMob mapping line, each raised by the
//! largest step the policy allows.
//!
//! The MAX generator is deliberately narrow: it proposes country-floor moves from
//! the audit snapshot, scores each candidate with one Thompson draw from the
//! hierarchical posterior of its bucket, and keeps only draws inside the
//! policy's `(0, max_step_bp]` window -- the same window the write loop
//! enforces -- ranked by the draw. The draw is deterministic in the cycle's
//! seed, so a replayed cycle produces the same plan, and it is what lets a
//! fresh account propose: the skeptical global prior has mean zero, and a
//! gate on the mean would keep yielding an empty list.

use apex_audit::Snapshot;
use apex_core::PlatformId;
use apex_lift::candidates::floor_candidates;
use apex_lift::priors::{Bucket, LiftPriors, PosteriorCache};
use apex_lift::rng::SplitMix64;

use crate::model::{Action, AutopilotPolicy, Proposal};

/// The conservative price target for a current eCPM under the policy's
/// per-change magnitude cap.
///
/// Arithmetic (all integer micros, no floating point):
///
/// ```text
/// delta       = current_micros * max_step_bp / 10_000   (truncating)
/// target      = current_micros + delta
/// effect_bp   = delta * 10_000 / current_micros         (truncating)
/// ```
///
/// `delta` is the largest raise that still fits the policy's max step, so
/// `effects_bp <= max_step_bp` by construction. A current price whose cap
/// rounds to zero micros yields no proposal (`None`), which keeps the write
/// loop's `(0, max_step_bp]` gate intact.
fn conservative_price_target(current_micros: i64, max_step_bp: i64) -> Option<(i64, i64)> {
    if current_micros <= 0 || max_step_bp <= 0 {
        return None;
    }
    let delta = current_micros.checked_mul(max_step_bp)? / 10_000;
    if delta <= 0 {
        return None;
    }
    let target = current_micros.checked_add(delta)?;
    let effect_bp = delta.checked_mul(10_000)? / current_micros;
    if effect_bp <= 0 || effect_bp > max_step_bp {
        return None;
    }
    Some((target, effect_bp))
}

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
    // One pass over the bucket model for the whole proposal run. The earlier
    // lookup rescanned every bucket per floor: O(units x floors x buckets).
    let posteriors = PosteriorCache::new(priors);
    // Country-floor proposals are MAX's write surface only. A LevelPlay or
    // AdMob snapshot that happens to carry an ad-unit read must not generate
    // a MAX action: the cycle would then route that action to the wrong
    // adapter (and the ledger would label it for the wrong platform).
    for unit in &snapshot.ad_units {
        if snapshot.platform != PlatformId::AppLovinMax {
            break;
        }
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
            let posterior = posteriors.posterior(priors, &bucket);
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
    // LevelPlay and AdMob snapshots carry no ad units; their proposal surface
    // is the waterfall / mediation-group read instead. MAX is untouched: the
    // platform gate below yields nothing for it, so its plan (including the
    // deterministic Thompson draws) stays byte-identical.
    match snapshot.platform {
        PlatformId::LevelPlay => out.extend(levelplay_instance_proposals(snapshot, policy)),
        PlatformId::Admob => out.extend(admob_floor_proposals(snapshot, policy)),
        PlatformId::AppLovinMax | PlatformId::Billing => {}
    }
    out.sort_by(|a, b| {
        b.expected_effect_bp
            .cmp(&a.expected_effect_bp)
            .then_with(|| order_key(a).cmp(&order_key(b)))
    });
    out
}

/// One LevelPlay proposal per waterfall instance whose price is below the
/// conservative target. The target is
/// [`conservative_price_target`]`(price_ecpm, policy.max_step_bp)`: the current
/// instance price raised by the largest step the policy allows. The action
/// carries the platform `instanceId` and the target in micros.
fn levelplay_instance_proposals(snapshot: &Snapshot, policy: &AutopilotPolicy) -> Vec<Proposal> {
    let mut out = Vec::new();
    // One candidate per platform instance. The app-scoped audit read is
    // already unique, but a snapshot assembled by a per-unit reader can carry
    // the same instance once per unit; collapsing here keeps the idempotency
    // keys (and the plan) unique.
    let mut seen = std::collections::HashSet::new();
    for entry in &snapshot.waterfall {
        if entry.entry_id.trim().is_empty() || !seen.insert(entry.entry_id.as_str()) {
            continue;
        }
        let Some((target, effect_bp)) =
            conservative_price_target(entry.price_ecpm, policy.max_step_bp)
        else {
            continue;
        };
        // The target is above the current price by construction; the explicit
        // check keeps the "below target" rule visible and correct if the
        // arithmetic is ever changed.
        if entry.price_ecpm >= target {
            continue;
        }
        let mut previous = serde_json::Map::new();
        previous.insert("priceEcpm".to_string(), serde_json::json!(entry.price_ecpm));
        // The console's current-value cell keys the prior value by the
        // action's scope (the instance id); keep the raw price too so the
        // change record stays self-describing.
        previous.insert(entry.entry_id.clone(), serde_json::json!(entry.price_ecpm));
        out.push(Proposal {
            action: Action::SetLevelplayInstancePrice {
                instance_id: entry.entry_id.clone(),
                price_micros: target,
            },
            rationale: format!(
                "raise the {} instance {} price +{effect_bp} bp ({} -> {} micros eCPM; \
                 conservative target = current price + max step)",
                entry.network, entry.entry_id, entry.price_ecpm, target
            ),
            expected_effect_bp: effect_bp,
            prev_value: serde_json::Value::Object(previous).to_string(),
        });
    }
    out
}

/// One AdMob proposal per mediation group below its conservative floor target.
///
/// The frozen action names a mediation group and one floor, not a mapping
/// line, so a group's mappings collapse to a single candidate. The candidate
/// is the group's lowest positive mapping eCPM (ties broken by the mapping's
/// ad-unit id), which is the effective floor the group pays at its cheapest
/// line; the target is
/// [`conservative_price_target`]`(mapping.ecpm, policy.max_step_bp)`.
///
/// Emitting one candidate per mapping would be wrong as well as quadratic in
/// the group's mapping count: every candidate for a group has the same action
/// scope, so identical mapping prices would mint duplicate idempotency keys
/// and a second A/B experiment for a group that already has one.
fn admob_floor_proposals(snapshot: &Snapshot, policy: &AutopilotPolicy) -> Vec<Proposal> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for group in &snapshot.mediation_groups {
        if group.mediation_group_id.trim().is_empty()
            || !seen.insert(group.mediation_group_id.as_str())
        {
            continue;
        }
        // Deterministic representative: lowest positive eCPM, then ad-unit id.
        let Some(mapping) = group
            .mappings
            .iter()
            .filter(|mapping| mapping.ecpm > 0)
            .min_by(|left, right| {
                left.ecpm
                    .cmp(&right.ecpm)
                    .then_with(|| left.ad_unit_id.cmp(&right.ad_unit_id))
            })
        else {
            continue;
        };
        let Some((target, effect_bp)) = conservative_price_target(mapping.ecpm, policy.max_step_bp)
        else {
            continue;
        };
        if mapping.ecpm >= target {
            continue;
        }
        let mut previous = serde_json::Map::new();
        previous.insert(
            "mediationGroupId".to_string(),
            serde_json::json!(group.mediation_group_id),
        );
        previous.insert(
            "adUnitId".to_string(),
            serde_json::json!(mapping.ad_unit_id),
        );
        previous.insert("ecpm".to_string(), serde_json::json!(mapping.ecpm));
        // The console keys the current value by the action's scope.
        previous.insert(
            group.mediation_group_id.clone(),
            serde_json::json!(mapping.ecpm),
        );
        out.push(Proposal {
            action: Action::SetAdmobMediationFloor {
                group_id: group.mediation_group_id.clone(),
                floor_micros: target,
            },
            rationale: format!(
                "raise mediation group {} line {} ({}) floor +{effect_bp} bp \
                 ({} -> {} micros eCPM; conservative target = current floor + max step)",
                group.mediation_group_id, mapping.ad_unit_id, mapping.network, mapping.ecpm, target
            ),
            expected_effect_bp: effect_bp,
            prev_value: serde_json::Value::Object(previous).to_string(),
        });
    }
    out
}

/// The deterministic tie-break key. It borrows from the proposal on purpose:
/// the sort calls this once per comparison, and cloning the ids per comparison
/// would make a 10k-candidate plan (mostly equal effect draws, so mostly ties)
/// allocate on every comparison.
fn order_key(proposal: &Proposal) -> (&str, &str, i64) {
    match &proposal.action {
        Action::SetCountryFloor {
            ad_unit_id,
            country,
            floor_ecpm,
        } => (ad_unit_id, country, *floor_ecpm),
        Action::SetLevelplayInstancePrice {
            instance_id,
            price_micros,
        } => (instance_id, "instance-price", *price_micros),
        Action::SetAdmobMediationFloor {
            group_id,
            floor_micros,
        } => (group_id, "mediation-floor", *floor_micros),
        other => (other.ad_unit_id(), "", 0),
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
            mediation_totals_reported: false,
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
        assert_eq!(order_key(&proposal), ("u1", "", 0));
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

    fn platform_snapshot(platform: PlatformId) -> Snapshot {
        let mut snap = snapshot(vec![]);
        snap.platform = platform;
        snap
    }

    fn waterfall(entry_id: &str, price_ecpm: i64) -> apex_core::WaterfallEntry {
        apex_core::WaterfallEntry {
            entry_id: entry_id.into(),
            ad_unit_id: "u1".into(),
            network: "admob".into(),
            instance_name: format!("instance {entry_id}"),
            position: 1,
            price_ecpm,
        }
    }

    fn mediation_group(id: &str, ecpm: i64) -> apex_core::MediationGroup {
        apex_core::MediationGroup {
            mediation_group_id: id.into(),
            name: format!("group {id}"),
            format: AdFormat::Interstitial,
            mappings: vec![apex_core::Mapping {
                ad_unit_id: "u1".into(),
                network: "admob".into(),
                ecpm,
            }],
        }
    }

    #[test]
    fn a_levelplay_waterfall_yields_an_instance_price_proposal() {
        let mut snap = platform_snapshot(PlatformId::LevelPlay);
        snap.waterfall = vec![waterfall("i1", 2_000_000)];
        let proposals = floor_proposals(
            &snap,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        assert_eq!(proposals.len(), 1, "{proposals:?}");
        assert_eq!(
            proposals[0].action,
            Action::SetLevelplayInstancePrice {
                instance_id: "i1".into(),
                price_micros: 2_400_000,
            },
            "the conservative target is the +20% policy step"
        );
        assert_eq!(proposals[0].expected_effect_bp, 2_000);
        assert!(proposals[0].rationale.contains("conservative target"));
    }

    #[test]
    fn a_levelplay_waterfall_below_the_cap_rounds_down_to_a_real_step() {
        let mut snap = platform_snapshot(PlatformId::LevelPlay);
        snap.waterfall = vec![waterfall("i2", 2_345_000)];
        let proposals = floor_proposals(
            &snap,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        assert_eq!(proposals.len(), 1);
        assert_eq!(
            proposals[0].action,
            Action::SetLevelplayInstancePrice {
                instance_id: "i2".into(),
                // 2_345_000 * 2_000 / 10_000 = 469_000 exactly.
                price_micros: 2_814_000,
            }
        );
        assert_eq!(proposals[0].expected_effect_bp, 2_000);
    }

    #[test]
    fn an_admob_mediation_group_yields_a_floor_proposal() {
        let mut snap = platform_snapshot(PlatformId::Admob);
        snap.mediation_groups = vec![mediation_group("g1", 1_000_000)];
        let proposals = floor_proposals(
            &snap,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        assert_eq!(proposals.len(), 1, "{proposals:?}");
        assert_eq!(
            proposals[0].action,
            Action::SetAdmobMediationFloor {
                group_id: "g1".into(),
                floor_micros: 1_200_000,
            }
        );
        assert_eq!(proposals[0].expected_effect_bp, 2_000);
        assert!(proposals[0].rationale.contains("conservative target"));
    }

    #[test]
    fn a_per_unit_waterfall_fixture_does_not_duplicate_an_instance() {
        // A legacy snapshot built by reading the app once per configured unit
        // carries the same instance several times; only one instance-price
        // candidate (and one idempotency key) may result.
        let mut snap = platform_snapshot(PlatformId::LevelPlay);
        let mut first = waterfall("i1", 2_000_000);
        first.ad_unit_id = "u1".into();
        let mut second = waterfall("i1", 2_000_000);
        second.ad_unit_id = "u2".into();
        snap.waterfall = vec![first, second];
        let proposals = floor_proposals(
            &snap,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        assert_eq!(proposals.len(), 1, "{proposals:?}");
        assert_eq!(
            crate::keys::idempotency_key(&proposals[0].action).expect("key"),
            "levelplay-price:i1:i1:2.4"
        );
    }

    #[test]
    fn an_admob_group_collapses_its_mapping_lines_to_one_candidate() {
        // The action names the group and one floor, so the group's mappings
        // are one candidate. The representative is the lowest positive eCPM,
        // with the ad-unit id breaking ties.
        let mut snap = platform_snapshot(PlatformId::Admob);
        let mut group = mediation_group("g1", 1_000_000);
        group.mappings = vec![
            apex_core::Mapping {
                ad_unit_id: "u-high".into(),
                network: "admob".into(),
                ecpm: 2_000_000,
            },
            apex_core::Mapping {
                ad_unit_id: "u-tie".into(),
                network: "meta".into(),
                ecpm: 1_000_000,
            },
            apex_core::Mapping {
                ad_unit_id: "u-low".into(),
                network: "unity".into(),
                ecpm: 1_000_000,
            },
            apex_core::Mapping {
                ad_unit_id: "u-zero".into(),
                network: "noop".into(),
                ecpm: 0,
            },
        ];
        // A malformed fixture that repeats the group must still yield one
        // candidate (and one idempotency key) for it.
        snap.mediation_groups = vec![group, mediation_group("g1", 9_000_000)];
        let proposals = floor_proposals(
            &snap,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        assert_eq!(proposals.len(), 1, "{proposals:?}");
        assert_eq!(
            proposals[0].action,
            Action::SetAdmobMediationFloor {
                group_id: "g1".into(),
                floor_micros: 1_200_000,
            }
        );
        assert!(proposals[0].rationale.contains("u-low"));
        let previous: serde_json::Value =
            serde_json::from_str(&proposals[0].prev_value).expect("json");
        assert_eq!(previous["g1"], serde_json::json!(1_000_000));
        assert_eq!(previous["adUnitId"], serde_json::json!("u-low"));
    }

    #[test]
    fn the_console_current_value_keys_match_the_action_scope() {
        let mut snap = platform_snapshot(PlatformId::LevelPlay);
        snap.waterfall = vec![waterfall("i9", 1_500_000)];
        let proposals = floor_proposals(
            &snap,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        let previous: serde_json::Value =
            serde_json::from_str(&proposals[0].prev_value).expect("json");
        assert_eq!(previous["i9"], serde_json::json!(1_500_000));
        assert_eq!(previous["priceEcpm"], serde_json::json!(1_500_000));
    }

    #[test]
    fn ten_thousand_rows_stay_one_pass_and_one_candidate_per_group() {
        const N: usize = 10_000;
        let mut levelplay = platform_snapshot(PlatformId::LevelPlay);
        levelplay.waterfall = (0..N)
            .map(|index| waterfall(&format!("i{index}"), 1_000_000 + index as i64))
            .collect();
        let levelplay_proposals = floor_proposals(
            &levelplay,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        assert_eq!(levelplay_proposals.len(), N);
        let mut instance_ids: Vec<&str> = levelplay_proposals
            .iter()
            .filter_map(|proposal| match &proposal.action {
                Action::SetLevelplayInstancePrice { instance_id, .. } => Some(instance_id.as_str()),
                _ => None,
            })
            .collect();
        instance_ids.sort_unstable();
        instance_ids.dedup();
        assert_eq!(instance_ids.len(), N, "every instance is one candidate");

        let mut admob = platform_snapshot(PlatformId::Admob);
        admob.mediation_groups = (0..N)
            .map(|index| {
                let mut group = mediation_group(&format!("g{index}"), 1_000_000);
                group.mappings = (0..3)
                    .map(|line| apex_core::Mapping {
                        ad_unit_id: format!("g{index}-u{line}"),
                        network: "admob".into(),
                        ecpm: 1_000_000 + line * 1_000,
                    })
                    .collect();
                group
            })
            .collect();
        let admob_proposals = floor_proposals(
            &admob,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        assert_eq!(
            admob_proposals.len(),
            N,
            "one candidate per group, not one per mapping line"
        );
        let groups: std::collections::HashSet<&str> = admob_proposals
            .iter()
            .filter_map(|proposal| match &proposal.action {
                Action::SetAdmobMediationFloor { group_id, .. } => Some(group_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(groups.len(), N, "no group is proposed twice");
    }

    #[test]
    fn empty_levelplay_and_admob_snapshots_yield_no_proposals() {
        for platform in [PlatformId::LevelPlay, PlatformId::Admob] {
            let snap = platform_snapshot(platform);
            assert!(
                floor_proposals(
                    &snap,
                    &LiftPriors::default(),
                    &AutopilotPolicy::conservative(),
                    CYCLE_SEED,
                )
                .is_empty(),
                "{platform:?} with no rows proposes nothing"
            );
        }
    }

    #[test]
    fn a_max_snapshot_never_gets_instance_or_mediation_proposals() {
        // Even if a caller attaches foreign reads to a MAX snapshot, the
        // platform gate keeps the MAX plan floor-only and byte-identical.
        let mut snap = single_floor_snapshot();
        snap.waterfall = vec![waterfall("i1", 2_000_000)];
        snap.mediation_groups = vec![mediation_group("g1", 1_000_000)];
        let proposals = floor_proposals(
            &snap,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative(),
            CYCLE_SEED,
        );
        assert!(proposals
            .iter()
            .all(|proposal| matches!(proposal.action, Action::SetCountryFloor { .. })));
    }
}
