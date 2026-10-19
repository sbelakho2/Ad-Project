//! Configuration helpers shared by the API binary: environment reads, the
//! `publisher=<hex key>` allow-list and the hex decoding of wire-encoded
//! binary keys (e.g. the Adyen webhook HMAC key, which Adyen publishes as hex
//! while the verifier consumes raw bytes). They live in the library so they
//! are compiled once and unit tested.

use std::collections::HashMap;
use std::fmt;

use apex_proof::VerifyingKey;
use url::Url;

use crate::is_valid_publisher;

/// A trimmed, non-empty environment value.
pub fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// The environment variable naming the public console origin the emailed
/// sign-in link starts with.
pub const CONSOLE_URL_ENV: &str = "APEX_CONSOLE_URL";
/// The environment variable naming the public Help destination of the
/// authenticated console pages.
pub const SUPPORT_URL_ENV: &str = "APEX_SUPPORT_URL";
/// Optional operator declaration of provisioned workspace domains:
/// `{"publisher-key": "domain.example"}`. A provisioned workspace that
/// already holds runner data can only be claimed by an address on its
/// declared domain.
pub const WORKSPACE_DOMAINS_ENV: &str = "APEX_WORKSPACE_DOMAINS";
/// The Help destination used when [`SUPPORT_URL_ENV`] is unset.
pub const DEFAULT_SUPPORT_URL: &str = "https://apexmediation.ee/support/";

/// Why a configured public URL was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicUrlProblem {
    /// The value is not an absolute URL (`console.example`, `/support/`).
    NotAbsolute,
    /// The scheme is neither `http` nor `https` (`javascript:`, `ftp://`).
    NotHttp,
    /// The URL carries no host (`https://`).
    MissingHost,
    /// The URL carries a query or fragment.
    QueryOrFragment,
    /// Plain `http` is only accepted for the loopback dev hosts.
    InsecureScheme,
}

impl PublicUrlProblem {
    /// The operator-facing description of the refusal.
    pub const fn reason(self) -> &'static str {
        match self {
            PublicUrlProblem::NotAbsolute => "it must be an absolute URL with a host",
            PublicUrlProblem::NotHttp => "the scheme must be http or https",
            PublicUrlProblem::MissingHost => "the URL must name a host",
            PublicUrlProblem::QueryOrFragment => "the URL must not carry a query or fragment",
            PublicUrlProblem::InsecureScheme => {
                "http is only accepted for localhost, a *.localhost name or 127.0.0.1"
            }
        }
    }
}

/// A public URL configuration value that failed validation. The error names
/// the variable and the offending value so an operator log points at the
/// exact setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicUrlError {
    variable: String,
    value: String,
    problem: PublicUrlProblem,
}

impl PublicUrlError {
    fn new(variable: &str, value: &str, problem: PublicUrlProblem) -> Self {
        Self {
            variable: variable.to_string(),
            value: value.to_string(),
            problem,
        }
    }

    /// The environment variable that carried the invalid value.
    pub fn variable(&self) -> &str {
        &self.variable
    }

    /// The refused value, trimmed as it was validated.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The rule the value broke.
    pub fn problem(&self) -> PublicUrlProblem {
        self.problem
    }
}

impl fmt::Display for PublicUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is not a valid public URL: {} (got '{}')",
            self.variable,
            self.problem.reason(),
            self.value
        )
    }
}

impl std::error::Error for PublicUrlError {}

/// Validate an absolute `http(s)` URL with a host and no query/fragment.
///
/// `https` is required unless the host is the loopback dev host `localhost`
/// or `127.0.0.1`. The trimmed value is returned unchanged so callers can
/// apply their own trailing-slash policy.
pub fn validate_public_url(variable: &str, raw: &str) -> Result<String, PublicUrlError> {
    let raw = raw.trim();
    let parsed = Url::parse(raw)
        .map_err(|_| PublicUrlError::new(variable, raw, PublicUrlProblem::NotAbsolute))?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(PublicUrlError::new(
                variable,
                raw,
                PublicUrlProblem::NotHttp,
            ))
        }
    }
    let Some(host) = parsed.host_str().filter(|host| !host.is_empty()) else {
        return Err(PublicUrlError::new(
            variable,
            raw,
            PublicUrlProblem::MissingHost,
        ));
    };
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(PublicUrlError::new(
            variable,
            raw,
            PublicUrlProblem::QueryOrFragment,
        ));
    }
    // RFC 6761 reserves the whole `.localhost` namespace for loopback, so a
    // dev host such as `console.localhost` is just as local as `localhost`.
    let loopback = host == "localhost" || host == "127.0.0.1" || host.ends_with(".localhost");
    if parsed.scheme() == "http" && !loopback {
        return Err(PublicUrlError::new(
            variable,
            raw,
            PublicUrlProblem::InsecureScheme,
        ));
    }
    Ok(raw.to_string())
}

