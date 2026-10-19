//! Integration tests for the self-serve sign-in flow over the real router:
//! the workspace created by an accepted free-audit request, the hash-only
//! one-time link row, the session minted with the exchange's exact cookie
//! flags, single-use and expiry refusals, the no-mailer warning seam and the
//! fail-closed `503` when the leads root is unavailable.
//!
//! Security invariants pinned here: the raw token never appears in any file
//! under the leads root, no response ever contains the token, and a refused
//! link never mints a cookie.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use apex_api::app_ads_proof::HttpAppAdsProof;
use apex_api::signin::{SigninService, WarningLog, WorkspaceTokens};
use apex_api::{
    router, AppState, Clock, FileLeadStore, FixedClock, JsonFileAuditSource, StaticAppAdsProof,
    StaticAuditSource, StaticLedgerSource,
};
use apex_notify::sink::{
    DeliveryMessage, DeliveryReceipt, DeliveryState, NotificationSink, SinkError,
};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::*;
use sha2::{Digest, Sha256};

const HTML: &str = "text/html";
/// The refusal copy the dead-link page must carry.
const REFUSAL_COPY: &str =
    "This sign-in link is no longer valid. Request a new audit to receive a fresh link.";
/// A form submission that parses and validates.
const FREE_AUDIT_BODY: &str =
    "email=you%40studio.example&studio=Northstar+Studio&platforms=max&revenue=10k-50k";

/// The key a brand-new (no operator token) workspace claims for `email`.
fn claim_key(email: &str) -> String {
    apex_api::signin::workspace_key_for_claim(None, email).expect("workspace key")
}

/// A mail sink that captures every message in memory (never a file, so the
/// raw-token-never-stored scan stays honest).
#[derive(Debug, Default)]
struct CaptureMailer {
    sent: Mutex<Vec<DeliveryMessage>>,
}

