//! The fixture renderer: every page and every state of the control plane,
//! rendered from the illustrative dataset into one directory.
//!
//! The binary `render-fixtures` calls [`render_all`], and the integration
//! tests sweep the output: each file carries exactly one `<h1>`, contains no
//! `<script>`, and no customer credential or session handle appears. The
//! sweep names are stable so the `console-a11y` gate sees at least the
//! design document's page count.

use std::path::{Path, PathBuf};

use apex_analytics::queries::{
    ExperimentLandscapeRow, FeeRow, FeeState, FreshnessStatus, IngestionHealthRow, OperatorQueues,
};

use crate::actions::ActionState;
use crate::capabilities::{Capability, Role};
use crate::config::{ControlConfig, OperatorConfig, Operators};
use crate::data::ControlData;
use crate::fixtures::{AuditQuality, DemoData, PaymentEvent};
use crate::journal::{ActionJournal, ActionJournalEntry};
use crate::pages::{self, ActionFormSpec, PageMeta};
use crate::session::SessionView;
use crate::signing::{idempotency_fingerprint, random_hex_32, sha256_hex};
use crate::state::{ControlState, FixedClock, StateError};

/// The password every illustrative operator shares (documented, non-secret:
/// the fixture dataset is not a deployment).
pub const FIXTURE_PASSWORD: &str = "fixture-password";
/// The TOTP secret every illustrative operator shares (the RFC 6238 SHA-1
/// test secret).
pub const FIXTURE_TOTP_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

/// Build the illustrative operator directory (one per role).
pub fn fixture_operators() -> Result<Operators, StateError> {
    let phc = crate::passwords::hash_with_salt(FIXTURE_PASSWORD, b"apex-control-fixture")
        .map_err(|error| StateError::Journal(format!("fixture hash: {error}")))?;
    let entry = |id: &str, name: &str, role: Role| OperatorConfig {
        id: id.to_string(),
        name: name.to_string(),
        role: role.as_str().to_string(),
        password_argon2id_phc: phc.clone(),
        totp_secret_base32: Some(FIXTURE_TOTP_SECRET.to_string()),
    };
    Operators::from_entries(vec![
        entry("maya", "Maya", Role::Operator),
        entry("sam", "Sam", Role::Finance),
        entry("lina", "Lina", Role::Support),
        entry("ravi", "Ravi", Role::Observer),
        entry("ada", "Ada", Role::Security),
    ])
    .map_err(StateError::Config)
}

/// The fixture working directory: a fresh path under the system temp dir, so
/// the fixture renderer never touches the repository's `.apex` state.
pub fn fixture_work_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "apex-control-fixtures-{}-{}",
        std::process::id(),
        random_hex_32()
    ))
}

/// Build the full fixture state: illustrative data, pinned clock, demo
/// operators, a journal with representative outcomes and a governance
/// register with one pending draft and one pause.
pub fn fixture_state() -> Result<ControlState, StateError> {
    let work = fixture_work_dir();
    std::fs::create_dir_all(&work)
        .map_err(|error| StateError::Journal(format!("fixture dir: {error}")))?;
    let config = ControlConfig {
        bind: "127.0.0.1:0"
            .parse()
            .map_err(|error| StateError::Journal(format!("fixture bind: {error}")))?,
        operators_path: None,
        journal_path: work.join("journal.jsonl"),
        jobs_dir: work.join("jobs"),
        jobs_remote: None,
        sync_marker: None,
        environment: "PRODUCTION".to_string(),
        ledger_root: work.join("ledgers"),
        ledger_keys: std::collections::HashMap::new(),
        snapshot_root: work.join("snapshots"),
        notify_sink: None,
        metrics_textfile: None,
        invoice_register: work.join("invoices.jsonl"),
        totp_state_path: work.join("totp-steps.json"),
        operations_path: work.join("operations.json"),
        policies_path: work.join("policies.jsonl"),
        interventions_path: work.join("interventions.jsonl"),
        effective_policy_path: work.join("effective-policy.json"),
        connections_root: work.join("connections"),
        clickhouse: None,
    };
    // The journal of the illustration: one permitted request, one denial,
    // one step-up and one revocation, so `/control/access` shows every row
    // shape (including the denied row).
    let journal = ActionJournal::new(config.journal_path.clone());
    for entry in fixture_journal_entries() {
        journal
            .append(&entry)
            .map_err(|error| StateError::Journal(error.to_string()))?;
    }
    let governance = crate::governance::GovernanceStore::new(
        config.policies_path.clone(),
        config.interventions_path.clone(),
    );
    governance
        .append_policy(&crate::governance::PolicyRecord::draft(
            "maya",
            crate::fixtures::DEMO_NOW_MS - 3_600_000,
            serde_json::json!({"max_step_bp": 1_000}),
            serde_json::json!({"window_days": 14}),
            serde_json::json!({"fill_drop_frac": 0.1}),
            "future proposals drafted after activation; active experiments keep their registered policy",
            1,
            "corr-fixture-policy",
        ))
        .map_err(|error| StateError::Journal(error.to_string()))?;
    governance
        .append_intervention(&crate::governance::InterventionRecord::new(
            "pause_scheduling",
            Some("applovin_max"),
            "all publishers on MAX",
            "platform incident 2026-10-15",
            "no new experiments start; reads continue",
            "maya",
            crate::fixtures::DEMO_NOW_MS - 1_800_000,
            "corr-fixture-intervention",
        ))
        .map_err(|error| StateError::Journal(error.to_string()))?;
    let state = ControlState::new(config, ControlData::fixtures())?
        .with_clock(std::sync::Arc::new(FixedClock::new(
            crate::fixtures::DEMO_NOW_MS,
        )))
        .with_operators(fixture_operators()?);
    Ok(state)
}

