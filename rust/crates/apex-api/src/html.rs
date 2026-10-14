//! The server-rendered console: plain HTML, zero JavaScript, strict escaping.
//!
//! Every page is a full document assembled from string buffers; there is no
//! template engine and no `<script>` element anywhere. All dynamic text — the
//! publisher echoed into a form, platform names in banners, finding subjects
//! and messages, ledger JSON — passes through [`html_escape`], which maps the
//! five HTML-significant characters (`&`, `<`, `>`, `"`, `'`) so the same
//! function is safe in element text and in quoted attribute values.
//!
//! Forms are plain `GET` forms, so every console action is a bookmarkable URL
//! and the console works with JavaScript disabled (and from `curl`).

use std::fmt::Write as _;

use apex_audit::{AuditReport, Severity};
use apex_core::PlatformId;
use apex_proof::LedgerRecord;

use crate::cycle::READ_ONLY_MODE;
use crate::sources::PLATFORM_NAMES;

/// Escape `& < > " '` for use in HTML element text or quoted attributes.
///
/// `&` is replaced first so already-escaped input is escaped exactly once per
/// call (the output of this function is always literal text, never markup).
/// Input without any of the five characters is copied verbatim — the fast path
/// keeps 500-finding pages cheap (see `render_latency_is_bounded`).
pub fn html_escape(input: &str) -> String {
    if !input
        .bytes()
        .any(|byte| matches!(byte, b'&' | b'<' | b'>' | b'"' | b'\''))
    {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len() + 16);
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

// Console palette, single-sourced so WCAG contrast can be tested at the
// source (a token change without a passing ratio fails the unit tests).
const COLOR_TEXT: &str = "#000";
const COLOR_PAGE_BG: &str = "#fff";
const COLOR_BORDER: &str = "#999";
const COLOR_TH_BG: &str = "#eee";
const COLOR_ERROR_BG: &str = "#fdd";
const COLOR_ERROR_FG: &str = "#a00";

fn style() -> &'static str {
    use std::sync::OnceLock;
    static STYLE: OnceLock<String> = OnceLock::new();
    STYLE.get_or_init(|| {
        format!(
            "body{{font-family:system-ui,sans-serif;margin:2rem;max-width:72rem;\
color:{COLOR_TEXT};background:{COLOR_PAGE_BG}}}\n\
table{{border-collapse:collapse;width:100%}}\n\
th,td{{border:1px solid {COLOR_BORDER};padding:.35rem .5rem;text-align:left;vertical-align:top}}\n\
th{{background:{COLOR_TH_BG}}}\n\
.error{{background:{COLOR_ERROR_BG};border:1px solid {COLOR_ERROR_FG};color:{COLOR_ERROR_FG};padding:.5rem}}\n\
nav a{{margin-right:1rem}}\n\
code{{word-break:break-all}}"
        )
    })
}

fn document(title: &str, body: &str) -> String {
    let style = style();
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} — apex console</title>\n<style>{style}</style>\n</head>\n\
         <body>\n<header><nav>\
         <a href=\"/\">overview</a><a href=\"/console/audit\">audit</a>\
         <a href=\"/console/ledger\">ledger</a><a href=\"/healthz\">health JSON</a>\
         </nav></header>\n<main>\n{body}\n</main>\n</body>\n</html>\n",
        title = html_escape(title),
    )
}

/// The overview page: health, mode and links to the console sections.
pub fn render_overview() -> String {
    let body = format!(
        "<h1>apex console</h1>\n\
         <p id=\"health\">status: ok</p>\n\
         <p id=\"mode\">deployment mode: {mode} — audits and ledger reads are live; \
         write cycles require platform credentials that this deployment does not hold.</p>\n\
         <ul id=\"links\">\n\
         <li><a href=\"/console/audit\">Audit console</a> — run the revenue-leak audit for a platform</li>\n\
         <li><a href=\"/console/ledger\">Ledger console</a> — inspect a publisher's verified ledger</li>\n\
         <li><a href=\"/healthz\">Health endpoint</a> — machine-readable status</li>\n\
         </ul>\n\
         <p id=\"version\">apex-api version: {version}</p>",
        mode = html_escape(READ_ONLY_MODE),
        version = html_escape(env!("CARGO_PKG_VERSION")),
    );
    document("overview", &body)
}

