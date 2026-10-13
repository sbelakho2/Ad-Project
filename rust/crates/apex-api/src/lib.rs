//! apex-api — the HTTP surface of the no-SDK autopilot.
//!
//! One Axum 0.8 router serves two audiences:
//!
//! - a JSON API (`/healthz`, `/v1/audit/{platform}`, `/v1/ledger/{publisher}`,
//!   `/v1/cycle/{publisher}`, `/v1/stripe/webhook`) for machines, and
//! - a server-rendered HTML console (`/`, `/console/audit`, `/console/ledger`)
//!   that is fully functional with plain GET forms and **zero JavaScript**, so
//!   it works from any terminal browser or `curl` and adds no script surface
//!   to audit.
//!
//! ## Read-only deployment mode
//!
//! The cycle endpoint runs a [`CycleRunner`]. The one concrete runner shipped
//! here, [`LedgerOnlyRunner`], is a real implementation of this deployment's
//! only mode: it validates the request's policy inputs, re-runs the audit for
//! the named platform, reads (and thereby re-verifies) the publisher's ledger,
//! and reports the current state — `"mode":"read_only"` — without mutating
//! anything. Applying a write cycle would require platform management
//! credentials, which this deployment does not hold; the response says so
//! explicitly rather than pretending otherwise.
//!
//! ## Data sources
//!
//! Snapshots come from [`JsonFileAuditSource`] (`<root>/<platform>.json`) and
//! ledgers from [`FileLedgerSource`] (`<root>/<publisher>.jsonl`), which
//! re-verifies every line with `apex_proof::Ledger::verify` before serving a
//! single record. Publisher names are the *only* file names derived from the
//! URL, and they are restricted to `^[A-Za-z0-9_-]{1,64}$` (anything else is a
//! typed 400), so no path traversal is reachable.
//!
//! ## HTML safety
//!
//! Every piece of dynamic text on the console goes through
//! [`html_escape`] (`&`, `<`, `>`, `"`, `'`); the pages contain no `<script>`
//! elements at all, and the XSS tests drive hostile publisher names and
//! finding subjects through the real router to pin that.

#![forbid(unsafe_code)]
// Production code is held to the strict workspace lints; tests may unwrap/expect/panic
// because a panicking test IS a failing test.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod clock;
pub mod cycle;
pub mod error;
pub mod html;
pub mod routes;
pub mod sources;
pub mod state;

pub use clock::{Clock, FixedClock, SystemClock};
pub use cycle::{
    CyclePolicy, CycleRequest, CycleRunner, CycleSummary, LedgerOnlyRunner, READ_ONLY_MODE,
    READ_ONLY_NOTE,
};
pub use error::ApiError;
pub use html::{
    html_escape, render_audit_form, render_audit_page, render_error_page, render_ledger_form,
    render_ledger_records, render_overview,
};
pub use routes::{router, MAX_BODY_BYTES};
pub use sources::{
    is_valid_publisher, parse_platform, AuditSource, FileLedgerSource, JsonFileAuditSource,
    LedgerSource, StaticAuditSource, StaticLedgerSource, PLATFORM_NAMES,
};
pub use state::{AppState, DEFAULT_STRIPE_TOLERANCE_SECS};
