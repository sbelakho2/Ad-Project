//! The axum 0.8 router: JSON API plus the server-rendered console.
//!
//! Wire contract (JSON responses are `application/json`; `GET /metrics` is
//! Prometheus text):
//!
//! | route | success | refusals |
//! |---|---|---|
//! | `GET /healthz` | `200 {"status":"ok"}` | — |
//! | `GET /metrics` | `200` Prometheus text (`text/plain`, scraped on the loopback bind; the public edge refuses the path) | — |
//! | `GET /v1/audit/{platform}?publisher=` | `200` `AuditReport` for exactly that publisher's own snapshot (bearer-gated; the query parameter is required) | `400 unknown_platform / publisher_required / invalid_publisher / bad_request`, `401 unauthorized`, `404 not_found`, `503 live_audit_not_configured` |
//! | `POST /v1/audit/{platform}` | `200` live read-only `AuditReport` | `400 bad_request / unknown_platform / invalid_publisher`, `401 unauthorized`, `502 platform_unavailable`, `503 live_audit_not_configured` |
//! | `GET /v1/ledger/{publisher}` | `200 {"publisher","mode":"read_only","records"}` | `400 invalid_publisher / bad_request`, `401 unauthorized`, `404 not_found`, `422 {"error":"ledger_unverified","line":n}`, `503 ledger_auth_not_configured` |
//! | `POST /v1/cycle/{publisher}?platform=` | `200` read-only summary | `400 platform_required / unknown_platform / invalid_publisher / invalid_cycle_request / invalid_policy / bad_request`, `401 unauthorized`, `404`, `422`, `503 live_audit_not_configured` |
//! | `POST /v1/mollie/webhook` | `200 {"received":true,"id":"tr_...","status":"paid","new":true}` (an Apex fee invoice payment; the watch is free and has no payment; a replay is `new:false`) | `400 malformed_event`, `unknown_payment` or `invalid_payment`; `503 mollie_webhook_not_configured` or `collection_store_not_configured` |
//! | `POST /v1/leads` | `202 {"received":true}` | `400 invalid_lead`, `503 lead_capture_not_configured` |
//! | `POST /v1/operator/jobs` | `200` frozen job JSON (the created job, or the existing job on a duplicate `job_id`) | `400 bad_request`, `401 unauthorized`, `503 operator_jobs_not_configured` |
//! | `GET /v1/operator/jobs/{job_id}` | `200` frozen job JSON | `400 bad_request`, `401 unauthorized`, `404 not_found`, `503 operator_jobs_not_configured` |
//!
//! The operator-jobs receiver moves the control plane's durable job writes off
//! the shared disk: `POST /v1/operator/jobs` accepts the frozen job JSON (the
//! schema in `apex_autopilot::operator_jobs`) and persists the exact request
//! bytes into `APEX_OPERATOR_JOBS_DIR` (default `.apex/control/jobs`, the
//! directory the runner's daily pass consumes) with the control plane's
//! atomic temp-file-plus-rename discipline. Writes are idempotent by
//! `job_id`: a duplicate POST answers `200` with the existing file and
//! rewrites nothing. Both routes are gated by a bearer token from
//! `APEX_OPERATOR_JOBS_TOKEN` with the live-audit semantics: no configured
//! token is `503 operator_jobs_not_configured`, a missing or wrong bearer is
//! `401 unauthorized`.
//!
//! Unknown paths fall through to the fallback: `/v1/*`, `/healthz`,
//! `/metrics`, `/assets/*` and `/favicon*` keep
//! `404 {"error":"not_found"}`, while browser paths render the matching HTML
//! 404 shell (console for `/console/*`, marketing elsewhere). A known path
//! with the wrong method is axum's `405` (asserted by tests) on the API
//! routes, and the HTML 405 hint page on the console routes. The public POST
//! surfaces are rate limited per client key and route (see
//! [`PUBLIC_POST_LIMIT_PER_MINUTE`]).
//!
//! Console routes (server-rendered HTML, no JavaScript; every mutation is a
//! POST that answers `303 See Other`):
//!
//! Console pages are session-scoped: every console GET except the change-log
//! access panel itself (`GET /console/ledger`), the bare `/console` redirect,
//! the one-time sign-in link (`GET /console/signin`) and the public
//! free-audit and setup-watch sign-up flows requires a live session and
//! answers `303` to
//! `/console/ledger` without one; the session's bound publisher scopes the
//! page (there is no deployment-wide console publisher). A deployment with no
//! configured console credentials at all (no ledger tokens and no self-serve
//! workspace) can never mint a session, so its workspace pages keep the `503`
//! not-configured panel instead of redirecting to an access panel that could
//! not authorize anyone. The `303` outranks per-page query validation: an
//! anonymous request never learns whether its query was well formed.
//!
//! | route | success | states |
//! |---|---|---|
//! | `GET /` | `200` overview, for the session's publisher | first-run state when no source exists |
//! | `GET /console` | `303` to the change log | — |
//! | `GET /console/audit` | `200` form or report | `400` bad query / unknown platform, `404` no snapshot, `500` read failure |
//! | `GET /console/audit/findings/{id}` | `200` finding detail | `404` unknown id, `500` read failure |
//! | `GET /console/audit/export.csv` | `200` RFC 4180 CSV | `400` missing or unknown platform, `404` no snapshot, `500` read failure |
//! | `GET /console/ledger` | `200` access panel or verified records | `404` no ledger, `422` ledger_unverified, `500` read failure |
//! | `POST /console/ledger` | `303` to the change log + session cookie | `400` invalid publisher, `401` invalid access, `503` ledger_auth_not_configured |
//! | `GET /console/ledger/records/{seq}` | `200` record detail | `303` to the change log without a session, `404` unknown sequence |
//! | `POST /console/ledger/session/end` | `303` to the change log | `400` missing or wrong CSRF token |
//! | `GET /console/signin?token=` | `200` landing page with a POST button (never consumes the link) | `200` refusal page for a used, expired or unknown link, `400` for a missing or malformed token, `503` when the sign-in store is not configured |
//! | `POST /console/signin` | `303` to `/` with a session cookie | `200` refusal page for a used, expired or unknown token, `400` for a missing or malformed token, `503` when the sign-in store is not configured |
//! | `GET /console/connections` | `200` platform list | `303` without a session, `503` not configured (no console credentials), `500` store read failure |
//! | `GET /console/connections/{platform}` | `200` detail | `404` unknown platform |
//! | `POST /console/connections/{platform}/submit` | `303` back to the detail | `303` to the change log without a session, `400` field errors or wrong CSRF token, `404` unknown platform, `503` vault not configured |
//! | `POST /console/connections/{platform}/rotate` | `303` | same refusals as submit |
//! | `POST /console/connections/{platform}/disconnect` | `303` | same refusals as submit |
//! | `GET /console/proposals/{id}` | `200` review page | `404` unknown proposal, `503` proposals root not configured |
//! | `POST /console/proposals/{id}/decision` | `303` to the decided page (retry: original status) | `303` to the change log without a session, `400` missing idempotency key or CSRF, `404` unknown proposal, `409` already decided differently, `503` |
//! | `GET /console/experiments` | `200` list | `400` unknown filter, `404`/`422` ledger states |
//! | `GET /console/experiments/{id}` | `200` detail (GET-link tabs) | `400` unknown tab, `404` unknown experiment |
//! | `GET /console/experiments/{id}/incident` | `200` recovery state | `404` no incident recorded |
//! | `GET /console/billing` | `200` register | `404`/`422` ledger states |
//! | `GET /console/billing/invoices/{period}` | `200` fee calculation | `404` unknown period |
//! | `GET /console/settings` | `200` account | — |
//! | `POST /console/settings/theme` | `303` with the theme cookie | `303` to the change log without a session, `400` invalid value or CSRF |
//! | `GET /console/settings/access` | `200` session list | — |
//! | `POST /console/settings/access/revoke` | `303` | `303` without a session, `400` bad handle or CSRF |
//! | `GET /console/settings/policy` | `200` recorded policy | — |
//! | `GET /console/settings/monitoring` | `200` free monitoring health | `500` connection store read failure |
//! | `GET /console/analytics?view=&format=csv` | `200` tab, or CSV | `400` unknown view or platform filter, `503` store not configured (CSV exports answer `503 text/plain` with the reason) |
//! | `POST /free-audit` | `303` to `/audit-requested/?ref=` (workspace recorded; a new one needs its email domain's app-ads.txt proof, an existing one accepts only recorded contacts; the one-time sign-in link is mailed to that contact, best effort) | `400` field errors or a typed workspace refusal (unknown contact / unproven domain), `500`/`503` storage failure |
//! | `GET /free-audit` | `303` to `/free-audit/` | — |
//! | `GET /free-audit/` | `200` request page | — |
//! | `GET /audit-requested/` | `200` confirmation | — |
//! | `POST /watch` | `303` to `/watch/requested?ref=` (pending subscriber and confirmation email only; no checkout link is created; tier from the verified App Store/Play developer catalogues; repeat submissions inside 24 h deduplicate) | `400` field errors, `500`/`503` store unavailable |
//! | `GET /watch` | `303` to `/watch/` | — |
//! | `GET /watch/` | `200` sign-up page | — |
//! | `GET /watch/requested` | `200` free-watch confirmation; no checkout link, tier price or invoice | — |
//! | `GET /watch/confirm?token=` | `200` the inert confirmation button (the token is never consumed) | `400` missing or malformed token, `503` watch store not configured |
//! | `POST /watch/confirm` | `200` the billing-profile form after a confirmed email (or the refusal page) | `400` missing or malformed token, `503` watch store not configured |
//! | `POST /watch/billing-profile` | `303` to `/watch/billing-profile/recorded?ref=` after recording the profile and activating the free watch | `400` field errors or a spent/mismatched profile token, `200` used/expired/unknown token refusal, `503` watch store not configured |
//! | `GET /watch/billing-profile/recorded?ref=` | `200` stable recorded page | — |
//!
//! Refusals produced inside route extraction follow the same audience split:
//! a malformed path segment or duplicated query key on `/v1/*` is
//! `400 {"error":"bad_request"}`, the same failure on `/console/*` is the HTML
//! error page with status `400`. Request bodies are capped at
//! [`MAX_BODY_BYTES`] (256 KiB) via [`DefaultBodyLimit`]; an oversized body is
//! refused with `413` in the audience's shape (JSON on `/v1/*`, HTML
//! elsewhere), whether it is declared up front by `Content-Length` or caught
//! by the extractor inside the handler.

use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::uri::Authority;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tower_http::trace::TraceLayer;

use apex_connectors::transport::{HttpMethod, HttpRequest, Transport, TransportError};
use apex_core::PlatformId;

use crate::cycle::{CycleRequest, CycleSummary, READ_ONLY_MODE};
use crate::error::ApiError;
use crate::html::audit::{is_finding_id, AuditFormProblem, AuditReportView, FindingView};
use crate::html::leads_html::RequestPageState;
use crate::html::ledger::LedgerAccessProblem;
use crate::html::overview::{LatestAudit, LedgerState, OverviewData};
use crate::html::watch_html::{self, WatchFormValues, WatchPageState};
use crate::html::{audit, errors, leads_html, ledger, overview, ConsoleCtx};
use crate::leads::{self, Lead};
use crate::live::constant_time_eq;
use crate::session;
use crate::sources::{is_valid_publisher, parse_platform, PLATFORM_NAMES};
use crate::state::AppState;

/// Hard cap on every request body, enforced by [`DefaultBodyLimit`] and by the
/// pre-routing `Content-Length` check below. 256 KiB is far above the largest
/// legitimate body (the optional cycle policy JSON) and bounds the memory a
/// single request can make the server buffer.
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// Requests allowed per client key and route bucket in one minute for the
/// public POST surfaces. Generous enough for a legitimate form flow.
pub const PUBLIC_POST_LIMIT_PER_MINUTE: u32 = 30;
/// The sign-in exchange is the most abuse-sensitive public POST.
pub const SIGNIN_POST_LIMIT_PER_MINUTE: u32 = 10;
/// The fixed window each key is counted in.
pub const PUBLIC_RATE_LIMIT_WINDOW_MS: i64 = 60_000;
/// Hard bound on the limiter's memory: the oldest key is evicted first.
pub const PUBLIC_RATE_LIMIT_KEYS: usize = 4096;

#[derive(Debug)]
struct RateWindow {
    started_ms: i64,
    count: u32,
}

#[derive(Debug, Default)]
struct RateLimitInner {
    windows: HashMap<String, RateWindow>,
    /// Insertion order of the keys currently in `windows`.
    order: VecDeque<String>,
}

/// A bounded, in-memory fixed-window rate limiter for the public POST
/// surfaces. It is deliberately process-local: the edge proxy and the
/// per-route business limits remain the durable controls.
#[derive(Debug)]
pub struct PublicRateLimiter {
    inner: Mutex<RateLimitInner>,
    capacity: usize,
    window_ms: i64,
}

impl PublicRateLimiter {
    /// A limiter with explicit bounds.
    pub fn new(capacity: usize, window_ms: i64) -> Self {
        Self {
            inner: Mutex::new(RateLimitInner::default()),
            capacity: capacity.max(1),
            window_ms: window_ms.max(1),
        }
    }

    /// The deployment defaults: 4096 keys, one-minute windows.
    pub fn bounded() -> Self {
        Self::new(PUBLIC_RATE_LIMIT_KEYS, PUBLIC_RATE_LIMIT_WINDOW_MS)
    }

    /// Count one request for `key`. Returns `false` (without counting) when
    /// the key already used `limit` requests in the current window. At
    /// capacity the oldest key is evicted to keep memory bounded.
    pub fn allow(&self, key: &str, now_ms: i64, limit: u32) -> bool {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let expired = inner
            .windows
            .get(key)
            .is_some_and(|window| now_ms.saturating_sub(window.started_ms) >= self.window_ms);
        if expired {
            if let Some(window) = inner.windows.get_mut(key) {
                window.started_ms = now_ms;
                window.count = 0;
            }
        } else if !inner.windows.contains_key(key) {
            while inner.windows.len() >= self.capacity {
                match inner.order.pop_front() {
                    Some(oldest) => {
                        inner.windows.remove(&oldest);
                    }
                    None => break,
                }
            }
            inner.order.push_back(key.to_string());
            inner.windows.insert(
                key.to_string(),
                RateWindow {
                    started_ms: now_ms,
                    count: 0,
                },
            );
        }
        let Some(window) = inner.windows.get_mut(key) else {
            return false;
        };
        if window.count >= limit {
            return false;
        }
        window.count = window.count.saturating_add(1);
        true
    }

    /// Number of keys currently retained (tests).
    pub fn len(&self) -> usize {
        match self.inner.lock() {
            Ok(guard) => guard.windows.len(),
            Err(poisoned) => poisoned.into_inner().windows.len(),
        }
    }

    /// True when no key is retained (tests).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The per-minute limit of `path`, or `None` when the path is not one of the
/// rate-limited public POST surfaces.
fn public_post_limit(path: &str) -> Option<u32> {
    match path {
        "/console/signin" => Some(SIGNIN_POST_LIMIT_PER_MINUTE),
        "/v1/mollie/webhook" => Some(120),
        "/free-audit"
        | "/watch"
        | "/watch/confirm"
        | "/watch/billing-profile"
        | "/watch/cancel"
        | "/unsubscribe"
        | "/console/ledger"
        | "/v1/leads" => Some(PUBLIC_POST_LIMIT_PER_MINUTE),
        _ => None,
    }
}

/// The rate-limit client key: the **last** `X-Forwarded-For` entry (the hop
/// the trusted proxy appended). Earlier entries are client-supplied. The
/// value is bounded so a hostile header cannot grow the map key without
/// bound. A request without a header shares one `direct` bucket rather than
/// skipping the limiter; that closes the loopback/proxy-misconfiguration
/// bypass while every proxied client carries its own key.
fn forwarded_client_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next_back())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(64).collect())
}

fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The 429 shape: JSON on `/v1/*`, the house HTML page everywhere else.
fn rate_limited_response(path: &str) -> Response {
    if path.starts_with("/v1/") {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "rate_limited",
                "retry_after_seconds": PUBLIC_RATE_LIMIT_WINDOW_MS / 1000,
            })),
        )
            .into_response();
    }
    let ctx = ConsoleCtx::from_env();
    if path == "/console" || path.starts_with("/console/") {
        page(
            StatusCode::TOO_MANY_REQUESTS,
            errors::too_many_requests(&ctx),
        )
    } else {
        let body = "<section class=\"panel\"><h1 class=\"page-title\">Too many requests.</h1>\
                    <p class=\"panel-body\">Wait a moment, then try again.</p>\
                    </section>";
        page(
            StatusCode::TOO_MANY_REQUESTS,
            crate::html::marketing_document(&ctx, "Too many requests", body),
        )
    }
}

/// Rate-limit the named public POST routes before their handler runs.
async fn public_post_rate_limit(
    State(limiter): State<Arc<PublicRateLimiter>>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() != Method::POST {
        return next.run(request).await;
    }
    let path = request.uri().path().to_owned();
    let Some(limit) = public_post_limit(&path) else {
        return next.run(request).await;
    };
    // A request without a forwarded client is still counted, under one
    // shared direct bucket. Skipping it would make a direct loopback caller
    // (or a proxy that forgot the header) unbounded.
    let client = forwarded_client_key(request.headers()).unwrap_or_else(|| "direct".to_string());
    let key = format!("{path}\u{1f}{client}");
    if !limiter.allow(&key, system_now_ms(), limit) {
        return rate_limited_response(&path);
    }
    next.run(request).await
}

/// Build the complete router (API + console) over `state`.
///
/// The console routes are registered before
/// [`Router::method_not_allowed_fallback`] so only they get the HTML 405 hint
/// page; the API routes keep axum's default `405` with its `Allow` header.
pub fn router(state: AppState) -> Router {
    if let Ok(path) = std::env::var(crate::signin::LEDGER_TOKENS_FILE_ENV) {
        let path = path.trim();
        if !path.is_empty() {
            crate::signin::warn_if_world_readable(
                std::path::Path::new(path),
                "the ledger token file",
            );
        }
    }
    Router::new()
        .route("/", get(console_overview))
        .route("/console", get(console_bare))
        .route("/console/", get(console_bare))
        .route("/console/audit", get(console_audit))
        .route("/console/audit/export.csv", get(console_audit_csv))
        .route("/console/audit/findings/{id}", get(console_finding))
        .route(
            "/console/ledger",
            get(console_ledger).post(console_ledger_open),
        )
        .route("/console/ledger/records/{seq}", get(console_ledger_record))
        .route(
            "/console/ledger/session/end",
            post(console_ledger_session_end),
        )
        .route(
            "/console/signin",
            get(console_signin).post(console_signin_submit),
        )
        .route("/console/connections", get(console_connections))
        .route(
            "/console/connections/{platform}",
            get(console_connection_detail),
        )
        .route(
            "/console/connections/{platform}/submit",
            post(console_connection_submit),
        )
        .route(
            "/console/connections/{platform}/rotate",
            post(console_connection_rotate),
        )
        .route(
            "/console/connections/{platform}/disconnect",
            post(console_connection_disconnect),
        )
        .route("/console/proposals", get(console_proposals))
        .route("/console/proposals/{id}", get(console_proposal))
        .route(
            "/console/proposals/{id}/decision",
            post(console_proposal_decision),
        )
        .route("/console/experiments", get(console_experiments))
        .route("/console/experiments/{id}", get(console_experiment_detail))
        .route(
            "/console/experiments/{id}/incident",
            get(console_experiment_incident),
        )
        .route("/console/billing", get(console_billing))
        .route(
            "/console/billing/invoices/{period}",
            get(console_invoice_detail),
        )
        .route("/console/settings", get(console_settings))
        .route("/console/settings/theme", post(console_settings_theme))
        .route("/console/settings/access", get(console_settings_access))
        .route(
            "/console/settings/access/revoke",
            post(console_settings_access_revoke),
        )
        .route("/console/settings/policy", get(console_settings_policy))
        .route(
            "/console/settings/monitoring",
            get(console_settings_monitoring),
        )
        .route("/console/settings/billing", get(console_settings_billing))
        .route(
            "/console/settings/billing/managed/cancel",
            post(console_settings_billing_managed_cancel),
        )
        .route("/console/analytics", get(console_analytics))
        .route(
            "/free-audit",
            get(free_audit_redirect).post(free_audit_submit),
        )
        .route("/free-audit/", get(free_audit_page))
        .route("/audit-requested/", get(audit_requested))
        .route("/watch", get(watch_redirect).post(watch_submit))
        .route("/watch/", get(watch_page))
        .route("/watch/requested", get(watch_requested))
        .route("/watch/cancel", get(watch_cancel).post(watch_cancel_submit))
        .route(
            "/watch/confirm",
            get(watch_confirm).post(watch_confirm_submit),
        )
        .route(
            "/unsubscribe",
            get(unsubscribe_page).post(unsubscribe_submit),
        )
        .route(
            "/unsubscribe/",
            get(unsubscribe_page).post(unsubscribe_submit),
        )
        .route("/watch/billing-profile", post(watch_billing_profile))
        .route(
            "/watch/billing-profile/recorded",
            get(watch_billing_profile_recorded),
        )
        .method_not_allowed_fallback(console_method_not_allowed)
        .route("/healthz", get(healthz))
        .route("/favicon.ico", get(crate::favicon::ico))
        .route("/favicon.svg", get(crate::favicon::svg))
        .route("/assets/apex.css", get(crate::fonts::apex_css))
        .route("/assets/fonts/{name}", get(crate::fonts::font))
        .route("/metrics", get(crate::metrics::metrics))
        .route("/v1/audit/{platform}", get(audit_report).post(live_audit))
        .route("/v1/ledger/{publisher}", get(ledger))
        .route("/v1/cycle/{publisher}", post(run_cycle))
        .route("/v1/postbacks/max", post(max_postbacks))
        .route("/v1/mollie/webhook", post(mollie_webhook))
        .route("/v1/leads", post(lead_capture))
        .route("/v1/operator/jobs", post(operator_jobs_submit))
        .route("/v1/operator/jobs/{job_id}", get(operator_jobs_show))
        .route("/v1/operator/governance", post(operator_governance))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(
            Arc::new(PublicRateLimiter::bounded()),
            public_post_rate_limit,
        ))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn(shape_oversized_responses))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::metrics::track_requests,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Refuse a body that declares itself larger than [`MAX_BODY_BYTES`] before
/// routing, and re-shape any `413` produced by the body extractors afterwards
/// so an oversized body always answers in the audience's contract shape.
async fn shape_oversized_responses(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if declared.is_some_and(|length| length > MAX_BODY_BYTES as u64) {
        return body_too_large(&path);
    }
    let response = next.run(request).await;
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return body_too_large(&path);
    }
    response
}

/// The `413` response for `path`: the pinned JSON refusal on `/v1/*`, the
/// HTML error page everywhere else.
fn body_too_large(path: &str) -> Response {
    if path.starts_with("/v1/") {
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({ "error": "bad_request" })),
        )
            .into_response()
    } else {
        page(
            StatusCode::PAYLOAD_TOO_LARGE,
            errors::body_too_large(&ConsoleCtx::from_env(), MAX_BODY_BYTES / 1024),
        )
    }
}

/// Whether an unknown path belongs to a machine surface and keeps the JSON
/// `404 {"error":"not_found"}` answer.
fn machine_not_found(path: &str) -> bool {
    path == "/v1"
        || path.starts_with("/v1/")
        || path.starts_with("/healthz")
        || path.starts_with("/metrics")
        || path.starts_with("/assets/")
        || path.starts_with("/favicon")
}

