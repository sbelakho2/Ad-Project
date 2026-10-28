//! The daily check: fetch, run the rule, diff, alert once per transition.
//!
//! [`run_daily`] walks the active subscribers of a [`SubscriberStore`] and
//! for each one:
//!
//! 1. resolves the domain through the injected [`HostResolver`] and refuses
//!    any non-public answer before the fetch ([`crate::guard`]);
//! 2. reads the public `app-ads.txt` through the injected [`Fetcher`]
//!    (HTTPS only; a transport failure is reported, never downgraded);
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
//! Subscribers that watch the same domain share one resolve-guard-fetch-rule for
//! the run: the first worker computes the verdict and the others reuse it, so
//! the public file is read and parsed once per distinct domain, not once per
//! subscriber.
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

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use apex_audit::{AppAdsTxt, Snapshot};
use apex_core::PlatformId;
use apex_metrics::vocab::TemplateClass;
use apex_notify::{DeliveryMessage, NotificationSink};

use crate::fetch::{fetch_app_ads_conditional, AppAdsFetch, AppAdsRead, Fetcher};
use crate::findings::{diff_findings, findings_hash, StoredFinding};
use crate::guard::{guard_hosts, HostResolver, REFUSAL_MESSAGE};
use crate::model::Subscriber;
use crate::store::{DomainCacheEntry, SubscriberStore, WatchError};

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
    /// Whether the domain's serialized validators answered 304, so the
    /// previous findings were reused without parsing a body.
    pub not_modified: bool,
}

/// The exact skip reasons recorded when the daily loop passes a subscriber.
pub const SKIP_PENDING: &str = "the subscriber is pending confirmation; no check ran";
/// The skip reason for a canceled subscriber.
pub const SKIP_CANCELED: &str = "the subscriber is canceled; no check ran";
/// The skip reason for an active subscriber whose email is unconfirmed.
pub const SKIP_UNCONFIRMED: &str = "the subscriber's email is not confirmed; no check ran";
/// The skip reason for an active subscriber with no customer billing profile:
/// a paid legacy row is not check-eligible until the profile is recorded.
pub const SKIP_NO_PROFILE: &str =
    "the subscriber is active but has no billing profile; no check ran";

/// The typed reason a subscriber was skipped instead of checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailySkip {
    /// The publisher slug.
    pub publisher: String,
    /// The watched domain.
    pub domain: String,
    /// The stable skip reason (one of [`SKIP_PENDING`], [`SKIP_CANCELED`],
    /// [`SKIP_UNCONFIRMED`] or [`SKIP_NO_PROFILE`]).
    pub reason: String,
}

/// The full daily report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DailyReport {
    /// Active, confirmed subscribers checked.
    pub checked: usize,
    /// Alerts accepted by the sink.
    pub alerted: usize,
    /// Checks with no new or worse findings.
    pub unchanged: usize,
    /// Checks whose file could not be read.
    pub unreachable: usize,
    /// Per-subscriber lines, oldest subscriber first.
    pub entries: Vec<DailyEntry>,
    /// Subscribers that ran no check, in subscriber order, with the typed
    /// reason (pending, canceled, unconfirmed or no billing profile).
    pub skipped: Vec<DailySkip>,
    /// Subscriber rows that could not be recorded back.
    pub failures: Vec<String>,
    /// Distinct domains resolved and fetched (one verdict per domain).
    pub domain_fetches: usize,
    /// Subscriber checks served from a same-run domain verdict.
    pub domain_reuses: usize,
    /// The largest wave dispatched (at most [`DAILY_CONCURRENCY`]).
    pub max_wave: usize,
    /// Domain checks answered 304 and served from the cached findings.
    pub not_modified: usize,
}

/// The skip reason for `subscriber`, or `None` when it may be checked.
pub fn skip_reason(subscriber: &Subscriber) -> Option<&'static str> {
    match subscriber.status {
        crate::model::Status::Pending => Some(SKIP_PENDING),
        crate::model::Status::Canceled => Some(SKIP_CANCELED),
        crate::model::Status::Active if !subscriber.confirmed => Some(SKIP_UNCONFIRMED),
        // A confirmed email is not enough: the daily check is only eligible
        // once the customer billing profile is recorded too.
        crate::model::Status::Active if subscriber.billing_profile.is_none() => {
            Some(SKIP_NO_PROFILE)
        }
        crate::model::Status::Active => None,
    }
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
    /// Whether the domain answered 304 and reused cached findings.
    not_modified: bool,
    /// The observed app-ads.txt line set to persist as the identity
    /// baseline: `Some` only for a present, non-empty file, so an absent or
    /// empty response preserves the last known-good authorizations.
    app_ads_lines: Option<Vec<String>>,
}

/// The verdict one domain's resolve-guard-fetch-rule produced, before the
/// per-subscriber diff is applied. The rule output depends only on the public
/// file, so it is shared by every subscriber watching the domain.
#[derive(Debug, Clone)]
enum DomainOutcome {
    /// The file was read (or absent) and the rule ran once. The line set is
    /// `Some` for a present file (empty when it carries no lines) and `None`
    /// for an absent one; the flag says the server answered 304 and both the
    /// findings and the lines are the cached ones.
    Findings(Vec<StoredFinding>, Option<Vec<String>>, bool),
    /// The domain could not be read, with the typed reason.
    Unreachable(String),
}

