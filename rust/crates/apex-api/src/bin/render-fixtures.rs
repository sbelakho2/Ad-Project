//! render-fixtures — renders every console page and state to an output
//! directory from checked-in sample fixtures, so the UI/UX accessibility lane
//! scans the real rendered HTML (never a hand-written approximation).
//!
//! Usage: cargo run -p apex-api --bin render-fixtures -- <fixtures-dir> <out-dir>
//!
//! Fixture files: `admob.json` (a real audit Snapshot; the Northstar Studio
//! illustration: 5 findings, 1 critical) and `northstar.jsonl` (a real signed
//! ledger written with the published test key: 24 records covering every
//! record variant). The ledger fixture is generated once when absent; its
//! generation is deterministic.
//!
//! Phase-2 surfaces (connections, proposals, experiments, billing, settings,
//! analytics) are rendered from the same Northstar data: realistic platform
//! connection states, a reviewable and a decided proposal, five experiment
//! lifecycle states including the restore-failed incident, the billing
//! register with settled and open months, the settings pages and the
//! analytics tabs with both populated and not-configured states.
use std::collections::HashMap;
use std::path::PathBuf;

use apex_analytics::queries::{
    DataQuality, FeeDetail, FeeRow, FeeState, LiftRow, Money, QueryScope, RevenueBreakdownRow,
    RevenueDailyRow, StaleSource,
};
use apex_api::billing_view::{BillingView, ManagedView};
use apex_api::connections::{ConnectionMeta, CredentialState, PlatformState};
use apex_api::experiments::ExperimentView;
use apex_api::html::analytics::{
    AnalyticsData, AnalyticsPageData, VIEW_DATA_QUALITY, VIEW_FEES, VIEW_LIFT, VIEW_REVENUE,
};
use apex_api::html::audit::{self, AuditFormProblem, AuditReportView, FindingView};
use apex_api::html::billing as billing_html;
use apex_api::html::connections as connections_html;
use apex_api::html::errors;
use apex_api::html::experiments as experiments_html;
use apex_api::html::leads_html::{self, RequestPageState};
use apex_api::html::ledger::{self, LedgerAccessProblem};
use apex_api::html::overview::{LatestAudit, LedgerState, OverviewData};
use apex_api::html::proposals as proposals_html;
use apex_api::html::settings as settings_html;
use apex_api::html::watch_html::{self, WatchPageState};
use apex_api::html::{overview, ConsoleCtx};
use apex_api::leads::{
    FieldError, FreeAuditInvalid, FreeAuditRequest, EMAIL_ERROR, PLATFORM_ERROR,
};
use apex_api::proposals::{ApprovalDecision, ApprovalRecord, ConsoleProposal};
use apex_api::settings::{AccountView, PolicyView, RegisteredPolicy};
use apex_api::sources::{FileLedgerSource, LedgerSource};
use apex_api::vault::VaultEntryMeta;
use apex_core::platform::ExperimentMetric;
use apex_core::{
    ExperimentResult, ExperimentSpec, ExperimentState, ExperimentVariantResult, PlatformId,
};
use apex_proof::{ChangeKind, Keypair, Ledger, LedgerRecord, VerifyingKey};
use sha2::{Digest, Sha256};

/// The published test key of the ledger fixture (documented, not a secret).
const TEST_KEY_SEED: [u8; 32] = [7u8; 32];
/// The publisher of the Northstar illustration.
const PUBLISHER: &str = "northstar";
/// The illustrative "verified" moment: 15 Oct 2026, 09:10 UTC.
const NOW_MS: i64 = 1_792_055_400_000;
/// A fixed, non-secret CSRF placeholder for the rendered session form.
const CSRF: &str = "0123456789abcdef0123456789abcdef";

/// Every fixture page this binary renders.
pub const PAGES: [&str; 62] = [
    "overview.html",
    "overview-empty.html",
    "audit-form.html",
    "audit-report.html",
    "audit-no-findings.html",
    "audit-no-snapshot.html",
    "audit-unknown-platform.html",
    "finding-detail.html",
    "finding-detail-not-estimated.html",
    "ledger-access.html",
    "ledger-session-expired.html",
    "ledger-records.html",
    "ledger-empty.html",
    "ledger-record-detail.html",
    "ledger-unverified.html",
    "error-400.html",
    "error-404.html",
    "error-405.html",
    "error-409.html",
    "error-413.html",
    "error-422.html",
    "error-429.html",
    "error-500.html",
    "error-503.html",
    "free-audit.html",
    "free-audit-errors.html",
    "free-audit-failure.html",
    "audit-requested.html",
    "watch.html",
    "connections-list.html",
    "connection-detail-max.html",
    "connection-detail-levelplay.html",
    "connection-detail-admob.html",
    "connection-detail-not-configured.html",
    "proposals-index.html",
    "proposals-index-empty.html",
    "proposal-review.html",
    "proposal-decided.html",
    "proposal-expired.html",
    "proposal-not-found.html",
    "experiments-list.html",
    "experiment-detail-waiting.html",
    "experiment-detail-kept.html",
    "experiment-detail-inputs.html",
    "experiment-detail-guardrails.html",
    "experiment-detail-history.html",
    "experiment-incident.html",
    "experiments-list-empty.html",
    "billing.html",
    "invoice-draft.html",
    "invoice-not-found.html",
    "settings-account.html",
    "settings-access.html",
    "settings-policy.html",
    "settings-monitoring.html",
    "settings-billing.html",
    "analytics-revenue.html",
    "analytics-lift.html",
    "analytics-fees.html",
    "analytics-data-quality.html",
    "analytics-not-configured.html",
    "analytics-bad-view.html",
];

/// Read the Northstar AdMob fixture.
///
/// The committed illustration keeps its historical `fixtures/admob.json`
/// path (the autopilot crate includes those bytes in its own tests); the
/// console's source layout is per publisher, so this renderer reads the one
/// committed snapshot directly rather than pretending the flat fixture
/// directory is a publisher-scoped root.
fn load_fixture_snapshot(
    fixtures: &std::path::Path,
) -> Result<apex_audit::Snapshot, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(fixtures.join("admob.json"))?;
    let snapshot: apex_audit::Snapshot = serde_json::from_slice(&bytes)?;
    if snapshot.platform != PlatformId::Admob {
        return Err(format!(
            "the committed fixture declares {} but is rendered as AdMob",
            snapshot.platform
        )
        .into());
    }
    Ok(snapshot)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let fixtures = PathBuf::from(
        args.next()
            .ok_or("usage: render-fixtures <fixtures-dir> <out-dir>")?,
    );
    let out = PathBuf::from(
        args.next()
            .ok_or("usage: render-fixtures <fixtures-dir> <out-dir>")?,
    );
    run(&fixtures, &out).await
}

