//! Ingestion `/control/ingestion` (page 59): freshness before volume, four
//! clocks, per-account retry.

use apex_analytics::queries::{FreshnessStatus, IngestionHealthRow};
use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::status::StatusKind;

use crate::copy;
use crate::data::CoveragePlatformRow;
use crate::fixtures::CoverageRow;

use super::{action_form, document, header, kpis, ActionFormSpec, PageMeta};

/// One retry-read form pre-filled by the route (CSRF, stale-state token and
/// idempotency key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRetryForm {
    /// Form fields.
    pub spec: ActionFormSpec,
}

/// The ingestion page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestionData {
    /// Freshness rows per source.
    pub health: Result<Vec<IngestionHealthRow>, String>,
    /// The illustrative per-day verdict matrix (fixture dataset only).
    pub coverage: Result<Vec<CoverageRow>, String>,
    /// The live per-day coverage matrix built from verified ledgers and
    /// runner snapshots; `None` in fixture mode.
    pub live_coverage: Option<Result<Vec<CoveragePlatformRow>, String>>,
    /// Retry forms per account the actor may act on.
    pub retry_forms: Vec<AccountRetryForm>,
}

/// Render the ingestion page.
pub fn render(meta: &PageMeta, data: &IngestionData) -> String {
    let mut body = header(meta, "Ingestion");
    match &data.health {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The ingestion freshness read is not available: {reason}"),
                Some((copy::TELEMETRY_ABSENT_ACTION, "/control/system")),
            ));
        }
        Ok(rows) => {
            body.push_str(&render_platform_readiness(rows));
            let stale = rows
                .iter()
                .filter(|row| row.status == FreshnessStatus::Stale)
                .count();
            let missing = rows
                .iter()
                .filter(|row| row.status == FreshnessStatus::MissingData)
                .count();
            if stale > 0 || missing > 0 {
                body.push_str(&notice(
                    StatusKind::Attention,
                    copy::DELAYED_INGESTION,
                    &format!(
                        "{} and {} in the current rows.",
                        super::count(stale, "stale source", "stale sources"),
                        super::count(missing, "missing-data source", "missing-data sources")
                    ),
                    Some((copy::DELAYED_INGESTION_ACTION, "#freshness-rows")),
                ));
            }
            body.push_str(&render_freshness_rows(meta, rows, &data.retry_forms));
        }
    }
    match &data.live_coverage {
        Some(live) => body.push_str(&render_live_coverage(live)),
        None => body.push_str(&render_coverage(&data.coverage)),
    }
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(copy::FOUR_CLOCKS)
    ));
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(
            "Retry reads carry a backoff: one request per idempotency key creates one job, a duplicate submit reuses it."
        )
    ));
    document(meta, "Ingestion", &["Ingestion"], &body)
}

fn render_platform_readiness(rows: &[IngestionHealthRow]) -> String {
    let mut platforms: Vec<String> = rows.iter().map(|row| row.platform.clone()).collect();
    platforms.sort();
    platforms.dedup();
    let cells: Vec<Vec<Cell>> = platforms
        .iter()
        .map(|platform| {
            let platform_rows: Vec<&IngestionHealthRow> = rows
                .iter()
                .filter(|row| &row.platform == platform)
                .collect();
            let fresh = platform_rows
                .iter()
                .filter(|row| row.status == FreshnessStatus::Fresh)
                .count();
            let stale = platform_rows
                .iter()
                .filter(|row| row.status == FreshnessStatus::Stale)
                .count();
            let missing = platform_rows
                .iter()
                .filter(|row| row.status == FreshnessStatus::MissingData)
                .count();
            vec![
                Cell::Text(platform.clone()),
                Cell::Num(format!("{fresh} / {}", platform_rows.len())),
                Cell::Muted(apex_ui::format::or_not_available(
                    "the read model does not carry pull latency",
                )),
                Cell::Text(format!("{stale} stale, {missing} missing data")),
                Cell::Text(if stale + missing == 0 {
                    "No action".to_string()
                } else {
                    format!(
                        "Inspect {}",
                        super::count(stale + missing, "account", "accounts")
                    )
                }),
            ]
        })
        .collect();
    data_table(
        Some("Platform readiness: final-day coverage"),
        &[
            "Platform",
            "Final-day coverage",
            "Pull p95",
            "Auth and limits",
            "Next action",
        ],
        &cells,
        None,
        Some("Expected schedules are platform-specific."),
    )
}