/// One run's domain verdicts: every subscriber watching the same domain
/// shares one resolve/guard/fetch. The first worker to ask computes the
/// verdict; concurrent workers on the same domain wait on that `OnceLock`
/// instead of issuing a second request. Distinct domains are independent.
#[derive(Debug)]
struct DomainFetches {
    entries: Mutex<HashMap<String, Arc<OnceLock<DomainOutcome>>>>,
    /// The per-domain conditional-GET state loaded from the watch state.
    cache: Mutex<HashMap<String, DomainCacheEntry>>,
    fetches: AtomicU64,
    reuses: AtomicU64,
    #[cfg(test)]
    key_allocations: AtomicU64,
}

impl Default for DomainFetches {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

impl DomainFetches {
    fn new(cache: HashMap<String, DomainCacheEntry>) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            cache: Mutex::new(cache),
            fetches: AtomicU64::new(0),
            reuses: AtomicU64::new(0),
            #[cfg(test)]
            key_allocations: AtomicU64::new(0),
        }
    }

    /// The persisted state of `domain`, when it can be used conditionally.
    fn cached(&self, domain: &str) -> Option<DomainCacheEntry> {
        self.cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(domain).cloned())
    }

    /// Replace `domain`'s cached state after a 200.
    fn remember(&self, domain: &str, entry: DomainCacheEntry) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(domain.to_string(), entry);
        }
    }

    /// Drop `domain`'s cached state after a definitive "no file" answer.
    fn forget(&self, domain: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(domain);
        }
    }

    /// The cache as it stands at the end of the run, for persistence.
    fn take_cache(&self) -> HashMap<String, DomainCacheEntry> {
        self.cache
            .lock()
            .map(|mut cache| std::mem::take(&mut *cache))
            .unwrap_or_default()
    }

    fn outcome(&self, domain: &str, fetch: impl FnOnce() -> DomainOutcome) -> DomainOutcome {
        let cell = {
            let mut entries = match self.entries.lock() {
                Ok(entries) => entries,
                Err(poisoned) => poisoned.into_inner(),
            };
            // A hit must not allocate a fresh key: the previous
            // `entry(domain.to_string())` allocated for every subscriber even
            // when the domain had already been claimed in this run.
            match entries.get(domain) {
                Some(cell) => {
                    self.reuses.fetch_add(1, Ordering::Relaxed);
                    Arc::clone(cell)
                }
                None => {
                    #[cfg(test)]
                    self.key_allocations.fetch_add(1, Ordering::Relaxed);
                    Arc::clone(entries.entry(domain.to_string()).or_default())
                }
            }
        };
        cell.get_or_init(|| {
            self.fetches.fetch_add(1, Ordering::Relaxed);
            fetch()
        })
        .clone()
    }

    /// Distinct domain verdicts computed in this run.
    fn fetch_count(&self) -> usize {
        usize::try_from(self.fetches.load(Ordering::Relaxed)).unwrap_or(usize::MAX)
    }

    /// Subscriber checks served from an existing verdict.
    fn reuse_count(&self) -> usize {
        usize::try_from(self.reuses.load(Ordering::Relaxed)).unwrap_or(usize::MAX)
    }

    #[cfg(test)]
    fn key_allocations(&self) -> u64 {
        self.key_allocations.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }
}

