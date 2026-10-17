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
//! - Redirects are never followed by the transport; the fetcher handles each
//!   3xx itself, validating every hop's scheme and host with the same rules,
//!   and refuses a chain longer than [`MAX_REDIRECTS`].
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use apex_audit::AppAdsTxt;
use apex_core::{PlatformError, PlatformId};

use crate::transport::{HttpMethod, HttpRequest, HttpResponse, Transport, TransportError};

/// Fetch path, rooted at the developer website.
pub const APP_ADS_PATH: &str = "/app-ads.txt";

/// Parse cap for one app-ads.txt response.
pub const MAX_APP_ADS_BYTES: usize = 1024 * 1024;

/// Redirect cap: every hop is validated, and a chain longer than this is
/// refused rather than followed.
pub const MAX_REDIRECTS: usize = 5;

/// Suffixes that never name a public developer website.
const RESERVED_SUFFIXES: [&str; 6] = [".local", ".internal", ".home", ".lan", ".test", ".invalid"];

/// The operation label every app-ads.txt fetch carries; the cache below only
/// intercepts requests with this label.
const APP_ADS_OPERATION: &str = "fetchAppAdsTxt";

/// One cached app-ads.txt transport answer. Both shapes are what the fetcher
/// would have seen without the cache, so replays map to the same result.
#[derive(Debug, Clone)]
enum CachedAppAds {
    /// The transport answered (2xx, a redirect hop, or 404/410).
    Response(HttpResponse),
    /// A wrapping transport classified 404/410 as an HTTP error.
    Absent(u16),
}

impl CachedAppAds {
    fn into_result(self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        match self {
            Self::Response(response) => Ok(response),
            Self::Absent(status) => Err(TransportError::Platform(PlatformError::Http {
                platform: request.platform,
                operation: request.operation.clone(),
                status,
            })),
        }
    }
}

/// A per-run cache of `app-ads.txt` fetches, keyed by the exact URL.
///
/// The fleet audit asks every platform for the same publisher's developer
/// site. That is one public file, so it is fetched once per distinct URL per
/// run and replayed to the other platform builders, including the 404/410
/// "no file" verdict. Only app-ads.txt operations are intercepted; every
/// other request passes straight through. A 5xx or transport failure is never
/// cached, so a transient failure is not replayed as a verdict.
#[derive(Debug, Default)]
pub struct AppAdsCache {
    entries: Mutex<HashMap<String, CachedAppAds>>,
}

impl AppAdsCache {
    /// An empty cache for one audit run.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap `inner` so app-ads.txt reads are served from this cache.
    #[must_use]
    pub fn wrap(self: &Arc<Self>, inner: Arc<dyn Transport>) -> Arc<dyn Transport> {
        Arc::new(CachedAppAdsTransport {
            inner,
            cache: Arc::clone(self),
        })
    }

    fn get(&self, url: &str) -> Option<CachedAppAds> {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(url).cloned())
    }

    fn insert(&self, url: String, answer: CachedAppAds) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(url, answer);
        }
    }
}

/// The transport decorator behind [`AppAdsCache::wrap`].
struct CachedAppAdsTransport {
    inner: Arc<dyn Transport>,
    cache: Arc<AppAdsCache>,
}

