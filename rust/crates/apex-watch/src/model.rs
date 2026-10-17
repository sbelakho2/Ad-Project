//! The subscriber record, its JSON shape and the sign-up validation.
//!
//! One [`Subscriber`] is one publisher's watch subscription: the publisher
//! slug, the watched domain, the contact email, the tier, optional
//! per-network account ids and the check history. The record is serialized
//! camelCase, so `created_ms` is `createdMs` on disk and `last_alert_at` is
//! `lastAlertAt`.
//!
//! `subscribe` (in [`crate::store`]) normalizes the sign-up through
//! [`validate_watch_signup`] first: the domain is normalized to a bare
//! lowercase host (no scheme, path, port or reserved name), the email is
//! trimmed and the account-id textarea is parsed into `(network, account)`
//! pairs. Every field error carries the stable house copy so the public form
//! can re-render it verbatim.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use apex_billing::CustomerProfile;

use crate::fetch::normalize_domain;

/// The free-form account-id map cap, in characters of the textarea input.
pub const MAX_ACCOUNT_TEXT_CHARS: usize = 2_000;
/// The most account-id rows one submission may carry.
pub const MAX_ACCOUNT_ROWS: usize = 20;
/// The evidence threshold that makes a sign-up the core-portfolio tier: a
/// publisher with at least this many apps is core. The tier is derived from
/// the evidence (the app count the caller observed) and never from a form
/// checkbox.
pub const CORE_TIER_MIN_APP_COUNT: u32 = 10;
/// Lifetime of the one-time email-confirmation token, mirroring the
/// self-serve sign-in link: 72 hours.
pub const CONFIRMATION_TTL_MS: i64 = 72 * 60 * 60 * 1000;
/// Length of the one-time confirmation token: 32 lowercase hex characters
/// (128 random bits), the same shape as the console sign-in token.
pub const CONFIRMATION_TOKEN_HEX_LEN: usize = 32;

/// House copy for an unusable domain.
pub const DOMAIN_ERROR: &str = "Enter the domain where your app-ads.txt lives.";
/// House copy for an unusable email address.
pub const EMAIL_ERROR: &str = "Enter an email address we can reply to.";
/// House copy for an unparseable account-id list.
pub const ACCOUNTS_ERROR: &str = "List one network = account id per line.";
/// House copy for an over-long account-id list.
pub const ACCOUNTS_LENGTH_ERROR: &str = "Shorten the account list to 2,000 characters.";
/// House copy for too many account-id rows.
pub const ACCOUNTS_ROWS_ERROR: &str = "List at most 20 network accounts.";
/// The summary line above a field-error list.
pub const FORM_SUMMARY: &str = "Check the highlighted fields before sending.";

/// One field-level validation error of the public sign-up form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchFieldError {
    /// The form field id (`domain`, `email`, `accounts`).
    pub field: &'static str,
    /// The stable house copy shown at the field.
    pub message: &'static str,
}

/// The subscription tier: the standard $99 watch or the core-portfolio $199
/// watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// The standard tier.
    Normal,
    /// The core-portfolio tier (the published $199 price).
    Core,
}

impl Tier {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Core => "core",
        }
    }
}

/// Derive the subscription tier from the evidence: the number of apps the
/// publisher reported. A publisher with at least
/// [`CORE_TIER_MIN_APP_COUNT`] apps is the core-portfolio tier; every smaller
/// count is the standard tier. The sign-up form has no tier input that this
/// reads: a checkbox can never buy the higher tier.
#[must_use]
pub fn tier_from_evidence(app_count: u32) -> Tier {
    if app_count >= CORE_TIER_MIN_APP_COUNT {
        Tier::Core
    } else {
        Tier::Normal
    }
}

/// Whether the subscription is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The sign-up exists but has not paid yet: no check runs.
    Pending,
    /// The watch is on and the monthly line bills.
    Active,
    /// The watch is off; cancelled rows stay recorded, never deleted.
    Canceled,
}

