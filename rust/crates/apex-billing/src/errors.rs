//! Billing error taxonomy.
//!
//! Every refusal is typed and names the offending ledger `seq` (or the
//! `experiment_id` for duplicate experiments) so an operator can go straight
//! to the record that caused it. Adyen transport errors are re-classified at
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

    /// Adyen amounts are integer minor units (cents): the fee must convert
    /// exactly (`fee_micros % 10_000 == 0`).
    #[error("invoice line seq {seq}: fee is not an exact cent amount (fee_micros % 10_000 != 0)")]
    SubCentAmount { seq: u64 },

    #[error("upstream authentication failed (HTTP {status})")]
    Auth { status: u16 },

    #[error("upstream request failed (HTTP {status})")]
    Http { status: u16 },

    #[error("upstream returned a malformed payload: {detail}")]
    Schema { detail: String },

    #[error("upstream transport failure: {detail}")]
    Network { detail: String },

    /// A transport-level failure with no upstream-specific classification.
    #[error("transport error: {detail}")]
    Transport { detail: String },

    /// One or more items were created, then one failed. The idempotency keys
    /// are deterministic, so the caller resumes the same invoice safely; the
    /// `cause` names why the run stopped (Display of the underlying error).
    #[error(
        "partial collection: {completed} item(s) created, seq {failed_seq} failed: {cause}",
        completed = .completed.len()
    )]
    PartialCollection {
        completed: Vec<String>,
        failed_seq: u64,
        cause: String,
    },

    /// An identifier cannot be placed in an Adyen reference: it contains a
    /// control character (including CR/LF) or the derived reference exceeds
    /// the length cap. Refused before any request is sent.
    #[error("invoice line seq {seq} has an identifier unusable in an Adyen reference: {detail}")]
    UnsafeIdentifier { seq: u64, detail: String },

    /// The signature header could not be parsed (or a ledger record was not a
    /// billing proof).
    #[error("malformed input: {0}")]
    Malformed(String),

    /// The HMAC digest did not match the signed payload.
    #[error("adyen signature verification failed")]
    InvalidSignature,
}

impl From<std::io::Error> for BillingError {
    fn from(error: std::io::Error) -> Self {
        Self::Malformed(format!("storage i/o: {error}"))
    }
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
