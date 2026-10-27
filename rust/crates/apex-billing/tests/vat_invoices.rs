//! VAT invoice compliance: byte-exact goldens for the three treatments, the
//! gapless register numbering (continuation and concurrency), and the typed
//! refusal matrix for every missing profile/customer field.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write as _;
use std::path::Path;

use apex_billing::{
    compute_invoice, BillingError, BillingPeriod, BillingProfile, BillingProofRecord,
    CustomerProfile, Invoice, InvoiceRegister, VatTreatment, ViesCheck, ViesClient, ViesError,
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

fn eu_customer() -> CustomerProfile {
    CustomerProfile {
        legal_name: "Beispiel GmbH".into(),
        address_lines: vec!["Kurfürstendamm 21".into(), "10719 Berlin".into()],
        country: "DE".into(),
        vat_number: Some("DE123456789".into()),
    }
}

fn non_eu_customer() -> CustomerProfile {
    CustomerProfile {
        legal_name: "Example Inc".into(),
        address_lines: vec!["350 Fifth Avenue".into(), "New York, NY 10118".into()],
        country: "US".into(),
        vat_number: None,
    }
}

fn period() -> BillingPeriod {
    BillingPeriod {
        year_month: "2023-11".into(),
        start_unix_ms: 1_698_796_800_000,
        end_unix_ms: 1_701_475_200_000,
    }
}

/// A consistent proof: `fee = lift * rate / 10_000`.
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

fn proofs() -> Vec<BillingProofRecord> {
    vec![
        proof(1, "exp-a", 2_500_000, 2_000),
        proof(2, "exp-b", 1_000_000, 1_500),
    ]
}

/// A stub VIES client: the integration tests never reach the network.
#[derive(Debug)]
struct StubVies;

impl ViesClient for StubVies {
    fn check_vat_number(&self, _country: &str, number: &str) -> Result<ViesCheck, ViesError> {
        Ok(ViesCheck::Valid {
            evidence: format!("VIES confirmation: {number} is valid (test stub)"),
            vat_number: number.to_string(),
        })
    }
}

/// A fully prepared invoice: verified lines, attached customer, decided
/// treatment — but no register number yet.
fn ready(customer: CustomerProfile) -> Invoice {
    compute_invoice(PUBLISHER, period(), &proofs())
        .expect("verified proofs")
        .with_customer(customer)
        .apply_vat_treatment_with_vies(&seller(), &StubVies)
        .expect("complete profiles")
}

fn append_fixture(path: &Path, number: u64) {
    let mut invoice = ready(domestic_customer());
    invoice.number = number;
    let mut line = serde_json::to_string(&invoice).expect("serialize");
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open fixture");
    file.write_all(line.as_bytes()).expect("write fixture");
}

fn register_numbers(path: &Path) -> Vec<u64> {
    std::fs::read_to_string(path)
        .expect("read register")
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("line json")["number"]
                .as_u64()
                .expect("number")
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Goldens: byte-exact, gaplessly numbered, both parties complete.
// ---------------------------------------------------------------------------

const GOLDEN_DOMESTIC: &str = "\
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
Net total: 650000 micros
VAT (2000 bp): 130000 micros
Gross total: 780000 micros
";

const GOLDEN_REVERSE_CHARGE: &str = "\
VAT invoice No. 2 for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
15551 Tallinn
EE
VAT number: EE123456789
Customer:
Beispiel GmbH
Kurfürstendamm 21
10719 Berlin
DE
VAT number: DE123456789
Line items:
  #1 exp-a: net 500000 micros, VAT 0 micros, gross 500000 micros
  #2 exp-b: net 150000 micros, VAT 0 micros, gross 150000 micros
Net total: 650000 micros
VAT: reverse charge
VIES: VIES confirmation: DE123456789 is valid (test stub)
Gross total: 650000 micros
";

const GOLDEN_OUTSIDE_EU: &str = "\
VAT invoice No. 3 for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
15551 Tallinn
EE
VAT number: EE123456789
Customer:
Example Inc
350 Fifth Avenue
New York, NY 10118
US
Line items:
  #1 exp-a: net 500000 micros, VAT 0 micros, gross 500000 micros
  #2 exp-b: net 150000 micros, VAT 0 micros, gross 150000 micros
Net total: 650000 micros
Gross total: 650000 micros
";

#[test]
fn golden_domestic_reverse_charge_and_outside_eu_invoices() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("billing").join("invoices.jsonl");
    let register = InvoiceRegister::open(&path).expect("register");

    // Distinct publishers keep the one-invoice-per-(publisher, period) gate
    // satisfied while the golden documents pin numbers 1..3.
    let mut domestic = ready(domestic_customer());
    domestic.publisher_id = "pub-dom".into();
    assert_eq!(domestic.vat_treatment, VatTreatment::Domestic);
    assert_eq!(register.issue(&mut domestic).unwrap(), 1);

    let mut reverse_charge = ready(eu_customer());
    reverse_charge.publisher_id = "pub-rev".into();
    assert_eq!(reverse_charge.vat_treatment, VatTreatment::ReverseCharge);
    assert_eq!(register.issue(&mut reverse_charge).unwrap(), 2);

    let mut outside_eu = ready(non_eu_customer());
    outside_eu.publisher_id = "pub-out".into();
    assert_eq!(outside_eu.vat_treatment, VatTreatment::OutsideEu);
    assert_eq!(register.issue(&mut outside_eu).unwrap(), 3);

    // Byte-exact documents: gapless numbers, both parties' names, addresses
    // and VAT numbers, the exact "reverse charge" wording in the stated-tax
    // position, and no VAT line outside the EU.
    assert_eq!(
        domestic.render_vat_invoice(&seller()).unwrap(),
        GOLDEN_DOMESTIC
    );
    assert_eq!(
        reverse_charge.render_vat_invoice(&seller()).unwrap(),
        GOLDEN_REVERSE_CHARGE
    );
    assert_eq!(
        outside_eu.render_vat_invoice(&seller()).unwrap(),
        GOLDEN_OUTSIDE_EU
    );
    assert!(
        GOLDEN_REVERSE_CHARGE.contains("VAT: reverse charge\n"),
        "the stated-tax position carries the exact wording"
    );
    assert!(
        !GOLDEN_OUTSIDE_EU.contains("VAT:"),
        "no VAT line outside the EU"
    );

    // The register persisted the whole sequence and never reuses.
    assert_eq!(register_numbers(&path), [1, 2, 3]);
    let reopened = InvoiceRegister::open(&path).expect("reopen");
    assert_eq!(reopened.highest().unwrap(), 3);
    let mut next = ready(domestic_customer());
    assert_eq!(reopened.issue(&mut next).unwrap(), 4);
}

// ---------------------------------------------------------------------------
// Register: continuation from an existing register, and concurrency.
// ---------------------------------------------------------------------------

#[test]
fn register_continues_from_the_highest_existing_number() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("invoices.jsonl");
    for number in 1..=3 {
        append_fixture(&path, number);
    }
    let register = InvoiceRegister::open(&path).expect("register");
    assert_eq!(register.highest().unwrap(), 3);

    let mut fourth = ready(domestic_customer());
    fourth.period.year_month = "2023-12".into();
    assert_eq!(register.issue(&mut fourth).unwrap(), 4); // 1..3 -> next is 4
    assert_eq!(register_numbers(&path), [1, 2, 3, 4]);

    // A fresh handle keeps counting up from disk, never reusing a number.
    let reopened = InvoiceRegister::open(&path).expect("reopen");
    let mut fifth = ready(domestic_customer());
    fifth.period.year_month = "2024-01".into();
    assert_eq!(reopened.issue(&mut fifth).unwrap(), 5);
    let mut sixth = ready(domestic_customer());
    sixth.period.year_month = "2024-02".into();
    assert_eq!(reopened.issue(&mut sixth).unwrap(), 6);
    assert_eq!(register_numbers(&path), [1, 2, 3, 4, 5, 6]);
}

