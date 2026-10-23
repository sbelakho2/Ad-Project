#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

//! Live verification harness for the setup watch.
//!
//! These tests run only with `APEX_LIVE_WATCH=1`: they read and write real state
//! files, resolve real public hostnames, fetch real app-ads.txt files over
//! the network and (when `APEX_LIVE_EMAIL` is set) deliver real mail through
//! `/usr/sbin/sendmail`. They are run explicitly during a live check:
//!
//! ```sh
//! APEX_LIVE_ROOT=/abs/state APEX_LIVE_EMAIL=user@localhost.localdomain \
//!     APEX_LIVE_DOMAINS=digitalturbine.com,vungle.com,startapp.com \
//!     cargo test -p apex-watch --test live_verify -- --ignored --nocapture
//! ```
//!
//! The staging tests create active subscribers through the product's own
//! store API, which is the same row shape the payment webhook writes. No
//! card provider is involved: a live first charge is blocked without Adyen
//! credentials, and these tests never fabricate one.

use std::collections::BTreeMap;

use apex_notify::EmailSink;
fn live_enabled() -> bool {
    matches!(
        std::env::var("APEX_LIVE_WATCH").as_deref(),
        Ok("1" | "true" | "yes")
    )
}

use apex_watch::{
    current_month, now_unix_ms, run_bill, run_daily, verify_apps, BillDeps, CurlAppLookup,
    CurlFetcher, DailyDeps, StdResolver, SubscriberStore,
};

fn live_root() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("APEX_LIVE_ROOT").expect("APEX_LIVE_ROOT must name the live state root"),
    )
}

fn live_email() -> String {
    std::env::var("APEX_LIVE_EMAIL").expect("APEX_LIVE_EMAIL must name the mailbox")
}

fn live_domains() -> Vec<String> {
    std::env::var("APEX_LIVE_DOMAINS")
        .unwrap_or_else(|_| "digitalturbine.com,www.google.com,startapp.com".to_string())
        .split(',')
        .map(str::trim)
        .filter(|domain| !domain.is_empty())
        .map(str::to_string)
        .collect()
}

/// Stage one active, confirmed subscriber per domain through the product's
/// legacy direct-signup path (the row shape the webhook activation writes).
#[test]
fn stage_active_subscribers() {
    if !live_enabled() {
        return;
    }
    let store = SubscriberStore::open(live_root());
    let email = live_email();
    let now = now_unix_ms();
    for domain in live_domains() {
        let outcome = store
            .subscribe(now, &domain, &email, "", false)
            .expect("live subscriber row");
        let profile = apex_watch::BillingProfile {
            legal_name: "Buyer Live OÜ".to_string(),
            address_lines: vec!["Telliskivi 1".to_string(), "10412 Tallinn".to_string()],
            country: "EE".to_string(),
            vat_number: None,
        };
        store
            .set_billing_profile(&outcome.subscriber.id, &profile, now)
            .expect("live billing profile");
        println!(
            "staged {} {} {} cancel_token={:?}",
            outcome.subscriber.id,
            outcome.subscriber.domain,
            outcome.subscriber.status,
            outcome.cancel_token
        );
    }
}

/// Activate one pending subscriber (the webhook's state transition, without
/// the blocked provider charge).
#[test]
fn activate_reference() {
    if !live_enabled() {
        return;
    }
    let reference = std::env::var("APEX_LIVE_REF").expect("APEX_LIVE_REF");
    let outcome = SubscriberStore::open(live_root())
        .activate(now_unix_ms(), &reference)
        .expect("live activation");
    println!(
        "activated {} already_active={}",
        outcome.reference, outcome.already_active
    );
}

