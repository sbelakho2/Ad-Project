//! The `apex-sales` command line, as a testable function.
//!
//! ```text
//! apex-sales import-seeds <seeds.csv>  seed list (domain,name,country,app_store_ids,email,contact_name,role)
//! apex-sales import-leads <leads.jsonl> the apex-api free-audit leads file
//! apex-sales import-outcomes [file]    the frozen apex-api outcomes file (default .apex/sales/outcomes.jsonl)
//! apex-sales discover [--limit N]      fetch app-ads.txt and App Store data
//! apex-sales plan                      show today's plan with every gate reason
//! apex-sales run                       plan and send what is due (hourly from a timer);
//!                                      imports outcomes automatically when the file exists
//! apex-sales approve <touch-id>        approve a planned touch (approval mode)
//! apex-sales ingest <message.eml>      classify a reply or bounce and record it
//! apex-sales ingest --stdin            the same, reading one message from standard input
//! apex-sales ingest --maildir <dir>    process every *.eml in a directory (a Maildir), once
//! apex-sales record <target> <outcome> [note]   e.g. record studio.example paid
//! apex-sales suppress <target>         never contact an address or publisher
//! apex-sales reset-health [domain]     after investigating a quarantine (default: every domain)
//! apex-sales report                    what the models believe, with intervals
//! apex-sales simulate [--publishers N] [--days D] [--seed S]
//! ```
//!
//! Configuration comes from the environment:
//!
//! | variable | meaning | default |
//! |---|---|---|
//! | `APEX_SALES_DIR` | state directory | `.apex/sales` |
//! | `APEX_SALES_MODE` | `off`, `shadow`, `approval`, `autonomous` | `shadow` |
//! | `APEX_SALES_FROM` / `APEX_SALES_FROM_NAME` | the one sender when no senders file/list is set | required to send |
//! | `APEX_SALES_COMPANY` / `APEX_SALES_ADDRESS` | legal entity and postal address | required to send |
//! | `APEX_SALES_SENDERS_FILE` | CSV `email,name,company,postal_address,unsubscribe_url`: one sender per sending domain | none |
//! | `APEX_SALES_SENDERS` | inline senders: `;`-separated entries, `|`-separated fields in the file's column order | none |
//! | `APEX_SALES_WATCH_CHECKOUT_URL` | HTTPS Setup Watch checkout base (publisher key appended); absent disables the offer variant | none |
//! | `APEX_SALES_OUTCOMES_FILE` | the frozen apex-api outcomes JSONL imported by `import-outcomes` and `run` | `.apex/sales/outcomes.jsonl` |
//! | `APEX_SALES_UNSUBSCRIBE_URL` | HTTPS one-click base URL (token appended) | mailto only |
//! | `APEX_SALES_TOKEN_SALT` | salt for unsubscribe tokens | none |
//! | `APEX_SALES_HOLDOUT` / `APEX_SALES_HOLDOUT_SALT` | holdout share and salt | `0.10` |
//! | `APEX_SALES_EXPERIMENT` | top-two message experiments | `true` |
//! | `APEX_SALES_LEGAL_FILE` | CSV `country,rule` (`opt_out`, `consent`, `review`; `*` = unknown) | built-in table |
//! | `APEX_SALES_DAILY_MAX` | per-domain ceiling of the warm-up ramp (the engine's daily cap is the sum over domains) | `150` |
//! | `APEX_SALES_FETCH_DELAY_MS` | politeness delay between fetches | `1000` |
//!
//! With several sending domains each identity carries its own daily cap and
//! its own health; the day's plan is spread across the domains that may send
//! (see [`crate::engine::plan`]).

use std::collections::{BTreeMap, HashSet};

use crate::classify::ReplyClassifier;
use crate::compose::{build_message, Sender};
use crate::decision::{Autonomy, Verdict};
use crate::engine::{learn, plan, Config, PlanItem};
use crate::holdout::fnv1a;
use crate::io::{Fetcher, Mailer};
use crate::model::{normalize_email, Outcome, UnixSecs};
use crate::policy::{LegalTable, Rule};
use crate::simulate::{compare, SimConfig, SimResult};
use crate::store::{Event, Snapshot, StateDir};

/// Environment variables, captured once.
pub type Env = BTreeMap<String, String>;

/// The default file the apex-api outcomes appender writes.
pub const DEFAULT_OUTCOMES_PATH: &str = ".apex/sales/outcomes.jsonl";

/// The durable seen set of the maildir reply ingest.
pub const INGEST_SEEN_FILE: &str = "ingest-seen.jsonl";

fn var<'a>(env: &'a Env, key: &str) -> Option<&'a str> {
    env.get(key)
        .map(String::as_str)
        .filter(|v| !v.trim().is_empty())
}

/// Parse a legal table CSV.
pub fn parse_legal_table(csv: &str) -> Result<LegalTable, String> {
    let mut table = LegalTable {
        version: format!("file-{:08x}", fnv1a(csv.as_bytes()) as u32),
        rules: Vec::new(),
        unknown: Rule::OptOut,
    };
    for (i, row) in crate::ops::parse_csv(csv).into_iter().enumerate() {
        let (Some(country), Some(rule)) = (row.first(), row.get(1)) else {
            return Err(format!("legal table row {}: expected country,rule", i + 1));
        };
        if i == 0 && country.eq_ignore_ascii_case("country") {
            continue;
        }
        let rule = match rule.trim() {
            "opt_out" => Rule::OptOut,
            "consent" => Rule::ConsentRequired,
            "review" => Rule::Review,
            other => return Err(format!("legal table row {}: unknown rule {other}", i + 1)),
        };
        if country.trim() == "*" {
            table.unknown = rule;
        } else {
            table
                .rules
                .push((country.trim().to_ascii_uppercase(), rule));
        }
    }
    Ok(table)
}

/// Parse a senders file: CSV with a header naming `email`, `name`,
/// `company`, `postal_address` and optionally `unsubscribe_url`, one sender
/// per sending domain. Domains must be unique; a row with an invalid email
/// refuses the whole file.
pub fn parse_senders(csv: &str) -> Result<Vec<Sender>, String> {
    let rows = crate::ops::parse_csv(csv);
    let Some(header) = rows.first() else {
        return Err("senders file is empty".into());
    };
    let col = |name: &str| {
        header
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };
    let (c_email, c_name, c_company, c_address, c_unsubscribe) = (
        col("email"),
        col("name"),
        col("company"),
        col("postal_address"),
        col("unsubscribe_url"),
    );
    if c_email.is_none() {
        return Err("senders file needs an email column".into());
    }
    let mut senders: Vec<Sender> = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(1) {
        let get = |c: Option<usize>| {
            c.and_then(|c| row.get(c))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
        };
        let email = get(c_email)
            .and_then(normalize_email)
            .ok_or_else(|| format!("senders file row {}: missing or invalid email", i + 1))?;
        let domain = email.rsplit('@').next().unwrap_or_default().to_string();
        if senders.iter().any(|s| s.domain() == domain) {
            return Err(format!(
                "senders file row {}: the domain {domain} already has a sender",
                i + 1
            ));
        }
        senders.push(Sender {
            name: get(c_name).unwrap_or("Apex").to_string(),
            email,
            company: get(c_company).unwrap_or_default().to_string(),
            postal_address: get(c_address).unwrap_or_default().to_string(),
            unsubscribe_url: get(c_unsubscribe).map(str::to_string),
        });
    }
    if senders.is_empty() {
        return Err("senders file has no sender rows".into());
    }
    Ok(senders)
}