/// Render every fixture page into `out`, generating the ledger fixture when
/// absent.
async fn run(
    fixtures: &std::path::Path,
    out: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;

    let ledger_path = fixtures.join(format!("{PUBLISHER}.jsonl"));
    if !ledger_path.exists() {
        generate_ledger(&ledger_path)?;
    }

    let keypair = Keypair::from_bytes(&TEST_KEY_SEED);
    let mut keys: HashMap<String, VerifyingKey> = HashMap::new();
    keys.insert(PUBLISHER.to_string(), keypair.verifying_key());
    let ledger_source = FileLedgerSource::new(fixtures.to_path_buf(), keys);

    let snapshot = load_fixture_snapshot(fixtures)?;
    let report = apex_audit::run(&snapshot);
    let records = ledger_source.records(PUBLISHER).await.map_err(|error| {
        std::io::Error::other(format!("{PUBLISHER} ledger read failed: {error}"))
    })?;

    // This renderer is the offline branch of the console: it builds the
    // ConsoleCtx directly from the fixture data (label, currency, read-only)
    // and calls the render functions — no AppState, no router and no session.
    // The router's session gate never applies to static fixture rendering.
    let ctx = ConsoleCtx::new("Northstar Studio", "USD", true);
    let mut pages: Vec<(String, String)> = Vec::new();

    // Overview: the populated dashboard and the first-run state.
    let critical = report
        .findings
        .iter()
        .filter(|finding| finding.severity == apex_audit::Severity::Critical)
        .count();
    let mut warning = 0usize;
    let mut info = 0usize;
    for finding in &report.findings {
        match finding.severity {
            apex_audit::Severity::Critical => {}
            apex_audit::Severity::Warning => warning += 1,
            apex_audit::Severity::Info => info += 1,
        }
    }
    let populated = OverviewData {
        latest_audit: Some(LatestAudit {
            platform_label: audit::platform_label(PlatformId::Admob).to_string(),
            captured_at_unix_ms: snapshot.fetched_at_unix_ms,
            findings: report.findings.len(),
            critical,
            warning,
            info,
        }),
        ledger: LedgerState::Verified {
            records: records.len() as u64,
            unavailable: 0,
        },
    };
    pages.push(("overview.html".into(), overview::render(&ctx, &populated)));
    pages.push((
        "overview-empty.html".into(),
        overview::render(
            &ctx,
            &OverviewData {
                latest_audit: None,
                ledger: LedgerState::NotConfigured,
            },
        ),
    ));

    // Audit: form states, the report, the clean report and both finding
    // details (money model and no money model).
    pages.push((
        "audit-form.html".into(),
        audit::render_audit_form(&ctx, None),
    ));
    pages.push((
        "audit-no-snapshot.html".into(),
        audit::render_audit_form(
            &ctx,
            Some(&AuditFormProblem::NoSnapshot("levelplay".to_string())),
        ),
    ));
    pages.push((
        "audit-unknown-platform.html".into(),
        audit::render_audit_form(
            &ctx,
            Some(&AuditFormProblem::UnknownPlatform("tiktok".to_string())),
        ),
    ));
    let view = AuditReportView {
        platform: PlatformId::Admob,
        snapshot: &snapshot,
        report: &report,
    };
    pages.push((
        "audit-report.html".into(),
        audit::render_audit_report(&ctx, &view),
    ));

    let clean: apex_audit::Snapshot = serde_json::from_value(serde_json::json!({
        "platform": "admob",
        "fetchedAtUnixMs": NOW_MS,
        "adUnits": [{
            "platformUnitId": "rw_de_01",
            "name": "Rewarded / Germany",
            "format": "rewarded",
            "enabled": true,
            "countryFloors": [{"country": "DE", "floorEcpm": 4_000_000}]
        }],
        "waterfall": [],
        "mediationGroups": [],
        "bidderAccounts": [{
            "network": "mintegral",
            "hasApexManagedAccount": true,
            "enabledInPlatform": true
        }],
        "countryRevenue": [{
            "country": "DE",
            "adUnitId": "rw_de_01",
            "revenue30d": 350_000_000,
            "impressions30d": 100_000
        }],
        "appAdsTxt": {
            "present": true,
            "lines": ["admob.com, pub-northstar, DIRECT"],
            "expectedNetworks": ["admob.com"]
        },
        "platformTotals30d": {"revenue30d": 350_000_000, "impressions30d": 100_000},
        "mediationTotals30d": {"revenue30d": 350_000_000, "impressions30d": 100_000}
    }))?;
    let clean_report = apex_audit::run(&clean);
    pages.push((
        "audit-no-findings.html".into(),
        audit::render_audit_report(
            &ctx,
            &AuditReportView {
                platform: PlatformId::Admob,
                snapshot: &clean,
                report: &clean_report,
            },
        ),
    ));

    let critical_finding = find_finding(&report, "ad_unit:rw_de_01:country:DE")?;
    pages.push((
        "finding-detail.html".into(),
        audit::render_finding_detail(
            &ctx,
            &FindingView {
                snapshot: &snapshot,
                finding: critical_finding,
            },
        ),
    ));
    let no_model_finding = find_finding(&report, "ad_unit:bn_fr_01:country:FR")?;
    pages.push((
        "finding-detail-not-estimated.html".into(),
        audit::render_finding_detail(
            &ctx,
            &FindingView {
                snapshot: &snapshot,
                finding: no_model_finding,
            },
        ),
    ));

    // Change log: access panel, expired session, records, empty ledger,
    // record detail and the unverified state.
    pages.push((
        "ledger-access.html".into(),
        ledger::render_access_panel(&ctx, None, None),
    ));
    pages.push((
        "ledger-session-expired.html".into(),
        ledger::render_access_panel(&ctx, Some(LedgerAccessProblem::SessionExpired), None),
    ));
    pages.push((
        "ledger-records.html".into(),
        ledger::render_records(
            &ctx,
            PUBLISHER,
            ledger::RecordsPage {
                records: &records,
                total: records.len() as u64,
                page: 1,
            },
            NOW_MS,
            CSRF,
        ),
    ));
    pages.push((
        "ledger-empty.html".into(),
        ledger::render_records(
            &ctx,
            PUBLISHER,
            ledger::RecordsPage {
                records: &[],
                total: 0,
                page: 1,
            },
            NOW_MS,
            CSRF,
        ),
    ));
    let first = records.first().ok_or("the ledger fixture has no records")?;
    pages.push((
        "ledger-record-detail.html".into(),
        ledger::render_record_detail(&ctx, PUBLISHER, first, records.len(), CSRF),
    ));
    pages.push((
        "ledger-unverified.html".into(),
        errors::ledger_unverified(&ctx, 18),
    ));

    // The error family, one fixture per status.
    pages.push(("error-400.html".into(), errors::bad_request(&ctx, None)));
    pages.push(("error-404.html".into(), errors::not_found(&ctx)));
    pages.push(("error-405.html".into(), errors::method_not_allowed(&ctx)));
    pages.push(("error-409.html".into(), errors::conflict(&ctx)));
    pages.push((
        "error-413.html".into(),
        errors::body_too_large(&ctx, apex_api::MAX_BODY_BYTES / 1024),
    ));
    pages.push(("error-422.html".into(), errors::ledger_unverified(&ctx, 18)));
    pages.push(("error-429.html".into(), errors::too_many_requests(&ctx)));
    pages.push((
        "error-500.html".into(),
        errors::audit_read_failure(&ctx, "/console/audit at 15 Oct 2026, 09:10 UTC"),
    ));
    pages.push(("error-503.html".into(), errors::not_configured(&ctx)));

    // The free-audit flow: the request page, a validation failure, a storage
    // failure and the confirmation.
    pages.push((
        "free-audit.html".into(),
        leads_html::render_request_page(&ctx, &RequestPageState::Fresh),
    ));
    let invalid = FreeAuditInvalid {
        request: FreeAuditRequest {
            email: "founder@northstar".into(),
            studio: "Northstar Studio".into(),
            platforms: vec!["applovin_max".into()],
            revenue: Some("$50,000 to $100,000".into()),
            notes: None,
        },
        errors: vec![
            FieldError {
                field: "email",
                message: EMAIL_ERROR.to_string(),
            },
            FieldError {
                field: "platforms",
                message: PLATFORM_ERROR.to_string(),
            },
        ],
    };
    pages.push((
        "free-audit-errors.html".into(),
        leads_html::render_request_page(&ctx, &RequestPageState::Invalid(Box::new(invalid))),
    ));
    let failed = FreeAuditRequest {
        email: "founder@northstar.example".into(),
        studio: "Northstar Studio".into(),
        platforms: vec!["admob".into(), "levelplay".into()],
        revenue: None,
        notes: Some("We run rewarded video in the DACH region.".into()),
    };
    pages.push((
        "free-audit-failure.html".into(),
        leads_html::render_request_page(
            &ctx,
            &RequestPageState::SubmissionFailed(Box::new(failed)),
        ),
    ));
    pages.push((
        "audit-requested.html".into(),
        leads_html::render_confirmation(&ctx, Some("req_0123456789")),
    ));

    // The setup-watch sign-up (the confirmation shares the marketing layout
    // and is exercised by the integration tests).
    pages.push((
        "watch.html".into(),
        watch_html::render_request_page(&ctx, &WatchPageState::Fresh),
    ));

    // --- phase 2: connections -----------------------------------------------
    let connections_list = connections_html::ConnectionsListData {
        publisher: PUBLISHER.to_string(),
        as_of_unix_ms: NOW_MS,
        rows: vec![
            connections_html::ConnectionRow {
                wire: "applovin_max",
                label: "AppLovin MAX",
                credential_state: CredentialState::Validated,
                write_label: "Approval required",
                last_report_unix_ms: Some(1_792_055_340_000),
            },
            connections_html::ConnectionRow {
                wire: "levelplay",
                label: "LevelPlay",
                credential_state: CredentialState::Rejected,
                write_label: "Not enabled",
                last_report_unix_ms: None,
            },
            connections_html::ConnectionRow {
                wire: "admob",
                label: "AdMob",
                credential_state: CredentialState::Unvalidated,
                write_label: "Not enabled",
                last_report_unix_ms: None,
            },
        ],
    };
    pages.push((
        "connections-list.html".into(),
        connections_html::render_list(&ctx, &connections_list),
    ));

    let blob_meta = VaultEntryMeta {
        scheme: "aes-256-gcm",
        ciphertext_len: 412,
    };
    let max_state = PlatformState {
        state: Some(CredentialState::Validated),
        validated_at_unix_ms: Some(1_792_055_340_000),
        reporting: Some(true),
        credential_fields: vec!["account_id".into(), "api_key".into(), "report_key".into()],
        failure_class: None,
    };
    pages.push((
        "connection-detail-max.html".into(),
        connections_html::render_detail(
            &ctx,
            &connections_html::ConnectionDetailData {
                publisher: PUBLISHER,
                platform: PlatformId::AppLovinMax,
                label: "AppLovin MAX",
                state: max_state,
                blob_meta: Some(&blob_meta),
                writable: true,
                csrf: Some(CSRF),
                errors: Vec::new(),
                preserved: Vec::new(),
                running_work: Some(1),
                last_report_unix_ms: Some(1_792_055_340_000),
            },
        ),
    ));
    let levelplay_state = PlatformState {
        state: Some(CredentialState::Rejected),
        validated_at_unix_ms: Some(1_792_000_000_000),
        reporting: Some(false),
        credential_fields: vec![
            "account_id".into(),
            "api_key".into(),
            "refresh_token".into(),
        ],
        failure_class: Some("auth_refused".into()),
    };
    pages.push((
        "connection-detail-levelplay.html".into(),
        connections_html::render_detail(
            &ctx,
            &connections_html::ConnectionDetailData {
                publisher: PUBLISHER,
                platform: PlatformId::LevelPlay,
                label: "LevelPlay",
                state: levelplay_state,
                blob_meta: None,
                writable: true,
                csrf: Some(CSRF),
                errors: vec![(
                    "api_key".to_string(),
                    "This field is required to confirm access.".to_string(),
                )],
                preserved: vec![(
                    "account_id".to_string(),
                    "com.northstar.rewarded".to_string(),
                )],
                running_work: Some(0),
                last_report_unix_ms: None,
            },
        ),
    ));
    pages.push((
        "connection-detail-admob.html".into(),
        connections_html::render_detail(
            &ctx,
            &connections_html::ConnectionDetailData {
                publisher: PUBLISHER,
                platform: PlatformId::Admob,
                label: "AdMob",
                state: PlatformState {
                    state: Some(CredentialState::Unvalidated),
                    validated_at_unix_ms: None,
                    reporting: None,
                    credential_fields: vec!["account_id".into(), "api_key".into()],
                    failure_class: None,
                },
                blob_meta: Some(&blob_meta),
                writable: true,
                csrf: Some(CSRF),
                errors: Vec::new(),
                preserved: Vec::new(),
                running_work: Some(0),
                last_report_unix_ms: None,
            },
        ),
    ));
    pages.push((
        "connection-detail-not-configured.html".into(),
        connections_html::render_detail(
            &ctx,
            &connections_html::ConnectionDetailData {
                publisher: PUBLISHER,
                platform: PlatformId::AppLovinMax,
                label: "AppLovin MAX",
                state: PlatformState::default(),
                blob_meta: None,
                writable: false,
                csrf: None,
                errors: Vec::new(),
                preserved: Vec::new(),
                running_work: Some(0),
                last_report_unix_ms: None,
            },
        ),
    ));

    // --- phase 2: proposals -------------------------------------------------
    let fixture_policy_hash = apex_autopilot::model::AutopilotPolicy::conservative()
        .canonical_hash()
        .unwrap_or_default();
    let proposal = ConsoleProposal {
        publisher: Some("northstar".to_string()),
        proposal: apex_autopilot::model::Proposal {
            action: apex_autopilot::model::Action::SetCountryFloor {
                ad_unit_id: "rw_de_01".to_string(),
                country: "DE".to_string(),
                floor_ecpm: 3_200_000,
            },
            rationale: "Raise the DE rewarded floor by 8%, from $3.46 to $3.20".to_string(),
            expected_effect_bp: 260,
            prev_value: "{\"DE\":4000000}".to_string(),
        },
        snapshot: "14 Oct 2026, 08:10 UTC".to_string(),
        created_at_unix_ms: 1_792_000_000_000,
        policy_version: "apex-autopilot-0.1.0".to_string(),
        policy_hash: fixture_policy_hash.clone(),
        expires_at_unix_ms: Some(NOW_MS + 5 * 86_400_000),
        test_share: 0.25,
        burn_in_days: 3,
        window_days: 14,
        rate_bp: 2_000,
        thresholds: apex_lift::decision::GuardrailThresholds::default(),
    };
    pages.push((
        "proposal-review.html".into(),
        proposals_html::render_detail(
            &ctx,
            &proposals_html::ProposalPageData {
                publisher: PUBLISHER,
                id: "p_0247",
                proposal: &proposal,
                approval: None,
                expired: false,
                csrf: Some(CSRF),
                refusal: None,
            },
        ),
    ));
    pages.push((
        "proposals-index.html".into(),
        proposals_html::render_index(
            &ctx,
            &[proposals_html::IndexEntry {
                id: "p_0247",
                proposal: &proposal,
                approval: None,
                expired: false,
            }],
            0,
        ),
    ));
    pages.push((
        "proposals-index-empty.html".into(),
        proposals_html::render_index(&ctx, &[], 0),
    ));
    let approval = ApprovalRecord {
        proposal_id: "p_0247".to_string(),
        decision: ApprovalDecision::Approved,
        decided_at_unix_ms: NOW_MS,
        idempotency_key: apex_api::proposals::idempotency_key(
            "p_0247",
            ApprovalDecision::Approved,
            &proposal.change_hash(),
        ),
        change_hash: proposal.change_hash(),
        snapshot: proposal.snapshot.clone(),
        policy_version: proposal.policy_version.clone(),
        policy_hash: proposal.policy_hash.clone(),
        approval_actor: apex_api::proposals::APPROVAL_ACTOR.to_string(),
    };
    pages.push((
        "proposal-decided.html".into(),
        proposals_html::render_detail(
            &ctx,
            &proposals_html::ProposalPageData {
                publisher: PUBLISHER,
                id: "p_0247",
                proposal: &proposal,
                approval: Some(&approval),
                expired: false,
                csrf: Some(CSRF),
                refusal: None,
            },
        ),
    ));
    pages.push((
        "proposal-expired.html".into(),
        proposals_html::render_detail(
            &ctx,
            &proposals_html::ProposalPageData {
                publisher: PUBLISHER,
                id: "p_0248",
                proposal: &proposal,
                approval: None,
                expired: true,
                csrf: Some(CSRF),
                refusal: None,
            },
        ),
    ));
    pages.push((
        "proposal-not-found.html".into(),
        proposals_html::render_not_found(&ctx),
    ));

    // --- phase 2: experiments ----------------------------------------------
    let experiment_records = experiment_fixture_records()?;
    let alerts = vec![apex_autopilot::alerts::Alert::FailedRollback {
        experiment_id: "exp_0239".to_string(),
        detail: "rollback also failed for max-floor:rw_de_01:DE:2".to_string(),
    }];
    let views = apex_api::experiments::project(&experiment_records, &alerts, NOW_MS);
    pages.push((
        "experiments-list.html".into(),
        experiments_html::render_list(
            &ctx,
            &experiments_html::ExperimentsListData {
                publisher: PUBLISHER,
                as_of_unix_ms: NOW_MS,
                filter: experiments_html::ExperimentFilter::All,
                views: views.clone(),
            },
        ),
    ));
    pages.push((
        "experiments-list-empty.html".into(),
        experiments_html::render_list(
            &ctx,
            &experiments_html::ExperimentsListData {
                publisher: PUBLISHER,
                as_of_unix_ms: NOW_MS,
                filter: experiments_html::ExperimentFilter::Finalized,
                views: Vec::new(),
            },
        ),
    ));
    let history_for = |id: &str| apex_api::experiments::history(&experiment_records, id);
    let waiting = find_experiment(&views, "exp_0247")?;
    pages.push((
        "experiment-detail-waiting.html".into(),
        experiments_html::render_detail(
            &ctx,
            &experiments_html::ExperimentDetailData {
                publisher: PUBLISHER,
                view: waiting,
                tab: experiments_html::ExperimentTab::Summary,
                history: history_for("exp_0247"),
            },
        ),
    ));
    let kept = find_experiment(&views, "exp_0246")?;
    pages.push((
        "experiment-detail-kept.html".into(),
        experiments_html::render_detail(
            &ctx,
            &experiments_html::ExperimentDetailData {
                publisher: PUBLISHER,
                view: kept,
                tab: experiments_html::ExperimentTab::Summary,
                history: history_for("exp_0246"),
            },
        ),
    ));
    pages.push((
        "experiment-detail-inputs.html".into(),
        experiments_html::render_detail(
            &ctx,
            &experiments_html::ExperimentDetailData {
                publisher: PUBLISHER,
                view: kept,
                tab: experiments_html::ExperimentTab::Inputs,
                history: history_for("exp_0246"),
            },
        ),
    ));
    pages.push((
        "experiment-detail-guardrails.html".into(),
        experiments_html::render_detail(
            &ctx,
            &experiments_html::ExperimentDetailData {
                publisher: PUBLISHER,
                view: find_experiment(&views, "exp_0248")?,
                tab: experiments_html::ExperimentTab::Guardrails,
                history: history_for("exp_0248"),
            },
        ),
    ));
    pages.push((
        "experiment-detail-history.html".into(),
        experiments_html::render_detail(
            &ctx,
            &experiments_html::ExperimentDetailData {
                publisher: PUBLISHER,
                view: kept,
                tab: experiments_html::ExperimentTab::History,
                history: history_for("exp_0246"),
            },
        ),
    ));
    let incident_view = find_experiment(&views, "exp_0239")?;
    pages.push((
        "experiment-incident.html".into(),
        experiments_html::render_incident(&ctx, PUBLISHER, incident_view),
    ));

    // --- phase 2: billing ---------------------------------------------------
    let billing = billing_fixture()?;
    pages.push((
        "billing.html".into(),
        billing_html::render_billing(
            &ctx,
            &billing_html::BillingPageData {
                view: &billing,
                as_of_unix_ms: NOW_MS,
            },
        ),
    ));
    pages.push((
        "invoice-draft.html".into(),
        billing_html::render_invoice(
            &ctx,
            &billing_html::InvoicePageData {
                view: &billing,
                period: "2026-10",
                vat_rate_bp: Some(2_000),
            },
        ),
    ));
    pages.push((
        "invoice-not-found.html".into(),
        billing_html::render_period_not_found(&ctx),
    ));

    // --- phase 2: settings --------------------------------------------------
    let account = AccountView {
        workspace: "Northstar Studio".to_string(),
        contact: Some("ops@northstar.example".to_string()),
        timezone: "UTC".to_string(),
        currency: "USD".to_string(),
        retention_months: Some(24),
        sessions_bound_to_credentials: true,
    };
    pages.push((
        "settings-account.html".into(),
        settings_html::render_account(
            &ctx,
            &settings_html::AccountPageData {
                account: &account,
                as_of_unix_ms: NOW_MS,
                theme: apex_ui::theme::Theme::Auto,
                csrf: Some(CSRF),
                theme_error: None,
            },
        ),
    ));
    pages.push((
        "settings-access.html".into(),
        settings_html::render_access(
            &ctx,
            &settings_html::AccessPageData {
                publisher: PUBLISHER,
                sessions: vec![
                    apex_api::session::SessionSummary {
                        handle: "0f3a9c1d7b2e4865".to_string(),
                        publisher: PUBLISHER.to_string(),
                        created_ms: NOW_MS - 22 * 60_000,
                        last_seen_ms: NOW_MS - 60_000,
                    },
                    apex_api::session::SessionSummary {
                        handle: "5c81e4b0aa73d921".to_string(),
                        publisher: PUBLISHER.to_string(),
                        created_ms: NOW_MS - 3 * 3_600_000,
                        last_seen_ms: NOW_MS - 12 * 60_000,
                    },
                ],
                current_handle: Some("0f3a9c1d7b2e4865"),
                csrf: Some(CSRF),
                as_of_unix_ms: NOW_MS,
            },
        ),
    ));
    let policy = apex_autopilot::model::AutopilotPolicy::conservative();
    let policy_view = PolicyView {
        latest_registration: Some(RegisteredPolicy {
            experiment_id: "exp_0246".to_string(),
            code_version_hash: "apex-lift-0.1.0".to_string(),
            rate_bp: 2_000,
            burn_in_days: policy.burn_in_days,
            window_days: policy.window_days as u32,
            thresholds: serde_json::json!({
                "harmPoint": -0.05,
                "fillRewarded": 0.02,
                "fillBanner": 0.05,
                "splitChiSquare": 10.83
            }),
            seq: 22,
            at_unix_ms: 1_791_967_080_000,
        }),
    };
    pages.push((
        "settings-policy.html".into(),
        settings_html::render_policy(
            &ctx,
            &settings_html::PolicyPageData {
                publisher: PUBLISHER,
                policy: Some(&policy),
                approval_required_now: Some(policy.approval_required(NOW_MS)),
                view: &policy_view,
                as_of_unix_ms: NOW_MS,
            },
        ),
    ));
    let mut connection_meta = ConnectionMeta::default();
    connection_meta.set_platform(
        "applovin_max",
        PlatformState {
            state: Some(CredentialState::Validated),
            validated_at_unix_ms: Some(1_792_055_340_000),
            reporting: Some(true),
            credential_fields: vec!["account_id".into()],
            failure_class: None,
        },
    );
    connection_meta.set_platform(
        "levelplay",
        PlatformState {
            state: Some(CredentialState::Expired),
            validated_at_unix_ms: Some(1_792_000_000_000),
            reporting: Some(false),
            credential_fields: vec!["account_id".into()],
            failure_class: Some("auth_refused".into()),
        },
    );
    let monitoring_rows = apex_api::settings::monitoring_rows(
        &[(PlatformId::AppLovinMax, 1_792_055_340_000)],
        &connection_meta,
    );
    pages.push((
        "settings-monitoring.html".into(),
        settings_html::render_monitoring(
            &ctx,
            &settings_html::MonitoringPageData {
                publisher: PUBLISHER,
                rows: monitoring_rows,
                alert_log_configured: true,
                as_of_unix_ms: NOW_MS,
                csrf: Some("fixture-csrf"),
            },
        ),
    ));
    // The billing settings page: the managed ad-ops line with its written
    // scope and cancel action.
    let managed_plan = ManagedView::from_state(
        &apex_billing::ManagedPlan::default().activate(900_000 * 1_000_000, "2026-06-01"),
    );
    pages.push((
        "settings-billing.html".into(),
        settings_html::render_billing_settings(
            &ctx,
            &settings_html::BillingSettingsPageData {
                publisher: PUBLISHER,
                managed: &managed_plan,
                as_of_unix_ms: NOW_MS,
                csrf: Some("fixture-csrf"),
            },
        ),
    ));

    // --- phase 2: analytics -------------------------------------------------
    let scope = QueryScope::new(
        PUBLISHER,
        apex_analytics::UtcMillis::from_unix_millis(NOW_MS),
    )
    .with_currency("USD")
    .with_day_range("2026-09-15", "2026-10-14");
    let quality = fixture_quality();
    let revenue = AnalyticsData::Revenue {
        daily: vec![
            RevenueDailyRow {
                day: "2026-10-11".to_string(),
                platform: "applovin_max".to_string(),
                revenue: Money::new(42_500_000, "USD"),
                impressions: Some(980_000),
                source_days: 1,
                final_days: 1,
                provisional_days: 0,
            },
            RevenueDailyRow {
                day: "2026-10-12".to_string(),
                platform: "applovin_max".to_string(),
                revenue: Money::new(43_100_000, "USD"),
                impressions: Some(995_000),
                source_days: 1,
                final_days: 1,
                provisional_days: 0,
            },
            // 2026-10-13 is missing: a gap in the chart and the table.
            RevenueDailyRow {
                day: "2026-10-14".to_string(),
                platform: "applovin_max".to_string(),
                revenue: Money::new(44_050_000, "USD"),
                impressions: Some(1_010_000),
                source_days: 1,
                final_days: 0,
                provisional_days: 1,
            },
        ],
        by_platform: vec![RevenueBreakdownRow {
            dimension_value: "applovin_max".to_string(),
            revenue: Money::new(129_650_000, "USD"),
            impressions: Some(2_985_000),
            rows: 3,
            share_bp: Some(10_000),
        }],
        by_format: vec![RevenueBreakdownRow {
            dimension_value: "rewarded".to_string(),
            revenue: Money::new(129_650_000, "USD"),
            impressions: Some(2_985_000),
            rows: 3,
            share_bp: Some(10_000),
        }],
        by_country: vec![RevenueBreakdownRow {
            dimension_value: "DE".to_string(),
            revenue: Money::new(88_200_000, "USD"),
            impressions: Some(1_900_000),
            rows: 3,
            share_bp: Some(6_800),
        }],
        quality: quality.clone(),
    };
    pages.push((
        "analytics-revenue.html".into(),
        apex_api::html::analytics::render(
            &ctx,
            &AnalyticsPageData {
                publisher: PUBLISHER.to_string(),
                tab: VIEW_REVENUE,
                scope: scope.clone(),
                data: Some(revenue),
                unavailable_reason: None,
                selected_change: None,
            },
        ),
    ));
    let lift = AnalyticsData::Lift {
        rows: vec![
            LiftRow {
                experiment_id: "exp_0246".to_string(),
                change_ref: Some("ch_0246".to_string()),
                observed_lift_bp: Some(300),
                billable_bound_bp: Some(120),
                outcome: "keep".to_string(),
                measurement_source: Some("apex-lift-0.1.0".to_string()),
                measurement_version: Some("sha256:9f2c".to_string()),
                decided_at: Some("2026-10-10T09:30:00.000Z".to_string()),
            },
            LiftRow {
                experiment_id: "exp_0248".to_string(),
                change_ref: Some("ch_0248".to_string()),
                observed_lift_bp: Some(40),
                billable_bound_bp: None,
                outcome: "rollback".to_string(),
                measurement_source: Some("apex-lift-0.1.0".to_string()),
                measurement_version: Some("sha256:1a4b".to_string()),
                decided_at: Some("2026-10-14T09:30:00.000Z".to_string()),
            },
        ],
        quality: quality.clone(),
    };
    pages.push((
        "analytics-lift.html".into(),
        apex_api::html::analytics::render(
            &ctx,
            &AnalyticsPageData {
                publisher: PUBLISHER.to_string(),
                tab: VIEW_LIFT,
                scope: scope.clone(),
                data: Some(lift),
                unavailable_reason: None,
                selected_change: None,
            },
        ),
    ));
    let fees = AnalyticsData::Fees {
        rows: vec![
            FeeRow {
                change_id: "ch_0246".to_string(),
                period: "2026-10".to_string(),
                state: FeeState::Estimated,
                amount: Some(Money::new(300_000_000, "USD")),
                source_amount: Some(Money::new(51_500_000_000, "USD")),
                bound_bp: Some(120),
                rate_bp: Some(2_000),
                active_days: Some(31),
                invoice_ref: None,
                proof_refs: vec![
                    "pre:sha256:9f2c".to_string(),
                    "result:sha256:1a4b".to_string(),
                ],
            },
            FeeRow {
                change_id: "ch_0242".to_string(),
                period: "2026-09".to_string(),
                state: FeeState::Invoiced,
                amount: Some(Money::new(225_000_000, "USD")),
                source_amount: Some(Money::new(40_000_000_000, "USD")),
                bound_bp: Some(200),
                rate_bp: Some(2_000),
                active_days: Some(30),
                invoice_ref: Some("INV-2026-009".to_string()),
                proof_refs: vec!["pre:sha256:44dd".to_string()],
            },
        ],
        detail: Some(FeeDetail {
            change_id: "ch_0246".to_string(),
            period: "2026-10".to_string(),
            eligible_revenue: Money::new(51_500_000_000, "USD"),
            conservative_bound_bp: 120,
            agreed_rate_bp: 2_000,
            attributed_increment: Some(Money::new(600_000_000, "USD")),
            fee_before_tax: Some(Money::new(120_000_000, "USD")),
            active_days: Some(31),
            proof_refs: vec!["pre:sha256:9f2c".to_string()],
            calculated_at: Some("2026-10-15T00:00:00.000Z".to_string()),
            unavailable: Vec::new(),
        }),
        quality: quality.clone(),
    };
    pages.push((
        "analytics-fees.html".into(),
        apex_api::html::analytics::render(
            &ctx,
            &AnalyticsPageData {
                publisher: PUBLISHER.to_string(),
                tab: VIEW_FEES,
                scope: scope.clone(),
                data: Some(fees),
                unavailable_reason: None,
                selected_change: Some("ch_0246".to_string()),
            },
        ),
    ));
    pages.push((
        "analytics-data-quality.html".into(),
        apex_api::html::analytics::render(
            &ctx,
            &AnalyticsPageData {
                publisher: PUBLISHER.to_string(),
                tab: VIEW_DATA_QUALITY,
                scope: scope.clone(),
                data: Some(AnalyticsData::Quality {
                    quality: quality.clone(),
                }),
                unavailable_reason: None,
                selected_change: None,
            },
        ),
    ));
    pages.push((
        "analytics-not-configured.html".into(),
        apex_api::html::analytics::render(
            &ctx,
            &AnalyticsPageData::unavailable(
                PUBLISHER,
                VIEW_REVENUE,
                scope.clone(),
                apex_api::analytics::NOT_CONFIGURED_REASON,
            ),
        ),
    ));
    pages.push((
        "analytics-bad-view.html".into(),
        apex_api::html::analytics::render_bad_view(
            &ctx,
            "The view is not one of: revenue, lift, fees, data-quality.",
        ),
    ));

    if pages.len() != PAGES.len() {
        return Err(format!(
            "the renderer produced {} pages but PAGES lists {}",
            pages.len(),
            PAGES.len()
        )
        .into());
    }
    for (name, html) in &pages {
        let expected = PAGES
            .iter()
            .find(|page| *page == name)
            .ok_or_else(|| format!("page {name} is not listed in PAGES"))?;
        std::fs::write(out.join(expected), html)?;
    }
    println!(
        "rendered {} console pages to {}",
        pages.len(),
        out.display()
    );
    Ok(())
}