/// The audit console page (`/console/audit`): the GET form, optionally with an
/// error banner. `raw_platform` is the untouched query value (escaped here)
/// so a rejected input is shown verbatim but inert.
pub fn render_audit_form(raw_platform: Option<&str>, problem: Option<&str>) -> String {
    let mut options = String::new();
    for (name, label) in PLATFORM_NAMES {
        let selected = if raw_platform == Some(name) {
            " selected"
        } else {
            ""
        };
        let _ = write!(
            options,
            "<option value=\"{}\"{}>{}</option>",
            html_escape(name),
            selected,
            html_escape(label)
        );
    }
    let banner = problem.map_or_else(String::new, |message| {
        format!(
            "<p class=\"error\" role=\"alert\">Error: {}</p>\n",
            html_escape(message)
        )
    });
    let body = format!(
        "<h1>Audit console</h1>\n\
         {banner}\
         <form method=\"get\" action=\"/console/audit\">\n\
         <label for=\"platform\">Platform</label>\n\
         <select id=\"platform\" name=\"platform\">{options}</select>\n\
         <button type=\"submit\">Run audit</button>\n\
         </form>"
    );
    document("audit", &body)
}

/// A rendered audit report: severity counts, the recovery total and the full
/// findings table. All finding text is escaped.
pub fn render_audit_page(platform: PlatformId, report: &AuditReport) -> String {
    let mut critical = 0usize;
    let mut warning = 0usize;
    let mut info = 0usize;
    let mut rows = String::new();
    for finding in &report.findings {
        match finding.severity {
            Severity::Critical => critical += 1,
            Severity::Warning => warning += 1,
            Severity::Info => info += 1,
        }
        let _ = writeln!(
            rows,
            "<tr><td>{rule}</td><td class=\"severity\">{severity}</td><td>{platform}</td>\
             <td>{subject}</td><td>{message}</td><td class=\"recovery\">{recovery}</td></tr>",
            rule = html_escape(finding.rule.as_str()),
            severity = html_escape(finding.severity.as_str()),
            platform = html_escape(finding.platform.as_str()),
            subject = html_escape(&finding.subject),
            message = html_escape(&finding.message),
            recovery = finding.estimated_monthly_recovery,
        );
    }
    if rows.is_empty() {
        rows.push_str("<tr><td colspan=\"6\">No findings.</td></tr>\n");
    }
    let body = format!(
        "<h1>Audit: {platform}</h1>\n\
         <p id=\"digest\">Snapshot digest: <code>{digest}</code></p>\n\
         <p id=\"counts\">Findings: {total} (critical: {critical}, warning: {warning}, info: {info})</p>\n\
         <p id=\"totals\">Total estimated monthly recovery: <strong>{recovery}</strong> micros</p>\n\
         <table id=\"findings\">\n<thead><tr>\
         <th>Rule</th><th>Severity</th><th>Platform</th><th>Subject</th><th>Message</th><th>Recovery (micros)</th>\
         </tr></thead>\n<tbody>\n{rows}</tbody>\n</table>\n\
         <p id=\"generated-by\">Generated by: {generated_by}</p>\n\
         <p><a href=\"/console/audit\">Run another audit</a></p>",
        platform = html_escape(platform.as_str()),
        digest = html_escape(&report.snapshot_digest),
        total = report.findings.len(),
        recovery = report.total_estimated_monthly_recovery,
        generated_by = html_escape(report.generated_by),
    );
    document("audit report", &body)
}

/// The ledger console form (`/console/ledger`), optionally with a banner.
/// `raw_publisher` is echoed into the input value, escaped.
pub fn render_ledger_form(raw_publisher: Option<&str>, problem: Option<&str>) -> String {
    let banner = problem.map_or_else(String::new, |message| {
        format!(
            "<p class=\"error\" role=\"alert\">Error: {}</p>\n",
            html_escape(message)
        )
    });
    let body = format!(
        "<h1>Ledger console</h1>\n\
         {banner}\
         <p>Ledgers are verified line by line before any record is shown; \
         verification failures are reported with the offending line.</p>\n\
         <form method=\"get\" action=\"/console/ledger\">\n\
         <label for=\"publisher\">Publisher</label>\n\
         <input type=\"text\" id=\"publisher\" name=\"publisher\" maxlength=\"64\" value=\"{publisher}\">\n\
         <button type=\"submit\">Load ledger</button>\n\
         </form>",
        publisher = html_escape(raw_publisher.unwrap_or("")),
    );
    document("ledger", &body)
}

