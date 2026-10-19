//! `/console/analytics`: the tenant-scoped read views over `AnalyticsReader`.
//!
//! Tabs are stable GET links (`?view=revenue|lift|fees|data-quality`). Every
//! view states its scope, currency and as-of time; a missing day is a gap in
//! the chart and a row in the table, never a zero. When the store is not
//! configured or did not answer, the route renders the honest not-available
//! panel with the reason instead of numbers. CSV exports are generated from
//! the same rows the page shows, keep UTC days verbatim, end every line with
//! CRLF, and neutralise spreadsheet formula triggers in text cells (numbers
//! keep their plain form).

use std::fmt::Write as _;

use apex_analytics::queries::{
    DataQuality, FeeDetail, FeeRow, LiftRow, QueryScope, RevenueBreakdownRow, RevenueDailyRow,
};
use apex_ui::charts::{line_chart, ChartColor, ChartSeries, ChartSpec};
use apex_ui::components::{action, data_table, notice, panel, tabs, Cell, EmptyState};
use apex_ui::format::{money_fixed, or_not_available, percent, push_csv_text_field};
use apex_ui::status::StatusKind;

use super::{console_document, html_escape, ConsoleCtx};

/// The tab query names.
pub const VIEW_REVENUE: &str = "revenue";
/// The lift tab.
pub const VIEW_LIFT: &str = "lift";
/// The fees tab.
pub const VIEW_FEES: &str = "fees";
/// The data-quality tab.
pub const VIEW_DATA_QUALITY: &str = "data-quality";

/// The lift-table standing note (frozen).
pub const LIFT_NOTE: &str = "A billable bound appears only after a qualifying Keep decision. A \
                             positive estimate by itself does not establish a fee.";
/// The revenue breakdown note (frozen).
pub const REVENUE_NOTE: &str = "Breakdown below: app, ad unit, country, format and source date. \
                                The table, SVG and CSV use the same query; missing dates are \
                                gaps.";
/// The data-quality rule (frozen).
pub const DATA_QUALITY_NOTE: &str = "Zero, missing and provisional data are three distinct \
                                     states. A missing day is not zero, and a provisional day is \
                                     excluded from final fee inputs.";
/// The exports note (UTC).
pub const EXPORT_NOTE: &str = "Exports keep UTC dates exactly as stored; the CSV matches the \
                               visible rows.";

/// One tab's data.
#[derive(Debug, Clone, PartialEq)]
pub enum AnalyticsData {
    /// The revenue tab.
    Revenue {
        /// Daily rows (gaps are days with no row).
        daily: Vec<RevenueDailyRow>,
        /// Breakdown by platform.
        by_platform: Vec<RevenueBreakdownRow>,
        /// Breakdown by format.
        by_format: Vec<RevenueBreakdownRow>,
        /// Breakdown by country.
        by_country: Vec<RevenueBreakdownRow>,
        /// The data-quality badge of the same query.
        quality: DataQuality,
    },
    /// The lift tab.
    Lift {
        /// Decision rows.
        rows: Vec<LiftRow>,
        /// The data-quality badge.
        quality: DataQuality,
    },
    /// The fees tab.
    Fees {
        /// Fee lifecycle rows.
        rows: Vec<FeeRow>,
        /// The selected change's calculation, when one is requested.
        detail: Option<FeeDetail>,
        /// The data-quality badge.
        quality: DataQuality,
    },
    /// The data-quality tab.
    Quality {
        /// The badge.
        quality: DataQuality,
    },
}

/// The analytics page data.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyticsPageData {
    /// Workspace publisher.
    pub publisher: String,
    /// The active tab.
    pub tab: &'static str,
    /// The scope the read executed with.
    pub scope: QueryScope,
    /// The data when the read succeeded.
    pub data: Option<AnalyticsData>,
    /// The not-available reason when it did not.
    pub unavailable_reason: Option<String>,
    /// The change selected on the fees tab, when any.
    pub selected_change: Option<String>,
}

impl AnalyticsPageData {
    /// A not-available page for one tab.
    pub fn unavailable(
        publisher: &str,
        tab: &'static str,
        scope: QueryScope,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            publisher: publisher.to_string(),
            tab,
            scope,
            data: None,
            unavailable_reason: Some(reason.into()),
            selected_change: None,
        }
    }
}

fn tab_label(tab: &str) -> &'static str {
    match tab {
        VIEW_REVENUE => "Revenue and reporting coverage",
        VIEW_LIFT => "What changed, and what is billable",
        VIEW_FEES => "Fee lifecycle",
        VIEW_DATA_QUALITY => "Data quality",
        _ => "Analytics",
    }
}

fn tab_href(tab: &str) -> String {
    format!("/console/analytics?view={tab}")
}

/// Render the analytics page for the active tab.
pub fn render(ctx: &ConsoleCtx, data: &AnalyticsPageData) -> String {
    let mut body = format!("<h1>{}</h1>", html_escape(tab_label(data.tab)));
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&scope_line(data))
    ));
    body.push_str(&tabs(&[
        (
            "Revenue".to_string(),
            tab_href(VIEW_REVENUE),
            data.tab == VIEW_REVENUE,
        ),
        (
            "Lift".to_string(),
            tab_href(VIEW_LIFT),
            data.tab == VIEW_LIFT,
        ),
        (
            "Fees".to_string(),
            tab_href(VIEW_FEES),
            data.tab == VIEW_FEES,
        ),
        (
            "Data quality".to_string(),
            tab_href(VIEW_DATA_QUALITY),
            data.tab == VIEW_DATA_QUALITY,
        ),
    ]));
    match (&data.data, &data.unavailable_reason) {
        (_, Some(reason)) => {
            body.push_str(&notice(
                StatusKind::Attention,
                "This view is not available right now.",
                reason,
                None,
            ));
            body.push_str(&panel(
                "What the analytics views contain",
                "<p>Revenue, lift, fees and data quality are separate read models over the \
                 analytics store. When the store is not configured or did not answer, this page \
                 shows the reason instead of numbers: a missing number is never rendered as \
                 zero.</p>",
                None,
            ));
        }
        (
            Some(AnalyticsData::Revenue {
                daily,
                by_platform,
                by_format,
                by_country,
                quality,
            }),
            None,
        ) => {
            body.push_str(&revenue_view(
                ctx,
                data,
                daily,
                by_platform,
                by_format,
                by_country,
                quality,
            ));
        }
        (Some(AnalyticsData::Lift { rows, quality }), None) => {
            body.push_str(&lift_view(data, rows, quality));
        }
        (
            Some(AnalyticsData::Fees {
                rows,
                detail,
                quality,
            }),
            None,
        ) => {
            body.push_str(&fees_view(data, rows, detail.as_ref(), quality));
        }
        (Some(AnalyticsData::Quality { quality }), None) => {
            body.push_str(&quality_view(quality));
        }
        (None, None) => {
            body.push_str(&notice(
                StatusKind::Attention,
                "This view is not available right now.",
                "the analytics read model returned no data",
                None,
            ));
        }
    }
    console_document(ctx, tab_label(data.tab), "Analytics", &body)
}

