//! The engine's domain: publishers, their apps, contacts, evidence and the
//! outcome ladder every email can climb.
//!
//! A *publisher* is the account being sold to (a studio, keyed by its
//! normalized web domain). A *contact* is one mailbox at that publisher.
//! *Evidence* is a fact the engine observed itself, with its source, and is
//! the only material an email may assert (see [`crate::compose`]).

/// Seconds since the Unix epoch, UTC.
pub type UnixSecs = i64;

/// Seconds in a day.
pub const DAY: i64 = 86_400;

/// Which store an app is listed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Store {
    /// Apple App Store.
    AppStore,
    /// Google Play.
    GooglePlay,
}

impl Store {
    /// Stable string form.
    pub fn as_str(self) -> &'static str {
        match self {
            Store::AppStore => "app_store",
            Store::GooglePlay => "google_play",
        }
    }

    /// Parse the stable string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "app_store" => Some(Store::AppStore),
            "google_play" => Some(Store::GooglePlay),
            _ => None,
        }
    }
}

/// One app a publisher ships.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    /// Store the listing is in.
    pub store: Store,
    /// Store id (numeric App Store id or Play package name).
    pub id: String,
    /// Listing name.
    pub name: String,
    /// Primary genre as the store names it.
    pub genre: Option<String>,
    /// Ratings count, the public proxy for install base.
    pub rating_count: Option<u64>,
    /// Free to download.
    pub free: Option<bool>,
    /// Days since the latest version, at observation time.
    pub days_since_update: Option<u32>,
}

/// Where a piece of evidence came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvidenceKind {
    /// The publisher's own `app-ads.txt`.
    AppAdsTxt,
    /// A public store listing.
    StoreListing,
    /// What the publisher told us through the free-audit form.
    InboundForm,
    /// Entered by the operator.
    Operator,
}

impl EvidenceKind {
    /// Stable string form.
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceKind::AppAdsTxt => "app_ads_txt",
            EvidenceKind::StoreListing => "store_listing",
            EvidenceKind::InboundForm => "inbound_form",
            EvidenceKind::Operator => "operator",
        }
    }

    /// Parse the stable string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "app_ads_txt" => Some(EvidenceKind::AppAdsTxt),
            "store_listing" => Some(EvidenceKind::StoreListing),
            "inbound_form" => Some(EvidenceKind::InboundForm),
            "operator" => Some(EvidenceKind::Operator),
            _ => None,
        }
    }
}

/// A fact the engine observed, with provenance. Emails may only assert
/// statements that are evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence {
    /// Stable id, unique within the publisher.
    pub id: String,
    /// Source kind.
    pub kind: EvidenceKind,
    /// URL or file the fact was read from.
    pub source: String,
    /// The fact, phrased so it can be quoted to the publisher.
    pub statement: String,
    /// When it was observed.
    pub observed_at: UnixSecs,
}

/// The account being sold to.
#[derive(Debug, Clone, PartialEq)]
pub struct Publisher {
    /// Normalized key: the registrable web domain, or a slug for inbound
    /// leads without one.
    pub key: String,
    /// Display name.
    pub name: String,
    /// Web domain, when known (`studio.example`).
    pub domain: Option<String>,
    /// ISO 3166-1 alpha-2 country, uppercase, when known.
    pub country: Option<String>,
    /// Apps observed.
    pub apps: Vec<App>,
    /// Raw `app-ads.txt` body, when it was fetched (parsed on demand).
    pub ads_txt_text: Option<String>,
    /// Whether `app-ads.txt` was looked for and not found.
    pub ads_txt_missing: bool,
    /// Observed facts.
    pub evidence: Vec<Evidence>,
    /// Came in through the free-audit form.
    pub inbound: bool,
    /// Mediation platforms the publisher declared (inbound form).
    pub declared_platforms: Vec<String>,
}

