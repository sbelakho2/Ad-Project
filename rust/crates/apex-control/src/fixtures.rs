//! The deterministic illustrative dataset.
//!
//! These are the "illustrative data" samples of the design document: the
//! Northstar, Orbit, Atlas and Comet accounts, the exp_0246 to exp_0250 test
//! fleet and the known-value fee fixture (USD 51,500.00 at a 3 percent bound
//! and the agreed 20 percent rate gives a USD 300.00 fee). Every page marks
//! the dataset as `ILLUSTRATIVE DATA`; the fixture renderer and the tests
//! use it so the real rendered HTML can be scanned, and the live deployment
//! never touches it.
//!
//! The instant is the document's own: 15 Oct 2026, 09:10 UTC.

use apex_analytics::queries::{
    ActivationFunnel, DataQuality, ExperimentEffectiveness, ExperimentLandscapeRow, FeeDetail,
    FeeRow, FeeState, FreshnessStatus, FunnelStageRow, GrowthCohortRow, IngestionHealthRow,
    LiftRow, Money, ObjectTimelineEntry, OperatorQueues, StaleSource,
};
use apex_analytics::{EventSource, EventType, UtcMillis};

use crate::data::{
    DurableExperiment, InvoiceLineRecord, InvoiceRecord, OperationEvidence, OperationsEvidence,
};
use crate::signing::sha256_hex;

/// The illustrative instant: 15 Oct 2026, 09:10 UTC.
pub const DEMO_NOW_MS: i64 = 1_792_055_400_000;

/// One customer row of the account inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerRow {
    /// Account identity.
    pub account_id: String,
    /// Publisher identity.
    pub publisher_id: String,
    /// Display name.
    pub display_name: String,
    /// Lifecycle stage (`active_tests`, `paying`, `audit_requested`, ...).
    pub lifecycle: String,
    /// Platform readiness lines (`MAX: write verified`, ...).
    pub platform_readiness: Vec<String>,
    /// Health phrase (`Restore failed`, `Proof hold`, ...).
    pub health: String,
    /// Owner display name.
    pub owner: Option<String>,
    /// Days since the stage last moved.
    pub stage_age_days: i64,
    /// Last meaningful activity description.
    pub last_activity: String,
    /// Masked contact line for broad views.
    pub contact_masked: Option<String>,
    /// Running tests count.
    pub running_tests: u64,
    /// Billing state phrase.
    pub billing_state: String,
}

/// One calendar day of reporting coverage in the ingestion matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageRow {
    /// Publisher.
    pub publisher_id: String,
    /// Platform.
    pub platform: String,
    /// Platform account.
    pub platform_account_id: String,
    /// Expected reporting days.
    pub expected_days: Vec<String>,
    /// Per-day verdict: `Final`, `Provisional`, `Missing`, `Not expected`.
    pub verdicts: Vec<String>,
    /// Latest finalized day, when any.
    pub last_final_day: Option<String>,
    /// Latest completed pull time (display string).
    pub last_pull: Option<String>,
    /// Latest finalized source day (display string).
    pub last_final_display: Option<String>,
}

/// One per-rule audit quality row (page 60).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRuleRow {
    /// Rule name.
    pub rule: String,
    /// Eligible runs (`118 / 140`).
    pub eligible_runs: String,
    /// Findings count.
    pub findings: u64,
    /// Review signal text.
    pub review_signal: String,
}

/// The audit quality snapshot (page 60).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQuality {
    /// Successful runs (`138 / 140`).
    pub successful_runs: String,
    /// Eligible rule checks (`612 / 700`).
    pub eligible_checks: String,
    /// Runs that could not run (`88 lack required inputs`).
    pub ineligible_note: String,
    /// Critical findings.
    pub critical_findings: u64,
    /// Customers affected by critical findings.
    pub critical_customers: u64,
    /// Per-rule rows.
    pub rules: Vec<AuditRuleRow>,
}

/// An incident case assembled for the detail page (page 64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentCase {
    /// Incident identity (`inc_018`).
    pub id: String,
    /// Severity (`critical`/`warning`).
    pub severity: String,
    /// Age in seconds.
    pub age_seconds: u64,
    /// Publisher.
    pub publisher_id: String,
    /// Owner.
    pub owner: Option<String>,
    /// The headline above the fold.
    pub headline: String,
    /// Affected experiment, when known.
    pub experiment: Option<String>,
    /// Affected scope, when known.
    pub scope: Option<String>,
    /// Last confirmed platform state, when known.
    pub last_confirmed_state: Option<String>,
    /// The failed operation, when known.
    pub failed_operation: Option<String>,
    /// Occurrence time in the console's UTC format.
    pub occurred_display: Option<String>,
    /// Next update commitment.
    pub next_update: Option<String>,
    /// Customer communication draft.
    pub communication_draft: String,
    /// Whether the operation can affect active traffic.
    pub affects_traffic: Option<bool>,
    /// Timeline entries, merged by occurrence time.
    pub timeline: Vec<ObjectTimelineEntry>,
}

