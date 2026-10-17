//! Audit console pages: the platform form, the report, the finding detail and
//! the CSV export.
//!
//! Finding ids are stable across renders: `f_` plus the first 12 hex
//! characters of SHA-256 over `domain tag | rule | platform | subject`
//! (`apex-finding-v1`). The engine sorts findings by severity, so the report
//! table reads as severity groups; each row's Priority chip names its group.
//!
//! Estimates: the report and the finding detail render
//! `estimated_monthly_recovery` through [`apex_ui::format::money`] with the
//! deployment currency. A finding whose evidence carries no money model
//! (`not_applicable`, `insufficient_data`, `unavailable_ecpm`, or no model at
//! all) says `Not estimated` and the finding page adds the rule-level note
//! for that model. Money is never inferred from an unlabelled integer.

use std::fmt::Write as _;

use apex_audit::{AuditReport, Finding, RuleId, Severity, Snapshot};
use apex_core::PlatformId;
use apex_ui::components::{
    action, data_table, field, kpi_card, kpi_row, notice, panel, select, split, Cell, EmptyState,
};
use apex_ui::format::{self, money, or_not_available, utc_datetime};
use apex_ui::status::{self, StatusKind};
use sha2::{Digest, Sha256};

use crate::sources::PLATFORM_NAMES;

use super::{console_document, ConsoleCtx};

/// Domain tag of every finding id; changing it changes every id.
const FINDING_ID_DOMAIN: &str = "apex-finding-v1";
/// Length of the hex hash carried by a finding id.
const FINDING_ID_HEX: usize = 12;

/// The stable id of one finding: `f_` plus 12 hex characters of SHA-256 over
/// `apex-finding-v1\0rule\0platform\0subject`.
pub fn finding_id(rule: &str, platform: &str, subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FINDING_ID_DOMAIN.as_bytes());
    for part in [rule, platform, subject] {
        hasher.update([0]);
        hasher.update(part.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    format!("f_{}", &digest[..FINDING_ID_HEX])
}

/// Whether `value` has the shape of a finding id (`f_` + 12 hex).
pub fn is_finding_id(value: &str) -> bool {
    value.strip_prefix("f_").is_some_and(|hex| {
        hex.len() == FINDING_ID_HEX && hex.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// The human label of a platform id.
pub fn platform_label(platform: PlatformId) -> &'static str {
    PLATFORM_NAMES
        .iter()
        .find(|(name, _)| *name == platform.as_str())
        .map(|(_, label)| *label)
        .unwrap_or("Platform")
}

/// The capitalized severity label of a finding.
pub fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "Critical",
        Severity::Warning => "Warning",
        Severity::Info => "Info",
    }
}

/// Why the audit form is being rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditFormProblem {
    /// The query string could not be interpreted (duplicate keys).
    BadParameters(String),
    /// The requested platform is not in the closed set.
    UnknownPlatform(String),
    /// No snapshot exists for the selected platform.
    NoSnapshot(String),
}

/// Form states that must answer an error status.
pub fn form_problem_status(problem: &AuditFormProblem) -> u16 {
    match problem {
        AuditFormProblem::NoSnapshot(_) => 404,
        _ => 400,
    }
}

/// The audit form page (`/console/audit` without a usable platform).
pub fn render_audit_form(ctx: &ConsoleCtx, problem: Option<&AuditFormProblem>) -> String {
    let (selected, field_error) = match problem {
        Some(AuditFormProblem::NoSnapshot(platform)) => (Some(platform.as_str()), None),
        _ => (None, Some("Choose a supported platform.")),
    };
    let mut body = String::from("<h1>Review an audit snapshot</h1>");
    body.push_str("<p>Choose a platform to inspect the snapshot available to this workspace.</p>");
    body.push_str(&problem_notice(problem));
    let options: Vec<(String, String)> = PLATFORM_NAMES
        .iter()
        .map(|(name, label)| ((*name).to_string(), (*label).to_string()))
        .collect();
    let form = format!(
        "<form method=\"get\" action=\"/console/audit\">{}\
         <div class=\"form-actions\">\
         <button class=\"action action-primary\" type=\"submit\">View audit</button></div></form>",
        field(
            "platform",
            "Platform",
            Some("The report shows its source, scope and capture time."),
            field_error,
            &select("platform", "platform", &options, selected),
        )
    );
    let checks = panel(
        "What is checked",
        "<ul><li>Floors and bidder settings</li>\
         <li>Country coverage</li>\
         <li>Revenue reporting gaps</li>\
         <li>app-ads.txt</li></ul>",
        None,
    );
    body.push_str(&split(&form, &checks));
    console_document(ctx, "Audit", "Audit", &body)
}

