//! The monthly bill: one setup-watch line per active subscriber.
//!
//! [`run_bill`] issues one VAT invoice per active subscriber through the
//! same [`apex_billing::InvoiceRegister`] path the runner uses (the default
//! register is [`apex_billing::DEFAULT_REGISTER_PATH`], the deployment's
//! `.apex/billing/invoices.jsonl`). The line is the frozen
//! [`apex_billing::watch_plan`] shape: $99 for the normal tier and $199 for
//! the core tier, in [`apex_billing::PLAN_CURRENCY`].
//!
//! The customer profile comes from the subscriber row (`billing`), or from
//! the operator sidecar `<watch root>/<publisher>.billing.json` when the row
//! does not carry one. A subscriber without either is refused with a typed
//! reason before the register is touched, so a refusal never consumes a
//! number and no half invoice is ever rendered. A subscriber already
//! recorded as billed for the month is skipped, so rerunning the monthly
//! timer cannot issue a second line.
//!
//! Cancelled subscribers are never billed: the cancel action flips `status`
//! to `canceled` and the monthly pass only walks active rows.

use std::path::Path;

use apex_billing::{
    compute_invoice, watch_plan, BillingPeriod, BillingProfile, InvoiceRegister,
    WATCH_CORE_PRICE_MICROS, WATCH_PRICE_MICROS,
};
use serde::{Deserialize, Serialize};

use crate::model::{Subscriber, Tier};
use crate::store::{SubscriberStore, WatchError};

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
    /// Subscribers refused this month.
    pub refused: Vec<RefusedInvoice>,
    /// Subscribers skipped as already billed.
    pub skipped: Vec<SkippedInvoice>,
}

/// The injected dependencies of one monthly run.
#[derive(Debug)]
pub struct BillDeps<'a> {
    /// The seller profile the VAT treatment loads.
    pub seller: &'a BillingProfile,
    /// The invoice register path the runner also writes.
    pub register_path: &'a Path,
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

/// Issue the month's setup-watch lines for every active subscriber.
pub fn run_bill(
    store: &SubscriberStore,
    month: &str,
    deps: &BillDeps<'_>,
) -> Result<BillReport, WatchError> {
    let Some((start_unix_ms, end_unix_ms)) = month_bounds(month) else {
        return Err(WatchError::Malformed(format!(
            "month '{month}' is not a YYYY-MM calendar month"
        )));
    };
    let register = InvoiceRegister::open(deps.register_path)?;
    let mut report = BillReport::default();
    for subscriber in store.list_active()? {
        bill_one(
            store,
            &subscriber,
            month,
            start_unix_ms,
            end_unix_ms,
            deps,
            &register,
            &mut report,
        )?;
    }
    Ok(report)
}

fn refuse(report: &mut BillReport, publisher: &str, reason: String) {
    report.refused.push(RefusedInvoice {
        publisher: publisher.to_string(),
        reason,
    });
}

