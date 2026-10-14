//! Adyen webhook verification at the billing boundary.
//!
//! [`verify_adyen_webhook`] collapses the low-level HMAC taxonomy into the
//! billing taxonomy:
//!
//! - a missing or unparseable `X-Adyen-Hmac-SHA256` header is a
//!   [`BillingError::Malformed`] input,
//! - a digest that does not match the body under the HMAC key is
//!   [`BillingError::InvalidSignature`].
//!
//! The HMAC scheme itself (base64(HMAC-SHA256(raw body, key)), constant-time
//! comparison) lives in [`crate::adyen`]. Adyen signs no timestamp, so
//! replay protection is the caller's job: deduplicate on `eventCode` plus
//! `pspReference` and record processed events.

use crate::adyen::{verify_adyen_hmac, AdyenHmacError};
use crate::errors::BillingError;

/// Proof that a webhook delivery was signed by the endpoint HMAC key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerifiedDelivery;

/// Verify an Adyen webhook delivery.
///
/// `payload` is the exact raw request body (never a re-serialized JSON
/// value), `sig_header` the `X-Adyen-Hmac-SHA256` header value and `hmac_key`
/// the endpoint's webhook HMAC key.
pub fn verify_adyen_webhook(
    payload: &[u8],
    sig_header: &str,
    hmac_key: &[u8],
) -> Result<VerifiedDelivery, BillingError> {
    match verify_adyen_hmac(hmac_key, payload, sig_header) {
        Ok(()) => Ok(VerifiedDelivery),
        Err(AdyenHmacError::MissingHeader) => Err(BillingError::Malformed(
            "X-Adyen-Hmac-SHA256 header is missing".to_string(),
        )),
        Err(AdyenHmacError::MalformedHeader) => Err(BillingError::Malformed(
            "X-Adyen-Hmac-SHA256 header is not base64".to_string(),
        )),
        Err(AdyenHmacError::NoMatchingSignature) => Err(BillingError::InvalidSignature),
    }
}
