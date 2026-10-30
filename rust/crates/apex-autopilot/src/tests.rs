//! Adversarial tests for the autopilot loop. Fixtures are built through the
//! real audit engine (no hand-rolled Finding literals), the ops double logs
//! every call, and the ledger is the real proof-crate InMemoryLedger.

use apex_audit::{run as run_audit, Severity, Snapshot};
use apex_core::{ExperimentResult, ExperimentState, PlatformError, PlatformId};
use apex_lift::guardrails::FillFormat;
use apex_lift::window::ArmDay;
use apex_proof::{ChangeKind, InMemoryLedger, Keypair, LedgerRecord};
use serde_json::json;

use std::sync::Arc;

use crate::keys::{experiment_name, idempotency_key, operation_of, platform_of};
use crate::ledger_index::LedgerIndex;
use crate::model::{Action, ActionOutcome, AutopilotPolicy, PlanError, Proposal, SettleOutcome};
use crate::ops::{ExperimentDays, PlatformOps, RecordingOps, UnmeasuredConclusion};
use crate::settle::EditCheck;
use crate::{run_cycle, settle_experiment};

/// A scripted adapter whose reporting read sleeps, so tests can observe how
/// many pulls ran at once and how long a pass took. Every other call
/// delegates to the recording adapter.
pub(crate) struct DelayedOps {
    pub(crate) inner: std::sync::Arc<RecordingOps>,
    delay: std::time::Duration,
    fail_id: Option<String>,
    in_flight: std::sync::atomic::AtomicUsize,
    max_in_flight: std::sync::atomic::AtomicUsize,
}

impl DelayedOps {
    pub(crate) fn new(
        inner: std::sync::Arc<RecordingOps>,
        delay: std::time::Duration,
        fail_id: Option<String>,
    ) -> Self {
        Self {
            inner,
            delay,
            fail_id,
            in_flight: std::sync::atomic::AtomicUsize::new(0),
            max_in_flight: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl PlatformOps for DelayedOps {
    fn supports_player_feed(&self) -> bool {
        false
    }

    fn platform(&self) -> PlatformId {
        self.inner.platform()
    }

    async fn writable(&self, operation: &str) -> bool {
        self.inner.writable(operation).await
    }

    async fn start_experiment(
        &self,
        request: &crate::ops::ExperimentRequest,
    ) -> Result<String, PlatformError> {
        self.inner.start_experiment(request).await
    }

    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError> {
        self.inner.experiment(experiment_id).await
    }

    fn measures_variants(&self) -> bool {
        self.inner.measures_variants()
    }

    fn promotion_requires_manual_action(&self) -> bool {
        self.inner.promotion_requires_manual_action()
    }

    async fn promotion_status(
        &self,
        experiment_id: &str,
        change: &apex_proof::ChangeKind,
    ) -> Result<crate::ops::PromotionStatus, PlatformError> {
        self.inner.promotion_status(experiment_id, change).await
    }

    async fn conclude_unmeasured_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<UnmeasuredConclusion, PlatformError> {
        self.inner
            .conclude_unmeasured_experiment(experiment_id)
            .await
    }

    async fn experiment_days(&self, experiment_id: &str) -> Result<ExperimentDays, PlatformError> {
        use std::sync::atomic::Ordering;
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        let result = if self.fail_id.as_deref() == Some(experiment_id) {
            Err(PlatformError::Validation(format!(
                "injected pull failure for {experiment_id}"
            )))
        } else {
            Ok(ExperimentDays {
                control: Vec::new(),
                test: Vec::new(),
            })
        };
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        result
    }

    async fn promote_experiment(&self, experiment_id: &str) -> Result<(), PlatformError> {
        self.inner.promote_experiment(experiment_id).await
    }

    async fn deprecate_experiment(&self, experiment_id: &str) -> Result<(), PlatformError> {
        self.inner.deprecate_experiment(experiment_id).await
    }

    async fn apply(&self, action: &Action, key: &str) -> Result<(), PlatformError> {
        self.inner.apply(action, key).await
    }

    async fn rollback(&self, action: &Action) -> Result<(), PlatformError> {
        self.inner.rollback(action).await
    }

    fn call_log(&self) -> Vec<String> {
        self.inner.call_log()
    }

    async fn current_settings(&self, ad_unit_id: &str) -> Result<String, PlatformError> {
        self.inner.current_settings(ad_unit_id).await
    }

    async fn scope_revenue(
        &self,
        ad_unit_id: &str,
        start: &str,
        end: &str,
    ) -> Result<apex_core::Micros, PlatformError> {
        self.inner.scope_revenue(ad_unit_id, start, end).await
    }
}

// ---------- fixtures ----------

fn keys() -> Keypair {
    Keypair::from_bytes(&[7u8; 32])
}

fn ledger() -> InMemoryLedger {
    InMemoryLedger::new(keys())
}

fn snapshot_from(value: serde_json::Value) -> Snapshot {
    serde_json::from_value(value).expect("snapshot fixture")
}

/// A snapshot with a Critical finding (no app-ads.txt present).
fn critical_findings() -> Vec<apex_audit::Finding> {
    let snapshot = snapshot_from(json!({
        "platform": "admob",
        "fetchedAtUnixMs": 1_700_000_000_000i64,
        "adUnits": [],
        "waterfall": [],
        "mediationGroups": [],
        "bidderAccounts": [],
        "countryRevenue": [],
        "appAdsTxt": { "present": false, "lines": [], "expectedNetworks": [] },
        "platformTotals30d": { "revenue30d": 0, "impressions30d": 0 },
        "mediationTotals30d": { "revenue30d": 0, "impressions30d": 0 }
    }));
    let report = run_audit(&snapshot);
    assert!(report
        .findings
        .iter()
        .any(|f| f.severity == Severity::Critical));
    report.findings
}

/// A snapshot whose only finding is a Warning (3% reporting gap).
fn warning_only_findings() -> Vec<apex_audit::Finding> {
    let mut snapshot = snapshot_from(json!({
        "platform": "admob",
        "fetchedAtUnixMs": 1_700_000_000_000i64,
        "adUnits": [],
        "waterfall": [],
        "mediationGroups": [],
        "bidderAccounts": [],
        "countryRevenue": [],
        "appAdsTxt": { "present": true, "lines": [], "expectedNetworks": [] },
        "platformTotals30d": { "revenue30d": 10_000_000, "impressions30d": 10_000 },
        "mediationTotals30d": { "revenue30d": 9_700_000, "impressions30d": 10_000 }
    }));
    // The fixture measures both sides of the gap; the flag is a transient
    // in-memory signal (it is not part of the stored snapshot contract).
    snapshot.mediation_totals_reported = true;
    let report = run_audit(&snapshot);
    assert!(report
        .findings
        .iter()
        .all(|f| f.severity != Severity::Critical));
    assert!(report
        .findings
        .iter()
        .any(|f| f.severity == Severity::Warning));
    report.findings
}

const NOW: i64 = 1_700_000_000_000;
const DAY: i64 = 86_400_000;

fn floor_action() -> Action {
    Action::SetCountryFloor {
        ad_unit_id: "u1".into(),
        country: "ee".into(),
        floor_ecpm: 2_500_000,
    }
}

fn ecpm_action() -> Action {
    Action::SetMappingEcpm {
        mediation_group_id: "g1".into(),
        ad_unit_id: "u2".into(),
        ecpm: 1_750_000,
    }
}

fn proposal(action: Action) -> Proposal {
    Proposal {
        action,
        rationale: "raise the value after the audit found a stale baseline".into(),
        expected_effect_bp: 500,
        prev_value: "{\"cpm\":1.0}".into(),
    }
}

fn seed_change(ledger: &mut InMemoryLedger, at: i64, key: &str) -> u64 {
    ledger
        .append(LedgerRecord::change_applied(
            at,
            PlatformId::Admob,
            "u1",
            ChangeKind::EcpmSet {
                mediation_group_id: "g1".into(),
                ad_unit_id: "u1".into(),
                ecpm: 1_000_000,
            },
            key,
            "{\"cpm\":1.0}",
        ))
        .expect("seed change")
        .seq()
}

// ---------- lift-engine fixtures ----------

/// A seven-day analysis window needs the three calendar burn-in days plus the
/// seven analyzed days: ten pulls per arm.
const WINDOW_PULL_DAYS: usize = 10;

/// A seven-day window with no interim look and no extension, so the day
/// fixtures in this file line up exactly with the decision the engine makes.
fn settle_policy() -> AutopilotPolicy {
    AutopilotPolicy {
        window_days: 7,
        interim_look: false,
        extension_allowed: false,
        ..AutopilotPolicy::conservative()
    }
}

fn arm_day(index: usize, revenue: i64) -> ArmDay {
    ArmDay {
        day: format!("2026-10-{:02}", index + 1),
        revenue,
        users: None,
        second_pull_revenue: Some(revenue),
        second_pull_users: None,
        fill: None,
        second_pull_fill: None,
        first_observed_at_unix_s: Some(1_700_000_000),
        second_observed_at_unix_s: Some(1_700_000_000 + apex_lift::window::SECOND_PULL_MIN_SECS),
        first_source_sha256: Some("a".repeat(64)),
        second_source_sha256: Some("b".repeat(64)),
    }
}

/// Constant daily arm values. MAX derives the per-user value by dividing the
/// reported revenue by the arm's allocated share (0.75 control, 0.25 test),
/// so the raw revenue is scaled back to make the *values* exactly the pair.
fn constant_days(control_value: i64, test_value: i64) -> ExperimentDays {
    ExperimentDays {
        control: (0..WINDOW_PULL_DAYS)
            .map(|i| arm_day(i, control_value / 4 * 3))
            .collect(),
        test: (0..WINDOW_PULL_DAYS)
            .map(|i| arm_day(i, test_value / 4))
            .collect(),
    }
}

/// Both arms collapse by more than half on two consecutive days: a platform
/// incident that voids the test.
fn incident_days() -> ExperimentDays {
    let mut control: Vec<ArmDay> = (0..WINDOW_PULL_DAYS)
        .map(|i| arm_day(i, 4_000_000))
        .collect();
    let mut test: Vec<ArmDay> = (0..WINDOW_PULL_DAYS)
        .map(|i| arm_day(i, 1_100_000))
        .collect();
    control[7] = arm_day(7, 1_800_000);
    control[8] = arm_day(8, 800_000);
    test[7] = arm_day(7, 500_000);
    test[8] = arm_day(8, 220_000);
    ExperimentDays { control, test }
}

/// A live experiment: the platform has not promoted or deprecated it yet.
fn running_experiment() -> ExperimentResult {
    ExperimentResult {
        experiment_id: "exp-1".into(),
        state: ExperimentState::Running,
        results: vec![],
        lift_percent_bp: None,
        confidence: None,
        winner_variant_id: None,
    }
}

/// Drive one full cycle so the ledger carries the signed pre-registration and
/// the experiment-start record the settle path reads back. Returns the
/// `changeApplied` sequence number.
async fn start_experiment(
    ops: &crate::ops::RecordingOps,
    ledger: &mut InMemoryLedger,
    at: i64,
) -> u64 {
    ops.set_change_in_test_arm(true);
    ops.script_start(vec![Ok("exp-1".into())]);
    let outcome = run_cycle(
        ops,
        ledger,
        &settle_policy(),
        &[],
        vec![proposal(floor_action())],
        at,
    )
    .await
    .expect("cycle");
    match outcome.outcomes.into_iter().next() {
        Some(ActionOutcome::Applied { change_seq, .. }) => change_seq,
        other => panic!("expected Applied, got {other:?}"),
    }
}

async fn settle(
    ops: &crate::ops::RecordingOps,
    ledger: &mut InMemoryLedger,
    experiment_id: &str,
    change_seq: u64,
    at: i64,
) -> Result<SettleOutcome, PlanError> {
    settle_experiment(
        ops,
        ledger,
        experiment_id,
        change_seq,
        FillFormat::Rewarded,
        &EditCheck::VerifiedSame,
        at,
    )
    .await
}

/// Start an AdMob mediation-floor experiment through the real cycle. AdMob
/// reports no per-variant measurement, so the adapter capability is off.
async fn start_admob_experiment(
    ops: &crate::ops::RecordingOps,
    ledger: &mut InMemoryLedger,
    at: i64,
) -> u64 {
    ops.set_platform(PlatformId::Admob);
    ops.set_change_in_test_arm(true);
    ops.set_measures_variants(false);
    ops.script_start(vec![Ok("exp-1".into())]);
    let outcome = run_cycle(
        ops,
        ledger,
        &settle_policy(),
        &[],
        vec![proposal(Action::SetAdmobMediationFloor {
            group_id: "g1".into(),
            floor_micros: 1_200_000,
        })],
        at,
    )
    .await
    .expect("cycle");
    match outcome.outcomes.into_iter().next() {
        Some(ActionOutcome::Applied { change_seq, .. }) => change_seq,
        other => panic!("expected Applied, got {other:?}"),
    }
}

/// The registered AdMob window is the 3 calendar burn-in days plus the 7
/// analysis days; a start day is complete once the next day passes day 10.
const ADMOB_WINDOW_END: i64 = NOW + 10 * DAY;

fn conclusion(leader: &str, keep_treatment: bool) -> UnmeasuredConclusion {
    UnmeasuredConclusion {
        experiment_state: "ENDED".into(),
        variant_leader: Some(leader.into()),
        keep_treatment,
    }
}

// ---------- keys & attribution ----------

#[test]
fn idempotency_keys_are_pinned_exactly() {
    assert_eq!(
        idempotency_key(&floor_action()).unwrap(),
        "max-floor:u1:EE:2.5"
    );
    assert_eq!(
        idempotency_key(&ecpm_action()).unwrap(),
        "admob-ecpm:g1:u2:1.75"
    );
    assert_eq!(
        idempotency_key(&Action::SetWaterfallPrice {
            ad_unit_id: "u1".into(),
            entry_id: "e1".into(),
            price_ecpm: 2_345_000,
        })
        .unwrap(),
        "levelplay-price:u1:e1:2.345"
    );
    assert_eq!(
        idempotency_key(&Action::CreateWaterfallEntry {
            ad_unit_id: "u1".into(),
            network: "mintegral".into(),
            instance_name: "MTG_1".into(),
            price_ecpm: 750_000,
        })
        .unwrap(),
        "levelplay-create:u1:mintegral:MTG_1:0.75"
    );
    assert_eq!(
        idempotency_key(&Action::RemoveWaterfallEntry {
            ad_unit_id: "u1".into(),
            entry_id: "e9".into(),
        })
        .unwrap(),
        "levelplay-remove:u1:e9"
    );
}

#[test]
fn invalid_floor_country_is_a_typed_error() {
    let bad = Action::SetCountryFloor {
        ad_unit_id: "u1".into(),
        country: "EST".into(),
        floor_ecpm: 1,
    };
    assert!(idempotency_key(&bad).is_err());
}

#[test]
fn experiment_names_are_deterministic_and_case_stable() {
    let a = experiment_name(&floor_action());
    let b = experiment_name(&Action::SetCountryFloor {
        ad_unit_id: "u1".into(),
        country: "EE".into(),
        floor_ecpm: 2_500_000,
    });
    assert_eq!(a, b, "country case must not change the experiment key");
    assert_eq!(a, "max-experiment:autopilot-floor-u1-ee-2-5:u1");
    assert_eq!(
        experiment_name(&ecpm_action()),
        "max-experiment:autopilot-ecpm-g1-u2-1-75:u2"
    );
}

#[test]
fn platform_and_operation_attribution_is_pinned() {
    assert_eq!(platform_of(&floor_action()), PlatformId::AppLovinMax);
    assert_eq!(platform_of(&ecpm_action()), PlatformId::Admob);
    assert_eq!(
        platform_of(&Action::SetWaterfallPrice {
            ad_unit_id: "u".into(),
            entry_id: "e".into(),
            price_ecpm: 1
        }),
        PlatformId::LevelPlay
    );
    assert_eq!(operation_of(&floor_action()), "setCountryFloor");
    assert_eq!(operation_of(&ecpm_action()), "updateMediationGroup");
}

#[test]
fn the_frozen_instance_and_mediation_actions_keep_their_platform_and_fee_rules() {
    let instance = Action::SetLevelplayInstancePrice {
        instance_id: "i1".into(),
        price_micros: 2_400_000,
    };
    assert_eq!(platform_of(&instance), PlatformId::LevelPlay);
    assert_eq!(operation_of(&instance), "updateEntryPrice");
    assert_eq!(
        idempotency_key(&instance).expect("key"),
        "levelplay-price:i1:i1:2.4",
        "the instance id is the existing price key's scope"
    );

    let floor = Action::SetAdmobMediationFloor {
        group_id: "g1".into(),
        floor_micros: 1_200_000,
    };
    assert_eq!(platform_of(&floor), PlatformId::Admob);
    assert_eq!(operation_of(&floor), "updateMediationGroup");
    assert_eq!(
        idempotency_key(&floor).expect("key"),
        "admob-ecpm:g1:g1:1.2"
    );
    // The existing fee eligibility rule is platform-based: an AdMob kept
    // change (including the new mediation-floor action) stays fee-ineligible.
    let ineligible =
        apex_lift::billing::fee_eligibility(platform_of(&floor)).expect_err("AdMob has no fee");
    assert_eq!(
        ineligible.reason,
        apex_lift::billing::ADMOB_NO_MEASUREMENT_REASON
    );
}

// ---------- cycle: gates ----------

#[tokio::test]
async fn critical_findings_blackout_every_write() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &critical_findings(),
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect_err("blackout");
    match err {
        PlanError::PolicyViolation { reason } => assert!(reason.contains("blackout")),
        other => panic!("expected PolicyViolation, got {other}"),
    }
    assert!(
        ops.call_log().is_empty(),
        "blackout must cost zero platform calls"
    );
    assert!(l.is_empty());
}

#[tokio::test]
async fn blackout_can_be_disabled_by_policy() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let policy = AutopilotPolicy {
        blackout_on_critical_findings: false,
        ..AutopilotPolicy::conservative()
    };
    let outcome = run_cycle(
        &ops,
        &mut l,
        &policy,
        &critical_findings(),
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("proceeds");
    assert_eq!(outcome.applied, 1);
}

#[tokio::test]
async fn warning_only_findings_do_not_block() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &warning_only_findings(),
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("warnings do not blackout");
    assert_eq!(outcome.applied, 1);
}

#[tokio::test]
async fn daily_cap_is_counted_from_the_ledger() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    for i in 0..3 {
        seed_change(&mut l, NOW - i, &format!("seed-{i}"));
    }
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect_err("cap");
    assert!(matches!(err, PlanError::DailyCapReached { n: 3 }));
    assert!(ops.call_log().is_empty());
}

