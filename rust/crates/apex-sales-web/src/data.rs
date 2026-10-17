//! Loading the engine's state for a request: the snapshot, the learned
//! models and the configuration, loaded exactly the way the `apex-sales`
//! CLI loads them.
//!
//! Every page renders from one [`RequestData`]. The durable snapshot (the
//! four JSONL stores the engine folds) is re-read only when one of its files
//! changes: [`SnapshotCache`] keys the parsed snapshot on each file's length,
//! modification time and (on Unix) ctime and inode, the same invalidation
//! semantics as the control plane's ledger cache, so a page load whose state
//! did not move answers from the verified parse instead of re-reading and
//! re-parsing megabytes of JSON.
//! The learner still runs per request (a pure fold over the history, as of
//! the request's `now`), so an operator's action is visible on the next page
//! load: the next page load sees the appended file, re-parses it and
//! re-derives every model. [`load_from`] keeps the uncached behavior for
//! callers that hold no server state (fixtures, tests).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use apex_sales::cli::{self, Env};
use apex_sales::engine::{learn, Config, Learned};
use apex_sales::model::UnixSecs;
use apex_sales::policy::{SuppressionReason, Suppressions};
use apex_sales::store::{Snapshot, StateDir};

use crate::state::WebState;

/// The state directory environment variable.
pub const DIR_ENV: &str = "APEX_SALES_DIR";
/// The default state directory the CLI documents.
pub const DEFAULT_DIR: &str = ".apex/sales";
/// The outbox environment variable.
pub const OUTBOX_ENV: &str = "APEX_SALES_OUTBOX";
/// The mode environment variable.
pub const MODE_ENV: &str = "APEX_SALES_MODE";
/// The largest `.eml` preview read; a bigger file renders as a refusal, not
/// a load.
pub const MAX_PREVIEW_BYTES: u64 = 262_144;

/// The store files the snapshot is parsed from, in the order
/// [`StateDir::load`] reads them.
const SNAPSHOT_FILES: [&str; 4] = [
    "publishers.jsonl",
    "contacts.jsonl",
    "events.jsonl",
    "approvals.jsonl",
];

/// One file's cache identity: byte length, modification time (`None` when the
/// filesystem does not expose one), and on Unix the ctime and inode, which an
/// unprivileged writer cannot restore. A same-length rewrite that puts the
/// mtime back (`touch -t`) still changes the ctime, so it invalidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
    #[cfg(unix)]
    ino: u64,
}

fn identity_of(path: &Path) -> Option<FileIdentity> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        Some(FileIdentity {
            len: metadata.len(),
            modified,
            ctime: metadata.ctime(),
            ctime_nsec: metadata.ctime_nsec(),
            ino: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Some(FileIdentity {
            len: metadata.len(),
            modified,
        })
    }
}

/// A cached parse: the store files' identities at parse time and the parsed
/// snapshot itself.
type CachedSnapshot = (Vec<Option<FileIdentity>>, Arc<Snapshot>);

/// The mtime-keyed snapshot cache: one verified parse per unchanged store.
///
/// A load stats the four store files first, and serves the cached snapshot
/// only when every file's identity (length, modification time and on Unix
/// ctime and inode) is unchanged since the parse. Any change (or a file
/// appearing, vanishing or losing its modification time) re-parses. The
/// parse counter is the seam tests use to prove n reads are one parse until a
/// file changes.
#[derive(Debug, Default)]
pub struct SnapshotCache {
    cached: Mutex<Option<CachedSnapshot>>,
    parses: AtomicU64,
}

impl SnapshotCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many snapshot parses this cache has performed.
    pub fn parses(&self) -> u64 {
        self.parses.load(Ordering::Relaxed)
    }

    /// The snapshot of `dir`, re-parsed only when a store file changed.
    ///
    /// The identities are stat'ed before the parse, so a file change that
    /// races the read is treated as changed on the next load (an extra
    /// re-parse, never a stale snapshot).
    fn load(&self, dir: &StateDir) -> std::io::Result<Arc<Snapshot>> {
        let identities: Vec<Option<FileIdentity>> = SNAPSHOT_FILES
            .iter()
            .map(|name| identity_of(&dir.root().join(name)))
            .collect();
        if let Ok(guard) = self.cached.lock() {
            if let Some((cached, snap)) = guard.as_ref() {
                if *cached == identities {
                    return Ok(Arc::clone(snap));
                }
            }
        }
        let snap = Arc::new(dir.load()?);
        self.parses.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut guard) = self.cached.lock() {
            *guard = Some((identities, Arc::clone(&snap)));
        }
        Ok(snap)
    }
}

