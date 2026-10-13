//! Stripe webhook signature verification (the documented `Stripe-Signature`
//! scheme), implemented over `hmac`/`sha2` with constant-time comparison.
//!
//! ## Header format
//!
//! ```text
//! Stripe-Signature: t=1492774577,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd[,v1=...][,v0=...]
//! ```
//!
//! The signed message is the ASCII timestamp, a literal `.`, then the raw
//! request body bytes: `"1492774577." + body`. The signature is the lowercase
//! hex HMAC-SHA256 of that message under the endpoint secret.
//!
//! ## Why every check is mandatory
//!
//! - The timestamp is replayed inside the signed message, so an attacker cannot
//!   age a captured delivery without invalidating the signature; the tolerance
//!   check bounds capture-and-replay in time.
//! - `v1` values from *all* entries are candidates and the first match wins;
//!   any mismatch is not distinguished from a missing match at this layer
//!   (callers that expose errors must collapse the taxonomy outward, see
//!   `apex-api`'s `invalid_signature` response).
//! - The header is parsed strictly: an unknown item without `=` or an empty
//!   field is malformed, never silently ignored.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use apex_connectors::transport::{HttpBody, HttpMethod, HttpRequest, Transport, FORM_CONTENT_TYPE};
use apex_core::PlatformId;
use serde::{Deserialize, Serialize};

use crate::encode::encode_form_component;
use crate::errors::BillingError;
use crate::invoice::{Invoice, InvoiceLine};

type HmacSha256 = Hmac<Sha256>;

/// A signature-verification failure. Every variant is a refusal; none carries
/// attacker-controlled data beyond lengths and skews.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StripeSignatureError {
    /// The `Stripe-Signature` header is not a `k=v,k=v` list (empty item,
    /// empty key, empty value) or the timestamp is not an integer.
    #[error("malformed Stripe-Signature header")]
    MalformedHeader,

    /// The header carries no `v1` signature at all.
    #[error("Stripe-Signature header has no v1 signature")]
    MissingV1Signature,

    /// The signed timestamp's absolute skew against the verifying clock is
    /// larger than the configured tolerance: a stale or future-dated delivery.
    #[error("timestamp skew of {skew_secs}s exceeds the {tolerance_secs}s tolerance")]
    TimestampOutOfTolerance { skew_secs: u64, tolerance_secs: u64 },

    /// No `v1` signature matches the body under the secret.
    #[error("no v1 signature matches the payload")]
    NoMatchingSignature,
}

/// Proof that a payload was signed by the secret: the timestamp that was part
/// of the signed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedEvent {
    /// The `t` value from the verified header (Unix seconds).
    pub timestamp: i64,
}

/// A configuration refusal while building a [`StripeClient`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StripeClientError {
    /// The webhook signing secret was empty. HMAC accepts an empty key, so an
    /// empty secret would verify every delivery signed with the empty key;
    /// this is a misconfiguration, never a working endpoint.
    #[error("stripe webhook signing secret must not be empty")]
    EmptySecret,
}

/// A Stripe endpoint client: the webhook signing secret plus the freshness
/// tolerance. This type performs no network I/O; it is the credential +
/// verification surface the API server holds.
#[derive(Clone)]
pub struct StripeClient {
    webhook_secret: Vec<u8>,
    tolerance_secs: u64,
}

impl std::fmt::Debug for StripeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render the signing secret; `webhook_secret()` is the
        // deliberate accessor.
        f.debug_struct("StripeClient")
            .field("webhook_secret", &"[redacted]")
            .field("tolerance_secs", &self.tolerance_secs)
            .finish()
    }
}

