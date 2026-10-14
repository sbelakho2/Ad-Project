//! Adyen billing client: Checkout payment links plus the documented webhook
//! HMAC verification.
//!
//! ## Webhook verification
//!
//! Adyen signs webhook deliveries with HMAC-SHA256 over the raw request body;
//! the digest travels base64-encoded in the `X-Adyen-Hmac-SHA256` header.
//! There is no timestamp in the scheme, so freshness and replay protection
//! live with the caller: it must deduplicate on `eventCode` + `pspReference`
//! (the API layer records processed events in the billing ledger).
//!
//! ## Payment links
//!
//! Monthly fees are collected with Checkout payment links
//! (`POST /v1/paymentLinks`): the customer gets the link, Adyen collects, and
//! the webhook turns the collection state into ledger records. The link
//! `reference` doubles as the deterministic idempotency key: Adyen returns
//! the existing link when a reference repeats, so a resumed run never double
//! bills.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;

use apex_connectors::transport::{HttpBody, HttpMethod, HttpRequest, Transport};
use apex_core::PlatformId;

use crate::errors::BillingError;
use crate::invoice::InvoiceLine;

type HmacSha256 = Hmac<Sha256>;

/// The currency invoices are billed in: fees are USD micros, sent to Adyen as
/// cents (USD minor units).
pub const ADYEN_CURRENCY: &str = "USD";

/// Micros per Adyen minor unit (cent) for USD: the conversion must be exact,
/// so a fee that is not a whole number of cents is refused rather than
/// rounded.
pub const MICROS_PER_MINOR_UNIT: i64 = 10_000;

/// Adyen's documented reference length limit; the builder caps itself lower
/// so an over-long reference is a local, typed error rather than a remote 422.
pub const MAX_REFERENCE_LEN: usize = 80;

/// A webhook-signature refusal. No variant carries attacker-controlled data
/// beyond lengths.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdyenHmacError {
    /// The `X-Adyen-Hmac-SHA256` header is missing or empty.
    #[error("X-Adyen-Hmac-SHA256 header is missing")]
    MissingHeader,

    /// The header value is not valid base64.
    #[error("X-Adyen-Hmac-SHA256 header is not base64")]
    MalformedHeader,

    /// The digest does not match the payload under the HMAC key.
    #[error("HMAC digest does not match the payload")]
    NoMatchingSignature,
}

/// A configuration refusal while building an [`AdyenClient`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdyenClientError {
    /// The Checkout API key was empty: every call would fail closed anyway,
    /// but the misconfiguration must be refused at construction.
    #[error("adyen checkout api key must not be empty")]
    EmptyApiKey,

    /// The webhook HMAC key was empty: an empty HMAC key accepts forgeries
    /// signed with the empty key.
    #[error("adyen webhook hmac key must not be empty")]
    EmptyHmacKey,

    /// The merchant account was empty.
    #[error("adyen merchant account must not be empty")]
    EmptyMerchantAccount,
}

/// Adyen endpoint client: Checkout API credentials plus the webhook HMAC key.
/// The type performs I/O only through the shared transport.
#[derive(Clone)]
pub struct AdyenClient {
    api_key: String,
    merchant_account: String,
    hmac_key: Vec<u8>,
    checkout_base: String,
}

impl std::fmt::Debug for AdyenClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdyenClient")
            .field("api_key", &"[redacted]")
            .field("merchant_account", &self.merchant_account)
            .field("hmac_key", &"[redacted]")
            .field("checkout_base", &self.checkout_base)
            .finish()
    }
}