/// Everything a page renders from.
#[derive(Debug, Clone)]
pub struct RequestData {
    /// The durable publisher/contact/event history. Shared with the cache
    /// when the store is unchanged, so a page load without a state change
    /// copies no history.
    pub snap: Arc<Snapshot>,
    /// The models and ledgers rebuilt from that history.
    pub learned: Learned,
    /// The engine configuration (from the captured environment).
    pub config: Config,
    /// The engine's "now", in Unix seconds.
    pub now: UnixSecs,
    /// The state directory the data came from.
    pub dir: PathBuf,
}

impl RequestData {
    /// The engine configuration from the captured environment map, exactly
    /// as `apex-sales` reads it.
    pub fn engine_config(env: &Env) -> Result<Config, String> {
        cli::config_from_env(env)
    }

    /// The state directory from the captured environment map.
    pub fn state_dir(env: &Env) -> PathBuf {
        PathBuf::from(
            env.get(DIR_ENV)
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_DIR),
        )
    }

    /// An O(1)-lookup view of the learned suppressions for pages that
    /// classify every contact or item.
    pub fn suppression_index(&self) -> SuppressionIndex<'_> {
        SuppressionIndex::build(&self.learned.suppressions)
    }
}

/// A hash-lookup view of [`Suppressions`] with exactly the semantics of
/// [`Suppressions::check`]: an address match wins over a publisher match, and
/// the first recorded entry of a list wins. Pages that classify every contact
/// once per row build this once per render.
#[derive(Debug)]
pub struct SuppressionIndex<'a> {
    emails: HashMap<&'a str, SuppressionReason>,
    publishers: HashMap<&'a str, SuppressionReason>,
}

impl<'a> SuppressionIndex<'a> {
    /// Index `suppressions` by address and by publisher key.
    pub fn build(suppressions: &'a Suppressions) -> Self {
        let exported = suppressions.emails();
        let mut emails = HashMap::with_capacity(exported.len());
        for (email, reason) in exported {
            emails.entry(email.as_str()).or_insert(*reason);
        }
        let exported = suppressions.publishers();
        let mut publishers = HashMap::with_capacity(exported.len());
        for (publisher, reason) in exported {
            publishers.entry(publisher.as_str()).or_insert(*reason);
        }
        Self { emails, publishers }
    }

    /// Why the contact's address or publisher is suppressed, if either is.
    pub fn check(&self, contact: &apex_sales::model::Contact) -> Option<SuppressionReason> {
        self.emails
            .get(contact.email.as_str())
            .or_else(|| self.publishers.get(contact.publisher_key.as_str()))
            .copied()
    }
}

/// Load the engine state for one request, or a human reason why that failed.
///
/// The durable snapshot comes from the server's [`SnapshotCache`] (re-read
/// only when a store file changed); the models are rebuilt from it as of the
/// request's `now`.
pub fn load(state: &WebState) -> Result<RequestData, String> {
    let config = RequestData::engine_config(&state.config.env)?;
    let dir_path = RequestData::state_dir(&state.config.env);
    let dir = StateDir::open(&dir_path).map_err(|error| error.to_string())?;
    let snap = state
        .snapshots
        .load(&dir)
        .map_err(|error| error.to_string())?;
    let now = state.now_secs();
    let learned = learn(&snap, &config, now);
    Ok(RequestData {
        snap,
        learned,
        config,
        now,
        dir: dir_path,
    })
}

