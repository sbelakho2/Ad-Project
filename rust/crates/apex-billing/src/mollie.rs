//! Mollie billing client: payment links, first/recurring payments, and the
//! server-side webhook fetch that replaces provider signatures.
//!
//! ## Webhooks
//!
//! Mollie does not sign webhooks. A delivery is only a form-encoded `id`
//! (`application/x-www-form-urlencoded`, `id=tr_...`). The body is therefore
//! untrusted input: [`MollieClient::fetch_payment`] fetches the payment from
//! Mollie with the API key, and [`crate::webhook`] validates the fetched
//! status, amount, currency and metadata before anything is recorded. A
//! forged or unknown id cannot become a payment because the fetch fails
//! closed. Replay protection deduplicates on the fetched `(payment id, status)`
//! pair, so Mollie's retries never activate a subscriber twice.
//!
//! ## Collection
//!
//! Invoice lines are collected with payment links
//! (`POST /v2/payment-links`). The deterministic reference travels in
//! `metadata.reference` and as the `Idempotency-Key` header, so a resumed run
//! returns the same link instead of double billing. The setup-watch first
//! payment is a Mollie payment (`POST /v2/payments`, `sequenceType: "first"`)
//! created for a customer, which stores the mandate; later months charge that
//! mandate with `sequenceType: "recurring"`. A refused recurring charge is a
//! typed [`BillingError::StoredChargeRefused`] and the caller falls back to a
//! payment link. Anything uncertain (network, timeout, 5xx) propagates: a
//! fallback after an uncertain charge could double-bill.
//!
//! Every request carries the [`PlatformId::Billing`] domain label.

use serde::{Deserialize, Serialize};

use apex_connectors::transport::{HttpBody, HttpMethod, HttpRequest, Transport};
use apex_core::PlatformId;

use crate::errors::BillingError;
use crate::invoice::InvoiceLine;

/// The platform/domain label every billing request and error carries.
pub const BILLING_PLATFORM: PlatformId = PlatformId::Billing;

/// The statuses Mollie reports for a payment. A webhook for any other status
/// is a malformed payload, not a silently ignored one.
pub const KNOWN_PAYMENT_STATUSES: [&str; 9] = [
    "open",
    "pending",
    "authorized",
    "paid",
    "canceled",
    "expired",
    "failed",
    "refunded",
    "charged_back",
];

/// The status that means the money was collected.
pub const COLLECTED_PAYMENT_STATUS: &str = "paid";

/// The currency invoices are billed in: fees are USD micros, sent to Mollie as
/// a decimal string with two places.
pub const BILLING_CURRENCY: &str = "USD";

/// Micros per minor unit (cent) for USD. The conversion must be exact, so a
/// fee that is not a whole number of cents is refused rather than rounded.
pub const MICROS_PER_MINOR_UNIT: i64 = 10_000;

/// The number of decimal places in [`BILLING_CURRENCY`].
pub const CURRENCY_EXPONENT: u32 = 2;

/// Mollie's metadata is bounded; the deterministic reference is capped lower
/// so an over-long reference is a local, typed error rather than a remote 422.
pub const MAX_REFERENCE_LEN: usize = 80;

/// The default Mollie API origin.
pub const DEFAULT_MOLLIE_API_BASE: &str = "https://api.mollie.com";

/// Whether an HTTP status is an accepted Mollie success.
fn is_success_status(status: u16) -> bool {
    (200..300).contains(&status)
}

fn operation_error(operation: &str, status: u16) -> BillingError {
    match status {
        401 | 403 => BillingError::from(apex_core::PlatformError::Auth {
            platform: BILLING_PLATFORM,
            operation: operation.to_string(),
            status,
        }),
        404 => BillingError::from(apex_core::PlatformError::Http {
            platform: BILLING_PLATFORM,
            operation: operation.to_string(),
            status,
        }),
        429 => BillingError::from(apex_core::PlatformError::RateLimited {
            platform: BILLING_PLATFORM,
            operation: operation.to_string(),
            retry_after: None,
        }),
        other => BillingError::from(apex_core::PlatformError::Http {
            platform: BILLING_PLATFORM,
            operation: operation.to_string(),
            status: other,
        }),
    }
}

