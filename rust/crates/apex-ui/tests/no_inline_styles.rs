#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The no-inlining guarantee: every public fragment and every page
//! composition the design system can render separates structure from style.
//!
//! No component emits a `style="` attribute (SVG presentation attributes like
//! `fill=` are geometry, not CSS) and no component emits a `<style>` block of
//! its own. The only stylesheet is the single global `<style>` block the
//! shell embeds, so a new surface built from these components cannot
//! introduce a design deviation through local style overrides.

use apex_ui::charts::{bar_chart, line_chart, sparkline, ChartColor, ChartSeries, ChartSpec};
use apex_ui::components::{
    action, action_button, breadcrumb, checkbox, csrf_hidden, data_table, details, field,
    filter_bar, form_error_summary, input, kpi_card, kpi_row, notice, pagination, panel, select,
    split, status_pair, tabs, textarea, Cell, EmptyState,
};
use apex_ui::shell::{page, Area, PageCtx, Theme};
use apex_ui::status::{chip, StatusKind};
use apex_ui::theme::theme_form;
use apex_ui::{logo, tokens};

/// Every public fragment, rendered with representative data, into one string.
fn all_components() -> String {
    let mut html = String::new();
    html.push_str("<h1>Design system sweep</h1>");

    html.push_str(&kpi_row(&[
        kpi_card(
            "Estimated opportunity",
            "$540.00",
            "Model estimate, not measured lift",
            Some("/console/audit"),
        ),
        kpi_card("Verified records", "24", "Signed history", None),
    ]));

    html.push_str(&notice(
        StatusKind::Neutral,
        "Reporting is delayed",
        "Two days are not final.",
        Some(("Open audit", "/console/audit")),
    ));
    html.push_str(&notice(
        StatusKind::Attention,
        "Attention",
        "Needs review",
        None,
    ));
    html.push_str(&notice(
        StatusKind::Failure,
        "Failed",
        "The read failed",
        None,
    ));

    html.push_str(&panel(
        "Next steps",
        "<p>Review a reporting gap.</p>",
        Some("Open audit"),
    ));
    html.push_str(&split("<p>main</p>", "<p>side</p>"));

    let rows = vec![
        vec![
            Cell::Text("Reporting totals disagree".to_string()),
            Cell::Mono("exp_0247".to_string()),
            Cell::Num("1,250.00".to_string()),
            Cell::Status("Verified".to_string(), StatusKind::Verified),
            Cell::Link("Open".to_string(), "/console/audit".to_string()),
            Cell::Muted("snapshot".to_string()),
        ],
        vec![Cell::Text("Reporting totals disagree".to_string())],
    ];
    html.push_str(&data_table(
        Some("Findings"),
        &["Finding", "ID", "Amount", "Status", "Action", "Source"],
        &rows,
        None,
        Some("2 findings / 14 Oct 2026, 08:10 UTC"),
    ));
    html.push_str(&data_table(
        Some("Findings"),
        &["Finding", "Status"],
        &[],
        Some(EmptyState {
            reason: "No findings in this snapshot.".to_string(),
            action: Some(("Open audit".to_string(), "/console/audit".to_string())),
        }),
        None,
    ));
    html.push_str(&data_table(None, &["A"], &[], None, None));

    html.push_str(&field(
        "platform",
        "Platform",
        Some("Choose the account that produced this report."),
        Some("Choose a supported platform."),
        &select(
            "platform",
            "platform",
            &[
                ("applovin_max".to_string(), "AppLovin MAX".to_string()),
                ("levelplay".to_string(), "LevelPlay".to_string()),
            ],
            Some("levelplay"),
        ),
    ));
    html.push_str(&field(
        "search",
        "Search",
        None,
        None,
        &input("search", "search", "text", Some("northstar"), false, None),
    ));
    html.push_str(&input(
        "email",
        "work_email",
        "email",
        Some("you@studio.com"),
        true,
        Some("email"),
    ));
    html.push_str(&textarea("notes", "notes", Some("hello"), 6, Some(2000)));
    html.push_str(&checkbox(
        "finalized",
        "finalized",
        "Finalized data only",
        true,
    ));
    html.push_str(&form_error_summary(&[
        (
            "platform".to_string(),
            "Choose a supported platform.".to_string(),
        ),
        ("skipped".to_string(), "  ".to_string()),
    ]));

    html.push_str(&details("Technical details", "<p>digest</p>", true));
    html.push_str(&filter_bar(
        &[("platform".to_string(), "applovin_max".to_string())],
        "/console/audit",
        "1 result",
    ));
    html.push_str(&tabs(&[
        (
            "All states".to_string(),
            "/console/experiments".to_string(),
            true,
        ),
        (
            "Finalized data only".to_string(),
            "/console/experiments?final=1".to_string(),
            false,
        ),
    ]));
    html.push_str(&pagination(
        Some("/console/ledger?page=1"),
        Some("/console/ledger?page=3"),
        "Page 2 of 4 / 24 records",
    ));
    html.push_str(&breadcrumb(&["Workspace".to_string(), "Audit".to_string()]));
    html.push_str(&action("View audit", "/console/audit", true));
    html.push_str(&action("Export", "/console/audit/export.csv", false));
    html.push_str(&action_button("Approve", "decision", "approve", true));
    html.push_str(&action_button("Reject", "decision", "reject", false));
    html.push_str(&csrf_hidden("token-123"));
    html.push_str(&status_pair("Read access", StatusKind::Verified));

    for kind in [
        StatusKind::Neutral,
        StatusKind::ReadOnly,
        StatusKind::Verified,
        StatusKind::Attention,
        StatusKind::Failure,
        StatusKind::Unavailable,
        StatusKind::InProgress,
        StatusKind::Completed,
        StatusKind::Queued,
    ] {
        html.push_str(&chip("Status", kind));
    }

    let spec = ChartSpec {
        title: "Observed lift".to_string(),
        desc: "Control and test over the analysis window".to_string(),
        width: 640,
        height: 240,
        y_label: "Revenue / allocated share".to_string(),
        x_labels: vec![
            "01 Oct".to_string(),
            "02 Oct".to_string(),
            "03 Oct".to_string(),
        ],
        zero_line: true,
    };
    html.push_str(&line_chart(
        &spec,
        &[
            ChartSeries {
                label: "Control".to_string(),
                color: ChartColor::Control,
                points: vec![Some(1.0), Some(1.1), Some(1.2)],
            },
            ChartSeries {
                label: "Test".to_string(),
                color: ChartColor::Test,
                points: vec![Some(1.0), None, Some(1.4)],
            },
        ],
        0.0,
        2.0,
    ));
    html.push_str(&bar_chart(
        &spec,
        &[Some(4.0), None, Some(-2.0)],
        ChartColor::Ink,
    ));
    html.push_str(&sparkline(&[Some(1.0), None, Some(3.0), Some(4.0)]));

    html.push_str(&logo::mark(20, tokens::INK));
    html.push_str(&logo::lockup("#ffffff", 20));
    html.push_str(&logo::lockup(tokens::INK, 32));
    html.push_str(&theme_form("/console/settings", Theme::Auto));
    html
}

