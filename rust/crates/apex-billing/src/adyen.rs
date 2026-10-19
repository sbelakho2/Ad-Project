//! Adyen billing client: Checkout payment links, the card-on-file token
//! charge, and the documented webhook HMAC verification.
//!
//! ## Webhook verification
//!
//! Adyen standard webhooks arrive as a `notificationItems` envelope; each
//! `NotificationRequestItem` carries its own signature in
//! `additionalData.hmacSignature`. The signature is HMAC-SHA256 (base64) over
//! the eight documented fields joined with `:`, in order:
//! `pspReference`, `originalReference`, `merchantAccountCode`,
//! `merchantReference`, `amount.value`, `amount.currency`, `eventCode`,
//! `success`. Each field is escaped before joining: `\` becomes `\\` and `:`
//! becomes `\:` (Adyen's documented scheme; merchant references commonly
//! contain colons). Verification is per item and constant-time. There is no
//! timestamp in the scheme, so replay protection lives with the caller: it
//! deduplicates on `eventCode` plus `pspReference`.
//!
//! ## Payment links
//!
//! Monthly fees are collected with Checkout payment links
//! (`POST /v71/paymentLinks`): the customer gets the link, Adyen collects, and
//! the webhook turns the collection state into ledger records. The deterministic
//! `reference` is also sent as the `Idempotency-Key` header, so a resumed run
//! returns the same link instead of double billing. A link request sets
//! `storePaymentMethod: true`, so the first payment stores the card and the
//! webhook reports the stored method id in
//! `additionalData.storedPaymentMethodId`.
//!
//! ## Card on file
//!
//! Once a stored method exists, the monthly collection charges it directly
//! (`POST /v71/payments` with `storedPaymentMethodId`,
//! `shopperInteraction: ContAuth` and `recurringProcessingModel:
//! Subscription`). A refusal (a non-accepted `resultCode`) is a typed
//! [`BillingError::StoredChargeRefused`] and the collection falls back to a
//! fresh payment link with the refusal recorded on the line — a token
//! problem never silently stops billing and never silently double-charges
//! (network/transport failures propagate instead of falling back, because
//! the charge may have landed).
//!
//! ## Platform label
//!
//! Every request and error carries [`PlatformId::Billing`]: billing is its
//! own domain (the metrics vocabulary value `billing`), not an ad-network
//! mediation platform. Billing calls must never be labeled `Admob` — the
//! regression test pins this.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use apex_connectors::transport::{HttpBody, HttpMethod, HttpRequest, Transport};
use apex_core::PlatformId;

use crate::errors::BillingError;
use crate::invoice::InvoiceLine;

type HmacSha256 = Hmac<Sha256>;

/// The platform/domain label every billing request and error carries.
pub const BILLING_PLATFORM: PlatformId = PlatformId::Billing;

/// The stored-method charge result codes accepted as collected. Every other
/// code (Refused, Error, Cancelled, ...) is a typed refusal that falls back
/// to a payment link.
pub const ACCEPTED_CHARGE_RESULT_CODES: [&str; 2] = ["Authorised", "Received"];

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

/// Whether an HTTP status is an accepted Adyen success. Checkout
/// `paymentLinks` answers `201 Created`; `/payments` answers `200 OK`. Both
/// are successes, and treating a 201 as a failure used to discard a link
/// Adyen had already created (and billed against on resume).
fn is_success_status(status: u16) -> bool {
    (200..300).contains(&status)
}