/// The before-intervention preview fields (page 64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterventionPreview {
    /// Publisher.
    pub publisher: String,
    /// Operation.
    pub operation: String,
    /// Prior state.
    pub prior_state: String,
    /// Target state.
    pub target_state: String,
    /// Governing policy.
    pub policy: String,
    /// Permissions required.
    pub permissions: String,
    /// Whether the operation can affect active traffic.
    pub affects_traffic: bool,
}

/// The invoice register line for one invoice id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceRegister {
    /// Invoices.
    pub invoices: Vec<InvoiceRecord>,
    /// Payment timeline events per invoice id.
    pub payments: Vec<PaymentEvent>,
}

/// One payment timeline event (page 68).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentEvent {
    /// Invoice id.
    pub invoice_id: String,
    /// Event code (Adyen notification code).
    pub event_code: String,
    /// Whether the provider reported success.
    pub success: bool,
    /// PSP reference (restricted, masked for display).
    pub psp_reference_masked: String,
    /// Amount and currency.
    pub amount: Money,
    /// UTC time of the event.
    pub at_ms: i64,
    /// Adyen event received.
    pub received: bool,
    /// HMAC verified.
    pub hmac_verified: bool,
    /// Event deduplicated.
    pub deduplicated: bool,
    /// Payment state reconciled.
    pub reconciled: bool,
    /// Amount allocated.
    pub allocated: bool,
}

/// One row of the monthly service economics series (page 66).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomicsMonth {
    /// Month label `YYYY-MM`.
    pub month: String,
    /// Lift-fee revenue in micros.
    pub lift_fee_revenue_micros: i64,
    /// Monitoring revenue in micros.
    pub monitoring_revenue_micros: i64,
    /// Allocated cost in micros; `None` when not allocated.
    pub allocated_cost_micros: Option<i64>,
    /// Unpaid balance in micros.
    pub unpaid_micros: i64,
    /// Support time in minutes.
    pub support_minutes: i64,
}

/// One release-health row (page 72).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseHealthRow {
    /// Service name.
    pub service: String,
    /// Deployed version or `Not instrumented`.
    pub version: String,
    /// Migration state.
    pub migration_state: String,
    /// Report-schema version.
    pub report_schema: String,
    /// Metric-schema version.
    pub metric_schema: String,
    /// Data-quality check summary.
    pub data_quality: String,
    /// Recent errors summary.
    pub recent_errors: String,
}

/// One component row of the system page (page 70).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemComponentRow {
    /// Component name.
    pub component: String,
    /// Last success in the console's UTC time format or a reason.
    pub last_success: String,
    /// Signal text.
    pub signal: String,
    /// Next action text.
    pub next_action: String,
}

/// The audit FIXTURE dataset.
#[derive(Debug, Clone)]
pub struct DemoData {
    as_of: UtcMillis,
}

impl Default for DemoData {
    fn default() -> Self {
        Self::new()
    }
}

impl DemoData {
    /// The dataset pinned at [`DEMO_NOW_MS`].
    pub fn new() -> Self {
        Self {
            as_of: UtcMillis::from_unix_millis(DEMO_NOW_MS),
        }
    }

    /// The dataset instant.
    pub fn as_of(&self) -> UtcMillis {
        self.as_of
    }

    fn minutes_ago(&self, minutes: i64) -> i64 {
        DEMO_NOW_MS - minutes * 60_000
    }

    /// The five queues: three incidents, one due decision, two stale
    /// accounts, one overdue job and one held fee.
    pub fn queues(&self) -> OperatorQueues {
        use apex_analytics::projections::OperatorQueueRow;
        let row = |queue: &str,
                   object_id: &str,
                   publisher_id: &str,
                   severity: &str,
                   age_seconds: u64,
                   owner: Option<&str>,
                   next_action: &str| OperatorQueueRow {
            queue: queue.to_string(),
            object_id: object_id.to_string(),
            publisher_id: publisher_id.to_string(),
            severity: severity.to_string(),
            age_seconds,
            owner: owner.map(str::to_string),
            next_action: next_action.to_string(),
        };
        OperatorQueues {
            as_of: self.as_of.to_rfc3339_millis(),
            incidents: vec![
                row(
                    "incidents",
                    "inc_018",
                    "northstar",
                    "critical",
                    480,
                    Some("Maya"),
                    "review_incident",
                ),
                row(
                    "incidents",
                    "inc_019",
                    "orbit",
                    "critical",
                    300,
                    Some("Sam"),
                    "review_incident",
                ),
                row(
                    "incidents",
                    "inc_017",
                    "atlas",
                    "warning",
                    7_200,
                    Some("Lina"),
                    "review_incident",
                ),
            ],
            due_decisions: vec![row(
                "due_decisions",
                "exp_0246",
                "orbit",
                "warning",
                3_600,
                None,
                "record_decision",
            )],
            stale_accounts: vec![
                row(
                    "stale_accounts",
                    "acct_northstar_max_01",
                    "northstar",
                    "warning",
                    259_200,
                    Some("Lina"),
                    "check_connection",
                ),
                row(
                    "stale_accounts",
                    "acct_atlas_admob_01",
                    "atlas",
                    "warning",
                    172_800,
                    Some("Lina"),
                    "check_connection",
                ),
            ],
            overdue_jobs: vec![row(
                "overdue_jobs",
                "job_daily",
                "northstar",
                "warning",
                1_080,
                None,
                "inspect_job",
            )],
            held_fees: vec![row(
                "held_fees",
                "ch_0246:2026-10",
                "northstar",
                "warning",
                86_400,
                None,
                "verify_proof",
            )],
        }
    }

