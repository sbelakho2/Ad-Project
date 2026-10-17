//! Flat plan lines in the invoice path: byte-exact goldens per monitoring
//! tier, the managed ad-ops price points and scope, the frozen setup-watch
//! line, and the cancelled-not-deleted stop-period record.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use apex_billing::subscription::{
    watch_plan, ManagedPlan, MonitoringPlan, MonitoringTier, PlanId, PlanState, MANAGED_SCOPE,
    WATCH_CORE_PRICE_MICROS, WATCH_PRICE_MICROS,
};
use apex_billing::{
    compute_invoice, BillingPeriod, BillingProfile, BillingProofRecord, CustomerProfile, Invoice,
    InvoiceRegister,
};

const PUBLISHER: &str = "pub-1";

fn seller() -> BillingProfile {
    BillingProfile {
        legal_name: "Apex Media OÜ".into(),
        address_lines: vec!["Sepapaja tn 6".into(), "15551 Tallinn".into()],
        country: "EE".into(),
        vat_number: "EE123456789".into(),
        vat_rate_bp: 2_000,
    }
}

fn domestic_customer() -> CustomerProfile {
    CustomerProfile {
        legal_name: "Nurme OÜ".into(),
        address_lines: vec!["Narva mnt 5".into(), "10117 Tallinn".into()],
        country: "EE".into(),
        vat_number: Some("EE987654321".into()),
    }
}

fn period() -> BillingPeriod {
    BillingPeriod {
        year_month: "2023-11".into(),
        start_unix_ms: 1_698_796_800_000,
        end_unix_ms: 1_701_475_200_000,
    }
}

fn proof(seq: u64, experiment: &str, lift: i64, rate_bp: u16) -> BillingProofRecord {
    BillingProofRecord {
        seq,
        at_unix_ms: 1_700_000_000_000,
        publisher_id: PUBLISHER.into(),
        experiment_id: experiment.into(),
        proven_lift: lift,
        rate_bp,
        fee_micros: lift * i64::from(rate_bp) / 10_000,
    }
}

/// A verified two-line invoice with one plan line appended (the runner's
/// exact shape: the plan fee is checked into the net total, then VAT is
/// applied to every line).
fn with_plan_line(state: &PlanState, month: &str) -> Invoice {
    let mut invoice = compute_invoice(
        PUBLISHER,
        period(),
        &[
            proof(1, "exp-a", 2_500_000, 2_000),
            proof(2, "exp-b", 1_000_000, 1_500),
        ],
    )
    .expect("verified proofs");
    let next_seq = invoice
        .line_items
        .iter()
        .map(|line| line.seq)
        .max()
        .unwrap_or(0)
        + 1;
    let line = state
        .monthly_line(next_seq, month)
        .expect("the fixture period bills");
    invoice.push_plan_line(line).expect("no overflow");
    invoice
        .with_customer(domestic_customer())
        .apply_vat_treatment(&seller())
        .expect("complete profiles")
}

fn monitoring_invoice(tier: MonitoringTier) -> Invoice {
    let plan = MonitoringPlan::default();
    let revenue = match tier {
        MonitoringTier::Base => 50_000 * 1_000_000,
        MonitoringTier::Mid => 350_000 * 1_000_000,
        MonitoringTier::Top => 900_000 * 1_000_000,
    };
    let state = plan.activate(revenue, "2023-11-01");
    assert_eq!(state.tier, Some(tier));
    with_plan_line(&state, "2023-11")
}

const GOLDEN_MONITORING_BASE: &str = "\
VAT invoice No. 1 for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
15551 Tallinn
EE
VAT number: EE123456789
Customer:
Nurme OÜ
Narva mnt 5
10117 Tallinn
EE
VAT number: EE987654321
Line items:
  #1 exp-a: net 500000 micros, VAT 100000 micros, gross 600000 micros
  #2 exp-b: net 150000 micros, VAT 30000 micros, gross 180000 micros
  #3 monitoring:2023-11: net 500000000 micros, VAT 100000000 micros, gross 600000000 micros
Net total: 500650000 micros
VAT (2000 bp): 100130000 micros
Gross total: 600780000 micros
";

