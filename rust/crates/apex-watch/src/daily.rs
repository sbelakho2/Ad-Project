//! The daily check: fetch, run the rule, diff, alert once per transition.
//!
//! [`run_daily`] walks the active subscribers of a [`SubscriberStore`] and
//! for each one:
//!
//! 1. resolves the domain through the injected [`HostResolver`] and refuses
//!    any non-public answer before the fetch ([`crate::guard`]);
//! 2. reads the public `app-ads.txt` through the injected [`Fetcher`]
//!    (HTTPS first, a plain HTTP fallback after a transport-level failure);
//! 3. runs the `apex-audit` `app_ads_txt` rule over the bytes, with the
//!    `billing` platform label and no expected-network list (the watch has
//!    no platform account);
//! 4. diffs the findings against the ones recorded by the previous check;
//! 5. sends exactly one alert through the injected [`NotificationSink`] when
//!    new, worsened or changed findings appear.
//!
//! The recorded baseline moves to the current findings when the check found
//! nothing to alert or when the alert was accepted. A failed alert leaves
//! the previous baseline in place, so the next run retries the same alert
//! instead of losing it. An unreadable file is recorded as the finding kind
//! `unreachable` with the typed reason; a check never panics.
//!
//! A worker that panics anyway is contained: every worker handle of a wave
//! is joined, the panic is converted to a typed [`WatchError::Malformed`]
//! naming the subscriber, and the wave's results are applied in subscriber
//! order only up to the first failure. Waves that already ran stay applied
//! and the subscribers after the failed one are untouched, so the failed
//! subscriber's state is never half-written and the process never unwinds
//! at the scope.
//!
//! The alert copy lives in the constants of this module: what leaked, the
//! watched domain, one line per changed finding, the watch page link (from
//! `APEX_WATCH_PAGE_URL`, or `APEX_CONSOLE_URL` plus `/watch`) and the
//! cancel line.

use apex_audit::{AppAdsTxt, Snapshot};
use apex_core::PlatformId;
use apex_metrics::vocab::TemplateClass;
use apex_notify::{DeliveryMessage, NotificationSink};

use crate::fetch::{fetch_app_ads, AppAdsFetch, Fetcher};
use crate::findings::{diff_findings, findings_hash, StoredFinding};
use crate::guard::{guard_hosts, HostResolver, REFUSAL_MESSAGE};
use crate::model::Subscriber;
use crate::store::{SubscriberStore, WatchError};

/// The alert subject prefix; the watched domain follows.
pub const ALERT_SUBJECT_PREFIX: &str = "Setup watch alert:";
/// The alert's opening line, before the domain.
pub const ALERT_INTRO: &str =
    "The daily setup watch found new or worse problems in the app-ads.txt of";
/// The consequence copy: what a broken app-ads.txt lets leak.
pub const ALERT_IMPACT: &str = "Missing or broken records in app-ads.txt can leave your inventory authorized through channels you did not intend.";
/// The label before the watch page URL.
pub const WATCH_PAGE_PREFIX: &str = "View your setup watch:";
/// The link text of the watch page URL.
pub const WATCH_PAGE_LINK_TEXT: &str = "View your setup watch";
/// The cancel line of every alert.
pub const UNSUBSCRIBE_NOTE: &str =
    "To cancel your setup watch and stop these alerts, reply to this email.";

/// Environment variable naming the full watch page URL.
pub const WATCH_PAGE_ENV: &str = "APEX_WATCH_PAGE_URL";
/// Environment variable naming the console base URL (the watch page is
/// `<base>/watch` when the dedicated variable is unset).
pub const CONSOLE_URL_ENV: &str = "APEX_CONSOLE_URL";

/// The watch page URL for the alert copy: `APEX_WATCH_PAGE_URL` as given, or
/// `APEX_CONSOLE_URL` plus `/watch`. Unset means no link line.
pub fn watch_page_url(lookup: &impl Fn(&str) -> Option<String>) -> Option<String> {
    let value = |name: &str| {
        lookup(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    if let Some(page) = value(WATCH_PAGE_ENV) {
        return Some(page);
    }
    let base = value(CONSOLE_URL_ENV)?;
    Some(format!("{}/watch", base.trim_end_matches('/')))
}

/// What one daily run reports about one subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyEntry {
    /// The publisher slug.
    pub publisher: String,
    /// The watched domain.
    pub domain: String,
    /// How many findings the check produced.
    pub findings: usize,
    /// How many of them were new, worsened or changed.
    pub changes: usize,
    /// Whether the alert was accepted by the sink.
    pub alerted: bool,
    /// Why the alert failed, when it did.
    pub alert_error: Option<String>,
    /// The typed reason of an unreadable file, when it was.
    pub unreachable: Option<String>,
}

/// The full daily report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DailyReport {
    /// Active subscribers checked.
    pub checked: usize,
    /// Alerts accepted by the sink.
    pub alerted: usize,
    /// Checks with no new or worse findings.
    pub unchanged: usize,
    /// Checks whose file could not be read.
    pub unreachable: usize,
    /// Per-subscriber lines, oldest subscriber first.
    pub entries: Vec<DailyEntry>,
    /// Subscriber rows that could not be recorded back.
    pub failures: Vec<String>,
}

