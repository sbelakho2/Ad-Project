//! Finance `/control/finance`, `/control/finance/invoices/{id}` and
//! `/control/finance?view=analytics` (pages 65, 66 and 68).

use apex_analytics::queries::{FeeRow, FeeState};
use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::status::StatusKind;

use crate::copy;
use crate::data::{InvoicePaymentRow, InvoiceRecord, LiveEconomicsMonth};
use crate::fixtures::{EconomicsMonth, PaymentEvent};

use super::{action_form, document, header, kpis, ActionFormSpec, PageMeta};

/// The finance page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinanceData {
    /// Fee rows.
    pub fee_rows: Result<Vec<FeeRow>, String>,
    /// The currency filter (from the URL).
    pub currency: Option<String>,
    /// The illustrative analytics view (page 66), when requested.
    pub analytics: Option<Result<Vec<EconomicsMonth>, String>>,
    /// The live monthly economics (verified ledger fees + invoice register),
    /// when requested in live mode.
    pub live_economics: Option<Result<Vec<LiveEconomicsMonth>, String>>,
}

/// The four verified totals per currency: `(currency, verified, invoiced,
/// collected, held, held_rows)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeTotals {
    /// Currency code.
    pub currency: String,
    /// Verified lift fees (estimated plus pending).
    pub verified_micros: i64,
    /// Invoiced.
    pub invoiced_micros: i64,
    /// Collected (paid).
    pub collected_micros: i64,
    /// On hold.
    pub held_micros: i64,
    /// Number of held lines.
    pub held_rows: usize,
    /// Number of exception rows.
    pub exception_rows: usize,
}

/// Aggregate fee rows per currency.
pub fn totals(rows: &[FeeRow]) -> Vec<FeeTotals> {
    let mut totals: Vec<FeeTotals> = Vec::new();
    for row in rows {
        let Some(amount) = &row.amount else {
            continue;
        };
        if !totals.iter().any(|total| total.currency == amount.currency) {
            totals.push(FeeTotals {
                currency: amount.currency.clone(),
                verified_micros: 0,
                invoiced_micros: 0,
                collected_micros: 0,
                held_micros: 0,
                held_rows: 0,
                exception_rows: 0,
            });
        }
        let total = match totals
            .iter_mut()
            .find(|total| total.currency == amount.currency)
        {
            Some(total) => total,
            None => continue,
        };
        match row.state {
            FeeState::Estimated | FeeState::Pending => {
                total.verified_micros = total.verified_micros.saturating_add(amount.micros);
            }
            FeeState::Invoiced => {
                total.invoiced_micros = total.invoiced_micros.saturating_add(amount.micros);
            }
            FeeState::Paid => {
                total.collected_micros = total.collected_micros.saturating_add(amount.micros);
            }
            FeeState::Held => {
                total.held_micros = total.held_micros.saturating_add(amount.micros);
                total.held_rows += 1;
                total.exception_rows += 1;
            }
            FeeState::Credited => {}
        }
    }
    totals.sort_by(|left, right| left.currency.cmp(&right.currency));
    totals
}

fn currency_filter(all: &[FeeTotals], selected: Option<&str>) -> String {
    let mut out = String::from("<nav class=\"tabs\" aria-label=\"Currency filter\">");
    let current = |value: Option<&str>, label: &str| {
        let selected_now = selected.unwrap_or("all") == value.unwrap_or("all");
        format!(
            "<a class=\"tab\" href=\"/control/finance{}\"{}>{}</a>",
            value
                .map(|currency| format!("?currency={}", apex_ui::escape::attr(currency)))
                .unwrap_or_default(),
            if selected_now {
                " aria-current=\"page\""
            } else {
                ""
            },
            html_escape(label)
        )
    };
    out.push_str(&current(None, "All currencies"));
    for total in all {
        out.push_str(&current(Some(&total.currency), &total.currency));
    }
    out.push_str("</nav>");
    out
}