/// Parse the inline `APEX_SALES_SENDERS` list: entries separated by `;`,
/// fields by `|` in the senders file's column order
/// (`email|name|company|postal_address|unsubscribe_url`). Name defaults to
/// `Apex`; company, postal address and unsubscribe URL may be empty.
pub fn parse_senders_env(value: &str) -> Result<Vec<Sender>, String> {
    let mut senders: Vec<Sender> = Vec::new();
    for (i, entry) in value
        .split(';')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .enumerate()
    {
        let mut fields = entry.split('|').map(str::trim);
        let email = fields.next().and_then(normalize_email).ok_or_else(|| {
            format!(
                "APEX_SALES_SENDERS entry {}: missing or invalid email",
                i + 1
            )
        })?;
        let name = fields.next().unwrap_or("Apex");
        let company = fields.next().unwrap_or_default();
        let postal_address = fields.next().unwrap_or_default();
        let unsubscribe_url = fields.next().filter(|v| !v.is_empty()).map(str::to_string);
        if fields.next().is_some() {
            return Err(format!(
                "APEX_SALES_SENDERS entry {}: expected at most five | separated fields",
                i + 1
            ));
        }
        let domain = email.rsplit('@').next().unwrap_or_default().to_string();
        if senders.iter().any(|s| s.domain() == domain) {
            return Err(format!(
                "APEX_SALES_SENDERS entry {}: the domain {domain} already has a sender",
                i + 1
            ));
        }
        senders.push(Sender {
            name: name.to_string(),
            email,
            company: company.to_string(),
            postal_address: postal_address.to_string(),
            unsubscribe_url,
        });
    }
    if senders.is_empty() {
        return Err("APEX_SALES_SENDERS has no entries".into());
    }
    Ok(senders)
}

/// Build the engine configuration from the environment.
pub fn config_from_env(env: &Env) -> Result<Config, String> {
    let senders = match (
        var(env, "APEX_SALES_SENDERS_FILE"),
        var(env, "APEX_SALES_SENDERS"),
    ) {
        (Some(_), Some(_)) => {
            return Err(
                "set either APEX_SALES_SENDERS_FILE or APEX_SALES_SENDERS, not both".to_string(),
            )
        }
        (Some(path), None) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("APEX_SALES_SENDERS_FILE: {e}"))?;
            parse_senders(&text)?
        }
        (None, Some(list)) => parse_senders_env(list)?,
        (None, None) => vec![Sender {
            name: var(env, "APEX_SALES_FROM_NAME")
                .unwrap_or("Apex")
                .to_string(),
            email: var(env, "APEX_SALES_FROM")
                .unwrap_or("sales@apexmediation.ee")
                .to_string(),
            company: var(env, "APEX_SALES_COMPANY")
                .unwrap_or_default()
                .to_string(),
            postal_address: var(env, "APEX_SALES_ADDRESS")
                .unwrap_or_default()
                .to_string(),
            unsubscribe_url: var(env, "APEX_SALES_UNSUBSCRIBE_URL").map(str::to_string),
        }],
    };
    let mut config = Config::with_senders(senders);
    // Only an https base enables the Setup Watch offer variant; anything
    // else is treated as not configured (the bandit never serves it).
    config.watch_checkout_url = var(env, "APEX_SALES_WATCH_CHECKOUT_URL")
        .filter(|url| url.starts_with("https://"))
        .map(str::to_string);
    if let Some(mode) = var(env, "APEX_SALES_MODE") {
        config.autonomy =
            Autonomy::parse(mode).ok_or_else(|| format!("APEX_SALES_MODE: unknown mode {mode}"))?;
    }
    if let Some(share) = var(env, "APEX_SALES_HOLDOUT") {
        let s: f64 = share
            .parse()
            .map_err(|_| "APEX_SALES_HOLDOUT must be a number".to_string())?;
        if !(0.0..=0.5).contains(&s) {
            return Err("APEX_SALES_HOLDOUT must be between 0 and 0.5".into());
        }
        config.holdout.share = s;
    }
    if let Some(salt) = var(env, "APEX_SALES_HOLDOUT_SALT") {
        config.holdout.salt = salt.to_string();
    }
    if let Some(e) = var(env, "APEX_SALES_EXPERIMENT") {
        config.experiment = e != "false" && e != "0";
    }
    if let Some(max) = var(env, "APEX_SALES_DAILY_MAX") {
        config.health.max_cap = max
            .parse()
            .map_err(|_| "APEX_SALES_DAILY_MAX must be a whole number".to_string())?;
    }
    if let Some(path) = var(env, "APEX_SALES_LEGAL_FILE") {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("APEX_SALES_LEGAL_FILE: {e}"))?;
        config.legal = parse_legal_table(&text)?;
    }
    Ok(config)
}

