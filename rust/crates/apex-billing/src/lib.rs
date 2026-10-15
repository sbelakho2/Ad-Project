//! Apex billing: proven-lift fees through Adyen.
//!
//! - [`adyen`]: Checkout payment links for monthly collection, plus the
//!   documented webhook HMAC verification. Adyen is spoken over the shared
//!   [`apex_connectors::transport`] stack, so retries, timeout budgets and
//!   redacted logging are identical to the platform connectors.
//! - [`webhook`]: the billing-boundary verification contract used by the API
//!   layer.
//! - [`invoice`]: the deterministic monthly invoice derived from verified
//!   billing proofs (`compute_invoice`).
//!
//! Design invariants:
//!
//! - **Fail closed:** malformed Adyen responses and malformed signature
//!   headers are typed refusals, never silent skips.
//! - **Idempotent billing:** every payment link carries the deterministic
//!   reference `apex-fee:<publisher>:<experiment>:<period>`, which Adyen
//!   deduplicates, so a resumed month cannot double bill.
//! - **No rounding:** a fee that is not a whole number of cents is refused
//!   (`BillingError::SubCentAmount`).

pub mod adyen;
pub mod collection;
pub mod errors;
pub mod invoice;
pub mod subscription;
pub mod webhook;

pub use adyen::{
    create_payment_links, parse_notification_items, sign_notification_item,
    verify_notification_item, AdyenClient, AdyenClientError, AdyenHmacError, BillingReceipt,
    InvoiceLineItem, NotificationItem, PaymentLinkResponse, ADYEN_CURRENCY, MAX_REFERENCE_LEN,
    MICROS_PER_MINOR_UNIT,
};
pub use collection::{CollectionEvent, CollectionStore, FileCollectionStore};
pub use errors::BillingError;
pub use invoice::{
    compute_invoice, compute_invoice_from_basis, round_to_cent, BillingBasisRecord, BillingPeriod,
    BillingProofRecord, Invoice, InvoiceLine,
};
pub use subscription::MonitoringPlan;
pub use webhook::verify_adyen_webhook;

#[cfg(test)]
mod tests;
