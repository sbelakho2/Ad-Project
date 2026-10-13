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
/// APIs that mandate form encoding (Stripe's v1 rest endpoints).
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
    name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("idempotency-key")
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
        let resp = builder.send().await.map_err(|e| {
            if e.is_timeout() {
                TransportError::Timeout(e.to_string())
            } else {
                TransportError::Network(e.to_string())
            }
        })?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = resp
            .bytes()
            .await
            .map_err(|e| TransportError::Network(e.to_string()))?
            .to_vec();
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}
