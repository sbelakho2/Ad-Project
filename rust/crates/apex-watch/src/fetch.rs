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

use serde::{Deserialize, Serialize};

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

/// The conditional-GET validators a response advertised and a later request
/// sends back. Both are optional: a server that sends neither keeps being
/// fetched unconditionally.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpValidators {
    /// The `ETag` response header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// The `Last-Modified` response header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

impl HttpValidators {
    /// Whether this carries at least one validator.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none()
    }

    /// The `(name, value)` header pairs a conditional request sends. A value
    /// with a control character is dropped rather than injected into the
    /// header stream.
    #[must_use]
    pub fn headers(&self) -> Vec<(&'static str, &str)> {
        let mut headers = Vec::with_capacity(2);
        if let Some(etag) = self.etag.as_deref().filter(|value| header_value_ok(value)) {
            headers.push(("If-None-Match", etag));
        }
        if let Some(modified) = self
            .last_modified
            .as_deref()
            .filter(|value| header_value_ok(value))
        {
            headers.push(("If-Modified-Since", modified));
        }
        headers
    }
}

/// A header value is safe when it is non-empty and carries no control
/// characters (notably no CR/LF that could split the header block).
fn header_value_ok(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(|ch| ch.is_control())
}

/// A fetched page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// HTTP status.
    pub status: u16,
    /// Body bytes (capped by curl and refused above [`MAX_APP_ADS_BYTES`]).
    pub body: Vec<u8>,
    /// The URL curl would have followed for a 3xx answer.
    pub redirect: Option<String>,
    /// The `ETag` response header, when the server sent one.
    pub etag: Option<String>,
    /// The `Last-Modified` response header, when the server sent one.
    pub last_modified: Option<String>,
}

impl Fetched {
    /// The validators this response advertised.
    #[must_use]
    pub fn validators(&self) -> HttpValidators {
        HttpValidators {
            etag: self.etag.clone(),
            last_modified: self.last_modified.clone(),
        }
    }
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

    /// GET `url` with the given conditional validators. The default keeps a
    /// scripted or replay fetcher that does not model headers unconditional.
    fn get_conditional(
        &self,
        url: &str,
        approved: IpAddr,
        validators: Option<&HttpValidators>,
    ) -> Result<Fetched, FetchError> {
        let _ = validators;
        self.get(url, approved)
    }
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

/// Resolve a bare binary name through the search path once, at construction. The
/// fetch clears the child environment (vault keys/tokens), so the command
/// must be an absolute path that `Command` can exec without the child search path.
fn resolve_binary(name: &str) -> String {
    if name.contains('/') {
        return name.to_string();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return name.to_string();
    };
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.display().to_string())
        .unwrap_or_else(|| name.to_string())
}

impl Default for CurlFetcher {
    fn default() -> Self {
        Self {
            binary: resolve_binary("curl"),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_bytes: MAX_APP_ADS_BYTES,
            user_agent: DEFAULT_USER_AGENT.into(),
        }
    }
}

impl Fetcher for CurlFetcher {
    fn get(&self, url: &str, approved: IpAddr) -> Result<Fetched, FetchError> {
        self.get_conditional(url, approved, None)
    }