/// The injected dependencies of one daily run.
#[derive(Debug)]
pub struct DailyDeps<'a> {
    /// The fetch seam.
    pub fetcher: &'a dyn Fetcher,
    /// The resolve-time guard seam.
    pub resolver: &'a dyn HostResolver,
    /// Where alerts go (the deployment wires the email sink).
    pub sink: &'a dyn NotificationSink,
    /// The watch page URL of the alert copy, when configured.
    pub watch_page_url: Option<String>,
}

/// The bounded fan-out of one daily run: at most this many subscribers are
/// resolved and fetched at the same time. Each check is independent — the
/// resolve-time guard runs before any request inside the check, redirect
/// hops stay sequential per subscriber, and one subscriber's findings never
/// feed another's — so the bound is the only concurrency control. Alerts and
/// subscriber rows are applied afterwards in subscriber order, which keeps
/// the report, the alert sequence and the store's last-writer-wins fold
/// identical to a sequential run.
pub const DAILY_CONCURRENCY: usize = 8;

/// The facts one check produced, before any alert or store effect.
struct CheckFacts {
    /// The finding set of the check.
    current: Vec<StoredFinding>,
    /// The material differences against the recorded baseline.
    changes: Vec<crate::findings::Change>,
    /// The typed reason when the file could not be read.
    unreachable: Option<String>,
}

/// The synchronous phase of one check: resolve, guard, fetch, run the rule
/// and diff against the recorded baseline. Pure with respect to the run's
/// shared state, so it can execute on a worker thread; a check never panics.
fn check_facts(
    subscriber: &Subscriber,
    fetcher: &dyn Fetcher,
    resolver: &dyn HostResolver,
) -> CheckFacts {
    let mut unreachable = None;
    let current = match resolver.resolve(&subscriber.domain) {
        Err(detail) => {
            let reason = format!("the domain did not resolve: {detail}");
            unreachable = Some(reason.clone());
            vec![StoredFinding::unreachable(&reason)]
        }
        Ok(addresses) => match guard_hosts(&addresses) {
            Err(refusal) => {
                let class = refusal.class;
                let reason = format!("{} ({class} {})", REFUSAL_MESSAGE, refusal.address);
                unreachable = Some(reason.clone());
                vec![StoredFinding::unreachable(&reason)]
            }
            Ok(()) => match fetch_app_ads(&subscriber.domain, fetcher, resolver) {
                Ok(AppAdsFetch::Text(lines)) => rule_findings(true, lines),
                Ok(AppAdsFetch::Missing) => rule_findings(false, Vec::new()),
                Err(unreachable_error) => {
                    unreachable = Some(unreachable_error.reason.clone());
                    vec![StoredFinding::unreachable(&unreachable_error.reason)]
                }
            },
        },
    };
    let previous = subscriber.last_findings.clone().unwrap_or_default();
    let changes = diff_findings(&previous, &current);
    CheckFacts {
        current,
        changes,
        unreachable,
    }
}

/// Run the daily check over every active subscriber.
///
/// Checks fan out with a bounded number of worker threads
/// ([`DAILY_CONCURRENCY`]); alerts and store updates are applied in
/// subscriber order after each wave, so the report equals a sequential
/// run's and the store fold (newest row per id wins) cannot depend on
/// completion order.
///
/// Every worker of a wave is joined before any result is applied, so a
/// panicking worker cannot escape as a scope panic. Results are then applied
/// in subscriber order up to the first failure: earlier subscribers keep
/// their alerts and rows, while the failed subscriber and every later one
/// are left untouched. The typed error names the panicking subscriber.
pub async fn run_daily(
    store: &SubscriberStore,
    now_ms: i64,
    deps: &DailyDeps<'_>,
) -> Result<DailyReport, WatchError> {
    let subscribers = store.list_active()?;
    let mut report = DailyReport::default();
    for wave in subscribers.chunks(DAILY_CONCURRENCY.max(1)) {
        // Only the synchronous check runs on worker threads; the shared
        // sink and the store stay on this thread.
        let outcomes: Vec<Result<CheckFacts, WatchError>> = std::thread::scope(|scope| {
            let handles: Vec<_> = wave
                .iter()
                .map(|subscriber| {
                    let fetcher = deps.fetcher;
                    let resolver = deps.resolver;
                    (
                        subscriber,
                        scope.spawn(move || check_facts(subscriber, fetcher, resolver)),
                    )
                })
                .collect();
            // Join every handle, in subscriber order, even after a panic:
            // dropping the later unjoined handles would make the scope
            // re-panic at exit ("a scoped thread panicked") instead of
            // returning the typed error.
            handles
                .into_iter()
                .map(|(subscriber, handle)| {
                    handle.join().map_err(|_| {
                        WatchError::Malformed(format!(
                            "the daily check for '{}' panicked",
                            subscriber.publisher
                        ))
                    })
                })
                .collect()
        });
        for (subscriber, outcome) in wave.iter().zip(outcomes) {
            // Apply in subscriber order and stop at the first failure: the
            // earlier subscribers keep their alerts and rows, the failed
            // subscriber's state stays untouched.
            let facts = outcome?;
            report.checked += 1;
            apply_check(store, subscriber, now_ms, facts, deps, &mut report).await;
        }
    }
    Ok(report)
}