/// One real daily run: real DNS, real HTTPS, real parse, real mail. Run it
/// twice to observe the conditional-GET 304 on a server that supports it.
#[tokio::test]
async fn daily_real_domains() {
    if !live_enabled() {
        return;
    }
    let store = SubscriberStore::open(live_root());
    let fetcher = CurlFetcher::default();
    let resolver = StdResolver;
    let from = std::env::var("APEX_LIVE_FROM")
        .unwrap_or_else(|_| "watch-live@localhost.localdomain".to_string());
    let sink = EmailSink::new(Some(from.as_str()), "/usr/sbin/sendmail").expect("real email sink");
    let deps = DailyDeps {
        fetcher: &fetcher,
        resolver: &resolver,
        sink: &sink,
        watch_page_url: Some("http://127.0.0.1:19051/watch/".to_string()),
    };
    let report = run_daily(&store, now_unix_ms(), &deps)
        .await
        .expect("live daily run");
    println!(
        "daily checked={} alerted={} unchanged={} unreachable={} skipped={} domain_fetches={} reuses={} not_modified={}",
        report.checked,
        report.alerted,
        report.unchanged,
        report.unreachable,
        report.skipped.len(),
        report.domain_fetches,
        report.domain_reuses,
        report.not_modified
    );
    for entry in &report.entries {
        println!(
            "entry domain={} findings={} changes={} alerted={} not_modified={} unreachable={:?} alert_error={:?}",
            entry.domain,
            entry.findings,
            entry.changes,
            entry.alerted,
            entry.not_modified,
            entry.unreachable,
            entry.alert_error
        );
        assert!(
            entry.alert_error.is_none(),
            "the real alert failed: {entry:?}"
        );
    }

    // One verdict per distinct domain even when two subscribers watch it.
    let subscribers = store.list().expect("live list");
    let mut by_domain: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for subscriber in &subscribers {
        if subscriber.is_checkable() {
            by_domain
                .entry(subscriber.domain.clone())
                .or_default()
                .push(subscriber.clone());
        }
    }
    assert_eq!(
        report.domain_fetches,
        by_domain.len(),
        "one fetch per domain"
    );
    assert!(report.domain_reuses >= 1, "a shared domain must be reused");

    // Every checkable domain produced its own recorded baseline; no
    // subscriber carries another domain's findings.
    for (domain, rows) in &by_domain {
        for subscriber in rows {
            let findings = subscriber
                .last_findings
                .as_ref()
                .unwrap_or_else(|| panic!("{domain} recorded no findings"));
            let unreachable = findings
                .iter()
                .any(|finding| finding.subject == "unreachable");
            println!(
                "baseline {} {} {} findings={:?} unreachable={unreachable}",
                subscriber.id,
                subscriber.domain,
                subscriber.status,
                findings
                    .iter()
                    .map(|finding| finding.subject.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }
    let digital = by_domain
        .get("digitalturbine.com")
        .expect("digitalturbine subscriber")
        .first()
        .expect("row");
    let missing = by_domain
        .get("www.google.com")
        .expect("google subscriber")
        .first()
        .expect("row");
    let tls = by_domain
        .get("startapp.com")
        .expect("startapp subscriber")
        .first()
        .expect("row");
    let digital_subjects: Vec<_> = digital
        .last_findings
        .as_ref()
        .expect("digital findings")
        .iter()
        .map(|finding| finding.subject.clone())
        .collect();
    let missing_subjects: Vec<_> = missing
        .last_findings
        .as_ref()
        .expect("missing findings")
        .iter()
        .map(|finding| finding.subject.clone())
        .collect();
    assert!(digital_subjects
        .iter()
        .any(|subject| subject.starts_with("app_ads_txt")));
    assert_eq!(missing_subjects, vec!["app_ads_txt".to_string()]);
    assert_eq!(
        missing.last_findings.as_ref().expect("missing")[0].severity,
        "critical"
    );
    assert_eq!(
        tls.last_findings.as_ref().expect("tls")[0].subject,
        "unreachable"
    );
}

/// A real second daily run: the same files must answer 304 / unchanged and
/// must not send a second alert for the same findings.
#[tokio::test]
async fn second_daily_run_is_conditional_and_silent() {
    if !live_enabled() {
        return;
    }
    let store = SubscriberStore::open(live_root());
    let before: BTreeMap<String, i64> = store
        .list()
        .expect("list")
        .into_iter()
        .map(|row| (row.id, row.last_alert_at.unwrap_or(0)))
        .collect();
    let fetcher = CurlFetcher::default();
    let resolver = StdResolver;
    let from = std::env::var("APEX_LIVE_FROM")
        .unwrap_or_else(|_| "watch-live@localhost.localdomain".to_string());
    let sink = EmailSink::new(Some(from.as_str()), "/usr/sbin/sendmail").expect("real email sink");
    let report = run_daily(
        &store,
        now_unix_ms(),
        &DailyDeps {
            fetcher: &fetcher,
            resolver: &resolver,
            sink: &sink,
            watch_page_url: Some("http://127.0.0.1:19051/watch/".to_string()),
        },
    )
    .await
    .expect("second live daily run");
    println!(
        "second daily checked={} alerted={} unchanged={} not_modified={}",
        report.checked, report.alerted, report.unchanged, report.not_modified
    );
    assert_eq!(report.alerted, 0, "unchanged findings must not re-alert");
    assert!(
        report.not_modified >= 1,
        "digitalturbine.com advertises validators and answers 304"
    );
    let after: BTreeMap<String, i64> = store
        .list()
        .expect("list")
        .into_iter()
        .map(|row| (row.id, row.last_alert_at.unwrap_or(0)))
        .collect();
    assert_eq!(before, after, "a silent run must not move lastAlertAt");
}

/// The first invoice is paid by the accepted first payment, not by a card:
/// the live report must never claim a stored-card charge.
#[tokio::test]
async fn first_invoice_without_card_claims_no_charge() {
    if !live_enabled() {
        return;
    }
    let store = SubscriberStore::open(live_root());
    let seller = apex_billing::BillingProfile::from_env().expect("APEX_BILLING_PROFILE");
    let register_path = std::env::var("APEX_LIVE_REGISTER")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(".apex/billing/invoices.jsonl"));
    let vies = apex_billing::HttpViesClient::default();
    let lookup = CurlAppLookup::default();
    let month = current_month(now_unix_ms());
    let report = run_bill(
        &store,
        &month,
        &BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: None,
            transport: None,
            lookup: Some(&lookup),
        },
    )
    .await
    .expect("first live bill");
    println!(
        "first bill {month}: issued={} charged={} refused={} skipped={}",
        report.issued.len(),
        report.charged.len(),
        report.refused.len(),
        report.skipped.len()
    );
    assert!(!report.issued.is_empty());
    for issued in &report.issued {
        println!(
            "first issued {} {} gross={} charged={} paid_by_first_payment={}",
            issued.number,
            issued.publisher,
            issued.gross_micros,
            issued.charged,
            issued.paid_by_first_payment
        );
        assert!(!issued.charged, "no card charge exists without Adyen");
        assert!(issued.paid_by_first_payment);
    }
    assert!(
        report.charged.is_empty(),
        "a first payment is not a stored-card charge"
    );
    assert!(report.refused.is_empty(), "the first invoice needs no card");
    let numbers: Vec<u64> = store
        .list_active()
        .expect("rows")
        .iter()
        .map(|row| row.first_invoice_number.expect("first invoice"))
        .collect();
    let unique: std::collections::BTreeSet<_> = numbers.iter().copied().collect();
    assert_eq!(
        unique.len(),
        numbers.len(),
        "register numbers are gapless and unique"
    );
}

/// The next month's renewal without a card: real invoices are issued with
/// gapless numbers, every stored-card charge is refused with the typed
/// no-card reason, and a rerun neither re-issues nor invents a success.
#[tokio::test]
async fn monthly_renewal_without_card_is_honest_and_idempotent() {
    if !live_enabled() {
        return;
    }
    let store = SubscriberStore::open(live_root());
    let seller = apex_billing::BillingProfile::from_env().expect("APEX_BILLING_PROFILE");
    let register_path = std::env::var("APEX_LIVE_REGISTER")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(".apex/billing/invoices.jsonl"));
    let vies = apex_billing::HttpViesClient::default();
    let month = std::env::var("APEX_LIVE_RENEWAL_MONTH").unwrap_or_else(|_| "2026-11".to_string());
    let lookup = CurlAppLookup::default();
    let deps = BillDeps {
        seller: &seller,
        register_path: &register_path,
        vies: &vies,
        adyen: None,
        transport: None,
        lookup: Some(&lookup),
    };
    let register_before = std::fs::read_to_string(&register_path)
        .unwrap_or_default()
        .lines()
        .count();
    let first = run_bill(&store, &month, &deps)
        .await
        .expect("first live renewal");
    println!(
        "renewal {month}: issued={} charged={} refused={} skipped={}",
        first.issued.len(),
        first.charged.len(),
        first.refused.len(),
        first.skipped.len()
    );
    assert!(!first.issued.is_empty());
    for issued in &first.issued {
        println!(
            "renewal issued {} {} gross={} charged={}",
            issued.number, issued.publisher, issued.gross_micros, issued.charged
        );
        assert!(
            !issued.charged,
            "no Adyen credentials, so no charge may be claimed"
        );
        assert!(!issued.paid_by_first_payment);
    }
    assert!(
        first.charged.is_empty(),
        "a charge without Adyen is impossible"
    );
    assert_eq!(first.refused.len(), first.issued.len());
    for refused in &first.refused {
        println!("refused {}: {}", refused.publisher, refused.reason);
        assert!(
            refused.reason.contains("no stored payment method"),
            "{}",
            refused.reason
        );
    }
    let register_after_first = std::fs::read_to_string(&register_path)
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(
        register_after_first - register_before,
        first.issued.len(),
        "one register row per issued invoice"
    );

    let second = run_bill(&store, &month, &deps)
        .await
        .expect("second live renewal");
    println!(
        "second renewal {month}: issued={} charged={} refused={} skipped={}",
        second.issued.len(),
        second.charged.len(),
        second.refused.len(),
        second.skipped.len()
    );
    assert!(second.issued.is_empty(), "same month must not re-issue");
    assert!(second.charged.is_empty());
    assert_eq!(
        second.refused.len(),
        first.refused.len(),
        "the same charge is retried"
    );
    let register_after_second = std::fs::read_to_string(&register_path)
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(
        register_after_second, register_after_first,
        "a retry consumes no register number"
    );
    let rows = store.list_active().expect("rows");
    let mut numbers: Vec<u64> = rows
        .iter()
        .filter_map(|row| row.last_invoice_number)
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    assert_eq!(
        numbers.len(),
        rows.len(),
        "every invoice number is distinct"
    );
}