fn render_freshness_rows(
    meta: &PageMeta,
    rows: &[IngestionHealthRow],
    retry_forms: &[AccountRetryForm],
) -> String {
    let stale = rows
        .iter()
        .filter(|row| row.status == FreshnessStatus::Stale)
        .count();
    let mut out =
        String::from("<h2 class=\"section-heading\" id=\"freshness-rows\">Freshness rows</h2>");
    out.push_str(&kpis(&[
        (
            "Stale sources",
            &stale.to_string(),
            "Last final day > expected",
            Some("/control/ingestion"),
        ),
        (
            "Missing data",
            &rows
                .iter()
                .filter(|row| row.status == FreshnessStatus::MissingData)
                .count()
                .to_string(),
            "No finalized day recorded",
            None,
        ),
        (
            "Fresh sources",
            &rows
                .iter()
                .filter(|row| row.status == FreshnessStatus::Fresh)
                .count()
                .to_string(),
            "Final day inside the expected window",
            None,
        ),
    ]));
    let cells: Vec<Vec<Cell>> = rows
        .iter()
        .map(|row| {
            vec![
                Cell::Mono(format!("{}:{}", row.platform, row.platform_account_id)),
                Cell::Text(row.publisher_id.clone()),
                Cell::Text(
                    row.last_final_day
                        .clone()
                        .unwrap_or_else(|| "None recorded".to_string()),
                ),
                Cell::Num(
                    row.freshness_days
                        .map(|days| format!("{days}d"))
                        .unwrap_or_else(|| "-".to_string()),
                ),
                Cell::Status(row.status.as_str().to_string(), freshness_kind(row.status)),
            ]
        })
        .collect();
    out.push_str(&data_table(
        Some("One row per source account"),
        &[
            "Source account",
            "Publisher",
            "Last final day",
            "Age",
            "Status",
        ],
        &cells,
        Some(EmptyState {
            reason: "No ingestion sources are recorded.".to_string(),
            action: None,
        }),
        Some(&format!(
            "{} / as of {}",
            super::count(rows.len(), "source row", "source rows"),
            meta.as_of_display()
        )),
    ));
    for retry in retry_forms {
        out.push_str(&format!(
            "<div class=\"panel\"><h3 class=\"panel-heading\">Retry read: {}</h3>{}</div>",
            html_escape(&retry.spec.target),
            action_form(&retry.spec)
        ));
    }
    out
}

fn freshness_kind(status: FreshnessStatus) -> StatusKind {
    match status {
        FreshnessStatus::Fresh => StatusKind::Verified,
        FreshnessStatus::Stale => StatusKind::Attention,
        FreshnessStatus::MissingData => StatusKind::Failure,
    }
}

fn render_coverage(coverage: &Result<Vec<CoverageRow>, String>) -> String {
    match coverage {
        Err(reason) => format!(
            "<section class=\"panel\"><h2 class=\"panel-heading\">Coverage matrix</h2>{}</section>",
            notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("Per-day coverage is not available: {reason}"),
                None,
            )
        ),
        Ok(rows) if rows.is_empty() => format!(
            "<section class=\"panel\"><h2 class=\"panel-heading\">Coverage matrix</h2>{}</section>",
            notice(
                StatusKind::Neutral,
                "No coverage rows are recorded for this window.",
                "The matrix fills as finalized report days arrive.",
                None,
            )
        ),
        Ok(rows) => {
            let day_count = rows.iter().map(|row| row.verdicts.len()).max().unwrap_or(0);
            let mut headers: Vec<String> =
                vec!["Platform / account".to_string(), "Publisher".to_string()];
            for index in 0..day_count {
                let day = rows
                    .iter()
                    .find_map(|row| row.expected_days.get(index).cloned())
                    .unwrap_or_else(|| format!("Day {}", index + 1));
                headers.push(day);
            }
            headers.push("Latest pull".to_string());
            headers.push("Latest final".to_string());
            let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
            let cells: Vec<Vec<Cell>> = rows
                .iter()
                .map(|row| {
                    let mut cells = vec![
                        Cell::Mono(format!("{}:{}", row.platform, row.platform_account_id)),
                        Cell::Text(row.publisher_id.clone()),
                    ];
                    for index in 0..day_count {
                        let verdict = row
                            .verdicts
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| "Not expected".to_string());
                        cells.push(Cell::Status(verdict.clone(), verdict_kind(&verdict)));
                    }
                    cells.push(Cell::Text(
                        row.last_pull
                            .clone()
                            .unwrap_or_else(|| "None recorded".to_string()),
                    ));
                    cells.push(Cell::Text(
                        row.last_final_display
                            .clone()
                            .unwrap_or_else(|| "None recorded".to_string()),
                    ));
                    cells
                })
                .collect();
            format!(
                "<section class=\"panel\"><h2 class=\"panel-heading\">Coverage matrix</h2>{}</section>",
                data_table(
                    Some("Final, Provisional, Missing or Not expected per day"),
                    &header_refs,
                    &cells,
                    None,
                    Some("The matrix never averages days together."),
                )
            )
        }
    }
}