fn problem_notice(problem: Option<&AuditFormProblem>) -> String {
    match problem {
        None => String::new(),
        Some(AuditFormProblem::BadParameters(detail)) => format!(
            "{}<p class=\"table-note\">{}</p>",
            notice(
                StatusKind::Failure,
                "Check the request.",
                "A field or upload could not be accepted.",
                None,
            ),
            super::html_escape(detail)
        ),
        Some(AuditFormProblem::UnknownPlatform(raw)) => format!(
            "{}<p class=\"table-note\">Received platform: <code>{}</code></p>",
            notice(
                StatusKind::Failure,
                "Check the request.",
                "A field or upload could not be accepted.",
                None,
            ),
            super::html_escape(raw)
        ),
        Some(AuditFormProblem::NoSnapshot(_)) => notice(
            StatusKind::Failure,
            "No audit snapshot is available.",
            "This platform has no report in the current workspace.",
            Some(("Contact support", "/support/")),
        ),
    }
}

/// A rendered audit report.
#[derive(Debug, Clone, Copy)]
pub struct AuditReportView<'a> {
    /// The platform the snapshot was captured for.
    pub platform: PlatformId,
    /// The snapshot the report was computed from.
    pub snapshot: &'a Snapshot,
    /// The engine report.
    pub report: &'a AuditReport,
}

/// Rule groups with input available, from the snapshot's own fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// Groups whose inputs exist in the snapshot.
    pub available: usize,
    /// Human labels of the groups whose inputs are absent.
    pub missing: Vec<&'static str>,
}

impl Coverage {
    /// Total number of rule groups.
    pub fn total(&self) -> usize {
        RuleId::ALL.len()
    }

    /// True when every group had input.
    pub fn complete(&self) -> bool {
        self.missing.is_empty()
    }
}

/// The coverage contract: which rule groups can run on this snapshot, derived
/// from the snapshot's own fields (never from the finding count). The
/// app-ads.txt fetch result and the two 30-day totals are the rules' input by
/// definition and are always present; the country revenue rows feed both floor
/// rules and the bidder accounts feed the disabled-bidder rule.
pub fn coverage(snapshot: &Snapshot) -> Coverage {
    let mut missing = Vec::new();
    if snapshot.country_revenue.is_empty() {
        missing.push("Stale floor");
        missing.push("Missing country floor");
    }
    if snapshot.bidder_accounts.is_empty() {
        missing.push("Disabled bidder");
    }
    Coverage {
        available: RuleId::ALL.len() - missing.len(),
        missing,
    }
}

/// The recovery model carried in the finding's evidence, when present.
pub fn recovery_model(finding: &Finding) -> Option<&str> {
    finding
        .evidence
        .get("recovery_model")
        .and_then(|value| value.as_str())
}

/// Models that estimate no money, so the finding reads `Not estimated`.
const NO_MONEY_MODELS: [&str; 3] = ["not_applicable", "insufficient_data", "unavailable_ecpm"];

/// Whether this finding carries a money model.
pub fn has_money_model(finding: &Finding) -> bool {
    matches!(recovery_model(finding), Some(model) if !NO_MONEY_MODELS.contains(&model))
}

/// The Estimate cell text: `USD 420.00 / month`, or `Not estimated` when the
/// finding carries no money model.
pub fn estimate_label(finding: &Finding, currency: &str) -> String {
    if has_money_model(finding) {
        format!(
            "{} / month",
            money(finding.estimated_monthly_recovery, currency)
        )
    } else {
        "Not estimated".to_string()
    }
}

/// The rule-level note shown where the model estimates no money.
pub fn missing_model_note(finding: &Finding) -> &'static str {
    match recovery_model(finding) {
        Some("not_applicable") => "This rule does not model a monthly recovery.",
        Some("insufficient_data") => {
            "The snapshot does not carry enough data for a monthly estimate."
        }
        Some("unavailable_ecpm") => {
            "The snapshot cannot support a monthly estimate for this finding."
        }
        _ => "No recovery model applies to this finding.",
    }
}

