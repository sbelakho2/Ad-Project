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

/// The environment variable a timer uses to pass the expected start time
/// (unix milliseconds), the fallback for `--scheduled-at`.
pub const SCHEDULED_AT_ENV: &str = "APEX_SCHEDULED_AT_MS";

/// The expected start time in unix milliseconds: `--scheduled-at <unix-ms>`
/// wins, the `APEX_SCHEDULED_AT_MS` fallback is second. A malformed value or
/// a non-positive timestamp is a typed error, never a silently skipped (or
/// ~57-year) lag sample. A timestamp in the future is accepted: the lag is
/// clamped to zero at the recording site.
pub fn scheduled_at(
    args: &[String],
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<i64>, String> {
    let raw = match flag(args, "--scheduled-at") {
        Some(raw) => Some(raw),
        None => env(SCHEDULED_AT_ENV),
    };
    match raw {
        Some(raw) if !raw.trim().is_empty() => {
            let value = raw
                .trim()
                .parse::<i64>()
                .map_err(|_| "--scheduled-at must be unix milliseconds".to_string())?;
            if value <= 0 {
                return Err(
                    "scheduled-at must be a positive unix-millisecond timestamp".to_string()
                );
            }
            Ok(Some(value))
        }
        _ => Ok(None),
    }
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

/// Re-deliver the analytics spool once at runner start, logging the
/// re-delivery. Shared by the dispatch path and the audit command so every
/// process drains a spool left by earlier failed runs before emitting.
pub async fn replay_spool(events: Option<&crate::events::RunnerEvents>) {
    let Some(events) = events else {
        return;
    };
    let replayed = events.replay().await;
    if replayed > 0 {
        eprintln!("apex-runner: analytics spool: {replayed} batches re-delivered");
    }
}

/// The workspace VAT rate in basis points, mirroring apex-api
/// (`APEX_BILLING_VAT_RATE_BP` is the primary name; the historical
/// `APEX_INVOICE_VAT_BP` is a deprecated fallback announced once). The rate
/// must be a number capped at 10,000 bp: anything else is a typed error,
/// never a silently unset tax rate.
pub fn vat_rate_bp(env: &impl Fn(&str) -> Option<String>) -> Result<Option<u16>, String> {
    const PRIMARY: &str = "APEX_BILLING_VAT_RATE_BP";
    const DEPRECATED: &str = "APEX_INVOICE_VAT_BP";
    match env_nonempty(env, PRIMARY) {
        Some(raw) => parse_vat_rate(PRIMARY, &raw).map(Some),
        None => match env_nonempty(env, DEPRECATED) {
            Some(raw) => {
                eprintln!(
                    "apex-runner: {DEPRECATED} is deprecated; use {PRIMARY} (the apex-api name)"
                );
                parse_vat_rate(DEPRECATED, &raw).map(Some)
            }
            None => Ok(None),
        },
    }
}

fn parse_vat_rate(name: &str, raw: &str) -> Result<u16, String> {
    let bp: i64 = raw
        .trim()
        .parse()
        .map_err(|_| format!("{name} must be a number"))?;
    if !(0..=10_000).contains(&bp) {
        return Err(format!("{name} must be between 0 and 10000 basis points"));
    }
    Ok(bp as u16)
}

/// The monitoring plan fee in micros, mirroring apex-api
/// (`APEX_MONITORING_FEE_MICROS` is the primary name; the historical
/// `APEX_MONITORING_MONTHLY_MICROS` is a deprecated fallback announced once).
/// apex-api treats a non-positive fee as "not selected", so the runner
/// mirrors that exactly; a malformed value is a typed error.
pub fn monitoring_fee_micros(env: &impl Fn(&str) -> Option<String>) -> Result<Option<i64>, String> {
    const PRIMARY: &str = "APEX_MONITORING_FEE_MICROS";
    const DEPRECATED: &str = "APEX_MONITORING_MONTHLY_MICROS";
    match env_nonempty(env, PRIMARY) {
        Some(raw) => parse_monitoring_fee(PRIMARY, &raw),
        None => match env_nonempty(env, DEPRECATED) {
            Some(raw) => {
                eprintln!(
                    "apex-runner: {DEPRECATED} is deprecated; use {PRIMARY} (the apex-api name)"
                );
                parse_monitoring_fee(DEPRECATED, &raw)
            }
            None => Ok(None),
        },
    }
}

fn parse_monitoring_fee(name: &str, raw: &str) -> Result<Option<i64>, String> {
    let fee: i64 = raw
        .trim()
        .parse()
        .map_err(|_| format!("{name} must be a number"))?;
    Ok((fee > 0).then_some(fee))
}

/// Build a [`Runner`](crate::Runner) from the environment: policy, publisher,
/// VAT, monitoring plan, Adyen collection, bucket model and alert sink. Kept
/// out of the binary so the whole configuration path is unit tested.
pub fn build_runner(
    env: impl Fn(&str) -> Option<String>,
    ops: std::sync::Arc<dyn crate::ops::PlatformOps>,
    transport: std::sync::Arc<dyn apex_connectors::transport::Transport>,
    instrumentation: &Instrumentation,
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
    let publisher = env_nonempty(&env, "APEX_RUNNER_PUBLISHER");
    if let Some(publisher) = &publisher {
        runner = runner.with_publisher(publisher.clone());
    }
    // The billing configuration reads the same names apex-api reads. The
    // historical runner-only names stay as deprecated fallbacks and are
    // announced on stderr when used, so a deployment never silently runs on
    // two divergent spellings.
    if let Some(bp) = vat_rate_bp(&env)? {
        runner = runner.with_vat_rate(bp);
    }
    if let Some(fee) = monitoring_fee_micros(&env)? {
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
        // A missing priors file is "no priors yet" (the defaults); a file
        // that exists but cannot be read is a typed error, never silently
        // replaced by defaults.
        let priors = match std::fs::read_to_string(&path) {
            Ok(raw) if !raw.trim().is_empty() => {
                serde_json::from_str::<apex_lift::priors::LiftPriors>(&raw)
                    .map_err(|e| format!("priors: {e}"))?
            }
            Ok(_) => apex_lift::priors::LiftPriors::default(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                apex_lift::priors::LiftPriors::default()
            }
            Err(error) => return Err(format!("priors at {path}: {error}")),
        };
        runner = runner.with_priors(priors, &path);
    }
    if let Some(path) = env_nonempty(&env, "APEX_ALERT_FILE") {
        let sink: std::sync::Arc<dyn crate::alerts::AlertSink> =
            std::sync::Arc::new(crate::alerts::FileAlertSink::new(path));
        runner = runner.with_alerts(sink);
    }
    runner = runner.with_telemetry(instrumentation.telemetry.clone());
    if let Some(events) = &instrumentation.events {
        // The instrumentation handle is built before the publisher is known;
        // stamp it here so every emitted event carries the tenant. Without
        // this the store refuses the whole batch (required field).
        let events = match &publisher {
            Some(publisher) => {
                std::sync::Arc::new((**events).clone().with_publisher(publisher.clone()))
            }
            None => events.clone(),
        };
        runner = runner.with_events(events);
    }
    if let Some(projections) = &instrumentation.projections {
        runner = runner.with_projections(projections.clone());
    }
    if let Some(notify) = &instrumentation.notify {
        runner = runner.with_notifications(notify.clone());
    }
    runner = runner.with_operator_jobs(env_or(
        &env,
        crate::operator_jobs::JOBS_DIR_ENV,
        crate::operator_jobs::DEFAULT_JOBS_DIR,
    ));
    if let Some(events) = env_nonempty(&env, "APEX_PAYMENT_EVENTS") {
        runner = runner.with_payment_events(
            events,
            env_nonempty(&env, "APEX_PAYMENT_LEDGER").map(std::path::PathBuf::from),
            env_nonempty(&env, "APEX_PAYMENT_RETRIES").map(std::path::PathBuf::from),
        );
    }
    if let Some(cursor) = env_nonempty(&env, crate::payment_events::PAYMENT_CURSOR_ENV) {
        runner = runner.with_payment_cursor(cursor);
    }
    // The verifier re-reads the ledger file the runner writes; without the
    // path and key it is absent, and the report says "appended" without
    // claiming a verification.
    if let (Some(path), Some(seed)) = (
        env_nonempty(&env, "APEX_RUNNER_LEDGER"),
        env_nonempty(&env, "APEX_RUNNER_SIGNING_KEY_HEX"),
    ) {
        if let Ok(seed) = parse_seed(&seed) {
            let key = apex_proof::Keypair::from_bytes(&seed);
            runner = runner.with_verifier(std::sync::Arc::new(
                crate::verify::FileLedgerVerifier::new(path, key.verifying_key()),
            ));
        }
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

/// The audit rule-group coverage of a snapshot: how many of the engine's
/// five rule groups had their declared inputs, the expected count, and which
/// inputs were missing. The mapping is the engine's own rule set
/// (`apex-audit`'s five rules); a rule without its input cannot run, and the
/// report says so instead of claiming full coverage.
pub fn audit_coverage(snapshot: &apex_audit::Snapshot) -> (u32, u32, Vec<String>) {
    let groups: [(&str, bool); 5] = [
        (
            "stale_floor",
            !snapshot.ad_units.is_empty() && !snapshot.country_revenue.is_empty(),
        ),
        ("missing_country_floor", !snapshot.ad_units.is_empty()),
        (
            "disabled_bidder_with_account",
            !snapshot.mediation_groups.is_empty() && !snapshot.bidder_accounts.is_empty(),
        ),
        ("reporting_gap", !snapshot.country_revenue.is_empty()),
        ("app_ads_txt", snapshot.app_ads_txt.present),
    ];
    let expected = groups.len() as u32;
    let eligible = groups.iter().filter(|(_, ok)| *ok).count() as u32;
    let missing = groups
        .iter()
        .filter(|(_, ok)| !*ok)
        .map(|(name, _)| (*name).to_string())
        .collect();
    (eligible, expected, missing)
}

/// The rule release string of the audit engine (`apex-audit v0.1.0` becomes
/// `v0.1.0`).
pub fn audit_rule_version() -> String {
    apex_audit::GENERATED_BY
        .rsplit(' ')
        .next()
        .map(|version| format!("v{version}"))
        .unwrap_or_else(|| "v0.1.0".to_string())
}

/// The result of one dispatched command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchOutcome {
    /// The human-readable report (page 98 shape).
    pub human: String,
    /// The versioned JSON document (`{"schema_version": 1, ...}`).
    pub json: String,
    /// Whether the command saw a failure the process should report (a non-zero
    /// exit for `daily`).
    pub failed: bool,
}

/// The JSON schema version of every dispatched report.
pub const REPORT_SCHEMA_VERSION: u64 = 1;

/// The durable run reference, `job_YYYYMMDD_HHMM` in UTC.
pub fn run_reference(now_unix_ms: i64) -> String {
    let day = crate::calendar::format_day(now_unix_ms.div_euclid(86_400_000));
    let minutes = now_unix_ms.div_euclid(60_000).rem_euclid(24 * 60);
    format!(
        "job_{}_{:02}{:02}",
        day.replace('-', ""),
        minutes / 60,
        minutes % 60
    )
}

/// The runners' collaborators built from the environment: the bounded metric
/// registry, the analytics emitter and the notification plan. The clear
/// disabled-state log lines ride in `logs` so the binary prints them.
#[derive(Debug)]
pub struct Instrumentation {
    /// The process registry every metric series lands in.
    pub registry: std::sync::Arc<apex_metrics::Registry>,
    /// The runner telemetry handle.
    pub telemetry: crate::telemetry::RunnerTelemetry,
    /// The analytics emitter; `None` when `APEX_CLICKHOUSE_HTTP` is unset.
    pub events: Option<std::sync::Arc<crate::events::RunnerEvents>>,
    /// The daily projection refresh; `None` when `APEX_CLICKHOUSE_HTTP` is
    /// unset (the same environment contract as the emitter).
    pub projections: Option<crate::projections_job::RunnerProjections>,
    /// The notification plan; `None` when `APEX_NOTIFY_CONTACTS` is unset.
    pub notify: Option<std::sync::Arc<crate::notify::NotifyPlan>>,
    /// Operator-facing log lines (disabled states, delivery caveats).
    pub logs: Vec<String>,
}

/// Build the instrumentation from the environment.
pub fn build_instrumentation(
    env: impl Fn(&str) -> Option<String>,
) -> Result<Instrumentation, String> {
    let registry = std::sync::Arc::new(apex_metrics::Registry::new());
    let telemetry = crate::telemetry::RunnerTelemetry::new(registry.clone());
    let mut logs = Vec::new();
    let events = match crate::events::RunnerEvents::from_lookup(&env) {
        Ok(Some(events)) => Some(std::sync::Arc::new(events)),
        Ok(None) => {
            logs.push(
                "apex-analytics: APEX_CLICKHOUSE_HTTP is not set; event emission is disabled."
                    .to_string(),
            );
            None
        }
        Err(error) => return Err(error),
    };
    let projections = crate::projections_job::RunnerProjections::from_lookup(&env)?;
    let notify = match crate::notify::NotifyPlan::from_lookup(&env, registry.clone()) {
        Ok((plan, notify_logs)) => {
            logs.extend(notify_logs);
            plan
        }
        Err(error) => return Err(error.to_string()),
    };
    Ok(Instrumentation {
        registry,
        telemetry,
        events,
        projections,
        notify,
        logs,
    })
}

/// What one command produced, before the shared ledger and job framing.
struct CommandReport {
    command: &'static str,
    job_type: apex_metrics::vocab::JobType,
    lines: Vec<String>,
    json: serde_json::Map<String, serde_json::Value>,
    failed: bool,
}

fn scope_line(publisher: &str) -> String {
    if publisher.trim().is_empty() {
        "Mode: MAX / Publisher: not configured".to_string()
    } else {
        format!("Mode: MAX / Publisher: {publisher}")
    }
}

fn ledger_state(appended: u64, check: Option<&crate::verify::LedgerCheck>) -> (&'static str, bool) {
    match (appended > 0, check) {
        (true, Some(check)) if check.is_verified() => ("appended_and_verified", true),
        (true, Some(_)) => ("appended_verification_failed", false),
        (true, None) => ("appended", false),
        (false, Some(check)) if check.is_verified() => ("no_new_records_verified", true),
        (false, Some(_)) => ("no_new_records_verification_failed", false),
        (false, None) => ("no_new_records", false),
    }
}

fn ledger_line(appended: u64, check: Option<&crate::verify::LedgerCheck>) -> String {
    match (appended > 0, check) {
        (true, Some(check)) if check.is_verified() => "Ledger: appended and verified".to_string(),
        (true, Some(_)) => "Ledger: appended; verification failed".to_string(),
        (true, None) => "Ledger: appended".to_string(),
        (false, Some(check)) if check.is_verified() => {
            "Ledger: no new records; verified".to_string()
        }
        (false, Some(_)) => "Ledger: no new records; verification failed".to_string(),
        (false, None) => "Ledger: no new records".to_string(),
    }
}

fn ledger_json(appended: u64, check: Option<&crate::verify::LedgerCheck>) -> serde_json::Value {
    let (state, verified) = ledger_state(appended, check);
    let mut ledger = serde_json::json!({
        "appended": appended > 0,
        "records_appended": appended,
        "state": state,
    });
    if let Some(check) = check {
        ledger["verified"] = serde_json::Value::Bool(check.is_verified());
        match check {
            crate::verify::LedgerCheck::Verified {
                highest_sequence,
                duration_ms,
            } => {
                ledger["highest_sequence"] = serde_json::Value::from(*highest_sequence);
                ledger["duration_ms"] = serde_json::Value::from(*duration_ms);
            }
            crate::verify::LedgerCheck::Failed {
                result,
                reason,
                failure_line,
            } => {
                ledger["failure"] = serde_json::json!({
                    "result": result.as_str(),
                    "reason": reason,
                    "line": failure_line,
                });
            }
        }
    } else {
        ledger["verified"] = serde_json::Value::Null;
    }
    let _ = verified;
    ledger
}

/// Run one command against a built runner and ledger. Kept in the library so
/// every command's logic is unit tested; the binary only reads the
/// environment, builds the runner and prints the report.
///
/// The report shape follows section 06 page 98: a scope line, only the
/// durable outcomes the run supports (honest about unresolved states), the
/// ledger line and the run reference. Every command also records its bounded
/// job metrics/events and the ledger verification result.
pub async fn dispatch(
    command: &str,
    args: &[String],
    runner: &crate::runner::Runner,
    ledger: &mut dyn apex_proof::LedgerLog,
    now_unix_ms: i64,
) -> Result<DispatchOutcome, String> {
    use apex_metrics::vocab::Outcome;

    // The spool from earlier failed runs is re-delivered before the command
    // starts; a re-delivery is logged, never silent.
    replay_spool(runner.events().map(AsRef::as_ref)).await;
    // A timer that knows its expected start passes it (`--scheduled-at`, or
    // the `APEX_SCHEDULED_AT_MS` fallback); the lag sample lands in the
    // registry for the job that is about to run.
    if let Some(scheduled_at) = scheduled_at(args, |name| std::env::var(name).ok())? {
        if let Some(telemetry) = runner.telemetry() {
            let lag_seconds = (now_unix_ms.saturating_sub(scheduled_at) as f64 / 1_000.0).max(0.0);
            telemetry.schedule_lag(job_type_of(command), lag_seconds);
        }
    }
    let started = std::time::Instant::now();
    let records_before = ledger.len() as u64;
    let report = match dispatch_inner(command, args, runner, ledger, now_unix_ms).await {
        Ok(report) => report,
        Err(error) => {
            if let Some(telemetry) = runner.telemetry() {
                telemetry.job_finished(job_type_of(command), Outcome::Failed, started);
            }
            if let Some(events) = runner.events() {
                events
                    .job_run(
                        job_type_of(command).as_str(),
                        &run_reference(now_unix_ms),
                        now_unix_ms,
                        "failed",
                        started.elapsed().as_millis() as u64,
                    )
                    .await;
            }
            return Err(error);
        }
    };
    let records_after = ledger.len() as u64;
    let appended = records_after.saturating_sub(records_before);

    // The verification line is honest: it reflects an actual verification of
    // the ledger this run wrote, never an assumption.
    let check = runner.verifier().map(|verifier| verifier.verify(ledger));
    if let Some(check) = &check {
        if let Some(telemetry) = runner.telemetry() {
            match check {
                crate::verify::LedgerCheck::Verified { .. } => telemetry.ledger(
                    apex_metrics::vocab::LedgerResult::Verified,
                    crate::verify::VERIFIER_VERSION,
                ),
                crate::verify::LedgerCheck::Failed { result, .. } => {
                    telemetry.ledger(*result, crate::verify::VERIFIER_VERSION);
                }
            }
        }
        if let Some(events) = runner.events() {
            match check {
                crate::verify::LedgerCheck::Verified {
                    highest_sequence,
                    duration_ms,
                } => {
                    events
                        .ledger_verified(
                            *highest_sequence,
                            crate::verify::VERIFIER_VERSION,
                            *duration_ms,
                            now_unix_ms,
                        )
                        .await;
                }
                crate::verify::LedgerCheck::Failed {
                    failure_line,
                    reason,
                    ..
                } => {
                    events
                        .ledger_verification_failed(
                            ledger.len() as u64,
                            crate::verify::VERIFIER_VERSION,
                            *failure_line,
                            0,
                            now_unix_ms,
                        )
                        .await;
                    let bounded = crate::text::bounded_chars(reason, 200);
                    eprintln!("apex-runner: ledger verification failed: {bounded}");
                }
            }
        }
    }

    let job_outcome = if report.failed {
        Outcome::Failed
    } else {
        Outcome::Completed
    };
    if let Some(telemetry) = runner.telemetry() {
        telemetry.job_finished(report.job_type, job_outcome, started);
    }
    let job_label = if report.failed { "failed" } else { "completed" };
    if let Some(events) = runner.events() {
        events
            .job_run(
                report.job_type.as_str(),
                &run_reference(now_unix_ms),
                now_unix_ms,
                job_label,
                started.elapsed().as_millis() as u64,
            )
            .await;
    }

    let publisher = runner.publisher_id().to_string();
    let mut lines = vec![scope_line(&publisher)];
    lines.extend(report.lines.clone());
    lines.push(ledger_line(appended, check.as_ref()));
    lines.push(format!("Run reference: {}", run_reference(now_unix_ms)));

    let mut json = serde_json::Map::new();
    json.insert(
        "schema_version".into(),
        serde_json::Value::from(REPORT_SCHEMA_VERSION),
    );
    json.insert("command".into(), serde_json::Value::from(report.command));
    json.insert("mode".into(), serde_json::Value::from("max"));
    json.insert(
        "publisher".into(),
        if publisher.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::from(publisher.clone())
        },
    );
    json.insert(
        "run_reference".into(),
        serde_json::Value::from(run_reference(now_unix_ms)),
    );
    for (key, value) in report.json {
        json.insert(key, value);
    }
    json.insert("ledger".into(), ledger_json(appended, check.as_ref()));
    Ok(DispatchOutcome {
        human: lines.join("\n"),
        json: serde_json::Value::Object(json).to_string(),
        failed: report.failed,
    })
}

fn job_type_of(command: &str) -> apex_metrics::vocab::JobType {
    use apex_metrics::vocab::JobType;
    match command {
        "daily" => JobType::Daily,
        "monthly" => JobType::Monthly,
        "recheck" => JobType::Recheck,
        "aa" => JobType::Aa,
        "aa-eval" => JobType::AaEval,
        _ => JobType::Cycle,
    }
}

fn usd(micros: i64) -> String {
    apex_notify::format_usd(micros).unwrap_or_else(|_| format!("{micros} micros"))
}

async fn dispatch_inner(
    command: &str,
    args: &[String],
    runner: &crate::runner::Runner,
    ledger: &mut dyn apex_proof::LedgerLog,
    now_unix_ms: i64,
) -> Result<CommandReport, String> {
    match command {
        "daily" => {
            let report = runner
                .run_daily(ledger, now_unix_ms)
                .await
                .map_err(|e| format!("daily: {e}"))?;
            let mut counts = std::collections::BTreeMap::new();
            for (_, status) in &report.outcomes {
                *counts.entry(status.clone()).or_insert(0u32) += 1;
            }
            let kept = counts.get("kept").copied().unwrap_or(0);
            let rolled_back = counts.get("rolled_back").copied().unwrap_or(0);
            let voided = counts.get("voided").copied().unwrap_or(0);
            let mut lines = Vec::new();
            lines.push(format!("Reporting pulls: {} completed", report.pulled));
            let mut decisions_line =
                format!("Decisions: {} pending / {} kept", report.pending, kept);
            if rolled_back > 0 {
                decisions_line.push_str(&format!(" / {rolled_back} rolled back"));
            }
            if voided > 0 {
                decisions_line.push_str(&format!(" / {voided} voided"));
            }
            lines.push(decisions_line);
            lines.push(if report.failures.is_empty() {
                "Platform operations: no unresolved result".to_string()
            } else {
                format!(
                    "Platform operations: {} unresolved result(s)",
                    report.failures.len()
                )
            });
            if let Some(jobs) = &report.operator_jobs {
                if jobs.any() {
                    let mut line = format!(
                        "Operator jobs: {} completed / {} failed",
                        jobs.completed, jobs.failed
                    );
                    if jobs.unknown > 0 {
                        line.push_str(&format!(" / {} unknown", jobs.unknown));
                    }
                    lines.push(line);
                }
            }
            if let Some(payments) = &report.payments {
                if payments.any() {
                    let mut line = format!(
                        "Payment events: {} received / {} failed",
                        payments.received, payments.failed
                    );
                    if payments.reconciled > 0 {
                        line.push_str(&format!(" / {} reconciled", payments.reconciled));
                    }
                    if payments.refunded > 0 {
                        line.push_str(&format!(" / {} refunded", payments.refunded));
                    }
                    if payments.disputed > 0 {
                        line.push_str(&format!(" / {} disputed", payments.disputed));
                    }
                    lines.push(line);
                }
            }
            if let Some(projections) = &report.projections {
                lines.push(projections.summary());
            } else if let Some(error) = &report.projection_failure {
                lines.push(format!("Projections: refresh failed ({error})"));
            }
            let mut json = serde_json::Map::new();
            json.insert(
                "reporting_pulls".into(),
                serde_json::json!({
                    "completed": report.pulled,
                    "failed": report.pull_failures,
                }),
            );
            json.insert(
                "decisions".into(),
                serde_json::json!({
                    "pending": report.pending,
                    "kept": kept,
                    "rolled_back": rolled_back,
                    "voided": voided,
                }),
            );
            json.insert(
                "platform_operations".into(),
                serde_json::json!({ "unresolved": report.failures.len() }),
            );
            if let Some(jobs) = &report.operator_jobs {
                json.insert(
                    "operator_jobs".into(),
                    serde_json::json!({
                        "completed": jobs.completed,
                        "failed": jobs.failed,
                        "unknown": jobs.unknown,
                        "skipped": jobs.skipped,
                    }),
                );
            }
            if let Some(projections) = &report.projections {
                let lag: serde_json::Map<String, serde_json::Value> = projections
                    .lag_seconds
                    .iter()
                    .map(|(name, seconds)| (name.clone(), serde_json::json!(seconds)))
                    .collect();
                json.insert(
                    "projections".into(),
                    serde_json::json!({
                        "batch_id": projections.batch_id,
                        "published_version": projections.published_version,
                        "projections": projections.projections,
                        "rows": projections.rows,
                        "switched": projections.switched,
                        "lag_seconds": lag,
                    }),
                );
            }
            if let Some(error) = &report.projection_failure {
                json.insert("projection_failure".into(), serde_json::json!(error));
            }
            if let Some(payments) = &report.payments {
                json.insert(
                    "payment_events".into(),
                    serde_json::json!({
                        "received": payments.received,
                        "failed": payments.failed,
                        "retries": payments.retries,
                        "reconciled": payments.reconciled,
                        "refunded": payments.refunded,
                        "disputed": payments.disputed,
                        "skipped": payments.skipped,
                    }),
                );
            }
            Ok(CommandReport {
                command: "daily",
                job_type: apex_metrics::vocab::JobType::Daily,
                lines,
                json,
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
            Ok(CommandReport {
                command: "cycle",
                job_type: apex_metrics::vocab::JobType::Cycle,
                lines: vec![format!(
                    "Cycle: proposed {} applied {} awaiting approval {}",
                    report.proposed, report.applied, report.awaiting_approval
                )],
                json: [(
                    "cycle".to_string(),
                    serde_json::json!({
                        "proposed": report.proposed,
                        "applied": report.applied,
                        "awaiting_approval": report.awaiting_approval,
                    }),
                )]
                .into_iter()
                .collect(),
                failed: false,
            })
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
            let mut lines = vec![
                format!("Billing: {} fee(s) for {}", report.fees.len(), report.month),
                format!(
                    "Invoice: {} gross {}",
                    report.invoice_reference,
                    usd(report.invoice_total_micros)
                ),
            ];
            lines.push(if report.collected {
                format!(
                    "Collection: {} payment link(s) created",
                    report.payment_links.len()
                )
            } else {
                "Collection: not configured".to_string()
            });
            Ok(CommandReport {
                command: "monthly",
                job_type: apex_metrics::vocab::JobType::Monthly,
                lines,
                json: [(
                    "billing".to_string(),
                    serde_json::json!({
                        "month": report.month,
                        "fees": report.fees.len(),
                        "invoice_reference": report.invoice_reference,
                        "invoice_gross_micros": report.invoice_total_micros,
                        "payment_links": report.payment_links.len(),
                        "collected": report.collected,
                    }),
                )]
                .into_iter()
                .collect(),
                failed: false,
            })
        }
        "recheck" => {
            let due = runner.due_rechecks(ledger, now_unix_ms);
            let mut lines = vec![format!("Due work: {} re-check(s)", due.len())];
            let mut items = Vec::new();
            for item in &due {
                lines.push(format!(
                    "  {} (scope {}, {} months, {} days at {:.0}%)",
                    item.change_id,
                    item.scope,
                    item.months_since_keep,
                    item.plan.analysis_days,
                    item.plan.old_setting_share * 100.0
                ));
                items.push(serde_json::json!({
                    "change_id": item.change_id,
                    "scope": item.scope,
                    "months_since_keep": item.months_since_keep,
                    "analysis_days": item.plan.analysis_days,
                }));
            }
            // The command lists work; it never claims a completion that only
            // the reversal test and its signed result can produce.
            lines.push("Re-check execution: not run by this command".to_string());
            Ok(CommandReport {
                command: "recheck",
                job_type: apex_metrics::vocab::JobType::Recheck,
                lines,
                json: [(
                    "recheck".to_string(),
                    serde_json::json!({
                        "due": items,
                        "execution": "not_run",
                    }),
                )]
                .into_iter()
                .collect(),
                failed: false,
            })
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
            Ok(CommandReport {
                command: "aa",
                job_type: apex_metrics::vocab::JobType::Aa,
                lines: vec![format!("A/A experiment: started {id}")],
                json: [(
                    "aa".to_string(),
                    serde_json::json!({ "experiment_id": id, "state": "started" }),
                )]
                .into_iter()
                .collect(),
                failed: false,
            })
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
            let label = format!("{conclusion:?}");
            Ok(CommandReport {
                command: "aa-eval",
                job_type: apex_metrics::vocab::JobType::AaEval,
                lines: vec![format!("A/A conclusion: {label}")],
                json: [(
                    "aa_eval".to_string(),
                    serde_json::json!({ "experiment_id": experiment, "conclusion": label }),
                )]
                .into_iter()
                .collect(),
                failed: false,
            })
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
    fn scheduled_at_reads_the_flag_then_the_environment() {
        let args = vec![
            "daily".to_string(),
            "--scheduled-at".to_string(),
            "1792087200000".to_string(),
        ];
        assert_eq!(
            scheduled_at(&args, |_| None).unwrap(),
            Some(1_792_087_200_000)
        );
        // The environment is the fallback, and the flag wins over it.
        let env = |name: &str| (name == SCHEDULED_AT_ENV).then(|| "1000".to_string());
        assert_eq!(scheduled_at(&[], env).unwrap(), Some(1_000));
        let both = |name: &str| (name == SCHEDULED_AT_ENV).then(|| "1000".to_string());
        assert_eq!(scheduled_at(&args, both).unwrap(), Some(1_792_087_200_000));
        assert_eq!(scheduled_at(&[], |_| None).unwrap(), None);
        assert_eq!(scheduled_at(&[], |_| Some("  ".into())).unwrap(), None);
        // A malformed value is a typed error, never a silently skipped sample.
        let bad = vec!["daily".to_string(), "--scheduled-at".into(), "soon".into()];
        assert!(scheduled_at(&bad, |_| None)
            .expect_err("bad")
            .contains("unix milliseconds"));
        // Zero or negative timestamps are refused: 0 would record ~57 years
        // of lag, and a negative one is not a schedule at all.
        for bad in ["0", "-1", "-1792087200000", " 0 "] {
            let args = vec!["daily".to_string(), "--scheduled-at".into(), bad.into()];
            let error = scheduled_at(&args, |_| None).expect_err(bad);
            assert!(
                error.contains("positive unix-millisecond timestamp"),
                "{bad}: {error}"
            );
        }
        let env_zero = |name: &str| (name == SCHEDULED_AT_ENV).then(|| "0".to_string());
        assert!(scheduled_at(&[], env_zero).is_err());
        // A future timestamp is valid here; the recording site clamps its
        // lag to zero.
        let future = vec![
            "daily".to_string(),
            "--scheduled-at".into(),
            "9999999999999".into(),
        ];
        assert_eq!(
            scheduled_at(&future, |_| None).unwrap(),
            Some(9_999_999_999_999)
        );
    }

    #[test]
    fn vat_and_monitoring_env_read_the_apex_api_names_first() {
        // The apex-api names are primary.
        let env = |name: &str| match name {
            "APEX_BILLING_VAT_RATE_BP" => Some("2000".to_string()),
            _ => None,
        };
        assert_eq!(vat_rate_bp(&env).unwrap(), Some(2_000));
        // The historical name is the deprecated fallback.
        let env = |name: &str| match name {
            "APEX_INVOICE_VAT_BP" => Some("1900".to_string()),
            _ => None,
        };
        assert_eq!(vat_rate_bp(&env).unwrap(), Some(1_900));
        // The primary name wins when both are set.
        let env = |name: &str| match name {
            "APEX_BILLING_VAT_RATE_BP" => Some("2000".to_string()),
            "APEX_INVOICE_VAT_BP" => Some("1900".to_string()),
            _ => None,
        };
        assert_eq!(vat_rate_bp(&env).unwrap(), Some(2_000));
        // Out of range is a typed error, not a silently unset rate.
        let env = |name: &str| match name {
            "APEX_BILLING_VAT_RATE_BP" => Some("10001".to_string()),
            _ => None,
        };
        let error = vat_rate_bp(&env).expect_err("out of range");
        assert!(error.contains("10000"), "{error}");
        let env = |name: &str| match name {
            "APEX_BILLING_VAT_RATE_BP" => Some("nine".to_string()),
            _ => None,
        };
        assert!(vat_rate_bp(&env).is_err());
        assert_eq!(vat_rate_bp(&|_| None).unwrap(), None);

        // Monitoring mirrors apex-api: > 0 selects the plan, otherwise the
        // plan is not selected; malformed values are typed errors.
        let env = |name: &str| match name {
            "APEX_MONITORING_FEE_MICROS" => Some("500000000".to_string()),
            _ => None,
        };
        assert_eq!(monitoring_fee_micros(&env).unwrap(), Some(500_000_000));
        let env = |name: &str| match name {
            "APEX_MONITORING_MONTHLY_MICROS" => Some("500000000".to_string()),
            _ => None,
        };
        assert_eq!(monitoring_fee_micros(&env).unwrap(), Some(500_000_000));
        let env = |name: &str| match name {
            "APEX_MONITORING_FEE_MICROS" => Some("0".to_string()),
            _ => None,
        };
        assert_eq!(monitoring_fee_micros(&env).unwrap(), None);
        let env = |name: &str| match name {
            "APEX_MONITORING_FEE_MICROS" => Some("lots".to_string()),
            _ => None,
        };
        assert!(monitoring_fee_micros(&env).is_err());
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
            "APEX_BILLING_VAT_RATE_BP" => Some("2000".to_string()),
            "APEX_MONITORING_FEE_MICROS" => Some("500000000".to_string()),
            "APEX_RUNNER_POLICY" => Some("not json".to_string()),
            _ => None,
        };
        // A malformed policy is a typed error, not a panic.
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");
        assert!(build_runner(
            env,
            std::sync::Arc::new(NoOps),
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .is_err());
        // A missing priors file means "no priors yet" (the defaults); a
        // path that exists but cannot be read is a typed error, never
        // silently replaced by defaults.
        let priors_dir =
            std::env::temp_dir().join(format!("apex-cli-priors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&priors_dir);
        std::fs::create_dir_all(&priors_dir).unwrap();
        let env = |name: &str| match name {
            "APEX_RUNNER_PRIORS" => Some(priors_dir.display().to_string()),
            _ => None,
        };
        let error = build_runner(
            env,
            std::sync::Arc::new(NoOps),
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect_err("directory priors");
        assert!(error.contains("priors"), "{error}");
        let env = |name: &str| match name {
            "APEX_RUNNER_PRIORS" => Some("/nope/priors.json".to_string()),
            _ => None,
        };
        assert!(build_runner(
            env,
            std::sync::Arc::new(NoOps),
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .is_ok());
        let _ = std::fs::remove_dir_all(&priors_dir);
        assert!(instrumentation.events.is_none());
        assert!(instrumentation.projections.is_none());
        assert!(instrumentation.notify.is_none());
        assert!(instrumentation
            .logs
            .iter()
            .any(|line| line.contains("event emission is disabled")));
        assert!(instrumentation
            .logs
            .iter()
            .any(|line| line.contains("notifications are disabled")));

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
            &instrumentation,
        )
        .expect("runner");
        assert!(format!("{runner:?}").contains("Runner"));
        assert!(runner.telemetry().is_some());
        assert!(runner.verifier().is_none(), "no ledger path in this env");
    }

    /// The fixed run time the golden outputs pin: 2026-10-15 08:10 UTC.
    fn golden_now() -> i64 {
        crate::calendar::day_number("2026-10-15").unwrap() * 86_400_000 + (8 * 60 + 10) * 60_000
    }

    fn golden_runner() -> crate::runner::Runner {
        let ops = std::sync::Arc::new(crate::ops::RecordingOps::new());
        crate::runner::Runner::new(
            ops,
            crate::model::AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_publisher("northstar")
    }

    async fn run(
        runner: &crate::runner::Runner,
        command: &str,
        args: &[&str],
        now: i64,
    ) -> DispatchOutcome {
        use apex_proof::{InMemoryLedger, Keypair};
        let mut ledger = InMemoryLedger::new(Keypair::from_bytes(&[3u8; 32]));
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        dispatch(command, &args, runner, &mut ledger, now)
            .await
            .expect(command)
    }

    #[tokio::test]
    async fn daily_golden_human_and_json() {
        let runner = golden_runner();
        let outcome = run(&runner, "daily", &[], golden_now()).await;
        assert_eq!(
            outcome.human,
            "Mode: MAX / Publisher: northstar\n\
             Reporting pulls: 0 completed\n\
             Decisions: 0 pending / 0 kept\n\
             Platform operations: no unresolved result\n\
             Ledger: no new records\n\
             Run reference: job_20261015_0810"
        );
        assert!(!outcome.failed);
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(
            json,
            serde_json::json!({
                "schema_version": 1,
                "command": "daily",
                "mode": "max",
                "publisher": "northstar",
                "run_reference": "job_20261015_0810",
                "reporting_pulls": { "completed": 0, "failed": 0 },
                "decisions": { "pending": 0, "kept": 0, "rolled_back": 0, "voided": 0 },
                "platform_operations": { "unresolved": 0 },
                "ledger": {
                    "appended": false,
                    "records_appended": 0,
                    "state": "no_new_records",
                    "verified": serde_json::Value::Null,
                },
            })
        );
    }

    #[tokio::test]
    async fn build_runner_stamps_the_publisher_on_the_event_handle() {
        use apex_analytics::store::{EventStore as _, ProjectionStore as _};
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
        let store = std::sync::Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let registry = std::sync::Arc::new(apex_metrics::Registry::new());
        let instrumentation = Instrumentation {
            registry: registry.clone(),
            telemetry: crate::telemetry::RunnerTelemetry::new(registry),
            events: Some(std::sync::Arc::new(
                crate::events::RunnerEvents::new(
                    std::sync::Arc::new(apex_analytics::sink::AnalyticsClient::new(
                        store.clone(),
                        "dev",
                        "apex-runner-test",
                    )),
                    "",
                )
                .with_environment("dev"),
            )),
            projections: None,
            notify: None,
            logs: Vec::new(),
        };
        let env = |name: &str| match name {
            "APEX_RUNNER_PUBLISHER" => Some("northstar".to_string()),
            _ => None,
        };
        let runner = build_runner(
            env,
            std::sync::Arc::new(NoOps),
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("runner");
        let events = runner.events().expect("events attached");
        events
            .ledger_verified(24, "v0.1.0", 150, 1_792_055_400_000)
            .await;
        let stored = store.load_events().await.expect("stored");
        assert_eq!(stored.len(), 1, "the event was accepted, not refused");
        assert_eq!(stored[0].publisher_id.as_deref(), Some("northstar"));
    }

    #[tokio::test]
    async fn daily_reports_the_projection_refresh_when_configured() {
        use apex_analytics::store::EventStore;
        let store = std::sync::Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let projections = crate::projections_job::RunnerProjections::new(store, "staging");
        let runner = golden_runner().with_projections(projections);
        let outcome = run(&runner, "daily", &[], golden_now()).await;
        assert_eq!(
            outcome.human,
            "Mode: MAX / Publisher: northstar\n\
             Reporting pulls: 0 completed\n\
             Decisions: 0 pending / 0 kept\n\
             Platform operations: no unresolved result\n\
             Projections: rebuilt 4 (0 rows) and switched\n\
             Ledger: no new records\n\
             Run reference: job_20261015_0810"
        );
        assert!(!outcome.failed);
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["projections"]["projections"], serde_json::json!(4));
        assert_eq!(json["projections"]["switched"], serde_json::json!(true));
        assert_eq!(
            json["projections"]["batch_id"],
            serde_json::json!("daily-2026-10-15")
        );
        assert_eq!(
            json["projections"]["published_version"],
            serde_json::json!(1)
        );
    }

    #[tokio::test]
    async fn a_dispatch_with_scheduled_at_records_the_lag_sample() {
        let registry = std::sync::Arc::new(apex_metrics::Registry::new());
        let runner = golden_runner()
            .with_telemetry(crate::telemetry::RunnerTelemetry::new(registry.clone()));
        let scheduled = (golden_now() - 30_000).to_string();
        let outcome = run(
            &runner,
            "daily",
            &["--scheduled-at", scheduled.as_str()],
            golden_now(),
        )
        .await;
        assert!(!outcome.failed);
        let family = registry
            .family_snapshot("apex_job_schedule_lag_seconds")
            .expect("lag family");
        assert_eq!(family.series.len(), 1);
        let histogram = family.series[0].histogram.as_ref().expect("histogram");
        assert_eq!(histogram.count, 1);
        assert!(
            (histogram.sum - 30.0).abs() < 1e-9,
            "lag was {}",
            histogram.sum
        );
        assert!(
            family.series[0]
                .labels
                .iter()
                .any(|(name, value)| *name == "job_type" && value.as_str() == "daily"),
            "{:?}",
            family.series[0].labels
        );
    }

    #[tokio::test]
    async fn a_future_scheduled_at_clamps_the_lag_to_zero() {
        let registry = std::sync::Arc::new(apex_metrics::Registry::new());
        let runner = golden_runner()
            .with_telemetry(crate::telemetry::RunnerTelemetry::new(registry.clone()));
        let scheduled = (golden_now() + 60_000).to_string();
        let outcome = run(
            &runner,
            "daily",
            &["--scheduled-at", scheduled.as_str()],
            golden_now(),
        )
        .await;
        assert!(!outcome.failed);
        let family = registry
            .family_snapshot("apex_job_schedule_lag_seconds")
            .expect("lag family");
        let histogram = family.series[0].histogram.as_ref().expect("histogram");
        assert_eq!(histogram.count, 1);
        assert!(histogram.sum.abs() < 1e-9, "lag was {}", histogram.sum);
    }

    #[tokio::test]
    async fn dispatch_replays_the_analytics_spool_before_the_command() {
        use apex_analytics::store::{EventStore as _, ProjectionStore as _};

        /// A primary sink that refuses every batch deterministically.
        #[derive(Debug)]
        struct RefusingSink;

        #[async_trait::async_trait]
        impl apex_analytics::EventSink for RefusingSink {
            async fn emit(
                &self,
                _events: &[apex_analytics::DomainEvent],
            ) -> Result<apex_analytics::EmitOutcome, apex_analytics::SinkError> {
                Err(apex_analytics::SinkError::Store("refusing".into()))
            }
        }

        // A failed batch lands in the spool first (the primary sink refuses).
        let dir = std::env::temp_dir().join(format!(
            "apex-cli-spool-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let spool = dir.join("spool.jsonl");
        assert!(!spool.exists(), "a fresh spool directory");
        let spooled = crate::events::RunnerEvents::with_spool(
            std::sync::Arc::new(RefusingSink),
            spool.clone(),
        )
        .with_environment("dev")
        .with_publisher("northstar");
        spooled
            .job_run("daily", "job_spooled", golden_now(), "completed", 1)
            .await;
        let withheld = tokio::fs::read_to_string(&spool).await.expect("spool");
        assert_eq!(
            withheld
                .lines()
                .filter(|line| line.contains("job_spooled"))
                .count(),
            1,
            "the batch spooled: {withheld}"
        );
        drop(spooled);

        // Runner start (dispatch) swaps in a working store over the same
        // spool path and re-delivers before the command runs.
        let store = std::sync::Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let events = crate::events::RunnerEvents::with_spool(
            std::sync::Arc::new(apex_analytics::sink::AnalyticsClient::new(
                store.clone(),
                "dev",
                "apex-runner-test",
            )),
            spool.clone(),
        )
        .with_environment("dev")
        .with_publisher("northstar");
        let runner = golden_runner().with_events(std::sync::Arc::new(events));
        let outcome = run(&runner, "recheck", &[], golden_now()).await;
        assert!(!outcome.failed);
        let stored = store.load_events().await.expect("events");
        assert!(
            stored
                .iter()
                .any(|event| event.event_id == "job_run:daily:job_spooled"),
            "the spooled batch was re-delivered at runner start"
        );
        assert!(
            stored.iter().any(|event| event.event_id
                == format!("job_run:recheck:{}", run_reference(golden_now()))),
            "the command's own event landed too"
        );
        assert!(tokio::fs::read_to_string(&spool)
            .await
            .expect("spool")
            .is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn monthly_golden_human_and_json() {
        let runner = golden_runner();
        let outcome = run(&runner, "monthly", &[], golden_now()).await;
        assert_eq!(
            outcome.human,
            "Mode: MAX / Publisher: northstar\n\
             Billing: 0 fee(s) for 2026-09\n\
             Invoice: INV-2026-09-northstar gross USD 0.00\n\
             Collection: not configured\n\
             Ledger: no new records\n\
             Run reference: job_20261015_0810"
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(
            json,
            serde_json::json!({
                "schema_version": 1,
                "command": "monthly",
                "mode": "max",
                "publisher": "northstar",
                "run_reference": "job_20261015_0810",
                "billing": {
                    "month": "2026-09",
                    "fees": 0,
                    "invoice_reference": "INV-2026-09-northstar",
                    "invoice_gross_micros": 0,
                    "payment_links": 0,
                    "collected": false,
                },
                "ledger": {
                    "appended": false,
                    "records_appended": 0,
                    "state": "no_new_records",
                    "verified": serde_json::Value::Null,
                },
            })
        );
    }

    #[tokio::test]
    async fn recheck_golden_is_honest_and_lists_due_work() {
        let runner = golden_runner();
        let outcome = run(&runner, "recheck", &[], golden_now()).await;
        assert_eq!(
            outcome.human,
            "Mode: MAX / Publisher: northstar\n\
             Due work: 0 re-check(s)\n\
             Re-check execution: not run by this command\n\
             Ledger: no new records\n\
             Run reference: job_20261015_0810"
        );
        // Never claims a completion the reversal test did not produce.
        assert!(!outcome.human.contains("completed"));
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["recheck"]["execution"], "not_run");
        assert_eq!(json["recheck"]["due"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn aa_golden_reports_the_started_run_only() {
        let runner = golden_runner();
        let outcome = run(&runner, "aa", &["--unit", "u1"], golden_now()).await;
        assert_eq!(
            outcome.human,
            "Mode: MAX / Publisher: northstar\n\
             A/A experiment: started apex-aa-u1\n\
             Ledger: appended\n\
             Run reference: job_20261015_0810"
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["aa"]["experiment_id"], "apex-aa-u1");
        assert_eq!(json["aa"]["state"], "started");
        assert_eq!(json["ledger"]["appended"], true);
    }

    #[tokio::test]
    async fn the_run_reference_is_utc_minute_shaped() {
        assert_eq!(run_reference(0), "job_19700101_0000");
        assert_eq!(run_reference(golden_now()), "job_20261015_0810");
        assert_eq!(
            run_reference(
                crate::calendar::day_number("2026-12-31").unwrap() * 86_400_000
                    + 23 * 3_600_000
                    + 59 * 60_000
            ),
            "job_20261231_2359"
        );
    }

    #[tokio::test]
    async fn dispatch_runs_every_command_without_a_platform() {
        let ops = std::sync::Arc::new(crate::ops::RecordingOps::new());
        let runner = crate::runner::Runner::new(
            ops,
            crate::model::AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_publisher("pub1");
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[3u8; 32]));

        // daily on an empty ledger is a no-op and not a failure.
        let out = dispatch("daily", &[], &runner, &mut ledger, 1)
            .await
            .expect("daily");
        assert!(out.human.contains("Reporting pulls: 0 completed"));
        assert!(!out.failed);

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

        // monthly with a publisher proceeds and bills nothing.
        let err = dispatch("monthly", &[], &runner, &mut ledger, 1).await;
        assert!(err.is_ok(), "{err:?}");

        assert!(dispatch("bogus", &[], &runner, &mut ledger, 1)
            .await
            .is_err());

        // A zero or negative --scheduled-at is a typed error, never a
        // ~57-year lag sample.
        let err = dispatch(
            "daily",
            &["--scheduled-at".to_string(), "0".to_string()],
            &runner,
            &mut ledger,
            1,
        )
        .await
        .expect_err("zero scheduled-at");
        assert!(err.contains("positive unix-millisecond timestamp"), "{err}");
    }

    #[tokio::test]
    async fn a_verifier_makes_the_ledger_line_honest() {
        #[derive(Debug)]
        struct AlwaysVerified;
        impl crate::verify::LedgerVerifier for AlwaysVerified {
            fn verify(&self, ledger: &dyn apex_proof::LedgerLog) -> crate::verify::LedgerCheck {
                crate::verify::LedgerCheck::Verified {
                    highest_sequence: ledger.len().saturating_sub(1) as u64,
                    duration_ms: 1,
                }
            }
        }
        #[derive(Debug)]
        struct AlwaysFailed;
        impl crate::verify::LedgerVerifier for AlwaysFailed {
            fn verify(&self, _ledger: &dyn apex_proof::LedgerLog) -> crate::verify::LedgerCheck {
                crate::verify::LedgerCheck::Failed {
                    result: apex_metrics::vocab::LedgerResult::InvalidChain,
                    reason: "line 1: prevHash does not link to the previous record".to_string(),
                    failure_line: Some(1),
                }
            }
        }
        let runner = golden_runner().with_verifier(std::sync::Arc::new(AlwaysVerified));
        let outcome = run(&runner, "recheck", &[], golden_now()).await;
        assert!(outcome.human.contains("Ledger: no new records; verified"));

        let runner = golden_runner().with_verifier(std::sync::Arc::new(AlwaysFailed));
        let outcome = run(&runner, "aa", &["--unit", "u1"], golden_now()).await;
        assert!(outcome
            .human
            .contains("Ledger: appended; verification failed"));
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["ledger"]["verified"], false);
        assert_eq!(json["ledger"]["failure"]["line"], 1);
        assert_eq!(json["ledger"]["state"], "appended_verification_failed");
    }

    #[test]
    fn now_is_after_the_product_epoch() {
        assert!(now_unix_ms() > 1_700_000_000_000);
    }
}