impl AdyenClient {
    pub fn new(
        api_key: impl Into<String>,
        merchant_account: impl Into<String>,
        hmac_key: impl Into<Vec<u8>>,
        checkout_base: impl Into<String>,
    ) -> Result<Self, AdyenClientError> {
        let api_key = api_key.into();
        let merchant_account = merchant_account.into();
        let hmac_key = hmac_key.into();
        if api_key.is_empty() {
            return Err(AdyenClientError::EmptyApiKey);
        }
        if merchant_account.is_empty() {
            return Err(AdyenClientError::EmptyMerchantAccount);
        }
        if hmac_key.is_empty() {
            return Err(AdyenClientError::EmptyHmacKey);
        }
        Ok(Self {
            api_key,
            merchant_account,
            hmac_key,
            checkout_base: checkout_base.into(),
        })
    }

    /// The configured Checkout API key.
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// The configured merchant account.
    pub fn merchant_account(&self) -> &str {
        &self.merchant_account
    }

    /// The configured webhook HMAC key.
    pub fn hmac_key(&self) -> &[u8] {
        &self.hmac_key
    }

    /// Verify `signature_header` (the `X-Adyen-Hmac-SHA256` value) against the
    /// exact raw `payload` bytes.
    pub fn verify(&self, payload: &[u8], signature_header: &str) -> Result<(), AdyenHmacError> {
        verify_adyen_hmac(&self.hmac_key, payload, signature_header)
    }

    /// Create one payment link for one non-zero invoice line. The reference is
    /// the line's deterministic idempotency key, so a retried month returns
    /// the same link instead of double billing.
    pub async fn create_payment_link(
        &self,
        transport: &Arc<dyn Transport>,
        item: &InvoiceLineItem,
    ) -> Result<PaymentLinkResponse, BillingError> {
        let body = serde_json::json!({
            "reference": item.reference,
            "amount": {"value": item.amount_minor_units, "currency": item.currency},
            "merchantAccount": self.merchant_account,
            "shopperReference": item.shopper_reference,
            "recurringProcessingModel": "Subscription",
            "description": item.description,
        });
        let request = HttpRequest {
            platform: PlatformId::Admob,
            operation: "createPaymentLink".to_string(),
            method: HttpMethod::Post,
            url: format!(
                "{}/v1/paymentLinks",
                self.checkout_base.trim_end_matches('/')
            ),
            headers: vec![
                ("X-API-Key".into(), self.api_key.clone()),
                ("Accept".into(), "application/json".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: Some(HttpBody::Json(body)),
            timeout_ms: 30_000,
        };
        let response = match transport.request(request).await {
            Ok(response) => response,
            Err(apex_connectors::transport::TransportError::Platform(error)) => {
                return Err(BillingError::from(error))
            }
            Err(apex_connectors::transport::TransportError::Network(detail)) => {
                return Err(BillingError::from(apex_core::PlatformError::Network {
                    platform: PlatformId::Admob,
                    operation: "createPaymentLink".into(),
                    detail,
                }))
            }
            Err(apex_connectors::transport::TransportError::Timeout(detail)) => {
                return Err(BillingError::from(apex_core::PlatformError::Network {
                    platform: PlatformId::Admob,
                    operation: "createPaymentLink".into(),
                    detail: format!("timeout: {detail}"),
                }))
            }
        };
        if response.status != 200 {
            let status = response.status;
            return Err(match status {
                401 | 403 => BillingError::from(apex_core::PlatformError::Auth {
                    platform: PlatformId::Admob,
                    operation: "createPaymentLink".into(),
                    status,
                }),
                429 => BillingError::from(apex_core::PlatformError::RateLimited {
                    platform: PlatformId::Admob,
                    operation: "createPaymentLink".into(),
                    retry_after: None,
                }),
                other => BillingError::from(apex_core::PlatformError::Http {
                    platform: PlatformId::Admob,
                    operation: "createPaymentLink".into(),
                    status: other,
                }),
            });
        }
        let parsed: serde_json::Value =
            serde_json::from_slice(&response.body).map_err(|error| {
                BillingError::Malformed(format!("payment link response is not JSON: {error}"))
            })?;
        let id = parsed
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                BillingError::Malformed("payment link response missing id".to_string())
            })?;
        let url = parsed
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                BillingError::Malformed("payment link response missing url".to_string())
            })?;
        Ok(PaymentLinkResponse {
            id: id.to_string(),
            url: url.to_string(),
        })
    }
}

