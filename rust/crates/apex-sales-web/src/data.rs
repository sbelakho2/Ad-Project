//! Loading the engine's state for a request: the snapshot, the learned
//! models and the configuration, loaded exactly the way the `apex-sales`
//! CLI loads them.
//!
//! Every page renders from one [`RequestData`], rebuilt per request (the
//! state directory is tiny and the learner is a pure fold over its event
//! history, so a request always sees the durable truth on disk). Nothing is
//! cached: an operator's action is visible on the next page load because the
//! next page load re-reads the directory.

use std::path::{Path, PathBuf};

use apex_sales::cli::{self, Env};
use apex_sales::engine::{learn, Config, Learned};
use apex_sales::model::UnixSecs;
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

/// Everything a page renders from.
#[derive(Debug, Clone)]
pub struct RequestData {
    /// The durable publisher/contact/event history.
    pub snap: Snapshot,
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
}

/// Load the engine state for one request, or a human reason why that failed.
pub fn load(state: &WebState) -> Result<RequestData, String> {
    load_from(&state.config.env, state.now_secs())
}

/// Load the engine state from an environment map as of `now`.
pub fn load_from(env: &Env, now: UnixSecs) -> Result<RequestData, String> {
    let config = RequestData::engine_config(env)?;
    let dir_path = RequestData::state_dir(env);
    let dir = StateDir::open(&dir_path).map_err(|error| error.to_string())?;
    let snap = dir.load().map_err(|error| error.to_string())?;
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
}
