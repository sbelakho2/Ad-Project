//! Adversarial tests for the autopilot loop. Fixtures are built through the
//! real audit engine (no hand-rolled Finding literals), the ops double logs
//! every call, and the ledger is the real proof-crate InMemoryLedger.

use apex_audit::{run as run_audit, Severity, Snapshot};
use apex_core::{ExperimentResult, ExperimentState, PlatformError, PlatformId};
use apex_lift::guardrails::FillFormat;
use apex_lift::window::ArmDay;
use apex_proof::{ChangeKind, InMemoryLedger, Keypair, LedgerRecord};
use serde_json::json;

use crate::keys::{experiment_name, idempotency_key, operation_of, platform_of};
use crate::model::{Action, ActionOutcome, AutopilotPolicy, PlanError, Proposal, SettleOutcome};
use crate::ops::{ExperimentDays, PlatformOps};
use crate::{run_cycle, settle_experiment};

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
    let snapshot = snapshot_from(json!({
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
        &[],
        at,
    )
    .await
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
                change_seq: 4,
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
    assert_eq!(l.len(), 3, "pre-registration + change + experiment");
    assert!(matches!(l.get(0), Some(LedgerRecord::LiftRecord { .. })));
    match (l.get(1), l.get(2)) {
        (
            Some(LedgerRecord::ChangeApplied {
                idempotency_key, ..
            }),
            Some(LedgerRecord::ExperimentStarted { experiment_id, .. }),
        ) => {
            assert_eq!(idempotency_key, "admob-ecpm:g1:u2:1.75");
            assert_eq!(experiment_id, "max-experiment:autopilot-ecpm-g1-u2-1-75:u2");
        }
        other => panic!("unexpected ledger shape: {other:?}"),
    }
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
    match l.get(2) {
        Some(LedgerRecord::RollbackPerformed {
            rolled_back_seq,
            reason,
            ..
        }) => {
            assert_eq!(*rolled_back_seq, 1);
            assert_eq!(reason, "experiment_start_failed");
        }
        other => panic!("expected rollback record, got {other:?}"),
    }
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
        2,
        "pre-registration + change, but no false rollback record"
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
            assert_eq!(result_seq, 4);
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
    assert!(matches!(
        l.get(3),
        Some(LedgerRecord::ExperimentObserved { .. })
    ));
    assert!(matches!(l.get(4), Some(LedgerRecord::LiftRecord { .. })));
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
