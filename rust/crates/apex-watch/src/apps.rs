//! App Store verification of the named apps that decide the subscription tier.
//!
//! The public sign-up form accepts one app-store id, package id or URL per
//! line. Apple ids are confirmed through the iTunes lookup API and Android
//! package ids through the public Play listing. Once one app is confirmed,
//! the developer's whole App Store catalogue and Play catalogue are read, so
//! a publisher who names only one app (or only an Android app) is priced on
//! the real portfolio rather than on the submitted lines. The submitted line
//! count is never the tier.
//!
//! [`verify_apps`] is the one verification rule. It deduplicates through the
//! same [`crate::model::app_identity`] fold the sign-up parser and the
//! historical tier count use, so ten copies of one id are still one app. A
//! network or timeout failure is recorded as an unavailable lookup rather
//! than treated as a confirmation; the caller keeps the previous verified tier
//! and records the honest "not re-verified" state. A catalogue read that
//! fails never downgrades a confirmed named-app count.
//!
//! The [`AppLookup`] trait is the fetcher seam. [`CurlAppLookup`] is the live
//! implementation (the same system-`curl` convention as the app-ads.txt
//! fetch), and tests script the trait directly.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::app_identity;

/// The public iTunes lookup endpoint.
pub const ITUNES_LOOKUP_URL: &str = "https://itunes.apple.com/lookup";
/// The public Google Play app details endpoint.
pub const GOOGLE_PLAY_DETAILS_URL: &str = "https://play.google.com/store/apps/details";
/// The public Google Play developer catalogue endpoint.
pub const GOOGLE_PLAY_DEVELOPER_URL: &str = "https://play.google.com/store/apps/dev";
/// Default timeout of one lookup, in seconds.
pub const DEFAULT_APP_LOOKUP_TIMEOUT_SECS: u32 = 8;
/// The lookup service's own user agent.
pub const DEFAULT_APP_LOOKUP_USER_AGENT: &str = "ApexWatch/1.0 (+https://apexmediation.ee)";
/// The marker separating the body from curl's status output.
pub const LOOKUP_STATUS_MARKER: &str = "\n@@apex-watch-lookup-status@@";

/// What one App Store lookup established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppLookupResult {
    /// The store returned a public listing for the id.
    Verified,
    /// The store answered definitively that it has no such listing (an empty
    /// result or a 404). The app is not verified.
    NotFound,
    /// The lookup could not answer (network, timeout, an unexpected HTTP
    /// status or an unreadable body). The app is not verified and the caller
    /// may keep a previous verified tier instead of downgrading on a transient
    /// failure.
    Unavailable(String),
}

/// One store's developer catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreCatalogue {
    /// The public store name (`app_store` or `google_play`).
    pub store: &'static str,
    /// The developer id the catalogue was read from.
    pub developer: String,
    /// Number of public listings in the catalogue.
    pub count: u32,
}

/// The catalogue evidence for one confirmed app.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Portfolio {
    /// The App Store developer catalogue, when the app resolved there.
    pub app_store: Option<StoreCatalogue>,
    /// The Google Play developer catalogue, when the app resolved there.
    pub google_play: Option<StoreCatalogue>,
    /// Why a catalogue could not be read.
    pub error: Option<String>,
}

impl Portfolio {
    /// The larger of the store catalogues the lookups answered. Cross-store
    /// duplicates are not identifiable from bundle/package ids, so summing
    /// would charge a six-app cross-platform publisher as twelve; the larger
    /// catalogue still closes the Android-only undercount. The named
    /// confirmations remain the floor when a catalogue cannot be read.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.app_store
            .as_ref()
            .map(|c| c.count)
            .unwrap_or(0)
            .max(self.google_play.as_ref().map(|c| c.count).unwrap_or(0))
    }
}

/// The injected App Store lookup seam.
#[async_trait::async_trait]
pub trait AppLookup: Send + Sync + fmt::Debug {
    /// Look one submitted app line up against the public store.
    async fn lookup(&self, app: &str) -> AppLookupResult;

    /// Read the developer catalogue behind a confirmed app. A lookup
    /// implementation that cannot read catalogues returns the default; the
    /// tier then falls back to the confirmed named apps.
    async fn portfolio(&self, _app: &str) -> Portfolio {
        Portfolio::default()
    }
}

/// The verdict of verifying one subscriber's submitted apps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppVerification {
    /// The distinct submitted lines the store confirmed.
    pub verified: Vec<String>,
    /// The distinct submitted lines the store did not confirm.
    pub unverified: Vec<String>,
    /// `verified.len()`, the only count that may decide the tier.
    pub verified_count: u32,
    /// The first network/timeout/unreadable-answer reason, when the lookup
    /// could not answer at all. A definite "no such listing" is not an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Distinct apps not looked up because the bound was reached. The tier
    /// only needs ten confirmed listings, so the lookup stops at twenty
    /// candidates or ten confirmations, whichever comes first.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub skipped: u32,
    /// The whole App Store catalogue of the first confirmed app's developer.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub app_store_catalogue_count: u32,
    /// The whole Google Play catalogue of the first confirmed app's
    /// developer. Android package ids are confirmed through Play, not the
    /// iTunes API.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub google_play_catalogue_count: u32,
    /// The authoritative portfolio size used for the tier: the developer
    /// catalogues when they answered, otherwise the confirmed named apps.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub portfolio_count: u32,
    /// Why the developer catalogue could not be read, when it could not.
    /// This never downgrades a named-app confirmation by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portfolio_error: Option<String>,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl AppVerification {
    /// A verification with no apps to check.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The honest state when no lookup seam is configured: nothing is
    /// verified, and the caller records why.
    #[must_use]
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            verified: Vec::new(),
            unverified: Vec::new(),
            verified_count: 0,
            error: Some(reason.into()),
            skipped: 0,
            app_store_catalogue_count: 0,
            google_play_catalogue_count: 0,
            portfolio_count: 0,
            portfolio_error: None,
        }
    }

    /// Whether the lookup could not answer for at least one app.
    #[must_use]
    pub fn was_unavailable(&self) -> bool {
        self.error.is_some()
    }

    /// The count that decides the tier. A whole developer catalogue wins;
    /// legacy rows and catalogues that could not be read fall back to the
    /// confirmed named apps.
    #[must_use]
    pub fn tier_count(&self) -> u32 {
        self.portfolio_count.max(self.verified_count)
    }
}

