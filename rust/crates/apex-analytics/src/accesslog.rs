//! The Caddy access-log parser: public page requests from the delivery
//! boundary, with query redaction and no personal data.
//!
//! Section 05 (page 73): "Collect public requests from the delivery/access-log
//! boundary with query redaction." The catalog row requires "Route template,
//! status, referrer host, allowed campaign code, bot classification. No
//! secret query string." Data minimization (page 86) bans complete query
//! strings, ad-user identifiers and free text from telemetry.
//!
//! What this parser keeps: the normalized route template, the HTTP status
//! and its class, the referrer host only, the campaign code of designated
//! campaign parameters, the allow-listed query parameter **keys** (never
//! their values), and a bounded bot classification. It never copies the
//! client address, the raw user agent, the raw path or any query value.
//!
//! The deployment Caddyfile needs a JSON access log on the site block:
//!
//! ```text
//! apexmediation.ee {
//!     log {
//!         output file /var/log/caddy/apex-access.json
//!         format json
//!     }
//! }
//! ```
//!
//! `apex-ingest /var/log/caddy/apex-access.json` then writes one sanitized
//! `request_logs` row per request line (30-day retention).

use serde::Deserialize;

use crate::envelope::{DomainEvent, EventSource, EventType};
use crate::store::RequestLogRow;
use crate::timestamp::UtcMillis;

/// Document routes the marketing site serves; anything else collapses to
/// `/unmatched` so the route dimension stays bounded.
pub const KNOWN_ROUTES: [&str; 10] = [
    "/",
    "/for-publishers",
    "/free-audit",
    "/how-it-works",
    "/pricing",
    "/privacy",
    "/proof",
    "/security",
    "/support",
    "/terms",
];

/// Query parameter keys recorded (as keys only). Unlisted keys collapse to
/// `other` so a hostile query cannot inflate the dimension.
pub const ALLOWED_QUERY_KEYS: [&str; 6] = [
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "ref",
    "campaign",
    "page",
];

/// Campaign parameters whose sanitized value is kept as the campaign code.
pub const CAMPAIGN_KEYS: [&str; 2] = ["utm_campaign", "campaign"];

/// Bounded bot classification. `Unclassified` is explicitly not a claim
/// that a human visited: the static site cannot honestly report human
/// attention from access logs (page 86, Static-site limitation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotClass {
    /// Declared search engine or AI crawler.
    SearchEngineBot,
    /// Automation client (HTTP library, monitoring probe, CLI).
    AutomationClient,
    /// Other declared bot (social preview, generic crawler token).
    OtherBot,
    /// No bot token matched; not a human claim.
    Unclassified,
}

impl BotClass {
    /// Stable lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            BotClass::SearchEngineBot => "search_engine_bot",
            BotClass::AutomationClient => "automation_client",
            BotClass::OtherBot => "other_bot",
            BotClass::Unclassified => "unclassified",
        }
    }

    /// Classify a user agent string with the documented token lists.
    pub fn classify(user_agent: Option<&str>) -> Self {
        let Some(user_agent) = user_agent else {
            return BotClass::Unclassified;
        };
        let ua = user_agent.to_lowercase();
        for token in SEARCH_ENGINE_TOKENS {
            if ua.contains(token) {
                return BotClass::SearchEngineBot;
            }
        }
        for token in AUTOMATION_TOKENS {
            if ua.contains(token) {
                return BotClass::AutomationClient;
            }
        }
        for token in OTHER_BOT_TOKENS {
            if ua.contains(token) {
                return BotClass::OtherBot;
            }
        }
        BotClass::Unclassified
    }
}

