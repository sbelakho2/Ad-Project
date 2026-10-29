//! Chaos property for the keep-to-bill pipeline: a change is applied, tested
//! and kept, then random "crashes" erase individual ledger appends (the
//! holdout-state bookkeeping, the signed monthly fee) or the issued invoice
//! record. Every pipeline stage is replayed from the signed ledger until it
//! completes, and the final state must be byte-for-byte the same business
//! state as a crash-free run: one experiment start, one confirmed holdout,
//! exactly one monthly fee record, and one invoice with the reference total.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use apex_autopilot::model::{Action, AutopilotPolicy, Proposal};
use apex_autopilot::ops::{ExperimentDays, PlatformOps, PlayerEvidence, RecordingOps};
use apex_autopilot::{calendar, run_cycle, Runner};
use apex_lift::guardrails::FillFormat;
use apex_lift::holdout::{holdout_experiment_id, AssignedRoster, HoldoutStatus, MEASUREMENT_DAYS};
use apex_lift::player_store::PlayerDay;
use apex_lift::records::LiftRecord;
use apex_lift::window::ArmDay;
use apex_proof::{InMemoryLedger, Keypair, LedgerError, LedgerLog, LedgerRecord};

const NOW: i64 = 1_700_000_000_000;
const MONTH: &str = "2023-11";
const MONTH_START: &str = "2023-11-01";
const MONTH_END: &str = "2023-11-30";
const MONTH_DAYS: u32 = 30;
const PUBLISHER: &str = "pub1";

fn policy() -> AutopilotPolicy {
    AutopilotPolicy {
        window_days: 7,
        interim_look: false,
        extension_allowed: false,
        ..AutopilotPolicy::conservative()
    }
}

fn floor_proposal() -> Proposal {
    Proposal {
        action: Action::SetCountryFloor {
            ad_unit_id: "u1".into(),
            country: "ee".into(),
            floor_ecpm: 2_500_000,
        },
        rationale: "raise the EE floor after the audit found a stale baseline".into(),
        expected_effect_bp: 500,
        prev_value: "{}".into(),
    }
}

fn arm_day(day: &str, revenue: i64) -> ArmDay {
    ArmDay {
        day: day.to_string(),
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

/// The engine's window: three burn-in days plus seven analyzed days.
fn constant_days(control: i64, test: i64) -> ExperimentDays {
    ExperimentDays {
        control: (0..10)
            .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), control / 4 * 3))
            .collect(),
        test: (0..10)
            .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), test / 4))
            .collect(),
    }
}

fn running_experiment() -> apex_core::ExperimentResult {
    apex_core::ExperimentResult {
        experiment_id: "exp-1".into(),
        state: apex_core::ExperimentState::Running,
        results: vec![],
        lift_percent_bp: None,
        confidence: None,
        winner_variant_id: None,
    }
}

/// The adapter whose cycle keeps the change: the test arm earns +10%, and the
/// live settings still match the pre-registered before-value (no edit).
fn kept_ops() -> Arc<RecordingOps> {
    let ops = Arc::new(RecordingOps::new());
    ops.set_change_in_test_arm(true);
    ops.script_start(vec![Ok("exp-1".into())]);
    ops.put_experiment("exp-1", running_experiment());
    ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
    ops.put_scope_revenue("u1", 1_000_000_000);
    ops.put_current_settings("u1", "{}");
    ops
}

fn seller_profile() -> apex_billing::BillingProfile {
    apex_billing::BillingProfile {
        legal_name: "Apex Media OÜ".into(),
        address_lines: vec!["Sepapaja tn 6".into()],
        country: "EE".into(),
        vat_number: "EE123456789".into(),
        vat_rate_bp: 2_000,
    }
}

fn customer(country: &str) -> apex_billing::CustomerProfile {
    apex_billing::CustomerProfile {
        legal_name: "Buyer OÜ".into(),
        address_lines: vec!["Telliskivi 1".into()],
        country: country.into(),
        vat_number: None,
    }
}

fn trial_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("apex-chaos-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("trial dir");
    dir
}

/// The parsed lift payloads of a ledger, in ledger order.
fn lift_payloads(ledger: &InMemoryLedger) -> Vec<LiftRecord> {
    ledger
        .records()
        .iter()
        .filter_map(|record| match record {
            LedgerRecord::LiftRecord { payload, .. } => {
                serde_json::from_str::<LiftRecord>(payload).ok()
            }
            _ => None,
        })
        .collect()
}

