//! EU membership and the VAT treatment of an invoice.
//!
//! EU membership is derived from the small constant table below — the 27
//! member states as ISO 3166-1 alpha-2 codes, sorted (Greece is `GR`, the ISO
//! code, not the `EL` VAT-number prefix). The table is tested: 27 unique
//! entries, sorted, known members present, known non-members absent. Nothing
//! else is consulted.
//!
//! **Reverse charge is stated only after the EU VIES register confirms the
//! customer's number.** The shape check below is necessary but not
//! sufficient: a customer VAT number is first checked for its face
//! (non-empty, ASCII alphanumeric, 4..=14 characters, optional country
//! prefix, no separators), then submitted to a [`ViesClient`] seam. The
//! production implementation ([`HttpViesClient`]) calls the real VIES REST
//! API over TLS with a bounded timeout; tests always inject a stub and no
//! test makes a network call.
//!
//! A VIES answer is kept as [`ViesEvidence`] on the decided treatment and
//! travels onto the invoice. The answer is one of:
//!
//! - **valid** — reverse charge may be stated, provided the confirmed
//!   country/number match the customer's exactly;
//! - **invalid** — the number is not on the register: domestic VAT applies;
//! - **unknown** — the register has no answer for the number: domestic VAT
//!   applies (the answer is still recorded, never read as valid);
//! - **unavailable** — the register could not answer (transport, timeout,
//!   non-success status): reverse charge is **refused** with a typed
//!   [`BillingError::ViesUnavailable`] carrying the reason. It is never
//!   silently treated as domestic and never silently treated as valid.
//!
//! The treatment rules per customer are:
//!
//! - outside the EU: no VAT is stated at all (no VAT line);
//! - same country as the seller: the profile's domestic rate
//!   (`vat_rate_bp`, configured via [`crate::profile::VAT_RATE_ENV`]);
//! - EU cross-border with a customer VAT number: reverse charge only with a
//!   matching VIES confirmation, otherwise domestic (invalid/unknown) or a
//!   typed refusal (unavailable);
//! - EU cross-border without a VAT number, or a non-EU seller: the domestic
//!   rate is charged as the documented fallback (no reverse charge without a
//!   number to cite).

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::errors::BillingError;
use crate::profile::{BillingProfile, CustomerProfile};

/// The highest VAT rate a profile may carry, in basis points (100%).
pub const MAX_VAT_RATE_BP: u16 = 10_000;

/// The EU VIES REST endpoint for one member state's VAT register.
pub const VIES_ENDPOINT: &str = "https://ec.europa.eu/taxation_customs/vies/rest-api/ms";
/// Default bound on one VIES request, in seconds. A register that does not
/// answer inside the bound is `unavailable`, never silently assumed valid.
pub const VIES_TIMEOUT_SECS: u32 = 10;
/// The bounded size of the evidence string kept with a VIES answer.
pub const VIES_MAX_EVIDENCE_CHARS: usize = 300;
/// The system `curl` binary the production client uses (the same TLS
/// convention as the watch fetcher; see [`HttpViesClient`]).
pub const VIES_CURL: &str = "curl";
/// The marker between a VIES response body and curl's status code.
const VIES_STATUS_MARKER: &str = "\n@@apex-vies-status@@";

/// The 27 EU member states as ISO 3166-1 alpha-2 codes, sorted.
pub const EU_MEMBER_STATES: [&str; 27] = [
    "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GR", "HR", "HU", "IE", "IT",
    "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI", "SK",
];

/// Whether `country` is an EU member state (case-insensitive; surrounding
/// whitespace ignored). Membership is decided by [`EU_MEMBER_STATES`] alone.
#[must_use]
pub fn is_eu_member(country: &str) -> bool {
    let code = country.trim();
    EU_MEMBER_STATES
        .iter()
        .any(|member| member.eq_ignore_ascii_case(code))
}

/// Whether `country` is exactly two ASCII letters. Codes are canonicalised to
/// uppercase at profile load; comparisons here and in [`treatment_for`] are
/// case-insensitive.
#[must_use]
pub fn is_valid_country_code(country: &str) -> bool {
    let code = country.trim();
    code.chars().count() == 2 && code.chars().all(|c| c.is_ascii_alphabetic())
}

/// The minimal VAT-number shape check: non-empty after trimming, at least 4
/// and at most 14 ASCII alphanumeric characters, no separators. The country
/// prefix (for example `EE` in `EE123456789`) is optional. This is only the
/// face check; reverse charge additionally requires a matching VIES
/// confirmation (see [`treatment_for_with_vies`]).
#[must_use]
pub fn is_valid_vat_number(number: &str) -> bool {
    let trimmed = number.trim();
    let length = trimmed.chars().count();
    (4..=14).contains(&length) && trimmed.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The VAT treatment of an invoice, decided by [`treatment_for`] and recorded
/// on the invoice so the renderer can state (or refuse to state) tax without
/// guessing. Serialized camelCase for the invoice register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VatTreatment {
    /// No treatment applied yet (a draft; the renderer and the register
    /// refuse it).
    #[default]
    NotApplied,
    /// Seller and customer in the same country: the profile's domestic rate.
    Domestic,
    /// EU cross-border business with a customer VAT number: the invoice
    /// states the words "reverse charge" where tax would otherwise appear.
    ReverseCharge,
    /// Customer outside the EU: no VAT is stated at all.
    OutsideEu,
}

