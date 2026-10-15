//! ClickHouse DDL as pure, testable SQL builders.
//!
//! The durable model follows the spec's minimal model (page 86): the raw
//! `domain_events` table plus the lifecycle and finance tables the spec
//! names (`accounts`, `connections`, `source_snapshot_index`, `report_days`,
//! `audit_runs`, `findings`, `proposals`, `experiments`, `decisions`,
//! `fee_lines`, `invoices`, `payment_events`, `jobs`, `incidents`) and the
//! read projections (`customer_daily`, `platform_daily`,
//! `lifecycle_cohorts`, `operator_queues`).
//!
//! Retention uses the exact spec numbers (page 86, Retention proposal):
//! operational raw events 90 days, sanitized HTTP request logs 30 days,
//! daily business aggregates 24 months. Financial proofs, action records and
//! ledgers carry no generic analytics TTL; they follow the contract and
//! legal retention policy, so their tables deliberately have none.
//!
//! Every builder validates the database identifier first: DDL interpolates
//! the database name (a deployment constant), never user input.

/// Operational raw events live for 90 days (spec page 86).
pub const RAW_EVENT_RETENTION_DAYS: u32 = 90;
/// Sanitized HTTP request logs live for 30 days (spec page 86).
pub const REQUEST_LOG_RETENTION_DAYS: u32 = 30;
/// Daily business aggregates live for 24 months (spec page 86).
pub const AGGREGATE_RETENTION_MONTHS: u32 = 24;

/// Schema building errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    /// A database or table identifier is not a plain SQL identifier.
    #[error("'{0}' is not a valid ClickHouse identifier")]
    InvalidIdentifier(String),
}

/// Validate a ClickHouse identifier (`[A-Za-z_][A-Za-z0-9_]*`, at most 128
/// bytes). Refusing anything else makes interpolated DDL injection-free.
pub fn validate_identifier(name: &str) -> Result<&str, SchemaError> {
    let mut bytes = name.bytes();
    let valid = match bytes.next() {
        Some(first) if first.is_ascii_alphabetic() || first == b'_' => {
            bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }
        _ => false,
    };
    if valid && name.len() <= 128 {
        Ok(name)
    } else {
        Err(SchemaError::InvalidIdentifier(name.to_string()))
    }
}

fn db(name: &str) -> Result<&str, SchemaError> {
    validate_identifier(name)
}

/// `CREATE DATABASE IF NOT EXISTS`, the first statement of [`all_ddl`].
pub fn create_database(database: &str) -> Result<String, SchemaError> {
    Ok(format!("CREATE DATABASE IF NOT EXISTS {}", db(database)?))
}

/// The raw event table.
///
/// The dedup key is `(source, event_id)`: the spec's stable source/event key
/// ("A stable source/event key has a uniqueness constraint", page 86).
/// `ReplacingMergeTree(version)` keeps the row with the greatest
/// `version` (milliseconds of `received_at_utc`) per key at merge time;
/// readers use `FINAL` for read-your-writes semantics. A replay of the same
/// event carries the same key, so it collapses to one row while a corrected
/// re-emit with a later received time wins deterministically.
pub fn domain_events(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.domain_events (\n\
         \x20   event_id String,\n\
         \x20   event_type LowCardinality(String),\n\
         \x20   schema_version UInt16,\n\
         \x20   occurred_at_utc DateTime64(3, 'UTC'),\n\
         \x20   received_at_utc DateTime64(3, 'UTC'),\n\
         \x20   version UInt64 DEFAULT toUnixTimestamp64Milli(received_at_utc),\n\
         \x20   environment LowCardinality(String),\n\
         \x20   service_version String,\n\
         \x20   source LowCardinality(String),\n\
         \x20   publisher_id Nullable(String),\n\
         \x20   platform LowCardinality(Nullable(String)),\n\
         \x20   object_type Nullable(String),\n\
         \x20   object_id Nullable(String),\n\
         \x20   correlation_id Nullable(String),\n\
         \x20   causation_id Nullable(String),\n\
         \x20   outcome Nullable(String),\n\
         \x20   reason_code Nullable(String),\n\
         \x20   currency LowCardinality(Nullable(String)),\n\
         \x20   amount_micros Nullable(Int64),\n\
         \x20   period Nullable(String),\n\
         \x20   lead_id Nullable(String),\n\
         \x20   account_id Nullable(String),\n\
         \x20   job_id Nullable(String),\n\
         \x20   experiment_id Nullable(String),\n\
         \x20   change_id Nullable(String),\n\
         \x20   proposal_id Nullable(String),\n\
         \x20   invoice_id Nullable(String),\n\
         \x20   incident_id Nullable(String),\n\
         \x20   platform_operation Nullable(String),\n\
         \x20   finality LowCardinality(Nullable(String)),\n\
         \x20   rule_version Nullable(String),\n\
         \x20   snapshot_digest Nullable(String),\n\
         \x20   guard_type Nullable(String),\n\
         \x20   payload_json String\n\
         ) ENGINE = ReplacingMergeTree(version)\n\
         PARTITION BY toYYYYMM(occurred_at_utc)\n\
         ORDER BY (source, event_id)\n\
         TTL occurred_at_utc + INTERVAL {RAW_EVENT_RETENTION_DAYS} DAY\n\
         SETTINGS index_granularity = 8192"
    ))
}