/// Browser-safe fallback: machine prefixes keep the JSON refusal, console
/// paths render the console shell's 404 page and every other unknown path
/// renders the marketing shell's 404 page.
async fn not_found(request: Request) -> Response {
    let path = request.uri().path().to_owned();
    if machine_not_found(&path) {
        return ApiError::NotFound.into_response();
    }
    if path == "/console" || path.starts_with("/console/") {
        return page(
            StatusCode::NOT_FOUND,
            errors::not_found(&ConsoleCtx::from_env()),
        );
    }
    let body = "<section class=\"panel\"><h1 class=\"page-title\">Not found.</h1>\
                <p class=\"panel-body\">The address does not match any page on this site.</p>\
                <p class=\"panel-body\"><a class=\"action\" href=\"/\">Open the overview</a></p>\
                </section>";
    page(
        StatusCode::NOT_FOUND,
        crate::html::marketing_document(&ConsoleCtx::from_env(), "Not found", body),
    )
}

/// The HTML 405 hint page for console routes (never attached to `/v1/*`,
/// `/healthz` or `/metrics`, which keep axum's default `405` + `Allow`).
async fn console_method_not_allowed(_request: Request) -> Response {
    page(
        StatusCode::METHOD_NOT_ALLOWED,
        errors::method_not_allowed(&ConsoleCtx::from_env()),
    )
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

/// Query parameters of `GET /v1/audit/{platform}`.
#[derive(Debug, Deserialize, Default)]
struct AuditReportParams {
    /// The publisher whose own snapshot is reported. Required: there is no
    /// deployment-wide default, so every read names exactly one publisher.
    #[serde(default)]
    publisher: Option<String>,
}

/// `GET /v1/audit/{platform}`: the read-only report for one publisher's own
/// snapshot.
///
/// This route carries the same bearer-token gate as `POST
/// /v1/audit/{platform}` and `/v1/cycle`: no configured auditor is `503
/// live_audit_not_configured`, a missing or wrong bearer is `401`. The
/// publisher is named by `?publisher=` (there is no deployment-wide default),
/// so an operator reads exactly one publisher's snapshot — never the whole
/// deployment's.
async fn audit_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    platform: Result<Path<String>, PathRejection>,
    params: Result<Query<AuditReportParams>, QueryRejection>,
) -> Result<Json<apex_audit::AuditReport>, ApiError> {
    let Some(auditor) = &state.live_auditor else {
        return Err(ApiError::LiveAuditNotConfigured);
    };
    auditor.authorize(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    )?;
    let Ok(Path(platform)) = platform else {
        return Err(ApiError::BadRequest("bad_request"));
    };
    let platform = parse_platform(&platform).ok_or(ApiError::BadRequest("unknown_platform"))?;
    let Ok(Query(params)) = params else {
        return Err(ApiError::BadRequest("bad_request"));
    };
    let publisher = params
        .publisher
        .ok_or(ApiError::BadRequest("publisher_required"))?;
    if !is_valid_publisher(&publisher) {
        return Err(ApiError::BadRequest("invalid_publisher"));
    }
    let (_, report) = state
        .audit_source
        .snapshot_report(&publisher, platform)
        .await?;
    Ok(Json(report))
}

/// Request body of `POST /v1/audit/{platform}`.
#[derive(Debug, Deserialize)]
struct LiveAuditBody {
    publisher: String,
}

/// Live read-only audit: build the snapshot from the publisher's own platform
/// keys and run the revenue-leak rules. The platform is never written to.
async fn live_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    platform: Result<Path<String>, PathRejection>,
    body: Result<Json<LiveAuditBody>, JsonRejection>,
) -> Result<Json<apex_audit::AuditReport>, ApiError> {
    let Some(auditor) = &state.live_auditor else {
        return Err(ApiError::LiveAuditNotConfigured);
    };
    auditor.authorize(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    )?;
    let Ok(Path(platform)) = platform else {
        return Err(ApiError::BadRequest("bad_request"));
    };
    let platform = parse_platform(&platform).ok_or(ApiError::BadRequest("unknown_platform"))?;
    let Ok(Json(body)) = body else {
        return Err(ApiError::BadRequest("bad_request"));
    };
    if !is_valid_publisher(&body.publisher) {
        return Err(ApiError::BadRequest("invalid_publisher"));
    }
    let (snapshot, report) = match auditor.audit_with_snapshot(&body.publisher, platform).await {
        Ok(value) => value,
        Err(error) => {
            state
                .analytics
                .emit_audit_failed(
                    now_unix_ms(&state),
                    &body.publisher,
                    platform.as_str(),
                    env!("CARGO_PKG_NAME"),
                    audit_error_class(&error),
                )
                .await;
            return Err(error);
        }
    };
    // Persist the snapshot so the read-only audit source and the write cycle
    // can consume the same bytes the live audit just built, keyed to the
    // publisher whose keys produced it.
    if let Err(error) = state
        .audit_source
        .write(&body.publisher, platform, &snapshot)
        .await
    {
        tracing::warn!(%error, "live audit snapshot could not be persisted");
    }
    emit_audit_completed(&state, &body.publisher, platform, &snapshot, &report).await;
    Ok(Json(report))
}

/// One bounded error class for the failure event.
fn audit_error_class(error: &ApiError) -> &'static str {
    match error {
        ApiError::BadRequest("invalid_credentials") => "auth_refused",
        ApiError::BadRequest("platform_not_onboarded") => "not_onboarded",
        ApiError::PlatformUnavailable(_) => "platform_unavailable",
        _ => "other",
    }
}

async fn emit_audit_completed(
    state: &AppState,
    publisher: &str,
    platform: PlatformId,
    snapshot: &apex_audit::Snapshot,
    report: &apex_audit::AuditReport,
) {
    // The engine evaluated all five documented rule groups; the revenue
    // groups depend on the 30-day reporting rows, so their absence is named
    // honestly instead of pretending the inputs existed.
    let missing_inputs: Vec<String> = if snapshot.country_revenue.is_empty() {
        vec!["country revenue".to_string()]
    } else {
        Vec::new()
    };
    let mut critical = 0u32;
    let mut warning = 0u32;
    let mut info = 0u32;
    for finding in &report.findings {
        match finding.severity {
            apex_audit::Severity::Critical => critical += 1,
            apex_audit::Severity::Warning => warning += 1,
            apex_audit::Severity::Info => info += 1,
        }
    }
    // The engine evaluates all five documented rule groups over every
    // snapshot; groups whose inputs are absent simply produce no findings.
    let rule_groups = 5u32;
    state
        .analytics
        .emit_audit_completed(
            now_unix_ms(state),
            publisher,
            platform.as_str(),
            &report.snapshot_digest,
            report.generated_by,
            rule_groups,
            rule_groups,
            critical,
            warning,
            info,
            missing_inputs,
            0,
        )
        .await;
}

/// Capture a free-audit lead: validate the shape, store it append-only, and
/// forward it to the mail webhook when one is configured. Identical retries
/// inside 24 hours deduplicate to the original request id inside the store.
async fn lead_capture(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let Some(store) = &state.leads else {
        return Err(ApiError::LeadCaptureNotConfigured);
    };
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let mut lead =
        leads::parse_lead(&body, content_type).map_err(|_| ApiError::BadRequest("invalid_lead"))?;
    // A malformed publisher is refused even when an unauthenticated call
    // would derive its own: the input contract is checked before the
    // attribution rule.
    if !lead.publisher.is_empty() && !is_valid_publisher(&lead.publisher) {
        return Err(ApiError::BadRequest("invalid_lead"));
    }
    match state.leads_token.as_deref() {
        Some(expected) => {
            let presented = headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .unwrap_or_default();
            if !constant_time_eq(presented.as_bytes(), expected) {
                return Err(ApiError::Unauthorized);
            }
        }
        None => {
            // Unauthenticated lead capture may not choose a tenancy: the
            // publisher comes from the email domain, exactly like the
            // website's free-audit submission.
            if let Some((_, domain)) = lead.email.rsplit_once('@') {
                lead.publisher = apex_watch::model::publisher_slug_for_domain(domain);
            }
        }
    }
    leads::validate_lead(&lead).map_err(|_| ApiError::BadRequest("invalid_lead"))?;
    let now_unix_ms = state.clock.now_unix_s().saturating_mul(1000);
    let request_id = store.store(&lead, now_unix_ms).await?;
    let (from_day, to_day) = default_audit_window(now_unix_ms);
    state
        .analytics
        .emit_audit_request_accepted(
            now_unix_ms,
            &request_id,
            lead.platforms.clone(),
            &from_day,
            &to_day,
            "api",
        )
        .await;
    Ok((StatusCode::ACCEPTED, Json(json!({ "received": true }))))
}

/// The default audit window: the 30 complete days ending yesterday.
fn default_audit_window(now_unix_ms: i64) -> (String, String) {
    let to = apex_analytics::UtcMillis::from_unix_millis(now_unix_ms.saturating_sub(86_400_000))
        .to_day_string();
    let from =
        apex_analytics::UtcMillis::from_unix_millis(now_unix_ms.saturating_sub(30 * 86_400_000))
            .to_day_string();
    (from, to)
}

// ---------------------------------------------------------------------------
// Operator jobs: the split-host receiver
// ---------------------------------------------------------------------------

/// Bearer token gating the operator-jobs routes; unset means the receiver is
/// closed (`503`).
pub const OPERATOR_JOBS_TOKEN_ENV: &str = "APEX_OPERATOR_JOBS_TOKEN";
/// The jobs directory shared with the runner's daily pass.
pub const OPERATOR_JOBS_DIR_ENV: &str = "APEX_OPERATOR_JOBS_DIR";
/// The shared default jobs directory (the runner's and the control plane's
/// default).
pub const DEFAULT_OPERATOR_JOBS_DIR: &str = ".apex/control/jobs";

/// One bounded serial number for unique temp names: the receiver is stateless
/// (two concurrent POSTs of one `job_id` share no lock), so each write uses
/// its own temp file.
static OPERATOR_JOB_TEMP_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Serializes intervention appends inside this process; combined with the
/// single `write_all` of one complete line below, two racing requests can
/// never interleave bytes in the governance register.
static GOVERNANCE_APPEND_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The configured jobs directory: `APEX_OPERATOR_JOBS_DIR` when set and
/// non-blank, the shared default otherwise.
fn operator_jobs_dir() -> std::path::PathBuf {
    std::env::var(OPERATOR_JOBS_DIR_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(DEFAULT_OPERATOR_JOBS_DIR))
}

/// Authorize one operator-jobs request with the live-audit bearer semantics
/// (`crate::live::LiveAuditor::authorize`): the configured token must arrive
/// as `Authorization: Bearer <token>` and is compared in constant time; a
/// configured-but-empty token refuses every request.
fn authorize_operator_jobs(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
    if expected.is_empty() {
        return Err(ApiError::Unauthorized);
    }
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

/// `POST /v1/operator/governance`: the control plane's immediate governance
/// channel for the split-host topology. The runner and the console read the
/// pause register and the effective policy on this host, so a control-plane
/// pause or policy change must land here, not only in the control host's
/// local register.
async fn operator_governance(
    State(state_app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = crate::config::env_value(OPERATOR_JOBS_TOKEN_ENV).unwrap_or_default();
    if token.trim().is_empty() {
        return operator_jobs_not_configured();
    }
    if let Err(error) = authorize_operator_jobs(&headers, &token) {
        return error.into_response();
    }
    let request: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "bad_request"})),
            )
                .into_response()
        }
    };
    match request.get("kind").and_then(serde_json::Value::as_str) {
        Some("intervention") => {
            let Some(record) = request.get("record") else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "bad_request"})),
                )
                    .into_response();
            };
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Check {
                kind: String,
                #[serde(default)]
                platform: Option<String>,
                #[serde(default)]
                reason: String,
                at_ms: i64,
            }
            let Ok(check) = serde_json::from_value::<Check>(record.clone()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "bad_request"})),
                )
                    .into_response();
            };
            let kind_ok = matches!(
                check.kind.as_str(),
                "pause_scheduling" | "resume_scheduling"
            );
            let platform_ok = check.platform.as_deref().is_none_or(|platform| {
                !platform.is_empty()
                    && platform.len() <= 64
                    && platform
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            });
            if !kind_ok || !platform_ok || check.at_ms <= 0 || check.reason.chars().count() > 500 {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "bad_request"})),
                )
                    .into_response();
            }
            let path = crate::config::env_value("APEX_CONTROL_INTERVENTIONS")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::PathBuf::from(apex_autopilot::interventions::DEFAULT_INTERVENTIONS)
                });
            let line = match serde_json::to_string(record) {
                Ok(line) => line,
                Err(_) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "bad_request"})),
                    )
                        .into_response()
                }
            };
            if let Err(error) = append_governance_line(&path, &line) {
                tracing::error!(%error, path = %path.display(), "intervention could not be recorded");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "internal"})),
                )
                    .into_response();
            }
            (
                StatusCode::OK,
                Json(json!({"applied": true, "kind": "intervention"})),
            )
                .into_response()
        }
        Some("managed_plan") => {
            let publisher = request
                .get("publisher")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|publisher| is_valid_publisher(publisher));
            let Some(publisher) = publisher else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "bad_request"})),
                )
                    .into_response();
            };
            let state = match request
                .get("state")
                .cloned()
                .map(serde_json::from_value::<apex_billing::PlanState>)
            {
                Some(Ok(state)) => state,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "bad_request"})),
                    )
                        .into_response()
                }
            };
            if state.plan != apex_billing::PlanId::Managed {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "bad_request"})),
                )
                    .into_response();
            }
            let Some(root) = connection_root(&state_app) else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({"error": "not_configured"})),
                )
                    .into_response();
            };
            let path =
                apex_billing::PlanState::path_for(root, publisher, apex_billing::PlanId::Managed);
            if let Err(error) = write_plan_state_atomically(&path, &state) {
                tracing::error!(%error, path = %path.display(), "managed plan state could not be written");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "internal"})),
                )
                    .into_response();
            }
            (
                StatusCode::OK,
                Json(json!({"applied": true, "kind": "managed_plan"})),
            )
                .into_response()
        }
        Some("policy") => {
            let policy = match request
                .get("policy")
                .cloned()
                .map(serde_json::from_value::<apex_autopilot::model::AutopilotPolicy>)
            {
                Some(Ok(policy)) => policy,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "bad_request"})),
                    )
                        .into_response()
                }
            };
            if let Err(reason) = policy.validate() {
                tracing::warn!(%reason, "control-plane policy was refused");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid_policy"})),
                )
                    .into_response();
            }
            let path = crate::config::env_value(crate::settings::EFFECTIVE_POLICY_ENV)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::PathBuf::from(crate::settings::DEFAULT_EFFECTIVE_POLICY)
                });
            if let Err(error) = write_policy_atomically(&path, &policy) {
                tracing::error!(%error, path = %path.display(), "effective policy could not be written");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "internal"})),
                )
                    .into_response();
            }
            (
                StatusCode::OK,
                Json(json!({"applied": true, "kind": "policy"})),
            )
                .into_response()
        }
        _ => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "bad_request"})),
        )
            .into_response(),
    }
}

/// Append one governance line with a single durability barrier and 0600.
fn append_governance_line(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let _guard = match GOVERNANCE_APPEND_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut file = options.open(path)?;
    // One complete line in one call: two concurrent interventions can never
    // interleave inside a record the readers parse line-by-line.
    let mut record = Vec::with_capacity(line.len() + 1);
    record.extend_from_slice(line.as_bytes());
    record.push(b'\n');
    file.write_all(&record)?;
    file.sync_data()?;
    drop(file);
    crate::config::sync_parent(path)
}

/// Write one publisher's plan state atomically with mode 0600, the same bytes
/// and path the console and runner read.
fn write_plan_state_atomically(
    path: &std::path::Path,
    state: &apex_billing::PlanState,
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let temp = path.with_file_name(format!(
        ".{}.{}.{nanos}.tmp",
        path.file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("plan"),
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        let mut file = options.open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path)?;
    crate::config::sync_parent(path)
}

/// Write the effective policy atomically so the runner never reads a torn
/// document, with mode 0600.
fn write_policy_atomically(
    path: &std::path::Path,
    policy: &apex_autopilot::model::AutopilotPolicy,
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(policy)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let temp = path.with_file_name(format!(
        ".{}.{}.{nanos}.tmp",
        path.file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("policy"),
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        let mut file = options.open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path)?;
    crate::config::sync_parent(path)
}

/// The fail-closed `503` when `APEX_OPERATOR_JOBS_TOKEN` is unset; the
/// taxonomy lives in `error.rs`, which this additive route does not extend.
fn operator_jobs_not_configured() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "operator_jobs_not_configured" })),
    )
        .into_response()
}

/// The frozen job id shape (`opj_` plus twelve lowercase hex digits) that the
/// runner and the control plane both enforce before trusting a job file. It
/// is also the path-safety check: only this shape reaches the filesystem.
fn is_frozen_job_id(job_id: &str) -> bool {
    let Some(hex) = job_id.strip_prefix("opj_") else {
        return false;
    };
    hex.len() == 12
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A `200 application/json` answer carrying the frozen job bytes verbatim, so
/// the receiver never reshapes the producer's recording.
fn job_json_response(body: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        body,
    )
        .into_response()
}

/// `POST /v1/operator/jobs`: validate the frozen job JSON, then persist the
/// exact request bytes into the shared jobs folder atomically. A duplicate
/// `job_id` returns the existing job unchanged (`200`, no rewrite).
async fn operator_jobs_submit(headers: HeaderMap, body: Bytes) -> Result<Response, ApiError> {
    let token = match std::env::var(OPERATOR_JOBS_TOKEN_ENV) {
        Ok(token) => token,
        Err(_) => return Ok(operator_jobs_not_configured()),
    };
    authorize_operator_jobs(&headers, &token)?;
    let job: apex_autopilot::OperatorJob =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest("bad_request"))?;
    let history_matches_state = job
        .state_history
        .last()
        .is_some_and(|change| change.state == job.state);
    if !is_frozen_job_id(&job.job_id)
        || job.kind.is_empty()
        || job.kind.len() > 64
        || !job.kind.is_ascii()
        || !history_matches_state
        || apex_autopilot::operator_jobs::JobState::parse(&job.state).is_none()
    {
        return Err(ApiError::BadRequest("bad_request"));
    }
    let dir = operator_jobs_dir();
    let path = dir.join(format!("{}.json", job.job_id));
    if path.exists() {
        let existing =
            std::fs::read(&path).map_err(|error| ApiError::Internal(error.to_string()))?;
        return Ok(job_json_response(existing));
    }
    std::fs::create_dir_all(&dir).map_err(|error| ApiError::Internal(error.to_string()))?;
    let sequence = OPERATOR_JOB_TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = dir.join(format!(
        ".{}.json.{}.{sequence}.tmp",
        job.job_id,
        std::process::id()
    ));
    // Write and sync the complete body first, then publish with a hard link:
    // link creation fails when the target already exists, so first-wins and
    // "the existing job stays second" are one atomic step. A check-then-rename
    // would let two concurrent POSTs overwrite each other's durable job.
    {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        use std::io::Write as _;
        file.write_all(&body)
            .and_then(|()| file.sync_all())
            .map_err(|error| ApiError::Internal(error.to_string()))?;
    }
    let bytes =
        publish_operator_job_file(&temp, &path, &body, |from, to| std::fs::hard_link(from, to))?;
    Ok(job_json_response(bytes))
}

/// Publish one fully-written temp file with `link`: first-wins is the link
/// itself, and a loser answers with the durable target. The linker is
/// injectable so the race arm is a unit test, not a timing hope.
fn publish_operator_job_file(
    temp: &std::path::Path,
    path: &std::path::Path,
    body: &[u8],
    link: impl FnOnce(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> Result<Vec<u8>, ApiError> {
    match link(temp, path) {
        Ok(()) => {
            let _ = std::fs::remove_file(temp);
            let _ = crate::config::sync_parent(path);
            Ok(body.to_vec())
        }
        Err(error) => {
            let _ = std::fs::remove_file(temp);
            // `hard_link` reports AlreadyExists exactly when the target is
            // there, so this is the whole race case: answer with the durable
            // file, never a conflict.
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                let existing = std::fs::read(path)
                    .map_err(|read_error| ApiError::Internal(read_error.to_string()))?;
                return Ok(existing);
            }
            Err(ApiError::Internal(error.to_string()))
        }
    }
}

/// `GET /v1/operator/jobs/{job_id}`: the current job JSON from the shared
/// folder, or `404` when the well-formed id has no file.
async fn operator_jobs_show(
    headers: HeaderMap,
    job_id: Result<Path<String>, PathRejection>,
) -> Result<Response, ApiError> {
    let token = match std::env::var(OPERATOR_JOBS_TOKEN_ENV) {
        Ok(token) => token,
        Err(_) => return Ok(operator_jobs_not_configured()),
    };
    authorize_operator_jobs(&headers, &token)?;
    let Ok(Path(job_id)) = job_id else {
        return Err(ApiError::BadRequest("bad_request"));
    };
    if !is_frozen_job_id(&job_id) {
        return Err(ApiError::BadRequest("bad_request"));
    }
    let path = operator_jobs_dir().join(format!("{job_id}.json"));
    match std::fs::read(&path) {
        Ok(existing) => Ok(job_json_response(existing)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(ApiError::NotFound),
        Err(error) => Err(ApiError::Internal(error.to_string())),
    }
}

async fn ledger(
    State(state): State<AppState>,
    headers: HeaderMap,
    publisher: Result<Path<String>, PathRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Ok(Path(publisher)) = publisher else {
        return Err(ApiError::BadRequest("bad_request"));
    };
    if !is_valid_publisher(&publisher) {
        return Err(ApiError::BadRequest("invalid_publisher"));
    }
    state.authorize_ledger(
        &publisher,
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    )?;
    let records = match state.ledger_source.records(&publisher).await {
        Ok(records) => records,
        Err(ApiError::LedgerUnverified { line }) => {
            state
                .analytics
                .emit_ledger_verification_failed(
                    now_unix_ms(&state),
                    &publisher,
                    0,
                    env!("CARGO_PKG_NAME"),
                    line as u64,
                    0,
                )
                .await;
            return Err(ApiError::LedgerUnverified { line });
        }
        Err(error) => return Err(error),
    };
    state
        .analytics
        .emit_ledger_verified(
            now_unix_ms(&state),
            &publisher,
            records.iter().map(|record| record.seq()).max().unwrap_or(0),
            env!("CARGO_PKG_NAME"),
            0,
        )
        .await;
    Ok(Json(json!({
        "publisher": publisher,
        "mode": READ_ONLY_MODE,
        "records": records,
    })))
}

#[derive(Debug, Deserialize, Default)]
struct CycleParams {
    #[serde(default)]
    platform: Option<String>,
}

async fn run_cycle(
    State(state): State<AppState>,
    headers: HeaderMap,
    publisher: Result<Path<String>, PathRejection>,
    params: Result<Query<CycleParams>, QueryRejection>,
    body: Bytes,
) -> Result<Json<CycleSummary>, ApiError> {
    // The cycle carries the same bearer-token gate as `POST
    // /v1/audit/{platform}`: no configured token source is
    // `503 live_audit_not_configured`, a missing or wrong bearer is `401`.
    // The gate runs before any path/query validation, so an anonymous caller
    // cannot probe publisher or platform handling.
    let Some(auditor) = &state.live_auditor else {
        return Err(ApiError::LiveAuditNotConfigured);
    };
    auditor.authorize(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    )?;
    let Ok(Path(publisher)) = publisher else {
        return Err(ApiError::BadRequest("bad_request"));
    };
    if !is_valid_publisher(&publisher) {
        return Err(ApiError::BadRequest("invalid_publisher"));
    }
    let Ok(Query(params)) = params else {
        return Err(ApiError::BadRequest("bad_request"));
    };
    let platform_name = params
        .platform
        .ok_or(ApiError::BadRequest("platform_required"))?;
    let platform =
        parse_platform(&platform_name).ok_or(ApiError::BadRequest("unknown_platform"))?;
    let request = if body.is_empty() {
        CycleRequest::default()
    } else {
        parse_cycle_request(&body)?
    };
    let summary = state
        .cycle_runner
        .run(&publisher, platform, request)
        .await?;
    Ok(Json(summary))
}

/// Parse the optional cycle body strictly: a JSON object with only the known
/// keys, whose `policy` (if present, non-null) is itself an object with only
/// the known keys. Serde's sequence form for structs (e.g. `[]`) and unknown
/// fields are refused, so a typo cannot silently change what runs.
fn parse_cycle_request(body: &[u8]) -> Result<CycleRequest, ApiError> {
    let invalid = || ApiError::BadRequest("invalid_cycle_request");
    let value: serde_json::Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    let object = value.as_object().ok_or_else(invalid)?;
    for key in object.keys() {
        if key != "policy" {
            return Err(invalid());
        }
    }
    if let Some(policy) = object.get("policy") {
        if !policy.is_null() {
            let policy = policy.as_object().ok_or_else(invalid)?;
            for key in policy.keys() {
                if key != "minLiftRateBp" && key != "maxFeeMicros" {
                    return Err(invalid());
                }
            }
        }
    }
    serde_json::from_value(value).map_err(|_| invalid())
}

#[derive(Debug, serde::Deserialize)]
struct PostbackParams {
    token: String,
}

async fn max_postbacks(Query(params): Query<PostbackParams>, body: Bytes) -> Response {
    let Some(expected) = std::env::var("APEX_POSTBACK_TOKEN")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        return (StatusCode::SERVICE_UNAVAILABLE, "postbacks_not_configured").into_response();
    };
    if params.token != expected {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let path = std::env::var("APEX_POSTBACK_SPOOL")
        .unwrap_or_else(|_| ".apex/postbacks/max.jsonl".to_string());
    match crate::postbacks::append(std::path::Path::new(&path), &body) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            tracing::warn!(%error, "MAX postback refused");
            (StatusCode::BAD_REQUEST, "bad_postback").into_response()
        }
    }
}