/// The request-shaping invariants every Checkout call shares. The invoice
/// builder already enforces them, but the client is public and the watch path
/// constructs [`InvoiceLineItem`] directly, so a malformed item is refused
/// locally instead of becoming a remote 422 (or, worse, a request Adyen
/// accepts with a different meaning).
fn validate_line_item(item: &InvoiceLineItem) -> Result<(), BillingError> {
    if item.amount_minor_units <= 0 {
        return Err(BillingError::Malformed(format!(
            "invoice line seq {} has a non-positive amount {} minor units",
            item.seq, item.amount_minor_units
        )));
    }
    if item.currency.len() != 3 || !item.currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(BillingError::Malformed(format!(
            "invoice line seq {} has an invalid ISO currency {:?}",
            item.seq, item.currency
        )));
    }
    if item.reference.trim().is_empty()
        || item.reference.len() > MAX_REFERENCE_LEN
        || item.reference.chars().any(char::is_control)
    {
        return Err(BillingError::Malformed(format!(
            "invoice line seq {} has an unusable Adyen reference (length {}, control characters or empty)",
            item.seq,
            item.reference.len()
        )));
    }
    if item.shopper_reference.trim().is_empty() {
        return Err(BillingError::Malformed(format!(
            "invoice line seq {} has an empty shopperReference",
            item.seq
        )));
    }
    Ok(())
}

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

    /// The Checkout base URL was empty: every request would be built against
    /// a relative path and could reach the wrong origin.
    #[error("adyen checkout base must not be empty")]
    EmptyCheckoutBase,
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
        let checkout_base = checkout_base.into();
        if api_key.is_empty() {
            return Err(AdyenClientError::EmptyApiKey);
        }
        if merchant_account.is_empty() {
            return Err(AdyenClientError::EmptyMerchantAccount);
        }
        if hmac_key.is_empty() {
            return Err(AdyenClientError::EmptyHmacKey);
        }
        if checkout_base.trim().is_empty() {
            return Err(AdyenClientError::EmptyCheckoutBase);
        }
        Ok(Self {
            api_key,
            merchant_account,
            hmac_key,
            checkout_base,
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

    /// Verify one parsed notification item's `hmacSignature`.
    pub fn verify(&self, item: &NotificationItem) -> Result<(), AdyenHmacError> {
        verify_notification_item(&self.hmac_key, item)
    }

    /// Create one payment link for one non-zero invoice line. The reference is
    /// the line's deterministic idempotency key, so a retried month returns
    /// the same link instead of double billing. The link stores the shopper's
    /// payment method (`storePaymentMethod: true`), so the first payment
    /// produces the card-on-file token the later months charge.
    pub async fn create_payment_link(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
    ) -> Result<PaymentLinkResponse, BillingError> {
        validate_line_item(item)?;
        let body = serde_json::json!({
            "reference": item.reference,
            "amount": {"value": item.amount_minor_units, "currency": item.currency},
            "merchantAccount": self.merchant_account,
            "shopperReference": item.shopper_reference,
            "recurringProcessingModel": "Subscription",
            "storePaymentMethod": true,
            "description": item.description,
        });
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "createPaymentLink".to_string(),
            method: HttpMethod::Post,
            url: format!(
                "{}/v71/paymentLinks",
                self.checkout_base.trim_end_matches('/')
            ),
            headers: vec![
                ("X-API-Key".into(), self.api_key.clone()),
                // Adyen deduplicates on this header; the deterministic
                // reference makes a resumed month return the same link.
                ("Idempotency-Key".into(), item.reference.clone()),
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
                    platform: BILLING_PLATFORM,
                    operation: "createPaymentLink".into(),
                    detail,
                }))
            }
            Err(apex_connectors::transport::TransportError::Timeout(detail)) => {
                return Err(BillingError::from(apex_core::PlatformError::Network {
                    platform: BILLING_PLATFORM,
                    operation: "createPaymentLink".into(),
                    detail: format!("timeout: {detail}"),
                }))
            }
        };
        if !is_success_status(response.status) {
            let status = response.status;
            return Err(match status {
                401 | 403 => BillingError::from(apex_core::PlatformError::Auth {
                    platform: BILLING_PLATFORM,
                    operation: "createPaymentLink".into(),
                    status,
                }),
                429 => BillingError::from(apex_core::PlatformError::RateLimited {
                    platform: BILLING_PLATFORM,
                    operation: "createPaymentLink".into(),
                    retry_after: None,
                }),
                other => BillingError::from(apex_core::PlatformError::Http {
                    platform: BILLING_PLATFORM,
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

    /// Charge the stored card (`POST /v71/payments`, `ContAuth` +
    /// `Subscription`). A non-accepted `resultCode` is the typed
    /// [`BillingError::StoredChargeRefused`] the collection falls back from;
    /// transport, auth, rate-limit and HTTP failures propagate unchanged
    /// (the caller must never fall back on an uncertain charge).
    pub async fn charge_stored_payment_method(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        stored_payment_method_id: &str,
    ) -> Result<PaymentReceipt, BillingError> {
        validate_line_item(item)?;
        if stored_payment_method_id.trim().is_empty() {
            return Err(BillingError::Malformed(
                "stored payment method id must not be empty".to_string(),
            ));
        }
        let body = serde_json::json!({
            "amount": {"value": item.amount_minor_units, "currency": item.currency},
            "merchantAccount": self.merchant_account,
            "reference": item.reference,
            "shopperReference": item.shopper_reference,
            "shopperInteraction": "ContAuth",
            "recurringProcessingModel": "Subscription",
            "storedPaymentMethodId": stored_payment_method_id,
            "description": item.description,
        });
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "chargeStoredPaymentMethod".to_string(),
            method: HttpMethod::Post,
            url: format!("{}/v71/payments", self.checkout_base.trim_end_matches('/')),
            headers: vec![
                ("X-API-Key".into(), self.api_key.clone()),
                ("Idempotency-Key".into(), item.reference.clone()),
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
                    platform: BILLING_PLATFORM,
                    operation: "chargeStoredPaymentMethod".into(),
                    detail,
                }))
            }
            Err(apex_connectors::transport::TransportError::Timeout(detail)) => {
                return Err(BillingError::from(apex_core::PlatformError::Network {
                    platform: BILLING_PLATFORM,
                    operation: "chargeStoredPaymentMethod".into(),
                    detail: format!("timeout: {detail}"),
                }))
            }
        };
        if !is_success_status(response.status) {
            let status = response.status;
            return Err(match status {
                401 | 403 => BillingError::from(apex_core::PlatformError::Auth {
                    platform: BILLING_PLATFORM,
                    operation: "chargeStoredPaymentMethod".into(),
                    status,
                }),
                429 => BillingError::from(apex_core::PlatformError::RateLimited {
                    platform: BILLING_PLATFORM,
                    operation: "chargeStoredPaymentMethod".into(),
                    retry_after: None,
                }),
                other => BillingError::from(apex_core::PlatformError::Http {
                    platform: BILLING_PLATFORM,
                    operation: "chargeStoredPaymentMethod".into(),
                    status: other,
                }),
            });
        }
        let parsed: serde_json::Value =
            serde_json::from_slice(&response.body).map_err(|error| {
                BillingError::Malformed(format!("payments response is not JSON: {error}"))
            })?;
        let result_code = parsed
            .get("resultCode")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| BillingError::Malformed("payments response missing resultCode".into()))?
            .to_string();
        if !ACCEPTED_CHARGE_RESULT_CODES.contains(&result_code.as_str()) {
            return Err(BillingError::StoredChargeRefused {
                detail: format!("resultCode {result_code}"),
            });
        }
        let psp_reference = parsed
            .get("pspReference")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                BillingError::Malformed("payments response missing pspReference".into())
            })?
            .to_string();
        Ok(PaymentReceipt {
            psp_reference,
            result_code,
            stored_payment_method_id: parse_stored_payment_method_id(&parsed),
        })
    }
}

