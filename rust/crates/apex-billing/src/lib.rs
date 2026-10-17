//! Apex billing: proven-lift fees through Adyen.
//!
//! - [`adyen`]: Checkout payment links and the card-on-file token charge for
//!   monthly collection, plus the documented webhook HMAC verification. Adyen
//!   is spoken over the shared [`apex_connectors::transport`] stack, so
//!   retries, timeout budgets and redacted logging are identical to the
//!   platform connectors; every billing call carries the
//!   [`PlatformId::Billing`](apex_core::PlatformId::Billing) domain label.
//! - [`subscription`]: the flat plan lines — monitoring (tiered by reported
//!   30-day revenue, on at connection, cancel any time), managed ad ops, and
//!   the frozen setup-watch interface — with their persisted cancel state.
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
//! - **Fail closed:** malformed Adyen responses and malformed signature
//!   headers are typed refusals, never silent skips.
//! - **Idempotent billing:** every payment link and token charge carries the
//!   deterministic reference `apex-fee:<publisher>:<experiment>:<period>`,
//!   which Adyen deduplicates, so a resumed month cannot double bill.
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
//! Monitoring's tier prices and legacy fallback are documented on
//! [`subscription`].
//!
//! # Console / downstream consumers
//!
//! Everything a settings page needs to display is typed and serializable:
//! [`BillingProfile`] (the configured seller identity and domestic rate),
//! [`CustomerProfile`] (the invoice's carried customer), [`VatTreatment`]
//! (the decided treatment), [`Invoice::number`] (the register number), and
//! the flat-plan state ([`PlanState`], [`MonitoringTier`],
//! [`MANAGED_SCOPE`]). The rendered document itself is
//! [`Invoice::render_vat_invoice`], which returns a typed refusal rather than
//! a partial document when anything it needs is missing.

pub mod adyen;
pub mod collection;
pub mod errors;
pub mod invoice;
pub mod profile;
pub mod register;
pub mod subscription;
pub mod vat;
pub mod webhook;

pub use adyen::{
    collect_invoice, collect_line, create_payment_links, parse_notification_items,
    parse_stored_payment_method_id, sign_notification_item, verify_notification_item, AdyenClient,
    AdyenClientError, AdyenHmacError, BillingReceipt, CollectionChannel, CollectionReceipt,
    InvoiceLineItem, LineCollection, NotificationItem, PaymentLinkResponse, PaymentReceipt,
    ACCEPTED_CHARGE_RESULT_CODES, ADYEN_CURRENCY, BILLING_PLATFORM, MAX_REFERENCE_LEN,
    MICROS_PER_MINOR_UNIT,
};
pub use collection::{
    stored_payment_method_id_for_publisher, CollectionEvent, CollectionStore, FileCollectionStore,
};
pub use errors::BillingError;
pub use invoice::{
    compute_invoice, compute_invoice_from_basis, round_to_cent, BillingBasisRecord, BillingPeriod,
    BillingProofRecord, Invoice, InvoiceLine,
};
pub use profile::{BillingProfile, CustomerProfile, PROFILE_ENV, VAT_RATE_ENV};
pub use register::{InvoiceRegister, DEFAULT_REGISTER_PATH};
pub use subscription::{
    watch_plan, ManagedPlan, MonitoringPlan, MonitoringPrices, MonitoringTier, PlanId, PlanLine,
    PlanState, MANAGED_BASE_FEE_MICROS, MANAGED_SCOPE, MANAGED_TOP_ABOVE_REVENUE_MICROS,
    MANAGED_TOP_FEE_MICROS, MONITORING_BASE_FEE_MICROS, MONITORING_FEE_ENV,
    MONITORING_MID_FEE_MICROS, MONITORING_TIER_BASE_ENV, MONITORING_TIER_MID_ENV,
    MONITORING_TIER_TOP_ENV, MONITORING_TOP_FEE_MICROS, PLAN_CURRENCY, WATCH_CORE_PRICE_MICROS,
    WATCH_PRICE_MICROS,
};
pub use vat::{
    is_eu_member, is_valid_vat_number, treatment_for, treatment_for_shape, treatment_for_with_vies,
    CachedVies, HttpViesClient, VatDecision, VatTreatment, ViesCheck, ViesClient, ViesError,
    ViesEvidence, ViesOutcome, EU_MEMBER_STATES, MAX_VAT_RATE_BP, VIES_ENDPOINT,
    VIES_MAX_EVIDENCE_CHARS, VIES_TIMEOUT_SECS,
};
pub use webhook::verify_adyen_webhook;

#[cfg(test)]
mod tests;