/// The synchronous phase of one check: resolve, guard, fetch, run the rule
/// and diff against the recorded baseline. Pure with respect to the run's
/// shared state except for the domain fetch cache, so it can execute on a
/// worker thread; a check never panics.
fn check_facts(
    subscriber: &Subscriber,
    fetcher: &dyn Fetcher,
    resolver: &dyn HostResolver,
    domains: &DomainFetches,
) -> CheckFacts {
    let domain = subscriber.domain.as_str();
    let cached = domains.cached(domain);
    let outcome = domains.outcome(domain, || {
        let validators = cached
            .as_ref()
            .and_then(DomainCacheEntry::conditional_validators);
        match resolver.resolve(domain) {
            Err(detail) => {
                DomainOutcome::Unreachable(format!("the domain did not resolve: {detail}"))
            }
            Ok(addresses) => match guard_hosts(&addresses) {
                Err(refusal) => {
                    let class = refusal.class;
                    DomainOutcome::Unreachable(format!(
                        "{} ({class} {})",
                        REFUSAL_MESSAGE, refusal.address
                    ))
                }
                Ok(()) => match addresses.first().copied() {
                    None => {
                        DomainOutcome::Unreachable("the resolver returned no addresses".to_string())
                    }
                    Some(approved) => match fetch_app_ads_conditional(
                        domain, fetcher, resolver, approved, validators,
                    ) {
                        Ok(AppAdsRead {
                            outcome: AppAdsFetch::Text(lines),
                            validators,
                        }) => {
                            let findings = rule_findings(true, lines.clone());
                            domains.remember(
                                domain,
                                DomainCacheEntry {
                                    validators,
                                    findings: Some(findings.clone()),
                                    lines: Some(lines.clone()),
                                },
                            );
                            DomainOutcome::Findings(findings, Some(lines), false)
                        }
                        Ok(AppAdsRead {
                            outcome: AppAdsFetch::Missing,
                            ..
                        }) => {
                            domains.forget(domain);
                            DomainOutcome::Findings(rule_findings(false, Vec::new()), None, false)
                        }
                        Ok(AppAdsRead {
                            outcome: AppAdsFetch::Unchanged,
                            validators,
                        }) => match cached.as_ref().and_then(|entry| entry.findings.clone()) {
                            Some(findings) => {
                                let lines = cached.as_ref().and_then(|entry| entry.lines.clone());
                                // The 304 is honest only if the cache is
                                // present; refresh any validators it moved.
                                if !validators.is_empty() {
                                    domains.remember(
                                        domain,
                                        DomainCacheEntry {
                                            validators,
                                            findings: Some(findings.clone()),
                                            lines: lines.clone(),
                                        },
                                    );
                                }
                                DomainOutcome::Findings(findings, lines, true)
                            }
                            None => DomainOutcome::Unreachable(
                                "the server answered 304 but no cached findings exist".to_string(),
                            ),
                        },
                        Err(unreachable_error) => {
                            DomainOutcome::Unreachable(unreachable_error.reason)
                        }
                    },
                },
            },
        }
    });
    let (mut current, observed_lines, unreachable, not_modified) = match outcome {
        DomainOutcome::Findings(findings, lines, not_modified) => {
            (findings, lines, None, not_modified)
        }
        DomainOutcome::Unreachable(reason) => {
            let findings = vec![StoredFinding::unreachable(&reason)];
            (findings, None, Some(reason), false)
        }
    };
    // Authorization identities are compared against the last known-good line
    // set, which survives absent and empty responses. An empty present file
    // therefore reports every lost line instead of quietly becoming the new
    // baseline.
    let mut identity_lost: u32 = 0;
    if let (Some(baseline), Some(observed)) = (&subscriber.last_app_ads_lines, &observed_lines) {
        if !baseline.is_empty() {
            identity_lost = crate::findings::app_ads_identities_lost(baseline, observed);
            if identity_lost > 0 {
                current.push(StoredFinding {
                    subject: crate::findings::APP_ADS_AUTHORIZE_LOST_SUBJECT.to_string(),
                    severity: crate::findings::APP_ADS_AUTHORIZE_LOST_SEVERITY.to_string(),
                    message: format!(
                        "{identity_lost} previously present app-ads.txt authorization line{} disappeared",
                        if identity_lost == 1 { "" } else { "s" }
                    ),
                });
            }
        }
    }
    let previous = subscriber.last_findings.clone().unwrap_or_default();
    let changes = diff_findings(&previous, &current);
    CheckFacts {
        current,
        changes,
        unreachable,
        not_modified,
        // While any recorded identity is missing the baseline stays at the
        // last complete set: a partial restore must not shrink it, or a
        // later compounding loss would go unseen.
        app_ads_lines: if identity_lost > 0 {
            None
        } else {
            observed_lines
                .map(|lines| {
                    lines
                        .into_iter()
                        .filter(|line| apex_audit::rules::app_ads_txt::is_authorization_line(line))
                        .collect::<Vec<String>>()
                })
                .filter(|lines| !lines.is_empty())
        },
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
    let subscribers = store.list()?;
    let mut report = DailyReport::default();
    let mut checkable = Vec::with_capacity(subscribers.len());
    for subscriber in subscribers {
        match skip_reason(&subscriber) {
            Some(reason) => report.skipped.push(DailySkip {
                publisher: subscriber.publisher.clone(),
                domain: subscriber.domain.clone(),
                reason: reason.to_string(),
            }),
            None => checkable.push(subscriber),
        }
    }
    // One domain verdict per distinct watched domain for the whole run, so a
    // domain watched by several subscribers is resolved and fetched once.
    // The persisted validators/findings make the fetch conditional.
    let domains = DomainFetches::new(store.load_domain_cache());
    let mut run_error = None;
    'waves: for wave in checkable.chunks(DAILY_CONCURRENCY.max(1)) {
        report.max_wave = report.max_wave.max(wave.len());
        // Only the synchronous check runs on worker threads; the shared
        // sink and the store stay on this thread.
        let outcomes: Vec<Result<CheckFacts, WatchError>> = std::thread::scope(|scope| {
            let handles: Vec<_> = wave
                .iter()
                .map(|subscriber| {
                    let fetcher = deps.fetcher;
                    let resolver = deps.resolver;
                    let domains = &domains;
                    (
                        subscriber,
                        scope.spawn(move || check_facts(subscriber, fetcher, resolver, domains)),
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
        // Rows are batched: the whole wave is one append (one open/write)
        // instead of one append per subscriber. Alerts still go out in
        // subscriber order before the rows are recorded.
        let mut updates = Vec::with_capacity(wave.len());
        for (subscriber, outcome) in wave.iter().zip(outcomes) {
            // Apply in subscriber order and stop at the first failure: the
            // earlier subscribers keep their alerts and rows, the failed
            // subscriber's state stays untouched.
            let facts = match outcome {
                Ok(facts) => facts,
                Err(error) => {
                    run_error = Some(error);
                    break;
                }
            };
            report.checked += 1;
            if let Some(updated) = apply_check(subscriber, now_ms, facts, deps, &mut report).await {
                updates.push(updated);
            }
        }
        if !updates.is_empty() {
            if let Err(error) = store.update_many(&updates) {
                for updated in &updates {
                    report
                        .failures
                        .push(format!("{}: {error}", updated.publisher));
                }
            }
        }
        if run_error.is_some() {
            break 'waves;
        }
    }
    // The per-run rendezvous map is state of this call only; report its
    // bounded size and release it before returning.
    report.domain_fetches = domains.fetch_count();
    report.domain_reuses = domains.reuse_count();
    let cache = domains.take_cache();
    drop(domains);
    if run_error.is_none() && report.failures.is_empty() {
        // A successful daily fold folds the append-only history into the
        // checkpoint and leaves a fresh delta for the next run. A checkpoint
        // failure only loses the optimization; the full log is still truth.
        let _ = store.checkpoint();
    }
    if let Err(error) = store.save_domain_cache(&cache) {
        if run_error.is_none() {
            return Err(error);
        }
    }
    match run_error {
        Some(error) => Err(error),
        None => Ok(report),
    }
}

/// The ordered phase of one check: alert on the computed changes and record
/// the subscriber row, exactly in the original sequence.
async fn apply_check(
    subscriber: &Subscriber,
    now_ms: i64,
    facts: CheckFacts,
    deps: &DailyDeps<'_>,
    report: &mut DailyReport,
) -> Option<Subscriber> {
    let CheckFacts {
        current,
        changes,
        unreachable,
        not_modified,
        app_ads_lines,
    } = facts;
    let mut entry = DailyEntry {
        publisher: subscriber.publisher.clone(),
        domain: subscriber.domain.clone(),
        findings: current.len(),
        changes: changes.len(),
        alerted: false,
        alert_error: None,
        unreachable,
        not_modified,
    };
    if entry.unreachable.is_some() {
        report.unreachable += 1;
    }
    if entry.not_modified {
        report.not_modified += 1;
    }

    let mut rotated_cancel_hash: Option<String> = None;
    if changes.is_empty() {
        report.unchanged += 1;
    } else {
        // A fresh single-use cancel token travels with the alert; the old
        // one stops matching as soon as the alert is accepted.
        let (cancel_raw, cancel_hash) = crate::store::SubscriberStore::mint_cancel_token();
        let cancel_url = deps
            .watch_page_url
            .as_deref()
            .and_then(|url| cancel_link(url, &cancel_raw));
        match send_alert(
            subscriber,
            &current,
            &changes,
            deps.watch_page_url.as_deref(),
            cancel_url.as_deref(),
            deps.sink,
        )
        .await
        {
            Ok(()) => {
                entry.alerted = true;
                report.alerted += 1;
                // Only a delivered alert rotates the token; a failed send
                // leaves the previous cancel link usable.
                rotated_cancel_hash = Some(cancel_hash);
            }
            Err(detail) => entry.alert_error = Some(detail),
        }
    }

    let mut updated = subscriber.clone();
    // Only a present, non-empty file advances the identity baseline; absent
    // and empty responses keep the last known-good authorizations.
    if let Some(lines) = &app_ads_lines {
        updated.last_app_ads_lines = Some(lines.clone());
    }
    if let Some(hash) = rotated_cancel_hash {
        updated.cancel_token_hash = hash;
    }
    updated.last_check_at = Some(now_ms);
    if changes.is_empty() || entry.alerted {
        updated.last_findings = Some(current.clone());
        updated.last_findings_hash = Some(findings_hash(&current));
        if entry.alerted {
            updated.last_alert_at = Some(now_ms);
        }
    }
    report.entries.push(entry);
    Some(updated)
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
        mediation_totals_reported: false,
    };
    apex_audit::rules::app_ads_txt::check(&snapshot)
        .iter()
        .map(StoredFinding::from_rule)
        .collect()
}

/// The absolute cancel URL derived from the configured watch page URL.
fn cancel_link(watch_page_url: &str, token: &str) -> Option<String> {
    let trimmed = watch_page_url.trim_end_matches('/');
    let base = trimmed.strip_suffix("/watch").unwrap_or(trimmed);
    if base.is_empty() {
        return None;
    }
    Some(format!("{base}/watch/cancel?token={token}"))
}

/// Compose and send the one alert for this transition.
async fn send_alert(
    subscriber: &Subscriber,
    current: &[StoredFinding],
    changes: &[crate::findings::Change],
    watch_page_url: Option<&str>,
    cancel_url: Option<&str>,
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
    if let Some(url) = cancel_url {
        text.push_str(&format!("\nCancel your setup watch: {url}\n"));
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
    if let Some(url) = cancel_url {
        html.push_str(&format!(
            "<p><a href=\"{}\">Cancel your setup watch</a></p>",
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
        list_unsubscribe: cancel_url.map(str::to_string),
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

    /// The legacy direct signup plus the billing profile a real subscriber
    /// records before the daily check is eligible. A paid row without the
    /// profile is intentionally not checkable; the tests that prove a check
    /// runs use this helper.
    fn subscribe_checkable(store: &SubscriberStore, now: i64, domain: &str, email: &str) {
        let outcome = store
            .subscribe(now, domain, email, "", false)
            .expect("subscribe");
        let mut subscriber = outcome.subscriber;
        subscriber.billing_profile = Some(crate::model::BillingProfile {
            legal_name: "Buyer OÜ".to_string(),
            address_lines: vec!["Telliskivi 1".to_string()],
            country: "EE".to_string(),
            vat_number: Some("EE111111111".to_string()),
        });
        store.update(&subscriber).expect("billing profile");
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
        fetcher: &'a dyn Fetcher,
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
        subscribe_checkable(&store, 10, "example.com", "ops@x.example");
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
        // The rendezvous map is per-run state: it is rebuilt and released per
        // call, so a later run starts from zero entries, not one more.
        assert_eq!(report.domain_fetches, 1);
        assert_eq!(report.max_wave, 1);
        assert_eq!(alert_rows(&outbox).len(), 1);
        let folded = store.list().expect("list");
        assert_eq!(folded[0].last_alert_at, Some(1_000), "no re-alert time");
        assert_eq!(folded[0].last_check_at, Some(2_000));
    }

    #[tokio::test]
    async fn recovery_is_silent_and_the_problem_re_alerts_when_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        subscribe_checkable(&store, 10, "example.com", "ops@x.example");
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
        subscribe_checkable(&store, 10, "example.com", "ops@x.example");
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
        subscribe_checkable(&store, 10, "example.com", "ops@x.example");
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
            subscribe_checkable(&store, 10 + index as i64, domain, "ops@x.example");
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
        subscribe_checkable(&store, 10, "example.com", "ops@x.example");
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
        subscribe_checkable(&store, 10, "example.com", "ops@x.example");
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

    #[tokio::test]
    async fn pending_and_unconfirmed_subscribers_run_no_checks_with_typed_reasons() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let pending = store
            .subscribe_pending(
                10,
                &crate::model::SubscribeRequest {
                    domain: "example.com".into(),
                    email: "ops@x.example".into(),
                    accounts: "admob.com = pub-1".into(),
                    apps: vec!["example-app".into()],
                    verified_app_count: Some(1),
                    verification_error: None,
                },
            )
            .expect("pending signup");
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
        assert_eq!(report.checked, 0);
        assert!(report.entries.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].domain, "example.com");
        assert_eq!(report.skipped[0].reason, SKIP_PENDING);
        assert!(clean.calls.lock().unwrap().is_empty(), "no check ran");
        assert_eq!(store.list().unwrap()[0].last_check_at, None);

        // Confirmation alone is not enough: the billing profile is required
        // too.
        let activated = store.activate(2_000, &pending.reference).expect("activate");
        assert!(!activated.already_active);
        let report = run_daily(
            &store,
            3_000,
            &deps(&clean, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.checked, 0);
        assert_eq!(report.skipped[0].reason, SKIP_UNCONFIRMED);
        assert!(clean.calls.lock().unwrap().is_empty(), "still no check");

        store
            .confirm_email(4_000, &pending.confirmation_token)
            .expect("confirm");
        // Confirmation alone is still not enough: the profile is recorded
        // before the check becomes eligible.
        let report = run_daily(
            &store,
            4_500,
            &deps(&clean, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.checked, 0);
        assert_eq!(report.skipped[0].reason, SKIP_NO_PROFILE);
        store
            .set_billing_profile(
                &pending.reference,
                &crate::model::BillingProfile {
                    legal_name: "Buyer OÜ".to_string(),
                    address_lines: vec!["Telliskivi 1".to_string()],
                    country: "EE".to_string(),
                    vat_number: None,
                },
                4_600,
            )
            .expect("profile");
        let report = run_daily(
            &store,
            5_000,
            &deps(&clean, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(
            report.checked, 1,
            "active, confirmed and profiled runs the check"
        );
        assert!(report.skipped.is_empty());
        assert_eq!(clean.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_canceled_subscriber_is_skipped_with_the_canceled_reason() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let signup = store
            .subscribe(10, "cancel.example", "ops@x.example", "", false)
            .expect("subscribe");
        store
            .cancel(signup.cancel_token.as_deref().unwrap(), 11)
            .expect("cancel")
            .expect("match");
        let clean = fetcher(vec![route(
            "https://example.com/app-ads.txt",
            200,
            b"admob.com, pub-1, DIRECT\n",
        )]);
        let resolver = public_resolver();
        let outbox = dir.path().join("alerts.jsonl");
        let report = run_daily(
            &store,
            12,
            &deps(&clean, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.checked, 0);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, SKIP_CANCELED);
        assert!(clean.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_paid_legacy_subscriber_without_a_profile_is_skipped_until_it_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        // The legacy direct signup is active and paid, but carries no billing
        // profile: exactly the row the defect let check while renewals were
        // refused.
        let outcome = store
            .subscribe(10, "legacy.example", "ops@x.example", "", false)
            .expect("subscribe");
        let clean = fetcher(vec![route(
            "https://legacy.example/app-ads.txt",
            200,
            b"admob.com, pub-1, DIRECT\n",
        )]);
        let resolver = ScriptedResolver::new(vec![("legacy.example", vec!["93.184.216.34"])]);
        let outbox = dir.path().join("alerts.jsonl");
        let report = run_daily(
            &store,
            1_000,
            &deps(&clean, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.checked, 0);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, SKIP_NO_PROFILE);
        assert!(
            clean.calls.lock().unwrap().is_empty(),
            "a profile-less subscriber is never fetched"
        );
        assert_eq!(store.list().unwrap()[0].last_check_at, None);

        // Recording the profile makes the same paid row check-eligible.
        store
            .set_billing_profile(
                &outcome.subscriber.id,
                &crate::model::BillingProfile {
                    legal_name: "Legacy OÜ".to_string(),
                    address_lines: vec!["Telliskivi 1".to_string()],
                    country: "EE".to_string(),
                    vat_number: None,
                },
                2_000,
            )
            .expect("profile");
        let report = run_daily(
            &store,
            3_000,
            &deps(&clean, &resolver, &FileSink::new(&outbox)),
        )
        .await
        .expect("run");
        assert_eq!(report.checked, 1, "payment + profile checks normally");
        assert_eq!(clean.calls.lock().unwrap().len(), 1);
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
        fn get(
            &self,
            _url: &str,
            _approved: IpAddr,
        ) -> Result<crate::fetch::Fetched, crate::fetch::FetchError> {
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
                etag: None,
                last_modified: None,
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
            subscribe_checkable(
                &store,
                10,
                &format!("fan{i}.example"),
                &format!("ops{i}@x.example"),
            );
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
    async fn one_domain_is_resolved_and_fetched_once_for_every_subscriber() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        for (created, email) in [(10, "alpha@x.example"), (11, "beta@x.example")] {
            subscribe_checkable(&store, created, "example.com", email);
        }
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
        assert_eq!(report.checked, 2, "both subscribers are checked");
        assert_eq!(report.entries.len(), 2);
        assert_eq!(
            clean.calls.lock().unwrap().len(),
            1,
            "the shared domain is fetched once"
        );
        assert_eq!(
            resolver.calls.lock().unwrap().len(),
            1,
            "the shared domain is resolved once"
        );
        assert_eq!(report.domain_fetches, 1, "one verdict for the domain");
        assert_eq!(report.domain_reuses, 1, "the second subscriber reuses it");
        assert_eq!(report.max_wave, 2);
    }

    #[tokio::test]
    async fn alerts_and_rows_are_applied_in_subscriber_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let domains: Vec<String> = (0..20).map(|i| format!("order{i}.example")).collect();
        for (i, domain) in domains.iter().enumerate() {
            subscribe_checkable(&store, 10 + i as i64, domain, &format!("ops{i}@x.example"));
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
        assert_eq!(report.domain_fetches, 20, "one verdict per distinct domain");
        assert_eq!(report.domain_reuses, 0, "no domain is watched twice here");
        assert_eq!(report.max_wave, DAILY_CONCURRENCY, "waves are bounded");
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
        fn get(
            &self,
            url: &str,
            _approved: IpAddr,
        ) -> Result<crate::fetch::Fetched, crate::fetch::FetchError> {
            if self.panic_domains.iter().any(|domain| url.contains(domain)) {
                panic!("scripted worker panic for {url}");
            }
            if self.missing.iter().any(|domain| url.contains(domain)) {
                return Ok(crate::fetch::Fetched {
                    status: 404,
                    body: Vec::new(),
                    redirect: None,
                    etag: None,
                    last_modified: None,
                });
            }
            Ok(crate::fetch::Fetched {
                status: 200,
                body: b"admob.com, pub-1, DIRECT\n".to_vec(),
                redirect: None,
                etag: None,
                last_modified: None,
            })
        }
    }

    /// 20 subscribers: three waves for [`DAILY_CONCURRENCY`] checks of 8.
    fn panic_harness(dir: &tempfile::TempDir) -> (std::sync::Arc<SubscriberStore>, Vec<String>) {
        let store = std::sync::Arc::new(SubscriberStore::open(dir.path()));
        let domains: Vec<String> = (0..20).map(|i| format!("panictest{i}.example")).collect();
        for (index, domain) in domains.iter().enumerate() {
            subscribe_checkable(
                &store,
                10 + index as i64,
                domain,
                &format!("ops{index}@x.example"),
            );
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

    /// A scripted HTTP fetcher that records the conditional validators each
    /// request carried and answers per URL.
    #[derive(Debug)]
    struct ConditionalFetcher {
        pages: std::sync::Mutex<Vec<(String, crate::fetch::Fetched)>>,
        requests: std::sync::Mutex<Vec<(String, Option<crate::fetch::HttpValidators>)>>,
    }

    impl ConditionalFetcher {
        fn new(pages: Vec<(&str, crate::fetch::Fetched)>) -> Self {
            Self {
                pages: std::sync::Mutex::new(
                    pages
                        .into_iter()
                        .map(|(url, fetched)| (url.to_string(), fetched))
                        .collect(),
                ),
                requests: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn set(&self, url: &str, fetched: crate::fetch::Fetched) {
            let mut pages = self.pages.lock().unwrap();
            if let Some(page) = pages.iter_mut().find(|(name, _)| name == url) {
                page.1 = fetched;
            } else {
                pages.push((url.to_string(), fetched));
            }
        }

        fn requests(&self) -> Vec<(String, Option<crate::fetch::HttpValidators>)> {
            self.requests.lock().unwrap().clone()
        }

        fn answer(
            &self,
            url: &str,
            validators: Option<&crate::fetch::HttpValidators>,
        ) -> Result<crate::fetch::Fetched, crate::fetch::FetchError> {
            self.requests
                .lock()
                .unwrap()
                .push((url.to_string(), validators.cloned()));
            self.pages
                .lock()
                .unwrap()
                .iter()
                .find(|(name, _)| name == url)
                .map(|(_, fetched)| fetched.clone())
                .ok_or_else(|| crate::fetch::FetchError::Transport(format!("no route to {url}")))
        }
    }

    impl Fetcher for ConditionalFetcher {
        fn get(
            &self,
            url: &str,
            _approved: IpAddr,
        ) -> Result<crate::fetch::Fetched, crate::fetch::FetchError> {
            self.answer(url, None)
        }

        fn get_conditional(
            &self,
            url: &str,
            _approved: IpAddr,
            validators: Option<&crate::fetch::HttpValidators>,
        ) -> Result<crate::fetch::Fetched, crate::fetch::FetchError> {
            self.answer(url, validators)
        }
    }

    fn fetched(
        status: u16,
        body: &[u8],
        etag: Option<&str>,
        modified: Option<&str>,
    ) -> crate::fetch::Fetched {
        crate::fetch::Fetched {
            status,
            body: body.to_vec(),
            redirect: None,
            etag: etag.map(str::to_string),
            last_modified: modified.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn a_200_stores_validators_and_a_304_reuses_the_cached_findings() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        subscribe_checkable(&store, 10, "example.com", "ops@x.example");
        let url = "https://example.com/app-ads.txt";
        let fetcher = ConditionalFetcher::new(vec![(
            url,
            fetched(200, b"broken line\n", Some("\"v1\""), Some("date-1")),
        )]);
        let resolver = public_resolver();
        let outbox = dir.path().join("alerts.jsonl");
        let sink = FileSink::new(&outbox);
        let deps = deps(&fetcher, &resolver, &sink);

        let first = run_daily(&store, 1_000, &deps).await.expect("first");
        assert_eq!(first.entries[0].findings, 1);
        assert!(!first.entries[0].not_modified);
        assert_eq!(fetcher.requests()[0].1, None, "the first fetch is plain");

        let cache = store.load_domain_cache();
        let entry = cache.get("example.com").expect("cached validators");
        assert_eq!(entry.validators.etag.as_deref(), Some("\"v1\""));
        assert_eq!(entry.validators.last_modified.as_deref(), Some("date-1"));
        assert_eq!(entry.findings.as_ref().map(Vec::len), Some(1));

        // The next run sends exactly the stored headers; the 304 carries no
        // body, so a wrongly-parsed body would produce a clean finding set.
        fetcher.set(url, fetched(304, b"", Some("\"v1\""), None));
        let second = run_daily(&store, 2_000, &deps).await.expect("second");
        assert_eq!(second.entries[0].findings, 1, "the 304 reused findings");
        assert!(second.entries[0].not_modified);
        assert_eq!(second.not_modified, 1);
        assert_eq!(second.unchanged, 1);
        assert_eq!(second.alerted, 0);
        let request = &fetcher.requests()[1];
        assert_eq!(request.0, url);
        let sent = request.1.as_ref().expect("conditional headers");
        assert_eq!(sent.etag.as_deref(), Some("\"v1\""));
        assert_eq!(sent.last_modified.as_deref(), Some("date-1"));

        // A changed 200 body updates the cached findings and validators.
        fetcher.set(
            url,
            fetched(
                200,
                b"admob.com, pub-1, DIRECT\n",
                Some("\"v2\""),
                Some("date-2"),
            ),
        );
        let third = run_daily(&store, 3_000, &deps).await.expect("third");
        assert_eq!(third.entries[0].findings, 0, "the changed body was parsed");
        assert!(!third.entries[0].not_modified);
        let cache = store.load_domain_cache();
        let entry = cache.get("example.com").expect("updated validators");
        assert_eq!(entry.validators.etag.as_deref(), Some("\"v2\""));
        assert_eq!(entry.findings.as_ref().map(Vec::len), Some(0));
        assert_eq!(alert_rows(&outbox).len(), 1, "only the first run alerted");
    }

    #[tokio::test]
    async fn absent_and_empty_app_ads_never_erase_the_authorization_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        subscribe_checkable(&store, 10, "example.com", "ops@x.example");
        let url = "https://example.com/app-ads.txt";
        let body = b"admob.com, pub-1, DIRECT\n# a comment\n";
        let fetcher = ConditionalFetcher::new(vec![(url, fetched(200, body, None, None))]);
        let resolver = public_resolver();
        let outbox = dir.path().join("alerts.jsonl");
        let sink = FileSink::new(&outbox);
        let deps = deps(&fetcher, &resolver, &sink);

        // The baseline stores the authorization record, not the comment.
        run_daily(&store, 1_000, &deps).await.expect("prime");
        let baseline = store.list().expect("list")[0].last_app_ads_lines.clone();
        assert_eq!(baseline, Some(vec!["admob.com, pub-1, DIRECT".to_string()]));

        // Absent: the daily critical fires, the baseline survives intact.
        fetcher.set(url, fetched(404, b"", None, None));
        let missing = run_daily(&store, 2_000, &deps).await.expect("missing");
        assert_eq!(missing.entries[0].findings, 1);
        let row = store.list().expect("list");
        assert_eq!(row[0].last_app_ads_lines, baseline, "absence preserves it");
        assert!(row[0].last_findings.as_ref().is_some_and(|findings| {
            !findings
                .iter()
                .any(|finding| finding.subject == crate::findings::APP_ADS_AUTHORIZE_LOST_SUBJECT)
        }));

        // Present but emptied (only a comment left): every authorization is
        // gone, and that must alert instead of becoming the new baseline.
        fetcher.set(url, fetched(200, b"# only a comment now\n", None, None));
        let emptied = run_daily(&store, 3_000, &deps).await.expect("emptied");
        assert_eq!(
            emptied.entries[0].findings, 1,
            "the loss itself is a finding"
        );
        assert!(emptied.entries[0].alerted, "the loss alerts");
        let row = store.list().expect("list");
        assert_eq!(
            row[0].last_app_ads_lines, baseline,
            "emptiness never becomes the baseline"
        );
        assert!(row[0]
            .last_findings
            .as_ref()
            .is_some_and(|findings| findings.iter().any(|finding| finding.subject
                == crate::findings::APP_ADS_AUTHORIZE_LOST_SUBJECT
                && finding
                    .message
                    .contains("1 previously present app-ads.txt authorization line disappeared")
                && !finding.message.contains("lines disappeared"))));

        // Restoring the same authorization is a clean recovery and the
        // baseline still points at what was authorized before.
        fetcher.set(url, fetched(200, body, None, None));
        let restored = run_daily(&store, 4_000, &deps).await.expect("restored");
        assert_eq!(restored.entries[0].findings, 0, "the loss is gone");
        assert!(!restored.entries[0].alerted, "a restored file is quiet");
        let row = store.list().expect("list");
        assert_eq!(row[0].last_app_ads_lines, baseline);
        assert!(row[0].last_findings.as_ref().is_some_and(|findings| {
            !findings
                .iter()
                .any(|finding| finding.subject == crate::findings::APP_ADS_AUTHORIZE_LOST_SUBJECT)
        }));
    }

    #[tokio::test]
    async fn a_missing_or_corrupt_validator_cache_falls_back_to_a_full_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        subscribe_checkable(&store, 10, "example.com", "ops@x.example");
        let url = "https://example.com/app-ads.txt";
        let fetcher = ConditionalFetcher::new(vec![(
            url,
            fetched(200, b"admob.com, pub-1, DIRECT\n", Some("\"v1\""), None),
        )]);
        let resolver = public_resolver();
        let outbox = dir.path().join("alerts.jsonl");
        let sink = FileSink::new(&outbox);
        let deps = deps(&fetcher, &resolver, &sink);
        run_daily(&store, 1_000, &deps).await.expect("prime");
        assert!(store.domain_cache_path().exists());

        // A corrupt cache is ignored, never fatal: the next request is a
        // full (unconditional) fetch.
        std::fs::write(store.domain_cache_path(), "{not json").unwrap();
        fetcher.set(url, fetched(200, b"ok\n", Some("\"v2\""), None));
        let report = run_daily(&store, 2_000, &deps)
            .await
            .expect("corrupt cache");
        assert_eq!(report.checked, 1);
        assert_eq!(
            fetcher.requests()[1].1,
            None,
            "a corrupt cache must not send stale validators"
        );
        assert_eq!(
            store
                .load_domain_cache()
                .get("example.com")
                .and_then(|entry| entry.validators.etag.clone()),
            Some("\"v2\"".to_string())
        );

        // A cache whose entry lost its findings is not conditional either.
        std::fs::write(
            store.domain_cache_path(),
            r#"{"example.com":{"etag":"\"v3\""}}"#,
        )
        .unwrap();
        fetcher.set(url, fetched(200, b"ok\n", Some("\"v4\""), None));
        let report = run_daily(&store, 3_000, &deps)
            .await
            .expect("partial cache");
        assert_eq!(report.checked, 1);
        assert_eq!(fetcher.requests()[2].1, None);
    }

    #[tokio::test]
    async fn validators_never_bleed_across_domains() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        subscribe_checkable(&store, 10, "alpha.example", "a@x.example");
        subscribe_checkable(&store, 11, "beta.example", "b@x.example");
        let alpha = "https://alpha.example/app-ads.txt";
        let beta = "https://beta.example/app-ads.txt";
        let fetcher = ConditionalFetcher::new(vec![
            (alpha, fetched(200, b"a\n", Some("\"alpha-1\""), None)),
            (beta, fetched(200, b"b\n", Some("\"beta-1\""), None)),
        ]);
        let resolver = AnyPublicResolver;
        let outbox = dir.path().join("alerts.jsonl");
        let deps = DailyDeps {
            fetcher: &fetcher,
            resolver: &resolver,
            sink: &FileSink::new(&outbox),
            watch_page_url: None,
        };
        run_daily(&store, 1_000, &deps).await.expect("first");
        fetcher.set(alpha, fetched(304, b"", Some("\"alpha-1\""), None));
        fetcher.set(beta, fetched(304, b"", Some("\"beta-1\""), None));
        let report = run_daily(&store, 2_000, &deps).await.expect("second");
        assert_eq!(report.not_modified, 2);
        let requests = fetcher.requests();
        let alpha_request = requests
            .iter()
            .rev()
            .find(|(url, _)| url == alpha)
            .expect("alpha request");
        let beta_request = requests
            .iter()
            .rev()
            .find(|(url, _)| url == beta)
            .expect("beta request");
        assert_eq!(
            alpha_request.1.as_ref().and_then(|v| v.etag.clone()),
            Some("\"alpha-1\"".to_string())
        );
        assert_eq!(
            beta_request.1.as_ref().and_then(|v| v.etag.clone()),
            Some("\"beta-1\"".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_validator_cache_is_written_atomically_with_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let mut cache = HashMap::new();
        cache.insert(
            "example.com".to_string(),
            crate::store::DomainCacheEntry {
                validators: crate::fetch::HttpValidators {
                    etag: Some("\"v1\"".to_string()),
                    last_modified: None,
                },
                findings: Some(Vec::new()),
                lines: None,
            },
        );
        store.save_domain_cache(&cache).expect("save");
        let mode = std::fs::metadata(store.domain_cache_path())
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(store.load_domain_cache(), cache);
    }

    /// The per-run rendezvous allocates one map key per distinct domain and
    /// none per repeated subscriber; a fresh run starts with an empty map, so
    /// nothing grows across daily runs.
    #[test]
    fn the_domain_rendezvous_allocates_one_key_per_domain_per_run() {
        let domains = DomainFetches::default();
        for _ in 0..50 {
            let outcome = domains.outcome("shared.example", || {
                DomainOutcome::Findings(Vec::new(), None, false)
            });
            assert!(matches!(outcome, DomainOutcome::Findings(_, _, _)));
        }
        assert_eq!(domains.len(), 1);
        assert_eq!(domains.fetch_count(), 1, "one verdict, not 50");
        assert_eq!(domains.reuse_count(), 49);
        assert_eq!(domains.key_allocations(), 1, "no per-subscriber key");

        domains.outcome("other.example", || DomainOutcome::Unreachable("x".into()));
        assert_eq!(domains.len(), 2);
        assert_eq!(domains.key_allocations(), 2);

        drop(domains);
        let next_run = DomainFetches::default();
        assert_eq!(next_run.len(), 0, "per-run state starts empty");
        assert_eq!(next_run.key_allocations(), 0);
    }
}