async fn send(
    transport: &dyn Transport,
    operation: &str,
    request: HttpRequest,
) -> Result<Vec<u8>, BillingError> {
    let response = match transport.request(request).await {
        Ok(response) => response,
        Err(apex_connectors::transport::TransportError::Platform(error)) => {
            return Err(BillingError::from(error))
        }
        Err(apex_connectors::transport::TransportError::Network(detail)) => {
            return Err(BillingError::from(apex_core::PlatformError::Network {
                platform: BILLING_PLATFORM,
                operation: operation.to_string(),
                detail,
            }))
        }
        Err(apex_connectors::transport::TransportError::Timeout(detail)) => {
            return Err(BillingError::from(apex_core::PlatformError::Network {
                platform: BILLING_PLATFORM,
                operation: operation.to_string(),
                detail: format!("timeout: {detail}"),
            }))
        }
    };
    if !is_success_status(response.status) {
        return Err(operation_error(operation, response.status));
    }
    Ok(response.body)
}

/// Format a non-negative minor-unit amount as Mollie's decimal string. USD
/// carries exactly two decimals; a negative or overflowing amount is refused.
pub fn format_amount(minor_units: i64) -> Result<String, BillingError> {
    if minor_units < 0 {
        return Err(BillingError::Malformed(format!(
            "amount {minor_units} minor units is negative"
        )));
    }
    let divisor = 10i64.pow(CURRENCY_EXPONENT);
    let major = minor_units / divisor;
    let minor = minor_units % divisor;
    Ok(format!("{major}.{minor:0width$}", width = CURRENCY_EXPONENT as usize))
}

/// Parse Mollie's decimal amount string into minor units. Only a non-negative
/// plain decimal with exactly [`CURRENCY_EXPONENT`] places is accepted: an
/// exponent, sign, comma or missing places is a typed refusal.
pub fn parse_amount(value: &str) -> Result<i64, BillingError> {
    let value = value.trim();
    let (major_text, minor_text) = value.split_once('.').ok_or_else(|| {
        BillingError::Malformed(format!("amount {value:?} is not a decimal string"))
    })?;
    if major_text.is_empty() {
        return Err(BillingError::Malformed(format!(
            "amount {value:?} has no integer part"
        )));
    }
    if major_text.len() > 15 {
        return Err(BillingError::Malformed(format!(
            "amount {value:?} has an integer part longer than 15 digits"
        )));
    }
    if !major_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BillingError::Malformed(format!(
            "amount {value:?} has a non-digit integer part"
        )));
    }
    if minor_text.len() != CURRENCY_EXPONENT as usize {
        return Err(BillingError::Malformed(format!(
            "amount {value:?} must have {CURRENCY_EXPONENT} decimal places"
        )));
    }
    if !minor_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BillingError::Malformed(format!(
            "amount {value:?} has non-digit decimal places"
        )));
    }
    let major: i64 = major_text
        .parse()
        .map_err(|_| BillingError::Malformed(format!("amount {value:?} is out of range")))?;
    let minor: i64 = minor_text
        .parse()
        .map_err(|_| BillingError::Malformed(format!("amount {value:?} is out of range")))?;
    major
        .checked_mul(10i64.pow(CURRENCY_EXPONENT))
        .and_then(|scaled| scaled.checked_add(minor))
        .ok_or_else(|| BillingError::Malformed(format!("amount {value:?} overflows minor units")))
}

/// Refuse a line item locally instead of letting Mollie accept a different
/// meaning (or answer with a remote 422).
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
            "invoice line seq {} has an unusable billing reference (length {}, control characters or empty)",
            item.seq,
            item.reference.len()
        )));
    }
    if item.customer_reference.trim().is_empty()
        || item.customer_reference.chars().any(char::is_control)
    {
        return Err(BillingError::Malformed(format!(
            "invoice line seq {} has an unusable publisher reference",
            item.seq
        )));
    }
    Ok(())
}

/// A configuration refusal while building a [`MollieClient`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MollieClientError {
    /// The API key was empty: every call would fail closed anyway, but the
    /// misconfiguration must be refused at construction.
    #[error("mollie api key must not be empty")]
    EmptyApiKey,

    /// The API base was empty or not an absolute http(s) origin.
    #[error("mollie api base must be an absolute http(s) URL")]
    EmptyApiBase,

    /// The webhook URL was empty: Mollie would have no delivery target.
    #[error("mollie webhook url must not be empty")]
    EmptyWebhookUrl,

    /// The redirect URL was empty: the shopper would have no return page.
    #[error("mollie redirect url must not be empty")]
    EmptyRedirectUrl,
}

