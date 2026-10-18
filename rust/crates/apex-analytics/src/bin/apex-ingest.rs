//! `apex-ingest` materializes Caddy JSON access logs as sanitized rows in
//! the analytics `request_logs` table (30-day retention per the spec) AND as
//! `public_page_requested` domain events in `domain_events` (90-day
//! retention): one sanitized row plus one bounded event per request line.
//! The sanitized fields (route template, status, referrer host, campaign
//! code, allow-listed query keys, bot class) are exactly the `request_logs`
//! columns; client addresses, raw user agents and query values never reach
//! the store. The domain event carries only the bounded classification
//! facts (route template, status, bot class and the campaign KEYS, never
//! values or the referrer host), so a replay of one line derives the same
//! stable event id and the `(source, event_id)` dedup key absorbs it.
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
//! A file input is consumed incrementally through a cursor: only lines after
//! the last durably handled one are parsed and delivered, so the shipped
//! hourly timer does not re-insert the whole file every run. The cursor is
//! written (atomically, via a temp file and a rename) only after every
//! consumed line was either delivered or preserved in the spool, and it
//! records the byte offset plus a short identity (length and hash of the
//! first line) of the file it belongs to: a replaced or shrunken file
//! restarts at byte 0 and its lines are reprocessed, where the stable event
//! id deduplicates replayed events while `request_logs` (a plain MergeTree)
//! may hold the duplicate row. An unterminated final line is held back until
//! a newline arrives; `--check` never reads or writes a cursor, and stdin
//! input has no cursor at all.
//!
//! Environment:
//!
//! - `APEX_CLICKHOUSE_HTTP` (default `http://127.0.0.1:8123`)
//! - `APEX_CLICKHOUSE_DB` (default `apex_analytics`)
//! - `APEX_ENVIRONMENT` (default `production`)
//! - `APEX_SERVICE_VERSION` (default `apex-ingest-<crate version>`)
//! - `APEX_INGEST_SPOOL` (default `.apex/analytics-spool/apex-ingest.jsonl`)
//! - `APEX_INGEST_CURSOR` (default `<input>.cursor`; file inputs only)
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

use apex_analytics::accesslog::{event_for_row, parse_line, AccessLogDefaults};
use apex_analytics::store::{ClickHouseConfig, ClickHouseStore, EventStore, RequestLogRow};
use apex_analytics::timestamp::UtcMillis;
use apex_analytics::DomainEvent;

/// How many events one insert batch carries.
const BATCH_SIZE: usize = 500;

/// How many bytes of the first line identify a log file across rotations.
const FILE_IDENTITY_BYTES: usize = 4_096;

#[cfg(test)]
thread_local! {
    /// File bytes read while planning and reading one input segment. An
    /// incremental run must read the identity prefix plus the new tail, not
    /// the whole growing log.
    static INPUT_BYTES_READ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_input_bytes_read() {
    INPUT_BYTES_READ.with(|count| count.set(0));
}

#[cfg(test)]
fn input_bytes_read() -> u64 {
    INPUT_BYTES_READ.with(std::cell::Cell::get)
}

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
    /// `public_page_requested` events built from those rows.
    events: u64,
    skipped_non_request: u64,
    rejected: u64,
    /// Request rows written into `request_logs`.
    delivered: u64,
    /// Events written into `domain_events`.
    events_delivered: u64,
    spooled: u64,
    spool_path: String,
    /// The consumed-offset cursor; absent for stdin and `--check` runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor_path: Option<String>,
    /// First byte this run read from (0 after a rotation or a fresh file).
    cursor_from: u64,
    /// First byte the next run reads from (the durable consumption point).
    cursor_to: u64,
    /// Whether a rotated or shrunken file forced a restart at byte 0.
    cursor_restarted: bool,
    /// First rejection reasons (bounded; the count above is complete).
    errors: Vec<String>,
}

/// What one batch write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct WriteOutcome {
    rows_written: usize,
    events_written: usize,
    spooled: usize,
}

/// What a spool replay did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ReplayOutcome {
    replayed: usize,
    corrupt: usize,
}

/// The consumed-offset cursor beside a file input: how many bytes of the
/// file are durably handled and which file those bytes belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileCursor {
    offset: u64,
    identity: String,
}

impl FileCursor {
    /// The one-line cursor file format, versioned so a future change is
    /// detectable rather than misread.
    fn render(&self) -> String {
        format!("v1 {} {}\n", self.offset, self.identity)
    }

    /// Parse a cursor file; anything unreadable yields `None` and the run
    /// restarts at byte 0.
    fn parse(text: &str) -> Option<Self> {
        let mut fields = text.split_whitespace();
        if fields.next() != Some("v1") {
            return None;
        }
        let offset = fields.next()?.parse().ok()?;
        let identity = fields.next()?;
        if fields.next().is_some() || identity.is_empty() {
            return None;
        }
        Some(Self {
            offset,
            identity: identity.to_string(),
        })
    }
}

/// FNV-1a 64-bit hex digest, used only to identify a file, never as a
/// security primitive.
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut state: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{state:016x}")
}