impl CaptureMailer {
    fn texts(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|message| message.text.clone())
            .collect()
    }

    fn last_text(&self) -> String {
        self.texts().pop().unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl NotificationSink for CaptureMailer {
    async fn send(&self, message: &DeliveryMessage) -> Result<DeliveryReceipt, SinkError> {
        self.sent.lock().unwrap().push(message.clone());
        Ok(DeliveryReceipt {
            state: DeliveryState::Accepted,
            provider_reference: None,
        })
    }
}

/// Build the router with the self-serve wiring over `root`: one leads store,
/// the live workspace token file and the sign-in service. The operator token
/// is added only when `with_operator_token` is set, so the self-serve-only
/// deployment path (no boot-time tokens) is exercised too.
fn build_app(
    root: &Path,
    clock_secs: i64,
    ttl_hours: i64,
    mailer: Option<Arc<CaptureMailer>>,
    warnings: &Arc<WarningLog>,
    with_operator_token: bool,
) -> (axum::Router, Arc<SigninService>, Arc<WorkspaceTokens>) {
    build_app_with_base(
        root,
        clock_secs,
        ttl_hours,
        Some("https://console.example"),
        mailer,
        warnings,
        with_operator_token,
    )
}

/// The same wiring with an explicit console base: `None` exercises the
/// documented disabled-links deployment.
fn build_app_with_base(
    root: &Path,
    clock_secs: i64,
    ttl_hours: i64,
    public_base: Option<&str>,
    mailer: Option<Arc<CaptureMailer>>,
    warnings: &Arc<WarningLog>,
    with_operator_token: bool,
) -> (axum::Router, Arc<SigninService>, Arc<WorkspaceTokens>) {
    build_app_with_clock(
        root,
        Arc::new(FixedClock::new(clock_secs)),
        ttl_hours,
        public_base,
        mailer,
        warnings,
        with_operator_token,
    )
}

/// A clock the tests can advance between requests, for the issuance
/// cooldown and any other time-window behavior.
#[derive(Debug, Clone)]
struct StepClock(Arc<AtomicI64>);

impl StepClock {
    fn new(secs: i64) -> Self {
        Self(Arc::new(AtomicI64::new(secs)))
    }

    fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for StepClock {
    fn now_unix_s(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The same wiring with a caller-controlled clock.
fn build_app_with_step_clock(
    root: &Path,
    clock: StepClock,
    ttl_hours: i64,
    mailer: Option<Arc<CaptureMailer>>,
    warnings: &Arc<WarningLog>,
) -> (axum::Router, Arc<SigninService>, Arc<WorkspaceTokens>) {
    build_app_with_clock(
        root,
        Arc::new(clock),
        ttl_hours,
        Some("https://console.example"),
        mailer,
        warnings,
        false,
    )
}

fn build_app_with_clock(
    root: &Path,
    clock: Arc<dyn Clock>,
    ttl_hours: i64,
    public_base: Option<&str>,
    mailer: Option<Arc<CaptureMailer>>,
    warnings: &Arc<WarningLog>,
    with_operator_token: bool,
) -> (axum::Router, Arc<SigninService>, Arc<WorkspaceTokens>) {
    let leads = Arc::new(FileLeadStore::new(root.join("leads.jsonl")));
    let workspaces = Arc::new(WorkspaceTokens::open(root.join("tokens.json")));
    let mut signin = SigninService::open(root)
        .with_ttl_hours(ttl_hours)
        .with_workspaces(Arc::clone(&workspaces))
        .with_warnings(Arc::clone(warnings));
    if let Some(base) = public_base {
        signin = signin.with_public_base(base);
    }
    if let Some(mailer) = mailer {
        let mailer: Arc<dyn NotificationSink> = mailer;
        signin = signin.with_mailer(mailer);
    }
    let signin = Arc::new(signin);
    let mut state = AppState::new(
        Arc::new(StaticAuditSource::default()),
        Arc::new(StaticLedgerSource::default()),
    )
    .with_clock(clock)
    .with_leads(leads)
    .with_workspaces(Arc::clone(&workspaces))
    .with_signin(Arc::clone(&signin))
    .with_app_ads_proof(Arc::new(StaticAppAdsProof::accepting([
        "studio.example",
        "arcade.example",
        "digitalturbine.com",
        "evil.example",
    ])));
    if with_operator_token {
        state = state.with_ledger_token("operator-pub", b"operator-token".to_vec());
    }
    (router(state), signin, workspaces)
}

/// A GET carrying `X-Forwarded-Proto: https` (the proxy's TLS signal).
fn https_get(uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(uri)
        .header("x-forwarded-proto", "https");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).expect("request")
}

/// A form POST carrying `X-Forwarded-Proto: https`.
fn https_post_form(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("x-forwarded-proto", "https")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// A TLS form POST with explicit headers (Origin, Host, Sec-Fetch-Site).
fn https_post_form_with_headers(uri: &str, body: &str, extra: &[(&str, &str)]) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("x-forwarded-proto", "https")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    for (name, value) in extra {
        builder = builder.header(*name, *value);
    }
    builder.body(Body::from(body.to_string())).expect("request")
}

/// The sign-in POST that consumes `token`, optionally over the TLS signal.
fn signin_post(token: &str, https: bool) -> Request<Body> {
    let body = format!("token={token}");
    if https {
        https_post_form("/console/signin", &body)
    } else {
        post_form("/console/signin", &body)
    }
}

/// The `name=value` pair of a `Set-Cookie` value.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string()
}

/// The attribute tail of a cookie: everything after the `name=value` pair.
fn cookie_attributes(set_cookie: &str) -> String {
    set_cookie
        .split(';')
        .skip(1)
        .map(str::trim)
        .collect::<Vec<&str>>()
        .join("; ")
}

/// The sign-in URL carried by a mail, exactly as the recipient sees it.
fn signin_link_from_mail(text: &str) -> String {
    const MARK: &str = "Sign in to your Apex workspace:\n\n";
    let start = text.find(MARK).expect("the mail carries the link") + MARK.len();
    text[start..]
        .split_whitespace()
        .next()
        .expect("the mail carries a URL")
        .to_string()
}

/// The 32-hex token carried by the `/console/signin?token=` link in a mail.
fn token_from_mail(text: &str) -> String {
    const MARK: &str = "/console/signin?token=";
    let start = text.find(MARK).expect("the mail carries the link") + MARK.len();
    let token: String = text[start..].chars().take(32).collect();
    assert_eq!(token.len(), 32, "mail link: {text}");
    assert!(
        token.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "mail link: {text}"
    );
    token
}

/// Every file under `root`, recursively.
fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

/// Submit the free-audit form and return the confirmation reference.
async fn submit_audit(app: &axum::Router, body: &str) -> String {
    let response = send(app, post_form("/free-audit", body)).await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let location = response
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("location")
        .to_string();
    location
        .strip_prefix("/audit-requested/?ref=")
        .expect("the confirmation reference")
        .to_string()
}

#[tokio::test]
async fn free_audit_creates_the_workspace_and_a_hash_only_link_row() {
    let temp = TempDir::new("signin-request");
    // An operator-managed entry that every extend must preserve.
    std::fs::write(
        temp.path().join("tokens.json"),
        "{\n  \"operator-pub\": \"operator-token\"\n}\n",
    )
    .expect("seed tokens");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let (app, signin, _workspaces) = build_app(
        temp.path(),
        // 72 hours in: the default TTL window, and the exact expiry instant.
        1_700_000_000,
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
        false,
    );

    let reference = submit_audit(&app, FREE_AUDIT_BODY).await;
    assert!(reference.starts_with("req_"), "{reference}");

    // 1. The workspace exists: the publisher key derived exactly as the lead
    //    storage derives it has its own token entry, and the operator's entry
    //    is untouched.
    let tokens_raw = std::fs::read_to_string(temp.path().join("tokens.json")).expect("tokens");
    let tokens: BTreeMap<String, String> = serde_json::from_str(&tokens_raw).expect("json");
    let self_serve = tokens
        .get(claim_key("you@studio.example").as_str())
        .expect("the domain-keyed publisher entry");
    assert_eq!(self_serve.len(), 64, "32 random bytes as hex");
    assert!(self_serve.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        tokens.get("operator-pub").map(String::as_str),
        Some("operator-token"),
        "the operator entry survives"
    );

    // 2. The mapping is recorded through the existing lead store: the stored
    //    lead row carries the slug and the studio display name.
    let leads = std::fs::read_to_string(temp.path().join("leads.jsonl")).expect("leads");
    assert_eq!(leads.lines().count(), 1);
    assert!(
        leads.contains(&format!(
            "\"publisher\":\"{}\"",
            claim_key("you@studio.example")
        )),
        "{leads}"
    );
    assert!(
        leads.contains("\"displayName\":\"Northstar Studio\""),
        "{leads}"
    );

    // 3. The link row stores only the SHA-256 hash, with the documented
    //    fields and the configured expiry.
    let rows = signin.store().read_rows().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].at, 1_700_000_000_000);
    assert_eq!(rows[0].publisher, claim_key("you@studio.example"));
    assert_eq!(rows[0].expires_at, 1_700_000_000_000 + 72 * 3_600_000);
    assert_eq!(rows[0].used_at, None);
    let text = mailer.last_text();
    let token = token_from_mail(&text);
    assert_eq!(
        rows[0].token_hash,
        hex::encode(Sha256::digest(token.as_bytes()))
    );

    // The mail carries the link, the expiry and the "if this wasn't you"
    // line. The link is exactly the configured public console origin plus
    // the sign-in path and the 32-hex token parameter.
    let link = signin_link_from_mail(&text);
    assert_eq!(
        link,
        format!("https://console.example/console/signin?token={token}")
    );
    let after_scheme = link.strip_prefix("https://").expect("https scheme");
    let (authority, path_query) = after_scheme.split_once('/').expect("path");
    assert_eq!(authority, "console.example", "host");
    let (path, query) = path_query.split_once('?').expect("token parameter");
    assert_eq!(path, "console/signin", "path");
    assert_eq!(query, format!("token={token}"), "parameter");
    assert_eq!(token.len(), 32, "token shape: {token}");
    assert!(
        token.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{token}"
    );
    assert!(text.contains("expires on"), "{text}");
    assert!(text.contains("If this wasn't you"), "{text}");
    assert!(warnings.lines().is_empty(), "{:?}", warnings.lines());

    // 4. The raw token never appears in ANY file under the leads root: the
    //    durable log holds the hash; the mail lives in memory only.
    for path in files_under(temp.path()) {
        let bytes = std::fs::read(&path).expect("read file");
        assert!(
            !String::from_utf8_lossy(&bytes).contains(&token),
            "raw token stored in {}",
            path.display()
        );
    }
    let store_raw = std::fs::read_to_string(signin.store().path()).expect("signin log");
    assert!(!store_raw.contains(&token), "{store_raw}");
    assert!(store_raw.contains(&rows[0].token_hash), "{store_raw}");

    // 5. The workspace token is the seam the ledger surface authorizes
    //    against, without a restart: a wrong bearer is 401, the self-serve
    //    token passes authorization and the missing ledger is the honest 404.
    let wrong = send(
        &app,
        get_with_bearer(
            &format!("/v1/ledger/{}", claim_key("you@studio.example")),
            "not-the-token",
        ),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let authorized = send(
        &app,
        get_with_bearer(
            &format!("/v1/ledger/{}", claim_key("you@studio.example")),
            self_serve,
        ),
    )
    .await;
    assert_eq!(
        authorized.status,
        StatusCode::NOT_FOUND,
        "authorized; the new publisher has no ledger yet: {}",
        authorized.body
    );
    assert_eq!(authorized.json()["error"], "not_found");
}

#[tokio::test]
async fn without_a_console_base_the_audit_succeeds_but_no_link_is_created() {
    let temp = TempDir::new("signin-disabled");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let (app, signin, workspaces) = build_app_with_base(
        temp.path(),
        1_700_000_000,
        72,
        None,
        Some(Arc::clone(&mailer)),
        &warnings,
        false,
    );

    // The saved-lead contract is unchanged: the free audit still accepts and
    // redirects to the confirmation.
    let reference = submit_audit(&app, FREE_AUDIT_BODY).await;
    assert!(reference.starts_with("req_"), "{reference}");
    assert!(workspaces.has_workspace(claim_key("you@studio.example").as_str()));
    assert!(temp.path().join("leads.jsonl").exists());

    // Disabled links create no durable row and send no email.
    assert!(
        signin.store().read_rows().expect("rows").is_empty(),
        "no link row is written without a console origin"
    );
    assert!(mailer.texts().is_empty(), "{:?}", mailer.texts());

    // The operator warning names the missing variable exactly.
    let lines = warnings.lines();
    assert!(
        lines
            .iter()
            .any(|line| line == apex_api::signin::SIGNIN_LINKS_DISABLED_WARNING),
        "{lines:?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("token=")),
        "the warning never carries a link: {lines:?}"
    );
}

#[tokio::test]
async fn a_valid_link_mints_the_exchange_session_and_redirects_to_the_overview() {
    let temp = TempDir::new("signin-mint");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let (app, signin, workspaces) = build_app(
        temp.path(),
        1_700_000_000,
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
        true,
    );
    submit_audit(&app, FREE_AUDIT_BODY).await;
    let token = token_from_mail(&mailer.last_text());

    // Three mail-scanner GETs render the landing page and consume nothing.
    for _ in 0..3 {
        let landing = send(
            &app,
            https_get(&format!("/console/signin?token={token}"), None),
        )
        .await;
        assert_eq!(landing.status, StatusCode::OK, "{}", landing.body);
        assert!(landing.body.contains("Sign in to your workspace."));
        assert!(landing
            .body
            .contains("<form method=\"post\" action=\"/console/signin\">"));
        assert!(landing
            .body
            .contains(&format!("name=\"token\" value=\"{token}\"")));
        assert!(set_cookie(&landing).is_none(), "a GET never mints a cookie");
        assert_eq!(
            signin.store().read_rows().expect("rows")[0].used_at,
            None,
            "a GET never consumes the link"
        );
    }

    let response = send(&app, signin_post(&token, true)).await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response
            .headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/"),
        "the console overview is the landing page"
    );
    assert!(
        !response.body.contains(&token),
        "the redirect never echoes the token"
    );
    let cookie = set_cookie(&response).expect("the mint sets a cookie");
    assert!(cookie.contains(&format!("{}=", apex_api::SESSION_COOKIE)));
    assert!(cookie.contains("Path=/;"));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(cookie.contains("Max-Age=900"));
    assert!(
        cookie.ends_with("; Secure"),
        "the https request sets Secure: {cookie}"
    );

    // Differential: the ledger-token exchange on the same deployment produces
    // exactly the same cookie attribute set (only the ids differ).
    let exchange = send(
        &app,
        https_post_form(
            "/console/ledger",
            "publisher=operator-pub&token=operator-token",
        ),
    )
    .await;
    assert_eq!(exchange.status, StatusCode::SEE_OTHER, "{}", exchange.body);
    let exchange_cookie = set_cookie(&exchange).expect("exchange cookie");
    assert_eq!(
        cookie_attributes(&cookie),
        cookie_attributes(&exchange_cookie),
        "sign-in cookies carry the exchange's flags"
    );

    // The minted session is live: the overview renders and the access page
    // proves the session's publisher.
    let pair = cookie_pair(&cookie);
    let overview = send(&app, get_with_cookie("/", &pair)).await;
    assert_eq!(overview.status, 200);
    assert!(
        overview.body.contains("<h1>Your account at a glance</h1>"),
        "the redirect target is the console overview: {}",
        overview.body
    );
    let access = send(&app, get_with_cookie("/console/settings/access", &pair)).await;
    assert_eq!(access.status, 200, "{}", access.body);
    assert!(access.body.contains("studio-example"), "{}", access.body);

    // A second link consumed without the proxy's TLS signal mints the same
    // cookie without `Secure`: the flag follows the request, exactly as the
    // exchange's cookie does.
    workspaces
        .ensure("bright-arcade", "b@arcade.example", true)
        .expect("record the second workspace");
    let second = signin
        .issue_link("bright-arcade", "b@arcade.example", 1_700_000_000_000)
        .await
        .expect("issue");
    let plain = send(&app, signin_post(&second.token, false)).await;
    assert_eq!(plain.status, StatusCode::SEE_OTHER);
    let plain_cookie = set_cookie(&plain).expect("plain cookie");
    assert!(!plain_cookie.contains("Secure"), "{plain_cookie}");
    assert!(plain_cookie.contains("Path=/;"), "{plain_cookie}");
    assert!(!plain.body.contains(&second.token));
}

