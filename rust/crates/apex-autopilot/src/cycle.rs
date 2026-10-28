//! The write cycle job: generate proposals from an audit snapshot plus the
//! bucket model, then run the gated write loop.
//!
//! This is the piece that makes the engine drive the account: the daily pass
//! only settles experiments that already run; the cycle is what turns an
//! audit into a pre-registered, platform-native experiment. Approval mode is
//! enforced by the loop itself, so a new publisher's first thirty days
//! produce proposals without writing anything.

use std::collections::HashSet;
use std::path::Path;

use apex_audit::Snapshot;
use apex_proof::LedgerLog;
use sha2::Digest as _;

use crate::model::{Action, ActionOutcome, AutopilotPolicy, Proposal};
use crate::runner::Runner;

/// The decision a console recorded for one proposal. Every binding must match
/// the proposal being acted on, so a decision for an older change, snapshot
/// or policy version never unlocks a new write.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredDecision {
    decision: String,
    change_hash: String,
    snapshot: String,
    /// SHA-256 of the complete effective policy the decision was reviewed
    /// against. Legacy records without it cannot establish the binding and
    /// never unlock a write.
    #[serde(default)]
    policy_hash: String,
}

/// The canonical change hash the console records: SHA-256 over the exact JSON
/// bytes of the proposal it stored.
pub fn proposal_change_hash(proposal: &Proposal) -> String {
    match serde_json::to_string(proposal) {
        Ok(json) => hex::encode(sha2::Sha256::digest(json.as_bytes())),
        Err(_) => String::new(),
    }
}

/// The proposal envelope the console reads. It is written when a proposal is
/// held for approval and is immutable once published.
pub fn stored_proposal(
    publisher: &str,
    proposal: &Proposal,
    snapshot: &str,
    created_at_unix_ms: i64,
    policy: &AutopilotPolicy,
    expires_at_unix_ms: Option<i64>,
) -> serde_json::Value {
    serde_json::json!({
        "publisher": publisher,
        "proposal": proposal,
        "snapshot": snapshot,
        "createdAtUnixMs": created_at_unix_ms,
        "policyVersion": format!("apex-lift-{}", apex_lift::ENGINE_VERSION),
        "policyHash": policy.canonical_hash().unwrap_or_default(),
        "expiresAtUnixMs": expires_at_unix_ms,
        "testShare": policy.test_share,
        "burnInDays": policy.burn_in_days,
        "windowDays": policy.window_days,
        "rateBp": policy.rate_bp,
        "thresholds": policy.thresholds,
        "minPointLift": policy.min_point_lift,
    })
}

/// The decision recorded for `key`, when it is bound to this exact change,
/// snapshot and policy version.
fn recorded_decision(
    root: Option<&Path>,
    key: &str,
    change_hash: &str,
    snapshot: &str,
    policy_hash: &str,
) -> Option<String> {
    let root = root?;
    if policy_hash.trim().is_empty() {
        return None;
    }
    let id = crate::notify::proposal_id_for(key);
    let raw = std::fs::read_to_string(root.join(format!("{id}.approval.json"))).ok()?;
    let decision: StoredDecision = serde_json::from_str(&raw).ok()?;
    if decision.change_hash == change_hash
        && decision.snapshot == snapshot
        && !decision.policy_hash.trim().is_empty()
        && decision.policy_hash == policy_hash
    {
        Some(decision.decision)
    } else {
        None
    }
}

