//! The API error taxonomy and its HTTP rendering.
//!
//! Every variant maps to exactly one status and one small JSON body. Internal
//! detail is logged (never serialized) so responses cannot leak file paths,
//! parser messages or secret material; the Adyen webhook collapses its whole
//! verification taxonomy to the single `invalid_signature` code on purpose.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// A refusal produced by the API or one of its sources.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// A request the server refuses to interpret: the payload names the
    /// machine-readable reason (`unknown_platform`, `invalid_publisher`,
    /// `platform_required`, `invalid_cycle_request`, `invalid_policy`,
    /// `invalid_credentials`, `platform_not_onboarded`, and `bad_request` for
    /// a malformed path/query or an oversized body).
    #[error("bad request: {0}")]
    BadRequest(&'static str),

    /// The named resource has no backing file or key.
    #[error("not found")]
    NotFound,

    /// The ledger exists but does not verify; `line` is the 1-based first
    /// offending line, as reported by `apex_proof::Ledger::verify`.
    #[error("ledger verification failed at line {line}")]
    LedgerUnverified { line: usize },

    /// No Adyen webhook HMAC key is configured in this deployment.
    #[error("adyen webhook is not configured")]
    AdyenWebhookNotConfigured,

    /// No live-audit account store is configured in this deployment.
    #[error("live audit is not configured")]
    LiveAuditNotConfigured,

    /// The live-audit or ledger bearer token is missing or wrong.
    #[error("unauthorized")]
    Unauthorized,

    /// No per-publisher ledger token is configured: the ledger is closed
    /// rather than served without authentication.
    #[error("ledger authentication is not configured")]
    LedgerAuthNotConfigured,

    /// No lead store is configured in this deployment.
    #[error("lead capture is not configured")]
    LeadCaptureNotConfigured,

    /// No collection store is configured: the webhook refuses rather than
    /// accepting an event it cannot record.
    #[error("collection store is not configured")]
    CollectionStoreNotConfigured,

    /// An upstream platform refused or failed the call; the detail is logged
    /// but not exposed.
    #[error("platform unavailable: {0}")]
    PlatformUnavailable(String),

    /// The Adyen webhook signature did not pass verification. The response
    /// never says which check failed.
    #[error("invalid webhook signature")]
    InvalidSignature,

    /// A request conflicts with the current state: another writer owns the
    /// resource this call tried to publish. The code names the conflict.
    #[error("conflict: {0}")]
    Conflict(&'static str),

    /// The public console origin is not configured, so no sign-in link can
    /// be issued: a relative emailed link cannot work.
    #[error("sign-in link base is not configured")]
    MissingBase,

    /// An unexpected failure while reading a source; the detail is logged but
    /// not exposed.
    #[error("internal error: {0}")]
    Internal(String),
}

impl ApiError {
    /// The machine-readable `error` code carried in the JSON body.
    pub fn code(&self) -> &'static str {
        match self {
            ApiError::BadRequest(code) => code,
            ApiError::NotFound => "not_found",
            ApiError::LedgerUnverified { .. } => "ledger_unverified",
            ApiError::AdyenWebhookNotConfigured => "adyen_webhook_not_configured",
            ApiError::LiveAuditNotConfigured => "live_audit_not_configured",
            ApiError::Unauthorized => "unauthorized",
            ApiError::LedgerAuthNotConfigured => "ledger_auth_not_configured",
            ApiError::LeadCaptureNotConfigured => "lead_capture_not_configured",
            ApiError::CollectionStoreNotConfigured => "collection_store_not_configured",
            ApiError::PlatformUnavailable(_) => "platform_unavailable",
            ApiError::InvalidSignature => "invalid_signature",
            ApiError::Conflict(code) => code,
            ApiError::MissingBase => "missing_base",
            ApiError::Internal(_) => "internal",
        }
    }

    /// The HTTP status this error renders as.
    pub fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) | ApiError::InvalidSignature => StatusCode::BAD_REQUEST,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::LedgerUnverified { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::AdyenWebhookNotConfigured
            | ApiError::LiveAuditNotConfigured
            | ApiError::LedgerAuthNotConfigured
            | ApiError::LeadCaptureNotConfigured
            | ApiError::CollectionStoreNotConfigured
            | ApiError::MissingBase => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::PlatformUnavailable(_) => StatusCode::BAD_GATEWAY,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = match &self {
            ApiError::BadRequest(code) => {
                tracing::debug!(code, "request refused");
                json!({ "error": code })
            }
            ApiError::NotFound => json!({ "error": "not_found" }),
            ApiError::LedgerUnverified { line } => {
                json!({ "error": "ledger_unverified", "line": line })
            }
            ApiError::AdyenWebhookNotConfigured => {
                json!({ "error": "adyen_webhook_not_configured" })
            }
            ApiError::LiveAuditNotConfigured => json!({ "error": "live_audit_not_configured" }),
            ApiError::LedgerAuthNotConfigured => json!({ "error": "ledger_auth_not_configured" }),
            ApiError::LeadCaptureNotConfigured => json!({ "error": "lead_capture_not_configured" }),
            ApiError::CollectionStoreNotConfigured => {
                json!({ "error": "collection_store_not_configured" })
            }
            ApiError::Unauthorized => json!({ "error": "unauthorized" }),
            ApiError::InvalidSignature => json!({ "error": "invalid_signature" }),
            ApiError::Conflict(code) => json!({ "error": code }),
            ApiError::MissingBase => json!({ "error": "missing_base" }),
            ApiError::PlatformUnavailable(detail) => {
                tracing::error!(error = %detail, "platform call failed");
                json!({ "error": "platform_unavailable" })
            }
            ApiError::Internal(detail) => {
                tracing::error!(error = %detail, "internal api error");
                json!({ "error": "internal" })
            }
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    async fn render(error: ApiError) -> (StatusCode, serde_json::Value) {
        let response = error.into_response();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let value = serde_json::from_slice(&bytes).expect("json body");
        (status, value)
    }

    #[tokio::test]
    async fn every_variant_maps_to_its_pinned_status_and_code() {
        let cases = [
            (
                ApiError::BadRequest("unknown_platform"),
                400,
                "unknown_platform",
            ),
            (ApiError::NotFound, 404, "not_found"),
            (
                ApiError::LedgerUnverified { line: 7 },
                422,
                "ledger_unverified",
            ),
            (
                ApiError::AdyenWebhookNotConfigured,
                503,
                "adyen_webhook_not_configured",
            ),
            (
                ApiError::LiveAuditNotConfigured,
                503,
                "live_audit_not_configured",
            ),
            (
                ApiError::PlatformUnavailable("AdMob listMediationGroups: HTTP 500".into()),
                502,
                "platform_unavailable",
            ),
            (ApiError::Unauthorized, 401, "unauthorized"),
            (ApiError::InvalidSignature, 400, "invalid_signature"),
            (
                ApiError::Conflict("credential_write_conflict"),
                409,
                "credential_write_conflict",
            ),
            (ApiError::MissingBase, 503, "missing_base"),
            (ApiError::Internal("/etc/secret".into()), 500, "internal"),
        ];
        for (error, status, code) in cases {
            let (rendered_status, value) = render(error).await;
            assert_eq!(rendered_status.as_u16(), status);
            assert_eq!(value["error"], code);
        }
    }

    #[tokio::test]
    async fn unverified_ledger_body_names_the_line() {
        let (_, value) = render(ApiError::LedgerUnverified { line: 3 }).await;
        assert_eq!(value, json!({ "error": "ledger_unverified", "line": 3 }));
    }

    #[tokio::test]
    async fn platform_unavailable_never_serializes_its_detail() {
        let (status, value) = render(ApiError::PlatformUnavailable(
            "AdMob listMediationGroups: HTTP 500 for key sk_live_abc".into(),
        ))
        .await;
        assert_eq!(status.as_u16(), 502);
        let rendered = value.to_string();
        assert!(!rendered.contains("sk_live"), "{rendered}");
        assert!(!rendered.contains("500"), "{rendered}");
        assert_eq!(value, json!({ "error": "platform_unavailable" }));
    }

    #[tokio::test]
    async fn internal_errors_never_serialize_their_detail() {
        let (_, value) = render(ApiError::Internal(
            "read /nested/root/secret-key.pem: permission denied".into(),
        ))
        .await;
        let rendered = value.to_string();
        assert!(!rendered.contains("secret-key"), "{rendered}");
        assert!(!rendered.contains("permission denied"), "{rendered}");
        assert_eq!(value, json!({ "error": "internal" }));
    }
}