impl std::fmt::Display for BotClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Declared search engine and AI crawler tokens (lower case, substring
/// match on the lower-cased user agent).
pub const SEARCH_ENGINE_TOKENS: [&str; 16] = [
    "googlebot",
    "bingbot",
    "duckduckbot",
    "yandexbot",
    "applebot",
    "baiduspider",
    "sogou",
    "exabot",
    "ia_archiver",
    "petalbot",
    "gptbot",
    "ccbot",
    "claudebot",
    "perplexitybot",
    "oai-searchbot",
    "bytespider",
];

/// Automation client tokens: HTTP libraries, CLIs and monitoring probes.
pub const AUTOMATION_TOKENS: [&str; 18] = [
    "curl/",
    "wget/",
    "python-requests",
    "python-urllib",
    "httpx/",
    "go-http-client",
    "java/",
    "okhttp",
    "axios/",
    "node-fetch",
    "libwww-perl",
    "httpie",
    "postmanruntime",
    "insomnia/",
    "uptimerobot",
    "pingdom",
    "statuscake",
    "site24x7",
];

/// Other declared bots: social previews and generic crawler tokens.
pub const OTHER_BOT_TOKENS: [&str; 15] = [
    "facebookexternalhit",
    "embedly",
    "quora link preview",
    "telegrambot",
    "discordbot",
    "whatsapp",
    "skypeuripreview",
    "twitterbot",
    "linkedinbot",
    "pinterestbot",
    "slackbot",
    "ahrefsbot",
    "semrushbot",
    "mj12bot",
    "bot",
];

/// Parser failures. A line that fails here is rejected and counted, never
/// silently dropped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccessLogError {
    /// The line is not a JSON object, or a field has the wrong shape.
    #[error("line is not a readable Caddy JSON record: {0}")]
    Json(String),
    /// The record has no request URI at all.
    #[error("record has no request URI")]
    MissingUri,
    /// The record has no HTTP status.
    #[error("record has no HTTP status")]
    MissingStatus,
    /// The request URI is not a usable path.
    #[error("request URI is not a usable path")]
    BadPath,
    /// The timestamp is missing or unrepresentable.
    #[error("record has no usable timestamp")]
    BadTimestamp,
    /// The parsed event failed envelope validation.
    #[error("event validation failed: {0}")]
    Validation(String),
}

/// Deployment labels and receive time for parsed rows.
#[derive(Debug, Clone)]
pub struct AccessLogDefaults {
    /// Environment label.
    pub environment: String,
    /// Emitting service version label (carried by the validation envelope).
    pub service_version: String,
    /// When the parser received the line.
    pub received_at: UtcMillis,
}

#[derive(Debug, Deserialize)]
struct CaddyEntry {
    #[serde(default)]
    ts: Option<serde_json::Value>,
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    request: Option<CaddyRequest>,
}

#[derive(Debug, Deserialize)]
struct CaddyRequest {
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    headers: Option<CaddyHeaders>,
}

#[derive(Debug, Deserialize)]
struct CaddyHeaders {
    #[serde(rename = "User-Agent", default)]
    user_agent: Option<Vec<String>>,
    #[serde(rename = "Referer", default)]
    referer: Option<Vec<String>>,
}

/// Parse one Caddy JSON access-log line into a sanitized `request_logs`
/// row. Returns `Ok(None)` for records that are not HTTP requests (Caddy
/// writes partial lifecycle records too); those are counted by the caller
/// and never counted as pages.
///
/// The row is assembled only after the equivalent `public_page_requested`
/// envelope validates, so the envelope's value checks (credentials, raw
/// query strings and token-looking blobs are refused) still guard every
/// field, and the derived event identity becomes the row identity so a
/// replay of one line stays recognizable.
pub fn parse_line(
    line: &str,
    defaults: &AccessLogDefaults,
) -> Result<Option<RequestLogRow>, AccessLogError> {
    let entry: CaddyEntry =
        serde_json::from_str(line).map_err(|error| AccessLogError::Json(error.to_string()))?;
    let Some(request) = entry.request else {
        return Ok(None);
    };
    let Some(uri) = request.uri.as_deref() else {
        return Err(AccessLogError::MissingUri);
    };
    let occurred = parse_timestamp(entry.ts.as_ref())?;
    let (path, query) = match uri.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (uri, None),
    };
    let path = path.split('#').next().unwrap_or(path);
    let route_template = normalize_route(path).ok_or(AccessLogError::BadPath)?;
    let status = entry.status.ok_or(AccessLogError::MissingStatus)?;
    let user_agent = request
        .headers
        .as_ref()
        .and_then(|headers| headers.user_agent.as_ref())
        .and_then(|values| values.first())
        .map(String::as_str);
    let bot_class = BotClass::classify(user_agent);
    let referrer_host = request
        .headers
        .as_ref()
        .and_then(|headers| headers.referer.as_ref())
        .and_then(|values| values.first())
        .and_then(|value| referrer_host(value));
    let (query_keys, campaign_code) = match query {
        Some(query) => parse_query(query),
        None => (Vec::new(), None),
    };

    let payload = crate::envelope::EventPayload {
        route_template: Some(route_template.clone()),
        status: Some(status),
        referrer_host: referrer_host.clone(),
        campaign_code: campaign_code.clone(),
        bot_class: Some(bot_class.as_str().to_string()),
        query_keys: if query_keys.is_empty() {
            None
        } else {
            Some(query_keys.clone())
        },
        ..crate::envelope::EventPayload::default()
    };
    let event = DomainEvent::builder(EventType::PublicPageRequested, EventSource::AccessLog)
        .with_occurred_at(occurred)
        .with_received_at(defaults.received_at)
        .with_environment(defaults.environment.clone())
        .with_service_version(defaults.service_version.clone())
        .with_object("document", route_template.clone())
        .with_outcome(status_class(status))
        .with_payload(payload)
        .build()
        .map_err(|error| AccessLogError::Validation(error.to_string()))?;
    Ok(Some(RequestLogRow {
        event_id: event.event_id,
        occurred_at_utc: event.occurred_at_utc,
        received_at_utc: event.received_at_utc,
        route_template,
        status,
        status_class: status_class(status),
        bot_class: bot_class.as_str().to_string(),
        referrer_host,
        campaign_code,
        query_keys,
        environment: event.environment,
    }))
}