/// The live coverage matrix: one row per publisher/platform, one cell per
/// day in the window, plus the latest verified record and the snapshot state.
/// A day cell with no observation says `no data`; it is never a fabricated
/// verdict.
fn render_live_coverage(coverage: &Result<Vec<CoveragePlatformRow>, String>) -> String {
    let heading = "<section class=\"panel\"><h2 class=\"panel-heading\">Per-day coverage (verified ledger + snapshots)</h2>";
    match coverage {
        Err(reason) => format!(
            "{heading}{}</section>",
            notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("Per-day coverage is not available: {reason}"),
                None,
            )
        ),
        Ok(rows) if rows.is_empty() => format!(
            "{heading}{}</section>",
            notice(
                StatusKind::Neutral,
                "No coverage rows are recorded for this window.",
                "The matrix fills from verified ledger records and runner snapshots.",
                None,
            )
        ),
        Ok(rows) => {
            let day_count = rows.iter().map(|row| row.days.len()).max().unwrap_or(0);
            let mut headers: Vec<String> = vec!["Publisher".to_string(), "Platform".to_string()];
            for index in 0..day_count {
                let day = rows
                    .iter()
                    .find_map(|row| row.days.get(index).map(|cell| cell.day.clone()))
                    .unwrap_or_else(|| format!("Day {}", index + 1));
                headers.push(day);
            }
            headers.push("Last verified".to_string());
            headers.push("Snapshot".to_string());
            let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
            let cells: Vec<Vec<Cell>> = rows
                .iter()
                .map(|row| {
                    let mut cells = vec![
                        Cell::Mono(row.publisher.clone()),
                        Cell::Text(row.platform.clone()),
                    ];
                    for index in 0..day_count {
                        match row.days.get(index) {
                            Some(cell) if cell.records + cell.audits > 0 => {
                                cells
                                    .push(Cell::Num(format!("{} / {}", cell.records, cell.audits)));
                            }
                            _ => cells.push(Cell::Muted("no data".to_string())),
                        }
                    }
                    cells.push(Cell::Text(
                        row.last_verified_at_ms
                            .map(apex_ui::format::utc_datetime)
                            .unwrap_or_else(|| "No record".to_string()),
                    ));
                    cells.push(match (&row.snapshot_error, row.snapshot_present) {
                        (Some(error), _) => Cell::Text(format!("Unreadable: {error}")),
                        (None, true) => Cell::Text(format!(
                            "Present, age {}",
                            row.snapshot_age_ms
                                .map(|age| format!("{} h", age.max(0) / 3_600_000))
                                .unwrap_or_else(|| "unknown".to_string())
                        )),
                        (None, false) => Cell::Muted("Absent".to_string()),
                    });
                    cells
                })
                .collect();
            format!(
                "{heading}{}</section>",
                data_table(
                    Some("Day cells show verified records / audit-evidence records; `no data` means none was observed."),
                    &header_refs,
                    &cells,
                    None,
                    Some("Built from the verified ledger chain and the runner snapshot files; not a finalized reporting-day verdict."),
                )
            )
        }
    }
}

fn verdict_kind(verdict: &str) -> StatusKind {
    match verdict {
        "Final" => StatusKind::Verified,
        "Final (empty)" => StatusKind::Verified,
        "Provisional" => StatusKind::Attention,
        "Missing" => StatusKind::Failure,
        "Not expected" => StatusKind::Unavailable,
        _ => StatusKind::Neutral,
    }
}