#[tokio::test]
async fn a_used_link_refuses_a_second_mint_and_the_first_session_persists() {
    let temp = TempDir::new("signin-single-use");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let (app, signin, _workspaces) = build_app(
        temp.path(),
        1_700_000_000,
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
        false,
    );
    submit_audit(&app, FREE_AUDIT_BODY).await;
    let token = token_from_mail(&mailer.last_text());

    let first = send(&app, signin_post(&token, true)).await;
    assert_eq!(first.status, StatusCode::SEE_OTHER);
    let pair = cookie_pair(&set_cookie(&first).expect("cookie"));

    // The row is marked used at the first mint.
    let rows = signin.store().read_rows().expect("rows");
    assert_eq!(rows[0].used_at, Some(1_700_000_000_000));

    // The second use is refused: the documented refusal page, `200`, no
    // cookie, no token anywhere in the response.
    let second = send(&app, signin_post(&token, true)).await;
    assert_eq!(second.status, 200);
    assert!(second.content_type().starts_with(HTML));
    assert!(second.body.contains(REFUSAL_COPY), "{}", second.body);
    assert!(
        set_cookie(&second).is_none(),
        "a refused link mints nothing"
    );
    assert!(!second.body.contains(&token));
    assert!(!second.body.contains("token="), "{}", second.body);

    // The refusal did not move `used_at` (single use applies to minting).
    assert_eq!(
        signin.store().read_rows().expect("rows")[0].used_at,
        Some(1_700_000_000_000)
    );

    // A GET after the POST mints nothing and shows the refusal instead of a
    // fresh button: the link is spent.
    let after = send(
        &app,
        https_get(&format!("/console/signin?token={token}"), None),
    )
    .await;
    assert_eq!(after.status, 200);
    assert!(after.body.contains(REFUSAL_COPY), "{}", after.body);
    assert!(!after.body.contains("<form"), "{}", after.body);
    assert!(set_cookie(&after).is_none());

    // Semantics: single-use applies to minting only; the first session keeps
    // working for its own idle window.
    let overview = send(&app, get_with_cookie("/", &pair)).await;
    assert_eq!(overview.status, 200);
}