async fn mollie_webhook(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(client) = &state.mollie else {
        return Err(ApiError::MollieWebhookNotConfigured);
    };
    let Some(transport) = &state.billing_transport else {
        return Err(ApiError::MollieWebhookNotConfigured);
    };
    let Some(store) = &state.collection else {
        return Err(ApiError::CollectionStoreNotConfigured);
    };
    // Mollie does not sign webhooks: the body only names a payment id, and
    // the payment itself is fetched with the API key. Never trust the POST
    // body for status, amount or metadata.
    let payment = apex_billing::fetch_webhook_payment(client, transport.get(), &body)
        .await
        .map_err(|error| match error {
            apex_billing::BillingError::Malformed(_) => {
                tracing::debug!(%error, "mollie delivery refused");
                ApiError::BadRequest("malformed_event")
            }
            apex_billing::BillingError::Http { status: 404 } => {
                ApiError::BadRequest("unknown_payment")
            }
            apex_billing::BillingError::WebhookRejected { .. } => ApiError::InvalidWebhookPayment,
            other => {
                tracing::warn!(%other, "mollie payment fetch failed");
                ApiError::PlatformUnavailable("mollie payment fetch failed".into())
            }
        })?;
    let reference = payment
        .metadata_reference
        .as_deref()
        .unwrap_or_default()
        .to_string();
    // A3 B3: the watch is free, so no watch payment is accepted. Only an
    // Apex fee invoice has a payable reference; everything else is refused
    // before it is recorded.
    let expected = if reference.starts_with("apex-fee:") {
        None
    } else {
        return Err(ApiError::InvalidWebhookPayment);
    };
    apex_billing::validate_payment(&payment, None, expected).map_err(|error| match error {
        apex_billing::BillingError::WebhookRejected { .. } => {
            tracing::warn!(%error, "mollie payment failed validation");
            ApiError::InvalidWebhookPayment
        }
        other => {
            tracing::debug!(%other, "mollie payment shape refused");
            ApiError::BadRequest("malformed_event")
        }
    })?;
    let now_unix_ms = state.clock.now_unix_s().saturating_mul(1000);
    let event = apex_billing::CollectionEvent::from_payment(&payment, now_unix_ms);
    let new_item = store.record(&event).map_err(|error| {
        tracing::error!(%error, "mollie collection event could not be recorded");
        ApiError::Internal("collection_record_failed".into())
    })?;
    // Apply the fetched provider status to the durable invoice intent. This
    // is idempotent and order-safe (a replayed/reordered delivery never
    // regresses a paid intent); a corrupt journal fails closed with a 500 so
    // the provider retries instead of the webhook acknowledging lost state.
    if let Some(intents) = &state.payment_intents {
        intents
            .reconcile_payment(&payment, now_unix_ms)
            .map_err(|error| {
                tracing::error!(%error, "mollie payment intent could not be updated");
                ApiError::Internal("payment_intent_record_failed".into())
            })?;
    }
    if new_item {
        tracing::info!(
            payment_id = %payment.id,
            status = %payment.status,
            reference = %reference,
            "mollie collection event recorded"
        );
        // A collected results-fee invoice is recorded for the outcome log;
        // it never switches a free watch on. A status transition is recorded
        // once (the collection store key is payment id + status).
        if payment.is_collected() {
            if let Some(publisher) = event.publisher_id() {
                append_outcome(&state, publisher, "paid", &payment.id);
            }
        }
    } else {
        tracing::info!(
            payment_id = %payment.id,
            status = %payment.status,
            "mollie collection event already recorded (replay)"
        );
    }
    if let Some(path) = &state.collection_log_path {
        restrict_state_file(path);
    }
    Ok(Json(json!({
        "received": true,
        "id": payment.id,
        "status": payment.status,
        "new": new_item,
    })))
}

// ---------------------------------------------------------------------------
// Console handlers
// ---------------------------------------------------------------------------

/// The UTC "now" as epoch milliseconds, from the injected clock.
fn now_unix_ms(state: &AppState) -> i64 {
    state.clock.now_unix_s().saturating_mul(1000)
}

/// The origin/host pair of `value`, with the scheme's default port filled in.
fn normalized_authority(value: &str, default_port: u16) -> Option<(String, u16)> {
    let authority: Authority = value.trim().parse().ok()?;
    Some((
        authority.host().to_ascii_lowercase(),
        authority.port_u16().unwrap_or(default_port),
    ))
}

/// Login-CSRF guard for session-minting POSTs.
///
/// A cross-site form can submit to a session-minting endpoint even though the
/// session cookie is `SameSite=Lax`, so the request's own origin must agree
/// with its `Host` header. Absent `Origin` (tests, curl, non-browser clients)
/// is allowed; a present `Origin` must name the same authority, and must be
/// `https` when the request arrived over TLS. A `Sec-Fetch-Site: cross-site`
/// hint is always refused.
fn session_post_origin_allowed(headers: &HeaderMap) -> bool {
    if headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("cross-site"))
    {
        return false;
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let origin = origin.trim();
    if origin.is_empty() || origin.eq_ignore_ascii_case("null") {
        return false;
    }
    let Ok(uri) = origin.parse::<axum::http::Uri>() else {
        return false;
    };
    let Some(scheme) = uri.scheme_str() else {
        return false;
    };
    let default_port = if scheme.eq_ignore_ascii_case("https") {
        443
    } else if scheme.eq_ignore_ascii_case("http") {
        80
    } else {
        return false;
    };
    if session::request_is_secure(headers) && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let Some(origin_authority) = uri.authority() else {
        return false;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    match (
        normalized_authority(origin_authority.as_str(), default_port),
        normalized_authority(host, default_port),
    ) {
        (Some(origin), Some(host)) => origin == host,
        _ => false,
    }
}

/// A quotable support reference: the route and the UTC moment it failed.
fn console_reference(state: &AppState, route: &str) -> String {
    format!(
        "{route} at {}",
        apex_ui::format::utc_datetime(now_unix_ms(state))
    )
}

async fn console_overview(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let view = match require_session(&state, &headers) {
        Ok(view) => view,
        Err(response) => return *response,
    };
    let ctx = ConsoleCtx::from_env();
    let data = collect_overview(&state, &view.publisher).await;
    page(StatusCode::OK, overview::render(&ctx, &data))
}

/// `GET /console`: the bare address the deployment proxy routes to this
/// service and the docs advertise as the change log, redirected to its
/// stable location.
async fn console_bare() -> Response {
    Redirect::to("/console/ledger").into_response()
}

/// Read the real overview inputs for one publisher: the newest of *that
/// publisher's own* snapshots across the three platforms and the verified
/// record total of *that publisher's own* ledger. Source failures never
/// fabricate numbers; they surface as the documented unavailable states. No
/// page ever aggregates across publishers, so one session can never observe
/// another publisher's snapshot or history totals.
async fn collect_overview(state: &AppState, publisher: &str) -> OverviewData {
    // Read every platform concurrently. Each call is one stat/parse and one
    // cached report build; the old loop read a snapshot first and then read
    // the same snapshot again for its report, sequentially, three times.
    let (applovin, levelplay, admob) = tokio::join!(
        state
            .audit_source
            .snapshot_report_arc(publisher, PlatformId::AppLovinMax),
        state
            .audit_source
            .snapshot_report_arc(publisher, PlatformId::LevelPlay),
        state
            .audit_source
            .snapshot_report_arc(publisher, PlatformId::Admob),
    );
    let mut latest: Option<LatestAudit> = None;
    for ((name, label), result) in PLATFORM_NAMES.iter().zip([applovin, levelplay, admob]) {
        match result {
            Ok((snapshot, report)) => {
                if latest.as_ref().is_some_and(|current| {
                    current.captured_at_unix_ms >= snapshot.fetched_at_unix_ms
                }) {
                    continue;
                }
                let mut critical = 0usize;
                let mut warning = 0usize;
                let mut info = 0usize;
                for finding in &report.findings {
                    match finding.severity {
                        apex_audit::Severity::Critical => critical += 1,
                        apex_audit::Severity::Warning => warning += 1,
                        apex_audit::Severity::Info => info += 1,
                    }
                }
                latest = Some(LatestAudit {
                    platform_label: label.to_string(),
                    captured_at_unix_ms: snapshot.fetched_at_unix_ms,
                    findings: report.findings.len(),
                    critical,
                    warning,
                    info,
                });
            }
            Err(ApiError::NotFound) => {}
            Err(error) => {
                tracing::warn!(%error, platform = name, "overview report failed");
            }
        }
    }
    let any_ledger = state.ledger_source.has_publishers();
    let ledger = if !any_ledger {
        LedgerState::NotConfigured
    } else {
        match state.ledger_source.record_count(publisher).await {
            Ok(records) => LedgerState::Verified {
                records,
                unavailable: 0,
            },
            Err(error) => LedgerState::Unavailable {
                reason: overview_reason(&error),
            },
        }
    };
    OverviewData {
        latest_audit: latest,
        ledger,
    }
}

/// A short, human reason for an unreadable ledger in the overview.
fn overview_reason(error: &ApiError) -> String {
    match error {
        ApiError::LedgerUnverified { .. } => "history could not be verified".to_string(),
        ApiError::NotFound => "no history file".to_string(),
        _ => "history read failed".to_string(),
    }
}

#[derive(Debug, Deserialize, Default)]
struct AuditConsoleParams {
    #[serde(default)]
    platform: Option<String>,
}

async fn console_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<AuditConsoleParams>, QueryRejection>,
) -> Response {
    let view = match require_session(&state, &headers) {
        Ok(view) => view,
        Err(response) => return *response,
    };
    let ctx = ConsoleCtx::from_env();
    let Ok(Query(params)) = params else {
        return page(
            StatusCode::BAD_REQUEST,
            audit::render_audit_form(
                &ctx,
                Some(&AuditFormProblem::BadParameters(
                    "The request parameters could not be interpreted.".to_string(),
                )),
            ),
        );
    };
    let Some(platform_name) = params.platform else {
        return page(StatusCode::OK, audit::render_audit_form(&ctx, None));
    };
    let Some(platform) = parse_platform(&platform_name) else {
        return page(
            StatusCode::BAD_REQUEST,
            audit::render_audit_form(
                &ctx,
                Some(&AuditFormProblem::UnknownPlatform(platform_name)),
            ),
        );
    };
    match state
        .audit_source
        .snapshot_report_arc(&view.publisher, platform)
        .await
    {
        Ok((snapshot, report)) => page(
            StatusCode::OK,
            audit::render_audit_report(
                &ctx,
                &AuditReportView {
                    platform,
                    snapshot: &snapshot,
                    report: &report,
                },
            ),
        ),
        Err(ApiError::NotFound) => page(
            StatusCode::NOT_FOUND,
            audit::render_audit_form(&ctx, Some(&AuditFormProblem::NoSnapshot(platform_name))),
        ),
        Err(error) => {
            tracing::error!(%error, "audit console failed");
            let reference = console_reference(&state, "/console/audit");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::audit_read_failure(&ctx, &reference),
            )
        }
    }
}

async fn console_finding(
    State(state): State<AppState>,
    headers: HeaderMap,
    id: Result<Path<String>, PathRejection>,
) -> Response {
    let view = match require_session(&state, &headers) {
        Ok(view) => view,
        Err(response) => return *response,
    };
    let ctx = ConsoleCtx::from_env();
    let Ok(Path(id)) = id else {
        return page(StatusCode::NOT_FOUND, errors::finding_not_found(&ctx));
    };
    if !is_finding_id(&id) {
        return page(StatusCode::NOT_FOUND, errors::finding_not_found(&ctx));
    }
    let mut failed_internal = false;
    for (name, _label) in PLATFORM_NAMES {
        let Some(platform) = parse_platform(name) else {
            continue;
        };
        match state
            .audit_source
            .snapshot_report_arc(&view.publisher, platform)
            .await
        {
            Ok((snapshot, report)) => {
                if let Some(finding) = report.findings.iter().find(|finding| {
                    audit::finding_id(
                        finding.rule.as_str(),
                        finding.platform.as_str(),
                        &finding.subject,
                    ) == id
                }) {
                    return page(
                        StatusCode::OK,
                        audit::render_finding_detail(
                            &ctx,
                            &FindingView {
                                snapshot: &snapshot,
                                finding,
                            },
                        ),
                    );
                }
            }
            Err(ApiError::NotFound) => {}
            Err(error) => {
                failed_internal = true;
                tracing::warn!(%error, platform = name, "finding lookup snapshot read failed");
            }
        }
    }
    if failed_internal {
        let reference = console_reference(&state, "/console/audit/findings");
        page(
            StatusCode::INTERNAL_SERVER_ERROR,
            errors::internal(&ctx, &reference),
        )
    } else {
        page(StatusCode::NOT_FOUND, errors::finding_not_found(&ctx))
    }
}

async fn console_audit_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<AuditConsoleParams>, QueryRejection>,
) -> Response {
    let view = match require_session(&state, &headers) {
        Ok(view) => view,
        Err(response) => return *response,
    };
    let ctx = ConsoleCtx::from_env();
    let Ok(Query(params)) = params else {
        return page(
            StatusCode::BAD_REQUEST,
            errors::bad_request_with_detail(
                &ctx,
                "The request parameters could not be interpreted.",
            ),
        );
    };
    let Some(platform_name) = params.platform else {
        return page(
            StatusCode::BAD_REQUEST,
            errors::bad_request_with_detail(
                &ctx,
                "A platform parameter is required to export a report.",
            ),
        );
    };
    let Some(platform) = parse_platform(&platform_name) else {
        return page(
            StatusCode::BAD_REQUEST,
            errors::bad_request_with_detail(&ctx, "The platform is not one of the supported ones."),
        );
    };
    match state
        .audit_source
        .snapshot_report_arc(&view.publisher, platform)
        .await
    {
        Ok((snapshot, report)) => {
            let csv = audit::render_audit_csv(
                &AuditReportView {
                    platform,
                    snapshot: &snapshot,
                    report: &report,
                },
                &ctx.currency,
            );
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"audit-{}.csv\"", platform.as_str()),
                    ),
                ],
                csv,
            )
                .into_response()
        }
        Err(ApiError::NotFound) => page(StatusCode::NOT_FOUND, errors::snapshot_not_found(&ctx)),
        Err(error) => {
            tracing::error!(%error, "audit csv failed");
            let reference = console_reference(&state, "/console/audit/export.csv");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::audit_read_failure(&ctx, &reference),
            )
        }
    }
}

/// Query parameters of the paginated change log.
#[derive(Debug, Deserialize, Default)]
struct LedgerPageParams {
    /// The 1-based page number; absent means page 1. Kept as a string so the
    /// handler can answer with its own typed 400 page instead of a serde
    /// rejection.
    #[serde(default)]
    page: Option<String>,
}

/// Parse the `page` parameter: absent is page 1, a positive whole number is
/// accepted, and everything else (non-numeric, zero, overflow, duplicate or
/// unreadable query) is rejected so the caller can render the typed 400.
fn requested_ledger_page(
    params: Result<Query<LedgerPageParams>, QueryRejection>,
) -> Result<u64, ()> {
    let Ok(Query(params)) = params else {
        return Err(());
    };
    match params.page.as_deref() {
        None => Ok(1),
        Some(raw) => raw.parse::<u64>().ok().filter(|page| *page >= 1).ok_or(()),
    }
}

/// The session view of a request, if the cookie carries a live session.
fn live_session(
    state: &AppState,
    headers: &HeaderMap,
) -> (Option<String>, Option<crate::session::SessionView>) {
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok());
    let id = session::session_id_from_cookie(cookie).filter(|id| session::is_session_id(id));
    let view = id
        .as_deref()
        .and_then(|id| state.ledger_sessions.touch(id, now_unix_ms(state)));
    (id, view)
}

async fn console_ledger(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<LedgerPageParams>, QueryRejection>,
) -> Response {
    let ctx = ConsoleCtx::from_env();
    let (id, view) = live_session(&state, &headers);
    let Some(view) = view else {
        let problem = id.map(|_| LedgerAccessProblem::SessionExpired);
        return page(
            StatusCode::OK,
            ledger::render_access_panel(&ctx, problem, None),
        );
    };
    // The session gate runs first: an anonymous request never learns whether
    // its page parameter was well formed.
    let Ok(requested_page) = requested_ledger_page(params) else {
        return page(StatusCode::BAD_REQUEST, errors::ledger_page_invalid(&ctx));
    };
    let offset = usize::try_from(
        requested_page
            .saturating_sub(1)
            .saturating_mul(ledger::RECORDS_PER_PAGE as u64),
    )
    .unwrap_or(usize::MAX);
    let publisher = view.publisher.clone();
    match state
        .ledger_source
        .records_page(&publisher, offset, ledger::RECORDS_PER_PAGE)
        .await
    {
        Ok(found) => {
            let last_page = found.total.div_ceil(ledger::RECORDS_PER_PAGE as u64).max(1);
            if requested_page > last_page {
                return page(StatusCode::BAD_REQUEST, errors::ledger_page_invalid(&ctx));
            }
            state
                .analytics
                .emit_ledger_verified(
                    now_unix_ms(&state),
                    &publisher,
                    found.total.saturating_sub(1),
                    env!("CARGO_PKG_NAME"),
                    0,
                )
                .await;
            let html = ledger::render_records(
                &ctx,
                &publisher,
                ledger::RecordsPage {
                    records: &found.records,
                    total: found.total,
                    page: requested_page,
                },
                now_unix_ms(&state),
                &view.csrf,
            );
            let mut response = page(StatusCode::OK, html);
            renew_session_cookie(&mut response, id.as_deref(), &headers);
            response
        }
        Err(ApiError::LedgerUnverified { line }) => {
            state
                .analytics
                .emit_ledger_verification_failed(
                    now_unix_ms(&state),
                    &publisher,
                    0,
                    env!("CARGO_PKG_NAME"),
                    line as u64,
                    0,
                )
                .await;
            page(
                StatusCode::UNPROCESSABLE_ENTITY,
                errors::ledger_unverified(&ctx, line),
            )
        }
        Err(ApiError::NotFound) => page(StatusCode::NOT_FOUND, errors::ledger_not_found(&ctx)),
        Err(error) => {
            tracing::error!(%error, publisher = %publisher, "ledger console failed");
            let reference = console_reference(&state, "/console/ledger");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::internal(&ctx, &reference),
            )
        }
    }
}

/// Re-set the session cookie (rolling idle window) when the session is live.
fn renew_session_cookie(response: &mut Response, id: Option<&str>, headers: &HeaderMap) {
    let Some(id) = id else {
        return;
    };
    let cookie = session::session_cookie(id, session::request_is_secure(headers));
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
}

/// The token exchange: authorize against the configured allow-list, then mint
/// an opaque session. The access token is never stored and never reflected;
/// the cookie carries only the session id.
async fn console_ledger_open(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctx = ConsoleCtx::from_env();
    if !session_post_origin_allowed(&headers) {
        return page(StatusCode::FORBIDDEN, errors::bad_request(&ctx, None));
    }
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => {
            return page(
                StatusCode::BAD_REQUEST,
                ledger::render_access_panel(
                    &ctx,
                    Some(LedgerAccessProblem::InvalidPublisher),
                    None,
                ),
            )
        }
    };
    let publisher = fields
        .get("publisher")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let token = fields
        .get("token")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let preserved = if publisher.is_empty() {
        None
    } else {
        Some(publisher.as_str())
    };
    if !is_valid_publisher(&publisher) {
        return page(
            StatusCode::BAD_REQUEST,
            ledger::render_access_panel(
                &ctx,
                Some(LedgerAccessProblem::InvalidPublisher),
                preserved,
            ),
        );
    }
    let authorization = if token.is_empty() {
        None
    } else {
        Some(format!("Bearer {token}"))
    };
    match state.authorize_ledger(&publisher, authorization.as_deref()) {
        Ok(()) => {
            let opened = state.ledger_sessions.open(&publisher, now_unix_ms(&state));
            let cookie = session::session_cookie(&opened.id, session::request_is_secure(&headers));
            redirect_with_cookie("/console/ledger", Some(cookie))
        }
        Err(ApiError::LedgerAuthNotConfigured) => page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        ),
        Err(_) => page(
            StatusCode::UNAUTHORIZED,
            ledger::render_access_panel(&ctx, Some(LedgerAccessProblem::InvalidAccess), preserved),
        ),
    }
}

/// `303 See Other` with an optional `Set-Cookie`.
fn redirect_with_cookie(location: &str, cookie: Option<String>) -> Response {
    let mut response = Redirect::to(location).into_response();
    if let Some(cookie) = cookie {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response.headers_mut().insert(header::SET_COOKIE, value);
        }
    }
    response
}

async fn console_ledger_record(
    State(state): State<AppState>,
    headers: HeaderMap,
    seq: Result<Path<String>, PathRejection>,
) -> Response {
    let ctx = ConsoleCtx::from_env();
    let (id, view) = live_session(&state, &headers);
    let Some(view) = view else {
        // No session: the access panel on the change-log route is the stable
        // recovery point (GET-safe redirect).
        return Redirect::to("/console/ledger").into_response();
    };
    let Ok(Path(seq)) = seq else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let Ok(seq) = seq.parse::<u64>() else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let publisher = view.publisher.clone();
    match state.ledger_source.records_shared(&publisher).await {
        Ok(records) => {
            let Some(record) = records
                .get(seq as usize)
                .filter(|record| record.seq() == seq)
            else {
                return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
            };
            let html =
                ledger::render_record_detail(&ctx, &publisher, record, records.len(), &view.csrf);
            let mut response = page(StatusCode::OK, html);
            renew_session_cookie(&mut response, id.as_deref(), &headers);
            response
        }
        Err(ApiError::LedgerUnverified { line }) => page(
            StatusCode::UNPROCESSABLE_ENTITY,
            errors::ledger_unverified(&ctx, line),
        ),
        Err(ApiError::NotFound) => page(StatusCode::NOT_FOUND, errors::ledger_not_found(&ctx)),
        Err(error) => {
            tracing::error!(%error, publisher = %publisher, "record console failed");
            let reference = console_reference(&state, "/console/ledger/records");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::internal(&ctx, &reference),
            )
        }
    }
}

/// End the browser session (CSRF-protected) and clear the cookie. An expired
/// or unknown session is already ended, so the redirect still answers `303`.
async fn console_ledger_session_end(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctx = ConsoleCtx::from_env();
    let (id, view) = live_session(&state, &headers);
    let cleared = session::cleared_session_cookie();
    let Some(view) = view else {
        return redirect_with_cookie("/console/ledger", Some(cleared));
    };
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => {
            return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None));
        }
    };
    let csrf = fields.get(leads_csrf_field()).cloned().unwrap_or_default();
    if !crate::live::constant_time_eq(csrf.as_bytes(), view.csrf.as_bytes()) {
        return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None));
    }
    if let Some(id) = id {
        state.ledger_sessions.close(&id);
    }
    redirect_with_cookie("/console/ledger", Some(cleared))
}