#[tokio::test]
async fn daily_cap_ignores_other_days() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    for i in 0..5 {
        seed_change(&mut l, NOW - DAY + i, &format!("yesterday-{i}"));
    }
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("yesterday does not count");
    assert_eq!(outcome.applied, 1);
}

#[tokio::test]
async fn cap_boundary_mid_cycle_skips_the_rest() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let policy = AutopilotPolicy {
        max_actions_per_day: 2,
        ..AutopilotPolicy::conservative()
    };
    let a = proposal(floor_action());
    let b = proposal(ecpm_action());
    let c = proposal(Action::SetWaterfallPrice {
        ad_unit_id: "u3".into(),
        entry_id: "e1".into(),
        price_ecpm: 1_100_000,
    });
    let outcome = run_cycle(&ops, &mut l, &policy, &[], vec![a, b, c], NOW)
        .await
        .expect("cycle");
    assert_eq!(
        outcome.outcomes,
        vec![
            ActionOutcome::Applied {
                change_seq: 1,
                experiment_id: "max-experiment:autopilot-floor-u1-ee-2-5:u1".into(),
                idempotency_key: "max-floor:u1:EE:2.5".into(),
            },
            ActionOutcome::Applied {
                change_seq: 6,
                experiment_id: "max-experiment:autopilot-ecpm-g1-u2-1-75:u2".into(),
                idempotency_key: "admob-ecpm:g1:u2:1.75".into(),
            },
            ActionOutcome::Skipped {
                reason: "daily_cap_reached".into()
            },
        ]
    );
    assert_eq!(outcome.applied, 2);
    assert_eq!(outcome.skipped, 1);
    let applies = ops
        .call_log()
        .iter()
        .filter(|entry| entry.starts_with("apply:"))
        .count();
    assert_eq!(applies, 2, "the capped proposal costs zero applies");
}

#[tokio::test]
async fn step_cap_refuses_before_any_platform_call() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let aggressive = Proposal {
        action: floor_action(),
        prev_value: "{\"floor\": 1000000}".into(),
        rationale: "too big a step".into(),
        expected_effect_bp: 2_001,
    };
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![aggressive],
        NOW,
    )
    .await
    .expect_err("step cap");
    assert!(matches!(err, PlanError::PolicyViolation { .. }));
    assert!(ops.call_log().is_empty());
}

#[tokio::test]
async fn step_cap_boundary_is_inclusive() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let at_cap = Proposal {
        action: floor_action(),
        prev_value: "{\"floor\": 1000000}".into(),
        rationale: "exactly at the cap".into(),
        expected_effect_bp: 2_000,
    };
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![at_cap],
        NOW,
    )
    .await
    .expect("boundary allowed");
    assert_eq!(outcome.applied, 1);
}

#[tokio::test]
async fn zero_expected_effect_is_refused() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let nil = Proposal {
        action: floor_action(),
        prev_value: "{\"floor\": 1000000}".into(),
        rationale: "no stated effect".into(),
        expected_effect_bp: 0,
    };
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![nil],
        NOW,
    )
    .await
    .expect_err("zero effect");
    assert!(matches!(err, PlanError::PolicyViolation { .. }));
    assert!(ops.call_log().is_empty());
}

#[tokio::test]
async fn verify_live_endpoints_hold_the_proposal_without_aborting() {
    // The adapter's verified write set does not include the operation: the
    // proposal is held as a typed awaiting state and the pass continues —
    // nothing is written and no other proposal is aborted.
    let ops = crate::ops::RecordingOps::new();
    ops.set_writable("updateEntryPrice", false);
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(Action::SetWaterfallPrice {
            ad_unit_id: "u3".into(),
            entry_id: "e1".into(),
            price_ecpm: 1_100_000,
        })],
        NOW,
    )
    .await
    .expect("the pass continues past an unverified endpoint");
    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.skipped, 1);
    assert_eq!(
        outcome.awaiting_verification,
        vec!["awaiting live verification for AppLovinMax: updateEntryPrice".to_string()]
    );
    assert_eq!(
        outcome.outcomes,
        vec![ActionOutcome::Skipped {
            reason: "awaiting live verification for AppLovinMax: updateEntryPrice".into(),
        }]
    );
    assert!(l.is_empty(), "a held proposal writes nothing");
    let log = ops.call_log();
    assert_eq!(log, vec!["writable:updateEntryPrice".to_string()]);
}

#[tokio::test]
async fn a_capability_hold_records_only_the_ineligible_operation() {
    // One writable + one unverified operation: the writable proposal starts
    // its experiment, the unverified one is held with its typed reason and
    // nothing else is recorded against it.
    let ops = crate::ops::RecordingOps::new();
    ops.set_platform(PlatformId::Admob);
    ops.set_change_in_test_arm(true);
    ops.set_writable("updateMediationGroup", false);
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action()), proposal(ecpm_action())],
        NOW,
    )
    .await
    .expect("the pass continues");
    assert_eq!(outcome.applied, 1, "the writable proposal applies");
    assert_eq!(outcome.skipped, 1, "only the unverified op is held");
    assert_eq!(
        outcome.awaiting_verification,
        vec!["awaiting live verification for Admob: updateMediationGroup".to_string()]
    );
    assert!(outcome
        .outcomes
        .iter()
        .any(|outcome| matches!(outcome, ActionOutcome::Applied { .. })));
    assert!(outcome.outcomes.iter().any(|outcome| matches!(
        outcome,
        ActionOutcome::Skipped { reason }
            if reason == "awaiting live verification for Admob: updateMediationGroup"
    )));
}

#[tokio::test]
async fn a_max_floor_adapter_with_the_verified_operation_is_unchanged() {
    // The MAX path with its operation verified writable: the cycle applies
    // exactly as before and records no awaiting-verification state.
    let ops = crate::ops::RecordingOps::new();
    ops.set_change_in_test_arm(true);
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("cycle");
    assert_eq!(outcome.applied, 1);
    assert_eq!(outcome.skipped, 0);
    assert!(outcome.awaiting_verification.is_empty());
}

// ---------- cycle: apply paths ----------

#[tokio::test]
async fn the_frozen_actions_record_the_existing_live_change_kinds() {
    // LevelPlay's instance price and AdMob's mediation floor ride the existing
    // ledger change kinds the live adapters already understand; the ledger
    // platform comes from the action's platform attribution.
    let ops = crate::ops::RecordingOps::new();
    ops.set_change_in_test_arm(true);
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![
            proposal(Action::SetLevelplayInstancePrice {
                instance_id: "i1".into(),
                price_micros: 2_400_000,
            }),
            proposal(Action::SetAdmobMediationFloor {
                group_id: "g1".into(),
                floor_micros: 1_200_000,
            }),
        ],
        NOW,
    )
    .await
    .expect("cycle");
    assert_eq!(outcome.applied, 2);
    let mut changes = Vec::new();
    for record in l.records() {
        if let LedgerRecord::ChangeApplied {
            platform, change, ..
        } = record
        {
            changes.push((*platform, change.clone()));
        }
    }
    assert!(changes.contains(&(
        PlatformId::LevelPlay,
        ChangeKind::WaterfallEntryPriceSet {
            ad_unit_id: "i1".into(),
            entry_id: "i1".into(),
            price_ecpm: 2_400_000,
        }
    )));
    assert!(changes.contains(&(
        PlatformId::Admob,
        ChangeKind::EcpmSet {
            mediation_group_id: "g1".into(),
            ad_unit_id: "g1".into(),
            ecpm: 1_200_000,
        }
    )));
}

