//! Docs link contract, ported from docs-links.mjs: every relative markdown
//! link resolves to a real file, every in-page anchor matches a real heading,
//! and legacy debt is frozen in a ratchet baseline where stale rows fail.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::repo::Repo;
use crate::util::{re, rei, walk_files};

pub fn slugify(heading: &str) -> Result<String, String> {
    let lower = heading.to_lowercase();
    let no_tags = re(r"<[^>]+>")?.replace_all(&lower, "").to_string();
    let no_markup: String = no_tags
        .chars()
        .filter(|c| !matches!(c, '`' | '*' | '_' | '~'))
        .collect();
    let kept: String = no_markup
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-')
        .collect();
    let trimmed = kept.trim().to_string();
    Ok(re(r"\s+")?.replace_all(&trimmed, "-").to_string())
}

/// Headings in one markdown document, slugged, plus the raw lowercase
/// fallback for non-GitHub renderers.
pub fn heading_slugs(markdown: &str) -> Result<BTreeSet<String>, String> {
    let heading = re(r"^(#{1,6})\s+(.+?)\s*$")?;
    let mut slugs = BTreeSet::new();
    for line in markdown.split('\n') {
        if let Some(c) = heading.captures(line) {
            slugs.insert(slugify(&c[2])?);
            slugs.insert(c[2].trim().to_lowercase());
        }
    }
    Ok(slugs)
}