/// The CSRF form field name, matching `apex-ui`'s hidden-CSRF component.
fn leads_csrf_field() -> &'static str {
    "csrf_token"
}

#[derive(Debug, Deserialize, Default)]
struct SigninParams {
    #[serde(default)]
    token: Option<String>,
}

/// `GET /console/signin?token=<opaque>`: the self-serve one-time link.
///
/// The GET is deliberately inert: it validates the token's shape and shows a
/// page with a POST button, so a mail scanner that follows the link cannot
/// burn it. The POST consumes the token once, mints the same session the
/// ledger-token exchange mints and answers `303` to the console overview.
///
/// Status precedence, pinned by tests: a missing or malformed token is the
/// `400` refusal page on both methods; a well-formed but unknown, used or
/// expired token is the same refusal page with `200` (the page is the state,
/// exactly like the change-log access panel answering `200` with an
/// expired-session notice), and a deployment without a leads root answers the
/// `503` not-configured panel: the sign-in path fails closed.
async fn console_signin(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<SigninParams>, QueryRejection>,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let Some(signin) = &state.signin else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let token = match params {
        Ok(Query(params)) => params.token,
        Err(_) => None,
    };
    let Some(token) = token.filter(|token| crate::signin::is_signin_token(token)) else {
        return page(StatusCode::BAD_REQUEST, errors::signin_refused(&ctx));
    };
    match signin.is_live(&token, now_unix_ms(&state)) {
        Ok(true) => page(StatusCode::OK, errors::signin_confirm(&ctx, &token)),
        Ok(false) => page(StatusCode::OK, errors::signin_refused(&ctx)),
        Err(error) => {
            // Fail closed: an unreadable sign-in store mints nothing.
            tracing::warn!(%error, "sign-in store unavailable; the link was refused");
            page(
                StatusCode::SERVICE_UNAVAILABLE,
                errors::not_configured(&ctx),
            )
        }
    }
}

/// `POST /console/signin`: consume the one-time link and mint the session.
///
/// The token arrives in the landing page's hidden field (or a direct form
/// POST). Consumption is single-use under the store lock, so a replay is
/// refused without a cookie and without touching the already-used row. The
/// cookie flags are exactly the ledger-token exchange's.
async fn console_signin_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctx = ctx_for_request(&headers);
    if !session_post_origin_allowed(&headers) {
        return page(StatusCode::FORBIDDEN, errors::bad_request(&ctx, None));
    }
    let Some(signin) = &state.signin else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let Ok(fields) = leads::parse_form(&body) else {
        return page(StatusCode::BAD_REQUEST, errors::signin_refused(&ctx));
    };
    let Some(token) = fields
        .get("token")
        .map(|token| token.trim())
        .filter(|token| crate::signin::is_signin_token(token))
    else {
        return page(StatusCode::BAD_REQUEST, errors::signin_refused(&ctx));
    };
    match signin.consume(token, now_unix_ms(&state)) {
        Ok(Some(publisher)) => {
            let opened = state.ledger_sessions.open(&publisher, now_unix_ms(&state));
            let cookie = session::session_cookie(&opened.id, session::request_is_secure(&headers));
            redirect_with_cookie("/", Some(cookie))
        }
        Ok(None) => page(StatusCode::OK, errors::signin_refused(&ctx)),
        Err(error) => {
            // Fail closed: an unreadable sign-in store mints nothing.
            tracing::warn!(%error, "sign-in store unavailable; the link was refused");
            page(
                StatusCode::SERVICE_UNAVAILABLE,
                errors::not_configured(&ctx),
            )
        }
    }
}

// --- phase 2: connections, proposals, experiments, billing, settings -------

/// The console context of a request: deployment facts plus the resolved
/// theme cookie.
fn ctx_for_request(headers: &HeaderMap) -> ConsoleCtx {
    let mut ctx = ConsoleCtx::from_env();
    let theme_cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookie| cookie_value(cookie, apex_ui::theme::THEME_COOKIE));
    ctx.theme = apex_ui::theme::resolve(theme_cookie.as_deref());
    ctx
}

/// One cookie value from a `Cookie` header.
fn cookie_value(cookie_header: &str, name: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}

/// The console workspace gate: every console page that renders or mutates one
/// publisher's workspace resolves that publisher from the live session, never
/// from a deployment-wide default. Precedence (documented and pinned by
/// tests):
///
/// 1. A deployment with no console credentials at all — the boot-time ledger
///    tokens are empty *and* the live workspace file records no publisher —
///    can never mint a session, so the handler keeps its documented `503`
///    not-configured panel instead of redirecting to an access panel that
///    could not authorize anyone. A workspace created by a free-audit
///    submission counts as a credential from the moment it is written.
/// 2. No live session: `303` to `/console/ledger`, the stable access panel
///    every console page recovers through.
/// 3. A live session: its bound publisher scopes the page.
fn workspace_session(
    state: &AppState,
    ctx: &ConsoleCtx,
    headers: &HeaderMap,
) -> Result<(String, session::SessionView), Box<Response>> {
    let no_credentials = state.ledger_tokens.is_empty()
        && state
            .workspaces
            .as_ref()
            .is_none_or(|workspaces| workspaces.is_empty());
    if no_credentials {
        return Err(Box::new(page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(ctx),
        )));
    }
    let (_, view) = live_session(state, headers);
    let Some(view) = view else {
        return Err(Box::new(Redirect::to("/console/ledger").into_response()));
    };
    Ok((view.publisher.clone(), view))
}

/// The console access gate for pages that render deployment/platform facts
/// (overview, audit, findings and the CSV export): a live session is
/// required; without one the stable access panel is the recovery point.
fn require_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<session::SessionView, Box<Response>> {
    let (_, view) = live_session(state, headers);
    match view {
        Some(view) => Ok(view),
        None => Err(Box::new(Redirect::to("/console/ledger").into_response())),
    }
}

/// The per-session CSRF check shared by every console mutation: a missing or
/// wrong token is the HTML `400`, never a state change.
fn csrf_verified(
    ctx: &ConsoleCtx,
    session: &session::SessionView,
    fields: &std::collections::BTreeMap<String, String>,
) -> Result<(), Box<Response>> {
    let csrf = fields.get(leads_csrf_field()).cloned().unwrap_or_default();
    if !crate::live::constant_time_eq(csrf.as_bytes(), session.csrf.as_bytes()) {
        return Err(Box::new(page(
            StatusCode::BAD_REQUEST,
            errors::bad_request(ctx, None),
        )));
    }
    Ok(())
}

/// The explicit confirmation every destructive console action requires: the
/// rendered `confirm=yes` checkbox must be checked, or nothing is changed.
fn confirmation_verified(
    ctx: &ConsoleCtx,
    fields: &std::collections::BTreeMap<String, String>,
) -> Result<(), Box<Response>> {
    if fields
        .get("confirm")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("yes"))
    {
        Ok(())
    } else {
        Err(Box::new(page(
            StatusCode::BAD_REQUEST,
            errors::bad_request(ctx, Some("Confirm the action before it can be applied.")),
        )))
    }
}

/// `GET /console/connections`.
async fn console_connections(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, _session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let meta = match connection_meta(&state, &publisher).await {
        Ok(meta) => meta,
        Err(error) => return connection_store_failure(&state, &ctx, error),
    };
    let mut rows = Vec::new();
    for capability in crate::connections::CAPABILITIES {
        let platform = match parse_platform(capability.wire) {
            Some(platform) => platform,
            None => continue,
        };
        let last_report = state
            .audit_source
            .snapshot_arc(&publisher, platform)
            .await
            .ok()
            .map(|snapshot| snapshot.fetched_at_unix_ms);
        rows.push(crate::html::connections::ConnectionRow {
            wire: capability.wire,
            label: capability.label,
            credential_state: meta.platform(capability.wire).credential_state(),
            write_label: capability.write.label(),
            last_report_unix_ms: last_report,
        });
    }
    page(
        StatusCode::OK,
        crate::html::connections::render_list(
            &ctx,
            &crate::html::connections::ConnectionsListData {
                publisher,
                as_of_unix_ms: now_unix_ms(&state),
                rows,
            },
        ),
    )
}

async fn connection_meta(
    state: &AppState,
    publisher: &str,
) -> Result<crate::connections::ConnectionMeta, ApiError> {
    match &state.connection_store {
        Some(store) => store.load_meta(publisher).await,
        None => Ok(crate::connections::ConnectionMeta::default()),
    }
}

fn connection_store_failure(state: &AppState, ctx: &ConsoleCtx, error: ApiError) -> Response {
    let status = error.status();
    let reference = console_reference(state, "/console/connections");
    if status == StatusCode::BAD_REQUEST {
        page(status, errors::bad_request(ctx, None))
    } else if status == StatusCode::CONFLICT {
        page(status, errors::conflict(ctx))
    } else {
        page(
            StatusCode::INTERNAL_SERVER_ERROR,
            errors::internal(ctx, &reference),
        )
    }
}

/// `GET /console/connections/{platform}`.
async fn console_connection_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    platform_path: Result<Path<String>, PathRejection>,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Ok(Path(platform_name)) = platform_path else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let Some(capability) = crate::connections::CAPABILITIES
        .iter()
        .find(|capability| capability.wire == platform_name)
    else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let Some(platform) = parse_platform(capability.wire) else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    connection_detail_response(&state, &ctx, &publisher, platform, capability, &session).await
}

async fn connection_detail_response(
    state: &AppState,
    ctx: &ConsoleCtx,
    publisher: &str,
    platform: PlatformId,
    capability: &crate::connections::PlatformCapability,
    session: &session::SessionView,
) -> Response {
    let writable = state.connection_store.is_some();
    let meta = match connection_meta(state, publisher).await {
        Ok(meta) => meta,
        Err(error) => return connection_store_failure(state, ctx, error),
    };
    let blob_meta = match &state.connection_store {
        Some(store) => match store.blob_meta(publisher).await {
            Ok(meta) => meta,
            Err(error) => return connection_store_failure(state, ctx, error),
        },
        None => None,
    };
    let running_work = running_experiments(state, publisher, Some(platform)).await;
    let last_report = state
        .audit_source
        .snapshot_arc(publisher, platform)
        .await
        .ok()
        .map(|snapshot| snapshot.fetched_at_unix_ms);
    page(
        StatusCode::OK,
        crate::html::connections::render_detail(
            ctx,
            &crate::html::connections::ConnectionDetailData {
                publisher,
                platform,
                label: capability.label,
                state: meta.platform(capability.wire),
                blob_meta: blob_meta.as_ref(),
                writable,
                csrf: Some(session.csrf.as_str()),
                errors: Vec::new(),
                preserved: Vec::new(),
                running_work,
                last_report_unix_ms: last_report,
            },
        ),
    )
}

/// Running (not-finalized) experiments on the workspace, optionally filtered
/// to one platform. `None` when the change log is not readable here: the
/// disconnect warning then says so instead of claiming no running work.
async fn running_experiments(
    state: &AppState,
    publisher: &str,
    platform: Option<PlatformId>,
) -> Option<usize> {
    let records = match state.ledger_source.records_shared(publisher).await {
        Ok(records) => records,
        // No ledger file at all: nothing has been recorded for this
        // publisher, which is honestly zero running work.
        Err(ApiError::NotFound) => return Some(0),
        // A wrong store, a verification failure or a read error: the
        // warning must not claim the work was checked.
        Err(_) => return None,
    };
    // Reuse the parse-once alert view; this page must not reread the log.
    let alerts = state.alerts_cached().await.ok().unwrap_or_default();
    Some(
        crate::experiments::project(&records, &alerts, now_unix_ms(state))
            .iter()
            .filter(|view| !view.is_finalized())
            .filter(|view| platform.is_none_or(|platform| view.platform == Some(platform)))
            .count(),
    )
}

/// The per-platform credential fields a submission must carry.
fn connection_fields(platform: PlatformId) -> &'static [(&'static str, bool)] {
    match platform {
        PlatformId::AppLovinMax => &[
            ("account_id", true),
            ("api_key", true),
            ("report_key", false),
        ],
        PlatformId::LevelPlay => &[
            ("account_id", true),
            ("api_key", true),
            ("refresh_token", true),
        ],
        PlatformId::Admob => &[("account_id", true), ("api_key", true)],
        // The billing domain is not a connectable platform: it carries no
        // credential fields (the wire name never routes here).
        PlatformId::Billing => &[],
    }
}

/// Field errors plus the safe values to preserve after a refused submission.
type ConnectionFormErrors = (Vec<(String, String)>, Vec<(String, String)>);

/// Parse and validate a submission; returns the credentials or the field
/// errors with the safe values to preserve.
fn parse_connection_submission(
    platform: PlatformId,
    fields: &std::collections::BTreeMap<String, String>,
) -> Result<crate::live::LiveCredentials, ConnectionFormErrors> {
    let mut errors: Vec<(String, String)> = Vec::new();
    let mut preserved: Vec<(String, String)> = Vec::new();
    let mut values = std::collections::HashMap::new();
    for (name, required) in connection_fields(platform) {
        let value = fields
            .get(*name)
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        if value.is_empty() && *required {
            errors.push((
                (*name).to_string(),
                "This field is required to confirm access.".to_string(),
            ));
        }
        // Secrets are preserved only as presence: a value never round-trips
        // into the HTML.
        if !value.is_empty() && *name != "api_key" && *name != "refresh_token" {
            preserved.push(((*name).to_string(), value.clone()));
        }
        values.insert((*name).to_string(), value);
    }
    if !errors.is_empty() {
        return Err((errors, preserved));
    }
    let bases = crate::connections::platform_bases(platform, &|key| crate::config::env_value(key));
    let base_url = match &bases {
        Some((base, _)) => base.clone(),
        None => String::new(),
    };
    Ok(crate::live::LiveCredentials {
        api_key: values.get("api_key").cloned().unwrap_or_default(),
        account_id: values.get("account_id").cloned().unwrap_or_default(),
        base_url,
        report_key: values
            .get("report_key")
            .filter(|value| !value.is_empty())
            .cloned(),
        reporting_base_url: bases.and_then(|(_, report)| report),
        refresh_token: values
            .get("refresh_token")
            .filter(|value| !value.is_empty())
            .cloned(),
    })
}

/// `POST /console/connections/{platform}/submit`.
async fn console_connection_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    platform_path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Response {
    connection_mutation(state, headers, platform_path, body).await
}

/// `POST /console/connections/{platform}/rotate` shares the submission path:
/// rotation is the same upsert with a new bundle.
async fn console_connection_rotate(
    State(state): State<AppState>,
    headers: HeaderMap,
    platform_path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Response {
    connection_mutation(state, headers, platform_path, body).await
}

#[allow(clippy::too_many_arguments)]
async fn connection_mutation(
    state: AppState,
    headers: HeaderMap,
    platform_path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Ok(Path(platform_name)) = platform_path else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let Some(capability) = crate::connections::CAPABILITIES
        .iter()
        .find(|capability| capability.wire == platform_name)
    else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let Some(platform) = parse_platform(capability.wire) else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let Some(store) = state.connection_store.clone() else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    if let Err(response) = csrf_verified(&ctx, &session, &fields) {
        return *response;
    }
    let credentials = match parse_connection_submission(platform, &fields) {
        Ok(credentials) => credentials,
        Err((errors, preserved)) => {
            let meta = store.load_meta(&publisher).await.unwrap_or_default();
            return page(
                StatusCode::BAD_REQUEST,
                crate::html::connections::render_detail(
                    &ctx,
                    &crate::html::connections::ConnectionDetailData {
                        publisher: &publisher,
                        platform,
                        label: capability.label,
                        state: meta.platform(capability.wire),
                        blob_meta: None,
                        writable: true,
                        csrf: Some(session.csrf.as_str()),
                        errors,
                        preserved,
                        running_work: None,
                        last_report_unix_ms: None,
                    },
                ),
            );
        }
    };
    let mut account = match store.load_account(&publisher).await {
        Ok(account) => account.unwrap_or_else(|| crate::live::LiveAccount {
            developer_site: String::new(),
            expected_networks: Vec::new(),
            ad_unit_ids: Vec::new(),
            bidder_accounts: Vec::new(),
            platforms: std::collections::BTreeMap::new(),
        }),
        Err(error) => return connection_store_failure(&state, &ctx, error),
    };
    let mut meta = match store.load_meta(&publisher).await {
        Ok(meta) => meta,
        Err(error) => return connection_store_failure(&state, &ctx, error),
    };
    let previous = meta.platform(capability.wire);
    let must_validate = credentials.base_url.trim().is_empty();
    let reporting_hint = platform != PlatformId::AppLovinMax || credentials.report_key.is_some();
    let mut state_row = crate::connections::PlatformState {
        state: Some(crate::connections::CredentialState::Unvalidated),
        validated_at_unix_ms: None,
        reporting: None,
        credential_fields: connection_fields(platform)
            .iter()
            .filter(|(name, _)| {
                fields
                    .get(*name)
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
            })
            .map(|(name, _)| (*name).to_string())
            .collect(),
        failure_class: None,
    };
    account
        .platforms
        .insert(platform.as_str().to_string(), credentials);
    if let Err(error) = store.save_account(&publisher, &account).await {
        return connection_store_failure(&state, &ctx, error);
    }
    if must_validate {
        // No verified platform host in this deployment: store, but never
        // claim a validation.
        state_row = previous_state_preserved(&previous, state_row);
    } else if state.connection_validator.is_some() {
        apply_validation(
            &state,
            &publisher,
            platform,
            reporting_hint,
            &previous,
            &mut state_row,
        )
        .await;
    } else {
        state_row = previous_state_preserved(&previous, state_row);
    }
    meta.set_platform(capability.wire, state_row.clone());
    if let Err(error) = store.save_meta(&publisher, &meta).await {
        return connection_store_failure(&state, &ctx, error);
    }
    Redirect::to(&format!("/console/connections/{}", capability.wire)).into_response()
}

/// Keep the previously recorded state when this deployment cannot validate
/// (no live read path): a submission never invents a `validated` state.
fn previous_state_preserved(
    previous: &crate::connections::PlatformState,
    mut state_row: crate::connections::PlatformState,
) -> crate::connections::PlatformState {
    match previous.credential_state() {
        crate::connections::CredentialState::Validated
        | crate::connections::CredentialState::Rejected
        | crate::connections::CredentialState::Expired => {
            state_row.state = previous.state;
            state_row.validated_at_unix_ms = previous.validated_at_unix_ms;
            state_row.reporting = previous.reporting;
            state_row.failure_class = previous.failure_class.clone();
        }
        crate::connections::CredentialState::Unvalidated => {}
    }
    state_row
}

/// Run the live read against the freshly stored credentials and record what
/// actually happened. Returns whether the validation failed outright (used by
/// the caller only for logging precision).
async fn apply_validation(
    state: &AppState,
    publisher: &str,
    platform: PlatformId,
    reporting_hint: bool,
    previous: &crate::connections::PlatformState,
    state_row: &mut crate::connections::PlatformState,
) -> bool {
    let Some(validator) = &state.connection_validator else {
        return false;
    };
    let started = std::time::Instant::now();
    match validator.audit_with_snapshot(publisher, platform).await {
        Ok(_) => {
            state_row.state = Some(crate::connections::CredentialState::Validated);
            state_row.validated_at_unix_ms = Some(now_unix_ms(state));
            state_row.reporting = Some(reporting_hint);
            // The frozen sales outcome for a validated connection; best
            // effort, write-once per (kind, ref).
            append_outcome(
                state,
                publisher,
                "connected",
                &format!("{publisher}:{}", platform.as_str()),
            );
            state
                .analytics
                .emit_connection_validated(
                    now_unix_ms(state),
                    publisher,
                    platform.as_str(),
                    "console-connection",
                    true,
                    false,
                    env!("CARGO_PKG_NAME"),
                )
                .await;
            false
        }
        Err(ApiError::BadRequest("invalid_credentials")) => {
            let was_validated =
                previous.credential_state() == crate::connections::CredentialState::Validated;
            state_row.state = Some(if was_validated {
                crate::connections::CredentialState::Expired
            } else {
                crate::connections::CredentialState::Rejected
            });
            state_row.validated_at_unix_ms = Some(now_unix_ms(state));
            state_row.reporting = Some(false);
            state_row.failure_class = Some("auth_refused".to_string());
            if was_validated {
                state
                    .analytics
                    .emit_connection_expired(
                        now_unix_ms(state),
                        publisher,
                        platform.as_str(),
                        "console-connection",
                        false,
                        false,
                        env!("CARGO_PKG_NAME"),
                        "auth_refused",
                    )
                    .await;
            }
            true
        }
        Err(_) => {
            // The platform did not answer: keep the previous state instead of
            // claiming a rejection, and say so on the next page render.
            tracing::warn!(
                elapsed_ms = started.elapsed().as_millis(),
                platform = platform.as_str(),
                "connection validation did not complete"
            );
            true
        }
    }
}

/// `POST /console/connections/{platform}/disconnect`.
async fn console_connection_disconnect(
    State(state): State<AppState>,
    headers: HeaderMap,
    platform_path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Ok(Path(platform_name)) = platform_path else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let Some(platform) = parse_platform(&platform_name) else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let Some(store) = state.connection_store.clone() else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    if let Err(response) = csrf_verified(&ctx, &session, &fields) {
        return *response;
    }
    if let Err(response) = confirmation_verified(&ctx, &fields) {
        return *response;
    }
    match store.disconnect_platform(&publisher, platform).await {
        Ok(removed) => {
            if removed {
                state
                    .analytics
                    .emit_connection_revoked(
                        now_unix_ms(&state),
                        &publisher,
                        platform.as_str(),
                        "console-connection",
                        false,
                        false,
                        env!("CARGO_PKG_NAME"),
                        "customer_revoked",
                    )
                    .await;
            }
            Redirect::to(&format!("/console/connections/{}", platform.as_str())).into_response()
        }
        Err(error) => connection_store_failure(&state, &ctx, error),
    }
}

// --- proposals --------------------------------------------------------------

/// `GET /console/proposals/{id}`.
async fn console_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    id_path: Result<Path<String>, PathRejection>,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(store) = state.proposal_store.clone() else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let Ok(Path(id)) = id_path else {
        return page(
            StatusCode::NOT_FOUND,
            crate::html::proposals::render_not_found(&ctx),
        );
    };
    match store.get(&id).await {
        Ok(Some(proposal)) if proposal.owned_by(&publisher) => {
            let approval = store.approval(&id).await.ok().flatten();
            page(
                StatusCode::OK,
                crate::html::proposals::render_detail(
                    &ctx,
                    &crate::html::proposals::ProposalPageData {
                        publisher: &publisher,
                        id: &id,
                        proposal: &proposal,
                        approval: approval.as_ref(),
                        expired: proposal.is_expired(now_unix_ms(&state)),
                        csrf: Some(session.csrf.as_str()),
                        refusal: None,
                    },
                ),
            )
        }
        Ok(Some(_)) | Ok(None) => page(
            StatusCode::NOT_FOUND,
            crate::html::proposals::render_not_found(&ctx),
        ),
        Err(error) => proposal_store_failure(&state, &ctx, error),
    }
}

fn proposal_store_failure(state: &AppState, ctx: &ConsoleCtx, error: ApiError) -> Response {
    match error {
        ApiError::BadRequest(_) => page(
            StatusCode::NOT_FOUND,
            crate::html::proposals::render_not_found(ctx),
        ),
        other => {
            let reference = console_reference(state, "/console/proposals");
            tracing::error!(%other, "proposal store failed");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::internal(ctx, &reference),
            )
        }
    }
}

