//! Locale parity contract, ported from i18n-parity.mjs: every locale file in
//! an i18n tree must carry the same key set exactly, values must be non-empty
//! strings, and arrays or non-string leaves fail closed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::repo::Repo;
use crate::util::walk_files;

pub const EMPTY_MARKER: &str = "\u{0}EMPTY";
pub const ARRAY_MARKER: &str = "\u{0}ARRAY";
pub const TYPE_MARKER: &str = "\u{0}TYPE:";

/// Flatten nested JSON to dot-path keys; arrays and non-string leaves are
/// reported with marker suffixes instead of keys.
pub fn flatten_keys(value: &serde_json::Value, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(obj) = value.as_object() else {
        return out;
    };
    for (key, child) in obj {
        let full = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match child {
            serde_json::Value::Array(_) => out.push(format!("{full}{ARRAY_MARKER}")),
            serde_json::Value::Object(_) => out.extend(flatten_keys(child, &full)),
            serde_json::Value::String(s) => {
                if s.trim().is_empty() {
                    out.push(format!("{full}{EMPTY_MARKER}"));
                } else {
                    out.push(full);
                }
            }
            serde_json::Value::Number(_) => out.push(format!("{full}{TYPE_MARKER}number")),
            serde_json::Value::Bool(_) => out.push(format!("{full}{TYPE_MARKER}boolean")),
            serde_json::Value::Null => out.push(format!("{full}{TYPE_MARKER}object")),
        }
    }
    out
}

