//! `apex-runner` — the scheduled job entrypoint for one publisher (the
//! legacy single-publisher mode) or for every registered publisher (fleet
//! mode).
//!
//! In fleet mode the deployment names the vault root the console writes
//! (`APEX_CONNECTIONS_ROOT`, falling back to the apex-api spelling
//! `APEX_CREDENTIAL_ROOT`) plus the vault key (`APEX_VAULT_KEY_HEX`) and the
//! per-publisher ledger root (`APEX_RUNNER_LEDGER_ROOT`); the runner builds
//! one adapter per publisher whose connection the console validated and loops
//! every command over the registry, skipping unvalidated publishers with a
//! typed reason. Without a registry root, `APEX_RUNNER_PUBLISHER` selects one
//! publisher whose adapter comes from the `APEX_MAX_*` variables and whose
//! ledger is the single `APEX_RUNNER_LEDGER` file — the legacy
//! single-publisher mode, unchanged.
//!
//! systemd timers invoke it on a schedule; the logic of each job lives in the
//! library (`apex_autopilot::cli` and `apex_autopilot::cycle`) so it is unit
//! tested; this binary only reads the environment, builds the platform
//! adapter(s) and the runner(s), prints the report (human by default,
//! `--json` for the versioned document) and exports the bounded metrics
//! textfile on every exit path, hard errors included.
//!
//! The `audit` command builds one snapshot per registered publisher **and
//! platform** from that publisher's vaulted keys and writes each at
//! `<snapshot root>/<publisher>/<platform>.json` (the layout the console
//! reads); a later `cycle` command is handed exactly that publisher's own
//! per-platform files. The snapshot root is `--out` (audit), then
//! `APEX_RUNNER_SNAPSHOT`, then the console spelling `APEX_SNAPSHOT_ROOT`,
//! defaulting to `.apex/snapshots`. Without a registry root the legacy
//! single-publisher mode reads the `APEX_MAX_*` variables and writes the one
//! `APEX_RUNNER_SNAPSHOT` file exactly as before.
//!
//! Commands: `audit`, `cycle`, `daily`, `monthly`, `recheck`, `aa`, `aa-eval`.
//! Configuration (environment): `APEX_RUNNER_LEDGER`,
//! `APEX_RUNNER_LEDGER_ROOT`, `APEX_RUNNER_PUBLISHER` (legacy mode only),
//! `APEX_CONNECTIONS_ROOT` (fallback `APEX_CREDENTIAL_ROOT`), the vault root
//! the console writes, `APEX_VAULT_KEY_HEX`,
//! `APEX_RUNNER_SIGNING_KEY_HEX`, `APEX_MAX_API_KEY`, `APEX_MAX_REPORT_KEY`,
//! `APEX_MAX_MANAGEMENT_BASE`, `APEX_MAX_REPORTING_BASE`,
//! `APEX_RUNNER_WRITABLE`, `APEX_RUNNER_POLICY`, `APEX_RUNNER_PRIORS` (a file
//! in legacy mode, a directory of `<publisher>.json` models in fleet mode),
//! `APEX_RUNNER_SNAPSHOT` (the single file in legacy mode; the snapshot root
//! in fleet mode, falling back to `APEX_SNAPSHOT_ROOT`), `APEX_DEVELOPER_SITE`,
//! `APEX_EXPECTED_NETWORKS`,
//! `APEX_BILLING_PROFILE` (the seller billing profile the monthly VAT
//! issuance loads; `APEX_BILLING_VAT_RATE_BP` overrides its domestic rate),
//! `APEX_MOLLIE_API_KEY`, `APEX_MOLLIE_API_BASE`,
//! `APEX_MOLLIE_WEBHOOK_URL`, `APEX_MOLLIE_REDIRECT_URL`, `APEX_BILLING_VAT_RATE_BP` (the apex-api name;
//! the historical `APEX_INVOICE_VAT_BP` still reads as a deprecated fallback),
//! `APEX_MONITORING_TIER_BASE_MICROS`, `APEX_MONITORING_TIER_MID_MICROS` and
//! `APEX_MONITORING_TIER_TOP_MICROS` (the monitoring tier prices; monitoring
//! is on at connection and the tier is chosen from the connection snapshot's
//! reported 30-day revenue), with `APEX_MONITORING_FEE_MICROS` as the
//! deprecated base-tier fallback (a legacy `0` still turns monitoring off; a
//! zero tier price is a typed error),
//! `APEX_ALERT_FILE`,
//! `APEX_METRICS_TEXTFILE`, `APEX_CLICKHOUSE_HTTP`,
//! `APEX_ANALYTICS_ENV` (the apex-api name; the historical
//! `APEX_ANALYTICS_ENVIRONMENT` still reads as a deprecated fallback),
//! `APEX_ANALYTICS_SPOOL` (default
//! `<APEX_RUNNER_DATA_DIR>/analytics/spool.jsonl`), `APEX_RUNNER_DATA_DIR`
//! (default `.apex`), `APEX_NOTIFY_CONTACTS`,
//! `APEX_CONSOLE_URL`, `APEX_NOTIFY_WEBHOOK`, `APEX_NOTIFY_STATE`,
//! `APEX_OPERATOR_JOBS_DIR`, `APEX_PAYMENT_EVENTS`, `APEX_PAYMENT_LEDGER`,
//! `APEX_PAYMENT_RETRIES`, `APEX_PAYMENT_CURSOR` (default
//! `<APEX_PAYMENT_EVENTS>.cursor`).
//!
//! The monthly pass loads the seller profile named by `APEX_BILLING_PROFILE`
//! and each publisher's customer profile from
//! `<connections root>/<publisher>.billing.json`; a missing profile refuses
//! that publisher's invoice with the typed reason in the report. The invoice
//! register defaults to `.apex/billing/invoices.jsonl`.
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

    // Every exit path past this point exports the registry: the textfile is
    // the pending telemetry of a run that already recorded lag/failure
    // samples, so even a hard error must deliver it before the process ends.
    let result = run_with_textfile_export(
        &instrumentation.registry,
        |name| std::env::var(name).ok(),
        run_job(command, &args, as_json, now_unix_ms, &instrumentation),
    )
    .await;
    match result {
        Ok(false) => Ok(()),
        Ok(true) => std::process::exit(1),
        Err(error) => Err(error),
    }
}