/// Mollie API client: bearer credentials, the API origin and the URLs every
/// payment carries. I/O only goes through the shared transport.
#[derive(Clone)]
pub struct MollieClient {
    api_key: String,
    api_base: String,
    webhook_url: String,
    redirect_url: String,
}

impl std::fmt::Debug for MollieClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MollieClient")
            .field("api_key", &"[redacted]")
            .field("api_base", &self.api_base)
            .field("webhook_url", &self.webhook_url)
            .field("redirect_url", &self.redirect_url)
            .finish()
    }
}

impl MollieClient {
    pub fn new(
        api_key: impl Into<String>,
        api_base: impl Into<String>,
        webhook_url: impl Into<String>,
        redirect_url: impl Into<String>,
    ) -> Result<Self, MollieClientError> {
        let api_key = api_key.into();
        let api_base = api_base.into();
        let webhook_url = webhook_url.into();
        let redirect_url = redirect_url.into();
        if api_key.trim().is_empty() {
            return Err(MollieClientError::EmptyApiKey);
        }
        let api_base = api_base.trim().trim_end_matches('/').to_string();
        if !(api_base.starts_with("https://") || api_base.starts_with("http://"))
            || api_base.len() <= "https://".len()
        {
            return Err(MollieClientError::EmptyApiBase);
        }
        if webhook_url.trim().is_empty() {
            return Err(MollieClientError::EmptyWebhookUrl);
        }
        if redirect_url.trim().is_empty() {
            return Err(MollieClientError::EmptyRedirectUrl);
        }
        Ok(Self {
            api_key,
            api_base,
            webhook_url,
            redirect_url,
        })
    }

    /// The configured API key.
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// The configured API base.
    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    /// The configured webhook URL.
    pub fn webhook_url(&self) -> &str {
        &self.webhook_url
    }

    /// The configured shopper return URL.
    pub fn redirect_url(&self) -> &str {
        &self.redirect_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    fn headers(&self, idempotency_key: Option<&str>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Authorization".into(), format!("Bearer {}", self.api_key)),
            ("Accept".into(), "application/hal+json".into()),
            ("Content-Type".into(), "application/json".into()),
        ];
        if let Some(key) = idempotency_key {
            headers.push(("Idempotency-Key".into(), key.to_string()));
        }
        headers
    }

    fn payment_body(&self, item: &InvoiceLineItem) -> Result<serde_json::Value, BillingError> {
        let amount = format_amount(item.amount_minor_units)?;
        Ok(serde_json::json!({
            "amount": {"currency": item.currency, "value": amount},
            "description": item.description,
            "redirectUrl": self.redirect_url,
            "webhookUrl": self.webhook_url,
            "metadata": {
                "reference": item.reference,
                "publisher": item.customer_reference,
                "amount": amount,
            },
        }))
    }

