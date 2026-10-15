//! Restricted-value redaction before events and journals.
//!
//! The design document forbids tokens, secrets and unrestricted query bodies
//! in the analytics store (page 71, Audit trail) and the telemetry chapter
//! keeps actor and target identities in a restricted journal instead of
//! metrics. Everything this crate emits through [`crate::events`] is passed
//! through [`redact_text`] first, so a pasted credential can never reach the
//! event store.
//!
//! Redaction is conservative and visible: the replacement keeps the field
//! name and marks the value as withheld, so a reviewer sees that something
//! was removed rather than a silently altered sentence.

/// Field-name prefixes whose values are withheld: `token`, `secret`,
/// `password`, `authorization`, `api key`, `csrf`, `bearer`, `cookie`.
const SENSITIVE_KEYS: [&str; 9] = [
    "token",
    "secret",
    "password",
    "authorization",
    "auth",
    "api_key",
    "api key",
    "csrf",
    "cookie",
];

/// Long opaque strings without spaces (hex/base64-shaped runs of 32 or more
/// characters) are withheld even when no key labels them.
fn looks_like_credential(token: &str) -> bool {
    let is_opaque = token.len() >= 32
        && token.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+' | '/' | '=')
        })
        && token.chars().any(|ch| ch.is_ascii_digit())
        && token.chars().any(|ch| ch.is_ascii_alphabetic());
    is_opaque
}

/// Redact restricted values from `text`.
///
/// Two rules apply:
/// 1. `key: value` or `key = value` pairs whose key contains a sensitive word
///    have everything after the separator on that line replaced with
///    `[withheld]` (over-redacting a pasted credential is the safe
///    direction).
/// 2. Any standalone opaque token of 32+ mixed characters is replaced with
///    `[withheld-token]`.
pub fn redact_text(text: &str) -> String {
    redact_opaque(&redact_labelled(text))
}

/// Replace the value after every sensitive `key:` or `key =` label.
fn redact_labelled(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(separator) = find_label_separator(rest) {
        let after_separator = &rest[separator + 1..];
        let leading_space = after_separator.len() - after_separator.trim_start().len();
        let value_start = separator + 1 + leading_space;
        let after_value = &rest[value_start..];
        if after_value.trim().is_empty() {
            break;
        }
        let value_end = after_value
            .find('\n')
            .map(|index| value_start + index)
            .unwrap_or(rest.len());
        out.push_str(&rest[..value_start]);
        out.push_str("[withheld]");
        rest = &rest[value_end..];
    }
    out.push_str(rest);
    out
}

/// The index of the first `:` or `=` that follows a sensitive key at a word
/// boundary, when a value follows it.
fn find_label_separator(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let mut best: Option<usize> = None;
    for key in SENSITIVE_KEYS {
        let mut search_from = 0;
        while let Some(relative) = lower[search_from..].find(key) {
            let position = search_from + relative;
            let boundary = position == 0
                || !text
                    .as_bytes()
                    .get(position - 1)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric());
            if boundary {
                let after_key = position + key.len();
                let tail = &text[after_key..];
                let separator_offset = tail
                    .find([':', '='])
                    .filter(|index| tail[..*index].trim().is_empty());
                if let Some(separator_offset) = separator_offset {
                    let separator = after_key + separator_offset;
                    if !text[separator + 1..].trim().is_empty() {
                        best = Some(match best {
                            Some(existing) => existing.min(separator),
                            None => separator,
                        });
                    }
                }
            }
            search_from = position + key.len().max(1);
        }
    }
    best
}

fn redact_opaque(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for (index, part) in segment.split_inclusive([' ', '\n']).enumerate() {
        let _ = index;
        let word = part.trim_end_matches([' ', '\n']);
        if looks_like_credential(word) {
            out.push_str("[withheld-token]");
        } else {
            out.push_str(word);
        }
        if part.ends_with('\n') {
            out.push('\n');
        } else if part.ends_with(' ') {
            out.push(' ');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labelled_sensitive_pairs_are_withheld() {
        assert_eq!(
            redact_text("token: abc123def456ghi789").trim(),
            "token: [withheld]"
        );
        assert_eq!(
            redact_text("Authorization = Bearer xyz").trim(),
            "Authorization = [withheld]"
        );
        let both = redact_text("csrf_token=deadbeef password=hunter2");
        assert!(both.contains("[withheld]"));
        assert!(!both.contains("deadbeef"));
        assert!(!both.contains("hunter2"));
    }

    #[test]
    fn standalone_opaque_tokens_are_withheld() {
        let hex = "0123456789abcdef0123456789abcdef";
        assert_eq!(redact_text(hex).trim(), "[withheld-token]");
        let base64 = "QWxhZGRpbjpvcGVuIHNlc2FtZQabcdefgh";
        assert_eq!(redact_text(base64).trim(), "[withheld-token]");
    }

    #[test]
    fn ordinary_prose_is_untouched() {
        for text in [
            "Stop request refused for exp_0247",
            "Owner Maya assigned; next update 09:20",
            "The platform did not confirm the stop request.",
            "Reporting is behind the expected schedule.",
            "invoice INV-104 hold approved by finance",
        ] {
            assert_eq!(redact_text(text), text, "{text}");
        }
    }

    #[test]
    fn words_containing_key_substrings_are_not_mistaken_for_keys() {
        // "authored", "consistent", "tokens" (plural, no separator) stay.
        assert_eq!(redact_text("authored by maya"), "authored by maya");
        assert_eq!(
            redact_text("consistent with the policy"),
            "consistent with the policy"
        );
    }

    #[test]
    fn redaction_is_idempotent_over_its_own_output() {
        let input = "Authorization = Bearer abcdef0123456789abcdef0123456789";
        let once = redact_text(input);
        assert!(once.contains("[withheld]"));
        assert_eq!(redact_text(&once), once);
    }
}