impl std::fmt::Display for VatTreatment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotApplied => "not applied",
            Self::Domestic => "domestic",
            Self::ReverseCharge => "reverse charge",
            Self::OutsideEu => "outside the EU",
        })
    }
}

/// The outcome of one VIES check, as the register answered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViesOutcome {
    /// The register confirmed the number.
    Valid,
    /// The register answered that the number is not valid.
    Invalid,
    /// The register has no answer for the number (a typed unknown, never
    /// silently valid).
    Unknown,
    /// The register could not answer (transport, timeout, non-success).
    Unavailable,
}

impl ViesOutcome {
    /// The stable wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Unknown => "unknown",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One VIES answer kept as evidence on the decided treatment and the invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViesEvidence {
    /// The member state queried (uppercase ISO code).
    pub country: String,
    /// The number queried, exactly as the customer supplied it (trimmed).
    pub vat_number: String,
    /// What the register answered.
    pub outcome: ViesOutcome,
    /// The bounded response summary.
    pub detail: String,
}

impl ViesEvidence {
    /// Whether this evidence is a valid confirmation for `customer`: the
    /// register says valid, and the queried country/number are exactly the
    /// customer's. A valid answer for a *different* number never
    /// reverse-charges.
    #[must_use]
    pub fn confirms(&self, customer: &CustomerProfile) -> bool {
        self.outcome == ViesOutcome::Valid && self.confirms_country_and_number(customer)
    }

    /// Whether the evidence was queried for exactly the customer's country
    /// and number, regardless of the answer.
    #[must_use]
    pub fn confirms_country_and_number(&self, customer: &CustomerProfile) -> bool {
        // The register answers with the full number (`DE123456789`) while a
        // customer may supply the national form (`123456789`); compare the
        // national forms, each stripped against its own country, so the two
        // legitimate spellings of one number match.
        let national = |country: &str, value: &str| {
            national_vat_number(country, value)
                .trim()
                .to_ascii_uppercase()
        };
        self.country.eq_ignore_ascii_case(customer.country.trim())
            && national(&self.country, &self.vat_number)
                == national(
                    customer.country.trim(),
                    customer.vat_number.as_deref().unwrap_or_default(),
                )
    }
}

/// One typed VIES answer plus its bounded evidence string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViesCheck {
    /// The register confirmed the number (the returned number is the one the
    /// register actually confirmed, so a mismatch can be detected).
    Valid {
        /// Bounded response summary.
        evidence: String,
        /// The VAT number the register confirmed.
        vat_number: String,
    },
    /// The register answered that the number is not valid.
    Invalid {
        /// Bounded response summary.
        evidence: String,
    },
    /// The register has no answer for the number.
    Unknown {
        /// Bounded response summary.
        evidence: String,
    },
    /// The register could not answer.
    Unavailable {
        /// Bounded reason.
        evidence: String,
    },
}

impl ViesCheck {
    /// The typed outcome.
    #[must_use]
    pub fn outcome(&self) -> ViesOutcome {
        match self {
            Self::Valid { .. } => ViesOutcome::Valid,
            Self::Invalid { .. } => ViesOutcome::Invalid,
            Self::Unknown { .. } => ViesOutcome::Unknown,
            Self::Unavailable { .. } => ViesOutcome::Unavailable,
        }
    }

    /// The bounded evidence string.
    #[must_use]
    pub fn evidence(&self) -> &str {
        match self {
            Self::Valid { evidence, .. }
            | Self::Invalid { evidence }
            | Self::Unknown { evidence }
            | Self::Unavailable { evidence } => evidence,
        }
    }
}

/// Why a VIES client could not produce an answer. The treatment converts this
/// into the typed [`BillingError::ViesUnavailable`] refusal; it never becomes
/// a domestic charge or a reverse charge.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ViesError {
    /// The request could not be made or the transport failed.
    #[error("VIES request failed: {detail}")]
    Transport {
        /// Bounded failure detail.
        detail: String,
    },
    /// The answer was not the documented JSON shape.
    #[error("VIES response was malformed: {detail}")]
    Malformed {
        /// Bounded failure detail.
        detail: String,
    },
    /// The caller supplied a country/number the endpoint cannot carry.
    #[error("VIES configuration: {detail}")]
    Config {
        /// Bounded failure detail.
        detail: String,
    },
}

/// The register seam. Production injects [`HttpViesClient`]; tests inject a
/// stub and no test makes a live call.
pub trait ViesClient: Send + Sync + std::fmt::Debug {
    /// Check one member state's VAT number. `country` is the ISO code and
    /// `number` the full number (prefix included).
    fn check_vat_number(&self, country: &str, number: &str) -> Result<ViesCheck, ViesError>;
}

