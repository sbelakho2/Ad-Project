//! `/console/billing` and `/console/billing/invoices/{period}`.
//!
//! The register separates fee from payment with the frozen financial
//! language; the invoice detail reproduces the signed amount (input, amount,
//! meaning) with the rate, active days, recorded billing changes, credits
//! and the tax line as its own visible fact. Every amount is the signed
//! record's amount; nothing is recomputed in HTML.

use std::fmt::Write as _;

use apex_ui::components::{data_table, kpi_card, kpi_row, notice, panel, Cell, EmptyState};
use apex_ui::format::{money_fixed, or_not_available, percent, utc_datetime};
use apex_ui::status::StatusKind;

use crate::billing_view::{excluded_foreign_note, BillingFeeLine, BillingView, InvoiceStateView};

use super::{console_document, html_escape, ConsoleCtx};

/// The frozen financial language.
pub const FINANCIAL_LANGUAGE: &str = "Draft is not invoiced. Invoiced is not collected. Pending \
                                      is not paid. Credits, taxes and monitoring remain distinct.";
/// The dispute preservation note.
pub const DISPUTE_NOTE: &str = "A dispute, when raised, preserves this original calculation.";
/// The no-VAT reason.
pub const VAT_MISSING_REASON: &str = "no VAT rate is recorded for this workspace";

/// The billing register page data.
#[derive(Debug, Clone, PartialEq)]
pub struct BillingPageData<'a> {
    /// The read model.
    pub view: &'a BillingView,
    /// Read time of the page.
    pub as_of_unix_ms: i64,
}

/// The invoice detail page data.
#[derive(Debug, Clone, PartialEq)]
pub struct InvoicePageData<'a> {
    /// The read model (used for lines, payments and the state note).
    pub view: &'a BillingView,
    /// The period `YYYY-MM`.
    pub period: &'a str,
    /// The workspace VAT rate in basis points, when one is recorded.
    pub vat_rate_bp: Option<u16>,
}

/// Render the billing register.
pub fn render_billing(ctx: &ConsoleCtx, data: &BillingPageData<'_>) -> String {
    let view = data.view;
    let mut body = String::from("<h1>Billing</h1>");
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "{} / {} / as of {}",
            view.currency,
            month_name(&view.current_month),
            utc_datetime(data.as_of_unix_ms)
        ))
    ));
    body.push_str(&notice(
        StatusKind::ReadOnly,
        "Read-only view.",
        "Fee state comes from signed billing records and recorded payments.",
        None,
    ));

    let current_lines = view.lines_for(&view.current_month);
    let current_total: i64 = current_lines.iter().map(|line| line.fee_micros).sum();
    let fee_card = if current_lines.is_empty() {
        kpi_card(
            "Lift fees",
            &or_not_available("no signed monthly billing record exists yet"),
            "One qualifying kept change starts a monthly fee line.",
            None,
        )
    } else {
        kpi_card(
            "Lift fees",
            &money_fixed(current_total, &view.currency),
            &kept_note(current_lines.len()),
            None,
        )
    };
    let monitoring_card = match view.monitoring.fee_micros {
        Some(fee) => kpi_card(
            "Monitoring",
            view.monitoring.selected_label(),
            &format!(
                "{} per month{}",
                money_fixed(fee, &view.currency),
                view.monitoring
                    .since
                    .as_deref()
                    .map(|since| format!(" / since {since}"))
                    .unwrap_or_default()
            ),
            None,
        ),
        None => kpi_card(
            "Monitoring",
            "Not recorded",
            "No plan is recorded for this workspace.",
            None,
        ),
    };
    let (status_value, status_note) = match view.register_row(&view.current_month) {
        Some(row) => (row.state.label().to_string(), row.state.note().to_string()),
        None => (
            or_not_available("no signed monthly billing record exists yet"),
            "The payment status appears with the first draft.".to_string(),
        ),
    };
    let payment_card = kpi_card("Payment status", &status_value, &status_note, None);
    body.push_str(&kpi_row(&[fee_card, monitoring_card, payment_card]));

    let rows: Vec<Vec<Cell>> = view
        .register
        .iter()
        .map(|row| {
            vec![
                Cell::Link(
                    month_name(&row.period),
                    format!("/console/billing/invoices/{}", row.period),
                ),
                Cell::Text(row.state.invoice_column(&row.period)),
                Cell::Text(row.state.label().to_string()),
                Cell::Num(money_fixed(row.total_micros, &view.currency)),
            ]
        })
        .collect();
    let mut register_note = FINANCIAL_LANGUAGE.to_string();
    for row in &view.register {
        if let Some(note) = excluded_foreign_note(row.excluded_foreign_receipts) {
            let _ = write!(register_note, " {} {}", month_name(&row.period), note);
        }
    }
    body.push_str(&data_table(
        Some("Invoice register"),
        &["Period", "Invoice", "State", "Total before tax"],
        &rows,
        Some(EmptyState {
            reason: "No signed monthly billing record is stored for this workspace.".to_string(),
            action: None,
        }),
        Some(register_note.as_str()),
    ));
    console_document(ctx, "Billing", "Billing", &body)
}

