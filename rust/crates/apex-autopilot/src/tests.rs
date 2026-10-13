//! Adversarial tests for the autopilot loop. Fixtures are built through the
//! real audit engine (no hand-rolled Finding literals), the ops double logs
//! every call, and the ledger is the real proof-crate InMemoryLedger.

use apex_audit::{run as run_audit, Severity, Snapshot};
use apex_core::{
    ExperimentMetric, ExperimentResult, ExperimentSpec, ExperimentState, ExperimentVariantResult,
    PlatformError, PlatformId,
};
use apex_proof::{ChangeKind, InMemoryLedger, Keypair, LedgerRecord};
use serde_json::json;

use crate::keys::{experiment_name, idempotency_key, operation_of, platform_of};
use crate::model::{Action, ActionOutcome, AutopilotPolicy, PlanError, Proposal, SettleOutcome};
use crate::ops::PlatformOps;
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
    }
}

fn variant(id: &str, revenue: i64, impressions: u64) -> ExperimentVariantResult {
    let ecpm = apex_core::ecpm_micros(revenue, impressions).unwrap_or(0);
    ExperimentVariantResult {
        variant_id: id.into(),
        impressions,
        revenue,
        ecpm,
    }
}

fn finished_experiment(
    results: Vec<ExperimentVariantResult>,
    winner: Option<&str>,
    confidence_bp: Option<i32>,
) -> ExperimentResult {
    ExperimentResult {
        experiment_id: "exp-1".into(),
        state: ExperimentState::Completed,
        results,
        lift_percent_bp: None,
        confidence: confidence_bp,
        winner_variant_id: winner.map(str::to_string),
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
        ))
        .expect("seed change")
        .seq()
}

// ---------- keys & attribution ----------