/// Sanitized HTTP request logs: route template, status, bounded classes and
/// allow-listed query keys only. No addresses, no raw URL, no user agent; a
/// 30 day TTL per the spec.
pub fn request_logs(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.request_logs (\n\
         \x20   event_id String,\n\
         \x20   occurred_at_utc DateTime64(3, 'UTC'),\n\
         \x20   received_at_utc DateTime64(3, 'UTC'),\n\
         \x20   route_template LowCardinality(String),\n\
         \x20   status UInt16,\n\
         \x20   status_class LowCardinality(String),\n\
         \x20   bot_class LowCardinality(String),\n\
         \x20   referrer_host Nullable(String),\n\
         \x20   campaign_code Nullable(String),\n\
         \x20   query_keys Array(String),\n\
         \x20   environment LowCardinality(String)\n\
         ) ENGINE = MergeTree\n\
         PARTITION BY toYYYYMM(occurred_at_utc)\n\
         ORDER BY (route_template, occurred_at_utc, event_id)\n\
         TTL occurred_at_utc + INTERVAL {REQUEST_LOG_RETENTION_DAYS} DAY"
    ))
}

/// Account inventory: current lifecycle stage per tenant account.
pub fn accounts(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.accounts (\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   account_id String,\n\
         \x20   stage LowCardinality(String),\n\
         \x20   owner Nullable(String),\n\
         \x20   platforms Array(String),\n\
         \x20   first_seen DateTime64(3, 'UTC'),\n\
         \x20   stage_updated DateTime64(3, 'UTC'),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (publisher_id, account_id)"
    ))
}

/// Credential scope and connection state per tenant platform account.
pub fn connections(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.connections (\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   platform LowCardinality(String),\n\
         \x20   platform_account_id String,\n\
         \x20   read_capability Bool,\n\
         \x20   write_capability Bool,\n\
         \x20   state LowCardinality(String),\n\
         \x20   validator_version String,\n\
         \x20   validated_at DateTime64(3, 'UTC'),\n\
         \x20   failure_class Nullable(String),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (publisher_id, platform, platform_account_id)"
    ))
}

/// Digest index of captured source snapshots.
pub fn source_snapshot_index(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.source_snapshot_index (\n\
         \x20   snapshot_digest String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   platform LowCardinality(String),\n\
         \x20   captured_at DateTime64(3, 'UTC'),\n\
         \x20   report_start Date,\n\
         \x20   report_end Date,\n\
         \x20   rows UInt64,\n\
         \x20   finality LowCardinality(String),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY snapshot_digest"
    ))
}