/// Real store lookup: known public Apple bundle ids verify, a public Android
/// package verifies through Play, and an id neither store can confirm stays
/// unverified. Run with `APEX_LIVE_WATCH=1`.
#[tokio::test]
async fn live_app_store_lookup_verifies_known_bundle_ids() {
    if !live_enabled() {
        return;
    }
    let lookup = CurlAppLookup::default();
    let apps: Vec<String> = [
        "com.imvu.mobilecordova",
        "com.jirbo.iMahjongFree",
        "com.google.ios.youtube",
        "com.google.android.apps.youtube.music",
        "com.digitalturbine.ignite",
        "com.example.not-a-real-app-apex",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let verification = verify_apps(&apps, &lookup).await;
    println!(
        "live lookup verified={:?} unverified={:?} error={:?}",
        verification.verified, verification.unverified, verification.error
    );
    for known in [
        "com.imvu.mobilecordova",
        "com.jirbo.iMahjongFree",
        "com.google.ios.youtube",
    ] {
        assert!(
            verification.verified.iter().any(|app| app == known),
            "{known} must verify against the public App Store"
        );
    }
    assert!(
        verification
            .verified
            .iter()
            .any(|app| app == "com.google.android.apps.youtube.music"),
        "a public Android package must verify through Google Play"
    );
    assert_eq!(
        verification.verified_count, 4,
        "only the four real listings are counted"
    );
    assert!(
        verification
            .unverified
            .iter()
            .any(|app| app == "com.digitalturbine.ignite"),
        "an id neither store can confirm stays unverified"
    );
}

/// A3: one public Android package proves the developer's whole Play
/// catalogue, so a publisher naming one app can still be priced on the real
/// portfolio.
#[tokio::test]
async fn live_google_play_catalogue_undercounts_are_closed() {
    if !live_enabled() {
        return;
    }
    let lookup = CurlAppLookup::default();
    let verification = verify_apps(
        &["com.google.android.apps.youtube.music".to_string()],
        &lookup,
    )
    .await;
    println!(
        "live Play portfolio app_store={} play={} names={}",
        verification.app_store_catalogue_count,
        verification.google_play_catalogue_count,
        verification.verified_count
    );
    assert_eq!(verification.verified_count, 1);
    assert_eq!(verification.app_store_catalogue_count, 0);
    assert!(
        verification.google_play_catalogue_count >= 10,
        "the YouTube Music Play catalogue must reach the core threshold: {verification:?}"
    );
    assert_eq!(
        apex_watch::model::tier_from_apps(verification.tier_count()),
        apex_watch::model::Tier::Core
    );
}

/// Real token lifecycle: 128-bit shape, uniqueness, single use and expiry
/// for both the confirmation and billing-profile tokens.
#[test]
fn tokens_are_entropic_single_use_and_expiring() {
    if !live_enabled() {
        return;
    }
    use apex_watch::model::{SubscribeRequest, CONFIRMATION_TTL_MS};
    use apex_watch::WatchError;

    let root = live_root().join("token-tests");
    let _ = std::fs::remove_dir_all(&root);
    let store = SubscriberStore::open(&root);
    let now = now_unix_ms();
    let request = SubscribeRequest {
        domain: "tokens.example".to_string(),
        email: "ops@tokens.example".to_string(),
        accounts: String::new(),
        apps: vec!["one".to_string()],
        verified_app_count: Some(0),
        verification_error: None,
    };
    let first = store.subscribe_pending(now, &request).expect("pending");
    let second = store.subscribe_pending(now + 1, &request).expect("replay");
    assert_eq!(
        first.reference, second.reference,
        "dedup keeps the reference"
    );
    assert_ne!(
        first.confirmation_token, second.confirmation_token,
        "a replay mints a fresh token"
    );
    let mut seen = std::collections::BTreeSet::new();
    for token in [&first.confirmation_token, &second.confirmation_token] {
        assert_eq!(token.len(), 32, "128 bits as 32 hex characters");
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(seen.insert(token.clone()), "tokens are unique");
    }
    let raw = std::fs::read_to_string(store.subscribers_path()).expect("state");
    assert!(
        !raw.contains(&second.confirmation_token),
        "only the hash is stored"
    );

    let expired = store.confirm_email(now + CONFIRMATION_TTL_MS + 2, &second.confirmation_token);
    assert!(
        matches!(expired, Err(WatchError::ConfirmationExpired)),
        "{expired:?}"
    );

    let fresh = store
        .subscribe_pending(
            now,
            &SubscribeRequest {
                domain: "single.example".to_string(),
                email: "ops@single.example".to_string(),
                accounts: String::new(),
                apps: vec!["one".to_string()],
                verified_app_count: Some(0),
                verification_error: None,
            },
        )
        .expect("pending");
    let confirmed = store
        .confirm_email(now, &fresh.confirmation_token)
        .expect("confirm");
    let again = store.confirm_email(now, &fresh.confirmation_token);
    assert!(
        matches!(again, Err(WatchError::ConfirmationUsed)),
        "a consumed token is refused: {again:?}"
    );
    let profile = store
        .create_profile_token_at(now, &confirmed.reference)
        .expect("profile token");
    assert_eq!(profile.len(), 32);
    let profile_expired = store.consume_profile_token(now + CONFIRMATION_TTL_MS, &profile);
    assert!(
        matches!(profile_expired, Err(WatchError::ProfileTokenExpired)),
        "{profile_expired:?}"
    );
    let profile_two = store
        .create_profile_token_at(now, &confirmed.reference)
        .expect("profile token 2");
    store
        .consume_profile_token(now, &profile_two)
        .expect("consume once");
    let profile_again = store.consume_profile_token(now, &profile_two);
    assert!(
        matches!(profile_again, Err(WatchError::ProfileTokenUsed)),
        "{profile_again:?}"
    );
}

/// Real state: a checkpoint plus delta fold must equal the full-log fold,
/// a truncated delta must fall back to the log, and one batch write must
/// record every row. Every state file is owner-only.
#[test]
fn checkpoint_delta_and_batch_over_real_state() {
    if !live_enabled() {
        return;
    }
    use std::os::unix::fs::PermissionsExt as _;

    let root = live_root().join("state-tests");
    let _ = std::fs::remove_dir_all(&root);
    let store = SubscriberStore::open(&root);
    let now = now_unix_ms();
    let mut rows = Vec::new();
    for index in 0..60 {
        let outcome = store
            .subscribe(
                now,
                &format!("state{index}.example"),
                &format!("ops{index}@example.com"),
                "",
                false,
            )
            .expect("real subscribe");
        rows.push(outcome.subscriber);
    }
    for (index, row) in rows.iter_mut().enumerate() {
        row.last_check_at = Some(now + index as i64);
        store.update(row).expect("real update");
    }
    store.checkpoint().expect("real checkpoint");
    for row in &mut rows {
        row.last_alert_at = Some(now + 1_000);
    }
    store.update_many(&rows).expect("one batch update");
    let folded = store.list().expect("checkpoint+delta fold");
    let full = naive_fold(&store.subscribers_path());
    assert_eq!(folded, full, "checkpoint+delta equals the full log");
    assert_eq!(folded.len(), 60);

    // A crash-truncated delta must recover the same rows from the full log.
    let delta = std::fs::read(store.delta_path()).expect("delta");
    assert!(delta.len() > 32);
    std::fs::write(store.delta_path(), &delta[..delta.len() - 20]).expect("truncate delta");
    let recovered = SubscriberStore::open(&root)
        .list()
        .expect("truncated delta fallback");
    assert_eq!(recovered, full, "a truncated delta falls back to the log");
    assert!(recovered.iter().all(|row| row.last_alert_at.is_some()));

    // Batch updates: the whole slice is one append and every row lands.
    let mut batch = recovered;
    for (index, row) in batch.iter_mut().enumerate() {
        row.last_check_at = Some(now + 2_000 + index as i64);
    }
    let before = std::fs::metadata(store.subscribers_path()).unwrap().len();
    store.update_many(&batch).expect("batch update");
    let after = std::fs::metadata(store.subscribers_path()).unwrap().len();
    println!(
        "batch_rows={} bytes_written={}",
        batch.len(),
        after - before
    );
    assert!(after > before);
    let listed = SubscriberStore::open(&root).list().expect("after batch");
    assert_eq!(listed, batch, "every batched row is recorded");

    for path in [
        store.subscribers_path(),
        store.checkpoint_path(),
        store.delta_path(),
    ] {
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        println!("mode {} {:o}", path.display(), mode);
        assert_eq!(mode, 0o600, "{}", path.display());
    }
}

fn naive_fold(path: &std::path::Path) -> Vec<apex_watch::Subscriber> {
    let raw = std::fs::read_to_string(path).expect("full log");
    let mut order = Vec::new();
    let mut by_id = std::collections::HashMap::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let row: apex_watch::Subscriber = serde_json::from_str(line).expect("row");
        if !by_id.contains_key(&row.id) {
            order.push(row.id.clone());
        }
        by_id.insert(row.id.clone(), row);
    }
    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}

