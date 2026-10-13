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

const STYLE: &str = "body{font-family:system-ui,sans-serif;margin:2rem;max-width:72rem}\n\
table{border-collapse:collapse;width:100%}\n\
th,td{border:1px solid #999;padding:.35rem .5rem;text-align:left;vertical-align:top}\n\
th{background:#eee}\n\
.error{background:#fdd;border:1px solid #a00;padding:.5rem}\n\
nav a{margin-right:1rem}\n\
code{word-break:break-all}";

fn document(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} — apex console</title>\n<style>{STYLE}</style>\n</head>\n\
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
        // Documented smoke bound, generous on purpose (debug build, shared CI
        // machine): 200 renders of a 500-finding page must stay under 250 ms.
        // The real budgets live in the CI lane driven by `benches/api_latency`.
        let report = report_with(500);
        let started = std::time::Instant::now();
        let mut total_len = 0usize;
        for _ in 0..200 {
            total_len += render_audit_page(PlatformId::Admob, &report).len();
        }
        let elapsed = started.elapsed();
        assert!(total_len > 0);
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "200 renders of a 500-finding page took {elapsed:?}"
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
