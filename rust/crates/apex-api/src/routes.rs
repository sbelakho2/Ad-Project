//! The axum 0.8 router: JSON API plus the server-rendered console.
//!
//! Wire contract (JSON responses are `application/json`; `GET /metrics` is
//! Prometheus text):
//!
//! | route | success | refusals |
//! |---|---|---|
//! | `GET /healthz` | `200 {"status":"ok"}` | — |
//! | `GET /metrics` | `200` Prometheus text (`text/plain`) | — |
//! | `GET /v1/audit/{platform}` | `200` `AuditReport` | `400 unknown_platform / bad_request`, `404 not_found` |
//! | `POST /v1/audit/{platform}` | `200` live read-only `AuditReport` | `400 bad_request / unknown_platform / invalid_publisher`, `401 unauthorized`, `502 platform_unavailable`, `503 live_audit_not_configured` |
//! | `GET /v1/ledger/{publisher}` | `200 {"publisher","mode":"read_only","records"}` | `400 invalid_publisher / bad_request`, `401 unauthorized`, `404 not_found`, `422 {"error":"ledger_unverified","line":n}`, `503 ledger_auth_not_configured` |
//! | `POST /v1/cycle/{publisher}?platform=` | `200` read-only summary | `400 platform_required / unknown_platform / invalid_publisher / invalid_cycle_request / invalid_policy / bad_request`, `404`, `422` |
//! | `POST /v1/adyen/webhook` | `200 {"received":true,"items":n,"new":m}` | `400 invalid_signature` or `malformed_event`, `503 adyen_webhook_not_configured` or `collection_store_not_configured` |
//! | `POST /v1/leads` | `202 {"received":true}` | `400 invalid_lead`, `503 lead_capture_not_configured` |
//!
//! Unknown paths fall through to `404 {"error":"not_found"}`; a known path
//! with the wrong method is axum's `405` (asserted by tests) on the API
//! routes, and the HTML 405 hint page on the console routes.
//!
//! Console routes (server-rendered HTML, no JavaScript; every mutation is a
//! POST that answers `303 See Other`):
//!
//! | route | success | states |
//! |---|---|---|
//! | `GET /` | `200` overview | first-run state when no source exists |
//! | `GET /console` | `303` to the change log | — |
//! | `GET /console/audit` | `200` form or report | `400` bad query / unknown platform, `404` no snapshot, `500` read failure |
//! | `GET /console/audit/findings/{id}` | `200` finding detail | `404` unknown id, `500` read failure |
//! | `GET /console/audit/export.csv` | `200` RFC 4180 CSV | `400` missing or unknown platform, `404` no snapshot, `500` read failure |
//! | `GET /console/ledger` | `200` access panel or verified records | `404` no ledger, `422` ledger_unverified, `500` read failure |
//! | `POST /console/ledger` | `303` to the change log + session cookie | `400` invalid publisher, `401` invalid access, `503` ledger_auth_not_configured |
//! | `GET /console/ledger/records/{seq}` | `200` record detail | `303` to the change log without a session, `404` unknown sequence |
//! | `POST /console/ledger/session/end` | `303` to the change log | `400` missing or wrong CSRF token |
//! | `GET /console/connections` | `200` platform list | `503` workspace publisher not configured, `500` store read failure |
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
//! | `GET /console/settings/monitoring` | `200` health and plan | `500` connection store read failure |
//! | `GET /console/analytics?view=&format=csv` | `200` tab, or CSV | `400` unknown view or platform filter, `503` store not configured (CSV exports answer `503 text/plain` with the reason) |
//! | `POST /free-audit` | `303` to `/audit-requested/?ref=` | `400` field errors, `500`/`503` storage failure |
//! | `GET /free-audit` | `303` to `/free-audit/` | — |
//! | `GET /free-audit/` | `200` request page | — |
//! | `GET /audit-requested/` | `200` confirmation | — |
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
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tower_http::trace::TraceLayer;

use apex_core::PlatformId;

use crate::cycle::{CycleRequest, CycleSummary, READ_ONLY_MODE};
use crate::error::ApiError;
use crate::html::audit::{is_finding_id, AuditFormProblem, AuditReportView, FindingView};
use crate::html::leads_html::RequestPageState;
use crate::html::ledger::LedgerAccessProblem;
use crate::html::overview::{LatestAudit, LedgerState, OverviewData};
use crate::html::{audit, errors, leads_html, ledger, overview, ConsoleCtx};
use crate::leads::{self, Lead};
use crate::session;
use crate::sources::{is_valid_publisher, parse_platform, PLATFORM_NAMES};
use crate::state::AppState;