/// Parse [`CONSOLE_URL_ENV`]. Unset (or blank) means sign-in links are
/// disabled; a configured value must pass [`validate_public_url`]. Trailing
/// slashes are normalized away, as the link builder has always done.
pub fn console_url(value: Option<&str>) -> Result<Option<String>, PublicUrlError> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => {
            let validated = validate_public_url(CONSOLE_URL_ENV, raw)?;
            Ok(Some(validated.trim_end_matches('/').to_string()))
        }
        None => Ok(None),
    }
}

/// Parse [`SUPPORT_URL_ENV`], falling back to [`DEFAULT_SUPPORT_URL`] when
/// unset. A configured value follows the same rules as the console origin.
pub fn support_url(value: Option<&str>) -> Result<String, PublicUrlError> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => validate_public_url(SUPPORT_URL_ENV, raw),
        None => Ok(DEFAULT_SUPPORT_URL.to_string()),
    }
}

/// Parse `publisher=<64 hex>` entries into the key allow-list.
pub fn parse_keys(raw: &str) -> Result<HashMap<String, VerifyingKey>, String> {
    let mut keys = HashMap::new();
    for entry in raw.split(',').filter(|entry| !entry.trim().is_empty()) {
        let (publisher, hex_key) = entry.split_once('=').ok_or_else(|| {
            format!("APEX_LEDGER_KEYS entry {entry:?} is not publisher=<hex key>")
        })?;
        let publisher = publisher.trim();
        if !is_valid_publisher(publisher) {
            return Err(format!(
                "APEX_LEDGER_KEYS publisher {publisher:?} must match ^[A-Za-z0-9_-]{{1,64}}$"
            ));
        }
        if keys.contains_key(publisher) {
            return Err(format!(
                "APEX_LEDGER_KEYS lists publisher {publisher:?} twice"
            ));
        }
        keys.insert(publisher.to_string(), parse_key_hex(hex_key)?);
    }
    Ok(keys)
}

/// Decode a 64-hex ed25519 verifying key.
pub fn parse_key_hex(raw: &str) -> Result<VerifyingKey, String> {
    let raw = raw.trim();
    if raw.len() != 64 {
        return Err(format!(
            "ed25519 public key must be 64 hex characters, got {}",
            raw.len()
        ));
    }
    let mut bytes = [0u8; 32];
    let raw = raw.as_bytes();
    for (index, byte) in bytes.iter_mut().enumerate() {
        let high = hex_nibble(raw[2 * index])
            .ok_or_else(|| format!("public key has a non-hex character at byte {index}"))?;
        let low = hex_nibble(raw[2 * index + 1])
            .ok_or_else(|| format!("public key has a non-hex character at byte {index}"))?;
        *byte = (high << 4) | low;
    }
    VerifyingKey::from_bytes(&bytes).map_err(|error| format!("invalid ed25519 public key: {error}"))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// A configuration value documented as hex that failed to decode. The error
/// names the variable so an operator log points at the exact setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexKeyError {
    variable: String,
    reason: String,
}

impl HexKeyError {
    /// The environment variable that carried the invalid value.
    pub fn variable(&self) -> &str {
        &self.variable
    }
}

impl fmt::Display for HexKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is not valid hex: {}", self.variable, self.reason)
    }
}

impl std::error::Error for HexKeyError {}