    /// Freshness rows: four MAX accounts (one stale), two LevelPlay (one
    /// stale) and two AdMob (one with no finalized day).
    pub fn ingestion_health(&self) -> Vec<IngestionHealthRow> {
        let row = |publisher: &str,
                   platform: &str,
                   account: &str,
                   last_final: Option<&str>,
                   freshness: Option<i64>,
                   status: FreshnessStatus| IngestionHealthRow {
            publisher_id: publisher.to_string(),
            platform: platform.to_string(),
            platform_account_id: account.to_string(),
            last_final_day: last_final.map(str::to_string),
            freshness_days: freshness,
            status,
        };
        vec![
            row(
                "northstar",
                "applovin_max",
                "acct_northstar_max_01",
                Some("2026-10-11"),
                Some(4),
                FreshnessStatus::Stale,
            ),
            row(
                "northstar",
                "applovin_max",
                "acct_northstar_max_02",
                Some("2026-10-14"),
                Some(1),
                FreshnessStatus::Fresh,
            ),
            row(
                "comet",
                "applovin_max",
                "acct_comet_max_01",
                Some("2026-10-14"),
                Some(1),
                FreshnessStatus::Fresh,
            ),
            row(
                "comet",
                "applovin_max",
                "acct_comet_max_02",
                Some("2026-10-14"),
                Some(1),
                FreshnessStatus::Fresh,
            ),
            row(
                "orbit",
                "levelplay",
                "acct_orbit_lp_01",
                Some("2026-10-14"),
                Some(1),
                FreshnessStatus::Fresh,
            ),
            row(
                "atlas",
                "levelplay",
                "acct_atlas_lp_01",
                Some("2026-10-12"),
                Some(3),
                FreshnessStatus::Stale,
            ),
            row(
                "atlas",
                "admob",
                "acct_atlas_admob_01",
                None,
                None,
                FreshnessStatus::MissingData,
            ),
            row(
                "comet",
                "admob",
                "acct_comet_admob_01",
                Some("2026-10-14"),
                Some(1),
                FreshnessStatus::Fresh,
            ),
        ]
    }

    /// The coverage matrix rows for page 59.
    pub fn coverage_rows(&self) -> Vec<CoverageRow> {
        let days: Vec<String> = (1..=14).map(|day| format!("2026-10-{day:02}")).collect();
        let verdicts = |final_through: usize, provisional: usize, missing: usize| -> Vec<String> {
            let mut out = Vec::new();
            for index in 0..14 {
                if index < final_through {
                    out.push("Final".to_string());
                } else if index < final_through + provisional {
                    out.push("Provisional".to_string());
                } else if index < final_through + provisional + missing {
                    out.push("Missing".to_string());
                } else {
                    out.push("Not expected".to_string());
                }
            }
            out
        };
        vec![
            CoverageRow {
                publisher_id: "northstar".to_string(),
                platform: "applovin_max".to_string(),
                platform_account_id: "acct_northstar_max_01".to_string(),
                expected_days: days.clone(),
                verdicts: verdicts(11, 0, 3),
                last_final_day: Some("2026-10-11".to_string()),
                last_pull: Some("15 Oct 2026, 09:09 UTC".to_string()),
                last_final_display: Some("11 Oct 2026".to_string()),
            },
            CoverageRow {
                publisher_id: "northstar".to_string(),
                platform: "applovin_max".to_string(),
                platform_account_id: "acct_northstar_max_02".to_string(),
                expected_days: days.clone(),
                verdicts: verdicts(14, 0, 0),
                last_final_day: Some("2026-10-14".to_string()),
                last_pull: Some("15 Oct 2026, 09:09 UTC".to_string()),
                last_final_display: Some("14 Oct 2026".to_string()),
            },
            CoverageRow {
                publisher_id: "orbit".to_string(),
                platform: "levelplay".to_string(),
                platform_account_id: "acct_orbit_lp_01".to_string(),
                expected_days: days.clone(),
                verdicts: verdicts(14, 0, 0),
                last_final_day: Some("2026-10-14".to_string()),
                last_pull: Some("15 Oct 2026, 08:40 UTC".to_string()),
                last_final_display: Some("14 Oct 2026".to_string()),
            },
            CoverageRow {
                publisher_id: "atlas".to_string(),
                platform: "admob".to_string(),
                platform_account_id: "acct_atlas_admob_01".to_string(),
                expected_days: days,
                verdicts: verdicts(0, 0, 14),
                last_final_day: None,
                last_pull: Some("14 Oct 2026, 23:12 UTC".to_string()),
                last_final_display: None,
            },
        ]
    }