const GOLDEN_MONITORING_MID: &str = "\
VAT invoice No. 2 for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
15551 Tallinn
EE
VAT number: EE123456789
Customer:
Nurme OÜ
Narva mnt 5
10117 Tallinn
EE
VAT number: EE987654321
Line items:
  #1 exp-a: net 500000 micros, VAT 100000 micros, gross 600000 micros
  #2 exp-b: net 150000 micros, VAT 30000 micros, gross 180000 micros
  #3 monitoring:2023-11: net 1500000000 micros, VAT 300000000 micros, gross 1800000000 micros
Net total: 1500650000 micros
VAT (2000 bp): 300130000 micros
Gross total: 1800780000 micros
";

const GOLDEN_MONITORING_TOP: &str = "\
VAT invoice No. 3 for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
15551 Tallinn
EE
VAT number: EE123456789
Customer:
Nurme OÜ
Narva mnt 5
10117 Tallinn
EE
VAT number: EE987654321
Line items:
  #1 exp-a: net 500000 micros, VAT 100000 micros, gross 600000 micros
  #2 exp-b: net 150000 micros, VAT 30000 micros, gross 180000 micros
  #3 monitoring:2023-11: net 3000000000 micros, VAT 600000000 micros, gross 3600000000 micros
Net total: 3000650000 micros
VAT (2000 bp): 600130000 micros
Gross total: 3600780000 micros
";

#[test]
fn golden_monitoring_invoice_per_tier() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("billing").join("invoices.jsonl");
    let register = InvoiceRegister::open(&path).expect("register");

    let mut base = monitoring_invoice(MonitoringTier::Base);
    assert_eq!(register.issue(&mut base).unwrap(), 1);
    let mut mid = monitoring_invoice(MonitoringTier::Mid);
    assert_eq!(register.issue(&mut mid).unwrap(), 2);
    let mut top = monitoring_invoice(MonitoringTier::Top);
    assert_eq!(register.issue(&mut top).unwrap(), 3);

    // Byte-exact documents: the tier fee is its own flat line on the same
    // invoice, taxed with every other line, and the gross is collected.
    assert_eq!(
        base.render_vat_invoice(&seller()).unwrap(),
        GOLDEN_MONITORING_BASE
    );
    assert_eq!(
        mid.render_vat_invoice(&seller()).unwrap(),
        GOLDEN_MONITORING_MID
    );
    assert_eq!(
        top.render_vat_invoice(&seller()).unwrap(),
        GOLDEN_MONITORING_TOP
    );
    assert!(GOLDEN_MONITORING_BASE.contains("net 500000000 micros"));
    assert!(GOLDEN_MONITORING_MID.contains("net 1500000000 micros"));
    assert!(GOLDEN_MONITORING_TOP.contains("net 3000000000 micros"));
}

#[test]
fn golden_cancelled_monitoring_still_bills_its_stop_period() {
    // A cancelled plan bills the current period only; the line states the
    // stop, and the next period renders nothing (zero lines to invoice).
    let plan = MonitoringPlan::default();
    let mut state = plan.activate(50_000 * 1_000_000, "2023-09-01");
    state.cancel("2023-11");
    let mut invoice = with_plan_line(&state, "2023-11");
    let dir = tempfile::tempdir().expect("temp dir");
    let register = InvoiceRegister::open(&dir.path().join("invoices.jsonl")).expect("register");
    assert_eq!(register.issue(&mut invoice).unwrap(), 1);
    let rendered = invoice.render_vat_invoice(&seller()).unwrap();
    assert!(
        rendered.contains("#3 monitoring:2023-11:cancelled:stops-2023-11: net 500000000 micros")
    );
    assert!(
        rendered.contains("Net total: 500650000 micros"),
        "the stop period still bills: {rendered}"
    );
    // The next period is zero.
    assert_eq!(state.monthly_line(1, "2023-12"), None);
}

