//! `apex-runner` — the scheduled job entrypoint for one publisher.
//!
//! One process serves one publisher's ledger. systemd timers invoke it on a
//! schedule; the logic of each job lives in the library (`apex_autopilot::cli`
//! and `apex_autopilot::cycle`) so it is unit tested; this binary only reads
//! the environment, builds the platform adapter and the runner, prints the
//! report (human by default, `--json` for the versioned document) and exports
//! the bounded metrics textfile.
//!
//! Commands: `audit`, `cycle`, `daily`, `monthly`, `recheck`, `aa`, `aa-eval`.
//! Configuration (environment): `APEX_RUNNER_LEDGER`, `APEX_RUNNER_PUBLISHER`,
//! `APEX_RUNNER_SIGNING_KEY_HEX`, `APEX_MAX_API_KEY`, `APEX_MAX_REPORT_KEY`,
//! `APEX_MAX_MANAGEMENT_BASE`, `APEX_MAX_REPORTING_BASE`,
//! `APEX_RUNNER_WRITABLE`, `APEX_RUNNER_POLICY`, `APEX_RUNNER_PRIORS`,
//! `APEX_RUNNER_SNAPSHOT`, `APEX_DEVELOPER_SITE`, `APEX_EXPECTED_NETWORKS`,
//! `APEX_ADYEN_API_KEY`, `APEX_ADYEN_MERCHANT_ACCOUNT`, `APEX_ADYEN_HMAC_KEY`,
//! `APEX_ADYEN_CHECKOUT_BASE`, `APEX_BILLING_VAT_RATE_BP` (the apex-api name;
//! the historical `APEX_INVOICE_VAT_BP` still reads as a deprecated fallback),
//! `APEX_MONITORING_FEE_MICROS` (the apex-api name; the historical
//! `APEX_MONITORING_MONTHLY_MICROS` still reads as a deprecated fallback),
//! `APEX_ALERT_FILE`,
//! `APEX_METRICS_TEXTFILE`, `APEX_CLICKHOUSE_HTTP`,
//! `APEX_ANALYTICS_ENVIRONMENT`, `APEX_ANALYTICS_SPOOL` (default
//! `<APEX_RUNNER_DATA_DIR>/analytics/spool.jsonl`), `APEX_RUNNER_DATA_DIR`
//! (default `.apex`), `APEX_NOTIFY_CONTACTS`,
//! `APEX_CONSOLE_URL`, `APEX_NOTIFY_WEBHOOK`, `APEX_NOTIFY_STATE`,
//! `APEX_OPERATOR_JOBS_DIR`, `APEX_PAYMENT_EVENTS`, `APEX_PAYMENT_LEDGER`,
//! `APEX_PAYMENT_RETRIES`, `APEX_PAYMENT_CURSOR` (default
//! `<APEX_PAYMENT_EVENTS>.cursor`).
//!
//! Timers should pass their expected start time so the schedule-lag gauge is
//! recorded: `--scheduled-at <unix-ms>` or `APEX_SCHEDULED_AT_MS`. Every
//! command re-delivers the analytics spool at start (logged as
//! `apex-runner: analytics spool: N batches re-delivered` when N > 0).

#![forbid(unsafe_code)]

use std::sync::Arc;

use apex_connectors::max::{MAX_MANAGEMENT_BASE, MAX_REPORTING_BASE};
use apex_connectors::transport::{
    MeteredTransport, ReqwestTransport, RetryTransport, TokioSleeper, Transport,
};
use apex_metrics::vocab::Outcome;
use apex_proof::{Keypair, Ledger};

/// `cli::required` bound to the process environment.
fn cli_required(name: &str) -> Result<String, String> {
    apex_autopilot::cli::required(|key| std::env::var(key).ok(), name)
}