fn kept_note(lines: usize) -> String {
    match lines {
        0 => "No qualifying kept change".to_string(),
        1 => "One qualifying kept change".to_string(),
        count => format!("{count} qualifying kept changes"),
    }
}

/// The invoice-column word of a state (the subtitle uses the document's
/// `Draft` where the register column reads `Preview`).
fn invoice_word(state: InvoiceStateView) -> &'static str {
    match state {
        InvoiceStateView::Draft => "Draft",
        other => other.label(),
    }
}

/// The subtitle phrase of the fee calculation page.
fn active_kept_note(lines: usize) -> String {
    match lines {
        0 => "no active kept change".to_string(),
        1 => "one active kept change".to_string(),
        count => format!("{count} active kept changes"),
    }
}

/// Render one invoice/fee calculation page.
pub fn render_invoice(ctx: &ConsoleCtx, data: &InvoicePageData<'_>) -> String {
    let view = data.view;
    let lines = view.lines_for(data.period);
    let state = view
        .register_row(data.period)
        .map(|row| row.state)
        .unwrap_or(InvoiceStateView::Draft);
    let mut body = format!(
        "<h1>{} fee calculation</h1>",
        html_escape(&month_name(data.period))
    );
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "{} / {} / change {} / {}",
            invoice_word(state),
            view.currency,
            lines
                .first()
                .map(|line| line.change_id.clone())
                .unwrap_or_else(|| or_not_available("no change")),
            active_kept_note(lines.len())
        ))
    ));
    body.push_str(&notice(
        StatusKind::Neutral,
        state.note(),
        FINANCIAL_LANGUAGE,
        None,
    ));

    for line in &lines {
        body.push_str(&line_calculation(view, line));
    }
    if lines.is_empty() {
        body.push_str(&panel(
            "No fee line is recorded",
            "<p>This period has no signed monthly billing record, so there is no amount to \
             reproduce.</p>",
            None,
        ));
    }

    body.push_str(&vat_section(view, data.period, data.vat_rate_bp));
    body.push_str(&payments_section(view, data.period));
    if !view.billing_changes.is_empty() {
        body.push_str(&changes_section(view, data.period, &lines));
    }
    body.push_str(&format!("<p>{}</p>", html_escape(DISPUTE_NOTE)));
    console_document(
        ctx,
        &format!("{} fee calculation", month_name(data.period)),
        "Billing",
        &body,
    )
}