/// The immutable instance of the change the pipeline kept.
fn kept_instance(ledger: &InMemoryLedger) -> Option<String> {
    lift_payloads(ledger)
        .into_iter()
        .find_map(|parsed| match parsed {
            LiftRecord::Result {
                experiment_id,
                instance_id,
                decision,
                ..
            } if decision == "keep" => Some(if instance_id.is_empty() {
                experiment_id
            } else {
                instance_id
            }),
            _ => None,
        })
}

/// The keep is complete when its signed result carries a confirmed holdout
/// state and the daily scan's completion marker is on the ledger.
fn keep_complete(ledger: &InMemoryLedger) -> bool {
    let Some(instance) = kept_instance(ledger) else {
        return false;
    };
    let confirmed = lift_payloads(ledger).iter().any(|parsed| {
        matches!(
            parsed,
            LiftRecord::HoldoutState(state)
                if state.change_id == instance && state.status == HoldoutStatus::Active
        )
    });
    let observed = ledger.records().iter().any(|record| {
        matches!(
            record,
            LedgerRecord::ExperimentObserved { experiment_id, .. } if experiment_id == "exp-1"
        )
    });
    confirmed && observed
}

/// Script the holdout player feed the monthly pass reads: three hundred
/// identified players per day across the trailing 28 days, exposed +4%.
fn script_holdout_players(ops: &Arc<RecordingOps>, ledger: &InMemoryLedger) {
    let instance = kept_instance(ledger).expect("kept instance to script players for");
    let id = holdout_experiment_id(&instance);
    let end = calendar::day_number(MONTH_END).expect("month end day");
    let mut rows = Vec::new();
    for offset in 0..MEASUREMENT_DAYS {
        let day = calendar::format_day(end - 27 + offset as i64);
        for player in 0..270 {
            rows.push(PlayerDay {
                day: day.clone(),
                user_id: Some(format!("exposed-{player}")),
                idfv: None,
                idfa: None,
                test: false,
                revenue_micros: 104_008,
            });
            if player >= 30 {
                continue;
            }
            rows.push(PlayerDay {
                day: day.clone(),
                user_id: Some(format!("holdout-{player}")),
                idfv: None,
                idfa: None,
                test: true,
                revenue_micros: 100_000,
            });
        }
    }
    ops.put_experiment_players(
        &id,
        PlayerEvidence {
            rows,
            pretest: std::collections::BTreeMap::new(),
            measurement_scope: Some("u1".to_string()),
            roster: Some(AssignedRoster {
                exposed: 270,
                control: 30,
            }),
            eligible: None,
            certified_gain_lower: Some(900_000_000),
            certified_delta_lower: Some(0.04),
            assigned_test_control: None,
        },
    );
}

/// Which durable append the crash erases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Crash {
    /// The holdout-state append of the keep.
    HoldoutState,
    /// The signed monthly fee append.
    BillingFee,
}

fn draft_matches(crash: Crash, draft: &LedgerRecord) -> bool {
    let LedgerRecord::LiftRecord { payload, .. } = draft else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<LiftRecord>(payload) else {
        return false;
    };
    match crash {
        Crash::HoldoutState => matches!(parsed, LiftRecord::HoldoutState(_)),
        Crash::BillingFee => matches!(parsed, LiftRecord::MonthlyBilling(_)),
    }
}

/// A ledger that loses the next queued record kind: the process died between
/// the side effect and the append becoming durable. Everything else passes
/// through untouched, so the surviving chain stays verifiable.
struct CrashyLedger<'a> {
    inner: &'a mut InMemoryLedger,
    pending: &'a mut VecDeque<Crash>,
}

impl CrashyLedger<'_> {
    fn take_matching(&mut self, draft: &LedgerRecord) -> bool {
        let index = self
            .pending
            .iter()
            .position(|crash| draft_matches(*crash, draft));
        match index {
            Some(index) => self.pending.remove(index).is_some(),
            None => false,
        }
    }
}