#[tokio::test]
async fn apply_failure_leaves_no_change_record() {
    // An adapter that cannot carry the change in a test arm (LevelPlay/AdMob
    // here) applies first. The pre-registration is already signed, but a
    // failed apply must leave no change and start no experiment.
    let ops = crate::ops::RecordingOps::new();
    ops.script_apply(vec![Err(PlatformError::Auth {
        platform: PlatformId::LevelPlay,
        operation: "updateEntryPrice".into(),
        status: 401,
    })]);
    let mut l = ledger();
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(Action::SetWaterfallPrice {
            ad_unit_id: "u3".into(),
            entry_id: "e1".into(),
            price_ecpm: 1_100_000,
        })],
        NOW,
    )
    .await
    .expect_err("apply fails");
    assert!(matches!(err, PlanError::ApplyFailed { .. }));
    assert!(
        !l.records()
            .iter()
            .any(|r| matches!(r, LedgerRecord::ChangeApplied { .. })),
        "no change record for a failed apply"
    );
    let log = ops.call_log();
    assert!(
        !log.iter().any(|entry| entry.starts_with("start:")),
        "no experiment may start for a failed apply"
    );
}

#[tokio::test]
async fn successful_run_records_prereg_change_and_experiment() {
    // Order is load-bearing: the signed pre-registration precedes the change
    // and the experiment-start records.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(ecpm_action())],
        NOW,
    )
    .await
    .expect("cycle");
    assert_eq!(outcome.applied, 1);
    // The create saga adds the intent (before the platform call) and the
    // created confirmation (after it) to the change and start records. The
    // ecpm action is the apply-first path, so the change record precedes the
    // intent.
    assert_eq!(
        l.len(),
        5,
        "pre-registration + change + create intent + created + experiment"
    );
    assert!(matches!(l.get(0), Some(LedgerRecord::LiftRecord { .. })));
    match (l.get(1), l.get(2)) {
        (
            Some(LedgerRecord::ChangeApplied {
                idempotency_key, ..
            }),
            Some(LedgerRecord::ExperimentCreateIntent {
                experiment_id,
                spec,
                ..
            }),
        ) => {
            assert_eq!(idempotency_key, "admob-ecpm:g1:u2:1.75");
            assert_eq!(experiment_id, "max-experiment:autopilot-ecpm-g1-u2-1-75:u2");
            assert_eq!(spec.name, *experiment_id);
        }
        other => panic!("unexpected create saga records: {other:?}"),
    }
    match (l.get(3), l.get(4)) {
        (
            Some(LedgerRecord::ExperimentCreated { external_id, .. }),
            Some(LedgerRecord::ExperimentStarted { experiment_id, .. }),
        ) => {
            assert_eq!(external_id, "max-experiment:autopilot-ecpm-g1-u2-1-75:u2");
            assert_eq!(experiment_id, "max-experiment:autopilot-ecpm-g1-u2-1-75:u2");
        }
        other => panic!("unexpected ledger shape: {other:?}"),
    }
}

