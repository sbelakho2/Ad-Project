//! Error taxonomy shared by every connector and the autopilot.
//! Retry classification is a pinned contract (see `is_retryable` tests): the autopilot
//! only ever retries what this function allows, and writes additionally require an
//! idempotency key at the call site.

use std::time::Duration;

use crate::platform::PlatformId;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{platform} {operation}: auth failed (HTTP {status})")]
    Auth {
        platform: PlatformId,
        operation: String,
        status: u16,
    },

    #[error("{platform} {operation}: rate limited")]
    RateLimited {
        platform: PlatformId,
        operation: String,
        retry_after: Option<Duration>,
    },

    #[error("{platform} {operation}: HTTP {status}")]
    Http {
        platform: PlatformId,
        operation: String,
        status: u16,
    },

    #[error("{platform} {operation}: malformed response: {detail}")]
    Schema {
        platform: PlatformId,
        operation: String,
        detail: String,
    },

    #[error("{platform} {operation}: network error: {detail}")]
    Network {
        platform: PlatformId,
        operation: String,
        detail: String,
    },

    #[error("{platform} {operation}: unsupported: {detail}")]
    Unsupported {
        platform: PlatformId,
        operation: String,
        detail: String,
    },

    #[error("validation error: {0}")]
    Validation(String),
}

impl PlatformError {
    /// Fail-closed retry policy:
    /// - never retry auth (the key is wrong; retrying burns quota),
    /// - never retry schema/validation (deterministic),
    /// - retry rate limits, 5xx and network failures (bounded by the transport),
    /// - never retry other 4xx (the request itself is the problem).
    pub fn is_retryable(&self) -> bool {
        match self {
            PlatformError::RateLimited { .. } | PlatformError::Network { .. } => true,
            PlatformError::Http { status, .. } => (500..600).contains(status),
            PlatformError::Auth { .. }
            | PlatformError::Schema { .. }
            | PlatformError::Unsupported { .. }
            | PlatformError::Validation(_) => false,
        }
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            PlatformError::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> PlatformId {
        PlatformId::Admob
    }

    #[test]
    fn retry_classification_is_pinned() {
        let cases: Vec<(PlatformError, bool)> = vec![
            (
                PlatformError::Auth {
                    platform: p(),
                    operation: "op".into(),
                    status: 401,
                },
                false,
            ),
            (
                PlatformError::RateLimited {
                    platform: p(),
                    operation: "op".into(),
                    retry_after: Some(Duration::from_secs(3)),
                },
                true,
            ),
            (
                PlatformError::Http {
                    platform: p(),
                    operation: "op".into(),
                    status: 503,
                },
                true,
            ),
            (
                PlatformError::Http {
                    platform: p(),
                    operation: "op".into(),
                    status: 404,
                },
                false,
            ),
            (
                PlatformError::Schema {
                    platform: p(),
                    operation: "op".into(),
                    detail: "x".into(),
                },
                false,
            ),
            (
                PlatformError::Network {
                    platform: p(),
                    operation: "op".into(),
                    detail: "reset".into(),
                },
                true,
            ),
            (PlatformError::Validation("bad".into()), false),
        ];
        for (err, expected) in cases {
            assert_eq!(err.is_retryable(), expected, "case: {err}");
        }
    }

    #[test]
    fn retry_after_only_exposed_for_rate_limits() {
        let rl = PlatformError::RateLimited {
            platform: p(),
            operation: "o".into(),
            retry_after: Some(Duration::from_secs(2)),
        };
        assert_eq!(rl.retry_after(), Some(Duration::from_secs(2)));
        let http = PlatformError::Http {
            platform: p(),
            operation: "o".into(),
            status: 500,
        };
        assert_eq!(http.retry_after(), None);
    }
}