/// Verify the distinct named apps in `apps`. Duplicates collapse through
/// [`app_identity`]; only a confirmed public listing is counted.
pub async fn verify_apps(apps: &[String], lookup: &dyn AppLookup) -> AppVerification {
    let mut seen = std::collections::BTreeSet::new();
    let mut distinct = Vec::new();
    for app in apps {
        let trimmed = app.trim();
        let identity = app_identity(trimmed);
        if !identity.is_empty() && seen.insert(identity) {
            distinct.push(trimmed.to_string());
        }
    }
    // Bound the network work a submission can cause: ten confirmed listings
    // already decide the Core tier, and twenty candidates are enough of a
    // head start to find them. Anything beyond is not looked up.
    let limit = (crate::model::CORE_TIER_MIN_APP_COUNT as usize)
        .saturating_mul(2)
        .max(1);
    let mut verdicts = Vec::with_capacity(distinct.len().min(limit));
    let mut processed = 0usize;
    for chunk in distinct.chunks(APP_LOOKUP_CONCURRENCY.max(1)) {
        if processed >= limit {
            break;
        }
        let take = chunk.len().min(limit - processed);
        let chunk = &chunk[..take];
        verdicts.extend(lookup_chunk(chunk, lookup).await);
        processed += take;
        let confirmed = verdicts
            .iter()
            .filter(|verdict| matches!(verdict, AppLookupResult::Verified))
            .count();
        if confirmed >= crate::model::CORE_TIER_MIN_APP_COUNT as usize {
            break;
        }
    }
    let skipped = u32::try_from(distinct.len().saturating_sub(verdicts.len())).unwrap_or(u32::MAX);
    let mut verified = Vec::new();
    let mut unverified = Vec::new();
    let mut error = None;
    for (app, verdict) in distinct.into_iter().take(verdicts.len()).zip(verdicts) {
        match verdict {
            AppLookupResult::Verified => verified.push(app),
            AppLookupResult::NotFound => unverified.push(app),
            AppLookupResult::Unavailable(reason) => {
                if error.is_none() {
                    error = Some(reason);
                }
                unverified.push(app);
            }
        }
    }
    // One confirmed app is enough to read the developer's whole catalogue.
    // One confirmation is enough to read the first developer catalogue; the
    // named-app count remains the floor when the catalogue cannot be read.
    let mut portfolio = Portfolio::default();
    let needs_catalogue = (verified.len() as u32) < crate::model::CORE_TIER_MIN_APP_COUNT;
    if needs_catalogue {
        // Look through up to four confirmations: a publisher may name several
        // iOS apps before the one Android app that carries the Play catalogue.
        for app in verified.iter().take(4) {
            let evidence = lookup.portfolio(app).await;
            if let Some(catalogue) = evidence.app_store {
                if portfolio
                    .app_store
                    .as_ref()
                    .is_none_or(|current| catalogue.count > current.count)
                {
                    portfolio.app_store = Some(catalogue);
                }
            }
            if let Some(catalogue) = evidence.google_play {
                if portfolio
                    .google_play
                    .as_ref()
                    .is_none_or(|current| catalogue.count > current.count)
                {
                    portfolio.google_play = Some(catalogue);
                }
            }
            if portfolio.error.is_none() {
                portfolio.error = evidence.error;
            }
            if portfolio.app_store.is_some() && portfolio.google_play.is_some() {
                break;
            }
        }
    }
    let app_store_catalogue_count = portfolio
        .app_store
        .as_ref()
        .map(|catalogue| catalogue.count)
        .unwrap_or(0);
    let google_play_catalogue_count = portfolio
        .google_play
        .as_ref()
        .map(|catalogue| catalogue.count)
        .unwrap_or(0);
    AppVerification {
        verified_count: u32::try_from(verified.len()).unwrap_or(u32::MAX),
        portfolio_count: portfolio.count(),
        app_store_catalogue_count,
        google_play_catalogue_count,
        portfolio_error: portfolio.error,
        verified,
        unverified,
        error,
        skipped,
    }
}

/// The lookup calls in one chunk, in submission order. Four concurrent
/// lookups keep a fifty-app submission bounded without spawning a task per
/// app or making the result order depend on network timing.
const APP_LOOKUP_CONCURRENCY: usize = 4;

async fn lookup_chunk(chunk: &[String], lookup: &dyn AppLookup) -> Vec<AppLookupResult> {
    match chunk.len() {
        1 => vec![lookup.lookup(&chunk[0]).await],
        2 => {
            let (a, b) = tokio::join!(lookup.lookup(&chunk[0]), lookup.lookup(&chunk[1]));
            vec![a, b]
        }
        3 => {
            let (a, b, c) = tokio::join!(
                lookup.lookup(&chunk[0]),
                lookup.lookup(&chunk[1]),
                lookup.lookup(&chunk[2])
            );
            vec![a, b, c]
        }
        4 => {
            let (a, b, c, d) = tokio::join!(
                lookup.lookup(&chunk[0]),
                lookup.lookup(&chunk[1]),
                lookup.lookup(&chunk[2]),
                lookup.lookup(&chunk[3])
            );
            vec![a, b, c, d]
        }
        _ => {
            let mut verdicts = Vec::with_capacity(chunk.len());
            for app in chunk {
                verdicts.push(lookup.lookup(app).await);
            }
            verdicts
        }
    }
}

