//! Integration coverage for the frozen public API: this file compiles
//! against `apex_ui` exactly the way sibling workstreams will, and renders a
//! complete console page from the components.

use apex_ui::charts::{bar_chart, line_chart, sparkline, ChartColor, ChartSeries, ChartSpec};
use apex_ui::components::{
    action, action_button, breadcrumb, checkbox, csrf_hidden, data_table, details, field,
    filter_bar, form_error_summary, input, kpi_card, notice, pagination, panel, select,
    status_pair, tabs, textarea, Cell, EmptyState,
};
use apex_ui::format::{
    date_short, delta, ecpm, micros_exact, money, money_fixed, or_not_available, percent,
    utc_datetime,
};
use apex_ui::shell::{Area, PageCtx, Theme};
use apex_ui::status::{severity, StatusKind};
use apex_ui::tokens::console_css;
use apex_ui::{escape, logo, theme};

#[test]
fn escaped_helpers_agree_across_contexts() {
    let hostile = "\" onmouseover=\"alert('x')<b>&";
    assert!(escape::attr(hostile).contains("&quot;"));
    assert!(escape::html_escape(hostile).contains("&#39;"));
    assert!(!escape::attr(hostile).contains("\" onmouseover="));
}

#[test]
fn a_full_console_page_renders_from_the_components() {
    let rows = vec![vec![
        Cell::Text("Reporting totals disagree".to_string()),
        Cell::Status("Needs attention".to_string(), severity("warning")),
        Cell::Num(money_fixed(420_000_000, "USD")),
        Cell::Link("Open audit".to_string(), "/console/audit".to_string()),
        Cell::Mono("exp_0247".to_string()),
    ]];
    let mut body = String::new();
    body.push_str("<h1>MAX audit</h1>");
    body.push_str(&notice(
        StatusKind::Neutral,
        "Reporting is delayed",
        "Two days are not final. The decision is waiting for complete data.",
        None,
    ));
    body.push_str(&kpi_card(
        "Estimated opportunity",
        &money(540_000_000, "USD"),
        "Model estimate, not measured lift",
        None,
    ));
    body.push_str(&data_table(
        Some("Findings"),
        &["Finding", "Priority", "Estimate", "Action", "Reference"],
        &rows,
        None,
        Some("5 findings / 14 Oct 2026, 08:10 UTC"),
    ));
    body.push_str(&data_table(
        Some("Findings"),
        &["Finding"],
        &[],
        Some(EmptyState {
            reason: "No findings in this snapshot.".to_string(),
            action: None,
        }),
        None,
    ));
    body.push_str(&field(
        "platform",
        "Platform",
        Some("Choose the account that produced this report."),
        None,
        &select(
            "platform",
            "platform",
            &[("applovin_max".to_string(), "AppLovin MAX".to_string())],
            Some("applovin_max"),
        ),
    ));
    body.push_str(&filter_bar(
        &[("platform".to_string(), "applovin_max".to_string())],
        "/console/audit",
        "1 result",
    ));
    body.push_str(&tabs(&[(
        "All states".to_string(),
        "/console/audit".to_string(),
        true,
    )]));
    body.push_str(&breadcrumb(&["Workspace".to_string(), "Audit".to_string()]));
    body.push_str(&pagination(
        None,
        Some("/console/audit?page=2"),
        "Page 1 of 2",
    ));
    body.push_str(&details("Technical details", "<p>digest</p>", false));
    body.push_str(&panel(
        "Next steps",
        "<p>Review a reporting gap.</p>",
        Some("Open audit"),
    ));
    body.push_str(&status_pair("Read access", StatusKind::Verified));
    body.push_str(&action("View audit", "/console/audit", true));
    body.push_str(&action_button(
        "Approve this test",
        "decision",
        "approve",
        true,
    ));
    body.push_str(&csrf_hidden("token-123"));
    body.push_str(&textarea("note", "note", None, 4, Some(500)));
    body.push_str(&checkbox("final", "final", "Finalized data only", true));
    body.push_str(&form_error_summary(&[(
        "platform".to_string(),
        "Choose a supported platform.".to_string(),
    )]));
    body.push_str(&input(
        "email",
        "email",
        "email",
        None,
        false,
        Some("email"),
    ));

    let ctx = PageCtx {
        title: "Audit".to_string(),
        area: Area::PublisherConsole,
        publisher_label: Some("Northstar Studio".to_string()),
        breadcrumb: vec!["Workspace".to_string(), "Audit".to_string()],
        read_only: true,
        theme: Theme::Auto,
        help_href: None,
        marketing_header: false,
    };
    let page = apex_ui::shell::page(&ctx, &body);
    assert!(page.contains("<title>Audit \u{2014} Apex console</title>"));
    assert!(!page.contains("<script"));
    assert!(page.contains("Read-only mode. No platform settings are changed here."));
    assert!(page.contains(console_css()));
}