/// Render the audit report page.
pub fn render_audit_report(ctx: &ConsoleCtx, view: &AuditReportView<'_>) -> String {
    let mut critical = 0usize;
    let mut warning = 0usize;
    let mut info = 0usize;
    for finding in &view.report.findings {
        match finding.severity {
            Severity::Critical => critical += 1,
            Severity::Warning => warning += 1,
            Severity::Info => info += 1,
        }
    }
    let findings = view.report.findings.len();
    let covered = coverage(view.snapshot);
    let label = platform_label(view.platform);
    let mut body = format!(
        "<h1>{label} audit</h1><p>{}</p>",
        status::chip("Snapshot", StatusKind::Neutral)
    );
    body.push_str(&format!(
        "<p class=\"table-note\">{} / Captured {} / 30-day input</p>",
        super::html_escape(&ctx.publisher_label),
        utc_datetime(view.snapshot.fetched_at_unix_ms)
    ));
    body.push_str(&kpi_row(&[
        kpi_card(
            "Findings",
            &findings.to_string(),
            &format!("{critical} critical / {warning} warning / {info} info"),
            None,
        ),
        kpi_card(
            "Estimated opportunity",
            &format!(
                "{} / month",
                money(view.report.total_estimated_monthly_recovery, &ctx.currency)
            ),
            "Model estimate, not measured lift",
            None,
        ),
        kpi_card(
            "Coverage",
            &format!("{} of {} groups", covered.available, covered.total()),
            if covered.complete() {
                "All rule groups had input"
            } else {
                "Reporting input incomplete"
            },
            None,
        ),
    ]));

    let rows: Vec<Vec<Cell>> = view
        .report
        .findings
        .iter()
        .map(|finding| {
            vec![
                Cell::Status(
                    severity_label(finding.severity).to_string(),
                    status::severity(finding.severity.as_str()),
                ),
                Cell::Link(
                    finding.message.clone(),
                    format!(
                        "/console/audit/findings/{}",
                        finding_id(
                            finding.rule.as_str(),
                            finding.platform.as_str(),
                            &finding.subject
                        )
                    ),
                ),
                Cell::Mono(finding.subject.clone()),
                Cell::Text(estimate_label(finding, &ctx.currency)),
            ]
        })
        .collect();
    let empty = EmptyState {
        reason: "No findings in this snapshot. Five rule groups were checked where inputs were \
                 available."
            .to_string(),
        action: Some((
            "Run another audit".to_string(),
            "/console/audit".to_string(),
        )),
    };
    body.push_str(&data_table(
        Some("Findings"),
        &["Priority", "Finding", "Scope", "Estimate"],
        &rows,
        Some(empty),
        None,
    ));
    body.push_str(&format!(
        "<p class=\"table-note\">Total estimated monthly recovery: {}. Monthly amounts are model \
         estimates, not measured lift.</p>",
        money(view.report.total_estimated_monthly_recovery, &ctx.currency)
    ));
    let mut coverage_note = format!(
        "Rule groups with input available: {} of {}.",
        covered.available,
        covered.total()
    );
    if !covered.complete() {
        let _ = write!(
            coverage_note,
            " Missing input: {}.",
            covered.missing.join(", ")
        );
    }
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        super::html_escape(&coverage_note)
    ));
    body.push_str(&format!(
        "<p>{} {}</p>",
        action("Run another audit", "/console/audit", false),
        action(
            "Export the filtered report (CSV)",
            &format!(
                "/console/audit/export.csv?platform={}",
                view.platform.as_str()
            ),
            false
        )
    ));
    console_document(ctx, "Audit", "Audit", &body)
}

/// A rendered finding detail.
#[derive(Debug, Clone, Copy)]
pub struct FindingView<'a> {
    /// The snapshot the finding came from.
    pub snapshot: &'a Snapshot,
    /// The finding itself.
    pub finding: &'a Finding,
}