/// The iTunes lookup URL for one submitted line.
///
/// A bare bundle id is looked up by `bundleId`. An App Store app URL (the
/// other accepted form) names the numeric store id (`.../id919745514`), which
/// the same endpoint accepts as `id`. Anything that is neither a URL nor a
/// numeric id is treated as a bundle id; the store simply answers no result
/// for an id it cannot confirm.
#[must_use]
pub fn lookup_url(app: &str) -> String {
    let app = app.trim();
    let (parameter, value) = match split_lookup_target(app) {
        LookupTarget::BundleId(id) => ("bundleId", id),
        LookupTarget::StoreId(id) => ("id", id),
    };
    format!("{ITUNES_LOOKUP_URL}?{parameter}={}", percent_encode(value))
}

/// Which query parameter one line names.
enum LookupTarget<'a> {
    BundleId(&'a str),
    StoreId(&'a str),
}

fn split_lookup_target(app: &str) -> LookupTarget<'_> {
    let app = app.split('#').next().unwrap_or(app);
    if let Some((scheme, rest)) = app.split_once("://") {
        if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
            let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
            if let Some(value) = query_parameter(query, "bundleId") {
                return LookupTarget::BundleId(value);
            }
            if let Some(value) = query_parameter(query, "id") {
                return LookupTarget::StoreId(value);
            }
            if let Some(value) = path
                .split('/')
                .rev()
                .find_map(|segment| segment.strip_prefix("id"))
                .filter(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                })
            {
                return LookupTarget::StoreId(value);
            }
            let last = path.rsplit('/').find(|segment| !segment.is_empty());
            return LookupTarget::BundleId(last.unwrap_or(app));
        }
    }
    if !app.is_empty() && app.bytes().all(|byte| byte.is_ascii_digit()) {
        return LookupTarget::StoreId(app);
    }
    LookupTarget::BundleId(app)
}

/// One query parameter's raw value, before percent-decoding (the ids the
/// form accepts are already plain ASCII).
fn query_parameter<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        let value = value.split('#').next().unwrap_or(value);
        (key == name && !value.is_empty()).then_some(value)
    })
}

/// Percent-encode one query value. Bundle ids and numeric ids are already
/// safe; the escaping keeps a hostile line from injecting a second parameter.
fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

/// The live lookup: the system `curl`, with a bounded timeout, a size cap and
/// the service's user agent. The child environment is cleared so no vault key
/// or token reaches it, and the URL is passed after `--`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurlAppLookup {
    /// Path to `curl`.
    pub binary: String,
    /// Timeout in seconds.
    pub timeout_secs: u32,
    /// User-Agent sent.
    pub user_agent: String,
}

impl Default for CurlAppLookup {
    fn default() -> Self {
        Self {
            binary: resolve_binary("curl"),
            timeout_secs: DEFAULT_APP_LOOKUP_TIMEOUT_SECS,
            user_agent: DEFAULT_APP_LOOKUP_USER_AGENT.to_string(),
        }
    }
}

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

#[async_trait::async_trait]
impl AppLookup for CurlAppLookup {
    async fn lookup(&self, app: &str) -> AppLookupResult {
        let app = app.to_string();
        let binary = self.binary.clone();
        let timeout = self.timeout_secs.to_string();
        let user_agent = self.user_agent.clone();
        let joined = tokio::task::spawn_blocking(move || {
            run_live_lookup(&binary, &timeout, &user_agent, &app)
        })
        .await;
        joined.unwrap_or_else(|error| {
            AppLookupResult::Unavailable(format!("the lookup worker failed: {error}"))
        })
    }

    async fn portfolio(&self, app: &str) -> Portfolio {
        let app = app.to_string();
        let binary = self.binary.clone();
        let timeout = self.timeout_secs.to_string();
        let user_agent = self.user_agent.clone();
        tokio::task::spawn_blocking(move || run_portfolio(&binary, &timeout, &user_agent, &app))
            .await
            .unwrap_or_else(|error| Portfolio {
                error: Some(format!("the portfolio worker failed: {error}")),
                ..Portfolio::default()
            })
    }
}

fn run_curl_lookup(binary: &str, timeout: &str, user_agent: &str, url: &str) -> AppLookupResult {
    match curl_fetch(binary, timeout, user_agent, url) {
        Ok((status, body)) => classify_lookup(status, &body),
        Err(reason) => AppLookupResult::Unavailable(reason),
    }
}

