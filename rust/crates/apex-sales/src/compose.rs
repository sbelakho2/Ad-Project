//! Writing the email: evidence-grounded templates and RFC 5322 messages.
//!
//! # Grounding
//!
//! A template may state facts about the publisher only through `{finding}`,
//! which is filled from one of the publisher's [`Evidence`] statements; the
//! message records which evidence it cites. Everything else in a template is
//! a claim about Apex itself. A template whose placeholders cannot all be
//! filled from real data is refused rather than sent with a gap, and
//! rendered text containing a banned fake-personalization phrase is refused.
//!
//! # Offers
//!
//! There is one self-serve offer: the free read-only audit. Monitoring, the
//! audit and the app-ads.txt watch are free with every connection. There is no
//! paid Setup Watch or monitoring tier and no checkout link in the copy;
//! publishers above $1M/month go to the owner queue instead of a self-serve
//! offer.
//!
//! # Compliance
//!
//! Every message identifies the sender with a postal address, says why the
//! recipient is receiving it, offers a reply-to-unsubscribe path, and carries
//! `List-Unsubscribe` (RFC 2369) with one-click `List-Unsubscribe-Post`
//! (RFC 8058) when an HTTPS unsubscribe URL is configured.

use crate::calendar::rfc5322_date;
use crate::model::{Contact, Evidence, Publisher, UnixSecs};

/// Phrases that read as fake personalization or filler and are never sent.
pub const BANNED_PHRASES: [&str; 9] = [
    "hope this finds you well",
    "i came across",
    "quick question",
    "just following up",
    "circling back",
    "touch base",
    "i noticed you",
    "i hope you're well",
    "as per my last email",
];

/// A message template. `step` 0 is the first touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// Stable id, also the bandit arm id.
    pub id: &'static str,
    /// Sequence step this template serves.
    pub step: usize,
    /// Subject with placeholders.
    pub subject: &'static str,
    /// Body with placeholders.
    pub body: &'static str,
}

/// The default templates: four first-touch angles and three follow-ups.
///
/// Placeholders: `{first_name}` (falls back to "there"), `{app}`,
/// `{domain}`, `{finding}` and `{sender_name}`.
pub const TEMPLATES: [Template; 7] = [
    Template {
        id: "t0-finding",
        step: 0,
        subject: "{app}: a public check of your app-ads.txt",
        body: "Hi {first_name},\n\nA public check of {domain} turned up this: {finding}.\n\nApex finds and fixes revenue leaks like this on AppLovin MAX, LevelPlay and AdMob, with no SDK change. Every change runs as the platform's own A/B test, and we charge only a share of the lift that test proves.\n\nWould a free, read-only audit of {app} be useful?\n\n{sender_name}",
    },
    Template {
        id: "t0-proof",
        step: 0,
        subject: "Revenue lift for {app}, proven by your own A/B tests",
        body: "Hi {first_name},\n\nApex tunes ad mediation for studios like yours on AppLovin MAX, LevelPlay and AdMob. Each change runs as the platform's native A/B test, and the fee is a share of the lift it proves; no lift, no fee.\n\nFrom public data: {finding}.\n\nShall I run the free read-only audit on {app}?\n\n{sender_name}",
    },
    Template {
        id: "t0-short",
        step: 0,
        subject: "{app} ad revenue",
        body: "Hi {first_name},\n\n{finding}.\n\nApex can audit {app}'s mediation setup for free, read-only, and fix what it finds without an SDK change. Interested?\n\n{sender_name}",
    },
    Template {
        id: "t0-watch",
        step: 0,
        subject: "{app}: free monitoring and app-ads.txt watch",
        body: "Hi {first_name},\n\nA public check of {domain} turned up this: {finding}.\n\nMonitoring, the read-only audit and the app-ads.txt watch are free. If you connect and keep a change, Apex bills 30% of the gain the 10% holdout measures each month; a month with no gain bills nothing.\n\nWould the free audit of {app} be useful?\n\n{sender_name}",
    },
    Template {
        id: "t1-bump",
        step: 1,
        subject: "Re: {app}",
        body: "Hi {first_name},\n\nThe free audit for {app} takes a read-only reporting key and about ten minutes of your time. Worth a look?\n\n{sender_name}",
    },
    Template {
        id: "t2-value",
        step: 2,
        subject: "Re: {app}",
        body: "Hi {first_name},\n\nOne more note: Apex is paid only from revenue lift your own platform's A/B test proves, so the audit costs nothing to try.\n\n{sender_name}",
    },
    Template {
        id: "t3-close",
        step: 3,
        subject: "Closing the loop on {app}",
        body: "Hi {first_name},\n\nI won't follow up again. If the audit becomes useful later, just reply to this email.\n\n{sender_name}",
    },
];