/// Decode a hex-encoded binary key from configuration. Adyen publishes the
/// webhook HMAC key as hex text, but signatures are computed over the raw
/// key bytes, so the text form must be decoded before it reaches the
/// verifier. An empty value decodes to an empty key (the caller decides
/// whether that counts as configured); odd-length or non-hex values are a
/// [`HexKeyError`] naming `variable`.
pub fn decode_hex_env_key(variable: &str, raw: &str) -> Result<Vec<u8>, HexKeyError> {
    hex::decode(raw.trim()).map_err(|error| HexKeyError {
        variable: variable.to_string(),
        reason: error.to_string(),
    })
}

/// The environment variable naming the one-time sign-in link lifetime.
pub const SIGNIN_TTL_HOURS_ENV: &str = "APEX_SIGNIN_TTL_HOURS";
/// Lifetime of a sign-in link when [`SIGNIN_TTL_HOURS_ENV`] is unset.
pub const DEFAULT_SIGNIN_TTL_HOURS: i64 = 72;
/// Smallest accepted sign-in link lifetime.
pub const MIN_SIGNIN_TTL_HOURS: i64 = 1;
/// Largest accepted sign-in link lifetime.
pub const MAX_SIGNIN_TTL_HOURS: i64 = 720;

/// A sign-in TTL value outside the documented `1..=720` bounds (or not an
/// integer at all). The error names the variable and the offending value so
/// an operator log points at the exact setting; it is never silently clamped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigninTtlError {
    variable: String,
    value: String,
}

impl SigninTtlError {
    /// The environment variable that carried the invalid value.
    pub fn variable(&self) -> &str {
        &self.variable
    }
}

impl fmt::Display for SigninTtlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} must be an integer between {MIN_SIGNIN_TTL_HOURS} and \
             {MAX_SIGNIN_TTL_HOURS}, got '{}'",
            self.variable, self.value
        )
    }
}

impl std::error::Error for SigninTtlError {}

