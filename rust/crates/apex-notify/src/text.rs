//! Small text helpers for provider-supplied strings.
//!
//! `String::truncate` panics when the byte index lands inside a multibyte
//! character, so every bound in this crate counts characters, never bytes.

/// Bound `text` to at most `max_chars` characters on a character boundary.
pub(crate) fn bounded_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn bounds_count_characters_not_bytes() {
        let text = "é".repeat(500);
        let bounded = bounded_chars(&text, 200);
        assert_eq!(bounded.chars().count(), 200);
        assert_eq!(bounded, "é".repeat(200));
        // Byte 200 is inside a multibyte character here.
        let text = format!("{}€", "a".repeat(199));
        assert_eq!(bounded_chars(&text, 200), text);
        assert_eq!(bounded_chars("short", 200), "short");
    }
}
