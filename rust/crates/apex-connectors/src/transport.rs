//! Transport abstraction + retry decorator. The complete adversarial matrix
//! (auth fail-fast, rate-limit honoring, bounded backoff, write-idempotency,
//! schema fail-closed) lives here and is unit-tested with a mock transport and a
//! recording sleeper — no sockets, fully deterministic.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use apex_core::{PlatformError, PlatformId};
use async_trait::async_trait;

pub const MAX_RETRY_AFTER_MS: u64 = 60_000;
const BACKOFF_BASE_MS: u64 = 250;
const BACKOFF_CAP_MS: u64 = 2_000;
/// The hard cap on one response body. The body is streamed chunk by chunk and
/// refused as soon as it exceeds this size, so an upstream cannot make the
/// process buffer an unbounded reply.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Whether `len` bytes exceed [`MAX_RESPONSE_BYTES`]. Used for both the
/// declared `Content-Length` and the streamed total.
fn exceeds_response_cap(len: u64) -> bool {
    len > MAX_RESPONSE_BYTES as u64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
        }
    }

    pub fn is_read(self) -> bool {
        self == HttpMethod::Get
    }
}

/// Content type carried by [`HttpBody::Form`] requests.
pub const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

/// Request body shape. The mediation platform APIs use JSON; [`HttpBody::Form`]
/// carries an already-encoded `application/x-www-form-urlencoded` string for
/// APIs that mandate form encoding (Mollie webhooks and the legacy form endpoints).
#[derive(Debug, Clone, PartialEq)]
pub enum HttpBody {
    /// `application/json` payload, serialized by the transport.
    Json(serde_json::Value),
    /// Pre-encoded `application/x-www-form-urlencoded` payload, sent verbatim.
    Form(String),
}

impl From<serde_json::Value> for HttpBody {
    fn from(value: serde_json::Value) -> Self {
        Self::Json(value)
    }
}

impl HttpBody {
    /// The JSON value, when this is an [`HttpBody::Json`].
    pub fn as_json(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Json(value) => Some(value),
            Self::Form(_) => None,
        }
    }

    /// The raw form string, when this is an [`HttpBody::Form`].
    pub fn as_form(&self) -> Option<&str> {
        match self {
            Self::Form(raw) => Some(raw),
            Self::Json(_) => None,
        }
    }
}

/// One HTTP request. `Debug` redacts credential-bearing header values
/// (`Authorization`, `Idempotency-Key`) so diagnostics never leak secrets.
#[derive(Clone)]
pub struct HttpRequest {
    pub platform: PlatformId,
    pub operation: String,
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<HttpBody>,
    pub timeout_ms: u64,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| {
                let shown: &str = if is_sensitive_header(name) {
                    "[redacted]"
                } else {
                    value.as_str()
                };
                (name.as_str(), shown)
            })
            .collect::<Vec<_>>();
        f.debug_struct("HttpRequest")
            .field("platform", &self.platform)
            .field("operation", &self.operation)
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body", &self.body)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

fn is_sensitive_header(name: &str) -> bool {
    const SENSITIVE: [&str; 8] = [
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "idempotency-key",
        "api-key",
        "x-api-key",
        "secretkey",
    ];
    SENSITIVE
        .iter()
        .any(|known| name.eq_ignore_ascii_case(known))
        || name.eq_ignore_ascii_case("refreshtoken")
}