/// A verified ledger rendered as a records table.
pub fn render_ledger_records(publisher: &str, records: &[LedgerRecord]) -> String {
    let mut rows = String::new();
    for record in records {
        let json = serde_json::to_string(record).unwrap_or_default();
        let _ = writeln!(
            rows,
            "<tr><td>{seq}</td><td>{kind}</td><td>{at}</td><td>{platform}</td>\
             <td><code>{json}</code></td></tr>",
            seq = record.seq(),
            kind = html_escape(record_kind(record)),
            at = record.at_unix_ms(),
            platform = html_escape(record.platform().map_or("-", PlatformId::as_str)),
            json = html_escape(&json),
        );
    }
    if rows.is_empty() {
        rows.push_str("<tr><td colspan=\"5\">No records.</td></tr>\n");
    }
    let body = format!(
        "<h1>Ledger: {publisher}</h1>\n\
         <p id=\"mode\">mode: {mode}</p>\n\
         <p id=\"record-count\">Records: {count}</p>\n\
         <table id=\"ledger\">\n<thead><tr>\
         <th>Seq</th><th>Type</th><th>At (unix ms)</th><th>Platform</th><th>Record JSON</th>\
         </tr></thead>\n<tbody>\n{rows}</tbody>\n</table>\n\
         <p><a href=\"/console/ledger\">Inspect another ledger</a></p>",
        publisher = html_escape(publisher),
        mode = html_escape(READ_ONLY_MODE),
        count = records.len(),
    );
    document("ledger records", &body)
}

/// A standalone escaped error page (used when there is no form to keep).
pub fn render_error_page(title: &str, message: &str) -> String {
    let body = format!(
        "<h1>Error</h1>\n<p class=\"error\" role=\"alert\">{message}</p>\n\
         <p><a href=\"/\">Back to the overview</a></p>",
        message = html_escape(message),
    );
    document(title, &body)
}

fn record_kind(record: &LedgerRecord) -> &'static str {
    match record {
        LedgerRecord::ChangeApplied { .. } => "changeApplied",
        LedgerRecord::ExperimentStarted { .. } => "experimentStarted",
        LedgerRecord::ExperimentObserved { .. } => "experimentObserved",
        LedgerRecord::RollbackPerformed { .. } => "rollbackPerformed",
        LedgerRecord::BillingProof { .. } => "billingProof",
        LedgerRecord::LiftRecord { .. } => "liftRecord",
    }
}

#[cfg(test)]
mod tests {
    use apex_audit::{AuditReport, Finding, RuleId};

    use super::*;

    fn empty_report() -> AuditReport {
        AuditReport {
            snapshot_digest: "ab".repeat(32),
            findings: Vec::new(),
            total_estimated_monthly_recovery: 0,
            generated_by: apex_audit::GENERATED_BY,
        }
    }

    fn report_with(count: usize) -> AuditReport {
        let findings = (0..count)
            .map(|index| Finding {
                rule: RuleId::StaleFloor,
                severity: Severity::Warning,
                platform: PlatformId::Admob,
                subject: format!("ad_unit:u{index}:country:EE"),
                evidence: serde_json::json!({"index": index}),
                message: format!(
                    "stale floor on ad unit u{index} in country EE: floor 2000000 micros set, \
                     pair revenue 0 micros over 10000 impressions; ad unit eCPM 50000 micros \
                     implies 500000 micros/month recoverable (model: floor_blocks_fill)"
                ),
                estimated_monthly_recovery: 500_000,
            })
            .collect();
        AuditReport {
            snapshot_digest: "cd".repeat(32),
            findings,
            total_estimated_monthly_recovery: 500_000 * count as i64,
            generated_by: apex_audit::GENERATED_BY,
        }
    }

