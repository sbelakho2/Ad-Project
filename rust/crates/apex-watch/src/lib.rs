//! The setup watch: a paid daily app-ads.txt watch for publishers.
//!
//! One subscriber record holds the publisher slug, the watched domain, the
//! contact email, the tier (`core` or `normal`), optional per-network account
//! ids and the check history (`last_check_at`, `last_findings_hash`,
//! `last_alert_at`, the recorded findings themselves, and the cancel token
//! hash). The records are append-only JSONL under
//! [`WATCH_ROOT_ENV`] (default [`.apex/watch`]); the newest row per
//! subscriber id wins.
//!
//! - [`SubscriberStore::subscribe`] validates the domain and email shapes,
//!   deduplicates a repeat submission with the same `(domain, email)` within
//!   a 24-hour window and returns the live subscriber row. The cancel token
//!   travels back to the caller once; only its SHA-256 hash is stored.
//! - [`SubscriberStore::cancel`] matches a presented token against the stored
//!   hash and records the canceled row (cancelled rows stay, they are never
//!   deleted).
//! - [`SubscriberStore::list`] folds the file into the current subscriber set.
//!
//! # The daily check
//!
//! [`run_daily`] walks the active subscribers with a bounded fan-out
//! ([`daily::DAILY_CONCURRENCY`] checks at a time), then applies alerts and
//! subscriber rows in subscriber order. For each subscriber it resolves the
//! domain through the injected [`HostResolver`] and refuses private,
//! loopback, link-local, shared, multicast, unspecified, broadcast and
//! metadata answers before any request ([`guard_hosts`]). Then it fetches
//! the public `app-ads.txt` over HTTPS, falls back to HTTP only when the
//! HTTPS attempt fails at the transport level, and runs the
//! `apex-audit` app-ads.txt rule over the bytes. Every fetch failure is
//! recorded as the finding kind `unreachable` with a typed reason; a check
//! never panics.
//!
//! Findings are diffed against the previous check by subject and severity.
//! New, worsened or changed findings send exactly one alert email through
//! the injected [`NotificationSink`] (the deployment wires the
//! `apex-notify` email sink). Unchanged findings send nothing. When the
//! alert cannot be delivered the recorded baseline stays at the previous
//! findings, so the next run retries the same alert instead of losing it.
//!
//! # The monthly bill
//!
//! [`run_bill`] issues one invoice per active subscriber through the same
//! [`apex_billing::InvoiceRegister`] path the runner uses, with the frozen
//! [`apex_billing::watch_plan`] line: $99 for the normal tier and $199 for
//! the core tier, in [`apex_billing::PLAN_CURRENCY`]. A subscriber without a
//! recorded customer profile is refused with a typed reason, and a refusal
//! consumes no register number. A subscriber already billed for the month is
//! skipped, so a rerun can never double-bill.
//!
//! # Layering
//!
//! The crate holds no platform keys and no runner: it fetches only public
//! `app-ads.txt` files through the system `curl`, the same convention the
//! sales engine's `io` module uses, because the audit path's own fetcher is
//! HTTPS-only and carries a platform label this service does not have. The
//! resolve-time guard here is what keeps a hostile or mistyped domain away
//! from loopback and metadata addresses.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod bill;
pub mod daily;
pub mod fetch;
pub mod findings;
pub mod guard;
pub mod model;
pub mod store;

pub use bill::{current_month, month_bounds, run_bill, BillDeps, BillReport, IssuedInvoice};
pub use daily::{run_daily, watch_page_url, DailyDeps, DailyEntry, DailyReport};
pub use fetch::{normalize_domain, CurlFetcher, FetchError, Fetched, Fetcher, MapFetcher};
pub use findings::{diff_findings, findings_hash, Change, StoredFinding};
pub use guard::{guard_hosts, HostResolver, SsrfRefusal, StdResolver, REFUSAL_MESSAGE};
pub use model::{
    parse_account_ids, publisher_slug_for_domain, validate_watch_signup, Status, SubscribeOutcome,
    Subscriber, Tier, WatchFieldError, WatchSignup, ACCOUNTS_ERROR, DOMAIN_ERROR, EMAIL_ERROR,
    FORM_SUMMARY, MAX_ACCOUNT_TEXT_CHARS,
};
pub use store::{now_unix_ms, SubscriberStore, WatchError, DEFAULT_WATCH_ROOT, WATCH_ROOT_ENV};