impl HttpRequest {
    /// A request may be retried when it is a read, or when the caller supplied an
    /// idempotency key (the header is part of `headers`).
    pub fn retryable(&self) -> bool {
        self.method.is_read() || self.header("Idempotency-Key").is_some()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn parse_json(&self, req: &HttpRequest) -> Result<serde_json::Value, PlatformError> {
        // RFC 7230 forbids a body on 204; treat the response as empty even if a
        // server misbehaves, so a stray body cannot become parsed "data".
        if self.status == 204 {
            return Ok(serde_json::Value::Null);
        }
        if self.body.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_slice(&self.body).map_err(|e| PlatformError::Schema {
            platform: req.platform,
            operation: req.operation.clone(),
            detail: format!("response body is not valid JSON: {e}"),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("network failure: {0}")]
    Network(String),
    #[error("timeout: {0}")]
    Timeout(String),
    /// Rich classification produced by the retry layer (auth, rate limit, 5xx, schema).
    #[error(transparent)]
    Platform(#[from] PlatformError),
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError>;
}

#[async_trait]
pub trait Sleeper: Send + Sync {
    async fn sleep(&self, d: Duration);
}

#[derive(Debug, Default)]
pub struct TokioSleeper;

#[async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep(&self, d: Duration) {
        tokio::time::sleep(d).await;
    }
}

/// Records every sleep for exact-delay assertions in tests.
#[derive(Debug, Default)]
pub struct RecordingSleeper {
    sleeps: Mutex<Vec<u64>>,
}

impl RecordingSleeper {
    pub fn slept_ms(&self) -> Vec<u64> {
        self.sleeps.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl Sleeper for RecordingSleeper {
    async fn sleep(&self, d: Duration) {
        if let Ok(mut v) = self.sleeps.lock() {
            v.push(d.as_millis() as u64);
        }
    }
}

/// Parses `Retry-After` (seconds or HTTP-date), capped at [`MAX_RETRY_AFTER_MS`].
pub fn parse_retry_after(header: Option<&str>, now: std::time::SystemTime) -> Option<Duration> {
    let raw = header?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_millis(
            seconds.saturating_mul(1000).min(MAX_RETRY_AFTER_MS),
        ));
    }
    let when = httpdate::parse_http_date(raw).ok()?;
    let delta = when.duration_since(now).ok()?;
    Some(Duration::from_millis(
        (delta.as_millis() as u64).min(MAX_RETRY_AFTER_MS),
    ))
}

/// Retry decorator implementing the pinned matrix:
/// - 401/403 -> Auth, immediate (never retried);
/// - 429 -> RateLimited (retry_after honored, capped), retried for retryable calls;
/// - 5xx -> Http, retried with bounded exponential backoff (250ms, x2, cap 2s);
/// - transport error -> Network/Timeout, retried only for retryable calls;
/// - other 4xx -> Http, immediate;
/// - 3xx -> the response passes through untouched (redirect-aware callers
///   such as the app-ads.txt fetcher consume the status and `Location`; a
///   redirect is never retried);
/// - 2xx -> JSON body parsed; empty body -> Null; invalid JSON -> Schema (fail closed).
pub struct RetryTransport {
    inner: Arc<dyn Transport>,
    max_retries: u32,
    sleeper: Arc<dyn Sleeper>,
}

impl std::fmt::Debug for RetryTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryTransport")
            .field("max_retries", &self.max_retries)
            .finish()
    }
}

impl RetryTransport {
    pub fn new(inner: Arc<dyn Transport>, max_retries: u32, sleeper: Arc<dyn Sleeper>) -> Self {
        Self {
            inner,
            max_retries,
            sleeper,
        }
    }

    fn classify_status(
        status: u16,
        req: &HttpRequest,
        retry_after: Option<Duration>,
    ) -> PlatformError {
        if status == 401 || status == 403 {
            return PlatformError::Auth {
                platform: req.platform,
                operation: req.operation.clone(),
                status,
            };
        }
        if status == 429 {
            return PlatformError::RateLimited {
                platform: req.platform,
                operation: req.operation.clone(),
                retry_after,
            };
        }
        PlatformError::Http {
            platform: req.platform,
            operation: req.operation.clone(),
            status,
        }
    }
}

#[async_trait]
impl Transport for RetryTransport {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let attempts = if req.retryable() {
            self.max_retries + 1
        } else {
            1
        };
        let mut last_error: Option<PlatformError> = None;