/// `GET /console/proposals` renders the proposal index.
async fn console_proposals(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, _session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(store) = state.proposal_store.clone() else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    match store.list(&publisher).await {
        Ok(list) => {
            let now = now_unix_ms(&state);
            let entries: Vec<crate::html::proposals::IndexEntry<'_>> = list
                .entries
                .iter()
                .map(
                    |(id, proposal, approval)| crate::html::proposals::IndexEntry {
                        id,
                        proposal,
                        approval: approval.as_ref(),
                        expired: proposal.is_expired(now),
                    },
                )
                .collect();
            page(
                StatusCode::OK,
                crate::html::proposals::render_index(&ctx, &entries, list.skipped),
            )
        }
        Err(error) => proposal_store_failure(&state, &ctx, error),
    }
}

/// `POST /console/proposals/{id}/decision`.
async fn console_proposal_decision(
    State(state): State<AppState>,
    headers: HeaderMap,
    id_path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Some(store) = state.proposal_store.clone() else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let Ok(Path(id)) = id_path else {
        return page(
            StatusCode::NOT_FOUND,
            crate::html::proposals::render_not_found(&ctx),
        );
    };
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    if let Err(response) = csrf_verified(&ctx, &session, &fields) {
        return *response;
    }
    let decision = match fields.get("decision").map(String::as_str) {
        Some("approve") => crate::proposals::ApprovalDecision::Approved,
        Some("decline") => crate::proposals::ApprovalDecision::Declined,
        _ => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    let proposal = match store.get(&id).await {
        Ok(Some(proposal)) if proposal.owned_by(&publisher) => proposal,
        Ok(Some(_)) | Ok(None) => {
            return page(
                StatusCode::NOT_FOUND,
                crate::html::proposals::render_not_found(&ctx),
            )
        }
        Err(error) => return proposal_store_failure(&state, &ctx, error),
    };
    let now = now_unix_ms(&state);
    let change_hash = proposal.change_hash();
    let expected_key = crate::proposals::idempotency_key(&id, decision, &change_hash);
    let supplied_key = fields
        .get("idempotency_key")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    if supplied_key != expected_key {
        return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None));
    }
    if proposal.is_expired(now) {
        return page(
            StatusCode::OK,
            crate::html::proposals::render_detail(
                &ctx,
                &crate::html::proposals::ProposalPageData {
                    publisher: &publisher,
                    id: &id,
                    proposal: &proposal,
                    approval: None,
                    expired: true,
                    csrf: Some(session.csrf.as_str()),
                    refusal: None,
                },
            ),
        );
    }
    let record = crate::proposals::ApprovalRecord {
        proposal_id: id.clone(),
        decision,
        decided_at_unix_ms: now,
        idempotency_key: expected_key.clone(),
        change_hash: change_hash.clone(),
        snapshot: proposal.snapshot.clone(),
        policy_version: proposal.policy_version.clone(),
        policy_hash: proposal.policy_hash.clone(),
        approval_actor: publisher.clone(),
    };
    match store.record_decision(&id, &record).await {
        Ok(crate::proposals::DecisionOutcome::Recorded) => {
            // The emitted event derives from the winning record read back
            // from disk: the published decision file is the single source of
            // truth, so a request that lost an approve/decline race can never
            // make the event state the loser's intent.
            let durable = match store.approval(&id).await {
                Ok(Some(record)) => record,
                Ok(None) => {
                    return proposal_store_failure(
                        &state,
                        &ctx,
                        ApiError::Internal("approval record missing after publish".to_string()),
                    )
                }
                Err(error) => return proposal_store_failure(&state, &ctx, error),
            };
            match durable.decision {
                crate::proposals::ApprovalDecision::Approved => {
                    state
                        .analytics
                        .emit_proposal_approved(
                            durable.decided_at_unix_ms,
                            &publisher,
                            &id,
                            &durable.snapshot,
                            &durable.change_hash,
                            &durable.policy_version,
                            &durable.approval_actor,
                        )
                        .await;
                }
                crate::proposals::ApprovalDecision::Declined => {
                    state
                        .analytics
                        .emit_proposal_declined(
                            durable.decided_at_unix_ms,
                            &publisher,
                            &id,
                            &durable.snapshot,
                            &durable.change_hash,
                            &durable.policy_version,
                        )
                        .await;
                }
            }
            Redirect::to(&format!("/console/proposals/{id}")).into_response()
        }
        Ok(crate::proposals::DecisionOutcome::AlreadyDecidedSameKey) => {
            // A retry returns the original operation status.
            Redirect::to(&format!("/console/proposals/{id}")).into_response()
        }
        Ok(crate::proposals::DecisionOutcome::AlreadyDecided) => {
            // The losing request reports the winner's recorded decision, so
            // the page and the durable file always agree.
            let winner = store.approval(&id).await.ok().flatten();
            page(
                StatusCode::CONFLICT,
                crate::html::proposals::render_detail(
                    &ctx,
                    &crate::html::proposals::ProposalPageData {
                        publisher: &publisher,
                        id: &id,
                        proposal: &proposal,
                        approval: winner.as_ref(),
                        expired: false,
                        csrf: Some(session.csrf.as_str()),
                        refusal: Some(
                            "This proposal was already decided; the recorded decision stands.",
                        ),
                    },
                ),
            )
        }
        Err(error) => proposal_store_failure(&state, &ctx, error),
    }
}

// --- experiments ------------------------------------------------------------

/// Load the verified chain and the alert log once, then project the views.
///
/// The list, detail and incident pages all need the projection; the detail
/// page also needs the same records for its history, so returning the shared
/// chain here avoids the second full-ledger materialization.
async fn experiment_data(
    state: &AppState,
    publisher: &str,
) -> Result<
    (
        Arc<[apex_proof::LedgerRecord]>,
        Vec<crate::experiments::ExperimentView>,
    ),
    ApiError,
> {
    let records = state.ledger_source.records_shared(publisher).await?;
    let alerts = state.alerts_cached().await?;
    let views = crate::experiments::project(&records, &alerts, now_unix_ms(state));
    Ok((records, views))
}

/// `GET /console/experiments` with the documented filters.
async fn console_experiments(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<ExperimentsParams>, QueryRejection>,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, _session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let filter = match &params {
        Ok(Query(params)) => {
            match crate::html::experiments::ExperimentFilter::parse(params.filter.as_deref()) {
                Some(filter) => filter,
                None => {
                    return page(
                        StatusCode::BAD_REQUEST,
                        errors::bad_request_with_detail(
                            &ctx,
                            "The filter is not one of: all, final, 30d.",
                        ),
                    )
                }
            }
        }
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    match experiment_data(&state, &publisher).await {
        Ok((_records, views)) => page(
            StatusCode::OK,
            crate::html::experiments::render_list(
                &ctx,
                &crate::html::experiments::ExperimentsListData {
                    publisher: &publisher,
                    as_of_unix_ms: now_unix_ms(&state),
                    filter,
                    views,
                },
            ),
        ),
        Err(error) => ledger_failure_page(&state, &ctx, error, "/console/experiments"),
    }
}

#[derive(Debug, Deserialize, Default)]
struct ExperimentsParams {
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    tab: Option<String>,
}

fn ledger_failure_page(
    state: &AppState,
    ctx: &ConsoleCtx,
    error: ApiError,
    route: &str,
) -> Response {
    match error {
        ApiError::NotFound => page(StatusCode::NOT_FOUND, errors::ledger_not_found(ctx)),
        ApiError::LedgerUnverified { line } => page(
            StatusCode::UNPROCESSABLE_ENTITY,
            errors::ledger_unverified(ctx, line),
        ),
        other => {
            tracing::error!(%other, route, "console ledger read failed");
            let reference = console_reference(state, route);
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::internal(ctx, &reference),
            )
        }
    }
}

/// `GET /console/experiments/{id}` with the GET-link tabs.
async fn console_experiment_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    id_path: Result<Path<String>, PathRejection>,
    params: Result<Query<ExperimentsParams>, QueryRejection>,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, _session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Ok(Path(id)) = id_path else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    let tab = match &params {
        Ok(Query(params)) => {
            match crate::html::experiments::ExperimentTab::parse(params.tab.as_deref()) {
                Some(tab) => tab,
                None => {
                    return page(
                        StatusCode::BAD_REQUEST,
                        errors::bad_request_with_detail(
                            &ctx,
                            "The tab is not one of: summary, inputs, guardrails, history.",
                        ),
                    )
                }
            }
        }
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    match experiment_data(&state, &publisher).await {
        Ok((records, views)) => {
            let Some(view) = views.iter().find(|view| view.experiment_id == id) else {
                return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
            };
            let history = crate::experiments::history(&records, &id);
            page(
                StatusCode::OK,
                crate::html::experiments::render_detail(
                    &ctx,
                    &crate::html::experiments::ExperimentDetailData {
                        publisher: &publisher,
                        view,
                        tab,
                        history,
                    },
                ),
            )
        }
        Err(error) => ledger_failure_page(&state, &ctx, error, "/console/experiments"),
    }
}

/// `GET /console/experiments/{id}/incident`.
async fn console_experiment_incident(
    State(state): State<AppState>,
    headers: HeaderMap,
    id_path: Result<Path<String>, PathRejection>,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, _session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Ok(Path(id)) = id_path else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    match experiment_data(&state, &publisher).await {
        Ok((_records, views)) => {
            let Some(view) = views
                .iter()
                .find(|view| view.experiment_id == id && view.incident.is_some())
            else {
                return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
            };
            page(
                StatusCode::OK,
                crate::html::experiments::render_incident(&ctx, &publisher, view),
            )
        }
        Err(error) => ledger_failure_page(&state, &ctx, error, "/console/experiments"),
    }
}

// --- billing ----------------------------------------------------------------

async fn billing_view(
    state: &AppState,
    publisher: &str,
) -> Result<crate::billing_view::BillingView, ApiError> {
    let records = state.ledger_source.records_shared(publisher).await?;
    // The collection log is deployment-wide. A publisher page shows only the
    // fee events whose deterministic merchant reference names this
    // publisher; watch subscriptions are a different surface and never move
    // into a publisher's totals. The cached log is borrowed, and only the
    // matching events are cloned into the view.
    let payments = state
        .collection_log_cached()
        .await?
        .iter()
        .filter(|event| event.publisher_id() == Some(publisher))
        .cloned()
        .collect();
    let currency = ConsoleCtx::from_env().currency;
    Ok(crate::billing_view::build(
        &records,
        payments,
        now_unix_ms(state),
        &currency,
    ))
}

/// `GET /console/billing`.
async fn console_billing(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, _session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    match billing_view(&state, &publisher).await {
        Ok(view) => page(
            StatusCode::OK,
            crate::html::billing::render_billing(
                &ctx,
                &crate::html::billing::BillingPageData {
                    view: &view,
                    as_of_unix_ms: now_unix_ms(&state),
                },
            ),
        ),
        Err(error) => ledger_failure_page(&state, &ctx, error, "/console/billing"),
    }
}

/// `GET /console/billing/invoices/{period}`.
async fn console_invoice_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    period_path: Result<Path<String>, PathRejection>,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, _session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Ok(Path(period)) = period_path else {
        return page(
            StatusCode::NOT_FOUND,
            crate::html::billing::render_period_not_found(&ctx),
        );
    };
    if !crate::billing_view::is_period(&period) {
        return page(
            StatusCode::NOT_FOUND,
            crate::html::billing::render_period_not_found(&ctx),
        );
    }
    match billing_view(&state, &publisher).await {
        Ok(view) => {
            if !view.has_period(&period) {
                return page(
                    StatusCode::NOT_FOUND,
                    crate::html::billing::render_period_not_found(&ctx),
                );
            }
            let vat_rate =
                crate::billing_view::vat_rate_from_env(&|key| crate::config::env_value(key));
            page(
                StatusCode::OK,
                crate::html::billing::render_invoice(
                    &ctx,
                    &crate::html::billing::InvoicePageData {
                        view: &view,
                        period: &period,
                        vat_rate_bp: vat_rate,
                    },
                ),
            )
        }
        Err(error) => ledger_failure_page(&state, &ctx, error, "/console/billing"),
    }
}

// --- settings ---------------------------------------------------------------

/// `GET /console/settings`.
async fn console_settings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let account = crate::settings::account_view(
        &|key| crate::config::env_value(key),
        &publisher,
        &ctx.currency,
    );
    page(
        StatusCode::OK,
        crate::html::settings::render_account(
            &ctx,
            &crate::html::settings::AccountPageData {
                account: &account,
                as_of_unix_ms: now_unix_ms(&state),
                theme: ctx.theme,
                csrf: Some(session.csrf.as_str()),
                theme_error: None,
            },
        ),
    )
}

/// The theme cookie: one year, console-scoped, not readable by scripts (there
/// are none).
fn theme_cookie(value: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{}={value}; Path=/console; Max-Age=31536000; HttpOnly; SameSite=Lax",
        apex_ui::theme::THEME_COOKIE,
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// `POST /console/settings/theme`.
async fn console_settings_theme(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    if let Err(response) = csrf_verified(&ctx, &session, &fields) {
        return *response;
    }
    let requested = fields
        .get("theme")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    if !matches!(requested.as_str(), "auto" | "light" | "dark") {
        let account = crate::settings::account_view(
            &|key| crate::config::env_value(key),
            &publisher,
            &ctx.currency,
        );
        return page(
            StatusCode::BAD_REQUEST,
            crate::html::settings::render_account(
                &ctx,
                &crate::html::settings::AccountPageData {
                    account: &account,
                    as_of_unix_ms: now_unix_ms(&state),
                    theme: ctx.theme,
                    csrf: Some(session.csrf.as_str()),
                    theme_error: Some("Choose System, Light or Dark."),
                },
            ),
        );
    }
    let cookie = theme_cookie(&requested, session::request_is_secure(&headers));
    redirect_with_cookie("/console/settings", Some(cookie))
}

/// `GET /console/settings/access`.
async fn console_settings_access(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let sessions = state
        .ledger_sessions
        .summaries(now_unix_ms(&state))
        .into_iter()
        .filter(|summary| summary.publisher == session.publisher)
        .collect();
    let mut response = page(
        StatusCode::OK,
        crate::html::settings::render_access(
            &ctx,
            &crate::html::settings::AccessPageData {
                publisher: &publisher,
                sessions,
                current_handle: Some(session.handle.as_str()),
                csrf: Some(session.csrf.as_str()),
                as_of_unix_ms: now_unix_ms(&state),
            },
        ),
    );
    if let Some(id) = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookie| cookie_value(cookie, session::SESSION_COOKIE))
        .filter(|id| session::is_session_id(id))
    {
        renew_session_cookie(&mut response, Some(&id), &headers);
    }
    response
}

/// `POST /console/settings/access/revoke`.
async fn console_settings_access_revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (_, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    if let Err(response) = csrf_verified(&ctx, &session, &fields) {
        return *response;
    }
    if let Err(response) = confirmation_verified(&ctx, &fields) {
        return *response;
    }
    let handle = fields.get("handle").cloned().unwrap_or_default();
    if !session::is_session_handle(&handle) {
        return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None));
    }
    // Revoking an already-ended session is the same outcome: the session is
    // not live any more, so the redirect still answers 303.
    state
        .ledger_sessions
        .revoke_for(&session.publisher, &handle);
    Redirect::to("/console/settings/access").into_response()
}

/// `GET /console/settings/policy`.
async fn console_settings_policy(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, _session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    // One policy source: the control plane's effective file first, then the
    // legacy operator file. An invalid configured file is unavailable, never
    // silently replaced by defaults.
    let policy = crate::settings::configured_policy(&|name| crate::config::env_value(name))
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "configured policy is unavailable");
            None
        });
    let approval_required_now = policy
        .as_ref()
        .map(|policy| policy.approval_required(now_unix_ms(&state)));
    let view = match state.ledger_source.records(&publisher).await {
        Ok(records) => crate::settings::policy_view(&records),
        Err(_) => crate::settings::policy_view(&[]),
    };
    page(
        StatusCode::OK,
        crate::html::settings::render_policy(
            &ctx,
            &crate::html::settings::PolicyPageData {
                publisher: &publisher,
                policy: policy.as_ref(),
                approval_required_now,
                view: &view,
                as_of_unix_ms: now_unix_ms(&state),
            },
        ),
    )
}

/// The connections root the console writes: the vault root holds the
/// per-publisher plan state files (`<root>/<publisher>.<slug>.json`) beside
/// the customer billing profile and connection blobs.
fn connection_root(state: &AppState) -> Option<&std::path::Path> {
    state.connection_store.as_ref().map(|store| store.root())
}

/// The current billing period `YYYY-MM` at the read's as-of time.
fn current_period(as_of_unix_ms: i64) -> String {
    apex_analytics::UtcMillis::from_unix_millis(as_of_unix_ms).to_day_string()[..7].to_string()
}

/// Plan and collection files hold per-publisher commercial facts. The billing
/// crate writes them with the process umask, which is commonly 0644; tighten
/// the files this server owns to the same 0600 the other state files use.
fn restrict_state_file(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// `GET /console/settings/monitoring`.
async fn console_settings_monitoring(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let meta = match connection_meta(&state, &publisher).await {
        Ok(meta) => meta,
        Err(error) => return connection_store_failure(&state, &ctx, error),
    };
    let mut snapshots = Vec::new();
    for capability in crate::connections::CAPABILITIES {
        let Some(platform) = parse_platform(capability.wire) else {
            continue;
        };
        if let Ok(snapshot) = state.audit_source.snapshot_arc(&publisher, platform).await {
            snapshots.push((platform, snapshot.fetched_at_unix_ms));
        }
    }
    let rows = crate::settings::monitoring_rows(&snapshots, &meta);
    page(
        StatusCode::OK,
        crate::html::settings::render_monitoring(
            &ctx,
            &crate::html::settings::MonitoringPageData {
                publisher: &publisher,
                rows,
                alert_log_configured: state.alerts_path.is_some(),
                as_of_unix_ms: now_unix_ms(&state),
                csrf: Some(session.csrf.as_str()),
            },
        ),
    )
}

/// `GET /console/settings/billing`: the managed ad-ops plan line with its
/// written scope and cancel action.
async fn console_settings_billing(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let managed = match crate::billing_view::managed_state_view(connection_root(&state), &publisher)
    {
        Ok(managed) => managed,
        Err(error) => return ledger_failure_page(&state, &ctx, error, "/console/settings/billing"),
    };
    page(
        StatusCode::OK,
        crate::html::settings::render_billing_settings(
            &ctx,
            &crate::html::settings::BillingSettingsPageData {
                publisher: &publisher,
                managed: &managed,
                as_of_unix_ms: now_unix_ms(&state),
                csrf: Some(session.csrf.as_str()),
            },
        ),
    )
}

/// `POST /console/settings/billing/managed/cancel`. Records the managed
/// cancellation on the plan state file: the current period bills, the next
/// is zero, and the record is kept.
async fn console_settings_billing_managed_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    if let Err(response) = csrf_verified(&ctx, &session, &fields) {
        return *response;
    }
    if let Err(response) = confirmation_verified(&ctx, &fields) {
        return *response;
    }
    let Some(root) = connection_root(&state) else {
        return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None));
    };
    let path = apex_billing::PlanState::path_for(root, &publisher, apex_billing::PlanId::Managed);
    let mut plan = match apex_billing::PlanState::load(&path) {
        Ok(Some(plan)) => plan,
        Ok(None) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
        Err(error) => {
            return ledger_failure_page(
                &state,
                &ctx,
                ApiError::Internal(format!("plan state: {error}")),
                "/console/settings/billing",
            )
        }
    };
    plan.cancel(&current_period(now_unix_ms(&state)));
    if let Err(error) = plan.save(&path) {
        return ledger_failure_page(
            &state,
            &ctx,
            ApiError::Internal(format!("plan state: {error}")),
            "/console/settings/billing",
        );
    }
    restrict_state_file(&path);
    Redirect::to("/console/settings/billing").into_response()
}

// --- analytics --------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct AnalyticsParams {
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    change: Option<String>,
    #[serde(default)]
    platform: Option<String>,
}

/// The default read window: the 30 complete days ending yesterday.
fn default_window(now_unix_ms: i64) -> (String, String) {
    let yesterday =
        apex_analytics::UtcMillis::from_unix_millis(now_unix_ms.saturating_sub(86_400_000))
            .to_day_string();
    let from =
        apex_analytics::UtcMillis::from_unix_millis(now_unix_ms.saturating_sub(30 * 86_400_000))
            .to_day_string();
    (from, yesterday)
}

/// The `?format=csv` answer when no store can serve the export: a machine
/// client must get a status and a plain-text reason, never an HTML page.
fn csv_unavailable(reason: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!("export unavailable: {reason}\n"),
    )
        .into_response()
}

/// `GET /console/analytics?view=...&format=csv`.
async fn console_analytics(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<AnalyticsParams>, QueryRejection>,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let (publisher, _session) = match workspace_session(&state, &ctx, &headers) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let Ok(Query(params)) = params else {
        return page(
            StatusCode::BAD_REQUEST,
            crate::html::analytics::render_bad_view(
                &ctx,
                "The request parameters could not be interpreted.",
            ),
        );
    };
    let tab = match params.view.as_deref() {
        None | Some("") | Some(crate::html::analytics::VIEW_REVENUE) => {
            crate::html::analytics::VIEW_REVENUE
        }
        Some(crate::html::analytics::VIEW_LIFT) => crate::html::analytics::VIEW_LIFT,
        Some(crate::html::analytics::VIEW_FEES) => crate::html::analytics::VIEW_FEES,
        Some(crate::html::analytics::VIEW_DATA_QUALITY) => {
            crate::html::analytics::VIEW_DATA_QUALITY
        }
        Some(_) => {
            return page(
                StatusCode::BAD_REQUEST,
                crate::html::analytics::render_bad_view(
                    &ctx,
                    "The view is not one of: revenue, lift, fees, data-quality.",
                ),
            )
        }
    };
    if let Some(platform) = params.platform.as_deref().filter(|value| !value.is_empty()) {
        if parse_platform(platform).is_none() {
            return page(
                StatusCode::BAD_REQUEST,
                crate::html::analytics::render_bad_view(
                    &ctx,
                    "The platform filter is not one of the supported platforms.",
                ),
            );
        }
    }
    let csv_requested = matches!(params.format.as_deref(), Some("csv"));
    let (from, to) = default_window(now_unix_ms(&state));
    let mut scope = apex_analytics::queries::QueryScope::new(
        publisher.clone(),
        apex_analytics::UtcMillis::from_unix_millis(now_unix_ms(&state)),
    )
    .with_currency(ctx.currency.clone())
    .with_day_range(from, to);
    if let Some(platform) = params.platform.clone().filter(|value| !value.is_empty()) {
        scope = scope.with_platform(platform.clone());
    }
    let Some(reader) = state.analytics.reader().cloned() else {
        let reason = state
            .analytics
            .disabled_reason()
            .unwrap_or(crate::analytics::NOT_CONFIGURED_REASON);
        if csv_requested {
            return csv_unavailable(reason);
        }
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            crate::html::analytics::render(
                &ctx,
                &crate::html::analytics::AnalyticsPageData::unavailable(
                    &publisher, tab, scope, reason,
                ),
            ),
        );
    };
    let change = params
        .change
        .clone()
        .filter(|value| !value.trim().is_empty());
    let data = match tab {
        crate::html::analytics::VIEW_REVENUE => {
            let daily = reader.revenue_daily(&scope).await;
            let by_platform = reader
                .revenue_breakdown(
                    &scope,
                    apex_analytics::queries::BreakdownDimension::Platform,
                )
                .await;
            let by_format = reader
                .revenue_breakdown(&scope, apex_analytics::queries::BreakdownDimension::Format)
                .await;
            let by_country = reader
                .revenue_breakdown(&scope, apex_analytics::queries::BreakdownDimension::Country)
                .await;
            let quality = reader.data_quality(&scope).await;
            daily.and_then(|daily| {
                let quality = quality.unwrap_or_else(|error| {
                    tracing::warn!(%error, "revenue quality read failed");
                    empty_quality(&publisher)
                });
                match (by_platform, by_format, by_country) {
                    (Ok(by_platform), Ok(by_format), Ok(by_country)) => {
                        Ok(crate::html::analytics::AnalyticsData::Revenue {
                            daily,
                            by_platform,
                            by_format,
                            by_country,
                            quality,
                        })
                    }
                    _ => Err(apex_analytics::store::StoreError::Http(
                        "the revenue breakdown read failed".to_string(),
                    )),
                }
            })
        }
        crate::html::analytics::VIEW_LIFT => {
            let rows = reader.lift_rows(&scope).await;
            let quality = reader.data_quality(&scope).await;
            rows.and_then(|rows| {
                quality.map(|quality| crate::html::analytics::AnalyticsData::Lift { rows, quality })
            })
        }
        crate::html::analytics::VIEW_FEES => {
            let rows = reader.fee_rows(&scope).await;
            let detail = match &change {
                Some(change) => reader.fee_detail(&scope, change).await.ok().flatten(),
                None => None,
            };
            let quality = reader.data_quality(&scope).await;
            rows.and_then(|rows| {
                quality.map(|quality| crate::html::analytics::AnalyticsData::Fees {
                    rows,
                    detail,
                    quality,
                })
            })
        }
        crate::html::analytics::VIEW_DATA_QUALITY => reader
            .data_quality(&scope)
            .await
            .map(|quality| crate::html::analytics::AnalyticsData::Quality { quality }),
        _ => unreachable!("tab is validated above"),
    };
    let data = match data {
        Ok(data) => data,
        Err(error) => {
            tracing::warn!(%error, "analytics read failed");
            if csv_requested {
                return csv_unavailable(crate::analytics::UNREACHABLE_REASON);
            }
            let unavailable = crate::html::analytics::AnalyticsPageData::unavailable(
                &publisher,
                tab,
                scope,
                crate::analytics::UNREACHABLE_REASON,
            );
            return page(
                StatusCode::OK,
                crate::html::analytics::render(&ctx, &unavailable),
            );
        }
    };
    if csv_requested {
        let (body, filename) = match &data {
            crate::html::analytics::AnalyticsData::Revenue { daily, .. } => (
                crate::html::analytics::revenue_csv(daily),
                "revenue.csv".to_string(),
            ),
            crate::html::analytics::AnalyticsData::Lift { rows, .. } => (
                crate::html::analytics::lift_csv(rows),
                "lift.csv".to_string(),
            ),
            crate::html::analytics::AnalyticsData::Fees { rows, .. } => (
                crate::html::analytics::fees_csv(rows),
                "fees.csv".to_string(),
            ),
            crate::html::analytics::AnalyticsData::Quality { quality } => (
                crate::html::analytics::quality_csv(quality),
                "data-quality.csv".to_string(),
            ),
        };
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                ),
            ],
            body,
        )
            .into_response();
    }
    page(
        StatusCode::OK,
        crate::html::analytics::render(
            &ctx,
            &crate::html::analytics::AnalyticsPageData {
                publisher,
                tab,
                scope,
                data: Some(data),
                unavailable_reason: None,
                selected_change: change,
            },
        ),
    )
}

