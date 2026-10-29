//! Transactional billing storage: the SQLite [`InvoiceRegistry`].
//!
//! The registry is the transactional index and constraint layer beside the
//! append-only files: the signed proof ledger and the intent journal stay
//! exactly as they are (they are audit artifacts), while this database makes
//! the *relational facts* of billing impossible to duplicate and cheap to
//! look up:
//!
//! - `invoices` — one row per issued invoice, with
//!   **`UNIQUE(publisher, period)`** (one invoice per publisher and billing
//!   period, enforced by the database rather than a scan) and
//!   **`UNIQUE(invoice_number)`** (a number is never reused);
//! - `payment_allocations` — money received per provider payment, with
//!   **`UNIQUE(provider, external_payment_id)`** (the same provider payment
//!   can never be allocated twice: the double-collection guard);
//! - `intent_payment_index` — the provider-payment → intent lookup rows the
//!   [`PaymentIntentStore`](crate::intent::PaymentIntentStore) resolves
//!   through, also unique per `(provider, external_payment_id)`.
//!
//! ## Consistency
//!
//! Every write runs in one SQLite transaction (`IMMEDIATE`), and the
//! connection is opened with `foreign_keys=ON`, `journal_mode=WAL` and
//! `synchronous=FULL`, so a committed fact survives a crash and an
//! allocation can never reference a missing invoice. Uniqueness conflicts
//! and missing references surface as typed refusals
//! ([`BillingError::Registry`], [`BillingError::InvoicePeriodAlreadyIssued`])
//! — never silently ignored. A file that is not a usable database is refused
//! at [`InvoiceRegistry::open`].
//!
//! ## Numbering
//!
//! [`InvoiceRegistry::next_invoice_number`] continues the gapless sequence
//! from the highest issued number. The sequence is **global**, not
//! per publisher, because `UNIQUE(invoice_number)` is a global constraint —
//! the `publisher` argument scopes and validates the caller's intent and is
//! the fact `record_invoice` stores against. Concurrent writers in one
//! process serialize on the connection; concurrent processes serialize on
//! SQLite's own write lock, and the UNIQUE constraint backstops both.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::errors::BillingError;
use crate::mollie::BILLING_CURRENCY;

/// The registry path the deployment uses unless configured otherwise
/// (beside the invoice register and the intent journal).
pub const DEFAULT_REGISTRY_PATH: &str = ".apex/billing/registry.sqlite3";

/// The provider label this deployment records. The registry keys its
/// allocation and lookup tables per provider so a second PSP can be added
/// without touching the schema.
pub const PROVIDER_MOLLIE: &str = "mollie";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS invoices (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_number    INTEGER NOT NULL UNIQUE,
    publisher         TEXT    NOT NULL,
    period            TEXT    NOT NULL,
    reference         TEXT    NOT NULL,
    total_minor_units INTEGER NOT NULL,
    currency          TEXT    NOT NULL,
    created_unix_ms   INTEGER NOT NULL,
    UNIQUE (publisher, period)
);

CREATE TABLE IF NOT EXISTS payment_allocations (
    allocation_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    provider            TEXT    NOT NULL,
    external_payment_id TEXT    NOT NULL,
    invoice_number      INTEGER NOT NULL REFERENCES invoices(invoice_number),
    amount_minor_units  INTEGER NOT NULL,
    currency            TEXT    NOT NULL,
    allocated_unix_ms   INTEGER NOT NULL,
    UNIQUE (provider, external_payment_id)
);

CREATE TABLE IF NOT EXISTS intent_payment_index (
    lookup_id           INTEGER PRIMARY KEY AUTOINCREMENT,
    provider            TEXT    NOT NULL,
    external_payment_id TEXT    NOT NULL,
    intent_id           TEXT    NOT NULL,
    line_reference      TEXT    NOT NULL,
    updated_unix_ms     INTEGER NOT NULL,
    UNIQUE (provider, external_payment_id)
);

CREATE INDEX IF NOT EXISTS intent_payment_index_line
    ON intent_payment_index (line_reference);
"#;

/// One issued invoice as the registry stores it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceRecord {
    /// The gapless register number (`UNIQUE(invoice_number)`).
    pub invoice_number: u64,
    pub publisher: String,
    /// The billing period label (`YYYY-MM`).
    pub period: String,
    /// The invoice reference (`INV-<month>-<publisher>`).
    pub reference: String,
    /// The invoice total in minor units.
    pub total_minor_units: i64,
    pub currency: String,
    pub created_unix_ms: i64,
}