/// A session for the illustrative operator, with every operator's sessions
/// present so the access page shows a realistic list.
pub fn fixture_session(state: &ControlState) -> Result<SessionView, String> {
    let now = state.now_ms();
    let mut maya_view: Option<SessionView> = None;
    for (operator_id, name, role, device) in [
        ("maya", "Maya", Role::Operator, "Chrome on macOS"),
        ("sam", "Sam", Role::Finance, "Firefox on Windows"),
        ("ada", "Ada", Role::Security, "Safari on iOS"),
    ] {
        let opened = state
            .sessions
            .open(operator_id, name, role, device, now - 600_000);
        let _ = state.sessions.touch(&opened.id, now - 300_000);
        if operator_id == "maya" {
            maya_view = state.sessions.touch(&opened.id, now);
        }
    }
    maya_view.ok_or_else(|| "the maya fixture session could not be opened".to_string())
}

fn fixture_journal_entries() -> Vec<ActionJournalEntry> {
    let now = crate::fixtures::DEMO_NOW_MS;
    let entry = |at_ms: i64,
                 actor: &str,
                 role: &str,
                 capability: &str,
                 class: &str,
                 target: &str,
                 outcome: &str,
                 reason: &str,
                 key: &str| ActionJournalEntry {
        at_ms,
        actor: actor.to_string(),
        role: role.to_string(),
        capability: capability.to_string(),
        capability_class: class.to_string(),
        target: target.to_string(),
        outcome: outcome.to_string(),
        reason: reason.to_string(),
        before_digest: sha256_hex(b"before"),
        after_digest: sha256_hex(b"after"),
        idempotency_key: idempotency_fingerprint(key),
        payload_fingerprint: Some(crate::signing::payload_fingerprint(
            capability,
            target,
            &serde_json::json!({}),
        )),
        result_id: Some(format!("res_{}", &random_hex_32()[..12])),
        job_id: None,
        correlation_id: format!("corr-{}", &random_hex_32()[..8]),
    };
    vec![
        entry(
            now - 4_800_000,
            "lina",
            "support",
            "incident_acknowledge",
            "incident",
            "inc_017",
            "confirmed",
            "acknowledged: working with Atlas on the key rotation",
            "fixture-key-1",
        ),
        entry(
            now - 3_600_000,
            "ravi",
            "observer",
            "incident_resolve",
            "incident",
            "inc_018",
            "denied",
            "role does not hold the capability; nothing was requested",
            "fixture-key-2",
        ),
        entry(
            now - 1_800_000,
            "maya",
            "operator",
            "incident_resolve",
            "incident",
            "inc_018",
            "step_up_required",
            "re-authentication required before the action runs",
            "fixture-key-3",
        ),
        entry(
            now - 60_000,
            "ada",
            "security",
            "session_revoke",
            "access",
            "session:deadbeef",
            "confirmed",
            "revoked a stale session after the device was retired",
            "fixture-key-4",
        ),
    ]
}