fn curl_fetch(
    binary: &str,
    timeout: &str,
    user_agent: &str,
    url: &str,
) -> Result<(u16, Vec<u8>), String> {
    let output = std::process::Command::new(binary)
        .env_clear()
        .args([
            "--silent",
            "--show-error",
            "--compressed",
            "--max-time",
            timeout,
            "--max-filesize",
            "4194304",
            "--user-agent",
            user_agent,
            "--write-out",
            &format!("{LOOKUP_STATUS_MARKER}%{{http_code}}"),
            "--",
            url,
        ])
        .output()
        .map_err(|error| format!("curl could not run: {error}"))?;
    if !output.status.success() {
        let detail: String = String::from_utf8_lossy(&output.stderr)
            .trim()
            .chars()
            .take(300)
            .collect();
        let timed_out = output.status.code() == Some(28)
            || detail.to_ascii_lowercase().contains("timed out")
            || detail.to_ascii_lowercase().contains("timeout");
        return Err(if timed_out {
            format!("the lookup timed out: {detail}")
        } else if detail.is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            format!("the lookup transport failed: {detail}")
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body, status) = match stdout.rsplit_once(LOOKUP_STATUS_MARKER) {
        Some((body, status)) => (body, status.trim()),
        None => return Err("no status from the lookup".to_string()),
    };
    let status: u16 = status
        .parse()
        .map_err(|_| "bad status from the lookup".to_string())?;
    Ok((status, body.as_bytes().to_vec()))
}

/// Combine the two store answers without turning a transient failure into a
/// definite absence. NotFound is reported only when both stores answered and
/// neither listed the app; any Unavailable keeps the previous tier.
pub fn merge_store_answers(
    apple: AppLookupResult,
    play: Option<AppLookupResult>,
) -> AppLookupResult {
    if apple == AppLookupResult::Verified || play.as_ref() == Some(&AppLookupResult::Verified) {
        return AppLookupResult::Verified;
    }
    match (apple, play) {
        (AppLookupResult::NotFound, Some(AppLookupResult::NotFound)) => AppLookupResult::NotFound,
        (AppLookupResult::Unavailable(reason), _) => AppLookupResult::Unavailable(reason),
        (_, Some(AppLookupResult::Unavailable(reason))) => AppLookupResult::Unavailable(reason),
        (other, _) => other,
    }
}

/// The live lookup: Apple first, then Google Play for a package id the
/// iTunes API does not know. A Play miss after an Apple miss is a definite
/// "no listing"; a transport failure in either store is unavailable.
fn run_live_lookup(binary: &str, timeout: &str, user_agent: &str, app: &str) -> AppLookupResult {
    let apple = run_curl_lookup(binary, timeout, user_agent, &lookup_url(app));
    if apple == AppLookupResult::Verified {
        return apple;
    }
    let Some(package) = google_play_package(app) else {
        return apple;
    };
    let play = match curl_fetch(
        binary,
        timeout,
        user_agent,
        &google_play_details_url(&package),
    ) {
        Ok((200, body)) if google_play_listing_exists(&String::from_utf8_lossy(&body)) => {
            AppLookupResult::Verified
        }
        Ok((404 | 410, _)) => AppLookupResult::NotFound,
        Ok((200, _)) => AppLookupResult::Unavailable(
            "the Play listing answered 200 without a listing title (consent or bot shell)"
                .to_string(),
        ),
        Ok((status, _)) => {
            AppLookupResult::Unavailable(format!("the Play listing answered HTTP {status}"))
        }
        Err(reason) => AppLookupResult::Unavailable(reason),
    };
    merge_store_answers(apple, Some(play))
}

fn run_portfolio(binary: &str, timeout: &str, user_agent: &str, app: &str) -> Portfolio {
    let mut portfolio = Portfolio::default();
    let apple = curl_fetch(binary, timeout, user_agent, &lookup_url(app));
    if let Ok((200, body)) = &apple {
        if let Some(artist) = app_store_artist_id(body) {
            match curl_fetch(
                binary,
                timeout,
                user_agent,
                &app_store_catalogue_url(artist),
            ) {
                Ok((200, body)) => {
                    portfolio.app_store =
                        app_store_catalogue_count(&body).map(|count| StoreCatalogue {
                            store: "app_store",
                            developer: artist.to_string(),
                            count,
                        });
                }
                Ok(_) => {}
                Err(reason) => portfolio.error = Some(reason),
            }
        }
    }
    let Some(package) = google_play_package(app) else {
        if portfolio.app_store.is_none() && portfolio.error.is_none() {
            portfolio.error = Some("the app resolved in neither store".to_string());
        }
        return portfolio;
    };
    let details = match curl_fetch(
        binary,
        timeout,
        user_agent,
        &google_play_details_url(&package),
    ) {
        Ok((200, body)) if google_play_listing_exists(&String::from_utf8_lossy(&body)) => {
            String::from_utf8_lossy(&body).into_owned()
        }
        Ok((200, _)) => {
            if portfolio.error.is_none() {
                portfolio.error =
                    Some("the Play listing answered 200 without a listing title".to_string());
            }
            return portfolio;
        }
        Ok(_) => {
            if portfolio.app_store.is_none() && portfolio.error.is_none() {
                portfolio.error = Some("the Play listing is not public".to_string());
            }
            return portfolio;
        }
        Err(reason) => {
            if portfolio.error.is_none() {
                portfolio.error = Some(reason);
            }
            return portfolio;
        }
    };
    let developer = google_play_developer_id(&details);
    let count = match developer.as_deref() {
        Some(developer) => match curl_fetch(
            binary,
            timeout,
            user_agent,
            &google_play_developer_url(developer),
        ) {
            Ok((200, body)) => Some(google_play_catalogue_count(&String::from_utf8_lossy(&body))),
            Ok((status, _)) => {
                if portfolio.error.is_none() {
                    portfolio.error = Some(format!(
                        "the Play developer catalogue answered HTTP {status}"
                    ));
                }
                None
            }
            Err(reason) => {
                if portfolio.error.is_none() {
                    portfolio.error = Some(reason);
                }
                None
            }
        },
        None => Some(1),
    };
    if let Some(count) = count {
        portfolio.google_play = Some(StoreCatalogue {
            store: "google_play",
            developer: developer.unwrap_or_else(|| package.clone()),
            count: count.max(1),
        });
    }
    portfolio
}

/// Classify one lookup answer. A 200 with a non-empty `results` array is a
/// public listing; an empty result and a 404 are definite "no listing"; every
/// other status is an unavailable answer (never a confirmation).
#[must_use]
pub fn classify_lookup(status: u16, body: &[u8]) -> AppLookupResult {
    match status {
        200 => match serde_json::from_slice::<LookupResponse>(body) {
            Ok(response) if response.result_count > 0 => AppLookupResult::Verified,
            Ok(_) => AppLookupResult::NotFound,
            Err(error) => AppLookupResult::Unavailable(format!(
                "the lookup response is not readable: {error}"
            )),
        },
        404 | 410 => AppLookupResult::NotFound,
        other => AppLookupResult::Unavailable(format!("the lookup answered HTTP {other}")),
    }
}

#[derive(Debug, Deserialize)]
struct LookupResponse {
    #[serde(rename = "resultCount", default)]
    result_count: u64,
    #[serde(default)]
    results: Vec<LookupResult>,
}

#[derive(Debug, Deserialize)]
struct LookupResult {
    #[serde(rename = "artistId", default)]
    artist_id: Option<u64>,
}

/// The App Store developer id named by an app lookup answer.
#[must_use]
pub fn app_store_artist_id(body: &[u8]) -> Option<u64> {
    serde_json::from_slice::<LookupResponse>(body)
        .ok()?
        .results
        .first()?
        .artist_id
}

/// The iTunes URL for one developer's whole App Store catalogue.
#[must_use]
pub fn app_store_catalogue_url(artist_id: u64) -> String {
    format!("{ITUNES_LOOKUP_URL}?id={artist_id}&entity=software&limit=200")
}

/// The number of public listings in an App Store developer-catalogue answer.
#[must_use]
pub fn app_store_catalogue_count(body: &[u8]) -> Option<u32> {
    let response = serde_json::from_slice::<LookupResponse>(body).ok()?;
    u32::try_from(response.result_count).ok()
}

/// A Google Play package id from a package id, an Android package URL, or a
/// Play details URL. App Store URLs, numeric ids, and arbitrary prose are
/// not Play targets.
#[must_use]
pub fn google_play_package(app: &str) -> Option<String> {
    let app = app.trim();
    if let Some((scheme, rest)) = app.split_once("://") {
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return None;
        }
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        if !path
            .to_ascii_lowercase()
            .starts_with("play.google.com/store/apps/")
        {
            return None;
        }
        return query_parameter(query, "id").map(str::to_string);
    }
    let package = app.strip_prefix("id=").unwrap_or(app);
    let parts: Vec<&str> = package.split('.').collect();
    (parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        }))
    .then(|| package.to_string())
}

