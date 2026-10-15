//! The overview page (`/`): the account at a glance.
//!
//! The KPI cards and the next-steps table are driven by the real audit and
//! ledger sources: the latest snapshot across the three platforms and the
//! verified record count of every configured publisher ledger. When neither
//! source has data yet, the page renders the first-run introduction instead
//! of fabricated zeroes.

use apex_ui::components::{action, data_table, kpi_card, kpi_row, notice, panel, Cell, EmptyState};
use apex_ui::format::{or_not_available, utc_datetime};
use apex_ui::status::StatusKind;

use super::{console_document, ConsoleCtx};

/// The latest audit snapshot across platforms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestAudit {
    /// Human platform label (`AdMob`, `AppLovin MAX`, `LevelPlay`).
    pub platform_label: String,
    /// When the snapshot was captured (UTC).
    pub captured_at_unix_ms: i64,
    /// Total findings in the report.
    pub findings: usize,
    /// Severity split.
    pub critical: usize,
    /// Severity split.
    pub warning: usize,
    /// Severity split.
    pub info: usize,
}

/// The verified-history state of the configured publisher ledgers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerState {
    /// No publisher ledger is configured in this deployment.
    NotConfigured,
    /// Every configured ledger verified; `records` is the total.
    Verified {
        /// Verified records across all configured publishers.
        records: u64,
        /// Publishers whose ledger failed to read (0 when clean).
        unavailable: usize,
    },
    /// No configured ledger could be read.
    Unavailable {
        /// Short human reason (never internal detail).
        reason: String,
    },
}

/// Everything the overview renders from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewData {
    /// The newest snapshot across platforms, if any snapshot exists.
    pub latest_audit: Option<LatestAudit>,
    /// The verified-history state.
    pub ledger: LedgerState,
}

/// Render the overview page.
pub fn render(ctx: &ConsoleCtx, data: &OverviewData) -> String {
    let mut body = String::from("<h1>Your account at a glance</h1>");
    body.push_str(&notice(
        StatusKind::Neutral,
        "Read-only mode.",
        "No platform settings are changed here.",
        None,
    ));
    let first_run = data.latest_audit.is_none() && data.ledger == LedgerState::NotConfigured;
    if first_run {
        body.push_str(&first_run_panel());
    } else {
        body.push_str(&kpis(ctx, data));
        body.push_str("<h2 class=\"section-heading\">Next steps</h2>");
        body.push_str(&next_steps(data));
    }
    console_document(ctx, "Overview", "Overview", &body)
}

fn kpis(ctx: &ConsoleCtx, data: &OverviewData) -> String {
    let _ = ctx;
    let audit_card = match &data.latest_audit {
        Some(latest) => kpi_card(
            "Latest audit",
            &format!("{} {}", latest.findings, plural(latest.findings, "finding")),
            &format!(
                "Snapshot / {} / {}",
                latest.platform_label,
                utc_datetime(latest.captured_at_unix_ms)
            ),
            Some("/console/audit"),
        ),
        None => kpi_card(
            "Latest audit",
            &or_not_available("no snapshot"),
            "Open the audit to arrange the first read-only run.",
            Some("/console/audit"),
        ),
    };
    let critical_card = match &data.latest_audit {
        Some(latest) => kpi_card(
            "Critical findings",
            &latest.critical.to_string(),
            if latest.critical > 0 {
                "Review before considering changes"
            } else {
                "No critical findings in the latest snapshot"
            },
            Some("/console/audit"),
        ),
        None => kpi_card(
            "Critical findings",
            &or_not_available("no snapshot"),
            "Critical data issues appear here once an audit runs.",
            None,
        ),
    };
    let ledger_card = match &data.ledger {
        LedgerState::NotConfigured => kpi_card(
            "Verified records",
            &or_not_available("no publisher ledger configured"),
            "The signed change history appears here once a ledger is arranged.",
            Some("/console/ledger"),
        ),
        LedgerState::Verified {
            records,
            unavailable,
        } => kpi_card(
            "Verified records",
            &records.to_string(),
            match (*records, *unavailable) {
                (0, _) => "Publisher history is empty",
                (_, 0) => "Publisher history is available",
                _ => "Some histories are unavailable",
            },
            Some("/console/ledger"),
        ),
        LedgerState::Unavailable { reason } => kpi_card(
            "Verified records",
            &or_not_available(reason),
            "Open the change log for the access details.",
            Some("/console/ledger"),
        ),
    };
    kpi_row(&[audit_card, critical_card, ledger_card])
}

fn next_steps(data: &OverviewData) -> String {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    match &data.latest_audit {
        Some(latest) if latest.findings > 0 => rows.push(vec![
            Cell::Text("Review a reporting gap".to_string()),
            Cell::Status("Needs attention".to_string(), StatusKind::Attention),
            Cell::Link("Open audit".to_string(), "/console/audit".to_string()),
        ]),
        Some(_) => rows.push(vec![
            Cell::Text("Review the latest audit snapshot".to_string()),
            Cell::Status("Verified".to_string(), StatusKind::Verified),
            Cell::Link("Open audit".to_string(), "/console/audit".to_string()),
        ]),
        None => {}
    }
    match &data.ledger {
        LedgerState::NotConfigured => {}
        LedgerState::Verified { .. } => rows.push(vec![
            Cell::Text("Inspect the change history".to_string()),
            Cell::Status("Read-only".to_string(), StatusKind::ReadOnly),
            Cell::Link("Open change log".to_string(), "/console/ledger".to_string()),
        ]),
        LedgerState::Unavailable { .. } => rows.push(vec![
            Cell::Text("Inspect the change history".to_string()),
            Cell::Status("Needs attention".to_string(), StatusKind::Attention),
            Cell::Link("Open change log".to_string(), "/console/ledger".to_string()),
        ]),
    }
    let empty = EmptyState {
        reason: "No next steps are available yet.".to_string(),
        action: Some(("Open audit".to_string(), "/console/audit".to_string())),
    };
    data_table(
        None,
        &["Task", "Status", "Action"],
        &rows,
        Some(empty),
        None,
    )
}

