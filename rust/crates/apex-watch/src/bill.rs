//! The monthly bill: one setup-watch line per active subscriber.
//!
//! [`run_bill`] issues one VAT invoice per active subscriber through the
//! same [`apex_billing::InvoiceRegister`] path the runner uses (the default
//! register is [`apex_billing::DEFAULT_REGISTER_PATH`], the deployment's
//! `.apex/billing/invoices.jsonl`). The line is the frozen
//! [`apex_billing::watch_plan`] shape: $99 for the normal tier and $199 for
//! the core tier, in [`apex_billing::PLAN_CURRENCY`].
//!
//! The customer profile comes from the subscriber row (`billing_profile`).
//! A subscriber without one is refused with a typed reason before the
//! register is touched, so a refusal never consumes a number and no half
//! invoice is ever rendered. A subscriber already recorded as billed for the
//! month is skipped, so rerunning the monthly timer cannot issue a second
//! line.
//!
//! [`issue_first_invoice`] issues the invoice for the first payment too: an
//! active subscriber with a stored profile and an accepted first payment gets
//! one gapless register number for the payment's month, and a replay returns
//! the recorded invoice without consuming a second number. The first payment's
//! amount and currency are recorded on the subscriber.
//!
//! Cancelled subscribers are never billed: the cancel action flips `status`
//! to `canceled` and the monthly pass only walks active rows.

use std::path::Path;
use std::sync::Mutex;

use apex_billing::{
    compute_invoice, watch_plan, AdyenClient, BillingPeriod, BillingProfile, InvoiceLineItem,
    InvoiceRegister, ViesClient, MICROS_PER_MINOR_UNIT, PLAN_CURRENCY, WATCH_CORE_PRICE_MICROS,
    WATCH_PRICE_MICROS,
};
use apex_connectors::transport::Transport;
use serde::{Deserialize, Serialize};

use crate::model::{Subscriber, Tier};
use crate::store::{SubscriberStore, WatchError};

/// Serializes first-invoice checks and the register issue across concurrent
/// callers, so two requests cannot both observe "not issued yet" and consume
/// two numbers. The function holds no await point while this guard is held.
static FIRST_INVOICE_LOCK: Mutex<()> = Mutex::new(());

// The register opens this process made from the watch billing code. The
// scaling seam asserts one open serves every subscriber in a run instead of
// one open per subscriber; it is thread-local so parallel tests do not see
// each other's work.
#[cfg(test)]
thread_local! {
    static REGISTER_OPENS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_register_opens() {
    REGISTER_OPENS.with(|count| count.set(0));
}

#[cfg(test)]
fn register_open_count() -> u64 {
    REGISTER_OPENS.with(std::cell::Cell::get)
}

/// Open a register through the one seam the billing code uses, so the
/// scaling test can count opens without changing the register itself.
fn open_register(path: &Path) -> Result<InvoiceRegister, apex_billing::BillingError> {
    #[cfg(test)]
    REGISTER_OPENS.with(|count| count.set(count.get() + 1));
    let register = InvoiceRegister::open(path)?;
    // The register holds the customer's legal name and VAT number. The
    // billing crate creates it with the process umask; the watch owns this
    // path, so keep it owner-only from the first issue onward.
    restrict_register(path);
    Ok(register)
}

/// Make an existing invoice register owner-only. A missing file is created
/// by the first issue and restricted by [`restrict_register`]'s next call.
#[cfg(unix)]
fn restrict_register(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_register(_path: &Path) {}

/// One issued invoice line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedInvoice {
    /// The publisher slug.
    pub publisher: String,
    /// The tier billed.
    pub tier: Tier,
    /// The gapless register number.
    pub number: u64,
    /// The net amount in micros.
    pub net_micros: i64,
    /// The gross amount in micros (net plus VAT).
    pub gross_micros: i64,
    /// The rendered VAT document.
    pub document: String,
    /// Whether the stored card was charged for this invoice. This is never
    /// true without a PSP receipt: the first payment's invoice is reported
    /// through [`Self::paid_by_first_payment`] instead, so a deployment with
    /// no checkout credentials cannot claim a charge it never made.
    pub charged: bool,
    /// Whether the accepted first payment (collected by the payment link)
    /// covers this invoice. The first invoice is paid by that payment, not by
    /// a stored-card charge, so it is never reported as `charged`.
    #[serde(default)]
    pub paid_by_first_payment: bool,
    /// The deterministic charge reference, when a charge was attempted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge_reference: Option<String>,
    /// The Adyen PSP reference of an accepted charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psp_reference: Option<String>,
    /// Why the charge did not land, when it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge_error: Option<String>,
}

/// One successful stored-card charge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChargedInvoice {
    /// The publisher slug.
    pub publisher: String,
    /// The month billed (`YYYY-MM`).
    pub month: String,
    /// The gross amount charged, in micros.
    pub amount_micros: i64,
    /// The deterministic Adyen reference (idempotency key).
    pub reference: String,
    /// The Adyen PSP reference.
    pub psp_reference: String,
    /// The accepted result code (`Authorised` / `Received`).
    pub result_code: String,
}

/// One refused subscriber.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefusedInvoice {
    /// The publisher slug.
    pub publisher: String,
    /// The typed reason (no number was consumed).
    pub reason: String,
}

/// One subscriber skipped because the month is already billed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedInvoice {
    /// The publisher slug.
    pub publisher: String,
    /// The month already recorded.
    pub month: String,
}

/// The full monthly report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillReport {
    /// Invoices issued, oldest subscriber first.
    pub issued: Vec<IssuedInvoice>,
    /// Stored-card charges accepted this run (including retries of an
    /// invoice issued by an earlier run).
    pub charged: Vec<ChargedInvoice>,
    /// Subscribers refused this month.
    pub refused: Vec<RefusedInvoice>,
    /// Subscribers skipped as already billed.
    pub skipped: Vec<SkippedInvoice>,
}

/// The injected dependencies of one monthly run.
pub struct BillDeps<'a> {
    /// The seller profile the VAT treatment loads.
    pub seller: &'a BillingProfile,
    /// The invoice register path the runner also writes.
    pub register_path: &'a Path,
    /// The EU VAT register seam reverse charge is decided through.
    pub vies: &'a dyn ViesClient,
    /// The Adyen client that charges the stored card, when configured.
    pub adyen: Option<&'a AdyenClient>,
    /// The transport the Adyen client speaks through, when configured.
    pub transport: Option<&'a dyn Transport>,
}

impl std::fmt::Debug for BillDeps<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BillDeps")
            .field("seller", &self.seller)
            .field("register_path", &self.register_path)
            .field("vies", &self.vies)
            .field("adyen", &self.adyen)
            .field("transport_configured", &self.transport.is_some())
            .finish()
    }
}

/// The exact UTC bounds of one `YYYY-MM` calendar month, in unix
/// milliseconds: the first millisecond of the month and the first
/// millisecond of the next.
pub fn month_bounds(month: &str) -> Option<(i64, i64)> {
    let (year_text, month_text) = month.split_once('-')?;
    if year_text.len() != 4 || month_text.len() != 2 {
        return None;
    }
    if !year_text.bytes().all(|byte| byte.is_ascii_digit())
        || !month_text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let year: i64 = year_text.parse().ok()?;
    let month_number: u32 = month_text.parse().ok()?;
    if !(1..=12).contains(&month_number) {
        return None;
    }
    let start = days_from_civil(year, month_number, 1).checked_mul(86_400_000)?;
    let (next_year, next_month) = if month_number == 12 {
        (year + 1, 1)
    } else {
        (year, month_number + 1)
    };
    let end = days_from_civil(next_year, next_month, 1).checked_mul(86_400_000)?;
    Some((start, end))
}

/// The `YYYY-MM` month containing `now_ms` (UTC).
pub fn current_month(now_ms: i64) -> String {
    let days = now_ms.div_euclid(86_400_000);
    let (year, month) = civil_month_from_days(days);
    format!("{year:04}-{month:02}")
}

/// Days since 1970-01-01 for one civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = i64::from((month + 9) % 12);
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil `(year, month)` of a day count since 1970-01-01.
fn civil_month_from_days(days: i64) -> (i64, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let month = (month_prime + 2) % 12 + 1;
    let year = if month_prime >= 10 { year + 1 } else { year };
    (year, month as u32)
}

/// The bounded fan-out of stored-card charges in one monthly run. Invoices
/// and register numbers stay serial (a gapless register has one ordering);
/// only the PSP calls overlap, at most this many at a time.
pub const BILL_CHARGE_CONCURRENCY: usize = 4;

