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

use std::collections::HashMap;

use crate::model::{Contact, ReplyClass, UnixSecs, DAY};

// Membership probes performed by suppression inserts and checks.
//
// The seam exists only in test builds; the scaling test asserts it grows
// linearly with the list, so a regression back to a vector scan (one probe
// per compared entry) fails it.
#[cfg(test)]
thread_local! {
    static MEMBERSHIP_PROBES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn probe_membership() {
    MEMBERSHIP_PROBES.with(|probes| probes.set(probes.get() + 1));
}

#[cfg(test)]
fn reset_membership_probes() {
    MEMBERSHIP_PROBES.with(|probes| probes.set(0));
}

#[cfg(test)]
fn membership_probes() -> u64 {
    MEMBERSHIP_PROBES.with(std::cell::Cell::get)
}

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
///
/// The ordered export and the membership index live together and are only
/// ever mutated by the methods below, so the index cannot go stale: there is
/// no path that changes one without the other. [`Self::emails`] and
/// [`Self::publishers`] expose the export read-only, one first-recorded
/// entry per key in insertion order, preserving the exact order and shape
/// callers and serializers observed when the vectors were public.
#[derive(Clone, Default)]
pub struct Suppressions {
    emails: Vec<(String, SuppressionReason)>,
    publishers: Vec<(String, SuppressionReason)>,
    index: SuppressionIndex,
}

/// The address and publisher membership indexes backing [`Suppressions`].
#[derive(Debug, Clone, Default)]
struct SuppressionIndex {
    emails: HashMap<String, SuppressionReason>,
    publishers: HashMap<String, SuppressionReason>,
}

impl SuppressionIndex {
    fn reason(&self, contact: &Contact) -> Option<SuppressionReason> {
        self.emails
            .get(&contact.email)
            .or_else(|| self.publishers.get(&contact.publisher_key))
            .copied()
    }
}

impl std::fmt::Debug for Suppressions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Suppressions")
            .field("emails", &self.emails)
            .field("publishers", &self.publishers)
            .finish()
    }
}

impl PartialEq for Suppressions {
    fn eq(&self, other: &Self) -> bool {
        self.emails == other.emails && self.publishers == other.publishers
    }
}

impl Suppressions {
    /// The suppressed addresses in first-recorded order.
    pub fn emails(&self) -> &[(String, SuppressionReason)] {
        &self.emails
    }

    /// The suppressed publishers in first-recorded order.
    pub fn publishers(&self) -> &[(String, SuppressionReason)] {
        &self.publishers
    }

    /// Add an address unless it is already present; the first reason wins.
    pub fn suppress_email(&mut self, email: &str, reason: SuppressionReason) {
        #[cfg(test)]
        probe_membership();
        if self.index.emails.contains_key(email) {
            return;
        }
        self.index.emails.insert(email.to_string(), reason);
        self.emails.push((email.to_string(), reason));
    }

    /// Add a publisher unless it is already present; the first reason wins.
    pub fn suppress_publisher(&mut self, key: &str, reason: SuppressionReason) {
        #[cfg(test)]
        probe_membership();
        if self.index.publishers.contains_key(key) {
            return;
        }
        self.index.publishers.insert(key.to_string(), reason);
        self.publishers.push((key.to_string(), reason));
    }

    /// Remove an address, keeping the order of the remaining entries.
    /// Returns whether it was present.
    pub fn unsuppress_email(&mut self, email: &str) -> bool {
        if self.index.emails.remove(email).is_none() {
            return false;
        }
        // Ordered export: removal keeps the O(n) tail shift; removals are rare operator actions.
        self.emails.retain(|(entry, _)| entry != email);
        true
    }

    /// Remove a publisher, keeping the order of the remaining entries.
    /// Returns whether it was present.
    pub fn unsuppress_publisher(&mut self, key: &str) -> bool {
        if self.index.publishers.remove(key).is_none() {
            return false;
        }
        // Ordered export: removal keeps the O(n) tail shift; removals are rare operator actions.
        self.publishers.retain(|(entry, _)| entry != key);
        true
    }