/// Finalized platform report days: the revenue source facts.
///
/// Money is integer micros with an explicit currency; a missing revenue or
/// impression count is NULL and never zero. Breakdown dimensions (country,
/// ad unit, format) are nullable because "App/ad unit, country and format
/// only where source fields exist" (page 51). This table has no TTL: report
/// days are financial source facts, not raw telemetry.
pub fn report_days(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.report_days (\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   platform LowCardinality(String),\n\
         \x20   platform_account_id String,\n\
         \x20   day Date,\n\
         \x20   finality LowCardinality(String),\n\
         \x20   revenue_micros Nullable(Int64),\n\
         \x20   impressions Nullable(UInt64),\n\
         \x20   currency LowCardinality(Nullable(String)),\n\
         \x20   country LowCardinality(Nullable(String)),\n\
         \x20   ad_unit_id Nullable(String),\n\
         \x20   format LowCardinality(Nullable(String)),\n\
         \x20   source_digest Nullable(String),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (publisher_id, platform, platform_account_id, day, country, ad_unit_id, format)\n\
         SETTINGS allow_nullable_key = 1"
    ))
}

/// Audit executions per snapshot and rule version.
pub fn audit_runs(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.audit_runs (\n\
         \x20   snapshot_digest String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   platform LowCardinality(String),\n\
         \x20   rule_version String,\n\
         \x20   eligible_groups UInt32,\n\
         \x20   expected_groups UInt32,\n\
         \x20   findings_critical UInt32,\n\
         \x20   findings_warning UInt32,\n\
         \x20   findings_info UInt32,\n\
         \x20   missing_inputs UInt32,\n\
         \x20   duration_ms UInt64,\n\
         \x20   ran_at DateTime64(3, 'UTC'),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (snapshot_digest, rule_version)"
    ))
}

/// Stable findings of an audit snapshot.
pub fn findings(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.findings (\n\
         \x20   finding_id String,\n\
         \x20   snapshot_digest String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   rule String,\n\
         \x20   severity LowCardinality(String),\n\
         \x20   scope_type LowCardinality(String),\n\
         \x20   scope_id String,\n\
         \x20   model_version Nullable(String),\n\
         \x20   estimated_recovery_micros Nullable(Int64),\n\
         \x20   currency LowCardinality(Nullable(String)),\n\
         \x20   disposition LowCardinality(String),\n\
         \x20   created_at DateTime64(3, 'UTC'),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (snapshot_digest, finding_id)"
    ))
}

/// Change proposals bound to snapshot, change hash and policy version.
pub fn proposals(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.proposals (\n\
         \x20   proposal_id String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   platform LowCardinality(String),\n\
         \x20   snapshot String,\n\
         \x20   change_hash String,\n\
         \x20   policy_version String,\n\
         \x20   state LowCardinality(String),\n\
         \x20   approval_actor Nullable(String),\n\
         \x20   expiry Nullable(DateTime64(3, 'UTC')),\n\
         \x20   created_at DateTime64(3, 'UTC'),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY proposal_id"
    ))
}

/// Registered platform experiments.
pub fn experiments(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.experiments (\n\
         \x20   experiment_id String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   platform LowCardinality(String),\n\
         \x20   scope String,\n\
         \x20   state LowCardinality(String),\n\
         \x20   registered_metric String,\n\
         \x20   window_days UInt32,\n\
         \x20   source_digest String,\n\
         \x20   started_at Nullable(DateTime64(3, 'UTC')),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY experiment_id"
    ))
}

/// Signed decisions per experiment.
pub fn decisions(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.decisions (\n\
         \x20   result_hash String,\n\
         \x20   experiment_id String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   decision LowCardinality(String),\n\
         \x20   reason_class Nullable(String),\n\
         \x20   point_estimate_bp Nullable(Int64),\n\
         \x20   decision_bound_bp Nullable(Int64),\n\
         \x20   decided_at DateTime64(3, 'UTC'),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (experiment_id, result_hash)"
    ))
}