/// Render the finding detail page (`/console/audit/findings/{id}`).
pub fn render_finding_detail(ctx: &ConsoleCtx, view: &FindingView<'_>) -> String {
    let finding = view.finding;
    let mut body = format!("<h1>{}</h1>", super::html_escape(&finding.message));
    body.push_str(&format!(
        "<p>{} <code>{}</code></p>",
        status::chip(
            severity_label(finding.severity),
            status::severity(finding.severity.as_str())
        ),
        super::html_escape(&finding.subject)
    ));
    let estimate = estimate_label(finding, &ctx.currency);
    body.push_str(&format!(
        "<p><strong>Estimated monthly recovery:</strong> {}</p>",
        super::html_escape(&estimate)
    ));
    if !has_money_model(finding) {
        body.push_str(&format!(
            "<p class=\"table-note\">{}</p>",
            super::html_escape(missing_model_note(finding))
        ));
    }
    body.push_str(&panel(
        "The source evidence",
        &evidence_table(finding, &ctx.currency),
        None,
    ));
    body.push_str(&panel(
        "What to do next",
        "<p>In read-only mode, review the source evidence and export it. Test options appear only \
         when the action is supported and authorized.</p>",
        None,
    ));
    body.push_str(&format!(
        "<p class=\"table-note\">Affected scope: <code>{}</code> / Severity: {} / Platform: {} / \
         Captured {} / Snapshot digest: <code>{}</code></p>",
        super::html_escape(&finding.subject),
        severity_label(finding.severity),
        platform_label(finding.platform),
        utc_datetime(view.snapshot.fetched_at_unix_ms),
        super::html_escape(&apex_audit::snapshot_digest(view.snapshot))
    ));
    body.push_str(&format!(
        "<p>{} {}</p>",
        action("Run another audit", "/console/audit", false),
        action(
            "Export this report (CSV)",
            &format!(
                "/console/audit/export.csv?platform={}",
                view.snapshot.platform.as_str()
            ),
            false
        )
    ));
    console_document(ctx, "Finding", "Audit", &body)
}

/// The evidence table: one row per evidence entry, money for `*_micros`
/// values in the deployment currency, everything else as exact raw JSON.
fn evidence_table(finding: &Finding, currency: &str) -> String {
    let rows: Vec<Vec<Cell>> = match finding.evidence.as_object() {
        Some(entries) => entries
            .iter()
            .map(|(key, value)| {
                vec![
                    Cell::Mono(key.clone()),
                    Cell::Text(evidence_value(key, value, currency)),
                ]
            })
            .collect(),
        None => Vec::new(),
    };
    let empty = EmptyState {
        reason: "No evidence is carried by this finding.".to_string(),
        action: None,
    };
    data_table(None, &["Evidence", "Value"], &rows, Some(empty), None)
}

