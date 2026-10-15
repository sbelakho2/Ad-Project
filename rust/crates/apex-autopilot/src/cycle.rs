//! The write cycle job: generate proposals from an audit snapshot plus the
//! bucket model, then run the gated write loop.
//!
//! This is the piece that makes the engine drive the account: the daily pass
//! only settles experiments that already run; the cycle is what turns an
//! audit into a pre-registered, platform-native experiment. Approval mode is
//! enforced by the loop itself, so a new publisher's first thirty days
//! produce proposals without writing anything.

use std::path::Path;

use apex_audit::Snapshot;
use apex_proof::LedgerLog;

use crate::model::Action;
use crate::runner::Runner;

/// What one cycle job did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CycleJobReport {
    /// Proposals generated from the snapshot and the bucket model.
    pub proposed: usize,
    pub applied: u32,
    pub skipped: u32,
    pub awaiting_approval: u32,
}

/// Run one write cycle from `snapshot_path`: load the audit snapshot, load
/// the persisted bucket model when present, generate the ranked floor
/// proposals, and run the gated loop.
pub async fn run(
    runner: &Runner,
    ledger: &mut dyn LedgerLog,
    snapshot_path: &Path,
    now_unix_ms: i64,
) -> Result<CycleJobReport, crate::model::PlanError> {
    let raw = std::fs::read_to_string(snapshot_path).map_err(|error| {
        crate::model::PlanError::Validation(format!(
            "snapshot {}: {error}",
            snapshot_path.display()
        ))
    })?;
    let snapshot: Snapshot = serde_json::from_str(&raw)
        .map_err(|error| crate::model::PlanError::Validation(format!("snapshot json: {error}")))?;
    let priors = runner.runner_priors().cloned().unwrap_or_default();
    let proposals = crate::proposals::floor_proposals(&snapshot, &priors, runner.policy());
    let mut report = CycleJobReport {
        proposed: proposals.len(),
        ..CycleJobReport::default()
    };
    if proposals.is_empty() {
        return Ok(report);
    }
    let outcome = crate::plan::run_cycle(
        runner.platform_ops(),
        ledger,
        runner.policy(),
        &[],
        proposals,
        now_unix_ms,
    )
    .await?;
    report.applied = outcome.applied;
    report.skipped = outcome.skipped;
    report.awaiting_approval = outcome.awaiting_approval;
    Ok(report)
}

/// The action label for logs; the cycle reports outcomes verbatim.
#[allow(dead_code)]
pub(crate) fn action_label(action: &Action) -> String {
    match action {
        Action::SetCountryFloor {
            ad_unit_id,
            country,
            ..
        } => format!("floor:{ad_unit_id}:{country}"),
        Action::SetMappingEcpm {
            mediation_group_id,
            ad_unit_id,
            ..
        } => format!("ecpm:{mediation_group_id}:{ad_unit_id}"),
        Action::SetWaterfallPrice {
            ad_unit_id,
            entry_id,
            ..
        } => format!("price:{ad_unit_id}:{entry_id}"),
        Action::CreateWaterfallEntry {
            ad_unit_id,
            network,
            ..
        } => format!("create:{ad_unit_id}:{network}"),
        Action::RemoveWaterfallEntry {
            ad_unit_id,
            entry_id,
        } => format!("remove:{ad_unit_id}:{entry_id}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    use std::sync::Arc;

    use apex_audit::{AppAdsTxt, RevenueTotals, Snapshot};
    use apex_core::{AdFormat, AdUnit, CountryFloor, PlatformId};
    use apex_lift::priors::{Bucket, LiftPriors};
    use apex_proof::{InMemoryLedger, Keypair};

    use crate::model::AutopilotPolicy;
    use crate::ops::RecordingOps;

    fn snapshot() -> Snapshot {
        Snapshot {
            platform: PlatformId::AppLovinMax,
            fetched_at_unix_ms: 1,
            ad_units: vec![AdUnit {
                platform_unit_id: "u1".into(),
                name: "Rewarded".into(),
                format: AdFormat::Rewarded,
                enabled: true,
                country_floors: Some(vec![CountryFloor {
                    country: "EE".into(),
                    floor_ecpm: 2_500_000,
                    group: None,
                }]),
                platform: Some("android".into()),
                package_name: Some("com.example".into()),
            }],
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

    async fn unused_ops() -> RecordingOps {
        let ops = RecordingOps::new();
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops
    }

    #[tokio::test]
    async fn a_cycle_turns_a_snapshot_into_a_started_experiment() {
        let dir = std::env::temp_dir().join(format!("apex-cycle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let snapshot_path = dir.join("applovin_max.json");
        std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot()).unwrap()).unwrap();

        let ops = Arc::new(unused_ops().await);
        let runner = Runner::new(
            ops.clone(),
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_priors(seeded_priors(), dir.join("priors.json"));
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[1u8; 32]));

        let report = run(&runner, &mut ledger, &snapshot_path, 1_700_000_000_000)
            .await
            .expect("cycle");
        assert!(report.proposed > 0, "the model proposes a floor move");
        assert!(report.applied >= 1, "the write loop started experiments");
        assert!(ledger
            .records()
            .iter()
            .any(|r| matches!(r, apex_proof::LedgerRecord::ExperimentStarted { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cycle_with_no_prior_support_proposes_nothing() {
        let dir = std::env::temp_dir().join(format!("apex-cycle0-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let snapshot_path = dir.join("s.json");
        std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot()).unwrap()).unwrap();
        let ops = Arc::new(unused_ops().await);
        let runner = Runner::new(
            ops,
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        );
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[2u8; 32]));
        let report = run(&runner, &mut ledger, &snapshot_path, 1)
            .await
            .expect("cycle");
        assert_eq!(report.proposed, 0);
        assert_eq!(report.applied, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn action_labels_are_stable() {
        let label = action_label(&Action::SetCountryFloor {
            ad_unit_id: "u1".into(),
            country: "EE".into(),
            floor_ecpm: 1,
        });
        assert_eq!(label, "floor:u1:EE");
    }
}
