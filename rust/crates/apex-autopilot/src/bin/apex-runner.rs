//! `apex-runner` — the scheduled job entrypoint for one publisher.
//!
//! One process serves one publisher's ledger. systemd timers invoke it on a
//! schedule; each command is one idempotent job over the same durable ledger:
//!
//! - `daily` — pull reporting, record the pull, settle complete windows, and
//!   feed the results back into the bucket model;
//! - `cycle` — generate proposals from an audit snapshot and the bucket
//!   model, then run the write loop (gates, pre-registration, experiment);
//! - `monthly` — bill the month just closed and collect through Adyen;
//! - `recheck` — list kept changes whose month-6 re-check is due;
//! - `aa` / `aa-eval` — start and evaluate a null (A/A) calibration run.
//!
//! Configuration (environment): `APEX_RUNNER_LEDGER`, `APEX_RUNNER_PUBLISHER`,
//! `APEX_RUNNER_SIGNING_KEY_HEX`, `APEX_MAX_API_KEY`, `APEX_MAX_REPORT_KEY`,
//! `APEX_MAX_MANAGEMENT_BASE`, `APEX_MAX_REPORTING_BASE`,
//! `APEX_RUNNER_WRITABLE`, `APEX_RUNNER_POLICY`, `APEX_RUNNER_PRIORS`,
//! `APEX_ADYEN_API_KEY`, `APEX_ADYEN_MERCHANT_ACCOUNT`,
//! `APEX_ADYEN_HMAC_KEY`, `APEX_ADYEN_CHECKOUT_BASE`, `APEX_INVOICE_VAT_BP`,
//! `APEX_RUNNER_SNAPSHOT`,
//! `APEX_MONITORING_MONTHLY_MICROS`, `APEX_ALERT_FILE`.

#![forbid(unsafe_code)]

use std::sync::Arc;