/// One parsed Adyen standard-webhook notification item.
///
/// Adyen signs the eight fields below (joined with `:`, in this exact order)
/// with HMAC-SHA256 and carries the base64 digest in
/// `additionalData.hmacSignature`. Verification is over the *item*, not the
/// raw body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationItem {
    pub psp_reference: String,
    #[serde(default)]
    pub original_reference: String,
    pub merchant_account_code: String,
    pub merchant_reference: String,
    /// `amount.value` in minor units.
    pub amount_value: i64,
    pub amount_currency: String,
    pub event_code: String,
    /// `"true"` or `"false"` (Adyen sends a string).
    pub success: String,
    /// `additionalData.hmacSignature`, base64.
    pub hmac_signature: String,
    /// `additionalData.merchantReference`-independent payment method, kept for
    /// auditability.
    #[serde(default)]
    pub payment_method: String,
    /// `additionalData.storedPaymentMethodId` when the payment stored the
    /// shopper's method (the first payment of a subscription stores the
    /// card). Not part of the signed fields; empty when absent.
    #[serde(default)]
    pub stored_payment_method_id: String,
}

impl NotificationItem {
    /// The exact string Adyen signs: the eight documented fields, colon
    /// separated, in order, with each field escaped (`\` -> `\\`, `:` ->
    /// `\:`) exactly as Adyen's HMAC documentation specifies. Merchant
    /// references contain colons, so signing the unescaped fields would
    /// verify against a scheme Adyen never uses.
    pub fn signed_message(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}",
            escape_signed_field(&self.psp_reference),
            escape_signed_field(&self.original_reference),
            escape_signed_field(&self.merchant_account_code),
            escape_signed_field(&self.merchant_reference),
            self.amount_value,
            escape_signed_field(&self.amount_currency),
            escape_signed_field(&self.event_code),
            escape_signed_field(&self.success)
        )
    }
}