#[test]
fn idempotency_keys_are_pinned_exactly() {
    assert_eq!(idempotency_key(&floor_action()).unwrap(), "max-floor:u1:EE");
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
        "levelplay-price:u1:e1"
    );
    assert_eq!(
        idempotency_key(&Action::CreateWaterfallEntry {
            ad_unit_id: "u1".into(),
            network: "mintegral".into(),
            instance_name: "MTG_1".into(),
            price_ecpm: 750_000,
        })
        .unwrap(),
        "levelplay-create:u1:mintegral:MTG_1"
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
    assert_eq!(a, "max-experiment:autopilot-floor-u1-ee:u1");
    assert_eq!(
        experiment_name(&ecpm_action()),
        "max-experiment:autopilot-ecpm-g1-u2:u2"
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
        seed_change(&mut l, NOW + i, &format!("seed-{i}"));
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
                change_seq: 0,
                experiment_id: "exp-default".into(),
                idempotency_key: "max-floor:u1:EE".into(),
            },
            ActionOutcome::Applied {
                change_seq: 2,
                experiment_id: "exp-default".into(),
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
async fn verify_live_endpoints_refuse_before_apply() {
    let ops = crate::ops::RecordingOps::new();
    ops.set_writable("updateEntryPrice", false);
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
    .expect_err("not writable");
    assert!(
        matches!(err, PlanError::EndpointNotWritable { ref operation } if operation == "updateEntryPrice")
    );
    let log = ops.call_log();
    assert_eq!(log, vec!["writable:updateEntryPrice".to_string()]);
}

// ---------- cycle: apply paths ----------

#[tokio::test]
async fn apply_failure_leaves_no_ledger_record() {
    let ops = crate::ops::RecordingOps::new();
    ops.script_apply(vec![Err(PlatformError::Auth {
        platform: PlatformId::AppLovinMax,
        operation: "setCountryFloor".into(),
        status: 401,
    })]);
    let mut l = ledger();
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect_err("apply fails");
    assert!(matches!(err, PlanError::ApplyFailed { .. }));
    assert!(l.is_empty(), "no ledger record for a failed apply");
    let log = ops.call_log();
    assert!(
        !log.iter().any(|entry| entry.starts_with("start:")),
        "no experiment may start for a failed apply"
    );
}

#[tokio::test]
async fn successful_apply_records_change_and_experiment() {
    let ops = crate::ops::RecordingOps::new();
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
    assert_eq!(l.len(), 2);
    match (l.get(0), l.get(1)) {
        (
            Some(LedgerRecord::ChangeApplied {
                idempotency_key, ..
            }),
            Some(LedgerRecord::ExperimentStarted { experiment_id, .. }),
        ) => {
            assert_eq!(idempotency_key, "max-floor:u1:EE");
            assert_eq!(experiment_id, "exp-default");
        }
        other => panic!("unexpected ledger shape: {other:?}"),
    }
}

#[tokio::test]
async fn experiment_start_failure_rolls_back_exactly_once() {
    let ops = crate::ops::RecordingOps::new();
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
    .expect("cycle completes with a rollback outcome");
    assert_eq!(
        outcome.outcomes,
        vec![ActionOutcome::RolledBack {
            change_seq: 0,
            reason: "experiment_start_failed".into()
        }]
    );
    assert_eq!(outcome.applied, 1, "the apply itself was real and recorded");
    let rollbacks = ops
        .call_log()
        .iter()
        .filter(|entry| entry.starts_with("rollback:"))
        .count();
    assert_eq!(rollbacks, 1);
    match l.get(1) {
        Some(LedgerRecord::RollbackPerformed {
            rolled_back_seq,
            reason,
            ..
        }) => {
            assert_eq!(*rolled_back_seq, 0);
            assert_eq!(reason, "experiment_start_failed");
        }
        other => panic!("expected rollback record, got {other:?}"),
    }
}

#[tokio::test]
async fn rollback_failure_surfaces_loudly() {
    let ops = crate::ops::RecordingOps::new();
    ops.script_start(vec![Err(PlatformError::Http {
        platform: PlatformId::AppLovinMax,
        operation: "createExperiment".into(),
        status: 503,
    })]);
    ops.script_rollback(vec![Err(PlatformError::Network {
        platform: PlatformId::AppLovinMax,
        operation: "rollback".into(),
        detail: "reset".into(),
    })]);
    let mut l = ledger();
    let err = run_cycle(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        &[],
        vec![proposal(floor_action())],
        NOW,
    )
    .await
    .expect_err("loud failure");
    assert!(matches!(err, PlanError::RollbackFailed { .. }));
    assert_eq!(
        l.len(),
        1,
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
            skipped: 0
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

#[tokio::test]
async fn settle_refuses_a_running_experiment() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    ops.put_experiment(
        "exp-1",
        ExperimentResult {
            experiment_id: "exp-1".into(),
            state: ExperimentState::Running,
            results: vec![],
            lift_percent_bp: None,
            confidence: None,
            winner_variant_id: None,
        },
    );
    let err = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW,
    )
    .await
    .expect_err("running");
    assert!(matches!(err, PlanError::PolicyViolation { .. }));
}

#[tokio::test]
async fn settle_no_baseline_cases() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");

    ops.put_experiment("empty", finished_experiment(vec![], None, Some(9_900)));
    assert!(matches!(
        settle_experiment(
            &ops,
            &mut l,
            &AutopilotPolicy::conservative(),
            "pub",
            "empty",
            seq,
            NOW
        )
        .await,
        Err(PlanError::NoBaseline { .. })
    ));

    ops.put_experiment(
        "zero-control",
        finished_experiment(vec![variant("control", 0, 1_000)], None, Some(9_900)),
    );
    assert!(matches!(
        settle_experiment(
            &ops,
            &mut l,
            &AutopilotPolicy::conservative(),
            "pub",
            "zero-control",
            seq,
            NOW
        )
        .await,
        Err(PlanError::NoBaseline { .. })
    ));

    ops.put_experiment(
        "missing-winner",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("b", 1_100_000, 1_000),
            ],
            Some("ghost"),
            Some(9_900),
        ),
    );
    assert!(matches!(
        settle_experiment(
            &ops,
            &mut l,
            &AutopilotPolicy::conservative(),
            "pub",
            "missing-winner",
            seq,
            NOW
        )
        .await,
        Err(PlanError::NoBaseline { .. })
    ));

    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("v2", 1_100_000, 1_000),
            ],
            Some("v2"),
            Some(9_900),
        ),
    );
    assert!(matches!(
        settle_experiment(
            &ops,
            &mut l,
            &AutopilotPolicy::conservative(),
            "pub",
            "exp-1",
            999,
            NOW
        )
        .await,
        Err(PlanError::NoBaseline { .. })
    ));
}