fn parse_timestamp(ts: Option<&serde_json::Value>) -> Result<UtcMillis, AccessLogError> {
    match ts {
        Some(serde_json::Value::Number(number)) => {
            let seconds = number.as_f64().ok_or(AccessLogError::BadTimestamp)?;
            if !seconds.is_finite() {
                return Err(AccessLogError::BadTimestamp);
            }
            let millis = (seconds * 1_000.0).round();
            if !(-9e15..=9e15).contains(&millis) {
                return Err(AccessLogError::BadTimestamp);
            }
            Ok(UtcMillis::from_unix_millis(millis as i64))
        }
        Some(serde_json::Value::String(text)) => {
            UtcMillis::parse_rfc3339(text).map_err(|_| AccessLogError::BadTimestamp)
        }
        _ => Err(AccessLogError::BadTimestamp),
    }
}

/// HTTP status class label.
pub fn status_class(status: u16) -> String {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
    .to_string()
}

/// Normalize a request path to a bounded route template.
///
/// Known document routes pass through; `/guides/...` and `/rerun-kit/...`
/// collapse to one template each; anything else becomes `/unmatched`.
/// Percent-encoding is intentionally not decoded: the raw path never leaves
/// this function, only the template does.
pub fn normalize_route(path: &str) -> Option<String> {
    if !path.starts_with('/') || path.len() > 512 {
        return None;
    }
    let trimmed = if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    };
    if KNOWN_ROUTES.contains(&trimmed) {
        return Some(trimmed.to_string());
    }
    if trimmed.starts_with("/guides/") {
        return Some("/guides/{slug}".to_string());
    }
    if trimmed.starts_with("/rerun-kit/") {
        return Some("/rerun-kit/{asset}".to_string());
    }
    Some("/unmatched".to_string())
}

/// Extract the host of a referrer URL: lower-case, no scheme, path, port or
/// credentials. Never the full referrer.
pub fn referrer_host(referer: &str) -> Option<String> {
    let without_scheme = match referer.split_once("://") {
        Some((_, rest)) => rest,
        // A colon without "://" marks a non-web scheme (javascript:, mailto:).
        None if referer.contains(':') => return None,
        None => referer,
    };
    let without_userinfo = match without_scheme.split_once('@') {
        Some((_, rest)) => rest,
        None => without_scheme,
    };
    let host_port = without_userinfo
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_userinfo);
    let host = host_port
        .split(':')
        .next()
        .unwrap_or(host_port)
        .to_lowercase();
    let valid = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        && !host.starts_with('.')
        && !host.ends_with('.');
    if valid {
        Some(host)
    } else {
        None
    }
}