fn find_finding<'a>(
    report: &'a apex_audit::AuditReport,
    subject: &str,
) -> Result<&'a apex_audit::Finding, Box<dyn std::error::Error>> {
    report
        .findings
        .iter()
        .find(|finding| finding.subject == subject)
        .ok_or_else(|| format!("the fixture dataset lost the finding {subject}").into())
}

fn find_experiment<'a>(
    views: &'a [ExperimentView],
    id: &str,
) -> Result<&'a ExperimentView, Box<dyn std::error::Error>> {
    views
        .iter()
        .find(|view| view.experiment_id == id)
        .ok_or_else(|| format!("the fixture dataset lost the experiment {id}").into())
}

/// Seal one lift-record draft with its honest payload digest.
fn fixture_lift(payload: &str, at_unix_ms: i64) -> LedgerRecord {
    let digest = hex::encode(Sha256::digest(payload.as_bytes()));
    LedgerRecord::lift_record(at_unix_ms, &digest, payload)
}

fn fixture_data_pull(day: &str) -> LedgerRecord {
    fixture_lift(
        &format!(r#"{{"type":"dataPull","day":"{day}","rows_sha256":"ab","rows":[]}}"#),
        NOW_MS - 86_400_000,
    )
}

fn fixture_preregistration(id: &str) -> LedgerRecord {
    fixture_lift(
        &format!(
            r#"{{"type":"preRegistration","experiment_id":"{id}","scope":"rw_de_01","before_value":{{"DE":4000000}},"after_value":{{"DE":3200000}},"split":0.25,"metric":"RevenuePerUser","burn_in_days":3,"window_days":14,"looks":1,"extension_allowed":true,"guardrail_thresholds":{{"harmPoint":-0.05,"fillRewarded":0.02,"fillBanner":0.05,"splitChiSquare":10.83}},"code_version_hash":"apex-lift-0.1.0","rate_bp":2000}}"#
        ),
        NOW_MS - 12 * 86_400_000,
    )
}

fn fixture_started(id: &str, at_unix_ms: i64) -> LedgerRecord {
    LedgerRecord::experiment_started(
        at_unix_ms,
        PlatformId::AppLovinMax,
        id,
        ExperimentSpec {
            name: format!("{id}-de-floor"),
            instance_id: String::new(),
            ad_unit_id: "rw_de_01".into(),
            metric: ExperimentMetric::Ecpm,
            variants: 2,
            test_group_allocation: 25,
            change_description: "raise the DE floor".into(),
        },
    )
}

/// The five experiment states of the illustration, sealed in memory.
fn experiment_fixture_records() -> Result<Vec<LedgerRecord>, Box<dyn std::error::Error>> {
    let day = 86_400_000_i64;
    let day_string = |offset_days: i64| {
        apex_analytics::UtcMillis::from_unix_millis(NOW_MS - offset_days * day).to_day_string()
    };
    let mut drafts: Vec<LedgerRecord> = Vec::new();

    // exp_0247: awaiting final data (4 final days missing).
    drafts.push(fixture_started("exp_0247", NOW_MS - 12 * day));
    drafts.push(fixture_preregistration("exp_0247"));
    for offset in 0..=9 {
        drafts.push(fixture_data_pull(&day_string(offset)));
    }

    // exp_0246: kept, every analysis day final.
    drafts.push(fixture_started("exp_0246", NOW_MS - 30 * day));
    drafts.push(fixture_preregistration("exp_0246"));
    for offset in 14..=27 {
        drafts.push(fixture_data_pull(&day_string(offset)));
    }
    let ratios = [
        0.031, 0.028, 0.035, 0.030, 0.027, 0.033, 0.036, 0.029, 0.031, 0.034, 0.030, 0.032, 0.028,
        0.030,
    ];
    let ratios_json: Vec<String> = ratios.iter().map(|value| format!("{value}")).collect();
    drafts.push(fixture_lift(
        &format!(
            r#"{{"type":"result","experiment_id":"exp_0246","daily_log_ratios":[{}],"mean_log":0.0312,"lower_90":0.012,"lower_99":0.008,"bootstrap_lower":0.011,"decision":"keep","lambda":0.012}}"#,
            ratios_json.join(",")
        ),
        NOW_MS - 2 * day,
    ));

    // exp_0239: restore failed (the alert is supplied by the caller).
    drafts.push(fixture_started("exp_0239", NOW_MS - 14 * day));
    drafts.push(fixture_preregistration("exp_0239"));
    drafts.push(LedgerRecord::change_applied(
        NOW_MS - 13 * day,
        PlatformId::AppLovinMax,
        "rw_de_01",
        ChangeKind::CountryFloorSet {
            country: "DE".into(),
            floor_ecpm: 4_000_000,
        },
        "max-floor:rw_de_01:DE:9",
        "3200000",
    ));

    // exp_0248: stopped by a harm guard, awaiting the rollback record.
    drafts.push(fixture_started("exp_0248", NOW_MS - 9 * day));
    drafts.push(fixture_preregistration("exp_0248"));
    drafts.push(fixture_data_pull(&day_string(6)));
    drafts.push(fixture_lift(
        r#"{"type":"guardrail","day":"2026-10-09","guardrail":"fastHarm","inputs":{"pointLift":-0.061},"action":"rollback"}"#,
        NOW_MS - 6 * day,
    ));
    drafts.push(fixture_lift(
        r#"{"type":"result","experiment_id":"exp_0248","daily_log_ratios":[-0.04,-0.05,-0.06],"mean_log":-0.05,"lower_90":-0.08,"lower_99":-0.09,"bootstrap_lower":-0.082,"decision":"rollback","lambda":0.0}"#,
        NOW_MS - 5 * day,
    ));

    // exp_0249: burn-in.
    drafts.push(fixture_started("exp_0249", NOW_MS - day));
    drafts.push(fixture_preregistration("exp_0249"));

    // exp_0251: decision due.
    drafts.push(fixture_started("exp_0251", NOW_MS - 20 * day));
    drafts.push(fixture_preregistration("exp_0251"));
    for offset in 3..=16 {
        drafts.push(fixture_data_pull(&day_string(offset)));
    }

    let mut ledger = apex_proof::InMemoryLedger::new(Keypair::from_bytes(&TEST_KEY_SEED));
    let mut sealed = Vec::new();
    for draft in drafts {
        sealed.push(ledger.append(draft)?);
    }
    Ok(sealed)
}

/// The analytics data-quality badge of the illustration.
fn fixture_quality() -> DataQuality {
    DataQuality {
        publisher_id: PUBLISHER.to_string(),
        as_of: "2026-10-15T09:10:00.000Z".to_string(),
        matched_source_days: 28,
        missing_days: Some(2),
        provisional_days: 1,
        dedup_rejects: Some(3),
        unmapped_rows: 4,
        stale_sources: vec![StaleSource {
            platform: "levelplay".to_string(),
            platform_account_id: "app-northstar".to_string(),
            last_final_day: Some("2026-10-05".to_string()),
            freshness_days: Some(10),
        }],
        unavailable: Vec::new(),
    }
}

/// The billing register of the illustration: one draft month, one paid month,
/// one partly collected month and one uncollected month.
fn billing_fixture() -> Result<BillingView, Box<dyn std::error::Error>> {
    let billing = |change: &str, month: &str, fee: i64, days: u32| {
        fixture_lift(
            &format!(
                r#"{{"type":"monthlyBilling","changeId":"{change}","month":"{month}","scopeRevenue":51500000000,"lambda":0.03,"factor":1.03,"incremental":1500000000,"logShare":1.0,"rateBp":2000,"feeMicros":{fee},"billedDays":{days},"monthDays":31}}"#
            ),
            NOW_MS - 86_400_000,
        )
    };
    let change_record = fixture_lift(
        r#"{"type":"billingChange","changeId":"ch_0239","day":"2026-08-12","reason":"publisher override","billedLambda":0.0}"#,
        NOW_MS - 3 * 86_400_000,
    );
    let drafts = vec![
        billing("ch_0247", "2026-10", 300_000_000, 31),
        billing("ch_0246", "2026-09", 225_000_000, 30),
        billing("ch_0242", "2026-08", 100_000_000, 12),
        billing("ch_0239", "2026-07", 50_000_000, 31),
        change_record,
    ];
    let mut ledger = apex_proof::InMemoryLedger::new(Keypair::from_bytes(&TEST_KEY_SEED));
    let records: Vec<LedgerRecord> = drafts
        .into_iter()
        .map(|draft| ledger.append(draft))
        .collect::<Result<_, _>>()?;
    let payments = vec![
        apex_billing::CollectionEvent {
            at_unix_ms: NOW_MS - 20 * 86_400_000,
            payment_id: "8815397128890001".to_string(),
            reference: "apex-fee:northstar:exp_0246:2026-09".to_string(),
            status: "paid".to_string(),
            amount_minor_units: 22_500,
            amount_currency: "USD".to_string(),
            stored_customer_id: String::new(),
            stored_mandate_id: String::new(),
        },
        apex_billing::CollectionEvent {
            at_unix_ms: NOW_MS - 10 * 86_400_000,
            payment_id: "8815397128890002".to_string(),
            reference: "apex-fee:northstar:exp_0242:2026-08".to_string(),
            status: "paid".to_string(),
            amount_minor_units: 4_000,
            amount_currency: "USD".to_string(),
            stored_customer_id: String::new(),
            stored_mandate_id: String::new(),
        },
        // A receipt in another currency: excluded from the USD balance and
        // rendered as the explicit exclusion note, never summed in.
        apex_billing::CollectionEvent {
            at_unix_ms: NOW_MS - 9 * 86_400_000,
            payment_id: "8815397128890003".to_string(),
            reference: "apex-fee:northstar:exp_0242:2026-08".to_string(),
            status: "paid".to_string(),
            amount_minor_units: 300,
            amount_currency: "EUR".to_string(),
            stored_customer_id: String::new(),
            stored_mandate_id: String::new(),
        },
    ];
    Ok(apex_api::billing_view::build(
        &records, payments, NOW_MS, "USD",
    ))
}

/// Write the 24-record Northstar ledger with the published test key. The
/// script is deterministic: regeneration produces the same bytes.
fn generate_ledger(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut ledger = Ledger::open(path, Keypair::from_bytes(&TEST_KEY_SEED))?;
    let max = PlatformId::AppLovinMax;
    let levelplay = PlatformId::LevelPlay;
    let admob = PlatformId::Admob;

    let append =
        |ledger: &mut Ledger, draft: LedgerRecord| -> Result<(), Box<dyn std::error::Error>> {
            ledger.append(draft)?;
            Ok(())
        };
    let spec = |name: &str, metric: ExperimentMetric| ExperimentSpec {
        name: name.into(),
        instance_id: String::new(),
        ad_unit_id: "rw_de_01".into(),
        metric,
        variants: 2,
        test_group_allocation: 25,
        change_description: "raise the DE floor".into(),
    };
    let result = |id: &str,
                  state: ExperimentState,
                  lift_bp: Option<i64>,
                  confidence: Option<i32>,
                  winner: Option<&str>|
     -> ExperimentResult {
        ExperimentResult {
            experiment_id: id.into(),
            state,
            results: vec![ExperimentVariantResult {
                variant_id: "variant-a".into(),
                impressions: 50_000,
                revenue: 85_000_000,
                ecpm: 1_700_000,
            }],
            lift_percent_bp: lift_bp,
            confidence,
            winner_variant_id: winner.map(str::to_string),
        }
    };

    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_788_426_000_000,
            max,
            "rw_de_01",
            ChangeKind::CountryFloorSet {
                country: "DE".into(),
                floor_ecpm: 2_500_000,
            },
            "max-floor:rw_de_01:DE:1",
            "0",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::experiment_started(
            1_788_426_060_000,
            max,
            "exp_0246",
            spec("de-floor-raise", ExperimentMetric::Ecpm),
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_789_632_600_000,
            max,
            "rw_de_01",
            ChangeKind::CountryFloorSet {
                country: "DE".into(),
                floor_ecpm: 3_200_000,
            },
            "max-floor:rw_de_01:DE:2",
            "2500000",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::experiment_observed(
            1_790_842_140_000,
            max,
            "exp_0246",
            result(
                "exp_0246",
                ExperimentState::Completed,
                Some(120),
                Some(8_800),
                Some("variant-a"),
            ),
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::experiment_started(
            1_790_842_200_000,
            max,
            "exp_0247",
            spec("floor-test-de", ExperimentMetric::Ecpm),
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_791_100_980_000,
            levelplay,
            "rw_de_01",
            ChangeKind::WaterfallEntryCreated {
                ad_unit_id: "rw_de_01".into(),
                network: "mintegral".into(),
                instance_name: "MTG_1".into(),
                price_ecpm: 2_000_000,
            },
            "levelplay-create:rw_de_01:mintegral:MTG_1",
            "",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_791_101_040_000,
            admob,
            "int_us_01",
            ChangeKind::EcpmSet {
                mediation_group_id: "g_int_us".into(),
                ad_unit_id: "int_us_01".into(),
                ecpm: 1_750_000,
            },
            "admob-ecpm:g_int_us:int_us_01:1.75",
            "1600000",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_791_360_720_000,
            levelplay,
            "rw_de_01",
            ChangeKind::WaterfallEntryPriceSet {
                ad_unit_id: "rw_de_01".into(),
                entry_id: "e_1".into(),
                price_ecpm: 2_350_000,
            },
            "levelplay-price:rw_de_01:e_1",
            "2000000",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_791_360_780_000,
            levelplay,
            "rw_de_01",
            ChangeKind::WaterfallEntryCreated {
                ad_unit_id: "rw_de_01".into(),
                network: "unity".into(),
                instance_name: "UN_1".into(),
                price_ecpm: 1_900_000,
            },
            "levelplay-create:rw_de_01:unity:UN_1",
            "",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::experiment_observed(
            1_791_619_860_000,
            max,
            "exp_0247",
            result(
                "exp_0247",
                ExperimentState::Running,
                Some(260),
                Some(9_200),
                None,
            ),
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_791_619_920_000,
            levelplay,
            "rw_de_01",
            ChangeKind::WaterfallEntryRemoved {
                ad_unit_id: "rw_de_01".into(),
                entry_id: "e_9".into(),
            },
            "levelplay-remove:rw_de_01:e_9",
            "",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_791_792_240_000,
            max,
            "rw_de_01",
            ChangeKind::CountryFloorSet {
                country: "DE".into(),
                floor_ecpm: 3_800_000,
            },
            "max-floor:rw_de_01:DE:3",
            "3200000",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::experiment_started(
            1_791_792_300_000,
            max,
            "exp_0248",
            spec("hold-floor-de", ExperimentMetric::Revenue),
        ),
    )?;
    // Rollback of the 3.80 floor (sequence 11) after the first regression.
    ledger.append_rollback(
        1_791_878_760_000,
        max,
        "rw_de_01",
        11,
        "regression on EU traffic: restore the 3.20 floor",
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_791_878_820_000,
            max,
            "rw_de_01",
            ChangeKind::CountryFloorSet {
                country: "DE".into(),
                floor_ecpm: 3_200_000,
            },
            "max-floor:rw_de_01:DE:4",
            "3800000",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::experiment_observed(
            1_791_965_340_000,
            max,
            "exp_0248",
            result(
                "exp_0248",
                ExperimentState::Stopped,
                Some(40),
                Some(6_100),
                None,
            ),
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_791_967_140_000,
            max,
            "rw_de_01",
            ChangeKind::CountryFloorSet {
                country: "DE".into(),
                floor_ecpm: 4_000_000,
            },
            "max-floor:rw_de_01:DE:5",
            "3200000",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::experiment_observed(
            1_791_967_160_000,
            max,
            "exp_0247",
            result(
                "exp_0247",
                ExperimentState::Completed,
                Some(300),
                Some(9_800),
                Some("variant-a"),
            ),
        ),
    )?;
    append(
        &mut ledger,
        apex_proof::build_billing_proof(
            PUBLISHER,
            "exp_0247",
            10_000_000_000,
            10_300_000_000,
            2_000,
            1_791_967_200_000,
        )?,
    )?;
    let payload =
        r#"{"experimentId":"exp_0247","liftBp":300,"publisher":"northstar","type":"lift"}"#;
    let payload_sha256 = hex::encode(Sha256::digest(payload.as_bytes()));
    append(
        &mut ledger,
        LedgerRecord::lift_record(1_791_967_260_000, &payload_sha256, payload),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_792_055_340_000,
            admob,
            "int_us_01",
            ChangeKind::EcpmSet {
                mediation_group_id: "g_int_us".into(),
                ad_unit_id: "int_us_01".into(),
                ecpm: 1_800_000,
            },
            "admob-ecpm:g_int_us:int_us_01:1.8",
            "1750000",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::experiment_started(
            1_792_055_400_000,
            levelplay,
            "exp_0249",
            ExperimentSpec {
                name: "floor-hold-de".into(),
                instance_id: String::new(),
                ad_unit_id: "rw_de_01".into(),
                metric: ExperimentMetric::Ecpm,
                variants: 2,
                test_group_allocation: 10,
                change_description: "hold the DE floor on LevelPlay".into(),
            },
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::change_applied(
            1_792_055_460_000,
            levelplay,
            "rw_de_01",
            ChangeKind::WaterfallEntryCreated {
                ad_unit_id: "rw_de_01".into(),
                network: "meta".into(),
                instance_name: "FB_1".into(),
                price_ecpm: 2_100_000,
            },
            "levelplay-create:rw_de_01:meta:FB_1",
            "",
        ),
    )?;
    append(
        &mut ledger,
        LedgerRecord::experiment_observed(
            1_792_055_520_000,
            levelplay,
            "exp_0249",
            result("exp_0249", ExperimentState::Running, None, None, None),
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;

    use super::*;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    #[tokio::test]
    async fn renders_every_console_page_from_the_committed_fixtures() {
        let out = std::env::temp_dir().join(format!("apex-fixtures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        super::run(&fixtures_dir(), &out).await.expect("render");
        for page in super::PAGES {
            assert!(out.join(page).exists(), "{page} was not rendered");
        }
        let rendered = std::fs::read_dir(&out).unwrap().count();
        assert_eq!(
            rendered,
            super::PAGES.len(),
            "every rendered file is listed in PAGES"
        );
        let _ = std::fs::remove_dir_all(&out);
    }

    #[tokio::test]
    async fn the_ledger_fixture_is_generated_when_missing_with_24_records() {
        let source = fixtures_dir();
        let dir = std::env::temp_dir().join(format!("apex-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(source.join("admob.json"), dir.join("admob.json")).unwrap();
        assert!(!dir.join("northstar.jsonl").exists());
        let out = dir.join("out");
        super::run(&dir, &out).await.expect("render");
        let ledger = dir.join("northstar.jsonl");
        assert!(ledger.exists(), "the ledger is generated");
        let lines = std::fs::read_to_string(&ledger).unwrap();
        assert_eq!(lines.lines().count(), 24);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_committed_dataset_matches_the_illustration_counts() {
        let fixtures = fixtures_dir();
        let keypair = Keypair::from_bytes(&TEST_KEY_SEED);
        let verified =
            Ledger::read_verified(fixtures.join("northstar.jsonl"), &keypair.verifying_key())
                .expect("the committed fixture ledger verifies");
        assert_eq!(verified.len(), 24, "24 records");

        let snapshot = super::load_fixture_snapshot(&fixtures).expect("snapshot");
        let report = apex_audit::run(&snapshot);
        assert_eq!(report.findings.len(), 5, "5 findings");
        let critical = report
            .findings
            .iter()
            .filter(|finding| finding.severity == apex_audit::Severity::Critical)
            .count();
        assert_eq!(critical, 1, "1 critical finding");
        let covered = apex_api::html::audit::coverage(&snapshot);
        assert_eq!((covered.available, covered.total()), (4, 5));
        assert_eq!(covered.missing, vec!["Disabled bidder"]);
    }
}