#[tokio::test]
async fn immediate_repeat_audit_keeps_the_lead_but_sends_one_link() {
    let temp = TempDir::new("signin-cooldown");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let (app, signin, _workspaces) = build_app(
        temp.path(),
        1_700_000_000,
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
        false,
    );

    let first = submit_audit(&app, FREE_AUDIT_BODY).await;
    // A different studio label gives the second submission its own lead id, so
    // the one-row assertion below is about the link, not lead deduplication.
    let second = submit_audit(
        &app,
        "email=you%40studio.example&studio=Northstar+Studio+Two&platforms=max",
    )
    .await;
    assert!(first.starts_with("req_") && second.starts_with("req_"));
    // The saved-lead contract is preserved for both submissions.
    assert_eq!(
        std::fs::read_to_string(temp.path().join("leads.jsonl"))
            .expect("leads")
            .lines()
            .count(),
        2,
        "both submissions keep their lead"
    );
    // The anti-bombing cooldown issued exactly one row and one email.
    assert_eq!(
        signin.store().read_rows().expect("rows").len(),
        1,
        "one link row for two immediate submissions"
    );
    assert_eq!(mailer.texts().len(), 1, "one email for two submissions");
    let _ = std::fs::remove_dir_all(temp.path());
}

#[tokio::test]
async fn signin_post_rejects_cross_origin_and_cross_site_before_consuming() {
    let temp = TempDir::new("signin-origin");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let (app, signin, _workspaces) = build_app(
        temp.path(),
        1_700_000_000,
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
        true,
    );
    submit_audit(&app, FREE_AUDIT_BODY).await;
    let token = token_from_mail(&mailer.last_text());
    let body = format!("token={token}");

    for extra in [
        vec![
            ("host", "console.example"),
            ("origin", "https://evil.example"),
        ],
        // Same host, but a plain-http origin cannot mint a TLS session.
        vec![
            ("host", "console.example"),
            ("origin", "http://console.example"),
        ],
        // The browser's own cross-site hint is enough without an Origin.
        vec![
            ("host", "console.example"),
            ("sec-fetch-site", "cross-site"),
        ],
    ] {
        let response = send(
            &app,
            https_post_form_with_headers("/console/signin", &body, &extra),
        )
        .await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{extra:?}");
        assert!(
            set_cookie(&response).is_none(),
            "a refused origin mints no cookie: {extra:?}"
        );
        assert_eq!(
            signin.store().read_rows().expect("rows")[0].used_at,
            None,
            "the guard runs before consumption: {extra:?}"
        );
    }

    // Same origin is accepted and consumes the still-live link.
    let accepted = send(
        &app,
        https_post_form_with_headers(
            "/console/signin",
            &body,
            &[
                ("host", "console.example"),
                ("origin", "https://console.example"),
            ],
        ),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::SEE_OTHER);
    assert!(set_cookie(&accepted).is_some(), "the mint sets a cookie");
}