#[tokio::test]
async fn the_preregistration_and_start_share_one_instance_id() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(ecpm_action())],
        NOW,
    )
    .await
    .expect("cycle");

    let prereg = l
        .records()
        .iter()
        .find_map(|record| match record {
            apex_proof::LedgerRecord::LiftRecord { payload, .. } => {
                match serde_json::from_str::<apex_lift::records::LiftRecord>(payload).ok()? {
                    record @ apex_lift::records::LiftRecord::PreRegistration { .. } => {
                        apex_lift::decision::PreRegistration::from_record(&record).ok()
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .expect("signed pre-registration");
    let spec = l
        .records()
        .iter()
        .find_map(|record| match record {
            apex_proof::LedgerRecord::ExperimentStarted { spec, .. } => Some(spec.clone()),
            _ => None,
        })
        .expect("experiment start");

    assert!(!prereg.instance_id.trim().is_empty());
    assert_eq!(prereg.instance_id, spec.instance_id);
    assert_ne!(
        prereg.instance_id, spec.name,
        "the billing identity is not the reusable experiment name"
    );
    assert_eq!(prereg.experiment_id, spec.name);

    // A later attempt on the same name has a different deterministic id.
    let second = crate::keys::experiment_instance_id(&spec.name, NOW, "admob-ecpm:g1:u2:1.75", 1);
    let first = crate::keys::experiment_instance_id(&spec.name, NOW, "admob-ecpm:g1:u2:1.75", 0);
    assert_ne!(first, second);
    assert_eq!(
        first,
        crate::keys::experiment_instance_id(&spec.name, NOW, "admob-ecpm:g1:u2:1.75", 0),
        "replaying the same attempt is deterministic"
    );
}

#[tokio::test]
async fn every_action_shape_validates_its_values() {
    // Each action's value bounds are checked before any platform call.
    let cases = [
        Action::SetWaterfallPrice {
            ad_unit_id: "u".into(),
            entry_id: "e".into(),
            price_ecpm: 0,
        },
        Action::CreateWaterfallEntry {
            ad_unit_id: "u".into(),
            network: " ".into(),
            instance_name: "i".into(),
            price_ecpm: 1_000_000,
        },
        Action::RemoveWaterfallEntry {
            ad_unit_id: " ".into(),
            entry_id: "e".into(),
        },
    ];
    for action in cases {
        let ops = crate::ops::RecordingOps::new();
        let mut l = ledger();
        let err = run_cycle(
            &ops,
            &mut l,
            &AutopilotPolicy::conservative(),
            &[],
            vec![proposal(action.clone())],
            NOW,
        )
        .await
        .expect_err("invalid action");
        assert!(matches!(err, PlanError::Validation(_)), "{action:?}: {err}");
        assert!(l.is_empty(), "no ledger write for an invalid action");
    }
}

/// An `InMemoryLedger` that counts `records()` calls, so a test can pin that
/// the cycle indexes the ledger once instead of rescanning it per proposal.
struct CountingLedger {
    inner: InMemoryLedger,
    records_calls: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl apex_proof::LedgerLog for CountingLedger {
    fn append(&mut self, draft: LedgerRecord) -> Result<LedgerRecord, apex_proof::LedgerError> {
        self.inner.append(draft)
    }

    fn append_rollback(
        &mut self,
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: &str,
        rolled_back_seq: u64,
        reason: &str,
    ) -> Result<LedgerRecord, apex_proof::LedgerError> {
        self.inner
            .append_rollback(at_unix_ms, platform, ad_unit_id, rolled_back_seq, reason)
    }

    fn records(&self) -> Vec<&LedgerRecord> {
        self.records_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.records()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn get(&self, index: usize) -> Option<&LedgerRecord> {
        self.inner.get(index)
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[tokio::test]
async fn an_approval_pass_indexes_the_ledger_once_for_ten_thousand_proposals() {
    // A LevelPlay/AdMob snapshot can yield 10k candidates. The ledger gates
    // must be indexed once, not rebuilt per proposal: with N proposals this
    // used to be 2N full ledger scans (the applied check reads twice).
    const N: usize = 10_000;
    const PRIOR: usize = 100;
    let records_calls = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut inner = ledger();
    for index in 0..PRIOR {
        seed_change(&mut inner, NOW - DAY, &format!("prior-{index}"));
    }
    let mut counting = CountingLedger {
        inner,
        records_calls: std::sync::Arc::clone(&records_calls),
    };
    let proposals: Vec<Proposal> = (0..N)
        .map(|index| {
            proposal(Action::SetCountryFloor {
                ad_unit_id: format!("u{index}"),
                country: "ee".into(),
                floor_ecpm: 2_500_000,
            })
        })
        .collect();
    let policy = AutopilotPolicy {
        approval_mode: true,
        ..AutopilotPolicy::conservative()
    };
    let outcome = run_cycle(
        &crate::ops::RecordingOps::new(),
        &mut counting,
        &policy,
        &[],
        proposals,
        NOW,
    )
    .await
    .expect("cycle");
    assert_eq!(outcome.awaiting_approval as usize, N);
    assert_eq!(
        records_calls.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "one index build serves both the gates and the daily-cap count, \
         independent of proposal count"
    );
}

#[tokio::test]
async fn approval_mode_holds_proposals_without_writing() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let policy = AutopilotPolicy {
        approval_mode: true,
        ..AutopilotPolicy::conservative()
    };
    let outcome = run_cycle(
        &ops,
        &mut l,
        &policy,
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("cycle");
    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.awaiting_approval, 1);
    assert_eq!(
        outcome.outcomes,
        vec![ActionOutcome::AwaitingApproval {
            idempotency_key: "max-floor:u1:EE:2.5".into(),
        }]
    );
    assert!(l.is_empty(), "approval mode writes nothing");
    let writes = ops
        .call_log()
        .iter()
        .filter(|e| e.starts_with("apply:") || e.starts_with("start:"))
        .count();
    assert_eq!(writes, 0, "approval mode costs zero platform writes");
}

#[tokio::test]
async fn an_approved_key_unlocks_the_change_and_a_declined_key_is_final() {
    use std::collections::HashSet;
    let policy = AutopilotPolicy {
        approval_mode: true,
        ..AutopilotPolicy::conservative()
    };
    let key = "max-floor:u1:EE:2.5".to_string();
    let empty = HashSet::new();

    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let outcome = crate::plan::run_cycle_observed_with(
        &ops,
        &mut l,
        &policy,
        &[],
        &[proposal(floor_action())],
        None,
        &HashSet::from([key.clone()]),
        &empty,
        NOW,
    )
    .await
    .expect("approved cycle");
    assert_eq!(outcome.applied, 1, "the approved change proceeds");
    assert_eq!(outcome.awaiting_approval, 0);
    assert!(!l.is_empty(), "the pre-registration is signed");

    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let outcome = crate::plan::run_cycle_observed_with(
        &ops,
        &mut l,
        &policy,
        &[],
        &[proposal(floor_action())],
        None,
        &empty,
        &HashSet::from([key]),
        NOW,
    )
    .await
    .expect("declined cycle");
    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.skipped, 1);
    assert_eq!(
        outcome.outcomes,
        vec![ActionOutcome::Skipped {
            reason: "approval_declined".into(),
        }]
    );
    assert!(l.is_empty(), "a declined proposal writes nothing");
}

#[test]
fn a_new_publisher_is_in_approval_mode_for_thirty_days() {
    let policy = AutopilotPolicy {
        onboarding_unix_ms: Some(NOW),
        ..AutopilotPolicy::conservative()
    };
    assert!(policy.approval_required(NOW));
    assert!(policy.approval_required(NOW + 29 * 86_400_000));
    assert!(!policy.approval_required(NOW + 31 * 86_400_000));
    assert!(!AutopilotPolicy::conservative().approval_required(NOW));
}

#[tokio::test]
async fn pre_registration_is_signed_before_the_platform_call() {
    // The registration is in the ledger even though the platform call fails:
    // it is written first, and a failed start cannot retract it. The split is
    // the pre-registered share, never a fixed 50.
    let ops = crate::ops::RecordingOps::new();
    ops.set_change_in_test_arm(true);
    ops.script_start(vec![Err(PlatformError::Http {
        platform: PlatformId::AppLovinMax,
        operation: "createExperiment".into(),
        status: 503,
    })]);
    let mut l = ledger();
    run_cycle(
        &ops,
        &mut l,
        &settle_policy(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("cycle");

    let Some(LedgerRecord::LiftRecord { payload, .. }) = l.get(0) else {
        panic!("the first ledger record must be the signed pre-registration");
    };
    let record: apex_lift::records::LiftRecord = serde_json::from_str(payload).expect("payload");
    match record {
        apex_lift::records::LiftRecord::PreRegistration {
            experiment_id,
            split,
            window_days,
            ..
        } => {
            assert_eq!(experiment_id, experiment_name(&floor_action()));
            assert!((split - 0.25).abs() < 1e-12, "pre-registered split {split}");
            assert_eq!(window_days, 7);
        }
        other => panic!("expected a pre-registration, got {other:?}"),
    }
    assert!(
        !l.records()
            .iter()
            .any(|r| matches!(r, LedgerRecord::ChangeApplied { .. })),
        "no change record when the experiment never started"
    );
}

#[tokio::test]
async fn a_retry_does_not_start_a_second_experiment() {
    // The first attempt signs the pre-registration, then the start fails with
    // no change record. A retry must not start a second experiment on the same
    // name; it is refused as in flight until an operator resolves it.
    let ops = crate::ops::RecordingOps::new();
    ops.set_change_in_test_arm(true);
    let error = || {
        Err(PlatformError::Http {
            platform: PlatformId::AppLovinMax,
            operation: "createExperiment".into(),
            status: 503,
        })
    };
    ops.script_start(vec![error(), error()]);
    let mut l = ledger();
    run_cycle(
        &ops,
        &mut l,
        &settle_policy(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("first");
    let second = run_cycle(
        &ops,
        &mut l,
        &settle_policy(),
        &[],
        vec![proposal(floor_action())],
        NOW + 1,
    )
    .await
    .expect("second");
    assert_eq!(
        second.outcomes,
        vec![ActionOutcome::Skipped {
            reason: "experiment_in_flight".into(),
        }]
    );
    let starts = ops
        .call_log()
        .iter()
        .filter(|entry| entry.starts_with("start:"))
        .count();
    assert_eq!(starts, 1, "the retry must not start a second experiment");
}

#[tokio::test]
async fn apply_path_start_failure_rolls_back_exactly_once() {
    // The adapter applied the change to the live unit; a failed start must
    // undo it and record the rollback against the change.
    let ops = crate::ops::RecordingOps::new();
    ops.script_start(vec![Err(PlatformError::Http {
        platform: PlatformId::Admob,
        operation: "createExperiment".into(),
        status: 503,
    })]);
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(ecpm_action())],
        NOW,
    )
    .await
    .expect("cycle completes with a rollback outcome");
    assert_eq!(
        outcome.outcomes,
        vec![ActionOutcome::RolledBack {
            change_seq: 1,
            reason: "experiment_start_failed".into()
        }]
    );
    assert_eq!(outcome.applied, 0, "a rolled-back start is not an apply");
    let rollbacks = ops
        .call_log()
        .iter()
        .filter(|entry| entry.starts_with("rollback:"))
        .count();
    assert_eq!(rollbacks, 1);
    match l.get(3) {
        Some(LedgerRecord::RollbackPerformed {
            rolled_back_seq,
            reason,
            ..
        }) => {
            // The rollback follows the create intent's records.
            assert_eq!(*rolled_back_seq, 1);
            assert_eq!(reason, "experiment_start_failed");
        }
        other => panic!("expected rollback record, got {other:?}"),
    }
}

#[tokio::test]
async fn a_max_non_split_refusal_is_audit_only_with_the_exact_reason() {
    // A MAX unit still on waterfall networks cannot be split into experiment
    // arms. The refusal is terminal: the pre-registration stays as the
    // audit-only fact, no retry is attempted and nothing is rolled back.
    let ops = crate::ops::RecordingOps::new();
    ops.set_change_in_test_arm(true);
    let detail =
        "android ad unit u1 cannot split the waterfall: it still serves network waterfall demand";
    ops.script_start(vec![Err(PlatformError::Schema {
        platform: PlatformId::AppLovinMax,
        operation: "createExperiment".into(),
        detail: detail.into(),
    })]);
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("cycle completes");
    match outcome.outcomes.as_slice() {
        [ActionOutcome::AuditOnly { reason }] => {
            assert!(
                reason.contains(detail),
                "the exact refusal is preserved: {reason}"
            );
        }
        other => panic!("expected audit-only, got {other:?}"),
    }
    assert_eq!(outcome.applied, 0);
    let log = ops.call_log();
    assert_eq!(
        log.iter()
            .filter(|entry| entry.starts_with("start:"))
            .count(),
        1,
        "the refused start is never retried: {log:?}"
    );
    assert!(!log.iter().any(|entry| entry.starts_with("rollback:")));
    // The signed pre-registration remains; no changeApplied and no
    // experimentStarted record claims the change ran.
    let types: Vec<&str> = l
        .records()
        .iter()
        .map(|record| match record {
            LedgerRecord::LiftRecord { .. } => "lift",
            LedgerRecord::ChangeApplied { .. } => "changeApplied",
            LedgerRecord::ExperimentStarted { .. } => "experimentStarted",
            _ => "other",
        })
        .collect();
    assert_eq!(
        types
            .iter()
            .filter(|kind| **kind == "changeApplied")
            .count(),
        0
    );
    assert_eq!(
        types
            .iter()
            .filter(|kind| **kind == "experimentStarted")
            .count(),
        0
    );
    assert_eq!(types.iter().filter(|kind| **kind == "lift").count(), 1);
    // The terminal refusal resolves the create intent as abandoned, so the
    // restart recovery never replays it.
    let index = LedgerIndex::build(&l);
    assert!(index.pending_creates().is_empty());
    assert!(index.is_abandoned("max-experiment:autopilot-floor-u1-ee-2-5:u1"));
}

/// The adapter source the recovery tests resolve through: one scripted
/// adapter answers for every platform.
struct SingleOpsSource(Arc<crate::ops::RecordingOps>);

impl crate::settle::PlatformOpsSource for SingleOpsSource {
    fn adapter(
        &self,
        _platform: apex_core::PlatformId,
    ) -> Result<&dyn crate::ops::PlatformOps, PlanError> {
        Ok(self.0.as_ref())
    }
}

#[tokio::test]
async fn an_open_create_intent_is_replayed_on_restart() {
    // The cycle's start fails transiently: the intent stays open, and the
    // next pass replays the exact recorded request (the platform create is
    // idempotent by name) and completes the durable bookkeeping.
    let ops = Arc::new(crate::ops::RecordingOps::new());
    ops.set_change_in_test_arm(true);
    ops.script_start(vec![Err(PlatformError::Http {
        platform: PlatformId::AppLovinMax,
        operation: "createExperiment".into(),
        status: 503,
    })]);
    let mut l = ledger();
    let outcome = run_cycle(
        ops.as_ref(),
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("cycle");
    assert!(matches!(
        outcome.outcomes.as_slice(),
        [ActionOutcome::RolledBack { .. }]
    ));
    let name = "max-experiment:autopilot-floor-u1-ee-2-5:u1";
    {
        let index = LedgerIndex::build(&l);
        assert_eq!(index.pending_creates().len(), 1);
        let intent = index.pending_creates()[0].clone();
        assert_eq!(intent.experiment_id, name);
        assert_eq!(intent.idempotency_key, "max-floor:u1:EE:2.5");
        assert_eq!(intent.spec.ad_unit_id, "u1");
    }
    assert!(
        !l.records()
            .iter()
            .any(|record| matches!(record, LedgerRecord::ChangeApplied { .. })),
        "the test-arm path records no change before the start"
    );

    // Restart: the platform answers this time.
    ops.script_start(vec![Ok("platform-exp-7".into())]);
    ops.put_experiment("platform-exp-7", running_experiment());
    let mut index = LedgerIndex::build(&l);
    let recovery = crate::settle::reconcile_experiment_creates(
        &mut l,
        &mut index,
        &SingleOpsSource(ops.clone()),
        NOW + 1,
    )
    .await;
    assert!(recovery.failures.is_empty(), "{:?}", recovery.failures);
    assert_eq!(recovery.recovered, vec![name.to_string()]);
    assert!(index.pending_creates().is_empty());
    assert!(index.is_created(name));
    let types: Vec<String> = l
        .records()
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::ExperimentCreated {
                experiment_id,
                external_id,
                ..
            } => Some(format!("created:{experiment_id}:{external_id}")),
            LedgerRecord::ChangeApplied {
                idempotency_key, ..
            } => Some(format!("change:{idempotency_key}")),
            LedgerRecord::ExperimentStarted { experiment_id, .. } => {
                Some(format!("started:{experiment_id}"))
            }
            _ => None,
        })
        .collect();
    assert!(types.contains(&format!("created:{name}:platform-exp-7")));
    assert!(types.contains(&"change:max-floor:u1:EE:2.5".to_string()));
    assert!(types.contains(&"started:platform-exp-7".to_string()));
    // The replayed create carries the recorded experiment name.
    let log = ops.call_log();
    assert!(
        log.iter()
            .any(|entry| entry.starts_with(&format!("start:{name}"))),
        "{log:?}"
    );
}

#[tokio::test]
async fn a_created_intent_completes_without_a_second_platform_call() {
    // A crash between the created confirmation and the start record: the
    // recovery completes the durable bookkeeping but never re-calls the
    // platform for an experiment it already confirmed.
    let ops = Arc::new(crate::ops::RecordingOps::new());
    let mut l = ledger();
    let spec = apex_core::ExperimentSpec {
        name: "floors-ee".into(),
        instance_id: "floors-ee@instance".into(),
        ad_unit_id: "u1".into(),
        metric: apex_core::ExperimentMetric::Revenue,
        variants: 2,
        change_description: "raise EE".into(),
        test_group_allocation: 10,
    };
    l.append(LedgerRecord::experiment_create_intent(
        NOW,
        PlatformId::AppLovinMax,
        "floors-ee",
        "u1",
        "max-floor:u1:EE:2.5",
        ChangeKind::CountryFloorSet {
            country: "EE".into(),
            floor_ecpm: 2_500_000,
        },
        "prev-value",
        spec.clone(),
    ))
    .expect("intent");
    l.append(LedgerRecord::experiment_created(
        NOW,
        PlatformId::AppLovinMax,
        "floors-ee",
        "platform-exp-7",
    ))
    .expect("created");
    let mut index = LedgerIndex::build(&l);
    let recovery = crate::settle::reconcile_experiment_creates(
        &mut l,
        &mut index,
        &SingleOpsSource(ops.clone()),
        NOW + 1,
    )
    .await;
    assert!(recovery.failures.is_empty(), "{:?}", recovery.failures);
    assert_eq!(recovery.recovered, vec!["floors-ee".to_string()]);
    assert!(
        !ops.call_log()
            .iter()
            .any(|entry| entry.starts_with("start:")),
        "a confirmed create is never replayed: {:?}",
        ops.call_log()
    );
    assert!(index.any_start_with_name("floors-ee"));
    assert!(l
        .records()
        .iter()
        .any(|record| matches!(record, LedgerRecord::ExperimentStarted { .. })));
}

#[tokio::test]
async fn an_abandoned_intent_is_never_replayed() {
    let ops = Arc::new(crate::ops::RecordingOps::new());
    let mut l = ledger();
    let spec = apex_core::ExperimentSpec {
        name: "floors-ee".into(),
        instance_id: String::new(),
        ad_unit_id: "u1".into(),
        metric: apex_core::ExperimentMetric::Revenue,
        variants: 2,
        change_description: "raise EE".into(),
        test_group_allocation: 10,
    };
    l.append(LedgerRecord::experiment_create_intent(
        NOW,
        PlatformId::AppLovinMax,
        "floors-ee",
        "u1",
        "max-floor:u1:EE:2.5",
        ChangeKind::CountryFloorSet {
            country: "EE".into(),
            floor_ecpm: 2_500_000,
        },
        "prev-value",
        spec,
    ))
    .expect("intent");
    l.append(LedgerRecord::experiment_create_abandoned(
        NOW,
        PlatformId::AppLovinMax,
        "floors-ee",
        "the platform refused the split terminally",
    ))
    .expect("abandoned");
    let mut index = LedgerIndex::build(&l);
    let recovery = crate::settle::reconcile_experiment_creates(
        &mut l,
        &mut index,
        &SingleOpsSource(ops.clone()),
        NOW + 1,
    )
    .await;
    assert!(recovery.recovered.is_empty());
    assert!(recovery.failures.is_empty());
    assert!(index.pending_creates().is_empty());
    assert!(
        ops.call_log().is_empty(),
        "an abandoned intent costs zero platform calls: {:?}",
        ops.call_log()
    );
}

#[tokio::test]
async fn an_unresolved_intent_adopts_the_active_experiment_instead_of_replaying() {
    // NN: AppLovin creates an experiment only while the ad unit has none
    // active. A crash between the platform's acceptance and the ledger's
    // bookkeeping leaves the live experiment running with no record, and a
    // replayed create is refused on every pass after it. The recovery checks
    // for an active experiment first and adopts it.
    let ops = Arc::new(crate::ops::RecordingOps::new());
    ops.put_active_experiment("u1", "u1");
    let mut l = ledger();
    let spec = apex_core::ExperimentSpec {
        name: "floors-ee".into(),
        instance_id: String::new(),
        ad_unit_id: "u1".into(),
        metric: apex_core::ExperimentMetric::Revenue,
        variants: 2,
        change_description: "raise EE".into(),
        test_group_allocation: 10,
    };
    l.append(LedgerRecord::experiment_create_intent(
        NOW,
        PlatformId::AppLovinMax,
        "floors-ee",
        "u1",
        "max-floor:u1:EE:2.5",
        ChangeKind::CountryFloorSet {
            country: "EE".into(),
            floor_ecpm: 2_500_000,
        },
        "prev-value",
        spec,
    ))
    .expect("intent");
    let mut index = LedgerIndex::build(&l);
    assert_eq!(index.pending_creates().len(), 1);
    let recovery = crate::settle::reconcile_experiment_creates(
        &mut l,
        &mut index,
        &SingleOpsSource(ops.clone()),
        NOW + 1,
    )
    .await;
    assert!(recovery.failures.is_empty(), "{:?}", recovery.failures);
    assert_eq!(recovery.recovered, vec!["floors-ee".to_string()]);
    assert!(
        !ops.call_log()
            .iter()
            .any(|entry| entry.starts_with("start:")),
        "the live experiment is adopted, never started again: {:?}",
        ops.call_log()
    );
    assert!(index.pending_creates().is_empty());
    assert!(index.is_created("floors-ee"));
    assert_eq!(
        index.created_external_id("floors-ee"),
        Some(&"u1".to_string()),
        "the adopted experiment's platform id is the one the create would have returned"
    );
    assert!(index.any_start_with_name("floors-ee"));
    // The adoption lands the same durable records the replay would have.
    let types: Vec<String> = l
        .records()
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::ExperimentCreated {
                experiment_id,
                external_id,
                ..
            } => Some(format!("created:{experiment_id}:{external_id}")),
            LedgerRecord::ChangeApplied {
                idempotency_key, ..
            } => Some(format!("change:{idempotency_key}")),
            LedgerRecord::ExperimentStarted { experiment_id, .. } => {
                Some(format!("started:{experiment_id}"))
            }
            _ => None,
        })
        .collect();
    assert!(types.contains(&"created:floors-ee:u1".to_string()));
    assert!(types.contains(&"change:max-floor:u1:EE:2.5".to_string()));
    assert!(types.contains(&"started:u1".to_string()));
}

#[tokio::test]
async fn test_arm_start_failure_touches_nothing() {
    // The MAX path carries the change in the experiment's test arm: a failed
    // start never mutates the live unit, so there is nothing to roll back.
    let ops = crate::ops::RecordingOps::new();
    ops.set_change_in_test_arm(true);
    ops.script_start(vec![Err(PlatformError::Http {
        platform: PlatformId::AppLovinMax,
        operation: "createExperiment".into(),
        status: 503,
    })]);
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("cycle completes");
    assert_eq!(
        outcome.outcomes,
        vec![ActionOutcome::RolledBack {
            change_seq: 0,
            reason: "experiment_start_failed".into()
        }]
    );
    let log = ops.call_log();
    assert!(
        !log.iter().any(|entry| entry.starts_with("apply:")),
        "the test-arm path never applies to the live unit"
    );
    assert!(
        !log.iter().any(|entry| entry.starts_with("rollback:")),
        "nothing was mutated, so nothing is rolled back"
    );
    assert!(
        !l.records()
            .iter()
            .any(|r| matches!(r, LedgerRecord::ChangeApplied { .. })),
        "no change record when the experiment never started"
    );
    assert!(
        l.records()
            .iter()
            .any(|r| matches!(r, LedgerRecord::LiftRecord { .. })),
        "the signed pre-registration still stands"
    );
}

#[tokio::test]
async fn rollback_failure_surfaces_loudly() {
    let ops = crate::ops::RecordingOps::new();
    ops.script_start(vec![Err(PlatformError::Http {
        platform: PlatformId::Admob,
        operation: "createExperiment".into(),
        status: 503,
    })]);
    ops.script_rollback(vec![Err(PlatformError::Network {
        platform: PlatformId::Admob,
        operation: "rollback".into(),
        detail: "reset".into(),
    })]);
    let mut l = ledger();
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(ecpm_action())],
        NOW,
    )
    .await
    .expect_err("loud failure");
    assert!(matches!(err, PlanError::RollbackFailed { .. }));
    assert_eq!(
        l.len(),
        3,
        "pre-registration + change + create intent, but no false rollback record"
    );
    assert!(
        !l.records()
            .iter()
            .any(|r| matches!(r, LedgerRecord::RollbackPerformed { .. })),
        "no false rollback record when the platform rollback failed"
    );
}

#[tokio::test]
async fn rerun_skips_already_applied_actions() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let first = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("first run");
    assert_eq!(first.applied, 1);
    let second = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW + 1_000,
    )
    .await
    .expect("rerun");
    assert_eq!(
        second.outcomes,
        vec![ActionOutcome::Skipped {
            reason: "already_applied".into()
        }]
    );
    assert_eq!(second.applied, 0);
    let applies = ops
        .call_log()
        .iter()
        .filter(|entry| entry.starts_with("apply:"))
        .count();
    assert_eq!(applies, 1, "rerun must not re-apply");
}

#[tokio::test]
async fn rerun_with_a_changed_floor_value_applies_again() {
    // The repricing falsifier: an earlier value-less key would mark the second
    // proposal as already applied and silently skip the newer floor.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let first = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("first run");
    assert_eq!(first.applied, 1);

    let mut changed = floor_action();
    if let Action::SetCountryFloor { floor_ecpm, .. } = &mut changed {
        *floor_ecpm = 3_000_000;
    }
    let second = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(changed)],
        NOW + 1_000,
    )
    .await
    .expect("second run");
    assert_eq!(
        second.applied, 1,
        "a changed floor value must apply, not skip"
    );

    let keys: Vec<&String> = l
        .records()
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::ChangeApplied {
                idempotency_key, ..
            } => Some(idempotency_key),
            _ => None,
        })
        .collect();
    assert_eq!(keys, vec!["max-floor:u1:EE:2.5", "max-floor:u1:EE:3"]);
}