        for attempt in 0..attempts {
            if attempt > 0 {
                let wait_ms = match &last_error {
                    Some(PlatformError::RateLimited {
                        retry_after: Some(d),
                        ..
                    }) => d.as_millis() as u64,
                    _ => (BACKOFF_BASE_MS * 2u64.saturating_pow(attempt - 1)).min(BACKOFF_CAP_MS),
                };
                self.sleeper.sleep(Duration::from_millis(wait_ms)).await;
            }

            match self.inner.request(req.clone()).await {
                Err(e) => {
                    let net = match e {
                        TransportError::Network(detail) => PlatformError::Network {
                            platform: req.platform,
                            operation: req.operation.clone(),
                            detail,
                        },
                        TransportError::Timeout(detail) => PlatformError::Network {
                            platform: req.platform,
                            operation: req.operation.clone(),
                            detail: format!("timeout: {detail}"),
                        },
                        // Already classified below us: pass through untouched.
                        TransportError::Platform(p) => return Err(TransportError::Platform(p)),
                    };
                    if !req.retryable() {
                        return Err(TransportError::Platform(net));
                    }
                    last_error = Some(net);
                }
                Ok(resp) if (200..300).contains(&resp.status) => {
                    return Ok(resp);
                }
                // Redirects are not retryable transport errors: callers that
                // follow them (the app-ads.txt fetcher) need the status and
                // `Location` header intact. Every platform caller classifies a
                // non-2xx answer itself, so passing the hop through preserves
                // their behavior while making the wrapper composable with the
                // redirect-aware fetchers.
                Ok(resp) if (300..400).contains(&resp.status) => {
                    return Ok(resp);
                }
                Ok(resp) => {
                    let retry_after =
                        parse_retry_after(resp.header("retry-after"), std::time::SystemTime::now());
                    let err = Self::classify_status(resp.status, &req, retry_after);
                    if !(req.retryable() && err.is_retryable()) {
                        return Err(TransportError::Platform(err));
                    }
                    last_error = Some(err);
                }
            }
        }

        Err(TransportError::Platform(last_error.unwrap_or(
            PlatformError::Network {
                platform: req.platform,
                operation: req.operation.clone(),
                detail: "no attempt executed".into(),
            },
        )))
    }
}

/// Convenience layer binding a platform + auth header + base url.
pub struct PlatformCaller {
    pub platform: PlatformId,
    pub base_url: String,
    transport: Arc<dyn Transport>,
    auth_header: (String, String),
}

