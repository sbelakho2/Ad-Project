//! Repository root discovery and file walking, mirroring the JS walker.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Directory names never walked: build outputs and tool caches.
pub const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "dist",
    ".next",
    "build",
    ".gradle",
    ".dart_tool",
    ".build",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".nyc_output",
    "Pods",
    "DerivedData",
    "test-results",
    "playwright-report",
    "artifacts",
    ".idea",
    ".kilo",
    ".gradle-dist",
    ".gradle-home",
    "shell-cov",
    "lhci-report",
    ".lighthouseci",
    "chrome",
    "chromium",
    "target",
    "vendor",
];

#[derive(Debug, Clone)]
pub struct Repo {
    pub root: PathBuf,
}

impl Repo {
    /// Discover the repository root: the nearest ancestor containing `Cargo.toml`
    /// and `tools/coverage`.
    pub fn discover() -> Result<Self, String> {
        let mut dir = std::env::current_dir().map_err(|e| e.to_string())?;
        loop {
            if dir.join("tools/coverage/policies.json").exists()
                && dir.join("rust/Cargo.toml").exists()
            {
                return Ok(Self { root: dir });
            }
            if !dir.pop() {
                return Err(
                    "repository root not found (no tools/coverage/policies.json above cwd)".into(),
                );
            }
        }
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn read(&self, rel: &str) -> Result<String, String> {
        std::fs::read_to_string(self.path(rel)).map_err(|e| format!("{rel}: {e}"))
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    /// Walk every file, returning repo-relative paths sorted, mirroring the JS
    /// walker: skipped dirs by name, symlinks included as entries.
    pub fn walk(&self) -> Vec<String> {
        let mut out = Vec::new();
        rec_walk(&self.root, "", &mut out);
        out.sort();
        out
    }

    /// Ask git for the tracked-file list with modes (`git ls-files -s`).
    pub fn tracked_files_with_modes(&self) -> Result<Vec<(String, String)>, String> {
        let output = std::process::Command::new("git")
            .args(["ls-files", "-s"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| format!("git ls-files failed: {e}"))?;
        if !output.status.success() {
            return Err("git ls-files exited non-zero".into());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut files = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Some((meta, path)) = line.split_once('\t') {
                let mode = meta.split_whitespace().next().unwrap_or("").to_string();
                files.push((mode, path.to_string()));
            }
        }
        Ok(files)
    }

    /// Names of all markdown documents, honoring the same skip set plus tool dirs.
    pub fn markdown_documents(&self) -> Vec<String> {
        let skip_extra = BTreeSet::from([".junie"]);
        let mut out = Vec::new();
        rec_markdown(&self.root, "", &skip_extra, &mut out);
        out.sort();
        out
    }
}

fn rec_walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            rec_walk(&entry.path(), &child_rel, out);
        } else {
            // Regular files and symlinks (symlink targets are not followed
            // here; the entry itself is part of the repo).
            out.push(child_rel);
        }
    }
}

fn rec_markdown(dir: &Path, rel: &str, extra: &BTreeSet<&str>, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) || extra.contains(name.as_str()) {
                continue;
            }
            rec_markdown(&entry.path(), &child_rel, extra, out);
        } else if name.ends_with(".md") {
            out.push(child_rel);
        }
    }
}