/// The Play details URL for one package.
#[must_use]
pub fn google_play_details_url(package: &str) -> String {
    format!(
        "{GOOGLE_PLAY_DETAILS_URL}?id={}&hl=en&gl=US",
        percent_encode(package)
    )
}

/// The Play developer id named by a details page, if the page exposes one.
#[must_use]
pub fn google_play_developer_id(html: &str) -> Option<String> {
    // Parse only <a> tags. A <link>, a script, or a description that merely
    // mentions the path cannot redirect the catalogue read.
    let mut rest = html;
    while let Some(index) = rest.find("<a") {
        rest = &rest[index..];
        let tag_end = rest.find('>')?;
        let tag = &rest[..tag_end];
        for marker in [
            "href=\"/store/apps/dev?id=",
            "href='/store/apps/dev?id=",
            "href=\"https://play.google.com/store/apps/dev?id=",
            "href='https://play.google.com/store/apps/dev?id=",
            "href=\"/store/apps/developer?id=",
            "href='/store/apps/developer?id=",
        ] {
            if let Some(at) = tag.find(marker) {
                let value: String = tag[at + marker.len()..]
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
                    .collect();
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
        rest = &rest[tag_end + 1..];
    }
    None
}

/// The Play developer catalogue URL.
#[must_use]
pub fn google_play_developer_url(developer: &str) -> String {
    format!(
        "{GOOGLE_PLAY_DEVELOPER_URL}?id={}&hl=en&gl=US",
        percent_encode(developer)
    )
}

/// Count the distinct public packages linked from the first Play developer
/// page. This is a lower bound: the page can paginate. Ten visible listings
/// already decide the core threshold; a smaller count is not upgraded on
/// assumption. A details page with no developer link is by definition a
/// one-app catalogue, but the caller only uses this after finding the link.
#[must_use]
pub fn google_play_catalogue_count(html: &str) -> u32 {
    let mut packages = std::collections::BTreeSet::new();
    for marker in ["/store/apps/details?id=", "details?id="] {
        let mut rest = html;
        while let Some(index) = rest.find(marker) {
            rest = &rest[index + marker.len()..];
            let package: String = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
                .collect();
            if !package.is_empty() && package.contains('.') {
                packages.insert(package);
            }
        }
    }
    u32::try_from(packages.len()).unwrap_or(u32::MAX)
}

/// Whether a Play details body is a real app listing rather than a block or
/// a 404 shell.
#[must_use]
pub fn google_play_listing_exists(html: &str) -> bool {
    html.contains("itemprop=\"name\"") || html.contains("property=\"og:title\"")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Debug)]
    struct ScriptedLookup {
        answers: HashMap<String, AppLookupResult>,
        calls: Mutex<Vec<String>>,
    }

    impl ScriptedLookup {
        fn new(answers: Vec<(&str, AppLookupResult)>) -> Self {
            Self {
                answers: answers
                    .into_iter()
                    .map(|(app, answer)| (app.to_string(), answer))
                    .collect(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl AppLookup for ScriptedLookup {
        async fn lookup(&self, app: &str) -> AppLookupResult {
            self.calls.lock().unwrap().push(app.to_string());
            self.answers
                .get(app)
                .cloned()
                .unwrap_or(AppLookupResult::NotFound)
        }
    }

    async fn verify(apps: &[&str], lookup: &ScriptedLookup) -> AppVerification {
        let apps: Vec<String> = apps.iter().map(|app| (*app).to_string()).collect();
        verify_apps(&apps, lookup).await
    }

    #[test]
    fn lookup_urls_name_the_bundle_id_or_the_numeric_store_id() {
        assert_eq!(
            lookup_url("com.imvu.mobilecordova"),
            "https://itunes.apple.com/lookup?bundleId=com.imvu.mobilecordova"
        );
        assert_eq!(
            lookup_url("919745514"),
            "https://itunes.apple.com/lookup?id=919745514"
        );
        assert_eq!(
            lookup_url("https://apps.apple.com/us/app/imvu/id919745514"),
            "https://itunes.apple.com/lookup?id=919745514"
        );
        assert_eq!(
            lookup_url("https://apps.apple.com/app/x?id=42"),
            "https://itunes.apple.com/lookup?id=42"
        );
        assert_eq!(
            lookup_url("https://itunes.apple.com/lookup?bundleId=com.spotify.client"),
            "https://itunes.apple.com/lookup?bundleId=com.spotify.client"
        );
        assert_eq!(
            lookup_url("bad id&bundleId=evil"),
            "https://itunes.apple.com/lookup?bundleId=bad%20id%26bundleId%3Devil"
        );
    }

    #[test]
    fn a_200_with_results_is_verified_and_empty_or_404_answers_are_not() {
        assert_eq!(
            classify_lookup(200, br#"{"resultCount":1,"results":[{"trackName":"X"}]}"#),
            AppLookupResult::Verified
        );
        assert_eq!(
            classify_lookup(200, br#"{"resultCount":0,"results":[]}"#),
            AppLookupResult::NotFound
        );
        assert_eq!(classify_lookup(404, b""), AppLookupResult::NotFound);
        assert!(matches!(
            classify_lookup(500, b""),
            AppLookupResult::Unavailable(_)
        ));
        assert!(matches!(
            classify_lookup(200, b"not json"),
            AppLookupResult::Unavailable(_)
        ));
    }

    #[tokio::test]
    async fn a_fifty_app_submission_is_bounded_to_twenty_lookups() {
        let lookup = ScriptedLookup::new(Vec::new());
        let apps: Vec<String> = (0..50)
            .map(|index| format!("com.example.app{index}"))
            .collect();
        let verification = verify_apps(&apps, &lookup).await;
        assert_eq!(lookup.calls().len(), 20, "the lookup is capped");
        assert_eq!(verification.skipped, 30);
        assert_eq!(verification.verified_count, 0);
        assert_eq!(
            crate::model::tier_from_apps(verification.tier_count()),
            crate::model::Tier::Normal
        );
    }

    #[tokio::test]
    async fn ten_confirmations_stop_the_lookup_early() {
        let mut answers = Vec::new();
        for index in 0..10 {
            answers.push((
                Box::leak(format!("com.example.app{index}").into_boxed_str()) as &str,
                AppLookupResult::Verified,
            ));
        }
        let lookup = ScriptedLookup::new(answers);
        let apps: Vec<String> = (0..50)
            .map(|index| format!("com.example.app{index}"))
            .collect();
        let verification = verify_apps(&apps, &lookup).await;
        assert_eq!(verification.verified_count, 10);
        assert_eq!(
            lookup.calls().len(),
            12,
            "three chunks of four reach the ten confirmations"
        );
        assert_eq!(verification.skipped, 38);
    }

    #[test]
    fn store_failures_never_become_a_definite_absence() {
        use AppLookupResult::{NotFound, Unavailable, Verified};
        assert_eq!(
            merge_store_answers(NotFound, Some(NotFound)),
            NotFound,
            "both stores must answer before a listing is called absent"
        );
        assert_eq!(
            merge_store_answers(NotFound, Some(Unavailable("play timeout".into()))),
            Unavailable("play timeout".into())
        );
        assert_eq!(
            merge_store_answers(Unavailable("apple timeout".into()), Some(NotFound)),
            Unavailable("apple timeout".into())
        );
        assert_eq!(
            merge_store_answers(Unavailable("apple timeout".into()), Some(Verified)),
            Verified
        );
    }

    #[test]
    fn zero_is_the_only_skipped_field() {
        assert!(is_zero(&0));
        assert!(!is_zero(&1));
    }

    #[test]
    fn developer_catalogues_are_parsed_from_both_stores() {
        let apple = br#"{"resultCount":1,"results":[{"artistId":42,"trackName":"One"}]}"#;
        assert_eq!(app_store_artist_id(apple), Some(42));
        assert_eq!(
            app_store_catalogue_url(42),
            "https://itunes.apple.com/lookup?id=42&entity=software&limit=200"
        );
        assert_eq!(
            app_store_catalogue_count(br#"{"resultCount":17,"results":[]}"#),
            Some(17)
        );

        assert_eq!(
            google_play_package("com.example.game"),
            Some("com.example.game".to_string())
        );
        assert_eq!(
            google_play_package(
                "https://play.google.com/store/apps/details?id=com.example.game&hl=en"
            ),
            Some("com.example.game".to_string())
        );
        assert_eq!(google_play_package("https://apps.apple.com/app/id42"), None);
        assert_eq!(
            google_play_package(
                "https://PLAY.GOOGLE.COM/store/apps/details?id=com.example.game#reviews"
            ),
            Some("com.example.game".to_string())
        );
        assert_eq!(google_play_package("919745514"), None);
        assert_eq!(
            lookup_url("https://apps.apple.com/app/x?id=42#reviews"),
            "https://itunes.apple.com/lookup?id=42"
        );

        let details =
            r#"<meta content="/store/apps/dev?id=tiny"><a href="/store/apps/dev?id=Dev_1">Dev</a>"#;
        assert_eq!(
            google_play_developer_id(details),
            Some("Dev_1".to_string()),
            "a description marker cannot choose the catalogue"
        );
        assert_eq!(
            google_play_developer_url("Dev_1"),
            "https://play.google.com/store/apps/dev?id=Dev_1&hl=en&gl=US"
        );
        let catalogue = r#"
            <a href="/store/apps/details?id=com.example.one">1</a>
            <a href="/store/apps/details?id=com.example.two&amp;hl=en">2</a>
            <a href="/store/apps/details?id=com.example.one">dup</a>
        "#;
        assert_eq!(google_play_catalogue_count(catalogue), 2);
        assert!(google_play_listing_exists(
            r#"<meta property="og:title" content="Game">"#
        ));
        assert!(!google_play_listing_exists("<html>not found</html>"));
    }

    #[tokio::test]
    async fn a_developer_catalogue_sets_the_tier_from_one_confirmed_app() {
        #[derive(Debug)]
        struct CatalogueLookup;
        #[async_trait::async_trait]
        impl AppLookup for CatalogueLookup {
            async fn lookup(&self, _app: &str) -> AppLookupResult {
                AppLookupResult::Verified
            }
            async fn portfolio(&self, _app: &str) -> Portfolio {
                Portfolio {
                    app_store: Some(StoreCatalogue {
                        store: "app_store",
                        developer: "42".to_string(),
                        count: 12,
                    }),
                    google_play: Some(StoreCatalogue {
                        store: "google_play",
                        developer: "Dev_1".to_string(),
                        count: 4,
                    }),
                    error: None,
                }
            }
        }
        let verification = verify_apps(&["com.example.one".to_string()], &CatalogueLookup).await;
        assert_eq!(verification.verified_count, 1);
        assert_eq!(verification.app_store_catalogue_count, 12);
        assert_eq!(verification.google_play_catalogue_count, 4);
        assert_eq!(verification.portfolio_count, 12, "larger store catalogue");
        assert_eq!(verification.tier_count(), 12);
        assert_eq!(
            crate::model::tier_from_apps(verification.tier_count()),
            crate::model::Tier::Core,
            "one named app can prove a core portfolio through its developer catalogue"
        );
    }

    // dogfood:skip: network-dependent store probe, run explicitly with --ignored.
    #[tokio::test]
    #[ignore = "live App Store / Google Play probe; run explicitly"]
    async fn live_catalogues_answer_for_a_real_android_app() {
        let lookup = CurlAppLookup::default();
        let verification = verify_apps(&["com.rovio.baba".to_string()], &lookup).await;
        assert!(
            verification.portfolio_count > 0,
            "a real Play package must expose its developer catalogue: {verification:?}"
        );
        assert!(
            verification.google_play_catalogue_count >= 10,
            "Rovio's Play catalogue should reach the core threshold: {verification:?}"
        );
    }

    #[tokio::test]
    async fn one_android_package_can_prove_a_core_catalogue() {
        #[derive(Debug)]
        struct PlayLookup;
        #[async_trait::async_trait]
        impl AppLookup for PlayLookup {
            async fn lookup(&self, _app: &str) -> AppLookupResult {
                AppLookupResult::Verified
            }
            async fn portfolio(&self, _app: &str) -> Portfolio {
                Portfolio {
                    app_store: None,
                    google_play: Some(StoreCatalogue {
                        store: "google_play",
                        developer: "Dev_1".to_string(),
                        count: 11,
                    }),
                    error: None,
                }
            }
        }
        let verification = verify_apps(&["com.example.game".to_string()], &PlayLookup).await;
        assert_eq!(verification.google_play_catalogue_count, 11);
        assert_eq!(verification.app_store_catalogue_count, 0);
        assert_eq!(verification.tier_count(), 11);
        assert_eq!(
            crate::model::tier_from_apps(verification.tier_count()),
            crate::model::Tier::Core
        );
    }

    #[tokio::test]
    async fn a_second_app_is_read_when_it_carries_the_other_store() {
        #[derive(Debug)]
        struct TwoStoreLookup;
        #[async_trait::async_trait]
        impl AppLookup for TwoStoreLookup {
            async fn lookup(&self, _app: &str) -> AppLookupResult {
                AppLookupResult::Verified
            }
            async fn portfolio(&self, app: &str) -> Portfolio {
                if app == "ios.example" {
                    Portfolio {
                        app_store: Some(StoreCatalogue {
                            store: "app_store",
                            developer: "42".to_string(),
                            count: 12,
                        }),
                        ..Portfolio::default()
                    }
                } else {
                    Portfolio {
                        google_play: Some(StoreCatalogue {
                            store: "google_play",
                            developer: "Dev_1".to_string(),
                            count: 11,
                        }),
                        ..Portfolio::default()
                    }
                }
            }
        }
        let verification = verify_apps(
            &["ios.example".to_string(), "android.example".to_string()],
            &TwoStoreLookup,
        )
        .await;
        assert_eq!(verification.app_store_catalogue_count, 12);
        assert_eq!(verification.google_play_catalogue_count, 11);
        assert_eq!(verification.tier_count(), 12, "the larger catalogue wins");
    }

    #[tokio::test]
    async fn an_android_app_is_not_missed_after_several_ios_apps() {
        #[derive(Debug)]
        struct LaterAndroid;
        #[async_trait::async_trait]
        impl AppLookup for LaterAndroid {
            async fn lookup(&self, _app: &str) -> AppLookupResult {
                AppLookupResult::Verified
            }
            async fn portfolio(&self, app: &str) -> Portfolio {
                if app == "android.example" {
                    Portfolio {
                        google_play: Some(StoreCatalogue {
                            store: "google_play",
                            developer: "Dev_1".to_string(),
                            count: 11,
                        }),
                        ..Portfolio::default()
                    }
                } else {
                    Portfolio {
                        app_store: Some(StoreCatalogue {
                            store: "app_store",
                            developer: "42".to_string(),
                            count: 2,
                        }),
                        ..Portfolio::default()
                    }
                }
            }
        }
        let verification = verify_apps(
            &[
                "ios1.example".to_string(),
                "ios2.example".to_string(),
                "ios3.example".to_string(),
                "android.example".to_string(),
            ],
            &LaterAndroid,
        )
        .await;
        assert_eq!(verification.google_play_catalogue_count, 11);
        assert_eq!(verification.tier_count(), 11);
    }

    #[tokio::test]
    async fn only_verified_distinct_apps_are_counted() {
        let lookup = ScriptedLookup::new(vec![
            ("com.example.app1", AppLookupResult::Verified),
            ("com.example.app2", AppLookupResult::Verified),
            ("com.example.app3", AppLookupResult::Verified),
        ]);
        let verification = verify(
            &[
                "com.example.app1",
                " COM.EXAMPLE.APP1 ",
                "com.example.app2",
                "com.example.app3",
                "com.example.missing",
                "com.example.androidonly",
            ],
            &lookup,
        )
        .await;
        assert_eq!(verification.verified_count, 3);
        assert_eq!(verification.verified.len(), 3);
        assert_eq!(
            verification.unverified,
            vec![
                "com.example.missing".to_string(),
                "com.example.androidonly".to_string()
            ]
        );
        assert_eq!(lookup.calls().len(), 5, "the duplicate is looked up once");
        assert_eq!(lookup.calls()[0], "com.example.app1");
    }

    #[tokio::test]
    async fn non_app_lines_and_empty_answers_are_unverified() {
        let lookup = ScriptedLookup::new(vec![("com.example.real", AppLookupResult::Verified)]);
        let verification = verify(
            &[
                "com.example.real",
                "not an app",
                "https://example.com/marketing",
                "com.example.delisted",
            ],
            &lookup,
        )
        .await;
        assert_eq!(verification.verified_count, 1);
        assert_eq!(
            verification.unverified,
            vec![
                "not an app".to_string(),
                "https://example.com/marketing".to_string(),
                "com.example.delisted".to_string()
            ]
        );
        assert_eq!(verification.error, None, "definite misses are not errors");
    }

    #[tokio::test]
    async fn ten_verified_apps_is_core_and_nine_is_not() {
        let apps: Vec<String> = (1..=10)
            .map(|index| format!("com.example.app{index}"))
            .collect();
        let lookup = ScriptedLookup::new(
            apps.iter()
                .map(|app| (app.as_str(), AppLookupResult::Verified))
                .collect(),
        );
        let verification = verify_apps(&apps, &lookup).await;
        assert_eq!(verification.verified_count, 10);
        assert_eq!(
            crate::model::tier_from_apps(verification.tier_count()),
            crate::model::Tier::Core
        );

        let nine = &apps[..9];
        let verification = verify_apps(nine, &lookup).await;
        assert_eq!(verification.verified_count, 9);
        assert_eq!(
            crate::model::tier_from_apps(verification.tier_count()),
            crate::model::Tier::Normal
        );
    }

    #[tokio::test]
    async fn a_timeout_is_not_a_confirmation_and_is_recorded() {
        let lookup = ScriptedLookup::new(vec![
            ("com.example.app1", AppLookupResult::Verified),
            (
                "com.example.app2",
                AppLookupResult::Unavailable("the lookup timed out".to_string()),
            ),
        ]);
        let verification = verify(&["com.example.app1", "com.example.app2"], &lookup).await;
        assert_eq!(verification.verified_count, 1);
        assert_eq!(
            verification.error.as_deref(),
            Some("the lookup timed out"),
            "the unavailability is recorded honestly"
        );
        assert_eq!(
            verification.unverified,
            vec!["com.example.app2".to_string()]
        );
    }

    #[tokio::test]
    async fn more_than_ten_ids_with_only_some_verified_stays_normal() {
        let mut answers = Vec::new();
        let mut apps = Vec::new();
        for index in 1..=12 {
            let app = format!("com.example.app{index}");
            // Only seven of the twelve are real listings; Android-only ids
            // and mistyped ids answer no result.
            let answer = if index <= 7 {
                AppLookupResult::Verified
            } else {
                AppLookupResult::NotFound
            };
            answers.push((app.clone(), answer));
            apps.push((index, app));
        }
        let lookup = ScriptedLookup::new(
            answers
                .iter()
                .map(|(app, answer)| (app.as_str(), answer.clone()))
                .collect(),
        );
        let verification = verify_apps(
            &apps.iter().map(|(_, app)| app.clone()).collect::<Vec<_>>(),
            &lookup,
        )
        .await;
        assert_eq!(verification.verified_count, 7);
        assert_eq!(
            crate::model::tier_from_apps(verification.tier_count()),
            crate::model::Tier::Normal,
            "twelve submitted ids with seven verified are not core"
        );
        assert!(
            verification.error.is_none(),
            "definite misses are not errors"
        );
    }

    #[test]
    fn curl_lookup_reads_a_scripted_body_and_status() {
        let dir =
            std::env::temp_dir().join(format!("apex-watch-app-lookup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("curl.sh");
        let argv_path = dir.join("argv.txt");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '%s' '{{\"resultCount\":1,\"results\":[{{\"trackName\":\"IMVU\"}}]}}{LOOKUP_STATUS_MARKER}200'\n",
                argv_path.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let lookup = CurlAppLookup {
            binary: script.display().to_string(),
            ..CurlAppLookup::default()
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(lookup.lookup("com.imvu.mobilecordova"));
        assert_eq!(result, AppLookupResult::Verified);
        let argv = std::fs::read_to_string(&argv_path).unwrap();
        assert!(argv.contains("https://itunes.apple.com/lookup?bundleId=com.imvu.mobilecordova"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_curl_is_unavailable_not_verified() {
        let dir =
            std::env::temp_dir().join(format!("apex-watch-app-lookup-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("curl.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'Operation timed out after 8001 milliseconds' >&2\nexit 28\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let lookup = CurlAppLookup {
            binary: script.display().to_string(),
            ..CurlAppLookup::default()
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(lookup.lookup("com.example.app"));
        assert!(
            matches!(result, AppLookupResult::Unavailable(ref reason) if reason.contains("timed out")),
            "{result:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
