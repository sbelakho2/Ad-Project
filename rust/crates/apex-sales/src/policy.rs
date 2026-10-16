//! Who may be emailed at all: legal basis, suppression and account-level
//! coordination. These are hard gates; the decision engine never weighs
//! them against expected value.
//!
//! # Legal basis
//!
//! A versioned country table decides whether cold B2B email needs prior
//! consent. The default table treats Germany as consent-only (UWG §7 para. 2
//! generally requires prior consent for email advertising, with no general
//! business-to-business exemption) and every other country as
//! opt-out-with-identification for business addresses. In the EU and EEA, a
//! free-webmail address is treated as a natural person's and needs consent.
//! Unknown countries follow [`LegalTable::unknown`]. **This table is a
//! starting point, not legal advice**: the operator owns it.
//!
//! # Account coordination
//!
//! One live sequence per publisher; a cool-off after "not now" (60 days) or
//! "not interested" (180 days); a spam complaint from anyone at a publisher
//! stops the whole publisher; any interested reply hands the publisher to a
//! person and stops automation.

use crate::model::{Contact, ReplyClass, UnixSecs, DAY};

/// Basis for contacting someone in a country.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Business addresses may be emailed with identification and an opt-out.
    OptOut,
    /// Prior consent is required.
    ConsentRequired,
    /// A person must decide.
    Review,
}

/// Verdict for one contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegalVerdict {
    /// May be emailed.
    Allowed,
    /// Needs consent the engine does not have.
    ConsentRequired(String),
    /// A person must decide.
    Review(String),
}

/// The country rule table.
#[derive(Debug, Clone, PartialEq)]
pub struct LegalTable {
    /// Table version, recorded on every decision.
    pub version: String,
    /// Explicit rules by ISO alpha-2 country.
    pub rules: Vec<(String, Rule)>,
    /// Rule for countries with no row and no inference.
    pub unknown: Rule,
}

/// EU and EEA countries (ISO alpha-2).
pub const EU_EEA: [&str; 30] = [
    "AT", "BE", "BG", "HR", "CY", "CZ", "DK", "EE", "FI", "FR", "DE", "GR", "HU", "IE", "IT", "LV",
    "LT", "LU", "MT", "NL", "PL", "PT", "RO", "SK", "SI", "ES", "SE", "IS", "LI", "NO",
];

/// Free-webmail domains: their users are treated as natural persons.
pub const FREE_MAIL: [&str; 16] = [
    "gmail.com",
    "yahoo.com",
    "hotmail.com",
    "outlook.com",
    "live.com",
    "icloud.com",
    "me.com",
    "aol.com",
    "gmx.de",
    "gmx.net",
    "web.de",
    "mail.ru",
    "yandex.ru",
    "qq.com",
    "163.com",
    "proton.me",
];

impl Default for LegalTable {
    fn default() -> Self {
        Self {
            version: "default-2026-10".into(),
            rules: vec![("DE".into(), Rule::ConsentRequired)],
            unknown: Rule::OptOut,
        }
    }
}

/// Country from a country-code top-level domain, when it is one.
pub fn cctld_country(domain: &str) -> Option<String> {
    let tld = domain.rsplit('.').next()?.to_ascii_uppercase();
    let tld = if tld == "UK" { "GB".to_string() } else { tld };
    (tld.len() == 2 && tld.chars().all(|c| c.is_ascii_alphabetic())).then_some(tld)
}

impl LegalTable {
    /// Rule for a country.
    pub fn rule(&self, country: &str) -> Rule {
        self.rules
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(country))
            .map_or(Rule::OptOut, |(_, r)| *r)
    }

    /// Verdict for `contact` at a publisher in `country`. The email's
    /// ccTLD is used when the publisher's country is unknown.
    pub fn evaluate(&self, country: Option<&str>, contact: &Contact) -> LegalVerdict {
        if contact.consent {
            return LegalVerdict::Allowed;
        }
        let domain = contact.email.rsplit('@').next().unwrap_or_default();
        let inferred = country
            .map(str::to_ascii_uppercase)
            .or_else(|| cctld_country(domain));
        let rule = match &inferred {
            Some(c) => self.rule(c),
            None => self.unknown,
        };
        let where_ = inferred.clone().unwrap_or_else(|| "unknown country".into());
        match rule {
            Rule::ConsentRequired => LegalVerdict::ConsentRequired(format!(
                "{where_} requires prior consent ({})",
                self.version
            )),
            Rule::Review => {
                LegalVerdict::Review(format!("{where_}: operator review ({})", self.version))
            }
            Rule::OptOut => {
                let free = FREE_MAIL.contains(&domain);
                let eu = inferred.as_deref().is_some_and(|c| EU_EEA.contains(&c));
                if free && eu {
                    LegalVerdict::ConsentRequired(format!(
                        "free-webmail address in {where_} is treated as a person ({})",
                        self.version
                    ))
                } else {
                    LegalVerdict::Allowed
                }
            }
        }
    }
}

