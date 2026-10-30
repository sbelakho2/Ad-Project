//! Mollie billing client: checkout payments, first/recurring payments, and the
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
//! An invoice is one receivable: [`InvoiceLineItem::for_invoice_total`] builds
//! the single request that pays the whole invoice total, and the durable intent
//! saga submits it exactly once — one first payment (`sequenceType: "first"`)
//! when no mandate is stored, or one stored-mandate charge
//! (`sequenceType: "recurring"`) when one is. It is never one checkout per
//! line *plus* an invoice-level payment link: two payable objects for one
//! invoice would let the same total be collected twice. The deterministic
//! invoice reference travels in `metadata.reference` and as the
//! `Idempotency-Key` header, so a resumed run returns the same payment instead
//! of double billing. The watch has no Mollie payment; only Apex fee invoices
//! create payments. A recurring charge that Mollie reports as `failed`,
//! `canceled` or `expired` is a typed [`BillingError::StoredChargeRefused`] and
//! the caller falls back to a checkout payment; `open` and `pending` are
//! submitted charges and wait for the webhook. Anything uncertain (network,
//! timeout, 5xx) propagates: a fallback after an uncertain charge could
//! double-bill.
//!
//! [`MollieClient::create_invoice_payment_link`] still exists for a durable
//! Payment Links API URL an operator or an older journal recorded, but the
//! collection path does not create one alongside the receivable's payment. A
//! first payment (`sequenceType: "first"`,
//! [`MollieClient::create_first_payment`]) stores the mandate later months
//! charge through [`MollieClient::charge_stored_payment_method_key`].
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
    Ok(format!(
        "{major}.{minor:0width$}",
        width = CURRENCY_EXPONENT as usize
    ))
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

    #[error("mollie api base must be https unless it is loopback")]
    InsecureApiBase,

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
        if api_base.starts_with("http://") {
            let host = api_base
                .trim_start_matches("http://")
                .split(['/', '?', '#'])
                .next()
                .unwrap_or_default();
            let host = host
                .strip_prefix('[')
                .and_then(|h| h.split(']').next())
                .unwrap_or(host);
            let host = host.split(':').next().unwrap_or(host);
            if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
                return Err(MollieClientError::InsecureApiBase);
            }
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
        self.create_first_payment_key(transport, item, &item.reference)
            .await
    }

    /// Create a first payment under an explicit Idempotency-Key: a resumed
    /// month must return the same first payment instead of opening a second
    /// one. The response carries the created customer id, so the caller can
    /// remember who the mandate will belong to.
    pub async fn create_first_payment_key(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        idempotency_key: &str,
    ) -> Result<PaymentLinkResponse, BillingError> {
        validate_line_item(item)?;
        let customer_id = self
            .create_customer(transport, &item.customer_reference)
            .await?;
        let mut body = self.payment_body(item)?;
        body["customerId"] = serde_json::Value::String(customer_id.clone());
        body["sequenceType"] = serde_json::Value::String("first".to_string());
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "createFirstPayment".to_string(),
            method: HttpMethod::Post,
            url: self.url("/v2/payments"),
            headers: self.headers(Some(idempotency_key)),
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

    /// Create one Mollie **payment link** (`POST /v2/payment-links`) for a
    /// whole invoice's total. Checkout payments expire within hours for most
    /// methods; a payment link does not, so it is the durable URL an invoice
    /// email and the console can hand the customer. The deterministic
    /// `apex-invoice:<publisher>:<period>` reference is the Idempotency-Key
    /// and travels in metadata, so a resumed run returns the same link
    /// instead of a second one.
    pub async fn create_invoice_payment_link(
        &self,
        transport: &dyn Transport,
        publisher_id: &str,
        year_month: &str,
        amount_minor_units: i64,
        currency: &str,
        description: &str,
    ) -> Result<PaymentLinkResponse, BillingError> {
        let reference = invoice_link_reference(publisher_id, year_month)?;
        if amount_minor_units <= 0 {
            return Err(BillingError::Malformed(format!(
                "invoice payment link amount {amount_minor_units} minor units is not positive"
            )));
        }
        let amount = format_amount(amount_minor_units)?;
        if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(BillingError::Malformed(format!(
                "invoice payment link currency {currency:?} is not an ISO code"
            )));
        }
        if description.trim().is_empty() || description.chars().any(char::is_control) {
            return Err(BillingError::Malformed(
                "invoice payment link description is empty or carries control characters"
                    .to_string(),
            ));
        }
        let customer_reference = format!("publisher-{publisher_id}");
        if customer_reference.chars().any(char::is_control) {
            return Err(BillingError::UnsafeIdentifier {
                seq: 0,
                detail: "publisher reference contains control characters".to_string(),
            });
        }
        let body = serde_json::json!({
            "amount": {"currency": currency, "value": amount},
            "description": description,
            "redirectUrl": self.redirect_url,
            "webhookUrl": self.webhook_url,
            "metadata": {
                "reference": reference,
                "publisher": customer_reference,
                "amount": amount,
            },
        });
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "createPaymentLink".to_string(),
            method: HttpMethod::Post,
            url: self.url("/v2/payment-links"),
            headers: self.headers(Some(&reference)),
            body: Some(HttpBody::Json(body)),
            timeout_ms: 30_000,
        };
        let raw = send(transport, "createPaymentLink", request).await?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            BillingError::Malformed(format!("payment link response is not JSON: {error}"))
        })?;
        let id = required_text(&parsed, "id", "payment link response missing id")?;
        // A payment link answers `_links.paymentUrl`; tolerate the checkout
        // shape so a simulator that answers with a payment document still
        // yields the shopper URL.
        let url = payment_link_url(&parsed).ok_or_else(|| {
            BillingError::Malformed("payment link response missing payment URL".to_string())
        })?;
        Ok(PaymentLinkResponse {
            id,
            url,
            customer_id: None,
        })
    }

    /// Create one checkout payment for one non-zero invoice line. The reference is
    /// the deterministic idempotency key and travels in metadata, so a retried
    /// month returns the same link instead of double billing.
    pub async fn create_payment_link(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
    ) -> Result<PaymentLinkResponse, BillingError> {
        self.create_payment_link_key(transport, item, &item.reference)
            .await
    }

    /// Create a checkout payment under an explicit Idempotency-Key: a retry
    /// after a definitive failure must not replay the failed response Mollie
    /// would return for the original reference.
    pub async fn create_payment_link_key(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        idempotency_key: &str,
    ) -> Result<PaymentLinkResponse, BillingError> {
        validate_line_item(item)?;
        let body = self.payment_body(item)?;
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "createCheckoutPayment".to_string(),
            method: HttpMethod::Post,
            url: self.url("/v2/payments"),
            headers: self.headers(Some(idempotency_key)),
            body: Some(HttpBody::Json(body)),
            timeout_ms: 30_000,
        };
        let raw = send(transport, "createCheckoutPayment", request).await?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            BillingError::Malformed(format!("checkout payment response is not JSON: {error}"))
        })?;
        let id = required_text(&parsed, "id", "checkout payment response missing id")?;
        let url = parsed
            .get("_links")
            .and_then(|links| links.get("checkout"))
            .and_then(|link| match link {
                serde_json::Value::String(url) => Some(url.clone()),
                value => value
                    .get("href")
                    .and_then(|href| href.as_str())
                    .map(str::to_string),
            })
            .filter(|url| !url.trim().is_empty())
            .ok_or_else(|| {
                BillingError::Malformed("payment response missing checkout URL".to_string())
            })?;
        Ok(PaymentLinkResponse {
            id,
            url,
            customer_id: None,
        })
    }

    /// Charge a stored mandate (`customerId` + `mandateId`,
    /// `sequenceType: "recurring"`). A charge Mollie reports as `failed`,
    /// `canceled` or `expired` is the typed [`BillingError::StoredChargeRefused`]
    /// the collection falls back from. `open` and `pending` are *submitted*
    /// charges (SEPA debits settle over days), not refusals: the caller
    /// records the provider id and waits for the webhook. Transport, auth,
    /// rate-limit and HTTP failures propagate.
    pub async fn charge_stored_payment_method(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        customer_id: &str,
        mandate_id: &str,
    ) -> Result<PaymentReceipt, BillingError> {
        self.charge_stored_payment_method_key(
            transport,
            item,
            customer_id,
            mandate_id,
            &item.reference,
        )
        .await
    }

    /// Charge a stored payment method under an explicit Idempotency-Key: a
    /// retry after a definitive refusal must see the customer's current
    /// mandate instead of Mollie's replayed refusal.
    #[allow(clippy::too_many_arguments)]
    pub async fn charge_stored_payment_method_key(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        customer_id: &str,
        mandate_id: &str,
        idempotency_key: &str,
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
            headers: self.headers(Some(idempotency_key)),
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
        if matches!(payment.status.as_str(), "failed" | "canceled" | "expired") {
            return Err(BillingError::StoredChargeRefused {
                detail: format!("status {}", payment.status),
                payment_id: Some(payment.id),
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

    /// Fetch every payment made against one payment link
    /// (`GET /v2/payment-links/{id}/payments`). The dunning sweep and the
    /// reconciliation pass use it to see whether a durable link has since
    /// been paid. A response without the embedded payments list is a typed
    /// refusal, never a silently empty list.
    pub async fn fetch_payment_link_payments(
        &self,
        transport: &dyn Transport,
        link_id: &str,
    ) -> Result<Vec<MolliePayment>, BillingError> {
        let link_id = link_id.trim();
        if !is_payment_link_id(link_id) {
            return Err(BillingError::Malformed(
                "payment link id must match pl_ followed by letters or digits".to_string(),
            ));
        }
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "fetchPaymentLinkPayments".to_string(),
            method: HttpMethod::Get,
            url: self.url(&format!("/v2/payment-links/{link_id}/payments")),
            headers: self.headers(None),
            body: None,
            timeout_ms: 30_000,
        };
        let raw = send(transport, "fetchPaymentLinkPayments", request).await?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            BillingError::Malformed(format!(
                "payment link payments response is not JSON: {error}"
            ))
        })?;
        let payments = parsed
            .get("_embedded")
            .and_then(|embedded| embedded.get("payments"))
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                BillingError::Malformed(
                    "payment link payments response carries no payments list".to_string(),
                )
            })?;
        payments
            .iter()
            .map(MolliePayment::from_json)
            .collect::<Result<Vec<_>, _>>()
    }

    /// Issue a refund for a collected payment
    /// (`POST /v2/payments/{id}/refunds`). The deterministic
    /// `refund:<payment id>:<amount>` Idempotency-Key makes a retried refund
    /// request return the original refund instead of double-refunding. The
    /// receipt is validated against the request: an echo that disagrees on
    /// amount or currency is a typed refusal.
    pub async fn issue_refund(
        &self,
        transport: &dyn Transport,
        payment_id: &str,
        amount_minor_units: i64,
        description: &str,
    ) -> Result<crate::provider::RefundReceipt, BillingError> {
        let payment_id = payment_id.trim();
        if !is_payment_id(payment_id) {
            return Err(BillingError::Malformed(
                "refund payment id must match tr_ followed by letters or digits".to_string(),
            ));
        }
        if amount_minor_units <= 0 {
            return Err(BillingError::Malformed(format!(
                "refund amount {amount_minor_units} minor units is not positive"
            )));
        }
        let amount = format_amount(amount_minor_units)?;
        if description.trim().is_empty() || description.chars().any(char::is_control) {
            return Err(BillingError::Malformed(
                "refund description is empty or carries control characters".to_string(),
            ));
        }
        let body = serde_json::json!({
            "amount": {"currency": BILLING_CURRENCY, "value": amount},
            "description": description,
        });
        let request = HttpRequest {
            platform: BILLING_PLATFORM,
            operation: "issueRefund".to_string(),
            method: HttpMethod::Post,
            url: self.url(&format!("/v2/payments/{payment_id}/refunds")),
            headers: self.headers(Some(&format!("refund:{payment_id}:{amount_minor_units}"))),
            body: Some(HttpBody::Json(body)),
            timeout_ms: 30_000,
        };
        let raw = send(transport, "issueRefund", request).await?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            BillingError::Malformed(format!("refund response is not JSON: {error}"))
        })?;
        let refund_id = required_text(&parsed, "id", "refund response missing id")?;
        let status = required_text(&parsed, "status", "refund response missing status")?;
        let refunded = parsed
            .get("amount")
            .ok_or_else(|| BillingError::Malformed("refund response missing amount".to_string()))?;
        let value = required_text(refunded, "value", "refund amount missing value")?;
        if value != amount {
            return Err(BillingError::Malformed(format!(
                "refund response amount {value} does not match the requested {amount}"
            )));
        }
        let currency = required_text(refunded, "currency", "refund amount missing currency")?;
        if currency != BILLING_CURRENCY {
            return Err(BillingError::Malformed(format!(
                "refund response currency {currency} is not the billing currency"
            )));
        }
        Ok(crate::provider::RefundReceipt {
            refund_id,
            payment_id: payment_id.to_string(),
            status,
            amount_minor_units,
            currency,
        })
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