impl Publisher {
    /// A bare publisher with only a key and a name.
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            domain: None,
            country: None,
            apps: Vec::new(),
            ads_txt_text: None,
            ads_txt_missing: false,
            evidence: Vec::new(),
            inbound: false,
            declared_platforms: Vec::new(),
        }
    }

    /// The largest ratings count across the publisher's apps.
    pub fn max_rating_count(&self) -> Option<u64> {
        self.apps.iter().filter_map(|a| a.rating_count).max()
    }

    /// The parsed `app-ads.txt`, when one was fetched.
    pub fn ads_txt(&self) -> Option<crate::signals::AdsTxtReport> {
        self.ads_txt_text
            .as_deref()
            .map(crate::signals::parse_app_ads_txt)
    }

    /// Evidence by id.
    pub fn evidence(&self, id: &str) -> Option<&Evidence> {
        self.evidence.iter().find(|e| e.id == id)
    }
}

/// What kind of mailbox a contact is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContactKind {
    /// A named person (`maria@studio.example`).
    Named,
    /// A function mailbox relevant to the offer (`bizdev@`, `ads@`).
    Role,
    /// A catch-all (`info@`, `support@`, `contact@`).
    Generic,
}

impl ContactKind {
    /// Classify a mailbox by its local part.
    pub fn of_email(email: &str) -> Self {
        let local = email
            .split('@')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        const GENERIC: [&str; 10] = [
            "info", "support", "contact", "hello", "help", "admin", "office", "mail", "team",
            "feedback",
        ];
        const ROLE: [&str; 9] = [
            "bizdev",
            "business",
            "ads",
            "monetization",
            "monetisation",
            "partners",
            "partnerships",
            "publishing",
            "growth",
        ];
        if GENERIC.contains(&local.as_str()) || local.starts_with("noreply") {
            ContactKind::Generic
        } else if ROLE.contains(&local.as_str()) {
            ContactKind::Role
        } else {
            ContactKind::Named
        }
    }

    /// Stable string form.
    pub fn as_str(self) -> &'static str {
        match self {
            ContactKind::Named => "named",
            ContactKind::Role => "role",
            ContactKind::Generic => "generic",
        }
    }
}

/// One mailbox at a publisher.
#[derive(Debug, Clone, PartialEq)]
pub struct Contact {
    /// Normalized address.
    pub email: String,
    /// Person's name, when known.
    pub name: Option<String>,
    /// Job title, when known.
    pub role: Option<String>,
    /// Owning publisher key.
    pub publisher_key: String,
    /// Mailbox kind.
    pub kind: ContactKind,
    /// Where the address came from (URL, file or `inbound`).
    pub source: String,
    /// The contact gave explicit consent to be emailed (inbound form).
    pub consent: bool,
}

/// How a reply was classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReplyClass {
    /// Interested: wants the audit, a call or details.
    Positive,
    /// Points to someone else at the publisher.
    Referral,
    /// Asks a question without committing.
    Question,
    /// Not now; may be interested later.
    NotNow,
    /// Not interested.
    NotInterested,
    /// Asks to be removed.
    OptOut,
    /// The classifier is not confident; a person must read it.
    Unclear,
}

impl ReplyClass {
    /// Stable string form.
    pub fn as_str(self) -> &'static str {
        match self {
            ReplyClass::Positive => "positive",
            ReplyClass::Referral => "referral",
            ReplyClass::Question => "question",
            ReplyClass::NotNow => "not_now",
            ReplyClass::NotInterested => "not_interested",
            ReplyClass::OptOut => "opt_out",
            ReplyClass::Unclear => "unclear",
        }
    }

    /// Parse the stable string form.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "positive" => ReplyClass::Positive,
            "referral" => ReplyClass::Referral,
            "question" => ReplyClass::Question,
            "not_now" => ReplyClass::NotNow,
            "not_interested" => ReplyClass::NotInterested,
            "opt_out" => ReplyClass::OptOut,
            "unclear" => ReplyClass::Unclear,
            _ => return None,
        })
    }

    /// Whether this reply counts as interest for the funnel.
    pub fn is_interest(self) -> bool {
        matches!(
            self,
            ReplyClass::Positive | ReplyClass::Referral | ReplyClass::Question
        )
    }
}