/// A checkpoint+delta fold over a real 100,000-row history: the fresh reader
/// must read the checkpoint rows, not the whole log, and match the full fold.
#[test]
fn checkpoint_fold_matches_full_log_at_100k() {
    if !live_enabled() {
        return;
    }
    use std::io::Write as _;

    let root = live_root().join("fold-100k");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("fold root");
    let path = root.join("subscribers.jsonl");
    let mut file = std::fs::File::create(&path).expect("history file");
    let started = std::time::Instant::now();
    for version in 0..100u32 {
        for id in 0..1_000u32 {
            let row = serde_json::json!({
                "id": format!("sub_{id:032x}"),
                "publisher": format!("history{id}"),
                "domain": format!("history{id}.example"),
                "email": format!("ops{id}@example.com"),
                "tier": "normal",
                "createdMs": 10,
                "status": "active",
                "confirmed": true,
                "lastCheckAt": version,
                "cancelTokenHash": format!("{id:064x}")
            });
            writeln!(file, "{row}").expect("history row");
        }
    }
    file.sync_all().expect("history durable");
    drop(file);
    println!("wrote 100000 rows in {:?}", started.elapsed());

    let store = SubscriberStore::open(&root);
    let checkpoint_started = std::time::Instant::now();
    store.checkpoint().expect("checkpoint");
    let checkpoint_elapsed = checkpoint_started.elapsed();
    let rows_read_at_checkpoint = store.rows_read();

    let reopened = SubscriberStore::open(&root);
    let fold_started = std::time::Instant::now();
    let folded = reopened.list().expect("checkpoint fold");
    let fold_elapsed = fold_started.elapsed();
    println!(
        "checkpoint_ms={} fold_ms={} checkpoint_rows_read={} fold_rows_read={} rows={}",
        checkpoint_elapsed.as_millis(),
        fold_elapsed.as_millis(),
        rows_read_at_checkpoint,
        reopened.rows_read(),
        folded.len()
    );
    assert_eq!(folded.len(), 1_000);
    assert_eq!(reopened.rows_read(), 1_000);
    assert!(folded.iter().all(|row| row.last_check_at == Some(99)));
    for path in [
        root.join("subscribers.jsonl"),
        root.join("subscribers.checkpoint.jsonl"),
        root.join("subscribers.delta.jsonl"),
    ] {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        println!("mode {} {:o}", path.display(), mode);
        assert_eq!(mode, 0o600, "{}", path.display());
    }
}