/// One money-received row: a provider payment allocated to an invoice.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentAllocation {
    /// The provider label ([`PROVIDER_MOLLIE`] for this deployment).
    pub provider: String,
    /// The provider's own payment id (`tr_...` for Mollie).
    pub external_payment_id: String,
    /// The invoice the money pays.
    pub invoice_number: u64,
    /// The allocated amount in minor units.
    pub amount_minor_units: i64,
    pub currency: String,
    pub allocated_unix_ms: i64,
}

/// What [`InvoiceRegistry::record_payment_allocation`] did. Recording the
/// same provider payment again is an idempotent no-op that returns the
/// existing row, so a replayed webhook or a rerun pass never double-counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentAllocationOutcome {
    /// The allocation was recorded.
    Inserted(PaymentAllocation),
    /// The allocation already existed (`UNIQUE(provider, external_payment_id)`);
    /// the stored row is returned unchanged.
    AlreadyRecorded(PaymentAllocation),
}

impl PaymentAllocationOutcome {
    /// The allocation row either way.
    #[must_use]
    pub fn allocation(&self) -> &PaymentAllocation {
        match self {
            Self::Inserted(allocation) | Self::AlreadyRecorded(allocation) => allocation,
        }
    }

    /// Whether this call was the one that recorded the allocation.
    #[must_use]
    pub fn is_new(&self) -> bool {
        matches!(self, Self::Inserted(_))
    }
}

/// The transactional billing registry. Open it once per process and share it
/// (`Arc`); every method is a short transaction on one connection.
pub struct InvoiceRegistry {
    path: PathBuf,
    connection: Mutex<Connection>,
}

impl std::fmt::Debug for InvoiceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The connection is never named: the registry's facts belong to the
        // deployment's storage, not to a debug log.
        f.debug_struct("InvoiceRegistry")
            .field("path", &self.path)
            .finish()
    }
}

fn refused(detail: impl std::fmt::Display) -> BillingError {
    BillingError::Registry {
        detail: detail.to_string(),
    }
}