/// Everything that can happen after a send, in funnel order where that
/// applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Handed to the mail transport.
    Sent,
    /// Permanent delivery failure.
    HardBounce,
    /// Temporary delivery failure.
    SoftBounce,
    /// Marked as spam.
    Complaint,
    /// Used the unsubscribe link or header.
    Unsubscribe,
    /// Out-of-office or other automatic reply.
    AutoReply,
    /// A human reply, classified.
    Reply(ReplyClass),
    /// Requested the free audit.
    AuditRequested,
    /// Connected a platform to the autopilot.
    Connected,
    /// First paid autopilot invoice.
    Paid,
}

impl Outcome {
    /// Stable string form (`reply:<class>` for replies).
    pub fn as_string(self) -> String {
        match self {
            Outcome::Sent => "sent".into(),
            Outcome::HardBounce => "hard_bounce".into(),
            Outcome::SoftBounce => "soft_bounce".into(),
            Outcome::Complaint => "complaint".into(),
            Outcome::Unsubscribe => "unsubscribe".into(),
            Outcome::AutoReply => "auto_reply".into(),
            Outcome::Reply(c) => format!("reply:{}", c.as_str()),
            Outcome::AuditRequested => "audit_requested".into(),
            Outcome::Connected => "connected".into(),
            Outcome::Paid => "paid".into(),
        }
    }

    /// Parse the stable string form.
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(class) = s.strip_prefix("reply:") {
            return ReplyClass::parse(class).map(Outcome::Reply);
        }
        Some(match s {
            "sent" => Outcome::Sent,
            "hard_bounce" => Outcome::HardBounce,
            "soft_bounce" => Outcome::SoftBounce,
            "complaint" => Outcome::Complaint,
            "unsubscribe" => Outcome::Unsubscribe,
            "auto_reply" => Outcome::AutoReply,
            "audit_requested" => Outcome::AuditRequested,
            "connected" => Outcome::Connected,
            "paid" => Outcome::Paid,
            _ => return None,
        })
    }
}

/// Normalize an email address: trim, lowercase; for Gmail addresses also
/// drop dots and `+tags` in the local part, which Gmail ignores. Returns
/// `None` for anything that is not `local@domain.tld` without spaces.
pub fn normalize_email(raw: &str) -> Option<String> {
    let e = raw
        .trim()
        .trim_matches(|c| c == '<' || c == '>')
        .to_ascii_lowercase();
    if e.chars().any(|c| c.is_whitespace() || c.is_control()) || e.matches('@').count() != 1 {
        return None;
    }
    let (local, domain) = e.split_once('@')?;
    if local.is_empty() || !is_domain(domain) {
        return None;
    }
    if domain == "gmail.com" || domain == "googlemail.com" {
        let base = local.split('+').next().unwrap_or_default().replace('.', "");
        if base.is_empty() {
            return None;
        }
        return Some(format!("{base}@gmail.com"));
    }
    Some(e)
}

