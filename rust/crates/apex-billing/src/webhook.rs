//! Adyen webhook verification at the billing boundary.
//!
//! [`verify_adyen_webhook`] parses the `notificationItems` envelope and
//! verifies **each** item's `additionalData.hmacSignature`, collapsing the
//! low-level HMAC taxonomy into the billing one:
//!
//! - a malformed envelope or a missing/unparseable signature is a
//!   [`BillingError::Malformed`] input,
//! - a digest that does not match the eight signed fields is
//!   [`BillingError::InvalidSignature`].
//!
//! The scheme itself (base64(HMAC-SHA256(fields joined by `:`)), constant-time
//! comparison) lives in [`crate::adyen`]. Adyen signs no timestamp, so replay
//! protection is the caller's job: deduplicate on `eventCode` plus
//! `pspReference` and record processed events.

use crate::adyen::{
    parse_notification_items, verify_notification_item, AdyenHmacError, NotificationItem,
};
use crate::errors::BillingError;

/// Verify every notification item in an Adyen delivery and return them.
///
/// `body` is the exact raw request body and `hmac_key` the endpoint's webhook
/// HMAC key. An empty envelope is refused: a delivery that verifies nothing
/// must never be treated as handled.
pub fn verify_adyen_webhook(
    body: &[u8],
    hmac_key: &[u8],
) -> Result<Vec<NotificationItem>, BillingError> {
    let items = parse_notification_items(body)?;
    if items.is_empty() {
        return Err(BillingError::Malformed(
            "notification envelope carried no items".to_string(),
        ));
    }
    for item in &items {
        verify_notification_item(hmac_key, item).map_err(|error| match error {
            AdyenHmacError::MissingHeader => BillingError::Malformed(
                "notification item has no additionalData.hmacSignature".to_string(),
            ),
            AdyenHmacError::MalformedHeader => {
                BillingError::Malformed("hmacSignature is not base64".to_string())
            }
            AdyenHmacError::NoMatchingSignature => BillingError::InvalidSignature,
        })?;
    }
    Ok(items)
}