    /// The data-quality badge behind the summaries.
    pub fn data_quality(&self) -> DataQuality {
        DataQuality {
            publisher_id: String::new(),
            as_of: self.as_of.to_rfc3339_millis(),
            matched_source_days: 58,
            missing_days: Some(3),
            provisional_days: 2,
            dedup_rejects: Some(1),
            unmapped_rows: 1,
            stale_sources: vec![
                StaleSource {
                    platform: "applovin_max".to_string(),
                    platform_account_id: "acct_northstar_max_01".to_string(),
                    last_final_day: Some("2026-10-11".to_string()),
                    freshness_days: Some(4),
                },
                StaleSource {
                    platform: "levelplay".to_string(),
                    platform_account_id: "acct_atlas_lp_01".to_string(),
                    last_final_day: Some("2026-10-12".to_string()),
                    freshness_days: Some(3),
                },
            ],
            unavailable: Vec::new(),
        }
    }

    /// Growth cohorts: August (mature) and September (still maturing).
    pub fn growth_cohorts(&self) -> Vec<GrowthCohortRow> {
        vec![
            GrowthCohortRow {
                cohort_month: "2026-08".to_string(),
                accepted: 52,
                accepted_accounts: 48,
                qualified: 35,
                connected: 24,
                audit_delivered: 21,
                paid: 8,
                mature: true,
                qualification_rate_bp: Some(7_291),
            },
            GrowthCohortRow {
                cohort_month: "2026-09".to_string(),
                accepted: 30,
                accepted_accounts: 28,
                qualified: 18,
                connected: 12,
                audit_delivered: 10,
                paid: 3,
                mature: false,
                qualification_rate_bp: Some(6_428),
            },
        ]
    }

    /// The funnel of one cohort.
    pub fn activation_funnel(
        &self,
        cohort_month: &str,
    ) -> Result<ActivationFunnel, crate::data::Unavailable> {
        let cohort = self
            .growth_cohorts()
            .into_iter()
            .find(|row| row.cohort_month == cohort_month)
            .ok_or_else(|| {
                crate::data::Unavailable::new(format!(
                    "no growth cohort {cohort_month} is present in this dataset"
                ))
            })?;
        Ok(ActivationFunnel {
            cohort_month: cohort.cohort_month.clone(),
            mature: cohort.mature,
            stages: vec![
                FunnelStageRow {
                    stage: "requested".to_string(),
                    accounts: cohort.accepted,
                },
                FunnelStageRow {
                    stage: "qualified".to_string(),
                    accounts: cohort.qualified,
                },
                FunnelStageRow {
                    stage: "connected".to_string(),
                    accounts: cohort.connected,
                },
                FunnelStageRow {
                    stage: "audit_delivered".to_string(),
                    accounts: cohort.audit_delivered,
                },
                FunnelStageRow {
                    stage: "paid".to_string(),
                    accounts: cohort.paid,
                },
            ],
        })
    }

    /// The experiment fleet landscape.
    pub fn experiment_landscape(&self) -> Vec<ExperimentLandscapeRow> {
        vec![
            ExperimentLandscapeRow {
                state: "running".to_string(),
                count: 18,
                oldest_waiting_reason: None,
                oldest_age_days: Some(4),
            },
            ExperimentLandscapeRow {
                state: "awaiting_data".to_string(),
                count: 5,
                oldest_waiting_reason: Some("finalized source days are missing".to_string()),
                oldest_age_days: Some(2),
            },
            ExperimentLandscapeRow {
                state: "decisions_due".to_string(),
                count: 3,
                oldest_waiting_reason: Some("registered window complete".to_string()),
                oldest_age_days: Some(1),
            },
            ExperimentLandscapeRow {
                state: "restore_failed".to_string(),
                count: 1,
                oldest_waiting_reason: Some("platform stop refused".to_string()),
                oldest_age_days: Some(0),
            },
        ]
    }

    /// One experiment detail row.
    pub fn experiment_detail(&self, experiment_id: &str) -> Option<DurableExperiment> {
        self.durable_experiments()
            .into_iter()
            .find(|experiment| experiment.experiment_id == experiment_id)
    }

    /// The effectiveness of one mature start cohort (page 67).
    pub fn experiment_effectiveness(
        &self,
        cohort_month: &str,
    ) -> Result<ExperimentEffectiveness, crate::data::Unavailable> {
        if cohort_month != "2026-08" {
            return Err(crate::data::Unavailable::new(format!(
                "no settled experiment start cohort {cohort_month} is present in this dataset"
            )));
        }
        Ok(ExperimentEffectiveness {
            cohort_label: "2026-08".to_string(),
            mature: true,
            started: 40,
            valid_settled: 30,
            kept: 18,
            rolled_back: 12,
            voided: 6,
            pending: 4,
            unknown: 0,
            in_flight: 0,
            keep_rate_valid_bp: Some(6_000),
            keep_rate_started_bp: Some(4_500),
            completion_p50_days: Some(21),
            completion_p95_days: Some(28),
            excluded_from_denominators: vec![
                "voided experiments are shown separately".to_string(),
                "pending experiments are not counted as successes".to_string(),
            ],
        })
    }

