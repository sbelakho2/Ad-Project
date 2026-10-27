//! Adversarial integration tests for the server-rendered console: the token
//! exchange and its cookie flags, CSRF-protected session end, stable finding
//! ids, the CSV export, the free-audit flow and XSS safety for hostile
//! publisher names, finding subjects, record fields and form values (no page
//! may ever contain `<script`).
//!
//! Security invariants pinned here: the ledger token travels only in a POST
//! body (never a URL, never echoed), the routes fail closed (`503` without a
//! configured token, `401` on a wrong one), every dynamic string is escaped,
//! every page carries exactly one `<h1>` and no page contains a script tag.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;

use apex_analytics::store::EventStore;
use apex_api::html::audit::finding_id;
use apex_api::router;
use apex_core::PlatformId;
use apex_proof::{ChangeKind, LedgerRecord};
use axum::body::Body;
use axum::http::header::CONTENT_LENGTH;
use axum::http::Request;
use common::*;

const HTML: &str = "text/html";

const HOSTILE: &str = "<script>alert(1)</script>";
/// `HOSTILE` percent-encoded as a single path/query segment.
const HOSTILE_ENCODED: &str = "%3Cscript%3Ealert(1)%3C%2Fscript%3E";

fn assert_no_raw_script(body: &str, context: &str) {
    // Case-insensitive: a regression that re-introduced markup could emit
    // an uppercase tag matches just as well.
    let lowercase = body.to_ascii_lowercase();
    assert!(
        !lowercase.contains("<script"),
        "{context}: page contains <script"
    );
    assert!(
        !lowercase.contains(&HOSTILE.to_ascii_lowercase()),
        "{context}: page contains the raw hostile string"
    );
}

fn assert_single_h1(body: &str, context: &str) {
    assert_eq!(
        body.matches("<h1").count(),
        1,
        "{context}: expected exactly one h1"
    );
}

// --- overview ---------------------------------------------------------------

#[tokio::test]
async fn overview_page_renders_cards_steps_and_health_links() {
    let temp = TempDir::new("console-root");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(&app, get_with_cookie("/", &cookie)).await;
    assert_eq!(response.status, 200);
    assert!(response.content_type().starts_with(HTML));
    for needle in [
        "<h1>Your account at a glance</h1>",
        "Read-only mode.",
        "No platform settings are changed here.",
        "Latest audit",
        "Verified records",
        "Publisher history is available",
        "Next steps",
        "Inspect the change history",
        "href=\"/console/audit\"",
        "href=\"/console/ledger\"",
    ] {
        assert!(response.body.contains(needle), "missing {needle}");
    }
    assert_single_h1(&response.body, "overview");
    assert_no_raw_script(&response.body, "overview");
}

/// The overview reads each platform's snapshot and report once per request:
/// the three reads run concurrently, and a repeated request is served from
/// the cache with one read per platform (three hits, not six), producing
/// byte-identical output.
#[tokio::test]
async fn overview_reads_each_platform_once_and_renders_identically() {
    let temp = TempDir::new("console-overview-once");
    for platform in ["applovin_max", "levelplay", "admob"] {
        let mut snapshot = minimal_snapshot(&["u1"]);
        snapshot["platform"] = serde_json::json!(platform);
        write_snapshot(temp.path(), platform, &snapshot);
    }
    let audit = Arc::new(apex_api::JsonFileAuditSource::new(temp.path()));
    let ledger = Arc::new(apex_api::FileLedgerSource::new(temp.path(), pub1_keys()));
    let state = apex_api::AppState::new(audit.clone(), ledger)
        .with_ledger_token(PUBLISHER, LEDGER_TOKEN.as_bytes().to_vec())
        .with_clock(Arc::new(apex_api::FixedClock::new(1_700_000_100)));
    let app = router(state);
    let cookie = open_ledger_session(&app).await;

    let first = send(&app, get_with_cookie("/", &cookie)).await;
    assert_eq!(first.status, 200);
    let first_stats = audit.cache_stats();
    assert_eq!(first_stats.parses, 3, "one parse per platform");
    assert_eq!(
        first_stats.hits, 0,
        "the first request reads each file once"
    );
    assert_eq!(first_stats.report_builds, 3, "one audit per platform");

    let second = send(&app, get_with_cookie("/", &cookie)).await;
    assert_eq!(second.status, 200);
    assert_eq!(second.body, first.body, "the page output is unchanged");
    let second_stats = audit.cache_stats();
    assert_eq!(
        second_stats.hits, 3,
        "the second request reads each platform once, not twice"
    );
    assert_eq!(second_stats.parses, 3, "no snapshot is reparsed");
    assert_eq!(
        second_stats.report_builds, 3,
        "no audit is recomputed per request"
    );
}

#[tokio::test]
async fn overview_first_run_explains_the_arrangement() {
    let temp = TempDir::new("console-first-run");
    // No snapshot and no configured publisher ledger: the first-run state,
    // rendered for the session's publisher.
    let state = apex_api::AppState::new(
        std::sync::Arc::new(apex_api::JsonFileAuditSource::new(temp.path())),
        std::sync::Arc::new(apex_api::FileLedgerSource::new(
            temp.path(),
            std::collections::HashMap::new(),
        )),
    )
    .with_ledger_token(PUBLISHER, LEDGER_TOKEN.as_bytes().to_vec());
    let app = router(state);
    let cookie = open_ledger_session(&app).await;
    let response = send(&app, get_with_cookie("/", &cookie)).await;
    assert_eq!(response.status, 200);
    assert!(response.body.contains("No audit snapshot is available."));
    assert!(
        response
            .body
            .contains("Ask your account team to arrange the first audit"),
        "{}",
        response.body
    );
    assert!(response.body.contains("<li>app-ads.txt</li>"));
    assert!(!response.body.contains("class=\"kpi\""));
    assert_single_h1(&response.body, "overview first run");
}

// --- audit ------------------------------------------------------------------

#[tokio::test]
async fn audit_form_is_a_plain_get_form_with_all_platforms() {
    let temp = TempDir::new("console-audit-form");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(&app, get_with_cookie("/console/audit", &cookie)).await;
    assert_eq!(response.status, 200);
    assert!(response.content_type().starts_with(HTML));
    assert!(response
        .body
        .contains("<form method=\"get\" action=\"/console/audit\">"));
    for (name, label) in [
        ("applovin_max", "AppLovin MAX"),
        ("levelplay", "LevelPlay"),
        ("admob", "AdMob"),
    ] {
        assert!(
            response
                .body
                .contains(&format!("<option value=\"{name}\">{label}</option>")),
            "missing option {name}"
        );
    }
    assert!(response
        .body
        .contains("<button class=\"action action-primary\" type=\"submit\">View audit</button>"));
    assert!(response.body.contains("What is checked"));
    assert!(response
        .body
        .contains("<li>Floors and bidder settings</li>"));
    assert_single_h1(&response.body, "audit form");
    assert_no_raw_script(&response.body, "audit form");
}

#[tokio::test]
async fn audit_page_ranks_findings_with_stable_detail_links() {
    let temp = TempDir::new("console-audit-golden");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(
        &app,
        get_with_cookie("/console/audit?platform=admob", &cookie),
    )
    .await;
    assert_eq!(response.status, 200);
    assert!(response.content_type().starts_with(HTML));
    assert!(response.body.contains("<h1>AdMob audit</h1>"));
    assert!(response.body.contains(">Snapshot<"));
    for header in ["Priority", "Finding", "Scope", "Estimate"] {
        assert!(
            response
                .body
                .contains(&format!("<th scope=\"col\">{header}</th>")),
            "missing {header}"
        );
    }
    let id = finding_id("stale_floor", "admob", "ad_unit:u1:country:EE");
    let link = format!("/console/audit/findings/{id}");
    assert!(response.body.contains(&link), "missing {link}");
    assert!(response.body.contains("Total estimated monthly recovery:"));
    assert!(response.body.contains("Export the filtered report (CSV)"));
    assert_single_h1(&response.body, "audit report");
    assert_no_raw_script(&response.body, "audit report");

    // The finding id is stable across renders and the link resolves.
    let again = send(
        &app,
        get_with_cookie("/console/audit?platform=admob", &cookie),
    )
    .await;
    assert!(again.body.contains(&link));
    let detail = send(&app, get_with_cookie(&link, &cookie)).await;
    assert_eq!(detail.status, 200, "{}", detail.body);
    assert!(detail.body.contains("Estimated monthly recovery:"));
    assert!(detail.body.contains("The source evidence"));
    assert!(detail.body.contains("Snapshot digest: <code>"));
    assert!(detail
        .body
        .contains("Test options appear only when the action is supported and authorized."));
    assert_single_h1(&detail.body, "finding detail");
    assert_no_raw_script(&detail.body, "finding detail");
}

