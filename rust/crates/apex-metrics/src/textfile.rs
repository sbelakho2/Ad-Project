//! Atomic textfile export for the node_exporter textfile collector.
//!
//! The runner writes its metrics to `APEX_METRICS_TEXTFILE` after every job
//! (this module's [`export_from_env`]). The file is written to a sibling
//! temporary path and then renamed over the destination, so a scrape never
//! observes a partially written file: `rename(2)` is atomic within a
//! filesystem.
//!
//! Deployment note: point node_exporter's
//! `--collector.textfile.directory` at the directory containing the file.
//! The `.tmp` suffix of the intermediate file is ignored by the textfile
//! collector (it only reads files matching `*.prom` by default), and the
//! rename removes it immediately in any case.

use std::path::{Path, PathBuf};

use crate::registry::Registry;

/// The environment variable naming the export path.
pub const TEXTFILE_ENV: &str = "APEX_METRICS_TEXTFILE";

/// Why an export failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextfileError {
    /// The directory could not be created.
    Directory {
        /// The directory involved.
        path: String,
        /// The underlying reason.
        reason: String,
    },
    /// The temporary file could not be written.
    Write {
        /// The file involved.
        path: String,
        /// The underlying reason.
        reason: String,
    },
    /// The rename over the destination failed.
    Rename {
        /// The destination involved.
        path: String,
        /// The underlying reason.
        reason: String,
    },
}

impl std::fmt::Display for TextfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextfileError::Directory { path, reason } => {
                write!(f, "metrics directory {path}: {reason}")
            }
            TextfileError::Write { path, reason } => {
                write!(f, "metrics temp file {path}: {reason}")
            }
            TextfileError::Rename { path, reason } => {
                write!(f, "metrics rename to {path}: {reason}")
            }
        }
    }
}

impl std::error::Error for TextfileError {}

/// Write `contents` to `path` atomically (temp file plus rename).
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), TextfileError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| TextfileError::Directory {
                path: parent.display().to_string(),
                reason: error.to_string(),
            })?;
        }
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, contents).map_err(|error| TextfileError::Write {
        path: tmp.display().to_string(),
        reason: error.to_string(),
    })?;
    std::fs::rename(&tmp, path).map_err(|error| TextfileError::Rename {
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

/// Export `registry` to `path` (atomic).
pub fn write_registry(path: &Path, registry: &Registry) -> Result<(), TextfileError> {
    write_atomic(path, &registry.render_prometheus())
}

/// Export to the path named by `APEX_METRICS_TEXTFILE`, when it is set and
/// non-blank. Returns the path written, or `None` when the variable is
/// unset.
pub fn export_from_env(
    lookup: impl Fn(&str) -> Option<String>,
    registry: &Registry,
) -> Result<Option<PathBuf>, TextfileError> {
    let Some(raw) = lookup(TEXTFILE_ENV) else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(raw);
    write_registry(&path, registry)?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::catalog;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("apex-metrics-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn write_atomic_replaces_the_destination_and_leaves_no_temp() {
        let dir = temp_dir("atomic");
        let path = dir.join("apex.prom");
        write_atomic(&path, "first\n").expect("first write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
        write_atomic(&path, "second\n").expect("second write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second\n");
        assert!(
            !dir.join("apex.prom.tmp").exists(),
            "the temp file is renamed away"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_failure_is_a_typed_error() {
        let dir = temp_dir("failure");
        std::fs::create_dir_all(&dir).unwrap();
        // A destination whose parent is a file cannot be a directory.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let path = blocker.join("nested").join("apex.prom");
        let error = write_atomic(&path, "x").expect_err("directory is a file");
        assert!(
            matches!(error, TextfileError::Directory { .. }),
            "{error:?}"
        );
        assert!(error.to_string().starts_with("metrics directory"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_from_env_writes_only_when_configured() {
        let registry = Registry::new();
        registry
            .increment(
                &catalog::JOB_RUNS_TOTAL,
                &[
                    ("job_type", "recheck"),
                    ("platform", "applovin_max"),
                    ("outcome", "completed"),
                ],
            )
            .unwrap();

        let none = export_from_env(|_| None, &registry).expect("unset is fine");
        assert!(none.is_none());
        let blank = export_from_env(|_| Some("   ".to_string()), &registry).expect("blank is fine");
        assert!(blank.is_none());

        let dir = temp_dir("export");
        let path = dir.join("apex-runner.prom");
        let path_text = path.display().to_string();
        let written = export_from_env(
            |name| (name == TEXTFILE_ENV).then(|| path_text.clone()),
            &registry,
        )
        .expect("export");
        assert_eq!(written.as_deref(), Some(path.as_path()));
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("apex_job_runs_total"));
        assert!(contents.contains("job_type=\"recheck\""));
        // Re-exporting is idempotent and never leaves the temp behind.
        write_registry(&path, &registry).expect("re-export");
        assert!(!dir.join("apex-runner.prom.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