/// Whether `s` looks like a hostname with a dot: labels of `[a-z0-9-]`, not
/// starting or ending in `-`, a final label of letters.
pub fn is_domain(s: &str) -> bool {
    let s = s.trim_end_matches('.');
    if s.len() > 253 || !s.contains('.') {
        return false;
    }
    let labels: Vec<&str> = s.split('.').collect();
    let tld_ok = labels
        .last()
        .is_some_and(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()));
    tld_ok
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// Normalize a web domain or URL to a registrable-looking key: lowercase,
/// no scheme, path, port or leading `www.`.
pub fn normalize_domain(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase();
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(&s);
    let host = s.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    let host = host
        .strip_prefix("www.")
        .unwrap_or(host)
        .trim_end_matches('.');
    if is_domain(host) {
        Some(host.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_normalize_and_reject_garbage() {
        assert_eq!(
            normalize_email(" <Maria@Studio.Example> "),
            Some("maria@studio.example".into())
        );
        assert_eq!(
            normalize_email("j.doe+sales@GoogleMail.com"),
            Some("jdoe@gmail.com".into())
        );
        for bad in [
            "",
            "no-at",
            "a@b",
            "a b@c.de",
            "a@@c.de",
            "@c.de",
            "+x@gmail.com",
        ] {
            assert_eq!(normalize_email(bad), None, "{bad}");
        }
    }

    #[test]
    fn domains_normalize_from_urls() {
        assert_eq!(
            normalize_domain("https://www.Studio.Example:443/games?x=1"),
            Some("studio.example".into())
        );
        assert_eq!(
            normalize_domain("studio.example."),
            Some("studio.example".into())
        );
        assert_eq!(
            normalize_domain("http://user@evil.example/"),
            Some("evil.example".into())
        );
        for bad in [
            "localhost",
            "-a.com",
            "a..com",
            "1.2.3.4",
            "exa mple.com",
            "",
        ] {
            assert_eq!(normalize_domain(bad), None, "{bad}");
        }
    }

    #[test]
    fn contact_kinds_follow_the_local_part() {
        assert_eq!(ContactKind::of_email("info@x.io"), ContactKind::Generic);
        assert_eq!(
            ContactKind::of_email("noreply-ads@x.io"),
            ContactKind::Generic
        );
        assert_eq!(ContactKind::of_email("bizdev@x.io"), ContactKind::Role);
        assert_eq!(ContactKind::of_email("maria@x.io"), ContactKind::Named);
        assert_eq!(ContactKind::Role.as_str(), "role");
    }

    #[test]
    fn stable_strings_round_trip() {
        for o in [
            Outcome::Sent,
            Outcome::HardBounce,
            Outcome::SoftBounce,
            Outcome::Complaint,
            Outcome::Unsubscribe,
            Outcome::AutoReply,
            Outcome::Reply(ReplyClass::Positive),
            Outcome::Reply(ReplyClass::Referral),
            Outcome::Reply(ReplyClass::Question),
            Outcome::Reply(ReplyClass::NotNow),
            Outcome::Reply(ReplyClass::NotInterested),
            Outcome::Reply(ReplyClass::OptOut),
            Outcome::Reply(ReplyClass::Unclear),
            Outcome::AuditRequested,
            Outcome::Connected,
            Outcome::Paid,
        ] {
            assert_eq!(Outcome::parse(&o.as_string()), Some(o));
        }
        assert_eq!(Outcome::parse("reply:nope"), None);
        assert_eq!(Outcome::parse("nope"), None);
        for s in [Store::AppStore, Store::GooglePlay] {
            assert_eq!(Store::parse(s.as_str()), Some(s));
        }
        for k in [
            EvidenceKind::AppAdsTxt,
            EvidenceKind::StoreListing,
            EvidenceKind::InboundForm,
            EvidenceKind::Operator,
        ] {
            assert_eq!(EvidenceKind::parse(k.as_str()), Some(k));
        }
        assert!(ReplyClass::Question.is_interest() && !ReplyClass::NotNow.is_interest());
        assert_eq!(Store::parse("x"), None);
        assert_eq!(EvidenceKind::parse("x"), None);
    }

    #[test]
    fn publisher_helpers() {
        let mut p = Publisher::new("studio.example", "Studio");
        assert_eq!(p.max_rating_count(), None);
        p.apps.push(App {
            store: Store::AppStore,
            id: "1".into(),
            name: "A".into(),
            genre: None,
            rating_count: Some(10),
            free: Some(true),
            days_since_update: None,
        });
        p.apps.push(App {
            store: Store::GooglePlay,
            id: "b".into(),
            name: "B".into(),
            genre: None,
            rating_count: Some(500),
            free: None,
            days_since_update: None,
        });
        assert_eq!(p.max_rating_count(), Some(500));
        p.evidence.push(Evidence {
            id: "e1".into(),
            kind: EvidenceKind::Operator,
            source: "x".into(),
            statement: "s".into(),
            observed_at: 0,
        });
        assert!(p.evidence("e1").is_some() && p.evidence("e2").is_none());
    }
}
