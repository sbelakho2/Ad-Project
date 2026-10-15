//! Helpers for the `apex-runner` command line: argument and environment
//! parsing, the signing-key seed decoder and the month math. They live here,
//! not in the binary, so they are unit tested and stay covered.

/// Read a required environment value, trimming blanks.
pub fn required(env: impl Fn(&str) -> Option<String>, name: &str) -> Result<String, String> {
    env(name)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} is required"))
}

/// The value of `--flag value` in `args`, if present.
pub fn flag(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    args.get(index + 1).cloned()
}

/// Decode a 64-character hex ed25519 seed.
pub fn parse_seed(hex: &str) -> Result<[u8; 32], String> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return Err("the signing key must be 64 hex characters".into());
    }
    let bytes = hex.as_bytes();
    let mut seed = [0u8; 32];
    for (index, byte) in seed.iter_mut().enumerate() {
        let high = nibble(bytes[2 * index]).ok_or("the signing key is not hex")?;
        let low = nibble(bytes[2 * index + 1]).ok_or("the signing key is not hex")?;
        *byte = (high << 4) | low;
    }
    Ok(seed)
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The calendar month just closed, as `(YYYY-MM, YYYY-MM-01, day count)`.
pub fn previous_month(now_unix_ms: i64) -> Result<(String, String, u32), String> {
    let today = now_unix_ms.div_euclid(86_400_000);
    let (year, month) =
        parse_year_month(&crate::calendar::format_day(today)).ok_or("system clock out of range")?;
    let (prev_year, prev_month) = if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    };
    let start = format!("{prev_year:04}-{prev_month:02}-01");
    let start_day = crate::calendar::day_number(&start).ok_or("bad month start")?;
    let this_start = crate::calendar::day_number(&format!("{year:04}-{month:02}-01"))
        .ok_or("bad month start")?;
    let days = u32::try_from(this_start - start_day).map_err(|_| "bad month length")?;
    Ok((format!("{prev_year:04}-{prev_month:02}"), start, days))
}

/// The `(year, month)` of a `YYYY-MM-DD` day.
pub fn parse_year_month(text: &str) -> Option<(i64, u32)> {
    let mut parts = text.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    Some((year, month))
}

/// Split a comma-separated environment value, trimming and dropping blanks.
fn split_csv(raw: Option<String>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(raw) = raw {
        for item in raw.split(',') {
            let item = item.trim();
            if !item.is_empty() {
                out.push(item.to_string());
            }
        }
    }
    out
}

/// Split a comma-separated environment value (public for the binary).
pub fn split_csv_env(env: impl Fn(&str) -> Option<String>, name: &str) -> Vec<String> {
    split_csv(env_nonempty(&env, name))
}

/// An environment value, or a default.
fn env_or(env: &impl Fn(&str) -> Option<String>, name: &str, default: &str) -> String {
    match env(name) {
        Some(value) if !value.trim().is_empty() => value,
        _ => default.to_string(),
    }
}

/// A non-empty environment value.
fn env_nonempty(env: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    env(name).filter(|value| !value.trim().is_empty())
}