fn optional(name: &str) -> Option<String> {
    apex_autopilot::cli::required(|key| std::env::var(key).ok(), name).ok()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args
        .first()
        .map(String::as_str)
        .ok_or("usage: apex-runner <audit|daily|cycle|monthly|recheck|aa|aa-eval> [flags]")?;
    let as_json = args.iter().any(|argument| argument == "--json");
    let now_unix_ms = apex_autopilot::cli::now_unix_ms();

    let instrumentation =
        apex_autopilot::cli::build_instrumentation(|name| std::env::var(name).ok())?;
    for line in &instrumentation.logs {
        eprintln!("{line}");
    }
    // One idempotent schema setup per process: the first run on a fresh
    // deployment creates the store instead of spooling against nothing.
    if let Some(projections) = &instrumentation.projections {
        if let Err(error) = projections.ensure_schema().await {
            eprintln!("apex-runner: analytics schema setup failed: {error}");
        }
    }
    let registry = instrumentation.registry.clone();

    // Retry outside, meter inside: every attempt is its own observation.
    let inner: Arc<dyn Transport> = Arc::new(MeteredTransport::new(
        Arc::new(ReqwestTransport::new().map_err(|e| format!("transport: {e}"))?),
        Some(registry.clone()),
    ));
    let transport: Arc<dyn Transport> =
        Arc::new(RetryTransport::new(inner, 2, Arc::new(TokioSleeper)));

    // `audit` builds the account snapshot from the MAX keys and needs no
    // ledger; every other command runs against the publisher's ledger.
    if command == "audit" {
        let publisher = optional("APEX_RUNNER_PUBLISHER").unwrap_or_default();
        // The audit events require the publisher label, and the
        // instrumentation handle is built before the publisher is known:
        // stamp it here so audit_completed/audit_failed can land.
        let audit_events = instrumentation.events.as_ref().map(|events| {
            if publisher.is_empty() {
                events.clone()
            } else {
                Arc::new((**events).clone().with_publisher(publisher.clone()))
            }
        });
        // Drain any spool left by earlier failed runs before this command.
        apex_autopilot::cli::replay_spool(audit_events.as_deref()).await;
        // A timer that knows its expected start passes it; the audit job's
        // lag sample lands the same way the dispatch path records its own.
        if let Some(scheduled_at) =
            apex_autopilot::cli::scheduled_at(&args, |name| std::env::var(name).ok())?
        {
            let lag_seconds = (now_unix_ms.saturating_sub(scheduled_at) as f64 / 1_000.0).max(0.0);
            instrumentation
                .telemetry
                .schedule_lag(apex_metrics::vocab::JobType::Audit, lag_seconds);
        }
        let rule_version = apex_autopilot::cli::audit_rule_version();
        let snapshot = apex_autopilot::audited_snapshot(
            &cli_required("APEX_MAX_API_KEY")?,
            optional("APEX_MAX_REPORT_KEY").as_deref(),
            &std::env::var("APEX_MAX_MANAGEMENT_BASE")
                .unwrap_or_else(|_| MAX_MANAGEMENT_BASE.to_string()),
            &std::env::var("APEX_MAX_REPORTING_BASE")
                .unwrap_or_else(|_| MAX_REPORTING_BASE.to_string()),
            transport,
            &cli_required("APEX_DEVELOPER_SITE")?,
            apex_autopilot::cli::split_csv_env(
                |name| std::env::var(name).ok(),
                "APEX_EXPECTED_NETWORKS",
            ),
            audit_events.as_deref(),
            &rule_version,
            now_unix_ms,
        )
        .await
        .map_err(|e| format!("snapshot: {e}"))?;
        let out = apex_autopilot::cli::flag(&args, "--out")
            .or_else(|| optional("APEX_RUNNER_SNAPSHOT"))
            .ok_or("audit requires --out (or APEX_RUNNER_SNAPSHOT)")?;
        apex_autopilot::write_snapshot(std::path::Path::new(&out), &snapshot)?;

        // The audit engine runs over the snapshot, and the result is a
        // durable event, a bounded metric and the audit-ready message.
        let audit_started = std::time::Instant::now();
        let report = apex_audit::run(&snapshot);
        let duration_ms = audit_started.elapsed().as_millis() as u64;
        let critical = report
            .findings
            .iter()
            .filter(|finding| finding.severity.as_str() == "critical")
            .count();
        let warning = report
            .findings
            .iter()
            .filter(|finding| finding.severity.as_str() == "warning")
            .count();
        let info = report.findings.len() - critical - warning;
        let (eligible, expected, missing) = apex_autopilot::cli::audit_coverage(&snapshot);
        instrumentation
            .telemetry
            .audit_run(&rule_version, Outcome::Completed);
        if let Some(events) = &audit_events {
            events
                .audit_completed(
                    &report.snapshot_digest,
                    &rule_version,
                    eligible,
                    expected,
                    critical as u32,
                    warning as u32,
                    info as u32,
                    missing.clone(),
                    duration_ms,
                    now_unix_ms,
                )
                .await;
        }
        if let Some(notify) = &instrumentation.notify {
            if !publisher.is_empty() {
                notify
                    .audit_ready(
                        &publisher,
                        &apex_autopilot::calendar::format_day(now_unix_ms.div_euclid(86_400_000)),
                        now_unix_ms,
                    )
                    .await;
            }
        }

        let run_reference = apex_autopilot::cli::run_reference(now_unix_ms);
        if as_json {
            let document = serde_json::json!({
                "schema_version": apex_autopilot::cli::REPORT_SCHEMA_VERSION,
                "command": "audit",
                "run_reference": run_reference,
                "publisher": if publisher.is_empty() { serde_json::Value::Null } else { serde_json::Value::from(publisher) },
                "snapshot": {
                    "path": out,
                    "ad_units": snapshot.ad_units.len(),
                    "digest": report.snapshot_digest,
                },
                "audit": {
                    "findings": report.findings.len(),
                    "critical": critical,
                    "warning": warning,
                    "info": info,
                    "eligible_rule_groups": eligible,
                    "expected_rule_groups": expected,
                    "missing_inputs": missing,
                    "rule_version": rule_version,
                },
            });
            println!("{document}");
        } else {
            println!(
                "Mode: MAX / Publisher: {}",
                if publisher.is_empty() {
                    "not configured"
                } else {
                    publisher.as_str()
                }
            );
            println!(
                "Audit: {} finding(s) ({} critical / {} warning / {} info)",
                report.findings.len(),
                critical,
                warning,
                info
            );
            if !missing.is_empty() {
                println!(
                    "Audit coverage: {eligible}/{expected} rule group(s); missing {missing:?}"
                );
            }
            println!(
                "Snapshot: wrote {} ad units to {out}",
                snapshot.ad_units.len()
            );
            println!("Run reference: {run_reference}");
        }
        export_textfile(&registry);
        return Ok(());
    }

    let keypair = Keypair::from_bytes(&apex_autopilot::cli::parse_seed(&cli_required(
        "APEX_RUNNER_SIGNING_KEY_HEX",
    )?)?);
    let mut ledger = Ledger::open(&cli_required("APEX_RUNNER_LEDGER")?, keypair)
        .map_err(|e| format!("ledger: {e}"))?;
    let ops =
        apex_autopilot::cli::build_max_ops(|name| std::env::var(name).ok(), transport.clone())
            .map_err(|e| format!("max adapter: {e}"))?;
    let runner = apex_autopilot::cli::build_runner(
        |name| std::env::var(name).ok(),
        ops,
        transport,
        &instrumentation,
    )
    .map_err(|e| format!("configuration: {e}"))?;

    let outcome = apex_autopilot::cli::dispatch(command, &args, &runner, &mut ledger, now_unix_ms)
        .await
        .map_err(|e| e.to_string())?;
    if as_json {
        println!("{}", outcome.json);
    } else {
        println!("{}", outcome.human);
    }
    export_textfile(&registry);
    if outcome.failed {
        std::process::exit(1);
    }
    Ok(())
}