    fn get_conditional(
        &self,
        url: &str,
        approved: IpAddr,
        validators: Option<&HttpValidators>,
    ) -> Result<Fetched, FetchError> {
        validate_url(url)?;
        let scheme = url.split(':').next().unwrap_or("https");
        let host = host_of(url);
        let resolve = resolve_entry(host, default_port(scheme), approved);
        let timeout = self.timeout_secs.to_string();
        let max_bytes = self.max_bytes.to_string();
        let mut command = Command::new(&self.binary);
        // The fetch must not inherit the service environment (vault keys,
        // tokens) and must not read a user curlrc: a compromised or
        // path-shadowed curl would otherwise receive every secret.
        command.env_clear();
        command.args([
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
        ]);
        if let Some(validators) = validators {
            for (name, value) in validators.headers() {
                command.arg("--header").arg(format!("{name}: {value}"));
            }
        }
        let output = command
            .args([
                "--write-out",
                // `header_json` (curl >= 7.83) exposes the response headers so
                // the next request can send the validators without a second
                // A body-less request.
                &format!("{STATUS_MARKER}%{{http_code}}|%{{redirect_url}}|%{{header_json}}"),
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

/// Split curl's stdout into the body, the status, the redirect URL and the
/// response validators. Older scripts that emit only `status|redirect` are
/// accepted (the validators are then absent).
pub fn parse_curl_output(stdout: &[u8], max_bytes: usize) -> Result<Fetched, FetchError> {
    let text = String::from_utf8_lossy(stdout);
    let (body, tail) = text
        .rsplit_once(STATUS_MARKER)
        .ok_or_else(|| FetchError::Transport("no status from curl".into()))?;
    let mut fields = tail.trim().splitn(3, '|');
    let status = fields.next().unwrap_or_default();
    let redirect = fields.next().unwrap_or_default();
    let header_json = fields.next().unwrap_or_default();
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
    let (etag, last_modified) = parse_header_json(header_json);
    let redirect = redirect.trim();
    Ok(Fetched {
        status,
        body: body.as_bytes().to_vec(),
        redirect: if redirect.is_empty() {
            None
        } else {
            Some(redirect.to_string())
        },
        etag,
        last_modified,
    })
}

/// Pull `ETag` and `Last-Modified` out of curl's `header_json` write-out.
/// Header names are matched case-insensitively; an absent or unparseable
/// document simply advertises no validators.
fn parse_header_json(header_json: &str) -> (Option<String>, Option<String>) {
    let header_json = header_json.trim();
    if header_json.is_empty() || header_json == "null" {
        return (None, None);
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(header_json) else {
        return (None, None);
    };
    let Some(object) = value.as_object() else {
        return (None, None);
    };
    let find = |name: &str| {
        object
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    (find("etag"), find("last-modified"))
}

/// What one successful `app-ads.txt` read produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAdsFetch {
    /// The file was read; the lines are UTF-8 text without the BOM.
    Text(Vec<String>),
    /// The server answered 404 or 410: no file.
    Missing,
    /// The server answered 304: the cached validators still describe the
    /// current file, so the caller reuses its previous findings.
    Unchanged,
}

/// One conditional read: the verdict plus the validators the response
/// advertised (for a 200) so the next run can send them back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAdsRead {
    /// What the read produced.
    pub outcome: AppAdsFetch,
    /// The validators of a 200 (or a refreshed 304) response.
    pub validators: HttpValidators,
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
    fetch_app_ads_conditional(domain, fetcher, resolver, approved, None).map(|read| read.outcome)
}

/// Read one domain's public app-ads.txt with conditional validators. The
/// validators are sent on the first hop only: they describe the watched
/// URL, not an arbitrary redirect target. The guarded HTTPS-only behavior
/// and redirect rules are exactly [`fetch_app_ads`]'s.
pub fn fetch_app_ads_conditional(
    domain: &str,
    fetcher: &dyn Fetcher,
    resolver: &dyn HostResolver,
    approved: IpAddr,
    validators: Option<&HttpValidators>,
) -> Result<AppAdsRead, AppAdsUnreachable> {
    let https_url = format!("https://{domain}{APP_ADS_PATH}");
    // HTTPS only. A plain-HTTP fallback after a transport failure would let
    // an on-path attacker force a TLS error and serve a forged app-ads.txt;
    // an unreachable or unverifiable file is the honest answer.
    match fetch_hop_chain(&https_url, true, approved, fetcher, resolver, validators) {
        Ok(fetched) => classify(fetched),
        Err(error) => Err(unreachable(&error.to_string())),
    }
}

fn unreachable(detail: &str) -> AppAdsUnreachable {
    AppAdsUnreachable {
        reason: detail.to_string(),
    }
}

fn classify(fetched: Fetched) -> Result<AppAdsRead, AppAdsUnreachable> {
    let validators = fetched.validators();
    let outcome = match fetched.status {
        200 => {
            let text = std::str::from_utf8(&fetched.body)
                .map_err(|_| unreachable("the response body is not valid UTF-8"))?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(text);
            AppAdsFetch::Text(text.lines().map(str::to_string).collect())
        }
        304 => AppAdsFetch::Unchanged,
        404 | 410 => AppAdsFetch::Missing,
        429 => return Err(unreachable("the server rate limited the request")),
        status => return Err(unreachable(&format!("the server answered HTTP {status}"))),
    };
    Ok(AppAdsRead {
        outcome,
        validators,
    })
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
    validators: Option<&HttpValidators>,
) -> Result<Fetched, FetchError> {
    let mut current = start_url.to_string();
    let mut current_approved = approved;
    for hop in 0..MAX_REDIRECTS {
        // Validators describe the watched URL: a redirect target is a
        // different resource, so only the first hop is conditional.
        let response = if hop == 0 {
            fetcher.get_conditional(&current, current_approved, validators)?
        } else {
            fetcher.get(&current, current_approved)?
        };
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
                etag: None,
                last_modified: None,
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
    fn curl_output_reads_validators_and_a_304_from_header_json() {
        let stdout = format!(
            "{STATUS_MARKER}304||{{\"etag\":\"\\\"v2\\\"\",\"last-modified\":\"Tue, 01 Oct 2026 10:00:00 GMT\",\"content-length\":\"0\"}}"
        );
        let fetched = parse_curl_output(stdout.as_bytes(), 64).unwrap();
        assert_eq!(fetched.status, 304);
        assert_eq!(fetched.body, b"");
        assert_eq!(fetched.etag.as_deref(), Some("\"v2\""));
        assert_eq!(
            fetched.last_modified.as_deref(),
            Some("Tue, 01 Oct 2026 10:00:00 GMT")
        );

        // Header names are matched case-insensitively and a missing document
        // simply advertises no validators.
        let stdout = format!("{STATUS_MARKER}200|url|{{\"ETag\":\"abc\"}}");
        let fetched = parse_curl_output(stdout.as_bytes(), 64).unwrap();
        assert_eq!(fetched.etag.as_deref(), Some("abc"));
        assert_eq!(fetched.last_modified, None);
        let stdout = format!("{STATUS_MARKER}200|url|not-json");
        let fetched = parse_curl_output(stdout.as_bytes(), 64).unwrap();
        assert_eq!(fetched.etag, None);
        assert_eq!(fetched.last_modified, None);
    }

    #[test]
    fn conditional_validator_headers_drop_control_characters() {
        let validators = HttpValidators {
            etag: Some("\"v1\"".to_string()),
            last_modified: Some("Tue, 01 Oct 2026 10:00:00 GMT".to_string()),
        };
        assert_eq!(
            validators.headers(),
            vec![
                ("If-None-Match", "\"v1\""),
                ("If-Modified-Since", "Tue, 01 Oct 2026 10:00:00 GMT"),
            ]
        );
        let hostile = HttpValidators {
            etag: Some("x\r\nX-Injected: yes".to_string()),
            last_modified: Some("  ".to_string()),
        };
        assert!(hostile.headers().is_empty());
    }

    #[test]
    fn a_304_is_classified_unchanged_and_a_200_carries_its_validators() {
        let fetched = Fetched {
            status: 304,
            body: Vec::new(),
            redirect: None,
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
        };
        let read = classify(fetched).expect("304");
        assert_eq!(read.outcome, AppAdsFetch::Unchanged);
        assert_eq!(read.validators.etag.as_deref(), Some("\"v1\""));

        let fetched = Fetched {
            status: 200,
            body: b"admob.com, pub-1, DIRECT\n".to_vec(),
            redirect: None,
            etag: Some("\"v2\"".to_string()),
            last_modified: Some("date".to_string()),
        };
        let read = classify(fetched).expect("200");
        assert!(matches!(read.outcome, AppAdsFetch::Text(_)));
        assert_eq!(read.validators.etag.as_deref(), Some("\"v2\""));
        assert_eq!(read.validators.last_modified.as_deref(), Some("date"));
    }

    #[test]
    fn the_scripted_fetcher_receives_the_validators() {
        let scripted = MapFetcher {
            pages: vec![page("https://example.com/app-ads.txt", 200, b"ok\n")],
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let validators = HttpValidators {
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
        };
        // The default `get_conditional` delegates to `get`, so the scripted
        // replay still answers; this pins the default contract.
        let fetched = scripted
            .get_conditional(
                "https://example.com/app-ads.txt",
                approved(),
                Some(&validators),
            )
            .expect("scripted");
        assert_eq!(fetched.status, 200);
        assert_eq!(scripted.calls.lock().unwrap().len(), 1);
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
    fn a_transport_failure_is_reported_not_downgraded() {
        let fetcher = MapFetcher {
            pages: vec![page(
                "http://example.com/app-ads.txt",
                200,
                b"admob.com, pub-1, DIRECT\n",
            )],
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let error = fetch_app_ads("example.com", &fetcher, &public(), approved())
            .expect_err("https is the only transport");
        assert!(error.reason.contains("no route"), "{}", error.reason);
        assert_eq!(
            fetcher.calls.lock().unwrap().clone(),
            vec!["https://example.com/app-ads.txt".to_string()],
            "the plain-HTTP URL is never requested"
        );
    }

    #[test]
    fn a_transport_failure_never_falls_back_to_plain_http() {
        #[derive(Debug)]
        struct Failing;
        impl Fetcher for Failing {
            fn get(&self, _url: &str, _approved: IpAddr) -> Result<Fetched, FetchError> {
                Err(FetchError::Transport("tls handshake failed".to_string()))
            }
        }
        let error =
            fetch_app_ads("example.com", &Failing, &public(), approved()).expect_err("unreachable");
        assert!(
            error.reason.contains("tls handshake failed"),
            "{}",
            error.reason
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
                etag: None,
                last_modified: None,
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

    #[test]
    fn curl_passes_conditional_headers_and_exposes_a_304() {
        let dir = std::env::temp_dir().join(format!("apex-watch-cond-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("curl.sh");
        let argv_path = dir.join("argv.txt");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '{STATUS_MARKER}304||{{\"etag\":\"v1\"}}'\n",
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
        let validators = HttpValidators {
            etag: Some("\"v1\"".to_string()),
            last_modified: Some("Tue, 01 Oct 2026 10:00:00 GMT".to_string()),
        };
        let fetched = fetcher
            .get_conditional(
                "https://example.com/app-ads.txt",
                approved(),
                Some(&validators),
            )
            .expect("fetch");
        assert_eq!(fetched.status, 304);
        assert_eq!(fetched.etag.as_deref(), Some("v1"));

        let argv = std::fs::read_to_string(&argv_path).expect("argv");
        let args: Vec<&str> = argv.lines().collect();
        let headers: Vec<&str> = args
            .windows(2)
            .filter(|pair| pair[0] == "--header")
            .map(|pair| pair[1])
            .collect();
        assert_eq!(
            headers,
            vec![
                "If-None-Match: \"v1\"",
                "If-Modified-Since: Tue, 01 Oct 2026 10:00:00 GMT",
            ],
            "{args:?}"
        );
        // The guarded, pinned fetch rules are unchanged.
        assert!(args.contains(&"--resolve"));
        assert!(args.contains(&"example.com:443:93.184.216.34"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
