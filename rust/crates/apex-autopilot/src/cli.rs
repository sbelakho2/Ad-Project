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

/// Every value of a repeated `--flag value` in `args`, in order. The cycle
/// consumes one snapshot per value so a publisher with several platforms is
/// handed several files.
pub fn flags(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == name {
            if let Some(value) = args.get(index + 1) {
                out.push(value.clone());
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    out
}

/// `args` with every `--flag value` pair removed: the fleet driver strips the
/// operator's snapshot arguments before injecting each publisher's own files.
pub fn without_flag(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        if args[index] == name {
            index += 2;
        } else {
            out.push(args[index].clone());
            index += 1;
        }
    }
    out
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

/// A configuration value documented as hex that failed to decode. The error
/// names the variable so an operator log points at the exact setting; it
/// mirrors apex-api's `HexKeyError` for the same variable (same class, same
/// message text).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{variable} is not valid hex: {reason}")]
pub struct HexKeyError {
    variable: String,
    reason: String,
}

impl HexKeyError {
    /// The environment variable that carried the invalid value.
    pub fn variable(&self) -> &str {
        &self.variable
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

/// The exact day count a `--month/--start` billing window covers: from the
/// month's first day to the next month's first day. A wrong `--start` or a
/// `--days` that disagrees with the calendar is a typed refusal, never a
/// silent 30-day default that drops the 31st day of revenue.
pub fn month_window_days(month: &str, start: &str) -> Result<u32, String> {
    let (year, month_number) = parse_year_month(month).ok_or("monthly --month must be YYYY-MM")?;
    let (next_year, next_month) = if month_number == 12 {
        (year + 1, 1)
    } else {
        (year, month_number + 1)
    };
    let month_start = format!("{year:04}-{month_number:02}-01");
    if start != month_start {
        return Err(format!(
            "monthly --start must be {month_start}, the first day of --month"
        ));
    }
    let next_start = format!("{next_year:04}-{next_month:02}-01");
    let start_ord = crate::calendar::day_number(&month_start).ok_or("bad month start")?;
    let next_ord = crate::calendar::day_number(&next_start).ok_or("bad next month start")?;
    u32::try_from(next_ord - start_ord).map_err(|_| "bad month length".to_string())
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

/// The effective policy file written by the control plane's `policy_set`
/// capability. When set, it wins over the inline `APEX_RUNNER_POLICY`.
pub const EFFECTIVE_POLICY_ENV: &str = "APEX_EFFECTIVE_POLICY";

/// The autopilot policy: the effective policy file in preference to the
/// inline `APEX_RUNNER_POLICY`, or the conservative default when neither is
/// configured. A configured but unreadable, malformed or out-of-bounds file
/// is a typed error, never a silent default (the cycle refuses).
fn policy_from_env(
    env: &impl Fn(&str) -> Option<String>,
) -> Result<crate::model::AutopilotPolicy, String> {
    let effective = env(EFFECTIVE_POLICY_ENV)
        .filter(|value| !value.trim().is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let default = std::path::PathBuf::from(".apex/effective-policy.json");
            default.exists().then_some(default)
        });
    if let Some(path) = effective {
        return crate::model::load_policy_file(&path)
            .map_err(|error| format!("effective policy: {error}"));
    }
    match env("APEX_RUNNER_POLICY") {
        Some(raw) if !raw.trim().is_empty() => {
            let policy = serde_json::from_str::<crate::model::AutopilotPolicy>(&raw)
                .map_err(|e| format!("policy: {e}"))?;
            policy.validate().map_err(|e| format!("policy: {e}"))?;
            Ok(policy)
        }
        _ => Ok(crate::model::AutopilotPolicy::conservative()),
    }
}

/// Where the bucket-model (priors) state comes from in this mode.
enum PriorsSource<'a> {
    /// `APEX_RUNNER_PRIORS` names one JSON file: the legacy single-publisher
    /// deployment.
    File,
    /// `APEX_RUNNER_PRIORS` names a directory of `<publisher>.json` files:
    /// each registered publisher learns into its own model.
    Directory(&'a str),
}

/// Build a [`Runner`](crate::Runner) from the environment: policy, publisher,
/// VAT, Mollie collection, bucket model, alert sink and account watch. Kept
/// out of the binary so the whole configuration path is unit tested.
pub fn build_runner(
    env: impl Fn(&str) -> Option<String>,
    ops: std::sync::Arc<dyn crate::ops::PlatformOps>,
    transport: std::sync::Arc<dyn apex_connectors::transport::Transport>,
    instrumentation: &Instrumentation,
) -> Result<crate::runner::Runner, String> {
    use apex_lift::guardrails::FillFormat;
    let policy = policy_from_env(&env)?;
    let publisher = env_nonempty(&env, "APEX_RUNNER_PUBLISHER");
    let mut runner = crate::runner::Runner::new(ops, policy, FillFormat::Rewarded);
    if let Some(publisher) = &publisher {
        runner = runner.with_publisher(publisher.clone());
    }
    let ledger_path = env_nonempty(&env, "APEX_RUNNER_LEDGER").map(std::path::PathBuf::from);
    let signing_key = env_nonempty(&env, "APEX_RUNNER_SIGNING_KEY_HEX")
        .and_then(|seed| parse_seed(&seed).ok())
        .map(|seed| apex_proof::Keypair::from_bytes(&seed));
    // The connections root resolves through the same fallbacks the fleet mode
    // uses (`APEX_CONNECTIONS_ROOT`, then the apex-api spelling
    // `APEX_CREDENTIAL_ROOT`), so a legacy deployment that names it records
    // monitoring state exactly as the fleet path does. Without it the monthly
    // pass records the typed monitoring-skip reason instead of aborting.
    let connections_root = crate::registry::vault_root_from_env(&env);
    runner = configure_runner(
        &env,
        runner,
        publisher.as_deref(),
        PriorsSource::File,
        true,
        ledger_path.as_deref().zip(signing_key.as_ref()),
        connections_root.as_deref(),
        transport,
        instrumentation,
    )?;
    Ok(runner)
}

/// Build one fleet publisher's runner: the platform adapter comes from the
/// vaulted keys the registry decrypted, everything else reads the same
/// environment names the legacy single-publisher runner reads. In a
/// multi-publisher fleet the fleet-level collaborators (verified payment
/// events, projection refresh) are NOT attached — `dispatch_fleet` runs them
/// once per run; the exactly-one-publisher registry keeps them per publisher
/// so its reports stay byte-identical to the legacy mode.
#[allow(clippy::too_many_arguments)]
fn build_publisher_runner(
    env: &impl Fn(&str) -> Option<String>,
    registered: &crate::registry::RunnablePublisher,
    ledger_root: &std::path::Path,
    vault_root: &std::path::Path,
    signing_key: &apex_proof::Keypair,
    transport: std::sync::Arc<dyn apex_connectors::transport::Transport>,
    instrumentation: &Instrumentation,
    fleet_level: bool,
) -> Result<crate::runner::Runner, String> {
    use apex_lift::guardrails::FillFormat;
    let policy = policy_from_env(env)?;
    let writable = split_csv(env_nonempty(env, "APEX_RUNNER_WRITABLE"));
    // The platform adapter comes from the vaulted keys the registry
    // decrypted: the report key and bases for MAX, the secret key / refresh
    // token / app key set for LevelPlay, the access token and account id for
    // AdMob. The registry validated the key set per platform before this
    // runner was built. Every vaulted platform gets its own adapter so a
    // secondary platform's proposals and experiments never route through the
    // primary adapter.
    let ops = build_platform_ops(
        env,
        &registered.publisher,
        &registered.platform,
        &registered.credentials,
        &transport,
        &writable,
    )?;
    let mut runner = crate::runner::Runner::new(ops, policy, FillFormat::Rewarded)
        .with_publisher(registered.publisher.clone());
    for wire in secondary_wires(&registered.platform, &registered.vaulted_platforms) {
        let Some(platform_credentials) = registered.vaulted_platforms.get(wire) else {
            continue;
        };
        let Some(platform) = wire_platform(wire) else {
            continue;
        };
        let secondary = build_platform_ops(
            env,
            &registered.publisher,
            wire,
            platform_credentials,
            &transport,
            &writable,
        )?;
        runner = runner.with_platform_ops(platform, secondary);
    }
    let ledger_path = ledger_root.join(format!("{}.jsonl", registered.publisher));
    let mut runner = configure_runner(
        env,
        runner,
        Some(&registered.publisher),
        PriorsSource::Directory(&registered.publisher),
        fleet_level,
        Some((ledger_path.as_path(), signing_key)),
        Some(vault_root),
        transport,
        instrumentation,
    )?;
    if !fleet_level {
        // Multi-publisher fleet: one pass claims only its own jobs.
        runner = runner.with_owner_scoped_operator_jobs(!fleet_level);
    }
    Ok(runner)
}

/// The vaulted wire names that need a secondary adapter: everything except
/// the publisher's validated primary platform. Keeping this pure makes the
/// one-primary/many-secondary routing rule directly testable.
fn secondary_wires<'a>(
    primary: &str,
    vaulted: &'a std::collections::BTreeMap<String, crate::registry::VaultedCredentials>,
) -> Vec<&'a str> {
    vaulted
        .keys()
        .map(String::as_str)
        .filter(|wire| *wire != primary)
        .collect()
}

/// The platform id for one registry wire name.
fn wire_platform(wire: &str) -> Option<apex_core::PlatformId> {
    match wire {
        crate::registry::MAX_WIRE => Some(apex_core::PlatformId::AppLovinMax),
        crate::registry::LEVELPLAY_WIRE => Some(apex_core::PlatformId::LevelPlay),
        crate::registry::ADMOB_WIRE => Some(apex_core::PlatformId::Admob),
        _ => None,
    }
}

/// Build the live adapter for one platform's vaulted key set.
fn build_platform_ops(
    env: &impl Fn(&str) -> Option<String>,
    publisher: &str,
    wire: &str,
    credentials: &crate::registry::VaultedCredentials,
    transport: &std::sync::Arc<dyn apex_connectors::transport::Transport>,
    writable: &[String],
) -> Result<std::sync::Arc<dyn crate::ops::PlatformOps>, String> {
    let writable = || writable.to_vec();
    match wire {
        crate::registry::MAX_WIRE => {
            let report_key = credentials.report_key.as_deref().ok_or_else(|| {
                format!("{publisher}: the vaulted MAX key set has no reporting key")
            })?;
            let reporting_base = credentials
                .reporting_base_url
                .clone()
                .unwrap_or_else(|| apex_connectors::max::MAX_REPORTING_BASE.to_string());
            let ops = crate::live_max::MaxPlatformOps::new(
                &credentials.api_key,
                report_key,
                &credentials.base_url,
                &reporting_base,
                std::sync::Arc::clone(transport),
                std::sync::Arc::new(crate::live_max::SystemWall),
                writable(),
            )
            .and_then(|ops| {
                ops.with_evidence_root(
                    crate::pull_evidence::evidence_root_from_env(env).join(publisher),
                )
            })
            .map_err(|error| format!("{publisher}: max adapter: {error}"))?;
            Ok(std::sync::Arc::new(ops))
        }
        crate::registry::LEVELPLAY_WIRE => {
            let refresh_token = credentials.refresh_token.as_deref().ok_or_else(|| {
                format!("{publisher}: the vaulted LevelPlay key set has no refresh token")
            })?;
            let ops = crate::live_levelplay::LevelPlayPlatformOps::new(
                &credentials.api_key,
                refresh_token,
                &credentials.account_id,
                &credentials.base_url,
                std::sync::Arc::clone(transport),
                std::sync::Arc::new(crate::live_max::SystemWall),
                writable(),
            )
            .and_then(|ops| {
                ops.with_evidence_root(
                    crate::pull_evidence::evidence_root_from_env(env).join(publisher),
                )
            })
            .map_err(|error| format!("{publisher}: levelplay adapter: {error}"))?;
            Ok(std::sync::Arc::new(ops))
        }
        crate::registry::ADMOB_WIRE => {
            // AdMob access tokens expire after about an hour. A runner job is
            // a short-lived process, so exchanging the refresh token once at
            // startup is enough; without client credentials the stored access
            // token is used as before.
            let access_token = match (
                credentials.refresh_token.as_deref(),
                env_nonempty(env, "APEX_ADMOB_CLIENT_ID"),
                env_nonempty(env, "APEX_ADMOB_CLIENT_SECRET"),
            ) {
                (Some(refresh), Some(client_id), Some(client_secret)) => {
                    let token_url = env_nonempty(env, "APEX_ADMOB_OAUTH_TOKEN_URL")
                        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string());
                    apex_connectors::admob::refresh_access_token_blocking(
                        &token_url,
                        &client_id,
                        &client_secret,
                        refresh,
                    )
                    .map_err(|error| format!("{publisher}: admob token refresh: {error}"))?
                }
                _ => credentials.api_key.clone(),
            };
            let ops = crate::live_admob::AdmobPlatformOps::new(
                &access_token,
                &credentials.account_id,
                &credentials.base_url,
                std::sync::Arc::clone(transport),
                writable(),
            )
            .map_err(|error| format!("{publisher}: admob adapter: {error}"))?;
            Ok(std::sync::Arc::new(ops))
        }
        other => Err(format!(
            "{publisher}: no runner adapter for platform '{other}'"
        )),
    }
}

/// Apply the shared environment configuration to a runner. `fleet_level`
/// controls the once-per-run collaborators; `verifier` attaches the ledger
/// verifier for a known ledger path and signing key.
#[allow(clippy::too_many_arguments)]
fn configure_runner(
    env: &impl Fn(&str) -> Option<String>,
    mut runner: crate::runner::Runner,
    publisher: Option<&str>,
    priors: PriorsSource<'_>,
    fleet_level: bool,
    verifier: Option<(&std::path::Path, &apex_proof::Keypair)>,
    connections_root: Option<&std::path::Path>,
    transport: std::sync::Arc<dyn apex_connectors::transport::Transport>,
    instrumentation: &Instrumentation,
) -> Result<crate::runner::Runner, String> {
    // The billing configuration reads the same names apex-api reads. The
    // historical runner-only names stay as deprecated fallbacks and are
    // announced on stderr when used, so a deployment never silently runs on
    // two divergent spellings.
    if let Some(bp) = vat_rate_bp(env)? {
        runner = runner.with_vat_rate(bp);
    }
    // Mollie needs the API key plus the two URLs every payment carries. A
    // partial set is not a client: it is refused so a deployment cannot
    // create links that never receive a webhook.
    match (
        env("APEX_MOLLIE_API_KEY"),
        env("APEX_MOLLIE_WEBHOOK_URL"),
        env("APEX_MOLLIE_REDIRECT_URL"),
    ) {
        (Some(api_key), Some(webhook_url), Some(redirect_url)) => {
            let base = env_or(env, "APEX_MOLLIE_API_BASE", "https://api.mollie.com");
            let client = apex_billing::MollieClient::new(
                api_key,
                base,
                webhook_url,
                redirect_url,
            )
            .map_err(|e| format!("mollie client: {e}"))?;
            runner = runner.with_mollie(std::sync::Arc::new(client), transport);
        }
        (None, None, None) => {}
        _ => {
            return Err(
                "APEX_MOLLIE_API_KEY, APEX_MOLLIE_WEBHOOK_URL and APEX_MOLLIE_REDIRECT_URL must be set together"
                    .into(),
            )
        }
    }
    if let Some(watch) = env_nonempty(env, "APEX_ACCOUNT_WATCH_FILE") {
        let threshold = match env_nonempty(env, "APEX_ACCOUNT_WATCH_THRESHOLD") {
            Some(raw) => raw.parse::<f64>().map_err(|_| {
                "APEX_ACCOUNT_WATCH_THRESHOLD must be a decimal fraction".to_string()
            })?,
            None => crate::account_watch::DEFAULT_DROP_THRESHOLD,
        };
        runner = runner.with_account_watch(watch, threshold)?;
    }
    if let Some(root) = env_nonempty(env, "APEX_PROPOSALS_ROOT") {
        runner = runner.with_proposals_root(root);
    }
    if let Some(path) = env_nonempty(env, "APEX_RUNNER_PRIORS") {
        // A missing priors file is "no priors yet" (the defaults); a file
        // that exists but cannot be read is a typed error, never silently
        // replaced by defaults. In fleet mode the path names a directory and
        // each publisher reads its own `<publisher>.json` model.
        let path = match priors {
            PriorsSource::File => std::path::PathBuf::from(path),
            PriorsSource::Directory(publisher) => {
                std::path::PathBuf::from(&path).join(format!("{publisher}.json"))
            }
        };
        let priors = match std::fs::read_to_string(&path) {
            Ok(raw) if !raw.trim().is_empty() => {
                serde_json::from_str::<apex_lift::priors::LiftPriors>(&raw)
                    .map_err(|e| format!("priors: {e}"))?
            }
            Ok(_) => apex_lift::priors::LiftPriors::default(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                apex_lift::priors::LiftPriors::default()
            }
            Err(error) => return Err(format!("priors at {}: {error}", path.display())),
        };
        runner = runner.with_priors(priors, &path);
    }
    if let Some(path) = env_nonempty(env, "APEX_ALERT_FILE") {
        let sink: std::sync::Arc<dyn crate::alerts::AlertSink> =
            std::sync::Arc::new(crate::alerts::FileAlertSink::new(path));
        runner = runner.with_alerts(sink);
    }
    runner = runner.with_telemetry(instrumentation.telemetry.clone());
    if let Some(events) = &instrumentation.events {
        // The instrumentation handle is built before the publisher is known;
        // stamp it here so every emitted event carries the tenant. Without
        // this the store refuses the whole batch (required field).
        let events = match publisher {
            Some(publisher) => {
                std::sync::Arc::new((**events).clone().with_publisher(publisher.to_string()))
            }
            None => events.clone(),
        };
        runner = runner.with_events(events);
    }
    if fleet_level {
        if let Some(projections) = &instrumentation.projections {
            runner = runner.with_projections(projections.clone());
        }
    }
    if let Some(notify) = &instrumentation.notify {
        runner = runner.with_notifications(notify.clone());
    }
    runner = runner.with_operator_jobs(env_or(
        env,
        crate::operator_jobs::JOBS_DIR_ENV,
        crate::operator_jobs::DEFAULT_JOBS_DIR,
    ));
    // The control plane's pause/resume register: read before every cycle, so
    // a pause stops writes while reads/measurement continue.
    runner = runner.with_interventions(env_or(
        env,
        crate::interventions::INTERVENTIONS_ENV,
        crate::interventions::DEFAULT_INTERVENTIONS,
    ));
    if fleet_level {
        if let Some(events) = env_nonempty(env, "APEX_PAYMENT_EVENTS") {
            runner = runner.with_payment_events(
                events,
                env_nonempty(env, "APEX_PAYMENT_LEDGER").map(std::path::PathBuf::from),
                env_nonempty(env, "APEX_PAYMENT_RETRIES").map(std::path::PathBuf::from),
            );
        }
        if let Some(cursor) = env_nonempty(env, crate::payment_events::PAYMENT_CURSOR_ENV) {
            runner = runner.with_payment_cursor(cursor);
        }
    }
    // The verifier re-reads the ledger file the runner writes; without the
    // path and key it is absent, and the report says "appended" without
    // claiming a verification.
    if let Some((ledger_path, signing_key)) = verifier {
        runner = runner.with_verifier(std::sync::Arc::new(crate::verify::FileLedgerVerifier::new(
            ledger_path,
            signing_key.verifying_key(),
        )));
    }
    if let Some(root) = connections_root {
        runner = runner.with_connections_root(root);
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
    // Keep each publisher's evidence apart even when the configured root is
    // shared by the whole deployment.
    let evidence_root = env_nonempty(&env, "APEX_RUNNER_PUBLISHER")
        .map(|publisher| crate::pull_evidence::evidence_root_from_env(&env).join(publisher))
        .unwrap_or_else(|| crate::pull_evidence::evidence_root_from_env(&env));
    let ops = crate::live_max::MaxPlatformOps::new(
        &api_key,
        &report_key,
        &management_base,
        &reporting_base,
        transport,
        std::sync::Arc::new(crate::live_max::SystemWall),
        writable,
    )
    .and_then(|ops| ops.with_evidence_root(evidence_root))
    .map_err(|e| format!("max adapter: {e}"))?;
    Ok(std::sync::Arc::new(ops))
}

/// One runnable publisher in a fleet run: its runner and its per-publisher
/// ledger at `<APEX_RUNNER_LEDGER_ROOT>/<publisher>.jsonl` (the naming
/// apex-api and apex-control read).
#[derive(Debug)]
pub struct FleetPublisher {
    /// The publisher name.
    pub publisher: String,
    /// The runner built from this publisher's vaulted keys.
    pub runner: crate::runner::Runner,
    /// The per-publisher ledger path.
    pub ledger_path: std::path::PathBuf,
    /// This publisher's own per-platform snapshots, the adapter platform
    /// first. The fleet cycle is handed exactly these files.
    pub snapshot_paths: Vec<std::path::PathBuf>,
}

/// One registry skip as the fleet report carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetSkip {
    /// The publisher name.
    pub publisher: String,
    /// The typed reason, rendered.
    pub reason: String,
}

/// The verified payment-event configuration for the fleet-level daily pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetPayments {
    /// The append-only collection file (`APEX_PAYMENT_EVENTS`).
    pub events_path: std::path::PathBuf,
    /// The consumed-offset cursor; default `<collection>.cursor`.
    pub cursor_path: Option<std::path::PathBuf>,
    /// The collected-reference ledger (`APEX_PAYMENT_LEDGER`).
    pub ledger_path: Option<std::path::PathBuf>,
    /// The scheduled-retry file (`APEX_PAYMENT_RETRIES`).
    pub retries_path: Option<std::path::PathBuf>,
}