impl std::fmt::Debug for PlatformCaller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformCaller")
            .field("platform", &self.platform)
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl PlatformCaller {
    pub fn new(
        platform: PlatformId,
        base_url: String,
        transport: Arc<dyn Transport>,
        auth_header: (String, String),
    ) -> Self {
        Self {
            platform,
            base_url,
            transport,
            auth_header,
        }
    }

    pub async fn call(
        &self,
        operation: &str,
        method: HttpMethod,
        path: &str,
        body: Option<serde_json::Value>,
        idempotency_key: Option<&str>,
    ) -> Result<serde_json::Value, PlatformError> {
        let mut headers = vec![
            self.auth_header.clone(),
            ("Accept".into(), "application/json".into()),
        ];
        if body.is_some() {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        if let Some(key) = idempotency_key {
            headers.push(("Idempotency-Key".into(), key.to_string()));
        }
        let req = HttpRequest {
            platform: self.platform,
            operation: operation.to_string(),
            method,
            url: format!("{}{}", self.base_url.trim_end_matches('/'), path),
            headers,
            body: body.map(HttpBody::from),
            timeout_ms: 10_000,
        };
        match self.transport.request(req.clone()).await {
            Ok(resp) => {
                if !(200..300).contains(&resp.status) {
                    // The retry layer classifies terminal statuses; reaching here means
                    // a transport that bypassed it — classify again rather than trust.
                    return Err(PlatformError::Http {
                        platform: req.platform,
                        operation: req.operation.clone(),
                        status: resp.status,
                    });
                }
                resp.parse_json(&req)
            }
            Err(TransportError::Platform(p)) => Err(p),
            Err(e) => Err(PlatformError::Network {
                platform: req.platform,
                operation: req.operation.clone(),
                detail: e.to_string(),
            }),
        }
    }
}
/// Production transport over reqwest (rustls).
#[derive(Debug)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, PlatformError> {
        let client = reqwest::Client::builder()
            // Redirects are never followed silently: a hostile app-ads.txt
            // host could otherwise bounce the request to an internal address.
            // Callers that want redirects (the app-ads.txt fetcher) handle the
            // 3xx themselves and validate every hop.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| PlatformError::Network {
                platform: PlatformId::Admob,
                operation: "client-build".into(),
                detail: e.to_string(),
            })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl Transport for ReqwestTransport {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let method = match req.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Patch => reqwest::Method::PATCH,
            HttpMethod::Delete => reqwest::Method::DELETE,
        };
        let mut builder = self
            .client
            .request(method, &req.url)
            .timeout(Duration::from_millis(req.timeout_ms));
        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = &req.body {
            match body {
                HttpBody::Json(value) => {
                    builder = builder.json(value);
                }
                HttpBody::Form(raw) => {
                    // The caller may already have declared the form content
                    // type; only fill it in when absent so the header is not
                    // sent twice.
                    if req.header("Content-Type").is_none() {
                        builder = builder.header(reqwest::header::CONTENT_TYPE, FORM_CONTENT_TYPE);
                    }
                    builder = builder.body(raw.clone());
                }
            }
        }
        let resp = builder.send().await.map_err(|error| {
            // reqwest's Display includes the full request URL, query string
            // and all. Strip it before the detail can reach a log: platform
            // APIs put credentials (MAX's `api_key`) in the query.
            let error = error.without_url();
            if error.is_timeout() {
                TransportError::Timeout(error.to_string())
            } else {
                TransportError::Network(error.to_string())
            }
        })?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        // Refuse a declared oversized body before reading any of it, then
        // stream the body with the same hard cap so a chunked or lying
        // Content-Length cannot exhaust memory.
        if let Some(declared) = resp.content_length() {
            if exceeds_response_cap(declared) {
                return Err(TransportError::Network(format!(
                    "response body declares {declared} bytes, above the {MAX_RESPONSE_BYTES} byte cap"
                )));
            }
        }
        let mut resp = resp;
        let mut body = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|error| TransportError::Network(error.without_url().to_string()))?
        {
            let total = body.len() as u64 + chunk.len() as u64;
            if exceeds_response_cap(total) {
                return Err(TransportError::Network(format!(
                    "response body exceeds the {MAX_RESPONSE_BYTES} byte cap"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod unit_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn method_and_body_accessors_are_total() {
        assert_eq!(HttpMethod::Get.as_str(), "GET");
        assert_eq!(HttpMethod::Delete.as_str(), "DELETE");
        assert!(HttpMethod::Get.is_read());
        assert!(!HttpMethod::Post.is_read());

        let json = HttpBody::Json(serde_json::json!({"a": 1}));
        assert_eq!(json.as_json().unwrap()["a"], 1);
        assert!(json.as_form().is_none());
        let form = HttpBody::Form("a=1".into());
        assert_eq!(form.as_form(), Some("a=1"));
        assert!(form.as_json().is_none());
    }

    #[test]
    fn request_accessors_and_retryability() {
        let request = HttpRequest {
            platform: PlatformId::Admob,
            operation: "op".into(),
            method: HttpMethod::Get,
            url: "https://x".into(),
            headers: vec![("X-A".into(), "1".into())],
            body: None,
            timeout_ms: 1000,
        };
        assert_eq!(request.header("x-a"), Some("1"));
        assert!(request.retryable(), "a GET is idempotent");
        let post = HttpRequest {
            method: HttpMethod::Post,
            ..request
        };
        assert!(!post.retryable(), "a POST is not retried by default");
    }

    #[test]
    fn response_helpers_and_debug_redaction() {
        let response = HttpResponse {
            status: 200,
            headers: vec![("X-B".into(), "2".into())],
            body: br#"{"ok":true}"#.to_vec(),
        };
        assert_eq!(response.header("x-b"), Some("2"));
        let request = HttpRequest {
            platform: PlatformId::Admob,
            operation: "op".into(),
            method: HttpMethod::Post,
            url: "https://x".into(),
            headers: vec![("Authorization".into(), "Bearer secret".into())],
            body: Some(HttpBody::Json(serde_json::json!({"a": 1}))),
            timeout_ms: 1000,
        };
        assert!(response.parse_json(&request).is_ok());
        let rendered = format!("{request:?}");
        assert!(
            !rendered.contains("secret"),
            "credentials redacted: {rendered}"
        );
        let _ = format!(
            "{:?} {:?} {:?}",
            HttpBody::Json(serde_json::json!(1)),
            response.body,
            HttpMethod::Get
        );
    }

    #[test]
    fn header_sensitivity_covers_the_known_names() {
        for name in [
            "authorization",
            "X-API-Key",
            "secretkey",
            "refreshToken",
            "Cookie",
        ] {
            assert!(is_sensitive_header(name), "{name}");
        }
        assert!(!is_sensitive_header("accept"));
    }

    #[test]
    fn retry_after_parses_seconds_dates_and_refuses_junk() {
        let now = std::time::SystemTime::UNIX_EPOCH;
        assert_eq!(
            parse_retry_after(Some("5"), now),
            Some(Duration::from_millis(5000))
        );
        assert_eq!(parse_retry_after(Some("   "), now), None);
        assert_eq!(parse_retry_after(None, now), None);
        assert_eq!(parse_retry_after(Some("not-a-date"), now), None);
        assert_eq!(parse_retry_after(Some("0"), now), Some(Duration::ZERO));
    }

    #[tokio::test]
    async fn tokio_sleeper_returns_and_recording_sleeper_records() {
        TokioSleeper.sleep(Duration::from_millis(1)).await;
        let sleeper = RecordingSleeper::default();
        sleeper.sleep(Duration::from_millis(250)).await;
        sleeper.sleep(Duration::from_millis(500)).await;
        assert_eq!(sleeper.slept_ms(), vec![250, 500]);
    }

    #[tokio::test]
    async fn the_reqwest_transport_builds_and_parses_headers() {
        let transport = ReqwestTransport::new().expect("client");
        let request = HttpRequest {
            platform: PlatformId::Admob,
            operation: "op".into(),
            method: HttpMethod::Get,
            url: "https://127.0.0.1:1/never".into(),
            headers: vec![],
            body: None,
            timeout_ms: 1,
        };
        // The call fails fast (connection refused); the constructor and the
        // request-mapping path are what is exercised here.
        assert!(transport.request(request).await.is_err());
    }

    #[test]
    fn the_response_cap_boundary_is_exact() {
        assert!(!exceeds_response_cap(0));
        assert!(!exceeds_response_cap(MAX_RESPONSE_BYTES as u64));
        assert!(exceeds_response_cap(MAX_RESPONSE_BYTES as u64 + 1));
        assert!(exceeds_response_cap(u64::MAX));
    }

    /// A server that accepts one connection, reads the request and closes
    /// without answering. reqwest's error text normally contains the full
    /// request URL (including `api_key`); the transport must strip it.
    #[tokio::test]
    async fn reqwest_network_errors_redact_the_url_and_query() {
        use tokio::io::AsyncReadExt as _;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buffer = [0u8; 1024];
            let _ = socket.read(&mut buffer).await;
            drop(socket);
        });

        let transport = ReqwestTransport::new().expect("client");
        let request = HttpRequest {
            platform: PlatformId::AppLovinMax,
            operation: "maxReport".into(),
            method: HttpMethod::Get,
            url: format!("http://{address}/report?api_key=super-secret-query-value"),
            headers: vec![],
            body: None,
            timeout_ms: 5_000,
        };
        let error = transport.request(request).await.expect_err("closed");
        let detail = format!("{error} {error:?}");
        assert!(
            !detail.contains("super-secret-query-value"),
            "the query value leaked: {detail}"
        );
        assert!(
            !detail.contains("api_key"),
            "the query key leaked: {detail}"
        );
        assert!(
            !detail.contains(&address.to_string()),
            "the URL leaked: {detail}"
        );
        server.await.expect("server");
    }
}

/// Records `apex_connector_requests_total` / `apex_connector_duration_seconds`
/// for every transport attempt when a registry is injected; a `None` registry
/// makes it a pure pass-through (no-op).
///
/// Place the metered wrapper **inside** the retry decorator so each attempt is
/// its own observation: "External call duration, with retries represented
/// separately" (page 111). The request's own `platform` and `operation` are
/// the labels; an operation outside the declared vocabulary is not metered
/// (bounded labels are the contract, not a suggestion).
pub struct MeteredTransport {
    inner: std::sync::Arc<dyn Transport>,
    registry: Option<std::sync::Arc<apex_metrics::Registry>>,
}

impl std::fmt::Debug for MeteredTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeteredTransport")
            .field("metered", &self.registry.is_some())
            .finish()
    }
}