    /// Create the Mollie customer a recurring mandate belongs to. The
    /// deterministic reference is the idempotency key, so a resumed first
    /// payment returns the same customer instead of creating a second one.
    pub async fn create_customer(
        &self,
        transport: &dyn Transport,
        reference: &str,
    ) -> Result<String, BillingError> {
        if reference.trim().is_empty() {
            return Err(BillingError::Malformed(
                "customer reference must not be empty".to_string(),
            ));
        }
        let body = serde_json::json!({ "name": reference });
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "createCustomer".to_string(),
            method: HttpMethod::Post,
            url: self.url("/v2/customers"),
            headers: self.headers(Some(&format!("customer:{reference}"))),
            body: Some(HttpBody::Json(body)),
            timeout_ms: 30_000,
        };
        let raw = send(transport, "createCustomer", request).await?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            BillingError::Malformed(format!("customer response is not JSON: {error}"))
        })?;
        let id = parsed
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| BillingError::Malformed("customer response missing id".to_string()))?;
        Ok(id.to_string())
    }

    /// Create a first payment for a customer (`sequenceType: "first"`), which
    /// is what makes Mollie store the mandate later months charge. Returns the
    /// payment id and the shopper checkout URL.
    pub async fn create_first_payment(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
    ) -> Result<PaymentLinkResponse, BillingError> {
        validate_line_item(item)?;
        let customer_id = self.create_customer(transport, &item.customer_reference).await?;
        let mut body = self.payment_body(item)?;
        body["customerId"] = serde_json::Value::String(customer_id.clone());
        body["sequenceType"] = serde_json::Value::String("first".to_string());
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "createFirstPayment".to_string(),
            method: HttpMethod::Post,
            url: self.url("/v2/payments"),
            headers: self.headers(Some(&item.reference)),
            body: Some(HttpBody::Json(body)),
            timeout_ms: 30_000,
        };
        let raw = send(transport, "createFirstPayment", request).await?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            BillingError::Malformed(format!("first payment response is not JSON: {error}"))
        })?;
        let id = required_text(&parsed, "id", "first payment response missing id")?;
        let url = checkout_url(&parsed).ok_or_else(|| {
            BillingError::Malformed("first payment response missing checkout URL".to_string())
        })?;
        Ok(PaymentLinkResponse {
            id,
            url,
            customer_id: Some(customer_id),
        })
    }

    /// Create one payment link for one non-zero invoice line. The reference is
    /// the deterministic idempotency key and travels in metadata, so a retried
    /// month returns the same link instead of double billing.
    pub async fn create_payment_link(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
    ) -> Result<PaymentLinkResponse, BillingError> {
        validate_line_item(item)?;
        let body = self.payment_body(item)?;
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "createPaymentLink".to_string(),
            method: HttpMethod::Post,
            url: self.url("/v2/payment-links"),
            headers: self.headers(Some(&item.reference)),
            body: Some(HttpBody::Json(body)),
            timeout_ms: 30_000,
        };
        let raw = send(transport, "createPaymentLink", request).await?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            BillingError::Malformed(format!("payment link response is not JSON: {error}"))
        })?;
        let id = required_text(&parsed, "id", "payment link response missing id")?;
        let url = parsed
            .get("_links")
            .and_then(|links| links.get("paymentLink"))
            .and_then(|link| match link {
                serde_json::Value::String(url) => Some(url.clone()),
                value => value.get("href").and_then(|href| href.as_str()).map(str::to_string),
            })
            .filter(|url| !url.trim().is_empty())
            .ok_or_else(|| {
                BillingError::Malformed("payment link response missing paymentLink URL".to_string())
            })?;
        Ok(PaymentLinkResponse {
            id,
            url,
            customer_id: None,
        })
    }

    /// Charge a stored mandate (`customerId` + `mandateId`,
    /// `sequenceType: "recurring"`). A payment Mollie does not report as paid
    /// is the typed [`BillingError::StoredChargeRefused`] the collection falls
    /// back from. Transport, auth, rate-limit and HTTP failures propagate.
    pub async fn charge_stored_payment_method(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        customer_id: &str,
        mandate_id: &str,
    ) -> Result<PaymentReceipt, BillingError> {
        validate_line_item(item)?;
        let customer_id = customer_id.trim();
        let mandate_id = mandate_id.trim();
        if customer_id.is_empty() || mandate_id.is_empty() {
            return Err(BillingError::Malformed(
                "mollie customer id and mandate id must not be empty".to_string(),
            ));
        }
        let mut body = self.payment_body(item)?;
        body["customerId"] = serde_json::Value::String(customer_id.to_string());
        body["mandateId"] = serde_json::Value::String(mandate_id.to_string());
        body["sequenceType"] = serde_json::Value::String("recurring".to_string());
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "chargeStoredPaymentMethod".to_string(),
            method: HttpMethod::Post,
            url: self.url("/v2/payments"),
            headers: self.headers(Some(&item.reference)),
            body: Some(HttpBody::Json(body)),
            timeout_ms: 30_000,
        };
        let raw = send(transport, "chargeStoredPaymentMethod", request).await?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            BillingError::Malformed(format!("payments response is not JSON: {error}"))
        })?;
        let payment = MolliePayment::from_json(&parsed)?;
        if payment.metadata_reference.as_deref() != Some(item.reference.as_str()) {
            return Err(BillingError::Malformed(
                "payments response metadata reference does not match the request".to_string(),
            ));
        }
        if payment.amount_minor_units != item.amount_minor_units
            || payment.currency != item.currency
        {
            return Err(BillingError::Malformed(
                "payments response amount does not match the request".to_string(),
            ));
        }
        if payment.status != COLLECTED_PAYMENT_STATUS {
            return Err(BillingError::StoredChargeRefused {
                detail: format!("status {}", payment.status),
            });
        }
        Ok(PaymentReceipt {
            payment_id: payment.id,
            status: payment.status,
            amount_minor_units: payment.amount_minor_units,
            currency: payment.currency,
            customer_id: payment.customer_id,
            mandate_id: payment.mandate_id,
        })
    }

    /// Fetch one payment by id. This is the webhook's only source of truth:
    /// the POST body carries no signature and is never trusted.
    pub async fn fetch_payment(
        &self,
        transport: &dyn Transport,
        payment_id: &str,
    ) -> Result<MolliePayment, BillingError> {
        let payment_id = payment_id.trim();
        if !is_payment_id(payment_id) {
            return Err(BillingError::Malformed(
                "payment id must match tr_ followed by letters or digits".to_string(),
            ));
        }
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "fetchPayment".to_string(),
            method: HttpMethod::Get,
            url: self.url(&format!("/v2/payments/{payment_id}")),
            headers: self.headers(None),
            body: None,
            timeout_ms: 30_000,
        };
        let raw = send(transport, "fetchPayment", request).await?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            BillingError::Malformed(format!("fetched payment is not JSON: {error}"))
        })?;
        let payment = MolliePayment::from_json(&parsed)?;
        if payment.id != payment_id {
            return Err(BillingError::Malformed(
                "fetched payment id does not match the requested id".to_string(),
            ));
        }
        Ok(payment)
    }
}