impl std::fmt::Debug for CachedAppAdsTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CachedAppAdsTransport")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl Transport for CachedAppAdsTransport {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        if request.operation != APP_ADS_OPERATION {
            return self.inner.request(request).await;
        }
        if let Some(answer) = self.cache.get(&request.url) {
            return answer.into_result(&request);
        }
        let result = self.inner.request(request.clone()).await;
        let cacheable = match &result {
            Ok(response) if (200..400).contains(&response.status) => {
                Some(CachedAppAds::Response(response.clone()))
            }
            Ok(response) if matches!(response.status, 404 | 410) => {
                Some(CachedAppAds::Absent(response.status))
            }
            Err(TransportError::Platform(PlatformError::Http { status, .. }))
                if matches!(status, 404 | 410) =>
            {
                Some(CachedAppAds::Absent(*status))
            }
            _ => None,
        };
        if let Some(answer) = cacheable {
            self.cache.insert(request.url, answer);
        }
        result
    }
}

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
        let mut current = format!("https://{host}{APP_ADS_PATH}");
        let mut hops = 0usize;
        let response = loop {
            let request = HttpRequest {
                platform,
                operation: "fetchAppAdsTxt".to_string(),
                method: HttpMethod::Get,
                url: current.clone(),
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
                // A wrapping transport (RetryTransport) converts any
                // non-2xx answer into PlatformError::Http before this
                // fetcher sees the response. A missing file is a legitimate
                // publisher state, not a failed audit: 404/410 mean absent,
                // exactly as the response arm below documents.
                Err(TransportError::Platform(PlatformError::Http {
                    status: 404 | 410, ..
                })) => {
                    return Ok(AppAdsTxt {
                        present: false,
                        lines: Vec::new(),
                        expected_networks,
                    });
                }
                Err(TransportError::Platform(error)) => return Err(error),
            };
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                hops += 1;
                if hops > MAX_REDIRECTS {
                    return Err(PlatformError::Schema {
                        platform,
                        operation: "fetchAppAdsTxt".to_string(),
                        detail: format!("redirect chain exceeds {MAX_REDIRECTS} hops"),
                    });
                }
                let location =
                    response
                        .header("location")
                        .ok_or_else(|| PlatformError::Schema {
                            platform,
                            operation: "fetchAppAdsTxt".to_string(),
                            detail: "redirect without a Location header".to_string(),
                        })?;
                current = resolve_redirect(&current, location).map_err(|detail| {
                    PlatformError::Validation(format!("{platform} fetchAppAdsTxt: {detail}"))
                })?;
                continue;
            }
            break response;
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

/// Resolve one redirect hop against the request url, refusing any hop that
/// leaves https or targets a non-public host. Root-relative and same-host
/// locations keep the current host; absolute and protocol-relative locations
/// must pass [`normalize_domain`] so a redirect can never reach an internal
/// address.
fn resolve_redirect(current_url: &str, location: &str) -> Result<String, String> {
    let location = location.trim();
    if location.is_empty() {
        return Err("redirect has an empty Location".to_string());
    }
    let current_host = host_of(current_url);
    if let Some(rest) = location.strip_prefix("//") {
        let (host, path) = split_host(rest);
        let host = normalize_domain(host)?;
        return Ok(format!(
            "https://{host}{}",
            if path.is_empty() { APP_ADS_PATH } else { path }
        ));
    }
    if let Some((scheme, rest)) = location.split_once("://") {
        if !scheme.eq_ignore_ascii_case("https") {
            return Err(format!("redirect scheme '{scheme}' is not https"));
        }
        let (host, path) = split_host(rest);
        let host = normalize_domain(host)?;
        return Ok(format!(
            "https://{host}{}",
            if path.is_empty() { APP_ADS_PATH } else { path }
        ));
    }
    if let Some(path) = location.strip_prefix('/') {
        return Ok(format!("https://{current_host}/{path}"));
    }
    // Any other relative location stays on the same (already validated) host,
    // resolved against the current path's directory.
    let base = current_url
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or(current_url);
    Ok(format!("{base}/{location}"))
}

/// The host of a `https://host/...` url (assumed already validated).
fn host_of(url: &str) -> &str {
    let rest = url.strip_prefix("https://").unwrap_or(url);
    split_host(rest).0
}