/// Reduce a query string to allow-listed keys and the campaign code.
fn parse_query(query: &str) -> (Vec<String>, Option<String>) {
    let mut keys: Vec<String> = Vec::new();
    let mut campaign_code = None;
    for pair in query.split('&').take(32) {
        let (key, value) = match pair.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (pair, None),
        };
        let key_lower = key.to_lowercase();
        let bounded_key = if ALLOWED_QUERY_KEYS.contains(&key_lower.as_str()) {
            key_lower.clone()
        } else {
            "other".to_string()
        };
        if !keys.contains(&bounded_key) {
            keys.push(bounded_key);
        }
        if CAMPAIGN_KEYS.contains(&key_lower.as_str()) {
            if let Some(value) = value {
                if let Some(code) = sanitize_campaign_code(value) {
                    campaign_code = Some(code);
                }
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys.truncate(16);
    (keys, campaign_code)
}

/// Campaign codes are bounded to a safe character set and length.
fn sanitize_campaign_code(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > 64 {
        return None;
    }
    let decoded = percent_decode(value);
    let sanitized: String = decoded
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect();
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

/// Decode percent-escapes for the campaign code only (values never reach
/// any other field).
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(hex) = hex {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                    continue;
                }
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn defaults() -> AccessLogDefaults {
        AccessLogDefaults {
            environment: "production".to_string(),
            service_version: "apex-0.1.0".to_string(),
            received_at: UtcMillis::parse_rfc3339("2026-10-15T10:10:00.000Z").expect("time"),
        }
    }

    fn line(uri: &str, status: u16, user_agent: &str, referer: &str) -> String {
        serde_json::json!({
            "ts": 1_792_055_400.125_f64,
            "status": status,
            "request": {
                "method": "GET",
                "host": "apexmediation.ee",
                "uri": uri,
                "remote_ip": "203.0.113.9",
                "headers": {
                    "User-Agent": [user_agent],
                    "Referer": [referer]
                }
            }
        })
        .to_string()
    }

    #[test]
    fn a_page_request_becomes_a_bounded_row() {
        let record = line(
            "/pricing?utm_source=newsletter&utm_campaign=fall%20launch&token=SECRET",
            200,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari/605.1.15",
            "https://news.example.com/article?ref=1",
        );
        let row = parse_line(&record, &defaults())
            .expect("parse")
            .expect("a row");
        assert_eq!(row.route_template, "/pricing");
        assert_eq!(row.status, 200);
        assert_eq!(row.status_class, "2xx");
        assert_eq!(row.bot_class, "unclassified");
        assert_eq!(row.referrer_host.as_deref(), Some("news.example.com"));
        assert_eq!(row.campaign_code.as_deref(), Some("falllaunch"));
        assert_eq!(row.environment, "production");
        assert_eq!(
            row.occurred_at_utc,
            UtcMillis::from_unix_millis(1_792_055_400_125)
        );
        assert_eq!(row.received_at_utc, defaults().received_at);
        assert_eq!(row.event_id.len(), 32);
        assert!(row.query_keys.contains(&"utm_source".to_string()));
        assert!(row.query_keys.contains(&"other".to_string()));
        // Values are redacted: neither the token nor the campaign text is
        // anywhere in the serialized row or its storage shape.
        let json = serde_json::to_string(&row).expect("row json");
        assert!(!json.contains("SECRET"), "{json}");
        assert!(!json.contains("fall%20launch"), "{json}");
        assert!(!json.contains("203.0.113.9"), "{json}");
        assert!(!json.contains("Safari/605"), "{json}");
        let storage = row.to_storage_value();
        assert_eq!(storage["occurred_at_utc"], "2026-10-15 09:10:00.125");
        assert_eq!(storage["route_template"], "/pricing");
        assert_eq!(storage["status_class"], "2xx");
    }

    #[test]
    fn routes_are_bounded_templates() {
        assert_eq!(normalize_route("/").as_deref(), Some("/"));
        assert_eq!(normalize_route("/pricing").as_deref(), Some("/pricing"));
        assert_eq!(normalize_route("/pricing/").as_deref(), Some("/pricing"));
        assert_eq!(
            normalize_route("/guides/raise-ecpm").as_deref(),
            Some("/guides/{slug}")
        );
        assert_eq!(
            normalize_route("/rerun-kit/rerun.py").as_deref(),
            Some("/rerun-kit/{asset}")
        );
        assert_eq!(
            normalize_route("/../../etc/passwd").as_deref(),
            Some("/unmatched")
        );
        assert_eq!(normalize_route("not-a-path"), None);
        assert_eq!(normalize_route(&format!("/{}", "x".repeat(600))), None);
    }

    #[test]
    fn bots_are_classified_by_documented_tokens() {
        assert_eq!(
            BotClass::classify(Some("Mozilla/5.0 (compatible; Googlebot/2.1)")),
            BotClass::SearchEngineBot
        );
        assert_eq!(
            BotClass::classify(Some("python-requests/2.31.0")),
            BotClass::AutomationClient
        );
        assert_eq!(
            BotClass::classify(Some("curl/8.4.0")),
            BotClass::AutomationClient
        );
        assert_eq!(
            BotClass::classify(Some("facebookexternalhit/1.1")),
            BotClass::OtherBot
        );
        assert_eq!(
            BotClass::classify(Some("SomeUnknownAgent/1.0")),
            BotClass::Unclassified
        );
        assert_eq!(BotClass::classify(None), BotClass::Unclassified);
        assert_eq!(
            BotClass::classify(Some("Mozilla/5.0 Safari/605.1.15")),
            BotClass::Unclassified
        );
    }

    #[test]
    fn referrer_hosts_lose_scheme_path_port_and_credentials() {
        assert_eq!(
            referrer_host("https://news.example.com/article?ref=1").as_deref(),
            Some("news.example.com")
        );
        assert_eq!(
            referrer_host("http://user:pass@bad.example.com:8080/x").as_deref(),
            Some("bad.example.com")
        );
        assert_eq!(referrer_host("https://[::1]/").as_deref(), None);
        assert_eq!(referrer_host("javascript:alert(1)").as_deref(), None);
        assert_eq!(referrer_host("").as_deref(), None);
    }

    #[test]
    fn non_request_records_are_skipped_not_failed() {
        let record = serde_json::json!({"ts": 1.0, "level": "info", "msg": "log rotated"});
        let parsed = parse_line(&record.to_string(), &defaults()).expect("parse");
        assert!(parsed.is_none());
    }

    #[test]
    fn malformed_lines_are_rejected_with_reasons() {
        assert!(matches!(
            parse_line("not json", &defaults()),
            Err(AccessLogError::Json(_))
        ));
        let no_uri = serde_json::json!({"ts": 1.0, "status": 200, "request": {}});
        assert!(matches!(
            parse_line(&no_uri.to_string(), &defaults()),
            Err(AccessLogError::MissingUri)
        ));
        let bad_ts = serde_json::json!({
            "status": 200,
            "request": {"uri": "/"}
        });
        assert!(matches!(
            parse_line(&bad_ts.to_string(), &defaults()),
            Err(AccessLogError::BadTimestamp)
        ));
        let bad_path = serde_json::json!({
            "ts": 1.0, "status": 200,
            "request": {"uri": "no-slash"}
        });
        assert!(matches!(
            parse_line(&bad_path.to_string(), &defaults()),
            Err(AccessLogError::BadPath)
        ));
    }

    #[test]
    fn replays_of_one_line_derive_the_same_identity() {
        let record = line("/pricing", 200, "curl/8.4.0", "");
        let first = parse_line(&record, &defaults())
            .expect("parse")
            .expect("row");
        let second = parse_line(&record, &defaults())
            .expect("parse")
            .expect("row");
        assert_eq!(first.event_id, second.event_id);
        assert_eq!(first.event_id.len(), 32);
    }

    #[test]
    fn status_classes_cover_every_range() {
        assert_eq!(status_class(103), "1xx");
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(301), "3xx");
        assert_eq!(status_class(404), "4xx");
        assert_eq!(status_class(503), "5xx");
    }

    #[test]
    fn query_parsing_is_value_free_and_bounded() {
        let (keys, code) = parse_query("utm_source=a&utm_source=b&secret=x&campaign=spring_2026");
        assert_eq!(keys, vec!["campaign", "other", "utm_source"]);
        assert_eq!(code.as_deref(), Some("spring_2026"));
        // A hostile campaign value sanitizes to a harmless code.
        let (_, code) = parse_query("campaign=<script>alert(1)</script>");
        assert_eq!(code.as_deref(), Some("scriptalert1script"));
        // No campaign key means no code.
        let (_, code) = parse_query("utm_source=x");
        assert_eq!(code, None);
        // A very long query cannot inflate the key list.
        let long: Vec<String> = (0..64).map(|i| format!("k{i}=v")).collect();
        let (keys, _) = parse_query(&long.join("&"));
        assert!(keys.len() <= 16);
        assert_eq!(keys.len(), 1, "every unknown key collapses to 'other'");
    }
}