fn demo() -> DemoData {
    DemoData::new()
}

fn meta(state: &ControlState, session: &SessionView) -> PageMeta {
    PageMeta {
        environment: state.environment().to_string(),
        as_of_ms: state.now_ms(),
        operator: Some(session.operator_name.clone()),
        role: Some(session.role),
        fixtures: true,
        csrf: session.csrf.clone(),
    }
}

fn form_spec(
    state: &ControlState,
    session: &SessionView,
    capability: Capability,
    target: &str,
    return_path: &str,
    key: &str,
) -> ActionFormSpec {
    let token = state.versions.observe(
        &format!("fixture:{target}"),
        &serde_json::json!({"target": target}),
    );
    ActionFormSpec {
        action: format!("/control/actions/{}", capability.as_str()),
        label: capability.label().to_string(),
        target: target.to_string(),
        csrf: session.csrf.clone(),
        idempotency_key: key.to_string(),
        version_token: token.encode(),
        return_path: return_path.to_string(),
        reason_required: capability.requires_reason(),
        source_document: capability.requires_source_document(),
        evidence: capability == Capability::IncidentResolve,
        confirmation: capability.typed_confirmation().map(str::to_string),
        policy_json: matches!(capability, Capability::PolicySet | Capability::PolicyDraft),
        revenue_30d: capability == Capability::ManagedPlanActivate,
        tier: capability == Capability::ManagedPlanActivate,
        extra: Vec::new(),
        problem: None,
    }
}

