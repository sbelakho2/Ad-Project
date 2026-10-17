//! `apex-watch` — the scheduled setup watch jobs.
//!
//! Commands: `daily` (fetch, rule, diff, alert), `bill [YYYY-MM]` (issue the
//! month's setup-watch lines; the month defaults to the current UTC month),
//! `list` (print the current subscriber rows) and `cancel <token>` (record
//! the cancellation the token names).
//!
//! Configuration (environment):
//!
//! - `APEX_WATCH_ROOT` — the subscriber root, default `.apex/watch` under
//!   the unit's `WorkingDirectory` (`/var/lib/apex` in the deployment).
//! - `APEX_NOTIFY_SINK` — `email` selects the `apex-notify` email sink;
//!   `APEX_NOTIFY_FROM` names the sender and `APEX_NOTIFY_SENDMAIL` overrides
//!   the `sendmail` path. The legacy `APEX_NOTIFY_WEBHOOK` and
//!   `APEX_NOTIFY_FILE` sinks keep working when `APEX_NOTIFY_SINK` is unset.
//!   A daily run without any configured sink refuses to start.
//! - `APEX_WATCH_PAGE_URL` — the watch page URL for the alert copy; when
//!   unset, `APEX_CONSOLE_URL` plus `/watch` is used.
//! - `APEX_BILLING_PROFILE` — the seller profile the monthly VAT treatment
//!   loads; `APEX_BILLING_VAT_RATE_BP` overrides its domestic rate. The
//!   invoice register defaults to the runner's `.apex/billing/invoices.jsonl`.
//!
//! The crate holds no platform keys and no runner state: the fetch uses the
//! system `curl`, and the only credentials any command needs live in the
//! environment file named by the unit.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::path::PathBuf;

use apex_watch::{
    current_month, now_unix_ms, run_bill, run_daily, watch_page_url, BillDeps, CurlFetcher,
    DailyDeps, StdResolver, SubscriberStore,
};

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

const USAGE: &str = "usage: apex-watch <daily|bill [YYYY-MM]|list|cancel <token>>";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("daily") => daily().await,
        Some("bill") => bill(args.get(1).map(String::as_str)),
        Some("list") => list(),
        Some("cancel") => cancel(args.get(1).map(String::as_str)),
        _ => Err(USAGE.into()),
    }
}

async fn daily() -> Result<(), Box<dyn std::error::Error>> {
    let store = SubscriberStore::from_env(env_value);
    let sink = apex_notify::select_sink(env_value)
        .map_err(|error| format!("the notification sink is misconfigured: {error}"))?
        .ok_or(
            "no notification sink is configured; set APEX_NOTIFY_SINK=email with APEX_NOTIFY_FROM \
             (the webhook and file sinks keep working as fallbacks)",
        )?;
    let fetcher = CurlFetcher::default();
    let resolver = StdResolver;
    let deps = DailyDeps {
        fetcher: &fetcher,
        resolver: &resolver,
        sink: sink.as_ref(),
        watch_page_url: watch_page_url(&env_value),
    };
    let report = run_daily(&store, now_unix_ms(), &deps)
        .await
        .map_err(|error| format!("the setup watch check failed: {error}"))?;
    println!(
        "setup watch daily: {} checked, {} alerted, {} unchanged, {} unreachable",
        report.checked, report.alerted, report.unchanged, report.unreachable
    );
    for entry in &report.entries {
        if let Some(reason) = &entry.unreachable {
            println!("  {}: unreachable ({reason})", entry.publisher);
        }
        if let Some(error) = &entry.alert_error {
            eprintln!(
                "  {}: the alert could not be sent: {error}",
                entry.publisher
            );
        }
    }
    for failure in &report.failures {
        eprintln!("  {failure}");
    }
    let alert_failures = report
        .entries
        .iter()
        .filter(|entry| entry.alert_error.is_some())
        .count();
    if alert_failures > 0 || !report.failures.is_empty() {
        return Err(format!(
            "{} alert(s) and {} subscriber record(s) failed to record",
            alert_failures,
            report.failures.len()
        )
        .into());
    }
    Ok(())
}

fn bill(month_arg: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let month = month_arg
        .map(str::to_string)
        .unwrap_or_else(|| current_month(now_unix_ms()));
    let store = SubscriberStore::from_env(env_value);
    let seller = apex_billing::BillingProfile::from_env().map_err(|error| {
        format!("the seller billing profile (APEX_BILLING_PROFILE) could not be loaded: {error}")
    })?;
    let register_path = PathBuf::from(apex_billing::DEFAULT_REGISTER_PATH);
    let report = run_bill(
        &store,
        &month,
        &BillDeps {
            seller: &seller,
            register_path: &register_path,
        },
    )
    .map_err(|error| format!("the setup watch bill failed: {error}"))?;
    println!(
        "setup watch bill {month}: {} issued, {} refused, {} skipped",
        report.issued.len(),
        report.refused.len(),
        report.skipped.len()
    );
    for issued in &report.issued {
        println!(
            "  invoice {} for {} ({}): {} micros gross",
            issued.number,
            issued.publisher,
            issued.tier.as_str(),
            issued.gross_micros
        );
    }
    for refused in &report.refused {
        eprintln!("  {}: refused: {}", refused.publisher, refused.reason);
    }
    if !report.refused.is_empty() {
        return Err(format!("{} subscriber(s) could not be billed", report.refused.len()).into());
    }
    Ok(())
}

fn list() -> Result<(), Box<dyn std::error::Error>> {
    let store = SubscriberStore::from_env(env_value);
    let subscribers = store
        .list()
        .map_err(|error| format!("the subscriber store could not be read: {error}"))?;
    println!("setup watch subscribers: {}", subscribers.len());
    for subscriber in subscribers {
        println!(
            "  {} {} {} {} status={} lastCheck={:?} lastAlert={:?}",
            subscriber.id,
            subscriber.domain,
            subscriber.email,
            subscriber.tier.as_str(),
            subscriber.status.as_str(),
            subscriber.last_check_at,
            subscriber.last_alert_at
        );
    }
    Ok(())
}

fn cancel(token_arg: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let token = token_arg.ok_or(USAGE)?;
    let store = SubscriberStore::from_env(env_value);
    match store
        .cancel(token, now_unix_ms())
        .map_err(|error| format!("the subscriber store could not be read: {error}"))?
    {
        Some(subscriber) => {
            println!(
                "setup watch canceled: {} ({})",
                subscriber.publisher, subscriber.domain
            );
            Ok(())
        }
        None => Err("no active subscriber matches that cancel token".into()),
    }
}