#[test]
fn two_concurrent_issues_get_distinct_consecutive_numbers() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("invoices.jsonl");
    for number in 1..=3 {
        append_fixture(&path, number);
    }
    // Two separate handles on the same register, issuing concurrently.
    let first_handle = InvoiceRegister::open(&path).expect("first handle");
    let second_handle = InvoiceRegister::open(&path).expect("second handle");
    let mut first_invoice = ready(domestic_customer());
    first_invoice.period.year_month = "2023-12".into();
    let mut second_invoice = ready(domestic_customer());
    second_invoice.publisher_id = "pub-2".into();

    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| first_handle.issue(&mut first_invoice).unwrap());
        let second = scope.spawn(|| second_handle.issue(&mut second_invoice).unwrap());
        (first.join().unwrap(), second.join().unwrap())
    });

    let mut numbers = vec![first, second];
    numbers.sort_unstable();
    assert_eq!(numbers, [4, 5], "distinct and consecutive, no duplicate");
    assert_eq!(register_numbers(&path), [1, 2, 3, 4, 5]);
    assert_eq!(first_handle.highest().unwrap(), 5);
}

// ---------------------------------------------------------------------------
// Refusals: every missing profile/customer field is named, never printed.
// ---------------------------------------------------------------------------

#[test]
fn render_refuses_an_unregistered_invoice() {
    let invoice = ready(domestic_customer());
    assert_eq!(invoice.number, 0);
    assert_eq!(
        invoice.render_vat_invoice(&seller()),
        Err(BillingError::UnregisteredInvoice)
    );
}