/// Parse the sign-in link lifetime from [`SIGNIN_TTL_HOURS_ENV`]. An absent
/// value is the documented default; anything non-numeric or outside the
/// bounds is a typed [`SigninTtlError`].
pub fn signin_ttl_hours(value: Option<&str>) -> Result<i64, SigninTtlError> {
    let Some(raw) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(DEFAULT_SIGNIN_TTL_HOURS);
    };
    let refuse = || SigninTtlError {
        variable: SIGNIN_TTL_HOURS_ENV.to_string(),
        value: raw.to_string(),
    };
    let hours = raw.parse::<i64>().map_err(|_| refuse())?;
    if (MIN_SIGNIN_TTL_HOURS..=MAX_SIGNIN_TTL_HOURS).contains(&hours) {
        Ok(hours)
    } else {
        Err(refuse())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn key_hex_roundtrips_a_seeded_keypair() {
        let keypair = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let mut hex = String::new();
        for byte in keypair.verifying_key().to_bytes() {
            hex.push_str(&format!("{byte:02x}"));
        }
        let parsed = parse_key_hex(&hex).expect("parse");
        assert_eq!(parsed, keypair.verifying_key());
        assert_eq!(parse_key_hex(&hex.to_uppercase()).expect("upper"), parsed);
    }

    #[test]
    fn key_and_entry_errors_are_named() {
        assert!(parse_key_hex("ab").expect_err("short").contains("64 hex"));
        assert!(parse_key_hex(&"zz".repeat(32))
            .expect_err("non-hex")
            .contains("non-hex"));
        assert!(parse_keys("no-equals")
            .expect_err("shape")
            .contains("publisher=<hex"));
        assert!(parse_keys("../evil=00").expect_err("charset").contains("^"));
        let good = format!("pub1={}", "00".repeat(32));
        assert!(parse_keys(&good).is_ok());
        assert!(parse_keys(&format!("{good},{good}"))
            .expect_err("duplicate")
            .contains("twice"));
        // Empty entries are skipped.
        assert!(parse_keys(&format!(" ,{good}, ")).is_ok());
    }

    #[test]
    fn env_value_trims_and_refuses_blanks() {
        std::env::set_var("APEX_CONFIG_TEST", "value");
        assert_eq!(env_value("APEX_CONFIG_TEST").as_deref(), Some("value"));
        std::env::set_var("APEX_CONFIG_TEST", "   ");
        assert!(env_value("APEX_CONFIG_TEST").is_none());
        std::env::remove_var("APEX_CONFIG_TEST");
        assert!(env_value("APEX_CONFIG_TEST").is_none());
    }

    #[test]
    fn hex_key_decoding_is_binary_and_named_errors_refuse_bad_hex() {
        // Adyen's key is hex text over raw bytes: "686d61635f6170695f74657374"
        // is the binary key b"hmac_api_test", not the 24-character text.
        let decoded = decode_hex_env_key("APEX_ADYEN_HMAC_KEY", "686d61635f6170695f74657374")
            .expect("valid hex");
        assert_eq!(decoded, b"hmac_api_test");
        assert_ne!(decoded, "686d61635f6170695f74657374".as_bytes());
        // Whitespace around the value is tolerated; the payload is not.
        assert_eq!(
            decode_hex_env_key("APEX_ADYEN_HMAC_KEY", " 686d61635f6170695f74657374 ")
                .expect("trimmed hex"),
            b"hmac_api_test"
        );
        // An absent key decodes to an empty key, not an error.
        assert_eq!(
            decode_hex_env_key("APEX_ADYEN_HMAC_KEY", "").expect("empty"),
            Vec::<u8>::new()
        );

        let odd = decode_hex_env_key("APEX_ADYEN_HMAC_KEY", "abc").expect_err("odd length");
        assert_eq!(odd.variable(), "APEX_ADYEN_HMAC_KEY");
        assert!(odd.to_string().contains("APEX_ADYEN_HMAC_KEY"), "{odd}");
        let non_hex = decode_hex_env_key("APEX_ADYEN_HMAC_KEY", "zz").expect_err("non-hex");
        assert_eq!(non_hex.variable(), "APEX_ADYEN_HMAC_KEY");
        assert!(
            non_hex.to_string().contains("APEX_ADYEN_HMAC_KEY"),
            "{non_hex}"
        );
        assert!(non_hex.to_string().to_ascii_lowercase().contains("hex"));
    }

    #[test]
    fn console_url_is_optional_normalized_and_strictly_validated() {
        assert_eq!(console_url(None).expect("unset"), None);
        assert_eq!(console_url(Some("   ")).expect("blank"), None);
        assert_eq!(
            console_url(Some(" https://console.example/ "))
                .expect("configured")
                .as_deref(),
            Some("https://console.example")
        );
        assert_eq!(
            console_url(Some("https://console.example///"))
                .expect("slashes")
                .as_deref(),
            Some("https://console.example")
        );
        assert_eq!(
            console_url(Some("http://localhost:8787"))
                .expect("loopback http")
                .as_deref(),
            Some("http://localhost:8787")
        );
        assert_eq!(
            console_url(Some("http://console.localhost:8081"))
                .expect("loopback namespace")
                .as_deref(),
            Some("http://console.localhost:8081")
        );
        assert_eq!(
            console_url(Some("http://127.0.0.1:8787"))
                .expect("loopback ip")
                .as_deref(),
            Some("http://127.0.0.1:8787")
        );

        let cases = [
            ("console.example", PublicUrlProblem::NotAbsolute),
            ("/console", PublicUrlProblem::NotAbsolute),
            ("javascript:alert(1)", PublicUrlProblem::NotHttp),
            ("ftp://console.example", PublicUrlProblem::NotHttp),
            ("https://", PublicUrlProblem::NotAbsolute),
            (
                "https://console.example?token=1",
                PublicUrlProblem::QueryOrFragment,
            ),
            (
                "https://console.example#fragment",
                PublicUrlProblem::QueryOrFragment,
            ),
            ("http://console.example", PublicUrlProblem::InsecureScheme),
        ];
        for (bad, problem) in cases {
            let error = console_url(Some(bad)).expect_err(bad);
            assert_eq!(error.variable(), CONSOLE_URL_ENV, "{bad}");
            assert_eq!(error.value(), bad, "{bad}");
            assert_eq!(error.problem(), problem, "{bad}");
            assert!(
                error.to_string().contains(CONSOLE_URL_ENV),
                "{bad}: {error}"
            );
            assert!(error.to_string().contains(bad), "{bad}: {error}");
        }
    }

    #[test]
    fn support_url_defaults_and_follows_the_same_rules() {
        assert_eq!(
            support_url(None).expect("default"),
            DEFAULT_SUPPORT_URL.to_string()
        );
        assert_eq!(
            support_url(Some(" https://help.example/support/ ")).expect("configured"),
            "https://help.example/support/"
        );
        assert_eq!(
            support_url(Some("http://localhost:3000/support/")).expect("loopback"),
            "http://localhost:3000/support/"
        );
        for bad in [
            "/support/",
            "javascript:void(0)",
            "ftp://help.example",
            "https://help.example?q=1",
            "http://help.example/support/",
        ] {
            let error = support_url(Some(bad)).expect_err(bad);
            assert_eq!(error.variable(), SUPPORT_URL_ENV, "{bad}");
            assert!(
                error.to_string().contains(SUPPORT_URL_ENV),
                "{bad}: {error}"
            );
        }
    }

    #[test]
    fn signin_ttl_defaults_and_refuses_values_outside_the_bounds() {
        assert_eq!(signin_ttl_hours(None).expect("default"), 72);
        assert_eq!(signin_ttl_hours(Some("  ")).expect("blank"), 72);
        assert_eq!(signin_ttl_hours(Some("72")).expect("explicit"), 72);
        assert_eq!(signin_ttl_hours(Some(" 1 ")).expect("bounds low"), 1);
        assert_eq!(signin_ttl_hours(Some("720")).expect("bounds high"), 720);
        for bad in ["0", "721", "-1", "abc", "72.5", "1e2"] {
            let error = signin_ttl_hours(Some(bad)).expect_err(bad);
            assert_eq!(error.variable(), SIGNIN_TTL_HOURS_ENV);
            assert!(
                error.to_string().contains(SIGNIN_TTL_HOURS_ENV),
                "{bad}: {error}"
            );
            assert!(error.to_string().contains(bad), "{bad}: {error}");
            assert!(error.to_string().contains("1 and 720"), "{bad}: {error}");
        }
    }
}