fn line_calculation(view: &BillingView, line: &BillingFeeLine) -> String {
    let rows = vec![
        vec![
            Cell::Text("Eligible scope revenue".to_string()),
            Cell::Num(money_fixed(line.scope_revenue, &view.currency)),
            Cell::Text("Finalized period, matching scope".to_string()),
        ],
        vec![
            Cell::Text("Billable lower bound".to_string()),
            Cell::Num(percent(line.lambda)),
            Cell::Text("Signed decision, not point estimate".to_string()),
        ],
        vec![
            Cell::Text("Attributed increment".to_string()),
            Cell::Num(money_fixed(line.incremental, &view.currency)),
            Cell::Text(format!(
                "{} x (1 - 1 / {:.2})",
                money_fixed(line.scope_revenue, &view.currency),
                line.factor
            )),
        ],
        vec![
            Cell::Text("Agreed fee rate".to_string()),
            Cell::Num(percent(f64::from(line.rate_bp) / 10_000.0)),
            Cell::Text("Rate from the order".to_string()),
        ],
        vec![
            Cell::Text("Lift fee before tax".to_string()),
            Cell::Num(money_fixed(line.fee_micros, &view.currency)),
            Cell::Text(format!(
                "{} x {}",
                money_fixed(line.incremental, &view.currency),
                percent(f64::from(line.rate_bp) / 10_000.0)
            )),
        ],
        vec![
            Cell::Text("Active days".to_string()),
            Cell::Num(format!("{} of {}", line.billed_days, line.month_days)),
            Cell::Text("Pro-rata applies to overrides and mid-month keeps".to_string()),
        ],
        vec![
            Cell::Text("Log-share of the combined factor".to_string()),
            Cell::Num(percent(line.log_share)),
            Cell::Text("Combined-factor method across active changes".to_string()),
        ],
        vec![
            Cell::Text("Signed record".to_string()),
            Cell::Mono(format!(
                "seq {} / sha {}",
                line.seq,
                line.payload_sha256.chars().take(16).collect::<String>()
            )),
            Cell::Link(
                "Open record".to_string(),
                format!("/console/ledger/records/{}", line.seq),
            ),
        ],
    ];
    format!(
        "<h2 class=\"section-heading\">{}</h2>{}",
        html_escape(&format!("Lift fee / change {}", line.change_id)),
        data_table(
            Some("Input"),
            &["Input", "Amount", "Meaning"],
            &rows,
            None,
            None,
        )
    )
}

fn vat_section(view: &BillingView, period: &str, vat_rate_bp: Option<u16>) -> String {
    let net = view.total_for(period);
    let rows = match vat_rate_bp {
        Some(bp) => {
            let vat = (net as f64 * f64::from(bp) / 10_000.0).round() as i64;
            vec![
                vec![
                    Cell::Text("Net total".to_string()),
                    Cell::Num(money_fixed(net, &view.currency)),
                ],
                vec![
                    Cell::Text(format!("VAT ({})", percent(f64::from(bp) / 10_000.0))),
                    Cell::Num(money_fixed(vat, &view.currency)),
                ],
                vec![
                    Cell::Text("Total with tax".to_string()),
                    Cell::Num(money_fixed(net.saturating_add(vat), &view.currency)),
                ],
            ]
        }
        None => vec![
            vec![
                Cell::Text("Net total".to_string()),
                Cell::Num(money_fixed(net, &view.currency)),
            ],
            vec![
                Cell::Text("VAT".to_string()),
                Cell::Text(or_not_available(VAT_MISSING_REASON)),
            ],
        ],
    };
    data_table(
        Some("Tax (kept distinct from fees and credits)"),
        &["Line", "Amount"],
        &rows,
        None,
        None,
    )
}