#[tokio::test]
async fn audit_finding_detail_escapes_hostile_subjects() {
    let temp = TempDir::new("console-audit-finding-xss");
    write_snapshot(temp.path(), "admob", &minimal_snapshot(&[HOSTILE]));
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let id = finding_id(
        "stale_floor",
        "admob",
        &format!("ad_unit:{HOSTILE}:country:EE"),
    );
    let response = send(
        &app,
        get_with_cookie(&format!("/console/audit/findings/{id}"), &cookie),
    )
    .await;
    assert_eq!(response.status, 200);
    assert!(response
        .body
        .contains("ad_unit:&lt;script&gt;alert(1)&lt;/script&gt;:country:EE"));
    assert_no_raw_script(&response.body, "finding detail with hostile subject");
}

#[tokio::test]
async fn audit_unknown_finding_id_is_a_404() {
    let temp = TempDir::new("console-audit-finding-404");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    for id in ["f_000000000000", "not-an-id", "f_0123456789ab"] {
        let response = send(
            &app,
            get_with_cookie(&format!("/console/audit/findings/{id}"), &cookie),
        )
        .await;
        assert_eq!(response.status, 404, "{id}");
        assert!(
            response.body.contains("No finding matches this address."),
            "{id}"
        );
        assert_no_raw_script(&response.body, id);
    }
}