/// Publish a proposal without ever overwriting one: the first review copy is
/// the durable one, and a concurrent writer loses the hard link race.
fn persist_proposal(root: &Path, id: &str, envelope: &serde_json::Value) -> bool {
    let target = root.join(format!("{id}.json"));
    if target.exists() {
        return true;
    }
    if let Err(error) = std::fs::create_dir_all(root) {
        eprintln!("apex-runner: proposal root is not writable (proposal {id}): {error}");
        return false;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let temp = root.join(format!(".{id}.{}.{nanos}.tmp", std::process::id()));
    let bytes = match serde_json::to_vec_pretty(envelope) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("apex-runner: proposal {id} could not be serialized: {error}");
            return false;
        }
    };
    let prepared = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        std::io::Write::write_all(&mut file, &bytes)?;
        file.sync_all()
    })();
    if let Err(error) = prepared {
        eprintln!("apex-runner: proposal {id} could not be written: {error}");
        let _ = std::fs::remove_file(&temp);
        return false;
    }
    let mut published = true;
    if let Err(error) = std::fs::hard_link(&temp, &target) {
        if !target.exists() {
            eprintln!("apex-runner: proposal {id} could not be published: {error}");
            published = false;
        }
    }
    let _ = std::fs::remove_file(&temp);
    if published {
        // The hard link is only durable with the directory entry behind it.
        #[cfg(unix)]
        if let Some(parent) = target.parent() {
            let parent = if parent.as_os_str().is_empty() {
                std::path::Path::new(".")
            } else {
                parent
            };
            if let Err(error) = std::fs::File::open(parent).and_then(|d| d.sync_all()) {
                eprintln!("apex-runner: proposal {id} directory could not be synced: {error}");
                published = false;
            }
        }
    }
    published
}

/// What one cycle job did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CycleJobReport {
    /// Proposals generated from the snapshot and the bucket model.
    pub proposed: usize,
    pub applied: u32,
    pub skipped: u32,
    pub awaiting_approval: u32,
    /// Proposals held because the platform adapter has not verified the
    /// operation as writable (`verify-live`): one typed reason per held
    /// proposal, `awaiting live verification for <platform>: <operation>`.
    /// The pass continues past a held proposal.
    pub awaiting_verification: Vec<String>,
    /// Platforms whose cycle was skipped because the latest fleet
    /// intervention for that platform (or an unscoped intervention) is a
    /// pause. Reads continued; only the writes stopped.
    pub paused: u32,
    /// One typed pause reason per paused platform.
    pub pause_reasons: Vec<String>,
}