#[tokio::test]
async fn mixed_rerun_applies_only_the_new_action() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("first run");
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action()), proposal(ecpm_action())],
        NOW + 1_000,
    )
    .await
    .expect("mixed rerun");
    assert_eq!(outcome.applied, 1);
    assert_eq!(outcome.skipped, 1);
    assert!(matches!(outcome.outcomes[0], ActionOutcome::Skipped { .. }));
    assert!(matches!(outcome.outcomes[1], ActionOutcome::Applied { .. }));
}

#[tokio::test]
async fn empty_proposals_produce_an_empty_cycle() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![],
        NOW,
    )
    .await
    .expect("empty");
    assert_eq!(
        outcome,
        crate::model::CycleOutcome {
            outcomes: vec![],
            applied: 0,
            skipped: 0,
            awaiting_approval: 0,
            awaiting_verification: vec![],
            provider_write_failures: 0,
        }
    );
}

#[tokio::test]
async fn cycles_are_deterministic_for_identical_inputs() {
    let ops_a = crate::ops::RecordingOps::new();
    let ops_b = crate::ops::RecordingOps::new();
    let mut la = ledger();
    let mut lb = ledger();
    let a = run_cycle(
        &ops_a,
        &mut la,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action()), proposal(ecpm_action())],
        NOW,
    )
    .await
    .expect("cycle");
    let b = run_cycle(
        &ops_b,
        &mut lb,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action()), proposal(ecpm_action())],
        NOW,
    )
    .await
    .expect("cycle");
    assert_eq!(a, b);
}

// ---------- settle ----------

fn stopped_experiment() -> ExperimentResult {
    ExperimentResult {
        experiment_id: "exp-1".into(),
        state: ExperimentState::Stopped,
        results: vec![],
        lift_percent_bp: None,
        confidence: None,
        winner_variant_id: None,
    }
}

#[tokio::test]
async fn settle_refuses_an_experiment_that_already_ended() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", stopped_experiment());
    let err = settle(&ops, &mut l, "exp-1", seq, NOW)
        .await
        .expect_err("already ended");
    assert!(matches!(err, PlanError::PolicyViolation { .. }));
}

#[tokio::test]
async fn settle_refuses_a_change_sequence_that_is_not_in_the_ledger() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    let err = settle(&ops, &mut l, "exp-1", 999, NOW)
        .await
        .expect_err("unknown change");
    assert!(matches!(err, PlanError::NoBaseline { .. }));
}

#[tokio::test]
async fn settle_refuses_a_non_change_sequence() {
    // Sequence 0 is the signed pre-registration, not a change.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    let err = settle(&ops, &mut l, "exp-1", 0, NOW)
        .await
        .expect_err("not a change");
    assert!(matches!(err, PlanError::NoBaseline { .. }));
}

#[tokio::test]
async fn settle_keep_promotes_and_records_the_signed_lift() {
    // A constant +10% revenue-per-user ratio across the window: point lift
    // 0.10 with zero spread, so the 90% lower bound equals the point estimate
    // and the bootstrap cross-check agrees exactly.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));

    let outcome = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    match outcome {
        SettleOutcome::Kept {
            experiment_id,
            lift,
            lambda,
            result_seq,
            ..
        } => {
            assert_eq!(experiment_id, "exp-1");
            assert!((lift - 0.1).abs() < 1e-9, "lift {lift}");
            assert!((lambda - 0.1).abs() < 1e-9, "lambda {lambda}");
            assert_eq!(result_seq, 5, "the signed result precedes the observation");
        }
        other => panic!("expected Kept, got {other:?}"),
    }
    let log = ops.call_log();
    assert_eq!(
        log.iter().filter(|e| e.starts_with("promote:")).count(),
        1,
        "a kept change is promoted to all users"
    );
    assert_eq!(
        log.iter().filter(|e| e.starts_with("deprecate:")).count(),
        0
    );
    assert!(matches!(l.get(5), Some(LedgerRecord::LiftRecord { .. })));
    assert!(l
        .records()
        .iter()
        .any(|record| matches!(record, LedgerRecord::ExperimentObserved { .. })));
    assert!(
        !l.records()
            .iter()
            .any(|r| matches!(r, LedgerRecord::BillingProof { .. })),
        "settlement never writes a billing proof"
    );
}

#[tokio::test]
async fn settle_below_the_point_floor_rolls_back_and_deprecates() {
    // +0.75% point lift is positive but under the 1% keep floor.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_030_000));

    let outcome = settle(&ops, &mut l, "exp-1", seq, NOW)
        .await
        .expect("settle");
    let SettleOutcome::RolledBack { rollback_seq, .. } = outcome else {
        panic!("expected RolledBack, got {outcome:?}");
    };
    let log = ops.call_log();
    assert_eq!(
        log.iter().filter(|e| e.starts_with("deprecate:")).count(),
        1,
        "a rollback deprecates the experiment"
    );
    assert_eq!(log.iter().filter(|e| e.starts_with("promote:")).count(), 0);
    assert!(matches!(
        l.get(rollback_seq as usize),
        Some(LedgerRecord::RollbackPerformed { .. })
    ));
    assert!(
        !l.records().iter().any(|r| matches!(
            r,
            LedgerRecord::BillingProof { .. } | LedgerRecord::ExperimentObserved { .. }
        )),
        "a rolled-back change is never observed and never billed"
    );
}

#[tokio::test]
async fn settle_zero_lift_rolls_back_without_billing() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_000_000));

    let outcome = settle(&ops, &mut l, "exp-1", seq, NOW)
        .await
        .expect("settle");
    assert!(matches!(outcome, SettleOutcome::RolledBack { .. }));
    assert!(!l
        .records()
        .iter()
        .any(|r| matches!(r, LedgerRecord::BillingProof { .. })));
}

#[tokio::test]
async fn settle_negative_lift_rolls_back() {
    // -10% revenue per user trips the fast-harm guardrail.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 3_600_000));

    let outcome = settle(&ops, &mut l, "exp-1", seq, NOW)
        .await
        .expect("settle");
    match outcome {
        SettleOutcome::RolledBack { lift, .. } => {
            assert!((lift - (-0.1)).abs() < 1e-9, "lift {lift}");
        }
        other => panic!("expected RolledBack, got {other:?}"),
    }
}

#[tokio::test]
async fn settle_voids_on_a_platform_incident_and_deprecates() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", incident_days());

    let outcome = settle(&ops, &mut l, "exp-1", seq, NOW)
        .await
        .expect("settle");
    assert!(
        matches!(outcome, SettleOutcome::Voided { .. }),
        "got {outcome:?}"
    );
    let log = ops.call_log();
    assert_eq!(
        log.iter().filter(|e| e.starts_with("deprecate:")).count(),
        1,
        "a void deprecates the experiment"
    );
    assert_eq!(
        log.iter().filter(|e| e.starts_with("rollback:")).count(),
        0,
        "the test-arm path never touches the live unit"
    );
    // A measured Void now carries a signed terminal Result, followed by the
    // observation that completes the side effect.
    assert!(
        l.records().iter().any(|record| {
            matches!(
                record,
                LedgerRecord::LiftRecord { payload, .. }
                    if payload.contains("\"decision\":\"void\"")
            )
        }),
        "a measured void records its signed result"
    );
    assert!(
        l.records()
            .iter()
            .any(|record| matches!(record, LedgerRecord::ExperimentObserved { .. })),
        "the void completion marker is durable"
    );
}

