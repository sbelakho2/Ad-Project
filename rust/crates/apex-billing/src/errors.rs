//! Billing error taxonomy.
//!
//! Every refusal is typed and names the offending ledger `seq` (or the
//! `experiment_id` for duplicate experiments) so an operator can go straight
//! to the record that caused it. Mollie transport errors are re-classified at
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

    /// Mollie amounts are decimal strings with two places: the fee must
    /// convert exactly (`fee_micros % 10_000 == 0`).
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

    /// An identifier cannot be placed in a billing reference: it contains a
    /// control character (including CR/LF) or the derived reference exceeds
    /// the length cap. Refused before any request is sent.
    #[error("invoice line seq {seq} has an identifier unusable in a billing reference: {detail}")]
    UnsafeIdentifier { seq: u64, detail: String },

    /// A required VAT field is missing or empty on a party's profile (or the
    /// config entry naming where the seller profile lives). `party` is
    /// `"seller"` or `"customer"`; `field` names the field — for example
    /// `legalName`, `addressLines`, `country`, `vatNumber`, `vatRateBp` — or
    /// the environment variable `APEX_BILLING_PROFILE`. A half-empty invoice
    /// is never rendered.
    #[error("incomplete {party} billing profile: missing {field}")]
    MissingProfileField {
        party: &'static str,
        field: &'static str,
    },

    /// The invoice carries no customer profile, so no VAT treatment can be
    /// stated lawfully.
    #[error("the invoice carries no customer profile, so no VAT treatment can be stated")]
    MissingCustomerProfile,

    /// A VAT rate (from the profile JSON or `APEX_BILLING_VAT_RATE_BP`) is
    /// outside the allowed `0..=10000` basis points.
    #[error("invalid VAT rate: {detail}")]
    InvalidVatRate { detail: String },

    /// A VAT number fails the minimal shape check (non-empty, ASCII
    /// alphanumeric, 4..=14 characters).
    #[error("invalid VAT number for the {party}: {detail}")]
    InvalidVatNumber { party: &'static str, detail: String },

    /// The EU VIES register could not confirm a customer VAT number, so the
    /// reverse-charge treatment is refused (a typed hold, never a silent
    /// domestic charge and never a silently stated reverse charge). The
    /// evidence carries the register's bounded reason.
    #[error("VIES could not confirm VAT number {country}{vat_number}: {evidence}")]
    ViesUnavailable {
        country: String,
        vat_number: String,
        evidence: String,
    },

    /// A country code is not exactly two ASCII letters.
    #[error("invalid country code for the {party}: {value:?}")]
    InvalidCountryCode { party: &'static str, value: String },

    /// No VAT treatment has been applied; the renderer refuses to guess one.
    #[error("no VAT treatment has been applied to this invoice")]
    VatTreatmentNotApplied,

    /// The stored VAT figures disagree with the treatment and profiles they
    /// would be rendered from.
    #[error("invoice figures do not match the applied VAT treatment: {detail}")]
    InconsistentVat { detail: String },

    /// A lawful VAT invoice needs its gapless register number first.
    #[error("invoice has no register number: issue it through the invoice register first")]
    UnregisteredInvoice,

    /// The invoice register is unreadable or not a register: a line that is
    /// not an invoice, carries no positive number, or repeats a number.
    #[error("invoice register is unusable: {detail}")]
    RegisterCorrupt { detail: String },

    /// A flat plan's persisted state file is unreadable or malformed. The
    /// console and the monthly pass refuse rather than fabricating a plan.
    #[error("plan state is unusable: {detail}")]
    PlanStateCorrupt { detail: String },

    /// A stored-mandate (recurring) charge was refused by the provider.
    /// The typed fallback path creates a checkout payment instead; the refusal
    /// is surfaced in the collection line, never silently skipped. A provider
    /// payment id is carried when the refusal response included one.
    #[error("stored mandate charge was refused: {detail}")]
    StoredChargeRefused {
        detail: String,
        payment_id: Option<String>,
    },

    /// An invoice was already issued to this publisher for this billing
    /// period. One invoice per `(publisher, billing period)` is enforced
    /// atomically at issuance.
    #[error(
        "publisher {publisher_id} already has invoice {number} for billing period {year_month}"
    )]
    InvoicePeriodAlreadyIssued {
        publisher_id: String,
        year_month: String,
        number: u64,
    },

    /// The durable payment-intent journal is unreadable or corrupt.
    #[error("payment intent journal is unusable: {detail}")]
    PaymentIntentStoreCorrupt { detail: String },

    /// A payment-intent transition conflicts with the durable intent (for
    /// example a different amount for the same intent id, or a second
    /// provider payment id where one is already recorded).
    #[error("payment intent conflict: {detail}")]
    PaymentIntentConflict { detail: String },

    /// A provider submission's outcome is not durable (no payment id) and the
    /// documented idempotency window has closed. Recovery refuses rather than
    /// creating a second charge.
    #[error("payment intent {intent_id} is unresolved; refuse to create a second charge")]
    PaymentIntentUncertain { intent_id: String },

    /// A webhook or ledger record could not be validated.
    #[error("malformed input: {0}")]
    Malformed(String),

    /// A Mollie webhook named a payment that does not exist or does not
    /// belong to this deployment. The message never echoes the id.
    #[error("webhook rejected: {detail}")]
    WebhookRejected { detail: String },
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
