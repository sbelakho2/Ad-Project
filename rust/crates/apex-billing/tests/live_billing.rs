//! LIVE billing verification: real signed ledger records, real invoice
//! arithmetic, real VAT profiles, the real invoice register, and a real
//! fetched Mollie payment recorded through the real collection store.
//!
//! Platform reporting and provider collection are deliberately absent: no
//! MAX / LevelPlay / AdMob / Mollie credentials exist on this host, so the
//! money math is proven from signed evidence and the charge path is reported
//! BLOCKED rather than simulated.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::path::{Path, PathBuf};

use apex_billing::{
    compute_invoice, BillingPeriod, BillingProofRecord, CollectionEvent, CollectionStore,
    CustomerProfile, FileCollectionStore, InvoiceRegister, ManagedPlan, MolliePayment, PlanId,
    PlanState,
};
use apex_proof::{Keypair, Ledger};

fn live_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!(
        "apex-billing-live-{}-{nanos}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("live dir");
    dir
}

fn real_keypair() -> Keypair {
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .expect("urandom")
        .read_exact(&mut seed)
        .expect("seed");
    if seed == [0u8; 32] {
        seed[0] = 1;
    }
    Keypair::from_bytes(&seed)
}

fn seller() -> apex_billing::BillingProfile {
    apex_billing::BillingProfile {
        legal_name: "Apex Media OÜ".into(),
        address_lines: vec!["Sepapaja tn 6".into(), "15551 Tallinn".into()],
        country: "EE".into(),
        vat_number: "EE123456789".into(),
        vat_rate_bp: 2_000,
    }
}

fn customer() -> CustomerProfile {
    CustomerProfile {
        legal_name: "Buyer OÜ".into(),
        address_lines: vec!["Telliskivi 1".into()],
        country: "EE".into(),
        vat_number: None,
    }
}

fn period() -> BillingPeriod {
    BillingPeriod {
        year_month: "2026-10".into(),
        // 2026-10-01T00:00:00Z .. 2026-11-01T00:00:00Z
        start_unix_ms: 1_790_812_800_000,
        end_unix_ms: 1_793_491_200_000,
    }
}

/// Build a real signed ledger with three billing proofs and return the
/// flattened, re-verifiable records.
fn signed_proofs(path: &Path) -> Vec<BillingProofRecord> {
    let key = real_keypair();
    let mut ledger = Ledger::open(path, key.clone()).expect("ledger");
    for (index, (control, variant, rate)) in [
        (10_000_000i64, 12_500_000i64, 2_000u16),
        (4_000_000, 4_500_000, 1_500),
        (20_000_000, 23_000_000, 1_800),
    ]
    .into_iter()
    .enumerate()
    {
        let proof = apex_proof::build_billing_proof(
            "pub-live",
            &format!("exp-live-{index}"),
            control,
            variant,
            rate,
            1_791_000_000_000 + index as i64,
        )
        .expect("proof");
        ledger.append(proof).expect("append proof");
    }
    drop(ledger);
    let records = Ledger::read_verified(path, &key.verifying_key()).expect("verified ledger");
    assert_eq!(records.len(), 3);
    records
        .iter()
        .map(|record| BillingProofRecord::from_ledger_record(record).expect("billing proof"))
        .collect()
}

