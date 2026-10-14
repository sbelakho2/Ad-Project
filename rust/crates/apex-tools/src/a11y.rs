//! Structural accessibility audit of the rendered console pages, replacing
//! the axe-under-jsdom runner with a Rust checker: document language, title,
//! single h1, script-free markup, image alts, labelled form controls,
//! discernible link text, heading order, table headers and inline-style
//! contrast (WCAG relative luminance, 4.5 for normal text).

use std::path::Path;

use crate::repo::Repo;
use crate::util::{re, rei};

/// Count of pages the rendered fixtures must provide.
pub const MIN_PAGES: usize = 4;
pub const CONTRAST_THRESHOLD: f64 = 4.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HexColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub fn parse_hex(value: &str) -> Option<HexColor> {
    let hex = value.trim().strip_prefix('#')?;
    let (r, g, b) = match hex.len() {
        3 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            (
                ((v >> 8) & 0xF) as u8 * 17,
                ((v >> 4) & 0xF) as u8 * 17,
                (v & 0xF) as u8 * 17,
            )
        }
        6 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            (
                ((v >> 16) & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                (v & 0xFF) as u8,
            )
        }
        _ => return None,
    };
    Some(HexColor { r, g, b })
}

fn channel(value: u8) -> f64 {
    let c = f64::from(value) / 255.0;
    if c <= 0.039_28 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn relative_luminance(color: HexColor) -> f64 {
    0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
}

pub fn contrast_ratio(a: HexColor, b: HexColor) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Find label element spans so controls can check for an enclosing label.
fn label_spans(html: &str) -> Vec<(usize, usize)> {
    let open = match re(r"(?i)<label\b") {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let close = match re(r"(?i)</label\s*>") {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut spans = Vec::new();
    let mut search_from = 0usize;
    for m in open.find_iter(html) {
        if m.start() < search_from {
            continue;
        }
        let Some(end) = close.find_at(html, m.end()) else {
            break;
        };
        spans.push((m.start(), end.end()));
        search_from = end.end();
    }
    spans
}

/// Structural + extended WCAG checks for one rendered page.
pub fn page_violations(rel: &str, html: &str) -> Result<Vec<String>, String> {
    let mut violations = Vec::new();
    if !rei(r#"<html[^>]+lang="[^"]+""#)?.is_match(html)
        && !rei(r"<html[^>]+lang=[a-z-]+")?.is_match(html)
    {
        violations.push(format!("{rel}: <html> is missing a lang attribute"));
    }
    if !rei(r"<title>[^<]+</title>")?.is_match(html) {
        violations.push(format!("{rel}: missing a non-empty <title>"));
    }
    let h1_count = rei(r"<h1[\s>]")?.find_iter(html).count();
    if h1_count != 1 {
        violations.push(format!(
            "{rel}: expected exactly one <h1>, found {h1_count}"
        ));
    }
    if rei(r"<script[\s>]")?.is_match(html) {
        violations.push(format!("{rel}: console must stay script-free"));
    }

    violations.extend(image_violations(rel, html)?);
    violations.extend(control_violations(rel, html)?);
    violations.extend(anchor_violations(rel, html)?);
    violations.extend(heading_order_violations(rel, html)?);
    violations.extend(table_violations(rel, html)?);
    violations.extend(contrast_violations(rel, html)?);
    Ok(violations)
}

fn tag_attrs(inner: &str) -> Result<std::collections::BTreeMap<String, String>, String> {
    crate::perfbudget::parse_attrs(inner)
}

fn image_violations(rel: &str, html: &str) -> Result<Vec<String>, String> {
    let img = rei(r"<img\b([^>]*)>")?;
    let mut violations = Vec::new();
    for c in img.captures_iter(html) {
        let attrs = tag_attrs(&c[1])?;
        let alt = attrs
            .get("alt")
            .map(|a| a.trim().to_string())
            .unwrap_or_default();
        if alt.is_empty() {
            let src = attrs.get("src").map(String::as_str).unwrap_or("(no src)");
            violations.push(format!("{rel}: <img> missing non-empty alt text: {src}"));
        }
    }
    Ok(violations)
}

fn control_violations(rel: &str, html: &str) -> Result<Vec<String>, String> {
    let control = rei(r"<(input|select|textarea)\b([^>]*)>")?;
    let label_for = rei(r#"<label[^>]*\bfor\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))"#)?;
    let mut labelled_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for c in label_for.captures_iter(html) {
        let value = c
            .get(2)
            .or_else(|| c.get(3))
            .or_else(|| c.get(4))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        labelled_ids.insert(value);
    }
    let spans = label_spans(html);
    let mut violations = Vec::new();
    for c in control.captures_iter(html) {
        let kind = c[1].to_lowercase();
        let attrs = tag_attrs(&c[2])?;
        if kind == "input" {
            let input_type = attrs
                .get("type")
                .map(|t| t.to_lowercase())
                .unwrap_or_default();
            if input_type == "hidden" {
                continue;
            }
        }
        let aria = |name: &str| {
            attrs
                .get(name)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
        };
        if aria("aria-label") || aria("aria-labelledby") {
            continue;
        }
        if let Some(id) = attrs.get("id") {
            if labelled_ids.contains(id) {
                continue;
            }
        }
        let enclosed = spans.iter().any(|(a, b)| {
            c.get(0)
                .map(|m| m.start() >= *a && m.end() <= *b)
                .unwrap_or(false)
        });
        if enclosed {
            continue;
        }
        let tag = c.get(0).map(|m| m.as_str()).unwrap_or("");
        violations.push(format!(
            "{rel}: <{kind}> has no label: {}",
            truncate(tag, 80)
        ));
    }
    Ok(violations)
}

fn anchor_violations(rel: &str, html: &str) -> Result<Vec<String>, String> {
    let anchor = rei(r"<a\b([^>]*)>")?;
    let close = rei(r"(?i)</a\s*>")?;
    let tags = rei(r"<[^>]+>")?;
    let mut violations = Vec::new();
    for c in anchor.captures_iter(html) {
        let attrs = tag_attrs(&c[1])?;
        let end = close.find_at(html, c.get(0).map(|m| m.end()).unwrap_or(html.len()));
        let inner = match end {
            Some(e) => &html[c.get(0).map(|m| m.end()).unwrap_or(0)..e.start()],
            None => "",
        };
        let text = tags.replace_all(inner, " ");
        let text = text.trim();
        let aria = attrs
            .get("aria-label")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        if text.is_empty() && !aria {
            violations.push(format!(
                "{rel}: <a> has no discernible text: {}",
                truncate(c.get(0).map(|m| m.as_str()).unwrap_or(""), 80)
            ));
        }
    }
    Ok(violations)
}

fn heading_order_violations(rel: &str, html: &str) -> Result<Vec<String>, String> {
    let heading = rei(r"<h([1-6])[\s>]")?;
    let mut violations = Vec::new();
    let mut prev: Option<u32> = None;
    for c in heading.captures_iter(html) {
        let level: u32 = c[1].parse().unwrap_or(1);
        if let Some(p) = prev {
            if level > p + 1 {
                violations.push(format!(
                    "{rel}: heading level h{p} -> h{level} skips a level"
                ));
            }
        }
        prev = Some(level);
    }
    Ok(violations)
}

fn table_violations(rel: &str, html: &str) -> Result<Vec<String>, String> {
    let table = rei(r"(?s)<table\b[^>]*>(.*?)</table\s*>")?;
    let th = rei(r"<th[\s>]")?;
    let td = rei(r"<td[\s>]")?;
    let mut violations = Vec::new();
    for c in table.captures_iter(html) {
        let body = &c[1];
        if !th.is_match(body) && td.is_match(body) {
            violations.push(format!("{rel}: data table has no <th> header cells"));
        }
    }
    Ok(violations)
}

/// Inline-style contrast: hex `color:` / `background` pairs declared in the
/// same rule must reach the WCAG 4.5 threshold for normal text.
pub fn contrast_violations(rel: &str, html: &str) -> Result<Vec<String>, String> {
    let style = rei(r"(?s)<style\b[^>]*>(.*?)</style\s*>")?;
    let rule = re(r"([^{}]+)\{([^}]*)\}")?;
    let color_decl = rei(r"(?m)(?:^|;)\s*color\s*:\s*([^;{}]+)")?;
    let bg_decl = rei(r"(?m)(?:^|;)\s*background(?:-color)?\s*:\s*([^;{}]+)")?;
    let hex = re(r"#[0-9a-fA-F]{3,6}")?;
    let mut violations = Vec::new();
    for block in style.captures_iter(html) {
        for r in rule.captures_iter(&block[1]) {
            let decls = &r[2];
            let Some(fg_raw) = color_decl
                .captures(decls)
                .and_then(|c| hex.find(&c[1]).map(|m| m.as_str().to_string()))
            else {
                continue;
            };
            let Some(bg_raw) = bg_decl
                .captures(decls)
                .and_then(|c| hex.find(&c[1]).map(|m| m.as_str().to_string()))
            else {
                continue;
            };
            let (Some(fg), Some(bg)) = (parse_hex(&fg_raw), parse_hex(&bg_raw)) else {
                continue;
            };
            let ratio = contrast_ratio(fg, bg);
            if ratio < CONTRAST_THRESHOLD {
                violations.push(format!(
                    "{rel}: low contrast {fg_raw} on {bg_raw} (ratio {ratio:.2} < {CONTRAST_THRESHOLD})"
                ));
            }
        }
    }
    Ok(violations)
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect::<String>() + "..."
}

pub fn console_a11y(repo: &Repo, dir_arg: Option<&str>) -> Result<(), String> {
    let argument = dir_arg.unwrap_or("rust/target/ui-fixtures");
    let dir = if Path::new(argument).is_absolute() {
        std::path::PathBuf::from(argument)
    } else {
        repo.path(argument)
    };
    if !dir.is_dir() {
        return Err(format!(
            "console-a11y: rendered dir not found: {} (run the render-fixtures bin first)",
            crate::util::slash(&dir)
        ));
    }
    let mut pages: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", crate::util::slash(&dir)))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "html").unwrap_or(false))
        .collect();
    pages.sort();
    if pages.len() < MIN_PAGES {
        return Err(format!(
            "console-a11y: expected at least {MIN_PAGES} rendered pages, found {}",
            pages.len()
        ));
    }
    let mut findings = Vec::new();
    for page in &pages {
        let rel = page
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let html = std::fs::read_to_string(page).map_err(|e| format!("{rel}: {e}"))?;
        findings.extend(page_violations(&rel, &html)?);
    }
    if !findings.is_empty() {
        return Err(format!(
            "CONSOLE A11Y FAILED ({}):\n  - {}",
            findings.len(),
            findings.join("\n  - ")
        ));
    }
    println!(
        "console a11y PASS: {} rendered pages, structural WCAG 2.0/2.1 A+AA checks clean (lang, title, single h1, script-free, labels, alt text, link names, heading order, table headers, inline contrast)",
        pages.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_pages_in_memory(pages: &[(String, String)]) -> Result<Vec<String>, String> {
        let mut findings = Vec::new();
        for (rel, html) in pages {
            findings.extend(page_violations(rel, html)?);
        }
        Ok(findings)
    }

    const GOOD: &str = r#"<!doctype html><html lang="en"><head><title>Console</title></head><body><main><h1>Overview</h1></main></body></html>"#;

    #[test]
    fn structure_a_clean_page_passes() {
        assert_eq!(
            page_violations("overview.html", GOOD).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn structure_missing_lang_title_h1_count_and_script_are_violations() {
        let html = "<html><head></head><body><h1>a</h1><h1>b</h1><script>1</script></body></html>";
        let v = page_violations("x.html", html).unwrap();
        assert!(v.iter().any(|m| m.contains("lang attribute")));
        assert!(v.iter().any(|m| m.contains("title")));
        assert!(v.iter().any(|m| m.contains("exactly one <h1>")));
        assert!(v.iter().any(|m| m.contains("script-free")));
    }

    #[test]
    fn img_requires_non_empty_alt() {
        let bad = GOOD.replace(
            "<main>",
            r#"<main><img src="a.png"><img src="b.png" alt="">"#,
        );
        let v = page_violations("x.html", &bad).unwrap();
        assert_eq!(v.iter().filter(|m| m.contains("alt text")).count(), 2);
        let ok = GOOD.replace("<main>", r#"<main><img src="a.png" alt="Chart">"#);
        assert!(page_violations("x.html", &ok).unwrap().is_empty());
    }

    #[test]
    fn controls_need_labels_but_hidden_inputs_are_exempt() {
        let bad = GOOD.replace(
            "<main>",
            r#"<main><input id="q"><select aria-label="Pick"><option>x</option></select><textarea></textarea><input type="hidden"><label for="other">O</label>"#,
        );
        let v = page_violations("x.html", &bad).unwrap();
        assert_eq!(v.iter().filter(|m| m.contains("has no label")).count(), 2);
        let ok = GOOD.replace(
            "<main>",
            r#"<main><label for="q">Search</label><input id="q"><label>Message<textarea></textarea></label><select aria-labelledby="t"><option>x</option></select><span id="t">P</span>"#,
        );
        assert!(page_violations("x.html", &ok).unwrap().is_empty());
    }

    #[test]
    fn anchors_need_text_or_aria_label() {
        let bad = GOOD.replace(
            "<main>",
            r#"<main><a href="/x"></a><a href="/y"><span></span></a>"#,
        );
        let v = page_violations("x.html", &bad).unwrap();
        assert_eq!(
            v.iter().filter(|m| m.contains("discernible text")).count(),
            2
        );
        let ok = GOOD.replace(
            "<main>",
            r#"<main><a href="/x">x</a><a href="/y" aria-label="y"></a>"#,
        );
        assert!(page_violations("x.html", &ok).unwrap().is_empty());
    }

    #[test]
    fn heading_levels_may_not_skip_downward() {
        let bad = GOOD.replace("<h1>Overview</h1>", "<h1>Overview</h1><h3>Deep</h3>");
        let v = page_violations("x.html", &bad).unwrap();
        assert!(v.iter().any(|m| m.contains("skips a level")));
        let ok = GOOD.replace(
            "<h1>Overview</h1>",
            "<h1>Overview</h1><h2>A</h2><h3>B</h3><h2>C</h2>",
        );
        assert!(page_violations("x.html", &ok).unwrap().is_empty());
    }

    #[test]
    fn data_tables_need_header_cells() {
        let bad = GOOD.replace("<main>", "<main><table><tr><td>1</td></tr></table>");
        let v = page_violations("x.html", &bad).unwrap();
        assert!(v.iter().any(|m| m.contains("no <th>")));
        let ok = GOOD.replace(
            "<main>",
            "<main><table><tr><th>h</th></tr><tr><td>1</td></tr></table>",
        );
        assert!(page_violations("x.html", &ok).unwrap().is_empty());
    }

    #[test]
    fn contrast_formula_matches_wcag_reference_values() {
        let black = parse_hex("#000").unwrap();
        let white = parse_hex("#fff").unwrap();
        assert!((contrast_ratio(black, white) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(white, white) - 1.0).abs() < 0.01);
        let grey = parse_hex("#777").unwrap();
        let ratio = contrast_ratio(grey, white);
        assert!(
            ratio > 4.4 && ratio < 4.6,
            "grey on white is near the threshold: {ratio}"
        );
    }

    #[test]
    fn inline_style_low_contrast_is_flagged() {
        let html = GOOD.replace(
            "<title>Console</title>",
            "<title>Console</title><style>.low{color:#aaa;background:#fff}.ok{color:#000;background:#fff}</style>",
        );
        let v = page_violations("x.html", &html).unwrap();
        assert_eq!(v.iter().filter(|m| m.contains("low contrast")).count(), 1);
    }

    #[test]
    fn in_memory_pages_report_per_page() {
        let pages = vec![
            ("a.html".to_string(), GOOD.to_string()),
            ("b.html".to_string(), GOOD.replace("lang=\"en\"", "")),
        ];
        let v = check_pages_in_memory(&pages).unwrap();
        assert_eq!(v.len(), 1);
        assert!(v[0].starts_with("b.html:"));
    }
}