#[test]
fn charts_render_missing_points_as_gaps_through_the_public_api() {
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
    let series = vec![
        ChartSeries {
            label: "Control".to_string(),
            color: ChartColor::Control,
            points: vec![Some(1.0), Some(1.1), Some(1.0)],
        },
        ChartSeries {
            label: "Test".to_string(),
            color: ChartColor::Test,
            points: vec![Some(1.0), None, Some(1.4)],
        },
    ];
    let html = line_chart(&spec, &series, 0.0, 2.0);
    assert!(html.contains("<title>Observed lift</title>"));
    assert!(html.contains("<desc>"));
    assert!(html.contains("Not available (no data)"));
    let bars = bar_chart(
        &spec,
        &[None, Some(250_000.0), Some(125_000.0)],
        ChartColor::Test,
    );
    assert!(bars.contains("<rect"));
    assert!(bars.contains("Not available (no data)"));
    let spark = sparkline(&[Some(1.0), None, Some(2.0)]);
    assert!(spark.contains("1 missing"));
}

#[test]
fn formatting_public_surface_matches_the_design_rules() {
    assert_eq!(money(1_250_000_000, "USD"), "USD 1,250.00");
    assert_eq!(money_fixed(4_000_000, "USD"), "$4.00");
    assert_eq!(ecpm(4_000_000, "USD"), "$4.00");
    assert_eq!(micros_exact(1_250_000_000), "1,250,000,000");
    assert_eq!(utc_datetime(1_791_965_400_000), "14 Oct 2026, 08:10 UTC");
    assert_eq!(date_short(1_791_965_400_000), "14 Oct 2026");
    assert_eq!(percent(0.03), "3.0%");
    assert_eq!(delta(1.0, 1.03), "+3.0%");
    assert_eq!(
        or_not_available("zero baseline"),
        "Not available (zero baseline)"
    );
    assert_eq!(apex_ui::format::csv_field("a,b"), "\"a,b\"");
}

#[test]
fn logo_and_theme_public_surface_is_frozen() {
    assert_eq!(logo::MARK_PATHS.len(), 3);
    assert!(logo::lockup("#ffffff", 20).contains("Apex"));
    assert_eq!(theme::THEME_COOKIE, "apex_theme");
    assert_eq!(theme::resolve(Some("dark")), Theme::Dark);
    assert!(theme::theme_form("/console/settings", Theme::Auto).contains("method=\"post\""));
}

#[test]
fn the_committed_faces_and_the_linked_stylesheet_shell_resolve() {
    for name in apex_ui::fonts::FONT_NAMES {
        let bytes = apex_ui::fonts::font_bytes(name).unwrap_or(&[]);
        assert!(
            bytes.starts_with(b"wOF2"),
            "{name} must be a committed WOFF2"
        );
        assert!(
            apex_ui::fonts::request_bytes(&format!("{name}.woff2")).is_some(),
            "{name} must resolve through the request path"
        );
    }
    assert!(apex_ui::fonts::request_bytes("manrope-400.ttf").is_none());
    assert!(apex_ui::fonts::request_bytes("../secrets").is_none());

    let ctx = PageCtx {
        title: "Leads".to_string(),
        area: Area::SalesPlane,
        publisher_label: Some("Northstar Studio".to_string()),
        breadcrumb: vec!["Workspace".to_string()],
        read_only: false,
        theme: Theme::Light,
        help_href: None,
        marketing_header: false,
    };
    let html = apex_ui::shell::page_with_css(&ctx, "<h1>Leads</h1>", "/assets/apex.css");
    assert_eq!(html.matches("<style").count(), 0, "no embedded stylesheet");
    assert_eq!(
        html.matches("<link rel=\"stylesheet\"").count(),
        1,
        "exactly one stylesheet link"
    );
    assert!(html.contains("href=\"/assets/apex.css\""));
    assert!(html.contains("href=\"/leads\""));
}
