//! Gapless invoice numbering in the invoice register.
//!
//! The register is the append-only JSONL file of issued invoices — the same
//! `.apex/billing/invoices.jsonl` the control plane reads read-only
//! (`APEX_CONTROL_INVOICE_REGISTER`), which is why a line here is exactly one
//! [`Invoice`] serialized camelCase with its `number` set. Extra fields are
//! ignored by that reader, so older deploy-time readers keep working.
//!
//! # Numbering guarantee
//!
//! - The next number is `highest existing number + 1`, re-scanned from the
//!   file **under the lock at every allocation**: a reopened or externally
//!   extended register continues without reuse, and a number is never handed
//!   out twice in this process.
//! - Allocation and the append are serialized per register path by a
//!   process-wide lock shared by every [`InvoiceRegister`] handle for the
//!   same path, so concurrent threads issuing invoices receive distinct,
//!   consecutive numbers even when they hold separate handles.
//! - The whole line (JSON plus newline) is appended to a file opened with
//!   `O_APPEND` in a single `write_all` call, so a reader can never observe a
//!   torn or interleaved line written by this process.
//! - The register only validates: a line that is not an invoice, carries no
//!   positive number, or repeats a number is a typed
//!   [`BillingError::RegisterCorrupt`] refusal — never silently skipped,
//!   because a corrupt register cannot prove gaplessness.
//!
//! Cross-process concurrent writers are **not** supported: allocation is
//! `scan highest -> append` and two processes racing on the same register
//! could pick the same number. The deployment has exactly one writer (the
//! runner); additional writers must coordinate outside this type. Before this
//! module existed there was no register writer at all (only the control
//! plane's read-only parse), so no legacy writer behavior is being changed.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::errors::BillingError;
use crate::invoice::Invoice;
use crate::vat::VatTreatment;

/// The register path the deployment uses unless configured otherwise
/// (`.apex/billing/invoices.jsonl`, the control plane's read-only default).
pub const DEFAULT_REGISTER_PATH: &str = ".apex/billing/invoices.jsonl";

type PathLocks = Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>;

static PATH_LOCKS: OnceLock<PathLocks> = OnceLock::new();