impl StripeClient {
    /// Build a client from the endpoint's webhook signing secret and the
    /// maximum accepted timestamp skew in seconds (Stripe emits skew well
    /// under a minute; 300 is the customary default).
    ///
    /// Refuses an empty secret ([`StripeClientError::EmptySecret`]): an empty
    /// HMAC key is legal and would accept forgeries signed with it.
    pub fn new(
        webhook_secret: impl Into<Vec<u8>>,
        tolerance_secs: u64,
    ) -> Result<Self, StripeClientError> {
        let webhook_secret = webhook_secret.into();
        if webhook_secret.is_empty() {
            return Err(StripeClientError::EmptySecret);
        }
        Ok(Self {
            webhook_secret,
            tolerance_secs,
        })
    }

    /// The configured webhook signing secret.
    pub fn webhook_secret(&self) -> &[u8] {
        &self.webhook_secret
    }

    /// The configured maximum timestamp skew in seconds.
    pub fn tolerance_secs(&self) -> u64 {
        self.tolerance_secs
    }

    /// Verify `signature_header` against `payload` as of `now_unix_s`.
    pub fn verify(
        &self,
        payload: &[u8],
        signature_header: &str,
        now_unix_s: i64,
    ) -> Result<VerifiedEvent, StripeSignatureError> {
        verify_stripe_signature(
            &self.webhook_secret,
            payload,
            signature_header,
            self.tolerance_secs,
            now_unix_s,
        )
    }
}

/// Verify a Stripe webhook delivery.
///
/// `secret` is the endpoint's signing secret, `payload` the exact raw request
/// bytes (never a re-serialized JSON value), `signature_header` the
/// `Stripe-Signature` value, `tolerance_secs` the maximum accepted absolute
/// timestamp skew and `now_unix_s` the verifier's clock (an explicit argument,
/// so verification is testable and deterministic).
pub fn verify_stripe_signature(
    secret: &[u8],
    payload: &[u8],
    signature_header: &str,
    tolerance_secs: u64,
    now_unix_s: i64,
) -> Result<VerifiedEvent, StripeSignatureError> {
    let parsed = ParsedHeader::parse(signature_header)?;
    // i128 arithmetic: no i64 subtraction path can wrap for extreme clocks.
    let skew = (i128::from(now_unix_s) - i128::from(parsed.timestamp)).unsigned_abs();
    if skew > u128::from(tolerance_secs) {
        return Err(StripeSignatureError::TimestampOutOfTolerance {
            skew_secs: u64::try_from(skew).unwrap_or(u64::MAX),
            tolerance_secs,
        });
    }
    let mut message = parsed.timestamp.to_string().into_bytes();
    message.push(b'.');
    message.extend_from_slice(payload);
    for candidate in &parsed.signatures {
        let Ok(bytes) = hex::decode(candidate) else {
            continue;
        };
        let Ok(signature) = <[u8; 32]>::try_from(bytes.as_slice()) else {
            continue;
        };
        // `verify_slice` is constant-time; the loop exits on the first match.
        if HmacSha256::new_from_slice(secret)
            .map(|mut mac| {
                mac.update(&message);
                mac.verify_slice(&signature)
            })
            .is_ok_and(|outcome| outcome.is_ok())
        {
            return Ok(VerifiedEvent {
                timestamp: parsed.timestamp,
            });
        }
    }
    Err(StripeSignatureError::NoMatchingSignature)
}

/// Produce the `Stripe-Signature` header value for `payload` at `timestamp`.
///
/// This is the signing half of the scheme: it exists for local dry runs and
/// integration tests that exercise a real verifier without contacting Stripe.
/// It uses the same HMAC construction as [`verify_stripe_signature`] and is
/// not a production network path.
pub fn sign_stripe_payload(secret: &[u8], payload: &[u8], timestamp: i64) -> String {
    let mut message = timestamp.to_string().into_bytes();
    message.push(b'.');
    message.extend_from_slice(payload);
    // HMAC accepts keys of any length, so `new_from_slice` cannot fail; the
    // `None` fallback (an empty signature) would simply never verify, so no
    // path here can fabricate an authentic-looking delivery.
    let signature = HmacSha256::new_from_slice(secret)
        .ok()
        .map(|mut mac| {
            mac.update(&message);
            hex::encode(mac.finalize().into_bytes())
        })
        .unwrap_or_default();
    format!("t={timestamp},v1={signature}")
}

