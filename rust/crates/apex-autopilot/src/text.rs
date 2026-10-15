//! Small text helpers shared by the runner modules.
//!
//! Provider-, file- and user-derived text is bounded before it reaches a log
//! line, an event payload or a state file. `String::truncate` panics when the
//! byte index lands inside a multibyte character, so every bound here counts
//! characters, never bytes.

/// Bound `text` to at most `max_chars` characters. A string already within
/// the bound is returned unchanged; a longer one is cut on a character
/// boundary, so a multibyte character at the bound is never split (and never
/// panics).
pub fn bounded_chars(text: &str, max_chars: usize) -> String {
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
        // 500 two-byte characters: a byte-based truncation at 200 would
        // panic or split a character; the character bound does neither.
        let text = "é".repeat(500);
        let bounded = bounded_chars(&text, 200);
        assert_eq!(bounded.chars().count(), 200);
        assert_eq!(bounded, "é".repeat(200));

        // A multibyte character straddling the byte bound stays whole.
        let text = format!("{}€{}", "a".repeat(199), "b".repeat(50));
        // Byte 200 is inside the euro sign; the character bound keeps it.
        let bounded = bounded_chars(&text, 200);
        assert_eq!(bounded.chars().count(), 200);
        assert!(bounded.ends_with('€'));

        // Short input is returned unchanged.
        assert_eq!(bounded_chars("short", 200), "short");
        assert_eq!(bounded_chars("", 0), "");
    }
}
