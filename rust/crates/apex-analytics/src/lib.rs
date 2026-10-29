//! apex-analytics — the analytics and event store of the Apex product.
//!
//! The authority for this crate is the analytics chapter of the design
//! document (section 05, pages 73 to 87): the required event envelope, the
//! event catalog, the customer and control metrics, storage, privacy and
//! replay, and the acceptance checks. ClickHouse replaces the historical
//! Postgres analytics tables; this crate is the only writer and reader of it.
//!
//! Conventions enforced here:
//!
//! - Every fact is a [`envelope::DomainEvent`] with the full spec envelope.
//!   Accepted does not mean completed: an event is emitted only after its
//!   stated boundary, and the builder refuses token-looking values and raw
//!   query strings in every string field.
//! - Deduplication follows the spec: a stable source/event key has a
//!   uniqueness constraint. The key is `(source, event_id)`; the ClickHouse
//!   table is a `ReplacingMergeTree` ordered by that key, and readers use
//!   `FINAL` for read-your-writes dedup semantics.
//! - Money is integer micros with an explicit currency, always
//!   ([`queries::Money`]); missing data is `None` plus a reason, never zero.
//! - Retention follows the spec numbers: operational raw events 90 days,
//!   sanitized HTTP request logs 30 days, daily business aggregates 24
//!   months. Financial proofs, action records and ledgers carry no generic
//!   analytics TTL; they follow the contract and legal retention policy.
//! - Projections are rebuilt from source facts under a new as-of version as
//!   STAGED (unpublished) rows, compared, and only then published
//!   ([`projections::rebuild`], [`projections::compare`],
//!   [`projections::publish`]): a failed compare leaves readers on the
//!   previous version. A database created before staging existed is migrated
//!   at boot: `ensure_schema` detects the legacy sorting keys and rebuilds
//!   every staged table through a crash-safe swap before any rebuild can
//!   collapse a published version away ([`schema::staged_migration_plan`]).
//! - No personal data, platform secrets, raw authorization headers or
//!   complete query strings are stored; the access-log parser reduces the
//!   user agent to a bounded class and never copies addresses. The
//!   `public_page_requested` event it feeds carries only the bounded
//!   classification facts (route template, status, bot class, campaign
//!   parameter keys) and powers the site-wide
//!   [`queries::AnalyticsReader::page_requests_daily`] counts.

#![forbid(unsafe_code)]
// Production code is held to the strict workspace lints; tests may
// unwrap/expect/panic because a panicking test IS a failing test.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

pub mod accesslog;
pub mod envelope;
pub mod projections;
pub mod queries;
pub mod schema;
pub mod sink;
pub mod store;
pub mod timestamp;

#[cfg(test)]
pub(crate) mod test_support;

pub use envelope::{
    check_string_value, DomainEvent, DomainEventBuilder, EventFamily, EventPayload, EventRow,
    EventSource, EventType, ForbiddenClass, ValidationError,
};
pub use projections::{
    compare, compare_report, publish, rebuild, ComparisonReport, ComparisonStatus, ProjectionError,
    ProjectionName, ProjectionRebuild, ProjectionReport, ProjectionTotals,
};
pub use queries::{
    ActivationFunnel, AnalyticsReader, BreakdownDimension, ClickHouseReader, DataQuality,
    ExperimentEffectiveness, ExperimentLandscapeRow, FeeDetail, FeeRow, FeeState, FreshnessStatus,
    FunnelStageRow, GrowthCohortRow, IngestionHealthRow, LiftRow, MemoryReader, Money,
    ObjectTimelineEntry, OperatorQueues, PageRequestsDay, QueryScope, RevenueBreakdownRow,
    RevenueDailyRow, StaleSource,
};
pub use schema::{
    all_ddl, validate_identifier, SchemaError, AGGREGATE_RETENTION_MONTHS,
    RAW_EVENT_RETENTION_DAYS, REQUEST_LOG_RETENTION_DAYS,
};
pub use sink::{AnalyticsClient, EmitOutcome, EventSink, SinkError, SpoolSink};
pub use store::{
    AccountRow, ClickHouseConfig, ClickHouseStore, EventStore, Health, InsertOutcome, MemoryStore,
    ProjectionStore, ProjectionVersionRow, ReportDayRow, RequestLogRow, StagedProjection,
    StoreError,
};
pub use timestamp::{TimeError, UtcMillis};