/// Parsed `Stripe-Signature` header: one timestamp plus every `v1` candidate.
struct ParsedHeader<'a> {
    timestamp: i64,
    signatures: Vec<&'a str>,
}

impl<'a> ParsedHeader<'a> {
    fn parse(header: &'a str) -> Result<Self, StripeSignatureError> {
        let mut timestamp: Option<i64> = None;
        let mut signatures = Vec::new();
        for item in header.split(',') {
            let item = item.trim();
            let Some((key, value)) = item.split_once('=') else {
                return Err(StripeSignatureError::MalformedHeader);
            };
            let key = key.trim();
            if key.is_empty() || value.trim().is_empty() {
                return Err(StripeSignatureError::MalformedHeader);
            }
            match key {
                "t" => {
                    // Exactly one timestamp, in canonical decimal form:
                    // `+1000`, `00001000`, padded values and a repeated `t`
                    // are malformed, never silently normalized.
                    if timestamp.is_some() {
                        return Err(StripeSignatureError::MalformedHeader);
                    }
                    let parsed = value
                        .parse::<i64>()
                        .map_err(|_| StripeSignatureError::MalformedHeader)?;
                    if value != parsed.to_string() {
                        return Err(StripeSignatureError::MalformedHeader);
                    }
                    timestamp = Some(parsed);
                }
                "v1" => signatures.push(value.trim()),
                // Unknown keys (e.g. `v0`, future schemes) are ignored: they
                // cannot weaken verification because only `v1` verifies.
                _ => {}
            }
        }
        let timestamp = timestamp.ok_or(StripeSignatureError::MalformedHeader)?;
        if signatures.is_empty() {
            return Err(StripeSignatureError::MissingV1Signature);
        }
        Ok(Self {
            timestamp,
            signatures,
        })
    }
}

/// Stripe API base URL. Operations append their `/v1/...` path; the base
/// carries no version and no trailing slash.
pub const STRIPE_API_BASE: &str = "https://api.stripe.com";

/// The currency invoices are billed in: fees are USD micros (the product and
/// `apex-core` money model are USD-denominated), sent to Stripe as cents.
pub const STRIPE_CURRENCY: &str = "usd";

/// Micros per Stripe minor unit (cent): the fee-to-cents conversion must be
/// exact, so a fee that is not a whole number of cents is refused rather than
/// rounded (money is never silently moved).
pub const MICROS_PER_MINOR_UNIT: i64 = 10_000;

/// Maximum length of the derived `Idempotency-Key` header value. Stripe's
/// documented limit is 255 characters; the builder caps itself lower so the
/// refusal is a local, typed error rather than a remote 400.
pub const MAX_IDEMPOTENCY_KEY_LEN: usize = 200;

/// One Stripe invoice-item request derived from a verified invoice line,
/// including its deterministic idempotency key:
/// `stripe-item:<publisher>:<experiment>:<YYYY-MM>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLineItem {
    /// Ledger sequence of the billing proof this item bills.
    pub seq: u64,
    /// Experiment the proven lift belongs to.
    pub experiment_id: String,
    /// Stripe amount in minor units (cents).
    pub amount_minor_units: i64,
    /// ISO currency code (always [`STRIPE_CURRENCY`] here).
    pub currency: String,
    /// Deterministic idempotency key sent with the request.
    pub idempotency_key: String,
    /// Human-readable description shown on the Stripe item.
    pub description: String,
}