/// Hard cap on every request body, enforced by [`DefaultBodyLimit`] and by the
/// pre-routing `Content-Length` check below. 256 KiB is far above the largest
/// legitimate body (the optional cycle policy JSON) and bounds the memory a
/// single request can make the server buffer.
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// Build the complete router (API + console) over `state`.
///
/// The console routes are registered before
/// [`Router::method_not_allowed_fallback`] so only they get the HTML 405 hint
/// page; the API routes keep axum's default `405` with its `Allow` header.
pub fn router(state: AppState) -> Router {
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
        .route("/console/analytics", get(console_analytics))
        .route(
            "/free-audit",
            get(free_audit_redirect).post(free_audit_submit),
        )
        .route("/free-audit/", get(free_audit_page))
        .route("/audit-requested/", get(audit_requested))
        .method_not_allowed_fallback(console_method_not_allowed)
        .route("/healthz", get(healthz))
        .route("/favicon.ico", get(crate::favicon::ico))
        .route("/favicon.svg", get(crate::favicon::svg))
        .route("/metrics", get(crate::metrics::metrics))
        .route("/v1/audit/{platform}", get(audit_report).post(live_audit))
        .route("/v1/ledger/{publisher}", get(ledger))
        .route("/v1/cycle/{publisher}", post(run_cycle))
        .route("/v1/adyen/webhook", post(adyen_webhook))
        .route("/v1/leads", post(lead_capture))
        .fallback(not_found)
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

async fn not_found() -> ApiError {
    ApiError::NotFound
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

async fn audit_report(
    State(state): State<AppState>,
    platform: Result<Path<String>, PathRejection>,
) -> Result<Json<apex_audit::AuditReport>, ApiError> {
    let Ok(Path(platform)) = platform else {
        return Err(ApiError::BadRequest("bad_request"));
    };
    let platform = parse_platform(&platform).ok_or(ApiError::BadRequest("unknown_platform"))?;
    let snapshot = state.audit_source.snapshot(platform).await?;
    Ok(Json(apex_audit::run(&snapshot)))
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
    // can consume the same bytes the live audit just built.
    if let Err(error) = state.audit_source.write(platform, &snapshot).await {
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
    let lead =
        leads::parse_lead(&body, content_type).map_err(|_| ApiError::BadRequest("invalid_lead"))?;
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
    publisher: Result<Path<String>, PathRejection>,
    params: Result<Query<CycleParams>, QueryRejection>,
    body: Bytes,
) -> Result<Json<CycleSummary>, ApiError> {
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

async fn adyen_webhook(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Precedence, pinned by tests: a configured AdyenClient supplies the HMAC
    // key; the state's explicit `adyen_hmac_key` is the fallback for
    // deployments that hold only the key. With neither the route is 503.
    let hmac_key = match &state.adyen {
        Some(client) => client.hmac_key().to_vec(),
        None => match &state.adyen_hmac_key {
            Some(key) => key.clone(),
            None => return Err(ApiError::AdyenWebhookNotConfigured),
        },
    };
    // The signature travels inside each notification item; a malformed
    // envelope is a bad request and a digest mismatch is an invalid signature.
    let items =
        apex_billing::verify_adyen_webhook(&body, &hmac_key).map_err(|error| match error {
            apex_billing::BillingError::InvalidSignature => ApiError::InvalidSignature,
            other => {
                tracing::debug!(%other, "adyen delivery refused");
                ApiError::BadRequest("malformed_event")
            }
        })?;
    let Some(store) = &state.collection else {
        return Err(ApiError::CollectionStoreNotConfigured);
    };
    // Record every accepted item; a replay of an already-recorded
    // (eventCode, pspReference) is accepted but not counted again.
    let now_unix_ms = state.clock.now_unix_s().saturating_mul(1000);
    let mut new_items = 0usize;
    for item in &items {
        let event = apex_billing::CollectionEvent::from_item(item, now_unix_ms);
        match store.record(&event) {
            Ok(true) => {
                new_items += 1;
                tracing::info!(
                    event_code = %item.event_code,
                    psp_reference = %item.psp_reference,
                    success = %item.success,
                    "adyen collection event recorded"
                );
            }
            Ok(false) => {
                tracing::info!(
                    psp_reference = %item.psp_reference,
                    "adyen collection event already recorded (replay)"
                );
            }
            Err(error) => {
                tracing::error!(%error, "adyen collection event could not be recorded");
                return Err(ApiError::Internal("collection_record_failed".into()));
            }
        }
    }
    Ok(Json(
        json!({ "received": true, "items": items.len(), "new": new_items }),
    ))
}

// ---------------------------------------------------------------------------
// Console handlers
// ---------------------------------------------------------------------------

/// The UTC "now" as epoch milliseconds, from the injected clock.
fn now_unix_ms(state: &AppState) -> i64 {
    state.clock.now_unix_s().saturating_mul(1000)
}

/// A quotable support reference: the route and the UTC moment it failed.
fn console_reference(state: &AppState, route: &str) -> String {
    format!(
        "{route} at {}",
        apex_ui::format::utc_datetime(now_unix_ms(state))
    )
}

async fn console_overview(State(state): State<AppState>) -> Html<String> {
    let ctx = ConsoleCtx::from_env();
    let data = collect_overview(&state).await;
    Html(overview::render(&ctx, &data))
}

/// `GET /console`: the bare address the deployment proxy routes to this
/// service and the docs advertise as the change log, redirected to its
/// stable location.
async fn console_bare() -> Response {
    Redirect::to("/console/ledger").into_response()
}

/// Read the real overview inputs: the newest snapshot across the three
/// platforms and the verified record totals of every configured publisher
/// ledger. Source failures never fabricate numbers; they surface as the
/// documented unavailable states.
async fn collect_overview(state: &AppState) -> OverviewData {
    let mut latest: Option<LatestAudit> = None;
    for (name, label) in PLATFORM_NAMES {
        let Some(platform) = parse_platform(name) else {
            continue;
        };
        match state.audit_source.snapshot(platform).await {
            Ok(snapshot) => {
                if latest.as_ref().is_some_and(|current| {
                    current.captured_at_unix_ms >= snapshot.fetched_at_unix_ms
                }) {
                    continue;
                }
                let report = apex_audit::run(&snapshot);
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
                tracing::warn!(%error, platform = name, "overview snapshot read failed");
            }
        }
    }
    let publishers = state.ledger_source.publishers();
    let ledger = if publishers.is_empty() {
        LedgerState::NotConfigured
    } else {
        let mut records = 0u64;
        let mut unavailable = 0usize;
        let mut first_reason: Option<String> = None;
        for publisher in &publishers {
            match state.ledger_source.records(publisher).await {
                Ok(list) => records += list.len() as u64,
                Err(error) => {
                    unavailable += 1;
                    if first_reason.is_none() {
                        first_reason = Some(overview_reason(&error));
                    }
                }
            }
        }
        if unavailable == publishers.len() {
            LedgerState::Unavailable {
                reason: first_reason.unwrap_or_else(|| "history unavailable".to_string()),
            }
        } else {
            LedgerState::Verified {
                records,
                unavailable,
            }
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
    params: Result<Query<AuditConsoleParams>, QueryRejection>,
) -> Response {
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
    match state.audit_source.snapshot(platform).await {
        Ok(snapshot) => {
            let report = apex_audit::run(&snapshot);
            page(
                StatusCode::OK,
                audit::render_audit_report(
                    &ctx,
                    &AuditReportView {
                        platform,
                        snapshot: &snapshot,
                        report: &report,
                    },
                ),
            )
        }
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
    id: Result<Path<String>, PathRejection>,
) -> Response {
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
        match state.audit_source.snapshot(platform).await {
            Ok(snapshot) => {
                let report = apex_audit::run(&snapshot);
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
    params: Result<Query<AuditConsoleParams>, QueryRejection>,
) -> Response {
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
    match state.audit_source.snapshot(platform).await {
        Ok(snapshot) => {
            let report = apex_audit::run(&snapshot);
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

async fn console_ledger(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ConsoleCtx::from_env();
    let (id, view) = live_session(&state, &headers);
    let Some(view) = view else {
        let problem = id.map(|_| LedgerAccessProblem::SessionExpired);
        return page(
            StatusCode::OK,
            ledger::render_access_panel(&ctx, problem, None),
        );
    };
    let publisher = view.publisher.clone();
    match state.ledger_source.records(&publisher).await {
        Ok(records) => {
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
            let html =
                ledger::render_records(&ctx, &publisher, &records, now_unix_ms(&state), &view.csrf);
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
    match state.ledger_source.records(&publisher).await {
        Ok(records) => {
            let Some(record) = records.iter().find(|record| record.seq() == seq) else {
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

/// The workspace publisher, or the HTML not-configured page (boxed: a
/// `Response` is large and this is the cold path).
fn workspace_ctx(state: &AppState, ctx: &ConsoleCtx) -> Result<String, Box<Response>> {
    match &state.workspace_publisher {
        Some(publisher) => Ok(publisher.clone()),
        None => Err(Box::new(page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(ctx),
        ))),
    }
}

/// The session plus CSRF check shared by every phase-2 mutation. A request
/// without a live session is redirected to the change-log access panel (the
/// stable recovery point); a wrong token is the HTML `400`.
fn mutation_session(
    state: &AppState,
    ctx: &ConsoleCtx,
    headers: &HeaderMap,
    fields: &std::collections::BTreeMap<String, String>,
) -> Result<crate::session::SessionView, Box<Response>> {
    let (_, view) = live_session(state, headers);
    let Some(view) = view else {
        return Err(Box::new(Redirect::to("/console/ledger").into_response()));
    };
    let csrf = fields.get(leads_csrf_field()).cloned().unwrap_or_default();
    if !crate::live::constant_time_eq(csrf.as_bytes(), view.csrf.as_bytes()) {
        return Err(Box::new(page(
            StatusCode::BAD_REQUEST,
            errors::bad_request(ctx, None),
        )));
    }
    Ok(view)
}

/// `GET /console/connections`.
async fn console_connections(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
            .snapshot(platform)
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
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
    connection_detail_response(&state, &ctx, &headers, &publisher, platform, capability).await
}

async fn connection_detail_response(
    state: &AppState,
    ctx: &ConsoleCtx,
    headers: &HeaderMap,
    publisher: &str,
    platform: PlatformId,
    capability: &crate::connections::PlatformCapability,
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
    let (_, session) = live_session(state, headers);
    let running_work = running_experiments(state, publisher, Some(platform)).await;
    let last_report = state
        .audit_source
        .snapshot(platform)
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
                csrf: session.as_ref().map(|session| session.csrf.as_str()),
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
    let records = match state.ledger_source.records(publisher).await {
        Ok(records) => records,
        // No ledger file at all: nothing has been recorded for this
        // publisher, which is honestly zero running work.
        Err(ApiError::NotFound) => return Some(0),
        // A wrong store, a verification failure or a read error: the
        // warning must not claim the work was checked.
        Err(_) => return None,
    };
    let alerts = state
        .alerts_path
        .as_deref()
        .and_then(|path| crate::experiments::read_alerts(path).ok())
        .unwrap_or_default();
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
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
    if let Err(response) = mutation_session(&state, &ctx, &headers, &fields) {
        return *response;
    }
    let credentials = match parse_connection_submission(platform, &fields) {
        Ok(credentials) => credentials,
        Err((errors, preserved)) => {
            let meta = store.load_meta(&publisher).await.unwrap_or_default();
            let (_, session) = live_session(&state, &headers);
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
                        csrf: session.as_ref().map(|session| session.csrf.as_str()),
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
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
    if let Err(response) = mutation_session(&state, &ctx, &headers, &fields) {
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
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
        Ok(Some(proposal)) => {
            let approval = store.approval(&id).await.ok().flatten();
            let (_, session) = live_session(&state, &headers);
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
                        csrf: session.as_ref().map(|session| session.csrf.as_str()),
                        refusal: None,
                    },
                ),
            )
        }
        Ok(None) => page(
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
    if let Err(response) = workspace_ctx(&state, &ctx) {
        return *response;
    }
    let Some(store) = state.proposal_store.clone() else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            errors::not_configured(&ctx),
        );
    };
    match store.list().await {
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
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
    if let Err(response) = mutation_session(&state, &ctx, &headers, &fields) {
        return *response;
    }
    let decision = match fields.get("decision").map(String::as_str) {
        Some("approve") => crate::proposals::ApprovalDecision::Approved,
        Some("decline") => crate::proposals::ApprovalDecision::Declined,
        _ => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    let proposal = match store.get(&id).await {
        Ok(Some(proposal)) => proposal,
        Ok(None) => {
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
        let (_, session) = live_session(&state, &headers);
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
                    csrf: session.as_ref().map(|session| session.csrf.as_str()),
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
        approval_actor: crate::proposals::APPROVAL_ACTOR.to_string(),
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
            let (_, session) = live_session(&state, &headers);
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
                        csrf: session.as_ref().map(|session| session.csrf.as_str()),
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

async fn experiment_views(
    state: &AppState,
    publisher: &str,
) -> Result<Vec<crate::experiments::ExperimentView>, ApiError> {
    let records = state.ledger_source.records(publisher).await?;
    let alerts = match &state.alerts_path {
        Some(path) => crate::experiments::read_alerts(path)?,
        None => Vec::new(),
    };
    Ok(crate::experiments::project(
        &records,
        &alerts,
        now_unix_ms(state),
    ))
}

/// `GET /console/experiments` with the documented filters.
async fn console_experiments(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<ExperimentsParams>, QueryRejection>,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
    match experiment_views(&state, &publisher).await {
        Ok(views) => page(
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
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
    match experiment_views(&state, &publisher).await {
        Ok(views) => {
            let Some(view) = views.iter().find(|view| view.experiment_id == id) else {
                return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
            };
            let history = state
                .ledger_source
                .records(&publisher)
                .await
                .map(|records| crate::experiments::history(&records, &id))
                .unwrap_or_default();
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
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
        Err(response) => return *response,
    };
    let Ok(Path(id)) = id_path else {
        return page(StatusCode::NOT_FOUND, errors::not_found(&ctx));
    };
    match experiment_views(&state, &publisher).await {
        Ok(views) => {
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
    let records = state.ledger_source.records(publisher).await?;
    let payments = match &state.collection_log_path {
        Some(path) => crate::billing_view::read_collection_log(path)?,
        None => Vec::new(),
    };
    let monitoring = crate::billing_view::monitoring_from_env(&|key| crate::config::env_value(key));
    let currency = ConsoleCtx::from_env().currency;
    Ok(crate::billing_view::build(
        &records,
        payments,
        now_unix_ms(state),
        monitoring,
        &currency,
    ))
}

/// `GET /console/billing`.
async fn console_billing(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
    let workspace = state
        .workspace_publisher
        .clone()
        .unwrap_or_else(|| ctx.publisher_label.clone());
    let account = crate::settings::account_view(
        &|key| crate::config::env_value(key),
        &workspace,
        &ctx.currency,
    );
    let (_, session) = live_session(&state, &headers);
    page(
        StatusCode::OK,
        crate::html::settings::render_account(
            &ctx,
            &crate::html::settings::AccountPageData {
                account: &account,
                as_of_unix_ms: now_unix_ms(&state),
                theme: ctx.theme,
                csrf: session.as_ref().map(|session| session.csrf.as_str()),
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
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    if let Err(response) = mutation_session(&state, &ctx, &headers, &fields) {
        return *response;
    }
    let requested = fields
        .get("theme")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    if !matches!(requested.as_str(), "auto" | "light" | "dark") {
        let workspace = state
            .workspace_publisher
            .clone()
            .unwrap_or_else(|| ctx.publisher_label.clone());
        let account = crate::settings::account_view(
            &|key| crate::config::env_value(key),
            &workspace,
            &ctx.currency,
        );
        let (_, session) = live_session(&state, &headers);
        return page(
            StatusCode::BAD_REQUEST,
            crate::html::settings::render_account(
                &ctx,
                &crate::html::settings::AccountPageData {
                    account: &account,
                    as_of_unix_ms: now_unix_ms(&state),
                    theme: ctx.theme,
                    csrf: session.as_ref().map(|session| session.csrf.as_str()),
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
    let (_, session) = live_session(&state, &headers);
    let publisher = state
        .workspace_publisher
        .clone()
        .unwrap_or_else(|| ctx.publisher_label.clone());
    let sessions = match &session {
        Some(view) => state
            .ledger_sessions
            .summaries(now_unix_ms(&state))
            .into_iter()
            .filter(|summary| summary.publisher == view.publisher)
            .collect(),
        None => Vec::new(),
    };
    let mut response = page(
        StatusCode::OK,
        crate::html::settings::render_access(
            &ctx,
            &crate::html::settings::AccessPageData {
                publisher: &publisher,
                sessions,
                current_handle: session.as_ref().map(|view| view.handle.as_str()),
                csrf: session.as_ref().map(|view| view.csrf.as_str()),
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
    let fields = match leads::parse_form(&body) {
        Ok(fields) => fields,
        Err(_) => return page(StatusCode::BAD_REQUEST, errors::bad_request(&ctx, None)),
    };
    let session = match mutation_session(&state, &ctx, &headers, &fields) {
        Ok(session) => session,
        Err(response) => return *response,
    };
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PolicyFile {
    policy: Option<serde_json::Value>,
}

/// `GET /console/settings/policy`.
async fn console_settings_policy(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ctx = ctx_for_request(&headers);
    let publisher = state
        .workspace_publisher
        .clone()
        .unwrap_or_else(|| ctx.publisher_label.clone());
    let policy = crate::config::env_value(crate::settings::POLICY_FILE_ENV)
        .and_then(|path| read_policy_file(&path));
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

/// Read the recorded policy file (`APEX_POLICY_FILE`) as the runner's own
/// `AutopilotPolicy` JSON, or as `{"policy": {...}}`. Unreadable or invalid
/// files are logged and reported as absent.
fn read_policy_file(path: &str) -> Option<apex_autopilot::model::AutopilotPolicy> {
    let raw = std::fs::read_to_string(path).ok()?;
    if let Ok(policy) = serde_json::from_str::<apex_autopilot::model::AutopilotPolicy>(&raw) {
        return Some(policy);
    }
    match serde_json::from_str::<PolicyFile>(&raw) {
        Ok(wrapper) => wrapper
            .policy
            .and_then(|value| serde_json::from_value(value).ok()),
        Err(error) => {
            tracing::warn!(%error, "policy file is not valid AutopilotPolicy JSON");
            None
        }
    }
}

/// `GET /console/settings/monitoring`.
async fn console_settings_monitoring(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let ctx = ctx_for_request(&headers);
    let publisher = state
        .workspace_publisher
        .clone()
        .unwrap_or_else(|| ctx.publisher_label.clone());
    let plan = crate::billing_view::monitoring_from_env(&|key| crate::config::env_value(key));
    let meta = match connection_meta(&state, &publisher).await {
        Ok(meta) => meta,
        Err(error) => return connection_store_failure(&state, &ctx, error),
    };
    let mut snapshots = Vec::new();
    for capability in crate::connections::CAPABILITIES {
        let Some(platform) = parse_platform(capability.wire) else {
            continue;
        };
        if let Ok(snapshot) = state.audit_source.snapshot(platform).await {
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
                plan: &plan,
                rows,
                alert_log_configured: state.alerts_path.is_some(),
                as_of_unix_ms: now_unix_ms(&state),
            },
        ),
    )
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
    let publisher = match workspace_ctx(&state, &ctx) {
        Ok(publisher) => publisher,
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
    if let Some(platform) = params.platform.as_deref() {
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
    if let Some(platform) = params.platform.clone() {
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
    let lead = Lead {
        publisher: leads::publisher_slug(&request.studio),
        email: request.email.trim().to_string(),
        platforms: request.platforms.clone(),
        monthly_revenue_range: request.revenue.clone(),
        notes: request.notes.clone(),
        display_name: Some(request.studio.trim().to_string()),
        request_id: None,
    };
    match store.store(&lead, now_unix_ms(&state)).await {
        Ok(request_id) => {
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
    value
        .strip_prefix("req_")
        .is_some_and(|hex| hex.len() == 10 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
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
        let app = router(AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        ));
        // A hostile finding id carrying a quote and a backslash,
        // percent-encoded in the URL: `zz"qq\rr`. The route matches (so the
        // request is recorded), the handler answers 404, and the labels must
        // hold the template only.
        let status = get_status(&app, "/console/audit/findings/zz%22qq%5Crr").await;
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
        std::fs::write(dir.join("admob.json"), "{not a snapshot}").expect("snapshot file");
        let state = AppState::new(
            Arc::new(crate::sources::JsonFileAuditSource::new(&dir)),
            Arc::new(StaticLedgerSource::default()),
        );
        let app = router(state);
        assert_eq!(get_status(&app, "/healthz").await, StatusCode::OK);
        assert_eq!(get_status(&app, "/free-audit").await, StatusCode::SEE_OTHER);
        assert_eq!(
            get_status(&app, "/console/audit/findings/not-a-finding").await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get_status(&app, "/console/audit?platform=admob").await,
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
}