#[tokio::test]
async fn settle_refuses_a_non_change_sequence() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    l.append(LedgerRecord::experiment_started(
        NOW,
        PlatformId::Admob,
        "exp-x",
        ExperimentSpec {
            name: "n".into(),
            ad_unit_id: "u1".into(),
            metric: ExperimentMetric::Ecpm,
            variants: 2,
            change_description: "c".into(),
        },
    ))
    .expect("started");
    ops.put_experiment(
        "exp-x",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("v2", 1_100_000, 1_000),
            ],
            Some("v2"),
            Some(9_900),
        ),
    );
    let err = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-x",
        0,
        NOW,
    )
    .await
    .expect_err("not a change");
    assert!(matches!(err, PlanError::NoBaseline { .. }));
}

#[tokio::test]
async fn settle_keep_golden_arithmetic_and_billing_proof() {
    // control ecpm 1.000000, winner ecpm 1.025000 => lift 250 bp; confidence 9800.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 50_000_000, 50_000),
                variant("v2", 51_250_000, 50_000),
            ],
            Some("v2"),
            Some(9_800),
        ),
    );
    let outcome = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW + 1,
    )
    .await
    .expect("settle");
    match outcome {
        SettleOutcome::Kept {
            lift_bp,
            confidence_bp,
            winner_variant_id,
            fee_micros,
            billing_seq,
        } => {
            assert_eq!(lift_bp, 250);
            assert_eq!(confidence_bp, 9_800);
            assert_eq!(winner_variant_id, "v2");
            // proven lift = 51_250_000 - 50_000_000 = 1_250_000 micros; 20% -> 250_000
            assert_eq!(fee_micros, Some(250_000));
            assert_eq!(billing_seq, Some(2));
        }
        other => panic!("expected Kept, got {other:?}"),
    }
    assert_eq!(l.len(), 3, "seeded change + observed + billing proof");
    match l.get(2) {
        Some(LedgerRecord::BillingProof {
            fee_micros,
            rate_bp,
            proven_lift,
            ..
        }) => {
            assert_eq!(*fee_micros, 250_000);
            assert_eq!(*rate_bp, 2_000);
            assert_eq!(*proven_lift, 1_250_000);
        }
        other => panic!("expected billing proof, got {other:?}"),
    }
}

#[tokio::test]
async fn settle_keep_boundaries_are_inclusive() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    // Exactly +100 bp and confidence exactly 9500.
    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("v2", 1_010_000, 1_000),
            ],
            Some("v2"),
            Some(9_500),
        ),
    );
    let outcome = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW,
    )
    .await
    .expect("settle");
    assert!(matches!(
        outcome,
        SettleOutcome::Kept {
            lift_bp: 100,
            confidence_bp: 9_500,
            ..
        }
    ));
}

#[tokio::test]
async fn settle_rolls_back_below_lift_floor() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("v2", 1_009_000, 1_000),
            ],
            Some("v2"),
            Some(9_900),
        ),
    );
    let outcome = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW,
    )
    .await
    .expect("settle");
    match outcome {
        SettleOutcome::RolledBack {
            lift_bp,
            rollback_seq,
            reason,
            ..
        } => {
            assert_eq!(lift_bp, 90);
            assert_eq!(reason, "below_policy");
            assert_eq!(rollback_seq, 1);
        }
        other => panic!("expected RolledBack, got {other:?}"),
    }
    let rollbacks = ops
        .call_log()
        .iter()
        .filter(|entry| entry.starts_with("rollback:"))
        .count();
    assert_eq!(rollbacks, 1);
    assert!(
        !l.records()
            .iter()
            .any(|r| matches!(r, LedgerRecord::BillingProof { .. })),
        "rolled-back lift never bills"
    );
}

