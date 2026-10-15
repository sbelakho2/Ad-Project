//! The axum 0.8 router: JSON API plus the server-rendered console.
//!
//! Wire contract (all JSON responses are `application/json`):
//!
//! | route | success | refusals |
//! |---|---|---|
//! | `GET /healthz` | `200 {"status":"ok"}` | — |
//! | `GET /v1/audit/{platform}` | `200` `AuditReport` | `400 unknown_platform / bad_request`, `404 not_found` |
//! | `GET /v1/ledger/{publisher}` | `200 {"publisher","mode":"read_only","records"}` | `400 invalid_publisher / bad_request`, `401 unauthorized`, `404 not_found`, `422 {"error":"ledger_unverified","line":n}`, `503 ledger_auth_not_configured` |
//! | `POST /v1/cycle/{publisher}?platform=` | `200` read-only summary | `400 platform_required / unknown_platform / invalid_publisher / invalid_cycle_request / invalid_policy / bad_request`, `404`, `422` |
//! | `POST /v1/adyen/webhook` | `200 {"received":true,"items":n}` | `400 invalid_signature` or `malformed_event`, `503 adyen_webhook_not_configured` |
//! | `POST /v1/leads` | `202 {"received":true}` | `400 invalid_lead`, `503 lead_capture_not_configured` |
//!
//! Unknown paths fall through to `404 {"error":"not_found"}`; a known path
//! with the wrong method is axum's `405` (asserted by tests).
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
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tower_http::trace::TraceLayer;

use crate::cycle::{CycleRequest, CycleSummary, READ_ONLY_MODE};
use crate::error::ApiError;
use crate::html::{
    render_audit_form, render_audit_page, render_error_page, render_ledger_form,
    render_ledger_records, render_overview,
};
use crate::sources::{is_valid_publisher, parse_platform};
use crate::state::AppState;

/// Hard cap on every request body, enforced by [`DefaultBodyLimit`] and by the
/// pre-routing `Content-Length` check below. 256 KiB is far above the largest
/// legitimate body (the optional cycle policy JSON) and bounds the memory a
/// single request can make the server buffer.
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// Build the complete router (API + console) over `state`.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(crate::metrics::metrics))
        .route("/v1/audit/{platform}", get(audit_report).post(live_audit))
        .route("/v1/ledger/{publisher}", get(ledger))
        .route("/v1/cycle/{publisher}", post(run_cycle))
        .route("/v1/adyen/webhook", post(adyen_webhook))
        .route("/v1/leads", post(lead_capture))
        .route("/", get(console_overview))
        .route("/console/audit", get(console_audit))
        .route("/console/ledger", get(console_ledger))
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
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            Html(render_error_page(
                "bad request",
                &format!(
                    "request body exceeds the deployment's {} KiB limit",
                    MAX_BODY_BYTES / 1024
                ),
            )),
        )
            .into_response()
    }
}

async fn not_found() -> ApiError {
    ApiError::NotFound
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
    Ok(Json(auditor.audit(&body.publisher, platform).await?))
}

/// Capture a free-audit lead: validate the shape, store it append-only, and
/// forward it to the mail webhook when one is configured.
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
    let lead = crate::leads::parse_lead(&body, content_type)
        .map_err(|_| ApiError::BadRequest("invalid_lead"))?;
    crate::leads::validate_lead(&lead).map_err(|_| ApiError::BadRequest("invalid_lead"))?;
    let now_unix_ms = state.clock.now_unix_s().saturating_mul(1000);
    store.store(&lead, now_unix_ms).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "received": true }))))
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
    let records = state.ledger_source.records(&publisher).await?;
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
    for item in &items {
        tracing::info!(
            event_code = %item.event_code,
            psp_reference = %item.psp_reference,
            success = %item.success,
            "adyen webhook item accepted"
        );
    }
    Ok(Json(json!({ "received": true, "items": items.len() })))
}

async fn console_overview() -> Html<String> {
    Html(render_overview())
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
    let Ok(Query(params)) = params else {
        return page(
            StatusCode::BAD_REQUEST,
            render_error_page(
                "bad request",
                "the request parameters could not be interpreted",
            ),
        );
    };
    let Some(platform_name) = params.platform else {
        return page(StatusCode::OK, render_audit_form(None, None));
    };
    let Some(platform) = parse_platform(&platform_name) else {
        let problem = format!("unknown platform \"{platform_name}\"");
        return page(
            StatusCode::BAD_REQUEST,
            render_audit_form(None, Some(&problem)),
        );
    };
    match state.audit_source.snapshot(platform).await {
        Ok(snapshot) => {
            let report = apex_audit::run(&snapshot);
            page(StatusCode::OK, render_audit_page(platform, &report))
        }
        Err(ApiError::NotFound) => page(
            StatusCode::NOT_FOUND,
            render_audit_form(
                Some(platform.as_str()),
                Some("no snapshot file exists for this platform in this deployment"),
            ),
        ),
        Err(error) => {
            tracing::error!(%error, "audit console failed");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                render_error_page("audit error", "the snapshot could not be read"),
            )
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct LedgerConsoleParams {
    #[serde(default)]
    publisher: Option<String>,
    /// The publisher's ledger token, submitted by the console form.
    #[serde(default)]
    token: Option<String>,
}

async fn console_ledger(
    State(state): State<AppState>,
    params: Result<Query<LedgerConsoleParams>, QueryRejection>,
) -> Response {
    let Ok(Query(params)) = params else {
        return page(
            StatusCode::BAD_REQUEST,
            render_error_page(
                "bad request",
                "the request parameters could not be interpreted",
            ),
        );
    };
    let Some(publisher) = params.publisher else {
        return page(StatusCode::OK, render_ledger_form(None, None));
    };
    if !is_valid_publisher(&publisher) {
        return page(
            StatusCode::BAD_REQUEST,
            render_ledger_form(
                Some(&publisher),
                Some("invalid publisher name (allowed: A-Z a-z 0-9 _ -, 1..=64 characters)"),
            ),
        );
    }
    let authorization = params
        .token
        .as_deref()
        .map(|token| format!("Bearer {token}"));
    match state.authorize_ledger(&publisher, authorization.as_deref()) {
        Ok(()) => {}
        Err(ApiError::LedgerAuthNotConfigured) => {
            return page(
                StatusCode::SERVICE_UNAVAILABLE,
                render_error_page(
                    "ledger closed",
                    "ledger authentication is not configured for this deployment",
                ),
            );
        }
        Err(_) => {
            return page(
                StatusCode::UNAUTHORIZED,
                render_ledger_form(
                    Some(&publisher),
                    Some("enter the publisher's ledger token to read the ledger"),
                ),
            );
        }
    }
    match state.ledger_source.records(&publisher).await {
        Ok(records) => page(StatusCode::OK, render_ledger_records(&publisher, &records)),
        Err(ApiError::LedgerUnverified { line }) => page(
            StatusCode::UNPROCESSABLE_ENTITY,
            render_ledger_form(
                Some(&publisher),
                Some(&format!(
                    "ledger verification failed at line {line}; no record is served from an \
                     unverified ledger"
                )),
            ),
        ),
        Err(ApiError::NotFound) => page(
            StatusCode::NOT_FOUND,
            render_ledger_form(
                Some(&publisher),
                Some("no ledger exists for this publisher in this deployment"),
            ),
        ),
        Err(error) => {
            tracing::error!(%error, "ledger console failed");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                render_error_page("ledger error", "the ledger could not be read"),
            )
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
}
