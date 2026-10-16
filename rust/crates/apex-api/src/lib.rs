//! apex-api — the HTTP surface of the no-SDK autopilot.
//!
//! One Axum 0.8 router serves two audiences:
//!
//! - a JSON API (`/healthz`, `/v1/audit/{platform}`, `/v1/ledger/{publisher}`,
//!   `/v1/cycle/{publisher}`, `/v1/adyen/webhook`, `/v1/leads`) for machines,
//!   and
//! - a server-rendered HTML console (`/`, `/console/audit`,
//!   `/console/ledger`, the finding/record detail pages and the free-audit
//!   request flow) that is fully functional with plain GET forms and **zero
//!   JavaScript**, so it works from any terminal browser or `curl` and adds no
//!   script surface to audit.
//!
//! ## Console rendering
//!
//! Every page is assembled from the frozen design system in `apex-ui`
//! ([`apex_ui::shell`], [`apex_ui::components`], [`apex_ui::format`],
//! [`apex_ui::status`]) by the [`html`] module, which also reads the
//! deployment context: `APEX_CONSOLE_PUBLISHER_NAME` (default
//! `Publisher workspace`), `APEX_CONSOLE_CURRENCY` (default `USD`) and the
//! deployment's read-only mode. Console mutations are POSTs with a `303`
//! result; the change-log session lives in an `HttpOnly`, host-only,
//! `SameSite=Lax` cookie holding only an opaque id ([`session`]).
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
//! elements at all, and the XSS tests drive hostile publisher names, finding
//! subjects and form values through the real router to pin that. The ledger
//! access token is only ever submitted in a POST body, never placed in a URL.

#![forbid(unsafe_code)]
// Production code is held to the strict workspace lints; tests may unwrap/expect/panic
// because a panicking test IS a failing test.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod analytics;
pub mod billing_view;
pub mod clock;
pub mod config;
pub mod connections;
pub mod cycle;
pub mod error;
pub mod experiments;
pub mod favicon;
pub mod fonts;
pub mod html;
pub mod leads;
pub mod live;
pub mod metrics;
pub mod proposals;
pub mod routes;
pub mod session;
pub mod settings;
pub mod sources;
pub mod state;
pub mod vault;

pub use analytics::AnalyticsAccess;
pub use clock::{Clock, FixedClock, SystemClock};
pub use config::{env_value, parse_keys};
pub use cycle::{
    CyclePolicy, CycleRequest, CycleRunner, CycleSummary, LedgerOnlyRunner, READ_ONLY_MODE,
    READ_ONLY_NOTE,
};
pub use error::ApiError;
pub use html::{html_escape, ConsoleCtx};
pub use leads::{validate_lead, FileLeadStore, Lead, LeadStore};
pub use live::{
    LiveAccount, LiveAccountStore, LiveAuditor, LiveCredentials, VaultedFileAccountStore,
};
pub use routes::{router, MAX_BODY_BYTES};
pub use session::{LedgerSessions, SESSION_COOKIE, SESSION_IDLE_MS};
pub use sources::{
    is_valid_publisher, parse_platform, AuditSource, FileLedgerSource, JsonFileAuditSource,
    LedgerSource, StaticAuditSource, StaticLedgerSource, PLATFORM_NAMES,
};
pub use state::AppState;