/// Why an address or publisher is suppressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionReason {
    /// Unsubscribed or asked to be removed.
    OptOut,
    /// Marked a message as spam (applies to the whole publisher).
    Complaint,
    /// Permanent delivery failure.
    HardBounce,
    /// Blocked by the operator.
    Operator,
}

/// The suppression list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Suppressions {
    /// Suppressed addresses.
    pub emails: Vec<(String, SuppressionReason)>,
    /// Suppressed publishers.
    pub publishers: Vec<(String, SuppressionReason)>,
}

impl Suppressions {
    /// Add an address.
    pub fn suppress_email(&mut self, email: &str, reason: SuppressionReason) {
        if !self.emails.iter().any(|(e, _)| e == email) {
            self.emails.push((email.to_string(), reason));
        }
    }

    /// Add a publisher.
    pub fn suppress_publisher(&mut self, key: &str, reason: SuppressionReason) {
        if !self.publishers.iter().any(|(p, _)| p == key) {
            self.publishers.push((key.to_string(), reason));
        }
    }

    /// Why `contact` is suppressed, if it is.
    pub fn check(&self, contact: &Contact) -> Option<SuppressionReason> {
        self.emails
            .iter()
            .find(|(e, _)| *e == contact.email)
            .or_else(|| {
                self.publishers
                    .iter()
                    .find(|(p, _)| *p == contact.publisher_key)
            })
            .map(|(_, r)| *r)
    }
}

/// What the account-coordination rules know about a publisher.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountState {
    /// Contacts with a live (unfinished) sequence.
    pub active_contacts: Vec<String>,
    /// Latest classified reply and when it arrived.
    pub last_reply: Option<(ReplyClass, UnixSecs)>,
}

/// Account-coordination verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountVerdict {
    /// May start or continue.
    Allowed,
    /// Wait until the given time.
    Deferred(UnixSecs, String),
    /// Stop.
    Denied(String),
}

/// Coordination limits.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountRules {
    /// Live sequences allowed per publisher.
    pub max_active_contacts: usize,
    /// Cool-off after "not now".
    pub not_now_days: i64,
    /// Cool-off after "not interested".
    pub not_interested_days: i64,
}

impl Default for AccountRules {
    fn default() -> Self {
        Self {
            max_active_contacts: 1,
            not_now_days: 60,
            not_interested_days: 180,
        }
    }
}