fn require_sender(config: &Config) -> Result<(), String> {
    if config.senders.is_empty() {
        return Err(
            "sending needs at least one sender (APEX_SALES_SENDERS_FILE, APEX_SALES_SENDERS or APEX_SALES_FROM)"
                .into(),
        );
    }
    for s in &config.senders {
        if normalize_email(&s.email).is_none()
            || s.company.trim().is_empty()
            || s.postal_address.trim().is_empty()
        {
            if config.senders.len() == 1 {
                return Err("sending needs APEX_SALES_FROM, APEX_SALES_COMPANY and APEX_SALES_ADDRESS (the postal address every message must carry)".into());
            }
            return Err(format!(
                "sender {} needs a company and a postal address (the postal address every message must carry)",
                s.email
            ));
        }
    }
    Ok(())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// The frozen outcomes path: `APEX_SALES_OUTCOMES_FILE` when set, else
/// [`DEFAULT_OUTCOMES_PATH`].
fn outcomes_path(env: &Env) -> String {
    var(env, "APEX_SALES_OUTCOMES_FILE")
        .unwrap_or(DEFAULT_OUTCOMES_PATH)
        .to_string()
}

/// Import the outcomes file when it exists: the new events are written to
/// the store and appended to `snap` in time order. Deduplication by
/// (publisher, kind, ref) makes a replay write nothing. Returns `None` when
/// there is no file, else the path and the import summary.
fn import_outcomes_from_env(
    env: &Env,
    dir: &StateDir,
    snap: &mut Snapshot,
) -> Result<Option<(String, crate::ops::Imported)>, String> {
    let path = outcomes_path(env);
    if !std::path::Path::new(&path).is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let out = crate::ops::import_outcomes(&text, snap);
    dir.put_events(&out.events).map_err(|e| e.to_string())?;
    snap.events.extend(out.events.iter().cloned());
    snap.events.sort_by_key(Event::at);
    Ok(Some((path, out)))
}

/// Classify one message and record it, the shared body of `ingest` in every
/// input mode.
fn ingest_one(raw: &str, snap: &Snapshot, dir: &StateDir, now: UnixSecs) -> Result<String, String> {
    let (event, c) = crate::ops::ingest_reply(raw, snap, &ReplyClassifier::default(), now);
    match event {
        Some(e) => {
            dir.put_event(&e).map_err(|e| e.to_string())?;
            Ok(format!(
                "recorded {} ({})",
                c.outcome.as_string(),
                c.rationale
            ))
        }
        None => Ok(format!(
            "unmatched sender: classified {} ({}); record it by hand",
            c.outcome.as_string(),
            c.rationale
        )),
    }
}

/// Process every `*.eml` file in `maildir` (newest first, then by name)
/// exactly once, mirroring the single-file `ingest` semantics per file. The
/// durable seen set lives at `<state>/ingest-seen.jsonl` and keys a file by
/// name, size and modification time, so a replayed run sees nothing.
/// Unreadable files are skipped with a counted reason and are not marked
/// seen, so a transient failure is retried on the next run.
fn ingest_maildir(maildir: &str, dir: &StateDir, now: UnixSecs) -> Result<String, String> {
    let entries = std::fs::read_dir(maildir).map_err(|e| format!("{maildir}: {e}"))?;
    let mut files: Vec<(i64, String, u64)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{maildir}: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".eml") || name.starts_with('.') {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(e) => {
                skipped.push(format!("{name}: {e}"));
                continue;
            }
        };
        if !metadata.is_file() {
            skipped.push(format!("{name}: not a regular file"));
            continue;
        }
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        files.push((modified_ms, name, metadata.len()));
    }
    files.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let seen_text = std::fs::read_to_string(dir.root().join(INGEST_SEEN_FILE)).unwrap_or_default();
    // A set, not a vector: a maildir with n already-seen files must cost
    // n membership probes, not n^2 (the previous `Vec::contains` was
    // quadratic for a large replay).
    let mut seen: std::collections::HashSet<String> = seen_text
        .lines()
        .filter_map(|line| {
            let v = crate::json::parse(line).ok()?;
            let file = v.str_at("file")?.to_string();
            let size = v.get("size").and_then(crate::json::Value::as_i64)?;
            let modified = v.get("modifiedMs").and_then(crate::json::Value::as_i64)?;
            Some(format!("{file}|{size}|{modified}"))
        })
        .collect();
    let snap = dir.load().map_err(|e| e.to_string())?;
    let classifier = ReplyClassifier::default();
    let mut processed = 0usize;
    let mut recorded = 0usize;
    let mut unmatched = 0usize;
    for (modified_ms, name, size) in files {
        let key = format!("{name}|{size}|{modified_ms}");
        if seen.contains(&key) {
            continue;
        }
        let raw = match std::fs::read_to_string(std::path::Path::new(maildir).join(&name)) {
            Ok(raw) => raw,
            Err(e) => {
                skipped.push(format!("{name}: {e}"));
                continue;
            }
        };
        let (event, _) = crate::ops::ingest_reply(&raw, &snap, &classifier, now);
        match event {
            Some(e) => {
                dir.put_event(&e).map_err(|e| e.to_string())?;
                recorded += 1;
            }
            None => unmatched += 1,
        }
        dir.put_ingest_seen(&crate::json::Value::object(vec![
            ("file", crate::json::Value::str(name.clone())),
            ("size", crate::json::Value::num(size as f64)),
            ("modifiedMs", crate::json::Value::num(modified_ms as f64)),
            ("at", crate::json::Value::num(now as f64)),
        ]))
        .map_err(|e| e.to_string())?;
        seen.insert(key);
        processed += 1;
    }
    let mut out = format!(
        "ingest {maildir}: processed {processed} new file(s); recorded {recorded}; unmatched {unmatched}; skipped {}",
        skipped.len()
    );
    for reason in &skipped {
        out.push_str(&format!("\n  skipped {reason}"));
    }
    Ok(out)
}

fn describe(item: &PlanItem) -> String {
    format!(
        "{} {:<14} {:<32} step {} arm {:<11} EV ${:>7.2} p {:.3} at {} {}",
        item.touch_id,
        item.decision.verdict.as_str(),
        item.email,
        item.step,
        item.arm.as_deref().unwrap_or("-"),
        item.expected_value,
        item.p_interest,
        crate::calendar::rfc5322_date(item.send_at),
        if item.decision.reasons.is_empty() {
            String::new()
        } else {
            format!("| {}", item.decision.reasons.join("; "))
        }
    )
}

fn sim_line(name: &str, r: &SimResult) -> String {
    format!(
        "{name:<9} sends {:>6}  interested {:>4}  complaints {:>3}  value of sends ${:>10.0}  per send ${:.2}",
        r.sends,
        r.interested,
        r.complaints,
        r.value_of_sends,
        r.value_of_sends / f64::from(r.sends.max(1))
    )
}

/// The transport for this environment: `.eml` files in shadow or off mode
/// or when `APEX_SALES_OUTBOX` is set, otherwise the local MTA's
/// `sendmail` (`APEX_SALES_SENDMAIL`, default `/usr/sbin/sendmail`).
pub fn mailer_from_env(env: &Env) -> Box<dyn Mailer> {
    let mode = var(env, "APEX_SALES_MODE")
        .and_then(Autonomy::parse)
        .unwrap_or(Autonomy::Shadow);
    let dir = var(env, "APEX_SALES_DIR").unwrap_or(".apex/sales");
    match var(env, "APEX_SALES_OUTBOX") {
        Some(outbox) => Box::new(crate::io::OutboxMailer { dir: outbox.into() }),
        None if matches!(mode, Autonomy::Shadow | Autonomy::Off) => {
            Box::new(crate::io::OutboxMailer {
                dir: std::path::Path::new(dir).join("outbox"),
            })
        }
        None => Box::new(crate::io::SendmailMailer {
            binary: var(env, "APEX_SALES_SENDMAIL")
                .unwrap_or("/usr/sbin/sendmail")
                .to_string(),
        }),
    }
}

