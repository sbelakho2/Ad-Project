//! The `curl`-based fetch of one public `app-ads.txt`.
//!
//! The fetch conventions mirror the sales engine's `io` module: the system
//! `curl` runs with a timeout, a size cap and an explicit user agent, the
//! URL is passed after `--` so it never reaches a shell, and a nonzero exit
//! is a typed transport failure carrying the bounded stderr excerpt.
//!
//! Redirects are never followed by curl. A 3xx answers its `Location`
//! through curl's `%{redirect_url}`, the hop is resolved by the same rules
//! the audit path uses (same scheme or an upgrade to HTTPS, a validated
//! public host, at most [`MAX_REDIRECTS`] hops) and the hop host passes the
//! resolve-time guard again before the next request. The watched domain is
//! read over HTTPS first; only a transport-level failure of the HTTPS
//! attempt falls back to a plain HTTP attempt, because a site without a
//! usable certificate is still a legitimate watch target.
//!
//! The body cap is [`MAX_APP_ADS_BYTES`] (the audit fetcher's parse cap); a
//! larger answer is refused instead of truncated. A 200 answers the file
//! text, 404 and 410 answer "no file" and every other status or failure is
//! an [`AppAdsUnreachable`] with a typed reason.

use std::net::IpAddr;
use std::process::{Command, Stdio};

use crate::guard::HostResolver;

/// Fetch path, rooted at the developer website.
pub const APP_ADS_PATH: &str = "/app-ads.txt";
/// Parse cap for one app-ads.txt response (mirrors the audit fetcher).
pub const MAX_APP_ADS_BYTES: usize = 1024 * 1024;
/// Redirect cap: every hop is validated and guarded, a longer chain fails.
pub const MAX_REDIRECTS: usize = 5;
/// Default fetch timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u32 = 10;
/// Default user agent, the watch service's own identity.
pub const DEFAULT_USER_AGENT: &str = "ApexWatch/1.0 (+https://apexmediation.ee)";
/// The marker separating the body from curl's status output.
pub const STATUS_MARKER: &str = "\n@@apex-watch-status@@";

/// Suffixes that never name a public developer website.
const RESERVED_SUFFIXES: [&str; 6] = [".local", ".internal", ".home", ".lan", ".test", ".invalid"];

/// A fetched page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// HTTP status.
    pub status: u16,
    /// Body bytes (capped by curl and refused above [`MAX_APP_ADS_BYTES`]).
    pub body: Vec<u8>,
    /// The URL curl would have followed for a 3xx answer.
    pub redirect: Option<String>,
}

/// Why a fetch failed before producing a status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The URL was refused before any network activity.
    InvalidUrl(String),
    /// The transport failed (DNS, TLS, timeout, nonzero curl exit).
    Transport(String),
    /// The body exceeded the parse cap.
    TooLarge {
        /// The observed body length in bytes.
        bytes: usize,
        /// The cap that refused it.
        cap: usize,
    },
    /// The redirect chain exceeded [`MAX_REDIRECTS`].
    TooManyRedirects(usize),
    /// A redirect hop was refused (an unusable Location, a scheme
    /// downgrade, or a hop host that resolves non-public).
    Redirect(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::InvalidUrl(url) => write!(formatter, "refused URL {url}"),
            FetchError::Transport(detail) => write!(formatter, "the transport failed: {detail}"),
            FetchError::TooLarge { bytes, cap } => {
                write!(
                    formatter,
                    "the response is {bytes} bytes, above the {cap} byte cap"
                )
            }
            FetchError::TooManyRedirects(hops) => {
                write!(formatter, "the redirect chain exceeds {hops} hops")
            }
            FetchError::Redirect(detail) => write!(formatter, "the redirect was refused: {detail}"),
        }
    }
}

/// Fetches one URL.
pub trait Fetcher: Send + Sync + std::fmt::Debug {
    /// GET `url` pinned to `approved`, the address the resolve-time guard
    /// already approved. The transport must not resolve the host name again:
    /// `curl` is given `--resolve host:port:address`, so a DNS answer that
    /// changes between the guard and the fetch cannot redirect the request.
    fn get(&self, url: &str, approved: IpAddr) -> Result<Fetched, FetchError>;
}