/// Templates for a sequence step.
pub fn templates_for_step(step: usize) -> Vec<&'static Template> {
    TEMPLATES.iter().filter(|t| t.step == step).collect()
}

/// Templates for a step that the engine can serve right now.
pub fn available_templates_for_step(step: usize) -> Vec<&'static Template> {
    templates_for_step(step)
}

/// Sender identity and compliance details.
///
/// One identity per sending domain: the engine keys the domain's daily cap
/// and health by [`Sender::domain`], and every message is built with the
/// identity of the domain that sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    /// Display name, e.g. "Aaron at Apex".
    pub name: String,
    /// From address.
    pub email: String,
    /// Legal entity name.
    pub company: String,
    /// Postal address.
    pub postal_address: String,
    /// HTTPS one-click unsubscribe URL base, when configured. The contact's
    /// token is appended.
    pub unsubscribe_url: Option<String>,
}

impl Sender {
    /// The from-domain: the health key and the spread key of this identity.
    pub fn domain(&self) -> &str {
        self.email.rsplit('@').next().unwrap_or("localhost")
    }
}

/// A rendered message, ready for a transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// Subject (decoded).
    pub subject: String,
    /// Plain-text body.
    pub body: String,
    /// Evidence ids the message cites.
    pub cites: Vec<String>,
}

/// Why a template could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    /// `{finding}` needs evidence and the publisher has none.
    NoEvidence,
    /// `{app}` needs an app and the publisher has none.
    NoApp,
    /// `{domain}` needs a domain.
    NoDomain,
    /// A placeholder is not known.
    UnknownPlaceholder(String),
    /// The rendered text contains a banned phrase.
    Banned(&'static str),
}

/// Pick the strongest evidence to cite: app-ads.txt findings first, then the
/// newest.
pub fn best_evidence(publisher: &Publisher) -> Option<&Evidence> {
    publisher.evidence.iter().max_by(|a, b| {
        let rank = |e: &Evidence| u8::from(e.kind == crate::model::EvidenceKind::AppAdsTxt);
        rank(a)
            .cmp(&rank(b))
            .then(a.observed_at.cmp(&b.observed_at))
    })
}