    /// Decision lift rows.
    pub fn lift_rows(&self) -> Vec<LiftRow> {
        vec![
            LiftRow {
                experiment_id: "exp_0246".to_string(),
                change_ref: Some("ch_0246".to_string()),
                observed_lift_bp: Some(540),
                billable_bound_bp: Some(300),
                outcome: "keep".to_string(),
                measurement_source: Some("revenue_reporting".to_string()),
                measurement_version: Some(format!("sha256:{}", sha256_hex(b"exp_0246-rows"))),
                decided_at: Some("2026-10-14T15:00:00.000Z".to_string()),
            },
            LiftRow {
                experiment_id: "exp_0248".to_string(),
                change_ref: Some("ch_0248".to_string()),
                observed_lift_bp: Some(40),
                billable_bound_bp: None,
                outcome: "rollback".to_string(),
                measurement_source: Some("revenue_reporting".to_string()),
                measurement_version: Some(format!("sha256:{}", sha256_hex(b"exp_0248-rows"))),
                decided_at: Some("2026-10-13T11:30:00.000Z".to_string()),
            },
        ]
    }

    /// Fee rows: eligible, invoiced, paid and two held exceptions.
    pub fn fee_rows(&self) -> Vec<FeeRow> {
        let money = |micros: i64| Some(Money::new(micros, "USD"));
        vec![
            FeeRow {
                change_id: "ch_0248".to_string(),
                period: "2026-10".to_string(),
                state: FeeState::Estimated,
                amount: money(8_400_000_000),
                source_amount: money(51_500_000_000),
                bound_bp: Some(300),
                rate_bp: Some(2_000),
                active_days: Some(30),
                invoice_ref: None,
                proof_refs: vec![format!("sha256:{}", sha256_hex(b"proof-ch_0248"))],
            },
            FeeRow {
                change_id: "ch_0247".to_string(),
                period: "2026-10".to_string(),
                state: FeeState::Invoiced,
                amount: money(7_900_000_000),
                source_amount: money(48_000_000_000),
                bound_bp: Some(300),
                rate_bp: Some(2_000),
                active_days: Some(28),
                invoice_ref: Some("INV-2026-10-northstar".to_string()),
                proof_refs: vec![format!("sha256:{}", sha256_hex(b"proof-ch_0247"))],
            },
            FeeRow {
                change_id: "ch_0246".to_string(),
                period: "2026-10".to_string(),
                state: FeeState::Paid,
                amount: money(6_800_000_000),
                source_amount: money(41_000_000_000),
                bound_bp: Some(300),
                rate_bp: Some(2_000),
                active_days: Some(26),
                invoice_ref: Some("INV-2026-09-orbit".to_string()),
                proof_refs: vec![format!("sha256:{}", sha256_hex(b"proof-ch_0246"))],
            },
            FeeRow {
                change_id: "ch_0245".to_string(),
                period: "2026-10".to_string(),
                state: FeeState::Held,
                amount: money(300_000_000),
                source_amount: Some(Money::new(10_000_000_000, "USD")),
                bound_bp: Some(300),
                rate_bp: Some(2_000),
                active_days: Some(12),
                invoice_ref: None,
                proof_refs: vec![format!("sha256:{}", sha256_hex(b"proof-ch_0245"))],
            },
            FeeRow {
                change_id: "ch_0244".to_string(),
                period: "2026-10".to_string(),
                state: FeeState::Held,
                amount: money(200_000_000),
                source_amount: Some(Money::new(7_000_000_000, "USD")),
                bound_bp: Some(300),
                rate_bp: Some(2_000),
                active_days: Some(9),
                invoice_ref: None,
                proof_refs: vec![format!("sha256:{}", sha256_hex(b"proof-ch_0244"))],
            },
        ]
    }

    /// The reproducible fee calculation of `ch_0246`: USD 51,500.00 at a
    /// 3 percent bound and the agreed 20 percent rate gives USD 300.00.
    pub fn fee_detail(&self, change_id: &str) -> Option<FeeDetail> {
        if change_id != "ch_0246" {
            return None;
        }
        Some(FeeDetail {
            change_id: "ch_0246".to_string(),
            period: "2026-10".to_string(),
            eligible_revenue: Money::new(51_500_000_000, "USD"),
            conservative_bound_bp: 300,
            agreed_rate_bp: 2_000,
            attributed_increment: Some(Money::new(1_500_000_000, "USD")),
            fee_before_tax: Some(Money::new(300_000_000, "USD")),
            active_days: Some(30),
            proof_refs: vec![
                format!("sha256:{}", sha256_hex(b"proof-ch_0246")),
                format!("sha256:{}", sha256_hex(b"decision-exp_0246")),
            ],
            calculated_at: Some("2026-10-14T15:05:00.000Z".to_string()),
            unavailable: Vec::new(),
        })
    }