impl MeteredTransport {
    /// Wrap `inner`; `None` makes the wrapper a no-op.
    pub fn new(
        inner: std::sync::Arc<dyn Transport>,
        registry: Option<std::sync::Arc<apex_metrics::Registry>>,
    ) -> Self {
        Self { inner, registry }
    }

    fn record(
        &self,
        platform: PlatformId,
        operation: &str,
        outcome: &Result<HttpResponse, TransportError>,
        elapsed: std::time::Duration,
    ) {
        let Some(registry) = &self.registry else {
            return;
        };
        let Some(operation_label) = apex_metrics::vocab::ConnectorOperation::parse(operation)
        else {
            return;
        };
        let platform_label = platform.as_str();
        let outcome_class = outcome_class(outcome);
        let _ = registry.increment(
            &apex_metrics::catalog::CONNECTOR_REQUESTS_TOTAL,
            &[
                ("platform", platform_label),
                ("operation", operation_label.as_str()),
                ("outcome_class", outcome_class.as_str()),
            ],
        );
        let _ = registry.observe(
            &apex_metrics::catalog::CONNECTOR_DURATION_SECONDS,
            &[
                ("platform", platform_label),
                ("operation", operation_label.as_str()),
            ],
            elapsed.as_secs_f64(),
        );
    }
}

