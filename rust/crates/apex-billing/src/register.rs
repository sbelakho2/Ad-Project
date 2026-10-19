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
//! - On [`InvoiceRegister::open`] the register is read once into an
//!   in-memory index: the highest number, every existing number, and the
//!   first number issued to each `(publisher, period)` pair.
//! - Every [`InvoiceRegister::issue`] then appends one line and updates that
//!   index in place. It never re-reads the whole register, so issuing `n`
//!   invoices over an existing history is O(n) appends after one scan, not
//!   O(n²) line parses.
//! - A different writer's append is detected by the file's hardened stamp —
//!   length, mtime, and on Unix ctime and inode (an unprivileged writer
//!   cannot restore ctime). When the stamp changes the index is re-scanned
//!   once under the lock before the next number is allocated, so a reopened
//!   or externally extended register continues without reuse.
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
//! Cross-process concurrent writers are **not** supported: allocation reads
//! the index, checks the stamp, and appends. Two processes racing on the same
//! register could still pick the same number inside that window. The
//! deployment has exactly one writer (the runner); additional writers must
//! coordinate outside this type. Before this module existed there was no
//! register writer at all (only the control plane's read-only parse), so no
//! legacy writer behavior is being changed.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

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
    state: Mutex<RegisterState>,
}

/// The register's derived facts, built once and advanced by each append.
#[derive(Debug, Default)]
struct RegisterState {
    index: RegisterIndex,
    /// The hardened identity of the file the index reflects. `None` when the
    /// file is missing or its mtime cannot be read, which forces a re-scan
    /// before the next allocation (fail-safe, never a stale number).
    stamp: Option<FileStamp>,
}

/// In-memory register facts: the highest number, the set of existing
/// numbers, and the first number issued per `(publisher, period)` pair.
#[derive(Debug, Default)]
struct RegisterIndex {
    highest: u64,
    numbers: HashSet<u64>,
    periods: HashMap<(String, String), u64>,
}

impl RegisterIndex {
    fn scan(path: &Path) -> Result<Self, BillingError> {
        #[cfg(test)]
        counters::scan();
        let raw = std::fs::read_to_string(path).map_err(|error| BillingError::RegisterCorrupt {
            detail: format!("{} could not be read: {error}", path.display()),
        })?;
        let mut index = Self::default();
        for (line_number, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            index.observe_line(line_number + 1, line)?;
        }
        Ok(index)
    }

    fn observe_line(&mut self, line_number: usize, line: &str) -> Result<(), BillingError> {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|error| BillingError::RegisterCorrupt {
                detail: format!("line {line_number} is not JSON: {error}"),
            })?;
        let number = value
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .filter(|number| *number > 0)
            .ok_or_else(|| BillingError::RegisterCorrupt {
                detail: format!("line {line_number} carries no positive invoice number"),
            })?;
        let invoice = serde_json::from_value::<Invoice>(value).map_err(|error| {
            BillingError::RegisterCorrupt {
                detail: format!("line {line_number} is not an invoice: {error}"),
            }
        })?;
        if !self.numbers.insert(number) {
            return Err(BillingError::RegisterCorrupt {
                detail: format!("line {line_number} reuses invoice number {number}"),
            });
        }
        self.periods
            .entry((invoice.publisher_id, invoice.period.year_month))
            .or_insert(number);
        self.highest = self.highest.max(number);
        Ok(())
    }

    fn next(&self, path: &Path) -> Result<u64, BillingError> {
        self.highest
            .checked_add(1)
            .ok_or_else(|| BillingError::RegisterCorrupt {
                detail: format!(
                    "register {} reached {}, no number left",
                    path.display(),
                    self.highest
                ),
            })
    }

    fn record(&mut self, number: u64, invoice: &Invoice) {
        self.highest = self.highest.max(number);
        self.numbers.insert(number);
        self.periods
            .entry((
                invoice.publisher_id.clone(),
                invoice.period.year_month.clone(),
            ))
            .or_insert(number);
    }
}

/// A file identity that an unprivileged writer cannot forge.
///
/// Length and mtime are the original key; on Unix ctime and inode are added
/// because ctime changes on every write and cannot be restored from user
/// space. The fallback keeps the original pair where those fields do not
/// exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
    #[cfg(unix)]
    ino: u64,
}

impl FileStamp {
    fn read(metadata: &std::fs::Metadata) -> Option<Self> {
        let modified = metadata.modified().ok()?;
        let len = metadata.len();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Some(Self {
                len,
                modified,
                ctime: metadata.ctime(),
                ctime_nsec: metadata.ctime_nsec(),
                ino: metadata.ino(),
            })
        }
        #[cfg(not(unix))]
        {
            Some(Self { len, modified })
        }
    }
}

#[cfg(test)]
mod counters {
    use std::cell::Cell;

    thread_local! {
        static SCANS: Cell<u64> = const { Cell::new(0) };
        static APPENDS: Cell<u64> = const { Cell::new(0) };
    }

