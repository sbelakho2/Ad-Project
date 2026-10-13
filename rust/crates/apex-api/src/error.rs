//! The API error taxonomy and its HTTP rendering.
//!
//! Every variant maps to exactly one status and one small JSON body. Internal
//! detail is logged (never serialized) so responses cannot leak file paths,
//! parser messages or secret material; the Stripe webhook collapses its whole
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
    /// `platform_required`, `invalid_cycle_request`, `invalid_policy`).
    #[error("bad request: {0}")]
    BadRequest(&'static str),

    /// The named resource has no backing file or key.
    #[error("not found")]
    NotFound,

    /// The ledger exists but does not verify; `line` is the 1-based first
    /// offending line, as reported by `apex_proof::Ledger::verify`.
    #[error("ledger verification failed at line {line}")]
    LedgerUnverified { line: usize },

    /// No Stripe webhook secret is configured in this deployment.
    #[error("stripe webhook is not configured")]
    WebhookNotConfigured,

    /// The Stripe signature did not pass verification. The response never says
    /// which check failed.
    #[error("invalid stripe signature")]
    InvalidSignature,

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
            ApiError::WebhookNotConfigured => "webhook_not_configured",
            ApiError::InvalidSignature => "invalid_signature",
            ApiError::Internal(_) => "internal",
        }
    }

    /// The HTTP status this error renders as.
    pub fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) | ApiError::InvalidSignature => StatusCode::BAD_REQUEST,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::LedgerUnverified { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::WebhookNotConfigured => StatusCode::SERVICE_UNAVAILABLE,
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
            ApiError::WebhookNotConfigured => json!({ "error": "webhook_not_configured" }),
            ApiError::InvalidSignature => json!({ "error": "invalid_signature" }),
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
                ApiError::WebhookNotConfigured,
                503,
                "webhook_not_configured",
            ),
            (ApiError::InvalidSignature, 400, "invalid_signature"),
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