#[tokio::test]
async fn settle_rejects_a_ledger_platform_mismatch() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = l
        .append(LedgerRecord::change_applied(
            NOW,
            PlatformId::LevelPlay, // action derived below is an AdMob ecpm change
            "u2",
            ChangeKind::EcpmSet {
                mediation_group_id: "g1".into(),
                ad_unit_id: "u2".into(),
                ecpm: 1_750_000,
            },
            "admob-ecpm:g1:u2:1.75",
            "{\"cpm\":1.0}",
        ))
        .expect("seed")
        .seq();
    ops.put_experiment("exp-1", running_experiment());
    let err = settle(&ops, &mut l, "exp-1", seq, NOW)
        .await
        .expect_err("platform mismatch");
    assert!(matches!(err, PlanError::Validation(_)));
}

#[tokio::test]
async fn settle_unknown_experiment_is_an_apply_failure() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    let err = settle(&ops, &mut l, "nope", seq, NOW)
        .await
        .expect_err("unknown experiment");
    assert!(matches!(err, PlanError::ApplyFailed { .. }));
}

#[tokio::test]
async fn a_manual_promotion_is_pending_until_confirmed_then_kept_and_billed() {
    use crate::ops::PromotionStatus;
    let ops = crate::ops::RecordingOps::new();
    ops.set_manual_promotion(PromotionStatus::Pending);
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    let lifts_before = l
        .records()
        .iter()
        .filter(|record| matches!(record, LedgerRecord::LiftRecord { .. }))
        .count();

    let first = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    assert!(
        matches!(first, SettleOutcome::PromotionPending { .. }),
        "{first:?}"
    );
    assert!(
        l.records()
            .iter()
            .any(|record| matches!(record, LedgerRecord::PromotionPending { .. })),
        "the pending keep is in the signed ledger"
    );
    assert_eq!(
        l.records()
            .iter()
            .filter(|record| matches!(record, LedgerRecord::LiftRecord { .. }))
            .count(),
        lifts_before,
        "no signed result (and no fee) before the promotion"
    );
    assert_eq!(
        ops.call_log()
            .iter()
            .filter(|e| e.starts_with("promote:"))
            .count(),
        0,
        "the manual adapter is never promote()d"
    );

    // A second pass while still pending is a wait, not a new pending record
    // and not a notification.
    let waiting = settle(&ops, &mut l, "exp-1", seq, NOW + 2)
        .await
        .expect("settle");
    assert!(
        matches!(waiting, SettleOutcome::Pending { .. }),
        "{waiting:?}"
    );

    ops.set_promotion_status(PromotionStatus::Confirmed);
    let confirmed = settle(&ops, &mut l, "exp-1", seq, NOW + 3)
        .await
        .expect("settle");
    assert!(
        matches!(confirmed, SettleOutcome::Kept { .. }),
        "{confirmed:?}"
    );
    assert!(
        l.records()
            .iter()
            .filter(|record| matches!(record, LedgerRecord::LiftRecord { .. }))
            .count()
            > lifts_before,
        "the confirmed promotion records the signed result and fee basis"
    );
    let replay = settle(&ops, &mut l, "exp-1", seq, NOW + 4)
        .await
        .expect_err("already settled");
    assert!(
        matches!(replay, PlanError::Validation(ref why) if why.contains("already settled")),
        "{replay:?}"
    );
    assert_eq!(
        ops.call_log()
            .iter()
            .filter(|e| e.starts_with("promote:"))
            .count(),
        0
    );
}

#[tokio::test]
async fn a_manual_test_promoted_before_the_window_ends_is_still_kept() {
    use crate::ops::PromotionStatus;
    let ops = crate::ops::RecordingOps::new();
    ops.set_manual_promotion(PromotionStatus::Confirmed);
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    // The dashboard test is gone (the publisher promoted it early); the
    // adapter reports Stopped with the live price already promoted.
    ops.put_experiment("exp-1", stopped_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    let outcome = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    assert!(matches!(outcome, SettleOutcome::Kept { .. }), "{outcome:?}");
    assert_eq!(
        ops.call_log()
            .iter()
            .filter(|e| e.starts_with("promote:"))
            .count(),
        0
    );
}

#[tokio::test]
async fn a_manual_promotion_the_publisher_declines_rolls_back_without_a_fee() {
    use crate::ops::PromotionStatus;
    let ops = crate::ops::RecordingOps::new();
    ops.set_manual_promotion(PromotionStatus::Declined);
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    let lifts_before = l
        .records()
        .iter()
        .filter(|record| matches!(record, LedgerRecord::LiftRecord { .. }))
        .count();
    let outcome = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    assert!(
        matches!(outcome, SettleOutcome::RolledBack { .. }),
        "{outcome:?}"
    );
    assert!(l
        .records()
        .iter()
        .any(|record| matches!(record, LedgerRecord::RollbackPerformed { .. })));
    assert_eq!(
        l.records()
            .iter()
            .filter(|record| matches!(record, LedgerRecord::LiftRecord { .. }))
            .count(),
        lifts_before + 1,
        "a declined promotion records the signed rollback proof"
    );
}

#[tokio::test]
async fn a_manual_promotion_past_its_deadline_is_rolled_back() {
    use crate::ops::PromotionStatus;
    let ops = crate::ops::RecordingOps::new();
    ops.set_manual_promotion(PromotionStatus::Pending);
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    let first = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    assert!(matches!(first, SettleOutcome::PromotionPending { .. }));
    let late = settle(&ops, &mut l, "exp-1", seq, NOW + 20 * 86_400_000)
        .await
        .expect("settle");
    assert!(matches!(late, SettleOutcome::RolledBack { .. }), "{late:?}");
    assert_eq!(
        ops.call_log()
            .iter()
            .filter(|e| e.starts_with("deprecate:"))
            .count(),
        1,
        "the expired split is ended"
    );
}

/// Live LevelPlay promotion, end to end: a 17-day-old signed ledger settles
/// to PromotionPending, the dashboard promotion is simulated with the real
/// client (delete the variant group, promote the live price), and the next
/// settle keeps the change and records the signed result. Requires the
/// provider simulator:
/// `LP_SIM_BASE=http://127.0.0.1:18199 LP_ENTRY_ID=1001 LP_GROUP_B=1002`.
#[tokio::test]
async fn live_levelplay_promotion_pending_confirms_and_keeps() {
    use crate::live_levelplay::LevelPlayPlatformOps;
    use crate::live_max::SystemWall;
    use crate::ops::PromotionStatus;
    use apex_core::PlatformId;
    let Ok(base) = std::env::var("LP_SIM_BASE") else {
        eprintln!("skipping live LevelPlay promotion: set LP_SIM_BASE");
        return;
    };
    let entry = std::env::var("LP_ENTRY_ID").unwrap_or_else(|_| "1001".to_string());
    let group_b = std::env::var("LP_GROUP_B").unwrap_or_else(|_| "1002".to_string());
    let app_key = std::env::var("LP_APP_KEY").unwrap_or_else(|_| "lp-app-key".to_string());
    let secret = std::env::var("LP_SECRET").unwrap_or_else(|_| "lp-secret".to_string());
    let refresh = std::env::var("LP_REFRESH").unwrap_or_else(|_| "lp-refresh".to_string());
    let transport = std::sync::Arc::new(
        apex_connectors::transport::ReqwestTransport::new().expect("transport"),
    );
    let ops = LevelPlayPlatformOps::new(
        &secret,
        &refresh,
        &app_key,
        &base,
        transport,
        std::sync::Arc::new(SystemWall),
        ["updateEntryPrice".to_string()],
    )
    .expect("adapter");
    assert!(ops.promotion_requires_manual_action());

    let recorder = crate::ops::RecordingOps::new();
    recorder.set_platform(PlatformId::LevelPlay);
    recorder.set_change_in_test_arm(true);
    recorder.script_start(vec![Ok(format!("{group_b}::aged-promotion"))]);
    recorder.put_current_settings(
        &entry,
        &format!(r#"{{"priceEcpm":2000000,"{entry}":2000000}}"#),
    );
    let mut ledger = ledger();
    // The simulator reports a 50/50 dashboard split; the pre-registration
    // must pin the same share or the split guardrail correctly voids it.
    let policy = AutopilotPolicy {
        test_share: 0.5,
        ..settle_policy()
    };
    // The simulator's reports are dated in the real calendar; age the
    // experiment relative to now so the registered window lines up with them.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(NOW);
    let started_at = now_ms - 17 * 86_400_000;
    let outcome = crate::plan::run_cycle(
        &recorder,
        &mut ledger,
        &policy,
        &[],
        vec![proposal(Action::SetWaterfallPrice {
            ad_unit_id: entry.clone(),
            entry_id: entry.clone(),
            price_ecpm: 2_400_000,
        })],
        started_at,
    )
    .await
    .expect("aged cycle");
    let change_seq = match outcome.outcomes.first() {
        Some(ActionOutcome::Applied { change_seq, .. }) => *change_seq,
        other => panic!("expected Applied, got {other:?}"),
    };

    // The adapter must see the live variant group before the first settle.
    let live_state = ops
        .experiment(&format!("{group_b}::aged-promotion"))
        .await
        .expect("experiment state");
    if !matches!(live_state.state, apex_core::ExperimentState::Running) {
        eprintln!(
            "skipping live promotion: simulator {group_b} is not in a fresh A/B state (run a fresh simulator)"
        );
        return;
    }
    let first = settle_experiment(
        &ops,
        &mut ledger,
        &format!("{group_b}::aged-promotion"),
        change_seq,
        FillFormat::Rewarded,
        &EditCheck::VerifiedSame,
        now_ms,
    )
    .await
    .expect("first settle");
    assert!(
        matches!(first, SettleOutcome::PromotionPending { .. }),
        "{first:?}"
    );

    // The publisher promotes the test in the dashboard: the variant group is
    // gone and the live entry now serves the tested price.
    let client = apex_connectors::levelplay::LevelPlayClient::new(
        &secret,
        &refresh,
        &app_key,
        &base,
        std::sync::Arc::new(
            apex_connectors::transport::ReqwestTransport::new().expect("transport"),
        ),
    )
    .expect("client");
    client.delete_group(&group_b).await.expect("end variant");
    client
        .update_entry_price(&entry, &entry, 2_400_000)
        .await
        .expect("promote price");
    assert_eq!(
        ops.promotion_status(
            &format!("{group_b}::aged-promotion"),
            &apex_proof::ChangeKind::WaterfallEntryPriceSet {
                ad_unit_id: entry.clone(),
                entry_id: entry.clone(),
                price_ecpm: 2_400_000,
            },
        )
        .await
        .expect("status"),
        PromotionStatus::Confirmed
    );

    let confirmed = settle_experiment(
        &ops,
        &mut ledger,
        &format!("{group_b}::aged-promotion"),
        change_seq,
        FillFormat::Rewarded,
        &EditCheck::VerifiedSame,
        now_ms,
    )
    .await
    .expect("confirmed settle");
    assert!(
        matches!(confirmed, SettleOutcome::Kept { .. }),
        "{confirmed:?}"
    );
    assert!(
        ledger
            .records()
            .iter()
            .any(|record| matches!(record, LedgerRecord::LiftRecord { .. })),
        "the confirmed promotion records the signed result"
    );
}

#[tokio::test]
async fn settle_missing_reporting_is_an_apply_failure_not_a_keep() {
    // Fail closed: an adapter that cannot read reporting must not settle.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    let err = settle(&ops, &mut l, "exp-1", seq, NOW)
        .await
        .expect_err("no reporting");
    assert!(matches!(err, PlanError::ApplyFailed { .. }));
    assert!(
        !l.records().iter().any(|r| matches!(
            r,
            LedgerRecord::ExperimentObserved { .. } | LedgerRecord::RollbackPerformed { .. }
        )),
        "no settlement records when the window could not be built"
    );
}

#[tokio::test]
async fn an_unmeasured_crash_after_the_stop_recovers_to_a_signed_result() {
    let ops = RecordingOps::new();
    let mut l = ledger();
    let seq = start_admob_experiment(&ops, &mut l, NOW).await;
    ops.put_conclusion("exp-1", conclusion("TREATMENT", true));
    let pending = l
        .append(LedgerRecord::promotion_pending(
            NOW,
            PlatformId::Admob,
            "exp-1",
            "admob-unit",
            seq,
            NOW + 86_400_000,
            "concluding an unmeasured experiment: signed terminal result pending",
        ))
        .expect("pending");
    assert!(pending.seq() > 0);
    let observed = l
        .append(LedgerRecord::experiment_observed(
            NOW + 1,
            PlatformId::Admob,
            "exp-1",
            apex_core::ExperimentResult {
                experiment_id: "exp-1".to_string(),
                state: apex_core::ExperimentState::Stopped,
                results: Vec::new(),
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: Some("TREATMENT".to_string()),
            },
        ))
        .expect("observed");
    assert!(observed.seq() > pending.seq());

    let outcome = settle(&ops, &mut l, "exp-1", seq, ADMOB_WINDOW_END)
        .await
        .expect("recovered settle");
    assert!(matches!(outcome, SettleOutcome::Kept { .. }), "{outcome:?}");
    assert!(l
        .records()
        .iter()
        .any(|record| matches!(record, LedgerRecord::LiftRecord { .. })));
}

#[tokio::test]
async fn admob_treatment_leader_keeps_with_zero_lift_and_no_fee() {
    let ops = RecordingOps::new();
    let mut l = ledger();
    let seq = start_admob_experiment(&ops, &mut l, NOW).await;
    ops.put_conclusion("exp-1", conclusion("TREATMENT", true));

    let outcome = settle(&ops, &mut l, "exp-1", seq, ADMOB_WINDOW_END)
        .await
        .expect("settle");
    match outcome {
        SettleOutcome::Kept {
            lift,
            lambda,
            unmeasured,
            variant_leader,
            daily_log_ratios,
            ..
        } => {
            assert_eq!(lift, 0.0, "a verdict is never measured lift");
            assert_eq!(lambda, 0.0, "no billable lift");
            assert!(unmeasured);
            assert_eq!(variant_leader.as_deref(), Some("TREATMENT"));
            assert!(daily_log_ratios.is_empty());
        }
        other => panic!("expected an unmeasured keep, got {other:?}"),
    }

    let log = ops.call_log();
    assert_eq!(
        log.iter()
            .filter(|entry| entry.starts_with("conclude:"))
            .count(),
        1,
        "the documented end runs once"
    );
    assert!(
        !log.iter().any(|entry| entry.starts_with("days:")),
        "AdMob is never asked for per-variant reporting"
    );
    assert_eq!(
        log.iter()
            .filter(|entry| entry.starts_with("promote:"))
            .count(),
        0,
        "AdMob has no API promotion; the stop response is the conclusion"
    );

    let payloads: Vec<&str> = l
        .records()
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::LiftRecord { payload, .. } => Some(payload.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        payloads
            .iter()
            .any(|payload| payload.contains("\"decision\":\"keep\"")
                && payload.contains("\"lambda\":0.0")),
        "the zero-lift keep is recorded durably"
    );
    assert!(
        payloads
            .iter()
            .any(|payload| payload.contains("billingChange")
                && payload.contains(apex_lift::billing::ADMOB_NO_MEASUREMENT_REASON)),
        "the fee-ineligible conclusion is durable"
    );
    assert!(
        !payloads
            .iter()
            .any(|payload| payload.contains("monthlyBilling")),
        "no fee record is written"
    );
    assert!(!l
        .records()
        .iter()
        .any(|record| matches!(record, LedgerRecord::BillingProof { .. })));
}

#[tokio::test]
async fn admob_control_or_insufficient_data_rolls_back_and_stops() {
    for leader in ["CONTROL", "INSUFFICIENT_DATA"] {
        let ops = RecordingOps::new();
        let mut l = ledger();
        let seq = start_admob_experiment(&ops, &mut l, NOW).await;
        ops.put_conclusion("exp-1", conclusion(leader, false));

        let outcome = settle(&ops, &mut l, "exp-1", seq, ADMOB_WINDOW_END)
            .await
            .expect("settle");
        match outcome {
            SettleOutcome::RolledBack {
                lift,
                unmeasured,
                variant_leader,
                rollback_seq,
                ..
            } => {
                assert_eq!(lift, 0.0, "{leader}");
                assert!(unmeasured, "{leader}");
                assert_eq!(variant_leader.as_deref(), Some(leader), "{leader}");
                assert!(
                    matches!(
                        l.get(rollback_seq as usize),
                        Some(LedgerRecord::RollbackPerformed { .. })
                    ),
                    "{leader}: the original lines are the recorded outcome"
                );
            }
            other => panic!("{leader}: expected RolledBack, got {other:?}"),
        }
        let log = ops.call_log();
        assert_eq!(
            log.iter()
                .filter(|entry| entry.starts_with("conclude:"))
                .count(),
            1,
            "{leader}"
        );
        assert_eq!(
            log.iter()
                .filter(|entry| entry.starts_with("promote:"))
                .count(),
            0,
            "{leader}"
        );
    }
}

#[tokio::test]
async fn a_not_yet_ended_admob_test_stays_running_and_is_not_stopped() {
    let ops = RecordingOps::new();
    let mut l = ledger();
    let seq = start_admob_experiment(&ops, &mut l, NOW).await;

    for at in [NOW, ADMOB_WINDOW_END - DAY] {
        let outcome = settle(&ops, &mut l, "exp-1", seq, at)
            .await
            .expect("settle");
        assert!(
            matches!(outcome, SettleOutcome::Pending { .. }),
            "the registered window has not ended at {at}: {outcome:?}"
        );
    }
    let log = ops.call_log();
    assert_eq!(
        log.iter()
            .filter(|entry| entry.starts_with("conclude:"))
            .count(),
        0,
        "a running test is retried honestly, never ended early"
    );
    assert!(
        !l.records()
            .iter()
            .any(|record| matches!(record, LedgerRecord::RollbackPerformed { .. })),
        "no rollback record before the end"
    );
}

#[tokio::test]
async fn an_ended_admob_test_concludes_once_and_replay_is_refused() {
    let ops = RecordingOps::new();
    let mut l = ledger();
    let seq = start_admob_experiment(&ops, &mut l, NOW).await;
    ops.put_conclusion("exp-1", conclusion("TREATMENT", true));

    let first = settle(&ops, &mut l, "exp-1", seq, ADMOB_WINDOW_END)
        .await
        .expect("first settle");
    assert!(matches!(
        first,
        SettleOutcome::Kept {
            unmeasured: true,
            ..
        }
    ));
    let len_after_first = l.len();
    let second = settle(&ops, &mut l, "exp-1", seq, ADMOB_WINDOW_END + 1)
        .await
        .expect_err("replay is refused before another stop");
    assert!(matches!(second, PlanError::Validation(_)), "got {second}");
    assert_eq!(l.len(), len_after_first, "no duplicate conclusion records");
    assert_eq!(
        ops.call_log()
            .iter()
            .filter(|entry| entry.starts_with("conclude:"))
            .count(),
        1,
        "the platform end is not replayed"
    );
}

// ---------------- verifier-round 2 regressions ----------------

#[tokio::test]
async fn repeated_settlement_is_refused_before_any_side_effect() {
    // Double-settlement falsifier: a repeat must not append a second
    // ExperimentObserved or a second signed lift record (double billing).
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    let first = settle(&ops, &mut l, "exp-1", seq, NOW)
        .await
        .expect("first settle keeps");
    assert!(matches!(first, SettleOutcome::Kept { .. }));
    let len_after_first = l.len();

    let second = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect_err("second settle must be refused");
    assert!(matches!(second, PlanError::Validation(_)), "got {second}");
    assert_eq!(l.len(), len_after_first, "no extra observed/result records");
    let observed = l
        .records()
        .iter()
        .filter(|r| matches!(r, LedgerRecord::ExperimentObserved { .. }))
        .count();
    assert_eq!(
        observed, 1,
        "exactly one observation per settled experiment"
    );
    let rollbacks = ops
        .call_log()
        .iter()
        .filter(|e| e.starts_with("rollback:"))
        .count();
    assert_eq!(rollbacks, 0, "refusal costs zero platform calls");
}

#[test]
fn create_entry_experiments_are_distinct_per_instance_name() {
    let base = Action::CreateWaterfallEntry {
        ad_unit_id: "u3".into(),
        network: "mintegral".into(),
        instance_name: "MTG_1".into(),
        price_ecpm: 2_000_000,
    };
    let other = Action::CreateWaterfallEntry {
        ad_unit_id: "u3".into(),
        network: "mintegral".into(),
        instance_name: "MTG_2".into(),
        price_ecpm: 2_000_000,
    };
    assert_ne!(experiment_name(&base), experiment_name(&other));
    assert_eq!(
        experiment_name(&base),
        "max-experiment:autopilot-wfc-u3-mintegral-mtg-1-2:u3"
    );
}

#[tokio::test]
async fn action_values_are_bounded_before_any_platform_call() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let wild = Proposal {
        action: Action::SetCountryFloor {
            ad_unit_id: "u1".into(),
            country: "EE".into(),
            floor_ecpm: i64::MAX,
        },
        rationale: "absurd floor".into(),
        expected_effect_bp: 1,
        prev_value: "{\"floor\":0}".into(),
    };
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![wild],
        NOW,
    )
    .await
    .expect_err("absurd value refused");
    assert!(matches!(err, PlanError::Validation(_)), "got {err}");
    assert!(ops.call_log().is_empty());
    assert!(l.is_empty());
}

