//! The free setup watch: a daily app-ads.txt watch for publishers.
//!
//! One subscriber record holds the publisher slug, the watched domain, the
//! contact email, the tier (`core` or `normal`), optional per-network account
//! ids and the check history (`last_check_at`, `last_findings_hash`,
//! `last_alert_at`, the recorded findings themselves, and the cancel token
//! hash). The records are append-only JSONL under
//! [`WATCH_ROOT_ENV`] (default [`.apex/watch`]); the newest row per
//! subscriber id wins.
//!
//! - [`SubscriberStore::subscribe_pending`] validates the domain and email
//!   shapes, records a `pending` subscriber, derives the tier from the
//!   App Store and Play developer catalogues behind a confirmed app
//!   ([`verify_apps`] and [`tier_from_apps`], never the submitted count and
//!   never ad-system account ids) and returns the one-time email confirmation
//!   token (only its SHA-256 hash is stored). A pending row runs no check and
//!   is never billed. No checkout link is created at sign-up.
//! - [`SubscriberStore::confirm_email`] consumes that token once, within
//!   [`CONFIRMATION_TTL_MS`]; a second use, an expired token or an unknown
//!   token is a typed refusal.
//! - [`SubscriberStore::activate`] flips the matching pending row to `active`
//!   when the payment is accepted, recording `paid_at`; an already-active row
//!   is a typed idempotent no-op and an unknown/canceled row is a typed
//!   refusal. [`SubscriberStore::subscribe`] remains as the legacy direct
//!   signup used by tools and older tests.
//! - [`SubscriberStore::create_profile_token`] mints the one-time billing
//!   profile link token (same shape and TTL as the email token, one
//!   outstanding); [`SubscriberStore::consume_profile_token`] enforces the
//!   single use and [`SubscriberStore::set_billing_profile`] validates and
//!   stores the profile the first and monthly invoices render with.
//! - [`SubscriberStore::cancel`] matches a presented token against the stored
//!   hash and records the canceled row (cancelled rows stay, they are never
//!   deleted).
//! - [`SubscriberStore::list`] folds the file into the current subscriber set.
//!
//! # The daily check
//!
//! [`run_daily`] walks only active, email-confirmed subscribers with a
//! recorded billing profile, with a bounded fan-out
//! ([`daily::DAILY_CONCURRENCY`] checks at a time), then applies alerts and
//! subscriber rows in subscriber order. Pending, canceled, unconfirmed and
//! profile-less rows are skipped with a typed recorded reason
//! ([`SKIP_PENDING`], [`SKIP_CANCELED`], [`SKIP_UNCONFIRMED`],
//! [`SKIP_NO_PROFILE`]). A paid legacy row without a profile is therefore not
//! check-eligible until the billing-profile form records one. For each
//! checked subscriber it resolves the domain through the injected
//! [`HostResolver`] and refuses private, loopback, link-local, shared,
//! multicast, unspecified, broadcast and metadata answers before any request
//! ([`guard_hosts`]). Then it fetches the public `app-ads.txt` over HTTPS,
//! falls back to HTTP only when the HTTPS attempt fails at the transport
//! level, and runs the `apex-audit` app-ads.txt rule over the bytes. The
//! fetch is pinned to the guard-approved address with curl's
//! `--resolve host:port:address`, so a DNS answer that changes between the
//! guard and the fetch cannot redirect the request. Subscribers watching the
//! same domain share one resolve-guard-fetch-rule per run, so the public file
//! is read and parsed once per distinct domain. Every fetch failure is
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
//! # No billing
//!
//! A3 made the app-ads.txt watch free. This service never issues an invoice,
//! never creates a checkout link and has no renewal, tier price or stored-card
//! charge path.
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

pub mod apps;
pub mod daily;
pub mod fetch;
pub mod findings;
pub mod guard;
pub mod model;
pub mod store;

pub use apps::{
    app_store_artist_id, app_store_catalogue_count, app_store_catalogue_url, classify_lookup,
    google_play_catalogue_count, google_play_details_url, google_play_developer_id,
    google_play_developer_name, google_play_developer_url, google_play_listing_exists,
    google_play_package, google_play_search_url, lookup_url, merge_store_answers, verify_apps,
    AppLookup, AppLookupResult, AppVerification, CurlAppLookup, Portfolio, StoreCatalogue,
};

pub use daily::{
    run_daily, skip_reason, watch_page_url, DailyDeps, DailyEntry, DailyReport, DailySkip,
    SKIP_CANCELED, SKIP_NO_PROFILE, SKIP_PENDING, SKIP_UNCONFIRMED,
};
pub use fetch::{normalize_domain, CurlFetcher, FetchError, Fetched, Fetcher, MapFetcher};
pub use findings::{diff_findings, findings_hash, Change, StoredFinding};
pub use guard::{guard_hosts, HostResolver, SsrfRefusal, StdResolver, REFUSAL_MESSAGE};
pub use model::{
    app_identity, distinct_app_count, distinct_apps, parse_account_ids, publisher_slug_for_domain,
    tier_from_apps, validate_watch_signup, ActivateOutcome, BillingProfile, ConfirmOutcome,
    PendingOutcome, Status, SubscribeOutcome, SubscribeRequest, Subscriber, Tier, WatchFieldError,
    WatchSignup, ACCOUNTS_ERROR, CONFIRMATION_TOKEN_HEX_LEN, CONFIRMATION_TTL_MS,
    CORE_TIER_MIN_APP_COUNT, DOMAIN_ERROR, EMAIL_ERROR, FORM_SUMMARY, MAX_ACCOUNT_TEXT_CHARS,
};
pub use store::{
    is_confirmation_token, now_unix_ms, SubscriberStore, WatchError, DEFAULT_WATCH_ROOT,
    WATCH_ROOT_ENV,
};