/// A short, append-stable file identity: the length and hash of the first
/// line (bounded to [`FILE_IDENTITY_BYTES`]). Appending lines leaves the
/// first line untouched, while a rotated file starts with a different line;
/// the line's content itself never leaves this function.
#[cfg(test)]
fn file_identity(text: &str) -> Option<String> {
    file_identity_bytes(text.as_bytes())
}

/// Byte form of [`file_identity`], so the identity can be computed from a
/// bounded prefix read instead of the whole file.
fn file_identity_bytes(bytes: &[u8]) -> Option<String> {
    let first_line = bytes.split_inclusive(|byte| *byte == b'\n').next()?;
    if first_line.is_empty() {
        return None;
    }
    let bounded = &first_line[..first_line.len().min(FILE_IDENTITY_BYTES)];
    Some(format!("{}-{}", bounded.len(), fnv1a_hex(bounded)))
}

/// Where one run starts reading and the cursor it may advance.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadPlan {
    cursor_path: Option<PathBuf>,
    start: usize,
    identity: Option<String>,
    restarted: bool,
}

impl ReadPlan {
    /// The plan for stdin or `--check` runs: everything, no cursor.
    fn whole() -> Self {
        Self {
            cursor_path: None,
            start: 0,
            identity: None,
            restarted: false,
        }
    }
}

/// The cursor file of a run: `APEX_INGEST_CURSOR` or `<input>.cursor`; stdin
/// input has none.
fn cursor_path(options: &Options, lookup: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let path = options.path.as_ref()?;
    if let Some(configured) = lookup("APEX_INGEST_CURSOR").filter(|value| !value.trim().is_empty())
    {
        return Some(PathBuf::from(configured));
    }
    let mut name = path.as_os_str().to_os_string();
    name.push(".cursor");
    Some(PathBuf::from(name))
}

/// Decide where this run starts reading a file from, based on the stored
/// cursor, the file's identity and its length. A missing cursor starts at 0;
/// a cursor whose file was replaced (identity differs), shrank past the
/// offset, or does not sit on a line boundary restarts at 0. The line
/// boundary itself is checked by [`read_file_segment`], which reads at most
/// the identity prefix and the new tail instead of the whole log.
async fn plan_read(
    options: &Options,
    lookup: &dyn Fn(&str) -> Option<String>,
    identity: Option<String>,
    file_len: u64,
) -> Result<ReadPlan, String> {
    let Some(cursor_path) = cursor_path(options, lookup) else {
        return Ok(ReadPlan::whole());
    };
    let stored = match tokio::fs::read_to_string(&cursor_path).await {
        Ok(text) => FileCursor::parse(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "cannot read cursor {}: {error}",
                cursor_path.display()
            ))
        }
    };
    let restart = |restarted: bool| ReadPlan {
        cursor_path: Some(cursor_path.clone()),
        start: 0,
        identity: identity.clone(),
        restarted,
    };
    match stored {
        Some(cursor)
            if Some(&cursor.identity) == identity.as_ref() && cursor.offset <= file_len =>
        {
            Ok(ReadPlan {
                cursor_path: Some(cursor_path),
                start: cursor.offset as usize,
                identity,
                restarted: false,
            })
        }
        Some(_) => Ok(restart(true)),
        None => Ok(restart(false)),
    }
}

/// Read the bounded identity prefix of a file.
async fn read_file_prefix(path: &Path) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut prefix = Vec::with_capacity(FILE_IDENTITY_BYTES);
    file.take(FILE_IDENTITY_BYTES as u64)
        .read_to_end(&mut prefix)
        .await
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    #[cfg(test)]
    INPUT_BYTES_READ.with(|count| count.set(count.get() + prefix.len() as u64));
    Ok(prefix)
}

