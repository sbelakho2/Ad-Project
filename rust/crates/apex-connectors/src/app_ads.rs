//! app-ads.txt fetch and parse for the free-audit lead source.
//!
//! Fetches `https://<developer website>/app-ads.txt` as defined by the IAB spec
//! and returns the audit engine's [`AppAdsTxt`] input. Only https is accepted.
//! The audit runs per platform, so every call carries its [`PlatformId`] for
//! error attribution and for the caller's expected-network list.
//!
//! Request safety:
//!
//! - The host is validated textually before any request: no userinfo, ports,
//!   brackets, spaces or non hostname characters, and a reserved or numeric
//!   literal host is refused. A literal address such as `127.0.0.1` or a name
//!   such as `localhost` can never be reached.
//! - DNS rebinding is not decided here. Egress that must survive a hostile
//!   domain needs a resolve-time check at the network layer.
//! - Response bodies above [`MAX_APP_ADS_BYTES`] are refused before parsing so
//!   a hostile file cannot drive unbounded work.
//!
//! Status mapping: 404 and 410 mean "no file", 429 is rate limiting, and any
//! other non-200 is an HTTP error. A website 401 or 403 is deliberately not
//! classified as an auth failure: the site refusing our fetch says nothing
//! about the publisher's platform key, and callers must not translate it into
//! a credential error. Retrying composes outside exactly as it does for the
//! platform clients: pass a retry-wrapped transport when the caller wants
//! retries.

use std::sync::Arc;

use apex_audit::AppAdsTxt;
use apex_core::{PlatformError, PlatformId};

use crate::transport::{HttpMethod, HttpRequest, Transport, TransportError};

/// Fetch path, rooted at the developer website.
pub const APP_ADS_PATH: &str = "/app-ads.txt";

/// Parse cap for one app-ads.txt response.
pub const MAX_APP_ADS_BYTES: usize = 1024 * 1024;

/// Suffixes that never name a public developer website.
const RESERVED_SUFFIXES: [&str; 6] = [".local", ".internal", ".home", ".lan", ".test", ".invalid"];

/// Fetch and parse app-ads.txt for one platform audit.
pub struct AppAdsTxtFetcher {
    transport: Arc<dyn Transport>,
    timeout_ms: u64,
}

impl std::fmt::Debug for AppAdsTxtFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppAdsTxtFetcher")
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

impl AppAdsTxtFetcher {
    pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            timeout_ms: Self::DEFAULT_TIMEOUT_MS,
        }
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Fetch `app-ads.txt` from the publisher's developer website.
    ///
    /// A 404 or 410 yields `present: false` with empty lines, the audit rule's
    /// explicit "no file" input. Any other non-200 status is an error.
    pub async fn fetch(
        &self,
        platform: PlatformId,
        raw_domain: &str,
        expected_networks: Vec<String>,
    ) -> Result<AppAdsTxt, PlatformError> {
        let host = normalize_domain(raw_domain).map_err(|detail| {
            PlatformError::Validation(format!("{platform} fetchAppAdsTxt: {detail}"))
        })?;
        let request = HttpRequest {
            platform,
            operation: "fetchAppAdsTxt".to_string(),
            method: HttpMethod::Get,
            url: format!("https://{host}{APP_ADS_PATH}"),
            headers: vec![("Accept".into(), "text/plain".into())],
            body: None,
            timeout_ms: self.timeout_ms,
        };
        let response = match self.transport.request(request).await {
            Ok(response) => response,
            Err(TransportError::Network(detail)) => {
                return Err(PlatformError::Network {
                    platform,
                    operation: "fetchAppAdsTxt".to_string(),
                    detail,
                });
            }
            Err(TransportError::Timeout(detail)) => {
                return Err(PlatformError::Network {
                    platform,
                    operation: "fetchAppAdsTxt".to_string(),
                    detail: format!("timeout: {detail}"),
                });
            }
            Err(TransportError::Platform(error)) => return Err(error),
        };
        match response.status {
            200 => {
                if response.body.len() > MAX_APP_ADS_BYTES {
                    return Err(PlatformError::Schema {
                        platform,
                        operation: "fetchAppAdsTxt".to_string(),
                        detail: format!(
                            "response is {} bytes, above the {MAX_APP_ADS_BYTES} byte parse cap",
                            response.body.len()
                        ),
                    });
                }
                let text =
                    std::str::from_utf8(&response.body).map_err(|_| PlatformError::Schema {
                        platform,
                        operation: "fetchAppAdsTxt".to_string(),
                        detail: "response body is not valid UTF-8".to_string(),
                    })?;
                let text = text.strip_prefix('\u{feff}').unwrap_or(text);
                let lines = text.lines().map(str::to_string).collect();
                Ok(AppAdsTxt {
                    present: true,
                    lines,
                    expected_networks,
                })
            }
            404 | 410 => Ok(AppAdsTxt {
                present: false,
                lines: Vec::new(),
                expected_networks,
            }),
            429 => Err(PlatformError::RateLimited {
                platform,
                operation: "fetchAppAdsTxt".to_string(),
                retry_after: None,
            }),
            status => Err(PlatformError::Http {
                platform,
                operation: "fetchAppAdsTxt".to_string(),
                status,
            }),
        }
    }
}