/// Verify an Adyen webhook delivery.
///
/// `hmac_key` is the endpoint's webhook HMAC key, `payload` the exact raw
/// request bytes (never a re-serialized JSON value) and `signature_header`
/// the `X-Adyen-Hmac-SHA256` value. The comparison is constant-time.
pub fn verify_adyen_hmac(
    hmac_key: &[u8],
    payload: &[u8],
    signature_header: &str,
) -> Result<(), AdyenHmacError> {
    if signature_header.trim().is_empty() {
        return Err(AdyenHmacError::MissingHeader);
    }
    let mut expected = [0u8; 32];
    let decoded_len = base64_decode_into(signature_header, &mut expected)
        .ok_or(AdyenHmacError::MalformedHeader)?;
    if decoded_len != 32 {
        return Err(AdyenHmacError::NoMatchingSignature);
    }
    let mut mac =
        HmacSha256::new_from_slice(hmac_key).map_err(|_| AdyenHmacError::NoMatchingSignature)?;
    mac.update(payload);
    let computed = mac.finalize().into_bytes();
    let mut difference = 0u8;
    for (a, b) in expected.iter().zip(computed.iter()) {
        difference |= a ^ b;
    }
    if difference == 0 {
        Ok(())
    } else {
        Err(AdyenHmacError::NoMatchingSignature)
    }
}

/// Test and tooling helper: produce the header value Adyen would send for a
/// payload, so tests sign real deliveries.
pub fn sign_adyen_payload(hmac_key: &[u8], payload: &[u8]) -> String {
    let mut mac = match HmacSha256::new_from_slice(hmac_key) {
        Ok(mac) => mac,
        Err(_) => return base64_encode(&[]),
    };
    mac.update(payload);
    let digest = mac.finalize().into_bytes();
    base64_encode(&digest)
}