/// Load the engine state from an environment map as of `now`, uncached: the
/// whole snapshot is parsed on every call. Fixtures and tests use this so
/// their output is a pure function of the store on disk.
pub fn load_from(env: &Env, now: UnixSecs) -> Result<RequestData, String> {
    let config = RequestData::engine_config(env)?;
    let dir_path = RequestData::state_dir(env);
    let dir = StateDir::open(&dir_path).map_err(|error| error.to_string())?;
    let snap = Arc::new(dir.load().map_err(|error| error.to_string())?);
    let learned = learn(&snap, &config, now);
    Ok(RequestData {
        snap,
        learned,
        config,
        now,
        dir: dir_path,
    })
}

/// How this deployment hands messages to the outside world, mirroring
/// [`apex_sales::cli::mailer_from_env`]: `.eml` files in shadow/off mode or
/// when [`OUTBOX_ENV`] is set, otherwise the local MTA's `sendmail`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// Messages are written as `.eml` files into the named directory.
    Outbox(PathBuf),
    /// Messages are piped to the local MTA's `sendmail`.
    Sendmail,
}

/// The transport this environment would use.
pub fn transport(env: &Env, sales_dir: &Path) -> Transport {
    if let Some(outbox) = env
        .get(OUTBOX_ENV)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Transport::Outbox(PathBuf::from(outbox));
    }
    let mode = env
        .get(MODE_ENV)
        .and_then(|value| apex_sales::decision::Autonomy::parse(value))
        .unwrap_or(apex_sales::decision::Autonomy::Shadow);
    match mode {
        apex_sales::decision::Autonomy::Shadow | apex_sales::decision::Autonomy::Off => {
            Transport::Outbox(sales_dir.join("outbox"))
        }
        _ => Transport::Sendmail,
    }
}

/// The transport's short label for tables.
pub fn transport_label(transport: &Transport) -> &'static str {
    match transport {
        Transport::Outbox(_) => "file outbox",
        Transport::Sendmail => "sendmail",
    }
}

/// One `.eml` file in the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxFile {
    /// File name (`.eml`).
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Last modification time, Unix milliseconds when available.
    pub modified_ms: Option<i64>,
}

/// List the outbox directory, newest first. A missing directory is an empty
/// listing, not an error (nothing has been written yet).
pub fn outbox_files(dir: &Path) -> Result<Vec<OutboxFile>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", dir.display())),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !valid_outbox_name(&name) {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            continue;
        }
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX));
        files.push(OutboxFile {
            name,
            size: metadata.len(),
            modified_ms,
        });
    }
    files.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(files)
}

/// Whether a requested preview name is a plain `.eml` file name: only
/// `[A-Za-z0-9._-]`, ends in `.eml`, at most 128 bytes, and never `.`, `..`
/// or a name containing `..`, so no path component can traverse.
pub fn valid_outbox_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.ends_with(".eml")
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Read one `.eml` for preview, refusing names that traverse or sizes past
/// [`MAX_PREVIEW_BYTES`]. Returns the raw message text.
pub fn read_outbox_file(dir: &Path, name: &str) -> Result<String, String> {
    if !valid_outbox_name(name) {
        return Err("Not available (the file name is not a plain .eml name)".into());
    }
    let path = dir.join(name);
    let metadata = std::fs::metadata(&path)
        .map_err(|_| format!("Not available (no outbox file named {name})"))?;
    if !metadata.is_file() {
        return Err(format!("Not available ({name} is not a regular file)"));
    }
    if metadata.len() > MAX_PREVIEW_BYTES {
        return Err(format!(
            "Not available ({name} is {} bytes; previews are capped at {MAX_PREVIEW_BYTES})",
            metadata.len()
        ));
    }
    std::fs::read_to_string(&path).map_err(|error| format!("Not available ({name}: {error})"))
}

/// One line of the engine's `decisions.jsonl` audit log, as written by the
/// CLI's send loop (see `apex-sales` `cli.rs`).
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionLine {
    /// When the decision was recorded (Unix seconds).
    pub at: Option<UnixSecs>,
    /// The touch id.
    pub touch_id: String,
    /// The recipient.
    pub email: String,
    /// The recorded verdict (`sent`).
    pub verdict: String,
    /// The expected value as recorded.
    pub expected_value: Option<f64>,
}

