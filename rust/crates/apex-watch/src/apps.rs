//! App Store verification of the named apps that decide the subscription tier.
//!
//! The public sign-up form accepts one app-store id or URL per line. Those
//! lines are evidence only when the developer's public App Store listing can
//! be read back: the iTunes lookup API answers a result for a real bundle id
//! and an empty result for anything it cannot confirm (an Android package id,
//! a mistyped id, a delisted app). The submitted line count is therefore never
//! the tier: only the lines the lookup confirms are counted.
//!
//! [`verify_apps`] is the one verification rule. It deduplicates through the
//! same [`crate::model::app_identity`] fold the sign-up parser and the
//! historical tier count use, so ten copies of one id are still one app. A
//! network or timeout failure is recorded as an unavailable lookup rather
//! than treated as a confirmation; the caller keeps the previous verified tier
//! and records the honest "not re-verified" state.
//!
//! The [`AppLookup`] trait is the fetcher seam. [`CurlAppLookup`] is the live
//! implementation (the same system-`curl` convention as the app-ads.txt
//! fetch), and tests script the trait directly.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::app_identity;

/// The public iTunes lookup endpoint.
pub const ITUNES_LOOKUP_URL: &str = "https://itunes.apple.com/lookup";
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

/// The injected App Store lookup seam.
#[async_trait::async_trait]
pub trait AppLookup: Send + Sync + fmt::Debug {
    /// Look one submitted app line up against the public store.
    async fn lookup(&self, app: &str) -> AppLookupResult;
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
        }
    }

    /// Whether the lookup could not answer for at least one app.
    #[must_use]
    pub fn was_unavailable(&self) -> bool {
        self.error.is_some()
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
    AppVerification {
        verified_count: u32::try_from(verified.len()).unwrap_or(u32::MAX),
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
        let url = lookup_url(app);
        let binary = self.binary.clone();
        let timeout = self.timeout_secs.to_string();
        let user_agent = self.user_agent.clone();
        let joined = tokio::task::spawn_blocking(move || {
            run_curl_lookup(&binary, &timeout, &user_agent, &url)
        })
        .await;
        joined.unwrap_or_else(|error| {
            AppLookupResult::Unavailable(format!("the lookup worker failed: {error}"))
        })
    }
}

fn run_curl_lookup(binary: &str, timeout: &str, user_agent: &str, url: &str) -> AppLookupResult {
    let output = std::process::Command::new(binary)
        .env_clear()
        .args([
            "--silent",
            "--show-error",
            "--max-time",
            timeout,
            "--max-filesize",
            "1048576",
            "--user-agent",
            user_agent,
            "--write-out",
            &format!("{LOOKUP_STATUS_MARKER}%{{http_code}}"),
            "--",
            url,
        ])
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => return AppLookupResult::Unavailable(format!("curl could not run: {error}")),
    };
    if !output.status.success() {
        let detail: String = String::from_utf8_lossy(&output.stderr)
            .trim()
            .chars()
            .take(300)
            .collect();
        let timed_out = output.status.code() == Some(28)
            || detail.to_ascii_lowercase().contains("timed out")
            || detail.to_ascii_lowercase().contains("timeout");
        return AppLookupResult::Unavailable(if timed_out {
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
        None => return AppLookupResult::Unavailable("no status from the lookup".to_string()),
    };
    let status: u16 = match status.parse() {
        Ok(status) => status,
        Err(_) => return AppLookupResult::Unavailable("bad status from the lookup".to_string()),
    };
    classify_lookup(status, body.as_bytes())
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
            crate::model::tier_from_apps(verification.verified_count),
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
            crate::model::tier_from_apps(verification.verified_count),
            crate::model::Tier::Core
        );

        let nine = &apps[..9];
        let verification = verify_apps(nine, &lookup).await;
        assert_eq!(verification.verified_count, 9);
        assert_eq!(
            crate::model::tier_from_apps(verification.verified_count),
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
            crate::model::tier_from_apps(verification.verified_count),
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