/// One page context per area: the three app areas and marketing.
fn contexts() -> [PageCtx; 4] {
    [
        PageCtx {
            title: "Audit".to_string(),
            area: Area::PublisherConsole,
            publisher_label: Some("Northstar Studio".to_string()),
            breadcrumb: vec!["Workspace".to_string(), "Audit".to_string()],
            read_only: true,
            theme: Theme::Auto,
            help_href: None,
            marketing_header: false,
        },
        PageCtx {
            title: "Customers".to_string(),
            area: Area::OperatorPlane,
            publisher_label: None,
            breadcrumb: vec!["Operator".to_string(), "Customers".to_string()],
            read_only: false,
            theme: Theme::Dark,
            help_href: None,
            marketing_header: false,
        },
        PageCtx {
            title: "Leads".to_string(),
            area: Area::SalesPlane,
            publisher_label: None,
            breadcrumb: vec!["Sales".to_string(), "Leads".to_string()],
            read_only: true,
            theme: Theme::Light,
            help_href: None,
            marketing_header: false,
        },
        PageCtx {
            title: "Pricing".to_string(),
            area: Area::Marketing,
            publisher_label: None,
            breadcrumb: Vec::new(),
            read_only: false,
            theme: Theme::Light,
            help_href: None,
            marketing_header: true,
        },
    ]
}

#[test]
fn components_and_pages_never_inline_styles() {
    let components = all_components();
    assert!(
        components.contains("class=\""),
        "the component sweep rendered nothing"
    );
    assert!(
        !components.contains("style=\""),
        "a component emits an inline style attribute"
    );
    assert!(
        !components.contains("<style"),
        "a component emits its own <style> block"
    );

    let mut combined = components.clone();
    let mut pages = 0usize;
    for ctx in contexts() {
        combined.push_str(&page(&ctx, &components));
        pages += 1;
    }
    assert!(
        !combined.contains("style=\""),
        "a page composition emits an inline style attribute"
    );
    // The only style block anywhere is the shell's one global stylesheet,
    // once per rendered page.
    assert_eq!(
        combined.matches("<style").count(),
        pages,
        "pages must embed exactly the one global stylesheet"
    );
    assert_eq!(combined.matches("</style>").count(), pages);
}