    /// One object timeline, merged by occurrence time.
    pub fn object_timeline(&self, object_type: &str, object_id: &str) -> Vec<ObjectTimelineEntry> {
        if object_type != "incident" || object_id != "inc_018" {
            return Vec::new();
        }
        let entry = |minutes: i64,
                     event_type: EventType,
                     outcome: Option<&str>,
                     reason_code: Option<&str>,
                     source: EventSource| ObjectTimelineEntry {
            occurred_at_utc: UtcMillis::from_unix_millis(self.minutes_ago(minutes))
                .to_rfc3339_millis(),
            received_at_utc: UtcMillis::from_unix_millis(self.minutes_ago(minutes) + 4_000)
                .to_rfc3339_millis(),
            event_type,
            source,
            outcome: outcome.map(str::to_string),
            reason_code: reason_code.map(str::to_string),
            correlation_id: Some("corr-inc018".to_string()),
            causation_id: None,
        };
        vec![
            entry(
                29,
                EventType::IncidentChanged,
                Some("critical"),
                Some("stop_refused"),
                EventSource::Worker,
            ),
            entry(
                28,
                EventType::GuardrailRestoreFailed,
                Some("failed"),
                Some("platform_stop_refused"),
                EventSource::Connector,
            ),
            entry(
                30,
                EventType::GuardrailFired,
                Some("warning"),
                Some("fill_drop"),
                EventSource::Worker,
            ),
            entry(
                31,
                EventType::GuardrailRestoreRequested,
                Some("requested"),
                None,
                EventSource::Worker,
            ),
            entry(
                51,
                EventType::IncidentChanged,
                Some("critical"),
                Some("opened"),
                EventSource::Worker,
            ),
        ]
    }

    /// The incident case for the detail page.
    pub fn incident_case(&self, incident_id: &str) -> Option<IncidentCase> {
        let queues = self.queues();
        let row = queues
            .incidents
            .iter()
            .find(|row| row.object_id == incident_id)?;
        let (headline, experiment, scope, last_state, failed_operation) = match incident_id {
            "inc_018" => (
                "Stop request was refused by MAX.",
                Some("exp_0247"),
                Some("rw_de_01"),
                Some("Live floor 4.00 enabled; test arm still serving the old floor"),
                Some("ad_unit_experiment / deprecate (stop)"),
            ),
            "inc_019" => (
                "Ledger verification failed for one history.",
                Some("exp_0246"),
                Some("rw_de_01"),
                Some("Last verified sequence 17 of 24"),
                Some("ledger verification (read)"),
            ),
            "inc_017" => (
                "Reporting key for Atlas Games expired.",
                None,
                Some("admob management access"),
                Some("Reads refused with auth failure since 07:10 UTC"),
                Some("reporting pull (read)"),
            ),
            _ => ("Incident is open.", None, None, None, None),
        };
        Some(IncidentCase {
            id: incident_id.to_string(),
            severity: row.severity.clone(),
            age_seconds: row.age_seconds,
            publisher_id: row.publisher_id.clone(),
            owner: row.owner.clone(),
            headline: headline.to_string(),
            experiment: experiment.map(str::to_string),
            scope: scope.map(str::to_string),
            last_confirmed_state: last_state.map(str::to_string),
            failed_operation: failed_operation.map(str::to_string),
            occurred_display: Some(
                apex_ui::format::utc_datetime(DEMO_NOW_MS - (row.age_seconds as i64) * 1_000),
            ),
            next_update: Some("09:20 UTC".to_string()),
            communication_draft: format!(
                "The platform did not confirm the stop request for experiment {}. We are checking the current state. The test is not marked restored. Your incident reference is {}.",
                experiment.unwrap_or("the test"),
                incident_id
            ),
            affects_traffic: Some(false),
            timeline: self.object_timeline("incident", incident_id),
        })
    }

    /// Durable experiment records for the fleet and detail pages.
    pub fn durable_experiments(&self) -> Vec<DurableExperiment> {
        vec![
            DurableExperiment {
                publisher: "northstar".to_string(),
                experiment_id: "exp_0247".to_string(),
                name: Some("floor-test-de".to_string()),
                metric: Some("ecpm".to_string()),
                change_description: Some("raise the DE floor".to_string()),
                started_at_ms: Some(self.minutes_ago(20_160)),
                state: Some("completed".to_string()),
                lift_bp: Some(300),
                confidence: Some(9_800),
                observed_at_ms: Some(self.minutes_ago(1_440)),
            },
            DurableExperiment {
                publisher: "orbit".to_string(),
                experiment_id: "exp_0246".to_string(),
                name: Some("de-floor-raise".to_string()),
                metric: Some("ecpm".to_string()),
                change_description: Some("raise the DE floor".to_string()),
                started_at_ms: Some(self.minutes_ago(28_800)),
                state: Some("completed".to_string()),
                lift_bp: Some(540),
                confidence: Some(9_500),
                observed_at_ms: Some(self.minutes_ago(2_880)),
            },
            DurableExperiment {
                publisher: "comet".to_string(),
                experiment_id: "exp_0250".to_string(),
                name: Some("int-us-floor".to_string()),
                metric: Some("revenue".to_string()),
                change_description: Some("raise the US interstitial floor".to_string()),
                started_at_ms: Some(self.minutes_ago(8_640)),
                state: Some("running".to_string()),
                lift_bp: None,
                confidence: None,
                observed_at_ms: Some(self.minutes_ago(120)),
            },
            DurableExperiment {
                publisher: "orbit".to_string(),
                experiment_id: "exp_0248".to_string(),
                name: Some("hold-floor-de".to_string()),
                metric: Some("revenue".to_string()),
                change_description: Some("hold the DE floor".to_string()),
                started_at_ms: Some(self.minutes_ago(43_200)),
                state: Some("stopped".to_string()),
                lift_bp: Some(40),
                confidence: Some(6_100),
                observed_at_ms: Some(self.minutes_ago(4_320)),
            },
        ]
    }

