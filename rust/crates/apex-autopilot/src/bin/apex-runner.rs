//! `apex-runner` — the scheduled job entrypoint for one publisher.
//!
//! One process serves one publisher's ledger. systemd timers invoke it on a
//! schedule (`daily`, `monthly`, `recheck`); each command is one idempotent
//! job over the same durable ledger.
//!
//! Configuration (environment):
//! - `APEX_RUNNER_LEDGER` — path to the publisher's ledger file (required);
//! - `APEX_RUNNER_SIGNING_KEY_HEX` — 64-hex ed25519 seed (required);
//! - `APEX_MAX_API_KEY`, `APEX_MAX_REPORT_KEY` — the MAX management and
//!   reporting keys (required);
//! - `APEX_MAX_MANAGEMENT_BASE`, `APEX_MAX_REPORTING_BASE` — optional bases;
//! - `APEX_RUNNER_WRITABLE` — comma-separated operations the operator has
//!   verified (e.g. `setCountryFloor`); empty refuses every write;
//! - `APEX_RUNNER_POLICY` — optional JSON policy, defaults to conservative.

#![forbid(unsafe_code)]

use std::sync::Arc;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).ok_or(
        "usage: apex-runner <daily|monthly|recheck|aa> [--month YYYY-MM] [--start YYYY-MM-DD] [--days N] [--unit U] [--name N]",
    )?;

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
    let ops = MaxPlatformOps::new(
        &cli_required("APEX_MAX_API_KEY")?,
        &cli_required("APEX_MAX_REPORT_KEY")?,
        &std::env::var("APEX_MAX_MANAGEMENT_BASE")
            .unwrap_or_else(|_| MAX_MANAGEMENT_BASE.to_string()),
        &std::env::var("APEX_MAX_REPORTING_BASE")
            .unwrap_or_else(|_| MAX_REPORTING_BASE.to_string()),
        transport,
        Arc::new(SystemWall),
        writable,
    )
    .map_err(|e| format!("max adapter: {e}"))?;

    let policy = match std::env::var("APEX_RUNNER_POLICY") {
        Ok(raw) if !raw.trim().is_empty() => {
            serde_json::from_str::<AutopilotPolicy>(&raw).map_err(|e| format!("policy: {e}"))?
        }
        _ => AutopilotPolicy::conservative(),
    };
    let runner = Runner::new(Arc::new(ops), policy, FillFormat::Rewarded);
    let now_unix_ms = apex_autopilot::cli::now_unix_ms();

    match command {
        "daily" => {
            let mut ledger = ledger;
            let report = runner
                .run_daily(&mut ledger, now_unix_ms)
                .await
                .map_err(|e| format!("daily: {e}"))?;
            println!(
                "daily: pulled {} settled {} {:?}",
                report.pulled, report.settled, report.outcomes
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
                // No month given: bill the calendar month just closed.
                None => apex_autopilot::cli::previous_month(now_unix_ms)?,
            };
            let mut ledger = ledger;
            let report = runner
                .run_monthly(&mut ledger, &month, &start, days, now_unix_ms)
                .await
                .map_err(|e| format!("monthly: {e}"))?;
            println!(
                "monthly {}: billed {} scope(s), {} fee(s): {:?}",
                report.month,
                report.billed_scopes,
                report.fees.len(),
                report.fees
            );
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
}