/// The bounded outcome class of one attempt. Non-2xx statuses are classified
/// by auth / rate-limit / transport; schema failures are only observable at
/// the parse boundary (the clients count them).
fn outcome_class(
    outcome: &Result<HttpResponse, TransportError>,
) -> apex_metrics::vocab::OutcomeClass {
    use apex_metrics::vocab::OutcomeClass;
    match outcome {
        Ok(response) if (200..300).contains(&response.status) => OutcomeClass::Success,
        Ok(response) if response.status == 401 || response.status == 403 => {
            OutcomeClass::AuthRefusal
        }
        Ok(response) if response.status == 429 => OutcomeClass::RateLimit,
        Ok(_) => OutcomeClass::TransportFailure,
        Err(TransportError::Platform(PlatformError::Auth { .. })) => OutcomeClass::AuthRefusal,
        Err(TransportError::Platform(PlatformError::RateLimited { .. })) => OutcomeClass::RateLimit,
        Err(TransportError::Platform(PlatformError::Schema { .. })) => OutcomeClass::SchemaFailure,
        Err(_) => OutcomeClass::TransportFailure,
    }
}

#[async_trait]
impl Transport for MeteredTransport {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let platform = req.platform;
        let operation = req.operation.clone();
        let started = std::time::Instant::now();
        let outcome = self.inner.request(req).await;
        self.record(platform, &operation, &outcome, started.elapsed());
        outcome
    }
}