use apex_autopilot::alerts::{AlertSink, FileAlertSink};
use apex_autopilot::live_max::{MaxPlatformOps, SystemWall};
use apex_autopilot::model::AutopilotPolicy;
use apex_autopilot::runner::Runner;
use apex_connectors::max::{MAX_MANAGEMENT_BASE, MAX_REPORTING_BASE};
use apex_connectors::transport::{ReqwestTransport, RetryTransport, TokioSleeper};
use apex_lift::guardrails::FillFormat;
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
        .ok_or("usage: apex-runner <daily|cycle|monthly|recheck|aa|aa-eval> [flags]")?;

    let ledger_path = cli_required("APEX_RUNNER_LEDGER")?;
    let keypair = Keypair::from_bytes(&apex_autopilot::cli::parse_seed(&cli_required(
        "APEX_RUNNER_SIGNING_KEY_HEX",
    )?)?);
    let ledger = Ledger::open(&ledger_path, keypair).map_err(|e| format!("ledger: {e}"))?;

    let transport: Arc<dyn apex_connectors::transport::Transport> = Arc::new(RetryTransport::new(
        Arc::new(ReqwestTransport::new().map_err(|e| format!("transport: {e}"))?),
        2,
        Arc::new(TokioSleeper),
    ));
    let writable: Vec<String> = std::env::var("APEX_RUNNER_WRITABLE")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let ops: Arc<dyn apex_autopilot::ops::PlatformOps> = Arc::new(MaxPlatformOps::new(
        &cli_required("APEX_MAX_API_KEY")?,
        &cli_required("APEX_MAX_REPORT_KEY")?,
        &std::env::var("APEX_MAX_MANAGEMENT_BASE")
            .unwrap_or_else(|_| MAX_MANAGEMENT_BASE.to_string()),
        &std::env::var("APEX_MAX_REPORTING_BASE")
            .unwrap_or_else(|_| MAX_REPORTING_BASE.to_string()),
        transport.clone(),
        Arc::new(SystemWall),
        writable,
    )?);

    let policy = match std::env::var("APEX_RUNNER_POLICY") {
        Ok(raw) if !raw.trim().is_empty() => {
            serde_json::from_str::<AutopilotPolicy>(&raw).map_err(|e| format!("policy: {e}"))?
        }
        _ => AutopilotPolicy::conservative(),
    };
    let mut runner = Runner::new(ops, policy, FillFormat::Rewarded);
    if let Some(publisher) = optional("APEX_RUNNER_PUBLISHER") {
        runner = runner.with_publisher(publisher);
    }
    if let Some(raw) = std::env::var("APEX_INVOICE_VAT_BP")
        .ok()
        .filter(|v| !v.is_empty())
    {
        let bp: u16 = raw
            .parse()
            .map_err(|_| "APEX_INVOICE_VAT_BP must be a number")?;
        runner = runner.with_vat_rate(bp);
    }
    if let Some(raw) = std::env::var("APEX_MONITORING_MONTHLY_MICROS")
        .ok()
        .filter(|v| !v.is_empty())
    {
        let fee: i64 = raw
            .parse()
            .map_err(|_| "APEX_MONITORING_MONTHLY_MICROS must be a number")?;
        runner = runner.with_monitoring(apex_billing::MonitoringPlan {
            monthly_fee_micros: fee,
        });
    }
    if let (Ok(api_key), Ok(merchant)) = (
        std::env::var("APEX_ADYEN_API_KEY"),
        std::env::var("APEX_ADYEN_MERCHANT_ACCOUNT"),
    ) {
        let hmac = std::env::var("APEX_ADYEN_HMAC_KEY")
            .unwrap_or_default()
            .into_bytes();
        let base = std::env::var("APEX_ADYEN_CHECKOUT_BASE")
            .unwrap_or_else(|_| "https://checkout-test.adyen.com".to_string());
        let client = apex_billing::AdyenClient::new(&api_key, &merchant, hmac, &base)
            .map_err(|e| format!("adyen client: {e}"))?;
        runner = runner.with_adyen(Arc::new(client), transport.clone());
    }
    if let Some(priors_path) = optional("APEX_RUNNER_PRIORS") {
        let priors = match std::fs::read_to_string(&priors_path) {
            Ok(raw) if !raw.trim().is_empty() => {
                serde_json::from_str::<apex_lift::priors::LiftPriors>(&raw)
                    .map_err(|e| format!("priors: {e}"))?
            }
            _ => apex_lift::priors::LiftPriors::default(),
        };
        runner = runner.with_priors(priors, &priors_path);
    }
    if let Some(path) = optional("APEX_ALERT_FILE") {
        let sink: Arc<dyn AlertSink> = Arc::new(FileAlertSink::new(path));
        runner = runner.with_alerts(sink);
    }
    let now_unix_ms = apex_autopilot::cli::now_unix_ms();

    match command {
        "daily" => {
            let mut ledger = ledger;
            let report = runner
                .run_daily(&mut ledger, now_unix_ms)
                .await
                .map_err(|e| format!("daily: {e}"))?;
            println!(
                "daily: pulled {} settled {} failures {} {:?}",
                report.pulled,
                report.settled,
                report.failures.len(),
                report.outcomes
            );
            if !report.failures.is_empty() {
                std::process::exit(1);
            }
        }
        "cycle" => {
            let snapshot_path = apex_autopilot::cli::flag(&args, "--snapshot")
                .ok_or("cycle requires --snapshot")?;
            let mut ledger = ledger;
            let report = apex_autopilot::cycle::run(
                &runner,
                &mut ledger,
                std::path::Path::new(&snapshot_path),
                now_unix_ms,
            )
            .await
            .map_err(|e| format!("cycle: {e}"))?;
            println!(
                "cycle: proposed {} applied {} awaiting approval {}",
                report.proposed, report.applied, report.awaiting_approval
            );
        }
        "monthly" => {
            let (month, start, days) = match apex_autopilot::cli::flag(&args, "--month") {
                Some(month) => {
                    let start = apex_autopilot::cli::flag(&args, "--start")
                        .ok_or("monthly with --month requires --start")?;
                    let days: u32 = apex_autopilot::cli::flag(&args, "--days")
                        .unwrap_or_else(|| "30".to_string())
                        .parse()
                        .map_err(|_| "--days must be a number")?;
                    (month, start, days)
                }
                None => apex_autopilot::cli::previous_month(now_unix_ms)?,
            };
            let publisher = cli_required("APEX_RUNNER_PUBLISHER")?;
            let mut ledger = ledger;
            let report = runner
                .run_monthly(&mut ledger, &publisher, &month, &start, days, now_unix_ms)
                .await
                .map_err(|e| format!("monthly: {e}"))?;
            println!(
                "monthly {}: billed {} scope(s), {} fee(s), gross {} micros, {} link(s), collected={}",
                report.month,
                report.billed_scopes,
                report.fees.len(),
                report.invoice_total_micros,
                report.payment_links.len(),
                report.collected
            );
        }
        "recheck" => {
            let due = runner.due_rechecks(&ledger, now_unix_ms);
            println!("recheck: {} due", due.len());
            for item in due {
                println!(
                    "  {} (scope {}, {} months, {} days at {:.0}%)",
                    item.change_id,
                    item.scope,
                    item.months_since_keep,
                    item.plan.analysis_days,
                    item.plan.old_setting_share * 100.0
                );
            }
        }
        "aa" => {
            let unit = apex_autopilot::cli::flag(&args, "--unit").ok_or("aa requires --unit")?;
            let name = apex_autopilot::cli::flag(&args, "--name")
                .unwrap_or_else(|| format!("apex-aa-{unit}"));
            let spec = apex_core::ExperimentSpec {
                name,
                ad_unit_id: unit,
                metric: apex_core::ExperimentMetric::Revenue,
                variants: 2,
                change_description: "A/A calibration".into(),
                test_group_allocation: 50,
            };
            let mut ledger = ledger;
            let id = runner
                .start_aa(
                    &mut ledger,
                    apex_core::PlatformId::AppLovinMax,
                    &spec,
                    now_unix_ms,
                )
                .await
                .map_err(|e| format!("aa: {e}"))?;
            println!("aa: started {id}");
        }
        "aa-eval" => {
            let experiment = apex_autopilot::cli::flag(&args, "--experiment")
                .ok_or("aa-eval requires --experiment")?;
            let split: f64 = apex_autopilot::cli::flag(&args, "--split")
                .unwrap_or_else(|| "0.5".to_string())
                .parse()
                .map_err(|_| "--split must be a number between 0 and 1")?;
            let conclusion = runner
                .aa_conclusion(&experiment, split)
                .await
                .map_err(|e| format!("aa-eval: {e}"))?;
            println!("aa-eval {experiment}: {conclusion:?}");
        }
        other => return Err(format!("unknown command '{other}'").into()),
    }
    Ok(())
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
}