/// Write the bounded metrics textfile when `APEX_METRICS_TEXTFILE` is set; a
/// write failure is reported, never fatal to the job that already ran.
fn export_textfile(registry: &apex_metrics::Registry) {
    match apex_metrics::textfile::export_from_env(|name| std::env::var(name).ok(), registry) {
        Ok(Some(path)) => eprintln!("apex-metrics: wrote {}", path.display()),
        Ok(None) => {}
        Err(error) => eprintln!("apex-metrics: textfile export failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    #[test]
    fn cli_required_reads_the_process_environment() {
        std::env::set_var("APEX_RUNNER_TEST_VAR", "value");
        assert_eq!(
            super::cli_required("APEX_RUNNER_TEST_VAR").unwrap(),
            "value"
        );
        std::env::remove_var("APEX_RUNNER_TEST_VAR");
        assert!(super::cli_required("APEX_RUNNER_TEST_VAR").is_err());
    }

    #[test]
    fn optional_returns_none_for_blanks_and_missing() {
        std::env::set_var("APEX_RUNNER_TEST_OPT", "  ");
        assert!(super::optional("APEX_RUNNER_TEST_OPT").is_none());
        std::env::remove_var("APEX_RUNNER_TEST_OPT");
        assert!(super::optional("APEX_RUNNER_TEST_OPT").is_none());
    }

    #[test]
    fn textfile_export_is_a_no_op_without_configuration() {
        std::env::remove_var(apex_metrics::textfile::TEXTFILE_ENV);
        let registry = apex_metrics::Registry::new();
        let _ = registry.increment(
            &apex_metrics::catalog::JOB_RUNS_TOTAL,
            &[
                ("job_type", "daily"),
                ("platform", "applovin_max"),
                ("outcome", "completed"),
            ],
        );
        super::export_textfile(&registry);
    }
}