/// The shopper-facing URL of a payment-link response: `_links.paymentUrl`
/// (the Payment Links API shape), falling back to the checkout shape.
fn payment_link_url(value: &serde_json::Value) -> Option<String> {
    let links = value.get("_links")?;
    let from = |key: &str| {
        links.get(key).and_then(|link| match link {
            serde_json::Value::String(url) => Some(url.clone()),
            value => value
                .get("href")
                .and_then(|href| href.as_str())
                .map(str::to_string),
        })
    };
    from("paymentUrl")
        .or_else(|| from("checkout"))
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// The deterministic reference of an invoice-level payment link:
/// `apex-invoice:<publisher>:<YYYY-MM>`. It doubles as the Idempotency-Key,
/// so a resumed run cannot create a second link for the same invoice.
pub fn invoice_link_reference(
    publisher_id: &str,
    year_month: &str,
) -> Result<String, BillingError> {
    let reference = format!("apex-invoice:{publisher_id}:{year_month}");
    if reference.trim().is_empty()
        || reference.len() > MAX_REFERENCE_LEN
        || reference.chars().any(char::is_control)
    {
        return Err(BillingError::UnsafeIdentifier {
            seq: 0,
            detail: format!(
                "invoice link reference is unusable (length {})",
                reference.len()
            ),
        });
    }
    Ok(reference)
}

/// Parse the deterministic reference of an invoice-level payment link back
/// into its `(publisher_id, year_month)`: the inverse of
/// [`invoice_link_reference`]. The publisher charset is the same strict set
/// the payment-event processor accepts, and the period must be a `YYYY-MM`
/// shape, so an attacker-controlled metadata reference can never name a
/// forged invoice row.
pub fn parse_invoice_link_reference(reference: &str) -> Option<(String, String)> {
    let rest = reference.strip_prefix("apex-invoice:")?;
    let (publisher_id, year_month) = rest.split_once(':')?;
    let publisher_id = publisher_id.trim();
    let year_month = year_month.trim();
    let valid_publisher = !publisher_id.is_empty()
        && publisher_id.len() <= 64
        && publisher_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let valid_period = year_month.len() == 7
        && year_month.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        && year_month.as_bytes()[4] == b'-'
        && year_month.as_bytes()[5..7].iter().all(u8::is_ascii_digit);
    (valid_publisher && valid_period).then(|| (publisher_id.to_string(), year_month.to_string()))
}

/// A Mollie payment id: `tr_` followed by letters and digits.
pub fn is_payment_id(value: &str) -> bool {
    value.strip_prefix("tr_").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 64
            && rest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    })
}