/// Escape one field for Adyen's signed message: backslash first, then colon,
/// so the escaping is unambiguous and reversible.
fn escape_signed_field(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            ':' => escaped.push_str("\\:"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Parse the `notificationItems` envelope into items. Fails closed on any
/// shape it does not recognise, so a malformed delivery can never be read as
/// a smaller, valid one.
pub fn parse_notification_items(body: &[u8]) -> Result<Vec<NotificationItem>, BillingError> {
    let value: serde_json::Value = serde_json::from_slice(body).map_err(|error| {
        BillingError::Malformed(format!("notification body is not JSON: {error}"))
    })?;
    let items = value
        .get("notificationItems")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            BillingError::Malformed("notification body has no notificationItems array".to_string())
        })?;
    let mut out = Vec::with_capacity(items.len());
    for wrapper in items {
        let item = wrapper.get("NotificationRequestItem").ok_or_else(|| {
            BillingError::Malformed(
                "notification item is not wrapped in NotificationRequestItem".to_string(),
            )
        })?;
        out.push(parse_item(item)?);
    }
    Ok(out)
}

fn parse_item(item: &serde_json::Value) -> Result<NotificationItem, BillingError> {
    let text = |value: &serde_json::Value, key: &str| -> Result<String, BillingError> {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| BillingError::Malformed(format!("notification item missing '{key}'")))
    };
    let amount = item
        .get("amount")
        .ok_or_else(|| BillingError::Malformed("notification item missing 'amount'".to_string()))?;
    let amount_value = amount
        .get("value")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| BillingError::Malformed("amount.value is not an integer".to_string()))?;
    let amount_currency = text(amount, "currency")?;
    let success = match item.get("success") {
        Some(serde_json::Value::Bool(flag)) => flag.to_string(),
        Some(serde_json::Value::String(text)) => text.clone(),
        _ => {
            return Err(BillingError::Malformed(
                "notification item missing 'success'".to_string(),
            ))
        }
    };
    let hmac_signature = item
        .get("additionalData")
        .and_then(|v| v.get("hmacSignature"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            BillingError::Malformed(
                "notification item missing additionalData.hmacSignature".to_string(),
            )
        })?;
    let payment_method = item
        .get("paymentMethod")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let stored_payment_method_id = parse_stored_payment_method_id(item).unwrap_or_default();
    Ok(NotificationItem {
        psp_reference: text(item, "pspReference")?,
        original_reference: item
            .get("originalReference")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        merchant_account_code: text(item, "merchantAccountCode")?,
        merchant_reference: text(item, "merchantReference")?,
        amount_value,
        amount_currency,
        event_code: text(item, "eventCode")?,
        success,
        hmac_signature,
        payment_method,
        stored_payment_method_id,
    })
}