fn payments_section(view: &BillingView, period: &str) -> String {
    let events = view.payments_for(period);
    let rows: Vec<Vec<Cell>> = events
        .iter()
        .map(|event| {
            vec![
                Cell::Text(utc_datetime(event.at_unix_ms)),
                Cell::Text(event.event_code.clone()),
                Cell::Mono(event.psp_reference.clone()),
                // Adyen reports minor units; the console renders the fixed
                // money form of the same micros used by the collection totals.
                Cell::Num(money_fixed(
                    event.amount_value.saturating_mul(10_000),
                    &event.amount_currency,
                )),
                Cell::Status(
                    if event.success.eq_ignore_ascii_case("true") {
                        "Succeeded".to_string()
                    } else {
                        "Failed".to_string()
                    },
                    if event.success.eq_ignore_ascii_case("true") {
                        StatusKind::Verified
                    } else {
                        StatusKind::Failure
                    },
                ),
                Cell::Text(event.merchant_reference.clone()),
            ]
        })
        .collect();
    let balance = view.balance_due_for(period);
    let mut note = format!(
        "Balance due: {} / verified collections: {}.",
        money_fixed(balance, &view.currency),
        money_fixed(view.collected_for(period), &view.currency)
    );
    if let Some(exclusion) = excluded_foreign_note(view.excluded_foreign_for(period)) {
        note.push(' ');
        note.push_str(&exclusion);
    }
    data_table(
        Some("Payment events"),
        &[
            "Recorded at",
            "Event",
            "Reference",
            "Amount",
            "Result",
            "Merchant reference",
        ],
        &rows,
        Some(EmptyState {
            reason: "No payment event is recorded for this period.".to_string(),
            action: None,
        }),
        Some(note.as_str()),
    )
}

fn changes_section(view: &BillingView, period: &str, lines: &[&BillingFeeLine]) -> String {
    let change_ids: std::collections::HashSet<&str> =
        lines.iter().map(|line| line.change_id.as_str()).collect();
    let rows: Vec<Vec<Cell>> = view
        .billing_changes
        .iter()
        .filter(|change| change_ids.contains(change.change_id.as_str()))
        .map(|change| {
            vec![
                Cell::Text(change.day.clone()),
                Cell::Mono(change.change_id.clone()),
                Cell::Text(change.reason.clone()),
                Cell::Num(percent(change.billed_lambda)),
                Cell::Text(format!("seq {}", change.seq)),
            ]
        })
        .collect();
    let _ = period;
    data_table(
        Some("Billing changes (overrides, re-checks, term ends)"),
        &[
            "Day",
            "Change",
            "Reason",
            "Billed lift after",
            "Signed record",
        ],
        &rows,
        Some(EmptyState {
            reason: "No override, re-check or term end is recorded.".to_string(),
            action: None,
        }),
        None,
    )
}

/// The invoice not-found page.
pub fn render_period_not_found(ctx: &ConsoleCtx) -> String {
    let body = format!(
        "<h1>Invoice not found</h1>{}",
        notice(
            StatusKind::Failure,
            "No fee period matches this address.",
            "The register lists every period with a signed monthly billing record.",
            Some(("Open billing", "/console/billing")),
        )
    );
    console_document(ctx, "Invoice not found", "Billing", &body)
}