#[allow(clippy::too_many_arguments)]
fn bill_one(
    store: &SubscriberStore,
    subscriber: &Subscriber,
    month: &str,
    start_unix_ms: i64,
    end_unix_ms: i64,
    deps: &BillDeps<'_>,
    register: &InvoiceRegister,
    report: &mut BillReport,
) -> Result<(), WatchError> {
    if subscriber.last_billed_month.as_deref() == Some(month) {
        report.skipped.push(SkippedInvoice {
            publisher: subscriber.publisher.clone(),
            month: month.to_string(),
        });
        return Ok(());
    }
    let publisher = subscriber.publisher.as_str();
    let customer = match subscriber.billing.clone() {
        Some(customer) => customer,
        None => match store.billing_sidecar(publisher) {
            Ok(Some(customer)) => customer,
            Ok(None) => {
                refuse(
                    report,
                    publisher,
                    "the subscriber has no customer billing profile recorded; the invoice is refused until one is".to_string(),
                );
                return Ok(());
            }
            Err(error) => {
                refuse(
                    report,
                    publisher,
                    format!("the customer billing profile could not be read: {error}"),
                );
                return Ok(());
            }
        },
    };
    if let Err(error) = customer.validate() {
        refuse(
            report,
            publisher,
            format!("the customer billing profile is unusable: {error}"),
        );
        return Ok(());
    }
    let price = match subscriber.tier {
        Tier::Core => WATCH_CORE_PRICE_MICROS,
        Tier::Normal => WATCH_PRICE_MICROS,
    };
    let plan = watch_plan(move || price, apex_billing::PLAN_CURRENCY);
    let period = BillingPeriod {
        year_month: month.to_string(),
        start_unix_ms,
        end_unix_ms,
    };
    let mut invoice = match compute_invoice(publisher, period, &[]) {
        Ok(invoice) => invoice,
        Err(error) => {
            refuse(
                report,
                publisher,
                format!("the invoice could not be prepared: {error}"),
            );
            return Ok(());
        }
    };
    if let Err(error) = invoice.push_plan_line(plan.monthly_line(1, month)) {
        refuse(
            report,
            publisher,
            format!("the watch line could not be attached: {error}"),
        );
        return Ok(());
    }
    let mut prepared = match invoice
        .with_customer(customer)
        .apply_vat_treatment(deps.seller)
    {
        Ok(prepared) => prepared,
        Err(error) => {
            refuse(
                report,
                publisher,
                format!("the VAT treatment was refused: {error}"),
            );
            return Ok(());
        }
    };
    // Mirror the runner exactly: the register assigns the gapless number and
    // persists the invoice, then the lawful document renders from the
    // numbered invoice.
    let number = match register.issue(&mut prepared) {
        Ok(number) => number,
        Err(error) => {
            refuse(
                report,
                publisher,
                format!("the register refused the invoice: {error}"),
            );
            return Ok(());
        }
    };
    let document = match prepared.render_vat_invoice(deps.seller) {
        Ok(document) => document,
        Err(error) => {
            refuse(
                report,
                publisher,
                format!("invoice {number} was issued but could not render: {error}"),
            );
            return Ok(());
        }
    };
    let mut updated = subscriber.clone();
    updated.last_billed_month = Some(month.to_string());
    store.update(&updated)?;
    report.issued.push(IssuedInvoice {
        publisher: subscriber.publisher.clone(),
        tier: subscriber.tier,
        number,
        net_micros: prepared.total_micros,
        gross_micros: prepared.gross_micros,
        document,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;

    use apex_billing::CustomerProfile;

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

    fn register_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("billing").join("invoices.jsonl")
    }

    fn bill_deps<'a>(seller: &'a BillingProfile, register_path: &'a Path) -> BillDeps<'a> {
        BillDeps {
            seller,
            register_path,
        }
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

    #[test]
    fn golden_invoices_for_normal_and_core_share_gapless_numbers() {
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
        normal.billing = Some(customer("Northstar OÜ"));
        store.update(&normal).expect("update");
        let mut core = core;
        core.billing = Some(customer("Bigport OÜ"));
        store.update(&core).expect("update");

        let seller = seller();
        let register_path = register_path(&dir);
        let report = run_bill(&store, "2026-10", &bill_deps(&seller, &register_path)).expect("run");
        assert_eq!(report.refused.len(), 0, "{:#?}", report.refused);
        assert_eq!(report.skipped.len(), 0);
        assert_eq!(report.issued.len(), 2);
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

        // Both rows remember the billed month.
        for subscriber in store.list().expect("list") {
            assert_eq!(subscriber.last_billed_month.as_deref(), Some("2026-10"));
        }
    }

    #[test]
    fn a_missing_profile_refuses_without_consuming_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe");
        let seller = seller();
        let register_path = register_path(&dir);
        let report = run_bill(&store, "2026-10", &bill_deps(&seller, &register_path)).expect("run");
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
        broken.billing = Some(CustomerProfile {
            legal_name: " ".into(),
            address_lines: vec!["x".into()],
            country: "EE".into(),
            vat_number: None,
        });
        store.update(&broken).expect("update");
        let report = run_bill(&store, "2026-10", &bill_deps(&seller, &register_path)).expect("run");
        assert_eq!(report.refused.len(), 1);
        assert!(report.refused[0].reason.contains("unusable"));
        assert!(!register_path.exists());
    }

    #[test]
    fn a_rerun_of_the_same_month_bills_nothing_twice() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let mut subscriber = store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe")
            .subscriber;
        subscriber.billing = Some(customer("Northstar OÜ"));
        store.update(&subscriber).expect("update");
        let seller = seller();
        let register_path = register_path(&dir);
        let first = run_bill(&store, "2026-10", &bill_deps(&seller, &register_path)).expect("run");
        assert_eq!(first.issued.len(), 1);
        let second = run_bill(&store, "2026-10", &bill_deps(&seller, &register_path)).expect("run");
        assert_eq!(second.issued.len(), 0);
        assert_eq!(second.skipped.len(), 1);
        assert_eq!(second.skipped[0].month, "2026-10");
        let raw = std::fs::read_to_string(&register_path).unwrap();
        assert_eq!(raw.lines().count(), 1, "still exactly one invoice");
    }

    #[test]
    fn canceled_subscribers_are_not_billed_next_month() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let first = store
            .subscribe(10, "northstar.example", "ops@northstar.example", "", false)
            .expect("subscribe");
        let second = store
            .subscribe(11, "bigport.example", "ops@bigport.example", "", true)
            .expect("subscribe");
        for mut subscriber in [first.subscriber.clone(), second.subscriber.clone()] {
            subscriber.billing = Some(customer("Buyer OÜ"));
            store.update(&subscriber).expect("update");
        }
        let seller = seller();
        let register_path = register_path(&dir);
        let october =
            run_bill(&store, "2026-10", &bill_deps(&seller, &register_path)).expect("run");
        assert_eq!(october.issued.len(), 2);

        // Cancel the first subscriber with its token, then bill November.
        let token = first.cancel_token.clone().expect("token");
        let canceled = store.cancel(&token, 20).expect("cancel").expect("match");
        assert!(!canceled.is_active());
        let november =
            run_bill(&store, "2026-11", &bill_deps(&seller, &register_path)).expect("run");
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

    #[test]
    fn an_invalid_month_is_a_typed_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let store = SubscriberStore::open(dir.path());
        let seller = seller();
        let register_path = register_path(&dir);
        assert!(matches!(
            run_bill(&store, "2026-13", &bill_deps(&seller, &register_path)),
            Err(WatchError::Malformed(_))
        ));
        assert!(!register_path.exists());
    }
}