/// A quality badge placeholder for the parts that failed: every count is the
/// honest absence, not a zero.
fn empty_quality(publisher: &str) -> apex_analytics::queries::DataQuality {
    apex_analytics::queries::DataQuality {
        publisher_id: publisher.to_string(),
        as_of: String::new(),
        matched_source_days: 0,
        missing_days: None,
        provisional_days: 0,
        dedup_rejects: None,
        unmapped_rows: 0,
        stale_sources: Vec::new(),
        unavailable: vec!["the breakdown read failed".to_string()],
    }
}

/// `GET /free-audit` is only the recipient of the form POST; the page lives
/// at the canonical trailing-slash address.
async fn free_audit_redirect() -> Redirect {
    Redirect::to("/free-audit/")
}

async fn free_audit_page() -> Html<String> {
    let ctx = ConsoleCtx::from_env();
    Html(leads_html::render_request_page(
        &ctx,
        &RequestPageState::Fresh,
    ))
}

/// `POST /free-audit`: validate the form fields, deduplicate retries to the
/// same request id, store the lead append-only and redirect to the
/// confirmation. Validation failures re-render the request page with the
/// exact field copy and the safe values preserved; storage failures re-render
/// it with the submission-failure copy and the retry action.
async fn free_audit_submit(State(state): State<AppState>, body: Bytes) -> Response {
    let ctx = ConsoleCtx::from_env();
    let request = match leads::parse_free_audit_request(&body) {
        Ok(request) => request,
        Err(invalid) => {
            let lead_id = leads::request_id_for_fields(
                &state.lead_id_salt,
                &invalid.request.email,
                &invalid.request.studio,
                &invalid.request.platforms,
                invalid.request.revenue.as_deref(),
                invalid.request.notes.as_deref(),
            );
            let (reason, reason_code) = match invalid.errors.first() {
                Some(error) => (error.field, "validation_failed"),
                None => ("summary", "validation_failed"),
            };
            state
                .analytics
                .emit_audit_request_rejected(
                    now_unix_ms(&state),
                    &lead_id,
                    "free-audit",
                    reason,
                    reason_code,
                )
                .await;
            return page(
                StatusCode::BAD_REQUEST,
                leads_html::render_request_page(&ctx, &RequestPageState::Invalid(invalid)),
            );
        }
    };
    let Some(store) = &state.leads else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            leads_html::render_request_page(
                &ctx,
                &RequestPageState::SubmissionFailed(Box::new(request)),
            ),
        );
    };
    // The workspace key is the proven email domain, never the free-text
    // studio name: two studios behind one domain share one workspace, and a
    // studio name cannot be claimed by whoever types it first. A genuinely
    // new workspace is filed under the collision-resistant hashed key; the
    // legacy slug key is reused only for a workspace the deployment already
    // holds (a claimed record or an operator-provisioned token), so two
    // colliding domains (`a.b.com` and `a-b.com`) get distinct keys. The
    // studio stays the lead's human label (`display_name`).
    let publisher =
        crate::signin::workspace_key_for_claim(state.workspaces.as_deref(), &request.email)
            .unwrap_or_else(|| leads::publisher_slug(&request.studio));
    let lead = Lead {
        publisher: publisher.clone(),
        email: request.email.trim().to_string(),
        platforms: request.platforms.clone(),
        monthly_revenue_range: request.revenue.clone(),
        notes: request.notes.clone(),
        display_name: Some(request.studio.trim().to_string()),
        request_id: None,
    };
    match store.store(&lead, now_unix_ms(&state)).await {
        Ok(request_id) => {
            // The saved lead is the contract: the self-serve workspace and
            // its one-time link are best effort and never gate acceptance.
            // An existing workspace only accepts its recorded contacts, and
            // a new one needs the creating email domain's app-ads.txt proof;
            // a refusal is recorded and no link is issued.
            match record_workspace(&state, &publisher, &lead.email).await {
                Ok(()) => issue_signin_link(&state, &publisher, lead.email.trim()).await,
                Err(refusal) => {
                    record_workspace_refusal(&state, &publisher, &refusal);
                }
            }
            let (from_day, to_day) = default_audit_window(now_unix_ms(&state));
            state
                .analytics
                .emit_audit_request_accepted(
                    now_unix_ms(&state),
                    &request_id,
                    request.platforms.clone(),
                    &from_day,
                    &to_day,
                    "free-audit",
                )
                .await;
            Redirect::to(&format!("/audit-requested/?ref={request_id}")).into_response()
        }
        Err(error) => {
            tracing::error!(%error, "free-audit lead could not be stored");
            let status = if matches!(error, ApiError::LeadCaptureNotConfigured) {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            page(
                status,
                leads_html::render_request_page(
                    &ctx,
                    &RequestPageState::SubmissionFailed(Box::new(request)),
                ),
            )
        }
    }
}

/// Record (or confirm) the submission's workspace before anything is stored.
///
/// - Existing workspace: only a recorded contact is accepted. The contact
///   list is never extended by a submission.
/// - New workspace: the creating email's domain must prove ownership through
///   its public `app-ads.txt` (`OWNERDOMAIN=` or `CONTACT=`) before the
///   workspace and its owner are recorded.
/// - No token file configured: the lead flow keeps working as before, but no
///   workspace or self-serve link can be recorded (logged, not fabricated).
async fn record_workspace(
    state: &AppState,
    publisher: &str,
    email: &str,
) -> Result<(), crate::signin::WorkspaceRefusal> {
    let Some(workspaces) = &state.workspaces else {
        tracing::warn!(
            publisher,
            "workspace not recorded: APEX_LEDGER_TOKENS_FILE is not configured"
        );
        return Ok(());
    };
    if workspaces.has_record(publisher) {
        // Claimed workspace: `ensure` refuses any address that is not
        // already recorded and never adds a contact. A token without
        // recorded ownership is an operator-provisioned entry that still
        // has to prove its domain before it is claimed.
        return workspaces.ensure(publisher, email, false).map(|_| ());
    }
    let domain = email
        .split_once('@')
        .map(|(_, domain)| domain.to_ascii_lowercase())
        .unwrap_or_default();
    // A provisioned workspace's domain is authoritative. A colliding slug
    // from another domain is refused exactly like an unrecorded contact.
    if let Some(bound) = state.workspace_domains.get(publisher) {
        if !bound.eq_ignore_ascii_case(&domain) {
            return Err(crate::signin::WorkspaceRefusal::UnknownContact {
                publisher: publisher.to_string(),
            });
        }
    } else if workspaces.token(publisher).is_some() && publisher_has_data(state, publisher).await {
        // The key already holds runner data but no operator declared which
        // domain owns it: self-serve cannot be allowed to guess.
        return Err(
            crate::signin::WorkspaceRefusal::OperatorProvisioningRequired {
                publisher: publisher.to_string(),
            },
        );
    }
    let Some(proof) = &state.app_ads_proof else {
        return Err(crate::signin::WorkspaceRefusal::UnprovenDomain {
            domain: email
                .split_once('@')
                .map(|(_, domain)| domain.to_string())
                .unwrap_or_default(),
            reason: "no app-ads.txt proof source is configured".to_string(),
        });
    };
    proof.prove_owner(email).await?;
    workspaces.ensure(publisher, email, true).map(|_| ())
}

/// Whether `publisher` already has runner-produced data. An unreadable
/// ledger counts as data (fail closed): claiming it could expose it.
async fn publisher_has_data(state: &AppState, publisher: &str) -> bool {
    if state.audit_source.has_snapshot(publisher) {
        return true;
    }
    match state.ledger_source.record_count(publisher).await {
        Ok(count) => count > 0,
        // No ledger is no data; a read/verification failure is ambiguous
        // and counts as data so the claim fails closed.
        Err(ApiError::NotFound) => false,
        Err(_) => true,
    }
}

/// Record the refusal reason on the operator channel (and the sign-in warning
/// log when configured). The message names the publisher and the rule but
/// never echoes the submitted address.
fn record_workspace_refusal(
    state: &AppState,
    publisher: &str,
    refusal: &crate::signin::WorkspaceRefusal,
) {
    let message = format!(
        "workspace for '{publisher}' refused: {} ({})",
        refusal.reason(),
        refusal
    );
    // `record_warning` already emits the line through `tracing::warn!`; only
    // log here when no sign-in service is configured to carry it, so one
    // refusal produces exactly one operator log line.
    match &state.signin {
        Some(signin) => signin.record_warning(message),
        None => tracing::warn!("{message}"),
    }
}

/// Issue the one-time sign-in link for an accepted audit request. Delivery is
/// best effort: the service records a warning when no email sink is
/// configured (or the send fails), and the confirmation — the audit success
/// contract — is unaffected.
async fn issue_signin_link(state: &AppState, publisher: &str, email: &str) {
    let Some(signin) = &state.signin else {
        tracing::warn!(
            publisher,
            "sign-in link not issued: the leads root is not configured"
        );
        return;
    };
    if let Err(error) = signin
        .issue_link(publisher, email, now_unix_ms(state))
        .await
    {
        tracing::warn!(%error, publisher, "sign-in link could not be created");
    }
}

#[derive(Debug, Deserialize, Default)]
struct RequestedParams {
    #[serde(default, rename = "ref")]
    reference: Option<String>,
}

async fn audit_requested(params: Result<Query<RequestedParams>, QueryRejection>) -> Html<String> {
    let ctx = ConsoleCtx::from_env();
    let reference = match params {
        Ok(Query(params)) => params.reference,
        Err(_) => None,
    };
    let reference = reference.filter(|value| is_request_id(value));
    Html(leads_html::render_confirmation(&ctx, reference.as_deref()))
}

/// `req_` plus 10 hex characters, the stable request-id shape.
fn is_request_id(value: &str) -> bool {
    value.strip_prefix("req_").is_some_and(|hex| {
        (hex.len() == 10 || hex.len() == 32) && hex.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

// --- setup watch ------------------------------------------------------------

/// `GET /watch` is only the recipient of the form POST; the page lives at
/// the canonical trailing-slash address.
#[derive(Debug, serde::Deserialize)]
struct UnsubscribeParams {
    email: String,
    token: String,
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + message.len());
    inner.extend(block.iter().map(|byte| byte ^ 0x36));
    inner.extend_from_slice(message);
    let inner = Sha256::digest(&inner);
    let mut outer = Vec::with_capacity(64 + 32);
    outer.extend(block.iter().map(|byte| byte ^ 0x5c));
    outer.extend_from_slice(&inner);
    Sha256::digest(&outer).into()
}

pub fn unsubscribe_token(email: &str) -> String {
    let salt = std::env::var("APEX_SALES_TOKEN_SALT").unwrap_or_default();
    if salt.is_empty() {
        // No secret, no forgeable token.
        return String::new();
    }
    let digest = hmac_sha256(salt.as_bytes(), email.to_ascii_lowercase().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn token_matches(expected: &str, supplied: &str) -> bool {
    if expected.is_empty() || expected.len() != supplied.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in expected.bytes().zip(supplied.bytes()) {
        difference |= left ^ right;
    }
    difference == 0
}

fn unsubscribe_email_valid(email: &str) -> bool {
    email.len() <= 254
        && email.matches('@').count() == 1
        && !email.chars().any(|c| c.is_control() || c.is_whitespace())
        && email.contains('.')
}

fn unsubscribe_page_html(state: &AppState, email: &str, token: &str, done: bool) -> String {
    let body = if done {
        "<main class=\"wrap page-body\"><section class=\"panel\"><h1>You are unsubscribed.</h1>\
         <p>Apex will not send you further sales email. Transactional mail for an active service is unchanged.</p></section></main>".to_string()
    } else {
        format!(
            "<main class=\"wrap page-body\"><section class=\"panel\"><h1>Unsubscribe</h1>\
             <p>Stop Apex sales email to <strong>{}</strong>.</p>\
             <form method=\"post\" action=\"/unsubscribe\">\
             <input type=\"hidden\" name=\"email\" value=\"{}\">\
             <input type=\"hidden\" name=\"token\" value=\"{}\">\
             <button class=\"button\" type=\"submit\">Unsubscribe</button></form></section></main>",
            apex_ui::escape::html_escape(email),
            apex_ui::escape::html_escape(email),
            apex_ui::escape::html_escape(token),
        )
    };
    let ctx = apex_ui::shell::PageCtx {
        title: "Unsubscribe".to_string(),
        area: apex_ui::shell::Area::Marketing,
        publisher_label: None,
        breadcrumb: Vec::new(),
        read_only: true,
        theme: apex_ui::theme::Theme::Light,
        help_href: Some("/support/".to_string()),
        marketing_header: true,
    };
    let _ = state;
    apex_ui::shell::page_with_css(&ctx, &body, "/assets/apex.css")
}

async fn unsubscribe_page(
    State(state): State<AppState>,
    Query(params): Query<UnsubscribeParams>,
) -> Response {
    if !unsubscribe_email_valid(&params.email) || params.token != unsubscribe_token(&params.email) {
        return page(
            StatusCode::BAD_REQUEST,
            "<main class=\"wrap\"><h1>Invalid unsubscribe link.</h1></main>".to_string(),
        );
    }
    page(
        StatusCode::OK,
        unsubscribe_page_html(&state, &params.email, &params.token, false),
    )
}

async fn unsubscribe_submit(
    State(state): State<AppState>,
    uri: axum::http::Uri,
    body: Bytes,
) -> Response {
    let mut fields = match crate::leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => std::collections::BTreeMap::new(),
    };
    // RFC 8058 one-click clients POST to the List-Unsubscribe URL and carry
    // the email/token in its query string, not in a form body.
    if let Some(query) = uri.query() {
        if let Ok(params) = crate::leads::parse_form(query.as_bytes()) {
            for (key, value) in params {
                fields.entry(key).or_insert(value);
            }
        }
    }
    let email = fields.get("email").cloned().unwrap_or_default();
    let token = fields.get("token").cloned().unwrap_or_default();
    if !unsubscribe_email_valid(&email) || !token_matches(&unsubscribe_token(&email), &token) {
        return page(
            StatusCode::BAD_REQUEST,
            "<main class=\"wrap\"><h1>Invalid unsubscribe link.</h1></main>".to_string(),
        );
    }
    let Some(appender) = &state.outcomes else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            "<main class=\"wrap\"><h1>Unsubscribe is temporarily unavailable.</h1></main>"
                .to_string(),
        );
    };
    if let Err(error) = appender.append(now_unix_ms(&state), &email, "unsubscribe", &token) {
        tracing::warn!(%error, "unsubscribe append failed");
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            "<main class=\"wrap\"><h1>Unsubscribe is temporarily unavailable.</h1></main>"
                .to_string(),
        );
    }
    page(
        StatusCode::OK,
        unsubscribe_page_html(&state, &email, &token, true),
    )
}

async fn watch_redirect() -> Redirect {
    Redirect::to("/watch/")
}

async fn watch_page() -> Html<String> {
    let ctx = ConsoleCtx::from_env();
    Html(watch_html::render_request_page(
        &ctx,
        &WatchPageState::Fresh,
    ))
}

/// The App Store lookup over the deployment's billing transport. The
/// transport is already the configuration seam for the Mollie checkout call, so
/// the same client (and the same test scripts) answers the public iTunes
/// lookup. Without a transport nothing is verified and the standard tier is
/// the honest answer.
struct TransportAppLookup<'a> {
    transport: &'a dyn Transport,
}

impl std::fmt::Debug for TransportAppLookup<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransportAppLookup")
            .finish_non_exhaustive()
    }
}

impl TransportAppLookup<'_> {
    async fn fetch(&self, url: &str, operation: &str) -> Result<(u16, Vec<u8>), String> {
        let request = HttpRequest {
            platform: PlatformId::Billing,
            operation: operation.to_string(),
            method: HttpMethod::Get,
            url: url.to_string(),
            headers: vec![("Accept".to_string(), "*/*".to_string())],
            body: None,
            timeout_ms: 12_000,
        };
        match self.transport.request(request).await {
            Ok(response) => Ok((response.status, response.body)),
            Err(TransportError::Timeout(detail)) => Err(format!("the lookup timed out: {detail}")),
            Err(TransportError::Network(detail)) => {
                Err(format!("the lookup transport failed: {detail}"))
            }
            Err(TransportError::Platform(error)) => {
                Err(format!("the lookup transport failed: {error}"))
            }
        }
    }
}

#[async_trait::async_trait]
impl apex_watch::AppLookup for TransportAppLookup<'_> {
    async fn lookup(&self, app: &str) -> apex_watch::AppLookupResult {
        let apple = match self.fetch(&apex_watch::lookup_url(app), "lookupApp").await {
            Ok((status, body)) => apex_watch::classify_lookup(status, &body),
            Err(reason) => apex_watch::AppLookupResult::Unavailable(reason),
        };
        if apple == apex_watch::AppLookupResult::Verified {
            return apple;
        }
        let Some(package) = apex_watch::google_play_package(app) else {
            return apple;
        };
        let play = match self
            .fetch(
                &apex_watch::google_play_details_url(&package),
                "lookupPlayListing",
            )
            .await
        {
            Ok((200, body))
                if apex_watch::google_play_listing_exists(&String::from_utf8_lossy(&body)) =>
            {
                apex_watch::AppLookupResult::Verified
            }
            Ok((404 | 410, _)) => apex_watch::AppLookupResult::NotFound,
            Ok((200, _)) => apex_watch::AppLookupResult::Unavailable(
                "the Play listing answered 200 without a listing title (consent or bot shell)"
                    .to_string(),
            ),
            Ok((status, _)) => apex_watch::AppLookupResult::Unavailable(format!(
                "the Play listing answered HTTP {status}"
            )),
            Err(reason) => apex_watch::AppLookupResult::Unavailable(reason),
        };
        apex_watch::merge_store_answers(apple, Some(play))
    }

    async fn portfolio(&self, app: &str) -> apex_watch::Portfolio {
        let mut portfolio = apex_watch::Portfolio::default();
        if let Ok((200, body)) = self
            .fetch(&apex_watch::lookup_url(app), "lookupAppCatalogue")
            .await
        {
            if let Some(artist) = apex_watch::app_store_artist_id(&body) {
                if let Ok((200, catalogue)) = self
                    .fetch(
                        &apex_watch::app_store_catalogue_url(artist),
                        "lookupAppStoreCatalogue",
                    )
                    .await
                {
                    portfolio.app_store =
                        apex_watch::app_store_catalogue_count(&catalogue).map(|count| {
                            apex_watch::StoreCatalogue {
                                store: "app_store",
                                developer: artist.to_string(),
                                count,
                            }
                        });
                }
            }
        }
        if let Some(package) = apex_watch::google_play_package(app) {
            if let Ok((200, details)) = self
                .fetch(
                    &apex_watch::google_play_details_url(&package),
                    "lookupPlayDetails",
                )
                .await
            {
                let details = String::from_utf8_lossy(&details);
                if !apex_watch::google_play_listing_exists(&details) {
                    portfolio.error =
                        Some("the Play listing answered 200 without a listing title".to_string());
                } else {
                    let developer = apex_watch::google_play_developer_id(&details);
                    let count = match developer.as_deref() {
                        Some(developer) => {
                            match self
                                .fetch(
                                    &apex_watch::google_play_developer_url(developer),
                                    "lookupPlayCatalogue",
                                )
                                .await
                            {
                                Ok((200, body)) => Some(apex_watch::google_play_catalogue_count(
                                    &String::from_utf8_lossy(&body),
                                )),
                                Ok((status, _)) => {
                                    portfolio.error = Some(format!(
                                        "the Play developer catalogue answered HTTP {status}"
                                    ));
                                    None
                                }
                                Err(reason) => {
                                    portfolio.error = Some(reason);
                                    None
                                }
                            }
                        }
                        None => Some(1),
                    };
                    if let Some(mut count) = count {
                        if count < apex_watch::model::CORE_TIER_MIN_APP_COUNT {
                            if let Some(name) = apex_watch::google_play_developer_name(&details) {
                                if let Ok((200, body)) = self
                                    .fetch(
                                        &apex_watch::google_play_search_url(&name),
                                        "lookupPlaySearch",
                                    )
                                    .await
                                {
                                    count = count.max(apex_watch::google_play_catalogue_count(
                                        &String::from_utf8_lossy(&body),
                                    ));
                                }
                            }
                        }
                        portfolio.google_play = Some(apex_watch::StoreCatalogue {
                            store: "google_play",
                            developer: developer.unwrap_or_else(|| package.clone()),
                            count: count.max(1),
                        });
                    }
                }
            }
        }
        if portfolio.app_store.is_none() && portfolio.google_play.is_none() {
            portfolio.error = Some("the app resolved in neither store".to_string());
        }
        portfolio
    }
}

/// Verify the submitted apps with the configured transport. A deployment
/// without one records the honest "not verified" state instead of guessing a
/// tier.
async fn verify_watch_apps(state: &AppState, apps: &[String]) -> apex_watch::AppVerification {
    if apps.is_empty() {
        return apex_watch::AppVerification::empty();
    }
    match &state.lookup_transport {
        Some(transport) => {
            match tokio::time::timeout(
                std::time::Duration::from_secs(60),
                apex_watch::verify_apps(
                    apps,
                    &TransportAppLookup {
                        transport: transport.get(),
                    },
                ),
            )
            .await
            {
                Ok(verification) => verification,
                Err(_) => apex_watch::AppVerification::unavailable(
                    "the app lookup exceeded its 60 second deadline",
                ),
            }
        }
        None => apex_watch::AppVerification::unavailable(
            "not verified: no app-store lookup transport is configured",
        ),
    }
}