impl InvoiceRegistry {
    /// Open (or create) the registry at `path`. Parent directories are
    /// created; a file that is not a usable SQLite database is a typed
    /// refusal.
    pub fn open(path: &Path) -> Result<Self, BillingError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    refused(format!("{}: parent directory: {error}", path.display()))
                })?;
            }
        }
        let connection = Connection::open(path)
            .map_err(|error| refused(format!("{} could not be opened: {error}", path.display())))?;
        Self::configure(connection, path.to_path_buf())
    }

    /// Open a private in-memory registry: the shape the tests and one-shot
    /// tools use.
    pub fn open_in_memory() -> Result<Self, BillingError> {
        let connection = Connection::open_in_memory()
            .map_err(|error| refused(format!("in-memory registry: {error}")))?;
        Self::configure(connection, PathBuf::from(":memory:"))
    }

    fn configure(connection: Connection, path: PathBuf) -> Result<Self, BillingError> {
        // Durability first: a committed allocation or invoice number must
        // survive a crash, and a foreign key must never dangle.
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 PRAGMA foreign_keys = ON;",
            )
            .map_err(|error| refused(format!("{} pragmas: {error}", path.display())))?;
        connection
            .execute_batch(SCHEMA)
            .map_err(|error| refused(format!("{} schema: {error}", path.display())))?;
        Ok(Self {
            path,
            connection: Mutex::new(connection),
        })
    }

    /// The registry path (`:memory:` for an in-memory registry).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run one immediate (write-locking) transaction under the connection
    /// lock. SQLite's own lock plus the UNIQUE constraints make concurrent
    /// handles safe; this mutex only keeps one thread per handle.
    fn write<T>(
        &self,
        work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, BillingError>,
    ) -> Result<T, BillingError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| refused("registry connection lock poisoned"))?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| refused(format!("transaction: {error}")))?;
        let produced = work(&transaction)?;
        transaction
            .commit()
            .map_err(|error| refused(format!("commit: {error}")))?;
        Ok(produced)
    }

    /// The next gapless invoice number. The sequence is global (see the
    /// module docs): `UNIQUE(invoice_number)` leaves no per-publisher
    /// sequences. The publisher argument is validated and names the invoice
    /// the caller is about to issue.
    pub fn next_invoice_number(&self, publisher: &str) -> Result<u64, BillingError> {
        validate_publisher(publisher)?;
        self.write(|tx| self.next_invoice_number_tx(tx))
    }

    /// Record one issued invoice and assign its number in one transaction:
    /// the allocation and the uniqueness enforcement are atomic. A second
    /// invoice for the same `(publisher, period)` is the typed
    /// [`BillingError::InvoicePeriodAlreadyIssued`]; a number collision can
    /// only fire across processes and is a typed [`BillingError::Registry`].
    #[allow(clippy::too_many_arguments)]
    pub fn record_invoice(
        &self,
        publisher: &str,
        period: &str,
        reference: &str,
        total_minor_units: i64,
        currency: &str,
        now_unix_ms: i64,
    ) -> Result<InvoiceRecord, BillingError> {
        validate_publisher(publisher)?;
        validate_period(period)?;
        validate_currency(currency)?;
        if reference.trim().is_empty() {
            return Err(refused("invoice reference must not be empty"));
        }
        if total_minor_units < 0 {
            return Err(refused(format!(
                "invoice total {total_minor_units} minor units is negative"
            )));
        }
        self.write(|tx| {
            // One invoice per (publisher, period), checked inside the same
            // transaction that inserts: the UNIQUE constraint is the
            // backstop, this check is the typed message.
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT invoice_number FROM invoices WHERE publisher = ?1 AND period = ?2",
                    [publisher, period],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| refused(format!("issued invoice lookup: {error}")))?;
            if let Some(number) = existing {
                let number = u64::try_from(number).unwrap_or(0);
                return Err(BillingError::InvoicePeriodAlreadyIssued {
                    publisher_id: publisher.to_string(),
                    year_month: period.to_string(),
                    number,
                });
            }
            let number = self.next_invoice_number_tx(tx)?;
            let record = InvoiceRecord {
                invoice_number: number,
                publisher: publisher.to_string(),
                period: period.to_string(),
                reference: reference.trim().to_string(),
                total_minor_units,
                currency: currency.to_string(),
                created_unix_ms: now_unix_ms,
            };
            let inserted = tx
                .execute(
                    "INSERT INTO invoices
                        (invoice_number, publisher, period, reference, total_minor_units, currency, created_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        i64::try_from(number).unwrap_or(i64::MAX),
                        record.publisher,
                        record.period,
                        record.reference,
                        record.total_minor_units,
                        record.currency,
                        record.created_unix_ms,
                    ],
                )
                .map_err(|error| refused(format!("invoice insert: {error}")))?;
            debug_assert_eq!(inserted, 1);
            Ok(record)
        })
    }

    /// The next number inside an open transaction.
    fn next_invoice_number_tx(&self, tx: &rusqlite::Transaction<'_>) -> Result<u64, BillingError> {
        let highest: Option<i64> = tx
            .query_row("SELECT MAX(invoice_number) FROM invoices", [], |row| {
                row.get(0)
            })
            .map_err(|error| refused(format!("highest invoice number: {error}")))?;
        let highest = highest.unwrap_or(0);
        u64::try_from(highest)
            .ok()
            .and_then(|highest| highest.checked_add(1))
            .ok_or_else(|| {
                refused(format!(
                    "registry {} reached {highest}, no invoice number left",
                    self.path.display()
                ))
            })
    }

    /// The invoice already issued to a publisher for a period, when any.
    pub fn issued_invoice(
        &self,
        publisher: &str,
        period: &str,
    ) -> Result<Option<InvoiceRecord>, BillingError> {
        validate_publisher(publisher)?;
        validate_period(period)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| refused("registry connection lock poisoned"))?;
        let mut statement = connection
            .prepare(
                "SELECT invoice_number, publisher, period, reference, total_minor_units, currency, created_unix_ms
                 FROM invoices WHERE publisher = ?1 AND period = ?2",
            )
            .map_err(|error| refused(format!("issued invoice lookup: {error}")))?;
        statement
            .query_row([publisher, period], row_to_invoice)
            .optional()
            .map_err(|error| refused(format!("issued invoice lookup: {error}")))
    }

    /// Record money received against an invoice. The same provider payment
    /// can never be recorded twice: a replay returns
    /// [`PaymentAllocationOutcome::AlreadyRecorded`] with the stored row.
    pub fn record_payment_allocation(
        &self,
        provider: &str,
        external_payment_id: &str,
        invoice_number: u64,
        amount_minor_units: i64,
        currency: &str,
        now_unix_ms: i64,
    ) -> Result<PaymentAllocationOutcome, BillingError> {
        validate_provider(provider)?;
        let external_payment_id = external_payment_id.trim();
        if external_payment_id.is_empty() {
            return Err(refused("external payment id must not be empty"));
        }
        validate_currency(currency)?;
        if amount_minor_units <= 0 {
            return Err(refused(format!(
                "allocation amount {amount_minor_units} minor units is not positive"
            )));
        }
        let invoice = i64::try_from(invoice_number)
            .map_err(|_| refused(format!("invoice number {invoice_number} is out of range")))?;
        self.write(|tx| {
            // Idempotent replay: the UNIQUE(provider, external_payment_id)
            // row already exists, return it unchanged.
            if let Some(existing) = allocation_for(tx, provider, external_payment_id)? {
                return Ok(PaymentAllocationOutcome::AlreadyRecorded(existing));
            }
            let allocation = PaymentAllocation {
                provider: provider.to_string(),
                external_payment_id: external_payment_id.to_string(),
                invoice_number,
                amount_minor_units,
                currency: currency.to_string(),
                allocated_unix_ms: now_unix_ms,
            };
            // The foreign key refuses an allocation to a missing invoice
            // (fail closed); a UNIQUE race across processes re-reads and
            // reports the stored row instead of failing.
            let inserted = tx.execute(
                "INSERT INTO payment_allocations
                    (provider, external_payment_id, invoice_number, amount_minor_units, currency, allocated_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    allocation.provider,
                    allocation.external_payment_id,
                    invoice,
                    allocation.amount_minor_units,
                    allocation.currency,
                    allocation.allocated_unix_ms,
                ],
            );
            if let Err(error) = inserted {
                if is_unique_violation(&error) {
                    let existing =
                        allocation_for(tx, provider, external_payment_id)?.ok_or_else(|| {
                            refused(format!(
                                "allocation race for {provider}:{external_payment_id} resolved to no row"
                            ))
                        })?;
                    return Ok(PaymentAllocationOutcome::AlreadyRecorded(existing));
                }
                if is_foreign_key_violation(&error) {
                    return Err(refused(format!(
                        "allocation names invoice {invoice_number}, which the registry does not hold"
                    )));
                }
                return Err(refused(format!("allocation insert: {error}")));
            }
            Ok(PaymentAllocationOutcome::Inserted(allocation))
        })
    }

    /// The balance a publisher still owes: the invoiced totals minus the
    /// money allocated, in minor units. A publisher invoiced or paid in more
    /// than one currency is a typed refusal — a single mixed-currency number
    /// would be a fabricated figure.
    pub fn outstanding_balance(&self, publisher: &str) -> Result<i64, BillingError> {
        validate_publisher(publisher)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| refused("registry connection lock poisoned"))?;
        let currencies: Vec<String> = {
            let mut statement = connection
                .prepare(
                    "SELECT DISTINCT currency FROM invoices WHERE publisher = ?1
                     UNION
                     SELECT DISTINCT a.currency FROM payment_allocations a
                     JOIN invoices i ON i.invoice_number = a.invoice_number
                     WHERE i.publisher = ?1",
                )
                .map_err(|error| refused(format!("currency scan: {error}")))?;
            let rows = statement
                .query_map([publisher], |row| row.get::<_, String>(0))
                .map_err(|error| refused(format!("currency scan: {error}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| refused(format!("currency scan: {error}")))?
        };
        if currencies
            .iter()
            .any(|currency| currency != BILLING_CURRENCY)
        {
            return Err(refused(format!(
                "publisher {publisher} carries mixed currencies {currencies:?}; a single outstanding balance cannot be stated"
            )));
        }
        connection
            .query_row(
                "SELECT
                    (SELECT COALESCE(SUM(total_minor_units), 0) FROM invoices WHERE publisher = ?1)
                  - (SELECT COALESCE(SUM(a.amount_minor_units), 0)
                     FROM payment_allocations a
                     JOIN invoices i ON i.invoice_number = a.invoice_number
                     WHERE i.publisher = ?1)",
                [publisher],
                |row| row.get(0),
            )
            .map_err(|error| refused(format!("outstanding balance: {error}")))
    }

    /// Record the (provider payment id → intent) lookup row the intent store
    /// resolves webhooks through. One provider payment id maps to exactly one
    /// intent forever: a conflicting second mapping is a typed refusal, never
    /// a silent overwrite.
    pub fn record_intent_payment(
        &self,
        provider: &str,
        external_payment_id: &str,
        intent_id: &str,
        line_reference: &str,
        now_unix_ms: i64,
    ) -> Result<(), BillingError> {
        validate_provider(provider)?;
        for value in [external_payment_id, intent_id, line_reference] {
            if value.trim().is_empty() {
                return Err(refused(
                    "intent lookup row needs an external payment id, intent id and line reference",
                ));
            }
        }
        self.write(|tx| {
            let existing: Option<(String, String)> = tx
                .query_row(
                    "SELECT intent_id, line_reference FROM intent_payment_index
                     WHERE provider = ?1 AND external_payment_id = ?2",
                    [provider, external_payment_id.trim()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|error| refused(format!("intent lookup: {error}")))?;
            match existing {
                Some((existing_intent, existing_reference)) => {
                    if existing_intent != intent_id || existing_reference != line_reference {
                        return Err(refused(format!(
                            "provider payment {provider}:{external_payment_id} is recorded for intent {existing_intent} ({existing_reference}), refusing {intent_id} ({line_reference})"
                        )));
                    }
                    // The same fact recorded again: touch nothing but the
                    // stamp, keeping the replay idempotent.
                    tx.execute(
                        "UPDATE intent_payment_index SET updated_unix_ms = ?1
                         WHERE provider = ?2 AND external_payment_id = ?3",
                        rusqlite::params![now_unix_ms, provider, external_payment_id.trim()],
                    )
                    .map_err(|error| refused(format!("intent lookup stamp: {error}")))?;
                    Ok(())
                }
                None => {
                    tx.execute(
                        "INSERT INTO intent_payment_index
                            (provider, external_payment_id, intent_id, line_reference, updated_unix_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![
                            provider,
                            external_payment_id.trim(),
                            intent_id,
                            line_reference.trim(),
                            now_unix_ms,
                        ],
                    )
                    .map_err(|error| refused(format!("intent lookup insert: {error}")))?;
                    Ok(())
                }
            }
        })
    }

    /// The intent id a provider payment belongs to, from the UNIQUE index.
    pub fn intent_for_provider_payment(
        &self,
        provider: &str,
        external_payment_id: &str,
    ) -> Result<Option<String>, BillingError> {
        validate_provider(provider)?;
        if external_payment_id.trim().is_empty() {
            return Ok(None);
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| refused("registry connection lock poisoned"))?;
        connection
            .query_row(
                "SELECT intent_id FROM intent_payment_index
                 WHERE provider = ?1 AND external_payment_id = ?2",
                [provider, external_payment_id.trim()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| refused(format!("intent lookup: {error}")))
    }

    /// Every intent id recorded for one line reference (indexed, deduplicated
    /// in insertion order). One intent can appear twice when a refused charge
    /// and its typed fallback each carry their own provider payment id.
    pub fn intent_ids_for_line_reference(
        &self,
        provider: &str,
        line_reference: &str,
    ) -> Result<Vec<String>, BillingError> {
        validate_provider(provider)?;
        if line_reference.trim().is_empty() {
            return Ok(Vec::new());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| refused("registry connection lock poisoned"))?;
        let mut statement = connection
            .prepare(
                "SELECT intent_id FROM intent_payment_index
                 WHERE provider = ?1 AND line_reference = ?2 ORDER BY lookup_id",
            )
            .map_err(|error| refused(format!("line lookup: {error}")))?;
        let rows = statement
            .query_map([provider, line_reference.trim()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| refused(format!("line lookup: {error}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| refused(format!("line lookup: {error}")))
    }
}

fn row_to_invoice(row: &rusqlite::Row<'_>) -> Result<InvoiceRecord, rusqlite::Error> {
    let number: i64 = row.get(0)?;
    Ok(InvoiceRecord {
        invoice_number: u64::try_from(number).unwrap_or(0),
        publisher: row.get(1)?,
        period: row.get(2)?,
        reference: row.get(3)?,
        total_minor_units: row.get(4)?,
        currency: row.get(5)?,
        created_unix_ms: row.get(6)?,
    })
}

fn allocation_for(
    tx: &rusqlite::Transaction<'_>,
    provider: &str,
    external_payment_id: &str,
) -> Result<Option<PaymentAllocation>, BillingError> {
    tx.query_row(
        "SELECT a.external_payment_id, a.invoice_number, a.amount_minor_units, a.currency, a.allocated_unix_ms
         FROM payment_allocations a
         WHERE a.provider = ?1 AND a.external_payment_id = ?2",
        [provider, external_payment_id],
        |row| {
            let invoice_number: i64 = row.get(1)?;
            Ok(PaymentAllocation {
                provider: provider.to_string(),
                external_payment_id: row.get(0)?,
                invoice_number: u64::try_from(invoice_number).unwrap_or(0),
                amount_minor_units: row.get(2)?,
                currency: row.get(3)?,
                allocated_unix_ms: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(|error| refused(format!("allocation lookup: {error}")))
}

/// Whether a SQLite error is a UNIQUE-constraint violation (a concurrent
/// writer stored the row first).
fn is_unique_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

/// Whether a SQLite error is a foreign-key violation (the referenced invoice
/// does not exist).
fn is_foreign_key_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
    )
}

fn validate_provider(provider: &str) -> Result<(), BillingError> {
    if provider.trim().is_empty() {
        Err(refused("provider label must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_publisher(publisher: &str) -> Result<(), BillingError> {
    if publisher.trim().is_empty() {
        Err(refused("publisher must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_period(period: &str) -> Result<(), BillingError> {
    if period.trim().is_empty() {
        Err(refused("billing period must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_currency(currency: &str) -> Result<(), BillingError> {
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        Err(refused(format!("currency {currency:?} is not an ISO code")))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn registry() -> InvoiceRegistry {
        InvoiceRegistry::open_in_memory().expect("registry")
    }

    fn invoice(registry: &InvoiceRegistry, publisher: &str, period: &str) -> InvoiceRecord {
        registry
            .record_invoice(
                publisher,
                period,
                &format!("INV-{period}-{publisher}"),
                1_000,
                BILLING_CURRENCY,
                1_000,
            )
            .expect("record invoice")
    }

    #[test]
    fn invoice_numbers_are_gapless_and_globally_unique() {
        let registry = registry();
        assert_eq!(registry.next_invoice_number("pub-1").expect("next"), 1);

        let first = invoice(&registry, "pub-1", "2026-10");
        let second = invoice(&registry, "pub-1", "2026-11");
        let other = invoice(&registry, "pub-2", "2026-10");
        assert_eq!(first.invoice_number, 1);
        assert_eq!(
            second.invoice_number, 2,
            "the sequence continues, never reuses"
        );
        assert_eq!(
            other.invoice_number, 3,
            "numbering is global: UNIQUE(invoice_number)"
        );
        assert_eq!(registry.next_invoice_number("pub-1").expect("next"), 4);

        // A peek allocates nothing: recording still gets the next number.
        let fourth = invoice(&registry, "pub-1", "2026-12");
        assert_eq!(fourth.invoice_number, 4);

        assert!(
            registry.next_invoice_number(" ").is_err(),
            "an empty publisher is refused"
        );
    }

    #[test]
    fn one_invoice_per_publisher_and_period_is_enforced_by_the_database() {
        let registry = registry();
        invoice(&registry, "pub-1", "2026-10");
        let error = registry
            .record_invoice(
                "pub-1",
                "2026-10",
                "INV-2026-10-pub-1",
                1_000,
                BILLING_CURRENCY,
                2_000,
            )
            .expect_err("duplicate period");
        assert_eq!(
            error,
            BillingError::InvoicePeriodAlreadyIssued {
                publisher_id: "pub-1".into(),
                year_month: "2026-10".into(),
                number: 1,
            }
        );
        // A different publisher may invoice the same period.
        invoice(&registry, "pub-2", "2026-10");
        assert!(registry
            .issued_invoice("pub-1", "2026-10")
            .expect("read")
            .is_some());
        assert!(registry
            .issued_invoice("pub-3", "2026-10")
            .expect("read")
            .is_none());
    }

    #[test]
    fn registrations_are_validated_before_any_write() {
        let registry = registry();
        assert!(registry
            .record_invoice("", "2026-10", "r", 1, "USD", 0)
            .is_err());
        assert!(registry
            .record_invoice("pub-1", " ", "r", 1, "USD", 0)
            .is_err());
        assert!(registry
            .record_invoice("pub-1", "2026-10", " ", 1, "USD", 0)
            .is_err());
        assert!(registry
            .record_invoice("pub-1", "2026-10", "r", -1, "USD", 0)
            .is_err());
        assert!(registry
            .record_invoice("pub-1", "2026-10", "r", 1, "usd", 0)
            .is_err());
        assert!(registry
            .record_invoice("pub-1", "2026-10", "r", 1, "EURO", 0)
            .is_err());
        assert!(
            registry
                .issued_invoice("pub-1", "2026-10")
                .expect("read")
                .is_none(),
            "a refused invoice recorded nothing"
        );
    }

    #[test]
    fn allocations_are_idempotent_per_provider_payment() {
        let registry = registry();
        let record = invoice(&registry, "pub-1", "2026-10");

        let first = registry
            .record_payment_allocation(
                "mollie",
                "tr_1",
                record.invoice_number,
                400,
                BILLING_CURRENCY,
                1_000,
            )
            .expect("allocate");
        assert!(first.is_new());
        // The same provider payment again is the stored row, not a new one.
        let replay = registry
            .record_payment_allocation(
                "mollie",
                "tr_1",
                record.invoice_number,
                400,
                BILLING_CURRENCY,
                2_000,
            )
            .expect("replay");
        assert!(!replay.is_new());
        assert_eq!(replay.allocation(), first.allocation());
        assert_eq!(
            replay.allocation().allocated_unix_ms,
            1_000,
            "the original row stands"
        );

        // A different provider id is a distinct allocation, even against the
        // same invoice; the same id from another provider is distinct too.
        assert!(registry
            .record_payment_allocation(
                "mollie",
                "tr_2",
                record.invoice_number,
                600,
                BILLING_CURRENCY,
                3_000
            )
            .expect("second")
            .is_new());
        assert!(registry
            .record_payment_allocation(
                "stripe",
                "tr_1",
                record.invoice_number,
                1,
                BILLING_CURRENCY,
                3_000
            )
            .expect("other provider")
            .is_new());

        // Refusals: unknown invoice (foreign key), empty ids, bad amounts.
        assert!(registry
            .record_payment_allocation("mollie", "tr_9", 999, 100, BILLING_CURRENCY, 4_000)
            .is_err());
        assert!(registry
            .record_payment_allocation(
                "mollie",
                " ",
                record.invoice_number,
                100,
                BILLING_CURRENCY,
                4_000
            )
            .is_err());
        assert!(registry
            .record_payment_allocation(
                "mollie",
                "tr_3",
                record.invoice_number,
                0,
                BILLING_CURRENCY,
                4_000
            )
            .is_err());
    }

    #[test]
    fn outstanding_balance_is_invoiced_minus_allocated() {
        let registry = registry();
        let first = invoice(&registry, "pub-1", "2026-10");
        let second = invoice(&registry, "pub-1", "2026-11");
        assert_eq!(
            registry.outstanding_balance("pub-1").expect("balance"),
            2_000
        );

        registry
            .record_payment_allocation(
                "mollie",
                "tr_1",
                first.invoice_number,
                400,
                BILLING_CURRENCY,
                1_000,
            )
            .expect("allocate");
        assert_eq!(
            registry.outstanding_balance("pub-1").expect("balance"),
            1_600
        );

        registry
            .record_payment_allocation(
                "mollie",
                "tr_2",
                second.invoice_number,
                1_600,
                BILLING_CURRENCY,
                2_000,
            )
            .expect("allocate");
        assert_eq!(registry.outstanding_balance("pub-1").expect("balance"), 0);

        // Another publisher's allocations never touch this balance.
        let other = invoice(&registry, "pub-2", "2026-10");
        registry
            .record_payment_allocation(
                "mollie",
                "tr_3",
                other.invoice_number,
                1_000,
                BILLING_CURRENCY,
                3_000,
            )
            .expect("allocate");
        assert_eq!(registry.outstanding_balance("pub-1").expect("balance"), 0);
        assert_eq!(registry.outstanding_balance("pub-2").expect("balance"), 0);

        // A second currency for the same publisher cannot state one number.
        registry
            .record_invoice("pub-1", "2026-12", "INV-2026-12-pub-1", 500, "EUR", 4_000)
            .expect("euro invoice");
        assert!(registry.outstanding_balance("pub-1").is_err());
    }

    #[test]
    fn the_intent_index_maps_one_provider_payment_to_one_intent() {
        let registry = registry();
        registry
            .record_intent_payment("mollie", "tr_1", "pi:ref-a", "ref-a", 1_000)
            .expect("record");
        assert_eq!(
            registry
                .intent_for_provider_payment("mollie", "tr_1")
                .expect("lookup")
                .as_deref(),
            Some("pi:ref-a")
        );
        // The same fact again is idempotent.
        registry
            .record_intent_payment("mollie", "tr_1", "pi:ref-a", "ref-a", 2_000)
            .expect("replay");
        // A conflicting mapping is refused, never overwritten.
        assert!(registry
            .record_intent_payment("mollie", "tr_1", "pi:ref-b", "ref-b", 3_000)
            .is_err());
        assert_eq!(
            registry
                .intent_for_provider_payment("mollie", "tr_1")
                .expect("lookup")
                .as_deref(),
            Some("pi:ref-a"),
            "the first mapping stands"
        );

        // The line index: one intent, two recorded payment ids (a refusal
        // and its fallback), reported once each in insertion order.
        registry
            .record_intent_payment("mollie", "tr_2", "pi:ref-a", "ref-a", 3_000)
            .expect("second id, same intent");
        registry
            .record_intent_payment("mollie", "tr_3", "pi:ref-c", "ref-a", 4_000)
            .expect("another intent, same line");
        assert_eq!(
            registry
                .intent_ids_for_line_reference("mollie", "ref-a")
                .expect("line index"),
            vec![
                "pi:ref-a".to_string(),
                "pi:ref-a".to_string(),
                "pi:ref-c".to_string()
            ],
            "insertion order, duplicates included for the caller to dedup"
        );
        assert!(registry
            .intent_ids_for_line_reference("mollie", "unknown")
            .expect("line index")
            .is_empty());
        assert_eq!(
            registry
                .intent_for_provider_payment("mollie", "unknown")
                .expect("lookup"),
            None
        );
        assert!(registry
            .record_intent_payment("mollie", "tr_4", " ", "ref-x", 5_000)
            .is_err());
    }

    #[test]
    fn a_file_that_is_not_a_database_is_a_typed_refusal() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("registry.sqlite3");
        std::fs::write(&path, b"definitely not a sqlite database\n").expect("write");
        let error = InvoiceRegistry::open(&path).expect_err("refusal");
        assert!(matches!(error, BillingError::Registry { .. }), "{error:?}");
    }

    #[test]
    fn the_registry_persists_and_continues_across_handles() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("billing").join("registry.sqlite3");
        let record = {
            let registry = InvoiceRegistry::open(&path).expect("open");
            invoice(&registry, "pub-1", "2026-10")
        };
        // A fresh handle (another process) continues the sequence and sees
        // the stored facts.
        let registry = InvoiceRegistry::open(&path).expect("reopen");
        assert_eq!(
            registry.next_invoice_number("pub-1").expect("next"),
            record.invoice_number + 1
        );
        assert_eq!(
            registry.issued_invoice("pub-1", "2026-10").expect("read"),
            Some(record)
        );
    }

    /// One transaction per record: a half-recorded invoice (number allocated
    /// but insert refused) must not consume the number. The UNIQUE
    /// constraints are exercised through concurrent handles on one file.
    #[test]
    fn concurrent_handles_cannot_duplicate_the_unique_facts() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("registry.sqlite3");
        let handles: Vec<InvoiceRegistry> = (0..4)
            .map(|_| InvoiceRegistry::open(&path).expect("open"))
            .collect();
        let numbers: Vec<u64> = std::thread::scope(|scope| {
            let workers: Vec<_> = handles
                .iter()
                .enumerate()
                .map(|(worker, registry)| {
                    scope.spawn(move || {
                        (0..10)
                            .map(|index| {
                                invoice(
                                    registry,
                                    &format!("pub-{worker}"),
                                    &format!("2026-10-{index:02}"),
                                )
                                .invoice_number
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|worker| worker.join().expect("joined"))
                .collect()
        });
        let mut sorted = numbers;
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            (1..=40).collect::<Vec<u64>>(),
            "40 invoices across four handles carry 40 distinct gapless numbers"
        );
    }
}