/// A per-run cache in front of one [`ViesClient`].
///
/// A billing run may invoice several subscribers that share one legal
/// customer (the same country and VAT number), and each invoice would
/// otherwise query the register again. The cache keys on the canonical
/// `(country, national number)` pair, so `DE123456789` and `123456789` for
/// `DE` are one entry. Every answer is replayed (valid, invalid, unknown and
/// unavailable), so a register outage cannot produce inconsistent treatments
/// for the same customer within a run. The cache lives for the caller's run
/// only; the answer that decided each invoice is still persisted as
/// [`ViesEvidence`] on that invoice.
#[derive(Debug)]
pub struct CachedVies<'a> {
    inner: &'a dyn ViesClient,
    answers: Mutex<HashMap<(String, String), Result<ViesCheck, ViesError>>>,
}

impl<'a> CachedVies<'a> {
    /// Wrap `inner` with a fresh empty cache.
    #[must_use]
    pub fn new(inner: &'a dyn ViesClient) -> Self {
        Self {
            inner,
            answers: Mutex::new(HashMap::new()),
        }
    }

    /// The number of distinct customer numbers answered so far (the test and
    /// metrics seam).
    #[must_use]
    pub fn len(&self) -> usize {
        self.answers.lock().map_or(0, |answers| answers.len())
    }

    /// Whether no register answer has been cached yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ViesClient for CachedVies<'_> {
    fn check_vat_number(&self, country: &str, number: &str) -> Result<ViesCheck, ViesError> {
        let key = vies_cache_key(country, number);
        if let Some(answer) = self
            .answers
            .lock()
            .ok()
            .and_then(|answers| answers.get(&key).cloned())
        {
            return answer;
        }
        let answer = self.inner.check_vat_number(country, number);
        if let Ok(mut answers) = self.answers.lock() {
            answers.insert(key, answer.clone());
        }
        answer
    }
}

/// The canonical key of one register question: uppercase country plus the
/// national number with the country prefix removed when it was supplied.
fn vies_cache_key(country: &str, number: &str) -> (String, String) {
    let country = country.trim().to_ascii_uppercase();
    let national = national_vat_number(&country, number)
        .trim()
        .to_ascii_uppercase();
    (country, national)
}

/// The production VIES client.
///
/// The billing crate's shared HTTP stack
/// ([`apex_connectors::transport`]) is asynchronous, but the VAT treatment is
/// a synchronous decision made inside invoice construction; this client
/// therefore mirrors the repository's smallest synchronous TLS convention
/// (the watch service's `curl` fetcher): the system `curl` with `--proto
/// =https`, a bounded `--max-time` and a bounded response size. Redirects are
/// not followed, and every failure becomes a typed [`ViesError`] (and then a
/// refused reverse charge), never a silent answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpViesClient {
    /// Path to `curl`.
    pub binary: String,
    /// Request timeout in seconds.
    pub timeout_secs: u32,
}

impl Default for HttpViesClient {
    fn default() -> Self {
        Self {
            binary: VIES_CURL.to_string(),
            timeout_secs: VIES_TIMEOUT_SECS,
        }
    }
}

impl HttpViesClient {
    /// The exact URL one check requests. VIES takes the member state
    /// separately, so the country prefix is stripped from the national
    /// number when the caller supplied it.
    pub fn url_for(country: &str, number: &str) -> String {
        let country = country.trim().to_ascii_uppercase();
        format!(
            "{VIES_ENDPOINT}/{country}/vat/{}",
            national_vat_number(&country, number)
        )
    }
}

