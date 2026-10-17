//! Small shared helpers: regex compilation without panics and file walking.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use regex::Regex;

#[cfg(test)]
thread_local! {
    /// Regex compilations performed on this thread. A thread-local counter
    /// keeps the "compiled once" tests deterministic under the parallel test
    /// runner: only compiles caused by the tested code are counted.
    static REGEX_COMPILES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Note one regex compilation (test seam only).
#[cfg(test)]
pub(crate) fn note_regex_compile() {
    REGEX_COMPILES.with(|count| count.set(count.get() + 1));
}

/// Compile a pattern, mapping failure to a message. Patterns are static
/// tables, so a failure is a programming error surfaced as a gate error.
pub fn re(pattern: &str) -> Result<Regex, String> {
    #[cfg(test)]
    note_regex_compile();
    Regex::new(pattern).map_err(|e| format!("regex '{pattern}': {e}"))
}

/// Compile a pattern once and reuse it: gates that test one static pattern
/// against thousands of files (test-case detection) must not pay a regex
/// compilation per file. The cache is process-wide and keyed by pattern
/// text; a clone of a compiled `Regex` is cheap (one `Arc`).
pub fn cached_re(pattern: &'static str) -> Result<Regex, String> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(regex) = guard.get(pattern) {
        return Ok(regex.clone());
    }
    let regex = re(pattern)?;
    guard.insert(pattern, regex.clone());
    Ok(regex)
}

/// How many `Regex::new` calls this thread made so far (test seam proving
/// the caches are hit).
#[cfg(test)]
pub(crate) fn regex_compile_count() -> u64 {
    REGEX_COMPILES.with(std::cell::Cell::get)
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