#[tokio::test]
async fn audit_unknown_platform_renders_escaped_banner() {
    let temp = TempDir::new("console-audit-400");
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(
        &app,
        get_with_cookie(
            &format!("/console/audit?platform={HOSTILE_ENCODED}"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(response.status, 400);
    assert!(response.content_type().starts_with(HTML));
    assert!(response.body.contains("Check the request."));
    assert!(response
        .body
        .contains("A field or upload could not be accepted."));
    assert!(response
        .body
        .contains("Received platform: <code>&lt;script&gt;alert(1)&lt;/script&gt;</code>"));
    assert_no_raw_script(&response.body, "audit unknown platform");
    assert!(response
        .body
        .contains("<form method=\"get\" action=\"/console/audit\">"));
}

#[tokio::test]
async fn audit_missing_snapshot_renders_the_frozen_404_with_the_selected_platform() {
    let temp = TempDir::new("console-audit-404");
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(
        &app,
        get_with_cookie("/console/audit?platform=levelplay", &cookie),
    )
    .await;
    assert_eq!(response.status, 404);
    assert!(response.body.contains("No audit snapshot is available."));
    assert!(response
        .body
        .contains("This platform has no report in the current workspace."));
    assert!(response
        .body
        .contains("<option value=\"levelplay\" selected>LevelPlay</option>"));
    assert_no_raw_script(&response.body, "audit missing snapshot");
}

#[tokio::test]
async fn csv_export_is_an_attachment_with_rfc4180_quoting_and_utc_dates() {
    let temp = TempDir::new("console-audit-csv");
    write_snapshot(
        temp.path(),
        "admob",
        &minimal_snapshot(&[HOSTILE, "a,b\"c"]),
    );
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(
        &app,
        get_with_cookie("/console/audit/export.csv?platform=admob", &cookie),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert!(response.content_type().starts_with("text/csv"));
    let disposition = response
        .headers
        .get("content-disposition")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(disposition.contains("attachment"), "{disposition}");
    assert!(disposition.contains("audit-admob.csv"), "{disposition}");
    assert!(response
        .body
        .starts_with("severity,rule,platform,subject,finding_id,message,"));
    // A subject containing a comma and a quote is quoted, quote doubled.
    assert!(
        response.body.contains("\"ad_unit:a,b\"\"c:country:EE\""),
        "{}",
        response.body
    );
    // A subject containing markup keeps it as inert CSV text.
    assert!(response
        .body
        .contains("ad_unit:<script>alert(1)</script>:country:EE"));
    assert!(response.body.contains(" UTC"));
    assert!(response.body.ends_with("\r\n"));

    // Missing or unknown filters answer the useful HTML 400, never CSV.
    for uri in [
        "/console/audit/export.csv",
        "/console/audit/export.csv?platform=tiktok",
    ] {
        let response = send(&app, get_with_cookie(uri, &cookie)).await;
        assert_eq!(response.status, 400, "{uri}");
        assert!(response.content_type().starts_with(HTML), "{uri}");
        assert!(response.body.contains("Check the request."), "{uri}");
        assert!(response.body.contains("Back to the overview"), "{uri}");
    }
    let response = send(
        &app,
        get_with_cookie("/console/audit/export.csv?platform=levelplay", &cookie),
    )
    .await;
    assert_eq!(response.status, 404);
    assert!(response.body.contains("No audit snapshot is available."));
}

// --- session gate ------------------------------------------------------------

/// Every console GET route that must require a live session, with a
/// well-formed placeholder for each path parameter.
const SESSION_REQUIRED_GET_ROUTES: &[&str] = &[
    "/",
    "/console/audit",
    "/console/audit?platform=admob",
    "/console/audit/export.csv?platform=admob",
    "/console/audit/findings/f_0123456789ab",
    "/console/ledger/records/0",
    "/console/connections",
    "/console/connections/admob",
    "/console/proposals",
    "/console/proposals/p_missing",
    "/console/experiments",
    "/console/experiments/exp_missing",
    "/console/experiments/exp_missing/incident",
    "/console/billing",
    "/console/billing/invoices/2026-10",
    "/console/settings",
    "/console/settings/access",
    "/console/settings/policy",
    "/console/settings/monitoring",
    "/console/analytics",
];

/// The billing record that gives [`PUBLISHER2`] its own fee-bearing month.
fn billing_record() -> LedgerRecord {
    use sha2::{Digest, Sha256};
    let payload = r#"{"type":"monthlyBilling","changeId":"ch_9999","month":"2026-10","scopeRevenue":51500000000,"lambda":0.03,"factor":1.03,"incremental":1500000000,"logShare":1.0,"rateBp":2000,"feeMicros":300000000,"billedDays":31,"monthDays":31}"#;
    let digest = hex::encode(Sha256::digest(payload.as_bytes()));
    LedgerRecord::lift_record(1_792_000_000_000, &digest, payload)
}

#[tokio::test]
async fn every_console_get_without_a_session_redirects_to_the_access_panel() {
    let temp = TempDir::new("console-gate");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    for uri in SESSION_REQUIRED_GET_ROUTES {
        let response = send(&app, get(uri)).await;
        assert_eq!(response.status, 303, "{uri}: {}", response.body);
        assert_eq!(
            response
                .headers
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some("/console/ledger"),
            "{uri}"
        );
    }

    // The documented exceptions stay reachable: the access panel itself is
    // the entry point (200), the bare console address redirects there, and
    // the free-audit flow is public marketing.
    let panel = send(&app, get("/console/ledger")).await;
    assert_eq!(panel.status, 200);
    assert!(panel.body.contains("<h1>Open your signed change log</h1>"));
    let bare = send(&app, get("/console")).await;
    assert_eq!(bare.status, 303);
    assert_eq!(
        bare.headers
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/console/ledger")
    );
    assert_eq!(send(&app, get("/free-audit/")).await.status, 200);
    assert_eq!(send(&app, get("/audit-requested/")).await.status, 200);
}

#[tokio::test]
async fn every_console_get_renders_for_the_live_session_publisher() {
    let temp = TempDir::new("console-gate-session");
    write_standard_fixture(temp.path());
    let proposals = temp.path().join("proposals");
    std::fs::create_dir_all(&proposals).expect("proposals dir");
    let store = Arc::new(apex_analytics::MemoryStore::new());
    store.ensure_schema().await.expect("schema");
    let analytics = apex_api::AnalyticsAccess::enabled(
        Arc::new(apex_analytics::MemoryReader::new((*store).clone())),
        apex_analytics::AnalyticsClient::new(
            {
                let concrete: Arc<apex_analytics::MemoryStore> = Arc::clone(&store);
                let store: Arc<dyn apex_analytics::store::EventStore> = concrete;
                store
            },
            "test",
            "apex-api-test",
        ),
    );
    let state = file_state(temp.path(), pub1_keys())
        .with_proposal_store(Arc::new(apex_api::proposals::FileProposalStore::new(
            &proposals,
        )))
        .with_analytics(analytics);
    let app = router(state);
    let cookie = open_ledger_session(&app).await;

    let cases: &[(&str, u16)] = &[
        ("/", 200),
        ("/console/audit", 200),
        ("/console/audit?platform=admob", 200),
        ("/console/audit/findings/f_0123456789ab", 404),
        ("/console/ledger/records/0", 200),
        ("/console/connections", 200),
        ("/console/connections/admob", 200),
        ("/console/proposals", 200),
        ("/console/proposals/p_missing", 404),
        ("/console/experiments", 200),
        ("/console/experiments/exp_missing", 404),
        ("/console/experiments/exp_missing/incident", 404),
        ("/console/billing", 200),
        ("/console/billing/invoices/2026-10", 404),
        ("/console/settings", 200),
        ("/console/settings/access", 200),
        ("/console/settings/policy", 200),
        ("/console/settings/monitoring", 200),
        ("/console/analytics", 200),
    ];
    for (uri, expected) in cases {
        let response = send(&app, get_with_cookie(uri, &cookie)).await;
        assert_eq!(
            response.status.as_u16(),
            *expected,
            "{uri}: {} {}",
            response.status,
            response.body
        );
        assert!(response.content_type().starts_with(HTML), "{uri}");
        assert_no_raw_script(&response.body, uri);
    }
    // The CSV export renders its machine shape for the session too.
    let csv = send(
        &app,
        get_with_cookie("/console/audit/export.csv?platform=admob", &cookie),
    )
    .await;
    assert_eq!(csv.status, 200, "{}", csv.body);
    assert!(
        csv.content_type().starts_with("text/csv"),
        "{}",
        csv.content_type()
    );
}

#[tokio::test]
async fn a_deployment_without_console_credentials_keeps_the_503_panels() {
    // No ledger tokens and no stores: no session can ever be opened, so the
    // workspace pages keep the not-configured panel instead of redirecting to
    // an access panel that could not authorize anyone.
    let temp = TempDir::new("console-no-auth");
    let state = apex_api::AppState::new(
        std::sync::Arc::new(apex_api::JsonFileAuditSource::new(temp.path())),
        std::sync::Arc::new(apex_api::FileLedgerSource::new(
            temp.path(),
            std::collections::HashMap::new(),
        )),
    );
    let app = router(state);
    for uri in [
        "/console/connections",
        "/console/connections/admob",
        "/console/proposals",
        "/console/experiments",
        "/console/billing",
        "/console/settings",
        "/console/settings/access",
        "/console/settings/policy",
        "/console/settings/monitoring",
        "/console/analytics",
    ] {
        let response = send(&app, get(uri)).await;
        assert_eq!(response.status, 503, "{uri}: {}", response.body);
        assert!(
            response
                .body
                .contains("This view is not available in this deployment."),
            "{uri}"
        );
    }
}

#[tokio::test]
async fn two_sessions_see_their_own_publishers_data() {
    let temp = TempDir::new("console-two-sessions");
    write_standard_fixture(temp.path());
    write_ledger(
        temp.path(),
        PUBLISHER2,
        &seed_keypair(9),
        vec![billing_record()],
    );
    let state = file_state(temp.path(), two_publisher_keys())
        .with_ledger_token(PUBLISHER2, LEDGER_TOKEN2.as_bytes().to_vec());
    let app = router(state);
    let cookie1 = open_ledger_session(&app).await;
    let exchange = send(
        &app,
        post_form(
            "/console/ledger",
            &format!("publisher={PUBLISHER2}&token={LEDGER_TOKEN2}"),
        ),
    )
    .await;
    assert_eq!(exchange.status, 303, "second publisher exchange");
    let cookie2 = set_cookie(&exchange)
        .expect("second session cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    let page1 = send(&app, get_with_cookie("/console/ledger", &cookie1)).await;
    assert_eq!(page1.status, 200);
    assert!(page1.body.contains("Publisher pub1"), "{}", page1.body);
    assert!(page1.body.contains("2 records / Verified"));
    assert!(!page1.body.contains("Publisher pub2"));

    let page2 = send(&app, get_with_cookie("/console/ledger", &cookie2)).await;
    assert_eq!(page2.status, 200);
    assert!(page2.body.contains("Publisher pub2"), "{}", page2.body);
    assert!(page2.body.contains("1 record / Verified"));
    assert!(!page2.body.contains("Publisher pub1"));

    // Billing reads the session's publisher too: pub2's recorded month
    // renders its signed fee, pub1's register never shows it.
    let billing2 = send(&app, get_with_cookie("/console/billing", &cookie2)).await;
    assert_eq!(billing2.status, 200, "{}", billing2.body);
    assert!(billing2.body.contains("$300.00"), "{}", billing2.body);
    let billing1 = send(&app, get_with_cookie("/console/billing", &cookie1)).await;
    assert_eq!(billing1.status, 200, "{}", billing1.body);
    assert!(!billing1.body.contains("$300.00"), "{}", billing1.body);
}

#[tokio::test]
async fn console_audit_surfaces_are_scoped_to_the_session_publisher() {
    let temp = TempDir::new("console-audit-isolation");
    write_standard_fixture(temp.path());
    write_snapshot_for(
        temp.path(),
        PUBLISHER2,
        "admob",
        &minimal_snapshot(&["pub2-secret-unit", "pub2-second-unit"]),
    );
    write_ledger(
        temp.path(),
        PUBLISHER2,
        &seed_keypair(9),
        vec![billing_record()],
    );
    let state = file_state(temp.path(), two_publisher_keys())
        .with_ledger_token(PUBLISHER2, LEDGER_TOKEN2.as_bytes().to_vec());
    let app = router(state);
    let cookie1 = open_ledger_session(&app).await;
    let exchange = send(
        &app,
        post_form(
            "/console/ledger",
            &format!("publisher={PUBLISHER2}&token={LEDGER_TOKEN2}"),
        ),
    )
    .await;
    assert_eq!(exchange.status, 303, "second publisher exchange");
    let cookie2 = set_cookie(&exchange)
        .expect("second session cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    // The overview's newest-audit card is built only from the session's own
    // snapshots: pub1's one-unit snapshot and pub2's two-unit snapshot have
    // different finding counts, and neither page renders the other's data.
    let overview1 = send(&app, get_with_cookie("/", &cookie1)).await;
    assert_eq!(overview1.status, 200, "{}", overview1.body);
    assert!(overview1.body.contains("2 findings"), "{}", overview1.body);
    assert!(
        !overview1.body.contains("pub2-secret-unit"),
        "{}",
        overview1.body
    );
    let overview2 = send(&app, get_with_cookie("/", &cookie2)).await;
    assert_eq!(overview2.status, 200, "{}", overview2.body);
    assert!(overview2.body.contains("3 findings"), "{}", overview2.body);
    assert!(!overview2.body.contains("unit u1"), "{}", overview2.body);

    // Every audit surface reads the session's own snapshot only.
    for (cookie, own, other) in [
        (&cookie1, "u1", "pub2-secret-unit"),
        (&cookie2, "pub2-secret-unit", "u1"),
    ] {
        let report = send(
            &app,
            get_with_cookie("/console/audit?platform=admob", cookie),
        )
        .await;
        assert_eq!(report.status, 200, "{}", report.body);
        assert!(report.body.contains(own), "missing {own}: {}", report.body);
        assert!(
            !report.body.contains(other),
            "another publisher's finding leaked into the audit page: {}",
            report.body
        );
        let csv = send(
            &app,
            get_with_cookie("/console/audit/export.csv?platform=admob", cookie),
        )
        .await;
        assert_eq!(csv.status, 200, "{}", csv.body);
        assert!(csv.body.contains(own), "missing {own}: {}", csv.body);
        assert!(
            !csv.body.contains(other),
            "another publisher's data leaked into the CSV: {}",
            csv.body
        );
    }

    // A finding id belonging to pub2 is not found for pub1's session, and is
    // found for pub2's own session.
    let pub2_finding = finding_id(
        "stale_floor",
        "admob",
        "ad_unit:pub2-secret-unit:country:EE",
    );
    let leaked = send(
        &app,
        get_with_cookie(&format!("/console/audit/findings/{pub2_finding}"), &cookie1),
    )
    .await;
    assert_eq!(leaked.status, 404, "{}", leaked.body);
    assert!(!leaked.body.contains("pub2-secret-unit"));
    let own = send(
        &app,
        get_with_cookie(&format!("/console/audit/findings/{pub2_finding}"), &cookie2),
    )
    .await;
    assert_eq!(own.status, 200, "{}", own.body);
    assert!(own.body.contains("pub2-secret-unit"));
}

// --- change-log access and session ------------------------------------------

#[tokio::test]
async fn ledger_access_panel_is_a_post_form_that_never_echoes_query_secrets() {
    let temp = TempDir::new("console-ledger-panel");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        get(&format!(
            "/console/ledger?publisher={PUBLISHER}&token={LEDGER_TOKEN}"
        )),
    )
    .await;
    assert_eq!(response.status, 200);
    assert!(response.content_type().starts_with(HTML));
    assert!(response
        .body
        .contains("<h1>Open your signed change log</h1>"));
    assert!(response
        .body
        .contains("Records are verified before they are displayed."));
    assert!(response
        .body
        .contains("<form method=\"post\" action=\"/console/ledger\">"));
    assert!(
        !response.body.contains("method=\"get\""),
        "the token never rides a GET"
    );
    assert!(response.body.contains("type=\"password\""));
    assert!(response
        .body
        .contains("Sent in the request body, never in the URL."));
    assert!(
        !response.body.contains(LEDGER_TOKEN),
        "the query token must not be echoed"
    );
    assert!(!response.body.contains("token="), "no token in any URL");
    assert_single_h1(&response.body, "ledger access panel");
    assert_no_raw_script(&response.body, "ledger access panel");
}

#[tokio::test]
async fn ledger_open_exchanges_the_token_for_an_httponly_session_cookie() {
    let temp = TempDir::new("console-ledger-open");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        post_form(
            "/console/ledger",
            &format!("publisher={PUBLISHER}&token={LEDGER_TOKEN}"),
        ),
    )
    .await;
    assert_eq!(response.status, 303, "{}", response.body);
    assert_eq!(
        response
            .headers
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/console/ledger")
    );
    assert!(
        !response.body.contains(LEDGER_TOKEN),
        "the token never reaches the response"
    );
    let cookie = set_cookie(&response).expect("session cookie");
    assert!(cookie.starts_with("apex_ledger_session="), "{cookie}");
    for attribute in ["HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(cookie.contains(attribute), "{cookie}");
    }
    assert!(!cookie.contains("Domain="), "host-only: {cookie}");
    assert!(!cookie.contains("Secure"), "http request: {cookie}");
    let value = cookie
        .trim_start_matches("apex_ledger_session=")
        .split(';')
        .next()
        .expect("cookie value");
    assert_eq!(value.len(), 32, "opaque 128-bit id: {value}");
    assert!(value.bytes().all(|b| b.is_ascii_hexdigit()));

    // HTTPS (behind the TLS proxy) adds the Secure attribute.
    let secure = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/console/ledger")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("x-forwarded-proto", "https")
            .body(Body::from(format!(
                "publisher={PUBLISHER}&token={LEDGER_TOKEN}"
            )))
            .expect("request"),
    )
    .await;
    assert_eq!(secure.status, 303);
    let cookie = set_cookie(&secure).expect("session cookie");
    assert!(cookie.ends_with("; Secure"), "{cookie}");
}

#[tokio::test]
async fn session_cookie_path_reaches_the_overview_like_a_browser() {
    let temp = TempDir::new("console-cookie-path");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let exchange = send(
        &app,
        post_form(
            "/console/ledger",
            &format!("publisher={PUBLISHER}&token={LEDGER_TOKEN}"),
        ),
    )
    .await;
    assert_eq!(exchange.status, 303, "{}", exchange.body);
    let cookie = set_cookie(&exchange).expect("session cookie");
    let path = cookie_path(&cookie);
    assert_eq!(path, "/", "the session covers the whole console origin");
    assert!(!cookie.contains("Domain="), "host-only: {cookie}");
    let pair = cookie.split(';').next().expect("cookie pair").to_string();

    // A cookie jar only sends the cookie when its Path matches. This is the
    // exact browser rule that made `Path=/console` leave `/` unreachable.
    let overview = send(
        &app,
        if cookie_path_matches(path, "/") {
            get_with_cookie("/", &pair)
        } else {
            get("/")
        },
    )
    .await;
    assert_eq!(overview.status, 200, "{}", overview.body);
    assert!(overview.body.contains("<h1>Your account at a glance</h1>"));

    let ledger = send(
        &app,
        if cookie_path_matches(path, "/console/ledger") {
            get_with_cookie("/console/ledger", &pair)
        } else {
            get("/console/ledger")
        },
    )
    .await;
    assert_eq!(ledger.status, 200, "{}", ledger.body);
    assert!(ledger.body.contains("<h1>Signed change log</h1>"));

    // The cookie is host-only, not path-only: another path on this host would
    // receive it too (every route still enforces the session itself).
    assert!(
        cookie_path_matches(path, "/healthz"),
        "a host-only cookie is sent to any path on the same host"
    );
}

/// The `Path` attribute of a `Set-Cookie` value.
fn cookie_path(cookie: &str) -> &str {
    cookie
        .split(';')
        .find_map(|part| part.trim().strip_prefix("Path="))
        .expect("the cookie carries a Path")
}

/// RFC 6265 section 5.1.4 path-match.
fn cookie_path_matches(cookie_path: &str, request_path: &str) -> bool {
    if request_path == cookie_path {
        return true;
    }
    if !request_path.starts_with(cookie_path) {
        return false;
    }
    cookie_path.ends_with('/') || request_path.as_bytes()[cookie_path.len()] == b'/'
}

#[tokio::test]
async fn ledger_access_fails_closed_on_bad_input_and_missing_configuration() {
    let temp = TempDir::new("console-ledger-401");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));

    let wrong = send(
        &app,
        post_form(
            "/console/ledger",
            &format!("publisher={PUBLISHER}&token=not-the-token"),
        ),
    )
    .await;
    assert_eq!(wrong.status, 401);
    assert!(wrong
        .body
        .contains("We could not open this publisher&#39;s history."));
    assert!(wrong
        .body
        .contains("Check the access details or contact support."));
    assert!(
        wrong.body.contains(&format!("value=\"{PUBLISHER}\"")),
        "safe value preserved"
    );
    assert!(
        !wrong.body.contains("not-the-token"),
        "the secret is never echoed"
    );

    let missing_token = send(
        &app,
        post_form("/console/ledger", &format!("publisher={PUBLISHER}")),
    )
    .await;
    assert_eq!(missing_token.status, 401);

    let bad_publisher = send(
        &app,
        post_form("/console/ledger", "publisher=%2E%2E&token=x"),
    )
    .await;
    assert_eq!(bad_publisher.status, 400);
    assert!(bad_publisher.body.contains("Enter a valid publisher ID."));
    assert!(bad_publisher.body.contains("aria-invalid=\"true\""));
    assert!(
        bad_publisher.body.contains("value=\"..\""),
        "safe preserved value"
    );

    // Without configured tokens the route is 503, never an unauthenticated read.
    let unconfigured = router(apex_api::AppState::new(
        std::sync::Arc::new(apex_api::JsonFileAuditSource::new(temp.path())),
        std::sync::Arc::new(apex_api::FileLedgerSource::new(temp.path(), pub1_keys())),
    ));
    let response = send(
        &unconfigured,
        post_form(
            "/console/ledger",
            &format!("publisher={PUBLISHER}&token={LEDGER_TOKEN}"),
        ),
    )
    .await;
    assert_eq!(response.status, 503);
    assert!(response
        .body
        .contains("This view is not available in this deployment."));
    assert!(response
        .body
        .contains("Contact the account team for access."));
    assert_no_raw_script(&response.body, "ledger 503");
}

#[tokio::test]
async fn ledger_session_lists_verified_records_with_human_labels() {
    let temp = TempDir::new("console-ledger-table");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(&app, get_with_cookie("/console/ledger", &cookie)).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert!(response.body.contains("<h1>Signed change log</h1>"));
    assert!(response.body.contains(">Verified<"));
    assert!(response.body.contains("records / Verified"));
    assert!(response.body.contains("Publisher pub1"));
    for header in ["When (UTC)", "Event", "Scope", "Result"] {
        assert!(
            response
                .body
                .contains(&format!("<th scope=\"col\">{header}</th>")),
            "missing {header}"
        );
    }
    assert!(response.body.contains(">Change applied<"));
    assert!(response.body.contains("/console/ledger/records/0"));
    assert!(response
        .body
        .contains("action=\"/console/ledger/session/end\""));
    assert!(response.body.contains("name=\"csrf_token\""));
    // The live session is renewed on every read.
    assert!(set_cookie(&response).is_some(), "session cookie renewed");
    assert_single_h1(&response.body, "ledger records");
    assert_no_raw_script(&response.body, "ledger records");

    let detail = send(&app, get_with_cookie("/console/ledger/records/0", &cookie)).await;
    assert_eq!(detail.status, 200, "{}", detail.body);
    assert!(detail.body.contains("<h1>Change applied / u1</h1>"));
    assert!(detail.body.contains("Record 0 of 2"));
    assert!(detail.body.contains("Technical details"));
    assert!(detail.body.contains("Previous hash: <code>"));
    assert!(detail.body.contains("Digest: <code>"));
    assert!(detail
        .body
        .contains("verified as part of the whole-file verification"));
    assert!(detail
        .body
        .contains("&quot;type&quot;:&quot;changeApplied&quot;"));
    assert!(detail.body.contains("Back to the change log"));
    assert_single_h1(&detail.body, "record detail");
    assert_no_raw_script(&detail.body, "record detail");

    let missing = send(
        &app,
        get_with_cookie("/console/ledger/records/999", &cookie),
    )
    .await;
    assert_eq!(missing.status, 404);

    // Without a session the record route answers the stable recovery redirect.
    let anonymous = send(&app, get("/console/ledger/records/0")).await;
    assert_eq!(anonymous.status, 303);
    assert_eq!(
        anonymous
            .headers
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/console/ledger")
    );
}

/// The record detail borrows the cached chain: serving it (twice) never
/// materializes the whole ledger, and the second render is byte-identical.
#[tokio::test]
async fn ledger_record_detail_uses_the_shared_cache_without_materializing() {
    let temp = TempDir::new("console-ledger-shared");
    write_standard_fixture(temp.path());
    let ledger = Arc::new(apex_api::FileLedgerSource::new(temp.path(), pub1_keys()));
    let state = apex_api::AppState::new(
        Arc::new(apex_api::JsonFileAuditSource::new(temp.path())),
        ledger.clone(),
    )
    .with_ledger_token(PUBLISHER, LEDGER_TOKEN.as_bytes().to_vec())
    .with_clock(Arc::new(apex_api::FixedClock::new(1_700_000_100)));
    let app = router(state);
    let cookie = open_ledger_session(&app).await;

    let first = send(&app, get_with_cookie("/console/ledger/records/0", &cookie)).await;
    assert_eq!(first.status, 200, "{}", first.body);
    let second = send(&app, get_with_cookie("/console/ledger/records/0", &cookie)).await;
    assert_eq!(second.status, 200, "{}", second.body);
    assert_eq!(second.body, first.body, "the page output is unchanged");
    let stats = ledger.cache_stats();
    assert_eq!(stats.verifications, 1, "the chain is verified once");
    assert_eq!(
        stats.materializations, 0,
        "the detail page borrows the shared chain"
    );
}

#[tokio::test]
async fn ledger_list_paginates_at_one_hundred_rows_with_stable_links() {
    let temp = TempDir::new("console-ledger-pages");
    write_snapshot(temp.path(), "admob", &minimal_snapshot(&["u1"]));
    write_ledger(
        temp.path(),
        PUBLISHER,
        &seed_keypair(7),
        (0..2_000u64).map(change_record).collect(),
    );
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;

    // Page 1: exactly 100 data rows plus the header row, the total in the
    // summary and a Next link that round-trips.
    let first = send(&app, get_with_cookie("/console/ledger", &cookie)).await;
    assert_eq!(first.status, 200, "{}", first.body);
    assert!(first.body.contains("2,000 records / Verified"));
    assert!(first.body.contains("Showing 1-100 of 2,000 records"));
    assert_eq!(first.body.matches("<tr>").count(), 101, "header + 100 rows");
    assert!(first
        .body
        .contains("rel=\"next\" href=\"/console/ledger?page=2\""));
    assert!(!first.body.contains("rel=\"prev\""));
    assert!(first.body.contains("/console/ledger/records/0"));
    assert!(!first.body.contains("/console/ledger/records/100"));

    // The KPI stays the whole ledger total, not the page size.
    let overview = send(&app, get_with_cookie("/", &cookie)).await;
    assert_eq!(overview.status, 200, "{}", overview.body);
    assert!(
        overview.body.contains(">2000</a>"),
        "the verified-records KPI is the total: {}",
        overview.body
    );

    // Page 2 round-trips to both neighbours.
    let second = send(&app, get_with_cookie("/console/ledger?page=2", &cookie)).await;
    assert_eq!(second.status, 200, "{}", second.body);
    assert!(second.body.contains("Showing 101-200 of 2,000 records"));
    assert!(second
        .body
        .contains("rel=\"prev\" href=\"/console/ledger?page=1\""));
    assert!(second
        .body
        .contains("rel=\"next\" href=\"/console/ledger?page=3\""));
    assert_eq!(second.body.matches("<tr>").count(), 101);

    // The last page has the remainder and no Next link.
    let last = send(&app, get_with_cookie("/console/ledger?page=20", &cookie)).await;
    assert_eq!(last.status, 200, "{}", last.body);
    assert!(last.body.contains("Showing 1,901-2,000 of 2,000 records"));
    assert!(last
        .body
        .contains("rel=\"prev\" href=\"/console/ledger?page=19\""));
    assert!(!last.body.contains("rel=\"next\""));
    assert!(last.body.contains("/console/ledger/records/1999"));
    assert_eq!(last.body.matches("<tr>").count(), 101);

    // Invalid pages are the typed 400 page: non-numeric, zero, past the last
    // page, empty, and a duplicated key. The rejected value is never echoed
    // and the recovery link is the stable first page.
    for uri in [
        "/console/ledger?page=abc",
        "/console/ledger?page=0",
        "/console/ledger?page=21",
        "/console/ledger?page=",
        "/console/ledger?page=2&page=3",
    ] {
        let bad = send(&app, get_with_cookie(uri, &cookie)).await;
        assert_eq!(bad.status, 400, "{uri}: {}", bad.body);
        assert!(bad.body.contains("Check the request."), "{uri}");
        assert!(
            bad.body.contains("href=\"/console/ledger?page=1\""),
            "{uri}: {}",
            bad.body
        );
        assert!(!bad.body.contains("page=abc"), "{uri}");
        assert_single_h1(&bad.body, uri);
    }

    // The record detail route is unchanged and still addresses the whole
    // chain, including the last record.
    let detail = send(
        &app,
        get_with_cookie("/console/ledger/records/1999", &cookie),
    )
    .await;
    assert_eq!(detail.status, 200, "{}", detail.body);
    assert!(detail.body.contains("Record 1999 of 2000"));
}

#[tokio::test]
async fn ledger_session_end_requires_csrf_and_clears_the_cookie() {
    let temp = TempDir::new("console-ledger-csrf");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let records = send(&app, get_with_cookie("/console/ledger", &cookie)).await;
    let csrf = extract_csrf(&records.body);

    let rejected = send(
        &app,
        post_form_with_cookie(
            "/console/ledger/session/end",
            "csrf_token=wrong-token",
            &cookie,
        ),
    )
    .await;
    assert_eq!(rejected.status, 400, "{}", rejected.body);
    assert!(rejected.body.contains("Check the request."));
    // The session survives a rejected CSRF attempt.
    let alive = send(&app, get_with_cookie("/console/ledger", &cookie)).await;
    assert_eq!(alive.status, 200);
    assert!(alive.body.contains("<h1>Signed change log</h1>"));

    let ended = send(
        &app,
        post_form_with_cookie(
            "/console/ledger/session/end",
            &format!("csrf_token={csrf}"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(ended.status, 303);
    let cleared = set_cookie(&ended).expect("clearing cookie");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert!(cleared.contains("HttpOnly"), "{cleared}");
    let after = send(&app, get_with_cookie("/console/ledger", &cookie)).await;
    assert_eq!(after.status, 200);
    assert!(after.body.contains("<h1>Open your signed change log</h1>"));
}

#[tokio::test]
async fn expired_session_asks_for_the_access_details_again() {
    let temp = TempDir::new("console-ledger-expired");
    write_standard_fixture(temp.path());
    let sessions = std::sync::Arc::new(apex_api::LedgerSessions::new(4, apex_api::SESSION_IDLE_MS));
    // A session opened long ago (1970) is idle-expired by now.
    let stale = sessions.open(PUBLISHER, 1_000);
    let app = router(file_state(temp.path(), pub1_keys()).with_ledger_sessions(sessions));

    let response = send(
        &app,
        get_with_cookie(
            "/console/ledger",
            &format!("apex_ledger_session={}", stale.id),
        ),
    )
    .await;
    assert_eq!(response.status, 200);
    assert!(response.body.contains("Your access session has ended."));
    assert!(response
        .body
        .contains("Enter the access details again to continue."));
    assert!(response
        .body
        .contains("<h1>Open your signed change log</h1>"));
    // The expired entry was dropped: a plain visit shows no expiry notice.
    let plain = send(&app, get("/console/ledger")).await;
    assert!(!plain.body.contains("Your access session has ended."));
}

#[tokio::test]
async fn empty_ledger_renders_the_frozen_empty_state() {
    let temp = TempDir::new("console-ledger-empty");
    write_ledger(temp.path(), PUBLISHER, &seed_keypair(7), vec![]);
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(&app, get_with_cookie("/console/ledger", &cookie)).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert!(response.body.contains(
        "No changes have been recorded. Your verified history is empty for this publisher."
    ));
    assert!(response.body.contains("href=\"/console/audit\""));
    assert!(response.body.contains("0 records / Verified"));
}

#[tokio::test]
async fn ledger_verification_failure_is_422_with_the_line_and_no_records() {
    let temp = TempDir::new("console-ledger-422");
    let path = write_ledger(
        temp.path(),
        PUBLISHER,
        &seed_keypair(7),
        vec![change_record(1), change_record(2), change_record(3)],
    );
    tamper_line(&path, 3);
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(&app, get_with_cookie("/console/ledger", &cookie)).await;
    assert_eq!(response.status, 422);
    assert!(response.content_type().starts_with(HTML));
    assert!(
        response
            .body
            .contains("This history could not be verified."),
        "{}",
        response.body
    );
    assert!(response
        .body
        .contains("Verification failed at line 3. No records are displayed."));
    assert!(response
        .body
        .contains("ledger verification failed at line 3"));
    assert!(response.body.contains("Contact support"));
    // No records leak from an unverified ledger.
    assert!(!response.body.contains("changeApplied"));
    assert_no_raw_script(&response.body, "ledger 422");
}

#[tokio::test]
async fn hostile_ledger_record_fields_render_escaped() {
    // The record detail serializes each record as JSON and escapes the whole
    // string, so hostile values in *any* record field (idempotency key,
    // ad-unit id, rollback reason, ...) must come out inert.
    let temp = TempDir::new("console-ledger-hostile-record");
    let hostile_key = "</code></td><script>alert(1)</script>";
    let hostile_unit = "\" onmouseover=\"alert(1)";
    write_ledger(
        temp.path(),
        PUBLISHER,
        &seed_keypair(7),
        vec![LedgerRecord::change_applied(
            1_700_000_000_000,
            PlatformId::Admob,
            hostile_unit,
            ChangeKind::CountryFloorSet {
                country: "EE".into(),
                floor_ecpm: 2_500_000,
            },
            hostile_key,
            "0",
        )],
    );
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let response = send(&app, get_with_cookie("/console/ledger/records/0", &cookie)).await;
    assert_eq!(response.status, 200);
    assert!(
        response
            .body
            .contains("&lt;/code&gt;&lt;/td&gt;&lt;script&gt;alert(1)&lt;/script&gt;"),
        "escaped idempotency key missing"
    );
    assert!(
        // serde_json escapes the embedded quotes, then `html_escape` maps the
        // resulting `"` characters; the raw `onmouseover="alert(1)"` form is
        // unreachable.
        response
            .body
            .contains("\\&quot; onmouseover=\\&quot;alert(1)"),
        "escaped ad-unit id missing"
    );
    assert!(!response.body.contains("onmouseover=\"alert(1)\""));
    assert_no_raw_script(&response.body, "hostile ledger record fields");
}

// --- free audit -------------------------------------------------------------

#[tokio::test]
async fn free_audit_get_redirects_and_the_page_renders_the_finished_form() {
    let temp = TempDir::new("free-audit-page");
    let app = router(file_state(temp.path(), pub1_keys()));
    let redirect = send(&app, get("/free-audit")).await;
    assert_eq!(redirect.status, 303);
    assert_eq!(
        redirect
            .headers
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/free-audit/")
    );
    let response = send(&app, get("/free-audit/")).await;
    assert_eq!(response.status, 200);
    assert!(response.content_type().starts_with(HTML));
    assert!(response
        .body
        .contains("<header class=\"marketing-header\">"));
    assert!(response
        .body
        .contains("<h1>Start with a clear picture.</h1>"));
    assert!(response
        .body
        .contains("We will confirm the scope of your free read-only audit"));
    for needle in [
        ">Work email</label>",
        ">Publisher or studio</label>",
        ">Platforms used</label>",
        ">Monthly ad revenue (optional)</label>",
        ">Anything we should know? (optional)</label>",
        ">Request the free audit</button>",
        "Do not include platform credentials.",
        "No contract is needed for the audit.",
        "Read the privacy notice",
    ] {
        assert!(response.body.contains(needle), "missing {needle}");
    }
    assert!(response
        .body
        .contains("<form method=\"post\" action=\"/free-audit\">"));
    for band in [
        "Under $10,000",
        "$10,000 to $50,000",
        "$50,000 to $100,000",
        "$100,000 to $300,000",
        "Over $300,000",
    ] {
        assert!(response.body.contains(band), "missing band {band}");
    }
    assert_single_h1(&response.body, "free-audit page");
    assert_no_raw_script(&response.body, "free-audit page");
}

#[tokio::test]
async fn free_audit_validation_renders_the_exact_copy_and_preserves_safe_values() {
    let temp = TempDir::new("free-audit-invalid");
    let app = router(file_state(temp.path(), pub1_keys()));
    let empty = send(&app, post_form("/free-audit", "")).await;
    assert_eq!(empty.status, 400);
    for message in [
        "Enter an email address we can reply to.",
        "Enter your publisher or studio name.",
        "Choose at least one platform.",
        "Check the highlighted fields before sending.",
    ] {
        assert!(empty.body.contains(message), "missing {message}");
    }
    assert!(!empty.body.contains("Your request was not confirmed."));

    let hostile = format!(
        "email=you%40studio.example&studio={}&platforms=tiktok&revenue=lots&notes={}",
        "%22%20onmouseover%3D%22alert(1)",
        "n".repeat(2001)
    );
    let response = send(&app, post_form("/free-audit", &hostile)).await;
    assert_eq!(response.status, 400);
    assert!(response.body.contains("Choose at least one platform."));
    assert!(response
        .body
        .contains("Choose a revenue band from the list."));
    assert!(response
        .body
        .contains("Shorten this note to 2,000 characters."));
    assert!(response
        .body
        .contains("value=\"&quot; onmouseover=&quot;alert(1)\""));
    assert!(!response.body.contains("onmouseover=\"alert(1)\""));
    assert_no_raw_script(&response.body, "free-audit invalid");
}

#[tokio::test]
async fn free_audit_submission_deduplicates_and_confirms() {
    let temp = TempDir::new("free-audit-ok");
    let store = std::sync::Arc::new(apex_api::FileLeadStore::new(
        temp.path().join("leads.jsonl"),
    ));
    let app = router(file_state(temp.path(), pub1_keys()).with_leads(store));
    let body =
        "email=You%40Studio.example&studio=Northstar+Studio&platforms=max&platforms=admob&revenue=10k-50k&notes=Hello+there";
    let first = send(&app, post_form("/free-audit", body)).await;
    assert_eq!(first.status, 303, "{}", first.body);
    let location = first
        .headers
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("location")
        .to_string();
    let reference = location
        .strip_prefix("/audit-requested/?ref=")
        .expect("the confirmation reference");
    assert!(reference.starts_with("req_"), "{reference}");
    assert_eq!(reference.len(), 14, "{reference}");
    assert!(
        reference[4..].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{reference}"
    );
    assert!(
        !location.to_ascii_lowercase().contains("studio"),
        "the id carries no personal data: {location}"
    );

    // An identical retry (case and order normalized) returns the same id.
    let retry = send(
        &app,
        post_form("/free-audit", "email=you%40studio.example&studio=Northstar+Studio&platforms=admob&platforms=max&revenue=10k-50k&notes=Hello+there"),
    )
    .await;
    assert_eq!(retry.status, 303);
    assert_eq!(
        retry
            .headers
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(location.as_str())
    );
    let lines = std::fs::read_to_string(temp.path().join("leads.jsonl")).expect("lead log");
    assert_eq!(lines.lines().count(), 1, "no duplicate lead line");

    let confirmation = send(&app, get(&location)).await;
    assert_eq!(confirmation.status, 200);
    assert!(confirmation
        .body
        .contains("<h1>Your request is with us.</h1>"));
    assert!(confirmation.body.contains(
        "We will email you to confirm the platform, portfolio scope and access needed for the audit. There is no platform access to grant on this page."
    ));
    assert!(confirmation.body.contains(
        "If your email address was incorrect, submit a new request. Otherwise, no further action is needed."
    ));
    assert!(confirmation
        .body
        .contains(&format!("Confirmation ID: <code>{reference}</code>")));
    assert!(confirmation.body.contains("Read how the audit works"));
    assert_single_h1(&confirmation.body, "confirmation");
    assert_no_raw_script(&confirmation.body, "confirmation");

    // A bogus reference is not printed back.
    let bare = send(&app, get("/audit-requested/?ref=%3Cscript%3E")).await;
    assert_eq!(bare.status, 200);
    assert!(!bare.body.contains("Confirmation ID"));
    assert_no_raw_script(&bare.body, "confirmation without a reference");
}

#[tokio::test]
async fn free_audit_storage_failure_preserves_the_form_with_the_failure_copy() {
    let temp = TempDir::new("free-audit-fail");
    // No lead store configured: the submit action cannot be saved.
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        post_form(
            "/free-audit",
            "email=you%40studio.example&studio=Northstar+Studio&platforms=admob",
        ),
    )
    .await;
    assert_eq!(response.status, 503);
    assert!(response.body.contains("Your request was not confirmed."));
    assert!(response.body.contains(
        "We could not confirm that the request was saved. Your details remain in the form so you can try again. You can also email hello@apexmediation.ee."
    ));
    assert!(response.body.contains(">Try again</button>"));
    assert!(!response.body.contains(">Request the free audit</button>"));
    assert!(response.body.contains("value=\"you@studio.example\""));
    assert_single_h1(&response.body, "free-audit failure");
    assert_no_raw_script(&response.body, "free-audit failure");
}

// --- cross-cutting ----------------------------------------------------------

#[tokio::test]
async fn console_pages_are_html_and_contain_no_script_elements() {
    let temp = TempDir::new("console-html");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    let pages: Vec<(&str, Option<&str>)> = vec![
        ("/", Some(cookie.as_str())),
        ("/console/audit", Some(cookie.as_str())),
        ("/console/audit?platform=admob", Some(cookie.as_str())),
        ("/console/ledger", None),
        ("/console/ledger", Some(cookie.as_str())),
        ("/console/ledger/records/0", Some(cookie.as_str())),
        ("/free-audit/", None),
        ("/audit-requested/", None),
    ];
    for (uri, cookie) in pages {
        let request = match cookie {
            Some(cookie) => get_with_cookie(uri, cookie),
            None => get(uri),
        };
        let response = send(&app, request).await;
        assert!(response.content_type().starts_with(HTML), "{uri}");
        assert!(response.body.contains("<!DOCTYPE html>"), "{uri}");
        assert_single_h1(&response.body, uri);
        assert_no_raw_script(&response.body, uri);
        assert!(
            !response.body.contains(LEDGER_TOKEN),
            "{uri}: the access token never appears"
        );
    }
}

#[tokio::test]
async fn console_duplicate_query_keys_are_html_bad_request() {
    // The rejection mapping yields the console HTML shape; plain-text axum
    // refusals must stay HTML with the pinned error page.
    let temp = TempDir::new("console-dup-query");
    let app = router(file_state(temp.path(), pub1_keys()));
    let cookie = open_ledger_session(&app).await;
    for uri in [
        "/console/audit?platform=admob&platform=admob",
        "/console/audit/export.csv?platform=admob&platform=admob",
    ] {
        let response = send(&app, get_with_cookie(uri, &cookie)).await;
        assert_eq!(response.status, 400, "{uri}");
        assert!(
            response.content_type().starts_with(HTML),
            "{uri}: {}",
            response.content_type()
        );
        assert!(response.body.contains("<!DOCTYPE html>"), "{uri}");
        assert!(
            response.body.contains("could not be interpreted"),
            "{uri}: {}",
            response.body
        );
        assert_no_raw_script(&response.body, uri);
    }
}

#[tokio::test]
async fn oversized_request_body_is_html_413() {
    let temp = TempDir::new("console-oversized");
    let app = router(file_state(temp.path(), pub1_keys()));
    let body = vec![b'x'; apex_api::MAX_BODY_BYTES + 1];
    let request = Request::builder()
        .method("GET")
        .uri("/console/audit")
        .header(CONTENT_LENGTH, body.len().to_string())
        .body(Body::from(body))
        .expect("request");
    let response = send(&app, request).await;
    assert_eq!(response.status, 413);
    assert!(
        response.content_type().starts_with(HTML),
        "{}",
        response.content_type()
    );
    assert!(
        response.body.contains("<!DOCTYPE html>"),
        "{}",
        response.body
    );
    assert!(
        response.body.contains("exceeds the deployment"),
        "{}",
        response.body
    );
}

// --- setup watch ------------------------------------------------------------

#[tokio::test]
async fn watch_get_redirects_and_the_page_renders_the_sign_up_form() {
    let temp = TempDir::new("watch-page");
    let app = router(file_state(temp.path(), pub1_keys()));
    let redirect = send(&app, get("/watch")).await;
    assert_eq!(redirect.status, 303);
    assert_eq!(
        redirect
            .headers
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/watch/")
    );
    let response = send(&app, get("/watch/")).await;
    assert_eq!(response.status, 200);
    assert!(response.content_type().starts_with(HTML));
    assert!(response
        .body
        .contains("<header class=\"marketing-header\">"));
    assert!(response
        .body
        .contains("<h1>Watch your app-ads.txt every day.</h1>"));
    for needle in [
        ">Domain</label>",
        ">Email</label>",
        ">Network account ids (optional)</label>",
        ">Start the setup watch</button>",
        "No platform keys are needed for the watch.",
        "Read the privacy notice",
    ] {
        assert!(response.body.contains(needle), "missing {needle}");
    }
    assert!(response
        .body
        .contains("<form method=\"post\" action=\"/watch\">"));
    assert_single_h1(&response.body, "watch page");
    assert_no_raw_script(&response.body, "watch page");
}

#[tokio::test]
async fn watch_validation_renders_the_exact_copy_and_preserves_safe_values() {
    let temp = TempDir::new("watch-invalid");
    let app = router(file_state(temp.path(), pub1_keys()));
    let empty = send(&app, post_form("/watch", "")).await;
    assert_eq!(empty.status, 400);
    for message in [
        "Enter the domain where your app-ads.txt lives.",
        "Enter an email address we can reply to.",
        "Check the highlighted fields before sending.",
    ] {
        assert!(empty.body.contains(message), "missing {message}");
    }
    assert!(!empty.body.contains("Your request was not confirmed."));

    let hostile =
        "domain=%22%20onmouseover%3D%22alert(1)&email=you%40studio.example&accounts=garbage+line";
    let response = send(&app, post_form("/watch", hostile)).await;
    assert_eq!(response.status, 400);
    assert!(response
        .body
        .contains("Enter the domain where your app-ads.txt lives."));
    assert!(response
        .body
        .contains("List one network = account id per line."));
    assert!(response
        .body
        .contains("value=\"&quot; onmouseover=&quot;alert(1)\""));
    assert!(!response.body.contains("onmouseover=\"alert(1)\""));
    assert_no_raw_script(&response.body, "watch invalid");
}

/// A transport that confirms every submitted app against a scripted public
/// listing, so the console test exercises the verified-count tier rule.
#[derive(Debug)]
struct VerifyingLookup;

#[async_trait::async_trait]
impl apex_connectors::transport::Transport for VerifyingLookup {
    async fn request(
        &self,
        request: apex_connectors::transport::HttpRequest,
    ) -> Result<apex_connectors::transport::HttpResponse, apex_connectors::transport::TransportError>
    {
        assert!(
            request
                .url
                .starts_with("https://itunes.apple.com/lookup?bundleId="),
            "the tier is verified through the public App Store lookup: {}",
            request.url
        );
        Ok(apex_connectors::transport::HttpResponse {
            status: 200,
            headers: vec![],
            body: br#"{"resultCount":1,"results":[{"trackName":"Test App"}]}"#.to_vec(),
        })
    }
}

#[tokio::test]
async fn watch_signup_stores_dedupes_and_appends_the_outcome_once() {
    let temp = TempDir::new("watch-ok");
    let watch_root = temp.path().join("watch");
    let outcomes_path = temp.path().join("sales").join("outcomes.jsonl");
    let state = file_state(temp.path(), pub1_keys())
        .with_watch(Arc::new(apex_watch::SubscriberStore::open(&watch_root)))
        .with_outcomes(Arc::new(apex_api::OutcomesAppender::new(&outcomes_path)))
        .with_billing_transport(Arc::new(VerifyingLookup));
    let app = router(state);
    let body = "domain=Northstar.Example&email=Founder%40northstar.example&accounts=admob.com+%3D+pub-1&apps=com.example.one%0Acom.example.two%0Acom.example.three%0Acom.example.four%0Acom.example.five%0Acom.example.six%0Acom.example.seven%0Acom.example.eight%0Acom.example.nine%0Acom.example.ten%0Acom.example.eleven%0Acom.example.twelve";
    let first = send(&app, post_form("/watch", body)).await;
    assert_eq!(first.status, 303, "{}", first.body);
    let location = first
        .headers
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("location")
        .to_string();
    let reference = location
        .strip_prefix("/watch/requested?ref=")
        .expect("the confirmation reference");
    assert!(reference.starts_with("sub_"), "{reference}");
    assert_eq!(reference.len(), 36, "{reference}");
    assert!(
        reference[4..].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{reference}"
    );
    assert!(
        !location.to_ascii_lowercase().contains("northstar"),
        "the id carries no personal data: {location}"
    );

    // The stored subscriber row carries the normalized values; a second row
    // rotates in the pending sign-up's cancel token.
    let stored = std::fs::read_to_string(watch_root.join("subscribers.jsonl")).expect("rows");
    let lines: Vec<&str> = stored.lines().collect();
    assert_eq!(lines.len(), 2, "creation plus the cancel-token rotation");
    let row: serde_json::Value = serde_json::from_str(lines[0]).expect("row");
    assert_eq!(row["domain"], "northstar.example");
    assert_eq!(row["email"], "Founder@northstar.example");
    assert_eq!(
        row["tier"], "core",
        "twelve distinct verified apps chose the tier"
    );
    assert_eq!(
        row["verifiedAppCount"], 12,
        "the tier reports the verified count"
    );
    assert_eq!(row["status"], "pending", "no check runs before payment");
    assert_eq!(row["confirmed"], false);
    let rotated: serde_json::Value = serde_json::from_str(lines[1]).expect("rotated row");
    assert_eq!(rotated["id"], row["id"]);
    assert!(
        rotated["cancelTokenHash"].is_string(),
        "the pending row carries a cancel capability hash"
    );
    assert_eq!(row["accountIds"]["admob.com"], "pub-1");
    assert_eq!(row["id"], reference);

    // The frozen outcome line rode along, exactly once.
    let outcomes = std::fs::read_to_string(&outcomes_path).expect("outcomes");
    let line: serde_json::Value = serde_json::from_str(outcomes.trim()).expect("line");
    assert_eq!(line["publisher"], "northstar-example");
    assert_eq!(line["kind"], "watch_signup");
    assert_eq!(line["ref"], reference);

    // A repeat submission inside the window returns the same id and appends
    // neither a subscriber row nor a second outcome line.
    let repeat = send(&app, post_form("/watch", body)).await;
    assert_eq!(repeat.status, 303);
    assert_eq!(
        repeat
            .headers
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(location.as_str())
    );
    // The repeat is the same subscription: one folded subscriber row and one
    // write-once outcome line. The confirmation token is refreshed, so the
    // append-only log carries a newer row for the same id.
    assert_eq!(
        apex_watch::SubscriberStore::open(&watch_root)
            .list()
            .expect("list")
            .len(),
        1,
        "a repeat inside the window is the same subscription"
    );
    assert_eq!(
        std::fs::read_to_string(&outcomes_path)
            .expect("outcomes")
            .lines()
            .count(),
        1
    );

    // The confirmation renders; the checkout line depends on the deployment
    // variable, which this test only observes.
    let confirmation = send(&app, get(&location)).await;
    assert_eq!(confirmation.status, 200);
    assert!(confirmation
        .body
        .contains("<h1>Your watch is requested.</h1>"));
    assert!(confirmation
        .body
        .contains(&format!("Confirmation ID: <code>{reference}</code>")));
    assert!(confirmation
        .body
        .contains("Confirm to start the free watch. There is no payment."));
    assert!(!confirmation.body.contains("Complete the checkout"));
    assert_single_h1(&confirmation.body, "watch confirmation");
    assert_no_raw_script(&confirmation.body, "watch confirmation");
}

#[tokio::test]
async fn watch_signup_without_a_store_is_503_with_the_retry_form() {
    let temp = TempDir::new("watch-no-store");
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        post_form("/watch", "domain=northstar.example&email=a%40x.example"),
    )
    .await;
    assert_eq!(response.status, 503);
    assert!(response.body.contains("Your request was not confirmed."));
    assert!(response.body.contains(">Try again</button>"));
    assert!(response.body.contains("value=\"northstar.example\""));
    assert_no_raw_script(&response.body, "watch no store");
}