/// Parse the optional apps textarea: one app-store id or URL per line.
///
/// Blank lines are ignored, duplicate values collapse to their first
/// occurrence, and more than [`watch_html::MAX_WATCH_APPS`] distinct apps or
/// any malformed line is the typed field error. Duplicates use the exact
/// [`apex_watch::app_identity`] rule of [`apex_watch::distinct_app_count`]
/// (Unicode lowercase after trimming), so the fifty-line bound and the tier
/// count cannot disagree about whether two lines name one app — including
/// the Kelvin sign, which folds to `k`. The tier is the number of distinct
/// apps the subscriber named, never a form field.
fn parse_watch_apps(text: &str) -> Result<Vec<String>, &'static str> {
    let mut seen = std::collections::BTreeSet::new();
    let mut apps = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !is_watch_app_line(line) {
            return Err(watch_html::WATCH_APPS_ERROR);
        }
        if seen.insert(apex_watch::app_identity(line)) {
            apps.push(line.to_string());
            if apps.len() > watch_html::MAX_WATCH_APPS {
                return Err(watch_html::WATCH_APPS_ERROR);
            }
        }
    }
    Ok(apps)
}

/// Whether one trimmed line names an app-store id or an http(s) URL. Values
/// are compared through the Unicode identity rule and stored as submitted;
/// the check only refuses whitespace/control characters, overlong lines,
/// empty values and URLs without a host. Unicode letters are accepted in an
/// id (the Kelvin sign included) so the parser can apply the same fold as
/// [`apex_watch::distinct_app_count`].
fn is_watch_app_line(line: &str) -> bool {
    if line.chars().count() > 2_048
        || line
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return false;
    }
    if let Some((scheme, rest)) = line.split_once("://") {
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return false;
        }
        let host = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .trim();
        return !host.is_empty() && host.contains('.');
    }
    !line.is_empty()
        && line.chars().all(|character| {
            character.is_alphanumeric() || "._~:/?#[]@!$&'()*+,;=%-".contains(character)
        })
}

/// `POST /watch`: validate the form fields, create a pending subscriber and
/// the free watch, email the one-time email-confirmation link and
/// redirect to the confirmation. Nothing is switched on here: the watch has
/// no active checks until the confirmation link activates the pending row.
///
/// The tier is the number of distinct apps the subscriber names (the store's
/// `tier_from_apps` rule), never a form checkbox and never the account ids in
/// app-ads.txt. Validation failures re-render the request page with the exact
/// field copy and the safe values preserved; storage failures re-render it
/// with the submission-failure copy and the retry action.
async fn watch_submit(State(state): State<AppState>, body: Bytes) -> Response {
    let ctx = ConsoleCtx::from_env();
    let Ok(fields) = leads::parse_form(&body) else {
        return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None));
    };
    let values = WatchFormValues {
        domain: fields.get("domain").cloned().unwrap_or_default(),
        email: fields.get("email").cloned().unwrap_or_default(),
        accounts: fields.get("accounts").cloned().unwrap_or_default(),
        apps: fields.get("apps").cloned().unwrap_or_default(),
    };
    let mut errors =
        apex_watch::validate_watch_signup(&values.domain, &values.email, &values.accounts, false)
            .err()
            .unwrap_or_default();
    let apps = match parse_watch_apps(&values.apps) {
        Ok(apps) => apps,
        Err(message) => {
            errors.push(apex_watch::WatchFieldError {
                field: "apps",
                message,
            });
            Vec::new()
        }
    };
    if !errors.is_empty() {
        return page(
            StatusCode::BAD_REQUEST,
            watch_html::render_request_page(&ctx, &WatchPageState::Invalid { values, errors }),
        );
    }
    // Refuse before any work when the store that would receive the
    // subscriber is not configured: a form that can never be stored must not
    // be accepted first.
    let Some(store) = &state.watch else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            watch_html::render_request_page(
                &ctx,
                &WatchPageState::SubmissionFailed(Box::new(values)),
            ),
        );
    };
    // A repeat inside the per-recipient cooldown does not need another
    // twenty store lookups; the first email's link is still live.
    if let Some(reference) = store
        .recent_pending(&values.domain, &values.email, now_unix_ms(&state))
        .ok()
        .flatten()
    {
        return Redirect::to(&format!("/watch/requested?ref={reference}")).into_response();
    }
    // The tier is decided by the developer catalogue behind a confirmed App
    // Store or Play app, never by the submitted line count, the ad-system
    // account ids in app-ads.txt, or a form field. A line the lookup cannot
    // confirm never buys the core price; a catalogue read failure falls back
    // to the confirmed named apps.
    let verification = verify_watch_apps(&state, &apps).await;
    let now = now_unix_ms(&state);
    let request = apex_watch::model::SubscribeRequest {
        domain: values.domain.clone(),
        email: values.email.clone(),
        accounts: values.accounts.clone(),
        apps,
        verified_app_count: Some(verification.tier_count()),
        verification_error: verification.error.clone(),
    };
    match store.subscribe_pending(now, &request) {
        Ok(outcome) => {
            // The sign-up records the pending subscriber and sends only the
            // confirmation email. There is no checkout link; the
            // billing-profile POST records the customer profile and activates
            // the free watch.
            //
            // The subscriber row derives its publisher slug from the same
            // domain, so the outcome line does not need to clone the whole
            // subscriber log just to look the row up again.
            let publisher = watch_publisher_for_domain(&values.domain);
            append_outcome(&state, &publisher, "watch_signup", &outcome.reference);
            // Mint the cancel capability now so a pending sign-up can be
            // stopped before confirmation; only the hash is stored.
            let (cancel_raw, cancel_hash) = apex_watch::SubscriberStore::mint_cancel_token();
            let cancel_token = match store.get(&outcome.reference) {
                Ok(Some(mut subscriber)) => {
                    subscriber.cancel_token_hash = cancel_hash;
                    if store.update(&subscriber).is_ok() {
                        Some(cancel_raw)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if outcome.send_email {
                send_watch_confirmation(&state, &values.email, &outcome, cancel_token.as_deref())
                    .await;
            }
            Redirect::to(&format!("/watch/requested?ref={}", outcome.reference)).into_response()
        }
        Err(error) => {
            tracing::error!(%error, "watch sign-up could not be stored");
            let status = if matches!(error, apex_watch::WatchError::Validation(_)) {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            let page_state = match &error {
                apex_watch::WatchError::Validation(errors) if !errors.is_empty() => {
                    WatchPageState::Invalid {
                        values,
                        errors: errors.clone(),
                    }
                }
                _ => WatchPageState::SubmissionFailed(Box::new(values)),
            };
            page(status, watch_html::render_request_page(&ctx, &page_state))
        }
    }
}

/// One recorded subscriber row by reference.
///
/// The store's id index answers this in one probe. The previous `list()`
/// implementation cloned every subscriber row and scanned the vector, which
/// made every confirmation and recorded-page render O(subscribers).
fn find_subscriber(
    store: &apex_watch::SubscriberStore,
    reference: &str,
) -> Option<apex_watch::model::Subscriber> {
    store.get(reference).ok().flatten()
}

/// The publisher slug the subscriber store derives for `domain` (its one
/// normalization rule). The sign-up path uses this instead of re-reading the
/// subscriber log it just appended to.
fn watch_publisher_for_domain(domain: &str) -> String {
    apex_watch::normalize_domain(domain)
        .map(|normalized| apex_watch::model::publisher_slug_for_domain(&normalized))
        .unwrap_or_else(|_| apex_watch::model::publisher_slug_for_domain(domain))
}

/// Best-effort confirmation email for a pending sign-up.
async fn send_watch_confirmation(
    state: &AppState,
    email: &str,
    outcome: &apex_watch::model::PendingOutcome,
    cancel_token: Option<&str>,
) {
    let Some(signin) = &state.signin else {
        tracing::warn!(
            reference = %outcome.reference,
            "setup-watch confirmation not sent: the leads root is not configured"
        );
        return;
    };
    let expires = now_unix_ms(state).saturating_add(apex_watch::model::CONFIRMATION_TTL_MS);
    signin
        .send_watch_confirmation(
            email,
            &outcome.reference,
            &outcome.confirmation_token,
            cancel_token,
            expires,
        )
        .await;
}

#[derive(Debug, Deserialize, Default)]
struct WatchCancelParams {
    #[serde(default, rename = "token")]
    token: Option<String>,
}

/// `GET /watch/cancel?token=`: inert confirmation. The token is never
/// consumed here, so a scanner cannot cancel a customer's watch.
async fn watch_cancel(
    State(state): State<AppState>,
    params: Result<Query<WatchCancelParams>, QueryRejection>,
) -> Response {
    let ctx = ConsoleCtx::from_env();
    let Some(store) = &state.watch else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let token = match params {
        Ok(Query(params)) => params.token,
        Err(_) => None,
    };
    let token = token
        .as_deref()
        .map(str::trim)
        .filter(|token| apex_watch::store::is_confirmation_token(token));
    let Some(token) = token else {
        return page(
            StatusCode::NOT_FOUND,
            watch_html::render_cancel_refused(&ctx),
        );
    };
    match store.find_by_cancel_token(token) {
        Ok(Some(_)) => page(StatusCode::OK, watch_html::render_cancel_form(&ctx, token)),
        Ok(None) => page(
            StatusCode::NOT_FOUND,
            watch_html::render_cancel_refused(&ctx),
        ),
        Err(error) => {
            tracing::warn!(%error, "cancel lookup failed");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::internal(&ctx, &console_reference(&state, "/watch/cancel")),
            )
        }
    }
}

/// `POST /watch/cancel`: consume the token (form body or, for RFC 8058
/// one-click unsubscribe, the query string with an empty body).
async fn watch_cancel_submit(
    State(state): State<AppState>,
    params: Result<Query<WatchCancelParams>, QueryRejection>,
    body: Bytes,
) -> Response {
    let ctx = ConsoleCtx::from_env();
    let Some(store) = &state.watch else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let from_query = match params {
        Ok(Query(params)) => params.token,
        Err(_) => None,
    };
    let from_body = leads::parse_form(&body)
        .ok()
        .and_then(|fields| fields.get("token").cloned());
    let token = from_body
        .as_deref()
        .or(from_query.as_deref())
        .map(str::trim)
        .filter(|token| apex_watch::store::is_confirmation_token(token));
    let Some(token) = token else {
        return page(
            StatusCode::BAD_REQUEST,
            watch_html::render_cancel_refused(&ctx),
        );
    };
    match store.cancel(token, now_unix_ms(&state)) {
        Ok(Some(subscriber)) => page(
            StatusCode::OK,
            watch_html::render_cancel_done(&ctx, &subscriber.domain),
        ),
        Ok(None) => page(
            StatusCode::NOT_FOUND,
            watch_html::render_cancel_refused(&ctx),
        ),
        Err(error) => {
            tracing::warn!(%error, "cancel failed");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::internal(&ctx, &console_reference(&state, "/watch/cancel")),
            )
        }
    }
}

async fn watch_requested(
    State(state): State<AppState>,
    params: Result<Query<RequestedParams>, QueryRejection>,
) -> Response {
    let ctx = ConsoleCtx::from_env();
    let reference = match params {
        Ok(Query(params)) => params.reference,
        Err(_) => None,
    };
    let reference = reference.filter(|value| is_subscriber_id(value));
    let subscriber = reference.as_deref().and_then(|reference| {
        state
            .watch
            .as_ref()
            .and_then(|store| find_subscriber(store, reference))
    });
    if subscriber.is_none() {
        return page(
            StatusCode::NOT_FOUND,
            watch_html::render_reference_not_found(&ctx),
        );
    }
    if let Some(row) = subscriber.as_ref() {
        if row.status == apex_watch::model::Status::Active && row.billing_profile.is_none() {
            return page(
                StatusCode::OK,
                watch_html::render_free_watch(&ctx, Some(&row.id), row.verified_app_count),
            );
        }
    }
    let verified_app_count = subscriber
        .as_ref()
        .and_then(|subscriber| subscriber.verified_app_count);
    let active = subscriber
        .as_ref()
        .is_some_and(|subscriber| subscriber.is_active());
    if active {
        page(
            StatusCode::OK,
            watch_html::render_free_watch(&ctx, reference.as_deref(), verified_app_count),
        )
    } else {
        page(
            StatusCode::OK,
            watch_html::render_confirmation(&ctx, reference.as_deref(), verified_app_count),
        )
    }
}

#[derive(Debug, Deserialize, Default)]
struct WatchConfirmParams {
    #[serde(default, rename = "token")]
    token: Option<String>,
}

/// `GET /watch/confirm?token=`: render the inert confirmation form.
///
/// The GET never consumes the token: a mail scanner that follows the link
/// only sees the button. Consumption happens exclusively in
/// [`watch_confirm_submit`], so the recipient's click is the only thing that
/// confirms the address.
async fn watch_confirm(
    State(state): State<AppState>,
    params: Result<Query<WatchConfirmParams>, QueryRejection>,
) -> Response {
    let ctx = ConsoleCtx::from_env();
    let Some(_store) = &state.watch else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let token = match params {
        Ok(Query(params)) => params.token,
        Err(_) => None,
    };
    let Some(token) = token.filter(|token| apex_watch::store::is_confirmation_token(token)) else {
        return page(
            StatusCode::BAD_REQUEST,
            watch_html::render_email_confirmation_refused(&ctx),
        );
    };
    page(
        StatusCode::OK,
        watch_html::render_email_confirmation_form(&ctx, &token),
    )
}

/// `POST /watch/confirm`: consume the one-time email confirmation.
/// Single use, expiry and unknown tokens are typed refusals; nothing mints a
/// session or switches a watch on (the later billing-profile POST does).
async fn watch_confirm_submit(State(state): State<AppState>, body: Bytes) -> Response {
    let ctx = ConsoleCtx::from_env();
    let Some(store) = &state.watch else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let Ok(fields) = leads::parse_form(&body) else {
        return page(
            StatusCode::BAD_REQUEST,
            watch_html::render_email_confirmation_refused(&ctx),
        );
    };
    let token = fields
        .get("token")
        .map(|token| token.trim())
        .filter(|token| apex_watch::store::is_confirmation_token(token));
    let Some(token) = token else {
        return page(
            StatusCode::BAD_REQUEST,
            watch_html::render_email_confirmation_refused(&ctx),
        );
    };
    let now = now_unix_ms(&state);
    match store.confirm_email(now, token) {
        Ok(outcome) => match store.create_profile_token_at(now, &outcome.reference) {
            Ok(profile_token) => {
                let values = watch_html::BillingProfileFormValues {
                    profile_token,
                    reference: outcome.reference,
                    ..Default::default()
                };
                page(
                    StatusCode::OK,
                    watch_html::render_billing_profile_form(
                        &ctx,
                        Some(&outcome.email),
                        &values,
                        &[],
                    ),
                )
            }
            Err(error) => {
                tracing::error!(%error, "billing-profile token could not be created");
                page(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    errors::internal(&ctx, &console_reference(&state, "/watch/confirm")),
                )
            }
        },
        Err(
            apex_watch::WatchError::ConfirmationUsed
            | apex_watch::WatchError::ConfirmationExpired
            | apex_watch::WatchError::ConfirmationUnknown
            | apex_watch::WatchError::AlreadyConfirmed,
        ) => page(
            StatusCode::OK,
            watch_html::render_email_confirmation_refused(&ctx),
        ),
        Err(error) => {
            tracing::error!(%error, "watch confirmation failed");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::internal(&ctx, &console_reference(&state, "/watch/confirm")),
            )
        }
    }
}

/// The stable recorded page after the billing profile is stored.
#[derive(Debug, Deserialize, Default)]
struct BillingProfileRecordedParams {
    #[serde(default, rename = "ref")]
    reference: Option<String>,
}

async fn watch_billing_profile_recorded(
    State(state): State<AppState>,
    params: Result<Query<BillingProfileRecordedParams>, QueryRejection>,
) -> Response {
    let ctx = ConsoleCtx::from_env();
    let reference = match params {
        Ok(Query(params)) => params.reference,
        Err(_) => None,
    };
    let reference = reference.filter(|value| is_subscriber_id(value));
    let subscriber = reference.as_deref().and_then(|reference| {
        state
            .watch
            .as_ref()
            .and_then(|store| find_subscriber(store, reference))
    });
    let active = subscriber
        .as_ref()
        .is_some_and(|subscriber| subscriber.is_active());
    let invoice_number = subscriber
        .as_ref()
        .and_then(|subscriber| subscriber.first_invoice_number);
    if subscriber.is_none() {
        return page(
            StatusCode::NOT_FOUND,
            watch_html::render_reference_not_found(&ctx),
        );
    }
    page(
        StatusCode::OK,
        watch_html::render_billing_profile_recorded(
            &ctx,
            reference.as_deref().unwrap_or_default(),
            active,
            invoice_number,
        ),
    )
}

/// Parse and validate the billing-profile form. Every refusal carries the
/// exact field copy and the safe values are preserved by the caller.
fn billing_profile_from_values(
    values: &watch_html::BillingProfileFormValues,
) -> Result<apex_watch::BillingProfile, Vec<watch_html::BillingProfileError>> {
    let mut errors = Vec::new();
    let legal_name = values.legal_name.trim();
    if legal_name.is_empty() || legal_name.chars().count() > 200 {
        errors.push(watch_html::BillingProfileError {
            field: "legal_name",
            message: watch_html::LEGAL_NAME_ERROR,
        });
    }
    let address_lines: Vec<String> = values
        .address_lines
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    if address_lines.is_empty()
        || address_lines.len() > 10
        || address_lines.iter().any(|line| line.chars().count() > 500)
    {
        errors.push(watch_html::BillingProfileError {
            field: "address_lines",
            message: watch_html::ADDRESS_ERROR,
        });
    }
    let country = values.country.trim().to_ascii_uppercase();
    if !apex_billing::vat::is_valid_country_code(&country) {
        errors.push(watch_html::BillingProfileError {
            field: "country",
            message: watch_html::COUNTRY_ERROR,
        });
    }
    let vat = values.vat_number.trim();
    let vat_number = if vat.is_empty() {
        None
    } else if apex_billing::is_valid_vat_number(vat) {
        Some(vat.to_string())
    } else {
        errors.push(watch_html::BillingProfileError {
            field: "vat_number",
            message: watch_html::VAT_ERROR,
        });
        None
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(apex_watch::BillingProfile {
        legal_name: legal_name.to_string(),
        address_lines,
        country,
        vat_number,
    })
}

/// `POST /watch/billing-profile`: consume the one-time profile token and
/// store the customer's legal identity. The token is validated and the form
/// checked before it is consumed, so a correctable field error does not burn
/// the link. A replay is refused without overwriting the stored profile.
async fn watch_billing_profile(State(state): State<AppState>, body: Bytes) -> Response {
    let ctx = ConsoleCtx::from_env();
    let Some(store) = &state.watch else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    let Ok(fields) = leads::parse_form(&body) else {
        return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None));
    };
    let values = watch_html::BillingProfileFormValues {
        profile_token: fields.get("profile_token").cloned().unwrap_or_default(),
        reference: fields.get("reference").cloned().unwrap_or_default(),
        legal_name: fields.get("legal_name").cloned().unwrap_or_default(),
        address_lines: fields.get("address_lines").cloned().unwrap_or_default(),
        country: fields.get("country").cloned().unwrap_or_default(),
        vat_number: fields.get("vat_number").cloned().unwrap_or_default(),
    };
    let token = values.profile_token.trim();
    if !apex_watch::is_confirmation_token(token) {
        return page(
            StatusCode::BAD_REQUEST,
            watch_html::render_billing_profile_refused(&ctx),
        );
    }
    let reference = values.reference.trim();
    if !reference.is_empty() && !is_subscriber_id(reference) {
        return page(
            StatusCode::BAD_REQUEST,
            watch_html::render_billing_profile_refused(&ctx),
        );
    }
    let profile = match billing_profile_from_values(&values) {
        Ok(profile) => profile,
        Err(errors) => {
            return page(
                StatusCode::BAD_REQUEST,
                watch_html::render_billing_profile_form(&ctx, None, &values, &errors),
            )
        }
    };
    let now = now_unix_ms(&state);
    match store.consume_profile_token(now, token) {
        Ok(consumed_reference) => {
            // The token is authoritative; a mismatched hidden reference is a
            // refusal (the token is already spent, so nothing can be
            // replayed against the other row).
            if !reference.is_empty() && reference != consumed_reference {
                return page(
                    StatusCode::BAD_REQUEST,
                    watch_html::render_billing_profile_refused(&ctx),
                );
            }
            match store.set_billing_profile(&consumed_reference, &profile, now) {
                Ok(subscriber) => {
                    // A3 B3: the app-ads.txt watch is free. Recording the
                    // billing profile activates it; there is no checkout, no
                    // first invoice and no stored-card charge.
                    match store.activate(now, &subscriber.id) {
                        Ok(outcome) => tracing::info!(
                            reference = %outcome.reference,
                            "setup watch activated free of charge"
                        ),
                        Err(error) => tracing::warn!(
                            %error,
                            reference = %subscriber.id,
                            "free setup watch could not be activated"
                        ),
                    }
                    Redirect::to(&format!(
                        "{}?ref={}",
                        watch_html::BILLING_PROFILE_RECORDED_PATH,
                        subscriber.id
                    ))
                    .into_response()
                }
                Err(error) => {
                    tracing::warn!(%error, "billing profile could not be stored");
                    page(
                        StatusCode::OK,
                        watch_html::render_billing_profile_refused(&ctx),
                    )
                }
            }
        }
        Err(
            apex_watch::WatchError::ProfileTokenUsed
            | apex_watch::WatchError::ProfileTokenExpired
            | apex_watch::WatchError::ProfileTokenUnknown,
        ) => page(
            StatusCode::OK,
            watch_html::render_billing_profile_refused(&ctx),
        ),
        Err(error) => {
            tracing::error!(%error, "billing-profile token could not be consumed");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                errors::internal(&ctx, &console_reference(&state, "/watch/billing-profile")),
            )
        }
    }
}