/// One stored-card charge waiting for the concurrent phase.
struct ChargeTask {
    /// The subscriber's position in the folded active set, so results are
    /// persisted and reported in subscriber order.
    index: usize,
    subscriber: Subscriber,
    month: String,
    amount_micros: i64,
    reference: String,
    /// The issued-invoice report row for a renewal (absent for a retry of an
    /// invoice an earlier run issued).
    issued: Option<IssuedInvoice>,
    /// Whether this retries an invoice issued by an earlier run.
    retry: bool,
    /// The subscriber row recorded before the charge (the issued invoice and
    /// its charge facts). The post-charge row starts from this copy.
    pre_updated: Subscriber,
}

/// Everything one subscriber contributes to the report, kept in subscriber
/// order and flattened at the end.
#[derive(Default)]
struct RowOutcome {
    issued: Option<IssuedInvoice>,
    charged: Option<ChargedInvoice>,
    refused: Option<RefusedInvoice>,
    skipped: Option<SkippedInvoice>,
}

/// Issue the month's setup-watch lines for every active subscriber and charge
/// the stored card.
///
/// An invoice is issued exactly once per month: the register number and the
/// charge facts are recorded on the subscriber row before the charge is
/// attempted. An issued-but-unpaid invoice is retried on the next run with
/// the same deterministic reference (the Adyen idempotency key), never
/// re-issued. Cancelled subscribers are not walked at all.
///
/// Invoice issuance and register numbering run serially in subscriber order.
/// The stored-card charges they produce then run with bounded concurrency
/// ([`BILL_CHARGE_CONCURRENCY`]); their results are persisted back in
/// subscriber order, so the register, the rows and the report are identical
/// to the serial run while the PSP latency overlaps.
pub async fn run_bill(
    store: &SubscriberStore,
    month: &str,
    deps: &BillDeps<'_>,
) -> Result<BillReport, WatchError> {
    let Some((start_unix_ms, end_unix_ms)) = month_bounds(month) else {
        return Err(WatchError::Malformed(format!(
            "month '{month}' is not a YYYY-MM calendar month"
        )));
    };
    let register = open_register(deps.register_path)?;
    // Several subscribers may be the same legal customer. One register
    // question per canonical (country, number) per run is enough; each
    // invoice still persists the answer it was decided from.
    let vies = apex_billing::CachedVies::new(deps.vies);
    let deps = BillDeps {
        seller: deps.seller,
        register_path: deps.register_path,
        vies: &vies,
        adyen: deps.adyen,
        transport: deps.transport,
    };
    let subscribers = store.list_active()?;
    let mut outcomes: Vec<RowOutcome> = subscribers.iter().map(|_| RowOutcome::default()).collect();
    let mut tasks = Vec::new();

    // Phase 1 (serial): decide, issue and record every invoice in subscriber
    // order. No PSP call happens here.
    for (index, subscriber) in subscribers.iter().enumerate() {
        // An accepted first payment whose invoice is not issued yet is the
        // first invoice, not a renewal: issuing the plan line and charging
        // the card again would collect the first month twice. The first
        // payment's month is the month containing `paid_at`; the price the
        // payment link charged is the tier price.
        if subscriber.first_invoice_number.is_none()
            && subscriber.first_payment_amount_micros.is_none()
            && subscriber
                .paid_at
                .is_some_and(|paid_at| (start_unix_ms..end_unix_ms).contains(&paid_at))
        {
            let price = match subscriber.tier {
                Tier::Core => WATCH_CORE_PRICE_MICROS,
                Tier::Normal => WATCH_PRICE_MICROS,
            };
            match issue_first_invoice_with_register(
                store,
                &subscriber.id,
                month,
                price,
                PLAN_CURRENCY,
                &deps,
                &register,
            )
            .await
            {
                Ok(invoice) => outcomes[index].issued = Some(invoice),
                Err(error) => {
                    outcomes[index].refused = Some(RefusedInvoice {
                        publisher: subscriber.publisher.clone(),
                        reason: format!("the first invoice could not be issued: {error}"),
                    });
                }
            }
            continue;
        }
        if subscriber.last_billed_month.as_deref() == Some(month) {
            if subscriber.last_paid_month.as_deref() == Some(month) {
                outcomes[index].skipped = Some(SkippedInvoice {
                    publisher: subscriber.publisher.clone(),
                    month: month.to_string(),
                });
                continue;
            }
            // Retry the stored-card charge of the invoice an earlier run
            // issued. The same deterministic reference is the idempotency
            // key, so the retry cannot double-charge.
            let (Some(reference), Some(amount_micros)) = (
                subscriber.last_charge_reference.clone(),
                subscriber.last_charge_amount_micros,
            ) else {
                outcomes[index].refused = Some(RefusedInvoice {
                    publisher: subscriber.publisher.clone(),
                    reason: format!(
                        "invoice {month} is recorded as issued but its charge facts are not recorded; it cannot be retried safely"
                    ),
                });
                continue;
            };
            tasks.push(ChargeTask {
                index,
                subscriber: subscriber.clone(),
                month: month.to_string(),
                amount_micros,
                reference,
                issued: None,
                retry: true,
                pre_updated: subscriber.clone(),
            });
            continue;
        }
        let publisher = subscriber.publisher.as_str();
        let Some(customer) = subscriber.billing_profile.clone() else {
            outcomes[index].refused = Some(RefusedInvoice {
                publisher: publisher.to_string(),
                reason:
                    "the subscriber has no customer billing profile recorded; the invoice is refused until one is"
                        .to_string(),
            });
            continue;
        };
        if let Err(error) = customer.validate() {
            outcomes[index].refused = Some(RefusedInvoice {
                publisher: publisher.to_string(),
                reason: format!("the customer billing profile is unusable: {error}"),
            });
            continue;
        }
        let price = match subscriber.tier {
            Tier::Core => WATCH_CORE_PRICE_MICROS,
            Tier::Normal => WATCH_PRICE_MICROS,
        };
        let plan = watch_plan(move || price, PLAN_CURRENCY);
        let period = BillingPeriod {
            year_month: month.to_string(),
            start_unix_ms,
            end_unix_ms,
        };
        let mut invoice = match compute_invoice(publisher, period, &[]) {
            Ok(invoice) => invoice,
            Err(error) => {
                outcomes[index].refused = Some(RefusedInvoice {
                    publisher: publisher.to_string(),
                    reason: format!("the invoice could not be prepared: {error}"),
                });
                continue;
            }
        };
        if let Err(error) = invoice.push_plan_line(plan.monthly_line(1, month)) {
            outcomes[index].refused = Some(RefusedInvoice {
                publisher: publisher.to_string(),
                reason: format!("the watch line could not be attached: {error}"),
            });
            continue;
        }
        let mut prepared = match invoice
            .with_customer(customer)
            .apply_vat_treatment_with_vies(deps.seller, deps.vies)
        {
            Ok(prepared) => prepared,
            Err(error) => {
                outcomes[index].refused = Some(RefusedInvoice {
                    publisher: publisher.to_string(),
                    reason: format!("the VAT treatment was refused: {error}"),
                });
                continue;
            }
        };
        // Mirror the runner exactly: the register assigns the gapless number
        // and persists the invoice, then the lawful document renders from
        // the numbered invoice. This stays serial so numbering is ordered.
        let number = match register.issue(&mut prepared) {
            Ok(number) => {
                restrict_register(deps.register_path);
                number
            }
            Err(error) => {
                outcomes[index].refused = Some(RefusedInvoice {
                    publisher: publisher.to_string(),
                    reason: format!("the register refused the invoice: {error}"),
                });
                continue;
            }
        };
        let document = match prepared.render_vat_invoice(deps.seller) {
            Ok(document) => document,
            Err(error) => {
                outcomes[index].refused = Some(RefusedInvoice {
                    publisher: publisher.to_string(),
                    reason: format!("invoice {number} was issued but could not render: {error}"),
                });
                continue;
            }
        };
        let reference = charge_reference(subscriber, month);
        let mut updated = subscriber.clone();
        updated.last_billed_month = Some(month.to_string());
        updated.last_invoice_number = Some(number);
        updated.last_charge_reference = Some(reference.clone());
        updated.last_charge_amount_micros = Some(prepared.gross_micros);
        updated.last_charge_error = None;
        // Record the issued invoice and its charge facts before the PSP
        // call: if the charge lands but the post-charge row write fails, the
        // next run sees the month as issued and retries the same idempotent
        // reference instead of issuing a second invoice.
        store.update(&updated)?;
        tasks.push(ChargeTask {
            index,
            subscriber: subscriber.clone(),
            month: month.to_string(),
            amount_micros: prepared.gross_micros,
            reference: reference.clone(),
            issued: Some(IssuedInvoice {
                publisher: subscriber.publisher.clone(),
                tier: subscriber.tier,
                number,
                net_micros: prepared.total_micros,
                gross_micros: prepared.gross_micros,
                document,
                charged: false,
                paid_by_first_payment: false,
                charge_reference: Some(reference),
                psp_reference: None,
                charge_error: None,
            }),
            retry: false,
            pre_updated: updated,
        });
    }

    // Phase 2: bounded concurrent PSP calls, persisted in subscriber order.
    for chunk in tasks.chunks(BILL_CHARGE_CONCURRENCY.max(1)) {
        let results = charge_chunk(chunk, &deps).await;
        for (task, outcome) in chunk.iter().zip(results) {
            let mut updated = task.pre_updated.clone();
            let mut issued = task.issued.clone();
            match outcome.charged {
                Some((psp_reference, result_code)) => {
                    record_charge(&mut updated, &task.month, &outcome.reference);
                    if let Some(issued) = issued.as_mut() {
                        issued.charged = true;
                        issued.psp_reference = Some(psp_reference.clone());
                    }
                    outcomes[task.index].charged = Some(ChargedInvoice {
                        publisher: task.subscriber.publisher.clone(),
                        month: task.month.clone(),
                        amount_micros: task.amount_micros,
                        reference: outcome.reference,
                        psp_reference,
                        result_code,
                    });
                }
                None => {
                    let reason = outcome.error.unwrap_or_else(|| {
                        "the stored-card charge did not land; the invoice stays unpaid".to_string()
                    });
                    updated.last_charge_error = Some(reason.clone());
                    if let Some(issued) = issued.as_mut() {
                        issued.charge_error = Some(reason.clone());
                    }
                    let number = updated
                        .last_invoice_number
                        .map(|number| number.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let detail = if task.retry {
                        format!(
                            "invoice {number} for {} was issued earlier but is still unpaid: {reason}",
                            task.month
                        )
                    } else {
                        format!("invoice {number} was issued but not charged: {reason}")
                    };
                    outcomes[task.index].refused = Some(RefusedInvoice {
                        publisher: task.subscriber.publisher.clone(),
                        reason: detail,
                    });
                }
            }
            store.update(&updated)?;
            outcomes[task.index].issued = issued;
        }
    }

    let mut report = BillReport::default();
    for outcome in outcomes {
        if let Some(issued) = outcome.issued {
            report.issued.push(issued);
        }
        if let Some(charged) = outcome.charged {
            report.charged.push(charged);
        }
        if let Some(refused) = outcome.refused {
            report.refused.push(refused);
        }
        if let Some(skipped) = outcome.skipped {
            report.skipped.push(skipped);
        }
    }
    Ok(report)
}

/// Run one chunk of charge tasks with `tokio::join!` so at most
/// [`BILL_CHARGE_CONCURRENCY`] PSP calls overlap without spawning `'static`
/// futures or adding a dependency.
async fn charge_chunk(chunk: &[ChargeTask], deps: &BillDeps<'_>) -> Vec<ChargeOutcome> {
    match chunk.len() {
        1 => vec![charge_task(&chunk[0], deps).await],
        2 => {
            let (a, b) = tokio::join!(charge_task(&chunk[0], deps), charge_task(&chunk[1], deps));
            vec![a, b]
        }
        3 => {
            let (a, b, c) = tokio::join!(
                charge_task(&chunk[0], deps),
                charge_task(&chunk[1], deps),
                charge_task(&chunk[2], deps)
            );
            vec![a, b, c]
        }
        4 => {
            let (a, b, c, d) = tokio::join!(
                charge_task(&chunk[0], deps),
                charge_task(&chunk[1], deps),
                charge_task(&chunk[2], deps),
                charge_task(&chunk[3], deps)
            );
            vec![a, b, c, d]
        }
        _ => {
            let mut outcomes = Vec::with_capacity(chunk.len());
            for task in chunk {
                outcomes.push(charge_task(task, deps).await);
            }
            outcomes
        }
    }
}

async fn charge_task(task: &ChargeTask, deps: &BillDeps<'_>) -> ChargeOutcome {
    charge_stored_card(
        &task.subscriber,
        &task.month,
        task.amount_micros,
        task.reference.clone(),
        deps,
    )
    .await
}

/// The deterministic charge reference for one subscriber-month. It is the
/// Adyen idempotency key, so a retry can never double-charge.
fn charge_reference(subscriber: &Subscriber, month: &str) -> String {
    format!("watch:{}:{month}", subscriber.id)
}

/// The outcome of one charge attempt.
struct ChargeOutcome {
    reference: String,
    charged: Option<(String, String)>,
    error: Option<String>,
}

/// Charge `amount_micros` to the subscriber's stored card. A missing card or
/// a missing Adyen configuration is a typed refusal: the invoice stays
/// issued and the reason is recorded, never silently skipped.
async fn charge_stored_card(
    subscriber: &Subscriber,
    month: &str,
    amount_micros: i64,
    reference: String,
    deps: &BillDeps<'_>,
) -> ChargeOutcome {
    let Some(card) = subscriber
        .stored_payment_method_id
        .as_deref()
        .map(str::trim)
        .filter(|card| !card.is_empty())
    else {
        return ChargeOutcome {
            reference,
            charged: None,
            error: Some(
                "no stored payment method is recorded; the invoice is issued and its charge is refused until a card is stored"
                    .to_string(),
            ),
        };
    };
    let (Some(client), Some(transport)) = (deps.adyen, deps.transport) else {
        return ChargeOutcome {
            reference,
            charged: None,
            error: Some(
                "no Adyen client is configured; the invoice is issued and its charge is refused until checkout credentials are configured"
                    .to_string(),
            ),
        };
    };
    if amount_micros <= 0 || amount_micros.checked_rem(MICROS_PER_MINOR_UNIT) != Some(0) {
        return ChargeOutcome {
            reference,
            charged: None,
            error: Some(format!(
                "the invoiced amount {amount_micros} micros is not a positive exact cent amount"
            )),
        };
    }
    let item = InvoiceLineItem {
        seq: 1,
        experiment_id: "setup-watch".to_string(),
        amount_minor_units: amount_micros / MICROS_PER_MINOR_UNIT,
        currency: PLAN_CURRENCY.to_string(),
        shopper_reference: subscriber.id.clone(),
        reference: reference.clone(),
        description: format!("Apex setup watch: {month}"),
    };
    match client
        .charge_stored_payment_method(transport, &item, card)
        .await
    {
        Ok(receipt) => ChargeOutcome {
            reference,
            charged: Some((receipt.psp_reference, receipt.result_code)),
            error: None,
        },
        Err(error) => ChargeOutcome {
            reference,
            charged: None,
            error: Some(error.to_string()),
        },
    }
}

/// Record an accepted charge on the subscriber row.
fn record_charge(subscriber: &mut Subscriber, month: &str, reference: &str) {
    subscriber.last_paid_month = Some(month.to_string());
    subscriber.last_charge_reference = Some(reference.to_string());
    subscriber.last_charge_error = None;
}

/// Issue the invoice for a subscriber's accepted first payment.
///
/// The subscriber must be active, have an accepted payment (`paid_at`) and
/// carry a [`crate::model::BillingProfile`]. The invoice is issued through
/// the same register as the monthly pass, so its number is gapless, and it is
/// rendered through the billing crate with the stored customer profile and
/// the injected VAT register. `amount_micros` is the accepted first payment's
/// amount (the invoice's net plan line) and `currency` its ISO currency; both
/// are recorded on the subscriber.
///
/// Idempotent per subscriber/period: once the first invoice exists, a replay
/// for the same `month` returns the recorded invoice without a register
/// question, a second number or a second row. A first invoice for a different
/// period, or a subscriber already billed for that month by the monthly pass,
/// is a typed refusal.
pub async fn issue_first_invoice(
    store: &SubscriberStore,
    reference: &str,
    month: &str,
    amount_micros: i64,
    currency: &str,
    deps: &BillDeps<'_>,
) -> Result<IssuedInvoice, WatchError> {
    // A one-off call opens its own register and VIES cache. The monthly run
    // shares both across every subscriber through
    // [`issue_first_invoice_with_register`], so a run performs one open and
    // one register question per distinct legal identity, not one per row.
    let register = open_register(deps.register_path)?;
    let vies = apex_billing::CachedVies::new(deps.vies);
    let deps = BillDeps {
        seller: deps.seller,
        register_path: deps.register_path,
        vies: &vies,
        adyen: deps.adyen,
        transport: deps.transport,
    };
    issue_first_invoice_with_register(
        store,
        reference,
        month,
        amount_micros,
        currency,
        &deps,
        &register,
    )
    .await
}

/// [`issue_first_invoice`] over an already-open register and the caller's
/// (already cached) VIES seam. The monthly pass uses this so the register is
/// opened once per run and one VIES cache is shared by every subscriber.
#[allow(clippy::too_many_arguments)]
async fn issue_first_invoice_with_register(
    store: &SubscriberStore,
    reference: &str,
    month: &str,
    amount_micros: i64,
    currency: &str,
    deps: &BillDeps<'_>,
    register: &InvoiceRegister,
) -> Result<IssuedInvoice, WatchError> {
    let Some((start_unix_ms, end_unix_ms)) = month_bounds(month) else {
        return Err(WatchError::Malformed(format!(
            "month '{month}' is not a YYYY-MM calendar month"
        )));
    };
    let _guard = FIRST_INVOICE_LOCK
        .lock()
        .map_err(|_| WatchError::StoreCorrupt {
            detail: "the first-invoice lock is poisoned".to_string(),
        })?;
    let reference = reference.trim();
    let subscriber = store
        .get(reference)?
        .ok_or_else(|| WatchError::UnknownSubscriber {
            reference: reference.to_string(),
        })?;
    if subscriber.status == crate::model::Status::Canceled {
        return Err(WatchError::NotBillable {
            reference: subscriber.id,
            status: crate::model::Status::Canceled,
        });
    }
    if !subscriber.is_active() {
        return Err(WatchError::NotBillable {
            reference: subscriber.id,
            status: subscriber.status,
        });
    }
    if subscriber.paid_at.is_none() {
        return Err(WatchError::Malformed(format!(
            "subscriber '{}' has no accepted first payment; no first invoice can be issued",
            subscriber.id
        )));
    }
    let customer =
        subscriber
            .billing_profile
            .clone()
            .ok_or_else(|| WatchError::BillingProfileMissing {
                reference: subscriber.id.clone(),
            })?;

    if let Some(number) = subscriber.first_invoice_number {
        if subscriber.first_invoice_month.as_deref() != Some(month) {
            return Err(WatchError::Malformed(format!(
                "subscriber '{}' already has first invoice {number} for {}; first invoices are per period",
                subscriber.id,
                subscriber
                    .first_invoice_month
                    .as_deref()
                    .unwrap_or("an unknown month")
            )));
        }
        return Ok(IssuedInvoice {
            publisher: subscriber.publisher.clone(),
            tier: subscriber.tier,
            number,
            net_micros: subscriber.first_invoice_net_micros.unwrap_or(amount_micros),
            gross_micros: subscriber
                .first_invoice_gross_micros
                .unwrap_or(amount_micros),
            document: subscriber
                .first_invoice_document
                .clone()
                .unwrap_or_default(),
            charged: false,
            paid_by_first_payment: true,
            charge_reference: subscriber.last_charge_reference.clone(),
            psp_reference: None,
            charge_error: None,
        });
    }
    if subscriber.last_billed_month.as_deref() == Some(month) {
        return Err(WatchError::Malformed(format!(
            "subscriber '{}' was already billed for {month}; the first invoice would duplicate the period",
            subscriber.id
        )));
    }
    if amount_micros <= 0 {
        return Err(WatchError::Malformed(format!(
            "the first payment amount {amount_micros} micros is not positive"
        )));
    }
    if amount_micros.checked_rem(MICROS_PER_MINOR_UNIT) != Some(0) {
        return Err(WatchError::Malformed(format!(
            "the first payment amount {amount_micros} micros is not an exact cent amount"
        )));
    }
    let currency = currency.trim().to_ascii_uppercase();
    if currency != PLAN_CURRENCY {
        return Err(WatchError::Malformed(format!(
            "the first payment currency '{currency}' is not the plan currency {PLAN_CURRENCY}"
        )));
    }

    let period = BillingPeriod {
        year_month: month.to_string(),
        start_unix_ms,
        end_unix_ms,
    };
    let plan = watch_plan(move || amount_micros, &currency);
    let mut invoice = compute_invoice(&subscriber.publisher, period, &[])?;
    invoice.push_plan_line(plan.monthly_line(1, month))?;
    let mut prepared = invoice
        .with_customer(customer)
        .apply_vat_treatment_with_vies(deps.seller, deps.vies)?;
    let number = register.issue(&mut prepared)?;
    restrict_register(deps.register_path);
    let document = prepared.render_vat_invoice(deps.seller)?;
    let reference = format!("watch:{}:{month}", subscriber.id);
    let mut updated = subscriber.clone();
    updated.first_payment_amount_micros = Some(amount_micros);
    updated.first_payment_currency = Some(currency);
    updated.first_invoice_number = Some(number);
    updated.first_invoice_month = Some(month.to_string());
    updated.first_invoice_net_micros = Some(prepared.total_micros);
    updated.first_invoice_gross_micros = Some(prepared.gross_micros);
    updated.first_invoice_document = Some(document.clone());
    updated.last_billed_month = Some(month.to_string());
    updated.last_paid_month = Some(month.to_string());
    updated.last_invoice_number = Some(number);
    updated.last_charge_reference = Some(reference.clone());
    updated.last_charge_amount_micros = Some(amount_micros);
    updated.last_charge_error = None;
    store.update(&updated)?;
    Ok(IssuedInvoice {
        publisher: subscriber.publisher,
        tier: subscriber.tier,
        number,
        net_micros: prepared.total_micros,
        gross_micros: prepared.gross_micros,
        document,
        charged: false,
        paid_by_first_payment: true,
        charge_reference: Some(reference),
        psp_reference: None,
        charge_error: None,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;
    use std::sync::Mutex;

    use apex_billing::{CustomerProfile, ViesCheck, ViesClient, ViesError};
    use apex_connectors::transport::{
        HttpBody, HttpMethod, HttpRequest, HttpResponse, Transport, TransportError,
    };

    use super::*;

    const OCT_START: i64 = 20_727 * 86_400_000;

    fn seller() -> BillingProfile {
        BillingProfile {
            legal_name: "Apex Media OÜ".into(),
            address_lines: vec!["Sepapaja tn 6".into()],
            country: "EE".into(),
            vat_number: "EE123456789".into(),
            vat_rate_bp: 2000,
        }
    }

    fn customer(legal_name: &str) -> CustomerProfile {
        CustomerProfile {
            legal_name: legal_name.into(),
            address_lines: vec!["Telliskivi 1".into()],
            country: "EE".into(),
            vat_number: Some("EE111111111".into()),
        }
    }

    fn cross_border_customer(legal_name: &str) -> CustomerProfile {
        CustomerProfile {
            legal_name: legal_name.into(),
            address_lines: vec!["Friedrichstrasse 1".into()],
            country: "DE".into(),
            vat_number: Some("DE123456789".into()),
        }
    }

    fn register_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("billing").join("invoices.jsonl")
    }

    /// The tests always inject a VAT-register stub; no test reaches the network.
    #[derive(Debug)]
    struct DomesticVies;

    impl ViesClient for DomesticVies {
        fn check_vat_number(&self, country: &str, number: &str) -> Result<ViesCheck, ViesError> {
            Ok(ViesCheck::Valid {
                evidence: format!("VIES confirmation: {country}{number} is valid"),
                vat_number: number.to_string(),
            })
        }
    }

    /// A VAT-register stub that counts the register questions.
    #[derive(Debug, Default)]
    struct CountingVies {
        calls: Mutex<u32>,
    }

    impl CountingVies {
        fn calls(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    impl ViesClient for CountingVies {
        fn check_vat_number(&self, country: &str, number: &str) -> Result<ViesCheck, ViesError> {
            *self.calls.lock().unwrap() += 1;
            Ok(ViesCheck::Valid {
                evidence: format!("VIES confirmation: {country}{number} is valid"),
                vat_number: number.to_string(),
            })
        }
    }

    /// A scripted Adyen transport: every request is recorded and answered
    /// from a queue.
    #[derive(Debug)]
    struct ScriptedTransport {
        responses: Mutex<std::collections::VecDeque<HttpResponse>>,
        requests: Mutex<Vec<HttpRequest>>,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<HttpResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<HttpRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Transport for ScriptedTransport {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.requests.lock().unwrap().push(req);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| TransportError::Network("no scripted response".to_string()))
        }
    }

    /// A transport that makes the subscriber log read-only at the moment the
    /// charge request is made, so the post-charge row write fails.
    #[cfg(unix)]
    #[derive(Debug)]
    struct ReadOnlyAfterCharge {
        subscribers: PathBuf,
    }

    #[cfg(unix)]
    #[async_trait::async_trait]
    impl Transport for ReadOnlyAfterCharge {
        async fn request(&self, _req: HttpRequest) -> Result<HttpResponse, TransportError> {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.subscribers, std::fs::Permissions::from_mode(0o444))
                .map_err(|error| TransportError::Network(error.to_string()))?;
            Ok(accepted())
        }
    }

    fn response(result_code: &str, psp_reference: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: serde_json::to_vec(&serde_json::json!({
                "pspReference": psp_reference,
                "resultCode": result_code,
            }))
            .unwrap(),
        }
    }

    fn accepted() -> HttpResponse {
        response("Authorised", "PSP-ACCEPTED")
    }

    fn adyen() -> AdyenClient {
        AdyenClient::new(
            "key",
            "DogfoodECOM",
            b"hmac".to_vec(),
            "https://checkout.test",
        )
        .expect("test client")
    }

    fn with_billing(subscriber: &mut Subscriber) {
        subscriber.billing_profile = Some(customer("Buyer OÜ"));
        subscriber.stored_payment_method_id = Some("card-1".to_string());
    }

    fn active_with_profile(
        store: &SubscriberStore,
        domain: &str,
        email: &str,
        now: i64,
    ) -> Subscriber {
        let pending = store
            .subscribe_pending(
                now,
                &crate::model::SubscribeRequest {
                    domain: domain.to_string(),
                    email: email.to_string(),
                    accounts: String::new(),
                    apps: vec!["app".to_string()],
                },
            )
            .expect("pending");
        store
            .confirm_email(now + 1, &pending.confirmation_token)
            .expect("confirm");
        store
            .activate(now + 2, &pending.reference)
            .expect("activate");
        store
            .set_billing_profile(&pending.reference, &customer("Buyer OÜ"), now + 3)
            .expect("profile");
        let mut subscriber = store.get(&pending.reference).expect("get").expect("row");
        subscriber.stored_payment_method_id = Some("card-1".to_string());
        store.update(&subscriber).expect("card");
        store.get(&pending.reference).expect("get").expect("row")
    }

    #[tokio::test]
    async fn first_invoices_are_gapless_replay_without_a_second_number_and_record_the_payment() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let first = active_with_profile(&store, "first.example", "ops@first.example", 10);
        let second = active_with_profile(&store, "second.example", "ops@second.example", 20);
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport = ScriptedTransport::new(Vec::new());
        let adyen = adyen();
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };

        let first_invoice =
            issue_first_invoice(&store, &first.id, "2026-10", 99_000_000, "USD", &deps)
                .await
                .expect("first invoice");
        assert_eq!(first_invoice.number, 1, "the register starts at one");
        assert!(
            !first_invoice.charged,
            "a first payment is not a stored-card charge"
        );
        assert!(first_invoice.paid_by_first_payment);
        assert!(first_invoice.document.contains("VAT invoice No. 1"));
        assert_eq!(first_invoice.net_micros, 99_000_000);
        assert_eq!(first_invoice.gross_micros, 118_800_000);

        // A replay returns the same invoice and consumes no second number.
        let replay = issue_first_invoice(&store, &first.id, "2026-10", 99_000_000, "USD", &deps)
            .await
            .expect("replay");
        assert_eq!(replay.number, first_invoice.number);
        assert_eq!(replay.document, first_invoice.document);
        let register_raw = std::fs::read_to_string(&register_path).unwrap();
        assert_eq!(register_raw.lines().count(), 1, "still one invoice");

        // A second subscriber issued in between gets the next gapless number.
        let second_invoice =
            issue_first_invoice(&store, &second.id, "2026-10", 199_000_000, "USD", &deps)
                .await
                .expect("second invoice");
        assert_eq!(second_invoice.number, 2);
        assert_eq!(
            std::fs::read_to_string(&register_path)
                .unwrap()
                .lines()
                .count(),
            2
        );

        let row = store.get(&first.id).unwrap().unwrap();
        assert_eq!(row.first_payment_amount_micros, Some(99_000_000));
        assert_eq!(row.first_payment_currency.as_deref(), Some("USD"));
        assert_eq!(row.first_invoice_number, Some(1));
        assert_eq!(row.first_invoice_month.as_deref(), Some("2026-10"));
        assert_eq!(row.last_paid_month.as_deref(), Some("2026-10"));

        // The monthly pass sees the first payment's month as paid: no second
        // invoice and no charge request.
        let report = run_bill(&store, "2026-10", &deps).await.expect("monthly");
        assert_eq!(report.issued.len(), 0);
        assert_eq!(report.charged.len(), 0);
        assert_eq!(report.skipped.len(), 2);
        assert!(
            transport.requests().is_empty(),
            "the first payment is never charged again"
        );
    }

    #[tokio::test]
    async fn the_next_month_charges_the_stored_card_with_the_exact_shape() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let subscriber = active_with_profile(&store, "renew.example", "ops@renew.example", 10);
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let first_transport = ScriptedTransport::new(Vec::new());
        let adyen = adyen();
        issue_first_invoice(
            &store,
            &subscriber.id,
            "2026-10",
            99_000_000,
            "USD",
            &BillDeps {
                seller: &seller,
                register_path: &register_path,
                vies: &vies,
                adyen: Some(&adyen),
                transport: Some(&first_transport),
            },
        )
        .await
        .expect("first invoice");

        let transport = ScriptedTransport::new(vec![accepted()]);
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };
        let report = run_bill(&store, "2026-11", &deps).await.expect("monthly");
        assert_eq!(report.refused.len(), 0, "{:#?}", report.refused);
        assert_eq!(report.issued.len(), 1);
        assert_eq!(report.charged.len(), 1);
        let issued = &report.issued[0];
        assert_eq!(issued.number, 2, "numbering continues gaplessly");
        assert!(issued.charged);
        assert_eq!(issued.net_micros, 99_000_000);
        assert_eq!(issued.gross_micros, 118_800_000);
        assert!(issued.document.contains("VAT invoice No. 2"));

        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://checkout.test/v71/payments");
        let body = match request.body.as_ref() {
            Some(HttpBody::Json(body)) => body,
            other => panic!("expected a JSON body, got {other:?}"),
        };
        assert_eq!(body["shopperReference"], subscriber.id.as_str());
        assert_eq!(body["storedPaymentMethodId"], "card-1");
        assert_eq!(body["shopperInteraction"], "ContAuth");
        assert_eq!(body["recurringProcessingModel"], "Subscription");
        assert_eq!(
            body["reference"],
            format!("watch:{}:2026-11", subscriber.id).as_str()
        );
        assert_eq!(body["amount"]["value"], 11_880);
        assert_eq!(body["amount"]["currency"], "USD");
        assert_eq!(
            std::fs::read_to_string(&register_path)
                .unwrap()
                .lines()
                .count(),
            2,
            "the first invoice and the renewal are the only register lines"
        );
    }

    #[tokio::test]
    async fn the_monthly_pass_issues_an_uninvoiced_first_payment_without_charging_it_again() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let mut subscriber = store
            .subscribe(
                OCT_START + 1_000,
                "activated.example",
                "ops@activated.example",
                "",
                false,
            )
            .expect("subscribe")
            .subscriber;
        with_billing(&mut subscriber);
        store.update(&subscriber).expect("update");
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport = ScriptedTransport::new(Vec::new());
        let adyen = adyen();
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };

        let report = run_bill(&store, "2026-10", &deps).await.expect("run");
        assert_eq!(report.refused.len(), 0, "{:#?}", report.refused);
        assert_eq!(report.issued.len(), 1);
        assert_eq!(report.issued[0].number, 1);
        assert!(!report.issued[0].charged, "no stored-card charge ran");
        assert!(report.issued[0].paid_by_first_payment);
        assert_eq!(report.issued[0].net_micros, 99_000_000);
        assert_eq!(
            report.charged.len(),
            0,
            "the first payment was collected by the payment link, not charged again"
        );
        assert!(
            transport.requests().is_empty(),
            "no card charge for an already-collected first payment"
        );
        let row = store.get(&subscriber.id).unwrap().unwrap();
        assert_eq!(row.first_payment_amount_micros, Some(99_000_000));
        assert_eq!(row.first_invoice_number, Some(1));
        assert_eq!(row.last_paid_month.as_deref(), Some("2026-10"));

        // The next run of the same month is a skip, not a second charge.
        let replay = run_bill(&store, "2026-10", &deps).await.expect("replay");
        assert_eq!(replay.issued.len(), 0);
        assert_eq!(replay.charged.len(), 0);
        assert_eq!(replay.skipped.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&register_path)
                .unwrap()
                .lines()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_first_invoice_without_a_profile_is_the_documented_typed_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let subscriber = store
            .subscribe(10, "noprofile.example", "ops@noprofile.example", "", false)
            .expect("subscribe")
            .subscriber;
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let adyen = adyen();
        let transport = ScriptedTransport::new(Vec::new());
        let error = issue_first_invoice(
            &store,
            &subscriber.id,
            "2026-10",
            99_000_000,
            "USD",
            &BillDeps {
                seller: &seller,
                register_path: &register_path,
                vies: &vies,
                adyen: Some(&adyen),
                transport: Some(&transport),
            },
        )
        .await
        .expect_err("no profile");
        assert!(
            error.to_string().contains("no customer billing profile"),
            "{error}"
        );
        assert!(!register_path.exists(), "a refusal consumes no number");
    }

    #[test]
    fn month_bounds_cover_exact_utc_calendar_months() {
        let (start, end) = month_bounds("2026-10").expect("bounds");
        assert_eq!(start, OCT_START);
        assert_eq!(end - start, 31 * 86_400_000);
        assert_eq!(current_month(start), "2026-10");
        assert_eq!(current_month(start - 1), "2026-09");
        let (december_start, january_end) = month_bounds("2026-12").expect("december");
        assert_eq!(january_end - december_start, 31 * 86_400_000);
        assert_eq!(current_month(january_end), "2027-01");
        let (feb_start, feb_end) = month_bounds("2024-02").expect("leap");
        assert_eq!(feb_end - feb_start, 29 * 86_400_000);
        for bad in ["2026-13", "2026-1", "26-10", "2026", "october", ""] {
            assert!(month_bounds(bad).is_none(), "accepted {bad:?}");
        }
    }

    #[tokio::test]
    async fn golden_invoices_for_normal_and_core_share_gapless_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let normal = store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe")
            .subscriber;
        let core = store
            .subscribe(11, "bigport.example", "ops@bigport.example", "", true)
            .expect("subscribe")
            .subscriber;
        let mut normal = normal;
        with_billing(&mut normal);
        let normal_id = normal.id.clone();
        store.update(&normal).expect("update");
        let mut core = core;
        with_billing(&mut core);
        store.update(&core).expect("update");

        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport = ScriptedTransport::new(vec![accepted(), accepted()]);
        let adyen = adyen();
        let report = run_bill(
            &store,
            "2026-10",
            &BillDeps {
                seller: &seller,
                register_path: &register_path,
                vies: &vies,
                adyen: Some(&adyen),
                transport: Some(&transport),
            },
        )
        .await
        .expect("run");
        assert_eq!(report.refused.len(), 0, "{:#?}", report.refused);
        assert_eq!(report.skipped.len(), 0);
        assert_eq!(report.issued.len(), 2);
        assert_eq!(report.charged.len(), 2);
        assert!(report.issued.iter().all(|row| row.charged));
        let numbers: Vec<u64> = report.issued.iter().map(|row| row.number).collect();
        assert_eq!(numbers, vec![1, 2], "consecutive and gap-free");
        let normal_row = report
            .issued
            .iter()
            .find(|row| row.publisher == "northstar-example")
            .expect("normal row");
        assert_eq!(normal_row.tier, Tier::Normal);
        assert_eq!(normal_row.net_micros, 99_000_000);
        assert_eq!(normal_row.gross_micros, 118_800_000);
        assert!(normal_row.document.contains("setup-watch:2026-10"));
        assert!(normal_row.document.contains("VAT invoice No. 1"));
        let core_row = report
            .issued
            .iter()
            .find(|row| row.publisher == "bigport-example")
            .expect("core row");
        assert_eq!(core_row.tier, Tier::Core);
        assert_eq!(core_row.net_micros, 199_000_000);
        assert_eq!(core_row.gross_micros, 238_800_000);
        assert!(core_row.document.contains("VAT invoice No. 2"));

        // The stored-method charge has the exact audited shape: the
        // subscriber is the shopperReference, the card is charged ContAuth
        // for a Subscription, and the reference is the deterministic key.
        let requests = transport.requests();
        assert_eq!(requests.len(), 2);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://checkout.test/v71/payments");
        let body = match request.body.as_ref() {
            Some(HttpBody::Json(body)) => body,
            other => panic!("expected a JSON body, got {other:?}"),
        };
        assert_eq!(body["shopperReference"], normal_id.as_str());
        assert_eq!(body["storedPaymentMethodId"], "card-1");
        assert_eq!(body["shopperInteraction"], "ContAuth");
        assert_eq!(body["recurringProcessingModel"], "Subscription");
        assert_eq!(
            body["reference"],
            format!("watch:{normal_id}:2026-10").as_str()
        );
        assert_eq!(body["amount"]["value"], 11_880);
        assert_eq!(body["amount"]["currency"], "USD");

        // The register holds exactly the two numbered invoices.
        let raw = std::fs::read_to_string(&register_path).unwrap();
        let lines: Vec<serde_json::Value> = raw
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["number"], 1);
        assert_eq!(lines[1]["number"], 2);
        assert_eq!(lines[0]["publisherId"], "northstar-example");

        // Both rows remember the billed and paid month.
        for subscriber in store.list().expect("list") {
            assert_eq!(subscriber.last_billed_month.as_deref(), Some("2026-10"));
            assert_eq!(subscriber.last_paid_month.as_deref(), Some("2026-10"));
            assert_eq!(subscriber.last_charge_error, None);
        }
    }

    #[tokio::test]
    async fn one_customer_vat_number_is_confirmed_once_per_billing_run() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        for (port, domain) in [(10, "alpha.example"), (11, "beta.example")] {
            let mut subscriber = store
                .subscribe(port, domain, &format!("ops@{domain}"), "", false)
                .expect("subscribe")
                .subscriber;
            subscriber.billing_profile = Some(cross_border_customer("Shared Buyer GmbH"));
            subscriber.stored_payment_method_id = Some("card-1".to_string());
            store.update(&subscriber).expect("update");
        }
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = CountingVies::default();
        let transport = ScriptedTransport::new(vec![accepted(), accepted()]);
        let adyen = adyen();
        let report = run_bill(
            &store,
            "2026-10",
            &BillDeps {
                seller: &seller,
                register_path: &register_path,
                vies: &vies,
                adyen: Some(&adyen),
                transport: Some(&transport),
            },
        )
        .await
        .expect("run");
        assert_eq!(report.issued.len(), 2);
        assert_eq!(report.charged.len(), 2);
        assert_eq!(
            vies.calls(),
            1,
            "two invoices for the same legal customer query VIES once per run"
        );
        // The evidence is still persisted on both invoices.
        let raw = std::fs::read_to_string(&register_path).unwrap();
        let lines: Vec<serde_json::Value> = raw
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            assert_eq!(line["vatTreatment"], "reverseCharge");
            assert!(line["viesEvidence"]["detail"]
                .as_str()
                .unwrap()
                .contains("confirmation"));
        }
    }

    #[tokio::test]
    async fn a_missing_profile_refuses_without_consuming_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe");
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport = ScriptedTransport::new(Vec::new());
        let adyen = adyen();
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };
        let report = run_bill(&store, "2026-10", &deps).await.expect("run");
        assert_eq!(report.issued.len(), 0);
        assert_eq!(report.refused.len(), 1);
        assert!(report.refused[0]
            .reason
            .contains("no customer billing profile"));
        assert!(
            !register_path.exists(),
            "a refusal must not create the register"
        );
        assert_eq!(
            store.list().expect("list")[0].last_billed_month,
            None,
            "a refused subscriber records no billed month"
        );

        // A malformed profile inside the row is also refused before issue.
        let mut broken = store.list().expect("list").remove(0);
        broken.billing_profile = Some(CustomerProfile {
            legal_name: " ".into(),
            address_lines: vec!["x".into()],
            country: "EE".into(),
            vat_number: None,
        });
        store.update(&broken).expect("update");
        let report = run_bill(&store, "2026-10", &deps).await.expect("run");
        assert_eq!(report.refused.len(), 1);
        assert!(report.refused[0].reason.contains("unusable"));
        assert!(!register_path.exists());
    }

    #[tokio::test]
    async fn a_subscriber_without_a_stored_card_gets_the_line_and_a_typed_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let mut subscriber = store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe")
            .subscriber;
        subscriber.billing_profile = Some(customer("Buyer OÜ"));
        subscriber.stored_payment_method_id = None;
        store.update(&subscriber).expect("update");
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport = ScriptedTransport::new(Vec::new());
        let adyen = adyen();
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };
        let report = run_bill(&store, "2026-10", &deps).await.expect("run");
        assert_eq!(report.issued.len(), 1, "the plan line is written");
        assert!(!report.issued[0].charged);
        assert_eq!(report.charged.len(), 0);
        assert!(transport.requests().is_empty(), "no card, no charge");
        assert_eq!(report.refused.len(), 1);
        assert!(
            report.refused[0]
                .reason
                .contains("no stored payment method"),
            "{}",
            report.refused[0].reason
        );
        let raw = std::fs::read_to_string(&register_path).unwrap();
        assert!(raw.contains("setup-watch:2026-10"), "{raw}");
        let row = store.list().expect("list").remove(0);
        assert_eq!(row.last_billed_month.as_deref(), Some("2026-10"));
        assert_eq!(row.last_paid_month, None);
        assert!(row
            .last_charge_error
            .unwrap()
            .contains("no stored payment method"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_charge_that_lands_before_a_failed_row_write_never_reissues_the_invoice() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let mut subscriber = store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe")
            .subscriber;
        with_billing(&mut subscriber);
        store.update(&subscriber).expect("update");
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let adyen = adyen();
        let subscribers_path = store.subscribers_path();
        let transport = ReadOnlyAfterCharge {
            subscribers: subscribers_path.clone(),
        };
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };
        assert!(
            run_bill(&store, "2026-10", &deps).await.is_err(),
            "the post-charge row write fails"
        );
        // The issued invoice and its charge facts were recorded before the
        // charge, so the next run retries the same reference.
        let row = store.list().expect("list").remove(0);
        assert_eq!(row.last_billed_month.as_deref(), Some("2026-10"));
        assert_eq!(row.last_paid_month, None);
        assert!(row.last_charge_reference.is_some());
        assert!(row.last_charge_amount_micros.is_some());
        std::fs::set_permissions(&subscribers_path, std::fs::Permissions::from_mode(0o600))
            .unwrap();

        let retry_transport = ScriptedTransport::new(vec![accepted()]);
        let retry_deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&retry_transport),
        };
        let report = run_bill(&store, "2026-10", &retry_deps)
            .await
            .expect("retry");
        assert_eq!(report.issued.len(), 0, "the invoice is not re-issued");
        assert_eq!(report.charged.len(), 1, "the same charge is retried");
        assert_eq!(
            std::fs::read_to_string(&register_path)
                .unwrap()
                .lines()
                .count(),
            1,
            "one invoice, not two"
        );
        let row = store.list().expect("list").remove(0);
        assert_eq!(row.last_paid_month.as_deref(), Some("2026-10"));
    }

    #[tokio::test]
    async fn a_rerun_of_the_same_month_bills_nothing_twice() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let mut subscriber = store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe")
            .subscriber;
        with_billing(&mut subscriber);
        store.update(&subscriber).expect("update");
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport = ScriptedTransport::new(vec![accepted()]);
        let adyen = adyen();
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };
        let first = run_bill(&store, "2026-10", &deps).await.expect("run");
        assert_eq!(first.issued.len(), 1);
        assert_eq!(first.charged.len(), 1);
        let second = run_bill(&store, "2026-10", &deps).await.expect("run");
        assert_eq!(second.issued.len(), 0);
        assert_eq!(second.charged.len(), 0);
        assert_eq!(second.skipped.len(), 1);
        assert_eq!(second.skipped[0].month, "2026-10");
        assert_eq!(transport.requests().len(), 1, "charged once");
        let raw = std::fs::read_to_string(&register_path).unwrap();
        assert_eq!(raw.lines().count(), 1, "still exactly one invoice");
    }

    #[tokio::test]
    async fn an_unpaid_invoice_retries_the_same_charge_without_reissuing() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let mut subscriber = store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe")
            .subscriber;
        with_billing(&mut subscriber);
        store.update(&subscriber).expect("update");
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport =
            ScriptedTransport::new(vec![response("Refused", "PSP-REFUSED"), accepted()]);
        let adyen = adyen();
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };
        let first = run_bill(&store, "2026-10", &deps).await.expect("run");
        assert_eq!(first.issued.len(), 1);
        assert_eq!(first.charged.len(), 0);
        assert_eq!(first.refused.len(), 1);
        let row = store.list().expect("list").remove(0);
        assert_eq!(row.last_billed_month.as_deref(), Some("2026-10"));
        assert_eq!(row.last_paid_month, None);
        assert!(row.last_charge_error.unwrap().contains("refused"));

        let second = run_bill(&store, "2026-10", &deps).await.expect("retry");
        assert_eq!(second.issued.len(), 0, "the invoice is not re-issued");
        assert_eq!(second.charged.len(), 1, "the charge is retried");
        assert_eq!(
            second.charged[0].reference,
            format!("watch:{}:2026-10", row.id)
        );
        let requests = transport.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0]
                .body
                .as_ref()
                .and_then(HttpBody::as_json)
                .unwrap()["reference"],
            requests[1]
                .body
                .as_ref()
                .and_then(HttpBody::as_json)
                .unwrap()["reference"],
            "the retry reuses the deterministic reference"
        );
        let row = store.list().expect("list").remove(0);
        assert_eq!(row.last_paid_month.as_deref(), Some("2026-10"));
        assert_eq!(row.last_charge_error, None);
        assert_eq!(
            std::fs::read_to_string(&register_path)
                .unwrap()
                .lines()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn canceled_subscribers_are_not_billed_next_month() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let first = store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe");
        let second = store
            .subscribe(11, "bigport.example", "ops@bigport.example", "", true)
            .expect("subscribe");
        for mut subscriber in [first.subscriber.clone(), second.subscriber.clone()] {
            with_billing(&mut subscriber);
            store.update(&subscriber).expect("update");
        }
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport = ScriptedTransport::new(vec![accepted(), accepted(), accepted()]);
        let adyen = adyen();
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };
        let october = run_bill(&store, "2026-10", &deps).await.expect("run");
        assert_eq!(october.issued.len(), 2);

        // Cancel the first subscriber with its token, then bill November.
        let token = first.cancel_token.clone().expect("token");
        let canceled = store.cancel(&token, 20).expect("cancel").expect("match");
        assert!(!canceled.is_active());
        let november = run_bill(&store, "2026-11", &deps).await.expect("run");
        assert_eq!(november.issued.len(), 1, "the canceled row is skipped");
        assert_eq!(november.issued[0].publisher, "bigport-example");
        assert_eq!(november.issued[0].number, 3, "numbering continues");
        let folded = store.list().expect("list");
        let canceled_row = folded
            .iter()
            .find(|row| row.publisher == "northstar-example")
            .expect("canceled row");
        assert_eq!(canceled_row.last_billed_month.as_deref(), Some("2026-10"));
        let active_row = folded
            .iter()
            .find(|row| row.publisher == "bigport-example")
            .expect("active row");
        assert_eq!(active_row.last_billed_month.as_deref(), Some("2026-11"));
        assert_eq!(
            std::fs::read_to_string(&register_path)
                .unwrap()
                .lines()
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn one_run_opens_one_register_and_asks_one_vies_question_per_identity() {
        for subscribers in [16usize, 32, 64] {
            let dir = tempfile::tempdir().unwrap();
            let store = SubscriberStore::open(dir.path());
            let (oct_start, _) = month_bounds("2026-10").expect("bounds");
            for index in 0..subscribers {
                let subscriber = active_with_profile(
                    &store,
                    &format!("shared{index}.example"),
                    &format!("ops{index}@shared.example"),
                    oct_start + 1_000 + index as i64,
                );
                let mut row = subscriber;
                row.billing_profile = Some(cross_border_customer("Shared Buyer GmbH"));
                store.update(&row).expect("shared profile");
            }
            let seller = seller();
            let register_path = register_path(&dir);
            let vies = CountingVies::default();
            let transport = ScriptedTransport::new(Vec::new());
            let adyen = adyen();
            reset_register_opens();
            let report = run_bill(
                &store,
                "2026-10",
                &BillDeps {
                    seller: &seller,
                    register_path: &register_path,
                    vies: &vies,
                    adyen: Some(&adyen),
                    transport: Some(&transport),
                },
            )
            .await
            .expect("run");
            println!(
                "subscribers={subscribers} register_opens={} vies_calls={} issued={}",
                register_open_count(),
                vies.calls(),
                report.issued.len()
            );
            assert_eq!(report.issued.len(), subscribers);
            assert_eq!(report.issued[0].number, 1);
            assert_eq!(
                report.issued[subscribers - 1].number,
                subscribers as u64,
                "numbers stay gapless"
            );
            assert_eq!(
                register_open_count(),
                1,
                "subscribers={subscribers}: the run opens the register once"
            );
            assert_eq!(
                vies.calls(),
                1,
                "subscribers={subscribers}: one legal identity is one register question"
            );
            assert!(
                transport.requests().is_empty(),
                "the accepted first payment is never charged again"
            );
            for row in store.list().expect("list") {
                assert!(row.first_invoice_number.is_some());
                assert!(row.first_payment_amount_micros.is_some());
            }
        }
    }

    /// A PSP transport that takes 200 ms per charge and records the
    /// observed in-flight overlap. One reference can be scripted to fail, so
    /// a failure's isolation from the rest of the run is observable.
    #[derive(Debug)]
    struct SlowTransport {
        delay: std::time::Duration,
        fail_reference: Option<String>,
        requests: Mutex<Vec<HttpRequest>>,
        in_flight: std::sync::atomic::AtomicUsize,
        max_in_flight: std::sync::atomic::AtomicUsize,
    }

    impl SlowTransport {
        fn new(delay: std::time::Duration, fail_reference: Option<String>) -> Self {
            Self {
                delay,
                fail_reference,
                requests: Mutex::new(Vec::new()),
                in_flight: std::sync::atomic::AtomicUsize::new(0),
                max_in_flight: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn requests(&self) -> Vec<HttpRequest> {
            self.requests.lock().unwrap().clone()
        }

        fn max_in_flight(&self) -> usize {
            self.max_in_flight.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    fn request_reference(request: &HttpRequest) -> Option<&str> {
        request
            .body
            .as_ref()
            .and_then(HttpBody::as_json)
            .and_then(|body| body["reference"].as_str())
    }

    #[async_trait::async_trait]
    impl Transport for SlowTransport {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            let in_flight = self
                .in_flight
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            self.max_in_flight
                .fetch_max(in_flight, std::sync::atomic::Ordering::SeqCst);
            let reference = request_reference(&req).map(str::to_string);
            self.requests.lock().unwrap().push(req);
            tokio::time::sleep(self.delay).await;
            self.in_flight
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if reference.is_some() && reference == self.fail_reference {
                return Ok(response("Refused", "PSP-REFUSED"));
            }
            Ok(accepted())
        }
    }

    fn many_renewals(count: usize) -> (tempfile::TempDir, SubscriberStore, Vec<Subscriber>) {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let (oct_start, _) = month_bounds("2026-10").expect("bounds");
        let mut subscribers = Vec::with_capacity(count);
        for index in 0..count {
            subscribers.push(active_with_profile(
                &store,
                &format!("renew{index}.example"),
                &format!("ops{index}@renew.example"),
                oct_start + 1_000 + index as i64,
            ));
        }
        (dir, store, subscribers)
    }

    #[tokio::test]
    async fn one_hundred_charges_overlap_four_at_a_time_and_charge_once() {
        const SUBSCRIBERS: usize = 100;
        const DELAY: std::time::Duration = std::time::Duration::from_millis(200);
        let (dir, store, subscribers) = many_renewals(SUBSCRIBERS);
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport = SlowTransport::new(DELAY, None);
        let adyen = adyen();
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };

        let started = std::time::Instant::now();
        let report = run_bill(&store, "2026-11", &deps).await.expect("run");
        let elapsed = started.elapsed();
        println!(
            "subscribers={SUBSCRIBERS} delay_ms={} elapsed_ms={} max_in_flight={}",
            DELAY.as_millis(),
            elapsed.as_millis(),
            transport.max_in_flight()
        );

        assert_eq!(report.refused.len(), 0, "{:#?}", report.refused);
        assert_eq!(report.issued.len(), SUBSCRIBERS);
        assert_eq!(report.charged.len(), SUBSCRIBERS);
        // 25 waves of four 200 ms charges: serial would be ~20 s.
        assert!(
            transport.max_in_flight() <= BILL_CHARGE_CONCURRENCY,
            "bounded at {BILL_CHARGE_CONCURRENCY}, got {}",
            transport.max_in_flight()
        );
        assert!(
            transport.max_in_flight() > 1,
            "charges must overlap, got {}",
            transport.max_in_flight()
        );
        assert!(
            elapsed < std::time::Duration::from_secs(9),
            "100 charges took {elapsed:?}; the serial path takes ~20s"
        );

        // Register numbers are unchanged by the concurrency: gapless and in
        // subscriber order.
        let numbers: Vec<u64> = report.issued.iter().map(|row| row.number).collect();
        assert_eq!(
            numbers,
            (1..=SUBSCRIBERS as u64).collect::<Vec<_>>(),
            "invoices stay serial and gapless"
        );
        assert_eq!(
            std::fs::read_to_string(&register_path)
                .unwrap()
                .lines()
                .count(),
            SUBSCRIBERS
        );

        // Exactly one charge per subscriber-month.
        let requests = transport.requests();
        assert_eq!(requests.len(), SUBSCRIBERS);
        for subscriber in &subscribers {
            let reference = format!("watch:{}:2026-11", subscriber.id);
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request_reference(request) == Some(reference.as_str()))
                    .count(),
                1,
                "{reference} must be charged exactly once"
            );
        }

        // A rerun of the same month is all skips and no second charge.
        let replay = run_bill(&store, "2026-11", &deps).await.expect("replay");
        assert_eq!(replay.issued.len(), 0);
        assert_eq!(replay.charged.len(), 0);
        assert_eq!(replay.skipped.len(), SUBSCRIBERS);
        assert_eq!(transport.requests().len(), SUBSCRIBERS);
        assert_eq!(
            std::fs::read_to_string(&register_path)
                .unwrap()
                .lines()
                .count(),
            SUBSCRIBERS
        );
    }

    #[tokio::test]
    async fn one_failed_charge_isolates_its_subscriber_and_retries_the_same_reference() {
        const SUBSCRIBERS: usize = 12;
        let (dir, store, subscribers) = many_renewals(SUBSCRIBERS);
        let failing = subscribers[5].clone();
        let failing_reference = format!("watch:{}:2026-11", failing.id);
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let transport = SlowTransport::new(
            std::time::Duration::from_millis(20),
            Some(failing_reference.clone()),
        );
        let adyen = adyen();
        let deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&transport),
        };

        let report = run_bill(&store, "2026-11", &deps).await.expect("run");
        assert_eq!(report.issued.len(), SUBSCRIBERS, "every invoice is issued");
        assert_eq!(report.charged.len(), SUBSCRIBERS - 1);
        assert_eq!(report.refused.len(), 1);
        assert!(
            report.refused[0].reason.contains("not charged"),
            "{}",
            report.refused[0].reason
        );
        assert_eq!(report.refused[0].publisher, failing.publisher);
        assert_eq!(transport.requests().len(), SUBSCRIBERS);

        // Only the failed subscriber keeps an unpaid month and an error.
        for subscriber in store.list().expect("list") {
            let row = store.get(&subscriber.id).unwrap().unwrap();
            if row.id == failing.id {
                assert_eq!(row.last_billed_month.as_deref(), Some("2026-11"));
                assert_eq!(row.last_paid_month, None);
                assert!(row.last_charge_error.is_some());
            } else {
                assert_eq!(row.last_paid_month.as_deref(), Some("2026-11"));
                assert_eq!(row.last_charge_error, None);
            }
        }

        // The next run retries only the failed subscriber, with the same
        // deterministic reference, and reissues nothing.
        let retry_transport = SlowTransport::new(std::time::Duration::from_millis(1), None);
        let retry_deps = BillDeps {
            seller: &seller,
            register_path: &register_path,
            vies: &vies,
            adyen: Some(&adyen),
            transport: Some(&retry_transport),
        };
        let retry = run_bill(&store, "2026-11", &retry_deps)
            .await
            .expect("retry");
        assert_eq!(retry.issued.len(), 0, "no invoice is re-issued");
        assert_eq!(retry.charged.len(), 1);
        assert_eq!(retry.skipped.len(), SUBSCRIBERS - 1);
        let requests = retry_transport.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            request_reference(&requests[0]),
            Some(failing_reference.as_str()),
            "the retry reuses the same idempotency reference"
        );
        assert_eq!(
            std::fs::read_to_string(&register_path)
                .unwrap()
                .lines()
                .count(),
            SUBSCRIBERS,
            "the register still holds one invoice per subscriber"
        );
        let row = store.get(&failing.id).unwrap().unwrap();
        assert_eq!(row.last_paid_month.as_deref(), Some("2026-11"));
        assert_eq!(row.last_charge_error, None);
    }

    #[tokio::test]
    async fn an_invalid_month_is_a_typed_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let seller = seller();
        let register_path = register_path(&dir);
        let vies = DomesticVies;
        let adyen = adyen();
        let transport = ScriptedTransport::new(Vec::new());
        assert!(matches!(
            run_bill(
                &store,
                "2026-13",
                &BillDeps {
                    seller: &seller,
                    register_path: &register_path,
                    vies: &vies,
                    adyen: Some(&adyen),
                    transport: Some(&transport),
                }
            )
            .await,
            Err(WatchError::Malformed(_))
        ));
        assert!(!register_path.exists());
    }
}