/// Read one file segment starting at `offset` (a cursor offset). `None` means
/// the offset is not a line boundary or does not decode as UTF-8, so the
/// caller must restart at byte 0. Only one byte before the offset and the
/// tail after it are read.
async fn read_file_segment(path: &Path, offset: u64) -> Result<Option<String>, String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if offset > 0 {
        let mut previous = [0u8; 1];
        file.seek(std::io::SeekFrom::Start(offset - 1))
            .await
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        file.read_exact(&mut previous)
            .await
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        #[cfg(test)]
        INPUT_BYTES_READ.with(|count| count.set(count.get() + 1));
        if previous[0] != b'\n' {
            return Ok(None);
        }
    }
    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut text = String::new();
    match file.read_to_string(&mut text).await {
        Ok(_) => {
            #[cfg(test)]
            INPUT_BYTES_READ.with(|count| count.set(count.get() + text.len() as u64));
            Ok(Some(text))
        }
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// Plan and read one file input: the identity prefix plus the cursor tail,
/// never the whole log unless the cursor is missing or invalid.
async fn read_file_input(
    options: &Options,
    lookup: &dyn Fn(&str) -> Option<String>,
    path: &Path,
) -> Result<(String, ReadPlan), String> {
    let len = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?
        .len();
    let identity = file_identity_bytes(&read_file_prefix(path).await?);
    let mut plan = plan_read(options, lookup, identity, len).await?;
    match read_file_segment(path, plan.start as u64).await? {
        Some(text) => Ok((text, plan)),
        None if plan.start == 0 => Err(format!(
            "cannot read {}: the file is not valid UTF-8",
            path.display()
        )),
        None => {
            plan.start = 0;
            plan.restarted = true;
            let text = read_file_segment(path, 0).await?.ok_or_else(|| {
                format!(
                    "cannot read {}: the file is not valid UTF-8",
                    path.display()
                )
            })?;
            Ok((text, plan))
        }
    }
}

/// Replace `path` through a `<path>.tmp` sibling and a rename: a crash
/// mid-write leaves either the old complete file or the new complete file,
/// never a truncated one.
async fn replace_file(path: &Path, contents: &str) -> Result<(), String> {
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = PathBuf::from(tmp_name);
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|error| format!("cannot write {}: {error}", tmp_path.display()))?;
    file.write_all(contents.as_bytes())
        .await
        .map_err(|error| format!("cannot write {}: {error}", tmp_path.display()))?;
    file.flush()
        .await
        .map_err(|error| format!("cannot write {}: {error}", tmp_path.display()))?;
    drop(file);
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

/// Write the consumed-offset cursor of a file input.
async fn write_cursor(path: &Path, cursor: &FileCursor) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    replace_file(path, &cursor.render()).await
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

    /// The `public_page_requested` events a batch of sanitized rows stands
    /// for. The event is a pure function of the row's bounded fields, so the
    /// parse path and a spool replay derive the same stable event id.
    fn events_for(
        &self,
        rows: &[RequestLogRow],
        defaults: &AccessLogDefaults,
    ) -> Result<Vec<DomainEvent>, String> {
        rows.iter()
            .map(|row| {
                event_for_row(row, defaults)
                    .map_err(|error| format!("cannot build the page-request event: {error}"))
            })
            .collect()
    }

    /// Spool a refused batch (rows only; a replay rebuilds the events).
    async fn spool(&self, rows: &[RequestLogRow], error: &str) -> Result<WriteOutcome, String> {
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
            rows_written: 0,
            events_written: 0,
            spooled: rows.len(),
        })
    }

    /// Keep the undelivered spool lines in place through an atomic replace.
    async fn keep_spooled(&self, deliverable_lines: &[&str]) -> Result<(), String> {
        let remaining = if deliverable_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", deliverable_lines.join("\n"))
        };
        replace_file(&self.spool_path, &remaining).await
    }

    /// Insert one batch: the `public_page_requested` events first, then the
    /// `request_logs` rows. The events go first because a later row refusal
    /// spools the batch and the replay re-inserts the events, which the
    /// `(source, event_id)` dedup key collapses; the reverse order could
    /// duplicate log rows the same way. A store refusal spools the rows
    /// instead of losing them.
    async fn write(
        &self,
        rows: &[RequestLogRow],
        defaults: &AccessLogDefaults,
    ) -> Result<WriteOutcome, String> {
        if rows.is_empty() {
            return Ok(WriteOutcome::default());
        }
        let events = self.events_for(rows, defaults)?;
        if let Err(error) = self.store.insert(&events).await {
            return self.spool(rows, &error.to_string()).await;
        }
        match self.store.insert_request_logs(rows).await {
            Ok(_) => Ok(WriteOutcome {
                rows_written: rows.len(),
                events_written: events.len(),
                spooled: 0,
            }),
            Err(error) => self.spool(rows, &error.to_string()).await,
        }
    }

    /// Re-deliver every spooled row and its event, then truncate atomically.
    /// Corrupt lines move to `<spool>.dead`. A failed delivery keeps every
    /// undelivered line spooled through an atomic replace.
    async fn replay(&self, defaults: &AccessLogDefaults) -> Result<ReplayOutcome, String> {
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
            let events = self.events_for(&rows, defaults)?;
            if let Err(error) = self.store.insert(&events).await {
                // Keep every undelivered line spooled; nothing is lost.
                self.keep_spooled(&deliverable_lines).await?;
                return Err(error.to_string());
            }
            if let Err(error) = self.store.insert_request_logs(&rows).await {
                self.keep_spooled(&deliverable_lines).await?;
                return Err(error.to_string());
            }
        }
        replace_file(&self.spool_path, "").await?;
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
        "apex-ingest: {} lines, {} request logs, {} events, {} skipped, {} rejected",
        summary.lines,
        summary.request_logs,
        summary.events,
        summary.skipped_non_request,
        summary.rejected
    );
    if summary.check_only {
        println!("check only: no store contacted, nothing written, no cursor touched");
    } else {
        println!(
            "delivered {} rows + {} events, spooled {} ({})",
            summary.delivered, summary.events_delivered, summary.spooled, summary.spool_path
        );
    }
    if let Some(cursor_path) = &summary.cursor_path {
        println!(
            "cursor: {cursor_path} bytes {} -> {}",
            summary.cursor_from, summary.cursor_to
        );
        if summary.cursor_restarted {
            println!("cursor restarted at byte 0: the file was rotated or shrank");
        }
    }
    for error in &summary.errors {
        println!("rejected: {error}");
    }
}