    /// The invoice register for the demo publisher.
    pub fn invoices(&self) -> Vec<InvoiceRecord> {
        vec![
            InvoiceRecord {
                publisher_id: "northstar".to_string(),
                period: "2026-10".to_string(),
                start_unix_ms: 1_790_928_000_000,
                end_unix_ms: 1_793_606_400_000,
                line_items: vec![InvoiceLineRecord {
                    seq: 11,
                    experiment_id: "exp_0247".to_string(),
                    proven_lift: 10_000_000_000,
                    rate_bp: 2_000,
                    fee_micros: 300_000_000,
                    vat_micros: 60_000_000,
                    gross_micros: 360_000_000,
                }],
                total_micros: 300_000_000,
                vat_rate_bp: 2_000,
                vat_micros: 60_000_000,
                gross_micros: 360_000_000,
            },
            InvoiceRecord {
                publisher_id: "orbit".to_string(),
                period: "2026-09".to_string(),
                start_unix_ms: 1_788_336_000_000,
                end_unix_ms: 1_790_928_000_000,
                line_items: vec![InvoiceLineRecord {
                    seq: 9,
                    experiment_id: "exp_0246".to_string(),
                    proven_lift: 5_500_000_000,
                    rate_bp: 2_000,
                    fee_micros: 225_000_000,
                    vat_micros: 45_000_000,
                    gross_micros: 270_000_000,
                }],
                total_micros: 225_000_000,
                vat_rate_bp: 2_000,
                vat_micros: 45_000_000,
                gross_micros: 270_000_000,
            },
        ]
    }

    /// The payment timeline events behind the demo invoices.
    pub fn payments(&self) -> Vec<PaymentEvent> {
        vec![PaymentEvent {
            invoice_id: "INV-2026-10-northstar".to_string(),
            event_code: "AUTHORISATION".to_string(),
            success: true,
            psp_reference_masked: "8836178412****".to_string(),
            amount: Money::new(360_000_000, "USD"),
            at_ms: self.minutes_ago(1_100),
            received: true,
            hmac_verified: true,
            deduplicated: true,
            reconciled: true,
            allocated: true,
        }]
    }

    /// The customer inventory of the illustrations.
    pub fn customers(&self) -> Vec<CustomerRow> {
        vec![
            CustomerRow {
                account_id: "acct_northstar".to_string(),
                publisher_id: "northstar".to_string(),
                display_name: "Northstar Studio".to_string(),
                lifecycle: "active_tests".to_string(),
                platform_readiness: vec!["MAX: write verified".to_string()],
                health: "Restore failed".to_string(),
                owner: Some("Maya".to_string()),
                stage_age_days: 4,
                last_activity: "Guardrail fired on exp_0247".to_string(),
                contact_masked: Some("n*****r@northstar.example".to_string()),
                running_tests: 2,
                billing_state: "USD 300.00 held".to_string(),
            },
            CustomerRow {
                account_id: "acct_orbit".to_string(),
                publisher_id: "orbit".to_string(),
                display_name: "Orbit Apps".to_string(),
                lifecycle: "paying".to_string(),
                platform_readiness: vec!["MAX: read connected".to_string()],
                health: "Proof hold".to_string(),
                owner: Some("Sam".to_string()),
                stage_age_days: 11,
                last_activity: "Invoice INV-2026-09-orbit paid".to_string(),
                contact_masked: Some("o***t@orbit.example".to_string()),
                running_tests: 1,
                billing_state: "USD 225.00 collected".to_string(),
            },
            CustomerRow {
                account_id: "acct_atlas".to_string(),
                publisher_id: "atlas".to_string(),
                display_name: "Atlas Games".to_string(),
                lifecycle: "audit_requested".to_string(),
                platform_readiness: vec!["AdMob: access missing".to_string()],
                health: "Waiting for access".to_string(),
                owner: Some("Lina".to_string()),
                stage_age_days: 6,
                last_activity: "Audit request accepted".to_string(),
                contact_masked: Some("a***s@atlas.example".to_string()),
                running_tests: 0,
                billing_state: "No fee lines".to_string(),
            },
            CustomerRow {
                account_id: "acct_comet".to_string(),
                publisher_id: "comet".to_string(),
                display_name: "Comet Mobile".to_string(),
                lifecycle: "audit_delivered".to_string(),
                platform_readiness: vec!["LevelPlay: read connected".to_string()],
                health: "Ready for review".to_string(),
                owner: Some("Maya".to_string()),
                stage_age_days: 2,
                last_activity: "First audit delivered".to_string(),
                contact_masked: Some("c***t@comet.example".to_string()),
                running_tests: 1,
                billing_state: "No fee lines".to_string(),
            },
        ]
    }

