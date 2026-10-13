//! Form-component percent-encoding for `application/x-www-form-urlencoded`
//! bodies, self-contained and pinned by tests.
//!
//! Stripe accepts percent-encoding for form values, so this encoder keeps the
//! RFC 3986 unreserved set (`ALPHA / DIGIT / "-" / "." / "_" / "~"`) and
//! percent-encodes everything else with uppercase hex. In particular a space
//! becomes `%20`, never `+` — the encoding is explicit, not HTML-form style.

/// Percent-encode one form component. The output is pure ASCII.
pub fn encode_form_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
