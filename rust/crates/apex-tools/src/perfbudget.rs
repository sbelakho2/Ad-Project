//! Hard performance budgets for the built Zola marketing site, ported from
//! perf-budget.mjs: zero JavaScript, per-page byte budgets, self-hosted
//! resources only, meta description plus canonical, robots and sitemap.

use std::collections::BTreeMap;

use crate::repo::Repo;
use crate::util::{re, rei, walk_files};

pub const HTML_BUDGET_BYTES: usize = 40_000;
pub const CSS_BUDGET_BYTES: usize = 12_000;

/// Parse attributes from a tag's inner text: quoted or unquoted, any order.
pub fn parse_attrs(tag_inner: &str) -> Result<BTreeMap<String, String>, String> {
    let attr = re(r#"([a-zA-Z_:][a-zA-Z0-9_:.-]*)(?:\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+)))?"#)?;
    let mut attrs = BTreeMap::new();
    for c in attr.captures_iter(tag_inner) {
        let value = c
            .get(3)
            .or_else(|| c.get(4))
            .or_else(|| c.get(5))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        attrs.insert(c[1].to_lowercase(), value);
    }
    Ok(attrs)
}

/// Pure: evaluate one built page; `origin` is the site's own origin, so
/// same-origin absolute resource URLs are self-hosted and fine.
pub fn check_page(
    rel: &str,
    html: &str,
    html_budget: usize,
    origin: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut violations = Vec::new();
    if html.len() > html_budget {
        violations.push(format!(
            "{rel}: {} bytes > html budget {html_budget}",
            html.len()
        ));
    }
    if rei(r"<script[\s>]")?.is_match(html) {
        violations.push(format!("{rel}: contains a <script> tag (zero-JS budget)"));
    }
    let meta_re = rei(r"<meta\b([^>]*)>")?;
    let metas: Vec<BTreeMap<String, String>> = meta_re
        .captures_iter(html)
        .map(|c| parse_attrs(&c[1]))
        .collect::<Result<_, _>>()?;
    let has_description = metas.iter().any(|attrs| {
        attrs.get("name").map(String::as_str) == Some("description")
            && attrs
                .get("content")
                .map(|c| !c.trim().is_empty())
                .unwrap_or(false)
    });
    if !has_description {
        violations.push(format!("{rel}: missing meta description"));
    }
    let link_re = rei(r"<link\b([^>]*)>")?;
    let links: Vec<BTreeMap<String, String>> = link_re
        .captures_iter(html)
        .map(|c| parse_attrs(&c[1]))
        .collect::<Result<_, _>>()?;
    if rel != "404.html" {
        let canonical = links.iter().any(|attrs| {
            attrs.get("rel").map(|r| r.to_lowercase()).as_deref() == Some("canonical")
                && attrs
                    .get("href")
                    .map(|h| !h.trim().is_empty())
                    .unwrap_or(false)
        });
        if !canonical {
            violations.push(format!("{rel}: missing canonical link"));
        }
    }
    let resource_kind = re(r"(stylesheet|preconnect|dns-prefetch|preload|prefetch|icon|manifest)")?;
    let absolute = rei(r"^(https?:)?//")?;
    for attrs in &links {
        let kind = attrs
            .get("rel")
            .map(|r| r.to_lowercase())
            .unwrap_or_default();
        let href = attrs.get("href").cloned().unwrap_or_default();
        if !resource_kind.is_match(&kind) {
            continue;
        }
        if !absolute.is_match(&href) {
            continue;
        }
        let same_origin = origin
            .map(|o| !o.is_empty() && href.starts_with(&format!("{o}/")))
            .unwrap_or(false);
        if !same_origin {
            violations.push(format!(
                "{rel}: external {kind} request (self-host only): {href}"
            ));
        }
    }
    Ok(violations)
}

/// Pure: evaluate the whole distribution.
pub fn check_dist(
    pages: &BTreeMap<String, String>,
    css_bytes: usize,
    html_budget: usize,
    css_budget: usize,
    min_pages: usize,
    origin: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut violations = Vec::new();
    for (rel, html) in pages {
        violations.extend(check_page(rel, html, html_budget, origin)?);
    }
    if pages.len() < min_pages {
        violations.push(format!(
            "built {} pages < required {min_pages}",
            pages.len()
        ));
    }
    if css_bytes > css_budget {
        violations.push(format!(
            "stylesheets total {css_bytes} bytes > budget {css_budget}"
        ));
    }
    Ok(violations)
}

fn site_origin(repo: &Repo) -> Option<String> {
    let config = repo.read("site/config.toml").ok()?;
    let base_url = re(r#"(?m)^base_url\s*=\s*"([^"]+)""#).ok()?;
    let caps = base_url.captures(&config)?;
    Some(caps[1].trim_end_matches('/').to_string())
}

pub fn perf_budget(repo: &Repo, dist: &str, min_pages: usize) -> Result<(), String> {
    let dist_path = repo.path(dist);
    if !dist_path.is_dir() {
        return Err(format!(
            "site perf budget: dist not found at {} — build the site first (zola build)",
            crate::util::slash(&dist_path)
        ));
    }
    let origin = site_origin(repo);
    let mut pages: BTreeMap<String, String> = BTreeMap::new();
    let mut css_bytes = 0usize;
    for file in walk_files(&dist_path, &[]) {
        let rel = file
            .strip_prefix(&dist_path)
            .map(crate::util::slash)
            .unwrap_or_else(|_| crate::util::slash(&file));
        match file.extension().and_then(|e| e.to_str()) {
            Some("html") => {
                let text = std::fs::read_to_string(&file).map_err(|e| format!("{rel}: {e}"))?;
                pages.insert(rel, text);
            }
            Some("css") => {
                if let Ok(meta) = std::fs::metadata(&file) {
                    css_bytes += meta.len() as usize;
                }
            }
            _ => {}
        }
    }
    let mut violations = check_dist(
        &pages,
        css_bytes,
        HTML_BUDGET_BYTES,
        CSS_BUDGET_BYTES,
        min_pages,
        origin.as_deref(),
    )?;
    if !dist_path.join("robots.txt").exists() {
        violations.push("dist missing robots.txt".to_string());
    }
    if !dist_path.join("sitemap.xml").exists() {
        violations.push("dist missing sitemap.xml".to_string());
    }
    if !violations.is_empty() {
        return Err(format!(
            "SITE PERF BUDGET FAILED ({}):\n  - {}",
            violations.len(),
            violations.join("\n  - ")
        ));
    }
    println!(
        "site perf budget PASS: {} pages (max html {HTML_BUDGET_BYTES}B), css {css_bytes}B (budget {CSS_BUDGET_BYTES}B), zero JS, robots+sitemap present",
        pages.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_PAGE: &str = r#"<!doctype html><html lang="en"><head><title>x</title>
<meta name="description" content="d"><link rel="canonical" href="https://a/">
<link rel="stylesheet" href="/style.css"></head><body>ok</body></html>"#;

    #[test]
    fn a_clean_small_page_passes() {
        assert_eq!(
            check_page("index.html", GOOD_PAGE, HTML_BUDGET_BYTES, None).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn script_tag_is_a_violation_even_when_tiny() {
        let html = GOOD_PAGE.replace("</body>", "<script>1</script></body>");
        let v = check_page("x.html", &html, HTML_BUDGET_BYTES, None).unwrap();
        assert!(v.iter().any(|m| m.contains("<script>")));
    }

    #[test]
    fn oversized_page_violates_the_byte_budget_at_the_boundary_plus_one() {
        let big = GOOD_PAGE.replace("ok", &"o".repeat(HTML_BUDGET_BYTES));
        let v = check_page("big.html", &big, HTML_BUDGET_BYTES, None).unwrap();
        assert!(v.iter().any(|m| m.contains("bytes > html budget")));
    }

    #[test]
    fn missing_meta_description_and_canonical_are_flagged() {
        let bare = "<!doctype html><html><head></head><body></body></html>";
        let v = check_page("b.html", bare, HTML_BUDGET_BYTES, None).unwrap();
        assert!(v.iter().any(|m| m.contains("meta description")));
        assert!(v.iter().any(|m| m.contains("canonical")));
    }

    #[test]
    fn minified_attribute_forms_still_pass() {
        let minified = r#"<!doctype html><html lang=en><head><meta charset=utf-8><meta content="d" name=description><title>t</title><link href=https://a/ rel=canonical><link href=/style.css rel=stylesheet></head><body></body></html>"#;
        assert_eq!(
            check_page("index.html", minified, HTML_BUDGET_BYTES, None).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn attribute_parser_handles_quotes_and_bare_attributes() {
        let attrs = parse_attrs(r#" content="a b" name=description defer"#).unwrap();
        assert_eq!(attrs.get("name").map(String::as_str), Some("description"));
        assert_eq!(attrs.get("content").map(String::as_str), Some("a b"));
        assert_eq!(attrs.get("defer").map(String::as_str), Some(""));
    }

    #[test]
    fn page_404_is_exempt_from_the_canonical_requirement_only() {
        let page404 = r#"<!doctype html><html><head><meta name=description content="missing page"></head><body></body></html>"#;
        assert_eq!(
            check_page("404.html", page404, HTML_BUDGET_BYTES, None).unwrap(),
            Vec::<String>::new()
        );
        let v = check_page("other.html", page404, HTML_BUDGET_BYTES, None).unwrap();
        assert!(v.iter().any(|m| m.contains("canonical")));
    }

    #[test]
    fn external_stylesheet_request_is_rejected_and_same_origin_absolute_passes() {
        let external = check_page(
            "x.html",
            &GOOD_PAGE.replace("/style.css", "https://cdn.example/x.css"),
            HTML_BUDGET_BYTES,
            None,
        )
        .unwrap();
        assert!(external.iter().any(|m| m.contains("external")));
        let same_origin =
            GOOD_PAGE.replace("/style.css", "https://apexmediation.example/style.css");
        assert_eq!(
            check_page(
                "x.html",
                &same_origin,
                HTML_BUDGET_BYTES,
                Some("https://apexmediation.example")
            )
            .unwrap(),
            Vec::<String>::new()
        );
        let cross = check_page(
            "x.html",
            &same_origin,
            HTML_BUDGET_BYTES,
            Some("https://other.example"),
        )
        .unwrap();
        assert!(cross.iter().any(|m| m.contains("external")));
        let protocol_relative = GOOD_PAGE.replace("/style.css", "//cdn.example/x.css");
        let v = check_page(
            "x.html",
            &protocol_relative,
            HTML_BUDGET_BYTES,
            Some("https://apexmediation.example"),
        )
        .unwrap();
        assert!(v.iter().any(|m| m.contains("external")));
    }

    #[test]
    fn dist_level_budgets_count_pages_and_css() {
        let pages: BTreeMap<String, String> = (0..6)
            .map(|i| (format!("p{i}.html"), GOOD_PAGE.to_string()))
            .collect();
        assert_eq!(
            check_dist(&pages, 1_000, HTML_BUDGET_BYTES, CSS_BUDGET_BYTES, 6, None).unwrap(),
            Vec::<String>::new()
        );
        let v = check_dist(
            &pages,
            999_999,
            HTML_BUDGET_BYTES,
            CSS_BUDGET_BYTES,
            6,
            None,
        )
        .unwrap();
        assert!(v.iter().any(|m| m.contains("stylesheets total")));
        let one: BTreeMap<String, String> = [("only.html".to_string(), GOOD_PAGE.to_string())]
            .into_iter()
            .collect();
        let v = check_dist(&one, 0, HTML_BUDGET_BYTES, CSS_BUDGET_BYTES, 6, None).unwrap();
        assert!(v.iter().any(|m| m.contains("pages")));
    }
}