impl LedgerLog for CrashyLedger<'_> {
    fn append(&mut self, draft: LedgerRecord) -> Result<LedgerRecord, LedgerError> {
        if self.take_matching(&draft) {
            return Err(LedgerError::Io(std::io::Error::other(
                "simulated crash: the append was lost before it became durable",
            )));
        }
        self.inner.append(draft)
    }

    fn append_rollback(
        &mut self,
        at_unix_ms: i64,
        platform: apex_core::PlatformId,
        ad_unit_id: &str,
        rolled_back_seq: u64,
        reason: &str,
    ) -> Result<LedgerRecord, LedgerError> {
        self.inner
            .append_rollback(at_unix_ms, platform, ad_unit_id, rolled_back_seq, reason)
    }

    fn records(&self) -> Vec<&LedgerRecord> {
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

/// The final business state of one pipeline run: the facts the chaos property
/// pins. Two runs with different crash placements must be exactly equal.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FinalState {
    kept_instance: String,
    /// `(change id, fee micros)` of every signed monthly fee in the ledger.
    fee_records: Vec<(String, i64)>,
    /// Holdout-state records for the kept change (request + confirmation).
    holdout_records: usize,
    /// The status the last holdout-state record carries.
    holdout_final_status: Option<HoldoutStatus>,
    /// The platform experiment id the holdout state names.
    holdout_platform_ids: Vec<String>,
    /// `experimentStarted` records for the tested experiment.
    started_records: usize,
    /// `experimentObserved` completion markers for the tested experiment.
    observed_records: usize,
    /// Platform `start` calls, deduplicated: one tested change means exactly
    /// one platform experiment, whatever the crash placements were.
    distinct_experiment_starts: Vec<String>,
    /// Platform `holdout` calls, deduplicated (a retried deterministic
    /// request is the same platform experiment, not an orphan).
    distinct_holdout_starts: Vec<String>,
    /// The invoice gross total the final monthly pass reported.
    invoice_total_micros: i64,
    /// The fee lines the final monthly pass billed.
    report_fees: Vec<(String, i64)>,
    /// Lines in the invoice register (one issued invoice per period).
    invoice_lines: usize,
}