/// Render every fixture page into `out_dir`. Returns the written file names.
pub fn render_all(out_dir: &Path) -> Result<Vec<String>, String> {
    let _ = std::fs::create_dir_all(out_dir);
    let state = fixture_state().map_err(|error| error.to_string())?;
    let session = fixture_session(&state)?;
    let demo = demo();
    let m = meta(&state, &session);
    let mut pages: Vec<(String, String)> = Vec::new();
    let mut push = |name: &str, html: String| pages.push((name.to_string(), html));

    // Sign-in family.
    push(
        "login-password",
        pages::login::render(
            &m,
            &pages::login::LoginState::Password {
                problem: None,
                preserved_operator: "maya".to_string(),
                preserved_next: Some("/control/incidents/inc_018".to_string()),
                reauth: false,
            },
        ),
    );
    push(
        "login-password-error",
        pages::login::render(
            &m,
            &pages::login::LoginState::Password {
                problem: Some(crate::copy::LOGIN_FAILED),
                preserved_operator: "maya".to_string(),
                preserved_next: Some("/control/".to_string()),
                reauth: false,
            },
        ),
    );
    push(
        "login-code",
        pages::login::render(
            &m,
            &pages::login::LoginState::Code {
                pending_token: random_hex_32(),
                operator_name: "Maya".to_string(),
                problem: None,
            },
        ),
    );
    push(
        "login-code-error",
        pages::login::render(
            &m,
            &pages::login::LoginState::Code {
                pending_token: random_hex_32(),
                operator_name: "Maya".to_string(),
                problem: Some(crate::copy::LOGIN_FAILED),
            },
        ),
    );
    push(
        "login-rate-limited",
        pages::login::render(
            &m,
            &pages::login::LoginState::RateLimited {
                until_ms: crate::fixtures::DEMO_NOW_MS + 600_000,
            },
        ),
    );
    push(
        "login-step-expired",
        pages::login::render(&m, &pages::login::LoginState::StepExpired),
    );
    push(
        "login-no-second-factor",
        pages::login::render(
            &m,
            &pages::login::LoginState::NoSecondFactor {
                operator_name: "Maya".to_string(),
            },
        ),
    );
    push(
        "login-unconfigured",
        pages::login::unconfigured(&m, "APEX_CONTROL_OPERATORS is not set"),
    );
    push("login-session-expired", pages::login::session_expired(&m));

    // Overview.
    let queues = demo.queues();
    push(
        "overview",
        pages::overview::render(
            &m,
            &pages::overview::OverviewData {
                queues: Ok(queues.clone()),
                fee_rows: Ok(demo.fee_rows()),
            },
        ),
    );
    push(
        "overview-empty",
        pages::overview::render(
            &m,
            &pages::overview::OverviewData {
                queues: Ok(OperatorQueues {
                    as_of: queues.as_of.clone(),
                    incidents: Vec::new(),
                    due_decisions: Vec::new(),
                    stale_accounts: Vec::new(),
                    overdue_jobs: Vec::new(),
                    held_fees: Vec::new(),
                }),
                fee_rows: Ok(Vec::new()),
            },
        ),
    );
    push(
        "overview-unavailable",
        pages::overview::render(
            &m,
            &pages::overview::OverviewData {
                queues: Err(crate::data::NO_ANALYTICS.to_string()),
                fee_rows: Err(crate::data::NO_ANALYTICS.to_string()),
            },
        ),
    );

    // Customers.
    let customers = demo.customers();
    let detail = |tab: pages::customers::DetailTab, name: &str| -> (String, String) {
        let row = customers[0].clone();
        let timeline = Ok(demo.object_timeline("account", &row.account_id));
        let experiments = Ok(demo
            .durable_experiments()
            .into_iter()
            .filter(|experiment| experiment.publisher == row.publisher_id)
            .collect());
        (
            name.to_string(),
            pages::customers::render_detail(
                &m,
                &pages::customers::CustomerDetail {
                    customer: row.clone(),
                    timeline,
                    experiments,
                    verified_records: None,
                    invoices: None,
                    forms: vec![
                        form_spec(
                            &state,
                            &session,
                            Capability::RequestReconnection,
                            &row.publisher_id,
                            &format!("/control/customers/{}", row.account_id),
                            "fixture-reconnect",
                        ),
                        form_spec(
                            &state,
                            &session,
                            Capability::ManagedPlanActivate,
                            &row.publisher_id,
                            &format!("/control/customers/{}", row.account_id),
                            "fixture-managed",
                        ),
                    ],
                },
                tab,
                &pages::customers::CustomerFilters::default(),
            ),
        )
    };
    push(
        "customers",
        pages::customers::render_list(
            &m,
            Ok(customers.clone()),
            &pages::customers::CustomerFilters::default(),
        ),
    );
    push(
        "customers-filtered",
        pages::customers::render_list(
            &m,
            Ok(customers.clone()),
            &pages::customers::CustomerFilters {
                lifecycle: None,
                platform: None,
                health: Some("Restore failed".to_string()),
                owner: None,
                search: None,
            },
        ),
    );
    push(
        "customers-no-matches",
        pages::customers::render_list(
            &m,
            Ok(customers.clone()),
            &pages::customers::CustomerFilters {
                lifecycle: None,
                platform: None,
                health: None,
                owner: None,
                search: Some("zzz-nobody".to_string()),
            },
        ),
    );
    for (tab, name) in [
        (pages::customers::DetailTab::Summary, "customer-summary"),
        (
            pages::customers::DetailTab::Connections,
            "customer-connections",
        ),
        (pages::customers::DetailTab::Audits, "customer-audits"),
        (
            pages::customers::DetailTab::Experiments,
            "customer-experiments",
        ),
        (pages::customers::DetailTab::Finance, "customer-finance"),
        (pages::customers::DetailTab::Activity, "customer-activity"),
    ] {
        let rendered = detail(tab, name);
        push(&rendered.0, rendered.1);
    }

    // Ingestion.
    let health = demo.ingestion_health();
    let retry_forms: Vec<pages::ingestion::AccountRetryForm> = health
        .iter()
        .filter(|row| row.status != FreshnessStatus::Fresh)
        .map(|row| pages::ingestion::AccountRetryForm {
            spec: form_spec(
                &state,
                &session,
                Capability::RetryRead,
                &format!(
                    "{}:{}:{}",
                    row.publisher_id, row.platform, row.platform_account_id
                ),
                "/control/ingestion",
                "fixture-retry-key",
            ),
        })
        .collect();
    push(
        "ingestion",
        pages::ingestion::render(
            &m,
            &pages::ingestion::IngestionData {
                health: Ok(health),
                coverage: Ok(demo.coverage_rows()),
                live_coverage: None,
                retry_forms,
            },
        ),
    );
    push(
        "ingestion-delayed",
        pages::ingestion::render(
            &m,
            &pages::ingestion::IngestionData {
                health: Ok(vec![stale_row()]),
                coverage: Ok(Vec::new()),
                live_coverage: None,
                retry_forms: Vec::new(),
            },
        ),
    );

    // Audits.
    push(
        "audits",
        pages::audits::render(
            &m,
            &pages::audits::AuditsData {
                quality: Ok(demo.audit_quality()),
                live: None,
            },
        ),
    );
    push(
        "audits-no-inputs",
        pages::audits::render(
            &m,
            &pages::audits::AuditsData {
                live: None,
                quality: Ok(AuditQuality {
                    successful_runs: "0 / 0".to_string(),
                    eligible_checks: "0 / 0".to_string(),
                    ineligible_note: "all rules lack required inputs".to_string(),
                    critical_findings: 0,
                    critical_customers: 0,
                    rules: Vec::new(),
                }),
            },
        ),
    );

    // Experiments.
    push(
        "experiments",
        pages::experiments::render_fleet(
            &m,
            &pages::experiments::ExperimentsData {
                landscape: Ok(demo.experiment_landscape()),
                effectiveness: None,
                cohorts: vec!["2026-08".to_string(), "2026-09".to_string()],
                selected_cohort: None,
                decisions_due: true,
            },
        ),
    );
    push(
        "experiments-effectiveness",
        pages::experiments::render_fleet(
            &m,
            &pages::experiments::ExperimentsData {
                landscape: Ok(demo.experiment_landscape()),
                effectiveness: Some(
                    demo.experiment_effectiveness("2026-08")
                        .map_err(|error| error.reason),
                ),
                cohorts: vec!["2026-08".to_string(), "2026-09".to_string()],
                selected_cohort: Some("2026-08".to_string()),
                decisions_due: false,
            },
        ),
    );
    push(
        "experiments-awaiting",
        pages::experiments::render_fleet(
            &m,
            &pages::experiments::ExperimentsData {
                landscape: Ok(vec![ExperimentLandscapeRow {
                    state: "awaiting_data".to_string(),
                    count: 5,
                    oldest_waiting_reason: Some("finalized source days are missing".to_string()),
                    oldest_age_days: Some(2),
                }]),
                effectiveness: Some(
                    demo.experiment_effectiveness("2026-09")
                        .map_err(|error| error.reason),
                ),
                cohorts: vec!["2026-09".to_string()],
                selected_cohort: Some("2026-09".to_string()),
                decisions_due: false,
            },
        ),
    );
    push(
        "experiment-detail",
        pages::experiments::render_detail(
            &m,
            "exp_0246",
            &pages::experiments::ExperimentDetail {
                experiment: demo
                    .experiment_detail("exp_0246")
                    .ok_or_else(|| "missing".to_string()),
                lift: demo
                    .lift_rows()
                    .into_iter()
                    .find(|row| row.experiment_id == "exp_0246"),
                checks: Ok(pages::experiments::demo_checks()),
                forms: vec![
                    form_spec(
                        &state,
                        &session,
                        Capability::RestoreReconcileRequest,
                        "exp_0246",
                        "/control/experiments/exp_0246",
                        "fixture-reconcile-exp",
                    ),
                    form_spec(
                        &state,
                        &session,
                        Capability::RecheckRequest,
                        "exp_0246",
                        "/control/experiments/exp_0246",
                        "fixture-recheck-exp",
                    ),
                ],
            },
        ),
    );

    // Incidents.
    push(
        "incidents",
        pages::incidents::render_list(&m, Ok(demo.queues().incidents)),
    );
    push(
        "incidents-empty",
        pages::incidents::render_list(&m, Ok(Vec::new())),
    );
    let case = demo
        .incident_case("inc_018")
        .ok_or_else(|| "fixture incident missing".to_string())?;
    push(
        "incident-detail",
        pages::incidents::render_detail(
            &m,
            "inc_018",
            &pages::incidents::IncidentDetail {
                case: Ok(case.clone()),
                preview: Some(Ok(crate::fixtures::InterventionPreview {
                    publisher: case.publisher_id.clone(),
                    operation: case
                        .failed_operation
                        .clone()
                        .unwrap_or_else(|| "review".to_string()),
                    prior_state: case
                        .last_confirmed_state
                        .clone()
                        .unwrap_or_else(|| "not available".to_string()),
                    target_state: "restored state confirmed by a platform read".to_string(),
                    policy: "conservative defaults; actions stay unavailable without evidence"
                        .to_string(),
                    permissions: "the acting operator's role and recent sign-in".to_string(),
                    affects_traffic: false,
                })),
                forms: vec![
                    form_spec(
                        &state,
                        &session,
                        Capability::IncidentAcknowledge,
                        "inc_018",
                        "/control/incidents/inc_018",
                        "fixture-ack",
                    ),
                    form_spec(
                        &state,
                        &session,
                        Capability::OwnerAssign,
                        "inc_018",
                        "/control/incidents/inc_018",
                        "fixture-owner",
                    ),
                    form_spec(
                        &state,
                        &session,
                        Capability::IncidentResolve,
                        "inc_018",
                        "/control/incidents/inc_018",
                        "fixture-resolve",
                    ),
                ],
                step_up_note: false,
            },
        ),
    );
    push(
        "incident-detail-stepup",
        pages::incidents::render_detail(
            &m,
            "inc_018",
            &pages::incidents::IncidentDetail {
                case: Ok(case),
                preview: Some(Err(
                    "the platform state read for this action is not wired in this deployment"
                        .to_string(),
                )),
                forms: Vec::new(),
                step_up_note: true,
            },
        ),
    );

    // Finance.
    let fee_rows = demo.fee_rows();
    push(
        "finance",
        pages::finance::render(
            &m,
            &pages::finance::FinanceData {
                fee_rows: Ok(fee_rows.clone()),
                currency: None,
                analytics: None,
                live_economics: None,
            },
        ),
    );
    push(
        "finance-analytics",
        pages::finance::render(
            &m,
            &pages::finance::FinanceData {
                fee_rows: Ok(fee_rows.clone()),
                currency: None,
                analytics: Some(Ok(demo.economics())),
                live_economics: None,
            },
        ),
    );
    let payment_rows: Vec<crate::data::InvoicePaymentRow> = demo
        .payments()
        .iter()
        .map(|payment| crate::data::InvoicePaymentRow {
            at_ms: Some(payment.at_ms),
            status: payment.status.clone(),
            reference_masked: Some(payment.payment_id_masked.clone()),
            amount_micros: Some(payment.amount.micros),
            currency: payment.amount.currency.clone(),
            received: Some(payment.received),
            verified_fetch: Some(payment.verified_fetch),
            deduplicated: Some(payment.deduplicated),
            reconciled: Some(payment.reconciled),
            allocated: Some(payment.allocated),
            note: None,
        })
        .collect();
    let invoices = demo.invoices();
    let invoice = invoices
        .first()
        .cloned()
        .ok_or_else(|| "fixture invoice missing".to_string())?;
    push(
        "invoice",
        pages::finance::render_invoice(
            &m,
            &pages::finance::InvoiceDetail {
                invoice,
                payments_note: None,
                payments: payment_rows,
                calculation_version: "calc-v3".to_string(),
                source_hashes: vec![format!("sha256:{}", sha256_hex(b"fixture-source"))],
                forms: vec![
                    form_spec(
                        &state,
                        &session,
                        Capability::InvoiceHold,
                        "INV-2026-10-northstar",
                        "/control/finance/invoices/INV-2026-10-northstar",
                        "fixture-hold",
                    ),
                    form_spec(
                        &state,
                        &session,
                        Capability::CollectionRetry,
                        "INV-2026-10-northstar",
                        "/control/finance/invoices/INV-2026-10-northstar",
                        "fixture-collect",
                    ),
                    form_spec(
                        &state,
                        &session,
                        Capability::DisputeMark,
                        "INV-2026-10-northstar",
                        "/control/finance/invoices/INV-2026-10-northstar",
                        "fixture-dispute",
                    ),
                    form_spec(
                        &state,
                        &session,
                        Capability::ManualReconcile,
                        "INV-2026-10-northstar",
                        "/control/finance/invoices/INV-2026-10-northstar",
                        "fixture-reconcile",
                    ),
                ],
            },
        ),
    );

    // Growth.
    push(
        "growth",
        pages::growth::render(
            &m,
            &pages::growth::GrowthData {
                cohorts: Ok(demo.growth_cohorts()),
                selected_cohort: Some("2026-08".to_string()),
                funnel: demo
                    .activation_funnel("2026-08")
                    .map_err(|error| error.reason),
                site_requests: Some(demo.site_requests_last_30_days()),
            },
        ),
    );
    push(
        "growth-incomplete",
        pages::growth::render(
            &m,
            &pages::growth::GrowthData {
                cohorts: Ok(demo.growth_cohorts()),
                selected_cohort: Some("2026-09".to_string()),
                funnel: demo
                    .activation_funnel("2026-09")
                    .map_err(|error| error.reason),
                site_requests: Some(demo.site_requests_last_30_days()),
            },
        ),
    );
    // The unavailable site-request read renders no line at all: an absent
    // number is not a zero.
    push(
        "growth-no-site-requests",
        pages::growth::render(
            &m,
            &pages::growth::GrowthData {
                cohorts: Ok(demo.growth_cohorts()),
                selected_cohort: Some("2026-08".to_string()),
                funnel: demo
                    .activation_funnel("2026-08")
                    .map_err(|error| error.reason),
                site_requests: None,
            },
        ),
    );

    // System and policies.
    push(
        "system",
        pages::system::render(
            &m,
            &pages::system::SystemData {
                overdue_jobs: Ok(demo.queues().overdue_jobs),
                component_rows: demo.system_components(),
                ledger_copy_line: ControlData::fixtures()
                    .ledger_copy(crate::fixtures::DEMO_NOW_MS)
                    .system_line(crate::fixtures::DEMO_NOW_MS),
                job_retry_forms: Vec::new(),
            },
        ),
    );
    let evidence = demo.operations_evidence();
    push(
        "system-policies",
        pages::system::render_policies(
            &m,
            &pages::system::PoliciesData {
                readiness: crate::data::readiness_rows(&evidence),
                evidence_problem: None,
                pending: state
                    .governance
                    .pending_policies()
                    .map_err(|error| error.to_string()),
                active: state
                    .governance
                    .active_policies()
                    .map_err(|error| error.to_string()),
                interventions: state
                    .governance
                    .interventions()
                    .map(|read| read.records)
                    .map_err(|error| error.to_string()),
                fleet_paused: true,
                release_health: demo.release_health(),
                forms: vec![
                    form_spec(
                        &state,
                        &session,
                        Capability::PolicyDraft,
                        "policy:draft",
                        "/control/system/policies",
                        "fixture-policy-draft",
                    ),
                    form_spec(
                        &state,
                        &session,
                        Capability::PolicySet,
                        "policy:effective",
                        "/control/system/policies",
                        "fixture-policy-set",
                    ),
                    form_spec(
                        &state,
                        &session,
                        Capability::FleetResumeScheduling,
                        "fleet:scheduling",
                        "/control/system/policies",
                        "fixture-fleet-resume",
                    ),
                ],
            },
        ),
    );
    push(
        "system-policies-no-evidence",
        pages::system::render_policies(
            &m,
            &pages::system::PoliciesData {
                readiness: crate::data::readiness_rows(&crate::data::OperationsEvidence::default()),
                evidence_problem: None,
                pending: Ok(Vec::new()),
                active: Ok(Vec::new()),
                interventions: Ok(Vec::new()),
                fleet_paused: false,
                release_health: demo.release_health(),
                forms: Vec::new(),
            },
        ),
    );

    // Access.
    push(
        "access",
        pages::access::render(
            &m,
            &pages::access::AccessData {
                history: state
                    .journal
                    .read()
                    .map(|read| read.entries)
                    .map_err(|error| error.to_string()),
                sessions: state.sessions.list(state.now_ms()),
                can_revoke: true,
                revoke_forms: vec![form_spec(
                    &state,
                    &session,
                    Capability::SessionRevoke,
                    "deadbeef",
                    "/control/access",
                    "fixture-revoke",
                )],
                review_forms: vec![form_spec(
                    &state,
                    &session,
                    Capability::OperatorActionReview,
                    &crate::actions::action_target_key(&fixture_journal_entries()[0]),
                    "/control/access",
                    "fixture-review",
                )],
            },
        ),
    );
    push(
        "access-denied",
        pages::access::render(
            &m,
            &pages::access::AccessData {
                history: Ok(fixture_journal_entries()),
                sessions: state.sessions.list(state.now_ms()),
                can_revoke: false,
                revoke_forms: Vec::new(),
                review_forms: Vec::new(),
            },
        ),
    );

    // Action results, denial and conflict.
    for (name, action_state, job_state) in [
        ("result-requested", ActionState::Requested, Some("Queued")),
        ("result-running", ActionState::Running, Some("Running")),
        ("result-confirmed", ActionState::Confirmed, None),
        ("result-failed", ActionState::Failed, Some("Failed")),
        (
            "result-state-unknown",
            ActionState::StateUnknown,
            Some("State unknown"),
        ),
    ] {
        push(
            name,
            pages::actions::result(
                &m,
                Capability::RetryRead,
                "res_0123456789ab",
                action_state,
                Some("opj_0123456789ab"),
                "/control/ingestion",
                "One operator request created one job; duplicate submits reuse it.",
                job_state,
            ),
        );
    }
    push(
        "denied",
        pages::actions::denied(
            &m,
            Capability::IncidentResolve,
            "inc_018",
            "/control/incidents/inc_018",
        ),
    );
    push(
        "conflict",
        pages::errors::conflict(
            &m,
            "the object moved to a newer version",
            "/control/incidents/inc_018",
        ),
    );

    // Error family.
    push(
        "error-bad-request",
        pages::errors::bad_request(&m, Some("the request parameters could not be interpreted")),
    );
    push("error-not-found", pages::errors::not_found(&m));
    push(
        "error-method-not-allowed",
        pages::errors::method_not_allowed(&m),
    );
    push(
        "error-internal",
        pages::errors::internal(&m, "/control/ at 15 Oct 2026, 09:10 UTC"),
    );
    push(
        "error-not-configured",
        pages::errors::not_configured(&m, "No operator configuration is present."),
    );

    let mut written = Vec::new();
    for (name, html) in pages {
        let path = out_dir.join(format!("{name}.html"));
        std::fs::write(&path, html)
            .map_err(|error| format!("writing {}: {error}", path.display()))?;
        written.push(format!("{name}.html"));
    }
    // Clean the per-run working directory (journal, registers, jobs), but
    // never anything outside the system temp directory.
    if let Some(parent) = state.jobs.dir().and_then(std::path::Path::parent) {
        if parent.starts_with(std::env::temp_dir()) {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
    Ok(written)
}

fn stale_row() -> IngestionHealthRow {
    IngestionHealthRow {
        publisher_id: "northstar".to_string(),
        platform: "applovin_max".to_string(),
        platform_account_id: "acct_northstar_max_01".to_string(),
        last_final_day: Some("2026-10-11".to_string()),
        freshness_days: Some(4),
        status: FreshnessStatus::Stale,
    }
}

/// The illustrative weekly fee-row sample for pages that need one row
/// (kept for parity tests).
pub fn demo_hold_row() -> FeeRow {
    let mut rows = DemoData::new().fee_rows();
    rows.retain(|row| row.state == FeeState::Held);
    rows.into_iter().next().unwrap_or_else(|| FeeRow {
        change_id: "none".to_string(),
        period: "2026-10".to_string(),
        state: FeeState::Held,
        amount: None,
        source_amount: None,
        bound_bp: None,
        rate_bp: None,
        active_days: None,
        invoice_ref: None,
        proof_refs: Vec::new(),
    })
}

/// A fixture payment for finance parity tests.
pub fn demo_payment() -> PaymentEvent {
    let mut payments = DemoData::new().payments();
    payments.remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_payment_is_received_and_fetched_verified() {
        let payment = demo_payment();
        assert!(payment.received);
        assert!(
            payment.verified_fetch,
            "the demo payment is a Mollie fetch, not an unverified row"
        );
        assert!(payment.success);
    }

    #[test]
    fn render_all_writes_every_page_with_one_h1_and_no_script() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-fixture-test-{}-{}",
            std::process::id(),
            random_hex_32()
        ));
        let files = render_all(&dir).expect("render");
        assert!(files.len() >= 28, "pages: {}", files.len());
        for file in &files {
            let html = std::fs::read_to_string(dir.join(file)).expect("read");
            assert_eq!(
                html.matches("<h1").count(),
                1,
                "{file} must have exactly one h1"
            );
            assert!(!html.contains("<script"), "{file} must be script free");
            assert!(html.contains("ILLUSTRATIVE DATA"), "{file} banner");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_fixture_page_renders_a_session_handle_or_password() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-fixture-leak-{}-{}",
            std::process::id(),
            random_hex_32()
        ));
        let files = render_all(&dir).expect("render");
        for file in &files {
            let html = std::fs::read_to_string(dir.join(file)).expect("read");
            assert!(
                !html.contains(FIXTURE_PASSWORD),
                "{file} leaks the password"
            );
            assert!(
                !html.to_ascii_lowercase().contains("argon2"),
                "{file} leaks a PHC string"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