/// Render one evidence value. `*_micros` keys are amounts in the deployment
/// currency; eCPM keys are per-1000-impression prices; every other value is
/// shown as its exact JSON text.
fn evidence_value(key: &str, value: &serde_json::Value, currency: &str) -> String {
    if key.ends_with("_micros") {
        if let Some(micros) = value.as_i64() {
            if key.contains("ecpm") {
                return format!("{} eCPM", format::ecpm(micros, currency));
            }
            if key.starts_with("estimated_monthly_recovery") {
                return format!("{} / month", money(micros, currency));
            }
            return money(micros, currency);
        }
    }
    match value {
        serde_json::Value::Null => or_not_available("missing"),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Render the RFC 4180 CSV of one filtered report. Text columns go through
/// [`apex_ui::format::csv_text_field`] (spreadsheet formula triggers are
/// neutralised), the numeric micros column keeps its plain form, and every
/// line ends CRLF; dates stay UTC.
pub fn render_audit_csv(view: &AuditReportView<'_>, currency: &str) -> String {
    let headers = [
        "severity",
        "rule",
        "platform",
        "subject",
        "finding_id",
        "message",
        "recovery_model",
        "estimated_monthly_recovery_micros",
        "currency",
        "captured_utc",
        "snapshot_digest",
        "generated_by",
    ];
    // One reservation for the whole export: the old shape built a 12-entry
    // Vec and a joined String per finding, so a 20k-row export paid 260k
    // temporary allocations. Fields now append straight into the output.
    let mut out = String::with_capacity(256 + view.report.findings.len() * 256);
    for (index, field) in headers.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        format::push_csv_field(&mut out, field);
    }
    out.push_str("\r\n");
    let captured = utc_datetime(view.snapshot.fetched_at_unix_ms);
    for finding in &view.report.findings {
        format::push_csv_text_field(&mut out, severity_csv_label(finding.severity));
        out.push(',');
        format::push_csv_text_field(&mut out, finding.rule.as_str());
        out.push(',');
        format::push_csv_text_field(&mut out, finding.platform.as_str());
        out.push(',');
        format::push_csv_text_field(&mut out, &finding.subject);
        out.push(',');
        format::push_csv_text_field(
            &mut out,
            &finding_id(
                finding.rule.as_str(),
                finding.platform.as_str(),
                &finding.subject,
            ),
        );
        out.push(',');
        format::push_csv_text_field(&mut out, &finding.message);
        out.push(',');
        format::push_csv_text_field(&mut out, recovery_model(finding).unwrap_or("missing"));
        out.push(',');
        // Numeric: never prefixed, so the number stays machine-readable.
        format::push_csv_field(&mut out, &finding.estimated_monthly_recovery.to_string());
        out.push(',');
        format::push_csv_text_field(&mut out, currency);
        out.push(',');
        format::push_csv_text_field(&mut out, &captured);
        out.push(',');
        format::push_csv_text_field(&mut out, &view.report.snapshot_digest);
        out.push(',');
        format::push_csv_text_field(&mut out, view.report.generated_by);
        out.push_str("\r\n");
    }
    out
}

/// The lowercase CSV spelling of a severity (no per-row allocation).
fn severity_csv_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_audit::RuleId;

    use super::*;

    fn snapshot() -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "platform": "admob",
            "fetchedAtUnixMs": 1_791_965_400_000i64,
            "adUnits": [],
            "waterfall": [],
            "mediationGroups": [],
            "bidderAccounts": [{"network": "mintegral", "hasApexManagedAccount": true, "enabledInPlatform": true}],
            "countryRevenue": [{"country": "DE", "adUnitId": "rw_de_01", "revenue30d": 0, "impressions30d": 120_000}],
            "appAdsTxt": {"present": true, "lines": [], "expectedNetworks": []},
            "platformTotals30d": {"revenue30d": 0, "impressions30d": 0},
            "mediationTotals30d": {"revenue30d": 0, "impressions30d": 0}
        }))
        .expect("snapshot")
    }

    fn finding(
        rule: RuleId,
        severity: Severity,
        subject: &str,
        model: &str,
        recovery: i64,
    ) -> Finding {
        Finding {
            rule,
            severity,
            platform: PlatformId::Admob,
            subject: subject.to_string(),
            evidence: serde_json::json!({
                "floor_ecpm_micros": 4_000_000,
                "impressions_30d": 120_000,
                "recovery_model": model,
                "estimated_monthly_recovery_micros": recovery,
            }),
            message: format!("finding on {subject}"),
            estimated_monthly_recovery: recovery,
        }
    }

    fn report(findings: Vec<Finding>) -> AuditReport {
        let total = findings.iter().map(|f| f.estimated_monthly_recovery).sum();
        AuditReport {
            snapshot_digest: "ab".repeat(32),
            findings,
            total_estimated_monthly_recovery: total,
            generated_by: apex_audit::GENERATED_BY,
        }
    }

    #[test]
    fn finding_ids_are_stable_shaped_and_distinct() {
        let a = finding_id("stale_floor", "admob", "ad_unit:rw_de_01:country:DE");
        let b = finding_id("stale_floor", "admob", "ad_unit:rw_de_01:country:DE");
        assert_eq!(a, b, "deterministic");
        assert!(a.starts_with("f_"));
        assert_eq!(a.len(), 14);
        assert!(is_finding_id(&a));
        assert_ne!(
            a,
            finding_id("reporting_gap", "admob", "ad_unit:rw_de_01:country:DE")
        );
        assert_ne!(
            a,
            finding_id("stale_floor", "levelplay", "ad_unit:rw_de_01:country:DE")
        );
        assert_ne!(
            a,
            finding_id("stale_floor", "admob", "ad_unit:rw_de_02:country:DE")
        );
        for bad in [
            "",
            "f_",
            "f_zzzzzzzzzzzz",
            "aabbccddeeff",
            &format!("f_{}", "a".repeat(11)),
        ] {
            assert!(!is_finding_id(bad), "{bad}");
        }
    }

    #[test]
    fn estimate_labels_render_money_or_not_estimated() {
        let paid = finding(
            RuleId::StaleFloor,
            Severity::Warning,
            "s",
            "floor_blocks_fill",
            420_000_000,
        );
        assert_eq!(estimate_label(&paid, "USD"), "USD 420.00 / month");
        assert!(has_money_model(&paid));
        for model in NO_MONEY_MODELS {
            let none = finding(RuleId::AppAdsTxt, Severity::Warning, "s", model, 0);
            assert_eq!(estimate_label(&none, "USD"), "Not estimated");
            assert!(!has_money_model(&none));
            assert!(!missing_model_note(&none).is_empty());
        }
        let mut absent = finding(RuleId::AppAdsTxt, Severity::Warning, "s", "x", 0);
        absent.evidence = serde_json::json!({});
        assert_eq!(estimate_label(&absent, "EUR"), "Not estimated");
        assert_eq!(
            missing_model_note(&absent),
            "No recovery model applies to this finding."
        );
        // Unknown currency keeps the code form; money is never unlabelled.
        assert_eq!(estimate_label(&paid, "SEK"), "SEK 420.00 / month");
    }

    #[test]
    fn coverage_reads_the_snapshot_never_the_findings() {
        let full = snapshot();
        let covered = coverage(&full);
        assert_eq!(covered.available, 5);
        assert!(covered.complete());
        let mut bare = snapshot();
        bare.country_revenue = Vec::new();
        let covered = coverage(&bare);
        assert_eq!(covered.available, 3);
        assert_eq!(
            covered.missing,
            vec!["Stale floor", "Missing country floor"]
        );
        let mut bare = snapshot();
        bare.bidder_accounts = Vec::new();
        let covered = coverage(&bare);
        assert_eq!(covered.available, 4);
        assert_eq!(covered.missing, vec!["Disabled bidder"]);
    }

    #[test]
    fn report_page_shows_chips_money_and_detail_links() {
        let ctx = ConsoleCtx::new("Northstar Studio", "USD", true);
        let snapshot = snapshot();
        let critical = finding(
            RuleId::StaleFloor,
            Severity::Critical,
            "ad_unit:rw_de_01:country:DE",
            "floor_blocks_fill",
            420_000_000,
        );
        let warning = finding(
            RuleId::AppAdsTxt,
            Severity::Warning,
            "app_ads_txt:networks",
            "not_applicable",
            0,
        );
        let report = report(vec![critical.clone(), warning]);
        let html = render_audit_report(
            &ctx,
            &AuditReportView {
                platform: PlatformId::Admob,
                snapshot: &snapshot,
                report: &report,
            },
        );
        assert!(html.contains("AdMob audit"));
        assert!(html.contains(">Snapshot<"));
        assert!(html.contains("Northstar Studio / Captured 14 Oct 2026, 08:10 UTC / 30-day input"));
        assert!(html.contains("Estimated opportunity"));
        assert!(html.contains("USD 420.00 / month"));
        assert!(html.contains("Model estimate, not measured lift"));
        assert!(html.contains("Coverage"));
        assert!(html.contains("5 of 5 groups"));
        assert!(html.contains(">Critical<"));
        assert!(html.contains(">Warning<"));
        assert!(html.contains("USD 420.00 / month"));
        assert!(html.contains("Not estimated"));
        assert!(html.contains(&format!(
            "/console/audit/findings/{}",
            finding_id("stale_floor", "admob", "ad_unit:rw_de_01:country:DE")
        )));
        assert!(html.contains("Total estimated monthly recovery: USD 420.00."));
        assert!(html.contains("Rule groups with input available: 5 of 5."));
        assert!(html.contains("Export the filtered report (CSV)"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn report_page_empty_state_keeps_the_frozen_copy() {
        let ctx = ConsoleCtx::default();
        let snapshot = snapshot();
        let report = report(vec![]);
        let html = render_audit_report(
            &ctx,
            &AuditReportView {
                platform: PlatformId::LevelPlay,
                snapshot: &snapshot,
                report: &report,
            },
        );
        assert!(html.contains(
            "No findings in this snapshot. Five rule groups were checked where inputs were available."
        ));
        assert!(html.contains(">Run another audit<"));
    }

    #[test]
    fn report_page_lists_missing_inputs() {
        let ctx = ConsoleCtx::default();
        let mut snapshot = snapshot();
        snapshot.bidder_accounts = Vec::new();
        let report = report(vec![]);
        let html = render_audit_report(
            &ctx,
            &AuditReportView {
                platform: PlatformId::Admob,
                snapshot: &snapshot,
                report: &report,
            },
        );
        assert!(html.contains("Coverage"));
        assert!(html.contains("4 of 5 groups"));
        assert!(html.contains("Reporting input incomplete"));
        assert!(html.contains("Missing input: Disabled bidder."));
    }

    #[test]
    fn form_page_renders_every_state_with_safe_preserved_values() {
        let ctx = ConsoleCtx::default();
        let plain = render_audit_form(&ctx, None);
        assert!(plain.contains("Review an audit snapshot"));
        assert!(plain.contains("The report shows its source, scope and capture time."));
        assert!(plain.contains(">View audit</button>"));
        for (name, label) in PLATFORM_NAMES {
            assert!(plain.contains(&format!("<option value=\"{name}\">{label}</option>")));
        }
        assert!(plain.contains("What is checked"));
        assert!(plain.contains("<li>Floors and bidder settings</li>"));

        let unknown = AuditFormProblem::UnknownPlatform("<script>\"x".to_string());
        let html = render_audit_form(&ctx, Some(&unknown));
        assert_eq!(form_problem_status(&unknown), 400);
        assert!(html.contains("Check the request."));
        assert!(html.contains("Received platform: <code>&lt;script&gt;&quot;x</code>"));
        assert!(!html.contains("<script>\"x"));
        assert!(html.contains("aria-invalid=\"true\""));

        let missing = AuditFormProblem::NoSnapshot("admob".to_string());
        let html = render_audit_form(&ctx, Some(&missing));
        assert_eq!(form_problem_status(&missing), 404);
        assert!(html.contains("No audit snapshot is available."));
        assert!(html.contains("This platform has no report in the current workspace."));
        assert!(html.contains("<option value=\"admob\" selected>AdMob</option>"));
        assert!(html.contains("Contact support"));

        let malformed = AuditFormProblem::BadParameters("<img src=x>".to_string());
        let html = render_audit_form(&ctx, Some(&malformed));
        assert!(html.contains("&lt;img src=x&gt;"));
        assert!(!html.contains("<img src=x>"));
    }

    #[test]
    fn finding_detail_shows_evidence_money_and_frozen_notes() {
        let ctx = ConsoleCtx::new("Northstar Studio", "USD", true);
        let snapshot = snapshot();
        let stale = finding(
            RuleId::StaleFloor,
            Severity::Warning,
            "ad_unit:rw_de_01:country:DE",
            "floor_blocks_fill",
            420_000_000,
        );
        let html = render_finding_detail(
            &ctx,
            &FindingView {
                snapshot: &snapshot,
                finding: &stale,
            },
        );
        assert!(html.contains("<h1>finding on ad_unit:rw_de_01:country:DE</h1>"));
        assert!(html.contains("Estimated monthly recovery:</strong> USD 420.00 / month"));
        assert!(html.contains("The source evidence"));
        assert!(html.contains("floor_ecpm_micros"));
        assert!(html.contains("$4.00 eCPM"));
        assert!(html.contains("Captured 14 Oct 2026, 08:10 UTC"));
        assert!(html.contains("Snapshot digest: <code>"));
        assert!(html.contains("Run another audit"));
        assert!(
            html.contains("Test options appear only when the action is supported and authorized.")
        );

        let no_money = finding(
            RuleId::DisabledBidderWithAccount,
            Severity::Info,
            "network:mintegral",
            "insufficient_data",
            0,
        );
        let html = render_finding_detail(
            &ctx,
            &FindingView {
                snapshot: &snapshot,
                finding: &no_money,
            },
        );
        assert!(html.contains("Estimated monthly recovery:</strong> Not estimated"));
        assert!(html.contains("The snapshot does not carry enough data for a monthly estimate."));
    }

    #[test]
    fn finding_detail_escapes_hostile_subjects_and_messages() {
        let ctx = ConsoleCtx::default();
        let snapshot = snapshot();
        let mut finding = finding(
            RuleId::AppAdsTxt,
            Severity::Warning,
            "</code><script>alert(1)</script>",
            "not_applicable",
            0,
        );
        finding.message = "\" onmouseover=\"alert(1)".to_string();
        let html = render_finding_detail(
            &ctx,
            &FindingView {
                snapshot: &snapshot,
                finding: &finding,
            },
        );
        assert!(!html.contains("<script>alert(1)"));
        assert!(!html.contains("onmouseover=\"alert(1)\""));
        assert!(html.contains("&lt;/code&gt;&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn csv_is_rfc4180_and_keeps_utc_dates() {
        let ctx = ConsoleCtx::default();
        let snapshot = snapshot();
        let mut finding = finding(
            RuleId::StaleFloor,
            Severity::Critical,
            "ad_unit:rw_de_01:country:DE",
            "floor_blocks_fill",
            420_000_000,
        );
        finding.message = "totals disagree, check \"floor\" now".to_string();
        let report = report(vec![finding]);
        let csv = render_audit_csv(
            &AuditReportView {
                platform: PlatformId::Admob,
                snapshot: &snapshot,
                report: &report,
            },
            &ctx.currency,
        );
        assert!(csv.starts_with("severity,rule,platform,subject,finding_id,message,"));
        assert!(csv.contains("\"totals disagree, check \"\"floor\"\" now\""));
        assert!(csv.contains("14 Oct 2026, 08:10 UTC"));
        assert!(csv.contains(&format!(
            "f_{}",
            &finding_id("stale_floor", "admob", "ad_unit:rw_de_01:country:DE")[2..]
        )));
        assert!(csv.contains("420000000"));
        assert!(csv.ends_with("\r\n"));
        assert_eq!(csv.lines().count(), 2);
    }

    /// The export is linear at the console's 20k-row reference size and
    /// reserves its buffer once (no per-row `Vec`/`join` temporaries).
    #[test]
    fn csv_renders_twenty_thousand_findings_linearly() {
        let snapshot = snapshot();
        let render = |count: usize| {
            let findings = (0..count)
                .map(|index| {
                    finding(
                        RuleId::StaleFloor,
                        Severity::Warning,
                        &format!("unit{index}"),
                        "floor_blocks_fill",
                        1,
                    )
                })
                .collect();
            let report = report(findings);
            render_audit_csv(
                &AuditReportView {
                    platform: PlatformId::Admob,
                    snapshot: &snapshot,
                    report: &report,
                },
                "USD",
            )
        };
        let five = render(5_000);
        let ten = render(10_000);
        let twenty = render(20_000);
        assert_eq!(twenty.lines().count(), 20_001, "one line per finding");
        let per_row = |csv: &str, rows: usize| csv.len() / rows;
        assert!(
            per_row(&ten, 10_000) <= per_row(&five, 5_000) + 8,
            "5k -> 10k rows is linear per row"
        );
        assert!(
            per_row(&twenty, 20_000) <= per_row(&ten, 10_000) + 8,
            "10k -> 20k rows is linear per row"
        );
        assert!(
            twenty.capacity() >= twenty.len(),
            "the reservation covers the export"
        );
        assert!(
            twenty.capacity() <= 256 + 20_000 * 256 + 4096,
            "one reserve sized from the row count (capacity {}, len {})",
            twenty.capacity(),
            twenty.len()
        );
    }

    #[test]
    fn csv_neutralizes_formula_text_cells_and_keeps_numbers() {
        let ctx = ConsoleCtx::default();
        let snapshot = snapshot();
        let mut hostile = finding(
            RuleId::StaleFloor,
            Severity::Critical,
            "=cmd|' /C calc'!A0",
            "not_applicable",
            -120,
        );
        hostile.message = "@user".to_string();
        hostile.evidence = serde_json::json!({
            "recovery_model": "+SUM(1,1)",
            "estimated_monthly_recovery_micros": -120,
        });
        let mut report = report(vec![hostile]);
        report.snapshot_digest = "-abc".to_string();
        let csv = render_audit_csv(
            &AuditReportView {
                platform: PlatformId::Admob,
                snapshot: &snapshot,
                report: &report,
            },
            &ctx.currency,
        );
        // The formula triggers are prefixed with an apostrophe; the +SUM
        // value is quoted because it contains a comma.
        assert!(
            csv.contains("'=cmd|' /C calc'!A0"),
            "subject is neutralised: {csv}"
        );
        assert!(csv.contains(",'@user,"), "message is neutralised: {csv}");
        assert!(
            csv.contains("\"'+SUM(1,1)\""),
            "recovery model is neutralised and quoted: {csv}"
        );
        assert!(csv.contains(",'-abc,"), "digest is neutralised: {csv}");
        // The numeric micros column is untouched.
        assert!(csv.contains(",-120,USD,"), "the number stays plain: {csv}");
        assert!(
            !csv.contains("'-120"),
            "a plain negative number is never apostrophe-prefixed: {csv}"
        );
        // Raw bytes: every line ends CRLF and there is no lone LF.
        assert!(csv.ends_with("\r\n"));
        for (index, byte) in csv.bytes().enumerate() {
            if byte == b'\n' {
                assert!(
                    index > 0 && csv.as_bytes()[index - 1] == b'\r',
                    "lone LF at byte {index}"
                );
            }
        }
        assert_eq!(csv.matches("\r\n").count(), 2, "header plus one finding");
    }

    #[test]
    fn platform_and_severity_labels_are_pinned() {
        assert_eq!(platform_label(PlatformId::Admob), "AdMob");
        assert_eq!(platform_label(PlatformId::AppLovinMax), "AppLovin MAX");
        assert_eq!(platform_label(PlatformId::LevelPlay), "LevelPlay");
        assert_eq!(severity_label(Severity::Critical), "Critical");
        assert_eq!(severity_label(Severity::Warning), "Warning");
        assert_eq!(severity_label(Severity::Info), "Info");
    }
}