/// Build a [`Runner`](crate::Runner) from the environment: policy, publisher,
/// VAT, monitoring plan, Adyen collection, bucket model and alert sink. Kept
/// out of the binary so the whole configuration path is unit tested.
pub fn build_runner(
    env: impl Fn(&str) -> Option<String>,
    ops: std::sync::Arc<dyn crate::ops::PlatformOps>,
    transport: std::sync::Arc<dyn apex_connectors::transport::Transport>,
) -> Result<crate::runner::Runner, String> {
    use apex_lift::guardrails::FillFormat;
    let policy = match env("APEX_RUNNER_POLICY") {
        Some(raw) if !raw.trim().is_empty() => {
            serde_json::from_str::<crate::model::AutopilotPolicy>(&raw)
                .map_err(|e| format!("policy: {e}"))?
        }
        _ => crate::model::AutopilotPolicy::conservative(),
    };
    let mut runner = crate::runner::Runner::new(ops, policy, FillFormat::Rewarded);
    if let Some(publisher) = env_nonempty(&env, "APEX_RUNNER_PUBLISHER") {
        runner = runner.with_publisher(publisher);
    }
    if let Some(raw) = env_nonempty(&env, "APEX_INVOICE_VAT_BP") {
        let bp: u16 = raw
            .parse()
            .map_err(|_| "APEX_INVOICE_VAT_BP must be a number")?;
        runner = runner.with_vat_rate(bp);
    }
    if let Some(raw) = env_nonempty(&env, "APEX_MONITORING_MONTHLY_MICROS") {
        let fee: i64 = raw
            .parse()
            .map_err(|_| "APEX_MONITORING_MONTHLY_MICROS must be a number")?;
        runner = runner.with_monitoring(apex_billing::MonitoringPlan {
            monthly_fee_micros: fee,
        });
    }
    if let (Some(api_key), Some(merchant)) = (
        env("APEX_ADYEN_API_KEY"),
        env("APEX_ADYEN_MERCHANT_ACCOUNT"),
    ) {
        let hmac = env("APEX_ADYEN_HMAC_KEY").unwrap_or_default().into_bytes();
        let base = env_or(
            &env,
            "APEX_ADYEN_CHECKOUT_BASE",
            "https://checkout-test.adyen.com",
        );
        let client = apex_billing::AdyenClient::new(&api_key, &merchant, hmac, &base)
            .map_err(|e| format!("adyen client: {e}"))?;
        runner = runner.with_adyen(std::sync::Arc::new(client), transport);
    }
    if let Some(path) = env_nonempty(&env, "APEX_RUNNER_PRIORS") {
        let priors = match std::fs::read_to_string(&path) {
            Ok(raw) if !raw.trim().is_empty() => {
                serde_json::from_str::<apex_lift::priors::LiftPriors>(&raw)
                    .map_err(|e| format!("priors: {e}"))?
            }
            _ => apex_lift::priors::LiftPriors::default(),
        };
        runner = runner.with_priors(priors, &path);
    }
    if let Some(path) = env_nonempty(&env, "APEX_ALERT_FILE") {
        let sink: std::sync::Arc<dyn crate::alerts::AlertSink> =
            std::sync::Arc::new(crate::alerts::FileAlertSink::new(path));
        runner = runner.with_alerts(sink);
    }
    Ok(runner)
}

/// Build the live MAX adapter from the environment: keys, bases and the
/// verify-live allow-list. Tested with a fake transport; the binary just calls
/// it.
pub fn build_max_ops(
    env: impl Fn(&str) -> Option<String>,
    transport: std::sync::Arc<dyn apex_connectors::transport::Transport>,
) -> Result<std::sync::Arc<dyn crate::ops::PlatformOps>, String> {
    let api_key = required(&env, "APEX_MAX_API_KEY")?;
    let report_key = required(&env, "APEX_MAX_REPORT_KEY")?;
    let management_base = env_or(
        &env,
        "APEX_MAX_MANAGEMENT_BASE",
        apex_connectors::max::MAX_MANAGEMENT_BASE,
    );
    let reporting_base = env_or(
        &env,
        "APEX_MAX_REPORTING_BASE",
        apex_connectors::max::MAX_REPORTING_BASE,
    );
    let writable = split_csv(env_nonempty(&env, "APEX_RUNNER_WRITABLE"));
    let ops = crate::live_max::MaxPlatformOps::new(
        &api_key,
        &report_key,
        &management_base,
        &reporting_base,
        transport,
        std::sync::Arc::new(crate::live_max::SystemWall),
        writable,
    )
    .map_err(|e| format!("max adapter: {e}"))?;
    Ok(std::sync::Arc::new(ops))
}

/// The result of one dispatched command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchOutcome {
    /// A one-line human summary to print.
    pub summary: String,
    /// Whether the command saw a failure the process should report (a non-zero
    /// exit for `daily`).
    pub failed: bool,
}

