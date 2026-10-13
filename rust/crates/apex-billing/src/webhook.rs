//! Stripe webhook verification at the billing boundary.
//!
//! This module exposes the documented verification contract used by the API
//! layer: [`verify_stripe_signature(payload, sig_header, secret,
//! tolerance_secs, now_unix_s)`](verify_stripe_signature) returns the signed
//! [`EventMeta`] or a typed [`BillingError`], collapsing the low-level
//! signature taxonomy into the billing taxonomy:
//!
//! - an unparseable header (or one without a `v1` component) is a
//!   [`BillingError::Malformed`] input,
//! - a signature that does not match the body under the secret is
//!   [`BillingError::InvalidSignature`],
//! - a timestamp outside the caller's tolerance window is
//!   [`BillingError::StaleTimestamp`] (both stale and future-dated).
//!
//! The HMAC scheme itself (`t=<unix>,v1=<hex>`, message `"{t}.{body}"`,
//! constant-time comparison, every `v1` candidate tried) lives in
//! [`crate::stripe`], which is also what the API server's `StripeClient`
//! delegates to.

use crate::errors::BillingError;
use crate::stripe::{verify_stripe_signature as verify_hmac, StripeSignatureError};

/// The customary freshness tolerance: Stripe emits skew well under a minute,
/// and 300 seconds is the long-standing default across Stripe integrations.
pub const DEFAULT_TOLERANCE_SECS: u64 = 300;

/// Proof that a webhook delivery was signed by the endpoint secret: the
/// timestamp that was part of the signed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventMeta {
    /// The `t` value from the verified `Stripe-Signature` header (Unix
    /// seconds).
    pub timestamp: i64,
}

/// Verify a Stripe webhook delivery and return its signed timestamp.
///
/// `payload` is the exact raw request body (never a re-serialized JSON
/// value), `sig_header` the `Stripe-Signature` header value, `secret` the
/// endpoint's signing secret, `tolerance_secs` the maximum accepted absolute
/// timestamp skew and `now_unix_s` the verifier's clock (an explicit argument
/// so verification is deterministic and testable).
pub fn verify_stripe_signature(
    payload: &[u8],
    sig_header: &str,
    secret: &[u8],
    tolerance_secs: u64,
    now_unix_s: i64,
) -> Result<EventMeta, BillingError> {
    match verify_hmac(secret, payload, sig_header, tolerance_secs, now_unix_s) {
        Ok(verified) => Ok(EventMeta {
            timestamp: verified.timestamp,
        }),
        Err(StripeSignatureError::MalformedHeader) => Err(BillingError::Malformed(
            "malformed Stripe-Signature header".to_string(),
        )),
        Err(StripeSignatureError::MissingV1Signature) => Err(BillingError::Malformed(
            "Stripe-Signature header has no v1 component".to_string(),
        )),
        Err(StripeSignatureError::NoMatchingSignature) => Err(BillingError::InvalidSignature),
        Err(StripeSignatureError::TimestampOutOfTolerance { .. }) => {
            let timestamp = header_timestamp(sig_header).ok_or_else(|| {
                BillingError::Malformed("Stripe-Signature header has no timestamp".to_string())
            })?;
            Err(BillingError::StaleTimestamp {
                timestamp,
                now_unix_s,
            })
        }
    }
}

/// The `t` value of a header that the signature layer already parsed.
fn header_timestamp(sig_header: &str) -> Option<i64> {
    sig_header.split(',').find_map(|item| {
        let (key, value) = item.trim().split_once('=')?;
        if key.trim() == "t" {
            value.trim().parse::<i64>().ok()
        } else {
            None
        }
    })
}
