//! Mollie webhook handling at the billing boundary.
//!
//! Mollie webhooks are not signed. The only trustworthy part of a delivery is
//! the form field `id`; [`fetch_webhook_payment`] parses it and then fetches
//! the payment from Mollie with the API key. A forged, unknown or replayed id
//! therefore cannot become a payment. The caller validates the fetched
//! status, amount, currency and metadata against what it expected before
//! recording anything.
//!
//! The body parser is deliberately strict: one non-empty `id`, a Mollie
//! payment-id shape, and no duplicate `id` field. An empty or malformed
//! delivery is [`BillingError::Malformed`], never a silently handled event.

use crate::errors::BillingError;
use crate::mollie::{is_payment_id, MollieClient, MolliePayment};
use apex_connectors::transport::Transport;

/// Percent-decode one `application/x-www-form-urlencoded` value.
fn form_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = (bytes[index + 1] as char).to_digit(16);
                let low = (bytes[index + 2] as char).to_digit(16);
                match (high, low) {
                    (Some(high), Some(low)) => {
                        out.push((high * 16 + low) as u8);
                        index += 3;
                    }
                    _ => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The single `id` field of a Mollie webhook form. Fails closed on malformed
/// bodies, duplicate ids and ids that are not Mollie payment ids.
pub fn parse_webhook_id(body: &[u8]) -> Result<String, BillingError> {
    let text = std::str::from_utf8(body)
        .map_err(|_| BillingError::Malformed("webhook body is not UTF-8".to_string()))?;
    let mut id: Option<String> = None;
    for field in text.split('&').filter(|field| !field.is_empty()) {
        let (key, value) = match field.split_once('=') {
            Some((key, value)) => (key, value),
            None => (field, ""),
        };
        if form_decode(key) != "id" {
            continue;
        }
        let value = form_decode(value).trim().to_string();
        if id.is_some() {
            return Err(BillingError::Malformed(
                "webhook body carries more than one id field".to_string(),
            ));
        }
        id = Some(value);
    }
    let id = id
        .filter(|id| !id.is_empty())
        .ok_or_else(|| BillingError::Malformed("webhook body has no id field".to_string()))?;
    if !is_payment_id(&id) {
        return Err(BillingError::Malformed(
            "webhook id is not a Mollie payment id".to_string(),
        ));
    }
    Ok(id)
}

/// Parse the delivery's id and fetch the payment from Mollie. Nothing from
/// the POST body other than the id is used.
pub async fn fetch_webhook_payment(
    client: &MollieClient,
    transport: &dyn Transport,
    body: &[u8],
) -> Result<MolliePayment, BillingError> {
    let id = parse_webhook_id(body)?;
    let payment = client.fetch_payment(transport, &id).await?;
    if payment.id != id {
        return Err(BillingError::WebhookRejected {
            detail: "fetched payment id does not match the webhook id".to_string(),
        });
    }
    Ok(payment)
}

/// Validate the fetched payment's metadata and optional expected amount.
///
/// `expected_reference` is the deterministic reference the deployment issued
/// for this payment; when present it must match exactly. `expected_amount` is
/// the tier/invoice amount the deployment expected; when present the fetched
/// amount must equal it exactly (this is the underpayment check).
pub fn validate_payment(
    payment: &MolliePayment,
    expected_reference: Option<&str>,
    expected_amount: Option<i64>,
) -> Result<(), BillingError> {
    let reference =
        payment
            .metadata_reference
            .as_deref()
            .ok_or_else(|| BillingError::WebhookRejected {
                detail: "payment metadata carries no reference".to_string(),
            })?;
    if reference.trim().is_empty() {
        return Err(BillingError::WebhookRejected {
            detail: "payment metadata reference is empty".to_string(),
        });
    }
    if let Some(expected) = expected_reference {
        if reference != expected {
            return Err(BillingError::WebhookRejected {
                detail: "payment metadata reference does not match the expected reference"
                    .to_string(),
            });
        }
    }
    if let Some(expected) = expected_amount {
        if payment.amount_minor_units != expected {
            return Err(BillingError::WebhookRejected {
                detail: "payment amount does not match the expected amount".to_string(),
            });
        }
    }
    if let Some(metadata_amount) = payment.metadata_amount_minor_units {
        if metadata_amount != payment.amount_minor_units {
            return Err(BillingError::WebhookRejected {
                detail: "payment amount does not match its metadata amount".to_string(),
            });
        }
    }
    if payment.currency != crate::mollie::BILLING_CURRENCY {
        return Err(BillingError::WebhookRejected {
            detail: "payment currency is not the billing currency".to_string(),
        });
    }
    Ok(())
}