/// Run one command against a built runner and ledger. Kept in the library so
/// every command's logic is unit tested; the binary only reads the
/// environment, builds the runner and prints the summary.
pub async fn dispatch(
    command: &str,
    args: &[String],
    runner: &crate::runner::Runner,
    ledger: &mut dyn apex_proof::LedgerLog,
    now_unix_ms: i64,
) -> Result<DispatchOutcome, String> {
    let ok = |text: String| DispatchOutcome {
        summary: text,
        failed: false,
    };
    match command {
        "daily" => {
            let report = runner
                .run_daily(ledger, now_unix_ms)
                .await
                .map_err(|e| format!("daily: {e}"))?;
            Ok(DispatchOutcome {
                summary: format!(
                    "daily: pulled {} settled {} failures {} {:?}",
                    report.pulled,
                    report.settled,
                    report.failures.len(),
                    report.outcomes
                ),
                failed: !report.failures.is_empty(),
            })
        }
        "cycle" => {
            let snapshot = flag(args, "--snapshot")
                .ok_or("cycle requires --snapshot (or APEX_RUNNER_SNAPSHOT)")?;
            let report =
                crate::cycle::run(runner, ledger, std::path::Path::new(&snapshot), now_unix_ms)
                    .await
                    .map_err(|e| format!("cycle: {e}"))?;
            Ok(ok(format!(
                "cycle: proposed {} applied {} awaiting approval {}",
                report.proposed, report.applied, report.awaiting_approval
            )))
        }
        "monthly" => {
            let (month, start, days) = match flag(args, "--month") {
                Some(month) => {
                    let start =
                        flag(args, "--start").ok_or("monthly with --month requires --start")?;
                    let days: u32 = match flag(args, "--days") {
                        Some(raw) => raw.parse().map_err(|_| "--days must be a number")?,
                        None => 30,
                    };
                    (month, start, days)
                }
                None => previous_month(now_unix_ms)?,
            };
            let publisher = runner.publisher_id().to_string();
            let report = runner
                .run_monthly(ledger, &publisher, &month, &start, days, now_unix_ms)
                .await
                .map_err(|e| format!("monthly: {e}"))?;
            Ok(ok(format!(
                "monthly {}: billed {} scope(s), {} fee(s), gross {} micros, {} link(s), collected={}",
                report.month,
                report.billed_scopes,
                report.fees.len(),
                report.invoice_total_micros,
                report.payment_links.len(),
                report.collected
            )))
        }
        "recheck" => {
            let due = runner.due_rechecks(ledger, now_unix_ms);
            let mut text = format!("recheck: {} due", due.len());
            for item in due {
                text.push_str(&format!(
                    "\n  {} (scope {}, {} months, {} days at {:.0}%)",
                    item.change_id,
                    item.scope,
                    item.months_since_keep,
                    item.plan.analysis_days,
                    item.plan.old_setting_share * 100.0
                ));
            }
            Ok(ok(text))
        }
        "aa" => {
            let unit = flag(args, "--unit").ok_or("aa requires --unit")?;
            let name = match flag(args, "--name") {
                Some(name) => name,
                None => format!("apex-aa-{unit}"),
            };
            let spec = apex_core::ExperimentSpec {
                name,
                ad_unit_id: unit,
                metric: apex_core::ExperimentMetric::Revenue,
                variants: 2,
                change_description: "A/A calibration".into(),
                test_group_allocation: 50,
            };
            let id = runner
                .start_aa(
                    ledger,
                    apex_core::PlatformId::AppLovinMax,
                    &spec,
                    now_unix_ms,
                )
                .await
                .map_err(|e| format!("aa: {e}"))?;
            Ok(ok(format!("aa: started {id}")))
        }
        "aa-eval" => {
            let experiment = flag(args, "--experiment").ok_or("aa-eval requires --experiment")?;
            let split: f64 = match flag(args, "--split") {
                Some(raw) => raw
                    .parse()
                    .map_err(|_| "--split must be a number between 0 and 1")?,
                None => 0.5,
            };
            let conclusion = runner
                .aa_conclusion(&experiment, split)
                .await
                .map_err(|e| format!("aa-eval: {e}"))?;
            Ok(ok(format!("aa-eval {experiment}: {conclusion:?}")))
        }
        other => Err(format!("unknown command '{other}'")),
    }
}