    pub(super) fn scan() {
        SCANS.with(|count| count.set(count.get() + 1));
    }

    pub(super) fn append() {
        APPENDS.with(|count| count.set(count.get() + 1));
    }

    pub(super) fn snapshot() -> (u64, u64) {
        (SCANS.with(Cell::get), APPENDS.with(Cell::get))
    }

    pub(super) fn reset() {
        SCANS.with(|count| count.set(0));
        APPENDS.with(|count| count.set(0));
    }
}

#[cfg(test)]
fn reset_counters() {
    counters::reset();
}

#[cfg(test)]
fn counter_snapshot() -> (u64, u64) {
    counters::snapshot()
}

impl InvoiceRegister {
    /// Open (or create) the register at `path`, refusing a file whose lines
    /// are not numbered invoices. The register is scanned once here; later
    /// issues advance the in-memory index and only re-scan when the file's
    /// hardened stamp shows another writer. Parent directories are created.
    pub fn open(path: &Path) -> Result<Self, BillingError> {
        let lock = lock_for(path)?;
        let register = Self {
            path: path.to_path_buf(),
            lock,
            state: Mutex::new(RegisterState::default()),
        };
        let guard = register.lock()?;
        register.refresh_locked()?;
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
        self.refresh_locked()?;
        let state = self.state()?;
        Ok(state.index.highest)
    }

    /// The number already issued to `publisher_id` for `year_month`, if the
    /// register carries one. This is the per-subscriber/period dedup fact the
    /// index maintains; it never allocates a number.
    pub fn issued_number(
        &self,
        publisher_id: &str,
        year_month: &str,
    ) -> Result<Option<u64>, BillingError> {
        let _guard = self.lock()?;
        self.refresh_locked()?;
        let state = self.state()?;
        Ok(state
            .index
            .periods
            .get(&(publisher_id.to_string(), year_month.to_string()))
            .copied())
    }

