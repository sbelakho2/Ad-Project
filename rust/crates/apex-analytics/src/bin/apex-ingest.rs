//! `apex-ingest` materializes Caddy JSON access logs as sanitized rows in
//! the analytics `request_logs` table (30-day retention per the spec). The
//! sanitized fields (route template, status, referrer host, campaign code,
//! allow-listed query keys, bot class) are exactly the `request_logs`
//! columns; client addresses, raw user agents and query values never reach
//! the store.
//!
//! Usage:
//!
//! ```text
//! apex-ingest [--check] [--json] [--spool <path>] [<access-log.json>]
//! ```
//!
//! Without a path it reads standard input, so the deployment can pipe a log
//! without staging it first:
//!
//! ```text
//! apex-ingest /var/log/caddy/apex-access.json
//! tail -n +1 /var/log/caddy/apex-access.json | apex-ingest --json
//! ```
//!
//! Environment:
//!
//! - `APEX_CLICKHOUSE_HTTP` (default `http://127.0.0.1:8123`)
//! - `APEX_CLICKHOUSE_DB` (default `apex_analytics`)
//! - `APEX_ENVIRONMENT` (default `production`)
//! - `APEX_SERVICE_VERSION` (default `apex-ingest-<crate version>`)
//! - `APEX_INGEST_SPOOL` (default `.apex/analytics-spool/apex-ingest.jsonl`)
//!
//! Flags:
//!
//! - `--check`: parse and validate only; nothing is written and no store is
//!   contacted.
//! - `--json`: print the summary as one JSON object.
//! - `--spool <path>`: override the spool file (failed batches are spooled
//!   for replay; corrupt spool lines move to `<path>.dead`).
//!
//! Exit codes: `0` all lines parsed and delivered (or validated under
//! `--check`); `1` some batches were spooled or some lines were rejected;
//! `2` usage or configuration error; `3` under `--check`, validation failed.
//!
//! The deployment needs a JSON access log on the site block. Exactly this
//! Caddyfile snippet:
//!
//! ```text
//! apexmediation.ee {
//!     log {
//!         output file /var/log/caddy/apex-access.json
//!         format json
//!     }
//! }
//! ```
//!
//! Then run the ingester on a timer or under a log shipper feeding stdin.
//! The parser keeps only bounded fields (route template, status, referrer
//! host, campaign code, allow-listed query keys, bot class); client
//! addresses, raw user agents and query values never reach the store.

#![forbid(unsafe_code)]
// Tests may unwrap/expect/panic; production paths handle every failure.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apex_analytics::accesslog::{parse_line, AccessLogDefaults};
use apex_analytics::store::{ClickHouseConfig, ClickHouseStore, EventStore, RequestLogRow};
use apex_analytics::timestamp::UtcMillis;

/// How many events one insert batch carries.
const BATCH_SIZE: usize = 500;

/// Parsed command-line options.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Options {
    path: Option<PathBuf>,
    check: bool,
    json: bool,
    spool: Option<PathBuf>,
}

/// The printed summary of one run.
#[derive(Debug, Clone, serde::Serialize)]
struct Summary {
    input: String,
    check_only: bool,
    lines: u64,
    /// Sanitized request rows parsed from the log.
    request_logs: u64,
    skipped_non_request: u64,
    rejected: u64,
    /// Request rows written into `request_logs`.
    delivered: u64,
    spooled: u64,
    spool_path: String,
    /// First rejection reasons (bounded; the count above is complete).
    errors: Vec<String>,
}

/// What one batch write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct WriteOutcome {
    written: usize,
    spooled: usize,
}

/// What a spool replay did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ReplayOutcome {
    replayed: usize,
    corrupt: usize,
}

/// Writes sanitized request rows into the store with a JSONL spool: a
/// failed batch is appended to the spool instead of being lost, and
/// [`RequestLogWriter::replay`] delivers it later. Corrupt spool lines move
/// to `<spool>.dead`, never dropped silently.
#[derive(Debug)]
struct RequestLogWriter<S: EventStore> {
    store: S,
    spool_path: PathBuf,
}

impl<S: EventStore> RequestLogWriter<S> {
    /// A writer over `store` spooling failed batches to `spool_path`.
    fn new(store: S, spool_path: PathBuf) -> Self {
        Self { store, spool_path }
    }

    /// The spool file path.
    fn spool_path(&self) -> &Path {
        &self.spool_path
    }