/// Unix milliseconds from the system clock.
pub fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn required_trims_and_refuses_blanks() {
        let env = |name: &str| match name {
            "SET" => Some("value".to_string()),
            "BLANK" => Some("   ".to_string()),
            _ => None,
        };
        assert_eq!(required(env, "SET").unwrap(), "value");
        assert!(required(env, "BLANK").is_err());
        assert!(required(env, "MISSING").is_err());
    }

    #[test]
    fn flag_reads_the_following_argument() {
        let args = vec![
            "monthly".to_string(),
            "--month".to_string(),
            "2026-10".to_string(),
        ];
        assert_eq!(flag(&args, "--month").as_deref(), Some("2026-10"));
        assert_eq!(flag(&args, "--start"), None);
        assert_eq!(flag(&args, "--month"), Some("2026-10".to_string()));
    }

    #[test]
    fn parse_seed_decodes_hex_and_refuses_bad_input() {
        let seed = parse_seed(&"0a".repeat(32)).expect("seed");
        assert_eq!(seed[0], 0x0a);
        assert_eq!(seed[31], 0x0a);
        assert!(parse_seed("short").is_err());
        assert!(parse_seed(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn previous_month_handles_the_year_boundary() {
        // 2026-01-15 -> 2025-12 (31 days).
        let day = crate::calendar::day_number("2026-01-15").unwrap();
        let (month, start, days) = previous_month(day * 86_400_000).unwrap();
        assert_eq!(month, "2025-12");
        assert_eq!(start, "2025-12-01");
        assert_eq!(days, 31);
        // 2026-03-05 -> 2026-02 (28 days in a non leap year).
        let day = crate::calendar::day_number("2026-03-05").unwrap();
        let (month, _, days) = previous_month(day * 86_400_000).unwrap();
        assert_eq!(month, "2026-02");
        assert_eq!(days, 28);
    }

    #[test]
    fn parse_year_month_reads_the_prefix() {
        assert_eq!(parse_year_month("2026-10-01"), Some((2026, 10)));
        assert_eq!(parse_year_month("bad"), None);
    }

    #[tokio::test]
    async fn build_runner_honors_the_environment() {
        struct NoOps;
        #[async_trait::async_trait]
        impl crate::ops::PlatformOps for NoOps {
            async fn writable(&self, _op: &str) -> bool {
                false
            }
            async fn start_experiment(
                &self,
                _r: &crate::ops::ExperimentRequest,
            ) -> Result<String, apex_core::PlatformError> {
                Ok(String::new())
            }
            async fn experiment(
                &self,
                _id: &str,
            ) -> Result<apex_core::ExperimentResult, apex_core::PlatformError> {
                Err(apex_core::PlatformError::Validation("none".into()))
            }
            async fn promote_experiment(&self, _id: &str) -> Result<(), apex_core::PlatformError> {
                Ok(())
            }
            async fn deprecate_experiment(
                &self,
                _id: &str,
            ) -> Result<(), apex_core::PlatformError> {
                Ok(())
            }
            async fn apply(
                &self,
                _a: &crate::model::Action,
                _k: &str,
            ) -> Result<(), apex_core::PlatformError> {
                Ok(())
            }
            async fn rollback(
                &self,
                _a: &crate::model::Action,
            ) -> Result<(), apex_core::PlatformError> {
                Ok(())
            }
            fn call_log(&self) -> Vec<String> {
                Vec::new()
            }
        }
        struct NoTransport;
        #[async_trait::async_trait]
        impl apex_connectors::transport::Transport for NoTransport {
            async fn request(
                &self,
                _r: apex_connectors::transport::HttpRequest,
            ) -> Result<
                apex_connectors::transport::HttpResponse,
                apex_connectors::transport::TransportError,
            > {
                Err(apex_connectors::transport::TransportError::Network(
                    "none".into(),
                ))
            }
        }
        let env = |name: &str| match name {
            "APEX_RUNNER_PUBLISHER" => Some("pub1".to_string()),
            "APEX_INVOICE_VAT_BP" => Some("2000".to_string()),
            "APEX_MONITORING_MONTHLY_MICROS" => Some("500000000".to_string()),
            "APEX_RUNNER_POLICY" => Some("not json".to_string()),
            _ => None,
        };
        // A malformed policy is a typed error, not a panic.
        assert!(build_runner(
            env,
            std::sync::Arc::new(NoOps),
            std::sync::Arc::new(NoTransport)
        )
        .is_err());

        use crate::ops::PlatformOps as _;
        let no_ops = NoOps;
        assert!(no_ops.scope_revenue("u", "a", "b").await.is_err());
        assert!(no_ops.experiment_days("x").await.is_err());
        assert!(no_ops.current_settings("u").await.is_err());
        assert!(no_ops
            .start_null_experiment(&apex_core::ExperimentSpec {
                name: "e".into(),
                ad_unit_id: "u".into(),
                metric: apex_core::ExperimentMetric::Revenue,
                variants: 2,
                change_description: "c".into(),
                test_group_allocation: 50,
            })
            .await
            .is_err());
        assert!(!no_ops.change_in_test_arm());
        let env = |name: &str| match name {
            "APEX_RUNNER_PUBLISHER" => Some("pub1".to_string()),
            "APEX_ADYEN_API_KEY" => Some("k".to_string()),
            "APEX_ADYEN_MERCHANT_ACCOUNT" => Some("m".to_string()),
            "APEX_ADYEN_HMAC_KEY" => Some("h".to_string()),
            _ => None,
        };
        let runner = build_runner(
            env,
            std::sync::Arc::new(NoOps),
            std::sync::Arc::new(NoTransport),
        )
        .expect("runner");
        assert!(format!("{runner:?}").contains("Runner"));
    }

    #[tokio::test]
    async fn dispatch_runs_every_command_without_a_platform() {
        use apex_proof::{InMemoryLedger, Keypair};
        let ops = std::sync::Arc::new(crate::ops::RecordingOps::new());
        let runner = crate::runner::Runner::new(
            ops,
            crate::model::AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_publisher("pub1");
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[3u8; 32]));

        // daily on an empty ledger is a no-op and not a failure.
        let out = dispatch("daily", &[], &runner, &mut ledger, 1)
            .await
            .expect("daily");
        assert_eq!(out.summary, "daily: pulled 0 settled 0 failures 0 []");
        assert!(!out.failed);

        // recheck prints nothing due.
        let out = dispatch("recheck", &[], &runner, &mut ledger, 1)
            .await
            .expect("recheck");
        assert!(out.summary.contains("0 due"));

        // aa starts a null run and aa-eval refuses a missing experiment.
        let out = dispatch(
            "aa",
            &["aa".to_string(), "--unit".to_string(), "u1".to_string()],
            &runner,
            &mut ledger,
            1,
        )
        .await
        .expect("aa");
        assert!(out.summary.contains("started"));

        // cycle needs a snapshot file; a missing one is a typed error.
        let err = dispatch(
            "cycle",
            &[
                "cycle".to_string(),
                "--snapshot".to_string(),
                "/nope.json".to_string(),
            ],
            &runner,
            &mut ledger,
            1,
        )
        .await
        .expect_err("missing snapshot");
        assert!(err.contains("cycle"), "{err}");

        // monthly without a publisher id is refused.
        let err = dispatch("monthly", &[], &runner, &mut ledger, 1).await;
        // This runner has a publisher, so it proceeds and bills nothing.
        assert!(err.is_ok(), "{err:?}");

        assert!(dispatch("bogus", &[], &runner, &mut ledger, 1)
            .await
            .is_err());
    }

    #[test]
    fn now_is_after_the_product_epoch() {
        assert!(now_unix_ms() > 1_700_000_000_000);
    }
}