impl ViesClient for HttpViesClient {
    fn check_vat_number(&self, country: &str, number: &str) -> Result<ViesCheck, ViesError> {
        let country = country.trim().to_ascii_uppercase();
        let number = number.trim();
        if !is_valid_country_code(&country) {
            return Err(ViesError::Config {
                detail: format!("'{country}' is not a two-letter country code"),
            });
        }
        if !is_valid_vat_number(number) {
            return Err(ViesError::Config {
                detail: format!("'{number}' is not a shape-valid VAT number"),
            });
        }
        let url = Self::url_for(&country, number);
        let timeout = self.timeout_secs.to_string();
        let output = Command::new(&self.binary)
            .args([
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--max-time",
                &timeout,
                "--max-filesize",
                "65536",
                "--write-out",
                &format!("{VIES_STATUS_MARKER}%{{http_code}}"),
                "--",
                &url,
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| ViesError::Transport {
                detail: bounded_evidence(&error.to_string()),
            })?;
        if !output.status.success() {
            let detail: String = String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(VIES_MAX_EVIDENCE_CHARS)
                .collect();
            return Err(ViesError::Transport {
                detail: if detail.is_empty() {
                    format!("curl exited with {}", output.status)
                } else {
                    detail
                },
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let (body, status) =
            stdout
                .rsplit_once(VIES_STATUS_MARKER)
                .ok_or_else(|| ViesError::Malformed {
                    detail: "no status from curl".to_string(),
                })?;
        let status: u16 = status.trim().parse().map_err(|_| ViesError::Malformed {
            detail: "bad status from curl".to_string(),
        })?;
        match status {
            200 => parse_vies_body(body, &country, number),
            404 => Ok(ViesCheck::Unknown {
                evidence: bounded_evidence(&format!(
                    "VIES answer: no record for {country}{number} (HTTP 404)"
                )),
            }),
            other => Ok(ViesCheck::Unavailable {
                evidence: bounded_evidence(&format!(
                    "VIES unavailable: HTTP {other} for {country}{number}: {}",
                    body.trim()
                )),
            }),
        }
    }
}

/// Parse the documented VIES REST body into a typed answer.
fn parse_vies_body(body: &str, country: &str, number: &str) -> Result<ViesCheck, ViesError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| ViesError::Malformed {
            detail: format!("response body is not JSON: {error}"),
        })?;
    let valid = value
        .get("valid")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| ViesError::Malformed {
            detail: "response body has no boolean 'valid'".to_string(),
        })?;
    let returned = value
        .get("vatNumber")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(number);
    // The REST response carries the national number (`123456789`); the
    // customer may have supplied the full form (`DE123456789`). Canonicalise
    // both to the full form so the exact-match rule is meaningful.
    let confirmed = full_vat_number(country, returned);
    let requested = value
        .get("requestDate")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown date");
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim();
    if valid {
        let evidence = bounded_evidence(&format!(
            "VIES confirmation: {confirmed} is valid (register {requested})"
        ));
        return Ok(ViesCheck::Valid {
            evidence,
            vat_number: confirmed,
        });
    }
    // The register answers `valid: false` with the placeholder name
    // `---` when it has no record; keep that as the typed unknown rather
    // than claiming the number was affirmatively invalid.
    let unknown = name.is_empty() || name == "---" || name == "**";
    let queried = full_vat_number(country, number);
    let evidence = bounded_evidence(&format!(
        "VIES answer: {queried} is not valid (register {confirmed}, {requested})"
    ));
    Ok(if unknown {
        ViesCheck::Unknown { evidence }
    } else {
        ViesCheck::Invalid { evidence }
    })
}

/// The national part of a VAT number (the country prefix removed when the
/// caller supplied it), for the VIES URL.
fn national_vat_number<'a>(country: &str, number: &'a str) -> &'a str {
    let number = number.trim();
    match number.get(..country.len()) {
        Some(prefix) if prefix.eq_ignore_ascii_case(country) => &number[country.len()..],
        _ => number,
    }
}

/// The canonical full form: uppercase, with the country prefix present
/// exactly once.
fn full_vat_number(country: &str, number: &str) -> String {
    let number = number.trim().to_ascii_uppercase();
    if number.starts_with(country) {
        number
    } else {
        format!("{country}{number}")
    }
}

/// Bound evidence by characters, never bytes, and never split a character.
fn bounded_evidence(value: &str) -> String {
    value.chars().take(VIES_MAX_EVIDENCE_CHARS).collect()
}

/// The decided VAT treatment plus the VIES answer it was decided from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VatDecision {
    /// The treatment to record on the invoice.
    pub treatment: VatTreatment,
    /// The VIES answer, when one was consulted.
    pub evidence: Option<ViesEvidence>,
}

/// Derive the VAT treatment for `customer`, consulting `vies` before any
/// reverse charge. See the module docs for the exact rules.
pub fn treatment_for_with_vies(
    seller: &BillingProfile,
    customer: &CustomerProfile,
    vies: &dyn ViesClient,
) -> Result<VatDecision, BillingError> {
    seller.validate()?;
    customer.validate()?;
    let seller_country = seller.country.trim().to_ascii_uppercase();
    let customer_country = customer.country.trim().to_ascii_uppercase();
    if !is_eu_member(&customer_country) {
        return Ok(VatDecision {
            treatment: VatTreatment::OutsideEu,
            evidence: None,
        });
    }
    if seller_country == customer_country || !is_eu_member(&seller_country) {
        return Ok(VatDecision {
            treatment: VatTreatment::Domestic,
            evidence: None,
        });
    }
    let Some(number) = customer.vat_number.as_deref() else {
        return Ok(VatDecision {
            treatment: VatTreatment::Domestic,
            evidence: None,
        });
    };
    let number = number.trim();
    let check = match vies.check_vat_number(&customer_country, number) {
        Ok(check) => check,
        Err(error) => {
            return Err(BillingError::ViesUnavailable {
                country: customer_country,
                vat_number: number.to_string(),
                evidence: bounded_evidence(&error.to_string()),
            })
        }
    };
    let confirmed_number = match &check {
        ViesCheck::Valid { vat_number, .. } => vat_number.trim().to_string(),
        _ => number.to_string(),
    };
    let evidence = ViesEvidence {
        country: customer_country.clone(),
        vat_number: confirmed_number,
        outcome: check.outcome(),
        detail: bounded_evidence(check.evidence()),
    };
    match check.outcome() {
        ViesOutcome::Valid => {
            if evidence.confirms(customer) {
                Ok(VatDecision {
                    treatment: VatTreatment::ReverseCharge,
                    evidence: Some(evidence),
                })
            } else {
                Err(BillingError::ViesUnavailable {
                    country: customer_country,
                    vat_number: number.to_string(),
                    evidence: bounded_evidence(&format!(
                        "VIES confirmed a different number: {}",
                        evidence.detail
                    )),
                })
            }
        }
        // The register answered: the safe treatment is domestic VAT, with
        // the answer recorded as evidence.
        ViesOutcome::Invalid | ViesOutcome::Unknown => Ok(VatDecision {
            treatment: VatTreatment::Domestic,
            evidence: Some(evidence),
        }),
        ViesOutcome::Unavailable => Err(BillingError::ViesUnavailable {
            country: customer_country,
            vat_number: number.to_string(),
            evidence: evidence.detail,
        }),
    }
}