/// Parse and validate [`WORKSPACE_DOMAINS_ENV`]. The value is a JSON object
/// mapping publisher keys to lowercase email domains. An invalid entry is a
/// startup refusal: silently dropping it would reopen the collision hole the
/// declaration exists to close.
pub fn workspace_domains(
    raw: Option<&str>,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let Some(raw) = raw else {
        return Ok(std::collections::BTreeMap::new());
    };
    let parsed: std::collections::BTreeMap<String, String> = serde_json::from_str(raw)
        .map_err(|error| format!("{WORKSPACE_DOMAINS_ENV} is not a JSON object: {error}"))?;
    let mut domains = std::collections::BTreeMap::new();
    for (publisher, domain) in parsed {
        if !crate::sources::is_valid_publisher(&publisher) {
            return Err(format!(
                "{WORKSPACE_DOMAINS_ENV} has an invalid publisher key: {publisher:?}"
            ));
        }
        let domain = domain.trim().to_ascii_lowercase();
        if domain.is_empty()
            || !domain
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            || !domain.contains('.')
        {
            return Err(format!(
                "{WORKSPACE_DOMAINS_ENV} has an invalid domain for {publisher:?}: {domain:?}"
            ));
        }
        domains.insert(publisher, domain);
    }
    Ok(domains)
}

#[cfg(test)]
mod workspace_domain_tests {
    use super::*;

    #[test]
    fn a_workspace_domain_map_is_validated_strictly() {
        let parsed = workspace_domains(Some(r#"{"studio-example":"Studio.Example"}"#)).unwrap();
        assert_eq!(
            parsed.get("studio-example").map(String::as_str),
            Some("studio.example")
        );
        assert!(workspace_domains(Some(r#"{"../etc":"studio.example"}"#)).is_err());
        assert!(workspace_domains(Some(r#"{"studio-example":"https://studio.example"}"#)).is_err());
        assert!(workspace_domains(Some(r#"{"studio-example":"localhost"}"#)).is_err());
        assert!(workspace_domains(Some("[]")).is_err());
        assert!(workspace_domains(None).unwrap().is_empty());
    }
}
