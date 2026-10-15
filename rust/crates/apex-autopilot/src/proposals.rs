//! Proposal generation: turn an audit snapshot plus the lift priors into the
//! candidate changes a cycle can test.
//!
//! The generator is deliberately narrow: it proposes country-floor moves from
//! the audit snapshot, ranks each by the hierarchical prior's expected lift
//! for its bucket, and drops anything outside the policy's step cap or without
//! positive prior support. It never fabricates a proposal the priors do not
//! back, so an empty prior yields an empty proposal list.

use apex_audit::Snapshot;
use apex_lift::candidates::floor_candidates;
use apex_lift::priors::{Bucket, LiftPriors};

use crate::model::{Action, AutopilotPolicy, Proposal};

/// Generate floor-move proposals from `snapshot`, ranked by prior-expected
/// lift (highest first, ties broken deterministically).
pub fn floor_proposals(
    snapshot: &Snapshot,
    priors: &LiftPriors,
    policy: &AutopilotPolicy,
) -> Vec<Proposal> {
    let platform = snapshot.platform.as_str().to_string();
    let cap_pct = (policy.max_step_bp as f64) / 100.0;
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
            let expected_effect_bp = (priors.posterior(&bucket).mean * 10_000.0).round() as i64;
            if expected_effect_bp <= 0 {
                continue;
            }
            for candidate in floor_candidates(&floor.country, floor.floor_ecpm, None) {
                if candidate.move_pct.abs() > cap_pct {
                    continue;
                }
                out.push(Proposal {
                    action: Action::SetCountryFloor {
                        ad_unit_id: unit.platform_unit_id.clone(),
                        country: floor.country.clone(),
                        floor_ecpm: candidate.proposed_ecpm,
                    },
                    rationale: format!(
                        "move the {} floor {:+.0}% (prior mean {:+.2}%)",
                        floor.country,
                        candidate.move_pct,
                        priors.posterior(&bucket).mean * 100.0
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
        priors.observe(
            &Bucket {
                lever: "floor".into(),
                ad_format: "rewarded".into(),
                country_tier: "EE".into(),
                platform: "applovin_max".into(),
            },
            0.05,
            0.01,
        );
        priors
    }

    #[test]
    fn proposals_need_positive_prior_support() {
        let snap = snapshot(vec![unit(
            true,
            vec![CountryFloor {
                country: "EE".into(),
                floor_ecpm: 2_500_000,
                group: None,
            }],
        )]);
        // The skeptical global prior has mean 0: nothing is proposed.
        assert!(floor_proposals(
            &snap,
            &LiftPriors::default(),
            &AutopilotPolicy::conservative()
        )
        .is_empty());
    }

    #[test]
    fn a_seeded_prior_yields_ranked_candidates_within_the_cap() {
        let snap = snapshot(vec![unit(
            true,
            vec![CountryFloor {
                country: "EE".into(),
                floor_ecpm: 2_500_000,
                group: None,
            }],
        )]);
        let policy = AutopilotPolicy::conservative();
        let proposals = floor_proposals(&snap, &seeded_priors(), &policy);
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
        assert!(
            floor_proposals(&snap, &seeded_priors(), &AutopilotPolicy::conservative()).is_empty()
        );
    }
}