fn scope_line(data: &AnalyticsPageData) -> String {
    let scope = &data.scope;
    format!(
        "{} / {} / platform {} / currency {} / {}{} / as of {}",
        data.publisher,
        data.tab,
        scope.platform.as_deref().unwrap_or("all"),
        scope.currency.as_deref().unwrap_or("all"),
        match (&scope.from_day, &scope.to_day) {
            (Some(from), Some(to)) => format!("{from} to {to}"),
            _ => "no day range".to_string(),
        },
        if scope.publisher_id.is_empty() {
            " / operator-wide".to_string()
        } else {
            String::new()
        },
        apex_ui::format::utc_datetime(scope.as_of.unix_millis())
    )
}

fn quality_panel(quality: &DataQuality) -> String {
    let rows = vec![
        vec![
            Cell::Text("Matched source days".to_string()),
            Cell::Num(format!("{}", quality.matched_source_days)),
        ],
        vec![
            Cell::Text("Missing days".to_string()),
            Cell::Text(match quality.missing_days {
                Some(days) => format!("{days}"),
                None => or_not_available("no day range was requested"),
            }),
        ],
        vec![
            Cell::Text("Provisional days".to_string()),
            Cell::Num(format!("{}", quality.provisional_days)),
        ],
        vec![
            Cell::Text("Dedup rejects".to_string()),
            Cell::Text(match quality.dedup_rejects {
                Some(rejects) => format!("{rejects}"),
                None => or_not_available("the backend cannot report dedup rejects"),
            }),
        ],
        vec![
            Cell::Text("Unmapped rows (no amount or currency)".to_string()),
            Cell::Num(format!("{}", quality.unmapped_rows)),
        ],
    ];
    let mut out = data_table(Some("Data quality"), &["Item", "Value"], &rows, None, None);
    if !quality.unavailable.is_empty() {
        out.push_str(&format!(
            "<p class=\"table-note\">{}</p>",
            html_escape(&format!(
                "Not reportable by this backend: {}",
                quality.unavailable.join("; ")
            ))
        ));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn revenue_view(
    ctx: &ConsoleCtx,
    data: &AnalyticsPageData,
    daily: &[RevenueDailyRow],
    by_platform: &[RevenueBreakdownRow],
    by_format: &[RevenueBreakdownRow],
    by_country: &[RevenueBreakdownRow],
    quality: &DataQuality,
) -> String {
    let mut out = String::new();
    let total: i64 = daily.iter().map(|row| row.revenue.micros).sum();
    let currency = data
        .scope
        .currency
        .clone()
        .or_else(|| daily.first().map(|row| row.revenue.currency.clone()))
        .unwrap_or_else(|| ctx.currency.clone());
    let final_days: u32 = daily.iter().map(|row| row.final_days).sum();
    let provisional: u32 = daily.iter().map(|row| row.provisional_days).sum();
    let _latest_final = daily
        .iter()
        .filter(|row| row.final_days > 0)
        .map(|row| row.day.clone())
        .max();
    out.push_str(&apex_ui::components::kpi_card(
        "Platform revenue",
        &money_fixed(total, &currency),
        "Final source rows only",
        None,
    ));
    out.push_str(&format!(
        "<h2 class=\"section-heading\">Daily platform revenue / {}</h2>",
        html_escape(&currency)
    ));
    if daily.is_empty() {
        out.push_str(&notice(
            StatusKind::Neutral,
            "No revenue row is stored for this scope.",
            "A missing day is a gap, not zero.",
            None,
        ));
    } else {
        let (labels, points, table_rows) = daily_series(daily, &currency, &data.scope);
        let (mut y_min, mut y_max) = (0.0f64, 0.0f64);
        for point in points.iter().flatten() {
            y_min = y_min.min(*point);
            y_max = y_max.max(*point);
        }
        let pad = ((y_max - y_min) * 0.1).max(1.0);
        let spec = ChartSpec {
            title: format!("Daily platform revenue / {currency}"),
            desc: format!(
                "Daily platform revenue in {currency} from {} to {} ({} rows, {} final source \
                 days, {} provisional source days). Missing days are gaps.",
                labels.first().cloned().unwrap_or_default(),
                labels.last().cloned().unwrap_or_default(),
                daily.len(),
                final_days,
                provisional
            ),
            width: 640,
            height: 240,
            y_label: format!("Revenue ({currency})"),
            x_labels: labels,
            zero_line: false,
        };
        let series = vec![ChartSeries {
            label: format!("Revenue ({currency})"),
            color: ChartColor::Ink,
            points,
        }];
        out.push_str(&line_chart(&spec, &series, y_min - pad, y_max + pad));
        out.push_str(&data_table(
            Some("Daily revenue rows"),
            &[
                "Day",
                "Platform",
                "Revenue",
                "Impressions",
                "Final days",
                "Provisional days",
            ],
            &table_rows,
            Some(EmptyState {
                reason: "No revenue day is stored in this range.".to_string(),
                action: None,
            }),
            None,
        ));
    }
    out.push_str(&breakdown_table("By platform", by_platform, &currency));
    out.push_str(&breakdown_table("By format", by_format, &currency));
    out.push_str(&breakdown_table("By country", by_country, &currency));
    out.push_str(&format!("<p>{}</p>", html_escape(REVENUE_NOTE)));
    out.push_str(&quality_panel(quality));
    out.push_str(&export_row(data, VIEW_REVENUE));
    out
}

fn daily_series(
    daily: &[RevenueDailyRow],
    currency: &str,
    scope: &QueryScope,
) -> (Vec<String>, Vec<Option<f64>>, Vec<Vec<Cell>>) {
    let mut days: Vec<String> = Vec::new();
    if let (Some(from), Some(to)) = (&scope.from_day, &scope.to_day) {
        if let (Ok(start), Ok(end)) = (
            apex_analytics::UtcMillis::parse_rfc3339(&format!("{from}T00:00:00.000Z")),
            apex_analytics::UtcMillis::parse_rfc3339(&format!("{to}T00:00:00.000Z")),
        ) {
            let start = start.day_index();
            let end = end.day_index();
            let mut index = start;
            // Bounded walk: a hostile range cannot spin the renderer.
            while index <= end && days.len() < 800 {
                days.push(
                    apex_analytics::UtcMillis::from_unix_millis(index.saturating_mul(86_400_000))
                        .to_day_string(),
                );
                index += 1;
            }
        }
    }
    if days.is_empty() {
        days = daily.iter().map(|row| row.day.clone()).collect();
        days.sort();
        days.dedup();
    }
    let mut points: Vec<Option<f64>> = Vec::new();
    let mut table_rows: Vec<Vec<Cell>> = Vec::new();
    // One pass buckets the rows by day. The earlier loop filtered the whole
    // daily vector for every displayed day: O(days x rows) with a fresh Vec
    // allocation per day.
    let mut by_day: std::collections::HashMap<&str, Vec<&RevenueDailyRow>> =
        std::collections::HashMap::with_capacity(daily.len());
    for row in daily {
        by_day.entry(row.day.as_str()).or_default().push(row);
    }
    for day in &days {
        let rows = by_day.get(day.as_str()).map(Vec::as_slice).unwrap_or(&[]);
        if rows.is_empty() {
            points.push(None);
            table_rows.push(vec![
                Cell::Text(day.clone()),
                Cell::Text(or_not_available("no source row for this day")),
                Cell::Text(or_not_available("no source row for this day")),
            ]);
            continue;
        }
        let sum: i64 = rows.iter().map(|row| row.revenue.micros).sum();
        points.push(Some(sum as f64 / 1_000_000.0));
        for row in rows {
            table_rows.push(vec![
                Cell::Text(row.day.clone()),
                Cell::Text(row.platform.clone()),
                Cell::Num(money_fixed(row.revenue.micros, &row.revenue.currency)),
                Cell::Text(match row.impressions {
                    Some(impressions) => impressions.to_string(),
                    None => or_not_available("not every contributing row supplied impressions"),
                }),
                Cell::Num(row.final_days.to_string()),
                Cell::Num(row.provisional_days.to_string()),
            ]);
        }
    }
    let _ = currency;
    (days, points, table_rows)
}

fn breakdown_table(title: &str, rows: &[RevenueBreakdownRow], currency: &str) -> String {
    let cells: Vec<Vec<Cell>> = rows
        .iter()
        .map(|row| {
            vec![
                Cell::Text(row.dimension_value.clone()),
                Cell::Num(money_fixed(row.revenue.micros, &row.revenue.currency)),
                Cell::Text(match row.impressions {
                    Some(impressions) => impressions.to_string(),
                    None => or_not_available("not every contributing row supplied impressions"),
                }),
                Cell::Text(match row.share_bp {
                    Some(share) => percent(f64::from(share) / 10_000.0),
                    None => or_not_available("the scope total is zero, so a share is undefined"),
                }),
                Cell::Num(row.rows.to_string()),
            ]
        })
        .collect();
    let _ = currency;
    data_table(
        Some(title),
        &["Value", "Revenue", "Impressions", "Share", "Source rows"],
        &cells,
        Some(EmptyState {
            reason: "No breakdown row is stored for this scope.".to_string(),
            action: None,
        }),
        None,
    )
}

fn lift_view(data: &AnalyticsPageData, rows: &[LiftRow], quality: &DataQuality) -> String {
    let mut out = String::new();
    let cells: Vec<Vec<Cell>> = rows
        .iter()
        .map(|row| {
            vec![
                Cell::Link(
                    row.change_ref
                        .clone()
                        .unwrap_or_else(|| row.experiment_id.clone()),
                    format!("/console/experiments/{}", row.experiment_id),
                ),
                Cell::Text(match row.observed_lift_bp {
                    Some(bp) => percent(bp as f64 / 10_000.0),
                    None => or_not_available("no estimate is recorded"),
                }),
                Cell::Text(match row.billable_bound_bp {
                    Some(bp) => percent(bp as f64 / 10_000.0),
                    None => "Not decided".to_string(),
                }),
                Cell::Text(row.outcome.clone()),
                Cell::Text(match &row.measurement_source {
                    Some(source) => source.clone(),
                    None => or_not_available("no registration event is stored"),
                }),
                Cell::Text(match &row.decided_at {
                    Some(at) => at.clone(),
                    None => or_not_available("no decision is recorded"),
                }),
            ]
        })
        .collect();
    out.push_str(&data_table(
        Some("Per-change results"),
        &[
            "Change",
            "Observed lift",
            "Billable bound",
            "Outcome",
            "Measurement source",
            "Decided at",
        ],
        &cells,
        Some(EmptyState {
            reason: "No decision is stored for this scope.".to_string(),
            action: None,
        }),
        Some(LIFT_NOTE),
    ));
    out.push_str(&quality_panel(quality));
    out.push_str(&export_row(data, VIEW_LIFT));
    out
}

fn fees_view(
    data: &AnalyticsPageData,
    rows: &[FeeRow],
    detail: Option<&FeeDetail>,
    quality: &DataQuality,
) -> String {
    let mut out = String::new();
    let cells: Vec<Vec<Cell>> = rows
        .iter()
        .map(|row| {
            vec![
                Cell::Link(
                    row.change_id.clone(),
                    format!(
                        "/console/analytics?view=fees&change={}",
                        html_escape(&row.change_id)
                    ),
                ),
                Cell::Text(row.period.clone()),
                Cell::Text(row.state.as_str().to_string()),
                Cell::Text(match &row.amount {
                    Some(amount) => money_fixed(amount.micros, &amount.currency),
                    None => or_not_available("held without a computable basis"),
                }),
                Cell::Text(match &row.source_amount {
                    Some(amount) => money_fixed(amount.micros, &amount.currency),
                    None => or_not_available("no eligible source amount is recorded"),
                }),
                Cell::Text(match row.bound_bp {
                    Some(bp) => percent(bp as f64 / 10_000.0),
                    None => or_not_available("no bound is recorded"),
                }),
                Cell::Text(match row.rate_bp {
                    Some(bp) => percent(bp as f64 / 10_000.0),
                    None => or_not_available("no rate is recorded"),
                }),
                Cell::Text(match row.active_days {
                    Some(days) => format!("{days}"),
                    None => or_not_available("no active-day count is recorded"),
                }),
                Cell::Text(match &row.invoice_ref {
                    Some(reference) => reference.clone(),
                    None => "Not invoiced".to_string(),
                }),
                Cell::Text(format!("{} proof refs", row.proof_refs.len())),
            ]
        })
        .collect();
    out.push_str(&data_table(
        Some("Fee lifecycle"),
        &[
            "Change",
            "Period",
            "State",
            "Amount",
            "Source amount",
            "Bound",
            "Rate",
            "Active days",
            "Invoice",
            "Proofs",
        ],
        &cells,
        Some(EmptyState {
            reason: "No fee line is stored for this scope.".to_string(),
            action: None,
        }),
        Some("Estimated, pending, held, invoiced, paid and credited are distinct states."),
    ));
    if let Some(detail) = detail {
        out.push_str(&fee_detail_table(detail));
    } else if data.selected_change.is_some() {
        out.push_str(&notice(
            StatusKind::Neutral,
            "No calculation is stored for this change.",
            "A fee calculation appears when the signed decision and the eligible revenue are \
             both stored.",
            None,
        ));
    }
    out.push_str(&quality_panel(quality));
    out.push_str(&export_row(data, VIEW_FEES));
    out
}

fn fee_detail_table(detail: &FeeDetail) -> String {
    let rows = vec![
        vec![
            Cell::Text("Eligible scope revenue".to_string()),
            Cell::Num(money_fixed(
                detail.eligible_revenue.micros,
                &detail.eligible_revenue.currency,
            )),
            Cell::Text("Finalized period, matching scope".to_string()),
        ],
        vec![
            Cell::Text("Billable lower bound".to_string()),
            Cell::Num(percent(detail.conservative_bound_bp as f64 / 10_000.0)),
            Cell::Text("Signed decision, not point estimate".to_string()),
        ],
        vec![
            Cell::Text("Attributed increment".to_string()),
            Cell::Text(match &detail.attributed_increment {
                Some(amount) => money_fixed(amount.micros, &amount.currency),
                None => or_not_available("the calculation could not be reproduced"),
            }),
            Cell::Text("R x (1 - 1 / F), combined-factor method".to_string()),
        ],
        vec![
            Cell::Text("Agreed fee rate".to_string()),
            Cell::Num(percent(f64::from(detail.agreed_rate_bp) / 10_000.0)),
            Cell::Text("Rate from the order".to_string()),
        ],
        vec![
            Cell::Text("Lift fee before tax".to_string()),
            Cell::Text(match &detail.fee_before_tax {
                Some(amount) => money_fixed(amount.micros, &amount.currency),
                None => or_not_available("the calculation could not be reproduced"),
            }),
            Cell::Text("Attributed increment x rate".to_string()),
        ],
        vec![
            Cell::Text("Active days".to_string()),
            Cell::Text(match detail.active_days {
                Some(days) => format!("{days}"),
                None => or_not_available("no active-day count is recorded"),
            }),
            Cell::Text("Pro-rata applies to overrides and mid-month keeps".to_string()),
        ],
    ];
    let mut out = format!(
        "<h2 class=\"section-heading\">{}</h2>",
        html_escape(&format!(
            "Fee calculation / {} / {}",
            detail.change_id, detail.period
        ))
    );
    out.push_str(&data_table(
        Some("Input"),
        &["Input", "Amount", "Meaning"],
        &rows,
        None,
        None,
    ));
    if !detail.unavailable.is_empty() {
        out.push_str(&format!(
            "<p class=\"table-note\">{}</p>",
            html_escape(&format!(
                "Not reproduced: {}",
                detail.unavailable.join("; ")
            ))
        ));
    }
    if !detail.proof_refs.is_empty() {
        out.push_str(&format!(
            "<p class=\"table-note\">Proof refs: <code>{}</code></p>",
            html_escape(&detail.proof_refs.join(", "))
        ));
    }
    out
}

fn quality_view(quality: &DataQuality) -> String {
    let mut out = quality_panel(quality);
    let rows: Vec<Vec<Cell>> = quality
        .stale_sources
        .iter()
        .map(|source| {
            vec![
                Cell::Text(source.platform.clone()),
                Cell::Mono(source.platform_account_id.clone()),
                Cell::Text(match &source.last_final_day {
                    Some(day) => day.clone(),
                    None => or_not_available("no finalized day is stored"),
                }),
                Cell::Text(match source.freshness_days {
                    Some(days) => format!("{days} days"),
                    None => or_not_available("no finalized day to measure from"),
                }),
            ]
        })
        .collect();
    out.push_str(&data_table(
        Some("Stale sources (oldest gap first)"),
        &["Platform", "Account", "Last final day", "Gap"],
        &rows,
        Some(EmptyState {
            reason: "No source is past the freshness threshold.".to_string(),
            action: None,
        }),
        None,
    ));
    out.push_str(&format!("<p>{}</p>", html_escape(DATA_QUALITY_NOTE)));
    out
}

fn export_row(_data: &AnalyticsPageData, view: &str) -> String {
    format!(
        "<p class=\"table-note\">{} {}</p>",
        html_escape(EXPORT_NOTE),
        action(
            "Export this view (CSV)",
            &format!("{}&format=csv", tab_href(view)),
            false
        )
    )
}

/// The lift CSV: the same rows the lift table shows, UTC dates verbatim.
/// Text cells are neutralised against spreadsheet formula triggers; numeric
/// columns keep their plain form and every line ends CRLF.
pub fn lift_csv(rows: &[LiftRow]) -> String {
    let mut out = String::from(
        "experiment_id,change_ref,observed_lift_bp,billable_bound_bp,outcome,measurement_source,measurement_version,decided_at_utc\r\n",
    );
    for row in rows {
        push_csv_text_field(&mut out, &row.experiment_id);
        out.push(',');
        push_csv_text_field(&mut out, row.change_ref.as_deref().unwrap_or(""));
        out.push(',');
        if let Some(bp) = row.observed_lift_bp {
            let _ = write!(&mut out, "{bp}");
        }
        out.push(',');
        if let Some(bp) = row.billable_bound_bp {
            let _ = write!(&mut out, "{bp}");
        }
        out.push(',');
        push_csv_text_field(&mut out, &row.outcome);
        out.push(',');
        push_csv_text_field(&mut out, row.measurement_source.as_deref().unwrap_or(""));
        out.push(',');
        push_csv_text_field(&mut out, row.measurement_version.as_deref().unwrap_or(""));
        out.push(',');
        push_csv_text_field(&mut out, row.decided_at.as_deref().unwrap_or(""));
        out.push_str("\r\n");
    }
    out
}

/// The fee CSV: lifecycle rows with proof references. Text cells are
/// neutralised against spreadsheet formula triggers; numeric columns keep
/// their plain form and every line ends CRLF.
pub fn fees_csv(rows: &[FeeRow]) -> String {
    let mut out = String::from(
        "change_id,period,state,amount_micros,currency,source_amount_micros,bound_bp,rate_bp,active_days,invoice_ref,proof_refs\r\n",
    );
    for row in rows {
        push_csv_text_field(&mut out, &row.change_id);
        out.push(',');
        push_csv_text_field(&mut out, &row.period);
        out.push(',');
        push_csv_text_field(&mut out, row.state.as_str());
        out.push(',');
        let amount_micros = row
            .amount
            .as_ref()
            .map(|amount| amount.micros)
            .unwrap_or_default();
        let _ = write!(&mut out, "{amount_micros}");
        out.push(',');
        push_csv_text_field(
            &mut out,
            row.amount
                .as_ref()
                .map(|amount| amount.currency.as_str())
                .unwrap_or(""),
        );
        out.push(',');
        let source_micros = row
            .source_amount
            .as_ref()
            .map(|amount| amount.micros)
            .unwrap_or_default();
        let _ = write!(&mut out, "{source_micros}");
        out.push(',');
        if let Some(bp) = row.bound_bp {
            let _ = write!(&mut out, "{bp}");
        }
        out.push(',');
        if let Some(bp) = row.rate_bp {
            let _ = write!(&mut out, "{bp}");
        }
        out.push(',');
        if let Some(days) = row.active_days {
            let _ = write!(&mut out, "{days}");
        }
        out.push(',');
        push_csv_text_field(&mut out, row.invoice_ref.as_deref().unwrap_or(""));
        out.push(',');
        push_csv_text_field(&mut out, &row.proof_refs.join(" "));
        out.push_str("\r\n");
    }
    out
}

/// The revenue CSV: the same daily rows as the table. Text cells are
/// neutralised against spreadsheet formula triggers; numeric columns keep
/// their plain form and every line ends CRLF.
pub fn revenue_csv(rows: &[RevenueDailyRow]) -> String {
    let mut out = String::from(
        "day,platform,revenue_micros,currency,impressions,source_days,final_days,provisional_days\r\n",
    );
    for row in rows {
        push_csv_text_field(&mut out, &row.day);
        out.push(',');
        push_csv_text_field(&mut out, &row.platform);
        out.push(',');
        let _ = write!(&mut out, "{}", row.revenue.micros);
        out.push(',');
        push_csv_text_field(&mut out, &row.revenue.currency);
        out.push(',');
        if let Some(impressions) = row.impressions {
            let _ = write!(&mut out, "{impressions}");
        }
        out.push(',');
        let _ = write!(&mut out, "{}", row.source_days);
        out.push(',');
        let _ = write!(&mut out, "{}", row.final_days);
        out.push(',');
        let _ = write!(&mut out, "{}", row.provisional_days);
        out.push_str("\r\n");
    }
    out
}

/// The data-quality CSV: the badge plus the stale sources. Text cells are
/// neutralised against spreadsheet formula triggers; numeric columns keep
/// their plain form and every line ends CRLF.
pub fn quality_csv(quality: &DataQuality) -> String {
    let mut out = String::from(
        "publisher_id,as_of_utc,matched_source_days,missing_days,provisional_days,dedup_rejects,unmapped_rows\r\n",
    );
    push_csv_text_field(&mut out, &quality.publisher_id);
    out.push(',');
    push_csv_text_field(&mut out, &quality.as_of);
    out.push(',');
    let _ = write!(&mut out, "{}", quality.matched_source_days);
    out.push(',');
    if let Some(days) = quality.missing_days {
        let _ = write!(&mut out, "{days}");
    }
    out.push(',');
    let _ = write!(&mut out, "{}", quality.provisional_days);
    out.push(',');
    if let Some(rejects) = quality.dedup_rejects {
        let _ = write!(&mut out, "{rejects}");
    }
    out.push(',');
    let _ = write!(&mut out, "{}", quality.unmapped_rows);
    out.push_str("\r\n");
    out.push_str("platform,platform_account_id,last_final_day,freshness_days\r\n");
    for source in &quality.stale_sources {
        push_csv_text_field(&mut out, &source.platform);
        out.push(',');
        push_csv_text_field(&mut out, &source.platform_account_id);
        out.push(',');
        push_csv_text_field(&mut out, source.last_final_day.as_deref().unwrap_or(""));
        out.push(',');
        if let Some(days) = source.freshness_days {
            let _ = write!(&mut out, "{days}");
        }
        out.push_str("\r\n");
    }
    out
}

/// The 400 page for an unknown tab or filter.
pub fn render_bad_view(ctx: &ConsoleCtx, detail: &str) -> String {
    let body = format!(
        "<h1>Analytics</h1>{}",
        notice(
            StatusKind::Failure,
            "The requested view is not one of the analytics tabs.",
            detail,
            Some(("Open revenue", "/console/analytics?view=revenue")),
        )
    );
    console_document(ctx, "Analytics", "Analytics", &body)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_analytics::queries::Money;
    use apex_analytics::UtcMillis;

    use super::*;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::new("Northstar Studio", "USD", true)
    }

    fn scope() -> QueryScope {
        QueryScope::new("northstar", UtcMillis::from_unix_millis(1_792_055_400_000))
            .with_currency("USD")
            .with_day_range("2026-10-12", "2026-10-14")
    }

    fn quality() -> DataQuality {
        DataQuality {
            publisher_id: "northstar".to_string(),
            as_of: "2026-10-15T09:30:00.000Z".to_string(),
            matched_source_days: 28,
            missing_days: Some(2),
            provisional_days: 2,
            dedup_rejects: Some(3),
            unmapped_rows: 4,
            stale_sources: vec![apex_analytics::queries::StaleSource {
                platform: "levelplay".to_string(),
                platform_account_id: "app-1".to_string(),
                last_final_day: Some("2026-10-05".to_string()),
                freshness_days: Some(10),
            }],
            unavailable: vec![],
        }
    }

    fn daily(day: &str, micros: i64, final_days: u32, provisional: u32) -> RevenueDailyRow {
        RevenueDailyRow {
            day: day.to_string(),
            platform: "applovin_max".to_string(),
            revenue: Money::new(micros, "USD"),
            impressions: Some(10),
            source_days: final_days + provisional,
            final_days,
            provisional_days: provisional,
        }
    }

    fn revenue_data() -> AnalyticsData {
        AnalyticsData::Revenue {
            daily: vec![
                daily("2026-10-12", 10_000_000, 1, 0),
                daily("2026-10-14", 5_000_000, 1, 0),
            ],
            by_platform: vec![RevenueBreakdownRow {
                dimension_value: "applovin_max".to_string(),
                revenue: Money::new(15_000_000, "USD"),
                impressions: Some(20),
                rows: 2,
                share_bp: Some(10_000),
            }],
            by_format: vec![],
            by_country: vec![],
            quality: quality(),
        }
    }

    fn day_string(index: i64) -> String {
        UtcMillis::from_unix_millis(index.saturating_mul(86_400_000)).to_day_string()
    }

    fn naive_daily_series(
        daily: &[RevenueDailyRow],
        currency: &str,
        scope: &QueryScope,
    ) -> (Vec<String>, Vec<Option<f64>>, Vec<Vec<Cell>>) {
        let mut days: Vec<String> = Vec::new();
        if let (Some(from), Some(to)) = (&scope.from_day, &scope.to_day) {
            if let (Ok(start), Ok(end)) = (
                UtcMillis::parse_rfc3339(&format!("{from}T00:00:00.000Z")),
                UtcMillis::parse_rfc3339(&format!("{to}T00:00:00.000Z")),
            ) {
                let start = start.day_index();
                let end = end.day_index();
                let mut index = start;
                while index <= end && days.len() < 800 {
                    days.push(day_string(index));
                    index += 1;
                }
            }
        }
        if days.is_empty() {
            days = daily.iter().map(|row| row.day.clone()).collect();
            days.sort();
            days.dedup();
        }
        let mut points: Vec<Option<f64>> = Vec::new();
        let mut table_rows: Vec<Vec<Cell>> = Vec::new();
        for day in &days {
            let rows: Vec<&RevenueDailyRow> = daily.iter().filter(|row| &row.day == day).collect();
            if rows.is_empty() {
                points.push(None);
                table_rows.push(vec![
                    Cell::Text(day.clone()),
                    Cell::Text(or_not_available("no source row for this day")),
                    Cell::Text(or_not_available("no source row for this day")),
                ]);
                continue;
            }
            let sum: i64 = rows.iter().map(|row| row.revenue.micros).sum();
            points.push(Some(sum as f64 / 1_000_000.0));
            for row in rows {
                table_rows.push(vec![
                    Cell::Text(row.day.clone()),
                    Cell::Text(row.platform.clone()),
                    Cell::Num(money_fixed(row.revenue.micros, &row.revenue.currency)),
                    Cell::Text(match row.impressions {
                        Some(impressions) => format!("{impressions}"),
                        None => or_not_available("not every contributing row supplied impressions"),
                    }),
                    Cell::Num(format!("{}", row.final_days)),
                    Cell::Num(format!("{}", row.provisional_days)),
                ]);
            }
        }
        let _ = currency;
        (days, points, table_rows)
    }

    #[test]
    fn daily_series_buckets_eight_hundred_days_with_identical_output() {
        let start = UtcMillis::parse_rfc3339("2024-01-01T00:00:00.000Z")
            .unwrap()
            .day_index();
        let end = start + 799;
        let mut daily = Vec::new();
        for i in 0..800 {
            if i % 11 == 0 {
                continue;
            }
            let day = day_string(start + i);
            daily.push(RevenueDailyRow {
                day: day.clone(),
                platform: "applovin_max".to_string(),
                revenue: Money::new(2_000_000 + i, "USD"),
                impressions: Some(100 + i as u64),
                source_days: 1,
                final_days: 1,
                provisional_days: 0,
            });
            if i % 3 == 0 {
                daily.push(RevenueDailyRow {
                    day: day.clone(),
                    platform: "levelplay".to_string(),
                    revenue: Money::new(1_000_000 + i, "USD"),
                    impressions: None,
                    source_days: 2,
                    final_days: 1,
                    provisional_days: 1,
                });
            }
            if i % 17 == 0 {
                daily.push(RevenueDailyRow {
                    day,
                    platform: "admob".to_string(),
                    revenue: Money::new(500_000, "USD"),
                    impressions: Some(0),
                    source_days: 1,
                    final_days: 0,
                    provisional_days: 1,
                });
            }
        }
        // The query result is not promised to be day-ordered; bucketing must
        // preserve the input order within each day exactly as the filter did.
        daily.reverse();
        let scope = QueryScope::new("northstar", UtcMillis::from_unix_millis(1_792_055_400_000))
            .with_currency("USD")
            .with_day_range(day_string(start), day_string(end));
        let expected = naive_daily_series(&daily, "USD", &scope);
        let actual = daily_series(&daily, "USD", &scope);
        assert_eq!(actual.0.len(), 800);
        assert_eq!(actual, expected, "800-day output must be byte-identical");

        // The no-range fallback (sorted distinct stored days) is unchanged too.
        let no_range = QueryScope::new("northstar", UtcMillis::from_unix_millis(1_792_055_400_000))
            .with_currency("USD");
        assert_eq!(
            daily_series(&daily, "USD", &no_range),
            naive_daily_series(&daily, "USD", &no_range)
        );
    }

    #[test]
    fn revenue_tab_shows_currency_scope_as_of_gaps_and_breakdowns() {
        let data = AnalyticsPageData {
            publisher: "northstar".to_string(),
            tab: VIEW_REVENUE,
            scope: scope(),
            data: Some(revenue_data()),
            unavailable_reason: None,
            selected_change: None,
        };
        let html = render(&ctx(), &data);
        assert!(html.contains("<h1>Revenue and reporting coverage</h1>"));
        assert!(html.contains("2026-10-12 to 2026-10-14"));
        assert!(html.contains("currency USD"));
        assert!(html.contains("as of 15 Oct 2026, 09:10 UTC"));
        assert!(html.contains(">Platform revenue<"));
        assert!(html.contains("$15.00"));
        assert!(html.contains("Final source rows only"));
        // 2026-10-13 is missing: a gap in the chart, a row in the table.
        assert!(html.contains("2026-10-13"));
        assert!(html.contains("Not available (no source row for this day)"));
        assert!(html.contains("By platform"));
        assert!(html.contains("By format"));
        assert!(html.contains("Matched source days"));
        assert!(html.contains("Dedup rejects"));
        assert!(html.contains("Unmapped rows"));
        assert!(html.contains("Export this view (CSV)"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn lift_tab_shows_rows_with_sources_and_exports() {
        let data = AnalyticsPageData {
            publisher: "northstar".to_string(),
            tab: VIEW_LIFT,
            scope: scope(),
            data: Some(AnalyticsData::Lift {
                rows: vec![LiftRow {
                    experiment_id: "exp_0247".to_string(),
                    change_ref: Some("ch_0247".to_string()),
                    observed_lift_bp: Some(540),
                    billable_bound_bp: Some(300),
                    outcome: "keep".to_string(),
                    measurement_source: Some("apex-lift-0.1.0".to_string()),
                    measurement_version: Some("sha".to_string()),
                    decided_at: Some("2026-10-14T09:30:00.000Z".to_string()),
                }],
                quality: quality(),
            }),
            unavailable_reason: None,
            selected_change: None,
        };
        let html = render(&ctx(), &data);
        assert!(html.contains("<h1>What changed, and what is billable</h1>"));
        assert!(html.contains(">ch_0247<"));
        assert!(html.contains("5.4%"));
        assert!(html.contains("3.0%"));
        assert!(html.contains(">keep<"));
        assert!(html.contains("apex-lift-0.1.0"));
        assert!(html.contains("Measurement source"));
        assert!(html.contains(LIFT_NOTE));
    }

    #[test]
    fn fees_tab_shows_lifecycle_rows_and_detail() {
        use apex_analytics::queries::{FeeDetail, FeeState};
        let data = AnalyticsPageData {
            publisher: "northstar".to_string(),
            tab: VIEW_FEES,
            scope: scope(),
            data: Some(AnalyticsData::Fees {
                rows: vec![FeeRow {
                    change_id: "ch_0247".to_string(),
                    period: "2026-10".to_string(),
                    state: FeeState::Estimated,
                    amount: Some(Money::new(300_000_000, "USD")),
                    source_amount: Some(Money::new(51_500_000_000, "USD")),
                    bound_bp: Some(300),
                    rate_bp: Some(2_000),
                    active_days: Some(31),
                    invoice_ref: None,
                    proof_refs: vec!["pre-hash".to_string(), "result-hash".to_string()],
                }],
                detail: Some(FeeDetail {
                    change_id: "ch_0247".to_string(),
                    period: "2026-10".to_string(),
                    eligible_revenue: Money::new(51_500_000_000, "USD"),
                    conservative_bound_bp: 300,
                    agreed_rate_bp: 2_000,
                    attributed_increment: Some(Money::new(1_500_000_000, "USD")),
                    fee_before_tax: Some(Money::new(300_000_000, "USD")),
                    active_days: Some(31),
                    proof_refs: vec!["pre-hash".to_string()],
                    calculated_at: Some("2026-10-15T00:00:00.000Z".to_string()),
                    unavailable: vec![],
                }),
                quality: quality(),
            }),
            unavailable_reason: None,
            selected_change: Some("ch_0247".to_string()),
        };
        let html = render(&ctx(), &data);
        assert!(html.contains("<h1>Fee lifecycle</h1>"));
        assert!(html.contains(">estimated<"));
        assert!(html.contains("$300.00"));
        assert!(html.contains("Not invoiced"));
        assert!(html.contains("Eligible scope revenue"));
        assert!(html.contains("Fee calculation / ch_0247 / 2026-10"));
        assert!(html.contains("pre-hash"));
    }

    #[test]
    fn quality_tab_lists_stale_sources_with_the_oldest_gap() {
        let data = AnalyticsPageData {
            publisher: "northstar".to_string(),
            tab: VIEW_DATA_QUALITY,
            scope: scope(),
            data: Some(AnalyticsData::Quality { quality: quality() }),
            unavailable_reason: None,
            selected_change: None,
        };
        let html = render(&ctx(), &data);
        assert!(html.contains("<h1>Data quality</h1>"));
        assert!(html.contains("Stale sources (oldest gap first)"));
        assert!(html.contains("levelplay"));
        assert!(html.contains("app-1"));
        assert!(html.contains("10 days"));
        assert!(html.contains(DATA_QUALITY_NOTE));
    }

    #[test]
    fn unavailable_pages_state_the_reason_and_never_numbers() {
        let data = AnalyticsPageData::unavailable(
            "northstar",
            VIEW_REVENUE,
            scope(),
            "the analytics store did not answer",
        );
        let html = render(&ctx(), &data);
        assert!(html.contains("This view is not available right now."));
        assert!(html.contains("the analytics store did not answer"));
        assert!(!html.contains("$0.00"));
        assert!(!html.contains(">0<"));
    }

    #[test]
    fn csv_exports_keep_utc_days_and_headers() {
        let rows = vec![daily("2026-10-12", 10_000_000, 1, 0)];
        let csv = revenue_csv(&rows);
        assert!(csv.starts_with("day,platform,revenue_micros,currency"));
        assert!(csv.contains("2026-10-12,applovin_max,10000000,USD"));
        let lift = lift_csv(&[LiftRow {
            experiment_id: "exp_1,quoted".to_string(),
            change_ref: None,
            observed_lift_bp: None,
            billable_bound_bp: None,
            outcome: "pending".to_string(),
            measurement_source: None,
            measurement_version: None,
            decided_at: None,
        }]);
        assert!(
            lift.contains("\"exp_1,quoted\""),
            "csv quoting holds: {lift}"
        );
        let quality = quality_csv(&quality());
        assert!(quality.contains("2026-10-15T09:30:00.000Z"));
        assert!(quality.contains("levelplay,app-1,2026-10-05,10"));
    }

    #[test]
    fn csv_exports_neutralize_formula_text_cells_and_end_every_line_with_crlf() {
        use apex_analytics::queries::{FeeState, Money};

        fn assert_crlf_only(csv: &str) {
            assert!(csv.ends_with("\r\n"), "the last line ends CRLF: {csv}");
            for (index, byte) in csv.bytes().enumerate() {
                if byte == b'\n' {
                    assert!(
                        index > 0 && csv.as_bytes()[index - 1] == b'\r',
                        "lone LF at byte {index}: {csv}"
                    );
                }
            }
        }

        let lift = lift_csv(&[LiftRow {
            experiment_id: "=cmd|' /C calc'!A0".to_string(),
            change_ref: Some("@user".to_string()),
            observed_lift_bp: Some(-120),
            billable_bound_bp: Some(30),
            outcome: "+SUM(1,1)".to_string(),
            measurement_source: Some("-abc".to_string()),
            measurement_version: Some("v1".to_string()),
            decided_at: Some("2026-10-10T09:30:00.000Z".to_string()),
        }]);
        assert_eq!(
            lift,
            "experiment_id,change_ref,observed_lift_bp,billable_bound_bp,outcome,measurement_source,measurement_version,decided_at_utc\r\n\
             '=cmd|' /C calc'!A0,'@user,-120,30,\"'+SUM(1,1)\",'-abc,v1,2026-10-10T09:30:00.000Z\r\n"
        );
        assert_crlf_only(&lift);

        let fees = fees_csv(&[FeeRow {
            change_id: "=cmd|' /C calc'!A0".to_string(),
            period: "@user".to_string(),
            state: FeeState::Estimated,
            amount: Some(Money::new(-120, "USD")),
            source_amount: Some(Money::new(51_500_000_000, "USD")),
            bound_bp: Some(-120),
            rate_bp: Some(2_000),
            active_days: Some(31),
            invoice_ref: Some("+SUM(1,1)".to_string()),
            proof_refs: vec!["-abc".to_string()],
        }]);
        assert_eq!(
            fees,
            "change_id,period,state,amount_micros,currency,source_amount_micros,bound_bp,rate_bp,active_days,invoice_ref,proof_refs\r\n\
             '=cmd|' /C calc'!A0,'@user,estimated,-120,USD,51500000000,-120,2000,31,\"'+SUM(1,1)\",'-abc\r\n"
        );
        assert_crlf_only(&fees);

        let revenue = revenue_csv(&[RevenueDailyRow {
            day: "=cmd|' /C calc'!A0".to_string(),
            platform: "@user".to_string(),
            revenue: Money::new(-120, "+SUM(1,1)"),
            impressions: Some(5),
            source_days: 1,
            final_days: 0,
            provisional_days: 0,
        }]);
        assert_eq!(
            revenue,
            "day,platform,revenue_micros,currency,impressions,source_days,final_days,provisional_days\r\n\
             '=cmd|' /C calc'!A0,'@user,-120,\"'+SUM(1,1)\",5,1,0,0\r\n"
        );
        assert_crlf_only(&revenue);

        let mut quality = quality();
        quality.publisher_id = "=cmd|' /C calc'!A0".to_string();
        quality.as_of = "@user".to_string();
        quality.stale_sources[0].platform = "+SUM(1,1)".to_string();
        quality.stale_sources[0].platform_account_id = "-abc".to_string();
        let csv = quality_csv(&quality);
        assert!(csv.contains("'=cmd|' /C calc'!A0,'@user,"));
        assert!(csv.contains("\"'+SUM(1,1)\",'-abc,"));
        assert_crlf_only(&csv);
    }

    #[test]
    fn unknown_views_render_a_useful_400_page() {
        let html = render_bad_view(
            &ctx(),
            "The view parameter is not one of revenue, lift, fees or data-quality.",
        );
        assert!(html.contains("<h1>Analytics</h1>"));
        assert!(html.contains("not one of the analytics tabs"));
        assert!(html.contains("Open revenue"));
    }
}