    /// The audit quality snapshot.
    pub fn audit_quality(&self) -> AuditQuality {
        AuditQuality {
            successful_runs: "138 / 140".to_string(),
            eligible_checks: "612 / 700".to_string(),
            ineligible_note: "88 lack required inputs".to_string(),
            critical_findings: 7,
            critical_customers: 5,
            rules: vec![
                AuditRuleRow {
                    rule: "Stale floor".to_string(),
                    eligible_runs: "118 / 140".to_string(),
                    findings: 28,
                    review_signal: "Model changed in v0.1.1".to_string(),
                },
                AuditRuleRow {
                    rule: "Reporting gap".to_string(),
                    eligible_runs: "93 / 140".to_string(),
                    findings: 7,
                    review_signal: "47 lack paired reports".to_string(),
                },
                AuditRuleRow {
                    rule: "app-ads.txt".to_string(),
                    eligible_runs: "140 / 140".to_string(),
                    findings: 16,
                    review_signal: "3 publisher dismissals".to_string(),
                },
            ],
        }
    }

    /// The monthly service economics series.
    pub fn economics(&self) -> Vec<EconomicsMonth> {
        (1..=9i64)
            .map(|index| EconomicsMonth {
                month: format!("2026-{index:02}"),
                lift_fee_revenue_micros: 6_300_000_000 + (index - 1) * 300_000_000,
                monitoring_revenue_micros: 4_000_000_000,
                allocated_cost_micros: Some(2_400_000_000),
                unpaid_micros: if index % 3 == 0 { 225_000_000 } else { 0 },
                support_minutes: 120 + index * 6,
            })
            .collect()
    }

    /// The release-health rows: the control plane reports its own facts; the
    /// sibling services are not instrumented for this read.
    pub fn release_health(&self) -> Vec<ReleaseHealthRow> {
        vec![
            ReleaseHealthRow {
                service: "apex-control".to_string(),
                version: format!(
                    "{} (build {})",
                    env!("CARGO_PKG_VERSION"),
                    crate::state::BUILD_LABEL
                ),
                migration_state: "No migrations (in-memory sessions, file journal)".to_string(),
                report_schema: "v1 domain event envelope".to_string(),
                metric_schema: "apex-metrics bounded registry (APEX_METRICS_TEXTFILE export)"
                    .to_string(),
                data_quality: "See the audits and ingestion pages".to_string(),
                recent_errors: "None recorded in the journal".to_string(),
            },
            ReleaseHealthRow {
                service: "apex-api".to_string(),
                version: "Not instrumented".to_string(),
                migration_state: "Not instrumented".to_string(),
                report_schema: "Not instrumented".to_string(),
                metric_schema: "Not instrumented".to_string(),
                data_quality: "Not instrumented".to_string(),
                recent_errors: "Not instrumented".to_string(),
            },
            ReleaseHealthRow {
                service: "apex-runner".to_string(),
                version: "Not instrumented".to_string(),
                migration_state: "Not instrumented".to_string(),
                report_schema: "Not instrumented".to_string(),
                metric_schema: "Not instrumented".to_string(),
                data_quality: "Not instrumented".to_string(),
                recent_errors: "Not instrumented".to_string(),
            },
        ]
    }

    /// The system component rows.
    pub fn system_components(&self) -> Vec<SystemComponentRow> {
        vec![
            SystemComponentRow {
                component: "Daily runner".to_string(),
                last_success: "15 Oct 2026, 08:40 UTC".to_string(),
                signal: "1 overdue publisher".to_string(),
                next_action: "Open job".to_string(),
            },
            SystemComponentRow {
                component: "Platform reads".to_string(),
                last_success: "15 Oct 2026, 09:09 UTC".to_string(),
                signal: "2 auth failures".to_string(),
                next_action: "Open ingestion".to_string(),
            },
            SystemComponentRow {
                component: "Ledger backup".to_string(),
                last_success: "15 Oct 2026, 02:00 UTC".to_string(),
                signal: "Restore test not recorded".to_string(),
                next_action: "Review backup".to_string(),
            },
        ]
    }

    /// The verified-operation evidence of the illustration: one action has
    /// complete evidence, the rest is on file partially or not at all.
    pub fn operations_evidence(&self) -> OperationsEvidence {
        OperationsEvidence::from_entries(vec![OperationEvidence {
            platform: "applovin_max".to_string(),
            action: "updateFloors".to_string(),
            sandbox_evidence: false,
            allowed_deployment: Some("staging only".to_string()),
            credential_scope: Some("publisher management key".to_string()),
            approval_required: true,
            release_version: Some("0.9.0".to_string()),
        }])
    }
}