impl InvoiceLineItem {
    /// Derive the request payload for one invoice line.
    ///
    /// Refuses a fee that is not an exact number of cents
    /// ([`BillingError::SubCentAmount`]) and a negative fee
    /// ([`BillingError::TamperedProof`], defensively: `compute_invoice` never
    /// produces one). Also refuses identifiers that cannot travel in the
    /// `Idempotency-Key` header: control characters (including CR/LF) or a
    /// derived key longer than [`MAX_IDEMPOTENCY_KEY_LEN`]
    /// ([`BillingError::UnsafeIdentifier`]).
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
        let idempotency_key = format!(
            "stripe-item:{publisher_id}:{}:{year_month}",
            line.experiment_id
        );
        if let Some(control) = idempotency_key.chars().find(|c| c.is_control()) {
            return Err(BillingError::UnsafeIdentifier {
                seq: line.seq,
                detail: format!(
                    "idempotency key contains control character U+{:04X}",
                    control as u32
                ),
            });
        }
        if idempotency_key.len() > MAX_IDEMPOTENCY_KEY_LEN {
            return Err(BillingError::UnsafeIdentifier {
                seq: line.seq,
                detail: format!(
                    "idempotency key is {} characters (max {MAX_IDEMPOTENCY_KEY_LEN})",
                    idempotency_key.len()
                ),
            });
        }
        Ok(Self {
            seq: line.seq,
            experiment_id: line.experiment_id.clone(),
            amount_minor_units: line.fee_micros / MICROS_PER_MINOR_UNIT,
            currency: STRIPE_CURRENCY.to_string(),
            idempotency_key,
            description: format!(
                "Apex proven-lift fee: experiment {} ({} bp of {} micros proven lift)",
                line.experiment_id, line.rate_bp, line.proven_lift
            ),
        })
    }
}

/// The exact `application/x-www-form-urlencoded` body for one invoice item:
/// `amount=<cents>&currency=usd&description=<percent-encoded>`. The amount is
/// already an exact cent count (enforced in
/// [`InvoiceLineItem::from_invoice_line`]) and the description is
/// percent-encoded, so no value can break the form or inject a field.
fn stripe_form_body(item: &InvoiceLineItem) -> String {
    format!(
        "amount={}&currency={}&description={}",
        item.amount_minor_units,
        encode_form_component(&item.currency),
        encode_form_component(&item.description),
    )
}

/// The fields of Stripe's invoice-item response that billing records. Stripe
/// returns more; unknown fields are ignored by serde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripeItemResponse {
    /// Stripe object id (`ii_...`).
    pub id: String,
}

/// What [`bill_invoice`] created: the Stripe item ids and their checked
/// total. An invoice whose every line is zero (or which has no lines) sends
/// no request at all and yields an empty receipt with a zero total. Zero-fee
/// lines among paid lines are intentionally omitted from `items`, so
/// `items.len()` can be smaller than the invoice's `line_items.len()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingReceipt {
    /// Publisher the invoice was built for.
    pub publisher_id: String,
    /// Billing period label (`YYYY-MM`).
    pub year_month: String,
    /// ISO currency code of all items.
    pub currency: String,
    /// Checked sum of the created item amounts, in minor units.
    pub total_minor_units: i64,
    /// Created Stripe items, in ledger `seq` order.
    pub items: Vec<StripeItemResponse>,
}