    /// The already-issued invoice for `(publisher, year_month)`, when the
    /// register holds one. The register is the authority for numbering, so a
    /// retried monthly pass can return the issued document (and its number)
    /// instead of allocating a second number for the same period.
    pub fn stored_invoice(
        &self,
        publisher_id: &str,
        year_month: &str,
    ) -> Result<Option<Invoice>, BillingError> {
        let _guard = self.lock()?;
        self.refresh_locked()?;
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(BillingError::RegisterCorrupt {
                    detail: format!("{} could not be read: {error}", self.path.display()),
                })
            }
        };
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let value: serde_json::Value =
                serde_json::from_str(line).map_err(|error| BillingError::RegisterCorrupt {
                    detail: format!("register line is not JSON: {error}"),
                })?;
            let invoice: Invoice =
                serde_json::from_value(value).map_err(|error| BillingError::RegisterCorrupt {
                    detail: format!("register line is not an invoice: {error}"),
                })?;
            if invoice.publisher_id == publisher_id && invoice.period.year_month == year_month {
                return Ok(Some(invoice));
            }
        }
        Ok(None)
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
        self.refresh_locked()?;
        let mut state = self.state()?;
        let number = state.index.next(&self.path)?;
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
        #[cfg(test)]
        counters::append();
        // The file is the authority; the in-memory index advances only on a
        // successful append. Its stamp is refreshed from the same handle so
        // the next issue on this handle takes the no-rescan fast path.
        state.index.record(number, invoice);
        state.stamp = file.metadata().ok().and_then(|m| FileStamp::read(&m));
        Ok(number)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, BillingError> {
        self.lock.lock().map_err(|_| BillingError::RegisterCorrupt {
            detail: "register lock poisoned".into(),
        })
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, RegisterState>, BillingError> {
        self.state
            .lock()
            .map_err(|_| BillingError::RegisterCorrupt {
                detail: "register state lock poisoned".into(),
            })
    }

    /// Bring the in-memory index up to date with the file, re-scanning only
    /// when the hardened stamp changed (or cannot be read). The caller holds
    /// the process-wide register lock.
    fn refresh_locked(&self) -> Result<(), BillingError> {
        let mut state = self.state()?;
        let metadata = std::fs::metadata(&self.path).ok();
        let Some(metadata) = metadata else {
            // A missing file is an empty register. Keep the empty index and
            // remember that there is no stamp to trust.
            if state.stamp.is_some() || !state.index.numbers.is_empty() {
                state.index = RegisterIndex::default();
            }
            state.stamp = None;
            return Ok(());
        };
        let stamp = FileStamp::read(&metadata);
        if stamp.is_some() && stamp == state.stamp {
            return Ok(());
        }
        state.index = RegisterIndex::scan(&self.path)?;
        state.stamp = stamp;
        Ok(())
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

    #[test]
    fn opening_ten_thousand_lines_then_issuing_a_thousand_scans_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invoices.jsonl");
        let mut body = String::new();
        for number in 1..=10_000u64 {
            let mut invoice = prepared_invoice();
            invoice.number = number;
            body.push_str(&serde_json::to_string(&invoice).unwrap());
            body.push('\n');
        }
        std::fs::write(&path, body).unwrap();

        reset_counters();
        let register = InvoiceRegister::open(&path).unwrap();
        assert_eq!(counter_snapshot(), (1, 0), "open scans the register once");
        for _ in 0..1_000 {
            let mut invoice = prepared_invoice();
            register.issue(&mut invoice).unwrap();
        }
        // One scan for 10,000 existing lines; 1,000 issues are appends only.
        assert_eq!(
            counter_snapshot(),
            (1, 1_000),
            "issues never re-read the register"
        );
        assert_eq!(register.highest().unwrap(), 11_000);
        assert_eq!(
            register.issued_number("pub-1", "2026-10").unwrap(),
            Some(1),
            "the per-subscriber/period index keeps the first number"
        );
    }

    #[test]
    fn an_external_append_between_allocation_is_discovered_by_the_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invoices.jsonl");
        append_fixture(&path, 1);
        let register = InvoiceRegister::open(&path).unwrap();
        assert_eq!(register.highest().unwrap(), 1);

        // Another writer appends outside this handle. The next allocation
        // must re-scan (the stamp changed), not reuse number 2.
        append_fixture(&path, 2);
        let mut next = prepared_invoice();
        assert_eq!(register.issue(&mut next).unwrap(), 3);
        assert_eq!(
            register.issued_number("pub-1", "2026-10").unwrap(),
            Some(1),
            "the re-scan rebuilt the period index from the file"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
    }

    #[test]
    fn issue_appends_the_byte_identical_invoice_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invoices.jsonl");
        let register = InvoiceRegister::open(&path).unwrap();
        let mut invoice = prepared_invoice();
        assert_eq!(register.issue(&mut invoice).unwrap(), 1);
        // The golden is the exact control-plane register line: one camelCase
        // Invoice, fields in declaration order, no wrapper and no extra keys,
        // followed by a single newline.
        let golden = concat!(
            r#"{"number":1,"publisherId":"pub-1","period":{"yearMonth":"2026-10","startUnixMs":0,"endUnixMs":1000},"#,
            r#""lineItems":[],"totalMicros":0,"vatRateBp":2000,"vatMicros":0,"grossMicros":0,"#,
            r#""customer":{"legalName":"Buyer OÜ","addressLines":["Telliskivi 1"],"country":"EE","vatNumber":"EE111111111"},"#,
            r#""vatTreatment":"domestic"}"#,
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{golden}\n"),
            "a register line is exactly one camelCase Invoice plus newline"
        );
    }

    #[test]
    fn concurrent_handles_issue_distinct_consecutive_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invoices.jsonl");
        let handles: Vec<InvoiceRegister> = (0..4)
            .map(|_| InvoiceRegister::open(&path).unwrap())
            .collect();
        let issued: Vec<u64> = std::thread::scope(|scope| {
            let workers: Vec<_> = handles
                .iter()
                .map(|register| {
                    scope.spawn(move || {
                        (0..25)
                            .map(|_| {
                                let mut invoice = prepared_invoice();
                                register.issue(&mut invoice).unwrap()
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|worker| worker.join().unwrap())
                .collect()
        });
        let mut numbers = issued;
        numbers.sort_unstable();
        assert_eq!(
            numbers,
            (1..=100).collect::<Vec<u64>>(),
            "100 issues across four handles are gapless and unique"
        );
    }

    /// n/2n/4n evidence: one scan regardless of history size, then one append
    /// per issue. Timings are printed, never asserted.
    // dogfood:skip: this evidence test is deliberately ignored in the default
    // suite and is run explicitly with `--ignored --nocapture`.
    #[test]
    #[ignore = "n/2n/4n register scaling evidence; run with --ignored --nocapture"]
    fn register_scales_linearly_in_lines_and_issues() {
        for n in [2_500usize, 5_000, 10_000] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("invoices.jsonl");
            let mut body = String::new();
            for number in 1..=n as u64 {
                let mut invoice = prepared_invoice();
                invoice.number = number;
                body.push_str(&serde_json::to_string(&invoice).unwrap());
                body.push('\n');
            }
            std::fs::write(&path, body).unwrap();

            reset_counters();
            let started = std::time::Instant::now();
            let register = InvoiceRegister::open(&path).unwrap();
            let open_ms = started.elapsed().as_secs_f64() * 1000.0;
            let issues = n / 2;
            let started = std::time::Instant::now();
            for _ in 0..issues {
                let mut invoice = prepared_invoice();
                register.issue(&mut invoice).unwrap();
            }
            let issue_ms = started.elapsed().as_secs_f64() * 1000.0;
            let (scans, appends) = counter_snapshot();
            eprintln!(
                "REGISTER lines={n:>6} issues={issues:>6} open={open_ms:8.1}ms \
                 issue={issue_ms:8.1}ms scans={scans} appends={appends}"
            );
            assert_eq!(scans, 1);
            assert_eq!(appends, issues as u64);
            assert_eq!(register.highest().unwrap(), (n + issues) as u64);
        }
    }
}