#[tokio::test]
async fn ledger_exchange_rejects_cross_origin_and_accepts_same_origin() {
    let temp = TempDir::new("ledger-origin");
    let warnings = Arc::new(WarningLog::default());
    let (app, _signin, _workspaces) =
        build_app(temp.path(), 1_700_000_000, 72, None, &warnings, true);
    let body = "publisher=operator-pub&token=operator-token";

    let crossed = send(
        &app,
        https_post_form_with_headers(
            "/console/ledger",
            body,
            &[
                ("host", "console.example"),
                ("origin", "https://evil.example"),
            ],
        ),
    )
    .await;
    assert_eq!(crossed.status, StatusCode::FORBIDDEN);
    assert!(set_cookie(&crossed).is_none());

    let accepted = send(
        &app,
        https_post_form_with_headers(
            "/console/ledger",
            body,
            &[
                ("host", "console.example"),
                ("origin", "https://console.example"),
            ],
        ),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::SEE_OTHER);
    assert!(set_cookie(&accepted).is_some());
}

#[tokio::test]
async fn expired_unknown_and_malformed_links_are_refused_without_echoing_the_token() {
    let temp = TempDir::new("signin-refusals");
    let warnings = Arc::new(WarningLog::default());
    // "Now" is two hours past the issuance instant; the link lives one hour.
    let (app, signin, workspaces) = build_app(
        temp.path(),
        1_700_000_000 + 7_200,
        1,
        None,
        &warnings,
        false,
    );
    workspaces
        .ensure("northstar-studio", "you@studio.example", true)
        .expect("record the workspace");
    let created = signin
        .issue_link("northstar-studio", "you@studio.example", 1_700_000_000_000)
        .await
        .expect("issue");
    assert_eq!(created.expires_at_unix_ms, 1_700_000_000_000 + 3_600_000);

    // Expired: the refusal page with `200`, and the row stays unused.
    let expired = send(
        &app,
        get(&format!("/console/signin?token={}", created.token)),
    )
    .await;
    assert_eq!(expired.status, 200);
    assert!(expired.content_type().starts_with(HTML));
    assert!(expired.body.contains(REFUSAL_COPY), "{}", expired.body);
    assert!(!expired.body.contains(&created.token));
    assert!(set_cookie(&expired).is_none());
    assert_eq!(
        signin.store().read_rows().expect("rows")[0].used_at,
        None,
        "an expired refusal never marks the row"
    );

    // Unknown but well-formed: the same refusal page.
    let unknown = send(
        &app,
        get(&format!("/console/signin?token={}", "0".repeat(32))),
    )
    .await;
    assert_eq!(unknown.status, 200);
    assert!(unknown.body.contains(REFUSAL_COPY));

    // Missing, empty and malformed tokens name no link: the `400` refusal.
    for uri in [
        "/console/signin",
        "/console/signin?token=",
        "/console/signin?token=zzzz",
        "/console/signin?token=0123456789ABCDEF0123456789ABCDEF",
        &format!("/console/signin?token={}", "a".repeat(31)),
        &format!("/console/signin?token={}", "a".repeat(33)),
    ] {
        let response = send(&app, get(uri)).await;
        assert_eq!(response.status, 400, "{uri}: {}", response.body);
        assert!(response.content_type().starts_with(HTML), "{uri}");
        assert!(response.body.contains(REFUSAL_COPY), "{uri}");
        assert!(!response.body.contains("token="), "{uri}");
        assert!(set_cookie(&response).is_none(), "{uri}");
    }

    // A wrong method on the path keeps the HTML 405 hint page.
    let wrong_method = send(
        &app,
        Request::builder()
            .method("PUT")
            .uri("/console/signin")
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(wrong_method.status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(wrong_method.body.contains("Method not allowed."));
}

#[tokio::test]
async fn without_an_email_sink_the_audit_succeeds_and_the_warning_is_recorded() {
    let temp = TempDir::new("signin-no-mailer");
    let warnings = Arc::new(WarningLog::default());
    let (app, signin, _workspaces) =
        build_app(temp.path(), 1_700_000_000, 72, None, &warnings, false);

    let reference = submit_audit(&app, FREE_AUDIT_BODY).await;
    assert!(reference.starts_with("req_"));
    // The audit success contract: the lead is saved and the confirmation is
    // the same as with a mailer.
    assert_eq!(
        std::fs::read_to_string(temp.path().join("leads.jsonl"))
            .expect("leads")
            .lines()
            .count(),
        1
    );
    // The link row is durable even though nothing was sent.
    let rows = signin.store().read_rows().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].used_at, None);

    // The warning is captured through the test seam, names the publisher and
    // the missing configuration, and never carries the link or the token.
    let lines = warnings.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("not sent"), "{}", lines[0]);
    assert!(lines[0].contains("APEX_NOTIFY_SINK=email"), "{}", lines[0]);
    assert!(lines[0].contains("studio-example"), "{}", lines[0]);
    assert!(!lines[0].contains("token="), "{}", lines[0]);
}

