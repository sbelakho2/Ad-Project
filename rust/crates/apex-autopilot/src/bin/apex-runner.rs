//! `apex-runner` — the scheduled job entrypoint for one publisher.
//!
//! One process serves one publisher's ledger. systemd timers invoke it on a
//! schedule; the logic of each job lives in the library (`apex_autopilot::cli`
//! and `apex_autopilot::cycle`) so it is unit tested; this binary only reads
//! the environment, builds the platform adapter and the runner, and prints the
//! summary.
//!
//! Commands: `audit`, `cycle`, `daily`, `monthly`, `recheck`, `aa`, `aa-eval`.
//! Configuration (environment): `APEX_RUNNER_LEDGER`, `APEX_RUNNER_PUBLISHER`,
//! `APEX_RUNNER_SIGNING_KEY_HEX`, `APEX_MAX_API_KEY`, `APEX_MAX_REPORT_KEY`,
//! `APEX_MAX_MANAGEMENT_BASE`, `APEX_MAX_REPORTING_BASE`,
//! `APEX_RUNNER_WRITABLE`, `APEX_RUNNER_POLICY`, `APEX_RUNNER_PRIORS`,
//! `APEX_RUNNER_SNAPSHOT`, `APEX_DEVELOPER_SITE`, `APEX_EXPECTED_NETWORKS`,
//! `APEX_ADYEN_API_KEY`, `APEX_ADYEN_MERCHANT_ACCOUNT`, `APEX_ADYEN_HMAC_KEY`,
//! `APEX_ADYEN_CHECKOUT_BASE`, `APEX_INVOICE_VAT_BP`,
//! `APEX_MONITORING_MONTHLY_MICROS`, `APEX_ALERT_FILE`.

#![forbid(unsafe_code)]

use std::sync::Arc;

use apex_connectors::max::{MAX_MANAGEMENT_BASE, MAX_REPORTING_BASE};
use apex_connectors::transport::{ReqwestTransport, RetryTransport, TokioSleeper};
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
    let now_unix_ms = apex_autopilot::cli::now_unix_ms();
    let transport: Arc<dyn apex_connectors::transport::Transport> = Arc::new(RetryTransport::new(
        Arc::new(ReqwestTransport::new().map_err(|e| format!("transport: {e}"))?),
        2,
        Arc::new(TokioSleeper),
    ));

    // `audit` builds the account snapshot from the MAX keys and needs no
    // ledger; every other command runs against the publisher's ledger.
    if command == "audit" {
        let snapshot = apex_autopilot::max_account_snapshot(
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
            now_unix_ms,
        )
        .await
        .map_err(|e| format!("snapshot: {e}"))?;
        let out = apex_autopilot::cli::flag(&args, "--out")
            .or_else(|| optional("APEX_RUNNER_SNAPSHOT"))
            .ok_or("audit requires --out (or APEX_RUNNER_SNAPSHOT)")?;
        apex_autopilot::write_snapshot(std::path::Path::new(&out), &snapshot)?;
        println!("audit: wrote {} ad units to {out}", snapshot.ad_units.len());
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
    let runner = apex_autopilot::cli::build_runner(|name| std::env::var(name).ok(), ops, transport)
        .map_err(|e| format!("configuration: {e}"))?;

    let outcome = apex_autopilot::cli::dispatch(command, &args, &runner, &mut ledger, now_unix_ms)
        .await
        .map_err(|e| e.to_string())?;
    println!("{}", outcome.summary);
    if outcome.failed {
        std::process::exit(1);
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