/// A fleet run: one runner per runnable registry publisher plus the skip list
/// and the fleet-level collaborators that run once per run.
#[derive(Debug)]
pub struct Fleet {
    /// The runnable publishers (sorted).
    pub publishers: Vec<FleetPublisher>,
    /// The skipped publishers with typed reasons (sorted).
    pub skipped: Vec<FleetSkip>,
    /// The process telemetry handle (fleet-level collaborators use it too).
    pub telemetry: crate::telemetry::RunnerTelemetry,
    /// The unstamped analytics emitter; per-publisher runners carry their own
    /// stamped handles.
    pub events: Option<std::sync::Arc<crate::events::RunnerEvents>>,
    /// The notification plan (messages are already publisher-scoped).
    pub notify: Option<std::sync::Arc<crate::notify::NotifyPlan>>,
    /// The fleet-level projection refresh, when the analytics store is
    /// configured.
    pub projections: Option<crate::projections_job::RunnerProjections>,
    /// The fleet-level verified payment events, when configured.
    pub payments: Option<FleetPayments>,
    /// The fleet signing key every per-publisher ledger is opened with.
    pub signing_key: apex_proof::Keypair,
}

impl Fleet {
    /// Exactly one runnable publisher and no skips: the reports keep the
    /// single-publisher shape and the runner keeps the fleet-level
    /// collaborators attached.
    pub fn is_singleton(&self) -> bool {
        self.publishers.len() == 1 && self.skipped.is_empty()
    }
}

/// Build the fleet from a loaded registry: one configured runner per runnable
/// publisher, the skip list rendered, and the fleet-level payment
/// configuration read once from the environment. The exactly-one-publisher
/// fleet attaches the fleet-level collaborators to its single runner (the
/// committed single-publisher shape); a multi-publisher fleet runs them once
/// in [`dispatch_fleet`].
#[allow(clippy::too_many_arguments)]
pub fn build_fleet(
    env: impl Fn(&str) -> Option<String>,
    registry: &crate::registry::PublisherRegistry,
    ledger_root: &std::path::Path,
    vault_root: &std::path::Path,
    snapshot_root: &std::path::Path,
    signing_key: &apex_proof::Keypair,
    transport: std::sync::Arc<dyn apex_connectors::transport::Transport>,
    instrumentation: &Instrumentation,
) -> Result<Fleet, String> {
    let singleton = registry.is_singleton();
    // Concurrent publisher passes share one operator-jobs directory; this
    // lock serializes claiming and transitioning each job file.
    let operator_jobs_lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let mut publishers = Vec::new();
    for registered in registry.runnable() {
        let mut runner = build_publisher_runner(
            &env,
            registered,
            ledger_root,
            vault_root,
            signing_key,
            std::sync::Arc::clone(&transport),
            instrumentation,
            singleton,
        )?;
        if !singleton {
            runner = runner.with_operator_jobs_lock(std::sync::Arc::clone(&operator_jobs_lock));
        }
        let snapshot_paths = registered
            .snapshot_platforms()
            .into_iter()
            .map(|wire| crate::registry::snapshot_path(snapshot_root, &registered.publisher, &wire))
            .collect();
        publishers.push(FleetPublisher {
            publisher: registered.publisher.clone(),
            runner,
            ledger_path: ledger_root.join(format!("{}.jsonl", registered.publisher)),
            snapshot_paths,
        });
    }
    let skipped = registry
        .skipped()
        .map(|skip| FleetSkip {
            publisher: skip.publisher.clone(),
            reason: skip.reason.to_string(),
        })
        .collect();
    let payments = if singleton {
        // Attached to the single runner instead.
        None
    } else {
        env_nonempty(&env, "APEX_PAYMENT_EVENTS").map(|events| FleetPayments {
            events_path: std::path::PathBuf::from(events),
            cursor_path: env_nonempty(&env, crate::payment_events::PAYMENT_CURSOR_ENV)
                .map(std::path::PathBuf::from),
            ledger_path: env_nonempty(&env, "APEX_PAYMENT_LEDGER").map(std::path::PathBuf::from),
            retries_path: env_nonempty(&env, "APEX_PAYMENT_RETRIES").map(std::path::PathBuf::from),
        })
    };
    Ok(Fleet {
        publishers,
        skipped,
        telemetry: instrumentation.telemetry.clone(),
        events: instrumentation.events.clone(),
        notify: instrumentation.notify.clone(),
        projections: if singleton {
            None
        } else {
            instrumentation.projections.clone()
        },
        payments,
        signing_key: signing_key.clone(),
    })
}

/// Dispatch one command over the fleet.
///
/// Every runnable publisher's command runs against its own runner and its own
/// ledger file (each ledger verifies as its own chain). A fleet with exactly
/// one publisher and no skips returns that publisher's outcome verbatim, so
/// the committed single-publisher output stays byte-identical; otherwise the
/// report carries the pinned top-level fields (`schema_version`, `command`,
/// `mode`, `publisher: null`, `run_reference`), a `publishers` array with one
/// block per publisher, a `skipped` array with the typed reasons, and an
/// aggregate `ledger` block. Fleet-level work runs once per run: the
/// projection refresh and the verified payment events (their publisher is
/// read from each reference) for the daily command.
pub async fn dispatch_fleet(
    command: &str,
    args: &[String],
    fleet: &Fleet,
    now_unix_ms: i64,
) -> Result<DispatchOutcome, String> {
    let mut outcomes: Vec<(String, DispatchOutcome)> = Vec::new();
    if command == "daily" && fleet.publishers.len() > 1 {
        // Publisher passes are independent: each has its own runner, ledger
        // chain and priors. The shared pieces are handled explicitly: the
        // analytics spool is replayed once here (not once per pass), the
        // shared emitter is flushed once after every pass, and the fleet-wide
        // operator-jobs lock in `Runner` serializes the shared jobs
        // directory. Ledger appends are per-publisher files, so there is no
        // cross-publisher append to interleave.
        replay_spool(fleet.events.as_deref()).await;
        use futures_util::StreamExt;
        let passes = futures_util::stream::iter(fleet.publishers.iter().enumerate().map(
            |(index, publisher)| {
                let call_args = if command == "cycle" && !publisher.snapshot_paths.is_empty() {
                    let mut injected = without_flag(args, "--snapshot");
                    for path in &publisher.snapshot_paths {
                        injected.push("--snapshot".to_string());
                        injected.push(path.display().to_string());
                    }
                    injected
                } else {
                    args.to_vec()
                };
                async move {
                    let result = async {
                        let _run_lock = RunLock::acquire(&publisher.ledger_path)
                            .map_err(|error| format!("{}: {error}", publisher.publisher))?;
                        let mut ledger = apex_proof::Ledger::open(
                            &publisher.ledger_path,
                            fleet.signing_key.clone(),
                        )
                        .map_err(|error| format!("ledger for {}: {error}", publisher.publisher))?;
                        dispatch_configured(
                            command,
                            &call_args,
                            &publisher.runner,
                            &mut ledger,
                            now_unix_ms,
                            false,
                            false,
                        )
                        .await
                    }
                    .await;
                    (index, publisher.publisher.clone(), result)
                }
            },
        ))
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
        // Completion order is nondeterministic; report (and surface the
        // first error) in the original sorted publisher order.
        let mut ordered = passes;
        ordered.sort_by_key(|(index, _, _)| *index);
        let mut first_error: Option<String> = None;
        for (_, publisher, result) in ordered {
            match result {
                Ok(outcome) => outcomes.push((publisher, outcome)),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        // One flush for the shared batching emitter after every pass has
        // stopped appending; per-publisher passes do not flush concurrently.
        for publisher in &fleet.publishers {
            if let Some(events) = publisher.runner.events() {
                events.flush().await;
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
    } else {
        for publisher in &fleet.publishers {
            let _run_lock = RunLock::acquire(&publisher.ledger_path)
                .map_err(|error| format!("{}: {error}", publisher.publisher))?;
            let mut ledger =
                apex_proof::Ledger::open(&publisher.ledger_path, fleet.signing_key.clone())
                    .map_err(|error| format!("ledger for {}: {error}", publisher.publisher))?;
            // The cycle consumes only this publisher's own per-platform snapshots.
            // A publisher with several platforms gets several files; the operator's
            // own `--snapshot` arguments (if any) are replaced, and every other
            // command keeps them untouched.
            let call_args = if command == "cycle" && !publisher.snapshot_paths.is_empty() {
                let mut injected = without_flag(args, "--snapshot");
                for path in &publisher.snapshot_paths {
                    injected.push("--snapshot".to_string());
                    injected.push(path.display().to_string());
                }
                injected
            } else {
                args.to_vec()
            };
            let outcome = dispatch(
                command,
                &call_args,
                &publisher.runner,
                &mut ledger,
                now_unix_ms,
            )
            .await?;
            outcomes.push((publisher.publisher.clone(), outcome));
        }
    }
    if fleet.is_singleton() {
        return Ok(outcomes
            .pop()
            .map(|(_, outcome)| outcome)
            .ok_or("singleton fleet has no publisher")?);
    }

    // Fleet-level work: once per run, never once per publisher.
    let mut extras_lines: Vec<String> = Vec::new();
    let mut extras_json = serde_json::Map::new();
    let mut extra_failed = false;
    if command == "daily" {
        if let Some(payments) = &fleet.payments {
            let (_report, lines, json) = fleet_payment_pass(fleet, payments, now_unix_ms).await?;
            extras_lines.extend(lines);
            extras_json.insert("payment_events".into(), json);
        }
        if let Some(projections) = &fleet.projections {
            let batch_id = crate::projections_job::daily_batch_id(now_unix_ms);
            match projections
                .refresh(&batch_id, Some(&fleet.telemetry), now_unix_ms)
                .await
            {
                Ok(refresh) => {
                    extras_lines.push(refresh.summary());
                    let lag: serde_json::Map<String, serde_json::Value> = refresh
                        .lag_seconds
                        .iter()
                        .map(|(name, seconds)| (name.clone(), serde_json::json!(seconds)))
                        .collect();
                    extras_json.insert(
                        "projections".into(),
                        serde_json::json!({
                            "batch_id": refresh.batch_id,
                            "published_version": refresh.published_version,
                            "projections": refresh.projections,
                            "rows": refresh.rows,
                            "switched": refresh.switched,
                            "lag_seconds": lag,
                        }),
                    );
                }
                Err(error) => {
                    eprintln!("apex-runner: projection refresh failed: {error}");
                    extras_lines.push(format!("Projections: refresh failed ({error})"));
                    extras_json.insert("projection_failure".into(), serde_json::json!(error));
                    extra_failed = true;
                }
            }
        }
    }

    let reference = run_reference(now_unix_ms);
    let mut lines = vec![format!(
        "Fleet: {} publisher(s) / {} skipped",
        outcomes.len(),
        fleet.skipped.len()
    )];
    let mut publisher_blocks = Vec::new();
    let mut any_appended = false;
    let mut records_appended: u64 = 0;
    let mut failed = extra_failed;
    for (publisher, outcome) in &outcomes {
        failed |= outcome.failed;
        let mut block: Vec<&str> = outcome.human.lines().collect();
        // Drop the per-publisher scope line and the run reference: the fleet
        // header names every publisher and the run reference prints once.
        if !block.is_empty() {
            block.remove(0);
        }
        if block
            .last()
            .map(|line| line.starts_with("Run reference: "))
            .unwrap_or(false)
        {
            block.pop();
        }
        lines.push(format!("Publisher: {publisher}"));
        lines.extend(block.iter().map(|line| (*line).to_string()));

        let document: serde_json::Value = serde_json::from_str(&outcome.json)
            .map_err(|error| format!("{publisher} report: {error}"))?;
        let mut object = document.as_object().cloned().unwrap_or_default();
        for key in ["schema_version", "command", "mode", "run_reference"] {
            object.remove(key);
        }
        object.insert(
            "publisher".into(),
            serde_json::Value::from(publisher.clone()),
        );
        if let Some(ledger) = object.get("ledger") {
            if ledger["appended"] == serde_json::Value::Bool(true) {
                any_appended = true;
            }
            records_appended =
                records_appended.saturating_add(ledger["records_appended"].as_u64().unwrap_or(0));
        }
        publisher_blocks.push(serde_json::Value::Object(object));
    }
    lines.extend(extras_lines);
    for skip in &fleet.skipped {
        lines.push(format!("Skipped: {}: {}", skip.publisher, skip.reason));
    }
    lines.push(format!("Run reference: {reference}"));

    // The fleet's mode is its publishers' platform when every runnable
    // publisher shares one; a fleet mixing platforms (or with no runnable
    // publisher at all) reports `fleet` instead of claiming a platform.
    let mut platforms: Vec<apex_core::PlatformId> = fleet
        .publishers
        .iter()
        .map(|publisher| publisher.runner.platform_ops().platform())
        .collect();
    platforms.sort_by_key(|platform| mode_of(*platform));
    platforms.dedup();
    let mode = match platforms.as_slice() {
        [only] => mode_of(*only).to_string(),
        _ => "fleet".to_string(),
    };
    let skipped: Vec<serde_json::Value> = fleet
        .skipped
        .iter()
        .map(|skip| {
            serde_json::json!({
                "publisher": skip.publisher,
                "reason": skip.reason,
            })
        })
        .collect();
    let mut json = serde_json::Map::new();
    json.insert(
        "schema_version".into(),
        serde_json::Value::from(REPORT_SCHEMA_VERSION),
    );
    json.insert("command".into(), serde_json::Value::from(command));
    json.insert("mode".into(), serde_json::Value::from(mode));
    json.insert("publisher".into(), serde_json::Value::Null);
    json.insert("run_reference".into(), serde_json::Value::from(reference));
    json.insert(
        "publishers".into(),
        serde_json::Value::Array(publisher_blocks),
    );
    json.insert("skipped".into(), serde_json::Value::Array(skipped));
    json.insert(
        "ledger".into(),
        serde_json::json!({
            "appended": any_appended,
            "records_appended": records_appended,
        }),
    );
    for (key, value) in extras_json {
        json.insert(key, value);
    }
    Ok(DispatchOutcome {
        human: lines.join("\n"),
        json: serde_json::Value::Object(json).to_string(),
        failed,
    })
}

/// One platform snapshot the fleet audit built and scored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditSnapshotReport {
    /// The platform wire name (`applovin_max`, `levelplay`, `admob`).
    pub platform: String,
    /// The adapter platform the wire maps to; drives the fleet mode label.
    pub platform_id: apex_core::PlatformId,
    /// The file the snapshot was written to.
    pub path: String,
    /// Ad units in the snapshot.
    pub ad_units: usize,
    /// The audit engine's digest of the snapshot.
    pub digest: String,
    pub findings: usize,
    pub critical: usize,
    pub warning: usize,
    pub info: usize,
    pub eligible_rule_groups: u32,
    pub expected_rule_groups: u32,
    pub missing_inputs: Vec<String>,
}

/// One publisher's audit: its per-platform snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPublisherReport {
    /// The publisher name.
    pub publisher: String,
    /// The snapshots built for it, adapter platform first.
    pub snapshots: Vec<AuditSnapshotReport>,
}

/// One platform snapshot the fleet audit could not build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditFailure {
    pub publisher: String,
    pub platform: String,
    /// The typed platform error, rendered.
    pub reason: String,
}

/// The whole fleet audit, before rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetAuditReport {
    /// The runnable publishers that were audited (sorted).
    pub publishers: Vec<AuditPublisherReport>,
    /// The registry's skipped publishers with typed reasons (sorted).
    pub skipped: Vec<FleetSkip>,
    /// Platform snapshots that could not be built.
    pub failures: Vec<AuditFailure>,
}