impl Status {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Canceled => "canceled",
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One watch subscription.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscriber {
    /// The stable subscriber id (`sub_` plus 10 hex characters).
    pub id: String,
    /// The publisher slug derived from the domain.
    pub publisher: String,
    /// The watched domain, normalized (lowercase bare host).
    pub domain: String,
    /// The contact email for alerts.
    pub email: String,
    /// The subscription tier.
    pub tier: Tier,
    /// Optional per-network account ids, stored as one map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub account_ids: BTreeMap<String, String>,
    /// When the subscription was recorded, in unix milliseconds.
    pub created_ms: i64,
    /// Whether the watch is on.
    pub status: Status,
    /// When the cancellation was recorded, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canceled_ms: Option<i64>,
    /// Whether the creating email address was confirmed through the one-time
    /// link. Legacy rows that predate the confirmation flow default to
    /// `true`, so an already-active deployment keeps checking; the pending
    /// sign-up flow always writes `false` explicitly.
    #[serde(default = "confirmed_by_default")]
    pub confirmed: bool,
    /// SHA-256 (lowercase hex) of the one-time confirmation token. The token
    /// itself is returned once by `subscribe_pending` and never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_token_hash: Option<String>,
    /// Absolute expiry of the confirmation token, in unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_expires_at: Option<i64>,
    /// When the confirmation token was consumed, in unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_used_at: Option<i64>,
    /// When the creating email address was confirmed, in unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_confirmed_at: Option<i64>,
    /// When the payment was accepted and the watch switched on, in unix
    /// milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_at: Option<i64>,
    /// The Adyen payment link created for this sign-up, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_link: Option<String>,
    /// The Adyen `shopperReference` the payment link was created with: the
    /// subscriber id, so the stored card and the subscriber row are the same
    /// identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shopper_reference: Option<String>,
    /// The stored card (Adyen stored payment method id) the monthly bill
    /// charges. Recorded by the webhook that stores the card on the first
    /// payment; absent until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_payment_method_id: Option<String>,
    /// When the last daily check ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check_at: Option<i64>,
    /// SHA-256 (lowercase hex) of the canonical JSON of `last_findings`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_findings_hash: Option<String>,
    /// The findings recorded by the last successful check, kept so the next
    /// check can diff against them instead of a bare hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_findings: Option<Vec<crate::findings::StoredFinding>>,
    /// When the last alert email was accepted, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_alert_at: Option<i64>,
    /// SHA-256 (lowercase hex) of the cancel token. The token itself is
    /// returned once by `subscribe` and never stored.
    pub cancel_token_hash: String,
    /// The customer billing profile the monthly line renders with, when the
    /// operator has recorded one on the subscriber row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing: Option<CustomerProfile>,
    /// The last month (`YYYY-MM`) this subscriber was billed, so a rerun of
    /// the monthly pass can never issue a second line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_billed_month: Option<String>,
    /// The last month whose issued invoice was successfully charged to the
    /// stored card. When it trails [`Self::last_billed_month`], the invoice is
    /// issued but unpaid and the next monthly run retries the same charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_paid_month: Option<String>,
    /// The register number of the last issued invoice, kept so an unpaid
    /// invoice can be retried without re-reading or re-issuing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_invoice_number: Option<u64>,
    /// The deterministic Adyen reference of the last charge attempt (the
    /// idempotency key the retry reuses).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_charge_reference: Option<String>,
    /// The gross amount (micros) of the last issued invoice, so a retry
    /// charges exactly the invoiced amount.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_charge_amount_micros: Option<i64>,
    /// Why the last charge attempt failed, when it did. The next monthly run
    /// retries; the reason is never hidden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_charge_error: Option<String>,
}

impl Subscriber {
    /// Whether the watch is active.
    pub fn is_active(&self) -> bool {
        self.status == Status::Active
    }

    /// Whether the watch may run a check: active and with a confirmed email.
    pub fn is_checkable(&self) -> bool {
        self.is_active() && self.confirmed
    }
}

/// Legacy rows predate the confirmation flow and count as confirmed, so an
/// already-active deployment keeps checking after an upgrade.
fn confirmed_by_default() -> bool {
    true
}

/// One public sign-up as the pending flow consumes it. `app_count` is the
/// evidence the tier is derived from; there is deliberately no tier checkbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeRequest {
    /// The submitted domain (normalized by the store).
    pub domain: String,
    /// The submitted contact email (trimmed by the store).
    pub email: String,
    /// The optional `network = account id` textarea.
    pub accounts: String,
    /// The number of apps the sign-up reported; the tier evidence.
    pub app_count: u32,
}

