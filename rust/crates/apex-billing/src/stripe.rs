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

use apex_connectors::transport::{HttpMethod, HttpRequest, Transport};
use apex_core::PlatformId;
use serde::{Deserialize, Serialize};

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

/// A Stripe endpoint client: the webhook signing secret plus the freshness
/// tolerance. This type performs no network I/O; it is the credential +
/// verification surface the API server holds.
#[derive(Debug, Clone)]
pub struct StripeClient {
    webhook_secret: Vec<u8>,
    tolerance_secs: u64,
}

impl StripeClient {
    /// Build a client from the endpoint's webhook signing secret and the
    /// maximum accepted timestamp skew in seconds (Stripe emits skew well
    /// under a minute; 300 is the customary default).
    pub fn new(webhook_secret: impl Into<Vec<u8>>, tolerance_secs: u64) -> Self {
        Self {
            webhook_secret: webhook_secret.into(),
            tolerance_secs,
        }
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
            let value = value.trim();
            if key.is_empty() || value.is_empty() {
                return Err(StripeSignatureError::MalformedHeader);
            }
            match key {
                "t" => {
                    let parsed = value
                        .parse::<i64>()
                        .map_err(|_| StripeSignatureError::MalformedHeader)?;
                    timestamp.get_or_insert(parsed);
                }
                "v1" => signatures.push(value),
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
    /// produces one).
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
        Ok(Self {
            seq: line.seq,
            experiment_id: line.experiment_id.clone(),
            amount_minor_units: line.fee_micros / MICROS_PER_MINOR_UNIT,
            currency: STRIPE_CURRENCY.to_string(),
            idempotency_key: format!(
                "stripe-item:{publisher_id}:{}:{year_month}",
                line.experiment_id
            ),
            description: format!(
                "Apex proven-lift fee: experiment {} ({} bp of {} micros proven lift)",
                line.experiment_id, line.rate_bp, line.proven_lift
            ),
        })
    }
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
/// billing. Refusals are typed:
///
/// - a non-exact cent fee refuses the whole invoice before any request
///   ([`BillingError::SubCentAmount`]);
/// - a failure on the *first* item surfaces the underlying typed error;
/// - a failure *after* at least one item was created is
///   [`BillingError::StripePartial`] with the created ids, so the caller can
///   resume precisely.
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
    let mut items: Vec<StripeItemResponse> = Vec::with_capacity(invoice.line_items.len());
    let mut total_minor_units: i64 = 0;
    for line in &invoice.line_items {
        if line.fee_micros == 0 {
            // A zero-fee line bills nothing; creating a zero-amount Stripe
            // item would be noise, not money.
            continue;
        }
        let item = InvoiceLineItem::from_invoice_line(
            line,
            &invoice.publisher_id,
            &invoice.period.year_month,
        )?;
        let request = HttpRequest {
            platform: PlatformId::Admob,
            operation: format!("stripe-invoice-item:seq:{}", line.seq),
            method: HttpMethod::Post,
            url: format!("{STRIPE_API_BASE}/v1/invoiceitems"),
            headers: vec![
                ("Authorization".to_string(), format!("Bearer {api_key}")),
                ("Accept".to_string(), "application/json".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Idempotency-Key".to_string(), item.idempotency_key.clone()),
            ],
            body: Some(serde_json::json!({
                "amount": item.amount_minor_units,
                "currency": item.currency,
                "description": item.description,
            })),
            timeout_ms: 10_000,
        };
        match transport.request(request).await {
            Ok(response) if (200..300).contains(&response.status) => {
                let parsed: StripeItemResponse =
                    serde_json::from_slice(&response.body).map_err(|error| {
                        BillingError::Schema {
                            detail: format!(
                                "invoice item for seq {} is not a Stripe item object: {error}",
                                line.seq
                            ),
                        }
                    })?;
                total_minor_units = total_minor_units
                    .checked_add(item.amount_minor_units)
                    .ok_or(BillingError::Overflow)?;
                items.push(parsed);
            }
            Ok(response) => {
                let error = classify_stripe_status(response.status);
                return Err(partial_or(error, items, line.seq));
            }
            Err(transport_error) => {
                let error = BillingError::from(transport_error);
                return Err(partial_or(error, items, line.seq));
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
/// is a partial application naming what already exists (resumable by the
/// deterministic idempotency keys).
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
    }
}