/// Audit every registered publisher, one snapshot per platform its vault
/// holds, each written at `<root>/<publisher>/<platform>.json` (the layout the
/// console's `JsonFileAuditSource` reads). A publisher that fails its
/// per-platform refresh is recorded as a typed failure and the pass continues;
/// a registry skip keeps its typed reason. Returns the built snapshots so the
/// caller renders the human and JSON reports.
#[allow(clippy::too_many_arguments)]
pub async fn audit_fleet_snapshots(
    registry: &crate::registry::PublisherRegistry,
    snapshot_root: &std::path::Path,
    transport: std::sync::Arc<dyn apex_connectors::transport::Transport>,
    instrumentation: &Instrumentation,
    rule_version: &str,
    now_unix_ms: i64,
) -> Result<FleetAuditReport, String> {
    let mut publishers = Vec::new();
    let mut failures = Vec::new();
    // Every platform of a publisher reads the same public app-ads.txt. One
    // per-run cache means one fetch per distinct developer site, not one per
    // platform; the 404/410 verdict is cached too.
    let app_ads_cache = std::sync::Arc::new(apex_connectors::app_ads::AppAdsCache::new());
    let transport = app_ads_cache.wrap(transport);
    for registered in registry.runnable() {
        let account = crate::audit_job::SnapshotAccount::from(registered);
        // The instrumentation handle is built before the publisher is known;
        // stamping a clone here makes every audit event tenant-scoped.
        let events = instrumentation.events.as_ref().map(|events| {
            std::sync::Arc::new(
                (**events)
                    .clone()
                    .with_publisher(registered.publisher.clone()),
            )
        });
        let mut snapshots = Vec::new();
        for wire in registered.snapshot_platforms() {
            let Some(credentials) = registered.vaulted_platforms.get(&wire) else {
                continue;
            };
            let path = crate::registry::snapshot_path(snapshot_root, &registered.publisher, &wire);
            let started = std::time::Instant::now();
            match crate::audit_job::audited_platform_snapshot(
                &wire,
                credentials,
                &account,
                std::sync::Arc::clone(&transport),
                events.as_deref(),
                rule_version,
                now_unix_ms,
            )
            .await
            {
                Ok(snapshot) => {
                    let report = apex_audit::run(&snapshot);
                    let duration_ms = started.elapsed().as_millis() as u64;
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
                    let (eligible, expected, missing) = audit_coverage(&snapshot);
                    // Atomic temp+rename beside the reads, exactly like the
                    // legacy single-file write. A write failure is this
                    // platform's typed failure, not a reason to abandon the
                    // rest of the fleet.
                    if let Err(error) = crate::write_snapshot(&path, &snapshot) {
                        instrumentation
                            .telemetry
                            .audit_run(rule_version, apex_metrics::vocab::Outcome::Failed);
                        failures.push(AuditFailure {
                            publisher: registered.publisher.clone(),
                            platform: wire.clone(),
                            reason: format!("snapshot write: {error}"),
                        });
                        continue;
                    }
                    instrumentation
                        .telemetry
                        .audit_run(rule_version, apex_metrics::vocab::Outcome::Completed);
                    if let Some(events) = &events {
                        events
                            .audit_completed(
                                &report.snapshot_digest,
                                rule_version,
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
                    snapshots.push(AuditSnapshotReport {
                        platform: wire.clone(),
                        platform_id: snapshot.platform,
                        path: path.display().to_string(),
                        ad_units: snapshot.ad_units.len(),
                        digest: report.snapshot_digest.clone(),
                        findings: report.findings.len(),
                        critical,
                        warning,
                        info,
                        eligible_rule_groups: eligible,
                        expected_rule_groups: expected,
                        missing_inputs: missing,
                    });
                }
                Err(error) => {
                    // The typed refresh error is recorded and the pass moves
                    // on: one flaky platform must not starve the rest of the
                    // fleet of its snapshots.
                    instrumentation
                        .telemetry
                        .audit_run(rule_version, apex_metrics::vocab::Outcome::Failed);
                    failures.push(AuditFailure {
                        publisher: registered.publisher.clone(),
                        platform: wire.clone(),
                        reason: error.to_string(),
                    });
                }
            }
        }
        if let Some(notify) = &instrumentation.notify {
            notify
                .audit_ready(
                    &registered.publisher,
                    &crate::calendar::format_day(now_unix_ms.div_euclid(86_400_000)),
                    now_unix_ms,
                )
                .await;
        }
        // The per-publisher handle shares the process-wide batching buffer;
        // flush it before the next publisher (and before the process exits).
        if let Some(events) = &events {
            events.flush().await;
        }
        publishers.push(AuditPublisherReport {
            publisher: registered.publisher.clone(),
            snapshots,
        });
    }
    let skipped = registry
        .skipped()
        .map(|skip| FleetSkip {
            publisher: skip.publisher.clone(),
            reason: skip.reason.to_string(),
        })
        .collect();
    Ok(FleetAuditReport {
        publishers,
        skipped,
        failures,
    })
}

/// Render one fleet audit into the human and versioned JSON reports.
pub fn render_fleet_audit(report: &FleetAuditReport, now_unix_ms: i64) -> DispatchOutcome {
    let reference = run_reference(now_unix_ms);
    let mut lines = vec![format!(
        "Fleet: {} publisher(s) / {} skipped / {} snapshot(s)",
        report.publishers.len(),
        report.skipped.len(),
        report
            .publishers
            .iter()
            .map(|publisher| publisher.snapshots.len())
            .sum::<usize>()
    )];
    let mut publisher_blocks = Vec::new();
    for publisher in &report.publishers {
        lines.push(format!("Publisher: {}", publisher.publisher));
        let mut snapshots_json = Vec::new();
        let mut findings = 0usize;
        let mut critical = 0usize;
        let mut warning = 0usize;
        let mut info = 0usize;
        let mut missing_inputs: Vec<String> = Vec::new();
        for snapshot in &publisher.snapshots {
            lines.push(format!(
                "Platform: {} / {} ad unit(s)",
                snapshot.platform, snapshot.ad_units
            ));
            lines.push(format!(
                "Audit: {} finding(s) ({} critical / {} warning / {} info)",
                snapshot.findings, snapshot.critical, snapshot.warning, snapshot.info
            ));
            if !snapshot.missing_inputs.is_empty() {
                lines.push(format!(
                    "Audit coverage: {}/{} rule group(s); missing {:?}",
                    snapshot.eligible_rule_groups,
                    snapshot.expected_rule_groups,
                    snapshot.missing_inputs
                ));
            }
            lines.push(format!(
                "Snapshot: wrote {} ad units to {}",
                snapshot.ad_units, snapshot.path
            ));
            findings += snapshot.findings;
            critical += snapshot.critical;
            warning += snapshot.warning;
            info += snapshot.info;
            for input in &snapshot.missing_inputs {
                if !missing_inputs.contains(input) {
                    missing_inputs.push(input.clone());
                }
            }
            snapshots_json.push(serde_json::json!({
                "platform": snapshot.platform,
                "path": snapshot.path,
                "ad_units": snapshot.ad_units,
                "digest": snapshot.digest,
                "findings": snapshot.findings,
                "critical": snapshot.critical,
                "warning": snapshot.warning,
                "info": snapshot.info,
                "eligible_rule_groups": snapshot.eligible_rule_groups,
                "expected_rule_groups": snapshot.expected_rule_groups,
                "missing_inputs": snapshot.missing_inputs,
            }));
        }
        missing_inputs.sort();
        publisher_blocks.push(serde_json::json!({
            "publisher": publisher.publisher,
            "snapshots": snapshots_json,
            "audit": {
                "findings": findings,
                "critical": critical,
                "warning": warning,
                "info": info,
                "missing_inputs": missing_inputs,
            },
        }));
    }
    for skip in &report.skipped {
        lines.push(format!("Skipped: {}: {}", skip.publisher, skip.reason));
    }
    for failure in &report.failures {
        lines.push(format!(
            "Failed: {}: {}: {}",
            failure.publisher, failure.platform, failure.reason
        ));
    }
    lines.push(format!("Run reference: {reference}"));

    // The fleet's mode is its snapshots' platform when every publisher has
    // exactly one and they all agree; a mixed fleet reports `fleet`.
    let mut platform_ids: Vec<apex_core::PlatformId> = report
        .publishers
        .iter()
        .flat_map(|publisher| publisher.snapshots.iter().map(|s| s.platform_id))
        .collect();
    platform_ids.sort_by_key(|platform| mode_of(*platform));
    platform_ids.dedup();
    let single_each = report
        .publishers
        .iter()
        .all(|publisher| publisher.snapshots.len() == 1);
    let mode = match platform_ids.as_slice() {
        [only] if single_each => mode_of(*only).to_string(),
        _ => "fleet".to_string(),
    };
    let skipped: Vec<serde_json::Value> = report
        .skipped
        .iter()
        .map(|skip| serde_json::json!({ "publisher": skip.publisher, "reason": skip.reason }))
        .collect();
    let failures: Vec<serde_json::Value> = report
        .failures
        .iter()
        .map(|failure| {
            serde_json::json!({
                "publisher": failure.publisher,
                "platform": failure.platform,
                "reason": failure.reason,
            })
        })
        .collect();
    let mut json = serde_json::Map::new();
    json.insert(
        "schema_version".into(),
        serde_json::Value::from(REPORT_SCHEMA_VERSION),
    );
    json.insert("command".into(), serde_json::Value::from("audit"));
    json.insert("mode".into(), serde_json::Value::from(mode));
    json.insert("publisher".into(), serde_json::Value::Null);
    json.insert("run_reference".into(), serde_json::Value::from(reference));
    json.insert(
        "publishers".into(),
        serde_json::Value::Array(publisher_blocks),
    );
    json.insert("skipped".into(), serde_json::Value::Array(skipped));
    json.insert("failures".into(), serde_json::Value::Array(failures));
    DispatchOutcome {
        human: lines.join("\n"),
        json: serde_json::Value::Object(json).to_string(),
        failed: !report.failures.is_empty(),
    }
}

/// The fleet-level verified payment pass: one cursor over the collection
/// file, events (and retries) grouped by the publisher named in their
/// reference (`apex-fee:<publisher>:...`, `INV-<month>-<publisher>`), each
/// group processed with its own publisher so the customer messages stay
/// scoped. The cursor advances once, after every group processed.
async fn fleet_payment_pass(
    fleet: &Fleet,
    payments: &FleetPayments,
    now_unix_ms: i64,
) -> Result<
    (
        crate::payment_events::PaymentsReport,
        Vec<String>,
        serde_json::Value,
    ),
    String,
> {
    let events_path = &payments.events_path;
    let cursor_path = payments
        .cursor_path
        .clone()
        .unwrap_or_else(|| crate::payment_events::default_cursor_path(events_path));
    let cursor = crate::payment_events::read_payment_cursor(&cursor_path)
        .map_err(|error| error.to_string())?;
    let (observed, consumed) =
        crate::payment_events::read_new_collection_events_from(events_path, cursor)
            .map_err(|error| error.to_string())?;
    let historical_received = crate::payment_events::read_received_totals(events_path)
        .map_err(|error| error.to_string())?;
    let payment_ledger = match &payments.ledger_path {
        Some(path) => {
            crate::payment_events::read_payment_ledger(path).map_err(|error| error.to_string())?
        }
        None => Vec::new(),
    };
    let retries = match &payments.retries_path {
        Some(path) => crate::payment_events::read_scheduled_retries(path)
            .map_err(|error| error.to_string())?,
        None => Vec::new(),
    };
    let mut by_publisher: std::collections::BTreeMap<
        String,
        (
            Vec<apex_billing::CollectionEvent>,
            Vec<crate::payment_events::ScheduledRetry>,
        ),
    > = std::collections::BTreeMap::new();
    for event in observed {
        let publisher =
            crate::payment_events::publisher_of_reference(&event.reference).unwrap_or_default();
        by_publisher.entry(publisher).or_default().0.push(event);
    }
    for retry in retries {
        let publisher = crate::payment_events::publisher_of_reference(&retry.invoice_reference)
            .unwrap_or_default();
        by_publisher.entry(publisher).or_default().1.push(retry);
    }
    let mut total = crate::payment_events::PaymentsReport::default();
    for (publisher, (events, retries)) in by_publisher {
        let report = crate::payment_events::process_payment_events_with_history(
            &historical_received,
            &events,
            &payment_ledger,
            &retries,
            fleet.notify.as_deref(),
            fleet.events.as_deref(),
            Some(&fleet.telemetry),
            &publisher,
            now_unix_ms,
        )
        .await
        .map_err(|error| error.to_string())?;
        total.received += report.received;
        total.failed += report.failed;
        total.retries += report.retries;
        total.reconciled += report.reconciled;
        total.refunded += report.refunded;
        total.disputed += report.disputed;
        total.skipped += report.skipped;
    }
    if consumed != cursor {
        crate::payment_events::write_payment_cursor(&cursor_path, consumed)
            .map_err(|error| error.to_string())?;
    }
    let mut lines = Vec::new();
    if total.any() {
        let mut line = format!(
            "Payment events: {} received / {} failed",
            total.received, total.failed
        );
        if total.reconciled > 0 {
            line.push_str(&format!(" / {} reconciled", total.reconciled));
        }
        if total.refunded > 0 {
            line.push_str(&format!(" / {} refunded", total.refunded));
        }
        if total.disputed > 0 {
            line.push_str(&format!(" / {} disputed", total.disputed));
        }
        lines.push(line);
    }
    let json = serde_json::json!({
        "received": total.received,
        "failed": total.failed,
        "retries": total.retries,
        "reconciled": total.reconciled,
        "refunded": total.refunded,
        "disputed": total.disputed,
        "skipped": total.skipped,
    });
    Ok((total, lines, json))
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
        Ok(Some(events)) => Some(std::sync::Arc::new(events.with_batching())),
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

/// The bounded JSON mode label of a platform's reports.
fn mode_of(platform: apex_core::PlatformId) -> &'static str {
    match platform {
        apex_core::PlatformId::AppLovinMax => "max",
        apex_core::PlatformId::LevelPlay => "levelplay",
        apex_core::PlatformId::Admob => "admob",
        apex_core::PlatformId::Billing => "billing",
    }
}

/// The human mode label of a platform's reports.
fn mode_label(platform: apex_core::PlatformId) -> &'static str {
    match platform {
        apex_core::PlatformId::AppLovinMax => "MAX",
        apex_core::PlatformId::LevelPlay => "LevelPlay",
        apex_core::PlatformId::Admob => "AdMob",
        apex_core::PlatformId::Billing => "Billing",
    }
}

fn scope_line(platform: apex_core::PlatformId, publisher: &str) -> String {
    if publisher.trim().is_empty() {
        format!("Mode: {} / Publisher: not configured", mode_label(platform))
    } else {
        format!("Mode: {} / Publisher: {}", mode_label(platform), publisher)
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

/// How long a held pass lock is trusted before it is treated as a crashed
/// predecessor and stolen: longer than any real settle+invoice pass.
const RUN_LOCK_STALE_MS: i64 = 6 * 60 * 60 * 1_000;

/// One inter-process pass lock for one ledger file. `Ledger::append` is
/// atomic per record but a settle pass is read–decide–append across many
/// records: two processes on the same ledger would both decide from the same
/// tail and both append. `create_new` makes holding the lock one atomic step,
/// a timestamped body lets a crashed holder be stolen only after it is
/// provably stale, and drop releases it.
#[derive(Debug)]
pub struct RunLock {
    path: std::path::PathBuf,
}

impl RunLock {
    /// Hold the ledger's pass lock, or refuse while another live process
    /// holds it. A lock older than [`RUN_LOCK_STALE_MS`] is a crashed
    /// predecessor's and is replaced.
    pub fn acquire(ledger_path: &std::path::Path) -> Result<Self, String> {
        let path = ledger_path.with_extension("jsonl.lock");
        for attempt in 0..2 {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    use std::io::Write as _;
                    let body = format!(
                        "{} {}\n",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|elapsed| elapsed.as_millis() as i64)
                            .unwrap_or_default()
                    );
                    let _ = file.write_all(body.as_bytes());
                    let _ = file.sync_all();
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if attempt == 0 && Self::is_stale(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    let holder = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|raw| raw.split_whitespace().next().map(str::to_string))
                        .unwrap_or_else(|| "an unknown process".to_string());
                    return Err(format!(
                        "another pass for this ledger is already running (lock {}: held by {holder})",
                        path.display()
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "the pass lock {} could not be created: {error}",
                        path.display()
                    ));
                }
            }
        }
        Err(format!(
            "the pass lock {} could not be acquired",
            path.display()
        ))
    }

    /// Whether the lock body (or, unparseably, its mtime) is older than the
    /// stale window. An unreadable lock fails closed: it is never stolen on a
    /// guess.
    fn is_stale(path: &std::path::Path) -> bool {
        let age = std::fs::read_to_string(path).ok().and_then(|raw| {
            raw.split_whitespace()
                .nth(1)
                .and_then(|stamp| stamp.parse::<i64>().ok())
                .and_then(|started| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|now| now.as_millis() as i64 - started)
                })
        });
        match age {
            Some(age) => age > RUN_LOCK_STALE_MS,
            None => std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|mtime| {
                    let started = mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|since| since.as_millis() as i64)?;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|since| since.as_millis() as i64)?;
                    Some(now - started)
                })
                .is_some_and(|age| age > RUN_LOCK_STALE_MS),
        }
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
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
    dispatch_configured(command, args, runner, ledger, now_unix_ms, true, true).await
}