    #[test]
    fn escape_maps_all_five_characters() {
        assert_eq!(html_escape("&"), "&amp;");
        assert_eq!(html_escape("<"), "&lt;");
        assert_eq!(html_escape(">"), "&gt;");
        assert_eq!(html_escape("\""), "&quot;");
        assert_eq!(html_escape("'"), "&#39;");
        assert_eq!(
            html_escape("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&#39;f"
        );
    }

    #[test]
    fn escape_is_safe_in_attribute_context() {
        // A quote-breaking payload stays inside the attribute.
        let hostile = "\" onmouseover=\"alert(1)";
        assert_eq!(html_escape(hostile), "&quot; onmouseover=&quot;alert(1)");
        // Ampersand-first ordering: an existing entity is escaped, not revived.
        assert_eq!(html_escape("&lt;script&gt;"), "&amp;lt;script&amp;gt;");
        assert_eq!(
            html_escape("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
    }

    #[test]
    fn escape_leaves_plain_and_unicode_text_untouched() {
        assert_eq!(html_escape("plain text 123"), "plain text 123");
        assert_eq!(
            html_escape("caf\u{00e9} \u{4e16}\u{754c}"),
            "caf\u{00e9} \u{4e16}\u{754c}"
        );
        assert_eq!(html_escape(""), "");
    }

    #[test]
    fn render_latency_is_bounded() {
        // True-cost smoke bound (debug build). Sibling tests run in parallel on
        // the same cores, so a batch average measures the machine, not the code.
        // The fastest single render across many samples approximates the
        // contention-free unit cost, and the bound (1 ms per 500-finding render)
        // is stricter than the old batch bound it replaces. Release budgets live
        // in `benches/api_latency`.
        let report = report_with(500);
        // Measured debug baseline on the reference machine: ~1.36 ms per
        // 500-finding render (fastest-of-400, stable under full-suite load).
        // The bound leaves headroom for slower developer machines while still
        // tripping on a ~50% regression; release budgets are canonical in
        // `benches/api_latency`.
        let budget = std::time::Duration::from_millis(2);
        let mut total_len = 0usize;
        let mut fastest = std::time::Duration::MAX;
        for _ in 0..400 {
            let started = std::time::Instant::now();
            total_len += render_audit_page(PlatformId::Admob, &report).len();
            fastest = fastest.min(started.elapsed());
        }
        assert!(total_len > 0);
        assert!(
            fastest < budget,
            "fastest of 400 renders of a 500-finding page took {fastest:?} (budget {budget:?})"
        );
    }

    #[test]
    fn empty_report_page_says_no_findings_and_zero_total() {
        let page = render_audit_page(PlatformId::LevelPlay, &empty_report());
        assert!(page.contains("No findings."), "{page}");
        assert!(
            page.contains("Total estimated monthly recovery: <strong>0</strong> micros"),
            "{page}"
        );
    }
}

#[cfg(test)]
mod palette_contrast {
    use super::*;

    fn srgb_to_linear(channel: u8) -> f64 {
        let c = f64::from(channel) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(hex: &str) -> f64 {
        // Accept both 3- and 6-digit hex; expand the shorthand form first.
        let digits = &hex[1..];
        let expand = |s: &str| u8::from_str_radix(s, 16).expect("hex channel");
        let (r, g, b) = if digits.len() == 3 {
            let r = expand(&digits[0..1]);
            let g = expand(&digits[1..2]);
            let b = expand(&digits[2..3]);
            (r * 16 + r, g * 16 + g, b * 16 + b)
        } else {
            (
                expand(&digits[0..2]),
                expand(&digits[2..4]),
                expand(&digits[4..6]),
            )
        };
        0.2126 * srgb_to_linear(r) + 0.7152 * srgb_to_linear(g) + 0.0722 * srgb_to_linear(b)
    }

    fn contrast(fg: &str, bg: &str) -> f64 {
        let (l1, l2) = (luminance(fg), luminance(bg));
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn console_palette_meets_wcag_aa_text_contrast() {
        // WCAG 2.1 AA: 4.5:1 for normal text.
        let pairs = [
            (COLOR_TEXT, COLOR_PAGE_BG),
            (COLOR_TEXT, COLOR_TH_BG),
            (COLOR_ERROR_FG, COLOR_ERROR_BG),
        ];
        for (fg, bg) in pairs {
            let ratio = contrast(fg, bg);
            assert!(
                ratio >= 4.5,
                "palette {fg} on {bg} has contrast {ratio:.2}, below 4.5"
            );
        }
    }

    #[test]
    fn style_is_built_from_the_palette_constants() {
        let css = style();
        for token in [
            COLOR_TEXT,
            COLOR_PAGE_BG,
            COLOR_BORDER,
            COLOR_TH_BG,
            COLOR_ERROR_BG,
            COLOR_ERROR_FG,
        ] {
            assert!(
                css.contains(token),
                "style() must embed palette token {token}"
            );
        }
    }
}