    fn dead_path(&self) -> PathBuf {
        let mut name = self
            .spool_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "spool.jsonl".to_string());
        name.push_str(".dead");
        self.spool_path.with_file_name(name)
    }

    async fn append_lines(&self, path: &Path, lines: &str) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        file.write_all(lines.as_bytes())
            .await
            .map_err(|error| format!("cannot append to {}: {error}", path.display()))
    }

    /// Insert one batch of rows; a store refusal spools the batch instead
    /// of losing it.
    async fn write(&self, rows: &[RequestLogRow]) -> Result<WriteOutcome, String> {
        if rows.is_empty() {
            return Ok(WriteOutcome::default());
        }
        match self.store.insert_request_logs(rows).await {
            Ok(_) => Ok(WriteOutcome {
                written: rows.len(),
                spooled: 0,
            }),
            Err(error) => {
                let mut lines = String::new();
                for row in rows {
                    let line = serde_json::to_string(row)
                        .map_err(|serialize| format!("cannot spool row: {serialize}"))?;
                    lines.push_str(&line);
                    lines.push('\n');
                }
                self.append_lines(&self.spool_path, &lines).await?;
                eprintln!(
                    "store refused the batch ({error}); {} rows spooled to {}",
                    rows.len(),
                    self.spool_path.display()
                );
                Ok(WriteOutcome {
                    written: 0,
                    spooled: rows.len(),
                })
            }
        }
    }

    /// Re-deliver every spooled row, then truncate. Corrupt lines move to
    /// `<spool>.dead`. A failed delivery keeps the undelivered lines spooled.
    async fn replay(&self) -> Result<ReplayOutcome, String> {
        let text = match tokio::fs::read_to_string(&self.spool_path).await {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ReplayOutcome::default())
            }
            Err(error) => {
                return Err(format!(
                    "cannot read {}: {error}",
                    self.spool_path.display()
                ))
            }
        };
        let mut rows: Vec<RequestLogRow> = Vec::new();
        let mut deliverable_lines: Vec<&str> = Vec::new();
        let mut corrupt_lines: Vec<&str> = Vec::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            match serde_json::from_str::<RequestLogRow>(line) {
                Ok(row) => {
                    rows.push(row);
                    deliverable_lines.push(line);
                }
                Err(_) => corrupt_lines.push(line),
            }
        }
        let corrupt = corrupt_lines.len();
        if !corrupt_lines.is_empty() {
            let mut dead = corrupt_lines.join("\n");
            dead.push('\n');
            self.append_lines(&self.dead_path(), &dead).await?;
        }
        if !rows.is_empty() {
            if let Err(error) = self.store.insert_request_logs(&rows).await {
                // Keep every undelivered line spooled; nothing is lost.
                let remaining = if deliverable_lines.is_empty() {
                    String::new()
                } else {
                    format!("{}\n", deliverable_lines.join("\n"))
                };
                tokio::fs::write(&self.spool_path, remaining)
                    .await
                    .map_err(|write_error| {
                        format!("cannot keep {}: {write_error}", self.spool_path.display())
                    })?;
                return Err(error.to_string());
            }
        }
        tokio::fs::write(&self.spool_path, "")
            .await
            .map_err(|error| format!("cannot truncate {}: {error}", self.spool_path.display()))?;
        Ok(ReplayOutcome {
            replayed: rows.len(),
            corrupt,
        })
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => options.check = true,
            "--json" => options.json = true,
            "--spool" => {
                index += 1;
                let Some(path) = args.get(index) else {
                    return Err("--spool needs a path".to_string());
                };
                options.spool = Some(PathBuf::from(path));
            }
            "--help" | "-h" => {
                return Err(
                    "usage: apex-ingest [--check] [--json] [--spool <path>] [<access-log.json>]"
                        .to_string(),
                )
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag '{other}'"));
            }
            other => {
                if options.path.is_some() {
                    return Err("only one input path is accepted; pipe for stdin".to_string());
                }
                options.path = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }
    Ok(options)
}

fn environment_label(lookup: &dyn Fn(&str) -> Option<String>) -> String {
    lookup("APEX_ENVIRONMENT")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "production".to_string())
}

fn service_version_label(lookup: &dyn Fn(&str) -> Option<String>) -> String {
    lookup("APEX_SERVICE_VERSION")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("apex-ingest-{}", env!("CARGO_PKG_VERSION")))
}

fn spool_path(options: &Options, lookup: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(path) = &options.spool {
        return path.clone();
    }
    lookup("APEX_INGEST_SPOOL")
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".apex/analytics-spool/apex-ingest.jsonl"))
}

async fn read_input(options: &Options) -> Result<String, String> {
    match &options.path {
        Some(path) => tokio::fs::read_to_string(path)
            .await
            .map_err(|error| format!("cannot read {}: {error}", path.display())),
        None => {
            use tokio::io::AsyncReadExt;
            let mut text = String::new();
            tokio::io::stdin()
                .read_to_string(&mut text)
                .await
                .map_err(|error| format!("cannot read stdin: {error}"))?;
            Ok(text)
        }
    }
}