/// The ordered phase of one check: alert on the computed changes and record
/// the subscriber row, exactly in the original sequence.
async fn apply_check(
    store: &SubscriberStore,
    subscriber: &Subscriber,
    now_ms: i64,
    facts: CheckFacts,
    deps: &DailyDeps<'_>,
    report: &mut DailyReport,
) {
    let CheckFacts {
        current,
        changes,
        unreachable,
    } = facts;
    let mut entry = DailyEntry {
        publisher: subscriber.publisher.clone(),
        domain: subscriber.domain.clone(),
        findings: current.len(),
        changes: changes.len(),
        alerted: false,
        alert_error: None,
        unreachable,
    };
    if entry.unreachable.is_some() {
        report.unreachable += 1;
    }

    if changes.is_empty() {
        report.unchanged += 1;
    } else {
        match send_alert(
            subscriber,
            &current,
            &changes,
            deps.watch_page_url.as_deref(),
            deps.sink,
        )
        .await
        {
            Ok(()) => {
                entry.alerted = true;
                report.alerted += 1;
            }
            Err(detail) => entry.alert_error = Some(detail),
        }
    }

    let mut updated = subscriber.clone();
    updated.last_check_at = Some(now_ms);
    if changes.is_empty() || entry.alerted {
        updated.last_findings = Some(current.clone());
        updated.last_findings_hash = Some(findings_hash(&current));
        if entry.alerted {
            updated.last_alert_at = Some(now_ms);
        }
    }
    if let Err(error) = store.update(&updated) {
        report
            .failures
            .push(format!("{}: {error}", subscriber.publisher));
    }
    report.entries.push(entry);
}

/// Build the snapshot the app-ads.txt rule consumes: only the file facts
/// matter for the watch, so every other vector stays empty.
fn rule_findings(present: bool, lines: Vec<String>) -> Vec<StoredFinding> {
    let snapshot = Snapshot {
        platform: PlatformId::Billing,
        fetched_at_unix_ms: 0,
        ad_units: Vec::new(),
        waterfall: Vec::new(),
        mediation_groups: Vec::new(),
        bidder_accounts: Vec::new(),
        country_revenue: Vec::new(),
        app_ads_txt: AppAdsTxt {
            present,
            lines,
            expected_networks: Vec::new(),
        },
        platform_totals_30d: apex_audit::RevenueTotals {
            revenue_30d: 0,
            impressions_30d: 0,
        },
        mediation_totals_30d: apex_audit::RevenueTotals {
            revenue_30d: 0,
            impressions_30d: 0,
        },
    };
    apex_audit::rules::app_ads_txt::check(&snapshot)
        .iter()
        .map(StoredFinding::from_rule)
        .collect()
}

/// Compose and send the one alert for this transition.
async fn send_alert(
    subscriber: &Subscriber,
    current: &[StoredFinding],
    changes: &[crate::findings::Change],
    watch_page_url: Option<&str>,
    sink: &dyn NotificationSink,
) -> Result<(), String> {
    let domain = &subscriber.domain;
    let subject = format!("{ALERT_SUBJECT_PREFIX} {domain}");
    let mut text = format!("{ALERT_INTRO} {domain}.\n\n");
    for change in changes {
        text.push_str(&change.alert_line());
        text.push('\n');
    }
    text.push('\n');
    text.push_str(ALERT_IMPACT);
    text.push('\n');
    if let Some(url) = watch_page_url {
        text.push_str(&format!("\n{WATCH_PAGE_PREFIX} {url}\n"));
    }
    text.push_str(&format!("\n{UNSUBSCRIBE_NOTE}\n"));

    let mut html = format!(
        "<p>{} <strong>{}</strong>.</p><ul>",
        escape(ALERT_INTRO),
        escape(domain)
    );
    for change in changes {
        html.push_str(&format!("<li>{}</li>", escape(&change.alert_line())));
    }
    html.push_str("</ul>");
    html.push_str(&format!("<p>{}</p>", escape(ALERT_IMPACT)));
    if let Some(url) = watch_page_url {
        html.push_str(&format!(
            "<p><a href=\"{}\">{WATCH_PAGE_LINK_TEXT}</a></p>",
            escape(url)
        ));
    }
    html.push_str(&format!("<p>{}</p>", escape(UNSUBSCRIBE_NOTE)));

    let message = DeliveryMessage {
        to: subscriber.email.clone(),
        subject,
        html,
        text,
        template_class: TemplateClass::AuditReady,
        dedup_key: format!(
            "watch_alert:{}:{}",
            subscriber.publisher,
            findings_hash(current)
        ),
        reference: Some(subscriber.id.clone()),
    };
    sink.send(&message)
        .await
        .map(|_receipt| ())
        .map_err(|error| error.to_string())
}