#[test]
fn render_refuses_a_missing_or_incomplete_customer() {
    let mut numbered = ready(domestic_customer());
    numbered.number = 7;
    numbered.customer = None;
    assert_eq!(
        numbered.render_vat_invoice(&seller()),
        Err(BillingError::MissingCustomerProfile)
    );

    let cases: Vec<(CustomerProfile, BillingError)> = vec![
        (
            CustomerProfile {
                legal_name: "  ".into(),
                ..domestic_customer()
            },
            BillingError::MissingProfileField {
                party: "customer",
                field: "legalName",
            },
        ),
        (
            CustomerProfile {
                address_lines: vec![],
                ..domestic_customer()
            },
            BillingError::MissingProfileField {
                party: "customer",
                field: "addressLines",
            },
        ),
        (
            CustomerProfile {
                address_lines: vec!["Narva mnt 5".into(), String::new()],
                ..domestic_customer()
            },
            BillingError::MissingProfileField {
                party: "customer",
                field: "addressLines",
            },
        ),
        (
            CustomerProfile {
                country: String::new(),
                ..domestic_customer()
            },
            BillingError::MissingProfileField {
                party: "customer",
                field: "country",
            },
        ),
        (
            CustomerProfile {
                country: "E1".into(),
                ..domestic_customer()
            },
            BillingError::InvalidCountryCode {
                party: "customer",
                value: "E1".into(),
            },
        ),
        (
            CustomerProfile {
                vat_number: Some(" ".into()),
                ..domestic_customer()
            },
            BillingError::MissingProfileField {
                party: "customer",
                field: "vatNumber",
            },
        ),
    ];
    for (customer, expected) in cases {
        let mut invoice = ready(domestic_customer());
        invoice.number = 7;
        invoice.customer = Some(customer);
        assert_eq!(invoice.render_vat_invoice(&seller()), Err(expected));
    }
    // A malformed customer VAT number is never silently accepted.
    let mut bad_vat = ready(domestic_customer());
    bad_vat.number = 7;
    bad_vat.customer = Some(CustomerProfile {
        vat_number: Some("EE 987654321".into()),
        ..domestic_customer()
    });
    assert!(matches!(
        bad_vat.render_vat_invoice(&seller()),
        Err(BillingError::InvalidVatNumber {
            party: "customer",
            ..
        })
    ));
}

#[test]
fn render_refuses_a_broken_seller_profile() {
    let mut invoice = ready(domestic_customer());
    invoice.number = 7;

    let mut unnamed = seller();
    unnamed.legal_name = String::new();
    assert_eq!(
        invoice.render_vat_invoice(&unnamed),
        Err(BillingError::MissingProfileField {
            party: "seller",
            field: "legalName",
        })
    );
    let mut no_address = seller();
    no_address.address_lines = vec![];
    assert_eq!(
        invoice.render_vat_invoice(&no_address),
        Err(BillingError::MissingProfileField {
            party: "seller",
            field: "addressLines",
        })
    );
    let mut no_country = seller();
    no_country.country = " ".into();
    assert_eq!(
        invoice.render_vat_invoice(&no_country),
        Err(BillingError::MissingProfileField {
            party: "seller",
            field: "country",
        })
    );
    let mut bad_country = seller();
    bad_country.country = "Estonia".into();
    assert!(matches!(
        invoice.render_vat_invoice(&bad_country),
        Err(BillingError::InvalidCountryCode {
            party: "seller",
            ..
        })
    ));
    let mut no_vat = seller();
    no_vat.vat_number = String::new();
    assert_eq!(
        invoice.render_vat_invoice(&no_vat),
        Err(BillingError::MissingProfileField {
            party: "seller",
            field: "vatNumber",
        })
    );
    let mut bad_vat = seller();
    bad_vat.vat_number = "!!".into();
    assert!(matches!(
        invoice.render_vat_invoice(&bad_vat),
        Err(BillingError::InvalidVatNumber {
            party: "seller",
            ..
        })
    ));
    let mut bad_rate = seller();
    bad_rate.vat_rate_bp = 10_001;
    assert!(matches!(
        invoice.render_vat_invoice(&bad_rate),
        Err(BillingError::InvalidVatRate { .. })
    ));
}

