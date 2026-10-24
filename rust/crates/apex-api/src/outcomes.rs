//! The frozen sales-outcomes appender.
//!
//! One JSON object per line, exactly the frozen shape the sales engine's
//! `import-outcomes` consumes:
//!
//! ```text
//! {"atUnixMs":<int>,"publisher":"<slug>","kind":"connected"|"paid"|"watch_signup","ref":"<id>"}
//! ```
//!
//! The file is `APEX_SALES_OUTCOMES_FILE` (default
//! `.apex/sales/outcomes.jsonl`). Writes are write-once per `(kind, ref)`:
//! a sidecar JSONL beside the outcomes file (`<outcomes>.seen.jsonl`) records
//! every key that was appended, so a restart, a replay or a repeated webhook
//! delivery never appends the same outcome twice. The sidecar is written
//! after the outcomes line, so a crash in between duplicates one line, which
//! the importer deduplicates by `(publisher, kind, ref)`; losing an outcome
//! would be worse.
//!
//! Every call site treats the appender as best-effort: the server-side
//! success of the primary action (a validated connection, an accepted
//! payment event, a stored watch sign-up) never depends on the outcomes file
//! being writable. Failures are logged by the caller.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

/// Environment variable naming the outcomes file.
pub const OUTCOMES_FILE_ENV: &str = "APEX_SALES_OUTCOMES_FILE";
/// The default outcomes file (the sales engine's default too).
pub const DEFAULT_OUTCOMES_PATH: &str = ".apex/sales/outcomes.jsonl";
/// The suffix of the seen sidecar beside the outcomes file.
pub const SEEN_SUFFIX: &str = ".seen.jsonl";
/// The frozen outcome kinds.
pub const KINDS: [&str; 4] = ["connected", "paid", "watch_signup", "unsubscribe"];

/// The frozen line shape; the field order of this struct is the byte order
/// of every appended line.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FrozenOutcome<'a> {
    at_unix_ms: i64,
    publisher: &'a str,
    kind: &'a str,
    #[serde(rename = "ref")]
    reference: &'a str,
}

/// Why an append was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutcomesError {
    /// The kind is not one of [`KINDS`].
    #[error("unknown outcomes kind '{0}'")]
    UnknownKind(String),
    /// A required field is empty.
    #[error("the outcome has no {0}")]
    Missing(&'static str),
    /// The file could not be written.
    #[error("outcomes storage: {0}")]
    Storage(String),
}

/// The append-only outcomes file with the write-once seen sidecar.
#[derive(Debug)]
pub struct OutcomesAppender {
    path: PathBuf,
    seen_path: PathBuf,
    state: Mutex<SeenState>,
}

#[derive(Debug, Default)]
struct SeenState {
    hydrated: bool,
    seen: HashSet<String>,
}

impl OutcomesAppender {
    /// An appender for `path`; nothing is created until the first append.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut seen = path.clone().into_os_string();
        seen.push(SEEN_SUFFIX);
        Self {
            seen_path: PathBuf::from(seen),
            path,
            state: Mutex::new(SeenState::default()),
        }
    }

    /// Build from the environment: [`OUTCOMES_FILE_ENV`] when set, else
    /// [`DEFAULT_OUTCOMES_PATH`].
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let path = lookup(OUTCOMES_FILE_ENV)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_OUTCOMES_PATH.to_string());
        Self::new(path)
    }

    /// The outcomes file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The seen sidecar path.
    pub fn seen_path(&self) -> &Path {
        &self.seen_path
    }

    /// Append one outcome. `Ok(true)` means this call appended the line;
    /// `Ok(false)` means the `(kind, ref)` pair was already appended by this
    /// appender (this process or a previous one).
    pub fn append(
        &self,
        at_unix_ms: i64,
        publisher: &str,
        kind: &str,
        reference: &str,
    ) -> Result<bool, OutcomesError> {
        if !KINDS.contains(&kind) {
            return Err(OutcomesError::UnknownKind(kind.to_string()));
        }
        let publisher = publisher.trim();
        let reference = reference.trim();
        if publisher.is_empty() {
            return Err(OutcomesError::Missing("publisher"));
        }
        if reference.is_empty() {
            return Err(OutcomesError::Missing("ref"));
        }
        let key = format!("{kind}:{reference}");
        let mut state = self
            .state
            .lock()
            .map_err(|_| OutcomesError::Storage("the outcomes lock is poisoned".into()))?;
        if !state.hydrated {
            self.hydrate(&mut state)?;
        }
        if state.seen.contains(&key) {
            return Ok(false);
        }
        let line = serde_json::to_string(&FrozenOutcome {
            at_unix_ms,
            publisher,
            kind,
            reference,
        })
        .map_err(|error| OutcomesError::Storage(format!("outcome serialization: {error}")))?;
        append_line(&self.path, &format!("{line}\n"))?;
        state.seen.insert(key);
        let seen_line = serde_json::json!({ "kind": kind, "ref": reference });
        let seen_text = format!("{seen_line}\n");
        // The outcome line is durable; a sidecar failure is reported so the
        // caller can log it, and in-process dedup still holds.
        append_line(&self.seen_path, &seen_text)?;
        Ok(true)
    }

    /// Load the seen keys from the sidecar. Unparseable lines are skipped:
    /// the sidecar is our own append-only record and a torn line must not
    /// wedge every future append.
    fn hydrate(&self, state: &mut SeenState) -> Result<(), OutcomesError> {
        let raw = match std::fs::read_to_string(&self.seen_path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                state.hydrated = true;
                return Ok(());
            }
            Err(error) => {
                return Err(OutcomesError::Storage(format!(
                    "{} could not be read: {error}",
                    self.seen_path.display()
                )))
            }
        };
        for line in raw.lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let (Some(kind), Some(reference)) = (
                value.get("kind").and_then(serde_json::Value::as_str),
                value.get("ref").and_then(serde_json::Value::as_str),
            ) else {
                continue;
            };
            state.seen.insert(format!("{kind}:{reference}"));
        }
        state.hydrated = true;
        Ok(())
    }
}