/// Normalize caller input into the hostname to fetch. Accepts
/// `example.com`, `Example.COM`, `https://example.com/...` and
/// `example.com.`, and returns the lowercase hostname without a trailing
/// dot. Anything that cannot be a public developer website is refused:
/// non-http schemes, ports, userinfo, brackets, whitespace, malformed
/// labels, reserved suffixes and numeric literals.
pub fn normalize_domain(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("domain is empty".to_string());
    }
    let host_part = match trimmed.split_once("://") {
        Some((scheme, rest)) => {
            if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
                return Err(format!("scheme '{scheme}' is not http or https"));
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
    if host == "localhost"
        || RESERVED_SUFFIXES
            .iter()
            .any(|suffix| host.ends_with(suffix))
    {
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

/// Validate one absolute fetch URL: http or https, a public hostname, no
/// userinfo, port, whitespace or control characters.
pub fn validate_url(url: &str) -> Result<(), FetchError> {
    let bad = || FetchError::InvalidUrl(url.chars().take(200).collect());
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) || url.len() > 2048 {
        return Err(bad());
    }
    let (scheme, rest) = url.split_once("://").ok_or_else(bad)?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return Err(bad());
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.contains('@') || host.contains(':') {
        return Err(bad());
    }
    normalize_domain(host).map_err(|_| bad())?;
    Ok(())
}

/// The `curl`-backed fetcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurlFetcher {
    /// Path to `curl`.
    pub binary: String,
    /// Timeout in seconds.
    pub timeout_secs: u32,
    /// Body size cap in bytes.
    pub max_bytes: usize,
    /// User-Agent sent.
    pub user_agent: String,
}

impl Default for CurlFetcher {
    fn default() -> Self {
        Self {
            binary: "curl".into(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_bytes: MAX_APP_ADS_BYTES,
            user_agent: DEFAULT_USER_AGENT.into(),
        }
    }
}

impl Fetcher for CurlFetcher {
    fn get(&self, url: &str, approved: IpAddr) -> Result<Fetched, FetchError> {
        validate_url(url)?;
        let scheme = url.split(':').next().unwrap_or("https");
        let host = host_of(url);
        let resolve = resolve_entry(host, default_port(scheme), approved);
        let timeout = self.timeout_secs.to_string();
        let max_bytes = self.max_bytes.to_string();
        let output = Command::new(&self.binary)
            .args([
                "--silent",
                "--show-error",
                "--proto",
                &format!("={scheme}"),
                "--max-time",
                &timeout,
                "--max-filesize",
                &max_bytes,
                // The guard already approved `approved`; pin curl to exactly
                // that address so it never resolves the name a second time.
                "--resolve",
                &resolve,
                "--user-agent",
                &self.user_agent,
                "--write-out",
                &format!("{STATUS_MARKER}%{{http_code}}|%{{redirect_url}}"),
                "--",
                url,
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| FetchError::Transport(error.to_string()))?;
        if !output.status.success() {
            let detail: String = String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(300)
                .collect();
            return Err(FetchError::Transport(if detail.is_empty() {
                format!("curl exited with {}", output.status)
            } else {
                detail
            }));
        }
        parse_curl_output(&output.stdout, self.max_bytes)
    }
}

/// The port a validated http(s) URL fetches from. [`validate_url`] refuses an
/// explicit port, so the scheme's default is the only possible port.
fn default_port(scheme: &str) -> u16 {
    if scheme.eq_ignore_ascii_case("http") {
        80
    } else {
        443
    }
}

/// One `curl --resolve` entry. IPv6 literals are bracketed, as curl's
/// `--resolve host:port:addr` grammar requires.
fn resolve_entry(host: &str, port: u16, approved: IpAddr) -> String {
    match approved {
        IpAddr::V4(address) => format!("{host}:{port}:{address}"),
        IpAddr::V6(address) => format!("{host}:{port}:[{address}]"),
    }
}

/// Split curl's stdout into the body, the status and the redirect URL.
pub fn parse_curl_output(stdout: &[u8], max_bytes: usize) -> Result<Fetched, FetchError> {
    let text = String::from_utf8_lossy(stdout);
    let (body, tail) = text
        .rsplit_once(STATUS_MARKER)
        .ok_or_else(|| FetchError::Transport("no status from curl".into()))?;
    let (status, redirect) = tail.trim().split_once('|').unwrap_or((tail.trim(), ""));
    let status: u16 = status
        .trim()
        .parse()
        .map_err(|_| FetchError::Transport("bad status from curl".into()))?;
    if body.len() > max_bytes {
        return Err(FetchError::TooLarge {
            bytes: body.len(),
            cap: max_bytes,
        });
    }
    let redirect = redirect.trim();
    Ok(Fetched {
        status,
        body: body.as_bytes().to_vec(),
        redirect: if redirect.is_empty() {
            None
        } else {
            Some(redirect.to_string())
        },
    })
}

/// What one successful `app-ads.txt` read produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAdsFetch {
    /// The file was read; the lines are UTF-8 text without the BOM.
    Text(Vec<String>),
    /// The server answered 404 or 410: no file.
    Missing,
}

/// Why a read never produced a file verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAdsUnreachable {
    /// The typed reason recorded in the subscriber's finding.
    pub reason: String,
}

/// Read one domain's public app-ads.txt with the guard and fallback rules of
/// the module documentation. The initial host must already have passed
/// [`crate::guard::guard_hosts`]; every redirect hop passes it here again.
pub fn fetch_app_ads(
    domain: &str,
    fetcher: &dyn Fetcher,
    resolver: &dyn HostResolver,
    approved: IpAddr,
) -> Result<AppAdsFetch, AppAdsUnreachable> {
    let https_url = format!("https://{domain}{APP_ADS_PATH}");
    match fetch_hop_chain(&https_url, true, approved, fetcher, resolver) {
        Ok(fetched) => classify(fetched),
        Err(FetchError::Transport(detail)) => {
            // Only a transport-level HTTPS failure earns the plain HTTP
            // attempt; an HTTP status answer is a verdict on its own. The
            // same guard-approved address is used: the host name is never
            // re-resolved.
            let http_url = format!("http://{domain}{APP_ADS_PATH}");
            match fetch_hop_chain(&http_url, false, approved, fetcher, resolver) {
                Ok(fetched) => classify(fetched),
                Err(error) => Err(unreachable(&format!("https failed ({detail}); {error}"))),
            }
        }
        Err(error) => Err(unreachable(&error.to_string())),
    }
}

fn unreachable(detail: &str) -> AppAdsUnreachable {
    AppAdsUnreachable {
        reason: detail.to_string(),
    }
}

fn classify(fetched: Fetched) -> Result<AppAdsFetch, AppAdsUnreachable> {
    match fetched.status {
        200 => {
            let text = std::str::from_utf8(&fetched.body)
                .map_err(|_| unreachable("the response body is not valid UTF-8"))?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(text);
            Ok(AppAdsFetch::Text(
                text.lines().map(str::to_string).collect(),
            ))
        }
        404 | 410 => Ok(AppAdsFetch::Missing),
        429 => Err(unreachable("the server rate limited the request")),
        status => Err(unreachable(&format!("the server answered HTTP {status}"))),
    }
}

/// Follow one guarded redirect chain. `https_only` pins the first attempt to
/// HTTPS and refuses a downgrade; the HTTP fallback accepts an upgrade to
/// HTTPS on a hop but never a later downgrade.
fn fetch_hop_chain(
    start_url: &str,
    https_only: bool,
    approved: IpAddr,
    fetcher: &dyn Fetcher,
    resolver: &dyn HostResolver,
) -> Result<Fetched, FetchError> {
    let mut current = start_url.to_string();
    let mut current_approved = approved;
    for _ in 0..MAX_REDIRECTS {
        let response = fetcher.get(&current, current_approved)?;
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let location = response.redirect.clone().ok_or_else(|| {
            FetchError::Redirect("the redirect carries no Location header".to_string())
        })?;
        let next = resolve_redirect(&current, &location, https_only).map_err(|detail| {
            FetchError::Redirect(format!("a redirect to {location} is refused: {detail}"))
        })?;
        current_approved = guard_hop(&next, resolver)?;
        current = next;
    }
    Err(FetchError::TooManyRedirects(MAX_REDIRECTS))
}

/// Guard one redirect hop's host before the next request, returning the
/// first approved address to pin the request to.
fn guard_hop(url: &str, resolver: &dyn HostResolver) -> Result<IpAddr, FetchError> {
    let host = host_of(url);
    let addresses = resolver.resolve(host).map_err(|detail| {
        FetchError::Redirect(format!("the hop host {host} did not resolve: {detail}"))
    })?;
    crate::guard::guard_hosts(&addresses).map_err(|refusal| {
        FetchError::Redirect(format!(
            "the hop host {host} resolves to a {} address",
            refusal.class
        ))
    })?;
    addresses
        .first()
        .copied()
        .ok_or_else(|| FetchError::Redirect(format!("the hop host {host} returned no addresses")))
}

fn host_of(url: &str) -> &str {
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest,
        None => url,
    };
    match rest.find(['/', '?', '#']) {
        Some(index) => &rest[..index],
        None => rest,
    }
}