/// Run one command against the built instrumentation. `Ok(true)` means the
/// command reported a failure the process must exit non-zero for; `Err` is a
/// hard error. The caller exports the metrics textfile on both outcomes.
async fn run_job(
    command: &str,
    args: &[String],
    as_json: bool,
    now_unix_ms: i64,
    instrumentation: &apex_autopilot::cli::Instrumentation,
) -> Result<bool, Box<dyn std::error::Error>> {
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

    // `audit` builds the account snapshots (one per registered publisher and
    // platform from the vaulted keys, or one MAX snapshot in legacy mode) and
    // needs no ledger; every other command runs against the publisher's ledger.
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
            apex_autopilot::cli::scheduled_at(args, |name| std::env::var(name).ok())?
        {
            let lag_seconds = (now_unix_ms.saturating_sub(scheduled_at) as f64 / 1_000.0).max(0.0);
            instrumentation
                .telemetry
                .schedule_lag(apex_metrics::vocab::JobType::Audit, lag_seconds);
        }
        let rule_version = apex_autopilot::cli::audit_rule_version();

        // Fleet mode: the deployment names the vault root the console writes,
        // so the audit runs once per registered publisher and platform and
        // writes each snapshot at `<root>/<publisher>/<platform>.json`.
        if let Some(root) =
            apex_autopilot::registry::vault_root_from_env(&|name| std::env::var(name).ok())
        {
            let key_hex = cli_required(apex_autopilot::registry::VAULT_KEY_ENV)?;
            let vault = apex_vault::Vault::from_hex_key(&key_hex).map_err(|error| {
                format!(
                    "{} is not a valid vault key: {error}",
                    apex_autopilot::registry::VAULT_KEY_ENV
                )
            })?;
            let registry = apex_autopilot::registry::PublisherRegistry::load(&root, &vault)
                .map_err(|error| error.to_string())?;
            let snapshot_root = apex_autopilot::cli::flag(args, "--out")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    apex_autopilot::registry::snapshot_root_from_env(&|name| {
                        std::env::var(name).ok()
                    })
                });
            let report = apex_autopilot::cli::audit_fleet_snapshots(
                &registry,
                &snapshot_root,
                transport,
                instrumentation,
                &rule_version,
                now_unix_ms,
            )
            .await
            .map_err(|e| format!("snapshot: {e}"))?;
            let outcome = apex_autopilot::cli::render_fleet_audit(&report, now_unix_ms);
            if as_json {
                println!("{}", outcome.json);
            } else {
                println!("{}", outcome.human);
            }
            return Ok(outcome.failed);
        }

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
        let out = apex_autopilot::cli::flag(args, "--out")
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
        // The audit path emits into the batching emitter; without this flush
        // the audit_completed family is lost when the process exits.
        if let Some(events) = &audit_events {
            events.flush().await;
        }
        return Ok(false);
    }

    let keypair = Keypair::from_bytes(&apex_autopilot::cli::parse_seed(&cli_required(
        "APEX_RUNNER_SIGNING_KEY_HEX",
    )?)?);
    // Fleet mode: the deployment names the vault root the console writes, so
    // every command loops over the registered publishers. Without it the
    // legacy single-publisher mode reads `APEX_RUNNER_PUBLISHER` and the
    // single `APEX_RUNNER_LEDGER` file exactly as before.
    let registry_root =
        apex_autopilot::registry::vault_root_from_env(&|name| std::env::var(name).ok());
    let outcome = match registry_root {
        Some(root) => {
            let key_hex = cli_required(apex_autopilot::registry::VAULT_KEY_ENV)?;
            let vault = apex_vault::Vault::from_hex_key(&key_hex).map_err(|error| {
                format!(
                    "{} is not a valid vault key: {error}",
                    apex_autopilot::registry::VAULT_KEY_ENV
                )
            })?;
            let registry = apex_autopilot::registry::PublisherRegistry::load(&root, &vault)
                .map_err(|error| error.to_string())?;
            let ledger_root = cli_required(apex_autopilot::registry::LEDGER_ROOT_ENV)?;
            // The snapshot root the audit wrote is the root the cycle reads
            // each publisher's own `--snapshot` files from.
            let snapshot_root =
                apex_autopilot::registry::snapshot_root_from_env(&|name| std::env::var(name).ok());
            let fleet = apex_autopilot::cli::build_fleet(
                |name| std::env::var(name).ok(),
                &registry,
                std::path::Path::new(&ledger_root),
                &root,
                &snapshot_root,
                &keypair,
                transport,
                instrumentation,
            )
            .map_err(|e| format!("configuration: {e}"))?;
            apex_autopilot::cli::dispatch_fleet(command, args, &fleet, now_unix_ms)
                .await
                .map_err(|e| e.to_string())?
        }
        None => {
            let mut ledger = Ledger::open(&cli_required("APEX_RUNNER_LEDGER")?, keypair)
                .map_err(|e| format!("ledger: {e}"))?;
            let ops = apex_autopilot::cli::build_max_ops(
                |name| std::env::var(name).ok(),
                transport.clone(),
            )
            .map_err(|e| format!("max adapter: {e}"))?;
            let runner = apex_autopilot::cli::build_runner(
                |name| std::env::var(name).ok(),
                ops,
                transport,
                instrumentation,
            )
            .map_err(|e| format!("configuration: {e}"))?;
            apex_autopilot::cli::dispatch(command, args, &runner, &mut ledger, now_unix_ms)
                .await
                .map_err(|e| e.to_string())?
        }
    };
    if as_json {
        println!("{}", outcome.json);
    } else {
        println!("{}", outcome.human);
    }
    Ok(outcome.failed)
}