/// An append-only invoice register with gapless sequential numbering.
///
/// Open it once at startup ([`InvoiceRegister::open`]), then issue each
/// prepared invoice through [`InvoiceRegister::issue`]; see the module docs
/// for the exact numbering guarantee.
#[derive(Debug)]
pub struct InvoiceRegister {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl InvoiceRegister {
    /// Open (or create) the register at `path`, refusing a file whose lines
    /// are not numbered invoices. Parent directories are created.
    pub fn open(path: &Path) -> Result<Self, BillingError> {
        let lock = lock_for(path)?;
        let register = Self {
            path: path.to_path_buf(),
            lock,
        };
        let guard = register.lock()?;
        if register.path.exists() {
            Self::scan(&register.path)?;
        }
        if let Some(parent) = register.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| BillingError::RegisterCorrupt {
                    detail: format!("{}: parent directory: {error}", register.path.display()),
                })?;
            }
        }
        drop(guard);
        Ok(register)
    }

    /// The highest number already allocated (0 when the register is missing
    /// or empty).
    pub fn highest(&self) -> Result<u64, BillingError> {
        let _guard = self.lock()?;
        if !self.path.exists() {
            return Ok(0);
        }
        Self::scan(&self.path)
    }

    /// Assign the next gapless number to `invoice` and append it to the
    /// register in one atomic step. The invoice must be issue-ready: a draft
    /// without an applied VAT treatment or a customer profile is refused
    /// (never persisted half-empty), as is an invoice that already carries a
    /// number. If the append fails, the assigned number is rolled back —
    /// a number that was never persisted was never issued.
    pub fn issue(&self, invoice: &mut Invoice) -> Result<u64, BillingError> {
        if invoice.number != 0 {
            return Err(BillingError::RegisterCorrupt {
                detail: format!(
                    "invoice already carries register number {}; numbers are never reused",
                    invoice.number
                ),
            });
        }
        if invoice.vat_treatment == VatTreatment::NotApplied {
            return Err(BillingError::VatTreatmentNotApplied);
        }
        if invoice.customer.is_none() {
            return Err(BillingError::MissingCustomerProfile);
        }
        let _guard = self.lock()?;
        let highest = if self.path.exists() {
            Self::scan(&self.path)?
        } else {
            0
        };
        let number = highest
            .checked_add(1)
            .ok_or(BillingError::RegisterCorrupt {
                detail: format!(
                    "register {} reached {highest}, no number left",
                    self.path.display()
                ),
            })?;
        invoice.number = number;
        let mut line = match serde_json::to_string(invoice) {
            Ok(line) => line,
            Err(error) => {
                invoice.number = 0;
                return Err(BillingError::RegisterCorrupt {
                    detail: format!("invoice could not be serialized: {error}"),
                });
            }
        };
        line.push('\n');
        // One `write_all` of the complete line on an `O_APPEND` handle: the
        // append is atomic for readers (see the module docs). Every failure
        // from here rolls the number back: a number that was never persisted
        // was never issued and must never be rendered.
        let mut file = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) => {
                invoice.number = 0;
                return Err(BillingError::RegisterCorrupt {
                    detail: format!(
                        "{} could not be opened for append: {error}",
                        self.path.display()
                    ),
                });
            }
        };
        if let Err(error) = file.write_all(line.as_bytes()) {
            invoice.number = 0;
            return Err(BillingError::RegisterCorrupt {
                detail: format!("{} could not be appended to: {error}", self.path.display()),
            });
        }
        Ok(number)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, BillingError> {
        self.lock.lock().map_err(|_| BillingError::RegisterCorrupt {
            detail: "register lock poisoned".into(),
        })
    }

    /// Re-scan the register: every non-empty line must be an invoice with a
    /// positive, unique number. Returns the highest number seen.
    fn scan(path: &Path) -> Result<u64, BillingError> {
        let raw = std::fs::read_to_string(path).map_err(|error| BillingError::RegisterCorrupt {
            detail: format!("{} could not be read: {error}", path.display()),
        })?;
        let mut highest: u64 = 0;
        let mut seen: HashSet<u64> = HashSet::new();
        for (index, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let line_number = index + 1;
            let value: serde_json::Value =
                serde_json::from_str(line).map_err(|error| BillingError::RegisterCorrupt {
                    detail: format!("line {line_number} is not JSON: {error}"),
                })?;
            let number = value
                .get("number")
                .and_then(serde_json::Value::as_u64)
                .filter(|number| *number > 0)
                .ok_or(BillingError::RegisterCorrupt {
                    detail: format!("line {line_number} carries no positive invoice number"),
                })?;
            serde_json::from_value::<Invoice>(value).map_err(|error| {
                BillingError::RegisterCorrupt {
                    detail: format!("line {line_number} is not an invoice: {error}"),
                }
            })?;
            if !seen.insert(number) {
                return Err(BillingError::RegisterCorrupt {
                    detail: format!("line {line_number} reuses invoice number {number}"),
                });
            }
            highest = highest.max(number);
        }
        Ok(highest)
    }
}