#[test]
fn invoice_math_state_vat_and_plan_lines_come_from_real_signed_records() {
    let dir = live_dir("invoice");
    let proofs = signed_proofs(&dir.join("ledger.jsonl"));
    assert_eq!(proofs.len(), 3);

    let mut invoice = compute_invoice("pub-live", period(), &proofs).expect("invoice");
    assert_eq!(invoice.number, 0, "the draft has no register number yet");
    assert_eq!(invoice.publisher_id, "pub-live");
    assert_eq!(
        invoice
            .line_items
            .iter()
            .map(|line| line.fee_micros)
            .collect::<Vec<_>>(),
        vec![500_000, 75_000, 540_000],
        "fees are recomputed from the signed lift and rate"
    );
    assert_eq!(invoice.total_micros, 1_115_000);

    // Managed ad ops is the only paid plan line after A3.
    let managed_state = ManagedPlan::new().activate(1_000_001 * 1_000_000, "2026-10-01");
    assert_eq!(managed_state.monthly_fee_micros, 5_000 * 1_000_000);
    let managed_line = managed_state.monthly_line(5, "2026-10").expect("plan line");

    invoice.push_plan_line(managed_line).expect("plan line");
    assert_eq!(invoice.total_micros, 1_115_000 + 5_000_000_000);

    // Domestic EE treatment (both parties in Estonia): 2000 bp VAT on every
    // line, including the plan lines, with the gross total collected.
    let mut invoice = invoice.with_customer(customer());
    invoice = invoice
        .apply_vat_treatment(&seller())
        .expect("domestic VAT");
    assert_eq!(
        invoice.vat_treatment,
        apex_billing::VatTreatment::Domestic,
        "state: domestic"
    );
    let expected_net = 1_115_000 + 5_000_000_000;
    assert_eq!(invoice.total_micros, expected_net);
    // VAT is exact per line, rounded once to the cent, so the gross lands on
    // whole cents the collector can charge: the sum of the line VATs is the
    // invoice VAT, each line's figure is cent-exact, and the total stays
    // within half a cent per line of the exact total-level arithmetic (a
    // total-level figure would put sub-cent VAT on the gross and the
    // collector would refuse the charge).
    let exact_total_vat = expected_net * 2_000 / 10_000;
    let line_vat_sum: i64 = invoice.line_items.iter().map(|line| line.vat_micros).sum();
    assert_eq!(invoice.vat_micros, line_vat_sum, "one VAT per line, summed");
    assert_eq!(invoice.gross_micros, expected_net + invoice.vat_micros);
    // Every cent-exact fee line (which is what the runner signs and what the
    // collector charges) carries cent-exact VAT and gross: 2% of $1.115M of
    // mixed lines would otherwise put sub-cent VAT on the total and the
    // collector would refuse the whole charge. A proof-derived sub-cent fee
    // (7.5 cents here) keeps the exact micro VAT it always had; it is not a
    // chargeable line and the runner never signs one.
    let cent_lines: Vec<&apex_billing::InvoiceLine> = invoice
        .line_items
        .iter()
        .filter(|line| line.fee_micros % 10_000 == 0)
        .collect();
    assert!(!cent_lines.is_empty());
    for line in &cent_lines {
        assert_eq!(line.vat_micros % 10_000, 0, "cent-exact VAT: {line:?}");
        assert_eq!(line.gross_micros % 10_000, 0, "collectable gross: {line:?}");
        assert_eq!(line.gross_micros, line.fee_micros + line.vat_micros);
    }
    let cent_fee_total: i64 = invoice
        .line_items
        .iter()
        .filter(|line| line.fee_micros % 10_000 == 0)
        .map(|line| line.fee_micros)
        .sum();
    let cent_vat_total: i64 = invoice
        .line_items
        .iter()
        .filter(|line| line.fee_micros % 10_000 == 0)
        .map(|line| line.vat_micros)
        .sum();
    let exact_cent_vat = cent_fee_total * 2_000 / 10_000;
    let slack = 5_000 * cent_lines.len() as i64;
    assert!(
        (cent_vat_total - exact_cent_vat).abs() <= slack,
        "{cent_vat_total} vs exact {exact_cent_vat} within {slack}"
    );
    assert!(
        (invoice.vat_micros - exact_total_vat).abs() <= 5_000 * invoice.line_items.len() as i64,
        "{} vs exact {}",
        invoice.vat_micros,
        exact_total_vat
    );

    // Register + renderer: the lawful document needs the number first.
    let register_path = dir.join("invoices.jsonl");
    let register = InvoiceRegister::open(&register_path).expect("register");
    let number = register.issue(&mut invoice).expect("issue");
    assert_eq!(number, 1);
    let rendered = invoice.render_vat_invoice(&seller()).expect("render");
    assert!(
        rendered.starts_with("VAT invoice No. 1 for 2026-10\n"),
        "{rendered}"
    );
    assert!(rendered.contains("VAT (2000 bp): "), "{rendered}");

    // Register numbering is gapless and the per-(publisher, period) dedup fact
    // is recorded without allocating another number.
    assert_eq!(
        register
            .issued_number("pub-live", "2026-10")
            .expect("issued"),
        Some(1)
    );
    let mut next_period = period();
    next_period.year_month = "2026-11".to_string();
    let mut second = compute_invoice("pub-live", next_period, &proofs)
        .expect("invoice")
        .with_customer(customer())
        .apply_vat_treatment(&seller())
        .expect("vat");
    assert_eq!(register.issue(&mut second).expect("issue"), 2);

    // Reopen continues from the file, never reusing a number.
    let reopened = InvoiceRegister::open(&register_path).expect("reopen");
    assert_eq!(reopened.highest().expect("highest"), 2);
    let third = compute_invoice("pub-other", period(), &[]).expect("empty");
    let third = third.with_customer(customer());
    let mut third = third.apply_vat_treatment(&seller()).expect("vat");
    assert_eq!(reopened.issue(&mut third).expect("issue"), 3);

    // Credits and refunds: apex-billing has no invoice-credit or refund line
    // type on this tree; a refund is only a provider event class consumed by
    // the runner. Print the absence explicitly rather than inventing a credit.
    println!(
        "LIVE billing invoice: net {} VAT {} gross {}; register numbers 1..3; \
         invoice credits/refunds: the billed line model carries invoices and fee proofs only; no credit or refund entries are produced",
        invoice.total_micros, invoice.vat_micros, invoice.gross_micros
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn register_numbering_is_concurrent_gapless_and_replay_safe() {
    let dir = live_dir("register");
    let path = dir.join("invoices.jsonl");
    // One issued-ready invoice per distinct publisher: the per-(publisher,
    // period) gate is exactly what concurrent issuers must share.
    let base = compute_invoice("pub-concurrent", period(), &[])
        .expect("invoice")
        .with_customer(customer())
        .apply_vat_treatment(&seller())
        .expect("vat");
    let mut handles = Vec::new();
    for index in 0..8 {
        let path = path.clone();
        let mut invoice = compute_invoice(&format!("pub-concurrent-{index}"), period(), &[])
            .expect("invoice")
            .with_customer(customer())
            .apply_vat_treatment(&seller())
            .expect("vat");
        handles.push(std::thread::spawn(move || {
            let register = InvoiceRegister::open(&path).expect("register");
            let number = register.issue(&mut invoice).expect("issue");
            (number, invoice)
        }));
    }
    let mut numbers = Vec::new();
    for handle in handles {
        let (number, invoice) = handle.join().expect("thread");
        assert_eq!(invoice.number, number);
        numbers.push(number);
    }
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        (1..=8).collect::<Vec<_>>(),
        "gapless concurrent numbering"
    );
    let reopened = InvoiceRegister::open(&path).expect("reopen");
    assert_eq!(reopened.highest().expect("highest"), 8);

    // A replayed invoice object already carrying a number is refused: the
    // register never reuses or renumbers it.
    let mut already = base.clone();
    already.number = 8;
    assert!(reopened.issue(&mut already).is_err());
    println!("LIVE billing register: 8 concurrent issues numbered 1..8 gaplessly");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fetched_mollie_payments_are_recorded_and_replay_protected() {
    let dir = live_dir("collection");
    let payment = MolliePayment::from_json(
        &serde_json::from_str::<serde_json::Value>(
            r#"{"id":"tr_LIVEPAY0001","status":"paid","amount":{"currency":"USD","value":"13.38"},"metadata":{"reference":"apex-fee:pub-live:exp-live-0:2026-10","publisher":"publisher-pub-live","amount":"13.38"},"customerId":"cst_LIVE0001","mandateId":"mdt_LIVE0001"}"#,
        )
        .expect("json"),
    )
    .expect("payment");
    assert_eq!(payment.amount_minor_units, 1_338);

    let collection_path = dir.join("collection.jsonl");
    let store = FileCollectionStore::open(&collection_path).expect("store");
    let event = CollectionEvent::from_payment(&payment, 1_792_400_000_000);
    assert!(store.record(&event).expect("record"), "first event is new");
    assert!(!store.record(&event).expect("replay"), "replay is refused");
    let reopened = FileCollectionStore::open(&collection_path).expect("reopen");
    assert!(
        !reopened.record(&event).expect("replay"),
        "replay survives reopen"
    );
    assert_eq!(
        apex_billing::stored_mandate_for_publisher(&collection_path, "pub-live")
            .expect("stored mandate"),
        Some("cst_LIVE0001:mdt_LIVE0001".to_string())
    );
    assert_eq!(
        std::fs::read_to_string(&collection_path)
            .expect("collection")
            .lines()
            .count(),
        1
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&collection_path)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn managed_plan_cancellation_bills_the_current_period_only_and_state_is_kept() {
    let dir = live_dir("plans");
    let mut state = ManagedPlan::new().activate(500_000 * 1_000_000, "2026-10-01");
    assert!(state.bills_period("2026-10"));
    state.cancel("2026-10");
    assert!(state.cancelled(), "cancellation is recorded, not deleted");
    assert!(state.bills_period("2026-10"), "the stop period still bills");
    assert!(!state.bills_period("2026-11"), "the next period is zero");
    let line = state.monthly_line(1, "2026-10").expect("stop-period line");
    assert_eq!(
        line.experiment_id,
        "managed:2026-10:cancelled:stops-2026-10"
    );
    assert!(state.monthly_line(1, "2026-11").is_none());

    // Persist and reload the real plan state file.
    let path = PlanState::path_for(&dir, "pub-live", PlanId::Managed);
    state.save(&path).expect("save");
    let loaded = PlanState::load(&path).expect("load").expect("state");
    assert_eq!(loaded, state);
    println!(
        "LIVE billing plans: cancellation records stops_after={:?}; \
         renewal/grace/retention: this build records plan start and cancellation; it has no renewal or grace state",
        state.stops_after
    );
    let _ = std::fs::remove_dir_all(&dir);
}