/// A Mollie payment-link id: `pl_` followed by letters and digits.
pub fn is_payment_link_id(value: &str) -> bool {
    value.strip_prefix("pl_").is_some_and(|rest| {
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
    /// `amountRefunded.value` in minor units, when the provider reports a
    /// refund against this payment (DD). A payment keeps the `paid` status
    /// through a refund; only this field says money went back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_refunded_minor_units: Option<i64>,
    /// `amountChargedBack.value` in minor units, when the provider reports a
    /// chargeback against this payment (DD). A payment keeps the `paid`
    /// status through a chargeback; only this field says the money was taken
    /// back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_charged_back_minor_units: Option<i64>,
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
        // DD: `amountRefunded`/`amountChargedBack` are optional amount
        // objects on a payment. A present field must parse and agree with
        // the payment's currency — a refund booked in another currency is a
        // malformed response, never a silently accepted reversal.
        let outer_currency = currency.clone();
        let reversal_amount = |key: &str| match value.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(amount) => {
                let text = required_text(amount, "value", &format!("payment {key} missing value"))?;
                let minor_units = parse_amount(&text)?;
                let currency = required_text(
                    amount,
                    "currency",
                    &format!("payment {key} missing currency"),
                )?;
                if currency != outer_currency {
                    return Err(BillingError::Malformed(format!(
                        "payment {key} currency {currency:?} is not the payment currency"
                    )));
                }
                Ok(Some(minor_units))
            }
        };
        let amount_refunded_minor_units = reversal_amount("amountRefunded")?;
        let amount_charged_back_minor_units = reversal_amount("amountChargedBack")?;
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
            amount_refunded_minor_units,
            amount_charged_back_minor_units,
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
    /// The single receivable that pays a whole invoice: one provider payment
    /// for the invoice total, carrying the deterministic
    /// `apex-invoice:<publisher>:<YYYY-MM>` reference (the same reference the
    /// invoice-level webhook routing parses). Collection submits exactly one
    /// of these per invoice — one first payment when no mandate is stored, or
    /// one recurring charge when one is — instead of one checkout per line
    /// plus an invoice payment link.
    ///
    /// Refuses a non-positive total and identifiers that cannot travel in
    /// the reference (control characters).
    pub fn for_invoice_total(
        invoice: &crate::invoice::Invoice,
        amount_minor_units: i64,
    ) -> Result<Self, BillingError> {
        let publisher_id = invoice.publisher_id.as_str();
        let year_month = invoice.period.year_month.as_str();
        if amount_minor_units <= 0 {
            return Err(BillingError::Malformed(format!(
                "invoice receivable amount {amount_minor_units} minor units is not positive"
            )));
        }
        let reference = invoice_link_reference(publisher_id, year_month)?;
        let customer_reference = format!("publisher-{publisher_id}");
        let description = format!("Apex invoice INV-{year_month}-{publisher_id}");
        for part in [&customer_reference, &description] {
            if let Some(control) = part.chars().find(|&control| control.is_control()) {
                return Err(BillingError::UnsafeIdentifier {
                    seq: 0,
                    detail: format!(
                        "invoice receivable text contains control character U+{:04X}",
                        control as u32
                    ),
                });
            }
        }
        Ok(Self {
            seq: 0,
            experiment_id: String::new(),
            amount_minor_units,
            currency: BILLING_CURRENCY.to_string(),
            customer_reference,
            reference,
            description,
        })
    }

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
    /// A checkout payment was created: the first payment, or the typed fallback.
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
    /// The created checkout payment on the link channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_link: Option<PaymentLinkResponse>,
    /// Whether the checkout payment is a **first payment**
    /// (`sequenceType: "first"`): completing it is what stores the mandate
    /// later months charge, so its URL is the one the customer must receive
    /// (E7). A plain checkout or typed fallback is not.
    #[serde(default)]
    pub first_payment: bool,
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
    /// The durable invoice-level payment link created for the invoice total,
    /// when the invoice had at least one billable line. This is the URL the
    /// invoice email and the console hand the customer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_link: Option<PaymentLinkResponse>,
}

impl CollectionReceipt {
    /// The URLs of the created checkout payments (the fallback channel), in line
    /// order. A receipt collected entirely by mandate has none.
    pub fn link_urls(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter_map(|line| line.payment_link.as_ref().map(|link| link.url.as_str()))
            .collect()
    }

    /// Whether the receipt contains at least one checkout payment.
    pub fn has_links(&self) -> bool {
        self.lines
            .iter()
            .any(|line| line.channel == CollectionChannel::PaymentLink)
    }
}

/// What [`create_payment_links`] created: the checkout payments and their checked
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
    /// Created checkout payments, in ledger `seq` order.
    pub links: Vec<PaymentLinkResponse>,
}

