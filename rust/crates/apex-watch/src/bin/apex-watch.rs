//! `apex-watch` — the scheduled setup watch jobs.
//!
//! Commands: `daily` (fetch, rule, diff, alert), `bill [YYYY-MM]` (issue and
//! report the free watch's daily results; the month defaults to the current
//! UTC month), `list` (print the current subscriber rows) and `cancel
//! <token>` (record the cancellation the token names).
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
//! - `APEX_MOLLIE_API_KEY`, `APEX_MOLLIE_API_BASE`,
//!   `APEX_MOLLIE_WEBHOOK_URL` and `APEX_MOLLIE_REDIRECT_URL` — together they
//!   configure the Mollie client the monthly bill calls with
//!   `sequenceType: recurring` for the stored mandate. Without them the
//!   invoice is still issued and the charge is refused with the typed reason;
//!   a partial set is
//!   a startup error, never a silently skipped charge.
//! - VIES: reverse charge is confirmed through the real VIES REST API over
//!   TLS with a bounded timeout. A register that cannot answer refuses reverse
//!   charge (a typed refusal) instead of guessing.
//!
//! The fetch uses the system `curl` pinned with `--resolve
//! host:port:address` to the address the resolve-time guard approved, so the
//! host name is never resolved a second time. The crate holds no runner state:
//! the only credentials any command needs live in the environment.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use apex_watch::{
    now_unix_ms, run_daily, watch_page_url, CurlFetcher, DailyDeps, StdResolver, SubscriberStore,
};

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

const USAGE: &str = "usage: apex-watch <daily|list|cancel <token>>";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("daily") => daily().await,
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
        "setup watch daily: {} checked, {} alerted, {} unchanged, {} unreachable, {} skipped",
        report.checked,
        report.alerted,
        report.unchanged,
        report.unreachable,
        report.skipped.len()
    );
    for skip in &report.skipped {
        println!(
            "  {} ({}): skipped: {}",
            skip.publisher, skip.domain, skip.reason
        );
    }
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