/// Fee lines with their proof references. Amounts are integer micros plus an
/// explicit currency; no analytics TTL (contract and legal retention).
pub fn fee_lines(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.fee_lines (\n\
         \x20   fee_line_id String,\n\
         \x20   change_id String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   invoice_id Nullable(String),\n\
         \x20   state LowCardinality(String),\n\
         \x20   period String,\n\
         \x20   currency LowCardinality(String),\n\
         \x20   source_amount_micros Int64,\n\
         \x20   bound_bp Int64,\n\
         \x20   factor_bp Nullable(Int64),\n\
         \x20   rate_bp UInt16,\n\
         \x20   active_days UInt32,\n\
         \x20   fee_micros Int64,\n\
         \x20   proof_hashes Array(String),\n\
         \x20   created_at DateTime64(3, 'UTC'),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (change_id, period, fee_line_id)"
    ))
}

/// Issued invoices. Tax, monitoring and credits stay distinct columns.
pub fn invoices(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.invoices (\n\
         \x20   invoice_id String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   currency LowCardinality(String),\n\
         \x20   issued_at Nullable(DateTime64(3, 'UTC')),\n\
         \x20   due_at Nullable(DateTime64(3, 'UTC')),\n\
         \x20   total_micros Int64,\n\
         \x20   tax_micros Int64,\n\
         \x20   monitoring_micros Int64,\n\
         \x20   credit_micros Int64,\n\
         \x20   state LowCardinality(String),\n\
         \x20   calculation_version String,\n\
         \x20   proof_refs Array(String),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY invoice_id"
    ))
}

/// Verified payment events, deduplicated by PSP event key.
pub fn payment_events(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.payment_events (\n\
         \x20   psp_event_key String,\n\
         \x20   provider LowCardinality(String),\n\
         \x20   event_class LowCardinality(String),\n\
         \x20   result LowCardinality(String),\n\
         \x20   invoice_id Nullable(String),\n\
         \x20   publisher_id LowCardinality(Nullable(String)),\n\
         \x20   amount_micros Nullable(Int64),\n\
         \x20   currency LowCardinality(Nullable(String)),\n\
         \x20   received_at DateTime64(3, 'UTC'),\n\
         \x20   reconciled_at Nullable(DateTime64(3, 'UTC')),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY psp_event_key"
    ))
}

/// Scheduled jobs and their durable outcomes.
pub fn jobs(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.jobs (\n\
         \x20   job_id String,\n\
         \x20   job_type LowCardinality(String),\n\
         \x20   platform LowCardinality(Nullable(String)),\n\
         \x20   publisher_id LowCardinality(Nullable(String)),\n\
         \x20   schedule DateTime64(3, 'UTC'),\n\
         \x20   state LowCardinality(String),\n\
         \x20   durable_outcome Nullable(String),\n\
         \x20   output_digest Nullable(String),\n\
         \x20   started_at Nullable(DateTime64(3, 'UTC')),\n\
         \x20   finished_at Nullable(DateTime64(3, 'UTC')),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY job_id"
    ))
}

/// Incidents with owner, impact and current state.
pub fn incidents(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.incidents (\n\
         \x20   incident_id String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   severity LowCardinality(String),\n\
         \x20   state LowCardinality(String),\n\
         \x20   owner String,\n\
         \x20   impact String,\n\
         \x20   opened_at DateTime64(3, 'UTC'),\n\
         \x20   updated_at DateTime64(3, 'UTC'),\n\
         \x20   evidence_ref Nullable(String),\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY incident_id"
    ))
}

/// As-of version bookkeeping per projection: the spec's rule that a backfill
/// changes the as-of version.
///
/// Versions are staged: a rebuild writes its rows and version row with
/// `published = 0`, and readers resolve `as_of_version` as
/// `max(as_of_version)` over rows with `published = 1`. Version rows are
/// keyed by `(projection_name, as_of_version)` so several versions (published
/// and staged) coexist across merges; `rebuilt_at` orders replacements of
/// one version (the publish marker). See
/// [`projection_versions_add_published`] for the column migration.
pub fn projection_versions(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.projection_versions (\n\
         \x20   projection_name LowCardinality(String),\n\
         \x20   as_of_version UInt64,\n\
         \x20   batch_id String,\n\
         \x20   source_event_max Nullable(DateTime64(3, 'UTC')),\n\
         \x20   rebuilt_at DateTime64(3, 'UTC'),\n\
         \x20   published UInt8 DEFAULT 0\n\
         ) ENGINE = ReplacingMergeTree(rebuilt_at)\n\
         ORDER BY (projection_name, as_of_version)"
    ))
}