/// The stored payment method id a webhook item or a payments response
/// reports, when present and non-empty. Adyen carries it in
/// `additionalData.storedPaymentMethodId` for both the first payment's
/// webhook and the `/payments` response; the field is not part of the signed
/// message.
pub fn parse_stored_payment_method_id(value: &serde_json::Value) -> Option<String> {
    value
        .get("additionalData")
        .and_then(|data| data.get("storedPaymentMethodId"))
        .and_then(|id| id.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Verify one notification item's `hmacSignature` against the endpoint's HMAC
/// key. Constant-time comparison.
pub fn verify_notification_item(
    hmac_key: &[u8],
    item: &NotificationItem,
) -> Result<(), AdyenHmacError> {
    if item.hmac_signature.trim().is_empty() {
        return Err(AdyenHmacError::MissingHeader);
    }
    let mut expected = [0u8; 32];
    let decoded_len = base64_decode_into(&item.hmac_signature, &mut expected)
        .ok_or(AdyenHmacError::MalformedHeader)?;
    if decoded_len != 32 {
        return Err(AdyenHmacError::NoMatchingSignature);
    }
    let mut mac =
        HmacSha256::new_from_slice(hmac_key).map_err(|_| AdyenHmacError::NoMatchingSignature)?;
    mac.update(item.signed_message().as_bytes());
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

/// Test and tooling helper: the base64 digest Adyen would put in
/// `additionalData.hmacSignature` for `item`.
pub fn sign_notification_item(hmac_key: &[u8], item: &NotificationItem) -> String {
    let mut mac = match HmacSha256::new_from_slice(hmac_key) {
        Ok(mac) => mac,
        Err(_) => return base64_encode(&[]),
    };
    mac.update(item.signed_message().as_bytes());
    base64_encode(&mac.finalize().into_bytes())
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
        // The collected amount is the gross (net fee + VAT); an invoice with no
        // VAT set has gross == fee.
        let gross_micros = if line.gross_micros > 0 {
            line.gross_micros
        } else {
            line.fee_micros
        };
        if gross_micros < 0 {
            return Err(BillingError::TamperedProof {
                seq: line.seq,
                reason: format!("line gross {gross_micros} micros is negative"),
            });
        }
        if gross_micros % MICROS_PER_MINOR_UNIT != 0 {
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
            amount_minor_units: gross_micros / MICROS_PER_MINOR_UNIT,
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

/// The fields of Adyen's `/payments` (stored-method charge) response that
/// billing records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentReceipt {
    /// Adyen PSP reference of the charge.
    pub psp_reference: String,
    /// The accepted `resultCode` (`Authorised` / `Received`).
    pub result_code: String,
    /// A refreshed stored method id when the response carried one (network
    /// token updates); the caller may persist it over the previous record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_payment_method_id: Option<String>,
}

/// How one line's payment was collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CollectionChannel {
    /// The recorded card-on-file token was charged (`ContAuth`).
    StoredMethod,
    /// A payment link was created: the first payment, or the typed fallback.
    PaymentLink,
}

/// One line's collection outcome. A fallback carries the typed reason it
/// happened — a token refusal or the absence of a stored method — so the
/// collection log never shows a silent channel switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineCollection {
    /// Ledger sequence of the line billed.
    pub seq: u64,
    /// The deterministic reference (idempotency key) sent to Adyen.
    pub reference: String,
    /// Amount collected, in minor units.
    pub amount_minor_units: i64,
    /// ISO currency code.
    pub currency: String,
    /// The channel used.
    pub channel: CollectionChannel,
    /// PSP reference of an accepted stored-method charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psp_reference: Option<String>,
    /// The accepted `resultCode` of a stored-method charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_code: Option<String>,
    /// The created payment link on the link channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_link: Option<PaymentLinkResponse>,
    /// Why the link channel was used: the stored-charge refusal that fell
    /// back, or that no stored method is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

/// What [`collect_invoice`] did: the per-line channel outcomes and their
/// checked total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionReceipt {
    /// Publisher the invoice was built for.
    pub publisher_id: String,
    /// Billing period label (`YYYY-MM`).
    pub year_month: String,
    /// ISO currency code of all items.
    pub currency: String,
    /// Checked sum of the collected line amounts, in minor units.
    pub total_minor_units: i64,
    /// One outcome per non-zero line, in ledger `seq` order.
    pub lines: Vec<LineCollection>,
}

impl CollectionReceipt {
    /// The URLs of the created payment links (the fallback channel), in line
    /// order. A receipt collected entirely by token charge has none.
    pub fn link_urls(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter_map(|line| line.payment_link.as_ref().map(|link| link.url.as_str()))
            .collect()
    }

    /// Whether the receipt contains at least one payment link.
    pub fn has_links(&self) -> bool {
        self.lines
            .iter()
            .any(|line| line.channel == CollectionChannel::PaymentLink)
    }
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
/// caller composes the retry transport exactly like the connectors do. This
/// is the link-only collection; [`collect_invoice`] prefers the card on file
/// and falls back to these links.
pub async fn create_payment_links(
    client: &AdyenClient,
    transport: &dyn Transport,
    invoice: &crate::invoice::Invoice,
) -> Result<BillingReceipt, BillingError> {
    let mut links = Vec::new();
    let mut total_minor_units = 0i64;
    for line in &invoice.line_items {
        let billable = if line.gross_micros > 0 {
            line.gross_micros
        } else {
            line.fee_micros
        };
        if billable == 0 {
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

/// Collect one prepared line item: charge the stored card when a token is
/// known, otherwise create a payment link. A stored-charge *refusal* falls
/// back to a link with the typed reason recorded; every other failure
/// propagates (a transport failure may mean the charge landed, so falling
/// back could double-bill).
pub async fn collect_line(
    client: &AdyenClient,
    transport: &dyn Transport,
    item: &InvoiceLineItem,
    stored_payment_method_id: Option<&str>,
) -> Result<LineCollection, BillingError> {
    let mut base = LineCollection {
        seq: item.seq,
        reference: item.reference.clone(),
        amount_minor_units: item.amount_minor_units,
        currency: item.currency.clone(),
        channel: CollectionChannel::PaymentLink,
        psp_reference: None,
        result_code: None,
        payment_link: None,
        fallback_reason: None,
    };
    match stored_payment_method_id.filter(|id| !id.trim().is_empty()) {
        Some(stored_id) => match client
            .charge_stored_payment_method(transport, item, stored_id)
            .await
        {
            Ok(receipt) => {
                base.channel = CollectionChannel::StoredMethod;
                base.psp_reference = Some(receipt.psp_reference);
                base.result_code = Some(receipt.result_code);
                Ok(base)
            }
            Err(BillingError::StoredChargeRefused { detail }) => {
                // Typed fallback: the refusal travels on the line.
                base.payment_link = Some(client.create_payment_link(transport, item).await?);
                base.fallback_reason = Some(detail);
                Ok(base)
            }
            Err(other) => Err(other),
        },
        None => {
            base.payment_link = Some(client.create_payment_link(transport, item).await?);
            base.fallback_reason = Some("no stored payment method is recorded".to_string());
            Ok(base)
        }
    }
}

/// Collect one invoice: [`collect_line`] per non-zero line, in `seq` order,
/// with the checked total. A zero invoice sends no request at all.
pub async fn collect_invoice(
    client: &AdyenClient,
    transport: &dyn Transport,
    invoice: &crate::invoice::Invoice,
    stored_payment_method_id: Option<&str>,
) -> Result<CollectionReceipt, BillingError> {
    let mut lines = Vec::new();
    let mut total_minor_units = 0i64;
    for line in &invoice.line_items {
        let billable = if line.gross_micros > 0 {
            line.gross_micros
        } else {
            line.fee_micros
        };
        if billable == 0 {
            continue;
        }
        let item = InvoiceLineItem::from_invoice_line(
            line,
            &invoice.publisher_id,
            &invoice.period.year_month,
        )?;
        let collected = collect_line(client, transport, &item, stored_payment_method_id).await?;
        total_minor_units = total_minor_units
            .checked_add(item.amount_minor_units)
            .ok_or(BillingError::Overflow)?;
        lines.push(collected);
    }
    Ok(CollectionReceipt {
        publisher_id: invoice.publisher_id.clone(),
        year_month: invoice.period.year_month.clone(),
        currency: ADYEN_CURRENCY.to_string(),
        total_minor_units,
        lines,
    })
}
