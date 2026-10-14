//! CSS contract suite, ported from css-lint.mjs: balanced braces and quotes,
//! no unjustified `!important`, lowercase hex colors, no empty rules and a
//! per-file size budget. The comment mask is length preserving so the pragma
//! comment above a rule stays addressable by offset.

use crate::repo::Repo;
use crate::util::{re, rei, walk_files};

pub const CSS_FILE_BUDGET_BYTES: usize = 120_000;

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

pub fn css_violations(rel: &str, text: &str) -> Result<Vec<String>, String> {
    let mut violations = Vec::new();
    let comment = re(r"(?s)/\*.*?\*/")?;
    let stripped = comment
        .replace_all(text, |caps: &regex::Captures| {
            " ".repeat(utf16_len(&caps[0]))
        })
        .to_string();

    let opens = stripped.matches('{').count();
    let closes = stripped.matches('}').count();
    if opens != closes {
        violations.push(format!(
            "{rel}: unbalanced braces ({{{opens}}} vs }}{closes}}})"
        ));
    }
    if stripped.matches('"').count() % 2 != 0 || stripped.matches('\'').count() % 2 != 0 {
        violations.push(format!("{rel}: unbalanced quotes"));
    }

    let motion_media = rei(r"@media[^{]*prefers-reduced-motion[^{]*\{.*?\n\}")?;
    let motion_ranges: Vec<(usize, usize)> = motion_media
        .find_iter(&stripped)
        .map(|m| (m.start(), m.end()))
        .collect();
    let in_motion = |idx: usize| motion_ranges.iter().any(|(a, b)| idx >= *a && idx <= *b);
    let motion = rei(
        r"(?:animation|transition)(?:-[a-z-]+)?\s*:\s*(none|auto|0\.001ms|\b1\b)|(?:scroll-behavior|transform)\s*:\s*(?:none|auto)\b",
    )?;
    let important = rei(r"!\s*important")?;
    let pragma = re(r"css-contract:allow-important[^*]*\*/\s*$")?;
    let mut important_positions = Vec::new();
    for m in important.find_iter(&stripped) {
        let idx = m.start();
        if in_motion(idx) {
            continue;
        }
        let line_start = stripped[..idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = &stripped[line_start..m.end()];
        if motion.is_match(line) {
            continue;
        }
        let floor = char_floor(text, line_start.saturating_sub(260));
        let context = &text[floor..line_start];
        if pragma.is_match(context) {
            continue;
        }
        important_positions.push(idx);
    }
    for idx in important_positions {
        violations.push(format!(
            "{rel}: !important without justification at offset {idx}"
        ));
    }

    if re(r"\{\s*\}")?.is_match(&stripped) {
        violations.push(format!("{rel}: empty rule"));
    }
    let hex = re(r"#[0-9A-F]{3,8}\b")?;
    for m in hex.find_iter(&stripped) {
        if m.as_str() != m.as_str().to_lowercase() {
            violations.push(format!(
                "{rel}: uppercase hex color {} (normalize to lowercase)",
                m.as_str()
            ));
        }
    }
    if text.len() > CSS_FILE_BUDGET_BYTES {
        violations.push(format!(
            "{rel}: {} bytes exceeds {CSS_FILE_BUDGET_BYTES}",
            text.len()
        ));
    }
    Ok(violations)
}

fn char_floor(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "public",
    ".next",
    "lhci-report",
    ".kilo",
];

pub fn css_lint(repo: &Repo) -> Result<(), String> {
    let roots = ["console/src", "website/src", "site/static"];
    let mut files = Vec::new();
    for root in roots {
        let path = repo.path(root);
        if !path.is_dir() {
            continue;
        }
        files.extend(walk_files(&path, SKIP_DIRS).into_iter().filter(|p| {
            p.extension()
                .map(|e| e == "css" || e == "scss")
                .unwrap_or(false)
        }));
    }
    files.sort();
    let mut failures = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&repo.root)
            .map(crate::util::slash)
            .unwrap_or_else(|_| crate::util::slash(file));
        let text = std::fs::read_to_string(file).map_err(|e| format!("{rel}: {e}"))?;
        failures.extend(css_violations(&rel, &text)?);
    }
    if !failures.is_empty() {
        return Err(format!(
            "CSS CONTRACT FAILED ({}):\n  - {}",
            failures.len(),
            failures.join("\n  - ")
        ));
    }
    println!(
        "css contract PASS: {} stylesheets clean (braces, quotes, no !important, lowercase hex, size budget)",
        files.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_clean_stylesheet_passes_and_each_banned_pattern_trips() {
        assert!(css_violations("x.css", "a{color:#1f4e79}\nb{margin:0}")
            .unwrap()
            .is_empty());
        assert!(css_violations("x.css", "a{color:red;")
            .unwrap()
            .iter()
            .any(|v| v.contains("unbalanced braces")));
        assert!(css_violations("x.css", "a{}")
            .unwrap()
            .iter()
            .any(|v| v.contains("empty rule")));
        assert!(css_violations("x.css", "a{color:#FFFFFF}")
            .unwrap()
            .iter()
            .any(|v| v.contains("uppercase hex")));
        assert!(css_violations("x.css", "a{color:red !important}")
            .unwrap()
            .iter()
            .any(|v| v.contains("without justification")));
    }

    #[test]
    fn justified_important_passes_media_motion_and_pragma() {
        let media = "@media (prefers-reduced-motion: reduce) { * { animation-duration: 0.001ms !important; } }";
        assert!(css_violations("x.css", media).unwrap().is_empty());
        assert!(
            css_violations("x.css", ".no-transform * { transform: none !important; }")
                .unwrap()
                .is_empty()
        );
        let pragma =
            "/* css-contract:allow-important reason here */\n.weird { color: red !important; }";
        assert!(css_violations("x.css", pragma).unwrap().is_empty());
    }

    #[test]
    fn comments_are_masked_before_hygiene_checks() {
        assert!(
            css_violations("x.css", "/* !important in a comment { */\na{color:#abc}")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn quotes_are_checked_on_masked_text() {
        assert!(css_violations("x.css", "a{content:'x'}\nb{content:\"y\"}")
            .unwrap()
            .is_empty());
        assert!(css_violations("x.css", "a{content:'x}")
            .unwrap()
            .iter()
            .any(|v| v.contains("unbalanced quotes")));
    }
}
