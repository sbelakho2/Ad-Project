//! Billing error taxonomy.
//!
//! Every refusal is typed and names the offending ledger `seq` (or the
//! `experiment_id` for duplicate experiments) so an operator can go straight
//! to the record that caused it. Stripe transport errors are re-classified at
//! this boundary: auth failures stay auth failures (and are never retried),
//! rate limits/5xx are retried by the shared transport policy before they can
//! surface here.

use apex_connectors::transport::TransportError;
use apex_core::PlatformError;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BillingError {
    /// The proof's timestamp falls outside `[period.start, period.end)`.
    #[error("billing proof seq {seq} is outside the billing period")]
    OutOfPeriod { seq: u64 },

    /// The proof belongs to a different publisher than the invoice.
    #[error("billing proof seq {seq} belongs to a different publisher")]
    WrongPublisher { seq: u64 },

    /// Re-verification failed: the fee is never trusted from the record.
    #[error("billing proof seq {seq} failed re-verification: {reason}")]
    TamperedProof { seq: u64, reason: String },

    /// A `seq` or an `experiment_id` appeared twice: the same experiment must
    /// never be billed twice in one period.
    #[error("duplicate billing entry: seq {seq}, experiment {experiment_id}")]
    Duplicate { seq: u64, experiment_id: String },

    /// The checked sum of the line fees does not fit into `i64` micros.
    #[error("invoice total overflows i64 micros")]
    Overflow,

    /// Stripe amounts are integer minor units (cents): the fee must convert
    /// exactly (`fee_micros % 10_000 == 0`).
    #[error("invoice line seq {seq}: fee is not an exact cent amount (fee_micros % 10_000 != 0)")]
    SubCentAmount { seq: u64 },

    #[error("stripe authentication failed (HTTP {status})")]
    Auth { status: u16 },

    #[error("stripe request failed (HTTP {status})")]
    Http { status: u16 },

    #[error("stripe returned a malformed payload: {detail}")]
    Schema { detail: String },

    #[error("stripe transport failure: {detail}")]
    Network { detail: String },

    /// A transport-level failure with no Stripe-specific classification.
    #[error("transport error: {detail}")]
    Transport { detail: String },

    /// One or more items were created, then one failed. The idempotency keys
    /// are deterministic, so the caller resumes the same invoice safely.
    #[error(
        "stripe partial failure: {completed} item(s) created, seq {failed_seq} failed",
        completed = .completed.len()
    )]
    StripePartial {
        completed: Vec<String>,
        failed_seq: u64,
    },

    /// The signature header could not be parsed (or a ledger record was not a
    /// billing proof).
    #[error("malformed input: {0}")]
    Malformed(String),

    /// The `v1` HMAC did not match the signed payload.
    #[error("stripe signature verification failed")]
    InvalidSignature,

    /// `|now - t|` exceeded the caller's tolerance.
    #[error("stripe signature timestamp {timestamp} outside tolerance (now {now_unix_s})")]
    StaleTimestamp { timestamp: i64, now_unix_s: i64 },
}

impl From<PlatformError> for BillingError {
    fn from(error: PlatformError) -> Self {
        match error {
            PlatformError::Auth { status, .. } => Self::Auth { status },
            PlatformError::RateLimited { .. } => Self::Http { status: 429 },
            PlatformError::Http { status, .. } => {
                if status == 401 || status == 403 {
                    Self::Auth { status }
                } else {
                    Self::Http { status }
                }
            }
            PlatformError::Schema { detail, .. } => Self::Schema { detail },
            PlatformError::Network { detail, .. } => Self::Network { detail },
            PlatformError::Validation(detail) => Self::Transport { detail },
        }
    }
}

impl From<TransportError> for BillingError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Network(detail) => Self::Network { detail },
            TransportError::Timeout(detail) => Self::Network {
                detail: format!("timeout: {detail}"),
            },
            TransportError::Platform(inner) => inner.into(),
        }
    }
}
