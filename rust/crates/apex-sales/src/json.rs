//! A small, strict JSON reader and writer.
//!
//! The engine persists its state as JSON lines and reads the App Store lookup
//! API, and it does so without external crates. Objects keep insertion order
//! so files diff cleanly. The parser follows RFC 8259: it rejects trailing
//! commas, comments, leading zeros, unescaped control characters and lone
//! surrogates, and it caps nesting depth so hostile input cannot overflow the
//! stack.

use std::fmt::Write as _;

/// Maximum nesting depth the parser accepts.
pub const MAX_DEPTH: usize = 64;

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null`
    Null,
    /// `true` / `false`
    Bool(bool),
    /// A number, held as an `f64`.
    Number(f64),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Value>),
    /// An object, in insertion order.
    Object(Vec<(String, Value)>),
}

/// A parse failure with its byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    /// Byte offset of the failure.
    pub offset: usize,
    /// What went wrong.
    pub message: &'static str,
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid JSON at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for JsonError {}

impl Value {
    /// Build an object from `(key, value)` pairs.
    pub fn object<K: Into<String>>(pairs: Vec<(K, Value)>) -> Self {
        Value::Object(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// A string value.
    pub fn str(s: impl Into<String>) -> Self {
        Value::String(s.into())
    }

    /// A number value.
    pub fn num(n: impl Into<f64>) -> Self {
        Value::Number(n.into())
    }

    /// An optional string: `null` when absent.
    pub fn opt_str(s: Option<&str>) -> Self {
        s.map_or(Value::Null, |s| Value::String(s.to_string()))
    }

    /// The member `key` of an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// The number, if this is one.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The number as an `i64` when it is integral and in range.
    pub fn as_i64(&self) -> Option<i64> {
        let n = self.as_f64()?;
        if n.fract() == 0.0 && n.abs() < 9.0e15 {
            Some(n as i64)
        } else {
            None
        }
    }

    /// The boolean, if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The elements, if this is an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    /// A string member, or `None` when absent, null or not a string.
    pub fn str_at(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// A numeric member.
    pub fn f64_at(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Value::as_f64)
    }

    /// Serialize compactly. Non-finite numbers become `null`.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        write_value(self, &mut out);
        out
    }
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => write_number(*n, out),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Object(pairs) => {
            out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write_value(v, out);
            }
            out.push('}');
        }
    }
}

fn write_number(n: f64, out: &mut String) {
    if !n.is_finite() {
        out.push_str("null");
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        let _ = write!(out, "{}", n as i64);
    } else {
        let _ = write!(out, "{n}");
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Parse one JSON document; trailing non-whitespace is an error.
pub fn parse(input: &str) -> Result<Value, JsonError> {
    let mut p = Parser {
        bytes: input.as_bytes(),
        pos: 0,
    };
    p.skip_ws();
    let v = p.value(0)?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(v)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, message: &'static str) -> JsonError {
        JsonError {
            offset: self.pos,
            message,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn expect_literal(&mut self, lit: &[u8], v: Value) -> Result<Value, JsonError> {
        if self.bytes.get(self.pos..self.pos + lit.len()) == Some(lit) {
            self.pos += lit.len();
            Ok(v)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        match self.peek() {
            Some(b'n') => self.expect_literal(b"null", Value::Null),
            Some(b't') => self.expect_literal(b"true", Value::Bool(true)),
            Some(b'f') => self.expect_literal(b"false", Value::Bool(false)),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
            None => Err(self.err("unexpected end of input")),
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.pos += 1;
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(pairs));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected a string key"));
            }
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.err("expected ':'"));
            }
            self.pos += 1;
            self.skip_ws();
            let v = self.value(depth + 1)?;
            pairs.push((key, v));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(pairs));
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.err("leading zero"));
                }
            }
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.err("expected a digit")),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("expected a fraction digit"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("expected an exponent digit"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        // The grammar above admits only ASCII digits, signs, '.', 'e' and 'E',
        // and every such string parses as an f64 (overflow becomes infinity).
        let text = std::str::from_utf8(self.bytes.get(start..self.pos).unwrap_or_default())
            .unwrap_or_default();
        match text.parse::<f64>() {
            Ok(n) => Ok(Value::Number(n)),
            Err(_) => Err(self.err("invalid number")),
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let slice = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.err("short unicode escape"))?;
        let text = std::str::from_utf8(slice).map_err(|_| self.err("bad unicode escape"))?;
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err("bad unicode escape"))?;
        self.pos += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            if self.pos > start {
                // The input is a `&str` and the chunk is bounded by ASCII bytes,
                // so it is always valid UTF-8.
                match std::str::from_utf8(self.bytes.get(start..self.pos).unwrap_or_default()) {
                    Ok(chunk) => out.push_str(chunk),
                    Err(_) => return Err(self.err("invalid UTF-8")),
                }
            }
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.peek().ok_or_else(|| self.err("unterminated escape"))?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                    return Err(self.err("lone high surrogate"));
                                }
                                self.pos += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("invalid low surrogate"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.err("lone low surrogate"));
                            } else {
                                hi
                            };
                            let c =
                                char::from_u32(code).ok_or_else(|| self.err("bad code point"))?;
                            out.push(c);
                        }
                        _ => return Err(self.err("unknown escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in string")),
                None => return Err(self.err("unterminated string")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_nested_values_in_order() {
        let v = Value::object(vec![
            ("b", Value::num(1)),
            (
                "a",
                Value::Array(vec![Value::Null, Value::Bool(true), Value::num(2.5)]),
            ),
            (
                "s",
                Value::str("quote \" slash \\ newline\n tab\t ctl\u{1} é 😀"),
            ),
        ]);
        let text = v.to_json();
        assert!(text.starts_with("{\"b\":1,\"a\":[null,true,2.5]"));
        assert_eq!(parse(&text), Ok(v));
    }

    #[test]
    fn parses_the_app_store_lookup_shape() {
        let text = r#"{"resultCount":1,"results":[{"trackId":123456789,"trackName":"Puzzle \u00e9","userRatingCount":48210,"price":0.0,"sellerUrl":"https://studio.example"}]}"#;
        let v = parse(text).unwrap_or(Value::Null);
        let r = &v
            .get("results")
            .and_then(Value::as_array)
            .unwrap_or_default()[0];
        assert_eq!(r.get("trackId").and_then(Value::as_i64), Some(123_456_789));
        assert_eq!(r.str_at("trackName"), Some("Puzzle é"));
        assert_eq!(r.f64_at("price"), Some(0.0));
        assert_eq!(v.get("missing"), None);
        assert_eq!(Value::num(1.5).as_i64(), None);
    }

    #[test]
    fn decodes_surrogate_pairs_and_rejects_lone_ones() {
        assert_eq!(parse(r#""\ud83d\ude00""#), Ok(Value::str("😀")));
        assert!(parse(r#""\ud83d""#).is_err());
        assert!(parse(r#""\ude00""#).is_err());
        assert!(parse(r#""\ud83d\u0041""#).is_err());
        // Short, non-hex and character-splitting escapes.
        for bad in ["\"\\u12\"", "\"\\u00\u{e9}0\"", "\"\\u000\u{e9}\"", "\"\\"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rejects_malformed_documents() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            "1e",
            "-",
            "tru",
            "\"a",
            "\"\u{1}\"",
            "{1:2}",
            "[1 2]",
            "{\"a\" 1}",
            "1 2",
            "\"\\x\"",
            "nul",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(MAX_DEPTH + 2);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn writes_non_finite_numbers_as_null_and_big_numbers_exactly() {
        assert_eq!(Value::num(f64::NAN).to_json(), "null");
        assert_eq!(Value::num(1.0e20).to_json(), "100000000000000000000");
        assert_eq!(Value::num(-3.0).to_json(), "-3");
        assert_eq!(Value::opt_str(None).to_json(), "null");
        assert_eq!(parse("-0.5e1"), Ok(Value::num(-5.0)));
        let err = parse("[1,]")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("byte"));
    }
}