fn print_summary(summary: &Summary, json: bool) {
    if json {
        match serde_json::to_string_pretty(summary) {
            Ok(text) => println!("{text}"),
            Err(error) => eprintln!("summary serialization failed: {error}"),
        }
        return;
    }
    println!(
        "apex-ingest: {} lines, {} request logs, {} skipped, {} rejected",
        summary.lines, summary.request_logs, summary.skipped_non_request, summary.rejected
    );
    if summary.check_only {
        println!("check only: no store contacted, nothing written");
    } else {
        println!(
            "delivered {}, spooled {} ({})",
            summary.delivered, summary.spooled, summary.spool_path
        );
    }
    for error in &summary.errors {
        println!("rejected: {error}");
    }
}

async fn run(options: Options) -> Result<ExitCode, String> {
    let lookup = |key: &str| std::env::var(key).ok();
    let environment = environment_label(&lookup);
    let service_version = service_version_label(&lookup);
    let received_at =
        UtcMillis::system_now().map_err(|error| format!("system clock unavailable: {error}"))?;

    let text = read_input(&options).await?;
    let input_label = options
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "stdin".to_string());

    let defaults = AccessLogDefaults {
        environment: environment.clone(),
        service_version: service_version.clone(),
        received_at,
    };
    let mut summary = Summary {
        input: input_label,
        check_only: options.check,
        lines: 0,
        request_logs: 0,
        skipped_non_request: 0,
        rejected: 0,
        delivered: 0,
        spooled: 0,
        spool_path: spool_path(&options, &lookup).display().to_string(),
        errors: Vec::new(),
    };
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        summary.lines += 1;
        match parse_line(line, &defaults) {
            Ok(Some(row)) => {
                summary.request_logs += 1;
                rows.push(row);
            }
            Ok(None) => summary.skipped_non_request += 1,
            Err(error) => {
                summary.rejected += 1;
                if summary.errors.len() < 10 {
                    summary.errors.push(format!("line {}: {error}", index + 1));
                }
            }
        }
    }

    if options.check {
        print_summary(&summary, options.json);
        return Ok(if summary.rejected > 0 {
            ExitCode::from(3)
        } else {
            ExitCode::SUCCESS
        });
    }

    // A check-mode-free path: build the store and deliver.
    let config = ClickHouseConfig::from_lookup(&lookup)
        .map_err(|error| format!("store configuration: {error}"))?;
    let store =
        ClickHouseStore::new(config).map_err(|error| format!("store configuration: {error}"))?;
    store
        .ensure_schema()
        .await
        .map_err(|error| format!("schema setup failed: {error}"))?;
    let writer = RequestLogWriter::new(store, spool_path(&options, &lookup));

    // Opportunistically drain anything a previous run spooled. A failed
    // replay keeps those lines spooled and is reported, never dropped.
    match writer.replay().await {
        Ok(replayed) if replayed.replayed > 0 || replayed.corrupt > 0 => {
            println!(
                "spool replay: {} delivered, {} corrupt lines preserved to {}.dead",
                replayed.replayed,
                replayed.corrupt,
                writer.spool_path().display()
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("spool replay deferred: {error}"),
    }

    for batch in rows.chunks(BATCH_SIZE) {
        let outcome = writer
            .write(batch)
            .await
            .map_err(|error| format!("delivery failed and spooling failed too: {error}"))?;
        summary.delivered += outcome.written as u64;
        summary.spooled += outcome.spooled as u64;
    }

    print_summary(&summary, options.json);
    Ok(if summary.spooled > 0 || summary.rejected > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(options).await {
        Ok(code) => code,
        Err(message) => {
            eprintln!("apex-ingest: {message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn arguments_parse_flags_and_one_path() {
        let options = parse_args(&[
            "--json".to_string(),
            "--spool".to_string(),
            "/tmp/spool.jsonl".to_string(),
            "access.json".to_string(),
        ])
        .expect("parse");
        assert!(options.json);
        assert!(!options.check);
        assert_eq!(options.path, Some(PathBuf::from("access.json")));
        assert_eq!(options.spool, Some(PathBuf::from("/tmp/spool.jsonl")));

        let check = parse_args(&["--check".to_string()]).expect("check");
        assert!(check.check);
        assert_eq!(check.path, None);
    }

    #[test]
    fn bad_arguments_are_refused() {
        assert!(parse_args(&["--spool".to_string()]).is_err());
        assert!(parse_args(&["--nope".to_string()]).is_err());
        assert!(parse_args(&["a".to_string(), "b".to_string()]).is_err());
    }

    #[test]
    fn labels_default_and_override() {
        let none = |_key: &str| None;
        assert_eq!(environment_label(&none), "production");
        assert!(service_version_label(&none).starts_with("apex-ingest-"));
        let staging = |key: &str| (key == "APEX_ENVIRONMENT").then(|| "staging".to_string());
        assert_eq!(environment_label(&staging), "staging");
        let blank = |_key: &str| Some("  ".to_string());
        assert_eq!(environment_label(&blank), "production");
    }

    fn request_row(id: &str) -> RequestLogRow {
        RequestLogRow {
            event_id: id.to_string(),
            occurred_at_utc: UtcMillis::from_unix_millis(1_792_055_400_125),
            received_at_utc: UtcMillis::from_unix_millis(1_792_055_401_000),
            route_template: "/pricing".to_string(),
            status: 200,
            status_class: "2xx".to_string(),
            bot_class: "unclassified".to_string(),
            referrer_host: Some("example.com".to_string()),
            campaign_code: None,
            query_keys: vec!["utm_source".to_string()],
            environment: "production".to_string(),
        }
    }

    fn spool_file(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("apex-ingest-{label}-{}.jsonl", std::process::id()))
    }

    #[tokio::test]
    async fn delivery_writes_sanitized_rows_into_the_store() {
        use apex_analytics::store::MemoryStore;
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        let spool = spool_file("deliver");
        let writer = RequestLogWriter::new(store.clone(), spool.clone());
        let rows = vec![request_row("line_1"), request_row("line_2")];
        let outcome = writer.write(&rows).await.expect("write");
        assert_eq!(outcome.written, 2);
        assert_eq!(outcome.spooled, 0);
        assert_eq!(store.request_logs(), rows);
        assert!(tokio::fs::metadata(&spool).await.is_err(), "no spool file");
    }

    #[tokio::test]
    async fn a_failed_delivery_spools_rows_and_replay_delivers_them() {
        use apex_analytics::store::MemoryStore;
        let store = MemoryStore::new();
        let spool = spool_file("spool");
        let _ = std::fs::remove_file(&spool);
        let writer = RequestLogWriter::new(store.clone(), spool.clone());

        // The schema is missing, so the write fails and the batch spools.
        let rows = vec![request_row("line_1"), request_row("line_2")];
        let outcome = writer.write(&rows).await.expect("spooled");
        assert_eq!(outcome.written, 0);
        assert_eq!(outcome.spooled, 2);
        let spooled = tokio::fs::read_to_string(&spool).await.expect("spool read");
        assert_eq!(spooled.lines().count(), 2);
        assert!(spooled.contains("\"route_template\":\"/pricing\""));

        // Replaying against the cold store fails and keeps the spool.
        assert!(writer.replay().await.is_err());
        let still = tokio::fs::read_to_string(&spool).await.expect("spool kept");
        assert_eq!(still.lines().count(), 2);

        // Warm the store; the replay delivers and truncates.
        store.ensure_schema().await.expect("schema");
        let replayed = writer.replay().await.expect("replay");
        assert_eq!(replayed.replayed, 2);
        assert_eq!(replayed.corrupt, 0);
        assert_eq!(store.request_logs(), rows);
        let truncated = tokio::fs::read_to_string(&spool)
            .await
            .expect("empty spool");
        assert!(truncated.is_empty());

        // Replaying an absent spool is a no-op.
        let _ = std::fs::remove_file(&spool);
        let replayed = writer.replay().await.expect("no spool");
        assert_eq!(replayed.replayed, 0);
        let _ = std::fs::remove_file(&spool);
    }

    #[tokio::test]
    async fn corrupt_spool_lines_are_preserved_to_dead() {
        use apex_analytics::store::MemoryStore;
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        let spool = spool_file("dead");
        let good = serde_json::to_string(&request_row("line_1")).expect("row json");
        tokio::fs::write(&spool, format!("{good}\nnot json at all\n"))
            .await
            .expect("write spool");
        let writer = RequestLogWriter::new(store.clone(), spool.clone());
        let replayed = writer.replay().await.expect("replay");
        assert_eq!(replayed.replayed, 1);
        assert_eq!(replayed.corrupt, 1);
        assert_eq!(store.request_logs().len(), 1);
        let dead = tokio::fs::read_to_string(writer.dead_path())
            .await
            .expect("dead file");
        assert_eq!(dead.trim(), "not json at all");
        let _ = std::fs::remove_file(&spool);
        let _ = std::fs::remove_file(writer.dead_path());
    }
}