/// Run `job` and export the registry's textfile afterwards, whatever the job
/// did: a dispatch error must still produce the textfile the run recorded.
/// The job's own result is returned unchanged.
async fn run_with_textfile_export<T, E>(
    registry: &apex_metrics::Registry,
    lookup: impl Fn(&str) -> Option<String>,
    job: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let result = job.await;
    export_textfile_with(&lookup, registry);
    result
}

/// Write the bounded metrics textfile when `APEX_METRICS_TEXTFILE` is set; a
/// write failure is reported, never fatal to the job that already ran. The
/// lookup drives `APEX_METRICS_TEXTFILE` resolution so the module tests can
/// exercise both configured and unconfigured states without touching the
/// process environment.
fn export_textfile_with(
    lookup: &impl Fn(&str) -> Option<String>,
    registry: &apex_metrics::Registry,
) {
    match apex_metrics::textfile::export_from_env(lookup, registry) {
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
        let registry = apex_metrics::Registry::new();
        let _ = registry.increment(
            &apex_metrics::catalog::JOB_RUNS_TOTAL,
            &[
                ("job_type", "daily"),
                ("platform", "applovin_max"),
                ("outcome", "completed"),
            ],
        );
        super::export_textfile_with(&|_| None, &registry);
    }

    #[tokio::test]
    async fn a_dispatch_error_still_produces_the_textfile() {
        let dir = std::env::temp_dir().join(format!("apex-runner-textfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("runner.prom");
        let registry = apex_metrics::Registry::new();
        let _ = registry.increment(
            &apex_metrics::catalog::JOB_RUNS_TOTAL,
            &[
                ("job_type", "daily"),
                ("platform", "applovin_max"),
                ("outcome", "failed"),
            ],
        );
        // The same lookup path the process environment provides, driven
        // explicitly so the test is independent of the real environment.
        let lookup = |name: &str| {
            (name == apex_metrics::textfile::TEXTFILE_ENV).then(|| path.display().to_string())
        };
        let result: Result<(), String> =
            super::run_with_textfile_export(&registry, lookup, async {
                Err("dispatch: the platform refused the pull".to_string())
            })
            .await;
        assert!(result.is_err(), "the job error is returned unchanged");
        let text = std::fs::read_to_string(&path).expect("textfile written despite the error");
        assert!(text.contains("apex_job_runs_total"), "{text}");
        assert!(text.contains("outcome=\"failed\""), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_successful_job_exports_the_textfile_too() {
        let dir =
            std::env::temp_dir().join(format!("apex-runner-textfile-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("runner.prom");
        let registry = apex_metrics::Registry::new();
        let lookup = |name: &str| {
            (name == apex_metrics::textfile::TEXTFILE_ENV).then(|| path.display().to_string())
        };
        let result: Result<bool, String> =
            super::run_with_textfile_export(&registry, lookup, async { Ok(true) }).await;
        assert!(result.expect("job ok"));
        assert!(path.is_file(), "the textfile was written");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