/// The process-wide lock for one register path, so separate handles on the
/// same file (same resolved path) still serialize their allocations.
fn lock_for(path: &Path) -> Result<Arc<Mutex<()>>, BillingError> {
    let key = std::path::absolute(path).map_err(|error| BillingError::RegisterCorrupt {
        detail: format!(
            "register path {} cannot be resolved: {error}",
            path.display()
        ),
    })?;
    let locks = PATH_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks.lock().map_err(|_| BillingError::RegisterCorrupt {
        detail: "register lock registry poisoned".into(),
    })?;
    Ok(locks.entry(key).or_default().clone())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::invoice::{compute_invoice, BillingPeriod};
    use crate::profile::{BillingProfile, CustomerProfile};

    fn seller() -> BillingProfile {
        BillingProfile {
            legal_name: "Apex Media OÜ".into(),
            address_lines: vec!["Sepapaja tn 6".into()],
            country: "EE".into(),
            vat_number: "EE123456789".into(),
            vat_rate_bp: 2000,
        }
    }

    fn prepared_invoice() -> Invoice {
        let period = BillingPeriod {
            year_month: "2026-10".into(),
            start_unix_ms: 0,
            end_unix_ms: 1_000,
        };
        compute_invoice("pub-1", period, &[])
            .unwrap()
            .with_customer(CustomerProfile {
                legal_name: "Buyer OÜ".into(),
                address_lines: vec!["Telliskivi 1".into()],
                country: "EE".into(),
                vat_number: Some("EE111111111".into()),
            })
            .apply_vat_treatment(&seller())
            .unwrap()
    }

    fn append_fixture(path: &Path, number: u64) {
        let mut invoice = prepared_invoice();
        invoice.number = number;
        let mut line = serde_json::to_string(&invoice).unwrap();
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(line.as_bytes()).unwrap();
    }

    #[test]
    fn issues_gapless_numbers_and_persists_the_invoice_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("billing").join("invoices.jsonl");
        let register = InvoiceRegister::open(&path).unwrap();
        assert_eq!(register.highest().unwrap(), 0);

        let mut first = prepared_invoice();
        assert_eq!(register.issue(&mut first).unwrap(), 1);
        assert_eq!(first.number, 1);
        let mut second = prepared_invoice();
        assert_eq!(register.issue(&mut second).unwrap(), 2);
        assert_eq!(register.highest().unwrap(), 2);

        // The persisted line is the issued invoice, camelCase, ready for the
        // control plane's read-only parser; `number` carries the sequence.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 2);
        for line in raw.lines() {
            assert!(line.contains("\"number\":"), "{line}");
            assert!(line.contains("\"publisherId\":\"pub-1\""), "{line}");
            assert!(line.contains("\"lineItems\":["), "{line}");
            assert!(line.contains("\"totalMicros\":"), "{line}");
            assert!(line.contains("\"vatTreatment\":\"domestic\""), "{line}");
        }
    }

    #[test]
    fn continues_from_the_highest_existing_number_never_reusing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invoices.jsonl");
        for number in [1, 2, 3] {
            append_fixture(&path, number);
        }
        let register = InvoiceRegister::open(&path).unwrap();
        assert_eq!(register.highest().unwrap(), 3);
        let mut next = prepared_invoice();
        assert_eq!(register.issue(&mut next).unwrap(), 4);

        // A fresh handle re-reads the file and continues, never reusing.
        let reopened = InvoiceRegister::open(&path).unwrap();
        let mut after = prepared_invoice();
        assert_eq!(reopened.issue(&mut after).unwrap(), 5);
        assert_eq!(reopened.highest().unwrap(), 5);
    }

    #[test]
    fn issue_refuses_drafts_and_numbered_invoices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invoices.jsonl");
        let register = InvoiceRegister::open(&path).unwrap();

        let mut numbered = prepared_invoice();
        numbered.number = 9;
        assert!(matches!(
            register.issue(&mut numbered),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("never reused")
        ));

        let period = BillingPeriod {
            year_month: "2026-10".into(),
            start_unix_ms: 0,
            end_unix_ms: 1_000,
        };
        let mut bare = compute_invoice("pub-1", period.clone(), &[]).unwrap();
        assert_eq!(
            register.issue(&mut bare),
            Err(BillingError::VatTreatmentNotApplied)
        );
        let mut customer_only = compute_invoice("pub-1", period, &[])
            .unwrap()
            .with_customer(CustomerProfile {
                legal_name: "Buyer OÜ".into(),
                address_lines: vec!["Telliskivi 1".into()],
                country: "EE".into(),
                vat_number: None,
            });
        assert_eq!(
            register.issue(&mut customer_only),
            Err(BillingError::VatTreatmentNotApplied)
        );
        // Treatment applied but the customer stripped back out: refused.
        let mut treated = prepared_invoice();
        treated.customer = None;
        assert_eq!(
            register.issue(&mut treated),
            Err(BillingError::MissingCustomerProfile)
        );
        // Nothing was appended by any refusal.
        assert!(!path.exists(), "refusals must not create the register");
    }

    #[test]
    fn corrupt_registers_are_refused_not_skipped() {
        let dir = tempfile::tempdir().unwrap();

        let not_json = dir.path().join("not-json.jsonl");
        std::fs::write(&not_json, "not json\n").unwrap();
        assert!(matches!(
            InvoiceRegister::open(&not_json),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("not JSON")
        ));

        let no_number = dir.path().join("no-number.jsonl");
        std::fs::write(
            &no_number,
            format!("{}\n", serde_json::json!({"publisherId": "pub-1"})),
        )
        .unwrap();
        assert!(matches!(
            InvoiceRegister::open(&no_number),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("no positive invoice number")
        ));

        let zero_number = dir.path().join("zero-number.jsonl");
        let zero = prepared_invoice();
        assert_eq!(zero.number, 0);
        std::fs::write(
            &zero_number,
            format!("{}\n", serde_json::to_string(&zero).unwrap()),
        )
        .unwrap();
        assert!(matches!(
            InvoiceRegister::open(&zero_number),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("no positive invoice number")
        ));

        let duplicate = dir.path().join("duplicate.jsonl");
        append_fixture(&duplicate, 3);
        append_fixture(&duplicate, 3);
        assert!(matches!(
            InvoiceRegister::open(&duplicate),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("reuses invoice number 3")
        ));

        // A line that is valid JSON with a number but not an invoice.
        let not_invoice = dir.path().join("not-invoice.jsonl");
        std::fs::write(&not_invoice, "{\"number\":7}\n").unwrap();
        assert!(matches!(
            InvoiceRegister::open(&not_invoice),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("is not an invoice")
        ));
    }

    #[test]
    fn open_reports_unusable_paths_with_typed_errors() {
        let dir = tempfile::tempdir().unwrap();

        // An empty path cannot be resolved at all.
        assert!(matches!(
            InvoiceRegister::open(Path::new("")),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("cannot be resolved")
        ));

        // The parent directory cannot be created: a file is in the way.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        assert!(matches!(
            InvoiceRegister::open(&blocker.join("invoices.jsonl")),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("parent directory")
        ));

        // The register path exists but cannot be read as a register (it is a
        // directory): fail closed instead of skipping the scan.
        let as_dir = dir.path().join("as-dir");
        std::fs::create_dir_all(&as_dir).unwrap();
        assert!(matches!(
            InvoiceRegister::open(&as_dir),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("could not be read")
        ));
    }

    #[test]
    fn issue_reports_an_unwritable_register_and_rolls_the_number_back() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invoices.jsonl");
        append_fixture(&path, 1);
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o444);
        std::fs::set_permissions(&path, permissions).unwrap();

        let register = InvoiceRegister::open(&path).unwrap();
        let mut next = prepared_invoice();
        assert!(matches!(
            register.issue(&mut next),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("could not be opened for append")
        ));
        assert_eq!(
            next.number, 0,
            "a number that was never persisted is rolled back"
        );
        assert_eq!(register.highest().unwrap(), 1);

        // Leave the file writable again for clean teardown.
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(&path, permissions).unwrap();
    }

    #[test]
    fn an_exhausted_register_refuses_instead_of_wrapping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invoices.jsonl");
        append_fixture(&path, u64::MAX);
        let register = InvoiceRegister::open(&path).unwrap();
        let mut next = prepared_invoice();
        assert!(matches!(
            register.issue(&mut next),
            Err(BillingError::RegisterCorrupt { ref detail }) if detail.contains("no number left")
        ));
        assert_eq!(next.number, 0, "the numberless invoice stays numberless");
    }
}