/// Render `template` for `contact` at `publisher`.
pub fn render(
    template: &Template,
    publisher: &Publisher,
    contact: &Contact,
    sender: &Sender,
) -> Result<Rendered, ComposeError> {
    let mut cites = Vec::new();
    let mut fill = |text: &str| -> Result<String, ComposeError> {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let end = after
                .find('}')
                .ok_or_else(|| ComposeError::UnknownPlaceholder(after.to_string()))?;
            let name = &after[..end];
            let value = match name {
                "first_name" => contact
                    .name
                    .as_deref()
                    .and_then(|n| n.split_whitespace().next())
                    .unwrap_or("there")
                    .to_string(),
                "app" => publisher
                    .apps
                    .iter()
                    .max_by_key(|a| a.rating_count.unwrap_or(0))
                    .map(|a| a.name.clone())
                    .ok_or(ComposeError::NoApp)?,
                "domain" => publisher.domain.clone().ok_or(ComposeError::NoDomain)?,
                "finding" => {
                    let e = best_evidence(publisher).ok_or(ComposeError::NoEvidence)?;
                    if !cites.contains(&e.id) {
                        cites.push(e.id.clone());
                    }
                    e.statement.trim_end_matches('.').to_string()
                }
                "sender_name" => sender.name.clone(),
                other => return Err(ComposeError::UnknownPlaceholder(other.to_string())),
            };
            out.push_str(&value);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    };
    let subject = fill(template.subject)?;
    let body = fill(template.body)?;
    let lower = format!("{subject}\n{body}").to_lowercase();
    if let Some(p) = BANNED_PHRASES.iter().find(|p| lower.contains(*p)) {
        return Err(ComposeError::Banned(p));
    }
    Ok(Rendered {
        subject,
        body,
        cites,
    })
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let idx = ((n >> (18 - 6 * i)) & 63) as usize;
                out.push(char::from(B64[idx]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// RFC 2047 encoded-words for a header value that is not plain ASCII. Each
/// word carries at most 45 bytes (60 base64 characters, 72 with framing,
/// under the 75-character limit), split on character boundaries and folded
/// onto continuation lines.
pub fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'@' | b'+') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn encode_header(value: &str) -> String {
    if value.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return value.to_string();
    }
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in value.chars() {
        if chunk.len() + c.len_utf8() > 45 {
            words.push(format!("=?UTF-8?B?{}?=", base64(chunk.as_bytes())));
            chunk.clear();
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(format!("=?UTF-8?B?{}?=", base64(chunk.as_bytes())));
    }
    words.join("\r\n ")
}

/// Wrap text at `width` columns on spaces, keeping blank lines.
pub fn wrap(text: &str, width: usize) -> String {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out.join("\n")
}

/// A complete outgoing message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// Envelope sender.
    pub from: String,
    /// Envelope recipient.
    pub to: String,
    /// Message-ID without angle brackets.
    pub message_id: String,
    /// Full RFC 5322 text with `\r\n` line endings.
    pub raw: String,
}

/// Build the RFC 5322 message. `token` identifies the contact for the
/// unsubscribe link; `reason` says why they are receiving it.
pub fn build_message(
    rendered: &Rendered,
    contact: &Contact,
    sender: &Sender,
    message_id: &str,
    now: UnixSecs,
    token: &str,
) -> Outgoing {
    let domain = sender.email.rsplit('@').next().unwrap_or("localhost");
    let mailto = format!("mailto:{}?subject=unsubscribe", sender.email);
    let https = sender
        .unsubscribe_url
        .as_ref()
        .filter(|u| {
            u.starts_with("https://")
                && !u
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>')
        })
        .map(|u| {
            let separator = if u.contains('?') { '&' } else { '?' };
            format!(
                "{u}{separator}email={}&token={token}",
                percent_encode(&contact.email)
            )
        });
    let mut headers = vec![
        format!("From: {} <{}>", encode_header(&sender.name), sender.email),
        format!("To: <{}>", contact.email),
        format!("Subject: {}", encode_header(&rendered.subject)),
        format!("Date: {}", rfc5322_date(now)),
        format!("Message-ID: <{message_id}@{domain}>"),
        "MIME-Version: 1.0".to_string(),
        "Content-Type: text/plain; charset=utf-8".to_string(),
        "Content-Transfer-Encoding: 8bit".to_string(),
    ];
    match &https {
        Some(url) => {
            headers.push(format!("List-Unsubscribe: <{url}>, <{mailto}>"));
            headers.push("List-Unsubscribe-Post: List-Unsubscribe=One-Click".to_string());
        }
        None => headers.push(format!("List-Unsubscribe: <{mailto}>")),
    }
    let unsubscribe_line = match &https {
        Some(url) => {
            format!("Reply \"unsubscribe\" or open {url} and you will not hear from us again.")
        }
        None => "Reply \"unsubscribe\" and you will not hear from us again.".to_string(),
    };
    // Display fields are operator configuration: flatten control characters
    // so a name or address can never start a line that reads like a header
    // even in the body.
    let flat = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    };
    let footer = format!(
        "--\n{}, {}, {}\nYou are receiving this because {} is listed publicly as a contact for {}'s apps. {}",
        flat(&sender.name),
        flat(&sender.company),
        flat(&sender.postal_address),
        flat(&contact.email),
        flat(&contact.publisher_key),
        unsubscribe_line
    );
    let body = format!("{}\n\n{}", wrap(&rendered.body, 76), wrap(&footer, 76));
    let mut raw = headers.join("\r\n");
    raw.push_str("\r\n\r\n");
    raw.push_str(&body.replace('\n', "\r\n"));
    raw.push_str("\r\n");
    Outgoing {
        from: sender.email.clone(),
        to: contact.email.clone(),
        message_id: format!("{message_id}@{domain}"),
        raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{App, ContactKind, EvidenceKind, Store};

    fn publisher() -> Publisher {
        let mut p = Publisher::new("studio.example", "Studio");
        p.domain = Some("studio.example".into());
        p.apps.push(App {
            store: Store::AppStore,
            id: "1".into(),
            name: "Tile Quest".into(),
            genre: None,
            rating_count: Some(900),
            free: Some(true),
            days_since_update: None,
        });
        p.apps.push(App {
            store: Store::AppStore,
            id: "2".into(),
            name: "Word Ölympics".into(),
            genre: None,
            rating_count: Some(12_000),
            free: Some(true),
            days_since_update: None,
        });
        p.evidence.push(Evidence {
            id: "ev-store".into(),
            kind: EvidenceKind::StoreListing,
            source: "s".into(),
            statement: "Word Ölympics has 12000 ratings".into(),
            observed_at: 10,
        });
        p.evidence.push(Evidence {
            id: "ev-ads".into(),
            kind: EvidenceKind::AppAdsTxt,
            source: "https://studio.example/app-ads.txt".into(),
            statement: "3 lines of https://studio.example/app-ads.txt fail the IAB format.".into(),
            observed_at: 5,
        });
        p
    }

    fn contact() -> Contact {
        Contact {
            email: "maria@studio.example".into(),
            name: Some("Maria Lopez".into()),
            role: None,
            publisher_key: "studio.example".into(),
            kind: ContactKind::Named,
            source: "t".into(),
            consent: false,
        }
    }

    fn sender() -> Sender {
        Sender {
            name: "Aaron at Apex".into(),
            email: "aaron@apexmediation.ee".into(),
            company: "Apex OÜ".into(),
            postal_address: "Tallinn, Estonia".into(),
            unsubscribe_url: Some("https://apexmediation.ee/u/".into()),
        }
    }

    fn template(id: &str) -> &'static Template {
        TEMPLATES
            .iter()
            .find(|t| t.id == id)
            .unwrap_or(&TEMPLATES[0])
    }

    #[test]
    fn renders_grounded_templates_and_cites_evidence() {
        let r = render(&TEMPLATES[0], &publisher(), &contact(), &sender()).unwrap();
        assert_eq!(
            r.subject,
            "Word Ölympics: a public check of your app-ads.txt"
        );
        assert!(r.body.starts_with("Hi Maria,"));
        assert!(r
            .body
            .contains("this: 3 lines of https://studio.example/app-ads.txt fail the IAB format."));
        assert_eq!(r.cites, vec!["ev-ads".to_string()]);
        for t in TEMPLATES.iter() {
            assert!(
                render(t, &publisher(), &contact(), &sender()).is_ok(),
                "{}",
                t.id
            );
        }
        assert_eq!(templates_for_step(0).len(), 4);
        assert_eq!(templates_for_step(9).len(), 0);
        assert_eq!(available_templates_for_step(0).len(), 4);
    }

    #[test]
    fn the_watch_copy_is_free_and_has_no_checkout() {
        let watch = template("t0-watch");
        let r = render(watch, &publisher(), &contact(), &sender()).unwrap();
        assert!(r.body.contains("free"));
        assert!(r.body.contains("30%"));
        assert!(!r.body.contains("checkout page"));
        assert!(!r.body.contains("/watch/?ref="));
        assert_eq!(r.cites, vec!["ev-ads".to_string()]);
        assert_eq!(available_templates_for_step(0).len(), 4);
    }

    #[test]
    fn copy_discipline_keeps_prices_in_one_constant_and_prose_sentence_case() {
        for t in TEMPLATES.iter() {
            assert!(!t.subject.contains('$'), "{}", t.id);
            assert!(!t.body.contains('$'), "{}", t.id);
            for sentence in t.body.split(['.', '!', '?']).map(str::trim) {
                let mut chars = sentence.chars();
                let Some(first) = chars.next() else { continue };
                let lowercases = sentence.chars().filter(|c| c.is_lowercase()).count();
                assert!(
                    !first.is_uppercase() || lowercases > 0 || sentence.chars().count() < 2,
                    "{}: all-caps prose: {sentence}",
                    t.id
                );
            }
        }
        assert!(!template("t0-watch").body.contains("{watch_price}"));
        assert!(!template("t0-watch").body.contains("{watch_link}"));
    }

    #[test]
    fn refuses_gaps_and_banned_phrases() {
        let mut bare = publisher();
        bare.evidence.clear();
        assert_eq!(
            render(&TEMPLATES[0], &bare, &contact(), &sender()),
            Err(ComposeError::NoEvidence)
        );
        bare.apps.clear();
        assert_eq!(
            render(template("t1-bump"), &bare, &contact(), &sender()),
            Err(ComposeError::NoApp)
        );
        let mut no_domain = publisher();
        no_domain.domain = None;
        assert_eq!(
            render(&TEMPLATES[0], &no_domain, &contact(), &sender()),
            Err(ComposeError::NoDomain)
        );
        assert_eq!(
            render(template("t0-watch"), &no_domain, &contact(), &sender(),),
            Err(ComposeError::NoDomain)
        );
        let bad = Template {
            id: "x",
            step: 0,
            subject: "{nope}",
            body: "",
        };
        assert!(matches!(
            render(&bad, &publisher(), &contact(), &sender()),
            Err(ComposeError::UnknownPlaceholder(_))
        ));
        let unclosed = Template {
            id: "x",
            step: 0,
            subject: "{app",
            body: "",
        };
        assert!(render(&unclosed, &publisher(), &contact(), &sender()).is_err());
        let filler = Template {
            id: "x",
            step: 0,
            subject: "Quick question about {app}",
            body: "",
        };
        assert_eq!(
            render(&filler, &publisher(), &contact(), &sender()),
            Err(ComposeError::Banned("quick question"))
        );
        let mut anon = contact();
        anon.name = None;
        let r = render(template("t1-bump"), &publisher(), &anon, &sender()).unwrap();
        assert!(r.body.starts_with("Hi there,"));
    }

    #[test]
    fn builds_a_compliant_rfc5322_message() {
        let r = render(&TEMPLATES[0], &publisher(), &contact(), &sender()).unwrap();
        let m = build_message(&r, &contact(), &sender(), "t-abc", 0, "tok123");
        assert!(m.raw.contains("\r\nSubject: =?UTF-8?B?"));
        assert!(m
            .raw
            .contains("From: Aaron at Apex <aaron@apexmediation.ee>\r\n"));
        assert!(m.raw.contains("Message-ID: <t-abc@apexmediation.ee>\r\n"));
        assert!(m.raw.contains("List-Unsubscribe: <https://apexmediation.ee/u/?email=maria@studio.example&token=tok123>, <mailto:aaron@apexmediation.ee?subject=unsubscribe>\r\n"));
        assert!(m
            .raw
            .contains("List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n"));
        assert!(m.raw.contains("Tallinn, Estonia"));
        assert!(!m.raw.replace("\r\n", "").contains('\n'), "CRLF only");
        assert!(m.raw.split("\r\n").all(|l| l.chars().count() <= 200));
        assert_eq!(m.message_id, "t-abc@apexmediation.ee");

        // A hostile unsubscribe base cannot add a header line; the message
        // falls back to the mailto path.
        let mut hostile = sender();
        hostile.unsubscribe_url =
            Some("https://apexmediation.ee/u/\r\nBcc: victim@else.example".into());
        let m = build_message(&r, &contact(), &hostile, "t-abc", 0, "tok");
        assert!(!m.raw.contains("Bcc:"), "no injected header");
        assert!(m.raw.contains("List-Unsubscribe: <mailto:"));

        // A display name or subject with CR/LF is RFC 2047 encoded, never a
        // header break.
        let mut crlf_sender = sender();
        crlf_sender.name = "Aaron\r\nBcc: victim@else.example".into();
        let m = build_message(&r, &contact(), &crlf_sender, "t-abc", 0, "tok");
        assert!(!m.raw.contains("\r\nBcc:"), "no injected header");
        let mut injected = r.clone();
        injected.subject = "Hi\r\nBcc: victim@else.example".into();
        let m = build_message(&injected, &contact(), &sender(), "t-abc", 0, "tok");
        assert!(!m.raw.contains("\r\nBcc:"), "no injected header");

        let mut plain = sender();
        plain.unsubscribe_url = None;
        let m = build_message(&r, &contact(), &plain, "t-abc", 0, "tok");
        assert!(m.raw.contains("List-Unsubscribe: <mailto:"));
        assert!(!m.raw.contains("One-Click"));
        assert_eq!(sender().domain(), "apexmediation.ee");
    }

    #[test]
    fn base64_and_wrapping() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(encode_header("plain"), "plain");
        let long = encode_header(&"é".repeat(40));
        assert_eq!(long.matches("=?UTF-8?B?").count(), 2);
        assert!(long.split("\r\n ").all(|w| w.len() <= 75));
        assert_eq!(wrap("aaa bbb ccc", 7), "aaa bbb\nccc");
        assert_eq!(wrap("a\n\nb", 10), "a\n\nb");
    }
}
