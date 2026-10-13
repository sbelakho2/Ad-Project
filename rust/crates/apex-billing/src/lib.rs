//! apex-billing — lift-based billing: 15%..20% of *platform-proven* lift only.
//!
//! The invoice is derived exclusively from signed
//! [`apex_proof::LedgerRecord::BillingProof`] entries. [`compute_invoice`]
//! re-verifies every proof (positive proven lift, rate inside the
//! 15.00%..=20.00% band, stored fee equal to
//! [`apex_core::lift_fee_micros`] of the proven lift) before a single micro is
//! billable, so a tampered ledger can never move money. No lift, no fee: an
//! empty proof set yields a zero invoice.
//!
//! Stripe is spoken over the shared [`apex_connectors::transport`] stack
//! (idempotent POSTs through the same retry policy as the platform
//! connectors), and webhook signatures are verified with the documented
//! `t=<unix>,v1=<hex>` HMAC-SHA256 scheme in constant time.
//!
//! ## Guarantees
//!
//! - **Never trust stored fees:** the fee in a proof is recomputed from
//!   `apex-core` arithmetic and compared; any mismatch is
//!   [`BillingError::TamperedProof`].
//! - **Deterministic invoices:** line items are ordered by ledger `seq` and
//!   the total is a checked sum (overflow is an error, never a wrap).
//! - **Fail closed:** malformed Stripe responses and malformed signature
//!   headers are typed errors; a request is never sent for a zero invoice.
//! - **Idempotent billing:** every Stripe invoice item carries the
//!   deterministic key `stripe-item:<publisher>:<experiment>:<period>`, so a
//!   partially failed invoice can be resumed without double billing.

#![forbid(unsafe_code)]
// Production code is held to the strict workspace lints; tests may unwrap/expect/panic
// because a panicking test IS a failing test.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

pub mod encode;
pub mod errors;
pub mod invoice;
pub mod stripe;
#[cfg(test)]
mod tests;
pub mod webhook;

pub use encode::encode_form_component;
pub use errors::BillingError;
pub use invoice::{compute_invoice, BillingPeriod, BillingProofRecord, Invoice, InvoiceLine};
pub use stripe::{
    bill_invoice, BillingReceipt, InvoiceLineItem, StripeClient, StripeItemResponse,
    STRIPE_API_BASE,
};
pub use webhook::{verify_stripe_signature, EventMeta, DEFAULT_TOLERANCE_SECS};