fn first_run_panel() -> String {
    let body = "<p>This workspace has no audit snapshot and no verified history yet. A \
                read-only audit reads your platform configuration and reports and never changes a \
                setting. Ask your account team to arrange the first audit; this page then shows \
                the findings, the estimates and your verified change history.</p>\
                <p>What the audit checks:</p>\
                <ul><li>Floors and bidder settings</li>\
                <li>Country coverage</li>\
                <li>Revenue reporting gaps</li>\
                <li>app-ads.txt</li></ul>";
    let mut out = panel("No audit snapshot is available.", body, None);
    out.push_str(&action("Open audit", "/console/audit", true));
    out
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::new("Northstar Studio", "USD", true)
    }

    fn populated() -> OverviewData {
        OverviewData {
            latest_audit: Some(LatestAudit {
                platform_label: "AdMob".to_string(),
                captured_at_unix_ms: 1_791_965_400_000,
                findings: 5,
                critical: 1,
                warning: 3,
                info: 1,
            }),
            ledger: LedgerState::Verified {
                records: 24,
                unavailable: 0,
            },
        }
    }

    #[test]
    fn populated_page_shows_the_documented_cards_and_steps() {
        let html = render(&ctx(), &populated());
        assert!(html.contains("<h1>Your account at a glance</h1>"));
        assert!(html.contains("Read-only mode."));
        assert!(html.contains("No platform settings are changed here."));
        assert!(html.contains(">Latest audit<"));
        assert!(html.contains(">5 findings<"));
        assert!(html.contains("Snapshot / AdMob / 14 Oct 2026, 08:10 UTC"));
        assert!(html.contains(">Critical findings<"));
        assert!(html.contains("Review before considering changes"));
        assert!(html.contains(">Verified records<"));
        assert!(html.contains("Publisher history is available"));
        assert!(html.contains(">Next steps<"));
        assert!(html.contains(">Review a reporting gap<"));
        assert!(html.contains(">Needs attention<"));
        assert!(html.contains(">Open audit<"));
        assert!(html.contains(">Inspect the change history<"));
        assert!(html.contains(">Read-only<"));
        assert!(html.contains(">Open change log<"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn one_finding_is_singular() {
        let mut data = populated();
        data.latest_audit = Some(LatestAudit {
            findings: 1,
            critical: 0,
            warning: 0,
            info: 1,
            ..data.latest_audit.expect("latest")
        });
        let html = render(&ctx(), &data);
        assert!(html.contains(">1 finding<"));
        assert!(html.contains("No critical findings in the latest snapshot"));
        assert!(html.contains(">Review a reporting gap<"));
    }

    #[test]
    fn first_run_state_explains_the_arrangement() {
        let data = OverviewData {
            latest_audit: None,
            ledger: LedgerState::NotConfigured,
        };
        let html = render(&ctx(), &data);
        assert!(html.contains("No audit snapshot is available."));
        assert!(html.contains("Ask your account team to arrange the first audit"));
        assert!(html.contains("<li>app-ads.txt</li>"));
        assert!(html.contains(">Open audit<"));
        assert!(!html.contains("class=\"kpi\""), "no fabricated KPI cards");
        assert!(!html.contains("Next steps"));
    }

    #[test]
    fn missing_snapshot_with_a_ledger_shows_honest_cards() {
        let data = OverviewData {
            latest_audit: None,
            ledger: LedgerState::Verified {
                records: 24,
                unavailable: 0,
            },
        };
        let html = render(&ctx(), &data);
        assert!(html.contains("Not available (no snapshot)"));
        assert!(html.contains(">24<"));
        assert!(html.contains(">Inspect the change history<"));
        assert!(!html.contains(">Review a reporting gap<"));
    }

    #[test]
    fn unavailable_and_empty_ledgers_render_honestly() {
        let mut data = populated();
        data.ledger = LedgerState::Unavailable {
            reason: "history could not be verified".to_string(),
        };
        let html = render(&ctx(), &data);
        assert!(html.contains("Not available (history could not be verified)"));
        assert!(html.contains("Open the change log for the access details."));
        data.ledger = LedgerState::Verified {
            records: 0,
            unavailable: 0,
        };
        let html = render(&ctx(), &data);
        assert!(html.contains("Publisher history is empty"));
    }

    #[test]
    fn partially_unavailable_history_is_labeled() {
        let mut data = populated();
        data.ledger = LedgerState::Verified {
            records: 4,
            unavailable: 1,
        };
        let html = render(&ctx(), &data);
        assert!(html.contains("Some histories are unavailable"));
    }
}
