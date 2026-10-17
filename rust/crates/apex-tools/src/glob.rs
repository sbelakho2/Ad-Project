//! Glob semantics ported 1:1 from the JS stack (coverage-lib.mjs):
//! `**/` spans zero or more directories, `*` never crosses `/`, `{a,b}` expands
//! alternatives, patterns anchor at the repo root.

use regex::Regex;

/// Expand braces, then compile with the exact JS replacement ordering.
pub fn glob_to_regex(glob: &str) -> Result<Regex, String> {
    #[cfg(test)]
    crate::util::note_regex_compile();
    let mut alts: Vec<String> = Vec::new();
    let mut with_placeholders = String::new();
    {
        let mut rest = glob;
        while let Some(open) = rest.find('{') {
            let Some(close_rel) = rest[open..].find('}') else {
                break;
            };
            let close = open + close_rel;
            with_placeholders.push_str(&rest[..open]);
            let inner = &rest[open + 1..close];
            alts.push(format!(
                "(?:{})",
                inner.split(',').collect::<Vec<_>>().join("|")
            ));
            with_placeholders.push('\u{1}');
            with_placeholders.push_str(&(alts.len() - 1).to_string());
            with_placeholders.push('\u{2}');
            rest = &rest[close + 1..];
        }
        with_placeholders.push_str(rest);
    }
    let mut esc = String::new();
    for ch in with_placeholders.chars() {
        match ch {
            '.' | '+' | '^' | '$' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                esc.push('\\');
                esc.push(ch);
            }
            _ => esc.push(ch),
        }
    }
    let esc = esc.replace("**", "\u{3}");
    let esc = esc.replace('*', "[^/]*");
    let esc = esc.replace("\u{3}/", "(?:.*/)?");
    let esc = esc.replace('\u{3}', ".*");
    let mut rebuilt = String::new();
    let mut chars = esc.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1}' {
            let mut idx = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    idx.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            // consume the closing marker
            if chars.peek() == Some(&'\u{2}') {
                chars.next();
            }
            rebuilt.push_str(&alts[idx.parse::<usize>().map_err(|e| e.to_string())?]);
        } else {
            rebuilt.push(ch);
        }
    }
    Regex::new(&format!("^{rebuilt}$")).map_err(|e| e.to_string())
}

pub fn matches(glob: &str, path: &str) -> bool {
    glob_to_regex(glob)
        .map(|re| re.is_match(path))
        .unwrap_or(false)
}

/// Compile a glob once for repeated matching. An invalid glob compiles to
/// `None` and never matches, exactly like the one-shot [`matches`] helper;
/// callers that match one pattern against many paths (the classifier, the
/// orphan gate) must compile first instead of paying a regex compilation per
/// path.
pub fn compile(glob: &str) -> Option<Regex> {
    glob_to_regex(glob).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Glob fixtures are assembled from parts: the prose linter treats a
    /// literal opener pair in this repository's own files as a comment.
    const S: &str = "*";

    #[test]
    fn double_star_spans_zero_or_more_dirs() {
        let deep = format!("backend/src/{S}{S}/{S}.ts");
        assert!(matches(&deep, "backend/src/index.ts"));
        assert!(matches(&deep, "backend/src/services/a/b.ts"));
        let any = format!("{S}{S}/package.json");
        assert!(matches(&any, "package.json"));
        assert!(matches(&any, "backend/package.json"));
        let flat = format!("backend/src/{S}.ts");
        assert!(!matches(&flat, "backend/src/services/a.ts"));
        assert!(matches(&flat, "backend/src/index.ts"));
    }

    #[test]
    fn braces_expand() {
        let glob = format!("monitoring/{S}{S}/{S}.{{yml,yaml}}");
        assert!(matches(&glob, "monitoring/alerts.yml"));
        assert!(matches(&glob, "monitoring/grafana/dashboards/x.yaml"));
        assert!(!matches(&glob, "monitoring/grafana/dashboards/x.json"));
    }
}