/// Resolve one redirect hop against the current URL. Same-scheme and
/// root-relative locations stay on the current (already guarded) host;
/// absolute and protocol-relative locations must name a public host and may
/// only upgrade to HTTPS.
fn resolve_redirect(current_url: &str, location: &str, https_only: bool) -> Result<String, String> {
    let location = location.trim();
    if location.is_empty() {
        return Err("the Location is empty".to_string());
    }
    let current_scheme = current_url
        .split_once("://")
        .map(|(scheme, _)| scheme)
        .unwrap_or("https");
    let allow_http = !https_only && current_scheme == "http";
    let current_host = host_of(current_url);
    if let Some(rest) = location.strip_prefix("//") {
        let (host, path) = split_host(rest);
        let host = normalize_domain(host)?;
        return Ok(format!(
            "{current_scheme}://{host}{}",
            if path.is_empty() { APP_ADS_PATH } else { path }
        ));
    }
    if let Some((scheme, rest)) = location.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if scheme == "http" && !allow_http {
            return Err("the redirect would leave the current scheme".to_string());
        }
        if scheme != "https" && scheme != "http" {
            return Err(format!("scheme '{scheme}' is not http or https"));
        }
        let (host, path) = split_host(rest);
        let host = normalize_domain(host)?;
        return Ok(format!(
            "{scheme}://{host}{}",
            if path.is_empty() { APP_ADS_PATH } else { path }
        ));
    }
    if let Some(path) = location.strip_prefix('/') {
        return Ok(format!("{current_scheme}://{current_host}/{path}"));
    }
    let base = current_url
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or(current_url);
    Ok(format!("{base}/{location}"))
}