fn audit_final_state(
    ops: &Arc<RecordingOps>,
    ledger: &InMemoryLedger,
    dir: &Path,
    report: &apex_autopilot::MonthlyReport,
) -> FinalState {
    let payloads = lift_payloads(ledger);
    let kept_instance = kept_instance(ledger).expect("kept instance");
    let mut holdout_records = 0;
    let mut holdout_final_status = None;
    let mut holdout_platform_ids = Vec::new();
    let mut fee_records = Vec::new();
    for parsed in &payloads {
        match parsed {
            LiftRecord::HoldoutState(state) if state.change_id == kept_instance => {
                holdout_records += 1;
                holdout_final_status = Some(state.status);
                holdout_platform_ids.push(state.holdout_experiment_id.clone());
            }
            LiftRecord::MonthlyBilling(billing) => {
                fee_records.push((billing.change_id.clone(), billing.fee_micros));
            }
            _ => {}
        }
    }
    let count_records = |predicate: &dyn Fn(&LedgerRecord) -> bool| {
        ledger
            .records()
            .iter()
            .filter(|record| predicate(record))
            .count()
    };
    let started_records = count_records(&|record| {
        matches!(
            record,
            LedgerRecord::ExperimentStarted { experiment_id, .. } if experiment_id == "exp-1"
        )
    });
    let observed_records = count_records(&|record| {
        matches!(
            record,
            LedgerRecord::ExperimentObserved { experiment_id, .. } if experiment_id == "exp-1"
        )
    });
    let log = ops.call_log();
    let mut distinct_experiment_starts = log
        .iter()
        .filter_map(|entry| entry.strip_prefix("start:"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    distinct_experiment_starts.sort();
    distinct_experiment_starts.dedup();
    let mut distinct_holdout_starts = log
        .iter()
        .filter_map(|entry| entry.strip_prefix("holdout:"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    distinct_holdout_starts.sort();
    distinct_holdout_starts.dedup();
    let register = std::fs::read_to_string(dir.join("invoices.jsonl")).unwrap_or_default();
    let invoice_lines = register
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    FinalState {
        kept_instance,
        fee_records,
        holdout_records,
        holdout_final_status,
        holdout_platform_ids,
        started_records,
        observed_records,
        distinct_experiment_starts,
        distinct_holdout_starts,
        invoice_total_micros: report.invoice_total_micros,
        report_fees: report.fees.clone(),
        invoice_lines,
    }
}

/// Run the whole keep-to-bill pipeline, replaying every stage until it
/// completes despite the queued crashes. `invoice_crash` wipes the issued
/// invoice record after the first successful monthly pass, so the replay must
/// re-derive the billing outcome from the ledger's signed fee alone.
async fn run_pipeline(
    crashes: &mut VecDeque<Crash>,
    invoice_crash: bool,
    label: &str,
) -> FinalState {
    let dir = trial_dir(label);
    std::fs::write(
        dir.join(format!("{PUBLISHER}.billing.json")),
        serde_json::to_string_pretty(&customer("EE")).expect("customer json"),
    )
    .expect("customer profile");
    let ops = kept_ops();
    let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[7u8; 32]));

    run_cycle(
        ops.as_ref(),
        &mut ledger,
        &policy(),
        &[],
        vec![floor_proposal()],
        NOW,
    )
    .await
    .expect("the write cycle applies the change and starts the experiment");

    let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
        .with_publisher(PUBLISHER)
        .with_vat_rate(2_000)
        .with_billing_profile(seller_profile())
        .with_connections_root(&dir)
        .with_register_path(dir.join("invoices.jsonl"));

    let mut replays = 0;
    loop {
        let mut crashy = CrashyLedger {
            inner: &mut ledger,
            pending: &mut *crashes,
        };
        let _ = runner.run_daily(&mut crashy, NOW + 1).await;
        replays += 1;
        if keep_complete(&ledger) {
            break;
        }
        assert!(
            replays < 8,
            "the keep never completed after {replays} replays; pending crashes: {crashes:?}"
        );
    }

    script_holdout_players(&ops, &ledger);

    // Audit item 8: holdout billing is gated on a valid A/A qualification
    // for the publisher+platform, so the fixture records the clean null run
    // before the monthly pass.
    runner
        .record_aa_qualification(
            &mut ledger,
            apex_core::PlatformId::AppLovinMax,
            0.5,
            1.0,
            0.0,
            NOW,
        )
        .expect("the clean A/A qualification records");

    let mut report = None;
    let mut replays = 0;
    loop {
        let mut crashy = CrashyLedger {
            inner: &mut ledger,
            pending: &mut *crashes,
        };
        let attempt = runner
            .run_monthly(
                &mut crashy,
                PUBLISHER,
                MONTH,
                MONTH_START,
                MONTH_DAYS,
                NOW + 2,
            )
            .await;
        replays += 1;
        let finished = match attempt {
            Ok(monthly) => {
                let done = monthly.invoice_number.is_some() || monthly.invoice_refusal.is_some();
                if !done {
                    eprintln!("chaos: monthly pass {replays} not done: {monthly:?}");
                }
                report = Some(monthly);
                done
            }
            Err(error) => {
                eprintln!("chaos: monthly pass {replays} failed: {error}");
                false
            }
        };
        if finished {
            break;
        }
        assert!(
            replays < 8,
            "the monthly pass never issued after {replays} replays; pending crashes: {crashes:?}"
        );
    }
    let mut report = report.expect("monthly report");

    if invoice_crash {
        let _ = std::fs::remove_file(dir.join("invoices.jsonl"));
        report = runner
            .run_monthly(
                &mut ledger,
                PUBLISHER,
                MONTH,
                MONTH_START,
                MONTH_DAYS,
                NOW + 3,
            )
            .await
            .expect("the invoice re-issues from the ledger's signed fee");
    }

    let state = audit_final_state(&ops, &ledger, &dir, &report);
    let _ = std::fs::remove_dir_all(&dir);
    state
}

/// A tiny deterministic xorshift: the property must be reproducible run to
/// run, so the crash placements are seeded, not wall-clock random.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[tokio::test]
async fn crashes_before_appends_replay_to_the_identical_final_state() {
    let reference = run_pipeline(&mut VecDeque::new(), false, "reference").await;
    assert!(
        reference.invoice_total_micros > 0,
        "the fixture must bill a positive fee: {reference:?}"
    );
    assert_eq!(
        reference.fee_records.len(),
        1,
        "exactly one fee: {reference:?}"
    );
    assert_eq!(
        reference.holdout_records, 2,
        "request + confirmation, nothing else: {reference:?}"
    );
    assert_eq!(
        reference.holdout_final_status,
        Some(HoldoutStatus::Active),
        "{reference:?}"
    );
    assert_eq!(reference.started_records, 1, "{reference:?}");
    assert_eq!(reference.observed_records, 1, "{reference:?}");
    assert_eq!(
        reference.distinct_experiment_starts.len(),
        1,
        "{reference:?}"
    );
    assert_eq!(reference.distinct_holdout_starts.len(), 1, "{reference:?}");
    assert_eq!(reference.invoice_lines, 1, "{reference:?}");
    assert_eq!(
        reference.distinct_holdout_starts[0],
        holdout_experiment_id(&reference.kept_instance),
        "{reference:?}"
    );

    for seed in 0..8_u64 {
        let mut rng = Rng::new(seed);
        let count = (rng.next() % 4) as usize;
        let mut crashes = VecDeque::new();
        let mut invoice_crash = false;
        for _ in 0..count {
            match rng.next() % 3 {
                0 => crashes.push_back(Crash::HoldoutState),
                1 => crashes.push_back(Crash::BillingFee),
                _ => invoice_crash = true,
            }
        }
        let trial = run_pipeline(&mut crashes, invoice_crash, &format!("seed-{seed}")).await;
        assert_eq!(
            trial, reference,
            "seed {seed} ({count} crashes, invoice crash: {invoice_crash}) diverged from the crash-free run"
        );
    }
}