/// `October 2026` from `2026-10`.
pub fn month_name(period: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let parts: Vec<&str> = period.split('-').collect();
    if parts.len() != 2 {
        return period.to_string();
    }
    let month: usize = match parts[1].parse() {
        Ok(month) => month,
        Err(_) => return period.to_string(),
    };
    match MONTHS.get(month.wrapping_sub(1)) {
        Some(name) => format!("{name} {}", parts[0]),
        None => period.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_billing::CollectionEvent;

    use crate::billing_view::{build, BillingChangeView, MonitoringView};

    use super::*;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::new("Northstar Studio", "USD", true)
    }

    fn line(change: &str, month: &str, fee: i64) -> BillingFeeLine {
        BillingFeeLine {
            change_id: change.to_string(),
            month: month.to_string(),
            scope_revenue: 51_500_000_000,
            lambda: 0.03,
            factor: 1.03,
            incremental: 1_500_000_000,
            log_share: 1.0,
            rate_bp: 2_000,
            fee_micros: fee,
            billed_days: 31,
            month_days: 31,
            seq: 7,
            payload_sha256: "ab".repeat(32),
        }
    }

    fn view_with(lines: Vec<BillingFeeLine>, payments: Vec<CollectionEvent>) -> BillingView {
        let mut view = build(
            &[],
            payments,
            1_792_055_400_000,
            MonitoringView::not_recorded(),
            "USD",
        );
        view.fee_lines = lines;
        view.billing_changes = vec![BillingChangeView {
            change_id: "ch_0247".to_string(),
            day: "2026-10-05".to_string(),
            reason: "publisher override".to_string(),
            billed_lambda: 0.0,
            seq: 3,
        }];
        let mut register: Vec<_> = view
            .fee_lines
            .iter()
            .map(|line| {
                let collected = view.collected_for(&line.month);
                let state = if line.month == view.current_month {
                    InvoiceStateView::Draft
                } else if collected >= line.fee_micros && line.fee_micros > 0 {
                    InvoiceStateView::Paid
                } else if collected > 0 {
                    InvoiceStateView::Pending
                } else {
                    InvoiceStateView::NotCollected
                };
                crate::billing_view::InvoiceRegisterRow {
                    period: line.month.clone(),
                    state,
                    total_micros: line.fee_micros,
                    collected_micros: collected,
                    excluded_foreign_receipts: view.excluded_foreign_for(&line.month),
                    is_current: line.month == view.current_month,
                }
            })
            .collect();
        register.sort_by(|a, b| b.period.cmp(&a.period));
        view.register = register;
        view
    }

    #[test]
    fn the_register_matches_the_documented_layout() {
        let view = view_with(
            vec![
                line("ch_0247", "2026-10", 300_000_000),
                line("ch_0246", "2026-09", 225_000_000),
            ],
            Vec::new(),
        );
        let html = render_billing(
            &ctx(),
            &BillingPageData {
                view: &view,
                as_of_unix_ms: 1_792_055_400_000,
            },
        );
        assert!(html.contains("<h1>Billing</h1>"));
        assert!(html.contains("USD / October 2026"));
        assert!(html.contains(">Lift fees<"));
        assert!(html.contains("$300.00"));
        assert!(html.contains("One qualifying kept change"));
        assert!(html.contains(">Monitoring<"));
        assert!(html.contains("Not recorded"));
        assert!(html.contains("No plan is recorded for this workspace."));
        assert!(html.contains(">Payment status<"));
        assert!(html.contains("Preview"));
        assert!(html.contains("Invoice is still a draft."));
        assert!(html.contains(">Period<"));
        assert!(html.contains(">October 2026<"));
        assert!(html.contains("Draft 2026-10"));
        assert!(html.contains("$225.00"));
        assert!(html.contains(FINANCIAL_LANGUAGE));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn an_empty_register_never_shows_a_zero_draft() {
        let view = view_with(Vec::new(), Vec::new());
        let html = render_billing(
            &ctx(),
            &BillingPageData {
                view: &view,
                as_of_unix_ms: 1_792_055_400_000,
            },
        );
        assert!(html.contains("Not available (no signed monthly billing record exists yet)"));
        assert!(html.contains("No signed monthly billing record is stored for this workspace."));
        assert!(!html.contains("$0.00</p>"), "no fabricated zero total");
    }

    #[test]
    fn the_invoice_detail_reproduces_the_amount() {
        let view = view_with(vec![line("ch_0247", "2026-10", 300_000_000)], Vec::new());
        let html = render_invoice(
            &ctx(),
            &InvoicePageData {
                view: &view,
                period: "2026-10",
                vat_rate_bp: Some(2_000),
            },
        );
        assert!(html.contains("<h1>October 2026 fee calculation</h1>"));
        assert!(html.contains("Draft / USD / change ch_0247 / one active kept change"));
        assert!(html.contains("Eligible scope revenue"));
        assert!(html.contains("$51,500.00"));
        assert!(html.contains("Billable lower bound"));
        assert!(html.contains("3.0%"));
        assert!(html.contains("Signed decision, not point estimate"));
        assert!(html.contains("Attributed increment"));
        assert!(html.contains("$1,500.00"));
        assert!(html.contains("x (1 - 1 / 1.03)"));
        assert!(html.contains("Agreed fee rate"));
        assert!(html.contains("20.0%"));
        assert!(html.contains("Lift fee before tax"));
        assert!(html.contains("$300.00"));
        assert!(html.contains("VAT (20.0%)"));
        assert!(html.contains("$60.00"));
        assert!(html.contains("Total with tax"));
        assert!(html.contains("$360.00"));
        assert!(html.contains("Balance due: $300.00"));
        assert!(html.contains("publisher override"));
        assert!(html.contains(DISPUTE_NOTE));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn missing_vat_is_not_zero() {
        let view = view_with(vec![line("ch_0247", "2026-10", 300_000_000)], Vec::new());
        let html = render_invoice(
            &ctx(),
            &InvoicePageData {
                view: &view,
                period: "2026-10",
                vat_rate_bp: None,
            },
        );
        assert!(html.contains("Not available (no VAT rate is recorded for this workspace)"));
        assert!(
            !html.contains("VAT (0"),
            "a missing VAT rate is never rendered as zero"
        );
        assert!(
            !html.contains("Total with tax"),
            "no fabricated gross total"
        );
    }

    #[test]
    fn payments_render_with_balance_and_status() {
        let view = view_with(
            vec![line("ch_0246", "2026-09", 225_000_000)],
            vec![CollectionEvent {
                at_unix_ms: 1_792_055_400_000,
                event_code: "AUTHORISATION".to_string(),
                psp_reference: "psp-1".to_string(),
                merchant_reference: "apex-fee:northstar:exp:2026-09".to_string(),
                success: "true".to_string(),
                amount_value: 22_500,
                amount_currency: "USD".to_string(),
                stored_payment_method_id: String::new(),
            }],
        );
        let html = render_invoice(
            &ctx(),
            &InvoicePageData {
                view: &view,
                period: "2026-09",
                vat_rate_bp: None,
            },
        );
        assert!(html.contains("psp-1"));
        assert!(html.contains(">Succeeded<"));
        assert!(html.contains("apex-fee:northstar:exp:2026-09"));
        assert!(html.contains("$225.00"));
        assert!(
            !html.contains("22500 USD"),
            "the raw minor-units amount is never printed"
        );
        assert!(html.contains("Balance due: $0.00"));
        assert!(html.contains("Verified payment covers this period."));
    }

    #[test]
    fn payment_events_render_minor_units_as_fixed_money() {
        // A $225.00 receipt arrives from Adyen as 22500 minor units; the
        // payment-events table renders the same fixed money form as every
        // other amount, never the raw integer.
        let view = view_with(
            vec![line("ch_0246", "2026-09", 225_000_000)],
            vec![CollectionEvent {
                at_unix_ms: 1_792_055_400_000,
                event_code: "AUTHORISATION".to_string(),
                psp_reference: "psp-225".to_string(),
                merchant_reference: "apex-fee:northstar:exp:2026-09".to_string(),
                success: "true".to_string(),
                amount_value: 22_500,
                amount_currency: "USD".to_string(),
                stored_payment_method_id: String::new(),
            }],
        );
        let html = render_invoice(
            &ctx(),
            &InvoicePageData {
                view: &view,
                period: "2026-09",
                vat_rate_bp: None,
            },
        );
        assert!(
            html.contains("<code>psp-225</code></td><td class=\"cell-num\">$225.00</td>"),
            "the payment row renders fixed money: {html}"
        );
        assert!(
            !html.contains("22500 USD") && !html.contains(">22500<"),
            "the raw minor-units amount never appears as the amount: {html}"
        );
    }

    #[test]
    fn unknown_periods_render_the_not_found_page() {
        let html = render_period_not_found(&ctx());
        assert!(html.contains("<h1>Invoice not found</h1>"));
        assert!(html.contains("No fee period matches this address."));
    }

    #[test]
    fn foreign_currency_receipts_are_excluded_with_an_explicit_note() {
        let view = view_with(
            vec![line("ch_0246", "2026-09", 225_000_000)],
            vec![CollectionEvent {
                at_unix_ms: 1_792_055_400_000,
                event_code: "AUTHORISATION".to_string(),
                psp_reference: "psp-eur".to_string(),
                merchant_reference: "apex-fee:northstar:exp:2026-09".to_string(),
                success: "true".to_string(),
                amount_value: 300,
                amount_currency: "EUR".to_string(),
                stored_payment_method_id: String::new(),
            }],
        );
        // The register page notes the exclusion against the period.
        let register = render_billing(
            &ctx(),
            &BillingPageData {
                view: &view,
                as_of_unix_ms: 1_792_055_400_000,
            },
        );
        assert!(
            register.contains(
                "September 2026 1 receipt in another currency is excluded from this balance."
            ),
            "the register names the excluded receipts: {register}"
        );
        // The invoice page states the untouched balance and the note.
        let html = render_invoice(
            &ctx(),
            &InvoicePageData {
                view: &view,
                period: "2026-09",
                vat_rate_bp: None,
            },
        );
        assert!(html.contains("Balance due: $225.00"));
        assert!(html.contains("verified collections: $0.00"));
        assert!(html.contains("1 receipt in another currency is excluded from this balance."));
        assert!(html.contains("<code>psp-eur</code>"));
    }

    #[test]
    fn month_names_are_formatted_and_unknown_periods_pass_through() {
        assert_eq!(month_name("2026-10"), "October 2026");
        assert_eq!(month_name("2026-01"), "January 2026");
        assert_eq!(month_name("2026-13"), "2026-13");
        assert_eq!(month_name("garbage"), "garbage");
    }

    #[test]
    fn hostile_change_ids_are_escaped() {
        let mut view = view_with(vec![line("ch_0247", "2026-10", 300_000_000)], Vec::new());
        view.fee_lines[0].change_id = "<script>alert(1)</script>".to_string();
        let html = render_invoice(
            &ctx(),
            &InvoicePageData {
                view: &view,
                period: "2026-10",
                vat_rate_bp: None,
            },
        );
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    /// The invoice's changes section selects the change records whose ids the
    /// period's fee lines name. At 10_000 recorded changes and 2_000 fee
    /// lines the set-based section must render exactly the 2_000 matching
    /// rows.
    ///
    /// The pre-fix section compared `change_ids.contains` on a `Vec`
    /// (O(changes x lines)); this pins the selection (row count and
    /// membership) that the set must reproduce.
    #[test]
    fn the_changes_section_selects_exactly_the_periods_lines_at_scale() {
        let lines: Vec<BillingFeeLine> = (0..2_000)
            .map(|i| line(&format!("ch_{i:05}"), "2026-09", 1_000_000))
            .collect();
        let mut view = view_with(lines, Vec::new());
        view.billing_changes = (0..10_000)
            .map(|i| BillingChangeView {
                change_id: format!("ch_{i:05}"),
                day: "2026-10-05".to_string(),
                reason: "publisher override".to_string(),
                billed_lambda: 0.0,
                seq: 0,
            })
            .collect();

        let period_lines = view.lines_for("2026-09");
        assert_eq!(period_lines.len(), 2_000);
        let html = changes_section(&view, "2026-09", &period_lines);
        assert_eq!(
            html.matches("seq 0").count(),
            2_000,
            "exactly the period's changes are rendered"
        );
        assert!(html.contains("ch_00000"));
        assert!(html.contains("ch_01999"));
        assert!(
            !html.contains("ch_02000"),
            "non-period changes are excluded"
        );
        assert!(!html.contains("ch_09999"));
    }
}