/// Create one Mollie checkout payment per non-zero invoice line, in `seq` order.
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
        total_minor_units = total_minor_units
            .checked_add(item.amount_minor_units)
            .ok_or(BillingError::Overflow)?;
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
/// known, otherwise create a checkout payment. A recurring *refusal* falls back
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
        first_payment: false,
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
            Err(BillingError::StoredChargeRefused { detail, .. }) => {
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
        invoice_link: None,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use apex_connectors::transport::{HttpResponse, TransportError};

    use super::*;

    /// A transport that replays scripted HTTP responses and records calls.
    #[derive(Default)]
    struct ScriptTransport {
        script: Mutex<VecDeque<HttpResponse>>,
        calls: Mutex<Vec<HttpRequest>>,
    }

    impl ScriptTransport {
        fn new(responses: Vec<HttpResponse>) -> Self {
            Self {
                script: Mutex::new(responses.into()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<HttpRequest> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Transport for ScriptTransport {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.calls.lock().unwrap().push(req);
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| TransportError::Network("script exhausted".into()))
        }
    }

    fn payment_body(id: &str, status: &str, reference: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![],
            body: format!(
                r#"{{"resource":"payment","id":"{id}","status":"{status}","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"{reference}","publisher":"publisher-pub-1","amount":"1.25"}},"customerId":"cst_1","mandateId":"mnd_1","sequenceType":"recurring","_links":{{"checkout":{{"href":"https://pay.mollie.test/{id}"}}}}}}"#
            )
            .into_bytes(),
        }
    }

    fn line_item() -> InvoiceLineItem {
        InvoiceLineItem {
            seq: 1,
            experiment_id: "exp-a".into(),
            amount_minor_units: 125,
            currency: "USD".into(),
            customer_reference: "publisher-pub-1".into(),
            reference: "apex-fee:pub-1:exp-a:2026-10".into(),
            description: "Apex proven-lift fee".into(),
        }
    }

    fn client() -> MollieClient {
        MollieClient::new(
            "key",
            "https://api.mollie.test",
            "https://apex.test/v1/mollie/webhook",
            "https://apex.test/watch/return",
        )
        .expect("client")
    }

    #[tokio::test]
    async fn a_pending_recurring_charge_is_submitted_not_refused() {
        // SEPA debits settle over days: `pending` is a live charge with a
        // durable provider id, never a refusal that would double-bill.
        let transport = ScriptTransport::new(vec![payment_body(
            "tr_pending",
            "pending",
            "apex-fee:pub-1:exp-a:2026-10",
        )]);
        let receipt = client()
            .charge_stored_payment_method(&transport, &line_item(), "cst_1", "mnd_1")
            .await
            .expect("pending is submitted");
        assert_eq!(receipt.payment_id, "tr_pending");
        assert_eq!(receipt.status, "pending");
    }

    #[tokio::test]
    async fn an_open_recurring_charge_is_submitted_not_refused() {
        let transport = ScriptTransport::new(vec![payment_body(
            "tr_open",
            "open",
            "apex-fee:pub-1:exp-a:2026-10",
        )]);
        let receipt = client()
            .charge_stored_payment_method(&transport, &line_item(), "cst_1", "mnd_1")
            .await
            .expect("open is submitted");
        assert_eq!(receipt.status, "open");
    }

    #[tokio::test]
    async fn definitive_recurring_failures_still_refuse() {
        for status in ["failed", "canceled", "expired"] {
            let transport = ScriptTransport::new(vec![payment_body(
                "tr_dead",
                status,
                "apex-fee:pub-1:exp-a:2026-10",
            )]);
            let error = client()
                .charge_stored_payment_method(&transport, &line_item(), "cst_1", "mnd_1")
                .await
                .expect_err("refusal");
            assert!(
                matches!(error, BillingError::StoredChargeRefused { .. }),
                "status {status}: {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_invoice_payment_link_posts_to_the_payment_links_api() {
        let transport = ScriptTransport::new(vec![HttpResponse {
            status: 200,
            headers: vec![],
            body: br#"{"resource":"payment-link","id":"pl_9","amount":{"currency":"USD","value":"4.20"},"description":"Apex invoice INV-2026-10-pub-1","_links":{"paymentUrl":{"href":"https://www.mollie.test/pay/pl_9"},"checkout":{"href":"https://www.mollie.test/checkout/pl_9"}}}"#.to_vec(),
        }]);
        let link = client()
            .create_invoice_payment_link(
                &transport,
                "pub-1",
                "2026-10",
                420,
                "USD",
                "Apex invoice INV-2026-10-pub-1",
            )
            .await
            .expect("payment link");
        assert_eq!(link.id, "pl_9");
        assert_eq!(link.url, "https://www.mollie.test/pay/pl_9");

        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].url, "https://api.mollie.test/v2/payment-links");
        assert_eq!(calls[0].operation, "createPaymentLink");
        assert!(
            calls[0]
                .headers
                .iter()
                .any(|(k, v)| k == "Idempotency-Key" && v == "apex-invoice:pub-1:2026-10"),
            "{:?}",
            calls[0].headers
        );
        let body = match calls[0].body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("unexpected body {other:?}"),
        };
        assert_eq!(body["amount"]["value"], "4.20");
        assert_eq!(body["amount"]["currency"], "USD");
        assert_eq!(body["metadata"]["reference"], "apex-invoice:pub-1:2026-10");
        assert_eq!(body["webhookUrl"], "https://apex.test/v1/mollie/webhook");
    }

    #[tokio::test]
    async fn the_invoice_payment_link_refuses_non_positive_amounts_and_bad_input() {
        let transport = ScriptTransport::new(vec![]);
        for amount in [0, -420] {
            let error = client()
                .create_invoice_payment_link(&transport, "pub-1", "2026-10", amount, "USD", "d")
                .await
                .expect_err("non-positive amount");
            assert!(matches!(error, BillingError::Malformed(_)), "{error:?}");
        }
        let error = client()
            .create_invoice_payment_link(&transport, "pub-1", "2026-10", 420, "usd", "d")
            .await
            .expect_err("lowercase currency");
        assert!(matches!(error, BillingError::Malformed(_)), "{error:?}");
        let error = client()
            .create_invoice_payment_link(&transport, "pub-1", "2026-10", 420, "USD", "")
            .await
            .expect_err("empty description");
        assert!(matches!(error, BillingError::Malformed(_)), "{error:?}");
        assert!(transport.calls().is_empty(), "refusals send nothing");
        assert_eq!(
            invoice_link_reference("pub-1", "2026-10").expect("reference"),
            "apex-invoice:pub-1:2026-10"
        );
    }

    #[test]
    fn a_fetched_payment_carries_refund_and_chargeback_amounts() {
        // DD: a payment keeps the collected status through reversals; the
        // amounts arrive as amountRefunded/amountChargedBack and are exposed
        // as parsed minor units.
        let payment = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(
                r#"{"id":"tr_1","status":"paid","amount":{"currency":"USD","value":"1.25"},
                    "amountRefunded":{"currency":"USD","value":"0.25"},
                    "amountChargedBack":{"currency":"USD","value":"0.50"}}"#,
            )
            .expect("json"),
        )
        .expect("payment");
        assert_eq!(payment.status, "paid");
        assert_eq!(payment.amount_refunded_minor_units, Some(25));
        assert_eq!(payment.amount_charged_back_minor_units, Some(50));

        // Absent fields read as None.
        let plain = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(
                r#"{"id":"tr_1","status":"paid","amount":{"currency":"USD","value":"1.25"}}"#,
            )
            .expect("json"),
        )
        .expect("payment");
        assert_eq!(plain.amount_refunded_minor_units, None);
        assert_eq!(plain.amount_charged_back_minor_units, None);

        // A reversal booked in another currency is a malformed response,
        // never a silently accepted reversal.
        let foreign = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(
                r#"{"id":"tr_1","status":"paid","amount":{"currency":"USD","value":"1.25"},
                    "amountRefunded":{"currency":"EUR","value":"0.25"}}"#,
            )
            .expect("json"),
        );
        assert!(foreign.is_err(), "a foreign-currency refund fails closed");
        let no_currency = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(
                r#"{"id":"tr_1","status":"paid","amount":{"currency":"USD","value":"1.25"},
                    "amountChargedBack":{"value":"0.50"}}"#,
            )
            .expect("json"),
        );
        assert!(
            no_currency.is_err(),
            "a currency-less chargeback fails closed"
        );
    }

    #[test]
    fn invoice_link_references_round_trip_strictly() {
        assert_eq!(
            parse_invoice_link_reference("apex-invoice:pub-1:2026-10"),
            Some(("pub-1".to_string(), "2026-10".to_string()))
        );
        assert_eq!(
            parse_invoice_link_reference("apex-fee:pub-1:exp:2026-10"),
            None
        );
        assert_eq!(parse_invoice_link_reference("apex-invoice:pub-1"), None);
        assert_eq!(
            parse_invoice_link_reference("apex-invoice:pub-1:2026-9"),
            None
        );
        assert_eq!(parse_invoice_link_reference("apex-invoice::2026-10"), None);
        assert_eq!(parse_invoice_link_reference(""), None);
    }
}