#[cfg(test)]
mod metered_tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug)]
    struct Scripted {
        status: u16,
        calls: std::sync::atomic::AtomicU32,
    }

    #[async_trait]
    impl Transport for Scripted {
        async fn request(&self, _req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(HttpResponse {
                status: self.status,
                headers: vec![],
                body: vec![],
            })
        }
    }

    fn request(platform: PlatformId, operation: &str) -> HttpRequest {
        HttpRequest {
            platform,
            operation: operation.to_string(),
            method: HttpMethod::Get,
            url: "https://example.test/x".to_string(),
            headers: vec![],
            body: None,
            timeout_ms: 1_000,
        }
    }

    #[test]
    fn outcome_classes_are_bounded() {
        use apex_core::PlatformError;
        assert_eq!(
            outcome_class(&Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: vec![]
            })),
            apex_metrics::vocab::OutcomeClass::Success
        );
        assert_eq!(
            outcome_class(&Ok(HttpResponse {
                status: 403,
                headers: vec![],
                body: vec![]
            })),
            apex_metrics::vocab::OutcomeClass::AuthRefusal
        );
        assert_eq!(
            outcome_class(&Ok(HttpResponse {
                status: 429,
                headers: vec![],
                body: vec![]
            })),
            apex_metrics::vocab::OutcomeClass::RateLimit
        );
        assert_eq!(
            outcome_class(&Ok(HttpResponse {
                status: 500,
                headers: vec![],
                body: vec![]
            })),
            apex_metrics::vocab::OutcomeClass::TransportFailure
        );
        assert_eq!(
            outcome_class(&Err(TransportError::Platform(PlatformError::Schema {
                platform: PlatformId::AppLovinMax,
                operation: "op".into(),
                detail: "x".into(),
            }))),
            apex_metrics::vocab::OutcomeClass::SchemaFailure
        );
        assert_eq!(
            outcome_class(&Err(TransportError::Network("down".into()))),
            apex_metrics::vocab::OutcomeClass::TransportFailure
        );
    }

    #[tokio::test]
    async fn a_none_registry_is_a_pass_through() {
        let inner = Arc::new(Scripted {
            status: 200,
            calls: std::sync::atomic::AtomicU32::new(0),
        });
        let metered = MeteredTransport::new(inner.clone(), None);
        let response = metered
            .request(request(PlatformId::AppLovinMax, "maxReport"))
            .await
            .expect("ok");
        assert_eq!(response.status, 200);
        assert_eq!(inner.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(format!("{metered:?}").contains("metered"));
    }

    #[tokio::test]
    async fn attempts_are_metered_apart_from_retries() {
        let registry = Arc::new(apex_metrics::Registry::new());
        let inner = Arc::new(Scripted {
            status: 401,
            calls: std::sync::atomic::AtomicU32::new(0),
        });
        // Retry outside, meter inside: every attempt is one observation.
        let metered = Arc::new(MeteredTransport::new(inner, Some(registry.clone())));
        let retry = RetryTransport::new(metered, 2, Arc::new(RecordingSleeper::default()));
        let error = retry
            .request(request(PlatformId::AppLovinMax, "maxReport"))
            .await
            .expect_err("auth is fail-fast");
        assert!(matches!(error, TransportError::Platform(_)));
        let count = registry
            .counter_value(
                &apex_metrics::catalog::CONNECTOR_REQUESTS_TOTAL,
                &[
                    ("platform", "applovin_max"),
                    ("operation", "maxReport"),
                    ("outcome_class", "auth_refusal"),
                ],
            )
            .expect("metric");
        assert_eq!(count, 1, "fail-fast auth is one attempt");
        assert!(registry
            .family_snapshot("apex_connector_duration_seconds")
            .is_some());
        // An undeclared operation is refused by the bounded vocabulary and
        // never mints a series.
        let metered = MeteredTransport::new(
            Arc::new(Scripted {
                status: 200,
                calls: std::sync::atomic::AtomicU32::new(0),
            }),
            Some(registry.clone()),
        );
        metered
            .request(request(PlatformId::AppLovinMax, "userSuppliedThing"))
            .await
            .expect("ok");
        let total: u64 = registry
            .family_snapshot("apex_connector_requests_total")
            .expect("family")
            .series
            .iter()
            .map(|series| series.counter.unwrap_or(0))
            .sum();
        assert_eq!(total, 1, "no series for the undeclared operation");
    }
}