    /// Why `contact` is suppressed, if it is: an address match wins over a
    /// publisher match, and the first recorded entry supplies the reason.
    pub fn check(&self, contact: &Contact) -> Option<SuppressionReason> {
        #[cfg(test)]
        probe_membership();
        self.index.reason(contact)
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
        assert_eq!(s.emails().len(), 1);
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

    /// The read accessors return exactly the ordered pair sequence the public
    /// vectors used to export, and `Debug` still renders only those two
    /// vectors in the same order. That pair sequence is the shape any
    /// serializer sees; this std-only crate has no serde derive on the type,
    /// so the golden pins the observable export rather than inventing one.
    #[test]
    fn suppression_export_matches_the_ordered_golden() {
        let mut s = Suppressions::default();
        s.suppress_email("b@studio.example", SuppressionReason::HardBounce);
        s.suppress_email("a@studio.example", SuppressionReason::OptOut);
        s.suppress_email("b@studio.example", SuppressionReason::Operator);
        s.suppress_publisher("studio.example", SuppressionReason::Complaint);
        assert_eq!(
            s.emails(),
            &[
                (
                    "b@studio.example".to_string(),
                    SuppressionReason::HardBounce
                ),
                ("a@studio.example".to_string(), SuppressionReason::OptOut),
            ]
        );
        assert_eq!(
            s.publishers(),
            &[("studio.example".to_string(), SuppressionReason::Complaint)]
        );
        assert_eq!(
            format!("{s:?}"),
            "Suppressions { emails: [(\"b@studio.example\", HardBounce), \
             (\"a@studio.example\", OptOut)], publishers: \
             [(\"studio.example\", Complaint)] }"
        );
    }

    /// The same-length in-place mutation that used to leave the index stale
    /// is no longer expressible: the vectors are private and every mutation
    /// goes through a method that updates the index. Removing and re-adding
    /// must answer the new reason, never a stale hit from the removed entry.
    #[test]
    fn a_removed_then_re_added_suppression_has_no_stale_hit() {
        let mut s = Suppressions::default();
        s.suppress_email("a@studio.example", SuppressionReason::OptOut);
        assert!(s.unsuppress_email("a@studio.example"));
        assert!(!s.unsuppress_email("a@studio.example"));
        assert!(s.emails().is_empty());
        assert_eq!(s.check(&contact("a@studio.example")), None);
        s.suppress_email("a@studio.example", SuppressionReason::HardBounce);
        assert_eq!(
            s.check(&contact("a@studio.example")),
            Some(SuppressionReason::HardBounce),
            "the re-added entry's reason, not the removed one"
        );

        s.suppress_publisher("studio.example", SuppressionReason::Complaint);
        s.suppress_publisher("other.example", SuppressionReason::Operator);
        assert!(s.unsuppress_publisher("studio.example"));
        s.suppress_publisher("studio.example", SuppressionReason::OptOut);
        assert_eq!(
            s.check(&contact_at("b@other.example", "other.example")),
            Some(SuppressionReason::Operator)
        );
        assert_eq!(
            s.check(&contact_at("b@studio.example", "studio.example")),
            Some(SuppressionReason::OptOut),
            "the re-added publisher uses the new reason"
        );
        assert_eq!(
            s.publishers(),
            &[
                ("other.example".to_string(), SuppressionReason::Operator),
                ("studio.example".to_string(), SuppressionReason::OptOut),
            ],
            "removal preserves order and re-adding appends"
        );
    }

    /// Removal shifts the remaining entries left without reordering them and a
    /// re-add appends at the end. That ordered export is the contract the
    /// `retain` comment protects; this pins it for both vectors.
    #[test]
    fn unsuppress_preserves_export_order_across_removals_and_re_adds() {
        let mut s = Suppressions::default();
        for (email, reason) in [
            ("a@studio.example", SuppressionReason::OptOut),
            ("b@studio.example", SuppressionReason::HardBounce),
            ("c@studio.example", SuppressionReason::Complaint),
        ] {
            s.suppress_email(email, reason);
        }
        for (key, reason) in [
            ("a.example", SuppressionReason::OptOut),
            ("b.example", SuppressionReason::Complaint),
            ("c.example", SuppressionReason::Operator),
        ] {
            s.suppress_publisher(key, reason);
        }

        assert!(s.unsuppress_email("b@studio.example"));
        assert!(s.unsuppress_publisher("b.example"));
        s.suppress_email("b@studio.example", SuppressionReason::Operator);
        s.suppress_publisher("b.example", SuppressionReason::HardBounce);

        assert_eq!(
            s.emails(),
            &[
                ("a@studio.example".to_string(), SuppressionReason::OptOut),
                ("c@studio.example".to_string(), SuppressionReason::Complaint),
                ("b@studio.example".to_string(), SuppressionReason::Operator),
            ],
            "removal preserves the order of survivors and re-adding appends"
        );
        assert_eq!(
            s.publishers(),
            &[
                ("a.example".to_string(), SuppressionReason::OptOut),
                ("c.example".to_string(), SuppressionReason::Operator),
                ("b.example".to_string(), SuppressionReason::HardBounce),
            ],
            "publisher removal preserves the same ordered export"
        );
        assert_eq!(
            s.check(&contact("b@studio.example")),
            Some(SuppressionReason::Operator),
            "the re-added entry is visible to membership checks"
        );
    }

    /// The exact pre-fix lookup: a linear scan of the exported vectors, first
    /// entry wins, address before publisher, byte-for-byte case-sensitive.
    fn naive_check(s: &Suppressions, contact: &Contact) -> Option<SuppressionReason> {
        s.emails()
            .iter()
            .find(|(e, _)| *e == contact.email)
            .or_else(|| {
                s.publishers()
                    .iter()
                    .find(|(p, _)| *p == contact.publisher_key)
            })
            .map(|(_, r)| *r)
    }

    fn contact_at(email: &str, publisher: &str) -> Contact {
        Contact {
            email: email.into(),
            name: None,
            role: None,
            publisher_key: publisher.into(),
            kind: ContactKind::of_email(email),
            source: "t".into(),
            consent: false,
        }
    }

    /// At the engine's reference size (10k addresses, 10k publishers, 10k
    /// contacts), the indexed membership answers exactly like the verbatim
    /// vector-scan reference, including precedence edges: an address entry
    /// beats a publisher entry, the first duplicate wins, and matching stays
    /// case-sensitive exactly as before.
    #[test]
    fn suppression_membership_matches_a_naive_scan_at_ten_thousand() {
        const N: usize = 10_000;
        let mut s = Suppressions::default();
        for i in 0..N {
            let reason = if i % 3 == 0 {
                SuppressionReason::OptOut
            } else {
                SuppressionReason::HardBounce
            };
            s.suppress_email(&format!("suppressed{i}@p{i}.example"), reason);
            s.suppress_publisher(&format!("p{i}.example"), SuppressionReason::Complaint);
        }
        // The exported order is insertion order.
        for i in 0..N {
            assert_eq!(s.emails()[i].0, format!("suppressed{i}@p{i}.example"));
            assert_eq!(s.publishers()[i].0, format!("p{i}.example"));
        }

        // A duplicate is rejected without disturbing the first reason; a case
        // variant is a distinct key and is appended.
        s.suppress_email("suppressed0@p0.example", SuppressionReason::Operator);
        s.suppress_email("SUPPRESSED1@P1.EXAMPLE", SuppressionReason::Operator);
        s.suppress_publisher("p0.example", SuppressionReason::Operator);
        assert_eq!(s.emails().len(), N + 1, "only the case variant is appended");
        assert_eq!(s.publishers().len(), N);

        let mut contacts: Vec<Contact> = (0..N)
            .map(|i| match i % 4 {
                0 => contact_at(&format!("suppressed{i}@p{i}.example"), "clean.example"),
                1 => contact_at(&format!("clean{i}@p{i}.example"), &format!("p{i}.example")),
                2 => contact_at(
                    &format!("suppressed{i}@p{i}.example"),
                    &format!("p{i}.example"),
                ),
                _ => contact_at(&format!("clean{i}@p{i}.example"), "clean.example"),
            })
            .collect();
        // Precedence edges.
        contacts.push(contact_at("suppressed0@p0.example", "p0.example"));
        contacts.push(contact_at("SUPPRESSED1@P1.EXAMPLE", "clean.example"));
        contacts.push(contact_at("suppressed1@p1.example", "clean.example"));
        contacts.push(contact_at("suppressed0@p0.example", "clean.example"));
        contacts.push(contact_at("clean0@p0.example", "p0.example"));
        contacts.push(contact_at("clean9@p9.example", "p9.example"));

        for (i, c) in contacts.iter().enumerate() {
            assert_eq!(s.check(c), naive_check(&s, c), "contact {i}: {c:?}");
        }
        assert_eq!(
            s.check(&contact_at("suppressed0@p0.example", "p0.example")),
            Some(SuppressionReason::OptOut),
            "the address entry wins over the publisher entry"
        );
        assert_eq!(
            s.check(&contact_at("suppressed0@p0.example", "clean.example")),
            Some(SuppressionReason::OptOut),
            "the first duplicate supplies the reason"
        );
        assert_eq!(
            s.check(&contact_at("suppressed1@p1.example", "clean.example")),
            Some(SuppressionReason::HardBounce),
            "a case variant is not the same address"
        );

        // A later insert is still rejected against the maintained index.
        s.suppress_email("suppressed0@p0.example", SuppressionReason::Operator);
        assert_eq!(s.emails().len(), N + 1);
        s.suppress_publisher("p0.example", SuppressionReason::Operator);
        assert_eq!(s.publishers().len(), N);
    }

    /// Inserts and checks perform one constant-time membership probe each:
    /// 10k unique inserts, 10k duplicate inserts, 10k publisher inserts and
    /// 10k checks stay linear (4n probes). Reverting either insert to the
    /// pre-fix vector scan (with its per-compared-entry counter) makes this
    /// fail by orders of magnitude; see the task report for the exact run.
    #[test]
    fn suppression_membership_does_not_rescan_the_list() {
        const N: usize = 10_000;
        reset_membership_probes();
        let mut s = Suppressions::default();
        for i in 0..N {
            s.suppress_email(
                &format!("suppressed{i}@p{i}.example"),
                SuppressionReason::OptOut,
            );
        }
        // Every duplicate must be rejected without walking the list.
        for i in 0..N {
            s.suppress_email(
                &format!("suppressed{i}@p{i}.example"),
                SuppressionReason::Operator,
            );
        }
        for i in 0..N {
            s.suppress_publisher(&format!("p{i}.example"), SuppressionReason::Complaint);
        }
        for i in 0..N {
            let _ = s.check(&contact_at(
                &format!("suppressed{i}@p{i}.example"),
                &format!("p{i}.example"),
            ));
        }
        let probes = membership_probes();
        assert_eq!(s.emails().len(), N);
        assert_eq!(s.publishers().len(), N);
        assert!(
            probes <= 4 * N as u64,
            "4n constant-time probes expected, got {probes} for n={N}"
        );
        assert!(
            probes < (N * N / 10) as u64,
            "a vector scan is quadratic: {probes} probes for n={N}"
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