fn required_text(
    value: &serde_json::Value,
    key: &str,
    detail: &str,
) -> Result<String, BillingError> {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .ok_or_else(|| BillingError::Malformed(detail.to_string()))
}

fn checkout_url(value: &serde_json::Value) -> Option<String> {
    value
        .get("_links")
        .and_then(|links| links.get("checkout"))
        .and_then(|checkout| checkout.get("href"))
        .and_then(|href| href.as_str())
        .map(str::to_string)
        .filter(|url| !url.trim().is_empty())
}

/// A Mollie payment id: `tr_` followed by letters and digits.
pub fn is_payment_id(value: &str) -> bool {
    value
        .strip_prefix("tr_")
        .is_some_and(|rest| {
            !rest.is_empty()
                && rest.len() <= 64
                && rest
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

/// A fetched Mollie payment, reduced to the fields billing validates. The
/// raw body is deliberately not kept: nothing downstream can accidentally
/// trust a delivery's body over this fetched object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MolliePayment {
    /// Mollie payment id (`tr_...`).
    pub id: String,
    /// Mollie status (`open`, `paid`, `failed`, ...).
    pub status: String,
    /// Amount in minor units, parsed from Mollie's decimal string.
    pub amount_minor_units: i64,
    /// ISO currency code.
    pub currency: String,
    /// `metadata.reference`, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_reference: Option<String>,
    /// `metadata.publisher`, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_publisher: Option<String>,
    /// `metadata.amount`, when present, parsed to minor units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_amount_minor_units: Option<i64>,
    /// `customerId` for recurring payments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,
    /// `mandateId` once a mandate exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mandate_id: Option<String>,
    /// `sequenceType` (`first`, `recurring`, `oneoff`), when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence_type: Option<String>,
}

impl MolliePayment {
    /// Parse the documented Mollie payment shape, failing closed on anything
    /// billing cannot validate.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, BillingError> {
        let id = required_text(value, "id", "payment missing id")?;
        if !is_payment_id(&id) {
            return Err(BillingError::Malformed(format!(
                "payment id {id:?} is not a Mollie payment id"
            )));
        }
        let status = required_text(value, "status", "payment missing status")?;
        if !KNOWN_PAYMENT_STATUSES.contains(&status.as_str()) {
            return Err(BillingError::Malformed(format!(
                "payment status {status:?} is not a known Mollie status"
            )));
        }
        let amount = value
            .get("amount")
            .ok_or_else(|| BillingError::Malformed("payment missing amount".to_string()))?;
        let amount_text = required_text(amount, "value", "payment amount missing value")?;
        let amount_minor_units = parse_amount(&amount_text)?;
        let currency = required_text(amount, "currency", "payment amount missing currency")?;
        if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(BillingError::Malformed(format!(
                "payment amount currency {currency:?} is not an ISO code"
            )));
        }
        let metadata = value.get("metadata");
        let metadata_reference = metadata
            .and_then(|item| item.get("reference"))
            .and_then(|item| item.as_str())
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string);
        let metadata_publisher = metadata
            .and_then(|item| item.get("publisher"))
            .and_then(|item| item.as_str())
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string);
        let metadata_amount_minor_units = match metadata
            .and_then(|item| item.get("amount"))
            .and_then(|item| item.as_str())
        {
            Some(text) => Some(parse_amount(text)?),
            None => None,
        };
        let optional = |key: &str| {
            value
                .get(key)
                .and_then(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
        };
        Ok(Self {
            id,
            status,
            amount_minor_units,
            currency,
            metadata_reference,
            metadata_publisher,
            metadata_amount_minor_units,
            customer_id: optional("customerId"),
            mandate_id: optional("mandateId"),
            sequence_type: optional("sequenceType"),
        })
    }

    /// Whether the fetched payment collected the expected money.
    pub fn is_collected(&self) -> bool {
        self.status == COLLECTED_PAYMENT_STATUS
    }
}