/// Compare locale key sets against the reference locale.
pub fn parity_violations(
    reference_name: &str,
    locales: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut violations = Vec::new();
    let reference: BTreeSet<String> = locales
        .get(reference_name)
        .map(|v| v.iter().cloned().collect())
        .unwrap_or_default();
    for (name, list) in locales {
        let set: BTreeSet<String> = list.iter().cloned().collect();
        let missing: Vec<&String> = reference.iter().filter(|k| !set.contains(*k)).collect();
        let extra: Vec<&String> = set.iter().filter(|k| !reference.contains(*k)).collect();
        if !missing.is_empty() {
            violations.push(format!(
                "{name}: {} missing key(s) vs {reference_name}: {}",
                missing.len(),
                missing
                    .iter()
                    .take(8)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !extra.is_empty() {
            violations.push(format!(
                "{name}: {} extra key(s) vs {reference_name}: {}",
                extra.len(),
                extra
                    .iter()
                    .take(8)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for k in list {
            if let Some(base) = k.strip_suffix(EMPTY_MARKER) {
                violations.push(format!("{name}: empty value: {base}"));
            }
            if let Some(base) = k.strip_suffix(ARRAY_MARKER) {
                violations.push(format!("{name}: array value not allowed: {base}"));
            }
            if k.contains(TYPE_MARKER) {
                violations.push(format!("{name}: non-string value: {k}"));
            }
        }
    }
    violations
}

/// Collect json files under one root; a `Some(filter)` requires the relative
/// path to contain the `i18n` path segment.
fn collect_json(root: &Path, require_i18n_segment: bool) -> Vec<PathBuf> {
    walk_files(root, &[])
        .into_iter()
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .filter(|p| {
            if !require_i18n_segment {
                return true;
            }
            p.strip_prefix(root)
                .map(|rel| rel.components().any(|c| c.as_os_str() == "i18n"))
                .unwrap_or(false)
        })
        .collect()
}

pub fn i18n_parity(repo: &Repo, extra_roots: &[String]) -> Result<(), String> {
    let root_specs: Vec<(PathBuf, bool)> = if extra_roots.is_empty() {
        ["console/src", "website/src", "site"]
            .iter()
            .map(|r| (repo.path(r), true))
            .collect()
    } else {
        extra_roots.iter().map(|r| (repo.path(r), false)).collect()
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for (root, require_i18n) in &root_specs {
        if !root.is_dir() {
            continue;
        }
        files.extend(collect_json(root, *require_i18n));
    }
    if files.is_empty() {
        println!("i18n-parity: no i18n trees found; nothing to compare");
        return Ok(());
    }
    files.sort();
    let mut by_dir: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for file in files {
        let dir = file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        by_dir
            .entry(dir.to_string_lossy().into_owned())
            .or_default()
            .push(file);
    }
    let mut failures = Vec::new();
    let mut compared = 0usize;
    for (dir, group) in &by_dir {
        let rel_dir = Path::new(dir)
            .strip_prefix(&repo.root)
            .map(crate::util::slash)
            .unwrap_or_else(|_| (*dir).clone());
        let mut locales: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for file in group {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let rel = file
                .strip_prefix(&repo.root)
                .map(crate::util::slash)
                .unwrap_or_else(|_| crate::util::slash(file));
            match std::fs::read_to_string(file)
                .map_err(|e| e.to_string())
                .and_then(|t| {
                    serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string())
                }) {
                Ok(v) => {
                    locales.insert(name, flatten_keys(&v, ""));
                }
                Err(error) => failures.push(format!("{rel}: invalid JSON: {error}")),
            }
        }
        let names: Vec<String> = locales.keys().cloned().collect();
        if names.is_empty() {
            continue;
        }
        let reference = if locales.contains_key("en.json") {
            "en.json".to_string()
        } else {
            names[0].clone()
        };
        for v in parity_violations(&reference, &locales) {
            failures.push(format!("{rel_dir}/{v}"));
        }
        compared += names.len();
    }
    if !failures.is_empty() {
        return Err(format!(
            "I18N PARITY FAILED ({}):\n  - {}",
            failures.len(),
            failures.join("\n  - ")
        ));
    }
    println!(
        "i18n-parity PASS: {compared} locale files across {} trees share identical key sets with no empty or non-string values",
        by_dir.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattening_reports_keys_empties_arrays_and_non_strings() {
        let keys = flatten_keys(
            &json!({"a": "x", "b": {"c": "y", "d": ""}, "e": ["z"], "f": 3}),
            "",
        );
        assert!(keys.contains(&"a".to_string()));
        assert!(keys.contains(&"b.c".to_string()));
        assert!(keys
            .iter()
            .any(|k| k.starts_with(&format!("b.d{EMPTY_MARKER}"))));
        assert!(keys
            .iter()
            .any(|k| k.starts_with(&format!("e{ARRAY_MARKER}"))));
        assert!(keys
            .iter()
            .any(|k| k.contains(&format!("{TYPE_MARKER}number"))));
    }

    #[test]
    fn missing_extra_empty_and_array_values_are_violations() {
        let mut locales = BTreeMap::new();
        locales.insert(
            "en.json".to_string(),
            vec!["a".to_string(), "b".to_string()],
        );
        locales.insert(
            "et.json".to_string(),
            vec!["a".to_string(), "c".to_string()],
        );
        let violations = parity_violations("en.json", &locales);
        assert!(violations.iter().any(|v| v.contains("1 missing key(s)")));
        assert!(violations.iter().any(|v| v.contains("1 extra key(s)")));
    }

    #[test]
    fn identical_locales_with_clean_values_pass() {
        let mut locales = BTreeMap::new();
        locales.insert(
            "en.json".to_string(),
            vec!["a.b".to_string(), "c".to_string()],
        );
        locales.insert(
            "et.json".to_string(),
            vec!["a.b".to_string(), "c".to_string()],
        );
        assert_eq!(parity_violations("en.json", &locales), Vec::<String>::new());
    }

    #[test]
    fn reference_locale_markers_are_still_reported() {
        let mut locales = BTreeMap::new();
        locales.insert(
            "en.json".to_string(),
            vec![format!("x{EMPTY_MARKER}"), format!("y{ARRAY_MARKER}")],
        );
        let violations = parity_violations("en.json", &locales);
        assert!(violations.iter().any(|v| v.contains("empty value: x")));
        assert!(violations
            .iter()
            .any(|v| v.contains("array value not allowed: y")));
    }
}
