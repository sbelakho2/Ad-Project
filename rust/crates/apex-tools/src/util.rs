//! Small shared helpers: regex compilation without panics and file walking.

use std::path::{Path, PathBuf};

use regex::Regex;

/// Compile a pattern, mapping failure to a message. Patterns are static
/// tables, so a failure is a programming error surfaced as a gate error.
pub fn re(pattern: &str) -> Result<Regex, String> {
    Regex::new(pattern).map_err(|e| format!("regex '{pattern}': {e}"))
}

/// Compile a case-insensitive pattern.
pub fn rei(pattern: &str) -> Result<Regex, String> {
    re(&format!("(?i){pattern}"))
}

/// Recursively collect regular files under a directory. Skipped directory
/// names are compared per path segment. Results are sorted by path bytes.
pub fn walk_files(root: &Path, skip_dirs: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_into(root, skip_dirs, &mut out);
    out.sort();
    out
}

fn walk_into(dir: &Path, skip_dirs: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if skip_dirs.contains(&name.as_str()) {
                continue;
            }
            walk_into(&entry.path(), skip_dirs, out);
        } else if ft.is_file() {
            out.push(entry.path());
        }
    }
}

/// Slash-separated relative path for messages, independent of the platform.
pub fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