/// [`dispatch`] with explicit prelude/postlude control. The fleet driver
/// replays the shared analytics spool once before its concurrent publisher
/// passes and flushes the shared emitter after them, so each pass runs with
/// `replay = false, flush = false`; the singleton path keeps both.
async fn dispatch_configured(
    command: &str,
    args: &[String],
    runner: &crate::runner::Runner,
    ledger: &mut dyn apex_proof::LedgerLog,
    now_unix_ms: i64,
    replay: bool,
    flush: bool,
) -> Result<DispatchOutcome, String> {
    use apex_metrics::vocab::Outcome;

    // The spool from earlier failed runs is re-delivered before the command
    // starts; a re-delivery is logged, never silent.
    if replay {
        replay_spool(runner.events().map(AsRef::as_ref)).await;
    }
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
                if flush {
                    events.flush().await;
                }
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
        if flush {
            events.flush().await;
        }
    }

    let publisher = runner.publisher_id().to_string();
    let platform = runner.platform_ops().platform();
    let mut lines = vec![scope_line(platform, &publisher)];
    lines.extend(report.lines.clone());
    lines.push(ledger_line(appended, check.as_ref()));
    lines.push(format!("Run reference: {}", run_reference(now_unix_ms)));

    let mut json = serde_json::Map::new();
    json.insert(
        "schema_version".into(),
        serde_json::Value::from(REPORT_SCHEMA_VERSION),
    );
    json.insert("command".into(), serde_json::Value::from(report.command));
    json.insert("mode".into(), serde_json::Value::from(mode_of(platform)));
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
            // One snapshot per platform this publisher owns. The fleet driver
            // hands each publisher only its own files; the legacy single-file
            // deployment names one `--snapshot` path (or `APEX_RUNNER_SNAPSHOT`).
            let mut snapshot_paths: Vec<std::path::PathBuf> = flags(args, "--snapshot")
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect();
            if snapshot_paths.is_empty() {
                if let Some(path) =
                    env_nonempty(&|name| std::env::var(name).ok(), "APEX_RUNNER_SNAPSHOT")
                {
                    snapshot_paths.push(std::path::PathBuf::from(path));
                }
            }
            if snapshot_paths.is_empty() {
                return Err("cycle requires --snapshot (or APEX_RUNNER_SNAPSHOT)".to_string());
            }
            let report = crate::cycle::run(runner, ledger, &snapshot_paths, now_unix_ms)
                .await
                .map_err(|e| format!("cycle: {e}"))?;
            let mut lines = vec![format!(
                "Cycle: proposed {} applied {} awaiting approval {}",
                report.proposed, report.applied, report.awaiting_approval
            )];
            // Proposals held by the adapter's verified write set: the typed
            // awaiting state is reported verbatim, identical reasons folded
            // with their count (the JSON keeps one entry per proposal); the
            // pass itself continues.
            let mut held: Vec<(&str, u32)> = Vec::new();
            for reason in &report.awaiting_verification {
                match held.iter_mut().find(|(seen, _)| *seen == reason) {
                    Some((_, count)) => *count += 1,
                    None => held.push((reason.as_str(), 1)),
                }
            }
            for (reason, count) in held {
                if count > 1 {
                    lines.push(format!("Skipped: {reason} ({count} proposals)"));
                } else {
                    lines.push(format!("Skipped: {reason}"));
                }
            }
            if report.paused > 0 {
                lines.push(format!(
                    "Paused: {} platform cycle(s) skipped by fleet scheduling; reads continue",
                    report.paused
                ));
                for reason in &report.pause_reasons {
                    lines.push(format!("Paused: {reason}"));
                }
            }
            let mut cycle = serde_json::json!({
                "proposed": report.proposed,
                "applied": report.applied,
                "awaiting_approval": report.awaiting_approval,
                "paused": report.paused,
            });
            if !report.awaiting_verification.is_empty() {
                cycle["awaiting_verification"] =
                    serde_json::json!(report.awaiting_verification.clone());
            }
            if !report.pause_reasons.is_empty() {
                cycle["pause_reasons"] = serde_json::json!(report.pause_reasons.clone());
            }
            Ok(CommandReport {
                command: "cycle",
                job_type: apex_metrics::vocab::JobType::Cycle,
                lines,
                json: [("cycle".to_string(), cycle)].into_iter().collect(),
                failed: false,
            })
        }
        "monthly" => {
            let (month, start, days) = match flag(args, "--month") {
                Some(month) => {
                    let start =
                        flag(args, "--start").ok_or("monthly with --month requires --start")?;
                    let days = month_window_days(&month, &start)?;
                    if let Some(raw) = flag(args, "--days") {
                        let explicit: u32 = raw.parse().map_err(|_| "--days must be a number")?;
                        if explicit != days {
                            return Err(format!(
                                "--days {explicit} disagrees with the calendar window {month} ({days} days)"
                            ));
                        }
                    }
                    (month, start, days)
                }
                None => previous_month(now_unix_ms)?,
            };
            let publisher = runner.publisher_id().to_string();
            let report = runner
                .run_monthly(ledger, &publisher, &month, &start, days, now_unix_ms)
                .await
                .map_err(|e| format!("monthly: {e}"))?;
            let mut lines = vec![format!(
                "Billing: {} fee(s) for {}",
                report.fees.len(),
                report.month
            )];
            let mut billing = serde_json::json!({
                "month": report.month,
                "fees": report.fees.len(),
                "invoice_reference": report.invoice_reference,
                "invoice_gross_micros": report.invoice_total_micros,
                "payment_links": report.payment_links.len(),
                "card_charges": report.card_charges.len(),
                "collected": report.collected,
            });
            for ineligible in &report.fee_ineligible {
                lines.push(format!(
                    "Fee ineligible: {}: {}",
                    ineligible.platform.as_str(),
                    ineligible.reason
                ));
            }
            if !report.fee_ineligible.is_empty() {
                billing["fee_ineligible"] = serde_json::json!(report
                    .fee_ineligible
                    .iter()
                    .map(|ineligible| serde_json::json!({
                        "platform": ineligible.platform.as_str(),
                        "reason": ineligible.reason,
                    }))
                    .collect::<Vec<_>>());
            }
            match &report.invoice_refusal {
                Some(reason) => {
                    // A refused invoice is a hold, not a charge: the typed
                    // reason stands where the invoice line would be, and no
                    // half-empty document is printed.
                    lines.push(format!("Invoice: refused ({reason})"));
                    billing["invoice_refusal"] = serde_json::Value::from(reason.clone());
                }
                None => {
                    lines.push(format!(
                        "Invoice: {} gross {}",
                        report.invoice_reference,
                        usd(report.invoice_total_micros)
                    ));
                    if let Some(rendered) = &report.rendered_invoice {
                        // The rendered VAT document is the invoice output:
                        // it prints in full, register line included.
                        lines.push(rendered.trim_end().to_string());
                    }
                }
            }
            if let Some(number) = report.invoice_number {
                billing["invoice_number"] = serde_json::Value::from(number);
            }
            if let Some(rendered) = &report.rendered_invoice {
                billing["rendered_invoice"] = serde_json::Value::from(rendered.clone());
            }
            lines.push(if report.collected {
                // Both collection channels are stated: every billed line
                // landed on a card charge or a link, and a link can carry a
                // typed fallback reason from a refused stored-method charge.
                format!(
                    "Collection: {} card charge(s) / {} payment link(s) created",
                    report.card_charges.len(),
                    report.payment_links.len()
                )
            } else {
                "Collection: not configured".to_string()
            });
            Ok(CommandReport {
                command: "monthly",
                job_type: apex_metrics::vocab::JobType::Monthly,
                lines,
                json: [("billing".to_string(), billing)].into_iter().collect(),
                failed: false,
            })
        }
        "recheck" => {
            let due = runner
                .due_rechecks(ledger, now_unix_ms)
                .map_err(|error| format!("recheck: {error}"))?;
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
                instance_id: String::new(),
                ad_unit_id: unit,
                metric: apex_core::ExperimentMetric::Revenue,
                variants: 2,
                change_description: "A/A calibration".into(),
                test_group_allocation: 50,
            };
            let id = runner
                .start_aa(ledger, runner.platform_ops().platform(), &spec, now_unix_ms)
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
            // Align the A/A read to the day the run started, exactly like the
            // production settle path; pre-start days are platform history. The
            // pre-registered name keys the durable evidence because the
            // platform id may be reused across runs.
            let index = crate::ledger_index::LedgerIndex::build(ledger);
            let start_day = index.started_days().get(&experiment).cloned();
            let instance = index
                .started_experiment_name(&experiment)
                .map(str::to_string);
            let conclusion = runner
                .aa_conclusion_from(
                    &experiment,
                    split,
                    start_day.as_deref(),
                    instance.as_deref(),
                )
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
    use crate::tests::DelayedOps;

    #[test]
    fn secondary_adapters_cover_every_platform_except_the_primary() {
        let credentials = |name: &str| crate::registry::VaultedCredentials {
            api_key: name.to_string(),
            account_id: "acct".to_string(),
            base_url: "https://example.invalid".to_string(),
            report_key: None,
            reporting_base_url: None,
            refresh_token: None,
        };
        let mut vaulted = std::collections::BTreeMap::new();
        vaulted.insert("applovin_max".to_string(), credentials("max"));
        vaulted.insert("levelplay".to_string(), credentials("lp"));
        vaulted.insert("admob".to_string(), credentials("admob"));

        assert_eq!(
            secondary_wires("applovin_max", &vaulted),
            vec!["admob", "levelplay"],
            "a MAX publisher still gets the AdMob and LevelPlay adapters"
        );
        assert_eq!(
            secondary_wires("levelplay", &vaulted),
            vec!["admob", "applovin_max"],
            "the primary platform never gets a duplicate adapter"
        );
        assert_eq!(
            wire_platform("applovin_max"),
            Some(apex_core::PlatformId::AppLovinMax)
        );
        assert_eq!(
            wire_platform("levelplay"),
            Some(apex_core::PlatformId::LevelPlay)
        );
        assert_eq!(wire_platform("admob"), Some(apex_core::PlatformId::Admob));
        assert_eq!(wire_platform("unknown"), None);
    }

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

    #[tokio::test]
    async fn one_ledger_is_passed_by_one_process_at_a_time() {
        let dir = std::env::temp_dir().join(format!("apex-runlock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = dir.join("pub1.jsonl");
        let held = RunLock::acquire(&ledger).expect("first hold");
        let second = RunLock::acquire(&ledger).expect_err("a second pass refuses");
        assert!(second.contains("already running"), "{second}");
        drop(held);
        let third = RunLock::acquire(&ledger).expect("released");
        drop(third);

        // A crashed predecessor's lock is stolen only past the stale window.
        std::fs::write(
            dir.join("pub1.jsonl.lock"),
            format!(
                "999999 {}\n",
                (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64)
                    - RUN_LOCK_STALE_MS
                    - 1_000
            ),
        )
        .unwrap();
        let stolen = RunLock::acquire(&ledger).expect("stale lock is stolen");
        drop(stolen);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn month_window_days_follows_the_calendar_not_a_30_day_default() {
        assert_eq!(month_window_days("2026-03", "2026-03-01").unwrap(), 31);
        assert_eq!(month_window_days("2026-02", "2026-02-01").unwrap(), 28);
        assert_eq!(month_window_days("2024-02", "2024-02-01").unwrap(), 29);
        assert_eq!(month_window_days("2026-12", "2026-12-01").unwrap(), 31);
        assert_eq!(month_window_days("2027-01", "2027-01-01").unwrap(), 31);
        // A start outside the month (or a start that is not day one) refuses
        // instead of inventing a window.
        assert!(month_window_days("2026-03", "2026-03-02").is_err());
        assert!(month_window_days("2026-03", "2026-04-01").is_err());
        assert!(month_window_days("bad", "bad-01").is_err());
    }

    #[test]
    fn parse_year_month_reads_the_prefix() {
        assert_eq!(parse_year_month("2026-10-01"), Some((2026, 10)));
        assert_eq!(parse_year_month("bad"), None);
    }

    #[tokio::test]
    async fn build_runner_honors_the_environment() {
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
            std::sync::Arc::new(NullOps),
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
            std::sync::Arc::new(NullOps),
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
            std::sync::Arc::new(NullOps),
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
        let no_ops = NullOps;
        assert!(no_ops.scope_revenue("u", "a", "b").await.is_err());
        assert!(no_ops.experiment_days("x").await.is_err());
        assert!(no_ops.current_settings("u").await.is_err());
        assert!(no_ops
            .start_null_experiment(&apex_core::ExperimentSpec {
                name: "e".into(),
                instance_id: String::new(),
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
            "APEX_MOLLIE_API_KEY" => Some("k".to_string()),
            "APEX_MOLLIE_WEBHOOK_URL" => Some("https://apex.test/hook".to_string()),
            "APEX_MOLLIE_REDIRECT_URL" => Some("https://apex.test/return".to_string()),
            _ => None,
        };
        let runner = build_runner(
            env,
            std::sync::Arc::new(NullOps),
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("runner");
        // A partial credential set refuses the runner: a client without a
        // webhook URL would create links that never receive a payment.
        let partial = |name: &str| match name {
            "APEX_MOLLIE_API_KEY" => Some("k".to_string()),
            _ => None,
        };
        let error = build_runner(
            partial,
            std::sync::Arc::new(NullOps),
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect_err("partial credentials refuse the runner");
        assert!(error.contains("APEX_MOLLIE_WEBHOOK_URL"), "{error}");
        assert!(format!("{runner:?}").contains("Runner"));
        assert!(runner.telemetry().is_some());
        assert!(runner.verifier().is_none(), "no ledger path in this env");
    }

    #[test]
    fn the_effective_policy_file_wins_over_the_inline_policy_and_fails_closed() {
        let dir = std::env::temp_dir().join(format!("apex-cli-policy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("effective.json");
        let mut effective = crate::model::AutopilotPolicy::conservative();
        effective.max_actions_per_day = 1;
        effective.max_step_bp = 500;
        std::fs::write(&path, serde_json::to_vec(&effective).unwrap()).unwrap();
        let env = |name: &str| match name {
            EFFECTIVE_POLICY_ENV => Some(path.display().to_string()),
            "APEX_RUNNER_POLICY" => {
                Some(serde_json::to_string(&crate::model::AutopilotPolicy::conservative()).unwrap())
            }
            _ => None,
        };
        let policy = policy_from_env(&env).expect("effective policy");
        assert_eq!(policy.max_actions_per_day, 1);
        assert_eq!(policy.max_step_bp, 500);

        // A configured file that is missing, malformed or out of bounds is a
        // refusal; it never falls through to the inline policy or defaults.
        let missing = |name: &str| match name {
            EFFECTIVE_POLICY_ENV => Some("/nope/effective.json".to_string()),
            "APEX_RUNNER_POLICY" => {
                Some(serde_json::to_string(&crate::model::AutopilotPolicy::conservative()).unwrap())
            }
            _ => None,
        };
        assert!(policy_from_env(&missing).is_err());
        std::fs::write(&path, "{not json}").unwrap();
        assert!(policy_from_env(&env).is_err());
        let mut invalid = crate::model::AutopilotPolicy::conservative();
        invalid.window_days = 9;
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(policy_from_env(&env).is_err());

        // No effective file configured: the inline policy is validated too.
        let inline_invalid = |name: &str| match name {
            "APEX_RUNNER_POLICY" => Some(serde_json::to_string(&invalid).unwrap()),
            _ => None,
        };
        assert!(policy_from_env(&inline_invalid).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The fixed run time the golden outputs pin: 2026-10-15 08:10 UTC.
    fn golden_now() -> i64 {
        crate::calendar::day_number("2026-10-15").unwrap() * 86_400_000 + (8 * 60 + 10) * 60_000
    }

    /// The seller billing profile the golden monthly runs load; with zero
    /// fees no invoice is issued, so the committed output shape is unchanged.
    fn golden_billing_profile() -> apex_billing::BillingProfile {
        apex_billing::BillingProfile {
            legal_name: "Apex Media OÜ".into(),
            address_lines: vec!["Sepapaja tn 6".into()],
            country: "EE".into(),
            vat_number: "EE123456789".into(),
            vat_rate_bp: 2_000,
        }
    }

    /// A per-process register path for the golden runners (no golden run has
    /// fees, so no line is ever written).
    fn golden_register_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("apex-cli-register-{}.jsonl", std::process::id()))
    }

    fn golden_runner() -> crate::runner::Runner {
        let ops = std::sync::Arc::new(crate::ops::RecordingOps::new());
        crate::runner::Runner::new(
            ops,
            crate::model::AutopilotPolicy::conservative(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_publisher("northstar")
        .with_billing_profile(golden_billing_profile())
        .with_register_path(golden_register_path())
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
            std::sync::Arc::new(NullOps),
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
                    "card_charges": 0,
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
    async fn a_legacy_monthly_with_the_root_env_records_no_monitoring_plan() {
        let seller = fleet_dir("legacy-seller");
        std::fs::create_dir_all(&seller).unwrap();
        let seller_path = seller.join("seller.json");
        std::fs::write(
            &seller_path,
            serde_json::json!({
                "legalName": "Apex Media OÜ",
                "addressLines": ["Sepapaja tn 6", "15551 Tallinn"],
                "country": "EE",
                "vatNumber": "EE123456789",
                "vatRateBp": 2000
            })
            .to_string(),
        )
        .unwrap();
        let dir = fleet_dir("legacy-free");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("northstar.billing.json"),
            serde_json::json!({
                "legalName": "Northstar OÜ",
                "addressLines": ["Telliskivi 1"],
                "country": "EE"
            })
            .to_string(),
        )
        .unwrap();
        let root = dir.clone();
        let seller_env = seller_path.clone();
        let env = move |name: &str| match name {
            "APEX_RUNNER_PUBLISHER" => Some("northstar".to_string()),
            "APEX_CONNECTIONS_ROOT" => Some(root.display().to_string()),
            "APEX_BILLING_PROFILE" => Some(seller_env.display().to_string()),
            _ => None,
        };
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");
        let runner = build_runner(
            env,
            std::sync::Arc::new(NullOps),
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("runner")
        .with_billing_profile(golden_billing_profile())
        .with_register_path(dir.join("invoices.jsonl"));
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[5u8; 32]));
        let outcome = dispatch("monthly", &[], &runner, &mut ledger, golden_now())
            .await
            .expect("monthly");
        assert!(!outcome.failed, "{outcome:?}");
        assert!(
            !dir.join("northstar.monitoring.json").exists(),
            "free monitoring never records a paid plan"
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["billing"]["fees"], 0);
        assert!(json["billing"]["rendered_invoice"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&seller);
    }

    #[tokio::test]
    async fn a_legacy_monthly_without_the_root_env_has_no_monitoring_plan_or_skip() {
        // Legacy mode with no registry root: there is no paid plan to skip.
        // Monitoring is free, so the pass records no monitoring line or
        // state at all and reports no invoice when nothing is billable.
        let dir = fleet_dir("legacy-no-root");
        let env = |name: &str| match name {
            "APEX_RUNNER_PUBLISHER" => Some("northstar".to_string()),
            _ => None,
        };
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");
        let runner = build_runner(
            env,
            std::sync::Arc::new(NullOps),
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("runner")
        .with_billing_profile(golden_billing_profile())
        .with_register_path(dir.join("invoices.jsonl"));
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[5u8; 32]));
        let outcome = dispatch("monthly", &[], &runner, &mut ledger, golden_now())
            .await
            .expect("the monthly pass continues without a state root");
        assert!(!outcome.failed, "{outcome:?}");
        assert!(!outcome.human.contains("Monitoring:"));
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["billing"]["fees"], 0);
        assert!(json["billing"]["rendered_invoice"].is_null());
        assert!(
            !dir.join("northstar.monitoring.json").exists(),
            "no plan state is fabricated without a root"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // Fleet mode: the registry, the per-publisher ledgers and the composed
    // multi-publisher reports.
    // ------------------------------------------------------------------

    const FLEET_KEY_A: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const FLEET_VAULT_KEY: &str =
        "1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100";

    fn fleet_dir(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("apex-cli-fleet-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_vaulted_publisher(
        root: &std::path::Path,
        publisher: &str,
        api_key: &str,
        state: &str,
    ) {
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault key");
        let account = serde_json::json!({
            "developer_site": "studio.example.com",
            "expected_networks": [],
            "ad_unit_ids": [],
            "bidder_accounts": [],
            "platforms": {
                "applovin_max": {
                    "api_key": api_key,
                    "account_id": "acct-1",
                    "base_url": "https://api.example.test",
                    "report_key": "report-key",
                    "reporting_base_url": null,
                    "refresh_token": null
                }
            }
        });
        std::fs::write(
            root.join(format!("{publisher}.blob")),
            vault.encrypt(&account.to_string()),
        )
        .expect("blob");
        let meta = serde_json::json!({
            "platforms": { "applovin_max": { "state": state } }
        });
        std::fs::write(
            root.join(format!("{publisher}.connections.json")),
            meta.to_string(),
        )
        .expect("meta");
    }

    fn test_policy() -> crate::model::AutopilotPolicy {
        crate::model::AutopilotPolicy {
            window_days: 7,
            interim_look: false,
            extension_allowed: false,
            ..crate::model::AutopilotPolicy::conservative()
        }
    }

    fn constant_days(control: i64, test: i64) -> crate::ops::ExperimentDays {
        let arm_day = |day: &str, revenue: i64| apex_lift::window::ArmDay {
            day: day.to_string(),
            revenue,
            users: None,
            second_pull_revenue: Some(revenue),
            second_pull_users: None,
            fill: None,
            second_pull_fill: None,
            first_observed_at_unix_s: Some(1_700_000_000),
            second_observed_at_unix_s: Some(
                1_700_000_000 + apex_lift::window::SECOND_PULL_MIN_SECS,
            ),
            first_source_sha256: Some("a".repeat(64)),
            second_source_sha256: Some("b".repeat(64)),
        };
        crate::ops::ExperimentDays {
            control: (0..10)
                .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), control / 4 * 3))
                .collect(),
            test: (0..10)
                .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), test / 4))
                .collect(),
        }
    }

    fn fleet_ops(experiment: &str) -> std::sync::Arc<crate::ops::RecordingOps> {
        let ops = std::sync::Arc::new(crate::ops::RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok(experiment.to_string())]);
        ops.put_experiment(
            experiment,
            apex_core::ExperimentResult {
                experiment_id: experiment.to_string(),
                state: apex_core::ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days(experiment, constant_days(4_000_000, 4_400_000));
        ops.put_scope_revenue("u1", 1_000_000_000);
        ops
    }

    async fn start_one_change(
        ops: &std::sync::Arc<crate::ops::RecordingOps>,
        ledger: &mut apex_proof::Ledger,
        at: i64,
    ) {
        crate::plan::run_cycle(
            ops.as_ref(),
            ledger,
            &test_policy(),
            &[],
            vec![crate::model::Proposal {
                action: crate::model::Action::SetCountryFloor {
                    ad_unit_id: "u1".into(),
                    country: "ee".into(),
                    floor_ecpm: 2_500_000,
                },
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            at,
        )
        .await
        .expect("cycle");
    }

    async fn keep_one_change(
        ops: &std::sync::Arc<crate::ops::RecordingOps>,
        ledger: &mut apex_proof::Ledger,
        at: i64,
    ) {
        start_one_change(ops, ledger, at).await;
        let runner = crate::runner::Runner::new(
            ops.clone(),
            test_policy(),
            apex_lift::guardrails::FillFormat::Rewarded,
        );
        runner.run_daily(ledger, at + 1).await.expect("daily");
    }

    /// A platform adapter that refuses every call: the configuration tests
    /// only need the runner built, never an operation.
    struct NullOps;

    #[async_trait::async_trait]
    impl crate::ops::PlatformOps for NullOps {
        fn supports_player_feed(&self) -> bool {
            false
        }

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
        async fn deprecate_experiment(&self, _id: &str) -> Result<(), apex_core::PlatformError> {
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

    /// A transport that refuses every call: the fleet construction tests
    /// never reach the network.
    struct NoTransport;

    #[async_trait::async_trait]
    impl apex_connectors::transport::Transport for NoTransport {
        async fn request(
            &self,
            _request: apex_connectors::transport::HttpRequest,
        ) -> Result<
            apex_connectors::transport::HttpResponse,
            apex_connectors::transport::TransportError,
        > {
            Err(apex_connectors::transport::TransportError::Network(
                "none".into(),
            ))
        }
    }

    fn empty_fleet(
        publishers: Vec<FleetPublisher>,
        skipped: Vec<FleetSkip>,
        signing_key: apex_proof::Keypair,
    ) -> Fleet {
        let registry = std::sync::Arc::new(apex_metrics::Registry::new());
        Fleet {
            publishers,
            skipped,
            telemetry: crate::telemetry::RunnerTelemetry::new(registry),
            events: None,
            notify: None,
            projections: None,
            payments: None,
            signing_key,
        }
    }

    /// Vault one publisher whose validated connection belongs to `platform`.
    fn write_vaulted_platform_publisher(
        root: &std::path::Path,
        publisher: &str,
        platform: &str,
        credentials: serde_json::Value,
        state: &str,
    ) {
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault key");
        let account = serde_json::json!({
            "developer_site": "studio.example.com",
            "expected_networks": [],
            "ad_unit_ids": [],
            "bidder_accounts": [],
            "platforms": { platform: credentials }
        });
        std::fs::write(
            root.join(format!("{publisher}.blob")),
            vault.encrypt(&account.to_string()),
        )
        .expect("blob");
        let meta = serde_json::json!({
            "platforms": { platform: { "state": state } }
        });
        std::fs::write(
            root.join(format!("{publisher}.connections.json")),
            meta.to_string(),
        )
        .expect("meta");
    }

    #[tokio::test]
    async fn a_validated_levelplay_connection_builds_the_runner_and_runs_its_passes() {
        let root = fleet_dir("levelplay-runner");
        write_vaulted_platform_publisher(
            &root,
            "studio",
            "levelplay",
            serde_json::json!({
                "api_key": "secret-key",
                "account_id": "app-key-1",
                "base_url": "https://platform.ironsrc.com",
                "refresh_token": "refresh-token"
            }),
            "validated",
        );
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault");
        let registry = crate::registry::PublisherRegistry::load(&root, &vault).expect("registry");
        assert_eq!(
            registry.runnable().count(),
            1,
            "a validated LevelPlay connection runs now"
        );

        let ledger_root = root.join("ledgers");
        std::fs::create_dir_all(&ledger_root).unwrap();
        let signing_key = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");
        let built = build_fleet(
            |name| match name {
                "APEX_RUNNER_WRITABLE" => Some("updateEntryPrice".to_string()),
                _ => None,
            },
            &registry,
            &ledger_root,
            &root,
            &root.join("snapshots"),
            &signing_key,
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("fleet built");
        assert_eq!(built.publishers.len(), 1);
        assert!(built.skipped.is_empty());
        assert!(built.is_singleton());
        let runner = &built.publishers[0].runner;
        assert_eq!(
            runner.platform_ops().platform(),
            apex_core::PlatformId::LevelPlay
        );
        assert!(runner.platform_ops().writable("updateEntryPrice").await);

        // The singleton fleet returns the per-publisher outcome verbatim, so
        // its scope line and mode name LevelPlay.
        let outcome = dispatch_fleet("daily", &[], &built, golden_now() + 2)
            .await
            .expect("fleet daily");
        assert!(!outcome.failed, "{outcome:?}");
        assert!(
            outcome
                .human
                .starts_with("Mode: LevelPlay / Publisher: studio\n"),
            "{}",
            outcome.human
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["mode"], "levelplay");
        assert_eq!(json["publisher"], "studio");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_validated_admob_connection_runs_reporting_only_until_configured() {
        let root = fleet_dir("admob-runner");
        write_vaulted_platform_publisher(
            &root,
            "studio",
            "admob",
            serde_json::json!({
                "api_key": "access-token",
                "account_id": "accounts/pub-1234567890",
                "base_url": "https://admob.googleapis.com"
            }),
            "validated",
        );
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault");
        let registry = crate::registry::PublisherRegistry::load(&root, &vault).expect("registry");
        assert_eq!(registry.runnable().count(), 1);

        let ledger_root = root.join("ledgers");
        std::fs::create_dir_all(&ledger_root).unwrap();
        let signing_key = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");

        // Without the write allow-list the cycle's verify-live gate refuses:
        // the AdMob publisher monitors, it starts no experiments.
        let built = build_fleet(
            |_| None,
            &registry,
            &ledger_root,
            &root,
            &root.join("snapshots"),
            &signing_key,
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("fleet built");
        let runner = &built.publishers[0].runner;
        assert_eq!(
            runner.platform_ops().platform(),
            apex_core::PlatformId::Admob
        );
        assert!(!runner.platform_ops().writable("updateMediationGroup").await);
        let outcome = dispatch_fleet("daily", &[], &built, golden_now() + 2)
            .await
            .expect("fleet daily");
        assert!(!outcome.failed, "{outcome:?}");
        assert!(outcome
            .human
            .starts_with("Mode: AdMob / Publisher: studio\n"));
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["mode"], "admob");
        assert_eq!(json["reporting_pulls"]["completed"], 0);
        assert_eq!(json["platform_operations"]["unresolved"], 0);

        // With the write configured, the experiment lifecycle gate opens.
        let built = build_fleet(
            |name| match name {
                "APEX_RUNNER_WRITABLE" => Some("updateMediationGroup".to_string()),
                _ => None,
            },
            &registry,
            &ledger_root,
            &root,
            &root.join("snapshots"),
            &signing_key,
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("fleet built");
        assert!(
            built.publishers[0]
                .runner
                .platform_ops()
                .writable("updateMediationGroup")
                .await
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn the_monthly_report_surfaces_a_fee_ineligible_kept_change() {
        use apex_core::{ExperimentMetric, PlatformId};
        use apex_lift::billing::ADMOB_NO_MEASUREMENT_REASON;
        use apex_lift::records::LiftRecord;

        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[9u8; 32]));
        ledger
            .append(apex_proof::LedgerRecord::ExperimentStarted {
                seq: 0,
                at_unix_ms: golden_now(),
                platform: PlatformId::Admob,
                experiment_id: "exp_ab".into(),
                spec: apex_core::ExperimentSpec {
                    name: "exp_ab".into(),
                    instance_id: String::new(),
                    ad_unit_id: "u1".into(),
                    metric: ExperimentMetric::Revenue,
                    variants: 2,
                    change_description: "mediation eCPM".into(),
                    test_group_allocation: 25,
                },
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            })
            .expect("started");
        let prereg = LiftRecord::PreRegistration {
            experiment_id: "exp_ab".into(),
            instance_id: String::new(),
            scope: "g1/u1".into(),
            before_value: serde_json::json!(1_000_000),
            after_value: serde_json::json!(1_200_000),
            split: 0.25,
            metric: "revenue_per_user".into(),
            burn_in_days: 3,
            window_days: 14,
            looks: 1,
            extension_allowed: false,
            guardrail_thresholds: serde_json::json!({}),
            code_version_hash: "deadbeef".into(),
            rate_bp: 2_000,
            min_point_lift: 0.01,
        };
        let payload = prereg.canonical_json().expect("json");
        let sha = prereg.sha256_hex().expect("sha");
        ledger
            .append(apex_proof::LedgerRecord::lift_record(
                golden_now(),
                &sha,
                &payload,
            ))
            .expect("prereg");
        let result = LiftRecord::Result {
            experiment_id: "exp_ab".into(),
            instance_id: String::new(),
            daily_log_ratios: vec![0.1],
            mean_log: 0.1,
            lower_90: 0.05,
            lower_99: 0.02,
            bootstrap_lower: 0.01,
            decision: "keep".into(),
            lambda: 0.03,
        };
        let payload = result.canonical_json().expect("json");
        let sha = result.sha256_hex().expect("sha");
        ledger
            .append(apex_proof::LedgerRecord::lift_record(
                golden_now() + 1,
                &sha,
                &payload,
            ))
            .expect("result");

        let runner = golden_runner();
        let outcome = dispatch("monthly", &[], &runner, &mut ledger, golden_now() + 2)
            .await
            .expect("monthly");
        assert!(!outcome.failed, "{outcome:?}");
        assert!(
            outcome.human.contains(&format!(
                "Fee ineligible: admob: {ADMOB_NO_MEASUREMENT_REASON}"
            )),
            "{}",
            outcome.human
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(
            json["billing"]["fee_ineligible"],
            serde_json::json!([{
                "platform": "admob",
                "reason": ADMOB_NO_MEASUREMENT_REASON,
            }])
        );
        assert_eq!(json["billing"]["fees"], 0);
    }

    #[tokio::test]
    async fn a_fleet_daily_run_writes_two_independent_ledger_chains() {
        // Two publishers whose validated connections exist in the vault root;
        // the registry decrypts both key sets.
        let root = fleet_dir("daily-two");
        write_vaulted_publisher(&root, "northstar", "management-key-north", "validated");
        write_vaulted_publisher(&root, "aurora", "management-key-aurora", "validated");
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault");
        let registry = crate::registry::PublisherRegistry::load(&root, &vault).expect("registry");
        assert_eq!(registry.runnable().count(), 2);
        let keys: Vec<(String, String)> = registry
            .runnable()
            .map(|publisher| {
                (
                    publisher.publisher.clone(),
                    publisher.credentials.api_key.clone(),
                )
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                ("aurora".to_string(), "management-key-aurora".to_string()),
                ("northstar".to_string(), "management-key-north".to_string()),
            ],
            "both vaulted key sets decrypted"
        );

        // The per-publisher runners (one kept change each) write through the
        // real fleet dispatcher into their own ledger files.
        let ledger_root = root.join("ledgers");
        std::fs::create_dir_all(&ledger_root).unwrap();
        let signing_key = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let mut publishers = Vec::new();
        for (index, registered) in registry.runnable().enumerate() {
            let experiment = format!("exp-{index}");
            let ops = fleet_ops(&experiment);
            let ledger_path = ledger_root.join(format!("{}.jsonl", registered.publisher));
            {
                let mut ledger =
                    apex_proof::Ledger::open(&ledger_path, signing_key.clone()).unwrap();
                // The experiment is running when the fleet daily pass starts:
                // that pass is what pulls and settles it.
                start_one_change(&ops, &mut ledger, golden_now()).await;
            }
            let runner = crate::runner::Runner::new(
                ops,
                test_policy(),
                apex_lift::guardrails::FillFormat::Rewarded,
            )
            .with_publisher(registered.publisher.clone());
            publishers.push(FleetPublisher {
                publisher: registered.publisher.clone(),
                runner,
                ledger_path,
                snapshot_paths: Vec::new(),
            });
        }
        let fleet = empty_fleet(publishers, Vec::new(), signing_key.clone());
        assert!(!fleet.is_singleton());
        let outcome = dispatch_fleet("daily", &[], &fleet, golden_now() + 2)
            .await
            .expect("fleet daily");
        assert!(!outcome.failed, "{outcome:?}");
        assert!(
            outcome
                .human
                .starts_with("Fleet: 2 publisher(s) / 0 skipped\n"),
            "{}",
            outcome.human
        );
        assert!(outcome.human.contains("Publisher: aurora\n"));
        assert!(outcome.human.contains("Publisher: northstar\n"));
        assert!(outcome.human.ends_with(&format!(
            "Run reference: {}",
            run_reference(golden_now() + 2)
        )));
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["command"], "daily");
        assert_eq!(json["mode"], "max");
        assert_eq!(json["publisher"], serde_json::Value::Null);
        assert_eq!(json["skipped"], serde_json::json!([]));
        let blocks = json["publishers"].as_array().expect("publishers");
        assert_eq!(blocks.len(), 2, "{json}");
        assert_eq!(blocks[0]["publisher"], "aurora");
        assert_eq!(blocks[1]["publisher"], "northstar");
        assert!(
            json["ledger"]["records_appended"].as_u64().unwrap() >= 2,
            "{json}"
        );

        // Each ledger is its own verified chain carrying only its own facts.
        for (publisher, experiment) in [("aurora", "exp-0"), ("northstar", "exp-1")] {
            let path = ledger_root.join(format!("{publisher}.jsonl"));
            let records = apex_proof::Ledger::read_verified(&path, &signing_key.verifying_key())
                .expect("each ledger verifies as its own chain");
            assert!(records.len() >= 2, "{publisher}: {} records", records.len());
            let started: Vec<String> = records
                .iter()
                .filter_map(|record| match record {
                    apex_proof::LedgerRecord::ExperimentStarted { experiment_id, .. } => {
                        Some(experiment_id.clone())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(started, vec![experiment.to_string()], "{publisher}");
        }
        // The daily append advanced both ledgers independently.
        for publisher in ["aurora", "northstar"] {
            let block = blocks
                .iter()
                .find(|block| block["publisher"] == publisher)
                .expect("publisher block");
            assert_eq!(block["ledger"]["appended"], true, "{publisher}");
            assert!(
                block["ledger"]["records_appended"].as_u64().unwrap() >= 1,
                "{publisher}"
            );
        }

        // build_fleet configures the same two publishers from the registry
        // (their adapters come from the vaulted keys; no call is made here).
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");
        let built = build_fleet(
            |_| None,
            &registry,
            &ledger_root,
            &root,
            &root.join("snapshots"),
            &signing_key,
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("fleet built");
        assert_eq!(built.publishers.len(), 2);
        assert!(built.skipped.is_empty());
        assert!(!built.is_singleton());
        assert!(built.publishers.iter().all(|publisher| publisher
            .ledger_path
            .ends_with(format!("{}.jsonl", publisher.publisher))));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn fleet_daily_passes_run_with_bounded_concurrency() {
        const PUBLISHERS: [&str; 4] = ["aurora", "northstar", "orion", "vega"];
        const DELAY_MS: u64 = 200;
        let root = fleet_dir("daily-concurrent");
        let ledger_root = root.join("ledgers");
        std::fs::create_dir_all(&ledger_root).unwrap();
        let signing_key = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let mut publishers = Vec::new();
        for (index, publisher) in PUBLISHERS.iter().enumerate() {
            let experiment = format!("exp-{index}");
            let inner = fleet_ops(&experiment);
            let ledger_path = ledger_root.join(format!("{publisher}.jsonl"));
            {
                let mut ledger =
                    apex_proof::Ledger::open(&ledger_path, signing_key.clone()).unwrap();
                start_one_change(&inner, &mut ledger, golden_now()).await;
            }
            let ops = std::sync::Arc::new(DelayedOps::new(
                inner,
                std::time::Duration::from_millis(DELAY_MS),
                None,
            ));
            let runner = crate::runner::Runner::new(
                ops,
                test_policy(),
                apex_lift::guardrails::FillFormat::Rewarded,
            )
            .with_publisher(*publisher);
            publishers.push(FleetPublisher {
                publisher: (*publisher).to_string(),
                runner,
                ledger_path,
                snapshot_paths: Vec::new(),
            });
        }
        let fleet = empty_fleet(publishers, Vec::new(), signing_key.clone());
        let started = std::time::Instant::now();
        let outcome = dispatch_fleet("daily", &[], &fleet, golden_now() + 2)
            .await
            .expect("fleet daily");
        let elapsed = started.elapsed();
        eprintln!(
            "fleet daily: {elapsed:?} for {} publisher(s); serial baseline {}ms x {} = {}ms",
            PUBLISHERS.len(),
            DELAY_MS,
            PUBLISHERS.len(),
            DELAY_MS * PUBLISHERS.len() as u64
        );
        assert!(!outcome.failed, "{outcome:?}");
        // Serial would be 4 * 200ms = 800ms; bounded concurrency 4 keeps the
        // four independent reporting reads in one wave.
        assert!(
            elapsed.as_millis() < 600,
            "fleet daily took {elapsed:?}, expected near 200ms"
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        let blocks = json["publishers"].as_array().expect("blocks");
        assert_eq!(blocks.len(), PUBLISHERS.len(), "{json}");
        let ordered: Vec<&str> = blocks
            .iter()
            .filter_map(|block| block["publisher"].as_str())
            .collect();
        assert_eq!(ordered, PUBLISHERS, "report order is the input order");
        // Every publisher's own ledger advanced independently.
        for publisher in PUBLISHERS {
            let records = apex_proof::Ledger::read_verified(
                ledger_root.join(format!("{publisher}.jsonl")),
                &signing_key.verifying_key(),
            )
            .expect("ledger verifies");
            assert!(
                records.iter().any(|record| matches!(
                    record,
                    apex_proof::LedgerRecord::LiftRecord { payload, .. }
                        if payload.contains("dataPull")
                )),
                "{publisher} appended its own pull record"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn unvalidated_and_undecryptable_publishers_are_skipped_with_reasons() {
        let root = fleet_dir("skips");
        write_vaulted_publisher(&root, "northstar", "management-key", "validated");
        // A blob + meta whose connection was never validated.
        write_vaulted_publisher(&root, "pending", "management-key", "unvalidated");
        // A validated meta whose blob was written under another vault key.
        let other_vault = apex_vault::Vault::from_hex_key(FLEET_KEY_A).expect("vault");
        std::fs::write(root.join("wrongkey.blob"), other_vault.encrypt("{}")).expect("blob");
        let meta = serde_json::json!({
            "platforms": { "applovin_max": { "state": "validated" } }
        });
        std::fs::write(root.join("wrongkey.connections.json"), meta.to_string()).expect("meta");

        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault");
        let registry = crate::registry::PublisherRegistry::load(&root, &vault).expect("registry");
        assert_eq!(registry.runnable().count(), 1);

        let ops = fleet_ops("exp-1");
        let ledger_root = root.join("ledgers");
        std::fs::create_dir_all(&ledger_root).unwrap();
        let signing_key = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let registered = registry.runnable().next().expect("northstar");
        let runner = crate::runner::Runner::new(
            ops,
            test_policy(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_publisher(registered.publisher.clone());
        let publishers = vec![FleetPublisher {
            publisher: registered.publisher.clone(),
            runner,
            ledger_path: ledger_root.join(format!("{}.jsonl", registered.publisher)),
            snapshot_paths: Vec::new(),
        }];
        let skipped: Vec<FleetSkip> = registry
            .skipped()
            .map(|skip| FleetSkip {
                publisher: skip.publisher.clone(),
                reason: skip.reason.to_string(),
            })
            .collect();
        let fleet = empty_fleet(publishers, skipped, signing_key.clone());
        let outcome = dispatch_fleet("daily", &[], &fleet, golden_now())
            .await
            .expect("fleet daily");
        assert!(
            outcome
                .human
                .starts_with("Fleet: 1 publisher(s) / 2 skipped\n"),
            "{}",
            outcome.human
        );
        assert!(
            outcome
                .human
                .contains("Skipped: pending: no validated platform connection"),
            "{}",
            outcome.human
        );
        assert!(
            outcome.human.contains(
                "Skipped: wrongkey: credential decrypt failed: decryption failed (wrong key or tampered ciphertext)"
            ),
            "{}",
            outcome.human
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(
            json["skipped"],
            serde_json::json!([
                {
                    "publisher": "pending",
                    "reason": "no validated platform connection",
                },
                {
                    "publisher": "wrongkey",
                    "reason": "credential decrypt failed: decryption failed (wrong key or tampered ciphertext)",
                },
            ])
        );
        assert_eq!(json["publishers"].as_array().expect("blocks").len(), 1);
        assert_eq!(json["publishers"][0]["publisher"], "northstar");
        // A skipped publisher never gets a ledger file.
        assert!(ledger_root.join("northstar.jsonl").exists());
        assert!(!ledger_root.join("pending.jsonl").exists());
        assert!(!ledger_root.join("wrongkey.jsonl").exists());

        // build_fleet carries the same skip list and configures the one
        // runnable publisher from the registry.
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");
        let built = build_fleet(
            |_| None,
            &registry,
            &ledger_root,
            &root,
            &root.join("snapshots"),
            &signing_key,
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("fleet built");
        assert_eq!(built.publishers.len(), 1);
        assert_eq!(
            built.skipped,
            vec![
                FleetSkip {
                    publisher: "pending".into(),
                    reason: "no validated platform connection".into(),
                },
                FleetSkip {
                    publisher: "wrongkey".into(),
                    reason:
                        "credential decrypt failed: decryption failed (wrong key or tampered ciphertext)"
                            .into(),
                },
            ]
        );
        assert!(built.payments.is_none());
        assert!(!built.is_singleton());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_fleet_monthly_run_issues_invoices_per_publisher_and_surfaces_refusals() {
        let root = fleet_dir("monthly");
        // Only northstar has a customer profile; aurora's invoice must refuse.
        std::fs::write(
            root.join("northstar.billing.json"),
            serde_json::to_string_pretty(&apex_billing::CustomerProfile {
                legal_name: "Nurme OÜ".into(),
                address_lines: vec!["Narva mnt 5".into()],
                country: "EE".into(),
                vat_number: Some("EE987654321".into()),
            })
            .unwrap(),
        )
        .expect("customer profile");
        let register_path = root.join("invoices.jsonl");
        let ledger_root = root.join("ledgers");
        std::fs::create_dir_all(&ledger_root).unwrap();
        let signing_key = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let at = 1_700_000_000_000; // 2023-11-14, inside the billed month
        let mut publishers = Vec::new();
        for (index, publisher) in ["aurora", "northstar"].iter().enumerate() {
            let ops = fleet_ops(&format!("exp-{index}"));
            let ledger_path = ledger_root.join(format!("{publisher}.jsonl"));
            {
                let mut ledger =
                    apex_proof::Ledger::open(&ledger_path, signing_key.clone()).unwrap();
                keep_one_change(&ops, &mut ledger, at).await;
            }
            let runner = crate::runner::Runner::new(
                ops,
                test_policy(),
                apex_lift::guardrails::FillFormat::Rewarded,
            )
            .with_publisher(*publisher)
            .with_billing_profile(golden_billing_profile())
            .with_connections_root(&root)
            .with_register_path(&register_path);
            publishers.push(FleetPublisher {
                publisher: (*publisher).to_string(),
                runner,
                ledger_path,
                snapshot_paths: Vec::new(),
            });
        }
        let fleet = empty_fleet(publishers, Vec::new(), signing_key.clone());
        let args: Vec<String> = [
            "--month",
            "2023-11",
            "--start",
            "2023-11-01",
            "--days",
            "30",
        ]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect();
        let outcome = dispatch_fleet("monthly", &args, &fleet, at + 2)
            .await
            .expect("fleet monthly");
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        let blocks = json["publishers"].as_array().expect("blocks");
        assert_eq!(blocks.len(), 2, "{json}");
        let northstar = blocks
            .iter()
            .find(|block| block["publisher"] == "northstar")
            .expect("northstar block");
        let aurora = blocks
            .iter()
            .find(|block| block["publisher"] == "aurora")
            .expect("aurora block");
        let number = northstar["billing"]["invoice_number"]
            .as_u64()
            .expect("issued number");
        assert!(number >= 1);
        assert!(northstar["billing"]["rendered_invoice"]
            .as_str()
            .expect("rendered")
            .starts_with(&format!("VAT invoice No. {number} for 2023-11\n")));
        assert_eq!(
            aurora["billing"]["invoice_refusal"],
            serde_json::json!(
                "the invoice carries no customer profile, so no VAT treatment can be stated"
            )
        );
        assert!(aurora["billing"]["invoice_number"].is_null());
        assert!(
            northstar["billing"]["invoice_refusal"].is_null()
                || northstar["billing"]["invoice_refusal"].as_str().is_none(),
            "{northstar}"
        );
        // Exactly one register line: the refused publisher consumed none.
        let raw = std::fs::read_to_string(&register_path).expect("register");
        assert_eq!(raw.lines().count(), 1, "{raw}");
        assert!(raw.contains(&format!("\"number\":{number}")), "{raw}");
        assert!(
            outcome
                .human
                .contains("Invoice: refused (the invoice carries no customer profile"),
            "{}",
            outcome.human
        );
        // Both per-publisher ledger files verify as independent chains.
        for publisher in ["aurora", "northstar"] {
            apex_proof::Ledger::verify(
                ledger_root.join(format!("{publisher}.jsonl")),
                &signing_key.verifying_key(),
            )
            .expect("chain verifies");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_fleet_daily_pass_processes_payment_events_once_grouped_by_publisher() {
        let root = fleet_dir("fleet-payments");
        let signing_key = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let ledger_root = root.join("ledgers");
        std::fs::create_dir_all(&ledger_root).unwrap();
        let mut publishers = Vec::new();
        for publisher in ["aurora", "northstar"] {
            let runner = crate::runner::Runner::new(
                std::sync::Arc::new(crate::ops::RecordingOps::new()),
                test_policy(),
                apex_lift::guardrails::FillFormat::Rewarded,
            )
            .with_publisher(publisher);
            publishers.push(FleetPublisher {
                publisher: publisher.to_string(),
                runner,
                ledger_path: ledger_root.join(format!("{publisher}.jsonl")),
                snapshot_paths: Vec::new(),
            });
        }
        let collection = root.join("collection.jsonl");
        let mut raw = String::new();
        for (psp, reference) in [
            ("psp_1", "apex-fee:northstar:ch_0246:2026-10"),
            ("psp_2", "apex-fee:aurora:ch_0247:2026-10"),
        ] {
            raw.push_str(
                &serde_json::to_string(&apex_billing::CollectionEvent {
                    at_unix_ms: golden_now(),
                    payment_id: psp.into(),
                    reference: reference.into(),
                    status: "paid".into(),
                    amount_minor_units: 300,
                    amount_currency: "USD".into(),
                    stored_customer_id: String::new(),
                    stored_mandate_id: String::new(),
                })
                .unwrap(),
            );
            raw.push('\n');
        }
        std::fs::write(&collection, &raw).unwrap();
        let mut fleet = empty_fleet(publishers, Vec::new(), signing_key.clone());
        fleet.payments = Some(FleetPayments {
            events_path: collection.clone(),
            cursor_path: None,
            ledger_path: None,
            retries_path: None,
        });
        let outcome = dispatch_fleet("daily", &[], &fleet, golden_now())
            .await
            .expect("fleet daily");
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(
            json["payment_events"]["received"], 2,
            "both publishers' events were processed"
        );
        assert_eq!(json["payment_events"]["failed"], 0);
        assert!(
            outcome
                .human
                .contains("Payment events: 2 received / 0 failed"),
            "{}",
            outcome.human
        );
        // One cursor advanced once, to the end of the collection.
        let cursor = crate::payment_events::default_cursor_path(&collection);
        assert_eq!(
            crate::payment_events::read_consumed_offset(&cursor).unwrap(),
            std::fs::metadata(&collection).unwrap().len()
        );
        // The next run consumes nothing new (the cursor is the only one).
        let repeat = dispatch_fleet("daily", &[], &fleet, golden_now() + 1)
            .await
            .expect("second fleet daily");
        let repeat_json: serde_json::Value = serde_json::from_str(&repeat.json).expect("json");
        assert_eq!(repeat_json["payment_events"]["received"], 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn the_legacy_env_mode_builds_and_dispatches_one_publisher() {
        // No registry root, only the publisher env: the legacy mode.
        let env = |name: &str| match name {
            "APEX_RUNNER_PUBLISHER" => Some("northstar".to_string()),
            _ => None,
        };
        assert!(crate::registry::vault_root_from_env(&env).is_none());
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");
        let runner = build_runner(
            env,
            std::sync::Arc::new(NullOps),
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("legacy runner");
        assert_eq!(runner.publisher_id(), "northstar");
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[3u8; 32]));
        let outcome = dispatch("daily", &[], &runner, &mut ledger, golden_now())
            .await
            .expect("legacy daily");
        // The committed single-publisher shape is exactly what the legacy
        // mode still emits.
        assert_eq!(
            outcome.human,
            "Mode: MAX / Publisher: northstar\n\
             Reporting pulls: 0 completed\n\
             Decisions: 0 pending / 0 kept\n\
             Platform operations: no unresolved result\n\
             Ledger: no new records\n\
             Run reference: job_20261015_0810"
        );
        assert!(outcome.json.contains("\"publisher\":\"northstar\""));
        assert!(!outcome.json.contains("\"publishers\""));
    }

    #[tokio::test]
    async fn audit_coverage_names_the_rule_groups_its_inputs_could_not_feed() {
        use apex_audit::Snapshot;
        let empty = Snapshot {
            platform: apex_core::PlatformId::AppLovinMax,
            fetched_at_unix_ms: golden_now(),
            ad_units: Vec::new(),
            waterfall: Vec::new(),
            mediation_groups: Vec::new(),
            bidder_accounts: Vec::new(),
            country_revenue: Vec::new(),
            app_ads_txt: apex_audit::AppAdsTxt {
                present: false,
                lines: Vec::new(),
                expected_networks: Vec::new(),
            },
            platform_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_reported: false,
        };
        let (covered, expected, missing) = audit_coverage(&empty);
        assert_eq!((covered, expected), (0, 5));
        assert_eq!(
            missing,
            vec![
                "stale_floor".to_string(),
                "missing_country_floor".to_string(),
                "disabled_bidder_with_account".to_string(),
                "reporting_gap".to_string(),
                "app_ads_txt".to_string(),
            ]
        );
        assert!(
            audit_rule_version().starts_with('v'),
            "{}",
            audit_rule_version()
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
        let runner = runner
            .with_billing_profile(golden_billing_profile())
            .with_register_path(golden_register_path());
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
    async fn the_cycle_command_never_activates_or_bills_monitoring() {
        use apex_audit::Snapshot;
        let dir =
            std::env::temp_dir().join(format!("apex-cli-free-monitoring-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let snapshot_path = dir.join("snapshot.json");
        let snapshot = Snapshot {
            platform: apex_core::PlatformId::AppLovinMax,
            fetched_at_unix_ms: golden_now(),
            ad_units: Vec::new(),
            waterfall: Vec::new(),
            mediation_groups: Vec::new(),
            bidder_accounts: Vec::new(),
            country_revenue: Vec::new(),
            app_ads_txt: apex_audit::AppAdsTxt {
                present: false,
                lines: Vec::new(),
                expected_networks: Vec::new(),
            },
            platform_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 350_000 * 1_000_000,
                impressions_30d: 0,
            },
            mediation_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_reported: false,
        };
        std::fs::write(&snapshot_path, serde_json::to_string(&snapshot).unwrap()).unwrap();
        let runner = golden_runner().with_connections_root(&dir);
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[3u8; 32]));
        let args = vec![
            "--snapshot".to_string(),
            snapshot_path.display().to_string(),
        ];
        let outcome = dispatch("cycle", &args, &runner, &mut ledger, golden_now())
            .await
            .expect("cycle");
        assert!(!outcome.human.contains("Monitoring: activated"));
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert!(json["cycle"]["monitoring_activated"].is_null());
        assert!(
            !dir.join("northstar.monitoring.json").exists(),
            "free monitoring records no paid plan state"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_cycle_report_records_a_capability_hold_against_the_admob_fixture() {
        // The exact live defect: fleet/AdMob mode consuming the committed
        // AdMob fixture, whose MAX-style floor proposals are not in the
        // AdMob verified write set. The pass exits 0, records the typed
        // awaiting state and never surfaces the historical hard error.
        let fixture_dir = std::env::temp_dir().join(format!(
            "apex-cli-admob-hold-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&fixture_dir);
        std::fs::create_dir_all(&fixture_dir).expect("dir");
        let fixture = fixture_dir.join("admob.json");
        let mut snapshot: apex_audit::Snapshot =
            serde_json::from_str(include_str!("../../apex-api/fixtures/admob.json"))
                .expect("fixture snapshot");
        // AdMob's real write surface is the mediation-group read, not a MAX
        // ad-unit floor. Keep the committed audit findings but give the
        // snapshot the operation an AdMob adapter actually owns.
        snapshot.ad_units.clear();
        snapshot.mediation_groups = vec![apex_core::MediationGroup {
            mediation_group_id: "g1".into(),
            name: "group g1".into(),
            format: apex_core::AdFormat::Rewarded,
            mappings: vec![apex_core::Mapping {
                ad_unit_id: "u1".into(),
                network: "meta".into(),
                ecpm: 1_000_000,
            }],
        }];
        std::fs::write(&fixture, serde_json::to_vec(&snapshot).unwrap()).expect("write");
        let ops = std::sync::Arc::new(crate::ops::RecordingOps::new());
        ops.set_platform(apex_core::PlatformId::Admob);
        ops.set_writable("updateMediationGroup", false);
        let mut policy = crate::model::AutopilotPolicy::conservative();
        // This fixture is about the adapter's verified write set, not the
        // audit blackout; the committed snapshot intentionally carries audit
        // findings so the hold path is exercised.
        policy.blackout_on_critical_findings = false;
        let runner =
            crate::runner::Runner::new(ops, policy, apex_lift::guardrails::FillFormat::Rewarded)
                .with_publisher("northstar")
                .with_billing_profile(golden_billing_profile())
                .with_register_path(golden_register_path());
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[5u8; 32]));
        let args = vec!["--snapshot".to_string(), fixture.display().to_string()];
        let outcome = dispatch("cycle", &args, &runner, &mut ledger, golden_now())
            .await
            .expect("the cycle continues past unverified operations");
        assert!(!outcome.failed, "{outcome:?}");
        assert!(
            outcome
                .human
                .contains("Skipped: awaiting live verification for Admob: updateMediationGroup"),
            "{}",
            outcome.human
        );
        assert!(
            !outcome.human.contains("not writable"),
            "the historical hard error never returns: {}",
            outcome.human
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["cycle"]["applied"], 0);
        assert!(json["cycle"]["proposed"].as_u64().unwrap_or(0) > 0);
        let holds = json["cycle"]["awaiting_verification"]
            .as_array()
            .expect("the typed awaiting record is present");
        assert!(!holds.is_empty());
        assert!(holds
            .iter()
            .all(|reason| reason == "awaiting live verification for Admob: updateMediationGroup"));
        let _ = std::fs::remove_dir_all(&fixture_dir);
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

    // ------------------------------------------------------------------
    // Fleet audit: one snapshot per publisher and platform.
    // ------------------------------------------------------------------

    /// A transport that answers from a fixed queue in call order and records
    /// the requests; the snapshot builders' call order is what the queue
    /// encodes, so an out-of-order build is a visible test failure.
    #[derive(Debug, Default)]
    struct ScriptedTransport {
        queue:
            std::sync::Mutex<std::collections::VecDeque<apex_connectors::transport::HttpResponse>>,
        seen: std::sync::Mutex<Vec<apex_connectors::transport::HttpRequest>>,
    }

    impl ScriptedTransport {
        fn push(&self, status: u16, body: &str) {
            self.queue
                .lock()
                .unwrap()
                .push_back(apex_connectors::transport::HttpResponse {
                    status,
                    headers: vec![],
                    body: body.as_bytes().to_vec(),
                });
        }
    }

    #[async_trait::async_trait]
    impl apex_connectors::transport::Transport for ScriptedTransport {
        async fn request(
            &self,
            request: apex_connectors::transport::HttpRequest,
        ) -> Result<
            apex_connectors::transport::HttpResponse,
            apex_connectors::transport::TransportError,
        > {
            self.seen.lock().unwrap().push(request);
            Ok(self.queue.lock().unwrap().pop_front().unwrap_or(
                apex_connectors::transport::HttpResponse {
                    status: 500,
                    headers: vec![],
                    body: b"script exhausted".to_vec(),
                },
            ))
        }
    }

    fn reporting_body(unit: &str) -> String {
        format!(
            r#"{{"code":200,"results":[{{"day":"2026-10-01","country":"EE","max_ad_unit_id":"{unit}","estimated_revenue":"12.5","impressions":"100"}}]}}"#
        )
    }

    fn revenue_totals_body(unit: &str) -> String {
        format!(
            r#"{{"code":200,"results":[{{"day":"2026-10-01","max_ad_unit_id":"{unit}","estimated_revenue":"12.5","impressions":"100"}}]}}"#
        )
    }

    fn levelplay_ab_body() -> String {
        r#"{"code":200,"results":[{"date":"2026-10-01","abTest":"A","revenue":"1.0","activeUsers":"10"}]}"#
            .to_string()
    }

    fn ad_units_body(unit: &str) -> String {
        format!(
            r#"[{{"id":"{unit}","name":"Rewarded","ad_format":"REWARD","platform":"android","package_name":"com.x"}}]"#
        )
    }

    fn levelplay_instances_body() -> String {
        r#"{"instances":[{"instanceId":"e1","adapterNetwork":"admob","instanceName":"ADM_1","price":1.0,"position":1}]}"#
            .to_string()
    }

    fn levelplay_report_body() -> String {
        r#"{"code":200,"results":[{"date":"2026-10-01","revenue":"1.0"}]}"#.to_string()
    }

    fn admob_report_body() -> String {
        r#"{"header":{"currencyCode":"USD"},"rows":[{"dimensionValues":{"DATE":{"value":"2026-10-01"}},"metricValues":{"ESTIMATED_EARNINGS":{"microsValue":"1000000"}}}]}"#
            .to_string()
    }

    fn admob_groups_body() -> String {
        r#"{"mediationGroups":[{"mediationGroupId":"g1","name":"Main","format":"interstitial","adUnitMappings":[{"adUnitId":"u1","network":"admob","ecpm":2}]}]}"#
            .to_string()
    }

    /// Vault a publisher with an arbitrary platform map and validated states.
    fn write_vaulted_account(
        root: &std::path::Path,
        publisher: &str,
        platforms: serde_json::Value,
        states: &[(&str, &str)],
        ad_unit_ids: &[&str],
    ) {
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault key");
        let account = serde_json::json!({
            "developer_site": "studio.example.com",
            "expected_networks": [],
            "ad_unit_ids": ad_unit_ids,
            "bidder_accounts": [],
            "platforms": platforms,
        });
        std::fs::write(
            root.join(format!("{publisher}.blob")),
            vault.encrypt(&account.to_string()),
        )
        .expect("blob");
        let mut meta_platforms = serde_json::Map::new();
        for (wire, state) in states {
            meta_platforms.insert(
                (*wire).to_string(),
                serde_json::json!({ "state": state, "credentialFields": ["api_key"] }),
            );
        }
        let meta = serde_json::json!({ "platforms": meta_platforms });
        std::fs::write(
            root.join(format!("{publisher}.connections.json")),
            meta.to_string(),
        )
        .expect("meta");
    }

    fn max_credentials(api_key: &str) -> serde_json::Value {
        serde_json::json!({
            "api_key": api_key,
            "account_id": "acct-1",
            "base_url": "https://api.example.test",
            "report_key": "report-key",
            "reporting_base_url": null,
            "refresh_token": null
        })
    }

    async fn audit_fleet(
        registry: &crate::registry::PublisherRegistry,
        snapshot_root: &std::path::Path,
        transport: std::sync::Arc<ScriptedTransport>,
    ) -> (FleetAuditReport, DispatchOutcome) {
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");
        let report = audit_fleet_snapshots(
            registry,
            snapshot_root,
            transport,
            &instrumentation,
            &audit_rule_version(),
            golden_now(),
        )
        .await
        .expect("fleet audit");
        let outcome = render_fleet_audit(&report, golden_now());
        (report, outcome)
    }

    fn read_snapshot(path: &std::path::Path) -> apex_audit::Snapshot {
        serde_json::from_slice(&std::fs::read(path).expect("snapshot file")).expect("snapshot json")
    }

    fn unit_ids(snapshot: &apex_audit::Snapshot) -> Vec<String> {
        snapshot
            .ad_units
            .iter()
            .map(|unit| unit.platform_unit_id.clone())
            .collect()
    }

    #[tokio::test]
    async fn two_publishers_in_one_audit_run_write_their_own_snapshots() {
        let root = fleet_dir("audit-two");
        let snapshot_root = root.join("snapshots");
        write_vaulted_publisher(&root, "aurora", "management-key-aurora", "validated");
        write_vaulted_publisher(&root, "northstar", "management-key-north", "validated");
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault");
        let registry = crate::registry::PublisherRegistry::load(&root, &vault).expect("registry");
        assert_eq!(registry.runnable().count(), 2);

        let transport = std::sync::Arc::new(ScriptedTransport::default());
        // aurora (sorted first): reporting + ad units + app-ads.txt. Both
        // fixtures share a developer site, so northstar replays the cached
        // app-ads.txt read.
        transport.push(200, &reporting_body("aurora-unit"));
        transport.push(200, &revenue_totals_body("aurora-unit"));
        transport.push(200, &ad_units_body("aurora-unit"));
        transport.push(200, "google.com, pub-1, DIRECT\n");
        // northstar: its own distinct unit.
        transport.push(200, &reporting_body("northstar-unit"));
        transport.push(200, &revenue_totals_body("northstar-unit"));
        transport.push(200, &ad_units_body("northstar-unit"));

        let (report, outcome) = audit_fleet(&registry, &snapshot_root, transport).await;
        assert_eq!(report.publishers.len(), 2);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.publishers[0].publisher, "aurora");
        assert_eq!(report.publishers[1].publisher, "northstar");

        let aurora_path = snapshot_root.join("aurora").join("applovin_max.json");
        let northstar_path = snapshot_root.join("northstar").join("applovin_max.json");
        assert!(aurora_path.is_file(), "each publisher gets its own file");
        assert!(northstar_path.is_file());
        let aurora = read_snapshot(&aurora_path);
        let northstar = read_snapshot(&northstar_path);
        assert_eq!(unit_ids(&aurora), vec!["aurora-unit".to_string()]);
        assert_eq!(unit_ids(&northstar), vec!["northstar-unit".to_string()]);
        assert_eq!(aurora.platform, apex_core::PlatformId::AppLovinMax);
        // The bytes never cross: no file carries another publisher's unit.
        let aurora_bytes = std::fs::read_to_string(&aurora_path).expect("bytes");
        let northstar_bytes = std::fs::read_to_string(&northstar_path).expect("bytes");
        assert!(!aurora_bytes.contains("northstar-unit"), "{aurora_bytes}");
        assert!(
            !northstar_bytes.contains("aurora-unit"),
            "{northstar_bytes}"
        );

        let aurora_digest = &report.publishers[0].snapshots[0].digest;
        let northstar_digest = &report.publishers[1].snapshots[0].digest;
        assert_ne!(
            aurora_digest, northstar_digest,
            "each snapshot has its own digest"
        );

        assert!(
            outcome
                .human
                .starts_with("Fleet: 2 publisher(s) / 0 skipped / 2 snapshot(s)\n"),
            "{}",
            outcome.human
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["command"], "audit");
        assert_eq!(json["mode"], "max");
        assert_eq!(json["publisher"], serde_json::Value::Null);
        let blocks = json["publishers"].as_array().expect("publishers");
        assert_eq!(blocks.len(), 2, "{json}");
        assert_eq!(blocks[0]["publisher"], "aurora");
        assert_eq!(blocks[1]["publisher"], "northstar");
        assert_eq!(blocks[0]["snapshots"][0]["platform"], "applovin_max");
        assert_eq!(
            blocks[0]["snapshots"][0]["path"],
            aurora_path.display().to_string()
        );
        assert_eq!(blocks[0]["snapshots"][0]["ad_units"], 1);
        assert!(blocks[0]["audit"]["findings"].as_u64().is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_publisher_per_platform_writes_three_files_with_the_right_platform_inside() {
        let root = fleet_dir("audit-platforms");
        let snapshot_root = root.join("snapshots");
        // Sorted order: admob-pub, levelplay-pub, max-pub.
        write_vaulted_account(
            &root,
            "admob-pub",
            serde_json::json!({ "admob": {
                "api_key": "access-token",
                "account_id": "accounts/pub-1234567890",
                "base_url": "https://admob.googleapis.com"
            }}),
            &[("admob", "validated")],
            &[],
        );
        write_vaulted_account(
            &root,
            "levelplay-pub",
            serde_json::json!({ "levelplay": {
                "api_key": "secret-key",
                "account_id": "app-key-1",
                "base_url": "https://platform.ironsrc.com",
                "refresh_token": "refresh-token"
            }}),
            &[("levelplay", "validated")],
            &["u1"],
        );
        write_vaulted_account(
            &root,
            "max-pub",
            serde_json::json!({ "applovin_max": max_credentials("management-key") }),
            &[("applovin_max", "validated")],
            &[],
        );
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault");
        let registry = crate::registry::PublisherRegistry::load(&root, &vault).expect("registry");
        assert_eq!(registry.runnable().count(), 3);

        let transport = std::sync::Arc::new(ScriptedTransport::default());
        // All three fixtures share one developer site, so the run cache
        // serves one app-ads.txt read to every publisher: only one response
        // is queued.
        // admob-pub: mediation report + groups + app-ads.txt.
        transport.push(200, &admob_report_body());
        transport.push(200, &admob_report_body());
        transport.push(200, &admob_groups_body());
        transport.push(200, "google.com, pub-1, DIRECT\n");
        // levelplay-pub: auth + app revenue + one unit's instances (app-ads is replayed).
        transport.push(200, r#""bearer-token""#);
        transport.push(200, &levelplay_report_body());
        transport.push(200, &levelplay_ab_body());
        transport.push(
            200,
            r#"{"groups":[{"groupId":"2001","abTest":"A","instances":[{"instanceId":"e1"}]}]}"#,
        );
        transport.push(200, &levelplay_instances_body());
        // max-pub: reporting + ad units (app-ads is replayed).
        transport.push(200, &reporting_body("max-unit"));
        transport.push(200, &revenue_totals_body("max-unit"));
        transport.push(200, &ad_units_body("max-unit"));

        let (report, _outcome) = audit_fleet(&registry, &snapshot_root, transport.clone()).await;
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.publishers.len(), 3);
        // One revenue read per publisher/platform, shared exactly once: the
        // app-ads cache serves the single developer-site fetch to every
        // platform and publisher, and each platform's 30-day read is called
        // once (no per-snapshot re-fetch).
        let seen = transport.seen.lock().unwrap();
        let operation_calls = |operation: &str| {
            seen.iter()
                .filter(|request| request.operation == operation)
                .count()
        };
        assert_eq!(operation_calls("mediationReport"), 1);
        assert_eq!(operation_calls("networkReport"), 1);
        assert_eq!(operation_calls("adUnitRevenue"), 1);
        assert_eq!(
            operation_calls("maxReport"),
            2,
            "country rows and the independent mediation total"
        );
        assert_eq!(
            seen.iter()
                .filter(|request| request.url.contains("app-ads.txt"))
                .count(),
            1,
            "one cached app-ads.txt read for the whole fleet"
        );
        drop(seen);

        let admob_path = snapshot_root.join("admob-pub").join("admob.json");
        let levelplay_path = snapshot_root.join("levelplay-pub").join("levelplay.json");
        let max_path = snapshot_root.join("max-pub").join("applovin_max.json");
        assert!(admob_path.is_file());
        assert!(levelplay_path.is_file());
        assert!(max_path.is_file());

        let admob = read_snapshot(&admob_path);
        let levelplay = read_snapshot(&levelplay_path);
        let max = read_snapshot(&max_path);
        assert_eq!(admob.platform, apex_core::PlatformId::Admob);
        assert_eq!(levelplay.platform, apex_core::PlatformId::LevelPlay);
        assert_eq!(max.platform, apex_core::PlatformId::AppLovinMax);
        // Each snapshot carries its own platform's reads.
        assert_eq!(admob.mediation_groups.len(), 1);
        assert_eq!(levelplay.waterfall.len(), 1);
        assert_eq!(unit_ids(&max), vec!["max-unit".to_string()]);
        // A mixed fleet does not claim one platform.
        let json: serde_json::Value =
            serde_json::from_str(&render_fleet_audit(&report, golden_now()).json).expect("json");
        assert_eq!(json["mode"], "fleet");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_validated_publisher_can_carry_several_platforms_each_with_its_own_file() {
        let root = fleet_dir("audit-multi");
        let snapshot_root = root.join("snapshots");
        // One publisher, both platform key sets validated: two snapshots.
        write_vaulted_account(
            &root,
            "studio",
            serde_json::json!({
                "applovin_max": max_credentials("management-key"),
                "levelplay": {
                    "api_key": "secret-key",
                    "account_id": "app-key-1",
                    "base_url": "https://platform.ironsrc.com",
                    "refresh_token": "refresh-token"
                }
            }),
            &[("applovin_max", "validated"), ("levelplay", "validated")],
            &["u1"],
        );
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault");
        let registry = crate::registry::PublisherRegistry::load(&root, &vault).expect("registry");
        let registered = registry.runnable().next().expect("studio");
        // The adapter platform (applovin_max) comes first so monitoring reads
        // the adapter's own snapshot.
        assert_eq!(
            registered.snapshot_platforms(),
            vec!["applovin_max".to_string(), "levelplay".to_string()]
        );

        let transport = std::sync::Arc::new(ScriptedTransport::default());
        // applovin_max first: reporting + ad units + app-ads.txt.
        transport.push(200, &reporting_body("multi-unit"));
        transport.push(200, &revenue_totals_body("multi-unit"));
        transport.push(200, &ad_units_body("multi-unit"));
        transport.push(200, "google.com, pub-1, DIRECT\n");
        // then levelplay: auth + app revenue + instances. Its app-ads.txt read
        // is replayed from the run cache, so no second app-ads response is
        // queued: a broken cache would consume the exhausted script and fail
        // the snapshot.
        transport.push(200, r#""bearer-token""#);
        transport.push(200, &levelplay_report_body());
        transport.push(200, &levelplay_ab_body());
        transport.push(
            200,
            r#"{"groups":[{"groupId":"2001","abTest":"A","instances":[{"instanceId":"e1"}]}]}"#,
        );
        transport.push(200, &levelplay_instances_body());

        let (report, _outcome) = audit_fleet(&registry, &snapshot_root, transport.clone()).await;
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.publishers.len(), 1);
        assert_eq!(report.publishers[0].snapshots.len(), 2);
        let app_ads_fetches = transport
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.operation == "fetchAppAdsTxt")
            .count();
        assert_eq!(
            app_ads_fetches, 1,
            "one app-ads.txt fetch for every platform of a publisher"
        );
        assert!(snapshot_root
            .join("studio")
            .join("applovin_max.json")
            .is_file());
        assert!(snapshot_root
            .join("studio")
            .join("levelplay.json")
            .is_file());

        // build_fleet hands the cycle both files, adapter platform first.
        let ledger_root = root.join("ledgers");
        std::fs::create_dir_all(&ledger_root).unwrap();
        let signing_key = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let instrumentation = build_instrumentation(|_| None).expect("instrumentation");
        let built = build_fleet(
            |_| None,
            &registry,
            &ledger_root,
            &root,
            &snapshot_root,
            &signing_key,
            std::sync::Arc::new(NoTransport),
            &instrumentation,
        )
        .expect("fleet built");
        assert_eq!(
            built.publishers[0].snapshot_paths,
            vec![
                snapshot_root.join("studio").join("applovin_max.json"),
                snapshot_root.join("studio").join("levelplay.json"),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_snapshot_write_failure_is_recorded_and_the_fleet_continues() {
        let root = fleet_dir("audit-write-failure");
        let snapshot_root = root.join("snapshots");
        write_vaulted_publisher(&root, "aurora", "management-key-aurora", "validated");
        write_vaulted_publisher(&root, "northstar", "management-key-north", "validated");
        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault");
        let registry = crate::registry::PublisherRegistry::load(&root, &vault).expect("registry");

        // northstar's snapshot directory is a file: the write must fail
        // typed for that publisher without aborting aurora's audit.
        std::fs::create_dir_all(&snapshot_root).unwrap();
        std::fs::write(snapshot_root.join("northstar"), b"not a directory").unwrap();

        let transport = std::sync::Arc::new(ScriptedTransport::default());
        transport.push(200, &reporting_body("aurora-unit"));
        transport.push(200, &revenue_totals_body("aurora-unit"));
        transport.push(200, &ad_units_body("aurora-unit"));
        transport.push(200, "google.com, pub-1, DIRECT\n");
        transport.push(200, &reporting_body("northstar-unit"));
        transport.push(200, &revenue_totals_body("northstar-unit"));
        transport.push(200, &ad_units_body("northstar-unit"));

        let (report, _outcome) = audit_fleet(&registry, &snapshot_root, transport).await;
        assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
        assert_eq!(report.failures[0].publisher, "northstar");
        assert!(
            report.failures[0].reason.contains("snapshot write"),
            "{}",
            report.failures[0].reason
        );
        assert!(snapshot_root
            .join("aurora")
            .join("applovin_max.json")
            .is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn unvalidated_and_undecryptable_publishers_are_skipped_in_the_audit_report() {
        let root = fleet_dir("audit-skips");
        let snapshot_root = root.join("snapshots");
        write_vaulted_publisher(&root, "northstar", "management-key", "validated");
        write_vaulted_publisher(&root, "pending", "management-key", "unvalidated");
        // A validated meta whose blob was written under another vault key.
        let other_vault = apex_vault::Vault::from_hex_key(FLEET_KEY_A).expect("vault");
        std::fs::write(root.join("wrongkey.blob"), other_vault.encrypt("{}")).expect("blob");
        let meta = serde_json::json!({
            "platforms": { "applovin_max": { "state": "validated" } }
        });
        std::fs::write(root.join("wrongkey.connections.json"), meta.to_string()).expect("meta");

        let vault = apex_vault::Vault::from_hex_key(FLEET_VAULT_KEY).expect("vault");
        let registry = crate::registry::PublisherRegistry::load(&root, &vault).expect("registry");
        assert_eq!(registry.runnable().count(), 1);

        let transport = std::sync::Arc::new(ScriptedTransport::default());
        transport.push(200, &reporting_body("northstar-unit"));
        transport.push(200, &revenue_totals_body("northstar-unit"));
        transport.push(200, &ad_units_body("northstar-unit"));
        transport.push(200, "google.com, pub-1, DIRECT\n");

        let (report, outcome) = audit_fleet(&registry, &snapshot_root, transport).await;
        assert_eq!(
            report.publishers.len(),
            1,
            "only the validated publisher ran"
        );
        assert_eq!(report.skipped.len(), 2);
        assert!(
            outcome
                .human
                .contains("Skipped: pending: no validated platform connection"),
            "{}",
            outcome.human
        );
        assert!(
            outcome.human.contains(
                "Skipped: wrongkey: credential decrypt failed: decryption failed (wrong key or tampered ciphertext)"
            ),
            "{}",
            outcome.human
        );
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(
            json["skipped"],
            serde_json::json!([
                { "publisher": "pending", "reason": "no validated platform connection" },
                {
                    "publisher": "wrongkey",
                    "reason": "credential decrypt failed: decryption failed (wrong key or tampered ciphertext)"
                },
            ])
        );
        // A skipped publisher never gets a snapshot directory.
        assert!(!snapshot_root.join("pending").exists());
        assert!(!snapshot_root.join("wrongkey").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Write a MAX snapshot carrying one enabled ad unit with an EE floor.
    fn write_unit_snapshot(path: &std::path::Path, unit: &str) {
        let snapshot = apex_audit::Snapshot {
            platform: apex_core::PlatformId::AppLovinMax,
            fetched_at_unix_ms: golden_now(),
            ad_units: vec![apex_core::AdUnit {
                platform_unit_id: unit.to_string(),
                name: "Rewarded".to_string(),
                format: apex_core::AdFormat::Rewarded,
                enabled: true,
                country_floors: Some(vec![apex_core::CountryFloor {
                    country: "EE".to_string(),
                    floor_ecpm: 2_500_000,
                    group: None,
                }]),
                platform: Some("android".to_string()),
                package_name: Some("com.example".to_string()),
            }],
            waterfall: Vec::new(),
            mediation_groups: Vec::new(),
            bidder_accounts: Vec::new(),
            country_revenue: Vec::new(),
            app_ads_txt: apex_audit::AppAdsTxt {
                present: true,
                lines: Vec::new(),
                expected_networks: Vec::new(),
            },
            platform_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_reported: false,
        };
        crate::write_snapshot(path, &snapshot).expect("snapshot");
    }

    #[tokio::test]
    async fn each_publishers_cycle_reads_only_its_own_snapshot() {
        let root = fleet_dir("cycle-scoped");
        let snapshot_root = root.join("snapshots");
        let ledger_root = root.join("ledgers");
        std::fs::create_dir_all(&snapshot_root).unwrap();
        std::fs::create_dir_all(&ledger_root).unwrap();
        let signing_key = apex_proof::Keypair::from_bytes(&[7u8; 32]);

        let mut publishers = Vec::new();
        for (publisher, unit) in [("aurora", "u-aurora"), ("northstar", "u-northstar")] {
            let path = crate::registry::snapshot_path(&snapshot_root, publisher, "applovin_max");
            write_unit_snapshot(&path, unit);
            let ops = std::sync::Arc::new(crate::ops::RecordingOps::new());
            ops.set_change_in_test_arm(true);
            ops.script_start(vec![Ok(format!("exp-{publisher}"))]);
            let runner = crate::runner::Runner::new(
                ops,
                test_policy(),
                apex_lift::guardrails::FillFormat::Rewarded,
            )
            .with_publisher(publisher);
            publishers.push(FleetPublisher {
                publisher: publisher.to_string(),
                runner,
                ledger_path: ledger_root.join(format!("{publisher}.jsonl")),
                snapshot_paths: vec![path],
            });
        }
        let fleet = empty_fleet(publishers, Vec::new(), signing_key.clone());
        let outcome = dispatch_fleet("cycle", &[], &fleet, golden_now())
            .await
            .expect("fleet cycle");
        assert!(!outcome.failed, "{outcome:?}");
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["command"], "cycle");
        assert_eq!(json["publishers"].as_array().expect("blocks").len(), 2);

        // Each ledger's started experiment names only that publisher's unit:
        // a wrong-publisher proposal is impossible.
        for (publisher, own, other) in [
            ("aurora", "u-aurora", "u-northstar"),
            ("northstar", "u-northstar", "u-aurora"),
        ] {
            let records = apex_proof::Ledger::read_verified(
                ledger_root.join(format!("{publisher}.jsonl")),
                &signing_key.verifying_key(),
            )
            .expect("ledger verifies");
            let units: Vec<String> = records
                .iter()
                .filter_map(|record| match record {
                    apex_proof::LedgerRecord::ExperimentStarted { spec, .. } => {
                        Some(spec.ad_unit_id.clone())
                    }
                    _ => None,
                })
                .collect();
            assert!(!units.is_empty(), "{publisher} started an experiment");
            assert!(
                units.iter().all(|unit| unit == own),
                "{publisher}: {units:?}"
            );
            assert!(
                units.iter().all(|unit| unit != other),
                "{publisher}: {units:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn the_legacy_single_snapshot_cycle_output_is_unchanged() {
        // The legacy mode (no registry) keeps the single `--snapshot` path and
        // the committed single-publisher report shape: no `publishers` block.
        let dir = fleet_dir("cycle-legacy");
        let path = dir.join("applovin_max.json");
        write_unit_snapshot(&path, "u-legacy");
        let ops = std::sync::Arc::new(crate::ops::RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-legacy".to_string())]);
        let runner = crate::runner::Runner::new(
            ops,
            test_policy(),
            apex_lift::guardrails::FillFormat::Rewarded,
        )
        .with_publisher("northstar");
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[3u8; 32]));
        let args = vec!["--snapshot".to_string(), path.display().to_string()];
        let outcome = dispatch("cycle", &args, &runner, &mut ledger, golden_now())
            .await
            .expect("legacy cycle");
        assert!(outcome
            .human
            .starts_with("Mode: MAX / Publisher: northstar\n"));
        assert!(!outcome.json.contains("\"publishers\""), "{}", outcome.json);
        let json: serde_json::Value = serde_json::from_str(&outcome.json).expect("json");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["publisher"], "northstar");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