#[tokio::test]
async fn two_publishers_get_independent_links_and_their_own_sessions() {
    let temp = TempDir::new("signin-two-publishers");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    // No boot-time operator token: the workspace gate must pass on the live
    // self-serve workspaces alone.
    let (app, signin, _workspaces) = build_app(
        temp.path(),
        1_700_000_000,
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
        false,
    );

    submit_audit(
        &app,
        "email=a%40studio.example&studio=Northstar+Studio&platforms=max",
    )
    .await;
    submit_audit(
        &app,
        "email=b%40arcade.example&studio=Bright+Arcade&platforms=admob",
    )
    .await;

    let texts = mailer.texts();
    assert_eq!(texts.len(), 2, "{texts:?}");
    let northstar_token = token_from_mail(&texts[0]);
    let bright_token = token_from_mail(&texts[1]);
    assert_ne!(northstar_token, bright_token, "independent tokens");

    let rows = signin.store().read_rows().expect("rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].publisher, claim_key("a@studio.example"));
    assert_eq!(rows[1].publisher, claim_key("b@arcade.example"));
    assert_ne!(rows[0].token_hash, rows[1].token_hash);
    assert!(!texts[0].contains(&bright_token), "no cross talk");
    assert!(!texts[1].contains(&northstar_token), "no cross talk");

    let tokens: BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("tokens.json")).expect("tokens"),
    )
    .expect("json");
    assert_eq!(tokens.len(), 2, "{tokens:?}");
    assert_ne!(
        tokens.get(claim_key("a@studio.example").as_str()),
        tokens.get(claim_key("b@arcade.example").as_str()),
        "independent workspace tokens"
    );

    // Both sign in; each session lands on its own workspace.
    let northstar_cookie = cookie_pair(
        &set_cookie(&send(&app, signin_post(&northstar_token, true)).await)
            .expect("northstar cookie"),
    );
    let bright_cookie = cookie_pair(
        &set_cookie(&send(&app, signin_post(&bright_token, true)).await).expect("bright cookie"),
    );
    assert_ne!(northstar_cookie, bright_cookie, "distinct sessions");

    for (cookie, own, other) in [
        (&northstar_cookie, "studio-example", "arcade-example"),
        (&bright_cookie, "arcade-example", "studio-example"),
    ] {
        let access = send(&app, get_with_cookie("/console/settings/access", cookie)).await;
        assert_eq!(access.status, 200, "{}", access.body);
        assert!(access.body.contains(own), "{}", access.body);
        assert!(
            !access.body.contains(other),
            "another publisher's key leaked: {}",
            access.body
        );
        let overview = send(&app, get_with_cookie("/", cookie)).await;
        assert_eq!(overview.status, 200);
    }
}

#[tokio::test]
async fn an_existing_workspace_refuses_an_unrecorded_address_and_keeps_the_link_private() {
    let temp = TempDir::new("signin-workspace-takeover");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let clock = StepClock::new(1_700_000_000);
    let (app, signin, _workspaces) = build_app_with_step_clock(
        temp.path(),
        clock.clone(),
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
    );
    submit_audit(&app, FREE_AUDIT_BODY).await;
    let leads_before = std::fs::read_to_string(temp.path().join("leads.jsonl")).expect("leads");
    let rows_before = signin.store().read_rows().expect("rows").len();
    assert_eq!(mailer.texts().len(), 1);

    // The same proven domain, a different address: the workspace exists, the
    // address is not recorded, so nothing is stored or sent.
    let response = send(
        &app,
        post_form(
            "/free-audit",
            "email=attacker%40studio.example&studio=Northstar+Studio&platforms=max",
        ),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::SEE_OTHER,
        "the saved lead is the contract; the link is refused, not the request: {}",
        response.body
    );
    assert_eq!(mailer.texts().len(), 1, "no second link was delivered");
    assert_eq!(signin.store().read_rows().expect("rows").len(), rows_before);
    let leads_after = std::fs::read_to_string(temp.path().join("leads.jsonl")).expect("leads");
    assert!(
        leads_after.lines().count() > leads_before.lines().count(),
        "the refused workspace still keeps the lead"
    );
    assert!(
        !warnings.lines().join(" ").contains("attacker"),
        "the refusal never echoes the address"
    );
    let lines = warnings.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("studio-example") && line.contains("recorded contact")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("attacker@evil.example")),
        "the refusal log never echoes the address: {lines:?}"
    );

    // The recorded contact still receives a fresh one-time link.
    clock.advance(61);
    submit_audit(&app, FREE_AUDIT_BODY).await;
    assert_eq!(
        mailer.texts().len(),
        2,
        "the recorded contact is delivered to"
    );
    let rows = signin.store().read_rows().expect("rows");
    assert_eq!(rows.len(), rows_before + 1);
    assert_eq!(
        rows.iter()
            .filter(|row| row.publisher == claim_key("you@studio.example"))
            .count(),
        rows_before + 1
    );
}