/// The full pipeline: read the input (from the cursor for file inputs),
/// parse the consumed segment, replay any spooled rows, deliver rows and
/// events, and advance the cursor only after everything is durably handled.
///
/// `store` is `None` only for `--check` runs, which contact nothing and
/// write nothing (including no cursor).
async fn run_with_store<S: EventStore>(
    options: &Options,
    lookup: &dyn Fn(&str) -> Option<String>,
    store: Option<S>,
) -> Result<(ExitCode, Summary), String> {
    let environment = environment_label(lookup);
    let service_version = service_version_label(lookup);
    let received_at =
        UtcMillis::system_now().map_err(|error| format!("system clock unavailable: {error}"))?;

    let (text, plan) = if options.check {
        (read_input(options).await?, ReadPlan::whole())
    } else if let Some(path) = &options.path {
        // Incremental file runs read only the identity prefix and the tail
        // after the cursor; a growing access log is never re-read in full.
        read_file_input(options, lookup, path).await?
    } else {
        (read_input(options).await?, ReadPlan::whole())
    };
    let input_label = options
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "stdin".to_string());

    let defaults = AccessLogDefaults {
        environment,
        service_version,
        received_at,
    };
    let mut summary = Summary {
        input: input_label,
        check_only: options.check,
        lines: 0,
        request_logs: 0,
        events: 0,
        skipped_non_request: 0,
        rejected: 0,
        delivered: 0,
        events_delivered: 0,
        spooled: 0,
        spool_path: spool_path(options, lookup).display().to_string(),
        cursor_path: plan
            .cursor_path
            .as_ref()
            .map(|path| path.display().to_string()),
        cursor_from: plan.start as u64,
        cursor_to: plan.start as u64,
        cursor_restarted: plan.restarted,
        errors: Vec::new(),
    };

    // Consume the segment line by line. An unterminated final line is held
    // back so a partially written log line is never parsed; it is picked up
    // by the next run once its newline arrives.
    let mut consumed: u64 = 0;
    let mut consumed_lines: u64 = 0;
    let mut rows = Vec::new();
    let mut rest = text.as_str();
    while let Some(newline) = rest.find('\n') {
        let line = rest[..newline]
            .strip_suffix('\r')
            .unwrap_or(&rest[..newline]);
        rest = &rest[newline + 1..];
        consumed += newline as u64 + 1;
        consumed_lines += 1;
        if line.trim().is_empty() {
            continue;
        }
        summary.lines += 1;
        match parse_line(line, &defaults) {
            Ok(Some(row)) => {
                summary.request_logs += 1;
                summary.events += 1;
                rows.push(row);
            }
            Ok(None) => summary.skipped_non_request += 1,
            Err(error) => {
                summary.rejected += 1;
                if summary.errors.len() < 10 {
                    summary
                        .errors
                        .push(format!("line {consumed_lines}: {error}"));
                }
            }
        }
    }

    if options.check {
        print_summary(&summary, options.json);
        return Ok((
            if summary.rejected > 0 {
                ExitCode::from(3)
            } else {
                ExitCode::SUCCESS
            },
            summary,
        ));
    }

    let store = store.ok_or_else(|| "a delivery run needs a store".to_string())?;
    store
        .ensure_schema()
        .await
        .map_err(|error| format!("schema setup failed: {error}"))?;
    let writer = RequestLogWriter::new(store, spool_path(options, lookup));

    // Opportunistically drain anything a previous run spooled. A failed
    // replay keeps those lines spooled and is reported, never dropped.
    match writer.replay(&defaults).await {
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
            .write(batch, &defaults)
            .await
            .map_err(|error| format!("delivery failed and spooling failed too: {error}"))?;
        summary.delivered += outcome.rows_written as u64;
        summary.events_delivered += outcome.events_written as u64;
        summary.spooled += outcome.spooled as u64;
    }

    // Advance the cursor only now: every consumed line was delivered or
    // durably preserved in the spool, so a crash before this point re-reads
    // the segment and the stable event id absorbs the replay.
    if consumed > 0 {
        if let (Some(cursor_path), Some(identity)) = (&plan.cursor_path, plan.identity.clone()) {
            let cursor = FileCursor {
                offset: summary.cursor_from + consumed,
                identity,
            };
            write_cursor(cursor_path, &cursor).await?;
            summary.cursor_to = cursor.offset;
        }
    }

    print_summary(&summary, options.json);
    Ok((
        if summary.spooled > 0 || summary.rejected > 0 {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        },
        summary,
    ))
}