impl AccountRules {
    /// Whether `email` may be (or keep being) contacted at a publisher in
    /// `state` at `now`.
    pub fn evaluate(&self, state: &AccountState, email: &str, now: UnixSecs) -> AccountVerdict {
        if let Some((class, at)) = state.last_reply {
            match class {
                ReplyClass::Positive
                | ReplyClass::Referral
                | ReplyClass::Question
                | ReplyClass::Unclear => {
                    return AccountVerdict::Denied(format!(
                        "a person must handle the {} reply",
                        class.as_str()
                    ));
                }
                ReplyClass::OptOut => {
                    return AccountVerdict::Denied("someone at this publisher opted out".into());
                }
                ReplyClass::NotNow | ReplyClass::NotInterested => {
                    let days = if class == ReplyClass::NotNow {
                        self.not_now_days
                    } else {
                        self.not_interested_days
                    };
                    let until = at.saturating_add(days.saturating_mul(DAY));
                    if now < until {
                        return AccountVerdict::Deferred(
                            until,
                            format!("{} reply cool-off of {days} days", class.as_str()),
                        );
                    }
                }
            }
        }
        let others = state.active_contacts.iter().filter(|c| *c != email).count();
        let already = state.active_contacts.iter().any(|c| c == email);
        if !already && others >= self.max_active_contacts {
            return AccountVerdict::Denied(format!(
                "{others} other contact(s) already in a sequence at this publisher"
            ));
        }
        AccountVerdict::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ContactKind;

    fn contact(email: &str) -> Contact {
        Contact {
            email: email.into(),
            name: None,
            role: None,
            publisher_key: "studio.example".into(),
            kind: ContactKind::of_email(email),
            source: "t".into(),
            consent: false,
        }
    }

    #[test]
    fn legal_table_defaults() {
        let t = LegalTable::default();
        assert_eq!(
            t.evaluate(Some("US"), &contact("a@studio.example")),
            LegalVerdict::Allowed
        );
        assert!(matches!(
            t.evaluate(Some("de"), &contact("a@studio.example")),
            LegalVerdict::ConsentRequired(_)
        ));
        // ccTLD inference when the country is unknown.
        assert!(matches!(
            t.evaluate(None, &contact("a@studio.de")),
            LegalVerdict::ConsentRequired(_)
        ));
        // Free webmail in the EU is a person.
        assert!(matches!(
            t.evaluate(Some("FR"), &contact("dev@gmail.com")),
            LegalVerdict::ConsentRequired(_)
        ));
        assert_eq!(
            t.evaluate(Some("US"), &contact("dev@gmail.com")),
            LegalVerdict::Allowed
        );
        let mut consented = contact("a@studio.de");
        consented.consent = true;
        assert_eq!(t.evaluate(Some("DE"), &consented), LegalVerdict::Allowed);
        let strict = LegalTable {
            unknown: Rule::Review,
            rules: vec![("TR".into(), Rule::Review)],
            ..LegalTable::default()
        };
        assert!(matches!(
            strict.evaluate(None, &contact("a@studio.com")),
            LegalVerdict::Review(_)
        ));
        assert!(matches!(
            strict.evaluate(Some("TR"), &contact("a@x.com")),
            LegalVerdict::Review(_)
        ));
        assert_eq!(cctld_country("studio.co.uk"), Some("GB".into()));
        assert_eq!(cctld_country("studio.games"), None);
    }

    #[test]
    fn suppressions_cover_addresses_and_publishers() {
        let mut s = Suppressions::default();
        s.suppress_email("a@studio.example", SuppressionReason::OptOut);
        s.suppress_email("a@studio.example", SuppressionReason::HardBounce);
        assert_eq!(s.emails.len(), 1);
        assert_eq!(
            s.check(&contact("a@studio.example")),
            Some(SuppressionReason::OptOut)
        );
        assert_eq!(s.check(&contact("b@studio.example")), None);
        s.suppress_publisher("studio.example", SuppressionReason::Complaint);
        s.suppress_publisher("studio.example", SuppressionReason::Operator);
        assert_eq!(
            s.check(&contact("b@studio.example")),
            Some(SuppressionReason::Complaint)
        );
    }

    #[test]
    fn account_rules() {
        let r = AccountRules::default();
        let now = 1_000 * DAY;
        let mut st = AccountState::default();
        assert_eq!(r.evaluate(&st, "a@x.io", now), AccountVerdict::Allowed);
        st.active_contacts = vec!["a@x.io".into()];
        assert_eq!(r.evaluate(&st, "a@x.io", now), AccountVerdict::Allowed);
        assert!(matches!(
            r.evaluate(&st, "b@x.io", now),
            AccountVerdict::Denied(_)
        ));
        st.active_contacts.clear();
        st.last_reply = Some((ReplyClass::NotNow, now - 10 * DAY));
        assert!(
            matches!(r.evaluate(&st, "a@x.io", now), AccountVerdict::Deferred(t, _) if t == now + 50 * DAY)
        );
        st.last_reply = Some((ReplyClass::NotNow, now - 61 * DAY));
        assert_eq!(r.evaluate(&st, "a@x.io", now), AccountVerdict::Allowed);
        st.last_reply = Some((ReplyClass::NotInterested, now - 61 * DAY));
        assert!(matches!(
            r.evaluate(&st, "a@x.io", now),
            AccountVerdict::Deferred(..)
        ));
        for class in [
            ReplyClass::Positive,
            ReplyClass::OptOut,
            ReplyClass::Unclear,
        ] {
            st.last_reply = Some((class, now));
            assert!(matches!(
                r.evaluate(&st, "a@x.io", now),
                AccountVerdict::Denied(_)
            ));
        }
        st.last_reply = Some((ReplyClass::NotNow, i64::MAX - 5));
        assert!(matches!(
            r.evaluate(&st, "a@x.io", now),
            AccountVerdict::Deferred(..)
        ));
    }
}