const GOLDEN_MANAGED_BASE: &str = "\
VAT invoice No. 1 for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
15551 Tallinn
EE
VAT number: EE123456789
Customer:
Nurme OÜ
Narva mnt 5
10117 Tallinn
EE
VAT number: EE987654321
Line items:
  #1 exp-a: net 500000 micros, VAT 100000 micros, gross 600000 micros
  #2 exp-b: net 150000 micros, VAT 30000 micros, gross 180000 micros
  #3 managed:2023-11: net 2500000000 micros, VAT 500000000 micros, gross 3000000000 micros
Net total: 2500650000 micros
VAT (2000 bp): 500130000 micros
Gross total: 3000780000 micros
";

const GOLDEN_MANAGED_TOP: &str = "\
VAT invoice No. 2 for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
15551 Tallinn
EE
VAT number: EE123456789
Customer:
Nurme OÜ
Narva mnt 5
10117 Tallinn
EE
VAT number: EE987654321
Line items:
  #1 exp-a: net 500000 micros, VAT 100000 micros, gross 600000 micros
  #2 exp-b: net 150000 micros, VAT 30000 micros, gross 180000 micros
  #3 managed:2023-11: net 5000000000 micros, VAT 1000000000 micros, gross 6000000000 micros
Net total: 5000650000 micros
VAT (2000 bp): 1000130000 micros
Gross total: 6000780000 micros
";

#[test]
fn golden_managed_line_per_price_point() {
    let dir = tempfile::tempdir().expect("temp dir");
    let register = InvoiceRegister::open(&dir.path().join("invoices.jsonl")).expect("register");

    // $2,500 up to $1M monthly revenue; $5,000 strictly above.
    let mut base = with_plan_line(
        &ManagedPlan::default().activate(900_000 * 1_000_000, "2023-11-01"),
        "2023-11",
    );
    assert_eq!(register.issue(&mut base).unwrap(), 1);
    let mut top = with_plan_line(
        &ManagedPlan::default().activate(1_200_000 * 1_000_000, "2023-11-01"),
        "2023-11",
    );
    assert_eq!(register.issue(&mut top).unwrap(), 2);

    assert_eq!(
        base.render_vat_invoice(&seller()).unwrap(),
        GOLDEN_MANAGED_BASE
    );
    assert_eq!(
        top.render_vat_invoice(&seller()).unwrap(),
        GOLDEN_MANAGED_TOP
    );
    // The written scope lives in the crate as data, ready for the console.
    assert_eq!(
        MANAGED_SCOPE,
        [
            "Network onboarding",
            "Bidder setup",
            "app-ads.txt setup",
            "A monthly review",
        ]
    );
}

const GOLDEN_WATCH_CORE: &str = "\
VAT invoice No. 1 for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
15551 Tallinn
EE
VAT number: EE123456789
Customer:
Nurme OÜ
Narva mnt 5
10117 Tallinn
EE
VAT number: EE987654321
Line items:
  #1 setup-watch:2023-11: net 199000000 micros, VAT 39800000 micros, gross 238800000 micros
Net total: 199000000 micros
VAT (2000 bp): 39800000 micros
Gross total: 238800000 micros
";

#[test]
fn golden_invoice_includes_the_frozen_watch_line() {
    // The frozen interface the setup-watch service wave calls: the provider
    // chooses the $99 / $199 core-publisher price.
    let core = watch_plan(|| WATCH_CORE_PRICE_MICROS, "USD");
    assert_eq!(core.plan, PlanId::SetupWatch);
    assert_eq!(core.monthly_minor_units(), 199);
    let base = watch_plan(|| WATCH_PRICE_MICROS, "USD");
    assert_eq!(base.monthly_minor_units(), 99);

    let mut invoice = compute_invoice(PUBLISHER, period(), &[]).expect("zero invoice");
    invoice
        .push_plan_line(core.monthly_line(1, "2023-11"))
        .expect("no overflow");
    let mut invoice = invoice
        .with_customer(domestic_customer())
        .apply_vat_treatment(&seller())
        .expect("complete profiles");
    let dir = tempfile::tempdir().expect("temp dir");
    let register = InvoiceRegister::open(&dir.path().join("invoices.jsonl")).expect("register");
    assert_eq!(register.issue(&mut invoice).unwrap(), 1);
    assert_eq!(
        invoice.render_vat_invoice(&seller()).unwrap(),
        GOLDEN_WATCH_CORE
    );
}