/// A stored recurring mandate. The encoded form is
/// `customerId:mandateId`, which is what the subscriber row persists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMandate {
    pub customer_id: String,
    pub mandate_id: String,
}

impl StoredMandate {
    pub fn new(customer_id: impl Into<String>, mandate_id: impl Into<String>) -> Option<Self> {
        let customer_id = customer_id.into().trim().to_string();
        let mandate_id = mandate_id.into().trim().to_string();
        if customer_id.is_empty()
            || mandate_id.is_empty()
            || customer_id.contains(':')
            || mandate_id.contains(':')
        {
            return None;
        }
        Some(Self {
            customer_id,
            mandate_id,
        })
    }

    /// Parse the encoded `customerId:mandateId` form.
    pub fn parse(value: &str) -> Option<Self> {
        let (customer_id, mandate_id) = value.trim().split_once(':')?;
        Self::new(customer_id, mandate_id)
    }

    /// Encode for the subscriber row's single stored-mandate field.
    pub fn encode(&self) -> String {
        format!("{}:{}", self.customer_id, self.mandate_id)
    }
}

/// One new-payment request derived from a verified invoice line. The
/// reference is the deterministic idempotency key:
/// `apex-fee:<publisher>:<experiment>:<YYYY-MM>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLineItem {
    /// Ledger sequence of the billing proof this item bills.
    pub seq: u64,
    /// Experiment the proven lift belongs to.
    pub experiment_id: String,
    /// Amount in minor units (USD cents).
    pub amount_minor_units: i64,
    /// ISO currency code (always [`BILLING_CURRENCY`] here).
    pub currency: String,
    /// Publisher reference carried in metadata.
    pub customer_reference: String,
    /// Deterministic reference (idempotency key) sent with the request.
    pub reference: String,
    /// Human-readable description shown on the payment page.
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
        let customer_reference = format!("publisher-{publisher_id}");
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
            currency: BILLING_CURRENCY.to_string(),
            customer_reference,
            reference,
            description: format!(
                "Apex proven-lift fee: experiment {} ({} bp of {} micros proven lift)",
                line.experiment_id, line.rate_bp, line.proven_lift
            ),
        })
    }
}

/// The fields of a Mollie payment-link response that billing records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentLinkResponse {
    /// Mollie link id (`pl_...`) or first-payment id (`tr_...`).
    pub id: String,
    /// Shopper-facing URL to complete the payment.
    pub url: String,
    /// The customer created for a first payment, when one was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,
}

/// The fields of a Mollie `/v2/payments` (stored-mandate charge) response
/// that billing records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentReceipt {
    /// Mollie payment id of the charge.
    pub payment_id: String,
    /// The collected status (`paid`).
    pub status: String,
    /// Amount collected, in minor units.
    pub amount_minor_units: i64,
    /// ISO currency code.
    pub currency: String,
    /// The customer the recurring payment ran for, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,
    /// The mandate the recurring payment used, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mandate_id: Option<String>,
}