/// The shape-only decision, without consulting any register: outside the EU,
/// domestic (same country, a non-EU seller, or no number), or the
/// cross-border case that *would* be reverse charge with a confirmation. The
/// renderer uses this to validate domestic/outside-EU invoices without a
/// network call; it never authorizes a reverse charge by itself.
pub fn treatment_for_shape(
    seller: &BillingProfile,
    customer: &CustomerProfile,
) -> Result<VatTreatment, BillingError> {
    seller.validate()?;
    customer.validate()?;
    let seller_country = seller.country.trim().to_ascii_uppercase();
    let customer_country = customer.country.trim().to_ascii_uppercase();
    if !is_eu_member(&customer_country) {
        return Ok(VatTreatment::OutsideEu);
    }
    if seller_country == customer_country {
        return Ok(VatTreatment::Domestic);
    }
    if is_eu_member(&seller_country) && customer.vat_number.is_some() {
        return Ok(VatTreatment::ReverseCharge);
    }
    Ok(VatTreatment::Domestic)
}

/// Derive the VAT treatment with the production VIES client. The answer is
/// discarded here; callers that keep evidence use
/// [`treatment_for_with_vies`].
pub fn treatment_for(
    seller: &BillingProfile,
    customer: &CustomerProfile,
) -> Result<VatTreatment, BillingError> {
    treatment_for_with_vies(seller, customer, &HttpViesClient::default())
        .map(|decision| decision.treatment)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn seller() -> BillingProfile {
        BillingProfile {
            legal_name: "Apex Media OÜ".into(),
            address_lines: vec!["Sepapaja tn 6".into(), "15551 Tallinn".into()],
            country: "EE".into(),
            vat_number: "EE123456789".into(),
            vat_rate_bp: 1_900,
        }
    }

    fn customer(country: &str, vat_number: Option<&str>) -> CustomerProfile {
        CustomerProfile {
            legal_name: "Buyer OÜ".into(),
            address_lines: vec!["Telliskivi 1".into()],
            country: country.into(),
            vat_number: vat_number.map(str::to_string),
        }
    }

    /// A stub VIES client. Tests always inject one; no test reaches the
    /// network.
    #[derive(Debug)]
    struct StubVies {
        answer: StubAnswer,
        calls: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum StubAnswer {
        Valid,
        ValidFor(String),
        Invalid,
        Unknown,
        Unavailable,
    }

    impl StubVies {
        fn new(answer: StubAnswer) -> Self {
            Self {
                answer,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl ViesClient for StubVies {
        fn check_vat_number(&self, country: &str, number: &str) -> Result<ViesCheck, ViesError> {
            self.calls
                .lock()
                .unwrap()
                .push((country.to_string(), number.to_string()));
            let evidence = match &self.answer {
                StubAnswer::Valid => format!("VIES confirmation: {country}{number} is valid"),
                StubAnswer::ValidFor(other) => {
                    format!("VIES confirmation: register answered {other}")
                }
                StubAnswer::Invalid => format!("VIES answer: {country}{number} is invalid"),
                StubAnswer::Unknown => format!("VIES answer: no record for {country}{number}"),
                StubAnswer::Unavailable => {
                    format!("VIES unavailable for {country}{number}")
                }
            };
            Ok(match &self.answer {
                StubAnswer::Valid => ViesCheck::Valid {
                    evidence,
                    vat_number: number.to_string(),
                },
                StubAnswer::ValidFor(other) => ViesCheck::Valid {
                    evidence,
                    vat_number: other.clone(),
                },
                StubAnswer::Invalid => ViesCheck::Invalid { evidence },
                StubAnswer::Unknown => ViesCheck::Unknown { evidence },
                StubAnswer::Unavailable => ViesCheck::Unavailable { evidence },
            })
        }
    }

    fn decide(customer: &CustomerProfile, answer: StubAnswer) -> (VatDecision, StubVies) {
        let vies = StubVies::new(answer);
        let decision =
            treatment_for_with_vies(&seller(), customer, &vies).expect("a complete decision");
        (decision, vies)
    }

    #[test]
    fn the_eu_table_is_the_27_sorted_unique_members() {
        assert_eq!(EU_MEMBER_STATES.len(), 27);
        let mut sorted = EU_MEMBER_STATES;
        sorted.sort_unstable();
        assert_eq!(EU_MEMBER_STATES, sorted, "must stay sorted");
        let unique: std::collections::HashSet<&str> = EU_MEMBER_STATES.into_iter().collect();
        assert_eq!(unique.len(), 27, "no duplicates");
        for member in [
            "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GR", "HR", "HU",
            "IE", "IT", "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI", "SK",
        ] {
            assert!(is_eu_member(member), "{member} must be a member");
        }
        for non_member in ["GB", "CH", "NO", "TR", "UA", "US", "LI", "IS"] {
            assert!(!is_eu_member(non_member), "{non_member} is not in the EU");
        }
        // Membership is case-insensitive and trims surrounding whitespace.
        assert!(is_eu_member("de"));
        assert!(is_eu_member(" FR "));
        assert!(!is_eu_member(""));
        assert!(!is_eu_member("EEE"));
    }

    #[test]
    fn country_codes_are_two_ascii_letters() {
        for valid in ["EE", "ee", "De", " AA "] {
            assert!(is_valid_country_code(valid), "{valid:?}");
        }
        for invalid in ["", "E", "EEE", "E1", "1E", "ÜÜ", "E €"] {
            assert!(!is_valid_country_code(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn vat_numbers_are_shape_checked_only() {
        for valid in ["EE123456789", " DE123456789 ", "12345678", "ABCdef1234"] {
            assert!(is_valid_vat_number(valid), "{valid:?}");
        }
        for invalid in [
            "",
            "   ",
            "ABC",
            "AB-12",
            "DE 1234",
            "123456789012345",
            "DE_1234",
        ] {
            assert!(!is_valid_vat_number(invalid), "{invalid:?}");
        }
        // 4 and 14 are the inclusive bounds.
        assert!(is_valid_vat_number("1234"));
        assert!(is_valid_vat_number("12345678901234"));
        assert!(!is_valid_vat_number("123456789012345"));
    }

    #[test]
    fn treatment_is_domestic_when_countries_match() {
        // Same EU country, even with different case on the customer side.
        assert_eq!(
            treatment_for(&seller(), &customer("EE", None)).unwrap(),
            VatTreatment::Domestic
        );
        assert_eq!(
            treatment_for(&seller(), &customer("ee", Some("EE987654321"))).unwrap(),
            VatTreatment::Domestic
        );
        // Outside the EU the customer is never charged EU VAT, even in the
        // seller's own country: "no VAT outside the EU" wins over "domestic".
        let mut non_eu_seller = seller();
        non_eu_seller.country = "US".into();
        assert_eq!(
            treatment_for(&non_eu_seller, &customer("US", None)).unwrap(),
            VatTreatment::OutsideEu
        );
    }

    #[test]
    fn treatment_is_reverse_charge_only_with_a_matching_vies_confirmation() {
        let (decision, vies) = decide(&customer("DE", Some("DE123456789")), StubAnswer::Valid);
        assert_eq!(decision.treatment, VatTreatment::ReverseCharge);
        let evidence = decision.evidence.expect("the VIES answer is kept");
        assert_eq!(evidence.outcome, ViesOutcome::Valid);
        assert_eq!(evidence.vat_number, "DE123456789");
        assert!(evidence.detail.contains("confirmation"), "{evidence:?}");
        assert_eq!(
            vies.calls(),
            vec![("DE".to_string(), "DE123456789".to_string())],
            "the stub client was the one used"
        );

        // Without a VAT number the documented fallback charges domestic VAT
        // and no register call is made.
        let vies = StubVies::new(StubAnswer::Valid);
        assert_eq!(
            treatment_for_with_vies(&seller(), &customer("DE", None), &vies)
                .unwrap()
                .treatment,
            VatTreatment::Domestic
        );
        assert!(vies.calls().is_empty());

        // A non-EU seller cannot state EU reverse charge.
        let mut non_eu_seller = seller();
        non_eu_seller.country = "US".into();
        assert_eq!(
            treatment_for_with_vies(
                &non_eu_seller,
                &customer("DE", Some("DE123456789")),
                &StubVies::new(StubAnswer::Valid),
            )
            .unwrap()
            .treatment,
            VatTreatment::Domestic
        );
    }

    #[test]
    fn a_valid_answer_for_another_number_never_reverse_charges() {
        let vies = StubVies::new(StubAnswer::ValidFor("DE999999999".to_string()));
        let error = treatment_for_with_vies(&seller(), &customer("DE", Some("DE123456789")), &vies)
            .expect_err("a different confirmed number is refused");
        assert!(matches!(error, BillingError::ViesUnavailable { .. }));
        assert!(error.to_string().contains("different number"), "{error}");
        assert_eq!(vies.calls().len(), 1);
    }

    #[test]
    fn an_invalid_vies_answer_applies_domestic_vat_with_evidence() {
        let (decision, _) = decide(&customer("DE", Some("DE123456789")), StubAnswer::Invalid);
        assert_eq!(decision.treatment, VatTreatment::Domestic);
        let evidence = decision.evidence.expect("the VIES answer is kept");
        assert_eq!(evidence.outcome, ViesOutcome::Invalid);
        assert!(evidence.detail.contains("invalid"), "{evidence:?}");
    }

    #[test]
    fn an_unknown_vies_answer_is_typed_and_domestic() {
        let (decision, _) = decide(&customer("DE", Some("DE123456789")), StubAnswer::Unknown);
        assert_eq!(decision.treatment, VatTreatment::Domestic);
        let evidence = decision.evidence.expect("the VIES answer is kept");
        assert_eq!(evidence.outcome, ViesOutcome::Unknown);
        assert!(!evidence.confirms(&customer("DE", Some("DE123456789"))));

        // A prefixless national number confirmed by the register is the same
        // number: reverse charge must apply (the register answers with the
        // full form).
        let prefixless = ViesEvidence {
            country: "DE".to_string(),
            vat_number: "DE123456789".to_string(),
            outcome: ViesOutcome::Valid,
            detail: "VIES confirmation: DE123456789 is valid".to_string(),
        };
        assert!(
            prefixless.confirms(&customer("DE", Some("123456789"))),
            "the national form of the confirmed number matches"
        );
        assert!(prefixless.confirms(&customer("DE", Some("de123456789"))));
    }

    #[test]
    fn one_customer_number_is_queried_once_per_run() {
        let inner = StubVies::new(StubAnswer::Valid);
        let cache = CachedVies::new(&inner);
        for _ in 0..4 {
            let decision =
                treatment_for_with_vies(&seller(), &customer("DE", Some("DE123456789")), &cache)
                    .expect("decision");
            assert_eq!(decision.treatment, VatTreatment::ReverseCharge);
            assert_eq!(
                decision.evidence.expect("evidence").outcome,
                ViesOutcome::Valid
            );
        }
        assert_eq!(
            inner.calls(),
            vec![("DE".to_string(), "DE123456789".to_string())],
            "the register was asked once for the shared customer"
        );
        assert_eq!(cache.len(), 1);

        // The country prefix is part of the canonical key, not a second
        // question.
        cache
            .check_vat_number("DE", "123456789")
            .expect("cached answer");
        assert_eq!(inner.calls().len(), 1, "prefixless form is the same key");
    }

    #[test]
    fn an_unavailable_answer_is_replayed_within_the_run() {
        let inner = StubVies::new(StubAnswer::Unavailable);
        let cache = CachedVies::new(&inner);
        for _ in 0..2 {
            let error =
                treatment_for_with_vies(&seller(), &customer("DE", Some("DE123456789")), &cache)
                    .expect_err("unavailable");
            assert!(matches!(error, BillingError::ViesUnavailable { .. }));
        }
        assert_eq!(
            inner.calls().len(),
            1,
            "a register outage is not hammered once per invoice"
        );
    }

    #[test]
    fn an_unavailable_vies_answer_refuses_reverse_charge_typed() {
        let vies = StubVies::new(StubAnswer::Unavailable);
        let error = treatment_for_with_vies(&seller(), &customer("DE", Some("DE123456789")), &vies)
            .expect_err("unavailable is a typed refusal");
        match &error {
            BillingError::ViesUnavailable {
                country,
                vat_number,
                evidence,
            } => {
                assert_eq!(country, "DE");
                assert_eq!(vat_number, "DE123456789");
                assert!(evidence.contains("unavailable"), "{evidence}");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(vies.calls().len(), 1);
    }

    #[test]
    fn a_client_error_is_the_same_typed_refusal() {
        #[derive(Debug)]
        struct Broken;
        impl ViesClient for Broken {
            fn check_vat_number(
                &self,
                _country: &str,
                _number: &str,
            ) -> Result<ViesCheck, ViesError> {
                Err(ViesError::Transport {
                    detail: "timed out".to_string(),
                })
            }
        }
        let error =
            treatment_for_with_vies(&seller(), &customer("DE", Some("DE123456789")), &Broken)
                .expect_err("a client error is unavailable");
        assert!(matches!(error, BillingError::ViesUnavailable { .. }));
    }

    #[test]
    fn treatment_has_no_vat_outside_the_eu() {
        assert_eq!(
            treatment_for(&seller(), &customer("US", None)).unwrap(),
            VatTreatment::OutsideEu
        );
        assert_eq!(
            treatment_for(&seller(), &customer("GB", Some("GB123456789"))).unwrap(),
            VatTreatment::OutsideEu
        );
        assert_eq!(
            treatment_for(&seller(), &customer("CH", None)).unwrap(),
            VatTreatment::OutsideEu
        );
    }

    #[test]
    fn treatment_refuses_broken_profiles() {
        let mut broken_seller = seller();
        broken_seller.vat_number = "x".into();
        assert!(matches!(
            treatment_for(&broken_seller, &customer("EE", None)),
            Err(BillingError::InvalidVatNumber {
                party: "seller",
                ..
            })
        ));
        // An invalid customer VAT number is never silently ignored.
        assert!(matches!(
            treatment_for(&seller(), &customer("DE", Some("DE-123"))),
            Err(BillingError::InvalidVatNumber {
                party: "customer",
                ..
            })
        ));
        // Outside the EU, a malformed number is still refused when present.
        assert!(matches!(
            treatment_for(&seller(), &customer("US", Some("!!"))),
            Err(BillingError::InvalidVatNumber {
                party: "customer",
                ..
            })
        ));
        let mut empty_vat = customer("DE", Some("DE123456789"));
        empty_vat.vat_number = Some("  ".into());
        assert_eq!(
            treatment_for(&seller(), &empty_vat),
            Err(BillingError::MissingProfileField {
                party: "customer",
                field: "vatNumber"
            })
        );
    }

    #[test]
    fn vies_bodies_become_typed_answers_with_bounded_evidence() {
        let valid = parse_vies_body(
            r#"{"countryCode":"DE","vatNumber":"123456789","requestDate":"2026-10-17","valid":true,"name":"Beispiel GmbH","address":"Berlin"}"#,
            "DE",
            "DE123456789",
        )
        .expect("valid body");
        assert_eq!(valid.outcome(), ViesOutcome::Valid);
        assert!(valid.evidence().contains("confirmation"), "{valid:?}");
        match &valid {
            ViesCheck::Valid { vat_number, .. } => assert_eq!(vat_number, "DE123456789"),
            other => panic!("unexpected {other:?}"),
        }

        let invalid = parse_vies_body(
            r#"{"countryCode":"DE","vatNumber":"DE123456789","requestDate":"2026-10-17","valid":false,"name":"Beispiel GmbH","address":"Berlin"}"#,
            "DE",
            "DE123456789",
        )
        .expect("invalid body");
        assert_eq!(invalid.outcome(), ViesOutcome::Invalid);

        // The register's `---` placeholder is the typed unknown, not a
        // silently valid number.
        let unknown = parse_vies_body(
            r#"{"countryCode":"DE","vatNumber":"DE999999999","requestDate":"2026-10-17","valid":false,"name":"---","address":"---"}"#,
            "DE",
            "DE999999999",
        )
        .expect("unknown body");
        assert_eq!(unknown.outcome(), ViesOutcome::Unknown);

        assert!(matches!(
            parse_vies_body(r#"{"requestDate":"2026-10-17"}"#, "DE", "DE1"),
            Err(ViesError::Malformed { .. })
        ));
        assert!(parse_vies_body("not json", "DE", "DE1").is_err());

        // Evidence is bounded by characters, never bytes.
        let long = format!(
            "{{\"vatNumber\":\"DE1\",\"valid\":true,\"name\":\"{}\"}}",
            "ü".repeat(VIES_MAX_EVIDENCE_CHARS * 2)
        );
        let bounded = parse_vies_body(&long, "DE", "DE1").expect("long body");
        assert!(bounded.evidence().chars().count() <= VIES_MAX_EVIDENCE_CHARS);
        assert_eq!(
            HttpViesClient::url_for(" de ", "DE123456789"),
            "https://ec.europa.eu/taxation_customs/vies/rest-api/ms/DE/vat/123456789"
        );
    }

    #[test]
    fn a_live_client_refuses_a_shape_broken_number_before_spawning_curl() {
        let client = HttpViesClient {
            binary: "/does/not/exist/curl".to_string(),
            timeout_secs: 1,
        };
        assert!(matches!(
            client.check_vat_number("D", "DE1"),
            Err(ViesError::Config { .. })
        ));
        assert!(matches!(
            client.check_vat_number("DE", "!!"),
            Err(ViesError::Config { .. })
        ));
    }

    #[test]
    fn treatments_render_and_default_to_not_applied() {
        assert_eq!(VatTreatment::default(), VatTreatment::NotApplied);
        assert_eq!(VatTreatment::NotApplied.to_string(), "not applied");
        assert_eq!(VatTreatment::Domestic.to_string(), "domestic");
        assert_eq!(VatTreatment::ReverseCharge.to_string(), "reverse charge");
        assert_eq!(VatTreatment::OutsideEu.to_string(), "outside the EU");
        // The register serialization is camelCase.
        assert_eq!(
            serde_json::to_string(&VatTreatment::ReverseCharge).unwrap(),
            "\"reverseCharge\""
        );
        assert_eq!(
            serde_json::from_str::<VatTreatment>("\"outsideEu\"").unwrap(),
            VatTreatment::OutsideEu
        );
    }
}