#[test]
fn render_refuses_undecided_or_inconsistent_treatments() {
    // A rate applied by hand is not a decided treatment.
    let mut undecided = compute_invoice(PUBLISHER, period(), &proofs())
        .unwrap()
        .with_customer(domestic_customer())
        .with_vat(2_000)
        .expect("vat");
    undecided.number = 7;
    assert_eq!(
        undecided.render_vat_invoice(&seller()),
        Err(BillingError::VatTreatmentNotApplied)
    );

    // The stated treatment must match the parties' profiles...
    let mut swapped = ready(domestic_customer());
    swapped.number = 7;
    swapped.customer = Some(eu_customer());
    assert!(matches!(
        swapped.render_vat_invoice(&seller()),
        Err(BillingError::InconsistentVat { .. })
    ));

    // ...the rate must match the treatment...
    let mut rate = ready(domestic_customer());
    rate.number = 7;
    rate.vat_rate_bp = 1_900;
    assert!(matches!(
        rate.render_vat_invoice(&seller()),
        Err(BillingError::InconsistentVat { .. })
    ));

    // ...and every line and total must re-derive from the rate.
    let mut line = ready(domestic_customer());
    line.number = 7;
    line.line_items[0].vat_micros += 1;
    assert!(matches!(
        line.render_vat_invoice(&seller()),
        Err(BillingError::InconsistentVat { .. })
    ));
    let mut total = ready(domestic_customer());
    total.number = 7;
    total.vat_micros += 1;
    assert!(matches!(
        total.render_vat_invoice(&seller()),
        Err(BillingError::InconsistentVat { .. })
    ));
    let mut gross = ready(domestic_customer());
    gross.number = 7;
    gross.gross_micros -= 1;
    assert!(matches!(
        gross.render_vat_invoice(&seller()),
        Err(BillingError::InconsistentVat { .. })
    ));
}

#[test]
fn register_issue_refuses_drafts_before_persisting_anything() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("invoices.jsonl");
    let register = InvoiceRegister::open(&path).expect("register");

    // A draft without a customer cannot even decide a treatment...
    let mut draft = compute_invoice(PUBLISHER, period(), &proofs()).unwrap();
    assert_eq!(
        draft
            .clone()
            .apply_vat_treatment_with_vies(&seller(), &StubVies),
        Err(BillingError::MissingCustomerProfile)
    );
    // ...and the register refuses the same draft.
    assert_eq!(
        register.issue(&mut draft),
        Err(BillingError::VatTreatmentNotApplied)
    );
    let mut customer_only = compute_invoice(PUBLISHER, period(), &proofs())
        .unwrap()
        .with_customer(domestic_customer());
    assert_eq!(
        register.issue(&mut customer_only),
        Err(BillingError::VatTreatmentNotApplied)
    );
    let mut stripped = ready(domestic_customer());
    stripped.customer = None;
    assert_eq!(
        register.issue(&mut stripped),
        Err(BillingError::MissingCustomerProfile)
    );
    assert!(!path.exists(), "refused drafts are never persisted");
    assert_eq!(draft.number, 0);
}

#[test]
fn environment_loading_refuses_incomplete_configuration() {
    // No profile file path at all: typed refusal naming the env variable.
    assert_eq!(
        BillingProfile::from_env_values(None, None),
        Err(BillingError::MissingProfileField {
            party: "seller",
            field: "APEX_BILLING_PROFILE",
        })
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("billing-profile.json");

    // A profile missing its rate (no JSON value, no override) is refused.
    std::fs::write(
        &path,
        r#"{"legalName":"Apex Media OÜ","addressLines":["Sepapaja tn 6"],"country":"EE","vatNumber":"EE123456789"}"#,
    )
    .unwrap();
    assert_eq!(
        BillingProfile::from_env_values(Some(path.to_str().unwrap()), None),
        Err(BillingError::MissingProfileField {
            party: "seller",
            field: "vatRateBp",
        })
    );

    // Rate bounds (0..=10000 bp) are enforced on the environment override.
    assert!(matches!(
        BillingProfile::from_env_values(Some(path.to_str().unwrap()), Some("10001")),
        Err(BillingError::InvalidVatRate { .. })
    ));
    std::fs::write(
        &path,
        r#"{"legalName":"Apex Media OÜ","addressLines":["Sepapaja tn 6"],"country":"EE","vatNumber":"EE123456789","vatRateBp":20000}"#,
    )
    .unwrap();
    assert!(matches!(
        BillingProfile::from_env_values(Some(path.to_str().unwrap()), None),
        Err(BillingError::InvalidVatRate { .. })
    ));

    // A complete file loads and the override wins.
    std::fs::write(
        &path,
        r#"{"legalName":"Apex Media OÜ","addressLines":["Sepapaja tn 6"],"country":"ee","vatNumber":"EE123456789","vatRateBp":1900}"#,
    )
    .unwrap();
    let profile = BillingProfile::from_env_values(Some(path.to_str().unwrap()), Some("2200"))
        .expect("complete");
    assert_eq!(profile.vat_rate_bp, 2200);
    assert_eq!(profile.country, "EE");
    let json = serde_json::to_string(&profile).expect("serialize for the console wave");
    assert!(json.contains("\"vatRateBp\":2200"), "{json}");
    assert!(json.contains("\"legalName\":\"Apex Media OÜ\""), "{json}");
}