/// Add the staged-publish marker to a pre-existing `projection_versions`
/// table. The `DEFAULT 1` is deliberate: rows written by deployments before
/// staging existed are already reader-visible, so they stay published, while
/// every new version row carries an explicit `published` value (`0` from
/// `write_projection`, `1` from `publish`).
///
/// The column migration cannot change the table's ordering key: legacy
/// tables keep collapsing to one row per projection, so before the next
/// publish their readers may see the previously published version only
/// until a merge drops it (never a wrong version). Deployments that want
/// staged history across merges recreate `projection_versions`; it is
/// bookkeeping and is rebuildable from source facts.
pub fn projection_versions_add_published(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "ALTER TABLE {db}.projection_versions \
         ADD COLUMN IF NOT EXISTS published UInt8 DEFAULT 1"
    ))
}

/// Tenant daily revenue and reporting coverage aggregate (24 month TTL).
/// The as-of version is part of the sorting key so a staged rebuild coexists
/// with (and never hides) the published version across merges; `publish`
/// prunes superseded versions.
pub fn customer_daily(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.customer_daily (\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   day Date,\n\
         \x20   platform LowCardinality(String),\n\
         \x20   currency LowCardinality(String),\n\
         \x20   revenue_micros Int64,\n\
         \x20   impressions Nullable(UInt64),\n\
         \x20   source_days UInt32,\n\
         \x20   final_days UInt32,\n\
         \x20   provisional_days UInt32,\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (publisher_id, day, platform, currency, as_of_version)\n\
         TTL day + INTERVAL {AGGREGATE_RETENTION_MONTHS} MONTH"
    ))
}

/// Platform daily revenue aggregate (24 month TTL). The as-of version is
/// part of the sorting key (staged and published versions coexist);
/// `publish` prunes superseded versions.
pub fn platform_daily(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.platform_daily (\n\
         \x20   platform LowCardinality(String),\n\
         \x20   day Date,\n\
         \x20   currency LowCardinality(String),\n\
         \x20   revenue_micros Int64,\n\
         \x20   impressions Nullable(UInt64),\n\
         \x20   final_accounts UInt32,\n\
         \x20   provisional_accounts UInt32,\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (platform, day, currency, as_of_version)\n\
         TTL day + INTERVAL {AGGREGATE_RETENTION_MONTHS} MONTH"
    ))
}

/// Lifecycle cohort aggregate (24 month TTL, the spec's only aggregate
/// horizon; cohorts older than the horizon are rebuilt from source facts if
/// ever needed again). The as-of version is part of the sorting key (staged
/// and published versions coexist); `publish` prunes superseded versions.
pub fn lifecycle_cohorts(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.lifecycle_cohorts (\n\
         \x20   cohort_month Date,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   stage LowCardinality(String),\n\
         \x20   accounts UInt32,\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (cohort_month, publisher_id, stage, as_of_version)\n\
         TTL cohort_month + INTERVAL {AGGREGATE_RETENTION_MONTHS} MONTH"
    ))
}

/// Operator queue snapshot: incidents, due decisions, stale accounts,
/// overdue jobs and held fees. Rebuilt whole each time, so it carries no TTL.
/// The as-of version is part of the sorting key (staged and published
/// versions coexist); `publish` prunes superseded versions.
pub fn operator_queues(database: &str) -> Result<String, SchemaError> {
    let db = db(database)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {db}.operator_queues (\n\
         \x20   queue LowCardinality(String),\n\
         \x20   object_id String,\n\
         \x20   publisher_id LowCardinality(String),\n\
         \x20   severity LowCardinality(String),\n\
         \x20   age_seconds UInt64,\n\
         \x20   owner Nullable(String),\n\
         \x20   next_action String,\n\
         \x20   as_of_version UInt64\n\
         ) ENGINE = ReplacingMergeTree(as_of_version)\n\
         ORDER BY (queue, object_id, as_of_version)"
    ))
}