/// What one accepted `subscribe_pending` call reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOutcome {
    /// The subscriber id the caller uses as its durable reference (and as the
    /// Adyen `shopperReference`).
    pub reference: String,
    /// The tier derived from the reported app count.
    pub tier: Tier,
    /// The plaintext one-time confirmation token, returned exactly once so
    /// the caller can email it. Only its SHA-256 hash is stored.
    pub confirmation_token: String,
}

/// What one accepted `confirm_email` call reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmOutcome {
    /// The subscriber whose email was confirmed.
    pub reference: String,
    /// The confirmed email address.
    pub email: String,
    /// When the confirmation was recorded, in unix milliseconds.
    pub confirmed_at: i64,
}

/// What one `activate` call reports. An already-active row is a typed no-op:
/// `already_active` is `true` and no second row is appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateOutcome {
    /// The subscriber the reference named.
    pub reference: String,
    /// When the payment was first accepted, in unix milliseconds.
    pub paid_at: i64,
    /// Whether the row was already active before this call.
    pub already_active: bool,
}

/// The validated sign-up as the store consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchSignup {
    /// The normalized domain.
    pub domain: String,
    /// The trimmed contact email.
    pub email: String,
    /// The parsed per-network account ids.
    pub account_ids: BTreeMap<String, String>,
    /// Whether the core-portfolio tier was chosen.
    pub core: bool,
}

impl WatchSignup {
    /// The subscriber tier of this sign-up.
    pub fn tier(&self) -> Tier {
        if self.core {
            Tier::Core
        } else {
            Tier::Normal
        }
    }
}

/// What one accepted `subscribe` call reports.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeOutcome {
    /// The live subscriber row (the existing one when the submission was a
    /// duplicate inside the dedup window).
    pub subscriber: Subscriber,
    /// Whether a new row was appended.
    pub created: bool,
    /// The cancel token, returned exactly once on creation. A deduplicated
    /// repeat returns `None` because the original token is never stored.
    pub cancel_token: Option<String>,
}

/// Validate one public sign-up. The domain must normalize to a public host,
/// the email must be a single address and the account list must parse into
/// `network = account id` pairs. Every failure is house copy at the field.
pub fn validate_watch_signup(
    domain: &str,
    email: &str,
    accounts: &str,
    core: bool,
) -> Result<WatchSignup, Vec<WatchFieldError>> {
    let mut errors = Vec::new();
    let domain = match normalize_domain(domain) {
        Ok(domain) => domain,
        Err(_) => {
            errors.push(WatchFieldError {
                field: "domain",
                message: DOMAIN_ERROR,
            });
            String::new()
        }
    };
    let email = email.trim().to_string();
    if !is_email(&email) {
        errors.push(WatchFieldError {
            field: "email",
            message: EMAIL_ERROR,
        });
    }
    let account_ids = match parse_account_ids(accounts) {
        Ok(rows) => rows,
        Err(error) => {
            errors.push(error);
            BTreeMap::new()
        }
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(WatchSignup {
        domain,
        email,
        account_ids,
        core,
    })
}

/// A minimal single-address check: printable ASCII, one `@`, a dotted domain
/// and no whitespace or option-shaped local part.
fn is_email(value: &str) -> bool {
    if value.is_empty() || value.len() > 254 || value.starts_with('-') {
        return false;
    }
    if !value
        .chars()
        .all(|c| c.is_ascii() && !c.is_ascii_whitespace() && !c.is_control())
    {
        return false;
    }
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return false;
    }
    domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// Parse the optional account-id textarea: one `network = account id` per
/// line, with `network, account id` accepted as the same shape. Blank lines
/// are skipped, network names are lowercased and later duplicates win.
pub fn parse_account_ids(accounts: &str) -> Result<BTreeMap<String, String>, WatchFieldError> {
    if accounts.chars().count() > MAX_ACCOUNT_TEXT_CHARS {
        return Err(WatchFieldError {
            field: "accounts",
            message: ACCOUNTS_LENGTH_ERROR,
        });
    }
    let mut rows: BTreeMap<String, String> = BTreeMap::new();
    for line in accounts.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let split = line
            .split_once('=')
            .or_else(|| line.split_once(','))
            .ok_or(WatchFieldError {
                field: "accounts",
                message: ACCOUNTS_ERROR,
            })?;
        let network = split.0.trim().to_ascii_lowercase();
        let account = split.1.trim();
        let network_ok = !network.is_empty()
            && network.len() <= 253
            && network.contains('.')
            && network
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
        if !network_ok || account.is_empty() || account.len() > 64 {
            return Err(WatchFieldError {
                field: "accounts",
                message: ACCOUNTS_ERROR,
            });
        }
        rows.insert(network, account.to_string());
    }
    if rows.len() > MAX_ACCOUNT_ROWS {
        return Err(WatchFieldError {
            field: "accounts",
            message: ACCOUNTS_ROWS_ERROR,
        });
    }
    Ok(rows)
}