/// `sub_` plus 10 hex characters, the subscriber-id shape.
fn is_subscriber_id(value: &str) -> bool {
    value.strip_prefix("sub_").is_some_and(|hex| {
        (hex.len() == 10 || hex.len() == 32) && hex.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// Best-effort frozen outcomes append: the primary action's success never
/// depends on the outcomes file, so every failure is only logged.
fn append_outcome(state: &AppState, publisher: &str, kind: &str, reference: &str) {
    let Some(appender) = &state.outcomes else {
        return;
    };
    match appender.append(now_unix_ms(state), publisher, kind, reference) {
        Ok(true) => tracing::info!(publisher, kind, "sales outcome appended"),
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, publisher, kind, "sales outcome could not be appended")
        }
    }
}

fn page(status: StatusCode, body: String) -> Response {
    (status, Html(body)).into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tower::ServiceExt;

    use crate::sources::{StaticAuditSource, StaticLedgerSource};
    use crate::state::AppState;

    use super::*;

    #[test]
    fn the_publish_race_answers_with_the_durable_target() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("apex-job-publish-{}-{stamp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("opj_race.json");
        std::fs::write(&path, b"{\"job\":\"first\"}").expect("target");
        let temp = dir.join(".opj_race.tmp");
        std::fs::write(&temp, b"{\"job\":\"second\"}").expect("temp");

        // The loser of a real race gets AlreadyExists and answers with the
        // winning file's bytes, never an error.
        let answer = publish_operator_job_file(&temp, &path, b"{\"job\":\"second\"}", |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
        })
        .expect("the winner's file is the answer");
        assert_eq!(answer, b"{\"job\":\"first\"}".to_vec());

        // A successful link answers with the written body.
        let temp = dir.join(".opj_race2.tmp");
        std::fs::write(&temp, b"{\"job\":\"mine\"}").expect("temp");
        let answer = publish_operator_job_file(&temp, &path, b"{\"job\":\"mine\"}", |t, dst| {
            std::fs::rename(t, dst)
        })
        .expect("published");
        assert_eq!(answer, b"{\"job\":\"mine\"}".to_vec());

        // Any other failure surfaces; it is never papered over as "existing".
        let temp = dir.join(".opj_race3.tmp");
        std::fs::write(&temp, b"x").expect("temp");
        let error = publish_operator_job_file(&temp, &path, b"x", |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .expect_err("permission failures are not a race");
        assert!(matches!(error, ApiError::Internal(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_apps_textarea_is_bounded_and_deduped_like_the_tier_count() {
        // Fifty case-insensitive-distinct apps fit exactly.
        let fifty: Vec<String> = (0..50)
            .map(|index| format!("com.example.app{index}"))
            .collect();
        assert_eq!(
            parse_watch_apps(&fifty.join("\n")).expect("fifty").len(),
            50
        );
        // Case and whitespace variants of the same app are one line.
        let variants = "App.One\n app.one \nAPP.ONE\n";
        assert_eq!(
            parse_watch_apps(variants).expect("variants"),
            vec!["App.One".to_string()],
            "the first spelling is preserved"
        );
        // The Kelvin reproduction: 49 ASCII apps plus K (U+212A) and k are
        // 50 distinct under the Unicode identity the tier count uses. The old
        // ASCII fold counted 51 and refused a valid core signup.
        let mut kelvin: Vec<String> = (1..=49)
            .map(|index| format!("com.example.app{index}"))
            .collect();
        kelvin.push("K".to_string());
        kelvin.push("k".to_string());
        let parsed = parse_watch_apps(&kelvin.join("\n")).expect("kelvin fifty");
        assert_eq!(parsed.len(), 50);
        assert_eq!(
            apex_watch::distinct_app_count(&parsed),
            50,
            "the parser and the tier count agree on the Kelvin fold"
        );
        assert_eq!(
            apex_watch::tier_from_apps(apex_watch::distinct_app_count(&parsed)),
            apex_watch::Tier::Core,
            "fifty distinct apps are the core tier"
        );
        // Blank lines are ignored; the bound counts distinct apps.
        let fifty_one = format!(
            "{}\n",
            (0..51)
                .map(|index| format!("app{index}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(
            parse_watch_apps(&fifty_one),
            Err(watch_html::WATCH_APPS_ERROR)
        );
        // Malformed lines are typed refusals, not silently dropped.
        let long = "a".repeat(2_049);
        for bad in [
            "app with spaces",
            "https://",
            "ftp://example.com/app",
            long.as_str(),
        ] {
            assert_eq!(
                parse_watch_apps(bad),
                Err(watch_html::WATCH_APPS_ERROR),
                "{bad}"
            );
        }
    }

    #[tokio::test]
    async fn metrics_endpoint_counts_requests_and_exposes_uptime() {
        let app = router(AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        ));
        let health = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/healthz")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(health.status(), axum::http::StatusCode::OK);

        let metrics = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(metrics.status(), axum::http::StatusCode::OK);
        let content_type = metrics
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(content_type.starts_with("text/plain"), "{content_type}");
        let body = axum::body::to_bytes(metrics.into_body(), usize::MAX)
            .await
            .expect("body");
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("apex_api_requests_total{status=\"2xx\"} 1"),
            "{text}"
        );
        assert!(text.contains("apex_api_uptime_seconds"), "{text}");
        assert!(text.contains("apex_build_info{version="), "{text}");
        assert!(text.contains("apex_api_requests_in_flight"), "{text}");
        // The registry render is appended to the same document: the routed
        // /healthz request is recorded under its template, not its raw path.
        assert!(
            text.contains(
                "apex_http_requests_total{service=\"apex-api\",route_template=\"/healthz\",method=\"GET\",status_class=\"2xx\"} 1"
            ),
            "{text}"
        );
        assert!(
            text.contains("# TYPE apex_http_request_duration_seconds histogram\n"),
            "{text}"
        );
        assert!(text.contains("apex_http_request_duration_seconds_count{service=\"apex-api\",route_template=\"/healthz\",method=\"GET\"} 1"), "{text}");
        assert_prometheus_text_shaped(&text);
    }

    /// Read the `/metrics` body.
    async fn metrics_text(app: &Router) -> String {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8_lossy(&body).to_string()
    }

    /// One GET through the real router, returning only the status.
    async fn get_status(app: &Router, uri: &str) -> StatusCode {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        response.status()
    }

    /// One GET through the real router with a session cookie, returning only
    /// the status.
    async fn get_status_with_cookie(app: &Router, uri: &str, cookie: &str) -> StatusCode {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .header(axum::http::header::COOKIE, cookie)
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        response.status()
    }

    /// A configured state plus a live console session bound to `publisher`.
    fn state_with_session(publisher: &str) -> (AppState, String) {
        let state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        )
        .with_ledger_token(publisher, b"test-token".to_vec());
        let opened = state
            .ledger_sessions
            .open(publisher, state.clock.now_unix_s() * 1000);
        (
            state,
            format!("{}={}", crate::session::SESSION_COOKIE, opened.id),
        )
    }

    #[tokio::test]
    async fn font_assets_are_served_from_the_router() {
        use axum::http::header;
        let app = router(AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        ));
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/assets/fonts/manrope-400")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("font/woff2")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert!(body.starts_with(b"wOF2"));
        assert_eq!(
            get_status(&app, "/assets/fonts/manrope-999").await,
            StatusCode::NOT_FOUND
        );
    }

    /// Structural checks of the Prometheus exposition: every sample line is
    /// `series value` with balanced quotes and braces, every comment line is
    /// a HELP/TYPE line, and the document ends with a newline.
    fn assert_prometheus_text_shaped(text: &str) {
        assert!(text.ends_with('\n'), "document must end with a newline");
        for line in text.lines() {
            if line.starts_with('#') {
                assert!(
                    line.starts_with("# HELP ") || line.starts_with("# TYPE "),
                    "unknown comment line: {line}"
                );
                continue;
            }
            let (series, value) = line
                .rsplit_once(' ')
                .unwrap_or_else(|| panic!("sample line without a value: {line}"));
            assert!(series.starts_with("apex_"), "series name: {line}");
            assert_eq!(
                series.matches('"').count() % 2,
                0,
                "unbalanced quotes: {line}"
            );
            assert_eq!(
                series.matches('{').count(),
                series.matches('}').count(),
                "unbalanced braces: {line}"
            );
            assert!(
                value.parse::<f64>().is_ok()
                    || value == "NaN"
                    || value == "+Inf"
                    || value == "-Inf",
                "value is not a Prometheus number: {line}"
            );
        }
    }

    #[tokio::test]
    async fn raw_route_ids_never_reach_the_metrics_and_the_text_stays_well_formed() {
        let (state, cookie) = state_with_session("pub1");
        let app = router(state);
        // A hostile finding id carrying a quote and a backslash,
        // percent-encoded in the URL: `zz"qq\rr`. The route matches (so the
        // request is recorded), the handler answers 404, and the labels must
        // hold the template only.
        let status =
            get_status_with_cookie(&app, "/console/audit/findings/zz%22qq%5Crr", &cookie).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let text = metrics_text(&app).await;
        assert!(
            text.contains(
                "apex_http_requests_total{service=\"apex-api\",route_template=\"/console/audit/findings/{id}\",method=\"GET\",status_class=\"4xx\"} 1"
            ),
            "{text}"
        );
        // Neither the encoded nor the decoded id (nor an escaped variant)
        // appears anywhere in the metrics document.
        assert!(!text.contains("zz"), "{text}");
        assert!(!text.contains("qq"), "{text}");
        assert_prometheus_text_shaped(&text);
    }

    #[tokio::test]
    async fn status_class_mapping_covers_real_route_outcomes() {
        let dir = std::env::temp_dir().join(format!("apex-api-metrics-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        // An unparseable snapshot makes the audit console take its real 500
        // path.
        std::fs::create_dir_all(dir.join("pub1")).expect("snapshot directory");
        std::fs::write(dir.join("pub1").join("admob.json"), "{not a snapshot}")
            .expect("snapshot file");
        let state = AppState::new(
            Arc::new(crate::sources::JsonFileAuditSource::new(&dir)),
            Arc::new(StaticLedgerSource::default()),
        )
        .with_ledger_token("pub1", b"test-token".to_vec());
        let opened = state
            .ledger_sessions
            .open("pub1", state.clock.now_unix_s() * 1000);
        let cookie = format!("{}={}", crate::session::SESSION_COOKIE, opened.id);
        let app = router(state);
        assert_eq!(get_status(&app, "/healthz").await, StatusCode::OK);
        assert_eq!(get_status(&app, "/free-audit").await, StatusCode::SEE_OTHER);
        assert_eq!(
            get_status_with_cookie(&app, "/console/audit/findings/not-a-finding", &cookie).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get_status_with_cookie(&app, "/console/audit?platform=admob", &cookie).await,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        let text = metrics_text(&app).await;
        for (route, status_class) in [
            ("/healthz", "2xx"),
            ("/free-audit", "3xx"),
            ("/console/audit/findings/{id}", "4xx"),
            ("/console/audit", "5xx"),
        ] {
            let series = format!(
                "apex_http_requests_total{{service=\"apex-api\",route_template=\"{route}\",method=\"GET\",status_class=\"{status_class}\"}} 1"
            );
            assert!(text.contains(&series), "missing {series} in\n{text}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_in_flight_gauge_returns_to_zero_after_requests() {
        let state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        );
        let app = router(state.clone());
        assert_eq!(get_status(&app, "/healthz").await, StatusCode::OK);
        // An unmatched path is still served (the 404 fallback) and must not
        // leak the gauge either.
        assert_eq!(
            get_status(&app, "/v1/does-not-exist").await,
            StatusCode::NOT_FOUND
        );
        let labels = [("service", "apex-api")];
        let family = state
            .metrics_registry
            .family_snapshot(apex_metrics::catalog::HTTP_REQUESTS_IN_FLIGHT.name)
            .expect("the guard wrote the series");
        assert_eq!(family.series[0].gauge, Some(0.0), "{family:?}");
        assert_eq!(
            state
                .metrics_registry
                .gauge_value(&apex_metrics::catalog::HTTP_REQUESTS_IN_FLIGHT, &labels)
                .expect("gauge readable"),
            0.0
        );
    }

    #[tokio::test]
    async fn fallback_and_method_mismatch_are_axum_defaults() {
        let app = router(AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        ));
        let missing = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/does-not-exist")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let wrong_method = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/healthz")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn console_method_mismatch_answers_the_html_405_hint() {
        let app = router(AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        ));
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(content_type.starts_with("text/html"), "{content_type}");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("Method not allowed."), "{text}");
        assert!(!text.contains("<script"), "console stays script-free");
    }

    #[test]
    fn request_id_shape_is_exact() {
        assert!(is_request_id("req_0123456789"));
        assert!(is_request_id("req_abcdefabcd"));
        assert!(!is_request_id("req_012345678"));
        assert!(!is_request_id("req_0123456789a"));
        assert!(!is_request_id("req_zzzzzzzzzz"));
        assert!(!is_request_id(""));
    }

    #[test]
    fn subscriber_id_shape_is_exact() {
        assert!(is_subscriber_id("sub_0123456789"));
        assert!(is_subscriber_id("sub_abcdefabcd"));
        assert!(!is_subscriber_id("sub_012345678"));
        assert!(!is_subscriber_id("sub_0123456789a"));
        assert!(!is_subscriber_id("sub_zzzzzzzzzz"));
        assert!(!is_subscriber_id("req_0123456789"));
        assert!(!is_subscriber_id(""));
    }

    /// A validation success through `apply_validation` (the console
    /// connection seam) appends the frozen `connected` outcome exactly once;
    /// a second validation of the same `(publisher, platform)` appends
    /// nothing, and without an outcomes appender the call site is a no-op.
    #[tokio::test]
    async fn a_validated_connection_appends_the_connected_outcome_write_once() {
        use std::collections::VecDeque;
        use std::sync::Mutex;

        use apex_connectors::transport::{
            HttpRequest, HttpResponse, RecordingSleeper, RetryTransport, Transport, TransportError,
        };

        #[derive(Debug)]
        struct ScriptedTransport {
            script: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
        }

        #[async_trait::async_trait]
        impl Transport for ScriptedTransport {
            async fn request(&self, _request: HttpRequest) -> Result<HttpResponse, TransportError> {
                self.script
                    .lock()
                    .expect("script lock")
                    .pop_front()
                    .unwrap_or(Err(TransportError::Network("script exhausted".into())))
            }
        }

        #[derive(Debug, Default)]
        struct StaticAccounts {
            account: Option<crate::live::LiveAccount>,
        }

        #[async_trait::async_trait]
        impl crate::live::LiveAccountStore for StaticAccounts {
            async fn account(
                &self,
                _publisher: &str,
            ) -> Result<crate::live::LiveAccount, ApiError> {
                self.account.clone().ok_or(ApiError::NotFound)
            }
        }

        fn ok(status: u16, body: &str) -> Result<HttpResponse, TransportError> {
            Ok(HttpResponse {
                status,
                headers: vec![],
                body: body.as_bytes().to_vec(),
            })
        }

        let mut platforms = std::collections::BTreeMap::new();
        platforms.insert(
            "applovin_max".to_string(),
            crate::live::LiveCredentials {
                api_key: "key".to_string(),
                account_id: "acct".to_string(),
                base_url: "https://api.example.test".to_string(),
                report_key: None,
                reporting_base_url: None,
                refresh_token: None,
            },
        );
        let account = crate::live::LiveAccount {
            developer_site: "studio.example.com".to_string(),
            expected_networks: Vec::new(),
            ad_unit_ids: Vec::new(),
            bidder_accounts: Vec::new(),
            platforms,
        };
        let transport = Arc::new(ScriptedTransport {
            script: Mutex::new(
                vec![
                    ok(
                        200,
                        r#"[{"id":"u1","name":"Banner","platform":"ios","package_name":"com.x","ad_format":"BANNER"}]"#,
                    ),
                    ok(200, "google.com, pub-1, DIRECT\n"),
                ]
                .into(),
            ),
        });
        let retry: Arc<dyn Transport> = Arc::new(RetryTransport::new(
            transport,
            0,
            Arc::new(RecordingSleeper::default()),
        ));
        let auditor = Arc::new(crate::live::LiveAuditor::new(
            Arc::new(StaticAccounts {
                account: Some(account),
            }),
            retry,
            Arc::new(crate::clock::FixedClock::new(1_700_000_000)),
        ));

        let dir = tempfile::tempdir().expect("temp dir");
        let outcomes_path = dir.path().join("outcomes.jsonl");
        let state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        )
        .with_connection_validator(auditor)
        .with_outcomes(Arc::new(crate::outcomes::OutcomesAppender::new(
            &outcomes_path,
        )));

        let previous = crate::connections::PlatformState::default();
        let mut row = crate::connections::PlatformState::default();
        apply_validation(
            &state,
            "pub1",
            PlatformId::AppLovinMax,
            true,
            &previous,
            &mut row,
        )
        .await;
        assert_eq!(
            row.credential_state(),
            crate::connections::CredentialState::Validated,
            "the scripted audit must validate"
        );
        let raw = std::fs::read_to_string(&outcomes_path).expect("outcomes");
        let line: serde_json::Value = serde_json::from_str(raw.trim()).expect("frozen line");
        assert_eq!(line["publisher"], "pub1");
        assert_eq!(line["kind"], "connected");
        assert_eq!(line["ref"], "pub1:applovin_max");

        // The second validation of the same connection appends nothing.
        let mut row = crate::connections::PlatformState::default();
        apply_validation(
            &state,
            "pub1",
            PlatformId::AppLovinMax,
            true,
            &previous,
            &mut row,
        )
        .await;
        assert_eq!(
            std::fs::read_to_string(&outcomes_path)
                .expect("outcomes")
                .lines()
                .count(),
            1,
            "write-once per (kind, ref)"
        );

        // Without an appender the call site stays a silent no-op.
        let plain = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        );
        append_outcome(&plain, "pub1", "connected", "pub1:applovin_max");
    }

    #[test]
    fn the_public_rate_limiter_counts_per_key_per_window() {
        let limiter = PublicRateLimiter::new(16, PUBLIC_RATE_LIMIT_WINDOW_MS);
        let now = 1_792_000_000_000;
        for _ in 0..3 {
            assert!(limiter.allow("a", now, 3));
        }
        assert!(!limiter.allow("a", now, 3), "the limit is reached");
        assert!(!limiter.allow("a", now + 59_999, 3), "still the window");
        assert!(
            limiter.allow("a", now + PUBLIC_RATE_LIMIT_WINDOW_MS, 3),
            "a new window resets the count"
        );
        assert!(limiter.allow("b", now, 3), "another key is unaffected");
    }

    #[test]
    fn the_public_rate_limiter_map_is_bounded_and_evicts_the_oldest() {
        let limiter = PublicRateLimiter::new(2, PUBLIC_RATE_LIMIT_WINDOW_MS);
        let now = 1_792_000_000_000;
        assert!(limiter.allow("first", now, 30));
        assert!(limiter.allow("second", now, 30));
        assert_eq!(limiter.len(), 2);
        assert!(limiter.allow("third", now, 30));
        assert_eq!(limiter.len(), 2, "capacity is a hard bound");
        assert!(!limiter.is_empty(), "the two newest keys are retained");
        // The oldest key's budget was evicted, so it starts a fresh window
        // even though its old count would have exhausted a 1-request limit.
        assert!(limiter.allow("first", now, 1));
    }

    /// A free-audit deployment: a leads store, the shared workspace token
    /// file, a sign-in service with a file mailer and a proof source accepting
    /// `accepted`. Returns the state, the shared workspace store, the token
    /// file path and the captured-mail path.
    fn free_audit_state(
        dir: &std::path::Path,
        accepted: &[&str],
    ) -> (
        AppState,
        Arc<crate::signin::WorkspaceTokens>,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let tokens_path = dir.join("tokens.json");
        let workspaces = Arc::new(crate::signin::WorkspaceTokens::open(&tokens_path));
        let mail_path = dir.join("mail.jsonl");
        let signin = Arc::new(
            crate::signin::SigninService::open(dir)
                .with_public_base("https://console.example")
                .with_workspaces(Arc::clone(&workspaces))
                .with_mailer(Arc::new(apex_notify::sink::FileSink::new(&mail_path))),
        );
        let state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        )
        .with_leads(Arc::new(crate::leads::FileLeadStore::new(
            dir.join("leads.jsonl"),
        )))
        .with_workspaces(Arc::clone(&workspaces))
        .with_signin(signin)
        .with_app_ads_proof(Arc::new(
            crate::app_ads_proof::StaticAppAdsProof::accepting(accepted.to_vec()),
        ));
        (state, workspaces, tokens_path, mail_path)
    }

    /// Submit one free-audit form through the real router.
    async fn post_free_audit(app: &Router, email: &str, studio: &str) -> StatusCode {
        let body = format!(
            "email={}&studio={}&platforms=max",
            email.replace('@', "%40"),
            studio.replace(' ', "+")
        );
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/free-audit")
                    .header(
                        axum::http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(axum::body::Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response")
            .status()
    }

    fn stored_tokens(path: &std::path::Path) -> std::collections::BTreeMap<String, String> {
        let raw = std::fs::read_to_string(path).expect("token file");
        serde_json::from_str(&raw).expect("token json object")
    }

    #[tokio::test]
    async fn a_new_workspace_stores_and_mails_a_hashed_key() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (state, _workspaces, tokens_path, mail_path) =
            free_audit_state(dir.path(), &["a.b.example"]);
        let app = router(state.clone());
        assert_eq!(
            post_free_audit(&app, "founder@a.b.example", "Northstar Studio").await,
            StatusCode::SEE_OTHER
        );
        let expected = crate::signin::hashed_workspace_key("a.b.example");
        let tokens = stored_tokens(&tokens_path);
        assert!(
            tokens.contains_key(&expected),
            "the new workspace key is hashed: {tokens:?}"
        );
        assert_eq!(tokens.len(), 1, "no legacy slug entry was written");
        let rows = state
            .signin
            .as_ref()
            .expect("sign-in configured")
            .store()
            .read_rows()
            .expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].publisher, expected,
            "the one-time link is bound to the hashed key"
        );
        let mail = std::fs::read_to_string(&mail_path).expect("mail captured");
        assert!(mail.contains("founder@a.b.example"), "{mail}");
        assert!(mail.contains("/console/signin?token="), "{mail}");
    }

    #[tokio::test]
    async fn a_token_provisioned_slug_keeps_the_legacy_key_and_still_claims() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (state, workspaces, tokens_path, _mail) =
            free_audit_state(dir.path(), &["a-b.example"]);
        std::fs::write(
            &tokens_path,
            "{\n  \"a-b-example\": \"operator-token\"\n}\n",
        )
        .expect("seed operator token");
        let app = router(state.clone());
        assert_eq!(
            post_free_audit(&app, "founder@a-b.example", "Dashed Studio").await,
            StatusCode::SEE_OTHER
        );
        // The legacy slug is kept (the operator's provisioning still matches)
        // and no hashed duplicate appears.
        let tokens = stored_tokens(&tokens_path);
        assert_eq!(
            tokens.get("a-b-example").map(String::as_str),
            Some("operator-token")
        );
        assert_eq!(tokens.len(), 1, "{tokens:?}");
        // The token-only entry was claimed by the proven domain.
        assert!(workspaces.contact_recorded("a-b-example", "founder@a-b.example"));
        assert_eq!(
            workspaces
                .record("a-b-example")
                .and_then(|record| record.domain),
            Some("a-b.example".to_string())
        );
        let rows = state
            .signin
            .as_ref()
            .expect("sign-in configured")
            .store()
            .read_rows()
            .expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].publisher, "a-b-example");
    }

    #[tokio::test]
    async fn a_colliding_domain_gets_a_distinct_hashed_key_and_cannot_reach_the_first_workspace() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (state, workspaces, tokens_path, _mail) =
            free_audit_state(dir.path(), &["a.b.example", "a-b.example"]);
        // The first domain already claimed the shared legacy slug `a-b-example`
        // (operator provisioning plus proof), binding it to `a-b.example`.
        workspaces
            .ensure("a-b-example", "owner@a-b.example", true)
            .expect("first claim");
        let first_token = workspaces.token("a-b-example").expect("legacy token");

        let app = router(state.clone());
        assert_eq!(
            post_free_audit(&app, "other@a.b.example", "Dotted Studio").await,
            StatusCode::SEE_OTHER
        );
        // The colliding domain gets the hashed key, distinct from the claimed
        // legacy slug.
        let expected = crate::signin::hashed_workspace_key("a.b.example");
        assert_ne!(expected, "a-b-example");
        let tokens = stored_tokens(&tokens_path);
        assert_eq!(
            tokens.get("a-b-example").map(String::as_str),
            Some(first_token.as_str()),
            "the first workspace's token is untouched"
        );
        assert!(
            tokens.contains_key(&expected),
            "the colliding domain is filed under the hashed key: {tokens:?}"
        );
        // It cannot see or extend the first workspace: the legacy record keeps
        // only its original owner, and the colliding address is not a contact.
        let first = workspaces.record("a-b-example").expect("first record");
        assert_eq!(first.owner_email, "owner@a-b.example");
        assert_eq!(
            first.contacts,
            std::collections::BTreeSet::from(["owner@a-b.example".to_string()]),
            "the colliding domain never extended the first workspace"
        );
        assert!(!workspaces.contact_recorded("a-b-example", "other@a.b.example"));
        // The colliding domain's own workspace is recorded under the hashed key
        // and bound to its own domain.
        let hashed_record = workspaces.record(&expected).expect("hashed record");
        assert_eq!(hashed_record.domain.as_deref(), Some("a.b.example"));
        // The one-time link names the hashed key, never the claimed slug.
        let rows = state
            .signin
            .as_ref()
            .expect("sign-in configured")
            .store()
            .read_rows()
            .expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].publisher, expected);
    }
}