/// Every DDL statement in dependency order, database first. `ensure_schema`
/// runs these one by one; each is idempotent.
pub fn all_ddl(database: &str) -> Result<Vec<String>, SchemaError> {
    let mut statements = vec![create_database(database)?];
    statements.push(domain_events(database)?);
    statements.push(request_logs(database)?);
    statements.push(accounts(database)?);
    statements.push(connections(database)?);
    statements.push(source_snapshot_index(database)?);
    statements.push(report_days(database)?);
    statements.push(audit_runs(database)?);
    statements.push(findings(database)?);
    statements.push(proposals(database)?);
    statements.push(experiments(database)?);
    statements.push(decisions(database)?);
    statements.push(fee_lines(database)?);
    statements.push(invoices(database)?);
    statements.push(payment_events(database)?);
    statements.push(jobs(database)?);
    statements.push(incidents(database)?);
    statements.push(projection_versions(database)?);
    statements.push(projection_versions_add_published(database)?);
    statements.push(customer_daily(database)?);
    statements.push(platform_daily(database)?);
    statements.push(lifecycle_cohorts(database)?);
    statements.push(operator_queues(database)?);
    Ok(statements)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn identifiers_are_refused_unless_plain() {
        assert_eq!(
            validate_identifier("apex_analytics").expect("plain"),
            "apex_analytics"
        );
        assert_eq!(
            validate_identifier("_x1").expect("leading underscore"),
            "_x1"
        );
        for bad in [
            "",
            "1db",
            "a-b",
            "db; DROP TABLE x",
            "`db`",
            "db name",
            "héllo",
        ] {
            assert!(
                matches!(
                    validate_identifier(bad),
                    Err(SchemaError::InvalidIdentifier(_))
                ),
                "{bad} must be refused"
            );
        }
        let long = "a".repeat(129);
        assert!(validate_identifier(&long).is_err());
    }

    #[test]
    fn ddl_refuses_a_hostile_database_name() {
        assert!(domain_events("db; DROP TABLE x").is_err());
        assert!(all_ddl("bad name").is_err());
    }

    #[test]
    fn raw_events_carry_the_spec_retention_and_dedup_key() {
        let sql = domain_events("db").expect("ddl");
        assert!(sql.contains("TTL occurred_at_utc + INTERVAL 90 DAY"));
        assert!(sql.contains("ORDER BY (source, event_id)"));
        assert!(sql.contains("ENGINE = ReplacingMergeTree(version)"));
        assert!(sql.contains("PARTITION BY toYYYYMM(occurred_at_utc)"));
        // The envelope fields all exist as columns.
        for column in [
            "event_id",
            "event_type",
            "schema_version",
            "occurred_at_utc",
            "received_at_utc",
            "environment",
            "service_version",
            "source",
            "publisher_id",
            "platform",
            "object_type",
            "object_id",
            "correlation_id",
            "causation_id",
            "outcome",
            "reason_code",
            "payload_json",
        ] {
            assert!(sql.contains(column), "missing {column}");
        }
    }

    #[test]
    fn request_logs_carry_the_thirty_day_ttl() {
        let sql = request_logs("db").expect("ddl");
        assert!(sql.contains("TTL occurred_at_utc + INTERVAL 30 DAY"));
        assert!(sql.contains("query_keys Array(String)"));
        assert!(!sql.contains("remote_ip"));
        assert!(!sql.contains("user_agent"));
    }

    #[test]
    fn aggregates_carry_the_twenty_four_month_ttl() {
        for sql in [
            customer_daily("db").expect("customer_daily"),
            platform_daily("db").expect("platform_daily"),
            lifecycle_cohorts("db").expect("lifecycle_cohorts"),
        ] {
            assert!(sql.contains("INTERVAL 24 MONTH"), "got {sql}");
        }
        assert!(!operator_queues("db").expect("queues").contains("TTL"));
    }

    #[test]
    fn financial_tables_have_no_generic_analytics_ttl() {
        for sql in [
            fee_lines("db").expect("fee_lines"),
            invoices("db").expect("invoices"),
            payment_events("db").expect("payment_events"),
            report_days("db").expect("report_days"),
            decisions("db").expect("decisions"),
            incidents("db").expect("incidents"),
        ] {
            assert!(
                !sql.contains("TTL"),
                "must follow contract retention: {sql}"
            );
        }
    }

    #[test]
    fn money_columns_are_integer_micros_with_a_currency() {
        let sql = report_days("db").expect("ddl");
        assert!(sql.contains("revenue_micros Nullable(Int64)"));
        assert!(sql.contains("currency LowCardinality(Nullable(String))"));
        let fees = fee_lines("db").expect("ddl");
        assert!(fees.contains("fee_micros Int64"));
        assert!(fees.contains("currency LowCardinality(String)"));
        assert!(fees.contains("proof_hashes Array(String)"));
    }

    #[test]
    fn projection_tables_keep_versions_apart() {
        // The as-of version is part of every projection's sorting key, so a
        // staged rebuild never collapses the published rows away at merge
        // time; `publish` prunes superseded versions instead.
        for sql in [
            customer_daily("db").expect("customer"),
            platform_daily("db").expect("platform"),
            lifecycle_cohorts("db").expect("cohorts"),
            operator_queues("db").expect("queues"),
        ] {
            let order = sql
                .split("ORDER BY")
                .nth(1)
                .expect("every projection orders by its keys");
            assert!(
                order.contains("as_of_version"),
                "version in the sorting key: {sql}"
            );
        }
    }

    #[test]
    fn all_ddl_names_every_table_of_the_minimal_model() {
        let statements = all_ddl("apex_analytics").expect("all");
        assert_eq!(statements.len(), 23);
        assert!(statements[0].contains("CREATE DATABASE IF NOT EXISTS apex_analytics"));
        for table in [
            "accounts",
            "connections",
            "source_snapshot_index",
            "report_days",
            "audit_runs",
            "findings",
            "proposals",
            "experiments",
            "decisions",
            "fee_lines",
            "invoices",
            "payment_events",
            "jobs",
            "incidents",
            "customer_daily",
            "platform_daily",
            "lifecycle_cohorts",
            "operator_queues",
            "projection_versions",
        ] {
            assert!(
                statements
                    .iter()
                    .any(|sql| sql.contains(&format!("{table} ("))),
                "missing table {table}"
            );
        }
        for sql in &statements {
            assert!(sql.contains("IF NOT EXISTS"), "idempotent: {sql}");
        }
    }

    #[test]
    fn projection_versions_stage_until_published() {
        let create = projection_versions("db").expect("create");
        assert!(create.contains("published UInt8 DEFAULT 0"), "{create}");
        // Version rows coexist (one key per version): a merge must never
        // collapse the published version away behind a staged one.
        assert!(
            create.contains("ORDER BY (projection_name, as_of_version)"),
            "{create}"
        );
        let migrate = projection_versions_add_published("db").expect("migrate");
        assert!(
            migrate.contains(
                "ALTER TABLE db.projection_versions ADD COLUMN IF NOT EXISTS published UInt8 DEFAULT 1"
            ),
            "{migrate}"
        );
        assert!(projection_versions_add_published("bad name").is_err());
        // The migration runs after the table exists so a fresh table is a
        // no-op and a legacy table gains the column without losing rows.
        let statements = all_ddl("db").expect("all");
        let create_index = statements
            .iter()
            .position(|sql| sql.contains("projection_versions ("))
            .expect("create projection_versions");
        let migrate_index = statements
            .iter()
            .position(|sql| sql.contains("ADD COLUMN IF NOT EXISTS published"))
            .expect("migration");
        assert!(create_index < migrate_index);
    }
}