/// The publisher slug for a domain: dots and other separators collapse to
/// one `-`, ASCII alphanumerics and `_` stay, the result is at most 48
/// characters. An empty result becomes `watch-subscriber`.
pub fn publisher_slug_for_domain(domain: &str) -> String {
    let mut slug = String::new();
    for ch in domain.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '_' {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' && (slug.is_empty() || slug.ends_with('-')) {
            continue;
        }
        slug.push(mapped);
    }
    let slug = slug
        .trim_end_matches('-')
        .chars()
        .take(48)
        .collect::<String>();
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "watch-subscriber".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_signups_normalize_to_the_stored_shape() {
        let signup = validate_watch_signup(
            "  Northstar.Example ",
            " founder@northstar.example ",
            "admob.com = pub-1\nunityads.com, pub-2\n\n",
            true,
        )
        .expect("valid");
        assert_eq!(signup.domain, "northstar.example");
        assert_eq!(signup.email, "founder@northstar.example");
        assert_eq!(signup.tier(), Tier::Core);
        assert_eq!(
            signup.account_ids.get("admob.com").map(String::as_str),
            Some("pub-1")
        );
        assert_eq!(
            signup.account_ids.get("unityads.com").map(String::as_str),
            Some("pub-2")
        );
    }

    #[test]
    fn every_bad_field_carries_its_house_copy() {
        let errors =
            validate_watch_signup("not a domain", "nope", "garbage", false).expect_err("refused");
        assert_eq!(errors.len(), 3);
        assert_eq!(
            errors[0],
            WatchFieldError {
                field: "domain",
                message: DOMAIN_ERROR,
            }
        );
        assert_eq!(
            errors[1],
            WatchFieldError {
                field: "email",
                message: EMAIL_ERROR,
            }
        );
        assert_eq!(
            errors[2],
            WatchFieldError {
                field: "accounts",
                message: ACCOUNTS_ERROR,
            }
        );
    }

    #[test]
    fn private_and_numeric_domains_are_refused() {
        for raw in [
            "",
            "localhost",
            "127.0.0.1",
            "example.local",
            "https://example.com:8443",
            "user@example.com",
        ] {
            assert!(
                validate_watch_signup(raw, "a@x.example", "", false).is_err(),
                "accepted {raw:?}"
            );
        }
    }

    #[test]
    fn account_rows_are_bounded_and_later_duplicates_win() {
        assert!(parse_account_ids("").expect("empty").is_empty());
        let rows = parse_account_ids("admob.com = 1\nADMOB.com = 2").expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows.get("admob.com").map(String::as_str), Some("2"));

        let many: String = (0..(MAX_ACCOUNT_ROWS + 1))
            .map(|i| format!("net{i}.example = pub-{i}\n"))
            .collect();
        assert_eq!(
            parse_account_ids(&many).expect_err("too many"),
            WatchFieldError {
                field: "accounts",
                message: ACCOUNTS_ROWS_ERROR,
            }
        );

        let long = "a".repeat(MAX_ACCOUNT_TEXT_CHARS + 1);
        assert_eq!(
            parse_account_ids(&long).expect_err("too long"),
            WatchFieldError {
                field: "accounts",
                message: ACCOUNTS_LENGTH_ERROR,
            }
        );
    }

    #[test]
    fn email_shape_is_minimal_and_strict() {
        for good in [
            "a@x.example",
            "founder+ads@studio.co.uk",
            "ops_1@sub.domain.example",
        ] {
            assert!(is_email(good), "{good}");
        }
        for bad in [
            "",
            "nope",
            "@x.example",
            "a@",
            "a@b",
            "a b@x.example",
            "a@x..example",
            "-a@x.example",
        ] {
            assert!(!is_email(bad), "accepted {bad:?}");
        }
    }

    #[test]
    fn publisher_slugs_come_from_the_domain() {
        assert_eq!(
            publisher_slug_for_domain("northstar.example"),
            "northstar-example"
        );
        assert_eq!(
            publisher_slug_for_domain("sub.studio.example"),
            "sub-studio-example"
        );
        assert_eq!(publisher_slug_for_domain("!!!"), "watch-subscriber");
    }

    #[test]
    fn the_tier_comes_from_the_app_count_boundary_never_a_checkbox() {
        assert_eq!(tier_from_evidence(0), Tier::Normal);
        assert_eq!(
            tier_from_evidence(CORE_TIER_MIN_APP_COUNT - 1),
            Tier::Normal
        );
        assert_eq!(tier_from_evidence(CORE_TIER_MIN_APP_COUNT), Tier::Core);
        assert_eq!(tier_from_evidence(CORE_TIER_MIN_APP_COUNT + 1), Tier::Core);
        assert_eq!(tier_from_evidence(u32::MAX), Tier::Core);
    }

    #[test]
    fn subscriber_rows_round_trip_camel_case() {
        let mut account_ids = BTreeMap::new();
        account_ids.insert("admob.com".to_string(), "pub-1".to_string());
        let subscriber = Subscriber {
            id: "sub_0123456789".to_string(),
            publisher: "northstar-example".to_string(),
            domain: "northstar.example".to_string(),
            email: "founder@northstar.example".to_string(),
            tier: Tier::Normal,
            account_ids,
            created_ms: 7,
            status: Status::Active,
            canceled_ms: None,
            confirmed: true,
            confirmation_token_hash: Some("ef".to_string()),
            confirmation_expires_at: Some(11),
            confirmation_used_at: Some(12),
            email_confirmed_at: Some(12),
            paid_at: Some(13),
            payment_link: Some("https://checkout.example/pl_1".to_string()),
            shopper_reference: Some("sub_0123456789".to_string()),
            stored_payment_method_id: Some("card_1".to_string()),
            last_check_at: Some(9),
            last_findings_hash: Some("ab".to_string()),
            last_findings: Some(vec![crate::findings::StoredFinding {
                subject: "app_ads_txt".to_string(),
                severity: "critical".to_string(),
                message: "no app-ads.txt found".to_string(),
            }]),
            last_alert_at: Some(10),
            cancel_token_hash: "cd".to_string(),
            billing: None,
            last_billed_month: Some("2026-10".to_string()),
            last_paid_month: Some("2026-10".to_string()),
            last_invoice_number: Some(4),
            last_charge_reference: Some("watch:sub_0123456789:2026-10".to_string()),
            last_charge_amount_micros: Some(118_800_000),
            last_charge_error: None,
        };
        let json = serde_json::to_string(&subscriber).expect("serialize");
        assert!(json.contains("\"createdMs\":7"), "{json}");
        assert!(json.contains("\"lastAlertAt\":10"), "{json}");
        assert!(json.contains("\"cancelTokenHash\":\"cd\""), "{json}");
        assert!(json.contains("\"tier\":\"normal\""), "{json}");
        assert!(json.contains("\"lastBilledMonth\":\"2026-10\""), "{json}");
        assert!(json.contains("\"confirmationTokenHash\":\"ef\""), "{json}");
        assert!(
            json.contains("\"storedPaymentMethodId\":\"card_1\""),
            "{json}"
        );
        let back: Subscriber = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, subscriber);
    }
}