/// Normalize caller input into the hostname to fetch.
///
/// Accepts `example.com`, `Example.COM`, `https://example.com/...` and
/// `example.com.`, and returns the lowercase hostname without a trailing dot.
/// Refuses anything that cannot be a public developer website: non-https
/// schemes, ports, userinfo, brackets, whitespace, malformed labels, reserved
/// suffixes and numeric literals.
pub fn normalize_domain(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("domain is empty".to_string());
    }
    let host_part = match trimmed.split_once("://") {
        Some((scheme, rest)) => {
            if !scheme.eq_ignore_ascii_case("https") {
                return Err(format!("scheme '{scheme}' is not https"));
            }
            rest
        }
        None => trimmed,
    };
    let host = host_part
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let host = host.trim_end_matches('.');
    if host.is_empty() {
        return Err("domain is empty".to_string());
    }
    if host.len() > 253 {
        return Err("domain is longer than 253 characters".to_string());
    }
    if host == "localhost" || RESERVED_SUFFIXES.iter().any(|s| host.ends_with(s)) {
        return Err(format!("host '{host}' is reserved for local use"));
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return Err(format!("host '{host}' has no public suffix"));
    }
    for label in &labels {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("host '{host}' has an empty or over-long label"));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(format!(
                "host '{host}' has a label with a leading or trailing '-'"
            ));
        }
        if !label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(format!(
                "host '{host}' has a character outside a-z, 0-9 and '-'"
            ));
        }
    }
    if labels
        .iter()
        .all(|label| label.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(format!("host '{host}' is a numeric address, not a website"));
    }
    Ok(host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HttpResponse;
    use std::sync::Mutex;

    struct ScriptedTransport {
        script: Mutex<std::collections::VecDeque<Result<HttpResponse, TransportError>>>,
        calls: Mutex<Vec<HttpRequest>>,
    }

    impl ScriptedTransport {
        fn new(items: Vec<Result<HttpResponse, TransportError>>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(items.into()),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> Vec<HttpRequest> {
            self.calls.lock().map(|c| c.clone()).unwrap_or_default()
        }
    }

    #[async_trait::async_trait]
    impl Transport for ScriptedTransport {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.calls
                .lock()
                .map_err(|_| TransportError::Network("poisoned".into()))?
                .push(req);
            self.script
                .lock()
                .map_err(|_| TransportError::Network("poisoned".into()))?
                .pop_front()
                .unwrap_or(Err(TransportError::Network("script exhausted".into())))
        }
    }

    fn ok(status: u16, body: &[u8]) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![],
            body: body.to_vec(),
        }
    }

    #[test]
    fn normalize_accepts_website_forms() {
        for raw in [
            "example.com",
            "Example.COM",
            "https://example.com",
            "https://example.com/path?q=1#frag",
            "example.com.",
            "sub.example.co.uk",
        ] {
            let got = normalize_domain(raw).expect(raw);
            assert!(
                got.starts_with("example") || got.starts_with("sub.example"),
                "{raw} -> {got}"
            );
            assert!(!got.ends_with('.'), "{raw} -> {got}");
            assert_eq!(got, got.to_lowercase(), "{raw} -> {got}");
        }
        assert_eq!(normalize_domain("Example.COM").unwrap(), "example.com");
        assert_eq!(
            normalize_domain("https://Sub.Example.com/x").unwrap(),
            "sub.example.com"
        );
    }

    #[test]
    fn normalize_refuses_unsafe_or_malformed_hosts() {
        for raw in [
            "",
            "http://example.com",
            "ftp://example.com",
            "example",
            "localhost",
            "127.0.0.1",
            "192.168.0.10",
            "example.local",
            "example.internal",
            "user@example.com",
            "example.com:8080",
            "[::1]",
            "-bad.example.com",
            "bad-.example.com",
            "bad..example.com",
            "exa mple.com",
            "exa_mple.com",
        ] {
            assert!(normalize_domain(raw).is_err(), "accepted {raw:?}");
        }
    }

    #[tokio::test]
    async fn fetch_returns_lines_and_requests_the_spec_url() {
        let transport = ScriptedTransport::new(vec![Ok(ok(
            200,
            b"\xef\xbb\xbfgoogle.com, pub-1, DIRECT\r\nexample.com,pub-2,RESELLER\n",
        ))]);
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let txt = fetcher
            .fetch(
                PlatformId::Admob,
                "Example.com",
                vec!["google.com".to_string(), "unityads.com".to_string()],
            )
            .await
            .expect("fetch");
        assert!(txt.present);
        assert_eq!(
            txt.lines,
            vec![
                "google.com, pub-1, DIRECT".to_string(),
                "example.com,pub-2,RESELLER".to_string()
            ]
        );
        assert_eq!(txt.expected_networks.len(), 2);
        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].url, "https://example.com/app-ads.txt");
        assert_eq!(calls[0].method, HttpMethod::Get);
        assert_eq!(calls[0].header("Accept"), Some("text/plain"));
        assert_eq!(calls[0].platform, PlatformId::Admob);
    }

    #[tokio::test]
    async fn missing_file_is_present_false() {
        for status in [404, 410] {
            let transport = ScriptedTransport::new(vec![Ok(ok(status, b""))]);
            let fetcher = AppAdsTxtFetcher::new(transport.clone());
            let txt = fetcher
                .fetch(
                    PlatformId::LevelPlay,
                    "example.com",
                    vec!["admob.com".to_string()],
                )
                .await
                .expect("fetch");
            assert!(!txt.present, "status {status}");
            assert!(txt.lines.is_empty(), "status {status}");
            assert_eq!(txt.expected_networks, vec!["admob.com".to_string()]);
        }
    }

    #[tokio::test]
    async fn statuses_map_to_the_shared_classification() {
        let cases = [
            // Website refusals stay HTTP errors: they are not credential auth.
            (401, "http"),
            (403, "http"),
            (429, "rate"),
            (500, "http"),
            (302, "http"),
        ];
        for (status, kind) in cases {
            let transport = ScriptedTransport::new(vec![Ok(ok(status, b""))]);
            let fetcher = AppAdsTxtFetcher::new(transport.clone());
            let err = fetcher
                .fetch(PlatformId::AppLovinMax, "example.com", vec![])
                .await
                .expect_err("must fail");
            match (kind, &err) {
                ("auth", PlatformError::Auth { status: got, .. }) => assert_eq!(*got, status),
                ("rate", PlatformError::RateLimited { .. }) => {}

                ("http", PlatformError::Http { status: got, .. }) => assert_eq!(*got, status),
                other => panic!("status {status} mapped to {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn non_utf8_and_oversized_bodies_are_refused() {
        let transport = ScriptedTransport::new(vec![Ok(ok(200, &[0xff, 0xfe, 0xfd]))]);
        let fetcher = AppAdsTxtFetcher::new(transport);
        assert!(matches!(
            fetcher
                .fetch(PlatformId::Admob, "example.com", vec![])
                .await,
            Err(PlatformError::Schema { .. })
        ));

        let big = vec![b'a'; MAX_APP_ADS_BYTES + 1];
        let transport = ScriptedTransport::new(vec![Ok(ok(200, &big))]);
        let fetcher = AppAdsTxtFetcher::new(transport);
        assert!(matches!(
            fetcher
                .fetch(PlatformId::Admob, "example.com", vec![])
                .await,
            Err(PlatformError::Schema { .. })
        ));
    }

    #[tokio::test]
    async fn transport_failures_are_reported_as_network_errors() {
        let transport =
            ScriptedTransport::new(vec![Err(TransportError::Network("dns failed".into()))]);
        let fetcher = AppAdsTxtFetcher::new(transport);
        assert!(matches!(
            fetcher
                .fetch(PlatformId::Admob, "example.com", vec![])
                .await,
            Err(PlatformError::Network { .. })
        ));
    }
}