fn base64_decode_into(input: &str, output: &mut [u8; 32]) -> Option<usize> {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    let mut written = 0usize;
    for ch in input.bytes() {
        if ch == b'=' {
            break;
        }
        let value = table.iter().position(|t| *t == ch)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            if written >= output.len() {
                return None;
            }
            output[written] = (buffer >> bits) as u8;
            written += 1;
        }
    }
    Some(written)
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(triple >> 18) as usize & 63] as char);
        out.push(TABLE[(triple >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(triple >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[triple as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// One Adyen payment-link request derived from a verified invoice line. The
/// reference is the deterministic idempotency key:
/// `apex-fee:<publisher>:<experiment>:<YYYY-MM>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLineItem {
    /// Ledger sequence of the billing proof this item bills.
    pub seq: u64,
    /// Experiment the proven lift belongs to.
    pub experiment_id: String,
    /// Adyen amount in minor units (USD cents).
    pub amount_minor_units: i64,
    /// ISO currency code (always [`ADYEN_CURRENCY`] here).
    pub currency: String,
    /// Shopper reference of the publisher.
    pub shopper_reference: String,
    /// Deterministic reference (idempotency key) sent with the request.
    pub reference: String,
    /// Human-readable description shown on the payment link.
    pub description: String,
}

impl InvoiceLineItem {
    /// Derive the request payload for one invoice line.
    ///
    /// Refuses a fee that is not an exact number of cents
    /// ([`BillingError::SubCentAmount`]) and a negative fee
    /// ([`BillingError::TamperedProof`], defensively: `compute_invoice` never
    /// produces one). Also refuses identifiers that cannot travel in the
    /// reference: control characters (including CR/LF) or a derived reference
    /// longer than [`MAX_REFERENCE_LEN`] ([`BillingError::UnsafeIdentifier`]).
    pub fn from_invoice_line(
        line: &InvoiceLine,
        publisher_id: &str,
        year_month: &str,
    ) -> Result<Self, BillingError> {
        if line.fee_micros < 0 {
            return Err(BillingError::TamperedProof {
                seq: line.seq,
                reason: format!("line fee {} micros is negative", line.fee_micros),
            });
        }
        if line.fee_micros % MICROS_PER_MINOR_UNIT != 0 {
            return Err(BillingError::SubCentAmount { seq: line.seq });
        }
        let reference = format!(
            "apex-fee:{publisher_id}:{}:{year_month}",
            line.experiment_id
        );
        let shopper_reference = format!("publisher-{publisher_id}");
        if let Some(control) = reference.chars().find(|c| c.is_control()) {
            return Err(BillingError::UnsafeIdentifier {
                seq: line.seq,
                detail: format!(
                    "reference contains control character U+{:04X}",
                    control as u32
                ),
            });
        }
        if reference.len() > MAX_REFERENCE_LEN {
            return Err(BillingError::UnsafeIdentifier {
                seq: line.seq,
                detail: format!(
                    "reference is {} characters (max {MAX_REFERENCE_LEN})",
                    reference.len()
                ),
            });
        }
        Ok(Self {
            seq: line.seq,
            experiment_id: line.experiment_id.clone(),
            amount_minor_units: line.fee_micros / MICROS_PER_MINOR_UNIT,
            currency: ADYEN_CURRENCY.to_string(),
            shopper_reference,
            reference,
            description: format!(
                "Apex proven-lift fee: experiment {} ({} bp of {} micros proven lift)",
                line.experiment_id, line.rate_bp, line.proven_lift
            ),
        })
    }
}

/// The fields of Adyen's payment-link response that billing records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentLinkResponse {
    /// Adyen link id (`PL...`).
    pub id: String,
    /// Shopper-facing URL to collect the payment.
    pub url: String,
}

/// What [`create_payment_links`] created: the payment links and their checked
/// total. An invoice whose every line is zero (or which has no lines) sends
/// no request at all and yields an empty receipt with a zero total. Zero-fee
/// lines among paid lines are intentionally omitted from `links`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingReceipt {
    /// Publisher the invoice was built for.
    pub publisher_id: String,
    /// Billing period label (`YYYY-MM`).
    pub year_month: String,
    /// ISO currency code of all items.
    pub currency: String,
    /// Checked sum of the created link amounts, in minor units.
    pub total_minor_units: i64,
    /// Created payment links, in ledger `seq` order.
    pub links: Vec<PaymentLinkResponse>,
}

/// Create one Adyen payment link per non-zero invoice line, in `seq` order.
///
/// Every request goes through the shared [`Transport`] with the
/// `X-API-Key` header; the link reference is the line's deterministic
/// idempotency key, so a partially applied invoice can be resumed without
/// double billing. Refusals are typed and never retried at this layer; the
/// caller composes the retry transport exactly like the connectors do.
pub async fn create_payment_links(
    client: &AdyenClient,
    transport: &Arc<dyn Transport>,
    invoice: &crate::invoice::Invoice,
) -> Result<BillingReceipt, BillingError> {
    let mut links = Vec::new();
    let mut total_minor_units = 0i64;
    for line in &invoice.line_items {
        if line.fee_micros == 0 {
            continue;
        }
        let item = InvoiceLineItem::from_invoice_line(
            line,
            &invoice.publisher_id,
            &invoice.period.year_month,
        )?;
        let link = client.create_payment_link(transport, &item).await?;
        total_minor_units += item.amount_minor_units;
        links.push(link);
    }
    Ok(BillingReceipt {
        publisher_id: invoice.publisher_id.clone(),
        year_month: invoice.period.year_month.clone(),
        currency: ADYEN_CURRENCY.to_string(),
        total_minor_units,
        links,
    })
}