async fn run(options: Options) -> Result<(ExitCode, Summary), String> {
    let lookup = |key: &str| std::env::var(key).ok();
    if options.check {
        return run_with_store(&options, &lookup, None::<ClickHouseStore>).await;
    }
    let config = ClickHouseConfig::from_lookup(&lookup)
        .map_err(|error| format!("store configuration: {error}"))?;
    let store =
        ClickHouseStore::new(config).map_err(|error| format!("store configuration: {error}"))?;
    run_with_store(&options, &lookup, Some(store)).await
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
        Ok((code, _summary)) => code,
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
    use apex_analytics::ProjectionStore;

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
        // Distinct occurrence times per id, like distinct log lines: the
        // derived event id is a function of the bounded row fields.
        let offset = id.bytes().map(i64::from).sum::<i64>();
        RequestLogRow {
            event_id: id.to_string(),
            occurred_at_utc: UtcMillis::from_unix_millis(1_792_055_400_125 + offset),
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

    fn defaults() -> AccessLogDefaults {
        AccessLogDefaults {
            environment: "production".to_string(),
            service_version: "apex-ingest-test".to_string(),
            received_at: UtcMillis::from_unix_millis(1_792_055_401_000),
        }
    }

    fn spool_file(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("apex-ingest-{label}-{}.jsonl", std::process::id()))
    }

    /// A fresh working directory for one file-input test.
    fn test_directory(label: &str) -> PathBuf {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "apex-ingest-{label}-{}-{sequence}",
            std::process::id()
        ))
    }

    fn tmp_sibling(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_os_string();
        name.push(".tmp");
        PathBuf::from(name)
    }

    fn log_line(uri: &str, status: u16, ts_seconds: f64) -> String {
        serde_json::json!({
            "ts": ts_seconds,
            "status": status,
            "request": {
                "method": "GET",
                "host": "apexmediation.ee",
                "uri": uri,
                "remote_ip": "203.0.113.9",
                "headers": { "User-Agent": ["Mozilla/5.0 Safari/605.1.15"] }
            }
        })
        .to_string()
    }

    /// A lookup that pins the cursor and the spool inside a test directory.
    fn test_env<'a>(cursor: &'a Path, spool: &'a Path) -> impl Fn(&str) -> Option<String> + 'a {
        move |key: &str| match key {
            "APEX_INGEST_CURSOR" => Some(cursor.display().to_string()),
            "APEX_INGEST_SPOOL" => Some(spool.display().to_string()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn delivery_writes_sanitized_rows_and_events_into_the_store() {
        use apex_analytics::store::MemoryStore;
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        let spool = spool_file("deliver");
        let writer = RequestLogWriter::new(store.clone(), spool.clone());
        let rows = vec![request_row("line_1"), request_row("line_2")];
        let outcome = writer.write(&rows, &defaults()).await.expect("write");
        assert_eq!(outcome.rows_written, 2);
        assert_eq!(outcome.events_written, 2);
        assert_eq!(outcome.spooled, 0);
        assert_eq!(store.request_logs(), rows);
        let events = store.load_events().await.expect("events");
        assert_eq!(events.len(), 2, "one public_page_requested per row");
        for event in &events {
            assert_eq!(
                event.event_type,
                apex_analytics::envelope::EventType::PublicPageRequested
            );
            assert_eq!(
                event.source,
                apex_analytics::envelope::EventSource::AccessLog
            );
        }
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
        let outcome = writer.write(&rows, &defaults()).await.expect("spooled");
        assert_eq!(outcome.rows_written, 0);
        assert_eq!(outcome.events_written, 0);
        assert_eq!(outcome.spooled, 2);
        let spooled = tokio::fs::read_to_string(&spool).await.expect("spool read");
        assert_eq!(spooled.lines().count(), 2);
        assert!(spooled.contains("\"route_template\":\"/pricing\""));
        assert!(
            !tmp_sibling(&spool).exists(),
            "the spool is replaced atomically, never left as a temp file"
        );

        // Replaying against the cold store fails and keeps the spool.
        assert!(writer.replay(&defaults()).await.is_err());
        let still = tokio::fs::read_to_string(&spool).await.expect("spool kept");
        assert_eq!(still.lines().count(), 2);
        assert!(
            !tmp_sibling(&spool).exists(),
            "an interrupted keep leaves no temp file"
        );

        // Warm the store; the replay delivers rows AND events, then truncates.
        store.ensure_schema().await.expect("schema");
        let replayed = writer.replay(&defaults()).await.expect("replay");
        assert_eq!(replayed.replayed, 2);
        assert_eq!(replayed.corrupt, 0);
        assert_eq!(store.request_logs(), rows);
        assert_eq!(store.load_events().await.expect("events").len(), 2);
        let truncated = tokio::fs::read_to_string(&spool)
            .await
            .expect("empty spool");
        assert!(truncated.is_empty());
        assert!(!tmp_sibling(&spool).exists());

        // Replaying an absent spool is a no-op.
        let _ = std::fs::remove_file(&spool);
        let replayed = writer.replay(&defaults()).await.expect("no spool");
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
        let replayed = writer.replay(&defaults()).await.expect("replay");
        assert_eq!(replayed.replayed, 1);
        assert_eq!(replayed.corrupt, 1);
        assert_eq!(store.request_logs().len(), 1);
        assert_eq!(store.load_events().await.expect("events").len(), 1);
        let dead = tokio::fs::read_to_string(writer.dead_path())
            .await
            .expect("dead file");
        assert_eq!(dead.trim(), "not json at all");
        let _ = std::fs::remove_file(&spool);
        let _ = std::fs::remove_file(writer.dead_path());
    }

    #[test]
    fn cursor_files_round_trip_and_refuse_tampering() {
        let cursor = FileCursor {
            offset: 42,
            identity: "10-abcd".to_string(),
        };
        assert_eq!(FileCursor::parse(&cursor.render()), Some(cursor));
        assert_eq!(FileCursor::parse(""), None);
        assert_eq!(FileCursor::parse("v2 1 x"), None);
        assert_eq!(FileCursor::parse("v1 x y"), None);
        assert_eq!(FileCursor::parse("v1 1"), None);
        assert_eq!(FileCursor::parse("v1 1 x y"), None);
        assert_eq!(FileCursor::parse("v1 1 "), None);
    }

    #[test]
    fn file_identities_track_appends_and_rotations() {
        let first = log_line("/pricing", 200, 10.0);
        let second = log_line("/proof", 404, 11.0);
        let appended = format!("{first}\n{second}\n");
        let extended = format!("{first}\n{second}\n{}\n", log_line("/", 200, 12.0));
        assert_eq!(
            file_identity(&appended),
            file_identity(&extended),
            "an append never changes the identity"
        );
        let rotated = format!("{}\n", log_line("/other", 200, 13.0));
        assert_ne!(file_identity(&appended), file_identity(&rotated));
        assert_eq!(file_identity(""), None);
        let huge_first_line = "x".repeat(10_000);
        let identity = file_identity(&huge_first_line).expect("identity");
        assert!(identity.starts_with("4096-"), "{identity}");
    }

    #[test]
    fn cursor_paths_default_beside_the_input() {
        let options = Options {
            path: Some(PathBuf::from("/var/log/caddy/access.json")),
            ..Options::default()
        };
        let none = |_key: &str| None;
        assert_eq!(
            cursor_path(&options, &none),
            Some(PathBuf::from("/var/log/caddy/access.json.cursor"))
        );
        let configured =
            |key: &str| (key == "APEX_INGEST_CURSOR").then(|| "/tmp/custom-cursor".to_string());
        assert_eq!(
            cursor_path(&options, &configured),
            Some(PathBuf::from("/tmp/custom-cursor"))
        );
        let blank = |_key: &str| Some("  ".to_string());
        assert_eq!(
            cursor_path(&options, &blank),
            Some(PathBuf::from("/var/log/caddy/access.json.cursor"))
        );
        // stdin input has no cursor.
        assert_eq!(cursor_path(&Options::default(), &none), None);
    }

    #[tokio::test]
    async fn incremental_runs_read_only_the_identity_prefix_and_new_tail() {
        use apex_analytics::store::MemoryStore;
        let directory = test_directory("cursor-tail-read");
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let log = directory.join("access.json");
        let cursor = directory.join("cursor");
        let spool = directory.join("spool.jsonl");

        // A large first line makes a whole-file read visible in the byte
        // counter while leaving the identity (its 4 KiB prefix) stable.
        let mut first: serde_json::Value =
            serde_json::from_str(&log_line("/pricing", 200, 1_767_225_600.0)).expect("line json");
        first["request"]["headers"]["User-Agent"] =
            serde_json::json!([format!("Mozilla/5.0 {}", "x".repeat(200_000))]);
        let first = first.to_string();
        let second = log_line("/proof", 200, 1_767_225_601.0);
        tokio::fs::write(&log, format!("{first}\n"))
            .await
            .expect("write log");
        assert!(first.len() > 200_000);

        let lookup = test_env(&cursor, &spool);
        let store = MemoryStore::new();
        let options = Options {
            path: Some(log.clone()),
            ..Options::default()
        };
        let (_, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("first run");
        assert_eq!(summary.lines, 1);
        assert_eq!(summary.cursor_to, (first.len() + 1) as u64);

        let mut content = tokio::fs::read_to_string(&log).await.expect("read");
        content.push_str(&format!("{second}\n"));
        tokio::fs::write(&log, &content).await.expect("append");
        reset_input_bytes_read();
        let (_, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("incremental run");
        assert_eq!(summary.lines, 1, "only the appended line is new");
        assert_eq!(summary.cursor_from, (first.len() + 1) as u64);
        assert!(
            input_bytes_read() < 10_000,
            "one incremental run read {} bytes of a {} byte log; it must read the \
             4 KiB identity prefix and the new tail, not the whole file",
            input_bytes_read(),
            content.len()
        );

        let _ = tokio::fs::remove_dir_all(&directory).await;
    }

    #[tokio::test]
    async fn a_cursor_inside_a_line_restarts_at_zero() {
        use apex_analytics::store::MemoryStore;
        let directory = test_directory("cursor-mid-line");
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let log = directory.join("access.json");
        let cursor = directory.join("cursor");
        let spool = directory.join("spool.jsonl");
        let content = format!(
            "{}\n{}\n",
            log_line("/pricing", 200, 1_767_225_600.0),
            log_line("/proof", 200, 1_767_225_601.0)
        );
        tokio::fs::write(&log, &content).await.expect("write log");
        let lookup = test_env(&cursor, &spool);
        let store = MemoryStore::new();
        let options = Options {
            path: Some(log.clone()),
            ..Options::default()
        };
        let (_, first) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("first run");
        assert_eq!(first.lines, 2);

        // A cursor offset that happens to be a UTF-8 boundary but sits inside
        // a line is not a durable line boundary: the documented contract is
        // to restart rather than parse a partial line.
        let stored = tokio::fs::read_to_string(&cursor).await.expect("cursor");
        let stored = FileCursor::parse(&stored).expect("cursor parses");
        let partial = FileCursor {
            offset: 5,
            identity: stored.identity,
        };
        tokio::fs::write(&cursor, partial.render())
            .await
            .expect("write cursor");
        let (_, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("mid-line run");
        assert!(summary.cursor_restarted, "a mid-line cursor restarts");
        assert_eq!(summary.cursor_from, 0);
        assert_eq!(summary.lines, 2, "both complete lines are reprocessed");

        let _ = tokio::fs::remove_dir_all(&directory).await;
    }

    #[tokio::test]
    async fn a_file_is_delivered_once_and_the_rerun_reports_nothing_new() {
        use apex_analytics::store::MemoryStore;
        let directory = test_directory("cursor-repeat");
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let log = directory.join("access.json");
        let cursor = directory.join("cursor");
        let spool = directory.join("spool.jsonl");
        let content = format!(
            "{}\n{}\n",
            log_line("/pricing", 200, 1_767_225_600.0),
            log_line("/proof", 404, 1_767_225_601.0)
        );
        tokio::fs::write(&log, &content).await.expect("write log");
        let lookup = test_env(&cursor, &spool);
        let store = MemoryStore::new();
        let options = Options {
            path: Some(log.clone()),
            ..Options::default()
        };

        let (code, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("first run");
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(summary.lines, 2);
        assert_eq!(summary.request_logs, 2);
        assert_eq!(summary.events, 2);
        assert_eq!(summary.delivered, 2);
        assert_eq!(summary.events_delivered, 2);
        assert_eq!(summary.spooled, 0);
        assert!(!summary.cursor_restarted);
        assert_eq!(summary.cursor_from, 0);
        assert_eq!(summary.cursor_to, content.len() as u64);
        assert_eq!(store.request_logs().len(), 2);
        assert_eq!(store.load_events().await.expect("events").len(), 2);
        assert!(
            !tmp_sibling(&cursor).exists(),
            "the cursor is replaced atomically"
        );

        let (code, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("second run");
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(summary.lines, 0, "nothing new to read");
        assert_eq!(summary.request_logs, 0, "summary reports 0 new");
        assert_eq!(summary.events, 0);
        assert_eq!(summary.delivered, 0);
        assert_eq!(summary.events_delivered, 0);
        assert_eq!(show_summary_cursor(&summary), content.len() as u64);
        assert_eq!(
            store.request_logs().len(),
            2,
            "request_logs count unchanged"
        );
        assert_eq!(
            store.load_events().await.expect("events").len(),
            2,
            "event count unchanged"
        );

        let _ = tokio::fs::remove_dir_all(&directory).await;
    }

    // The cursor offset a summary reports, for concise assertions.
    fn show_summary_cursor(summary: &Summary) -> u64 {
        summary.cursor_to
    }

    #[tokio::test]
    async fn appended_lines_are_processed_incrementally() {
        use apex_analytics::store::MemoryStore;
        let directory = test_directory("cursor-append");
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let log = directory.join("access.json");
        let cursor = directory.join("cursor");
        let spool = directory.join("spool.jsonl");
        let first = log_line("/pricing", 200, 1_767_225_600.0);
        tokio::fs::write(&log, format!("{first}\n"))
            .await
            .expect("write log");
        let lookup = test_env(&cursor, &spool);
        let store = MemoryStore::new();
        let options = Options {
            path: Some(log.clone()),
            ..Options::default()
        };

        let (code, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("first run");
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(summary.lines, 1);
        assert_eq!(store.request_logs().len(), 1);

        // Append one line: only it is new.
        let second = log_line("/proof", 200, 1_767_225_601.0);
        let mut content = tokio::fs::read_to_string(&log).await.expect("read");
        content.push_str(&format!("{second}\n"));
        tokio::fs::write(&log, &content).await.expect("append");
        let (code, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("second run");
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(summary.lines, 1, "only the appended line is new");
        assert_eq!(summary.request_logs, 1);
        assert_eq!(summary.events, 1);
        assert_eq!(summary.delivered, 1);
        assert_eq!(show_summary_cursor(&summary), content.len() as u64);
        assert_eq!(store.request_logs().len(), 2);
        assert_eq!(store.load_events().await.expect("events").len(), 2);

        // The stored cursor parses back to the durable offset.
        let stored = tokio::fs::read_to_string(&cursor).await.expect("cursor");
        let stored = FileCursor::parse(&stored).expect("cursor parses");
        assert_eq!(stored.offset, content.len() as u64);

        let _ = tokio::fs::remove_dir_all(&directory).await;
    }

    #[tokio::test]
    async fn a_rotated_file_restarts_and_the_event_id_dedups() {
        use apex_analytics::store::MemoryStore;
        let directory = test_directory("cursor-rotate");
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let log = directory.join("access.json");
        let cursor = directory.join("cursor");
        let spool = directory.join("spool.jsonl");
        let first = log_line("/pricing", 200, 1_767_225_600.0);
        let second = log_line("/proof", 200, 1_767_225_601.0);
        tokio::fs::write(&log, format!("{first}\n{second}\n"))
            .await
            .expect("write log");
        let lookup = test_env(&cursor, &spool);
        let store = MemoryStore::new();
        let options = Options {
            path: Some(log.clone()),
            ..Options::default()
        };

        let (_, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("first run");
        assert_eq!(summary.lines, 2);
        assert_eq!(store.request_logs().len(), 2);
        assert_eq!(store.load_events().await.expect("events").len(), 2);

        // Rotation that shrinks the file (same leading line): the offset is
        // past the end, so the run restarts and reprocesses everything; the
        // replayed event deduplicates, the plain log row may repeat.
        tokio::fs::write(&log, format!("{first}\n"))
            .await
            .expect("rotate smaller");
        let (code, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("shrunken run");
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(summary.cursor_restarted, "a shrunken file restarts at 0");
        assert_eq!(summary.cursor_from, 0);
        assert_eq!(summary.lines, 1);
        assert_eq!(store.request_logs().len(), 3, "the log row repeats");
        assert_eq!(
            store.load_events().await.expect("events").len(),
            2,
            "the stable event id deduplicates the replay"
        );

        // Rotation to different content (identity changed): also a restart.
        let rotated = format!(
            "{}\n{}\n",
            log_line("/other", 404, 1_767_225_700.0),
            log_line("/terms", 200, 1_767_225_701.0)
        );
        tokio::fs::write(&log, &rotated).await.expect("rotate new");
        let (_, summary) = run_with_store(&options, &lookup, Some(store.clone()))
            .await
            .expect("rotated run");
        assert!(summary.cursor_restarted, "a replaced file restarts at 0");
        assert_eq!(summary.lines, 2);
        assert_eq!(store.request_logs().len(), 5);
        assert_eq!(store.load_events().await.expect("events").len(), 4);

        let _ = tokio::fs::remove_dir_all(&directory).await;
    }

    #[tokio::test]
    async fn bypassing_the_cursor_duplicates_the_log_row_but_not_the_event() {
        use apex_analytics::store::MemoryStore;
        let directory = test_directory("cursor-bypass");
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let log = directory.join("access.json");
        let spool = directory.join("spool.jsonl");
        let line = log_line("/pricing", 200, 1_767_225_600.0);
        tokio::fs::write(&log, format!("{line}\n"))
            .await
            .expect("write log");
        let store = MemoryStore::new();
        let options = Options {
            path: Some(log.clone()),
            ..Options::default()
        };

        // Two runs with two different cursors read the same line twice, as a
        // bypassed or lost cursor would.
        let cursor_a = directory.join("cursor-a");
        let cursor_b = directory.join("cursor_b");
        let (_, first) =
            run_with_store(&options, &test_env(&cursor_a, &spool), Some(store.clone()))
                .await
                .expect("run a");
        let (_, second) =
            run_with_store(&options, &test_env(&cursor_b, &spool), Some(store.clone()))
                .await
                .expect("run b");
        assert_eq!(first.events, 1);
        assert_eq!(second.events, 1, "the same line is processed again");
        assert_eq!(store.request_logs().len(), 2, "the log row duplicates");
        assert_eq!(
            store.load_events().await.expect("events").len(),
            1,
            "the stable event id keeps one event"
        );

        let _ = tokio::fs::remove_dir_all(&directory).await;
    }

    #[tokio::test]
    async fn check_runs_never_touch_the_cursor() {
        let directory = test_directory("cursor-check");
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let log = directory.join("access.json");
        let cursor = directory.join("cursor");
        let spool = directory.join("spool.jsonl");
        tokio::fs::write(
            &log,
            format!("{}\n", log_line("/pricing", 200, 1_767_225_600.0)),
        )
        .await
        .expect("write log");
        let lookup = test_env(&cursor, &spool);
        let options = Options {
            path: Some(log.clone()),
            check: true,
            ..Options::default()
        };
        let (code, summary) = run_with_store(&options, &lookup, None::<ClickHouseStore>)
            .await
            .expect("check run");
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(summary.lines, 1);
        assert_eq!(summary.events, 1);
        assert_eq!(summary.cursor_path, None);
        assert!(
            tokio::fs::metadata(&cursor).await.is_err(),
            "--check writes no cursor"
        );
        let _ = tokio::fs::remove_dir_all(&directory).await;
    }

    #[tokio::test]
    async fn an_unterminated_final_line_is_held_back_until_complete() {
        let directory = test_directory("cursor-partial");
        tokio::fs::create_dir_all(&directory).await.expect("dir");
        let log = directory.join("access.json");
        let cursor = directory.join("cursor");
        let spool = directory.join("spool.jsonl");
        let line = log_line("/pricing", 200, 1_767_225_600.0);
        tokio::fs::write(&log, &line)
            .await
            .expect("write partial line");
        let lookup = test_env(&cursor, &spool);
        let options = Options {
            path: Some(log.clone()),
            check: true,
            ..Options::default()
        };
        let (_, summary) = run_with_store(&options, &lookup, None::<ClickHouseStore>)
            .await
            .expect("check run");
        assert_eq!(summary.lines, 0, "a line without its newline is not parsed");

        // Completing the line makes it processable.
        tokio::fs::write(&log, format!("{line}\n"))
            .await
            .expect("complete line");
        let (_, summary) = run_with_store(&options, &lookup, None::<ClickHouseStore>)
            .await
            .expect("check again");
        assert_eq!(summary.lines, 1);
        let _ = tokio::fs::remove_dir_all(&directory).await;
    }
}