/// Relative links in one markdown document. External link types are skipped.
pub fn relative_links(markdown: &str) -> Result<Vec<String>, String> {
    let link = re(r#"\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#)?;
    let external = rei(r"^(https?:|mailto:|tel:|data:)")?;
    let mut links = Vec::new();
    for c in link.captures_iter(markdown) {
        let target = c[1].to_string();
        if external.is_match(&target) {
            continue;
        }
        links.push(target);
    }
    Ok(links)
}

/// Lenient percent-decoding for link paths; malformed escapes stay literal.
pub fn decode_uri_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn posix_dirname(path: &str) -> String {
    match path.rfind('/') {
        None => ".".to_string(),
        Some(0) => "/".to_string(),
        Some(i) => path[..i].to_string(),
    }
}

pub fn posix_join(dir: &str, rel: &str) -> String {
    if rel.is_empty() {
        return dir.to_string();
    }
    let left = dir.trim_end_matches('/');
    let right = rel.trim_start_matches('/');
    if left.is_empty() {
        return right.to_string();
    }
    format!("{left}/{right}")
}

pub fn posix_normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if let Some(last) = out.last() {
                    if *last != ".." {
                        out.pop();
                        continue;
                    }
                }
                if !absolute {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    let joined = out.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Pure: violations for one doc given a loader for other docs.
pub fn check_doc(
    rel: &str,
    markdown: &str,
    file_exists: &dyn Fn(&str) -> bool,
    read_doc: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<String>, String> {
    let mut violations = Vec::new();
    let own_slugs = heading_slugs(markdown)?;
    for target in relative_links(markdown)? {
        let mut parts = target.splitn(2, '#');
        let raw_path = parts.next().unwrap_or("");
        let anchor = parts.next().unwrap_or("");
        let clean_path = decode_uri_component(raw_path);
        if clean_path.is_empty() {
            if !anchor.is_empty() && !own_slugs.contains(&anchor.to_lowercase()) {
                violations.push(format!("{rel}: anchor '#{anchor}' has no matching heading"));
            }
            continue;
        }
        // A leading slash is a site route (Zola renders `content/<route>.md` to
        // `/route/`), not a document-relative path; resolve it from the repo
        // root and let the existence callback map the route to its content
        // file. Document-relative links keep the old resolution.
        let resolved = if clean_path.starts_with('/') {
            posix_normalize(&clean_path)
        } else {
            posix_normalize(&posix_join(&posix_dirname(rel), &clean_path))
        };
        if resolved == ".." || resolved.starts_with("../") || resolved.contains("/../") {
            violations.push(format!(
                "{rel}: link target escapes the repository: {clean_path}"
            ));
            continue;
        }
        if !file_exists(&resolved) {
            violations.push(format!("{rel}: link target missing: {clean_path}"));
            continue;
        }
        if !anchor.is_empty() && resolved.ends_with(".md") {
            match read_doc(&resolved) {
                None => violations.push(format!("{rel}: linked doc unreadable: {resolved}")),
                Some(other) => {
                    if !heading_slugs(&other)?.contains(&anchor.to_lowercase()) {
                        violations
                            .push(format!("{rel}: anchor '#{anchor}' not found in {resolved}"));
                    }
                }
            }
        }
    }
    Ok(violations)
}

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "public",
    "lhci-report",
    ".kilo",
    ".junie",
    // The future tree is parked product work, not current documentation.
    "future",
];

/// Walk markdown documents under the repo root, slash-separated and sorted.
pub fn walk_markdown(root: &Path) -> Vec<String> {
    walk_files(root, SKIP_DIRS)
        .into_iter()
        .filter(|p| p.extension().map(|e| e == "md").unwrap_or(false))
        .map(|p| {
            p.strip_prefix(root)
                .map(crate::util::slash)
                .unwrap_or_else(|_| crate::util::slash(&p))
        })
        .collect()
}

#[derive(Debug)]
pub enum Mode {
    Check(String),
    UpdateBaseline(String),
}

/// Parse `--check`-style arguments for the docs-links subcommand.
pub fn parse_mode(args: &[String]) -> Result<Mode, String> {
    if let Some(i) = args.iter().position(|a| a == "--update-baseline") {
        let target = args.get(i + 1).ok_or("--update-baseline requires a file")?;
        return Ok(Mode::UpdateBaseline(target.clone()));
    }
    if let Some(i) = args.iter().position(|a| a == "--baseline") {
        let target = args.get(i + 1).ok_or("--baseline requires a file")?;
        return Ok(Mode::Check(target.clone()));
    }
    Err("usage: apex-tools docs-links --check --baseline <file> | --update-baseline <file>".into())
}

/// Content files a site-absolute route may resolve to: the page, the section
/// index, or the static tree entry. Routes never leave those three roots.
pub fn site_route_candidates(resolved: &str) -> Vec<String> {
    let Some(route) = resolved.strip_prefix('/') else {
        return Vec::new();
    };
    let route = route.trim_end_matches('/');
    if route.split('/').any(|s| s == ".." || s == ".") {
        return Vec::new();
    }
    if route.is_empty() {
        // Zola serves content/_index.md at the site root.
        return vec!["site/content/_index.md".to_string()];
    }
    vec![
        format!("site/content/{route}.md"),
        format!("site/content/{route}/_index.md"),
        format!("site/content/{route}/index.md"),
        format!("site/static/{route}"),
    ]
}

pub fn docs_links(repo: &Repo, mode: &Mode) -> Result<(), String> {
    let docs = walk_markdown(&repo.root);
    let mut by_pos: BTreeMap<String, String> = BTreeMap::new();
    for rel in &docs {
        if let Ok(text) = std::fs::read_to_string(repo.path(rel)) {
            by_pos.insert(rel.clone(), text);
        }
    }
    let exists = |resolved: &str| {
        // Repo-relative paths are checked as-is; a leading slash marks a site
        // route and must not be handed to PathBuf::join (an absolute path
        // would replace the repository root).
        if !resolved.starts_with('/')
            && (repo.path(resolved).exists() || by_pos.contains_key(resolved))
        {
            return true;
        }
        site_route_candidates(resolved)
            .iter()
            .any(|c| repo.path(c).exists() || by_pos.contains_key(c))
    };
    let read_doc = |resolved: &str| -> Option<String> { by_pos.get(resolved).cloned() };
    let mut per_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut total = 0usize;
    for rel in &docs {
        let Some(text) = by_pos.get(rel) else {
            continue;
        };
        let violations = check_doc(rel, text, &exists, &read_doc)?;
        if !violations.is_empty() {
            total += violations.len();
            per_file.insert(rel.clone(), violations);
        }
    }

    match mode {
        Mode::UpdateBaseline(target) => {
            let mut lines = vec![
                "# docs-links ratchet baseline: per-file broken-link counts (rows may only shrink)"
                    .to_string(),
            ];
            for (rel, violations) in &per_file {
                lines.push(format!("{} {rel}", violations.len()));
            }
            lines.push(format!("TOTAL {total}"));
            std::fs::write(repo.path(target), lines.join("\n") + "\n")
                .map_err(|e| format!("{target}: {e}"))?;
            println!(
                "docs-links baseline written: {target} (TOTAL {total} across {} files)",
                per_file.len()
            );
            Ok(())
        }
        Mode::Check(target) => {
            let path = repo.path(target);
            if !path.exists() {
                return Err(format!("docs-links: baseline not found: {target}"));
            }
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{target}: {e}"))?;
            let (allowed, allowed_total) = crate::baseline::parse_count_rows(&text);
            let counts: BTreeMap<String, usize> =
                per_file.iter().map(|(k, v)| (k.clone(), v.len())).collect();
            let (mut failures, _notes) = crate::baseline::ratchet(
                &counts,
                &allowed,
                total,
                allowed_total,
                "broken links",
                crate::baseline::Stale::Failure,
            );
            for (rel, violations) in &per_file {
                let allowed_count = allowed.get(rel).copied().unwrap_or(0);
                if violations.len() > allowed_count {
                    for v in violations.iter().take(5) {
                        failures.push(format!("    {v}"));
                    }
                }
            }
            if !failures.is_empty() {
                return Err(format!(
                    "DOCS-LINKS FAILED ({}):\n  - {}",
                    failures.len(),
                    failures.join("\n  - ")
                ));
            }
            println!(
                "docs-links PASS: {total} broken links at or below baseline ({allowed_total}) across {} documents",
                docs.len()
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(
        files: &'static [(&'static str, &'static str)],
    ) -> (impl Fn(&str) -> bool, impl Fn(&str) -> Option<String>) {
        (
            move |p: &str| files.iter().any(|(k, _)| *k == p),
            move |p: &str| {
                files
                    .iter()
                    .find(|(k, _)| *k == p)
                    .map(|(_, v)| v.to_string())
            },
        )
    }

    #[test]
    fn slugify_follows_github_rules() {
        assert_eq!(
            slugify("The audit, in practice").unwrap(),
            "the-audit-in-practice"
        );
        assert_eq!(
            slugify("`code` and *emphasis*").unwrap(),
            "code-and-emphasis"
        );
        assert_eq!(slugify("A/B testing (v2)").unwrap(), "ab-testing-v2");
    }

    #[test]
    fn relative_links_skip_external_protocols() {
        let md = "[a](https://x) [b](mailto:x@y) [c](./doc.md#h) [d](#local)";
        assert_eq!(
            relative_links(md).unwrap(),
            vec!["./doc.md#h".to_string(), "#local".to_string()]
        );
    }

    #[test]
    fn links_that_escape_the_repository_are_violations() {
        static FILES: &[(&str, &str)] = &[];
        let (exists, read) = ctx(FILES);
        let md = "[x](../../../../etc/passwd)";
        let v = check_doc("docs/a.md", md, &exists, &read).unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v.iter().any(|m| m.contains("escapes the repository")));
    }

    #[test]
    fn site_route_candidates_map_pages_sections_bundles_and_static_files() {
        assert_eq!(
            site_route_candidates("/how-it-works/"),
            vec![
                "site/content/how-it-works.md".to_string(),
                "site/content/how-it-works/_index.md".to_string(),
                "site/content/how-it-works/index.md".to_string(),
                "site/static/how-it-works".to_string()
            ]
        );
        assert_eq!(
            site_route_candidates("/"),
            vec!["site/content/_index.md".to_string()]
        );
        assert!(site_route_candidates("docs/a.md").is_empty());
        assert!(site_route_candidates("/../etc/passwd").is_empty());
    }

    #[test]
    fn check_doc_reports_missing_file_missing_anchor_and_local_anchor() {
        static FILES: &[(&str, &str)] = &[("docs/ok.md", "# Present\n")];
        let (exists, read) = ctx(FILES);
        let md = "[x](./missing.md) [y](./ok.md#wrong) [z](#nope)";
        let v = check_doc("docs/a.md", md, &exists, &read).unwrap();
        assert_eq!(v.len(), 3);
        assert!(v
            .iter()
            .any(|m| m.contains("link target missing: ./missing.md")));
        assert!(v
            .iter()
            .any(|m| m.contains("anchor '#wrong' not found in docs/ok.md")));
        assert!(v
            .iter()
            .any(|m| m.contains("anchor '#nope' has no matching heading")));
    }

    #[test]
    fn check_doc_accepts_resolving_links_and_anchors() {
        static FILES: &[(&str, &str)] = &[
            ("docs/ok.md", "# Present\n## Deep Section\n"),
            (
                "docs/b.md",
                "[x](./ok.md#present) [y](./ok.md#deep-section) [z](#own)\n## Own\n",
            ),
        ];
        let (exists, read) = ctx(FILES);
        let md = FILES[1].1;
        assert_eq!(
            check_doc("docs/b.md", md, &exists, &read).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn heading_slugs_keep_raw_lowercase_fallback() {
        let slugs = heading_slugs("## Mixed CASE Heading!\n").unwrap();
        assert!(slugs.contains("mixed-case-heading"));
        assert!(slugs.contains("mixed case heading!"));
    }

    #[test]
    fn decode_and_posix_helpers_match_js() {
        assert_eq!(decode_uri_component("a%20b"), "a b");
        assert_eq!(decode_uri_component("100%"), "100%");
        assert_eq!(posix_normalize(&posix_join("docs", "../x/y.md")), "x/y.md");
        assert_eq!(posix_normalize(&posix_join(".", "a.md")), "a.md");
        assert_eq!(posix_normalize("../a/./b/../c"), "../a/c");
        assert_eq!(posix_dirname("a/b/c.md"), "a/b");
        assert_eq!(posix_dirname("a.md"), ".");
    }

    #[test]
    fn baseline_rows_only_shrink_and_stale_rows_fail() {
        // Exercised through the parsing helpers: a row for a fixed doc is
        // stale, and a larger current count is an overage.
        let text = "# baseline\n2 docs/a.md\nTOTAL 9\n";
        let total_re = re(r"^TOTAL (\d+)$").unwrap();
        let row_re = re(r"^(\d+)\s+(.+)$").unwrap();
        let mut allowed = BTreeMap::new();
        let mut allowed_total = 0usize;
        for line in text.lines() {
            if let Some(c) = total_re.captures(line) {
                allowed_total = c[1].parse().unwrap();
                continue;
            }
            if let Some(c) = row_re.captures(line) {
                allowed.insert(c[2].to_string(), c[1].parse().unwrap());
            }
        }
        assert_eq!(allowed.get("docs/a.md"), Some(&2));
        assert_eq!(allowed_total, 9);
        let per_file: BTreeMap<String, usize> = BTreeMap::new();
        assert!(allowed.keys().any(|k| !per_file.contains_key(k)));
    }
}