/// Render the finance page (or analytics view).
pub fn render(meta: &PageMeta, data: &FinanceData) -> String {
    let mut body = header(meta, "Finance");
    match &data.fee_rows {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The fee read is not available: {reason}"),
                None,
            ));
        }
        Ok(rows) => {
            let all_totals = totals(rows);
            body.push_str(&currency_filter(&all_totals, data.currency.as_deref()));
            let view: Vec<FeeTotals> = all_totals
                .iter()
                .filter(|total| {
                    data.currency
                        .as_deref()
                        .is_none_or(|currency| total.currency == currency)
                })
                .cloned()
                .collect();
            body.push_str(&kpis(&[
                (
                    "Verified lift fees",
                    &sum_label(&view, |total| total.verified_micros),
                    "Eligible, not all invoiced",
                    None,
                ),
                (
                    "Invoiced",
                    &sum_label(&view, |total| total.invoiced_micros),
                    "Issued on invoices, not yet collected",
                    None,
                ),
                (
                    "Collected",
                    &sum_label(&view, |total| total.collected_micros),
                    "Reconciled and allocated payments",
                    None,
                ),
                (
                    "On hold",
                    &sum_label(&view, |total| total.held_micros),
                    &format!(
                        "{} proof exception(s); no collection is authorized from this state",
                        view.iter().map(|total| total.held_rows).sum::<usize>()
                    ),
                    Some("/control/finance"),
                ),
            ]));
            if view.iter().any(|total| total.held_rows > 0) {
                body.push_str(&notice(
                    StatusKind::Attention,
                    copy::PROOF_HOLD,
                    copy::NO_COLLECTION_AUTHORIZED,
                    Some((copy::PROOF_HOLD_ACTION, "#exceptions")),
                ));
            }
            body.push_str(&render_exceptions(rows, data.currency.as_deref()));
            if let Some(live) = &data.live_economics {
                body.push_str(&render_live_economics(live));
            } else if let Some(analytics) = &data.analytics {
                body.push_str(&render_analytics(analytics));
            } else {
                body.push_str(
                    "<p class=\"panel-body\"><a href=\"/control/finance?view=analytics\">Open service economics (month series)</a></p>",
                );
            }
        }
    }
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(copy::NO_MRR)
    ));
    document(meta, "Finance", &["Finance"], &body)
}