#[tokio::test]
async fn future_dated_records_do_not_consume_the_daily_cap() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    for i in 0..3 {
        seed_change(&mut l, NOW + 3_600_000 + i, &format!("future-{i}"));
    }
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("future-dated slots do not count");
    assert_eq!(outcome.applied, 1);
}

#[tokio::test]
async fn rolled_back_changes_free_the_cap_slot_and_retry_applies() {
    // Start-failure path: rollback, then the same proposal must apply again
    // (the rolled-back slot is neither a cap slot nor 'already applied').
    let ops = crate::ops::RecordingOps::new();
    ops.script_start(vec![Err(apex_core::PlatformError::Http {
        platform: apex_core::PlatformId::AppLovinMax,
        operation: "createExperiment".into(),
        status: 503,
    })]);
    let mut l = ledger();
    let policy = AutopilotPolicy {
        max_actions_per_day: 1,
        ..AutopilotPolicy::conservative()
    };
    let first = run_cycle(
        &ops,
        &mut l,
        &policy,
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect("first cycle rolls back");
    assert!(matches!(
        first.outcomes[0],
        ActionOutcome::RolledBack { .. }
    ));

    let second = run_cycle(
        &ops,
        &mut l,
        &policy,
        &[],
        vec![proposal(floor_action())],
        NOW + 1,
    )
    .await
    .expect("retry runs");
    assert_eq!(
        second.applied, 1,
        "the rolled-back change must be re-appliable"
    );
}

#[tokio::test]
async fn invalid_country_is_a_validation_error_not_an_apply_failure() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let bad = Proposal {
        prev_value: "{}".into(),
        action: Action::SetCountryFloor {
            ad_unit_id: "u1".into(),
            country: "EST".into(),
            floor_ecpm: 1_000_000,
        },
        rationale: "bad country".into(),
        expected_effect_bp: 100,
    };
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![bad],
        NOW,
    )
    .await
    .expect_err("bad country refused");
    assert!(matches!(err, PlanError::Validation(_)), "got {err}");
    assert!(ops.call_log().is_empty());
}

#[tokio::test]
async fn settle_uses_unique_players_for_the_estimate_and_split() {
    // 300 test and 900 control unique players, each seen on every analysis
    // day (a feed missing a day must not decide). The planned share is 25%;
    // the estimator must fold to the 300/900 unique players and use the HT
    // bound.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    let mut rows = Vec::new();
    let window_days: Vec<String> = (4..=10).map(|day| format!("2026-10-{day:02}")).collect();
    for user in 0..900 {
        for day in &window_days {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.clone(),
                user_id: Some(format!("c{user}")),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 100_000,
            });
        }
    }
    for user in 0..300 {
        for day in &window_days {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.into(),
                user_id: Some(format!("t{user}")),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 110_000,
            });
        }
    }
    // A pre-window user in both arms must not enter the analysis window.
    rows.push(apex_lift::player_store::PlayerDay {
        day: "2026-10-01".into(),
        user_id: Some("pre-window".into()),
        idfv: None,
        idfa: None,
        test: true,
        revenue_micros: 1,
    });
    rows.push(apex_lift::player_store::PlayerDay {
        day: "2026-10-02".into(),
        user_id: Some("pre-window".into()),
        idfv: None,
        idfa: None,
        test: false,
        revenue_micros: 1,
    });
    ops.put_experiment_players(
        "exp-1",
        crate::ops::PlayerEvidence {
            rows,
            pretest: std::collections::BTreeMap::new(),
            measurement_scope: Some("u1".to_string()),
            roster: None,
            eligible: None,
            certified_gain_lower: None,
            certified_delta_lower: None,
            assigned_test_control: None,
        },
    );
    let result = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    // The corrected 97.75% design-aware bound is not positive at this small
    // sample, so the decision rolls back; the +10% lift can only come from the
    // per-player estimate (the daily ratios are strongly negative), proving
    // the registered per-user path decided.
    match result {
        SettleOutcome::RolledBack { lift, reason, .. } => {
            assert!((lift - 0.10).abs() < 1e-9, "player HT lift {lift}");
            assert!(reason.contains("97.75%"), "{reason}");
        }
        other => panic!("expected RolledBack on insufficient information, got {other:?}"),
    }
}

#[tokio::test]
async fn a_registered_player_feed_failure_voids_instead_of_using_daily_ratios() {
    let ops = crate::ops::RecordingOps::new();
    ops.set_supports_player_feed(true);
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    // No experiment_players is scripted: RecordingOps returns a typed error.
    let result = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    match result {
        SettleOutcome::Voided { reason, .. } => {
            assert!(reason.contains("per-player feed"), "{reason}");
        }
        other => panic!("expected Voided, got {other:?}"),
    }
}