fn split_host(rest: &str) -> (&str, &str) {
    match rest.find(['/', '?', '#']) {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
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

    fn redirect(status: u16, location: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![("Location".into(), location.to_string())],
            body: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_wrapped_transports_404_means_absent_not_a_failed_fetch() {
        let transport =
            ScriptedTransport::new(vec![Err(TransportError::Platform(PlatformError::Http {
                platform: PlatformId::Admob,
                operation: "fetchAppAdsTxt".to_string(),
                status: 404,
            }))]);
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let txt = fetcher
            .fetch(PlatformId::Admob, "example.com", vec!["admob.com".into()])
            .await
            .expect("a missing app-ads.txt is absent, not an error");
        assert!(!txt.present);
        assert!(txt.lines.is_empty());
        assert_eq!(txt.expected_networks, vec!["admob.com".to_string()]);
    }

    #[tokio::test]
    async fn a_shared_cache_fetches_one_file_for_every_platform() {
        let inner = ScriptedTransport::new(vec![
            Ok(ok(200, b"google.com, pub-1, DIRECT\n")),
            Ok(ok(200, b"metaads.com, pub-2, DIRECT\n")),
        ]);
        let cache = Arc::new(AppAdsCache::new());
        let transport = cache.wrap(inner.clone());
        let max = AppAdsTxtFetcher::new(transport.clone());
        let levelplay = AppAdsTxtFetcher::new(transport);
        let max_txt = max
            .fetch(
                PlatformId::AppLovinMax,
                "example.com",
                vec!["google.com".to_string()],
            )
            .await
            .expect("first fetch");
        let levelplay_txt = levelplay
            .fetch(
                PlatformId::LevelPlay,
                "https://Example.com/any/path",
                vec!["admob.com".to_string()],
            )
            .await
            .expect("replayed fetch");
        assert!(max_txt.present);
        assert_eq!(max_txt.lines, levelplay_txt.lines);
        // The expected networks stay platform-specific; only the file bytes
        // are shared.
        assert_eq!(max_txt.expected_networks, vec!["google.com".to_string()]);
        assert_eq!(
            levelplay_txt.expected_networks,
            vec!["admob.com".to_string()]
        );
        assert_eq!(
            inner.calls().len(),
            1,
            "one app-ads.txt fetch for both platforms"
        );

        // A different developer site is a different file and is fetched.
        let other = levelplay
            .fetch(
                PlatformId::LevelPlay,
                "other.example.com",
                vec!["metaads.com".to_string()],
            )
            .await
            .expect("other domain");
        assert_eq!(other.lines, vec!["metaads.com, pub-2, DIRECT".to_string()]);
        assert_eq!(
            inner.calls().len(),
            2,
            "distinct domains are distinct files"
        );
    }

    #[tokio::test]
    async fn a_cached_404_is_absent_for_every_platform_without_refetching() {
        let inner =
            ScriptedTransport::new(vec![Err(TransportError::Platform(PlatformError::Http {
                platform: PlatformId::Admob,
                operation: "fetchAppAdsTxt".to_string(),
                status: 404,
            }))]);
        let cache = Arc::new(AppAdsCache::new());
        let transport = cache.wrap(inner.clone());
        for platform in [PlatformId::Admob, PlatformId::LevelPlay] {
            let txt = AppAdsTxtFetcher::new(transport.clone())
                .fetch(platform, "example.com", Vec::new())
                .await
                .expect("absent");
            assert!(!txt.present);
            assert!(txt.lines.is_empty());
        }
        assert_eq!(inner.calls().len(), 1, "the absent verdict is cached");
    }

    #[tokio::test]
    async fn transient_failures_are_not_cached_as_a_verdict() {
        let inner = ScriptedTransport::new(vec![
            Err(TransportError::Platform(PlatformError::Http {
                platform: PlatformId::Admob,
                operation: "fetchAppAdsTxt".to_string(),
                status: 500,
            })),
            Err(TransportError::Platform(PlatformError::Http {
                platform: PlatformId::Admob,
                operation: "fetchAppAdsTxt".to_string(),
                status: 500,
            })),
        ]);
        let cache = Arc::new(AppAdsCache::new());
        let transport = cache.wrap(inner.clone());
        for _ in 0..2 {
            let error = AppAdsTxtFetcher::new(transport.clone())
                .fetch(PlatformId::Admob, "example.com", Vec::new())
                .await
                .expect_err("500 stays an error");
            assert!(matches!(error, PlatformError::Http { status: 500, .. }));
        }
        assert_eq!(
            inner.calls().len(),
            2,
            "a transient failure is retried by the next platform"
        );
    }

    #[tokio::test]
    async fn other_wrapped_transport_statuses_still_fail_typed() {
        let transport =
            ScriptedTransport::new(vec![Err(TransportError::Platform(PlatformError::Http {
                platform: PlatformId::Admob,
                operation: "fetchAppAdsTxt".to_string(),
                status: 500,
            }))]);
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let error = fetcher
            .fetch(PlatformId::Admob, "example.com", vec![])
            .await
            .expect_err("500 stays a failed fetch");
        assert!(
            matches!(error, PlatformError::Http { status: 500, .. }),
            "{error}"
        );
    }

    #[tokio::test]
    async fn follows_a_same_host_redirect_and_validates_every_hop() {
        let transport = ScriptedTransport::new(vec![
            Ok(redirect(301, "/app-ads.txt")),
            Ok(ok(200, b"google.com, pub-1, DIRECT\n")),
        ]);
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let txt = fetcher
            .fetch(PlatformId::AppLovinMax, "example.com", vec![])
            .await
            .expect("fetch");
        assert!(txt.present);
        let calls = transport.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].url, "https://example.com/app-ads.txt");
        assert_eq!(calls[1].url, "https://example.com/app-ads.txt");
    }

    #[tokio::test]
    async fn follows_a_protocol_relative_redirect_to_a_public_host() {
        let transport = ScriptedTransport::new(vec![
            Ok(redirect(302, "//cdn.example.net/app-ads.txt")),
            Ok(ok(200, b"google.com, pub-1, DIRECT\n")),
        ]);
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        fetcher
            .fetch(PlatformId::Admob, "example.com", vec![])
            .await
            .expect("fetch");
        let calls = transport.calls();
        assert_eq!(calls[1].url, "https://cdn.example.net/app-ads.txt");
    }

    #[tokio::test]
    async fn refuses_a_redirect_to_an_internal_host() {
        let transport =
            ScriptedTransport::new(vec![Ok(redirect(302, "https://127.0.0.1/app-ads.txt"))]);
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let err = fetcher
            .fetch(PlatformId::Admob, "example.com", vec![])
            .await
            .expect_err("refuse");
        assert!(matches!(err, PlatformError::Validation(_)), "{err}");
        assert_eq!(
            transport.calls().len(),
            1,
            "the internal host is never requested"
        );
    }

    #[tokio::test]
    async fn refuses_a_redirect_that_leaves_https() {
        let transport =
            ScriptedTransport::new(vec![Ok(redirect(302, "http://example.com/app-ads.txt"))]);
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let err = fetcher
            .fetch(PlatformId::Admob, "example.com", vec![])
            .await
            .expect_err("refuse");
        assert!(matches!(err, PlatformError::Validation(_)), "{err}");
        assert_eq!(transport.calls().len(), 1);
    }

    #[tokio::test]
    async fn refuses_an_over_long_redirect_chain() {
        let mut script = Vec::new();
        for _ in 0..(MAX_REDIRECTS + 1) {
            script.push(Ok(redirect(302, "/app-ads.txt")));
        }
        let transport = ScriptedTransport::new(script);
        let fetcher = AppAdsTxtFetcher::new(transport);
        let err = fetcher
            .fetch(PlatformId::Admob, "example.com", vec![])
            .await
            .expect_err("refuse");
        assert!(matches!(err, PlatformError::Schema { .. }), "{err}");
    }

    #[tokio::test]
    async fn refuses_a_redirect_without_a_location() {
        let transport = ScriptedTransport::new(vec![Ok(HttpResponse {
            status: 302,
            headers: vec![],
            body: Vec::new(),
        })]);
        let fetcher = AppAdsTxtFetcher::new(transport);
        let err = fetcher
            .fetch(PlatformId::Admob, "example.com", vec![])
            .await
            .expect_err("refuse");
        assert!(matches!(err, PlatformError::Schema { .. }), "{err}");
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
            (418, "http"),
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