/// HTML-escape text and attribute values.
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::net::IpAddr;

    use apex_notify::FileSink;

    use super::*;
    use crate::fetch::MapFetcher;

    #[derive(Debug)]
    struct ScriptedResolver {
        answers: Vec<(String, Vec<String>)>,
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl ScriptedResolver {
        fn new(answers: Vec<(&str, Vec<&str>)>) -> Self {
            Self {
                answers: answers
                    .into_iter()
                    .map(|(host, addresses)| {
                        (
                            host.to_string(),
                            addresses.into_iter().map(str::to_string).collect(),
                        )
                    })
                    .collect(),
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl HostResolver for ScriptedResolver {
        fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
            self.calls.lock().unwrap().push(host.to_string());
            self.answers
                .iter()
                .find(|(name, _)| name == host)
                .map(|(_, addresses)| {
                    addresses
                        .iter()
                        .map(|address| address.parse::<IpAddr>().unwrap())
                        .collect::<Vec<IpAddr>>()
                })
                .ok_or_else(|| format!("no answer for {host}"))
        }
    }

    fn public_resolver() -> ScriptedResolver {
        ScriptedResolver::new(vec![("example.com", vec!["93.184.216.34"])])
    }

    fn route(url: &str, status: u16, body: &[u8]) -> (String, u16, Vec<u8>, Option<String>) {
        (url.to_string(), status, body.to_vec(), None)
    }

    fn fetcher(pages: Vec<(String, u16, Vec<u8>, Option<String>)>) -> MapFetcher {
        MapFetcher {
            pages,
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn deps<'a>(
        fetcher: &'a MapFetcher,
        resolver: &'a ScriptedResolver,
        sink: &'a dyn NotificationSink,
    ) -> DailyDeps<'a> {
        DailyDeps {
            fetcher,
            resolver,
            sink,
            watch_page_url: Some("https://apexmediation.ee/watch".to_string()),
        }
    }

    fn alert_rows(path: &std::path::Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn the_first_check_alerts_once_and_the_unchanged_rerun_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        store
            .subscribe(10, "example.com", "ops@x.example", "", false)
            .expect("subscribe");
        let outbox = dir.path().join("alerts.jsonl");

        let missing = fetcher(vec![route("https://example.com/app-ads.txt", 404, b"")]);
        let resolver = public_resolver();
        let report = run_daily(
            &store,
            1_000,
            &deps(&missing, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.checked, 1);
        assert_eq!(report.alerted, 1);
        assert_eq!(report.unchanged, 0);
        assert_eq!(report.entries[0].findings, 1);
        assert_eq!(report.entries[0].changes, 1);
        assert!(report.entries[0].alerted);

        let rows = alert_rows(&outbox);
        assert_eq!(rows.len(), 1, "exactly one alert email");
        let row = &rows[0];
        assert_eq!(row["to"], "ops@x.example");
        assert_eq!(row["subject"], "Setup watch alert: example.com");
        assert_eq!(row["template_class"], "audit_ready");
        let text = row["text"].as_str().unwrap();
        assert!(text.contains("no app-ads.txt found"), "{text}");
        assert!(
            text.contains("View your setup watch: https://apexmediation.ee/watch"),
            "{text}"
        );
        assert!(text.contains(UNSUBSCRIBE_NOTE), "{text}");
        assert!(
            row["dedup_key"]
                .as_str()
                .unwrap()
                .starts_with("watch_alert:example-com:"),
            "{row}"
        );

        let folded = store.list().expect("list");
        assert_eq!(folded[0].last_alert_at, Some(1_000));
        assert_eq!(folded[0].last_check_at, Some(1_000));
        let stored = folded[0].last_findings.clone().expect("findings");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].subject, "app_ads_txt");
        assert_eq!(
            folded[0].last_findings_hash.as_deref(),
            Some(findings_hash(&stored).as_str())
        );

        // The same result again: nothing new, nothing sent.
        let report = run_daily(
            &store,
            2_000,
            &deps(&missing, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 0);
        assert_eq!(report.unchanged, 1);
        assert_eq!(alert_rows(&outbox).len(), 1);
        let folded = store.list().expect("list");
        assert_eq!(folded[0].last_alert_at, Some(1_000), "no re-alert time");
        assert_eq!(folded[0].last_check_at, Some(2_000));
    }

    #[tokio::test]
    async fn recovery_is_silent_and_the_problem_re_alerts_when_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        store
            .subscribe(10, "example.com", "ops@x.example", "", false)
            .expect("subscribe");
        let outbox = dir.path().join("alerts.jsonl");
        let resolver = public_resolver();

        let missing = fetcher(vec![route("https://example.com/app-ads.txt", 404, b"")]);
        let report = run_daily(
            &store,
            1_000,
            &deps(&missing, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 1);

        // The file exists and is clean: the critical finding resolved, the
        // alert stays silent and the baseline moves to the empty set.
        let clean = fetcher(vec![route(
            "https://example.com/app-ads.txt",
            200,
            b"admob.com, pub-1, DIRECT\n",
        )]);
        let report = run_daily(
            &store,
            2_000,
            &deps(&clean, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 0);
        assert_eq!(report.entries[0].findings, 0);
        assert_eq!(report.entries[0].changes, 0);
        assert_eq!(
            store.list().expect("list")[0].last_findings.clone(),
            Some(Vec::new())
        );

        // The file disappears again: that is a new transition, one alert.
        let report = run_daily(
            &store,
            3_000,
            &deps(&missing, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 1);
        assert_eq!(alert_rows(&outbox).len(), 2);
    }

    #[tokio::test]
    async fn changed_finding_text_alerts_because_new_lines_moved() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        store
            .subscribe(10, "example.com", "ops@x.example", "", false)
            .expect("subscribe");
        let outbox = dir.path().join("alerts.jsonl");
        let resolver = public_resolver();

        let one = fetcher(vec![route(
            "https://example.com/app-ads.txt",
            200,
            b"broken line\n",
        )]);
        let report = run_daily(
            &store,
            1_000,
            &deps(&one, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 1);

        let two = fetcher(vec![route(
            "https://example.com/app-ads.txt",
            200,
            b"broken line\nsecond broken\n",
        )]);
        let report = run_daily(
            &store,
            2_000,
            &deps(&two, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 1, "the moved message re-alerts");
        assert_eq!(report.entries[0].changes, 1);

        let report = run_daily(
            &store,
            3_000,
            &deps(&two, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 0, "identical findings stay silent");
        assert_eq!(alert_rows(&outbox).len(), 2);
    }

    #[tokio::test]
    async fn an_unreadable_file_is_recorded_and_alerted_once_per_transition() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        store
            .subscribe(10, "example.com", "ops@x.example", "", false)
            .expect("subscribe");
        let outbox = dir.path().join("alerts.jsonl");
        let resolver = public_resolver();

        let none = fetcher(Vec::new());
        let report = run_daily(
            &store,
            1_000,
            &deps(&none, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.unreachable, 1);
        assert_eq!(report.alerted, 1);
        let rows = alert_rows(&outbox);
        assert!(
            rows[0]["text"]
                .as_str()
                .unwrap()
                .contains("app-ads.txt could not be read"),
            "{}",
            rows[0]["text"]
        );
        assert_eq!(
            store.list().expect("list")[0]
                .last_findings
                .clone()
                .unwrap()[0]
                .subject,
            "unreachable"
        );

        let report = run_daily(
            &store,
            2_000,
            &deps(&none, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 0);
        assert_eq!(alert_rows(&outbox).len(), 1);
    }

    #[tokio::test]
    async fn private_answers_are_refused_with_zero_fetch_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let cases = [
            ("loop.example", "127.0.0.1"),
            ("ten.example", "10.0.0.5"),
            ("link.example", "169.254.169.254"),
            ("loop6.example", "::1"),
        ];
        for (index, (domain, _)) in cases.iter().enumerate() {
            store
                .subscribe(10 + index as i64, domain, "ops@x.example", "", false)
                .expect("subscribe");
        }
        let outbox = dir.path().join("alerts.jsonl");
        let resolver = ScriptedResolver::new(
            cases
                .iter()
                .map(|(domain, address)| (*domain, vec![*address]))
                .collect(),
        );
        let fetcher = fetcher(Vec::new());
        let report = run_daily(
            &store,
            1_000,
            &deps(&fetcher, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");

        assert_eq!(report.checked, 4);
        assert_eq!(report.unreachable, 4);
        assert_eq!(report.alerted, 4);
        assert!(
            fetcher.calls.lock().unwrap().is_empty(),
            "a refused domain must produce zero fetch attempts"
        );
        for entry in &report.entries {
            let reason = entry.unreachable.as_deref().expect("refused");
            assert!(reason.contains(REFUSAL_MESSAGE), "{reason}");
        }
        let rows = alert_rows(&outbox);
        assert_eq!(rows.len(), 4);
        assert!(
            rows[0]["text"]
                .as_str()
                .unwrap()
                .contains("domain resolves to a private address; not fetched"),
            "{}",
            rows[0]["text"]
        );
    }

    #[tokio::test]
    async fn a_failed_alert_keeps_the_baseline_so_the_next_run_retries() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        store
            .subscribe(10, "example.com", "ops@x.example", "", false)
            .expect("subscribe");
        let blocked = dir.path().join("blocked");
        std::fs::create_dir_all(&blocked).unwrap();
        let missing = fetcher(vec![route("https://example.com/app-ads.txt", 404, b"")]);
        let resolver = public_resolver();

        let report = run_daily(
            &store,
            1_000,
            &deps(&missing, &resolver, &FileSink::new(&blocked)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 0);
        assert!(report.entries[0].alert_error.is_some());
        let folded = store.list().expect("list");
        assert!(
            folded[0].last_findings.is_none(),
            "the baseline must not move when the alert failed"
        );
        assert_eq!(folded[0].last_check_at, Some(1_000));

        // The sink works now: the same transition alerts and records.
        let outbox = dir.path().join("alerts.jsonl");
        let report = run_daily(
            &store,
            2_000,
            &deps(&missing, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 1);
        assert_eq!(report.entries[0].changes, 1);
        let folded = store.list().expect("list");
        assert!(folded[0].last_findings.is_some());
        assert_eq!(folded[0].last_alert_at, Some(2_000));
        assert_eq!(alert_rows(&outbox).len(), 1);
    }

    #[tokio::test]
    async fn a_clean_first_check_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        store
            .subscribe(10, "example.com", "ops@x.example", "", false)
            .expect("subscribe");
        let clean = fetcher(vec![route(
            "https://example.com/app-ads.txt",
            200,
            b"admob.com, pub-1, DIRECT\n",
        )]);
        let resolver = public_resolver();
        let outbox = dir.path().join("alerts.jsonl");
        let report = run_daily(
            &store,
            1_000,
            &deps(&clean, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.alerted, 0);
        assert_eq!(report.unchanged, 1);
        assert!(!outbox.exists(), "no alert file is created");
    }

    #[test]
    fn the_watch_page_url_prefers_the_dedicated_variable() {
        assert_eq!(watch_page_url(&|_| None), None);
        let console_only =
            |name: &str| (name == CONSOLE_URL_ENV).then(|| "https://console.example/".to_string());
        assert_eq!(
            watch_page_url(&console_only).as_deref(),
            Some("https://console.example/watch")
        );
        let both = |name: &str| match name {
            WATCH_PAGE_ENV => Some("https://apexmediation.ee/watch".to_string()),
            CONSOLE_URL_ENV => Some("https://console.example".to_string()),
            _ => None,
        };
        assert_eq!(
            watch_page_url(&both).as_deref(),
            Some("https://apexmediation.ee/watch")
        );
        let blank = |name: &str| (name == WATCH_PAGE_ENV).then(String::new);
        assert_eq!(watch_page_url(&blank), None);
    }
    /// A resolver that answers every host with one public address.
    #[derive(Debug)]
    struct AnyPublicResolver;

    impl HostResolver for AnyPublicResolver {
        fn resolve(&self, _host: &str) -> Result<Vec<IpAddr>, String> {
            Ok(vec!["93.184.216.34".parse().unwrap()])
        }
    }

    /// A rendezvous the first wave must meet: every check of the first wave
    /// reaches `get` concurrently with bounded fan-out, so all
    /// [`DAILY_CONCURRENCY`] arrivals release each other. Against a
    /// sequential run the rendezvous is never met, every fetch answers a
    /// transport failure and the assertions below fail. The deadline is a
    /// failure cap for that proof, not a performance assertion.
    struct RendezvousFetcher {
        state: std::sync::Mutex<usize>,
        ready: std::sync::Condvar,
        deadline: std::time::Instant,
        expected: usize,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl std::fmt::Debug for RendezvousFetcher {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("RendezvousFetcher").finish()
        }
    }

    impl Fetcher for RendezvousFetcher {
        fn get(&self, _url: &str) -> Result<crate::fetch::Fetched, crate::fetch::FetchError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut count = self
                .state
                .lock()
                .map_err(|_| crate::fetch::FetchError::Transport("rendezvous poisoned".into()))?;
            *count += 1;
            while *count < self.expected {
                let remaining = self
                    .deadline
                    .saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return Err(crate::fetch::FetchError::Transport(
                        "the concurrency rendezvous timed out".to_string(),
                    ));
                }
                let (guard, outcome) = self.ready.wait_timeout(count, remaining).map_err(|_| {
                    crate::fetch::FetchError::Transport("rendezvous poisoned".into())
                })?;
                count = guard;
                if outcome.timed_out() && *count < self.expected {
                    return Err(crate::fetch::FetchError::Transport(
                        "the concurrency rendezvous timed out".to_string(),
                    ));
                }
            }
            drop(count);
            self.ready.notify_all();
            Ok(crate::fetch::Fetched {
                status: 200,
                body: b"admob.com, pub-1, DIRECT\n".to_vec(),
                redirect: None,
            })
        }
    }

    #[tokio::test]
    async fn checks_fan_out_over_the_bounded_worker_pool() {
        // The documented fan-out bound is 8 subscribers: the first wave must
        // overlap that many checks. The rendezvous (not the clock) proves
        // it: sequential checks never meet it.
        const WAVE: usize = 8;
        let wave_bound = DAILY_CONCURRENCY;
        assert!(wave_bound >= WAVE, "the bound must cover a full wave");
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        for i in 0..WAVE {
            store
                .subscribe(
                    10,
                    &format!("fan{i}.example"),
                    &format!("ops{i}@x.example"),
                    "",
                    false,
                )
                .expect("subscribe");
        }
        let fetcher = RendezvousFetcher {
            state: std::sync::Mutex::new(0),
            ready: std::sync::Condvar::new(),
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(2),
            expected: WAVE,
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let resolver = AnyPublicResolver;
        let sink = FileSink::new(dir.path().join("alerts.jsonl"));
        let report = run_daily(
            &store,
            1_000,
            &DailyDeps {
                fetcher: &fetcher,
                resolver: &resolver,
                sink: &sink,
                watch_page_url: None,
            },
        )
        .await
        .expect("run");
        assert_eq!(report.checked, WAVE);
        assert_eq!(
            report.unreachable, 0,
            "all first-wave checks must overlap on the worker pool"
        );
        assert_eq!(
            fetcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            WAVE
        );
    }

    #[tokio::test]
    async fn alerts_and_rows_are_applied_in_subscriber_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let domains: Vec<String> = (0..20).map(|i| format!("order{i}.example")).collect();
        for (i, domain) in domains.iter().enumerate() {
            store
                .subscribe(
                    10 + i as i64,
                    domain,
                    &format!("ops{i}@x.example"),
                    "",
                    false,
                )
                .expect("subscribe");
        }
        // Subscribers 2, 9 and 17 have no app-ads.txt: they alert, in
        // subscriber order, regardless of completion order.
        let mut routes = Vec::new();
        for (i, domain) in domains.iter().enumerate() {
            let status = if matches!(i, 2 | 9 | 17) { 404 } else { 200 };
            let body = if status == 200 {
                b"admob.com, pub-1, DIRECT\n".to_vec()
            } else {
                Vec::new()
            };
            routes.push((format!("https://{domain}/app-ads.txt"), status, body, None));
        }
        let fetcher = fetcher(routes);
        let resolver = AnyPublicResolver;
        let outbox = dir.path().join("alerts.jsonl");
        let sink = FileSink::new(&outbox);
        let report = run_daily(
            &store,
            1_000,
            &DailyDeps {
                fetcher: &fetcher,
                resolver: &resolver,
                sink: &sink,
                watch_page_url: None,
            },
        )
        .await
        .expect("run");
        assert_eq!(report.checked, 20);
        assert_eq!(report.alerted, 3);
        let entry_domains: Vec<&str> = report
            .entries
            .iter()
            .map(|entry| entry.domain.as_str())
            .collect();
        let expected: Vec<&str> = domains.iter().map(String::as_str).collect();
        assert_eq!(entry_domains, expected, "report rows keep subscriber order");
        // Every subscriber row was recorded (last_check_at) exactly once.
        for subscriber in store.list().expect("list") {
            assert_eq!(subscriber.last_check_at, Some(1_000));
        }
        let rows = alert_rows(&outbox);
        assert_eq!(rows.len(), 3);
        let subjects: Vec<&str> = rows
            .iter()
            .map(|row| row["subject"].as_str().unwrap())
            .collect();
        assert_eq!(
            subjects,
            vec![
                "Setup watch alert: order2.example",
                "Setup watch alert: order9.example",
                "Setup watch alert: order17.example",
            ],
            "alerts go out in subscriber order"
        );
        // The baseline moved for every subscriber: the clean ones to the
        // empty set, the alerting ones to the recorded finding.
        let folded = store.list().expect("list");
        assert_eq!(folded[0].last_findings, Some(Vec::new()));
        assert!(folded[2]
            .last_findings
            .as_ref()
            .is_some_and(|f| !f.is_empty()));
    }

    /// A fetcher that panics for the scripted domains and answers clean
    /// (200) or missing (404) for the rest, so a wave can carry a scripted
    /// crash. The pre-fix short-circuit dropped the handles after the first
    /// failed join unjoined; a later panicking worker is what turned that
    /// drop into the escape `a scoped thread panicked`, so the regression
    /// keeps a second panicking worker behind the first.
    #[derive(Debug)]
    struct ScriptedPanicFetcher {
        panic_domains: Vec<String>,
        missing: Vec<String>,
    }

    impl Fetcher for ScriptedPanicFetcher {
        fn get(&self, url: &str) -> Result<crate::fetch::Fetched, crate::fetch::FetchError> {
            if self.panic_domains.iter().any(|domain| url.contains(domain)) {
                panic!("scripted worker panic for {url}");
            }
            if self.missing.iter().any(|domain| url.contains(domain)) {
                return Ok(crate::fetch::Fetched {
                    status: 404,
                    body: Vec::new(),
                    redirect: None,
                });
            }
            Ok(crate::fetch::Fetched {
                status: 200,
                body: b"admob.com, pub-1, DIRECT\n".to_vec(),
                redirect: None,
            })
        }
    }

    /// 20 subscribers: three waves for [`DAILY_CONCURRENCY`] checks of 8.
    fn panic_harness(dir: &tempfile::TempDir) -> (std::sync::Arc<SubscriberStore>, Vec<String>) {
        let store = std::sync::Arc::new(SubscriberStore::open(dir.path()));
        let domains: Vec<String> = (0..20).map(|i| format!("panictest{i}.example")).collect();
        for (index, domain) in domains.iter().enumerate() {
            store
                .subscribe(
                    10 + index as i64,
                    domain,
                    &format!("ops{index}@x.example"),
                    "",
                    false,
                )
                .expect("subscribe");
        }
        (store, domains)
    }

    fn alert_subjects(outbox: &std::path::Path) -> Vec<String> {
        alert_rows(outbox)
            .iter()
            .map(|row| row["subject"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn a_panic_second_in_a_wave_is_a_typed_error_not_a_scope_panic() {
        let dir = tempfile::tempdir().unwrap();
        let (store, domains) = panic_harness(&dir);
        let outbox = dir.path().join("alerts.jsonl");
        let fetcher = ScriptedPanicFetcher {
            panic_domains: vec![domains[9].clone(), domains[10].clone()],
            missing: vec![domains[3].clone(), domains[8].clone()],
        };
        let resolver = AnyPublicResolver;
        let sink = FileSink::new(&outbox);
        let shared = std::sync::Arc::clone(&store);
        let joined = tokio::spawn(async move {
            run_daily(
                &shared,
                1_000,
                &DailyDeps {
                    fetcher: &fetcher,
                    resolver: &resolver,
                    sink: &sink,
                    watch_page_url: None,
                },
            )
            .await
        })
        .await;

        // The worker panic was joined into a typed error; no scope panic
        // unwound the task.
        let result = joined.expect("the daily run must not unwind at the scope");
        let error = result.expect_err("a worker panic is a typed error");
        match error {
            WatchError::Malformed(message) => {
                assert!(message.contains("panictest9-example"), "{message}");
                assert!(message.contains("panicked"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }

        // Wave 1 and the first subscriber of wave 2 applied; the failing
        // subscriber and everyone after it keep their untouched state.
        let folded = store.list().expect("list");
        for (index, subscriber) in folded.iter().enumerate() {
            let expected = (index <= 8).then_some(1_000);
            assert_eq!(subscriber.last_check_at, expected, "subscriber {index}");
        }

        // The alerts before the failure were persisted in subscriber order;
        // the panicking subscriber has no alert.
        assert_eq!(
            alert_subjects(&outbox),
            vec![
                format!("{} {}", ALERT_SUBJECT_PREFIX, domains[3]),
                format!("{} {}", ALERT_SUBJECT_PREFIX, domains[8]),
            ]
        );
    }

    #[tokio::test]
    async fn a_panic_last_in_a_wave_is_still_the_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let (store, domains) = panic_harness(&dir);
        let outbox = dir.path().join("alerts.jsonl");
        let fetcher = ScriptedPanicFetcher {
            panic_domains: vec![domains[7].clone()],
            missing: vec![domains[3].clone()],
        };
        let resolver = AnyPublicResolver;
        let sink = FileSink::new(&outbox);
        let shared = std::sync::Arc::clone(&store);
        let joined = tokio::spawn(async move {
            run_daily(
                &shared,
                1_000,
                &DailyDeps {
                    fetcher: &fetcher,
                    resolver: &resolver,
                    sink: &sink,
                    watch_page_url: None,
                },
            )
            .await
        })
        .await;

        let result = joined.expect("the daily run must not unwind at the scope");
        let error = result.expect_err("a worker panic is a typed error");
        match error {
            WatchError::Malformed(message) => {
                assert!(message.contains("panictest7-example"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }

        // The subscribers before the failure applied; the panicking last
        // worker and the next wave are untouched.
        let folded = store.list().expect("list");
        for (index, subscriber) in folded.iter().enumerate() {
            let expected = (index < 7).then_some(1_000);
            assert_eq!(subscriber.last_check_at, expected, "subscriber {index}");
        }
        assert_eq!(
            alert_subjects(&outbox),
            vec![format!("{} {}", ALERT_SUBJECT_PREFIX, domains[3])]
        );
    }
}