#[tokio::test]
async fn settle_rolls_back_below_confidence_floor() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("v2", 1_100_000, 1_000),
            ],
            Some("v2"),
            Some(9_499),
        ),
    );
    let outcome = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW,
    )
    .await
    .expect("settle");
    assert!(matches!(
        outcome,
        SettleOutcome::RolledBack {
            confidence_bp: 9_499,
            ..
        }
    ));
}

#[tokio::test]
async fn settle_rolls_back_zero_lift_without_billing() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("v2", 1_000_000, 1_000),
            ],
            Some("v2"),
            Some(9_900),
        ),
    );
    let outcome = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW,
    )
    .await
    .expect("settle");
    assert!(matches!(
        outcome,
        SettleOutcome::RolledBack { lift_bp: 0, .. }
    ));
    assert!(!l
        .records()
        .iter()
        .any(|r| matches!(r, LedgerRecord::BillingProof { .. })));
}

#[tokio::test]
async fn settle_rolls_back_negative_lift() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("v2", 900_000, 1_000),
            ],
            Some("v2"),
            Some(9_900),
        ),
    );
    let outcome = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW,
    )
    .await
    .expect("settle");
    // 900k vs 1.0M micros of eCPM is -10% = -1000 basis points.
    assert!(
        matches!(
            outcome,
            SettleOutcome::RolledBack {
                lift_bp: -1_000,
                ..
            }
        ),
        "actual: {outcome:?}"
    );
}

#[tokio::test]
async fn settle_winner_fallback_picks_highest_ecpm_non_control() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("worse", 900_000, 1_000),
                variant("best", 1_300_000, 1_000),
            ],
            None,
            Some(9_900),
        ),
    );
    let outcome = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW,
    )
    .await
    .expect("settle");
    // 1.3M vs 1.0M micros of eCPM is +30% = 3000 basis points; the proven
    // revenue delta 300_000 micros at 20% bills 60_000 micros.
    assert!(
        matches!(
            outcome,
            SettleOutcome::Kept {
                lift_bp: 3_000,
                fee_micros: Some(60_000),
                ..
            } if outcome_winner(&outcome) == "best"
        ),
        "actual: {outcome:?}"
    );
}

fn outcome_winner(outcome: &SettleOutcome) -> &str {
    match outcome {
        SettleOutcome::Kept {
            winner_variant_id, ..
        } => winner_variant_id,
        SettleOutcome::RolledBack { .. } => "",
    }
}

#[tokio::test]
async fn settle_keep_without_revenue_delta_has_no_fee() {
    // Winner ecpm is higher (fewer impressions, same revenue): no incremental
    // revenue, so no billing proof even though the change is kept.
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 5_000_000, 10_000),
                variant("v2", 5_000_000, 5_000),
            ],
            Some("v2"),
            Some(9_900),
        ),
    );
    let outcome = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW,
    )
    .await
    .expect("settle");
    match outcome {
        SettleOutcome::Kept {
            lift_bp,
            fee_micros,
            billing_seq,
            ..
        } => {
            assert_eq!(lift_bp, 10_000);
            assert_eq!(fee_micros, None);
            assert_eq!(billing_seq, None);
        }
        other => panic!("expected Kept, got {other:?}"),
    }
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
        ))
        .expect("seed")
        .seq();
    ops.put_experiment(
        "exp-1",
        finished_experiment(
            vec![
                variant("control", 1_000_000, 1_000),
                variant("v2", 1_100_000, 1_000),
            ],
            Some("v2"),
            Some(9_900),
        ),
    );
    let err = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "exp-1",
        seq,
        NOW,
    )
    .await
    .expect_err("platform mismatch");
    assert!(matches!(err, PlanError::Validation(_)));
}

#[tokio::test]
async fn settle_unknown_experiment_is_an_apply_failure() {
    let ops = crate::ops::RecordingOps::new();
    let mut l = ledger();
    let seq = seed_change(&mut l, NOW, "k1");
    let err = settle_experiment(
        &ops,
        &mut l,
        &AutopilotPolicy::conservative(),
        "pub",
        "nope",
        seq,
        NOW,
    )
    .await
    .expect_err("unknown experiment");
    assert!(matches!(err, PlanError::ApplyFailed { .. }));
}