/// Create one Stripe invoice item per non-zero invoice line, in `seq` order.
///
/// Every request goes through the shared [`Transport`] with
/// `Authorization: Bearer <api_key>` and the deterministic `Idempotency-Key`
/// of the line, so a partially applied invoice can be resumed without double
/// billing. Each item is sent as an `application/x-www-form-urlencoded` body:
/// `amount=<cents>&currency=usd&description=<percent-encoded>`. Refusals are
/// typed:
///
/// - the whole invoice is refused **before any request** when any billable
///   line is a non-exact cent fee ([`BillingError::SubCentAmount`]), a
///   negative fee ([`BillingError::TamperedProof`]) or carries an identifier
///   unusable in a header ([`BillingError::UnsafeIdentifier`]) — every
///   request payload is built first;
/// - a failure on the *first* item surfaces the underlying typed error;
/// - a failure *after* at least one item was created is
///   [`BillingError::StripePartial`] with the created ids and the cause, so
///   the caller can resume precisely. A 2xx response whose body is not a
///   Stripe item object counts as created, because Stripe processed the
///   request.
///
/// Zero-fee lines are intentionally omitted from `items` (a positive proof
/// can truncate to a zero fee); `receipt.items` can therefore be shorter than
/// `invoice.line_items`.
///
/// The shared request envelope carries a platform tag for retry/error
/// classification; billing is platform-agnostic, so the tag is
/// [`PlatformId::Admob`] and is never used for routing or reporting (the
/// [`BillingError`] mapping ignores it).
pub async fn bill_invoice(
    api_key: &str,
    transport: &dyn Transport,
    invoice: &Invoice,
) -> Result<BillingReceipt, BillingError> {
    // Build every request payload before the first byte of I/O: a sub-cent,
    // negative or header-unsafe line must refuse the whole invoice without
    // charging anything (the partial-charge falsifier).
    let items_to_create: Vec<InvoiceLineItem> = invoice
        .line_items
        .iter()
        .filter(|line| line.fee_micros != 0)
        .map(|line| {
            InvoiceLineItem::from_invoice_line(
                line,
                &invoice.publisher_id,
                &invoice.period.year_month,
            )
        })
        .collect::<Result<_, _>>()?;

    let mut items: Vec<StripeItemResponse> = Vec::with_capacity(items_to_create.len());
    let mut total_minor_units: i64 = 0;
    for item in &items_to_create {
        let request = HttpRequest {
            platform: PlatformId::Admob,
            operation: format!("stripe-invoice-item:seq:{}", item.seq),
            method: HttpMethod::Post,
            url: format!("{STRIPE_API_BASE}/v1/invoiceitems"),
            headers: vec![
                ("Authorization".to_string(), format!("Bearer {api_key}")),
                ("Accept".to_string(), "application/json".to_string()),
                ("Content-Type".to_string(), FORM_CONTENT_TYPE.to_string()),
                ("Idempotency-Key".to_string(), item.idempotency_key.clone()),
            ],
            body: Some(HttpBody::Form(stripe_form_body(item))),
            timeout_ms: 10_000,
        };
        match transport.request(request).await {
            Ok(response) if (200..300).contains(&response.status) => {
                let parsed: StripeItemResponse = match serde_json::from_slice(&response.body) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        // A 2xx means Stripe accepted the request, so anything
                        // already created must still be named.
                        let error = BillingError::Schema {
                            detail: format!(
                                "invoice item for seq {} is not a Stripe item object: {error}",
                                item.seq
                            ),
                        };
                        return Err(partial_or(error, items, item.seq));
                    }
                };
                total_minor_units = total_minor_units
                    .checked_add(item.amount_minor_units)
                    .ok_or(BillingError::Overflow)?;
                items.push(parsed);
            }
            Ok(response) => {
                let error = classify_stripe_status(response.status);
                return Err(partial_or(error, items, item.seq));
            }
            Err(transport_error) => {
                let error = BillingError::from(transport_error);
                return Err(partial_or(error, items, item.seq));
            }
        }
    }
    Ok(BillingReceipt {
        publisher_id: invoice.publisher_id.clone(),
        year_month: invoice.period.year_month.clone(),
        currency: STRIPE_CURRENCY.to_string(),
        total_minor_units,
        items,
    })
}

fn classify_stripe_status(status: u16) -> BillingError {
    if status == 401 || status == 403 {
        BillingError::Auth { status }
    } else {
        BillingError::Http { status }
    }
}

/// A failure on the first item is the underlying error; a failure afterwards
/// is a partial application naming what already exists and why the run
/// stopped (resumable by the deterministic idempotency keys).
fn partial_or(
    error: BillingError,
    items: Vec<StripeItemResponse>,
    failed_seq: u64,
) -> BillingError {
    if items.is_empty() {
        return error;
    }
    BillingError::StripePartial {
        completed: items.into_iter().map(|item| item.id).collect(),
        failed_seq,
        cause: error.to_string(),
    }
}