/// Append one line, creating the parent directory on the way.
fn append_line(path: &Path, line: &str) -> Result<(), OutcomesError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                OutcomesError::Storage(format!("{}: parent directory: {error}", path.display()))
            })?;
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            OutcomesError::Storage(format!("{} could not be opened: {error}", path.display()))
        })?;
    file.write_all(line.as_bytes()).map_err(|error| {
        OutcomesError::Storage(format!(
            "{} could not be appended to: {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn appender(dir: &tempfile::TempDir) -> OutcomesAppender {
        OutcomesAppender::new(dir.path().join("sales").join("outcomes.jsonl"))
    }

    #[test]
    fn appended_lines_are_the_frozen_shape_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let appender = appender(&dir);
        assert!(appender
            .append(1_700_000_000_000, "northstar", "connected", "evt_1")
            .expect("append"));
        let raw = std::fs::read_to_string(appender.path()).unwrap();
        assert_eq!(
            raw,
            "{\"atUnixMs\":1700000000000,\"publisher\":\"northstar\",\"kind\":\"connected\",\"ref\":\"evt_1\"}\n"
        );
    }

    #[test]
    fn write_once_per_kind_and_ref() {
        let dir = tempfile::tempdir().unwrap();
        let appender = appender(&dir);
        assert!(appender
            .append(1, "northstar", "paid", "psp_1")
            .expect("first"));
        assert!(!appender
            .append(2, "northstar", "paid", "psp_1")
            .expect("duplicate"));
        // A different kind or ref is a different outcome.
        assert!(appender
            .append(3, "northstar", "connected", "psp_1")
            .expect("other kind"));
        assert!(appender
            .append(4, "northstar", "paid", "psp_2")
            .expect("other ref"));
        assert_eq!(
            std::fs::read_to_string(appender.path())
                .unwrap()
                .lines()
                .count(),
            3
        );
        let sidecar = std::fs::read_to_string(appender.seen_path()).unwrap();
        assert_eq!(sidecar.lines().count(), 3);
        assert!(
            sidecar.contains(r#"{"kind":"paid","ref":"psp_1"}"#),
            "{sidecar}"
        );
    }

    #[test]
    fn the_seen_sidecar_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outcomes.jsonl");
        let first = OutcomesAppender::new(&path);
        assert!(first
            .append(1, "northstar", "connected", "evt_1")
            .expect("a"));
        drop(first);
        let second = OutcomesAppender::new(&path);
        assert!(
            !second
                .append(2, "northstar", "connected", "evt_1")
                .expect("b"),
            "a fresh appender reads the sidecar and deduplicates"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    }

    #[test]
    fn malformed_sidecar_lines_are_skipped_and_unknown_kinds_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outcomes.jsonl");
        std::fs::write(
            format!("{}.seen.jsonl", path.display()),
            "not json\n{\"kind\":\"paid\",\"ref\":\"psp_1\"}\n{\"kind\":7}\n",
        )
        .unwrap();
        let appender = OutcomesAppender::new(&path);
        assert!(!appender
            .append(1, "northstar", "paid", "psp_1")
            .expect("seen"));
        assert_eq!(
            appender.append(1, "northstar", "exploded", "x"),
            Err(OutcomesError::UnknownKind("exploded".into()))
        );
        assert_eq!(
            appender.append(1, "  ", "paid", "x"),
            Err(OutcomesError::Missing("publisher"))
        );
        assert_eq!(
            appender.append(1, "northstar", "paid", "   "),
            Err(OutcomesError::Missing("ref"))
        );
    }

    #[test]
    fn from_env_reads_the_documented_default() {
        assert_eq!(
            OutcomesAppender::from_env(|_| None).path(),
            Path::new(DEFAULT_OUTCOMES_PATH)
        );
        let custom = OutcomesAppender::from_env(|name| {
            (name == OUTCOMES_FILE_ENV).then(|| "/tmp/outcomes.jsonl".to_string())
        });
        assert_eq!(custom.path(), Path::new("/tmp/outcomes.jsonl"));
    }
}