/// How one line's payment was collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CollectionChannel {
    /// A stored mandate was charged (`recurring`).
    StoredMandate,
    /// A payment link was created: the first payment, or the typed fallback.
    PaymentLink,
}

/// One line's collection outcome. A fallback carries the typed reason it
/// happened — a mandate refusal or the absence of a stored mandate — so the
/// collection log never shows a silent channel switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineCollection {
    /// Ledger sequence of the line billed.
    pub seq: u64,
    /// The deterministic reference (idempotency key) sent to Mollie.
    pub reference: String,
    /// Amount collected, in minor units.
    pub amount_minor_units: i64,
    /// ISO currency code.
    pub currency: String,
    /// The channel used.
    pub channel: CollectionChannel,
    /// Mollie payment id of an accepted recurring charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_id: Option<String>,
    /// The collected status of a recurring charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// The created payment link on the link channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_link: Option<PaymentLinkResponse>,
    /// Why the link channel was used: the charge refusal that fell back, or
    /// that no stored mandate is recorded.
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
    /// order. A receipt collected entirely by mandate has none.
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
/// no request at all and yields an empty receipt with a zero total.
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

/// Create one Mollie payment link per non-zero invoice line, in `seq` order.
///
/// Every request goes through the shared [`Transport`] with bearer auth; the
/// link reference is the line's deterministic idempotency key. Refusals are
/// typed and never retried at this layer.
pub async fn create_payment_links(
    client: &MollieClient,
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
        currency: BILLING_CURRENCY.to_string(),
        total_minor_units,
        links,
    })
}

/// Collect one prepared line item: charge the stored mandate when one is
/// known, otherwise create a payment link. A recurring *refusal* falls back
/// to a link with the typed reason recorded; every other failure propagates
/// (a transport failure may mean the charge landed, so falling back could
/// double-bill).
pub async fn collect_line(
    client: &MollieClient,
    transport: &dyn Transport,
    item: &InvoiceLineItem,
    stored_mandate: Option<&str>,
) -> Result<LineCollection, BillingError> {
    let mut base = LineCollection {
        seq: item.seq,
        reference: item.reference.clone(),
        amount_minor_units: item.amount_minor_units,
        currency: item.currency.clone(),
        channel: CollectionChannel::PaymentLink,
        payment_id: None,
        status: None,
        payment_link: None,
        fallback_reason: None,
    };
    match stored_mandate
        .and_then(StoredMandate::parse)
        .filter(|mandate| !mandate.customer_id.is_empty() && !mandate.mandate_id.is_empty())
    {
        Some(mandate) => match client
            .charge_stored_payment_method(
                transport,
                item,
                &mandate.customer_id,
                &mandate.mandate_id,
            )
            .await
        {
            Ok(receipt) => {
                base.channel = CollectionChannel::StoredMandate;
                base.payment_id = Some(receipt.payment_id);
                base.status = Some(receipt.status);
                Ok(base)
            }
            Err(BillingError::StoredChargeRefused { detail }) => {
                base.payment_link = Some(client.create_payment_link(transport, item).await?);
                base.fallback_reason = Some(detail);
                Ok(base)
            }
            Err(other) => Err(other),
        },
        None => {
            base.payment_link = Some(client.create_payment_link(transport, item).await?);
            base.fallback_reason = Some("no stored payment mandate is recorded".to_string());
            Ok(base)
        }
    }
}

/// Collect one invoice: [`collect_line`] per non-zero line, in `seq` order,
/// with the checked total. A zero invoice sends no request at all.
pub async fn collect_invoice(
    client: &MollieClient,
    transport: &dyn Transport,
    invoice: &crate::invoice::Invoice,
    stored_mandate: Option<&str>,
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
        let collected = collect_line(client, transport, &item, stored_mandate).await?;
        total_minor_units = total_minor_units
            .checked_add(item.amount_minor_units)
            .ok_or(BillingError::Overflow)?;
        lines.push(collected);
    }
    Ok(CollectionReceipt {
        publisher_id: invoice.publisher_id.clone(),
        year_month: invoice.period.year_month.clone(),
        currency: BILLING_CURRENCY.to_string(),
        total_minor_units,
        lines,
    })
}