fn split_host(rest: &str) -> (&str, &str) {
    match rest.find(['/', '?', '#']) {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    }
}

/// A fixed map of responses, for tests and replays.
#[derive(Debug, Default)]
pub struct MapFetcher {
    /// URL to `(status, body)`; every other URL is a transport failure.
    pub pages: Vec<(String, u16, Vec<u8>, Option<String>)>,
    /// Every URL requested, in order.
    pub calls: std::sync::Mutex<Vec<String>>,
}

impl Fetcher for MapFetcher {
    fn get(&self, url: &str, _approved: IpAddr) -> Result<Fetched, FetchError> {
        self.calls
            .lock()
            .map_err(|_| FetchError::Transport("map fetcher lock poisoned".into()))?
            .push(url.to_string());
        self.pages
            .iter()
            .find(|(page, ..)| page == url)
            .map(|(_, status, body, redirect)| Fetched {
                status: *status,
                body: body.clone(),
                redirect: redirect.clone(),
            })
            .ok_or_else(|| FetchError::Transport(format!("no route to {url}")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::net::IpAddr;

    use super::*;
    use crate::guard::HostResolver;

    /// A resolver with fixed answers; unknown hosts fail to resolve.
    #[derive(Debug)]
    struct FixedResolver {
        answers: Vec<(&'static str, &'static str)>,
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl FixedResolver {
        fn new(answers: Vec<(&'static str, &'static str)>) -> Self {
            Self {
                answers,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl HostResolver for FixedResolver {
        fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
            self.calls.lock().unwrap().push(host.to_string());
            let addresses: Vec<IpAddr> = self
                .answers
                .iter()
                .filter(|(name, _)| *name == host)
                .map(|(_, address)| address.parse::<IpAddr>().unwrap())
                .collect();
            if addresses.is_empty() {
                Err(format!("no answer for {host}"))
            } else {
                Ok(addresses)
            }
        }
    }

    fn page(url: &str, status: u16, body: &[u8]) -> (String, u16, Vec<u8>, Option<String>) {
        (url.to_string(), status, body.to_vec(), None)
    }

    fn redirect(url: &str, location: &str) -> (String, u16, Vec<u8>, Option<String>) {
        (url.to_string(), 302, Vec::new(), Some(location.to_string()))
    }

    fn public() -> FixedResolver {
        FixedResolver::new(vec![
            ("example.com", "93.184.216.34"),
            ("cdn.example.net", "93.184.216.35"),
            ("private.example", "10.0.0.9"),
        ])
    }

    fn approved() -> IpAddr {
        "93.184.216.34".parse().expect("test address")
    }

    #[test]
    fn normalize_accepts_website_forms_and_refuses_hostile_ones() {
        assert_eq!(normalize_domain("Example.COM").unwrap(), "example.com");
        assert_eq!(
            normalize_domain("https://Sub.Example.com/x").unwrap(),
            "sub.example.com"
        );
        assert_eq!(normalize_domain("example.com.").unwrap(), "example.com");
        for raw in [
            "",
            "localhost",
            "127.0.0.1",
            "192.168.0.10",
            "example.local",
            "user@example.com",
            "example.com:8080",
            "[::1]",
            "-bad.example.com",
            "bad..example.com",
            "exa mple.com",
            "ftp://example.com",
        ] {
            assert!(normalize_domain(raw).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn validate_url_accepts_both_schemes_and_refuses_exotic_shapes() {
        assert!(validate_url("https://example.com/app-ads.txt").is_ok());
        assert!(validate_url("http://example.com/app-ads.txt").is_ok());
        for bad in [
            "ftp://example.com/",
            "https://127.0.0.1/app-ads.txt",
            "https://example.com:8443/",
            "https://user@example.com/",
            "https://example.com/a b",
            "https://example.com/\n",
            "-https://example.com",
            "https://localhost/app-ads.txt",
        ] {
            assert!(validate_url(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn curl_output_parses_status_redirect_and_enforces_the_cap() {
        let stdout = format!("body bytes{STATUS_MARKER}200|");
        let fetched = parse_curl_output(stdout.as_bytes(), 64).unwrap();
        assert_eq!(fetched.status, 200);
        assert_eq!(fetched.body, b"body bytes");
        assert_eq!(fetched.redirect, None);

        let stdout = format!("{STATUS_MARKER}302|https://cdn.example.net/app-ads.txt");
        let fetched = parse_curl_output(stdout.as_bytes(), 64).unwrap();
        assert_eq!(fetched.status, 302);
        assert_eq!(
            fetched.redirect.as_deref(),
            Some("https://cdn.example.net/app-ads.txt")
        );

        assert!(matches!(
            parse_curl_output(b"no marker", 64),
            Err(FetchError::Transport(_))
        ));
        let long = format!("{}x{STATUS_MARKER}200|", "a".repeat(65));
        assert!(matches!(
            parse_curl_output(long.as_bytes(), 64),
            Err(FetchError::TooLarge { .. })
        ));
    }

    #[test]
    fn https_success_reads_lines() {
        let fetcher = MapFetcher {
            pages: vec![page(
                "https://example.com/app-ads.txt",
                200,
                b"\xef\xbb\xbfgoogle.com, pub-1, DIRECT\r\nexample.com,pub-2,RESELLER\n",
            )],
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let read = fetch_app_ads("example.com", &fetcher, &public(), approved()).expect("read");
        assert_eq!(
            read,
            AppAdsFetch::Text(vec![
                "google.com, pub-1, DIRECT".to_string(),
                "example.com,pub-2,RESELLER".to_string(),
            ])
        );
        assert_eq!(fetcher.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn missing_files_answer_missing_for_both_statuses() {
        for status in [404, 410] {
            let fetcher = MapFetcher {
                pages: vec![page("https://example.com/app-ads.txt", status, b"")],
                calls: std::sync::Mutex::new(Vec::new()),
            };
            assert_eq!(
                fetch_app_ads("example.com", &fetcher, &public(), approved()).expect("read"),
                AppAdsFetch::Missing,
                "status {status}"
            );
        }
    }

    #[test]
    fn a_transport_failure_falls_back_to_plain_http() {
        let fetcher = MapFetcher {
            pages: vec![page(
                "http://example.com/app-ads.txt",
                200,
                b"admob.com, pub-1, DIRECT\n",
            )],
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let read = fetch_app_ads("example.com", &fetcher, &public(), approved()).expect("read");
        assert!(matches!(read, AppAdsFetch::Text(_)));
        let calls = fetcher.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                "https://example.com/app-ads.txt".to_string(),
                "http://example.com/app-ads.txt".to_string(),
            ]
        );
    }

    #[test]
    fn an_http_status_answer_never_falls_back() {
        let fetcher = MapFetcher {
            pages: vec![page("https://example.com/app-ads.txt", 500, b"")],
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let error =
            fetch_app_ads("example.com", &fetcher, &public(), approved()).expect_err("unreachable");
        assert!(error.reason.contains("HTTP 500"), "{}", error.reason);
        assert_eq!(fetcher.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn same_host_redirects_are_followed_with_a_second_guard() {
        let fetcher = MapFetcher {
            pages: vec![
                redirect("https://example.com/app-ads.txt", "/files/app-ads.txt"),
                page("https://example.com/files/app-ads.txt", 404, b""),
            ],
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let resolver = public();
        let read = fetch_app_ads("example.com", &fetcher, &resolver, approved()).expect("read");
        assert_eq!(read, AppAdsFetch::Missing);
        let calls = fetcher.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                "https://example.com/app-ads.txt".to_string(),
                "https://example.com/files/app-ads.txt".to_string(),
            ]
        );
        assert!(
            resolver
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|host| host == "example.com"),
            "the hop host is guarded again"
        );
    }

    #[test]
    fn a_redirect_to_another_public_host_is_guarded_and_followed() {
        let fetcher = MapFetcher {
            pages: vec![
                redirect(
                    "https://example.com/app-ads.txt",
                    "https://cdn.example.net/app-ads.txt",
                ),
                page(
                    "https://cdn.example.net/app-ads.txt",
                    200,
                    b"admob.com, pub-1, DIRECT\n",
                ),
            ],
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let read = fetch_app_ads("example.com", &fetcher, &public(), approved()).expect("read");
        assert!(matches!(read, AppAdsFetch::Text(_)));
    }

    #[test]
    fn a_redirect_to_a_private_host_is_refused_before_the_request() {
        let fetcher = MapFetcher {
            pages: vec![redirect(
                "https://example.com/app-ads.txt",
                "https://private.example/app-ads.txt",
            )],
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let error =
            fetch_app_ads("example.com", &fetcher, &public(), approved()).expect_err("refused");
        assert!(error.reason.contains("private"), "{}", error.reason);
        let calls = fetcher.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "no fallback and no private request");
        assert!(
            calls.iter().all(|url| !url.contains("private.example")),
            "{calls:?}"
        );
    }

    #[test]
    fn a_redirect_downgrade_is_refused() {
        let fetcher = MapFetcher {
            pages: vec![redirect(
                "https://example.com/app-ads.txt",
                "http://example.com/app-ads.txt",
            )],
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let error =
            fetch_app_ads("example.com", &fetcher, &public(), approved()).expect_err("refused");
        assert!(
            error.reason.contains("leave the current scheme"),
            "{}",
            error.reason
        );
    }

    #[test]
    fn an_over_long_redirect_chain_fails_typed() {
        let mut pages = Vec::new();
        for _ in 0..(MAX_REDIRECTS + 1) {
            pages.push(redirect("https://example.com/app-ads.txt", "/app-ads.txt"));
        }
        let fetcher = MapFetcher {
            pages,
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let error =
            fetch_app_ads("example.com", &fetcher, &public(), approved()).expect_err("chain");
        assert!(error.reason.contains("redirect chain"), "{}", error.reason);
        assert_eq!(fetcher.calls.lock().unwrap().len(), MAX_REDIRECTS);
    }

    /// A resolver whose answer changes per call: the first call (the guard)
    /// answers a public address, a second call (a naive fetch-time
    /// resolution) would answer loopback.
    #[derive(Debug)]
    struct SwappingResolver {
        answers: std::sync::Mutex<Vec<IpAddr>>,
        calls: std::sync::Mutex<usize>,
    }

    impl HostResolver for SwappingResolver {
        fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let answers = self.answers.lock().unwrap();
            answers
                .get((*calls - 1).min(answers.len().saturating_sub(1)))
                .copied()
                .map(|address| vec![address])
                .ok_or_else(|| format!("no answer for {host}"))
        }
    }

    /// A fetcher that records the address it was pinned to and answers a
    /// body derived from it: a loopback pin is the rebinding canary.
    #[derive(Debug, Default)]
    struct RecordingFetcher {
        pins: std::sync::Mutex<Vec<(String, IpAddr)>>,
    }

    impl Fetcher for RecordingFetcher {
        fn get(&self, url: &str, approved: IpAddr) -> Result<Fetched, FetchError> {
            self.pins.lock().unwrap().push((url.to_string(), approved));
            let body = if approved.is_loopback() {
                b"REBOUND-TO-LOOPBACK.example, pub-evil, DIRECT\n".to_vec()
            } else {
                b"admob.com, pub-1, DIRECT\n".to_vec()
            };
            Ok(Fetched {
                status: 200,
                body,
                redirect: None,
            })
        }
    }

    #[test]
    fn a_dns_swap_between_guard_and_fetch_cannot_redirect_the_request() {
        let resolver = SwappingResolver {
            answers: std::sync::Mutex::new(vec![
                "93.184.216.34".parse().unwrap(),
                "127.0.0.1".parse().unwrap(),
            ]),
            calls: std::sync::Mutex::new(0),
        };
        // The guard's one resolution approves the public address.
        let addresses = resolver.resolve("example.com").expect("guard answer");
        assert_eq!(*resolver.calls.lock().unwrap(), 1);
        assert!(crate::guard::guard_hosts(&addresses).is_ok());

        let fetcher = RecordingFetcher::default();
        let read = fetch_app_ads("example.com", &fetcher, &resolver, addresses[0]).expect("read");
        assert_eq!(
            read,
            AppAdsFetch::Text(vec!["admob.com, pub-1, DIRECT".to_string()])
        );
        assert_eq!(
            *resolver.calls.lock().unwrap(),
            1,
            "the fetch must not resolve the host a second time"
        );
        let pins = fetcher.pins.lock().unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].0, "https://example.com/app-ads.txt");
        assert_eq!(pins[0].1, addresses[0]);
        assert!(
            !pins.iter().any(|(_, address)| address.is_loopback()),
            "the swapped-in loopback answer never reaches the fetch"
        );
    }

    #[test]
    fn resolve_entries_bracket_ipv6_and_keep_the_scheme_port() {
        assert_eq!(
            resolve_entry("example.com", 443, "93.184.216.34".parse().unwrap()),
            "example.com:443:93.184.216.34"
        );
        assert_eq!(
            resolve_entry("example.com", 80, "93.184.216.34".parse().unwrap()),
            "example.com:80:93.184.216.34"
        );
        assert_eq!(
            resolve_entry(
                "example.com",
                443,
                "2606:2800:220:1:248:1893:25c8:1946".parse().unwrap()
            ),
            "example.com:443:[2606:2800:220:1:248:1893:25c8:1946]"
        );
    }

    #[test]
    fn curl_fetcher_runs_curl_and_reads_its_status() {
        let dir = std::env::temp_dir().join(format!("apex-watch-curl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("curl.sh");
        let argv_path = dir.join("argv.txt");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf 'hello{STATUS_MARKER}200|'\n",
                argv_path.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let fetcher = CurlFetcher {
            binary: script.display().to_string(),
            ..CurlFetcher::default()
        };
        let fetched = fetcher
            .get("https://example.com/app-ads.txt", approved())
            .expect("fetch");
        assert_eq!(fetched.status, 200);
        assert_eq!(fetched.body, b"hello");

        // The exact argv pins the guard-approved address: host:443 because
        // `validate_url` refuses an explicit port and the URL is HTTPS.
        let argv = std::fs::read_to_string(&argv_path).expect("argv");
        let args: Vec<&str> = argv.lines().collect();
        let resolve = args
            .iter()
            .position(|arg| *arg == "--resolve")
            .expect("--resolve is passed");
        assert_eq!(args[resolve + 1], "example.com:443:93.184.216.34");
        assert!(
            args.contains(&"https://example.com/app-ads.txt"),
            "{args:?}"
        );

        let failing = dir.join("fail.sh");
        std::fs::write(
            &failing,
            "#!/bin/sh\necho 'could not resolve host' >&2\nexit 6\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let fetcher = CurlFetcher {
            binary: failing.display().to_string(),
            ..CurlFetcher::default()
        };
        let error = fetcher
            .get("https://example.com/app-ads.txt", approved())
            .expect_err("fail");
        assert!(matches!(error, FetchError::Transport(ref detail) if detail.contains("resolve")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
