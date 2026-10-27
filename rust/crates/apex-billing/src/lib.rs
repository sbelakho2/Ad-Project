//! Apex billing: proven-lift fees through Mollie.
//!
//! - [`mollie`]: checkout payments, first/recurring payments and the server-side
//!   webhook fetch (Mollie webhooks are unsigned). Mollie is spoken over the
//!   shared [`apex_connectors::transport`] stack, so retries, timeout budgets
//!   and redacted logging are identical to the platform connectors; every
//!   billing call carries the
//!   [`PlatformId::Billing`](apex_core::PlatformId::Billing) domain label.
//! - [`subscription`]: the flat plan line — managed ad ops — with its
//!   persisted cancel state. Monitoring and the app-ads.txt watch are free and
//!   have no plan line.
//! - [`webhook`]: the billing-boundary verification contract used by the API
//!   layer.
//! - [`invoice`]: the deterministic monthly invoice derived from verified
//!   billing proofs (`compute_invoice`), its VAT treatment, and the
//!   renderer that refuses to print anything short of a lawful VAT document.
//! - [`profile`]: the seller's billing profile (deployment config) and the
//!   customer's legal identity (carried on the invoice).
//! - [`vat`]: the in-crate EU membership table, the VAT-number shape check,
//!   the VIES register seam (production calls VIES over TLS; tests inject a
//!   stub), and the per-customer treatment rules.
//! - [`register`]: the append-only invoice register and its gapless
//!   sequential numbering.
//!
//! Design invariants:
//!
//! - **Fail closed:** malformed Mollie responses and malformed webhook
//!   deliveries are typed refusals, never silent skips.
//! - **Durable intents, then the provider:** every invoice line is recorded
//!   in Apex's own [`PaymentIntentStore`] (fsynced) before any Mollie call,
//!   then transitions `issued -> submittedPending/unknown -> paid/failed/
//!   refunded` with the provider payment id persisted after the response. A
//!   deterministic `apex-fee:<publisher>:<experiment>:<period>` reference still
//!   travels as Mollie's `Idempotency-Key`, but that key is only the
//!   provider's documented **one-hour** deduplication window, never an
//!   exactly-once guarantee. Recovery reads the durable intent or fetches the
//!   durable provider payment id; it never blindly creates a second charge.
//! - **No rounding:** a fee that is not a whole number of cents is refused
//!   (`BillingError::SubCentAmount`).
//! - **Lawful VAT invoices:** an invoice is only rendered (and only issued
//!   into the register) once the seller profile is complete, the customer
//!   profile is carried on it, a VAT treatment has been derived from the
//!   tested EU table, and its number has been assigned gaplessly. Every
//!   missing piece is a typed error naming the field — a half-empty invoice
//!   is never printed.
//!
//! # Configuration (one shape)
//!
//! The seller profile is a JSON file named by the environment variable
//! `APEX_BILLING_PROFILE` (`legalName`, `addressLines`, `country`,
//! `vatNumber`, `vatRateBp`); `APEX_BILLING_VAT_RATE_BP` overrides the
//! domestic rate. Load it at startup with [`BillingProfile::from_env`] and
//! refuse to run when it errors. The full contract, including the VIES
//! reverse-charge confirmation and evidence, is documented on [`vat`].
//! Managed ad ops is the only flat plan after A3; monitoring is free.
//!
//! # Console / downstream consumers
//!
//! Everything a settings page needs to display is typed and serializable:
//! [`BillingProfile`] (the configured seller identity and domestic rate),
//! [`CustomerProfile`] (the invoice's carried customer), [`VatTreatment`]
//! (the decided treatment), [`Invoice::number`] (the register number), and
//! the flat-plan state ([`PlanState`], [`MANAGED_SCOPE`]). The rendered document itself is
//! [`Invoice::render_vat_invoice`], which returns a typed refusal rather than
//! a partial document when anything it needs is missing.

pub mod collection;
pub mod errors;
pub mod intent;
pub mod invoice;
pub mod mollie;
pub mod profile;
pub mod register;
pub mod subscription;
pub mod vat;
pub mod webhook;

pub use collection::{
    stored_mandate_for_publisher, CollectionEvent, CollectionStore, FileCollectionStore,
};
pub use errors::BillingError;
pub use intent::{
    collect_invoice_with_intents, PaymentIntent, PaymentIntentChannel, PaymentIntentState,
    PaymentIntentStore, DEFAULT_INTENT_PATH, IDEMPOTENCY_WINDOW_MS,
};
pub use invoice::{
    compute_invoice, compute_invoice_from_basis, round_to_cent, BillingBasisRecord, BillingPeriod,
    BillingProofRecord, Invoice, InvoiceLine,
};
pub use mollie::{
    collect_invoice, collect_line, create_payment_links, format_amount, parse_amount,
    BillingReceipt, CollectionChannel, CollectionReceipt, InvoiceLineItem, LineCollection,
    MollieClient, MollieClientError, MolliePayment, PaymentLinkResponse, PaymentReceipt,
    StoredMandate, BILLING_CURRENCY, BILLING_PLATFORM, COLLECTED_PAYMENT_STATUS, CURRENCY_EXPONENT,
    DEFAULT_MOLLIE_API_BASE, KNOWN_PAYMENT_STATUSES, MAX_REFERENCE_LEN, MICROS_PER_MINOR_UNIT,
};
pub use profile::{BillingProfile, CustomerProfile, PROFILE_ENV, VAT_RATE_ENV};
pub use register::{InvoiceRegister, DEFAULT_REGISTER_PATH};
pub use subscription::{
    ManagedPlan, PlanId, PlanLine, PlanState, MANAGED_BASE_FEE_MICROS, MANAGED_SCOPE,
    MANAGED_TOP_ABOVE_REVENUE_MICROS, MANAGED_TOP_FEE_MICROS, PLAN_CURRENCY,
};
pub use vat::{
    is_eu_member, is_valid_vat_number, treatment_for, treatment_for_shape, treatment_for_with_vies,
    CachedVies, HttpViesClient, VatDecision, VatTreatment, ViesCheck, ViesClient, ViesError,
    ViesEvidence, ViesOutcome, EU_MEMBER_STATES, MAX_VAT_RATE_BP, VIES_ENDPOINT,
    VIES_MAX_EVIDENCE_CHARS, VIES_TIMEOUT_SECS,
};
pub use webhook::{fetch_webhook_payment, parse_webhook_id, validate_payment};

#[cfg(test)]
mod tests;