// Contact-membership probes during discovery. The scaling test asserts one
// probe per discovered address; the pre-fix `Vec::contains` cost one probe per
// already-known contact and failed the linear bound.
#[cfg(test)]
thread_local! {
    static CONTACT_COMPARES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_contact_compare() {
    CONTACT_COMPARES.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn reset_contact_compares() {
    CONTACT_COMPARES.with(|count| count.set(0));
}

#[cfg(test)]
fn contact_compares() -> u64 {
    CONTACT_COMPARES.with(std::cell::Cell::get)
}

/// The discovered-contact dedupe: an ordered vector (the output order) plus a
/// set for membership, so checking a new address is one hash probe instead of
/// a walk over every contact seen so far.
#[derive(Debug, Default)]
struct ContactAccumulator {
    ordered: Vec<String>,
    seen: HashSet<String>,
}

impl ContactAccumulator {
    fn new(existing: impl IntoIterator<Item = String>) -> Self {
        let ordered: Vec<String> = existing.into_iter().collect();
        let seen = ordered.iter().cloned().collect();
        Self { ordered, seen }
    }

    /// Record `email` when it is new, preserving discovery order. Returns
    /// whether it was new.
    fn insert(&mut self, email: &str) -> bool {
        #[cfg(test)]
        note_contact_compare();
        if self.seen.insert(email.to_string()) {
            self.ordered.push(email.to_string());
            true
        } else {
            false
        }
    }
}

/// Run one command. Returns the text to print.
pub fn run(
    args: &[String],
    env: &Env,
    now: UnixSecs,
    fetcher: &dyn Fetcher,
    mailer: &dyn Mailer,
) -> Result<String, String> {
    let io = |e: std::io::Error| e.to_string();
    let config = config_from_env(env)?;
    let dir = StateDir::open(var(env, "APEX_SALES_DIR").unwrap_or(".apex/sales")).map_err(io)?;
    let command = args.first().map(String::as_str).unwrap_or("help");
    let arg = |i: usize| {
        args.get(i)
            .cloned()
            .ok_or_else(|| format!("{command}: missing argument"))
    };
    match command {
        "import-seeds" | "import-leads" => {
            let text = std::fs::read_to_string(arg(1)?).map_err(io)?;
            let snap = dir.load().map_err(io)?;
            let out = if command == "import-seeds" {
                crate::ops::import_seeds(&text, &snap)
            } else {
                crate::ops::import_leads(&text, &snap)
            };
            // One durability barrier per file for the whole import: a crash
            // leaves a prefix of whole records and the next import converges
            // because the importer deduplicates against the store.
            dir.put_publishers(&out.publishers).map_err(io)?;
            dir.put_contacts(&out.contacts).map_err(io)?;
            dir.put_events(&out.events).map_err(io)?;
            let mut msg = format!(
                "imported {} publishers, {} contacts, {} events",
                out.publishers.len(),
                out.contacts.len(),
                out.events.len()
            );
            if out.already > 0 {
                msg.push_str(&format!(", {} already imported", out.already));
            }
            for r in out.refused {
                msg.push_str(&format!("\nrefused {r}"));
            }
            Ok(msg)
        }
        "discover" => {
            let limit: usize = flag(args, "--limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            let delay = var(env, "APEX_SALES_FETCH_DELAY_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000u64);
            let snap = dir.load().map_err(io)?;
            let stale = |p: &crate::model::Publisher| {
                p.domain.is_some()
                    && p.evidence
                        .iter()
                        .filter(|e| e.kind == crate::model::EvidenceKind::AppAdsTxt)
                        .map(|e| e.observed_at)
                        .max()
                        .is_none_or(|t| now - t > 30 * crate::model::DAY)
            };
            let mut lines = Vec::new();
            let mut existing = ContactAccumulator::new(snap.contacts.keys().cloned());
            for p in snap.publishers.values().filter(|p| stale(p)).take(limit) {
                let (updated, contacts, notes) = crate::ops::discover(p, fetcher, now);
                dir.put_publisher(&updated).map_err(io)?;
                for c in contacts {
                    if existing.insert(&c.email) {
                        dir.put_contact(&c).map_err(io)?;
                    }
                }
                lines.push(format!("{}: {}", p.key, notes.join("; ")));
                if delay > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                }
            }
            Ok(if lines.is_empty() {
                "nothing to discover".into()
            } else {
                lines.join("\n")
            })
        }
        "plan" => {
            let snap = dir.load().map_err(io)?;
            let learned = learn(&snap, &config, now);
            let items = plan(&snap, &learned, &config, now);
            let mut out = String::new();
            if config.senders.len() <= 1 {
                out.push_str(&format!(
                    "mode {}  domain {}  cap {} (sent today {})\n",
                    config.autonomy.as_str(),
                    learned.health.summary_state().as_str(),
                    learned.health.total_cap(),
                    learned.health.sent_today()
                ));
            } else {
                out.push_str(&format!(
                    "mode {}  domains {}  cap {} (sent today {})\n",
                    config.autonomy.as_str(),
                    config.senders.len(),
                    learned.health.total_cap(),
                    learned.health.sent_today()
                ));
                for domain in learned.health.domains() {
                    if let Some(h) = learned.health.get(domain) {
                        out.push_str(&format!(
                            "  {domain}: {}  cap {} (sent today {})\n",
                            h.state.as_str(),
                            h.cap,
                            learned.health.sent_today_for(domain)
                        ));
                    }
                }
            }
            for item in &items {
                out.push_str(&describe(item));
                out.push('\n');
            }
            Ok(out)
        }
        "run" => {
            // The hourly run imports the outcomes file first when it exists:
            // deduplication downstream makes a replay write nothing, and a
            // missing file changes nothing.
            let mut snap = dir.load().map_err(io)?;
            let imported = import_outcomes_from_env(env, &dir, &mut snap)?;
            let learned = learn(&snap, &config, now);
            let items = plan(&snap, &learned, &config, now);
            if config.autonomy != Autonomy::Shadow && config.autonomy != Autonomy::Off {
                require_sender(&config)?;
            }
            let salt = var(env, "APEX_SALES_TOKEN_SALT").unwrap_or_default();
            let mut sent = 0;
            let mut previews = 0;
            let mut waiting = 0;
            for item in &items {
                let approved = snap.approvals.contains(&item.touch_id);
                let go = match item.decision.verdict {
                    Verdict::Send => true,
                    Verdict::NeedsApproval => approved,
                    Verdict::Shadow => false,
                    _ => continue,
                };
                if item.send_at > now + 3600 {
                    waiting += 1;
                    continue;
                }
                let (Some(rendered), Some(arm), Some(contact)) =
                    (&item.rendered, &item.arm, snap.contacts.get(&item.email))
                else {
                    continue;
                };
                let Some(sender) = item
                    .sender
                    .as_deref()
                    .and_then(|domain| config.sender_for(domain))
                else {
                    continue;
                };
                let token = format!(
                    "{:016x}",
                    fnv1a(format!("{salt}|{}", item.email).as_bytes())
                );
                let msg = build_message(rendered, contact, sender, &item.touch_id, now, &token);
                if !go {
                    if item.decision.verdict == Verdict::Shadow {
                        mailer.send(&msg).map_err(|e| e.to_string())?;
                        previews += 1;
                    } else {
                        waiting += 1;
                    }
                    continue;
                }
                mailer.send(&msg).map_err(|e| e.to_string())?;
                dir.put_event(&Event::Touch {
                    at: now,
                    email: item.email.clone(),
                    publisher: item.publisher.clone(),
                    // Only a multi-domain configuration records the domain;
                    // single-domain history stays as it always was.
                    from: if config.senders.len() > 1 {
                        item.sender.clone()
                    } else {
                        None
                    },
                    step: item.step,
                    arm: arm.clone(),
                    message_id: msg.message_id.clone(),
                    local_hour: item.local_hour,
                    predicted: Some(item.p_interest),
                })
                .map_err(io)?;
                dir.put_decision(&crate::json::Value::object(vec![
                    ("at", crate::json::Value::num(now as f64)),
                    ("touchId", crate::json::Value::str(item.touch_id.clone())),
                    ("email", crate::json::Value::str(item.email.clone())),
                    ("verdict", crate::json::Value::str("sent")),
                    (
                        "expectedValue",
                        crate::json::Value::num(item.expected_value),
                    ),
                    ("pInterest", crate::json::Value::num(item.p_interest)),
                    (
                        "scoreReasons",
                        crate::json::Value::Array(
                            item.score_reasons
                                .iter()
                                .map(|r| crate::json::Value::str(r.clone()))
                                .collect(),
                        ),
                    ),
                    (
                        "cites",
                        crate::json::Value::Array(
                            rendered
                                .cites
                                .iter()
                                .map(|r| crate::json::Value::str(r.clone()))
                                .collect(),
                        ),
                    ),
                ]))
                .map_err(io)?;
                sent += 1;
            }
            let mut out = format!(
                "mode {}: sent {sent}, previews {previews}, waiting {waiting}",
                config.autonomy.as_str()
            );
            if let Some((path, imported)) = imported {
                if !imported.events.is_empty() {
                    out.push_str(&format!(
                        "\nimported {} outcomes from {path}",
                        imported.events.len()
                    ));
                }
            }
            Ok(out)
        }
        "approve" => {
            let id = arg(1)?;
            dir.put_approval(&id, now).map_err(io)?;
            Ok(format!("approved {id}"))
        }
        "ingest" => {
            if let Some(maildir) = flag(args, "--maildir") {
                ingest_maildir(&maildir, &dir, now)
            } else if args.iter().any(|a| a == "--stdin") {
                let raw = std::io::read_to_string(std::io::stdin()).map_err(io)?;
                let snap = dir.load().map_err(io)?;
                ingest_one(&raw, &snap, &dir, now)
            } else {
                let raw = std::fs::read_to_string(arg(1)?).map_err(io)?;
                let snap = dir.load().map_err(io)?;
                ingest_one(&raw, &snap, &dir, now)
            }
        }
        "import-outcomes" => {
            let dry_run = args.iter().any(|a| a == "--dry-run");
            let path = args
                .iter()
                .skip(1)
                .find(|a| !a.starts_with("--"))
                .cloned()
                .unwrap_or_else(|| outcomes_path(env));
            let text = std::fs::read_to_string(&path).map_err(io)?;
            let snap = dir.load().map_err(io)?;
            let out = crate::ops::import_outcomes(&text, &snap);
            if !dry_run {
                // The import's events are one durability unit: one barrier.
                dir.put_events(&out.events).map_err(io)?;
            }
            let mut msg = format!(
                "{} {} outcomes",
                if dry_run { "would import" } else { "imported" },
                out.events.len()
            );
            if out.already > 0 {
                msg.push_str(&format!(", {} already imported", out.already));
            }
            if !out.refused.is_empty() {
                msg.push_str(&format!(
                    ", {} malformed line(s) skipped",
                    out.refused.len()
                ));
                for reason in &out.refused {
                    msg.push_str(&format!("\nskipped {reason}"));
                }
            }
            if dry_run {
                msg = format!("dry run: {msg} (nothing written)");
            }
            Ok(msg)
        }
        "record" => {
            let target = arg(1)?;
            let outcome = Outcome::parse(&arg(2)?).ok_or("record: unknown outcome")?;
            dir.put_event(&crate::ops::record_outcome(
                &target,
                outcome,
                args.get(3).cloned(),
                now,
            ))
            .map_err(io)?;
            Ok(format!("recorded {} for {target}", outcome.as_string()))
        }
        "suppress" => {
            let target = arg(1)?;
            let email = normalize_email(&target).filter(|_| target.contains('@'));
            dir.put_event(&Event::Suppress {
                at: now,
                publisher: if email.is_none() {
                    Some(target.clone())
                } else {
                    None
                },
                email,
            })
            .map_err(io)?;
            Ok(format!("suppressed {target}"))
        }
        "reset-health" => {
            let domain = args.get(1).cloned();
            if let Some(d) = &domain {
                if config.sender_for(d).is_none() {
                    return Err(format!(
                        "reset-health: {d} is not a configured sending domain"
                    ));
                }
            }
            dir.put_event(&Event::HealthReset { at: now, domain })
                .map_err(io)?;
            Ok(match args.get(1) {
                Some(d) => format!("sending domain {d} reset to warming"),
                None => "sending domain reset to warming".into(),
            })
        }
        "report" => {
            let snap = dir.load().map_err(io)?;
            let learned = learn(&snap, &config, now);
            let mut out = crate::ops::report(&snap, &learned, &config, now);
            if snap.skipped_lines > 0 {
                out.push_str(&format!(
                    "\nwarning: {} unreadable lines were skipped\n",
                    snap.skipped_lines
                ));
            }
            Ok(out)
        }
        "simulate" => {
            let cfg = SimConfig {
                publishers: flag(args, "--publishers")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1500),
                days: flag(args, "--days")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
                seed: flag(args, "--seed")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
            };
            let (engine, baseline) = compare(&cfg);
            Ok(format!(
                "{} publishers, {} days, seed {}\n{}\n{}\nratio {:.2}",
                cfg.publishers,
                cfg.days,
                cfg.seed,
                sim_line("engine", &engine),
                sim_line("baseline", &baseline),
                engine.value_of_sends / baseline.value_of_sends.max(1e-9)
            ))
        }
        "help" | "--help" | "-h" => Ok(include_str!("cli_help.txt").to_string()),
        other => Err(format!("unknown command {other}; try `apex-sales help`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{MapFetcher, OutboxMailer};
    use crate::model::DAY;

    fn env(dir: &std::path::Path, mode: &str) -> Env {
        let mut e = Env::new();
        e.insert("APEX_SALES_DIR".into(), dir.display().to_string());
        e.insert("APEX_SALES_MODE".into(), mode.into());
        e.insert("APEX_SALES_FROM".into(), "aaron@apexmediation.ee".into());
        e.insert("APEX_SALES_FROM_NAME".into(), "Aaron".into());
        e.insert("APEX_SALES_COMPANY".into(), "Apex OÜ".into());
        e.insert("APEX_SALES_ADDRESS".into(), "Tallinn, Estonia".into());
        e.insert("APEX_SALES_HOLDOUT".into(), "0".into());
        e.insert("APEX_SALES_FETCH_DELAY_MS".into(), "0".into());
        e
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    const NOW: UnixSecs = 20_745 * DAY + 9 * 3600; // Monday 19 Oct 2026 09:00 UTC

    #[test]
    fn end_to_end_import_discover_plan_run_ingest_report() {
        let root = std::env::temp_dir().join(format!("apex-sales-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let seeds = root.join("seeds.csv");
        std::fs::write(
            &seeds,
            "domain,name,country,app_store_ids,email,contact_name,role\nstudio.example,Studio,FI,42,maria@studio.example,Maria Lopez,CEO\n",
        )
        .unwrap();
        let state = root.join("state");
        let outbox = OutboxMailer {
            dir: root.join("outbox"),
        };
        let fetcher = MapFetcher {
            pages: vec![
                ("https://studio.example/app-ads.txt".into(), 404, String::new()),
                (
                    "https://itunes.apple.com/lookup?id=42".into(),
                    200,
                    r#"{"results":[{"kind":"software","trackId":42,"trackName":"Tile Quest","primaryGenreName":"Games","userRatingCount":20000,"price":0}]}"#.into(),
                ),
            ],
        };
        let e = env(&state, "autonomous");
        let out = run(
            &s(&["import-seeds", seeds.to_str().unwrap()]),
            &e,
            NOW,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(out.starts_with("imported 1 publishers, 1 contacts"));
        let out = run(&s(&["discover"]), &e, NOW, &fetcher, &outbox).unwrap();
        assert!(out.contains("not found (404)"), "{out}");
        assert_eq!(
            run(&s(&["discover"]), &e, NOW, &fetcher, &outbox).unwrap(),
            "nothing to discover"
        );
        let out = run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).unwrap();
        assert!(
            out.contains(" send ") && out.contains("maria@studio.example"),
            "{out}"
        );
        // The chosen local hour may be later today or tomorrow; run hourly
        // until it is due, as the timer would.
        let mut sent = false;
        for h in 0..48 {
            let out = run(&s(&["run"]), &e, NOW + h * 3600, &fetcher, &outbox).unwrap();
            if out.contains("sent 1") {
                sent = true;
                break;
            }
        }
        assert!(sent);
        let emls: Vec<_> = std::fs::read_dir(root.join("outbox")).unwrap().collect();
        assert_eq!(emls.len(), 1);
        let reply = root.join("reply.eml");
        std::fs::write(
            &reply,
            "From: Maria <maria@studio.example>\nSubject: Re\n\nYes, please run the free audit.\n",
        )
        .unwrap();
        let out = run(
            &s(&["ingest", reply.to_str().unwrap()]),
            &e,
            NOW + DAY,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(out.starts_with("recorded reply:positive"), "{out}");
        assert!(run(
            &s(&["record", "studio.example", "audit_requested"]),
            &e,
            NOW + 2 * DAY,
            &fetcher,
            &outbox
        )
        .is_ok());
        assert!(run(&s(&["record", "x", "nope"]), &e, NOW, &fetcher, &outbox).is_err());
        assert!(run(
            &s(&["suppress", "spam@else.example"]),
            &e,
            NOW,
            &fetcher,
            &outbox
        )
        .is_ok());
        assert!(run(&s(&["reset-health"]), &e, NOW, &fetcher, &outbox).is_ok());
        assert!(run(&s(&["approve", "t1"]), &e, NOW, &fetcher, &outbox)
            .unwrap()
            .contains("t1"));
        let rep = run(&s(&["report"]), &e, NOW + 3 * DAY, &fetcher, &outbox).unwrap();
        assert!(rep.contains("lead score"));
        // The replied publisher is no longer planned.
        let out = run(&s(&["plan"]), &e, NOW + 5 * DAY, &fetcher, &outbox).unwrap();
        assert!(!out.contains("maria@studio.example"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn modes_and_configuration_guard_sending() {
        let root =
            std::env::temp_dir().join(format!("apex-sales-cli-modes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let outbox = OutboxMailer {
            dir: root.join("outbox"),
        };
        let fetcher = MapFetcher::default();
        let mut e = env(&root.join("state"), "autonomous");
        e.insert("APEX_SALES_ADDRESS".into(), String::new());
        assert!(run(&s(&["run"]), &e, NOW, &fetcher, &outbox)
            .unwrap_err()
            .contains("postal address"));
        e.insert("APEX_SALES_MODE".into(), "shadow".into());
        assert!(run(&s(&["run"]), &e, NOW, &fetcher, &outbox)
            .unwrap()
            .starts_with("mode shadow"));
        e.insert("APEX_SALES_MODE".into(), "chaos".into());
        assert!(run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).is_err());
        e.insert("APEX_SALES_MODE".into(), "off".into());
        e.insert("APEX_SALES_HOLDOUT".into(), "0.9".into());
        assert!(run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).is_err());
        e.insert("APEX_SALES_HOLDOUT".into(), "x".into());
        assert!(run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).is_err());
        e.insert("APEX_SALES_HOLDOUT".into(), "0.2".into());
        e.insert("APEX_SALES_DAILY_MAX".into(), "x".into());
        assert!(run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).is_err());
        e.remove("APEX_SALES_DAILY_MAX");
        e.insert("APEX_SALES_EXPERIMENT".into(), "false".into());
        assert!(!config_from_env(&e).unwrap().experiment);
        assert!(run(&s(&["help"]), &e, NOW, &fetcher, &outbox)
            .unwrap()
            .contains("apex-sales"));
        let probe = crate::compose::Outgoing {
            from: "-x@a.example".into(),
            to: "b@b.example".into(),
            message_id: "m".into(),
            raw: String::new(),
        };
        // Off mode writes previews; autonomous mode uses sendmail, which
        // refuses this hostile envelope sender before running anything.
        assert!(mailer_from_env(&e).send(&probe).is_ok());
        e.insert("APEX_SALES_MODE".into(), "autonomous".into());
        assert!(mailer_from_env(&e).send(&probe).is_err());
        e.insert(
            "APEX_SALES_OUTBOX".into(),
            root.join("box").display().to_string(),
        );
        assert!(mailer_from_env(&e).send(&probe).is_ok());
        e.insert("APEX_SALES_MODE".into(), "off".into());
        assert!(run(&s(&["bogus"]), &e, NOW, &fetcher, &outbox).is_err());
        assert!(run(&s(&["ingest"]), &e, NOW, &fetcher, &outbox)
            .unwrap_err()
            .contains("missing argument"));
        let out = run(
            &s(&["simulate", "--publishers", "60", "--days", "5"]),
            &e,
            NOW,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(out.contains("engine") && out.contains("baseline"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[derive(Debug)]
    struct BrokenMailer;

    impl Mailer for BrokenMailer {
        fn send(&self, _msg: &crate::compose::Outgoing) -> Result<(), crate::io::MailError> {
            Err(crate::io::MailError("relay down".into()))
        }
    }

    #[test]
    fn transport_and_file_failures_are_errors_and_record_nothing() {
        let root = std::env::temp_dir().join(format!("apex-sales-cli-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fetcher = MapFetcher {
            pages: vec![
                ("https://studio.example/app-ads.txt".into(), 404, String::new()),
                (
                    "https://itunes.apple.com/lookup?id=42".into(),
                    200,
                    r#"{"results":[{"kind":"software","trackId":42,"trackName":"Tile Quest","primaryGenreName":"Games","userRatingCount":20000,"price":0}]}"#.into(),
                ),
            ],
        };
        let mut e = env(&root.join("state"), "autonomous");
        let missing = root.join("missing.csv").display().to_string();
        let seeds_cmd = |path: &str| s(&["import-seeds", path]);
        assert!(run(&seeds_cmd(&missing), &e, NOW, &fetcher, &BrokenMailer).is_err());
        let seeds = root.join("seeds.csv");
        std::fs::write(
            &seeds,
            "domain,name,country,app_store_ids,email,contact_name,role\nstudio.example,Studio,FI,42,maria@studio.example,Maria Lopez,CEO\n",
        )
        .unwrap();
        let seeds = seeds.display().to_string();
        run(&seeds_cmd(&seeds), &e, NOW, &fetcher, &BrokenMailer).unwrap();
        // Without evidence there is nothing true to say, so nothing is planned
        // to send; discovery finds the app and the missing app-ads.txt.
        let out = run(&s(&["plan"]), &e, NOW, &fetcher, &BrokenMailer).unwrap();
        assert!(!out.contains(" send "), "{out}");
        run(&s(&["discover"]), &e, NOW, &fetcher, &BrokenMailer).unwrap();
        // Hourly runs until the touch is due: the failed hand-off is an
        // error, and no touch is recorded, so the contact stays planned.
        let failed = (0..48)
            .find_map(|h| run(&s(&["run"]), &e, NOW + h * 3600, &fetcher, &BrokenMailer).err());
        assert!(failed.is_some_and(|err| err.contains("relay down")));
        let out = run(&s(&["plan"]), &e, NOW, &fetcher, &BrokenMailer).unwrap();
        assert!(
            out.contains(" send ") && out.contains("maria@studio.example"),
            "{out}"
        );
        // Shadow previews go through the same transport and fail the same way.
        e.insert("APEX_SALES_MODE".into(), "shadow".into());
        let shadow_failed = (0..48)
            .any(|h| run(&s(&["run"]), &e, NOW + h * 3600, &fetcher, &BrokenMailer).is_err());
        assert!(shadow_failed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn legal_tables_parse() {
        let t = parse_legal_table("country,rule\nDE,consent\nTR,review\n*,review\n").unwrap();
        assert_eq!(t.rule("de"), Rule::ConsentRequired);
        assert_eq!(t.rule("TR"), Rule::Review);
        assert_eq!(t.unknown, Rule::Review);
        assert!(parse_legal_table("DE,maybe\n").is_err());
        assert!(parse_legal_table("DE\n").is_err());
        let dir = std::env::temp_dir().join(format!("apex-sales-legal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legal.csv");
        std::fs::write(&path, "FR,consent\n").unwrap();
        let mut e = Env::new();
        e.insert("APEX_SALES_LEGAL_FILE".into(), path.display().to_string());
        assert_eq!(
            config_from_env(&e).unwrap().legal.rule("FR"),
            Rule::ConsentRequired
        );
        e.insert(
            "APEX_SALES_LEGAL_FILE".into(),
            dir.join("missing.csv").display().to_string(),
        );
        assert!(config_from_env(&e).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sender_configuration_extends_the_single_sender() {
        let dir = std::env::temp_dir().join(format!("apex-sales-senders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The single APEX_SALES_FROM identity is still one sender.
        let mut e = Env::new();
        let c = config_from_env(&e).unwrap();
        assert_eq!(c.senders.len(), 1);
        assert_eq!(c.senders[0].email, "sales@apexmediation.ee");
        assert!(c.watch_checkout_url.is_none());
        // The inline list: entries by `;`, fields by `|`.
        e.insert(
            "APEX_SALES_SENDERS".into(),
            "aaron@one.example|Aaron|Apex OÜ|Tallinn, Estonia|https://one.example/u/;dana@two.example"
                .into(),
        );
        let c = config_from_env(&e).unwrap();
        assert_eq!(c.senders.len(), 2);
        assert_eq!(c.senders[0].domain(), "one.example");
        assert_eq!(c.senders[0].postal_address, "Tallinn, Estonia");
        assert_eq!(
            c.senders[0].unsubscribe_url.as_deref(),
            Some("https://one.example/u/")
        );
        assert_eq!(c.senders[1].name, "Apex", "the name defaults when omitted");
        assert_eq!(c.senders[1].domain(), "two.example");
        // Refusals: duplicate domains, invalid emails, too many fields.
        e.insert(
            "APEX_SALES_SENDERS".into(),
            "a@x.example|A;b@x.example|B".into(),
        );
        assert!(config_from_env(&e)
            .unwrap_err()
            .contains("already has a sender"));
        e.insert("APEX_SALES_SENDERS".into(), "not-an-email".into());
        assert!(config_from_env(&e).is_err());
        e.insert("APEX_SALES_SENDERS".into(), "a@x.example|A|C|D|E|F".into());
        assert!(config_from_env(&e).is_err());
        // A senders file carries the same identities; both sources refuse.
        let file = dir.join("senders.csv");
        std::fs::write(
            &file,
            "email,name,company,postal_address,unsubscribe_url\n\
             aaron@one.example,Aaron,Apex OÜ,\"Tallinn, Estonia\",https://one.example/u/\n\
             dana@two.example,Dana,Apex OÜ,Tallinn,\n",
        )
        .unwrap();
        e.remove("APEX_SALES_SENDERS");
        e.insert("APEX_SALES_SENDERS_FILE".into(), file.display().to_string());
        let c = config_from_env(&e).unwrap();
        assert_eq!(c.senders.len(), 2);
        assert_eq!(c.senders[0].postal_address, "Tallinn, Estonia");
        assert_eq!(c.senders[1].unsubscribe_url, None);
        e.insert("APEX_SALES_SENDERS".into(), "a@y.example".into());
        assert!(config_from_env(&e).unwrap_err().contains("not both"));
        e.remove("APEX_SALES_SENDERS");
        // Parse refusals name the row.
        assert!(parse_senders("name,email\nA,a@x.example\nB,nope\n")
            .unwrap_err()
            .contains("row 3"));
        assert!(parse_senders("name\nA\n")
            .unwrap_err()
            .contains("email column"));
        assert!(parse_senders("").unwrap_err().contains("empty"));
        // The Setup Watch checkout must be https to enable the offer.
        e.remove("APEX_SALES_SENDERS_FILE");
        e.insert(
            "APEX_SALES_WATCH_CHECKOUT_URL".into(),
            "https://apexmediation.ee/watch/".into(),
        );
        assert_eq!(
            config_from_env(&e).unwrap().watch_checkout_url.as_deref(),
            Some("https://apexmediation.ee/watch/")
        );
        e.insert(
            "APEX_SALES_WATCH_CHECKOUT_URL".into(),
            "http://insecure.example/watch/".into(),
        );
        assert!(config_from_env(&e).unwrap().watch_checkout_url.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_outcomes_is_idempotent_and_dry_run_writes_nothing() {
        let root = std::env::temp_dir().join(format!("apex-sales-outcomes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        let outbox = OutboxMailer {
            dir: root.join("outbox"),
        };
        let fetcher = MapFetcher::default();
        let mut e = env(&state, "off");
        let file = root.join("outcomes.jsonl");
        std::fs::write(
            &file,
            "{\"atUnixMs\":1700000000000,\"publisher\":\"tinyfox.io\",\"kind\":\"connected\",\"ref\":\"evt_1\"}\n\
             {\"atUnixMs\":1700000001000,\"publisher\":\"tinyfox.io\",\"kind\":\"paid\",\"ref\":\"inv_9\"}\n\
             {\"atUnixMs\":1700000002000,\"publisher\":\"solo\",\"kind\":\"watch_signup\",\"ref\":\"sub_3\"}\n\
             garbage\n\
             {\"atUnixMs\":1700000003000,\"publisher\":\"x.io\",\"kind\":\"nope\",\"ref\":\"e\"}\n",
        )
        .unwrap();
        e.insert(
            "APEX_SALES_OUTCOMES_FILE".into(),
            file.display().to_string(),
        );
        // Dry run: counts only, nothing durable.
        let out = run(
            &s(&["import-outcomes", "--dry-run"]),
            &e,
            NOW,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(out.starts_with("dry run: would import 3 outcomes"), "{out}");
        assert!(out.contains("2 malformed line(s) skipped"), "{out}");
        assert!(out.contains("nothing written"), "{out}");
        assert!(!state.join("events.jsonl").exists());
        // Real import: three outcome events, watch_signup included.
        let out = run(&s(&["import-outcomes"]), &e, NOW, &fetcher, &outbox).unwrap();
        assert!(out.starts_with("imported 3 outcomes"), "{out}");
        let events = std::fs::read_to_string(state.join("events.jsonl")).unwrap();
        assert_eq!(events.lines().count(), 3);
        assert!(events.contains("\"outcome\":\"connected\""), "{events}");
        assert!(events.contains("\"outcome\":\"paid\""), "{events}");
        assert!(events.contains("\"outcome\":\"watch_signup\""), "{events}");
        assert!(events.contains("\"note\":\"evt_1\""), "{events}");
        // The replay is idempotent, also when the path comes as an argument.
        let out = run(
            &s(&["import-outcomes", file.to_str().unwrap()]),
            &e,
            NOW,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(
            out.starts_with("imported 0 outcomes, 3 already imported"),
            "{out}"
        );
        assert_eq!(
            std::fs::read_to_string(state.join("events.jsonl"))
                .unwrap()
                .lines()
                .count(),
            3
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_hourly_run_auto_imports_outcomes_when_the_file_exists() {
        let root = std::env::temp_dir().join(format!("apex-sales-autolm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        let outbox = OutboxMailer {
            dir: root.join("outbox"),
        };
        let fetcher = MapFetcher::default();
        let mut e = env(&state, "shadow");
        let file = root.join("outcomes.jsonl");
        e.insert(
            "APEX_SALES_OUTCOMES_FILE".into(),
            file.display().to_string(),
        );
        // No file: the run output is exactly as before.
        let out = run(&s(&["run"]), &e, NOW, &fetcher, &outbox).unwrap();
        assert_eq!(out, "mode shadow: sent 0, previews 0, waiting 0");
        // The outcomes file appears: the run imports it before planning.
        std::fs::write(
            &file,
            "{\"atUnixMs\":1700000000000,\"publisher\":\"tinyfox.io\",\"kind\":\"connected\",\"ref\":\"evt_1\"}\n",
        )
        .unwrap();
        let out = run(&s(&["run"]), &e, NOW, &fetcher, &outbox).unwrap();
        assert!(out.contains("imported 1 outcomes from"), "{out}");
        assert!(std::fs::read_to_string(state.join("events.jsonl"))
            .unwrap()
            .contains("\"outcome\":\"connected\""));
        // A replayed run imports nothing new and writes no second event.
        let out = run(&s(&["run"]), &e, NOW, &fetcher, &outbox).unwrap();
        assert!(!out.contains("imported"), "{out}");
        assert_eq!(
            std::fs::read_to_string(state.join("events.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn maildir_ingest_processes_each_file_once_and_counts_skips() {
        let root = std::env::temp_dir().join(format!("apex-sales-maildir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        let outbox = OutboxMailer {
            dir: root.join("outbox"),
        };
        let fetcher = MapFetcher::default();
        let e = env(&state, "off");
        let seeds = root.join("seeds.csv");
        std::fs::write(
            &seeds,
            "domain,name,country,app_store_ids,email,contact_name,role\nstudio.example,Studio,FI,42,maria@studio.example,Maria Lopez,CEO\n",
        )
        .unwrap();
        run(
            &s(&["import-seeds", seeds.to_str().unwrap()]),
            &e,
            NOW,
            &fetcher,
            &outbox,
        )
        .unwrap();
        let maildir = root.join("maildir");
        std::fs::create_dir_all(&maildir).unwrap();
        std::fs::write(
            maildir.join("one.eml"),
            "From: Maria <maria@studio.example>\nSubject: Re\n\nYes, please run the free audit.\n",
        )
        .unwrap();
        std::fs::write(
            maildir.join("two.eml"),
            "From: someone@else.example\nSubject: Hi\n\nhello\n",
        )
        .unwrap();
        // Unreadable: invalid UTF-8. It must be counted and left unseen.
        std::fs::write(maildir.join("bad.eml"), [0xff_u8, 0xfe, 0xfd]).unwrap();
        let out = run(
            &s(&["ingest", "--maildir", maildir.to_str().unwrap()]),
            &e,
            NOW,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(
            out.starts_with(&format!(
                "ingest {}: processed 2 new file(s); recorded 1; unmatched 1; skipped 1",
                maildir.display()
            )),
            "{out}"
        );
        assert!(out.contains("skipped bad.eml"), "{out}");
        let events = std::fs::read_to_string(state.join("events.jsonl")).unwrap();
        assert_eq!(events.lines().count(), 1, "{events}");
        assert!(events.contains("reply:positive"), "{events}");
        let seen = std::fs::read_to_string(state.join("ingest-seen.jsonl")).unwrap();
        assert_eq!(seen.lines().count(), 2, "durable seen set");
        // Second run: the readable files were seen; only the unreadable one
        // is retried, and nothing is recorded twice.
        let out = run(
            &s(&["ingest", "--maildir", maildir.to_str().unwrap()]),
            &e,
            NOW + 3600,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(
            out.starts_with(&format!(
                "ingest {}: processed 0 new file(s); recorded 0; unmatched 0; skipped 1",
                maildir.display()
            )),
            "{out}"
        );
        assert_eq!(
            std::fs::read_to_string(state.join("events.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(state.join("ingest-seen.jsonl"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        // A missing directory is an error, not a silent success.
        assert!(run(
            &s(&["ingest", "--maildir", root.join("nope").to_str().unwrap()]),
            &e,
            NOW,
            &fetcher,
            &outbox
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reset_health_names_a_domain_and_still_resets_everything_bare() {
        let root = std::env::temp_dir().join(format!("apex-sales-reset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        let outbox = OutboxMailer {
            dir: root.join("outbox"),
        };
        let fetcher = MapFetcher::default();
        let mut e = env(&state, "off");
        e.insert(
            "APEX_SALES_SENDERS".into(),
            "aaron@one.example|Aaron|Apex|Tallinn;dana@two.example|Dana|Apex|Tallinn".into(),
        );
        let out = run(
            &s(&["reset-health", "one.example"]),
            &e,
            NOW,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert_eq!(out, "sending domain one.example reset to warming");
        let events = std::fs::read_to_string(state.join("events.jsonl")).unwrap();
        assert!(events.contains("\"domain\":\"one.example\""), "{events}");
        assert!(run(
            &s(&["reset-health", "unknown.example"]),
            &e,
            NOW,
            &fetcher,
            &outbox
        )
        .is_err());
        let out = run(&s(&["reset-health"]), &e, NOW, &fetcher, &outbox).unwrap();
        assert_eq!(out, "sending domain reset to warming");
        assert!(
            std::fs::read_to_string(state.join("events.jsonl"))
                .unwrap()
                .lines()
                .any(|line| line == format!("{{\"type\":\"health_reset\",\"at\":{NOW}}}")),
            "the bare reset writes the domain-free event"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ten_thousand_discovered_contacts_dedupe_linearly_and_preserve_order() {
        const N: usize = 10_000;
        reset_contact_compares();
        let seeds: Vec<String> = (0..N).map(|i| format!("seed{i}@p.example")).collect();
        let mut existing = ContactAccumulator::new(seeds.clone());

        // Ten thousand new addresses, then the same ten thousand again as the
        // duplicate rows a re-fetched app-ads.txt produces.
        for i in 0..N {
            assert!(existing.insert(&format!("new{i}@p.example")));
        }
        for i in 0..N {
            assert!(!existing.insert(&format!("new{i}@p.example")));
        }
        // An address that was already in the snapshot is also a duplicate.
        assert!(!existing.insert(&seeds[0]));

        let expected: Vec<String> = seeds
            .iter()
            .cloned()
            .chain((0..N).map(|i| format!("new{i}@p.example")))
            .collect();
        assert_eq!(
            existing.ordered, expected,
            "discovery order is the ordered vector's order"
        );
        assert_eq!(existing.seen.len(), 2 * N);
        let probes = contact_compares();
        println!(
            "contact_dedupe contacts={} probes={probes} pre_fix_worst={}",
            2 * N,
            (2 * N) * (2 * N) / 2
        );
        assert_eq!(
            probes,
            2 * N as u64 + 1,
            "one set membership probe per discovered address"
        );
        assert!(
            probes < (N * N / 10) as u64,
            "a Vec::contains scan is quadratic: {probes} probes for {N} contacts"
        );
    }
}