/// A provisioned token whose key already holds runner data cannot be
/// claimed by a domain that merely collides with the slug. The operator
/// declares the owning domain; a matching address may then claim it.
#[tokio::test]
async fn a_data_bearing_token_requires_the_declared_domain() {
    struct Wiring {
        app: axum::Router,
        mailer: Arc<CaptureMailer>,
        workspaces: Arc<WorkspaceTokens>,
        signin: Arc<SigninService>,
    }

    fn wire(root: &Path, bindings: &[(&str, &str)]) -> Wiring {
        let mailer = Arc::new(CaptureMailer::default());
        let warnings = Arc::new(WarningLog::default());
        let workspaces = Arc::new(WorkspaceTokens::open(root.join("tokens.json")));
        let signin = Arc::new(
            SigninService::open(root)
                .with_workspaces(Arc::clone(&workspaces))
                .with_public_base("https://console.example")
                .with_warnings(Arc::clone(&warnings))
                .with_mailer(Arc::clone(&mailer) as Arc<dyn NotificationSink>),
        );
        let state = AppState::new(
            Arc::new(JsonFileAuditSource::new(root.join("snapshots"))),
            Arc::new(StaticLedgerSource::default()),
        )
        .with_clock(Arc::new(FixedClock::new(1_700_000_000)))
        .with_leads(Arc::new(FileLeadStore::new(root.join("leads.jsonl"))))
        .with_workspaces(Arc::clone(&workspaces))
        .with_signin(Arc::clone(&signin))
        .with_app_ads_proof(Arc::new(StaticAppAdsProof::accepting(["studio.example"])))
        .with_workspace_domains(
            bindings
                .iter()
                .map(|(key, domain)| (key.to_string(), domain.to_string()))
                .collect(),
        );
        Wiring {
            app: router(state),
            mailer,
            workspaces,
            signin,
        }
    }

    async fn submit(app: &axum::Router, email_form: &str) -> TestResponse {
        send(
            app,
            post_form(
                "/free-audit",
                &format!("email={email_form}&studio=Studio&platforms=max"),
            ),
        )
        .await
    }

    let temp = TempDir::new("signin-data-claim");
    std::fs::write(
        temp.path().join("tokens.json"),
        r#"{"studio-example":"operator-ledger-token"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(temp.path().join("snapshots/studio-example")).unwrap();

    // No declaration: the key already holds data, so self-serve refuses.
    let unbound = wire(temp.path(), &[]);
    let refused = submit(&unbound.app, "founder%40studio.example").await;
    assert_eq!(refused.status, StatusCode::SEE_OTHER, "{}", refused.body);
    assert!(unbound.mailer.texts().is_empty(), "no link is sent");
    assert!(!unbound.workspaces.has_record("studio-example"));
    assert!(unbound.signin.store().read_rows().unwrap().is_empty());

    // A declaration for another domain is not a claim licence.
    let wrong = wire(temp.path(), &[("studio-example", "other.example")]);
    let refused = submit(&wrong.app, "founder%40studio.example").await;
    assert_eq!(refused.status, StatusCode::SEE_OTHER);
    assert!(
        wrong.mailer.texts().is_empty(),
        "a colliding domain gets no link"
    );
    assert!(!wrong.workspaces.has_record("studio-example"));

    // The declared domain claims the workspace once, after proof.
    let right = wire(temp.path(), &[("studio-example", "studio.example")]);
    let claimed = submit(&right.app, "founder%40studio.example").await;
    assert_eq!(claimed.status, StatusCode::SEE_OTHER);
    assert_eq!(
        right.mailer.texts().len(),
        1,
        "the declared owner gets the link"
    );
    assert!(right.workspaces.has_record("studio-example"));
    assert!(right
        .workspaces
        .contact_recorded("studio-example", "founder@studio.example"));
    assert_eq!(right.signin.store().read_rows().unwrap().len(), 1);
}

/// An operator-provisioned ledger token is not yet a workspace: the
/// publisher may claim it once with a domain proof, which records the owner
/// and makes the address the first recorded contact.
#[tokio::test]
async fn a_provisioned_token_is_claimed_once_with_a_domain_proof() {
    let temp = TempDir::new("signin-claim-token");
    std::fs::write(
        temp.path().join("tokens.json"),
        r#"{"studio-example":"operator-ledger-token"}"#,
    )
    .expect("seed token file");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let (app, signin, workspaces) = build_app(
        temp.path(),
        1_700_000_000,
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
        false,
    );

    assert!(
        workspaces.token("studio-example").is_some(),
        "the provisioned token is visible"
    );
    assert!(!workspaces.has_record("studio-example"));

    let claimed = send(
        &app,
        post_form(
            "/free-audit",
            "email=founder%40studio.example&studio=Northstar+Studio&platforms=max",
        ),
    )
    .await;
    assert_eq!(claimed.status, StatusCode::SEE_OTHER, "{}", claimed.body);
    assert_eq!(mailer.texts().len(), 1, "the owner receives the link");
    assert!(workspaces.has_record("studio-example"));
    assert!(workspaces.contact_recorded("studio-example", "founder@studio.example"));
    assert_eq!(signin.store().read_rows().expect("rows").len(), 1);

    // The workspace now exists: another address is never added to it, even
    // with a proof for the same domain.
    let refused = send(
        &app,
        post_form(
            "/free-audit",
            "email=other%40studio.example&studio=Northstar+Studio&platforms=max",
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::SEE_OTHER);
    assert_eq!(mailer.texts().len(), 1, "no second link is sent");
    assert!(!workspaces.contact_recorded("studio-example", "other@studio.example"));
}

/// The ownership proof is fetched exactly once, and only when a new
/// workspace actually needs it: an invalid form and an existing workspace
/// (accepted or refused) never trigger a second guarded fetch.
#[tokio::test]
async fn a_free_audit_fetches_the_ownership_proof_once_for_a_new_workspace() {
    const APP_ADS: &str = "OWNERDOMAIN=studio.example\n\
                           google.com, pub-1, DIRECT\n";
    let temp = TempDir::new("signin-proof-fetch");
    let counter = Arc::new(CountingAppAdsTransport::new(APP_ADS));
    let proof = HttpAppAdsProof::new(
        Arc::clone(&counter) as Arc<dyn apex_api::app_ads_proof::PinnedTransport>
    )
    .with_resolver(Arc::new(PublicResolver));
    let workspaces = Arc::new(WorkspaceTokens::open(temp.path().join("tokens.json")));
    let signin = Arc::new(
        SigninService::open(temp.path())
            .with_workspaces(Arc::clone(&workspaces))
            .with_public_base("https://console.example"),
    );
    let state = AppState::new(
        Arc::new(StaticAuditSource::default()),
        Arc::new(StaticLedgerSource::default()),
    )
    .with_clock(Arc::new(FixedClock::new(1_700_000_000)))
    .with_leads(Arc::new(FileLeadStore::new(
        temp.path().join("leads.jsonl"),
    )))
    .with_workspaces(Arc::clone(&workspaces))
    .with_signin(signin)
    .with_app_ads_proof(Arc::new(proof));
    let app = router(state);

    // Invalid form: no store write, no proof fetch.
    let invalid = send(
        &app,
        post_form(
            "/free-audit",
            "email=not-an-email&studio=Northstar+Studio&platforms=max",
        ),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(counter.calls(), 0, "validation refuses before the fetch");
    assert!(!workspaces.has_workspace("studio-example"));

    // A new workspace needs the domain's app-ads.txt: exactly one fetch.
    let created = send(
        &app,
        post_form(
            "/free-audit",
            "email=you%40studio.example&studio=Northstar+Studio&platforms=max",
        ),
    )
    .await;
    assert_eq!(created.status, StatusCode::SEE_OTHER, "{}", created.body);
    assert_eq!(
        counter.calls(),
        1,
        "one guarded fetch for the new workspace"
    );
    assert!(workspaces.has_workspace(claim_key("you@studio.example").as_str()));
    assert!(workspaces.contact_recorded(
        claim_key("you@studio.example").as_str(),
        "you@studio.example"
    ));

    // An existing workspace refuses an unrecorded address before any proof
    // work: the recorded contact list is the whole decision.
    let refused = send(
        &app,
        post_form(
            "/free-audit",
            "email=attacker%40studio.example&studio=Northstar+Studio&platforms=max",
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::SEE_OTHER, "{}", refused.body);
    assert_eq!(
        counter.calls(),
        1,
        "a refused existing workspace never fetches app-ads.txt"
    );
    assert!(!workspaces.contact_recorded("studio-example", "attacker@studio.example"));

    // The recorded contact is accepted from the existing record, also with
    // no fetch.
    let accepted = send(
        &app,
        post_form(
            "/free-audit",
            "email=you%40studio.example&studio=Northstar+Studio&platforms=max",
        ),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::SEE_OTHER, "{}", accepted.body);
    assert_eq!(counter.calls(), 1, "an existing workspace never re-proves");
}

#[tokio::test]
async fn a_new_workspace_without_an_app_ads_proof_gets_no_link() {
    let temp = TempDir::new("signin-unproven");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let (app, signin, workspaces) = build_app(
        temp.path(),
        1_700_000_000,
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
        false,
    );

    let response = send(
        &app,
        post_form(
            "/free-audit",
            "email=founder%40unproven.example&studio=Shadow+Studio&platforms=admob",
        ),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::SEE_OTHER,
        "the saved lead is the contract; the unproven workspace gets no link: {}",
        response.body
    );
    assert!(mailer.texts().is_empty(), "no link is sent");
    assert!(
        signin.store().read_rows().expect("rows").is_empty(),
        "no link row is created"
    );
    assert!(
        temp.path().join("leads.jsonl").exists(),
        "the lead is still stored"
    );
    assert!(!workspaces.has_workspace("unproven-example"));
    assert!(
        warnings
            .lines()
            .iter()
            .any(|line| line.contains("unproven-example") && line.contains("app-ads.txt")),
        "{:?}",
        warnings.lines()
    );
}

#[tokio::test]
async fn signin_fails_closed_without_a_leads_root_or_when_the_store_is_unavailable() {
    // (a) No leads root: the route answers the documented `503` panel.
    let temp = TempDir::new("signin-unconfigured");
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        get("/console/signin?token=0123456789abcdef0123456789abcdef"),
    )
    .await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        response
            .body
            .contains("This view is not available in this deployment."),
        "{}",
        response.body
    );

    // (b) The sign-in log path is not usable (a directory squats it): the
    //     store read fails and the path fails closed instead of minting.
    let broken = TempDir::new("signin-broken-store");
    std::fs::create_dir_all(broken.path().join("signin.jsonl")).expect("squat");
    let warnings = Arc::new(WarningLog::default());
    let (app, _signin, _workspaces) =
        build_app(broken.path(), 1_700_000_000, 72, None, &warnings, false);
    let response = send(
        &app,
        get("/console/signin?token=0123456789abcdef0123456789abcdef"),
    )
    .await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(response
        .body
        .contains("This view is not available in this deployment."));
    assert!(set_cookie(&response).is_none());
}

#[tokio::test]
async fn a_proven_domain_keys_the_workspace_and_each_studio_is_only_a_label() {
    let temp = TempDir::new("signin-domain-key");
    let warnings = Arc::new(WarningLog::default());
    let mailer = Arc::new(CaptureMailer::default());
    let clock = StepClock::new(1_700_000_000);
    let (app, signin, workspaces) = build_app_with_step_clock(
        temp.path(),
        clock.clone(),
        72,
        Some(Arc::clone(&mailer)),
        &warnings,
    );

    // Two studio names behind one proven domain. The same recorded contact is
    // accepted, and both human labels are kept on their lead rows.
    submit_audit(
        &app,
        "email=founder%40digitalturbine.com&studio=Turbine+Games&platforms=max",
    )
    .await;
    clock.advance(61);
    submit_audit(
        &app,
        "email=founder%40digitalturbine.com&studio=Turbine+Labs&platforms=admob",
    )
    .await;
    // A different proven domain is a different workspace.
    submit_audit(
        &app,
        "email=b%40arcade.example&studio=Bright+Arcade&platforms=admob",
    )
    .await;

    let tokens: BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("tokens.json")).expect("tokens"),
    )
    .expect("json");
    assert_eq!(
        tokens.keys().cloned().collect::<Vec<_>>(),
        vec![
            claim_key("b@arcade.example"),
            claim_key("founder@digitalturbine.com")
        ],
        "one token key per proven domain, in stable order"
    );
    assert!(
        !tokens.contains_key("turbine-games") && !tokens.contains_key("turbine-labs"),
        "a studio-name slug never becomes a workspace key: {tokens:?}"
    );

    let leads = std::fs::read_to_string(temp.path().join("leads.jsonl")).expect("leads");
    let publishers: Vec<String> = leads
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("lead")["lead"]["publisher"]
                .as_str()
                .expect("publisher")
                .to_string()
        })
        .collect();
    assert_eq!(
        publishers,
        vec![
            claim_key("founder@digitalturbine.com"),
            claim_key("founder@digitalturbine.com"),
            claim_key("b@arcade.example")
        ],
        "the lead contract carries the domain key"
    );
    for label in ["Turbine Games", "Turbine Labs"] {
        assert!(
            leads.contains(&format!("\"displayName\":\"{label}\"")),
            "both studio labels are persisted: {leads}"
        );
    }

    let rows = signin.store().read_rows().expect("rows");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].publisher, claim_key("founder@digitalturbine.com"));
    assert_eq!(rows[1].publisher, claim_key("founder@digitalturbine.com"));
    assert_eq!(rows[2].publisher, claim_key("b@arcade.example"));
    assert!(workspaces.has_workspace(claim_key("founder@digitalturbine.com").as_str()));
    assert!(workspaces.has_workspace(claim_key("b@arcade.example").as_str()));
    assert!(!workspaces.has_workspace("turbine-games"));
    assert!(warnings.lines().is_empty(), "{:?}", warnings.lines());
}