/// Run one write cycle from `snapshot_paths`: load each audit snapshot, load
/// the persisted bucket model when present, generate the ranked proposals per
/// snapshot, and run the gated loop. A publisher with several platforms is
/// handed several files (its own per-platform snapshots); the default
/// single-file call is the one-element list. Proposals from every snapshot are
/// accumulated: MAX yields country-floor moves, LevelPlay yields waterfall
/// instance-price moves, and AdMob yields mediation-group floor moves.
pub async fn run(
    runner: &Runner,
    ledger: &mut dyn LedgerLog,
    snapshot_paths: &[std::path::PathBuf],
    now_unix_ms: i64,
) -> Result<CycleJobReport, crate::model::PlanError> {
    let priors = runner.runner_priors().cloned().unwrap_or_default();
    let mut report = CycleJobReport::default();
    for snapshot_path in snapshot_paths {
        let raw = std::fs::read_to_string(snapshot_path).map_err(|error| {
            crate::model::PlanError::Validation(format!(
                "snapshot {}: {error}",
                snapshot_path.display()
            ))
        })?;
        let snapshot: Snapshot = serde_json::from_str(&raw).map_err(|error| {
            crate::model::PlanError::Validation(format!("snapshot json: {error}"))
        })?;
        // A3 M1: account-metric and app-ads identity lifecycle. The first
        // observation records the baseline; later cycles compare against it
        // and emit alerts fail-closed. Monitoring continues during a pause.
        runner
            .observe_account_snapshots(std::slice::from_ref(&snapshot), now_unix_ms)
            .await
            .map_err(crate::model::PlanError::Validation)?;
        // Fleet pause: the snapshot was still read (measurement continues),
        // but this platform's write cycle does not run. The latest applicable
        // pause/resume record decides; an unreadable register refuses the
        // pass instead of writing while the pause state is unknown.
        let platform = snapshot.platform.as_str();
        match crate::interventions::platform_pause(runner.interventions_path(), platform) {
            Ok(Some(pause)) => {
                report.paused += 1;
                report.pause_reasons.push(pause.report_reason(platform));
                continue;
            }
            Ok(None) => {}
            Err(reason) => {
                return Err(crate::model::PlanError::Validation(format!(
                    "fleet intervention check: {reason}"
                )))
            }
        }
        // The cycle's timestamp seeds the Thompson draws, so a replay of the
        // same cycle reproduces the same plan (the same seam `settle` seeds
        // its bootstrap with).
        let proposals = crate::proposals::floor_proposals(
            &snapshot,
            &priors,
            runner.policy(),
            now_unix_ms.max(0) as u64,
        );
        report.proposed += proposals.len();
        if proposals.is_empty() {
            continue;
        }
        // The actual audit findings of this platform's snapshot feed the
        // production write gate. Passing an empty list here would let a
        // Critical finding coexist with a write cycle.
        let findings = apex_audit::run(&snapshot).findings;
        // The adapter is chosen from the snapshot's own platform: a MAX key
        // set must never execute LevelPlay or AdMob proposals, and vice versa.
        let ops = runner.ops_for(snapshot.platform)?;
        if ops.platform() != snapshot.platform {
            return Err(crate::model::PlanError::Validation(format!(
                "snapshot platform {} is routed to a {} adapter",
                snapshot.platform.as_str(),
                ops.platform().as_str()
            )));
        }
        let snapshot_digest = apex_audit::snapshot_digest(&snapshot);
        let policy_version = format!("apex-lift-{}", apex_lift::ENGINE_VERSION);
        let policy_hash = runner.policy().canonical_hash().map_err(|reason| {
            crate::model::PlanError::Validation(format!("effective policy: {reason}"))
        })?;
        // Decisions the workspace recorded in the console. A decision only
        // counts when every binding still matches; a stale approval cannot
        // unlock a changed proposal or survive a policy change.
        let mut approved = HashSet::new();
        let mut declined = HashSet::new();
        for proposal in &proposals {
            let Ok(key) = crate::keys::idempotency_key(&proposal.action) else {
                continue;
            };
            let hash = proposal_change_hash(proposal);
            match recorded_decision(
                runner.proposals_root(),
                &key,
                &hash,
                &snapshot_digest,
                &policy_hash,
            )
            .as_deref()
            {
                Some("approved") => {
                    approved.insert(key);
                }
                Some("declined") => {
                    declined.insert(key);
                }
                _ => {}
            }
        }
        let outcome = crate::plan::run_cycle_observed_with(
            ops,
            ledger,
            runner.policy(),
            &findings,
            &proposals,
            runner.events().map(AsRef::as_ref),
            &approved,
            &declined,
            now_unix_ms,
        )
        .await?;
        report.applied += outcome.applied;
        report.skipped += outcome.skipped;
        report.awaiting_approval += outcome.awaiting_approval;
        report
            .awaiting_verification
            .extend(outcome.awaiting_verification.iter().cloned());

        // A proposal held for approval is a durable fact and a customer
        // message: `proposal_created` lands in analytics and the approval
        // request goes to the authorized contacts. The message states it does
        // not grant approval.
        // Index the plan once, lazily: an approval pass with N held proposals
        // must not rescan (and re-key) the plan N times. Duplicate keys keep
        // the first proposal, which is the candidate the loop acted on.
        let mut proposals_by_key: Option<
            std::collections::HashMap<String, &crate::model::Proposal>,
        > = None;
        for outcome in &outcome.outcomes {
            let ActionOutcome::AwaitingApproval { idempotency_key } = outcome else {
                continue;
            };
            let index = proposals_by_key.get_or_insert_with(|| {
                let mut index = std::collections::HashMap::with_capacity(proposals.len());
                for proposal in &proposals {
                    if let Ok(key) = crate::keys::idempotency_key(&proposal.action) {
                        index.entry(key).or_insert(proposal);
                    }
                }
                index
            });
            let proposal = index.get(idempotency_key.as_str()).copied();
            let (test_label, country) = proposal
                .map(|proposal| crate::notify::proposal_label(&proposal.action))
                .unwrap_or_else(|| ("Change".to_string(), "all countries".to_string()));
            let proposal_id = crate::notify::proposal_id_for(idempotency_key);
            let published = match (runner.proposals_root(), proposal) {
                (Some(root), Some(proposal)) if !runner.publisher_id().is_empty() => {
                    let expires = runner
                        .policy()
                        .onboarding_unix_ms
                        .map(|start| start.saturating_add(crate::model::APPROVAL_WINDOW_MS));
                    let envelope = stored_proposal(
                        runner.publisher_id(),
                        proposal,
                        &snapshot_digest,
                        now_unix_ms,
                        runner.policy(),
                        expires,
                    );
                    persist_proposal(root, &proposal_id, &envelope)
                }
                _ => false,
            };
            if !published {
                // The console reads the proposal from this host's store. A
                // link to a proposal that was never published would be a
                // dead end, so the message is suppressed (the event below
                // still records the held change).
                eprintln!(
                    "apex-runner: proposal {proposal_id} is held but not published; approval email suppressed"
                );
            }
            let change_hash = proposal
                .and_then(|proposal| serde_json::to_string(&proposal.action).ok())
                .map(|json| crate::notify::change_hash(&json))
                .unwrap_or_else(|| crate::notify::change_hash(idempotency_key));
            if let Some(events) = runner.events() {
                events
                    .proposal_created(
                        &proposal_id,
                        &snapshot_digest,
                        &change_hash,
                        &policy_version,
                        now_unix_ms,
                    )
                    .await;
            }
            if published {
                if let (Some(notify), false) = (runner.notify(), runner.publisher_id().is_empty()) {
                    notify
                        .approval_requested(
                            runner.publisher_id(),
                            &proposal_id,
                            &test_label,
                            &country,
                            now_unix_ms,
                        )
                        .await;
                }
            }
        }
    }
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
        Action::SetLevelplayInstancePrice {
            instance_id,
            price_micros,
        } => format!("instance-price:{instance_id}:{price_micros}"),
        Action::SetAdmobMediationFloor {
            group_id,
            floor_micros,
        } => format!("mediation-floor:{group_id}:{floor_micros}"),
        Action::SetMaxFrequencyCap {
            ad_unit_id,
            cap_type,
            ..
        } => format!("frequency-cap:{ad_unit_id}:{}", cap_type.as_str()),
        Action::SetLevelplayGroupCapping { group_id, .. } => {
            format!("group-capping:{group_id}")
        }
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
    use crate::ops::{PlatformOps as _, RecordingOps};

    fn sample() -> Proposal {
        Proposal {
            action: Action::SetCountryFloor {
                ad_unit_id: "rw_de_01".to_string(),
                country: "DE".to_string(),
                floor_ecpm: 3_200_000,
            },
            rationale: "Raise the DE floor".to_string(),
            expected_effect_bp: 260,
            prev_value: "{\"DE\":4000000}".to_string(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_be_synced_is_not_reported_published() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = std::env::temp_dir().join(format!("apex-proposal-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("root");
        // Write+execute but not read: the link publishes, the directory
        // fsync cannot open it, and the caller must hear that the durable
        // state is unproven instead of a silent success.
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o300)).expect("perm");
        let envelope = serde_json::json!({"id": "p1"});
        let published = persist_proposal(&root, "p1", &envelope);
        assert!(
            !published,
            "an unsynced directory entry must not report published"
        );

        // With a readable directory the same publish path succeeds (the
        // target already exists from the link above, which is the idempotent
        // branch).
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).expect("perm");
        assert!(
            persist_proposal(&root, "p1", &envelope),
            "a synced publish reports published"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_published_proposal_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!(
            "apex-proposal-contract-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let policy = AutopilotPolicy::conservative();
        let envelope = stored_proposal("northstar", &sample(), "snap", 1, &policy, None);
        assert_eq!(
            envelope["policyHash"],
            serde_json::json!(policy.canonical_hash().expect("hash")),
            "the review envelope binds the complete effective policy"
        );
        assert_eq!(
            envelope["minPointLift"],
            serde_json::json!(policy.min_point_lift)
        );
        persist_proposal(&dir, "prop_00000001", &envelope);
        let first = std::fs::read_to_string(dir.join("prop_00000001.json")).unwrap();
        assert!(first.contains("\"publisher\": \"northstar\""));
        // A later cycle with a different timestamp must not replace the
        // review copy a decision may already be bound to.
        let again = stored_proposal("northstar", &sample(), "snap", 2, &policy, None);
        persist_proposal(&dir, "prop_00000001", &again);
        assert_eq!(
            std::fs::read_to_string(dir.join("prop_00000001.json")).unwrap(),
            first
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_decision_bound_to_the_same_change_unlocks_it() {
        let dir = std::env::temp_dir().join(format!(
            "apex-decision-binding-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let proposal = sample();
        let key = crate::keys::idempotency_key(&proposal.action).unwrap();
        let id = crate::notify::proposal_id_for(&key);
        let hash = proposal_change_hash(&proposal);
        let record = |decision: &str, hash: &str| {
            serde_json::json!({
                "proposalId": id,
                "decision": decision,
                "decidedAtUnixMs": 10,
                "idempotencyKey": key,
                "changeHash": hash,
                "snapshot": "snap",
                "policyHash": "policy-hash",
                "policyVersion": "v1",
                "approvalActor": "northstar",
            })
        };
        std::fs::write(
            dir.join(format!("{id}.approval.json")),
            serde_json::to_vec(&record("approved", &hash)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            recorded_decision(Some(&dir), &key, &hash, "snap", "policy-hash").as_deref(),
            Some("approved")
        );
        assert_eq!(
            recorded_decision(Some(&dir), &key, &hash, "other-snapshot", "policy-hash"),
            None,
            "a decision for another snapshot is stale"
        );
        assert_eq!(
            recorded_decision(
                Some(&dir),
                &key,
                "0".repeat(64).as_str(),
                "snap",
                "policy-hash"
            ),
            None,
            "a decision for another change is stale"
        );
        assert_eq!(
            recorded_decision(Some(&dir), &key, &hash, "snap", "another-policy"),
            None,
            "a decision for another effective policy is stale"
        );
        // A legacy decision (or one with a changed policy) cannot establish
        // the complete-policy binding and never unlocks a write.
        let legacy: serde_json::Value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(dir.join(format!("{id}.approval.json"))).unwrap(),
        )
        .unwrap();
        let mut legacy = legacy;
        legacy.as_object_mut().unwrap().remove("policyHash");
        std::fs::write(
            dir.join(format!("{id}.approval.json")),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        assert_eq!(
            recorded_decision(Some(&dir), &key, &hash, "snap", "policy-hash"),
            None,
            "an unbound legacy decision is refused"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
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
            mediation_totals_reported: false,
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

        let report = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1_700_000_000_000,
        )
        .await
        .expect("cycle");
        assert!(report.proposed > 0, "the model proposes a floor move");
        assert!(report.applied >= 1, "the write loop started experiments");
        assert!(
            report.awaiting_verification.is_empty(),
            "a verified writable operation records no hold"
        );
        assert!(ledger
            .records()
            .iter()
            .any(|r| matches!(r, apex_proof::LedgerRecord::ExperimentStarted { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cycle_from_a_fresh_account_proposes_from_the_default_prior() {
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
        let report = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1,
        )
        .await
        .expect("cycle");
        assert!(
            report.proposed > 0,
            "the skeptical prior now proposes via the Thompson draw"
        );
        assert!(report.applied > 0, "the write loop starts the proposal");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cycle_holds_operations_the_adapter_has_not_verified() {
        // The committed AdMob fixture proposes country-floor moves against an
        // AdMob adapter whose verified write set holds none of them: every
        // proposal is held with its typed reason, the report carries the
        // holds and the pass exits Ok instead of aborting — the exact live
        // defect this pins.
        let dir = std::env::temp_dir().join(format!("apex-cycle-hold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let snapshot_path = dir.join("admob.json");
        // The committed AdMob fixture carries a MAX-shaped ad-unit read. A
        // real AdMob write surface is the mediation-group read, so build that
        // shape here and keep the hold path exercised against the operation
        // an AdMob adapter actually owns.
        let mut snapshot: Snapshot =
            serde_json::from_str(include_str!("../../apex-api/fixtures/admob.json"))
                .expect("fixture snapshot");
        snapshot.ad_units.clear();
        snapshot.mediation_groups = vec![apex_core::MediationGroup {
            mediation_group_id: "g1".into(),
            name: "group g1".into(),
            format: apex_core::AdFormat::Rewarded,
            mappings: vec![apex_core::Mapping {
                ad_unit_id: "u1".into(),
                network: "meta".into(),
                ecpm: 1_000_000,
            }],
        }];
        std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();

        let ops = Arc::new(RecordingOps::new());
        ops.set_platform(PlatformId::Admob);
        ops.set_writable("updateMediationGroup", false);
        let mut policy = AutopilotPolicy::conservative();
        // The committed fixture intentionally carries audit findings; this
        // test pins the adapter's verified-write hold, not the blackout.
        policy.blackout_on_critical_findings = false;
        let runner = Runner::new(ops, policy, apex_lift::guardrails::FillFormat::Rewarded);
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[5u8; 32]));

        let report = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1_700_000_000_000,
        )
        .await
        .expect("the cycle continues past unverified operations");
        assert!(report.proposed > 0, "the fixture proposes floor moves");
        assert_eq!(report.applied, 0, "nothing is written unverified");
        assert_eq!(report.skipped, report.proposed as u32);
        assert_eq!(
            report.awaiting_verification.len(),
            report.proposed,
            "one typed hold per proposal"
        );
        assert!(report
            .awaiting_verification
            .iter()
            .all(|reason| reason == "awaiting live verification for Admob: updateMediationGroup"));
        assert!(ledger.records().is_empty(), "a held cycle writes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn critical_audit_findings_stop_the_cycle_before_any_write() {
        let dir = cycle_dir("blackout");
        let snapshot_path = dir.join("s.json");
        let mut critical = snapshot();
        critical.app_ads_txt.present = false;
        std::fs::write(&snapshot_path, serde_json::to_vec(&critical).unwrap()).unwrap();
        let ops = Arc::new(unused_ops().await);
        let runner = Runner::new(
            ops.clone(),
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        );
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[11u8; 32]));
        let error = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1_700_000_000_000,
        )
        .await
        .expect_err("a real Critical audit finding blackouts the cycle");
        assert!(error.to_string().contains("blackout"), "{error}");
        assert!(error.to_string().contains("critical"), "{error}");
        assert!(ledger.records().is_empty(), "a blackout writes nothing");
        assert!(
            ops.call_log().is_empty(),
            "a blackout never calls the platform: {:?}",
            ops.call_log()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn each_snapshot_runs_on_its_own_platform_adapter() {
        let dir = cycle_dir("routing");
        let max_path = dir.join("max.json");
        let levelplay_path = dir.join("levelplay.json");
        std::fs::write(&max_path, serde_json::to_vec(&snapshot()).unwrap()).unwrap();
        std::fs::write(
            &levelplay_path,
            serde_json::to_vec(&other_snapshot(apex_core::PlatformId::LevelPlay)).unwrap(),
        )
        .unwrap();

        let max_ops = Arc::new(unused_ops().await);
        let levelplay_ops = Arc::new(unused_ops().await);
        levelplay_ops.set_platform(apex_core::PlatformId::LevelPlay);
        let runner = Runner::new(
            max_ops.clone(),
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_platform_ops(apex_core::PlatformId::LevelPlay, levelplay_ops.clone());
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[12u8; 32]));
        let report = run(
            &runner,
            &mut ledger,
            &[max_path, levelplay_path],
            1_700_000_000_000,
        )
        .await
        .expect("cycle");
        assert!(report.applied >= 2, "{report:?}");

        let max_calls = max_ops.call_log();
        let lp_calls = levelplay_ops.call_log();
        assert!(
            max_calls
                .iter()
                .any(|entry| entry == "writable:setCountryFloor"),
            "MAX executes its own floor proposal: {max_calls:?}"
        );
        assert!(
            !max_calls
                .iter()
                .any(|entry| entry == "writable:updateEntryPrice"),
            "a LevelPlay proposal never reaches the MAX adapter: {max_calls:?}"
        );
        assert!(
            lp_calls
                .iter()
                .any(|entry| entry == "writable:updateEntryPrice"),
            "LevelPlay executes its own instance-price proposal: {lp_calls:?}"
        );
        assert!(
            !lp_calls
                .iter()
                .any(|entry| entry == "writable:setCountryFloor"),
            "a MAX proposal never reaches the LevelPlay adapter: {lp_calls:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn intervention_line(kind: &str, platform: Option<&str>, at_ms: i64, reason: &str) -> String {
        serde_json::json!({
            "interventionId": "itv_test",
            "kind": kind,
            "platform": platform,
            "customerPopulation": "all",
            "reason": reason,
            "expectedEffect": "writes stop",
            "actor": "maya",
            "atMs": at_ms,
            "correlationId": "c1",
        })
        .to_string()
    }

    fn write_interventions(path: &std::path::Path, lines: &[String]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
    }

    fn cycle_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("apex-cycle-pause-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn other_snapshot(platform: apex_core::PlatformId) -> Snapshot {
        let mut value = snapshot();
        value.platform = platform;
        // Neither secondary platform publishes MAX-style ad units.
        value.ad_units.clear();
        if platform == apex_core::PlatformId::LevelPlay {
            value.waterfall = vec![apex_core::WaterfallEntry {
                entry_id: "i1".into(),
                ad_unit_id: "u1".into(),
                network: "admob".into(),
                instance_name: "instance i1".into(),
                position: 1,
                price_ecpm: 2_000_000,
            }];
        }
        if platform == apex_core::PlatformId::Admob {
            value.mediation_groups = vec![apex_core::MediationGroup {
                mediation_group_id: "g1".into(),
                name: "group g1".into(),
                format: apex_core::AdFormat::Rewarded,
                mappings: vec![apex_core::Mapping {
                    ad_unit_id: "u1".into(),
                    network: "meta".into(),
                    ecpm: 1_000_000,
                }],
            }];
        }
        value
    }

    #[tokio::test]
    async fn a_missing_intervention_file_is_not_paused() {
        let dir = cycle_dir("missing");
        let snapshot_path = dir.join("s.json");
        std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot()).unwrap()).unwrap();
        let runner = Runner::new(
            Arc::new(unused_ops().await),
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_interventions(dir.join("absent.jsonl"));
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[6u8; 32]));
        let report = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1_700_000_000_000,
        )
        .await
        .expect("cycle");
        assert_eq!(report.paused, 0);
        assert!(report.proposed > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_paused_platform_skips_its_cycle_but_other_platforms_still_run() {
        let dir = cycle_dir("scoped");
        let max_path = dir.join("max.json");
        let levelplay_path = dir.join("levelplay.json");
        std::fs::write(&max_path, serde_json::to_vec(&snapshot()).unwrap()).unwrap();
        std::fs::write(
            &levelplay_path,
            serde_json::to_vec(&other_snapshot(apex_core::PlatformId::LevelPlay)).unwrap(),
        )
        .unwrap();
        let interventions = dir.join("interventions.jsonl");
        write_interventions(
            &interventions,
            &[intervention_line(
                "pause_scheduling",
                Some("applovin_max"),
                10,
                "MAX incident",
            )],
        );
        let levelplay_ops = Arc::new(unused_ops().await);
        levelplay_ops.set_platform(apex_core::PlatformId::LevelPlay);
        let runner = Runner::new(
            Arc::new(unused_ops().await),
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_platform_ops(apex_core::PlatformId::LevelPlay, levelplay_ops)
        .with_interventions(&interventions);
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[7u8; 32]));
        let report = run(
            &runner,
            &mut ledger,
            &[max_path, levelplay_path],
            1_700_000_000_000,
        )
        .await
        .expect("cycle");
        assert_eq!(report.paused, 1, "only MAX is paused");
        assert_eq!(
            report.pause_reasons,
            vec!["scheduling paused for applovin_max: MAX incident".to_string()]
        );
        assert!(report.applied >= 1, "the LevelPlay cycle still writes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_latest_pause_wins_and_a_newer_resume_clears_it() {
        let dir = cycle_dir("latest");
        let snapshot_path = dir.join("s.json");
        std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot()).unwrap()).unwrap();
        let interventions = dir.join("interventions.jsonl");
        write_interventions(
            &interventions,
            &[
                intervention_line("pause_scheduling", None, 10, "first"),
                intervention_line("pause_scheduling", None, 20, "latest"),
            ],
        );
        let runner = Runner::new(
            Arc::new(unused_ops().await),
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_interventions(&interventions);
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[8u8; 32]));
        let report = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1_700_000_000_000,
        )
        .await
        .expect("cycle");
        assert_eq!(report.paused, 1);
        assert!(report.pause_reasons[0].contains("latest"));

        write_interventions(
            &interventions,
            &[
                intervention_line("pause_scheduling", None, 10, "first"),
                intervention_line("pause_scheduling", None, 20, "latest"),
                intervention_line("resume_scheduling", None, 30, "recovered"),
            ],
        );
        let runner = Runner::new(
            Arc::new(unused_ops().await),
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_interventions(&interventions);
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[9u8; 32]));
        let report = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1_700_000_000_000,
        )
        .await
        .expect("cycle");
        assert_eq!(report.paused, 0, "the newer resume clears the pause");
        assert!(report.proposed > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_corrupt_intervention_register_fails_closed() {
        let dir = cycle_dir("malformed");
        let snapshot_path = dir.join("s.json");
        std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot()).unwrap()).unwrap();
        let interventions = dir.join("interventions.jsonl");
        write_interventions(
            &interventions,
            &[
                "{not json".to_string(),
                intervention_line("pause_scheduling", Some("applovin_max"), 10, "incident"),
            ],
        );
        let runner = Runner::new(
            Arc::new(unused_ops().await),
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_interventions(&interventions);
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[10u8; 32]));
        let error = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1_700_000_000_000,
        )
        .await
        .expect_err("a corrupt register fails closed");
        assert!(error.to_string().contains("corrupt"), "{error}");
        assert!(
            ledger.records().is_empty(),
            "a refused cycle writes nothing"
        );
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

    #[tokio::test]
    async fn a_cycle_emits_the_experiment_lifecycle_events() {
        use apex_analytics::store::{EventStore as _, ProjectionStore as _};

        let dir = std::env::temp_dir().join(format!("apex-cycle-events-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let snapshot_path = dir.join("applovin_max.json");
        std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot()).unwrap()).unwrap();

        let ops = Arc::new(unused_ops().await);
        let store = Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = apex_analytics::AnalyticsClient::new(store.clone(), "dev", "apex-runner-test");
        let events = Arc::new(
            crate::events::RunnerEvents::new(Arc::new(client), "northstar").with_environment("dev"),
        );
        let runner = Runner::new(
            ops,
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_priors(seeded_priors(), dir.join("priors.json"))
        .with_events(events);
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[3u8; 32]));

        let report = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1_700_000_000_000,
        )
        .await
        .expect("cycle");
        assert!(report.applied >= 1);
        let stored = store.load_events().await.expect("events");
        let types: Vec<&str> = stored
            .iter()
            .map(|event| event.event_type.as_str())
            .collect();
        let registered = types
            .iter()
            .position(|event_type| *event_type == "experiment_registered")
            .expect("experiment_registered landed");
        let started = types
            .iter()
            .position(|event_type| *event_type == "experiment_started")
            .expect("experiment_started landed");
        assert!(
            registered < started,
            "the registration precedes the confirmed start: {types:?}"
        );
        assert!(stored
            .iter()
            .all(|event| event.publisher_id.as_deref() == Some("northstar")));
        let registration = &stored[registered];
        assert_eq!(
            registration.payload.registered_metric.as_deref(),
            Some("revenue_per_user")
        );
        assert_eq!(registration.payload.window_days, Some(14));
        assert!(registration.payload.source_digest.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_refused_platform_start_still_emits_the_registration() {
        use apex_analytics::store::{EventStore as _, ProjectionStore as _};

        let dir = std::env::temp_dir().join(format!("apex-cycle-refused-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let snapshot_path = dir.join("s.json");
        std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot()).unwrap()).unwrap();

        let ops = Arc::new(unused_ops().await);
        // Every platform start is refused.
        ops.script_start(
            (0..10)
                .map(|_| Err(apex_core::PlatformError::Validation("refused".into())))
                .collect(),
        );
        let store = Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = apex_analytics::AnalyticsClient::new(store.clone(), "dev", "apex-runner-test");
        let events = Arc::new(
            crate::events::RunnerEvents::new(Arc::new(client), "northstar").with_environment("dev"),
        );
        let runner = Runner::new(
            ops,
            AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_priors(seeded_priors(), dir.join("priors.json"))
        .with_events(events);
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[4u8; 32]));

        let report = run(
            &runner,
            &mut ledger,
            std::slice::from_ref(&snapshot_path),
            1_700_000_000_000,
        )
        .await
        .expect("cycle");
        assert!(report.applied == 0);
        let types: Vec<String> = store
            .load_events()
            .await
            .expect("events")
            .into_iter()
            .map(|event| event.event_type.as_str().to_string())
            .collect();
        assert!(
            types.contains(&"experiment_registered".to_string()),
            "the registration is a fact even when the start is refused: {types:?}"
        );
        assert!(
            !types.contains(&"experiment_started".to_string()),
            "a refused start is never reported as started"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