/// Read `decisions.jsonl` from the state directory, newest last. A missing
/// file is an empty log; malformed lines are skipped and their count
/// returned with the rows.
pub fn read_decisions(dir: &Path) -> (Vec<DecisionLine>, usize) {
    let text = match std::fs::read_to_string(dir.join("decisions.jsonl")) {
        Ok(text) => text,
        Err(_) => return (Vec::new(), 0),
    };
    let mut rows = Vec::new();
    let mut skipped = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Some(value) = apex_sales::json::parse(line).ok() else {
            skipped += 1;
            continue;
        };
        let Some(touch_id) = value.str_at("touchId").map(str::to_string) else {
            skipped += 1;
            continue;
        };
        rows.push(DecisionLine {
            at: value.get("at").and_then(apex_sales::json::Value::as_i64),
            touch_id,
            email: value.str_at("email").unwrap_or_default().to_string(),
            verdict: value.str_at("verdict").unwrap_or_default().to_string(),
            expected_value: value.f64_at("expectedValue"),
        });
    }
    (rows, skipped)
}

/// Whether `value` is a strict publisher domain: lowercase `[a-z0-9.-]`,
/// at most 253 bytes, no traversal, and a shape the engine's own
/// [`apex_sales::model::is_domain`] accepts.
pub fn valid_domain(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !value.contains("..")
        && !value.starts_with('.')
        && !value.ends_with('.')
        && value == value.to_ascii_lowercase()
        && apex_sales::model::is_domain(value)
}

/// An action target: a mailbox or a publisher key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A normalized mailbox.
    Email(String),
    /// A publisher key (normalized domain).
    Publisher(String),
}

/// Parse an action target the way the CLI does (`record`/`suppress`): an
/// address when it contains `@` and normalizes, otherwise a publisher key
/// that must be a strict domain.
pub fn parse_target(raw: &str) -> Option<Target> {
    if raw.contains('@') {
        return apex_sales::model::normalize_email(raw).map(Target::Email);
    }
    let key = raw.trim().to_ascii_lowercase();
    valid_domain(&key).then_some(Target::Publisher(key))
}