#[tokio::test]
async fn settle_uses_the_assigned_roster_for_the_split_guardrail() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    let mut rows = Vec::new();
    for day in [
        "2026-10-04",
        "2026-10-05",
        "2026-10-06",
        "2026-10-07",
        "2026-10-08",
        "2026-10-09",
        "2026-10-10",
    ] {
        for user in 0..300 {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.into(),
                user_id: Some(format!("t{user}")),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 110_000,
            });
        }
        for user in 0..900 {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.into(),
                user_id: Some(format!("c{user}")),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 100_000,
            });
        }
    }
    ops.put_experiment_players(
        "exp-1",
        crate::ops::PlayerEvidence {
            rows,
            pretest: std::collections::BTreeMap::new(),
            measurement_scope: Some("u1".to_string()),
            roster: None,
            eligible: None,
            certified_gain_lower: None,
            certified_delta_lower: None,
            // Observed players are the planned 300/900; the assignment roster
            // says 600/200 (75% test), which must trigger the split guardrail.
            assigned_test_control: Some((600, 200)),
        },
    );
    let result = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    match result {
        SettleOutcome::Voided { reason, .. } => {
            assert!(reason.to_lowercase().contains("split"), "{reason}");
        }
        other => panic!("expected Voided on the assigned-split mismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn priors_rebuild_from_the_signed_ledger_after_a_lost_file() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    // The rebuild maps results through the started experiment's change_seq:
    // settle must be allowed to record the platform start for it.
    ops.script_start(vec![Ok("exp-1".to_string())]);
    let outcome = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    assert!(
        matches!(outcome, SettleOutcome::Kept { .. }),
        "the fixture keeps: {outcome:?}"
    );

    // The signed ledger, not a mutable file, is the model's source of truth:
    // every terminal result with daily evidence is replayed.
    let rebuilt = crate::runner::rebuild_priors_from_ledger(&l);
    assert!(
        rebuilt.total_observations() > 0,
        "the learned result is recovered from signed records"
    );

    // An empty ledger rebuilds to the defaults, never to a fabricated state.
    let empty = ledger();
    assert_eq!(
        crate::runner::rebuild_priors_from_ledger(&empty).total_observations(),
        0
    );

    // A verdict-only record (no daily ratios) never trains the model.
    let mut verdict_only = ledger();
    let payload = serde_json::to_string(&apex_lift::records::LiftRecord::Result {
        experiment_id: "exp-1".into(),
        instance_id: "exp-1@1".into(),
        daily_log_ratios: Vec::new(),
        mean_log: 0.1,
        lower_90: 0.05,
        lower_99: 0.01,
        bootstrap_lower: 0.04,
        decision: "keep".into(),
        lambda: 0.05,
    })
    .expect("serialize");
    verdict_only
        .append(apex_proof::LedgerRecord::lift_record(
            NOW + 2,
            "0000000000000000000000000000000000000000000000000000000000000000",
            &payload,
        ))
        .expect("append");
    assert_eq!(
        crate::runner::rebuild_priors_from_ledger(&verdict_only).total_observations(),
        0
    );
}

#[tokio::test]
async fn settle_voids_when_the_registered_estimator_yields_no_reading() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    let mut rows = Vec::new();
    for day in [
        "2026-10-04",
        "2026-10-05",
        "2026-10-06",
        "2026-10-07",
        "2026-10-08",
        "2026-10-09",
        "2026-10-10",
    ] {
        for user in 0..300 {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.into(),
                user_id: Some(format!("t{user}")),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 110_000,
            });
        }
        for user in 0..300 {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.into(),
                user_id: Some(format!("c{user}")),
                idfv: None,
                idfa: None,
                test: false,
                // Zero control revenue makes the registered log-ratio
                // estimator yield no reading at all.
                revenue_micros: 0,
            });
        }
    }
    ops.put_experiment_players(
        "exp-1",
        crate::ops::PlayerEvidence {
            rows,
            pretest: std::collections::BTreeMap::new(),
            measurement_scope: Some("u1".to_string()),
            roster: None,
            eligible: None,
            certified_gain_lower: None,
            certified_delta_lower: None,
            assigned_test_control: None,
        },
    );
    let result = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    match result {
        SettleOutcome::Voided { reason, .. } => {
            assert!(reason.contains("estimate is unavailable"), "{reason}");
        }
        other => panic!("expected Voided, got {other:?}"),
    }
}

#[tokio::test]
async fn settle_voids_when_a_player_appears_in_both_arms() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    ops.put_experiment_players(
        "exp-1",
        crate::ops::PlayerEvidence {
            rows: vec![
                apex_lift::player_store::PlayerDay {
                    day: "2026-10-05".into(),
                    user_id: Some("shared".into()),
                    idfv: None,
                    idfa: None,
                    test: true,
                    revenue_micros: 100,
                },
                apex_lift::player_store::PlayerDay {
                    day: "2026-10-06".into(),
                    user_id: Some("shared".into()),
                    idfv: None,
                    idfa: None,
                    test: false,
                    revenue_micros: 100,
                },
            ],
            pretest: std::collections::BTreeMap::new(),
            measurement_scope: Some("u1".to_string()),
            roster: None,
            eligible: None,
            certified_gain_lower: None,
            certified_delta_lower: None,
            assigned_test_control: None,
        },
    );
    let result = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    match result {
        SettleOutcome::Voided { reason, .. } => {
            assert!(
                reason.contains("both the control and test arms"),
                "{reason}"
            );
        }
        other => panic!("expected Voided, got {other:?}"),
    }
}

#[tokio::test]
async fn settle_voids_when_the_player_feed_misses_analysis_days() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    // Two clean days of a seven-day analysis window: the feed silently lost
    // days, so the player path may not decide the experiment at all.
    let mut rows = Vec::new();
    for day in ["2026-10-05", "2026-10-06"] {
        for user in 0..300 {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.into(),
                user_id: Some(format!("t{user}")),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 110_000,
            });
        }
        for user in 0..300 {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.into(),
                user_id: Some(format!("c{user}")),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 100_000,
            });
        }
    }
    ops.put_experiment_players(
        "exp-1",
        crate::ops::PlayerEvidence {
            rows,
            pretest: std::collections::BTreeMap::new(),
            measurement_scope: Some("u1".to_string()),
            roster: None,
            eligible: None,
            certified_gain_lower: None,
            certified_delta_lower: None,
            assigned_test_control: None,
        },
    );
    let result = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    match result {
        SettleOutcome::Voided { reason, .. } => {
            assert!(
                reason.contains("only 2 of 7 analysis days"),
                "player evidence covers only 2 of 7 analysis days; a shortened feed cannot decide: {reason}"
            );
        }
        other => panic!("expected Voided, got {other:?}"),
    }
}

#[tokio::test]
async fn settle_voids_when_no_player_rows_touch_the_analysis_window() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    // A non-empty feed whose rows all sit before the analysis window: the
    // shortest possible coverage (zero days) must void, not fall back to the
    // daily estimate the way a fully empty feed does.
    let mut rows = Vec::new();
    for day in ["2026-10-01", "2026-10-02"] {
        for user in 0..300 {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.into(),
                user_id: Some(format!("t{user}")),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 110_000,
            });
        }
    }
    ops.put_experiment_players(
        "exp-1",
        crate::ops::PlayerEvidence {
            rows,
            pretest: std::collections::BTreeMap::new(),
            measurement_scope: Some("u1".to_string()),
            roster: None,
            eligible: None,
            certified_gain_lower: None,
            certified_delta_lower: None,
            assigned_test_control: None,
        },
    );
    let result = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    match result {
        SettleOutcome::Voided { reason, .. } => {
            assert!(
                reason.contains("covers 0 of 7 analysis days"),
                "player evidence covers 0 of 7 analysis days; a shortened feed cannot decide: {reason}"
            );
        }
        other => panic!("expected Voided, got {other:?}"),
    }
}

#[tokio::test]
async fn a_max_frequency_cap_proposal_reaches_the_ledger_and_start() {
    let ops = crate::ops::RecordingOps::new();
    ops.set_change_in_test_arm(true);
    ops.script_start(vec![Ok("exp-cap".into())]);
    let mut l = ledger();
    let outcome = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(Action::SetMaxFrequencyCap {
            ad_unit_id: "u1".into(),
            cap_type: crate::model::MaxCapType::Session,
            countries: vec!["us".into(), "ca".into()],
            session_limit: Some(4),
            day_limit: None,
            minute_frequency: None,
        })],
        NOW,
    )
    .await
    .expect("cycle");
    assert_eq!(outcome.applied, 1, "{outcome:?}");
    assert!(
        l.records().iter().any(|record| matches!(
            record,
            LedgerRecord::ChangeApplied {
                change: ChangeKind::FrequencyCapSet { .. },
                ..
            }
        )),
        "the cap action is recorded with its own change kind"
    );
    assert!(ops
        .call_log()
        .iter()
        .any(|entry| entry.starts_with("start:")));
}

#[tokio::test]
async fn invalid_cap_actions_are_validation_errors_before_any_call() {
    let cases = vec![
        Action::SetMaxFrequencyCap {
            ad_unit_id: "u1".into(),
            cap_type: crate::model::MaxCapType::Session,
            countries: vec!["us".into(), "US".into()],
            session_limit: Some(4),
            day_limit: None,
            minute_frequency: None,
        },
        Action::SetMaxFrequencyCap {
            ad_unit_id: "u1".into(),
            cap_type: crate::model::MaxCapType::Session,
            countries: vec!["us".into()],
            session_limit: None,
            day_limit: None,
            minute_frequency: None,
        },
        Action::SetLevelplayGroupCapping {
            group_id: "g-b".into(),
            session_limit: None,
            interstitial_limit: None,
            day_limit: None,
        },
    ];
    for action in cases {
        let ops = crate::ops::RecordingOps::new();
        let mut l = ledger();
        let error = run_cycle(
            &ops,
            &mut l,
            &AutopilotPolicy::conservative(),
            &[],
            vec![proposal(action.clone())],
            NOW,
        )
        .await
        .expect_err("invalid cap");
        assert!(
            matches!(error, PlanError::Validation(_)),
            "{action:?}: {error:?}"
        );
        assert!(ops.call_log().is_empty(), "{action:?} touched the platform");
    }
}

#[tokio::test]
async fn split_guardrail_uses_unique_players_even_when_days_are_skewed() {
    // The per-day arm counts are 600/200 (75% test), which the summed
    // player-day fallback would void at a registered 25% share. The persisted
    // player ids are 300/900 (25%), so the unique split must win.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = start_experiment(&ops, &mut l, NOW).await;
    ops.put_experiment("exp-1", running_experiment());
    // Test users are three times as many per day, so the revenue is scaled to
    // keep the per-user value at +10% while the summed split is 75% test.
    let mut control: Vec<ArmDay> = (0..WINDOW_PULL_DAYS)
        .map(|index| arm_day(index, 4_000_000))
        .collect();
    let mut test: Vec<ArmDay> = (0..WINDOW_PULL_DAYS)
        .map(|index| arm_day(index, 13_200_000))
        .collect();
    for day in &mut test {
        day.users = Some(600);
        day.second_pull_users = Some(600);
    }
    for day in &mut control {
        day.users = Some(200);
        day.second_pull_users = Some(200);
    }
    ops.put_experiment_days("exp-1", ExperimentDays { control, test });
    let mut rows = Vec::new();
    let window_days: Vec<String> = (4..=10).map(|day| format!("2026-10-{day:02}")).collect();
    for user in 0..900 {
        for day in &window_days {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.into(),
                user_id: Some(format!("c{user}")),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 100_000,
            });
        }
    }
    for user in 0..300 {
        for day in &window_days {
            rows.push(apex_lift::player_store::PlayerDay {
                day: day.clone(),
                user_id: Some(format!("t{user}")),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 110_000,
            });
        }
    }
    ops.put_experiment_players(
        "exp-1",
        crate::ops::PlayerEvidence {
            rows,
            pretest: std::collections::BTreeMap::new(),
            measurement_scope: Some("u1".to_string()),
            roster: None,
            eligible: None,
            certified_gain_lower: None,
            certified_delta_lower: None,
            assigned_test_control: None,
        },
    );
    let result = settle(&ops, &mut l, "exp-1", seq, NOW + 1)
        .await
        .expect("settle");
    match result {
        SettleOutcome::RolledBack {
            lift, guardrails, ..
        } => {
            assert!((lift - 0.10).abs() < 1e-9, "unique-player lift {lift}");
            assert!(
                !guardrails
                    .iter()
                    .any(|guard| format!("{guard:?}").contains("SplitMismatch")),
                "the assigned unique split must not trigger the skew guard: {guardrails:?}"
            );
        }
        other => panic!("expected RolledBack (unique split corrected), got {other:?}"),
    }
}