fn sum_label(view: &[FeeTotals], field: impl Fn(&FeeTotals) -> i64) -> String {
    match view {
        [] => apex_ui::format::or_not_available("no fee rows"),
        [single] => apex_ui::format::money(field(single), &single.currency),
        many => format!(
            "{} currencies: {}",
            many.len(),
            many.iter()
                .map(|total| apex_ui::format::money(field(total), &total.currency))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn render_exceptions(rows: &[FeeRow], currency: Option<&str>) -> String {
    let exceptions: Vec<&FeeRow> = rows
        .iter()
        .filter(|row| row.state == FeeState::Held)
        .filter(|row| {
            currency.is_none_or(|currency| {
                row.amount
                    .as_ref()
                    .is_some_and(|amount| amount.currency == currency)
            })
        })
        .collect();
    format!(
        "<section class=\"panel\"><h2 class=\"panel-heading\" id=\"exceptions\">Exceptions</h2>{}</section>",
        data_table(
            None,
            &["Change", "Period", "State", "Amount", "Next action"],
            &exceptions
                .iter()
                .map(|row| {
                    vec![
                        Cell::Mono(row.change_id.clone()),
                        Cell::Text(row.period.clone()),
                        Cell::Status(
                            row.state.as_str().to_string(),
                            match row.state {
                                FeeState::Held => StatusKind::Failure,
                                _ => StatusKind::Neutral,
                            },
                        ),
                        Cell::Text(
                            row.amount
                                .as_ref()
                                .map(|amount| apex_ui::format::money(amount.micros, &amount.currency))
                                .unwrap_or_else(|| apex_ui::format::or_not_available("amount not recorded")),
                        ),
                        Cell::Text(match row.state {
                            FeeState::Held => "Review proof".to_string(),
                            _ => "No action".to_string(),
                        }),
                    ]
                })
                .collect::<Vec<_>>(),
            Some(EmptyState {
                reason: "No fee exceptions are open.".to_string(),
                action: None,
            }),
            Some(&format!("{} exception row(s)", exceptions.len())),
        )
    )
}

fn render_analytics(analytics: &Result<Vec<EconomicsMonth>, String>) -> String {
    match analytics {
        Err(reason) => notice(
            StatusKind::Unavailable,
            "Service economics are not available.",
            reason,
            None,
        ),
        Ok(months) if months.is_empty() => notice(
            StatusKind::Neutral,
            "No finalized month is recorded yet.",
            "The month series fills from finalized months only; a provisional month is never plotted as final.",
            None,
        ),
        Ok(months) => {
            let labels: Vec<String> = months.iter().map(|month| month.month.clone()).collect();
            let chart = |title: &str, desc: &str, values: Vec<Option<f64>>, color: apex_ui::charts::ChartColor| {
                let spec = apex_ui::charts::ChartSpec {
                    title: title.to_string(),
                    desc: desc.to_string(),
                    width: 480,
                    height: 160,
                    y_label: title.to_string(),
                    x_labels: labels.clone(),
                    zero_line: true,
                };
                apex_ui::charts::bar_chart(&spec, &values, color)
            };
            let mut out = String::from("<h2 class=\"section-heading\">Service economics</h2>");
            out.push_str(&format!(
                "<p class=\"table-note\">{}</p>",
                html_escape(
                    "Small multiples on one common month axis; dollars and percentages never share an axis."
                )
            ));
            out.push_str(&chart(
                "Lift-fee revenue (USD)",
                "Monthly lift-fee revenue in USD.",
                months
                    .iter()
                    .map(|month| Some(month.lift_fee_revenue_micros as f64 / 1_000_000.0))
                    .collect(),
                apex_ui::charts::ChartColor::Test,
            ));
            out.push_str(&chart(
                "Monitoring revenue (USD)",
                "Monthly monitoring fees in USD.",
                months
                    .iter()
                    .map(|month| Some(month.monitoring_revenue_micros as f64 / 1_000_000.0))
                    .collect(),
                apex_ui::charts::ChartColor::Control,
            ));
            out.push_str(&chart(
                "Unpaid balance (USD)",
                "Monthly unpaid balance in USD.",
                months
                    .iter()
                    .map(|month| Some(month.unpaid_micros as f64 / 1_000_000.0))
                    .collect(),
                apex_ui::charts::ChartColor::Control,
            ));
            let cells: Vec<Vec<Cell>> = months
                .iter()
                .map(|month| {
                    let coverage = match month.allocated_cost_micros {
                        Some(cost) if month.lift_fee_revenue_micros > 0 => {
                            apex_ui::format::percent(
                                cost as f64 / month.lift_fee_revenue_micros as f64,
                            )
                        }
                        Some(_) => "Not available".to_string(),
                        None => "Not allocated".to_string(),
                    };
                    vec![
                        Cell::Mono(month.month.clone()),
                        Cell::Text(apex_ui::format::money(
                            month.lift_fee_revenue_micros,
                            "USD",
                        )),
                        Cell::Text(apex_ui::format::money(
                            month.monitoring_revenue_micros,
                            "USD",
                        )),
                        Cell::Text(
                            month
                                .allocated_cost_micros
                                .map(|cost| apex_ui::format::money(cost, "USD"))
                                .unwrap_or_else(|| apex_ui::format::or_not_available("not allocated")),
                        ),
                        Cell::Text(coverage),
                        Cell::Text(apex_ui::format::money(month.unpaid_micros, "USD")),
                        Cell::Num(format!("{}m", month.support_minutes)),
                    ]
                })
                .collect();
            out.push_str(&data_table(
                Some("Account economics by finalized month"),
                &[
                    "Month",
                    "Lift fees",
                    "Monitoring",
                    "Allocated cost",
                    "Cost coverage",
                    "Unpaid",
                    "Support",
                ],
                &cells,
                None,
                Some("Cost coverage is shown as a fraction; unallocated months say so."),
            ));
            out
        }
    }
}

/// The live monthly economics table: net and gross from the invoice
/// register, the verified ledger fee beside them, and an explicit `Not
/// recorded` for credits (no source carries a credit record).
fn render_live_economics(economics: &Result<Vec<LiveEconomicsMonth>, String>) -> String {
    match economics {
        Err(reason) => notice(
            StatusKind::Unavailable,
            "Service economics are not available.",
            reason,
            None,
        ),
        Ok(months) if months.is_empty() => notice(
            StatusKind::Neutral,
            "No finalized month is recorded yet.",
            "The month series fills from verified ledger fee records and the invoice register.",
            None,
        ),
        Ok(months) => {
            let mut out = String::from("<h2 class=\"section-heading\">Service economics</h2>");
            out.push_str(&format!(
                "<p class=\"table-note\">{}</p>",
                html_escape(
                    "Net and gross come from the invoice register (net is the invoiced net, gross adds VAT); verified fees come from the ledger billing-proof records; credits are not recorded by either source."
                )
            ));
            let cells: Vec<Vec<Cell>> = months
                .iter()
                .map(|month| {
                    vec![
                        Cell::Mono(month.month.clone()),
                        Cell::Text(month.currency.clone()),
                        Cell::Text(apex_ui::format::money(month.net_micros, &month.currency)),
                        Cell::Text(apex_ui::format::money(month.gross_micros, &month.currency)),
                        Cell::Muted("Not recorded".to_string()),
                        Cell::Text(apex_ui::format::money(
                            month.ledger_fee_micros,
                            &month.currency,
                        )),
                        Cell::Text(if month.is_empty() {
                            "No record".to_string()
                        } else {
                            format!(
                                "{} fee, {} invoice",
                                month.ledger_fee_records, month.invoice_records
                            )
                        }),
                    ]
                })
                .collect();
            out.push_str(&data_table(
                Some("Monthly gross, net and credits by currency"),
                &[
                    "Month",
                    "Currency",
                    "Net",
                    "Gross",
                    "Credits",
                    "Verified fees (ledger)",
                    "Records",
                ],
                &cells,
                None,
                Some("Every month between the first and last observed month is shown; an empty month says so."),
            ));
            out
        }
    }
}

/// The invoice detail page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceDetail {
    /// The invoice.
    pub invoice: InvoiceRecord,
    /// The payment timeline rows. Stages are `None` when no source records
    /// them, which renders as "Not recorded" rather than a pending state.
    pub payments: Vec<InvoicePaymentRow>,
    /// Why the verified ledger could not be read for the timeline, when it
    /// could not.
    pub payments_note: Option<String>,
    /// The pinned calculation version.
    pub calculation_version: String,
    /// Signed source hashes.
    pub source_hashes: Vec<String>,
    /// Scoped POST forms (preview credit, retry collection, dispute mark,
    /// manual reconcile).
    pub forms: Vec<ActionFormSpec>,
}

/// Render one invoice.
pub fn render_invoice(meta: &PageMeta, detail: &InvoiceDetail) -> String {
    let invoice = &detail.invoice;
    let invoice_id = invoice.id();
    let mut body = header(meta, &format!("Invoice {invoice_id}"));
    body.push_str(&format!(
        "<p class=\"panel-body\">Customer <code>{}</code> / Currency {} / Service period {} ({} to {}) / Legal identity: Apex (illustrative deployment identity)</p>",
        html_escape(&invoice.publisher_id),
        html_escape(
            invoice
                .line_items
                .first()
                .map(|_| "USD")
                .unwrap_or("USD")
        ),
        html_escape(&invoice.period),
        apex_ui::format::date_short(invoice.start_unix_ms),
        apex_ui::format::date_short(invoice.end_unix_ms)
    ));
    body.push_str(&kpis(&[
        (
            "Fee calculations",
            &invoice
                .line_items
                .iter()
                .map(|line| apex_ui::format::money(line.fee_micros, "USD"))
                .collect::<Vec<_>>()
                .join(" + "),
            "Pinned calculation version; signed source hashes below",
            None,
        ),
        (
            "Tax",
            &apex_ui::format::money(invoice.vat_micros, "USD"),
            &format!("VAT at {} bp", invoice.vat_rate_bp),
            None,
        ),
        (
            "Credit notes",
            "USD 0.00",
            "No credit note is recorded",
            None,
        ),
        (
            "Balance due",
            &apex_ui::format::money(invoice.gross_micros, "USD"),
            &format!(
                "Allocated payments {}",
                apex_ui::format::money(
                    detail
                        .payments
                        .iter()
                        .filter(|payment| payment.allocated == Some(true))
                        .map(|payment| payment.amount_micros.unwrap_or(0))
                        .sum(),
                    "USD"
                )
            ),
            None,
        ),
    ]));
    body.push_str(&format!(
        "<p class=\"table-note\">Calculation version: <code>{}</code>. The signed fee basis and the original payment amount are never edited inline.</p>",
        html_escape(&detail.calculation_version)
    ));
    body.push_str(&data_table(
        Some("Live and monitoring elections"),
        &["Election", "State"],
        &[
            vec![
                Cell::Text("Verified lift fees".to_string()),
                Cell::Status("On".to_string(), StatusKind::Verified),
            ],
            vec![
                Cell::Text("Monitoring".to_string()),
                Cell::Status("Not elected".to_string(), StatusKind::Neutral),
            ],
        ],
        None,
        None,
    ));
    let line_cells: Vec<Vec<Cell>> = invoice
        .line_items
        .iter()
        .map(|line| {
            vec![
                Cell::Num(line.seq.to_string()),
                Cell::Mono(line.experiment_id.clone()),
                Cell::Num(apex_ui::format::money(line.proven_lift, "USD")),
                Cell::Num(format!("{} bp", line.rate_bp)),
                Cell::Num(apex_ui::format::money(line.fee_micros, "USD")),
                Cell::Num(apex_ui::format::money(line.vat_micros, "USD")),
                Cell::Num(apex_ui::format::money(line.gross_micros, "USD")),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Fee lines"),
        &[
            "Seq",
            "Experiment",
            "Proven lift",
            "Rate",
            "Fee",
            "Tax",
            "Gross",
        ],
        &line_cells,
        Some(EmptyState {
            reason: "No fee lines are recorded on this invoice.".to_string(),
            action: None,
        }),
        None,
    ));
    let payment_cells: Vec<Vec<Cell>> = detail
        .payments
        .iter()
        .map(|payment| {
            vec![
                Cell::Text(
                    payment
                        .at_ms
                        .map(apex_ui::format::utc_datetime)
                        .unwrap_or_else(|| "Not recorded".to_string()),
                ),
                Cell::Text(payment.event_code.clone()),
                Cell::Text(
                    payment
                        .reference_masked
                        .clone()
                        .unwrap_or_else(|| "Not recorded".to_string()),
                ),
                Cell::Num(
                    payment
                        .amount_micros
                        .map(|micros| apex_ui::format::money(micros, &payment.currency))
                        .unwrap_or_else(|| "Not recorded".to_string()),
                ),
                stage_cell(payment.received),
                stage_cell(payment.hmac_verified),
                stage_cell(payment.deduplicated),
                stage_cell(payment.reconciled),
                stage_cell(payment.allocated),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Payment timeline"),
        &[
            "Time (UTC)",
            "Event",
            "Reference",
            "Amount",
            "Received",
            "HMAC verified",
            "Deduped",
            "Reconciled",
            "Allocated",
        ],
        &payment_cells,
        Some(EmptyState {
            reason: "No payment event is recorded for this invoice.".to_string(),
            action: None,
        }),
        Some("A stage with no source record says `Not recorded`; it is never shown as pending or done."),
    ));
    if let Some(note) = &detail.payments_note {
        body.push_str(&format!(
            "<p class=\"table-note\">Verified ledger not read for this timeline: {}</p>",
            html_escape(note)
        ));
    }
    for payment in &detail.payments {
        if let Some(note) = &payment.note {
            body.push_str(&format!(
                "<p class=\"table-note\">{}: {}</p>",
                html_escape(&payment.event_code),
                html_escape(note)
            ));
        }
    }
    body.push_str(&data_table(
        Some("Signed source hashes"),
        &["Hash"],
        &detail
            .source_hashes
            .iter()
            .map(|hash| vec![Cell::Mono(hash.clone())])
            .collect::<Vec<_>>(),
        Some(EmptyState {
            reason: "No signed source hash is recorded.".to_string(),
            action: None,
        }),
        None,
    ));
    body.push_str("<h2 class=\"section-heading\">Controls</h2>");
    for form in &detail.forms {
        body.push_str(&format!(
            "<section class=\"panel\"><h3 class=\"panel-heading\">{}</h3>{}</section>",
            html_escape(&form.label),
            action_form(form)
        ));
    }
    body.push_str(
        "<p class=\"panel-body\">Export: <a href=\"/control/finance/exports/invoices.csv\">invoice register (CSV)</a> / <a href=\"/control/finance/exports/transactions.csv\">transaction register (CSV)</a>, original currency and UTC times.</p>",
    );
    document(
        meta,
        &format!("Invoice {invoice_id}"),
        &["Finance", &invoice_id],
        &body,
    )
}

/// A payment stage cell: `Recorded`/`Pending` for a stage a payment record
/// carries, `Not recorded` when no source reports the stage at all.
fn stage_cell(stage: Option<bool>) -> Cell {
    match stage {
        Some(true) => Cell::Status("Recorded".to_string(), StatusKind::Verified),
        Some(false) => Cell::Status("Pending".to_string(), StatusKind::Queued),
        None => Cell::Muted("Not recorded".to_string()),
    }
}

/// The invoice register as CSV (RFC 4180 quoting, UTC times, original
/// currency).
pub fn invoices_csv(invoices: &[InvoiceRecord]) -> String {
    let mut out = String::from(
        "invoice_id,publisher,period,gross_micros,currency,line_count,balance_due_micros\r\n",
    );
    for invoice in invoices {
        let currency = "USD";
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\r\n",
            apex_ui::format::csv_field(&invoice.id()),
            apex_ui::format::csv_field(&invoice.publisher_id),
            apex_ui::format::csv_field(&invoice.period),
            invoice.gross_micros,
            currency,
            invoice.line_items.len(),
            invoice.gross_micros
        ));
    }
    out
}

/// The transaction register as CSV.
pub fn transactions_csv(payments: &[PaymentEvent]) -> String {
    let mut out = String::from(
        "invoice_id,event_code,at_utc,amount_micros,currency,received,hmac_verified,deduplicated,reconciled,allocated\r\n",
    );
    for payment in payments {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\r\n",
            apex_ui::format::csv_field(&payment.invoice_id),
            apex_ui::format::csv_field(&payment.event_code),
            apex_ui::format::csv_field(&apex_ui::format::utc_datetime(payment.at_ms)),
            payment.amount.micros,
            apex_ui::format::csv_field(&payment.amount.currency),
            payment.received,
            payment.hmac_verified,
            payment.deduplicated,
            payment.reconciled,
            payment.allocated
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::DemoData;

    #[test]
    fn totals_partition_the_rows_exactly() {
        let rows = DemoData::new().fee_rows();
        let totals = totals(&rows);
        assert_eq!(totals.len(), 1);
        let usd = &totals[0];
        assert_eq!(usd.currency, "USD");
        // Verified = estimated 8400; invoiced 7900; collected 6800; held
        // 300 + 200 with two exception rows.
        assert_eq!(usd.verified_micros, 8_400_000_000);
        assert_eq!(usd.invoiced_micros, 7_900_000_000);
        assert_eq!(usd.collected_micros, 6_800_000_000);
        assert_eq!(usd.held_micros, 500_000_000);
        assert_eq!(usd.held_rows, 2);
        assert_eq!(usd.exception_rows, 2);
        // Every row lands in exactly one bucket.
        let counted = rows.len();
        let bucketed = 1 + 1 + 1 + 2; // estimated+pending / invoiced / paid / held
        assert_eq!(counted, bucketed);
    }

    #[test]
    fn cost_coverage_is_a_ratio_rendered_once_by_the_formatter() {
        // 2026-01: allocated cost 2.4B micros over lift-fee revenue 6.3B
        // micros is a ratio of 0.381, rendered as 38.1%.
        // 2026-09: 2.4B over 8.7B is 27.6%.
        let html = render_analytics(&Ok(DemoData::new().economics()));
        assert!(html.contains("38.1%"), "2.4 of 6.3 is 38.1%");
        assert!(html.contains("27.6%"), "2.4 of 8.7 is 27.6%");
        assert!(!html.contains("3810.0%"), "the ratio is never scaled twice");
        assert!(!html.contains("2758.6%"), "the ratio is never scaled twice");
    }

    #[test]
    fn csv_quotes_and_uses_utc() {
        let payments = DemoData::new().payments();
        let csv = transactions_csv(&payments);
        assert!(csv.starts_with("invoice_id,event_code,at_utc,"));
        assert!(csv.contains("AUTHORISATION"));
        assert!(csv.contains("UTC"));
        assert!(csv.contains("360000000,USD"));
        let invoices = DemoData::new().invoices();
        let csv = invoices_csv(&invoices);
        assert!(csv.contains("INV-2026-10-northstar"));
        assert!(csv.contains("360000000"));
    }
}