/// Whether an ID is a planned touch id: `t` plus 16 lowercase hex digits,
/// the exact shape [`apex_sales::engine::touch_id`] produces.
pub fn valid_touch_id(value: &str) -> bool {
    value.strip_prefix('t').is_some_and(|rest| {
        rest.len() == 16
            && rest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_sales::engine::touch_id;

    #[test]
    fn transport_mirrors_the_cli_transport() {
        let dir = PathBuf::from("/state");
        let mut env = Env::new();
        assert_eq!(
            transport(&env, &dir),
            Transport::Outbox(PathBuf::from("/state/outbox")),
            "shadow is the default mode"
        );
        env.insert(MODE_ENV.into(), "autonomous".into());
        assert_eq!(transport(&env, &dir), Transport::Sendmail);
        env.insert(MODE_ENV.into(), "off".into());
        assert_eq!(
            transport(&env, &dir),
            Transport::Outbox(PathBuf::from("/state/outbox"))
        );
        env.insert(OUTBOX_ENV.into(), "/tmp/box".into());
        env.insert(MODE_ENV.into(), "autonomous".into());
        assert_eq!(
            transport(&env, &dir),
            Transport::Outbox(PathBuf::from("/tmp/box"))
        );
        assert_eq!(transport_label(&Transport::Sendmail), "sendmail");
    }

    #[test]
    fn outbox_names_are_strict() {
        for good in ["a.eml", "2026-10-16_x.eml", "A.b-1_2.eml"] {
            assert!(valid_outbox_name(good), "{good}");
        }
        for bad in [
            "",
            ".",
            "..",
            ".eml",
            ".hidden.eml",
            "../x.eml",
            "a/b.eml",
            "a\\b.eml",
            "x.txt",
            "x.eml.exe",
            "a..b.eml",
            "with space.eml",
            &"x".repeat(120),
        ] {
            assert!(!valid_outbox_name(bad), "{bad}");
        }
    }

    #[test]
    fn preview_refuses_traversal_and_oversize() {
        let dir =
            std::env::temp_dir().join(format!("apex-sales-web-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ok.eml"), "Subject: hi\n\nhello").unwrap();
        assert_eq!(
            read_outbox_file(&dir, "ok.eml").unwrap(),
            "Subject: hi\n\nhello"
        );
        for bad in ["../ok.eml", "..", "a/b.eml", ".hidden.eml"] {
            let error = read_outbox_file(&dir, bad).unwrap_err();
            assert!(error.starts_with("Not available"), "{bad}: {error}");
        }
        let big = vec![b'x'; (MAX_PREVIEW_BYTES + 1) as usize];
        std::fs::write(dir.join("big.eml"), &big).unwrap();
        let error = read_outbox_file(&dir, "big.eml").unwrap_err();
        assert!(error.contains("capped"), "{error}");
        // A directory named like an eml is not a file.
        std::fs::create_dir_all(dir.join("dir.eml")).unwrap();
        assert!(read_outbox_file(&dir, "dir.eml")
            .unwrap_err()
            .contains("not a regular file"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn domains_and_touch_ids_are_strictly_validated() {
        assert!(valid_domain("studio.example"));
        assert!(valid_domain("a-b.co"));
        for bad in [
            "UPPER.example",
            "..",
            "a..b.com",
            ".a.com",
            "a.com.",
            "-a.com",
            "a b.com",
            "localhost",
            &format!("{}.com", "a".repeat(250)),
        ] {
            assert!(!valid_domain(bad), "{bad}");
        }
        let good = touch_id("a@b.io", 0, 20_000);
        assert!(valid_touch_id(&good));
        assert!(!valid_touch_id("t1"));
        assert!(!valid_touch_id("T0123456789abcdef"));
        assert!(!valid_touch_id("t0123456789ABCDEF"));
    }

    #[test]
    fn targets_mirror_the_cli_split() {
        assert_eq!(
            parse_target("Maria@Studio.Example"),
            Some(Target::Email("maria@studio.example".into()))
        );
        assert_eq!(
            parse_target("Studio.Example"),
            Some(Target::Publisher("studio.example".into()))
        );
        assert_eq!(parse_target("not an email@"), None);
        assert_eq!(parse_target("../etc/passwd"), None);
    }

    #[test]
    fn decisions_parse_and_skip_malformed_lines() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-dec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("decisions.jsonl"),
            "{\"at\":100,\"touchId\":\"t0123456789abcdef\",\"email\":\"a@b.io\",\"verdict\":\"sent\",\"expectedValue\":2.5}\nnot json\n{\"nope\":1}\n",
        )
        .unwrap();
        let (rows, skipped) = read_decisions(&dir);
        assert_eq!(rows.len(), 1);
        assert_eq!(skipped, 2);
        assert_eq!(rows[0].touch_id, "t0123456789abcdef");
        assert_eq!(rows[0].expected_value, Some(2.5));
        let (rows, skipped) = read_decisions(&dir.join("missing"));
        assert!(rows.is_empty() && skipped == 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outbox_listing_sorts_newest_first_and_skips_foreign_names() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-box-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            outbox_files(&dir).unwrap().is_empty(),
            "missing dir is empty"
        );
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.eml"), "x").unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.join("sub.eml")).unwrap();
        let files = outbox_files(&dir).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "old.eml");
        assert_eq!(files[0].size, 1);
        assert!(files[0].modified_ms.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Snapshot cache
    // -----------------------------------------------------------------------

    fn seeded_sales_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = StateDir::open(&dir).expect("open store");
        store
            .put_publisher(&apex_sales::model::Publisher::new(
                "studio.example",
                "Studio",
            ))
            .expect("publisher");
        store
            .put_contact(&apex_sales::model::Contact {
                email: "maria@studio.example".into(),
                name: None,
                role: None,
                publisher_key: "studio.example".into(),
                kind: apex_sales::model::ContactKind::Named,
                source: "seed".into(),
                consent: true,
            })
            .expect("contact");
        store
            .put_event(&apex_sales::store::Event::Touch {
                at: 1_700_000_000,
                email: "maria@studio.example".into(),
                publisher: "studio.example".into(),
                from: None,
                step: 0,
                arm: "t1-bump".into(),
                message_id: "m1".into(),
                local_hour: 9,
                predicted: Some(0.04),
            })
            .expect("event");
        dir
    }

    fn env_for(dir: &Path) -> Env {
        let mut env = Env::new();
        env.insert(DIR_ENV.into(), dir.display().to_string());
        env
    }

    fn state_for(dir: &Path) -> WebState {
        let config = crate::config::WebConfig::from_env(&env_for(dir)).expect("config");
        WebState::new(config)
    }

    /// The parse-count seam: n loads over an unchanged store are exactly one
    /// parse, and any store change (an appended line, a same-length rewrite
    /// that only moves the mtime, a file appearing) re-parses exactly once.
    ///
    /// Verified to fail on the pre-fix code: restoring the pre-fix `load`
    /// (which called `StateDir::load` directly, bypassing `SnapshotCache`)
    /// leaves `parses()` at 0 and the first assertion fails.
    #[test]
    fn snapshot_parses_once_per_unchanged_store_then_once_per_change() {
        let dir = seeded_sales_dir("cache");
        let state = state_for(&dir);

        // Five reads: one parse, and every read sees the durable truth.
        for _ in 0..5 {
            let data = load(&state).expect("load");
            assert_eq!(data.snap.contacts.len(), 1);
            assert_eq!(data.snap.events.len(), 1);
        }
        assert_eq!(
            state.snapshots.parses(),
            1,
            "five loads over an unchanged store must be one parse"
        );

        // The cached reader agrees with the uncached loader byte-for-value.
        let cached = load(&state).expect("cached");
        let uncached = load_from(&env_for(&dir), state.now_secs()).expect("uncached");
        assert_eq!(cached.snap.contacts, uncached.snap.contacts);
        assert_eq!(cached.snap.events, uncached.snap.events);
        assert_eq!(cached.snap.publishers, uncached.snap.publishers);
        assert_eq!(cached.snap.approvals, uncached.snap.approvals);

        // An appended event changes length and mtime: one more parse, and
        // the new event is visible.
        let store = StateDir::open(&dir).expect("reopen");
        store
            .put_event(&apex_sales::store::Event::Outcome {
                at: 1_700_000_100,
                email: Some("maria@studio.example".into()),
                publisher: None,
                outcome: apex_sales::model::Outcome::Connected,
                note: None,
            })
            .expect("append");
        let data = load(&state).expect("after append");
        assert_eq!(data.snap.events.len(), 2);
        assert_eq!(
            state.snapshots.parses(),
            2,
            "a changed store re-parses once"
        );
        let _ = load(&state).expect("again");
        assert_eq!(state.snapshots.parses(), 2, "and then stays cached");

        // A same-length rewrite: only the mtime moves, and that must
        // invalidate too (the identity is length plus modification time).
        let path = dir.join("contacts.jsonl");
        let before = std::fs::read_to_string(&path).expect("read contacts");
        let mutated = before.replacen("studio.example", "studio.examplf", 1);
        assert_eq!(before.len(), mutated.len(), "the rewrite keeps the length");
        std::fs::write(&path, &mutated).expect("rewrite");
        let _ = load(&state).expect("after rewrite");
        assert_eq!(
            state.snapshots.parses(),
            3,
            "a same-length rewrite must invalidate through the mtime"
        );

        // A file appearing is a change of identity (`None` to a real file).
        std::fs::write(dir.join("approvals.jsonl"), "").expect("touch approvals");
        let _ = load(&state).expect("after new file");
        assert_eq!(state.snapshots.parses(), 4, "a new store file re-parses");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A same-length rewrite whose mtime is restored (`touch -t`) must still
    /// invalidate the snapshot: the Unix ctime is part of the file identity.
    /// A pair-keyed cache serves the stale snapshot here; this test fails on
    /// it.
    #[test]
    fn snapshot_cache_detects_a_same_length_rewrite_with_restored_mtime() {
        let dir = seeded_sales_dir("cache-ctime");
        let state = state_for(&dir);
        let _ = load(&state).expect("prime");
        assert_eq!(state.snapshots.parses(), 1);
        assert!(load(&state)
            .expect("first read")
            .snap
            .contacts
            .contains_key("maria@studio.example"));

        let path = dir.join("contacts.jsonl");
        let metadata = std::fs::metadata(&path).expect("stat");
        let modified = metadata.modified().expect("mtime");
        let before = std::fs::read_to_string(&path).expect("read contacts");
        let mutated = before.replacen("maria@studio.example", "mario@studio.example", 1);
        assert_eq!(before.len(), mutated.len(), "the rewrite keeps the length");
        assert_ne!(before, mutated);
        std::fs::write(&path, &mutated).expect("rewrite");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(modified)
            .expect("restore mtime");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("stat")
                .modified()
                .expect("mtime"),
            modified,
            "the mtime is restored to the pair-keyed value"
        );

        let data = load(&state).expect("after tamper");
        assert_eq!(
            state.snapshots.parses(),
            2,
            "the restored-mtime rewrite must invalidate the cache"
        );
        assert!(
            data.snap.contacts.contains_key("mario@studio.example"),
            "the new content is parsed"
        );
        assert!(!data.snap.contacts.contains_key("maria@studio.example"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_cache_stats_every_missing_or_changed_file() {
        let dir = seeded_sales_dir("cache-missing");
        let state = state_for(&dir);
        let _ = load(&state).expect("prime");
        assert_eq!(state.snapshots.parses(), 1);

        // Removing a store file (or any identity change to any of the four)
        // goes through a fresh parse: the snapshot must never outlive its
        // bytes on disk.
        std::fs::remove_file(dir.join("events.jsonl")).expect("remove");
        let data = load(&state).expect("after removal");
        assert!(data.snap.events.is_empty());
        assert_eq!(state.snapshots.parses(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Suppression index
    // -----------------------------------------------------------------------

    /// The index must answer exactly like the engine's own linear scan, at
    /// scale: 10k suppressed addresses, 10k suppressed publishers and 10k
    /// contacts (10_000 contacts is the protocol's reference size).
    #[test]
    fn suppression_index_matches_the_engine_scan_at_ten_thousand_contacts() {
        use apex_sales::model::ContactKind;
        use apex_sales::policy::{SuppressionReason, Suppressions};

        let mut suppressions = Suppressions::default();
        for i in 0..10_000 {
            suppressions.suppress_email(
                &format!("suppressed{i}@p{i}.example"),
                if i % 3 == 0 {
                    SuppressionReason::OptOut
                } else {
                    SuppressionReason::HardBounce
                },
            );
        }
        for i in 0..10_000 {
            suppressions.suppress_publisher(&format!("p{i}.example"), SuppressionReason::Complaint);
        }
        // A first entry wins over a later duplicate, exactly like the scan;
        // the duplicate add leaves the recorded reason untouched.
        suppressions.suppress_email("suppressed0@p0.example", SuppressionReason::Operator);
        suppressions.suppress_publisher("p0.example", SuppressionReason::Operator);
        assert_eq!(suppressions.emails().len(), 10_000);
        assert_eq!(suppressions.publishers().len(), 10_000);

        let contact = |email: &str, publisher: &str| apex_sales::model::Contact {
            email: email.to_string(),
            name: None,
            role: None,
            publisher_key: publisher.to_string(),
            kind: ContactKind::Named,
            source: "seed".into(),
            consent: true,
        };

        let index = SuppressionIndex::build(&suppressions);
        // Suppressed by address, suppressed by publisher, or clean; and the
        // address match must win over the publisher match.
        let cases: Vec<apex_sales::model::Contact> = (0..10_000)
            .map(|i| match i % 3 {
                0 => contact(&format!("suppressed{i}@p{i}.example"), "clean.example"),
                1 => contact(&format!("clean{i}@p{i}.example"), &format!("p{i}.example")),
                _ => contact(&format!("clean{i}@p{i}.example"), "clean.example"),
            })
            .collect();
        for (i, c) in cases.iter().enumerate() {
            assert_eq!(
                index.check(c),
                suppressions.check(c),
                "contact {i} must classify identically"
            );
        }
        assert_eq!(
            index.check(&contact("suppressed0@p0.example", "p0.example")),
            Some(SuppressionReason::OptOut),
            "the first recorded address entry wins, like the scan"
        );
    }
}
