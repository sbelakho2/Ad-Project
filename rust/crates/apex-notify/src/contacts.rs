//! Recipients: typed, validated contact addresses.
//!
//! Page 94 (delivery contract): "Send only to authorized account contacts
//! through a configured notification sink." A [`Contact`] is the only
//! recipient type the notifier accepts; it can only be built from a validated
//! email address, so a publisher id, a URL or a webhook name can never be
//! mistaken for a recipient.

use std::collections::BTreeMap;

/// Why an address was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContactError {
    /// The contact cannot exist.
    #[error("'{value}' is not a single email address")]
    NotAnEmail {
        /// The refused value.
        value: String,
    },
    /// The JSON contact map could not be parsed.
    #[error("APEX_NOTIFY_CONTACTS is not a JSON map of publisher to email list: {reason}")]
    BadMap {
        /// The parse failure.
        reason: String,
    },
}

/// One authorized recipient. The address is validated at construction; every
/// other string (a publisher id, a URL, a webhook name) is refused.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Contact {
    email: String,
}

impl Contact {
    /// Validate and wrap `email`.
    pub fn new(email: &str) -> Result<Self, ContactError> {
        if is_email(email) {
            Ok(Self {
                email: email.trim().to_string(),
            })
        } else {
            Err(ContactError::NotAnEmail {
                value: email.to_string(),
            })
        }
    }

    /// The address.
    pub fn email(&self) -> &str {
        &self.email
    }
}

impl std::fmt::Display for Contact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.email)
    }
}

/// A conservative single-address check: one `@`, a non-empty local part, a
/// dotted domain of letters/digits/hyphens, no whitespace or control bytes.
fn is_email(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.len() > 254 {
        return false;
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    if value.matches('@').count() != 1 {
        return false;
    }
    let (local, domain) = match value.split_once('@') {
        Some(pair) => pair,
        None => return false,
    };
    if local.is_empty() || local.len() > 64 || local.starts_with('.') || local.ends_with('.') {
        return false;
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return false;
    }
    let local_ok = local
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'));
    let domain_ok = domain.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    local_ok && domain_ok
}

/// The publisher to contacts map from `APEX_NOTIFY_CONTACTS`
/// (`{"northstar": ["ops@example.com"]}`).
///
/// The map is the only recipient source; no other code path can name a
/// recipient. An absent variable disables notifications entirely (the caller
/// logs that state; this type never invents a contact).
#[derive(Debug, Clone, Default)]
pub struct ContactBook {
    by_publisher: BTreeMap<String, Vec<Contact>>,
}

impl ContactBook {
    /// Parse the JSON map value.
    pub fn from_json(raw: &str) -> Result<Self, ContactError> {
        let parsed: BTreeMap<String, Vec<String>> =
            serde_json::from_str(raw).map_err(|error| ContactError::BadMap {
                reason: error.to_string(),
            })?;
        let mut by_publisher = BTreeMap::new();
        for (publisher, emails) in parsed {
            let mut contacts = Vec::with_capacity(emails.len());
            for email in emails {
                contacts.push(Contact::new(&email)?);
            }
            by_publisher.insert(publisher, contacts);
        }
        Ok(Self { by_publisher })
    }

    /// The contacts authorized for `publisher`, in configured order.
    pub fn for_publisher(&self, publisher: &str) -> &[Contact] {
        self.by_publisher
            .get(publisher)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Whether the book holds any contacts at all.
    pub fn is_empty(&self) -> bool {
        self.by_publisher.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn valid_addresses_are_accepted() {
        let contact = Contact::new("ops@example.com").expect("valid");
        assert_eq!(contact.email(), "ops@example.com");
        assert_eq!(contact.to_string(), "ops@example.com");
        assert!(Contact::new("first.last+billing@mail.example.co.uk").is_ok());
        assert!(Contact::new("  ops@example.com  ").is_ok());
    }

    #[test]
    fn ids_urls_and_non_addresses_are_refused() {
        for bad in [
            "northstar",
            "exp_0242",
            "https://example.com/notify",
            "ops@example",
            "@example.com",
            "ops@",
            "two words@example.com",
            "a@b@c",
            "",
        ] {
            let error = Contact::new(bad).expect_err(bad);
            assert!(matches!(error, ContactError::NotAnEmail { .. }), "{bad}");
        }
    }

    #[test]
    fn the_contact_book_parses_the_env_shape() {
        let book = ContactBook::from_json(
            r#"{"northstar": ["ops@example.com", "fin@example.com"], "acme": []}"#,
        )
        .expect("map");
        assert_eq!(book.for_publisher("northstar").len(), 2);
        assert_eq!(
            book.for_publisher("northstar")[0].email(),
            "ops@example.com"
        );
        assert!(book.for_publisher("acme").is_empty());
        assert!(book.for_publisher("unknown").is_empty());
        assert!(!book.is_empty());
        assert!(ContactBook::from_json("not json").is_err());
        assert!(ContactBook::from_json(r#"{"northstar": ["not an email"]}"#).is_err());
        assert!(ContactBook::default().is_empty());
    }
}
