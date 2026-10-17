//! HTML escaping: one function for element text and quoted attributes.
//!
//! [`html_escape`] maps the five HTML-significant characters (`& < > " '`)
//! so its output is literal text in both contexts; [`attr`] documents intent
//! at attribute call sites and is the same mapping. `&` is replaced first (as
//! a single pass over characters), so an existing entity like `&lt;` is
//! escaped to `&amp;lt;` and is never revived into markup.

/// Escape `& < > " '` for use in HTML element text or quoted attributes.
///
/// Input without any of the five characters is copied verbatim.
pub fn html_escape(input: &str) -> String {
    if !needs_escape(input) {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len() + 16);
    push_escaped_known_special(&mut out, input);
    out
}

/// Append `input` escaped for element text or a quoted attribute. This is the
/// allocation-free form of [`html_escape`]: repeated renders (large tables,
/// chart tables) append into one output buffer instead of building one
/// temporary `String` per value.
pub(crate) fn push_escaped(out: &mut String, input: &str) {
    if needs_escape(input) {
        push_escaped_known_special(out, input);
    } else {
        out.push_str(input);
    }
}

fn needs_escape(input: &str) -> bool {
    input
        .bytes()
        .any(|byte| matches!(byte, b'&' | b'<' | b'>' | b'"' | b'\''))
}

fn push_escaped_known_special(out: &mut String, input: &str) {
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
}

/// Escape a value for a quoted attribute context.
///
/// Same mapping as [`html_escape`]: with `"` mapped to `&quot;`, a payload
/// like `" onmouseover="alert(1)` cannot terminate the attribute.
pub fn attr(input: &str) -> String {
    html_escape(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_maps_all_five_characters() {
        assert_eq!(html_escape("&"), "&amp;");
        assert_eq!(html_escape("<"), "&lt;");
        assert_eq!(html_escape(">"), "&gt;");
        assert_eq!(html_escape("\""), "&quot;");
        assert_eq!(html_escape("'"), "&#39;");
        assert_eq!(
            html_escape("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&#39;f"
        );
    }

    #[test]
    fn attribute_payload_stays_inside_the_attribute() {
        let hostile = "\" onmouseover=\"alert(1)";
        assert_eq!(attr(hostile), "&quot; onmouseover=&quot;alert(1)");
        let attribute = format!("<a title=\"{}\">x</a>", attr(hostile));
        assert_eq!(
            attribute,
            "<a title=\"&quot; onmouseover=&quot;alert(1)\">x</a>"
        );
        assert!(!attribute.contains("onmouseover=\"alert"));
    }

    #[test]
    fn ampersand_is_replaced_first_and_entities_are_not_revived() {
        assert_eq!(html_escape("&lt;script&gt;"), "&amp;lt;script&amp;gt;");
        assert_eq!(
            html_escape("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
    }

    #[test]
    fn the_in_place_escaping_path_matches_the_allocating_one() {
        for input in [
            "",
            "plain text",
            "&",
            "<script>alert('x')</script>",
            "caf\u{00e9} \u{4e16}\u{754c} & \"quoted\"",
        ] {
            let mut out = String::from("prefix:");
            push_escaped(&mut out, input);
            assert_eq!(out, format!("prefix:{}", html_escape(input)), "{input:?}");
        }
    }

    #[test]
    fn plain_and_unicode_text_is_untouched() {
        assert_eq!(html_escape("plain text 123"), "plain text 123");
        assert_eq!(
            html_escape("caf\u{00e9} \u{4e16}\u{754c}"),
            "caf\u{00e9} \u{4e16}\u{754c}"
        );
        assert_eq!(html_escape(""), "");
    }
}
