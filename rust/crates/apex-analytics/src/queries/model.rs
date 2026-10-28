//! Pure read-model computation over source facts.
//!
//! Both readers (memory and ClickHouse) fetch their facts through the same
//! tenant scope and then run these functions, so the two backends produce
//! identical DTOs for identical facts; the integration test asserts that
//! equality on the real server.
//!
//! Rules encoded here come from section 05: rates always carry n/N and a
//! denominator (page 84, Mandatory context); missing data is `None` with a
//! reason, never zero (page 87, Zero, missing and provisional data);
//! cohort maturity gates conversion rates (page 87, Cohort maturity); and
//! attributed money uses the signature-free integer formula
//! `R x (1 - 1 / F)` with F = 1 + bound (page 77).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::envelope::{DomainEvent, EventType};
use crate::projections::OperatorQueueRow;
use crate::store::ReportDayRow;
use crate::timestamp::{UtcMillis, MILLIS_PER_DAY};

use super::dto::{
    ActivationFunnel, BreakdownDimension, DataQuality, ExperimentEffectiveness,
    ExperimentLandscapeRow, FeeDetail, FeeRow, FeeState, FreshnessStatus, FunnelStageRow,
    GrowthCohortRow, IngestionHealthRow, LiftRow, Money, ObjectTimelineEntry, OperatorQueues,
    PageRequestsDay, QueryScope, RevenueBreakdownRow, RevenueDailyRow, StaleSource,
};

/// A source is stale when its latest finalized day is more than this many
/// days before the as-of day. Expected schedules are platform-specific in
/// the full deployment; this crate applies one documented default and always
/// reports the exact last final day so the verdict can be recomputed.
pub const FRESHNESS_STALE_DAYS: i64 = 2;

/// The follow-up horizon a cohort must clear before its rates are compared
/// (spec page 79: "7/30-day follow-up horizon"; 30 days is the default).
pub const COHORT_MATURITY_DAYS: i64 = 30;

/// The lifecycle stages of the growth funnel, in order. A reached stage
/// counts for every later stage too (an account that is paid has also been
/// connected and delivered).
pub const FUNNEL_STAGES: [&str; 4] = ["qualified", "connected", "audit_delivered", "paid"];

/// Is a report row inside the scope?
pub(crate) fn day_in_scope(scope: &QueryScope, day: &str) -> bool {
    if let Some(from) = &scope.from_day {
        if day < from.as_str() {
            return false;
        }
    }
    if let Some(to) = &scope.to_day {
        if day > to.as_str() {
            return false;
        }
    }
    true
}

/// Is an event inside the tenant scope? `publisher_id` and `platform` are
/// optional on events, so a scoped read only sees events that carry the
/// tenant identity.
pub(crate) fn event_in_scope(scope: &QueryScope, event: &DomainEvent) -> bool {
    if !scope.publisher_id.is_empty()
        && event.publisher_id.as_deref() != Some(scope.publisher_id.as_str())
    {
        return false;
    }
    if let Some(platform) = &scope.platform {
        if event.platform.as_deref() != Some(platform.as_str()) {
            return false;
        }
    }
    if let Some(currency) = &scope.currency {
        if event.payload.currency.as_deref() != Some(currency.as_str()) {
            return false;
        }
    }
    // Most reads are unbounded; formatting a day string per event only to
    // find both bounds absent was the dominant allocation in scoped filtering.
    if scope.from_day.is_none() && scope.to_day.is_none() {
        return true;
    }
    let day = event.occurred_at_utc.to_day_string();
    day_in_scope(scope, &day)
}

/// Is a report day inside the scope?
pub(crate) fn report_day_in_scope(scope: &QueryScope, row: &ReportDayRow) -> bool {
    if !scope.publisher_id.is_empty() && row.publisher_id != scope.publisher_id {
        return false;
    }
    if let Some(platform) = &scope.platform {
        if row.platform != *platform {
            return false;
        }
    }
    if let Some(currency) = &scope.currency {
        if row.currency.as_deref() != Some(currency.as_str()) {
            return false;
        }
    }
    day_in_scope(scope, &row.day)
}

/// Rate in basis points of `part / total`; `None` when the denominator is
/// zero or the result does not fit (never a fabricated rate).
pub fn rate_basis_points(part: u64, total: u64) -> Option<u32> {
    if total == 0 {
        return None;
    }
    u32::try_from((u128::from(part) * 10_000 / u128::from(total)).min(u128::from(u32::MAX))).ok()
}

/// Attributed increment in integer micros: `R x bound / (1 + bound)` with
/// the bound in basis points. For the acceptance fixture,
/// 51,500,000,000 micros at 300 bp yields exactly 1,500,000,000 micros.
/// `None` for an unusable bound (-100 percent or worse).
pub fn attributed_increment_micros(revenue_micros: i64, bound_bp: i64) -> Option<i64> {
    let denominator = i128::from(10_000i64.checked_add(bound_bp)?);
    if denominator <= 0 {
        return None;
    }
    let increment = i128::from(revenue_micros).checked_mul(i128::from(bound_bp))? / denominator;
    i64::try_from(increment).ok()
}

/// Lift fee before tax in integer micros, following the contract rate bounds
/// of the core (15 to 20 percent). `None` when the rate is outside the
/// agreed contract: the fee then has no reproducible basis and must not be
/// invented.
pub fn fee_before_tax_micros(increment_micros: i64, rate_bp: u16) -> Option<i64> {
    apex_core::money::lift_fee_micros(increment_micros, rate_bp).ok()
}

/// Daily revenue rows for the scope.
///
/// Rows are accumulated per publisher as well as per day, platform and
/// currency: the ClickHouse reader reads the `customer_daily` projection,
/// which carries one row per publisher, so an operator-wide scope must never
/// merge two publishers into one number even though the DTO itself does not
/// carry the publisher identity. The returned order is `(day, platform,
/// currency, publisher)`, the same total order the ClickHouse statement
/// applies, so both readers emit an identical `Vec`.
pub fn revenue_daily(scope: &QueryScope, report_days: &[ReportDayRow]) -> Vec<RevenueDailyRow> {
    struct Accumulator {
        revenue_micros: i64,
        impressions: Option<u64>,
        source_days: u32,
        final_days: u32,
        provisional_days: u32,
    }
    let mut map: BTreeMap<(String, String, String, String), Accumulator> = BTreeMap::new();
    for row in report_days
        .iter()
        .filter(|row| report_day_in_scope(scope, row))
    {
        let (Some(revenue_micros), Some(currency)) = (row.revenue_micros, row.currency.as_deref())
        else {
            continue;
        };
        let key = (
            row.publisher_id.clone(),
            row.day.clone(),
            row.platform.clone(),
            currency.to_string(),
        );
        let accumulator = map.entry(key).or_insert_with(|| Accumulator {
            revenue_micros: 0,
            impressions: Some(0),
            source_days: 0,
            final_days: 0,
            provisional_days: 0,
        });
        accumulator.revenue_micros = accumulator.revenue_micros.saturating_add(revenue_micros);
        accumulator.impressions = match (accumulator.impressions, row.impressions) {
            (Some(sum), Some(value)) => sum.checked_add(value),
            _ => None,
        };
        accumulator.source_days += 1;
        match row.finality.as_str() {
            "final" => accumulator.final_days += 1,
            "provisional" => accumulator.provisional_days += 1,
            _ => {}
        }
    }
    let mut rows: Vec<(String, RevenueDailyRow)> = map
        .into_iter()
        .map(|((publisher_id, day, platform, currency), accumulator)| {
            (
                publisher_id,
                RevenueDailyRow {
                    day,
                    platform,
                    revenue: Money::new(accumulator.revenue_micros, currency),
                    impressions: accumulator.impressions,
                    source_days: accumulator.source_days,
                    final_days: accumulator.final_days,
                    provisional_days: accumulator.provisional_days,
                },
            )
        })
        .collect();
    rows.sort_by(|(left_publisher, left), (right_publisher, right)| {
        left.day
            .cmp(&right.day)
            .then(left.platform.cmp(&right.platform))
            .then(left.revenue.currency.cmp(&right.revenue.currency))
            .then(left_publisher.cmp(right_publisher))
    });
    rows.into_iter().map(|(_, row)| row).collect()
}

/// Revenue broken down by one dimension. Rows missing the dimension value
/// cannot be attributed and are excluded here; the data-quality badge counts
/// them as unmapped.
///
/// Both readers return the identical order: revenue descending, then the
/// dimension value ascending, then the currency. The ClickHouse statement
/// applies the same `ORDER BY revenue_micros DESC, dimension_value, currency`
/// so the two Vecs compare equal without re-sorting.
pub fn revenue_breakdown(
    scope: &QueryScope,
    report_days: &[ReportDayRow],
    dimension: BreakdownDimension,
) -> Vec<RevenueBreakdownRow> {
    let dimension_value = |row: &ReportDayRow| -> Option<String> {
        match dimension {
            BreakdownDimension::Platform => Some(row.platform.clone()),
            BreakdownDimension::Format => row.format.clone(),
            BreakdownDimension::Country => row.country.clone(),
            BreakdownDimension::AdUnit => row.ad_unit_id.clone(),
        }
    };
    struct Accumulator {
        revenue_micros: i64,
        impressions: Option<u64>,
        rows: u32,
        currency: String,
    }
    let mut map: BTreeMap<(String, String), Accumulator> = BTreeMap::new();
    for row in report_days
        .iter()
        .filter(|row| report_day_in_scope(scope, row))
    {
        let (Some(revenue_micros), Some(currency)) = (row.revenue_micros, row.currency.as_deref())
        else {
            continue;
        };
        let Some(value) = dimension_value(row) else {
            continue;
        };
        let accumulator = map
            .entry((value, currency.to_string()))
            .or_insert_with(|| Accumulator {
                revenue_micros: 0,
                impressions: Some(0),
                rows: 0,
                currency: currency.to_string(),
            });
        accumulator.revenue_micros = accumulator.revenue_micros.saturating_add(revenue_micros);
        accumulator.impressions = match (accumulator.impressions, row.impressions) {
            (Some(sum), Some(value)) => sum.checked_add(value),
            _ => None,
        };
        accumulator.rows += 1;
    }
    let totals_by_currency: BTreeMap<String, i64> = {
        let mut totals: BTreeMap<String, i64> = BTreeMap::new();
        for ((_, currency), accumulator) in &map {
            *totals.entry(currency.clone()).or_insert(0) += accumulator.revenue_micros;
        }
        totals
    };
    let mut rows: Vec<RevenueBreakdownRow> = map
        .into_iter()
        .map(|((dimension_value, _), accumulator)| {
            let total = totals_by_currency
                .get(&accumulator.currency)
                .copied()
                .unwrap_or(0);
            let share_bp = if accumulator.revenue_micros >= 0 {
                rate_basis_points(accumulator.revenue_micros as u64, total.max(0) as u64)
            } else {
                None
            };
            RevenueBreakdownRow {
                dimension_value,
                revenue: Money::new(accumulator.revenue_micros, accumulator.currency),
                impressions: accumulator.impressions,
                rows: accumulator.rows,
                share_bp,
            }
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .revenue
            .micros
            .cmp(&left.revenue.micros)
            .then(left.dimension_value.cmp(&right.dimension_value))
            .then(left.revenue.currency.cmp(&right.revenue.currency))
    });
    rows
}

/// Lift rows from registered experiments and decisions.
pub fn lift_rows(scope: &QueryScope, events: &[DomainEvent]) -> Vec<LiftRow> {
    let scoped: Vec<&DomainEvent> = events
        .iter()
        .filter(|event| event_in_scope(scope, event))
        .collect();
    let mut registrations: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
    for event in &scoped {
        if event.event_type != EventType::ExperimentRegistered {
            continue;
        }
        if let Some(experiment_id) = event.payload.experiment_id.clone() {
            registrations.insert(
                experiment_id,
                (
                    event.payload.registered_metric.clone(),
                    event.payload.source_digest.clone(),
                ),
            );
        }
    }
    let mut rows: Vec<LiftRow> = scoped
        .iter()
        .filter(|event| event.event_type == EventType::ExperimentDecided)
        .map(|event| {
            let experiment_id = event.payload.experiment_id.clone().unwrap_or_default();
            let decision = event
                .payload
                .decision
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let (measurement_source, registration_digest) = registrations
                .get(&experiment_id)
                .cloned()
                .unwrap_or((None, None));
            LiftRow {
                experiment_id,
                change_ref: event
                    .payload
                    .change_id
                    .clone()
                    .or_else(|| event.object_id.clone()),
                observed_lift_bp: event.payload.point_estimate_bp,
                billable_bound_bp: if decision == "keep" {
                    event.payload.decision_bound_bp
                } else {
                    None
                },
                outcome: decision,
                measurement_source,
                measurement_version: event.payload.source_digest.clone().or(registration_digest),
                decided_at: Some(event.occurred_at_utc.to_rfc3339_millis()),
            }
        })
        .collect();
    rows.sort_by(|left, right| {
        left.decided_at
            .cmp(&right.decided_at)
            .then(left.experiment_id.cmp(&right.experiment_id))
    });
    rows
}

/// One fee accumulator while folding events.
#[derive(Default)]
struct FeeAccumulator {
    currency: Option<String>,
    period: String,
    source_amount_micros: Option<i64>,
    bound_bp: Option<i64>,
    rate_bp: Option<u16>,
    active_days: Option<u32>,
    proof_refs: Vec<String>,
    invoice_ref: Option<String>,
    invoice_amount_micros: Option<i64>,
    held: bool,
    invoiced: bool,
    credited: bool,
    latest_key: Option<(UtcMillis, &'static str, String)>,
}

/// Fee rows with states, invoice references and proof hashes.
///
/// The fold is a latest-fact-wins reduction with a deterministic tie rule:
/// only a candidate strictly newer by `(occurred_at, source, event_id)` may
/// overwrite the accumulated fields, so an older replayed fact can never
/// win regardless of insertion order. An exact tie keeps the row folded
/// first; identical keys cannot carry different content because
/// `(source, event_id)` is the dedup key. Both readers feed events in the
/// same chronological order (`occurred_at_utc, source, event_id`).
pub fn fee_rows(
    scope: &QueryScope,
    events: &[DomainEvent],
    report_days: &[ReportDayRow],
) -> Vec<FeeRow> {
    let scoped: Vec<&DomainEvent> = events
        .iter()
        .filter(|event| event_in_scope(scope, event))
        .collect();
    let mut accumulators: BTreeMap<(String, String), FeeAccumulator> = BTreeMap::new();
    let mut paid_invoices: BTreeSet<String> = BTreeSet::new();
    // The periods with a provisional report day for the scoped publisher,
    // folded once: the per-change pending check below must not rescan every
    // report day for every fee row (quadratic over the window).
    let provisional_periods: HashSet<&str> = report_days
        .iter()
        .filter(|row| {
            !scope.publisher_id.is_empty()
                && row.publisher_id == scope.publisher_id
                && row.day.len() >= 7
                && row.finality == "provisional"
        })
        .map(|row| &row.day[..7])
        .collect();
    for event in &scoped {
        // The fee fold reads only these types (the same list the
        // ClickHouse reader binds): a non-fee event that happens to carry a
        // change/period pair must not advance the latest-key tie rule.
        if !matches!(
            event.event_type,
            EventType::FeeComputed
                | EventType::FeeHeld
                | EventType::FeeInvoiceIssued
                | EventType::FeeCredited
                | EventType::PaymentReconciled
        ) {
            continue;
        }
        if event.event_type == EventType::PaymentReconciled {
            if let Some(invoice_id) = event.payload.invoice_id.clone() {
                paid_invoices.insert(invoice_id);
            }
        }
        let (Some(change_id), Some(period)) = (
            event.payload.change_id.clone(),
            event.payload.period.clone(),
        ) else {
            continue;
        };
        let accumulator = accumulators
            .entry((change_id, period.clone()))
            .or_insert_with(|| FeeAccumulator {
                period,
                ..FeeAccumulator::default()
            });
        let candidate = (
            event.occurred_at_utc,
            event.source.as_str(),
            event.event_id.clone(),
        );
        let strictly_newer = match &accumulator.latest_key {
            Some(current) => candidate > *current,
            None => true,
        };
        if !strictly_newer {
            continue;
        }
        accumulator.latest_key = Some(candidate);
        match event.event_type {
            EventType::FeeComputed => {
                if let Some(currency) = event.payload.currency.clone() {
                    accumulator.currency = Some(currency);
                }
                accumulator.source_amount_micros = event.payload.source_amount_micros;
                accumulator.bound_bp = event.payload.bound_bp;
                accumulator.rate_bp = event.payload.rate_bp;
                accumulator.active_days = event.payload.active_days;
                if let Some(proofs) = &event.payload.proof_hashes {
                    accumulator.proof_refs = proofs.clone();
                }
            }
            EventType::FeeHeld => {
                accumulator.held = true;
                if accumulator.currency.is_none() {
                    accumulator.currency = event.payload.currency.clone();
                }
            }
            EventType::FeeInvoiceIssued | EventType::FeeCredited => {
                if event.event_type == EventType::FeeCredited {
                    accumulator.credited = true;
                } else {
                    accumulator.invoiced = true;
                }
                if let Some(invoice_id) = event.payload.invoice_id.clone() {
                    accumulator.invoice_ref = Some(invoice_id);
                }
                if event.payload.amount_micros.is_some() {
                    accumulator.invoice_amount_micros = event.payload.amount_micros;
                }
                if accumulator.currency.is_none() {
                    accumulator.currency = event.payload.currency.clone();
                }
            }
            _ => {}
        }
    }

    let mut rows: Vec<FeeRow> = accumulators
        .into_iter()
        .map(|((change_id, period), accumulator)| {
            let currency = accumulator.currency.clone().unwrap_or_default();
            let derived_amount = match (
                accumulator.source_amount_micros,
                accumulator.bound_bp,
                accumulator.rate_bp,
            ) {
                (Some(source), Some(bound), Some(rate)) => {
                    attributed_increment_micros(source, bound)
                        .and_then(|increment| fee_before_tax_micros(increment, rate))
                }
                _ => None,
            };
            let amount_micros = accumulator.invoice_amount_micros.or(derived_amount);
            let paid = accumulator
                .invoice_ref
                .as_ref()
                .is_some_and(|invoice| paid_invoices.contains(invoice));
            let pending_period = !scope.publisher_id.is_empty()
                && provisional_periods.contains(accumulator.period.as_str());
            let state = if accumulator.credited {
                FeeState::Credited
            } else if paid {
                FeeState::Paid
            } else if accumulator.invoiced {
                FeeState::Invoiced
            } else if accumulator.held {
                FeeState::Held
            } else if pending_period {
                FeeState::Pending
            } else {
                FeeState::Estimated
            };
            FeeRow {
                change_id,
                period,
                state,
                amount: amount_micros.map(|micros| Money::new(micros, currency.clone())),
                source_amount: accumulator
                    .source_amount_micros
                    .map(|micros| Money::new(micros, currency.clone())),
                bound_bp: accumulator.bound_bp,
                rate_bp: accumulator.rate_bp,
                active_days: accumulator.active_days,
                invoice_ref: accumulator.invoice_ref,
                proof_refs: accumulator.proof_refs,
            }
        })
        .collect();
    rows.sort_by(|left, right| {
        left.period
            .cmp(&right.period)
            .then(left.change_id.cmp(&right.change_id))
    });
    rows
}

/// Reproduce the fee calculation of one change from its latest computed
/// event (the "values are arithmetically consistent" table, page 48).
pub fn fee_detail(
    scope: &QueryScope,
    events: &[DomainEvent],
    change_id: &str,
) -> Option<FeeDetail> {
    let latest = events
        .iter()
        .filter(|event| event_in_scope(scope, event))
        .filter(|event| event.event_type == EventType::FeeComputed)
        .filter(|event| event.payload.change_id.as_deref() == Some(change_id))
        .max_by_key(|event| event.occurred_at_utc)?;
    let currency = latest.payload.currency.clone()?;
    let source_micros = latest.payload.source_amount_micros?;
    let bound_bp = latest.payload.bound_bp?;
    let rate_bp = latest.payload.rate_bp?;
    let mut unavailable = Vec::new();
    let increment = match attributed_increment_micros(source_micros, bound_bp) {
        Some(increment) => Some(increment),
        None => {
            unavailable.push("attributed_increment: bound is not usable".to_string());
            None
        }
    };
    let fee = match increment {
        Some(increment) => match fee_before_tax_micros(increment, rate_bp) {
            Some(fee) => Some(fee),
            None => {
                unavailable.push("fee_before_tax: rate outside the agreed contract".to_string());
                None
            }
        },
        None => {
            unavailable.push("fee_before_tax: no reproducible increment".to_string());
            None
        }
    };
    Some(FeeDetail {
        change_id: change_id.to_string(),
        period: latest.payload.period.clone().unwrap_or_default(),
        eligible_revenue: Money::new(source_micros, currency.clone()),
        conservative_bound_bp: bound_bp,
        agreed_rate_bp: rate_bp,
        attributed_increment: increment.map(|micros| Money::new(micros, currency.clone())),
        fee_before_tax: fee.map(|micros| Money::new(micros, currency.clone())),
        active_days: latest.payload.active_days,
        proof_refs: latest.payload.proof_hashes.clone().unwrap_or_default(),
        calculated_at: Some(latest.occurred_at_utc.to_rfc3339_millis()),
        unavailable,
    })
}

/// The data-quality badge for the scope.
pub fn data_quality(
    scope: &QueryScope,
    report_days: &[ReportDayRow],
    dedup_rejects: Option<u64>,
) -> DataQuality {
    // The aggregates fold over the scoped slice with borrowed keys: the
    // original passes cloned platform, account and day per row into owned
    // maps and sets, which dominated the read once a 90-day window covers
    // every account. Every aggregate below is a count or an explicitly
    // ordered output, so borrowing changes no result; the one scan builds
    // the compact scoped view. (Measured alternative: a single interleaved
    // pass over borrowed hash keys was slower — one tree/set per aggregate
    // stays cache-friendly and costs no hashing.)
    let scoped: Vec<&ReportDayRow> = report_days
        .iter()
        .filter(|row| report_day_in_scope(scope, row))
        .collect();
    let matched_source_days = scoped
        .iter()
        .map(|row| {
            (
                row.platform.as_str(),
                row.platform_account_id.as_str(),
                row.day.as_str(),
            )
        })
        .collect::<BTreeSet<_>>()
        .len() as u64;
    let provisional_days = scoped
        .iter()
        .filter(|row| row.finality == "provisional")
        .count() as u64;
    let unmapped_rows = scoped
        .iter()
        .filter(|row| row.revenue_micros.is_none() || row.currency.is_none())
        .count() as u64;

    let mut unavailable = Vec::new();
    if dedup_rejects.is_none() {
        unavailable.push("dedup_rejects: this store cannot report raw duplicate rows".to_string());
    }

    // Gaps: per account, requested days minus covered days. Without a
    // requested range there is nothing to be missing from.
    let (missing_days, missing_reason) = match (scope.from_day.as_deref(), scope.to_day.as_deref())
    {
        (Some(from), Some(to)) => {
            let parse = |text: &str| UtcMillis::parse_rfc3339(&format!("{text}T00:00:00Z")).ok();
            match (parse(from), parse(to)) {
                (Some(from), Some(to)) => {
                    let range_days = (to.day_index() - from.day_index() + 1).max(0) as u64;
                    let mut per_account: BTreeMap<(&str, &str), BTreeSet<&str>> = BTreeMap::new();
                    for row in &scoped {
                        per_account
                            .entry((row.platform.as_str(), row.platform_account_id.as_str()))
                            .or_default()
                            .insert(row.day.as_str());
                    }
                    if per_account.is_empty() {
                        // Zero rows for a requested range is missing
                        // data, never "every day present": missing is
                        // None with a reason.
                        (
                                None,
                                Some(
                                    "missing_days: the store holds no report rows for the requested range",
                                ),
                            )
                    } else {
                        (
                            Some(
                                per_account
                                    .values()
                                    .map(|days| range_days.saturating_sub(days.len() as u64))
                                    .sum(),
                            ),
                            None,
                        )
                    }
                }
                _ => (
                    None,
                    Some("missing_days: the requested day range is not parseable"),
                ),
            }
        }
        _ => (None, Some("missing_days: no day range was requested")),
    };
    if let Some(reason) = missing_reason {
        unavailable.push(reason.to_string());
    }

    // Stale sources, in the deterministic (platform, account) order.
    let mut sources: BTreeMap<(&str, &str), Option<&str>> = BTreeMap::new();
    for row in &scoped {
        let entry = sources
            .entry((row.platform.as_str(), row.platform_account_id.as_str()))
            .or_insert(None);
        if row.finality == "final" {
            match entry {
                Some(current) if *current >= row.day.as_str() => {}
                _ => *entry = Some(row.day.as_str()),
            }
        }
    }
    let as_of_day = scope.as_of.day_index();
    let mut stale_sources: Vec<StaleSource> = Vec::new();
    for ((platform, platform_account_id), last_final_day) in sources {
        let freshness_days = last_final_day.and_then(|day| {
            UtcMillis::parse_rfc3339(&format!("{day}T00:00:00Z"))
                .ok()
                .map(|at| as_of_day - at.day_index())
        });
        let stale = freshness_days
            .map(|days| days > FRESHNESS_STALE_DAYS)
            .unwrap_or(true);
        if stale {
            stale_sources.push(StaleSource {
                platform: platform.to_string(),
                platform_account_id: platform_account_id.to_string(),
                last_final_day: last_final_day.map(str::to_string),
                freshness_days,
            });
        }
    }

    DataQuality {
        publisher_id: scope.publisher_id.clone(),
        as_of: scope.as_of.to_rfc3339_millis(),
        matched_source_days,
        missing_days,
        provisional_days,
        dedup_rejects,
        unmapped_rows,
        stale_sources,
        unavailable,
    }
}

/// Object timeline: every scoped event about one object, merged by
/// occurrence time with the received time kept beside it.
pub fn object_timeline(
    scope: &QueryScope,
    events: &[DomainEvent],
    object_type: &str,
    object_id: &str,
) -> Vec<ObjectTimelineEntry> {
    let mut rows: Vec<ObjectTimelineEntry> = events
        .iter()
        .filter(|event| event_in_scope(scope, event))
        .filter(|event| event.object_type.as_deref() == Some(object_type))
        .filter(|event| event.object_id.as_deref() == Some(object_id))
        .map(|event| ObjectTimelineEntry {
            occurred_at_utc: event.occurred_at_utc.to_rfc3339_millis(),
            received_at_utc: event.received_at_utc.to_rfc3339_millis(),
            event_type: event.event_type,
            source: event.source,
            outcome: event.outcome.clone(),
            reason_code: event.reason_code.clone(),
            correlation_id: event.correlation_id.clone(),
            causation_id: event.causation_id.clone(),
        })
        .collect();
    rows.sort_by(|left, right| {
        left.occurred_at_utc
            .cmp(&right.occurred_at_utc)
            .then(left.received_at_utc.cmp(&right.received_at_utc))
    });
    rows
}

/// Freshness per publisher/platform/account: as-of day minus the latest
/// finalized source day.
pub fn ingestion_health(
    scope: &QueryScope,
    report_days: &[ReportDayRow],
) -> Vec<IngestionHealthRow> {
    let mut sources: BTreeMap<(String, String, String), Option<String>> = BTreeMap::new();
    for row in report_days
        .iter()
        .filter(|row| report_day_in_scope(scope, row))
    {
        let key = (
            row.publisher_id.clone(),
            row.platform.clone(),
            row.platform_account_id.clone(),
        );
        let entry = sources.entry(key).or_insert(None);
        if row.finality == "final" {
            match entry {
                Some(current) if current.as_str() >= row.day.as_str() => {}
                _ => *entry = Some(row.day.clone()),
            }
        }
    }
    let as_of_day = scope.as_of.day_index();
    sources
        .into_iter()
        .map(
            |((publisher_id, platform, platform_account_id), last_final_day)| {
                let freshness_days = last_final_day.as_deref().and_then(|day| {
                    UtcMillis::parse_rfc3339(&format!("{day}T00:00:00Z"))
                        .ok()
                        .map(|at| as_of_day - at.day_index())
                });
                let status = match freshness_days {
                    Some(days) if days > FRESHNESS_STALE_DAYS => FreshnessStatus::Stale,
                    Some(_) => FreshnessStatus::Fresh,
                    None => FreshnessStatus::MissingData,
                };
                IngestionHealthRow {
                    publisher_id,
                    platform,
                    platform_account_id,
                    last_final_day,
                    freshness_days,
                    status,
                }
            },
        )
        .collect()
}

/// Group computed queue entries into the five [`OperatorQueues`] lists.
///
/// The snapshot is as of the covered data: its membership and ages come
/// from the covered-data horizon the caller computed against, never from
/// the wall clock.
pub(crate) fn group_operator_queues(
    as_of: UtcMillis,
    rows: Vec<OperatorQueueRow>,
) -> OperatorQueues {
    let mut queues = OperatorQueues {
        as_of: as_of.to_rfc3339_millis(),
        incidents: Vec::new(),
        due_decisions: Vec::new(),
        stale_accounts: Vec::new(),
        overdue_jobs: Vec::new(),
        held_fees: Vec::new(),
    };
    for row in rows {
        match row.queue.as_str() {
            "incidents" => queues.incidents.push(row),
            "due_decisions" => queues.due_decisions.push(row),
            "stale_accounts" => queues.stale_accounts.push(row),
            "overdue_jobs" => queues.overdue_jobs.push(row),
            "held_fees" => queues.held_fees.push(row),
            _ => {}
        }
    }
    queues
}

/// The covered-data anchor of a queue read: the newest occurrence among the
/// events the queues are computed from, falling back to `fallback` when the
/// store holds no events at all. Both readers use their store's version of
/// this anchor: the memory store takes the max `occurred_at_utc` over its
/// stored events, the ClickHouse reader takes the published snapshot's
/// recorded `source_event_max`, which the rebuild wrote from the same
/// maximum.
pub(crate) fn covered_data_anchor(events: &[DomainEvent], fallback: UtcMillis) -> UtcMillis {
    events
        .iter()
        .map(|event| event.occurred_at_utc)
        .max()
        .unwrap_or(fallback)
}

/// Does `cohort_month` (YYYY-MM) meet the maturity horizon at `as_of`?
pub fn cohort_is_mature(cohort_month: &str, as_of: UtcMillis) -> bool {
    let start = UtcMillis::parse_rfc3339(&format!("{cohort_month}-01T00:00:00Z"));
    match start {
        Ok(start) => {
            as_of.unix_millis() - start.unix_millis() >= COHORT_MATURITY_DAYS * MILLIS_PER_DAY
        }
        Err(_) => false,
    }
}

/// Account identity of a lifecycle event: the object identity when the
/// event is about an account, else the payload's resolved account.
fn account_of(event: &DomainEvent) -> Option<String> {
    match event.event_type {
        EventType::AccountStageChanged => event
            .object_id
            .clone()
            .or_else(|| event.payload.account_id.clone()),
        EventType::AuditRequestAccepted => event.payload.account_id.clone(),
        _ => None,
    }
}

/// Per-account acceptance and stage history across the whole scoped window.
///
/// A cohort is fixed by the month an account was **first** accepted; later
/// stage movements belong to that cohort even when they happen in a
/// following month. Restricting the stage fold to the cohort month (the
/// previous shape) made every conversion after the calendar month boundary
/// invisible, so a cohort accepted at the end of a month could never show
/// the stages its 30-day follow-up horizon is meant to observe.
struct CohortFold {
    /// Distinct accepted lead identities per cohort month.
    leads_by_month: BTreeMap<i64, BTreeSet<String>>,
    /// Accounts whose first acceptance was in this month.
    accounts_by_month: BTreeMap<i64, BTreeSet<String>>,
    /// Stages reached after (or at) the account's first acceptance.
    stages: BTreeMap<String, BTreeSet<String>>,
}

fn cohort_fold<'a>(events: impl IntoIterator<Item = &'a DomainEvent>) -> CohortFold {
    let events: Vec<&DomainEvent> = events.into_iter().collect();
    // The first acceptance fixes the cohort. A replayed acceptance of the
    // same account is a later fact but never moves the account to a second
    // cohort.
    let mut accepted_at: BTreeMap<String, (i64, UtcMillis)> = BTreeMap::new();
    for event in &events {
        if event.event_type != EventType::AuditRequestAccepted {
            continue;
        }
        if let Some(account) = account_of(event) {
            let month = event.occurred_at_utc.month_index();
            let entry = accepted_at
                .entry(account)
                .or_insert((month, event.occurred_at_utc));
            if event.occurred_at_utc < entry.1 {
                *entry = (month, event.occurred_at_utc);
            }
        }
    }
    let mut leads_by_month: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    let mut accounts_by_month: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    for event in &events {
        if event.event_type != EventType::AuditRequestAccepted {
            continue;
        }
        let month = event.occurred_at_utc.month_index();
        if let Some(lead_id) = event.payload.lead_id.clone() {
            leads_by_month.entry(month).or_default().insert(lead_id);
        }
        if let Some(account) = account_of(event) {
            if accepted_at
                .get(&account)
                .is_some_and(|(cohort, _)| *cohort == month)
            {
                accounts_by_month.entry(month).or_default().insert(account);
            }
        }
    }
    let mut stages: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for event in &events {
        if event.event_type != EventType::AccountStageChanged {
            continue;
        }
        let Some(account) = account_of(event) else {
            continue;
        };
        let Some((_, accepted)) = accepted_at.get(&account) else {
            continue;
        };
        // A stage movement before the account existed in the request table
        // is not this cohort's progression.
        if event.occurred_at_utc < *accepted {
            continue;
        }
        if let Some(stage) = event.payload.next_stage.clone() {
            stages.entry(account).or_default().insert(stage);
        }
    }
    CohortFold {
        leads_by_month,
        accounts_by_month,
        stages,
    }
}

/// Count cohort accounts that reached `stage_index` or a later stage.
fn reached_stage_count(
    accounts: Option<&BTreeSet<String>>,
    stages: &BTreeMap<String, BTreeSet<String>>,
    stage_index: usize,
) -> u64 {
    accounts.map_or(0, |accounts| {
        accounts
            .iter()
            .filter(|account| {
                stages.get(*account).is_some_and(|reached| {
                    reached
                        .iter()
                        .any(|stage| FUNNEL_STAGES[stage_index..].contains(&stage.as_str()))
                })
            })
            .count() as u64
    })
}

/// Format a [`UtcMillis::month_index`] key as its `YYYY-MM` label. The fold
/// itself only compares integer keys; this allocation happens once per
/// emitted cohort, never once per event.
fn format_month_index(index: i64) -> String {
    let year = index.div_euclid(12);
    let month = index.rem_euclid(12) + 1;
    format!("{year:04}-{month:02}")
}

/// Parse a `YYYY-MM` cohort label into the integer key the folds compare.
/// An unparseable label matches no event, exactly like the previous string
/// comparison against the per-event label.
fn parse_month_index(label: &str) -> Option<i64> {
    let (year, month) = label.split_once('-')?;
    if label.len() != 7 || year.len() != 4 || month.len() != 2 {
        return None;
    }
    let year: i64 = year.parse().ok()?;
    let month: u32 = month.parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some(year * 12 + i64::from(month) - 1)
}

/// Growth cohorts by request month.
///
/// The scoped events are folded once by account: the first acceptance fixes
/// the cohort month and every later stage movement (inside the scoped
/// window) belongs to that cohort. The label is rendered only when the row is
/// emitted.
pub fn growth_cohorts(scope: &QueryScope, events: &[DomainEvent]) -> Vec<GrowthCohortRow> {
    growth_cohorts_from(scope, events.iter())
}

/// Borrowing fold behind [`growth_cohorts`]. It stores references, never
/// event clones, so the same code path can be exercised at the reader's
/// 250k-event bound without materializing the events twice.
fn growth_cohorts_from<'a>(
    scope: &QueryScope,
    events: impl IntoIterator<Item = &'a DomainEvent>,
) -> Vec<GrowthCohortRow> {
    let fold = cohort_fold(
        events
            .into_iter()
            .filter(|event| event_in_scope(scope, event)),
    );
    let mut months: BTreeSet<i64> = fold.leads_by_month.keys().copied().collect();
    months.extend(fold.accounts_by_month.keys().copied());
    months
        .into_iter()
        .map(|month_key| {
            let month = format_month_index(month_key);
            let accounts = fold.accounts_by_month.get(&month_key);
            let accepted_accounts = accounts.map_or(0, |accounts| accounts.len() as u64);
            let qualified = reached_stage_count(accounts, &fold.stages, 0);
            GrowthCohortRow {
                cohort_month: month.clone(),
                accepted: fold
                    .leads_by_month
                    .get(&month_key)
                    .map_or(0, |leads| leads.len() as u64),
                accepted_accounts,
                qualified,
                connected: reached_stage_count(accounts, &fold.stages, 1),
                audit_delivered: reached_stage_count(accounts, &fold.stages, 2),
                paid: reached_stage_count(accounts, &fold.stages, 3),
                mature: cohort_is_mature(&month, scope.as_of),
                qualification_rate_bp: rate_basis_points(qualified, accepted_accounts),
            }
        })
        .collect()
}

/// The activation funnel of one cohort.
pub fn activation_funnel(
    scope: &QueryScope,
    events: &[DomainEvent],
    cohort_month: &str,
) -> ActivationFunnel {
    activation_funnel_from(scope, events.iter(), cohort_month)
}

/// Borrowing fold behind [`activation_funnel`]: the scoped events are
/// referenced, never deep-cloned. The funnel uses the same cohort fold as
/// [`growth_cohorts`], so a stage movement after the cohort month still
/// counts for the account's cohort.
fn activation_funnel_from<'a>(
    scope: &QueryScope,
    events: impl IntoIterator<Item = &'a DomainEvent>,
    cohort_month: &str,
) -> ActivationFunnel {
    let fold = cohort_fold(
        events
            .into_iter()
            .filter(|event| event_in_scope(scope, event)),
    );
    // An unparseable label is not "every month": no account belongs to it.
    let accounts =
        parse_month_index(cohort_month).and_then(|month| fold.accounts_by_month.get(&month));
    let mut stages = vec![FunnelStageRow {
        stage: "requested".to_string(),
        accounts: accounts.map_or(0, |accounts| accounts.len() as u64),
    }];
    for (index, stage) in FUNNEL_STAGES.iter().enumerate() {
        stages.push(FunnelStageRow {
            stage: (*stage).to_string(),
            accounts: reached_stage_count(accounts, &fold.stages, index),
        });
    }
    ActivationFunnel {
        cohort_month: cohort_month.to_string(),
        mature: cohort_is_mature(cohort_month, scope.as_of),
        stages,
    }
}

/// The experiment fleet landscape: counts by state with the oldest waiting
/// reason.
pub fn experiment_landscape(
    scope: &QueryScope,
    events: &[DomainEvent],
) -> Vec<ExperimentLandscapeRow> {
    let scoped: Vec<&DomainEvent> = events
        .iter()
        .filter(|event| event_in_scope(scope, event))
        .filter(|event| {
            matches!(
                event.event_type,
                EventType::ExperimentRegistered
                    | EventType::ExperimentStarted
                    | EventType::ExperimentDecided
            )
        })
        .collect();
    let mut latest: BTreeMap<String, (&DomainEvent, String)> = BTreeMap::new();
    for event in &scoped {
        let Some(experiment_id) = event.payload.experiment_id.clone() else {
            continue;
        };
        let state = match event.event_type {
            EventType::ExperimentRegistered => "registered".to_string(),
            EventType::ExperimentStarted => event
                .payload
                .state
                .clone()
                .unwrap_or_else(|| "running".to_string()),
            EventType::ExperimentDecided => event
                .payload
                .decision
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            _ => continue,
        };
        // Only a strictly newer fact replaces the state; ties keep the
        // first event seen, and both stores feed events in the same
        // deterministic order (occurred, source, event_id), so the answer
        // does not depend on iteration luck.
        let replace = match latest.get(&experiment_id) {
            None => true,
            Some((existing, _)) => {
                event.occurred_at_utc > existing.occurred_at_utc
                    || (event.occurred_at_utc == existing.occurred_at_utc
                        && event.received_at_utc > existing.received_at_utc)
            }
        };
        if replace {
            latest.insert(experiment_id, (event, state));
        }
    }
    let mut by_state: BTreeMap<String, (u64, Option<&DomainEvent>)> = BTreeMap::new();
    for (event, state) in latest.values() {
        let entry = by_state.entry(state.clone()).or_insert((0, None));
        entry.0 += 1;
        match entry.1 {
            Some(oldest) if oldest.occurred_at_utc <= event.occurred_at_utc => {}
            _ => entry.1 = Some(event),
        }
    }
    by_state
        .into_iter()
        .map(|(state, (count, oldest))| ExperimentLandscapeRow {
            state,
            count,
            oldest_waiting_reason: oldest.and_then(|event| event.reason_code.clone()),
            oldest_age_days: oldest.map(|event| {
                (scope.as_of.unix_millis() - event.occurred_at_utc.unix_millis()) / MILLIS_PER_DAY
            }),
        })
        .collect()
}

/// Nearest-rank percentile of a sorted slice.
fn percentile(sorted: &[i64], percent: u32) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (u128::from(percent) * sorted.len() as u128).div_ceil(100);
    let index = rank.max(1).saturating_sub(1).min(sorted.len() as u128 - 1);
    sorted.get(index as usize).copied()
}

/// Effectiveness of a mature experiment start cohort: n/N rates with
/// explicit denominators, pending and in-flight tests excluded.
pub fn experiment_effectiveness(
    scope: &QueryScope,
    events: &[DomainEvent],
    cohort_month: &str,
) -> ExperimentEffectiveness {
    let scoped: Vec<&DomainEvent> = events
        .iter()
        .filter(|event| event_in_scope(scope, event))
        .collect();
    let cohort_key = parse_month_index(cohort_month);
    let mut started_at: BTreeMap<String, UtcMillis> = BTreeMap::new();
    for event in &scoped {
        if event.event_type == EventType::ExperimentStarted {
            if let Some(experiment_id) = event.payload.experiment_id.clone() {
                if Some(event.occurred_at_utc.month_index()) == cohort_key {
                    started_at
                        .entry(experiment_id)
                        .and_modify(|existing| {
                            if event.occurred_at_utc < *existing {
                                *existing = event.occurred_at_utc;
                            }
                        })
                        .or_insert(event.occurred_at_utc);
                }
            }
        }
    }
    let mut decided: BTreeMap<String, (UtcMillis, String)> = BTreeMap::new();
    for event in &scoped {
        if event.event_type != EventType::ExperimentDecided {
            continue;
        }
        let Some(experiment_id) = event.payload.experiment_id.clone() else {
            continue;
        };
        if !started_at.contains_key(&experiment_id) {
            continue;
        }
        let decision = event
            .payload
            .decision
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        decided
            .entry(experiment_id)
            .and_modify(|existing| {
                if event.occurred_at_utc < existing.0 {
                    *existing = (event.occurred_at_utc, decision.clone());
                }
            })
            .or_insert((event.occurred_at_utc, decision));
    }
    let mut kept = 0u64;
    let mut rolled_back = 0u64;
    let mut voided = 0u64;
    let mut pending = 0u64;
    let mut unknown = 0u64;
    let mut completion_days: Vec<i64> = Vec::new();
    for (experiment_id, (decided_at, decision)) in &decided {
        match decision.as_str() {
            "keep" => kept += 1,
            "rollback" => rolled_back += 1,
            "void" => voided += 1,
            "pending" => pending += 1,
            _ => unknown += 1,
        }
        if matches!(decision.as_str(), "keep" | "rollback") {
            if let Some(started) = started_at.get(experiment_id) {
                completion_days.push(
                    (decided_at.unix_millis() - started.unix_millis()).div_euclid(MILLIS_PER_DAY),
                );
            }
        }
    }
    completion_days.sort_unstable();
    let started = started_at.len() as u64;
    let valid_settled = kept + rolled_back;
    let in_flight = started.saturating_sub(decided.len() as u64);
    ExperimentEffectiveness {
        cohort_label: cohort_month.to_string(),
        mature: cohort_is_mature(cohort_month, scope.as_of),
        started,
        valid_settled,
        kept,
        rolled_back,
        voided,
        pending,
        unknown,
        in_flight,
        keep_rate_valid_bp: rate_basis_points(kept, valid_settled),
        keep_rate_started_bp: rate_basis_points(kept, started),
        completion_p50_days: percentile(&completion_days, 50),
        completion_p95_days: percentile(&completion_days, 95),
        excluded_from_denominators: vec![
            "pending".to_string(),
            "void".to_string(),
            "unknown".to_string(),
            "in_flight".to_string(),
        ],
    }
}

/// Site-wide public page requests per UTC day, inside the inclusive day
/// bounds (`""` means unbounded on that side).
///
/// The `public_page_requested` family describes the public site and carries
/// no tenant identity: this read deliberately ignores `publisher_id`,
/// `platform` and `currency` from the scope, because applying them would
/// silently hide requests (no event could ever match). Only aggregate counts
/// leave the store: one row per day, `not_found` for exactly HTTP 404, and
/// the bounded bot-class mix; never a request row, a query value or a
/// visitor identity.
pub fn page_requests_daily(
    events: &[DomainEvent],
    from_day: &str,
    to_day: &str,
) -> Vec<PageRequestsDay> {
    // Bot classes are borrowed from the events: only the handful of distinct
    // classes per day are turned into owned strings when the result is built,
    // instead of cloning a class name for every request.
    let mut days: BTreeMap<String, (u64, u64, BTreeMap<&str, u64>)> = BTreeMap::new();
    for event in events {
        if event.event_type != EventType::PublicPageRequested {
            continue;
        }
        let day = event.occurred_at_utc.to_day_string();
        if !from_day.is_empty() && day.as_str() < from_day {
            continue;
        }
        if !to_day.is_empty() && day.as_str() > to_day {
            continue;
        }
        let entry = days.entry(day).or_default();
        entry.0 += 1;
        if event.payload.status == Some(404) {
            entry.1 += 1;
        }
        // A stored event always carries the bounded class; a row without one
        // is reported as `unknown` rather than being guessed at.
        let bot_class = event.payload.bot_class.as_deref().unwrap_or("unknown");
        *entry.2.entry(bot_class).or_insert(0) += 1;
    }
    days.into_iter()
        .map(|(day, (requests, not_found, mix))| PageRequestsDay {
            day,
            requests,
            not_found,
            bot_class_mix: mix
                .into_iter()
                .map(|(class, count)| (class.to_string(), count))
                .collect(),
        })
        .collect()
}

/// The effective day bound of a page-request read: the explicit argument
/// when one is given, otherwise the scope's day range (empty means
/// unbounded on that side).
pub(crate) fn effective_day_bound(argument: &str, scope_bound: Option<&str>) -> String {
    if argument.is_empty() {
        scope_bound.unwrap_or_default().to_string()
    } else {
        argument.to_string()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::envelope::EventType;
    use crate::test_support::{payload_for, valid_event_builder};

    const N: usize = 10_000;

    fn scope() -> QueryScope {
        QueryScope::new(
            "northstar",
            UtcMillis::parse_rfc3339("2026-10-15T09:10:00.000Z").expect("as-of"),
        )
        .with_day_range("2026-08-01", "2026-10-14")
    }

    fn fee_event(i: usize, publisher: &str, period: &str) -> DomainEvent {
        let mut payload = payload_for(EventType::FeeComputed);
        payload.change_id = Some(format!("ch_{i:06}"));
        payload.period = Some(period.to_string());
        payload.currency = Some("USD".into());
        payload.source_amount_micros = Some(51_500_000_000);
        payload.bound_bp = Some(300);
        payload.rate_bp = Some(2_000);
        payload.active_days = Some(30);
        valid_event_builder(EventType::FeeComputed)
            .with_event_id(format!("fee_{i:07}"))
            .with_publisher_id(publisher)
            .with_occurred_at(UtcMillis::from_unix_millis(1_790_000_000_000 + i as i64))
            .with_received_at(UtcMillis::from_unix_millis(1_790_000_000_000 + i as i64))
            .with_payload(payload)
            .build()
            .expect("fee event")
    }

    fn report_day(i: usize, publisher: &str, day: String, finality: &str) -> ReportDayRow {
        ReportDayRow {
            publisher_id: publisher.to_string(),
            platform: "applovin_max".to_string(),
            platform_account_id: format!("acc_{:05}", i % 97),
            day,
            finality: finality.to_string(),
            revenue_micros: if i.is_multiple_of(11) {
                None
            } else {
                Some(1_500_000)
            },
            impressions: Some(1_000),
            currency: if i.is_multiple_of(13) {
                None
            } else {
                Some("USD".to_string())
            },
            country: Some("DE".to_string()),
            ad_unit_id: Some(format!("unit_{:02}", i % 8)),
            format: Some("rewarded".to_string()),
            source_digest: Some("d".to_string()),
        }
    }

    fn day_for(i: usize) -> String {
        match i % 6 {
            0 => "2026-08-01".to_string(),
            1 => "2026-09-15".to_string(),
            2 => "2026-10-14".to_string(),
            3 => "2026-07-31".to_string(),
            4 => "2026-10-15".to_string(),
            _ => format!("2026-09-{:02}", 1 + i % 28),
        }
    }

    /// The pre-optimization fee fold: the pending state rescans every report
    /// day for every accumulated change. Kept verbatim as the reference the
    /// single-fold implementation must equal.
    fn fee_rows_naive(
        scope: &QueryScope,
        events: &[DomainEvent],
        report_days: &[ReportDayRow],
    ) -> Vec<FeeRow> {
        let scoped: Vec<&DomainEvent> = events
            .iter()
            .filter(|event| event_in_scope(scope, event))
            .collect();
        let mut accumulators: BTreeMap<(String, String), FeeAccumulator> = BTreeMap::new();
        let mut paid_invoices: BTreeSet<String> = BTreeSet::new();
        for event in &scoped {
            if event.event_type == EventType::PaymentReconciled {
                if let Some(invoice_id) = event.payload.invoice_id.clone() {
                    paid_invoices.insert(invoice_id);
                }
            }
            let (Some(change_id), Some(period)) = (
                event.payload.change_id.clone(),
                event.payload.period.clone(),
            ) else {
                continue;
            };
            let accumulator = accumulators
                .entry((change_id, period.clone()))
                .or_insert_with(|| FeeAccumulator {
                    period,
                    ..FeeAccumulator::default()
                });
            let candidate = (
                event.occurred_at_utc,
                event.source.as_str(),
                event.event_id.clone(),
            );
            let strictly_newer = match &accumulator.latest_key {
                Some(current) => candidate > *current,
                None => true,
            };
            if !strictly_newer {
                continue;
            }
            accumulator.latest_key = Some(candidate);
            match event.event_type {
                EventType::FeeComputed => {
                    if let Some(currency) = event.payload.currency.clone() {
                        accumulator.currency = Some(currency);
                    }
                    accumulator.source_amount_micros = event.payload.source_amount_micros;
                    accumulator.bound_bp = event.payload.bound_bp;
                    accumulator.rate_bp = event.payload.rate_bp;
                    accumulator.active_days = event.payload.active_days;
                    if let Some(proofs) = &event.payload.proof_hashes {
                        accumulator.proof_refs = proofs.clone();
                    }
                }
                EventType::FeeHeld => {
                    accumulator.held = true;
                    if accumulator.currency.is_none() {
                        accumulator.currency = event.payload.currency.clone();
                    }
                }
                EventType::FeeInvoiceIssued | EventType::FeeCredited => {
                    if event.event_type == EventType::FeeCredited {
                        accumulator.credited = true;
                    } else {
                        accumulator.invoiced = true;
                    }
                    if let Some(invoice_id) = event.payload.invoice_id.clone() {
                        accumulator.invoice_ref = Some(invoice_id);
                    }
                    if event.payload.amount_micros.is_some() {
                        accumulator.invoice_amount_micros = event.payload.amount_micros;
                    }
                    if accumulator.currency.is_none() {
                        accumulator.currency = event.payload.currency.clone();
                    }
                }
                _ => {}
            }
        }
        let mut rows: Vec<FeeRow> = accumulators
            .into_iter()
            .map(|((change_id, period), accumulator)| {
                let currency = accumulator.currency.clone().unwrap_or_default();
                let derived_amount = match (
                    accumulator.source_amount_micros,
                    accumulator.bound_bp,
                    accumulator.rate_bp,
                ) {
                    (Some(source), Some(bound), Some(rate)) => {
                        attributed_increment_micros(source, bound)
                            .and_then(|increment| fee_before_tax_micros(increment, rate))
                    }
                    _ => None,
                };
                let amount_micros = accumulator.invoice_amount_micros.or(derived_amount);
                let paid = accumulator
                    .invoice_ref
                    .as_ref()
                    .is_some_and(|invoice| paid_invoices.contains(invoice));
                let pending_period = report_days.iter().any(|row| {
                    !scope.publisher_id.is_empty()
                        && row.publisher_id == scope.publisher_id
                        && row.day.len() >= 7
                        && &row.day[..7] == accumulator.period.as_str()
                        && row.finality == "provisional"
                });
                let state = if accumulator.credited {
                    FeeState::Credited
                } else if paid {
                    FeeState::Paid
                } else if accumulator.invoiced {
                    FeeState::Invoiced
                } else if accumulator.held {
                    FeeState::Held
                } else if pending_period {
                    FeeState::Pending
                } else {
                    FeeState::Estimated
                };
                FeeRow {
                    change_id,
                    period,
                    state,
                    amount: amount_micros.map(|micros| Money::new(micros, currency.clone())),
                    source_amount: accumulator
                        .source_amount_micros
                        .map(|micros| Money::new(micros, currency.clone())),
                    bound_bp: accumulator.bound_bp,
                    rate_bp: accumulator.rate_bp,
                    active_days: accumulator.active_days,
                    invoice_ref: accumulator.invoice_ref,
                    proof_refs: accumulator.proof_refs,
                }
            })
            .collect();
        rows.sort_by(|left, right| {
            left.period
                .cmp(&right.period)
                .then(left.change_id.cmp(&right.change_id))
        });
        rows
    }

    /// The pre-optimization data-quality fold: five passes over the scoped
    /// rows. Kept verbatim as the reference.
    fn data_quality_naive(
        scope: &QueryScope,
        report_days: &[ReportDayRow],
        dedup_rejects: Option<u64>,
    ) -> DataQuality {
        let scoped: Vec<&ReportDayRow> = report_days
            .iter()
            .filter(|row| report_day_in_scope(scope, row))
            .collect();
        let matched_source_days = scoped
            .iter()
            .map(|row| {
                (
                    row.platform.clone(),
                    row.platform_account_id.clone(),
                    row.day.clone(),
                )
            })
            .collect::<BTreeSet<_>>()
            .len() as u64;
        let provisional_days = scoped
            .iter()
            .filter(|row| row.finality == "provisional")
            .count() as u64;
        let unmapped_rows = scoped
            .iter()
            .filter(|row| row.revenue_micros.is_none() || row.currency.is_none())
            .count() as u64;
        let mut unavailable = Vec::new();
        if dedup_rejects.is_none() {
            unavailable
                .push("dedup_rejects: this store cannot report raw duplicate rows".to_string());
        }
        let (missing_days, missing_reason) = match (
            scope.from_day.as_deref(),
            scope.to_day.as_deref(),
        ) {
            (Some(from), Some(to)) => {
                let parse =
                    |text: &str| UtcMillis::parse_rfc3339(&format!("{text}T00:00:00Z")).ok();
                match (parse(from), parse(to)) {
                    (Some(from), Some(to)) => {
                        let range_days = (to.day_index() - from.day_index() + 1).max(0) as u64;
                        let mut per_account: BTreeMap<(String, String), BTreeSet<String>> =
                            BTreeMap::new();
                        for row in &scoped {
                            per_account
                                .entry((row.platform.clone(), row.platform_account_id.clone()))
                                .or_default()
                                .insert(row.day.clone());
                        }
                        if per_account.is_empty() {
                            (
                                    None,
                                    Some(
                                        "missing_days: the store holds no report rows for the requested range",
                                    ),
                                )
                        } else {
                            (
                                Some(
                                    per_account
                                        .values()
                                        .map(|days| range_days.saturating_sub(days.len() as u64))
                                        .sum(),
                                ),
                                None,
                            )
                        }
                    }
                    _ => (
                        None,
                        Some("missing_days: the requested day range is not parseable"),
                    ),
                }
            }
            _ => (None, Some("missing_days: no day range was requested")),
        };
        if let Some(reason) = missing_reason {
            unavailable.push(reason.to_string());
        }
        let mut sources: BTreeMap<(String, String), Option<String>> = BTreeMap::new();
        for row in &scoped {
            let key = (row.platform.clone(), row.platform_account_id.clone());
            let entry = sources.entry(key).or_insert(None);
            if row.finality == "final" {
                match entry {
                    Some(current) if current.as_str() >= row.day.as_str() => {}
                    _ => *entry = Some(row.day.clone()),
                }
            }
        }
        let as_of_day = scope.as_of.day_index();
        let mut stale_sources: Vec<StaleSource> = Vec::new();
        for ((platform, platform_account_id), last_final_day) in sources {
            let freshness_days = last_final_day.as_deref().and_then(|day| {
                UtcMillis::parse_rfc3339(&format!("{day}T00:00:00Z"))
                    .ok()
                    .map(|at| as_of_day - at.day_index())
            });
            let stale = freshness_days
                .map(|days| days > FRESHNESS_STALE_DAYS)
                .unwrap_or(true);
            if stale {
                stale_sources.push(StaleSource {
                    platform,
                    platform_account_id,
                    last_final_day,
                    freshness_days,
                });
            }
        }
        DataQuality {
            publisher_id: scope.publisher_id.clone(),
            as_of: scope.as_of.to_rfc3339_millis(),
            matched_source_days,
            missing_days,
            provisional_days,
            dedup_rejects,
            unmapped_rows,
            stale_sources,
            unavailable,
        }
    }

    /// The reference cohort fold: for every accepted account it collects
    /// the stages reached at or after its first acceptance, across the whole
    /// scoped window, then counts per cohort month. Deliberately written
    /// with owned sets and a scan per account, independent of the optimized
    /// account-centric fold.
    fn growth_cohorts_naive(scope: &QueryScope, events: &[DomainEvent]) -> Vec<GrowthCohortRow> {
        let scoped: Vec<&DomainEvent> = events
            .iter()
            .filter(|event| event_in_scope(scope, event))
            .collect();
        let mut first_accept: BTreeMap<String, (String, UtcMillis)> = BTreeMap::new();
        let mut leads_by_month: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for event in &scoped {
            if event.event_type != EventType::AuditRequestAccepted {
                continue;
            }
            let month = event.occurred_at_utc.to_day_string()[..7].to_string();
            if let Some(lead_id) = event.payload.lead_id.clone() {
                leads_by_month
                    .entry(month.clone())
                    .or_default()
                    .insert(lead_id);
            }
            if let Some(account) = account_of(event) {
                let entry = first_accept
                    .entry(account)
                    .or_insert((month.clone(), event.occurred_at_utc));
                if event.occurred_at_utc < entry.1 {
                    *entry = (month, event.occurred_at_utc);
                }
            }
        }
        let mut by_month: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (account, (month, _)) in &first_accept {
            by_month
                .entry(month.clone())
                .or_default()
                .insert(account.clone());
        }
        // A month can have accepted leads that resolved no account; it is
        // still a cohort row (with zero accounts).
        for month in leads_by_month.keys() {
            by_month.entry(month.clone()).or_default();
        }
        by_month
            .into_iter()
            .map(|(month, accounts)| {
                let mut reached: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                for event in &scoped {
                    if event.event_type != EventType::AccountStageChanged {
                        continue;
                    }
                    let Some(account) = account_of(event) else {
                        continue;
                    };
                    let Some((_, accepted_at)) = first_accept.get(&account) else {
                        continue;
                    };
                    if event.occurred_at_utc >= *accepted_at {
                        reached
                            .entry(account)
                            .or_default()
                            .insert(event.payload.next_stage.clone().unwrap_or_default());
                    }
                }
                let stage_count = |index: usize| -> u64 {
                    accounts
                        .iter()
                        .filter(|account| {
                            reached.get(*account).is_some_and(|stages| {
                                stages
                                    .iter()
                                    .any(|stage| FUNNEL_STAGES[index..].contains(&stage.as_str()))
                            })
                        })
                        .count() as u64
                };
                let qualified = stage_count(0);
                GrowthCohortRow {
                    cohort_month: month.clone(),
                    accepted: leads_by_month
                        .get(&month)
                        .map_or(0, |leads| leads.len() as u64),
                    accepted_accounts: accounts.len() as u64,
                    qualified,
                    connected: stage_count(1),
                    audit_delivered: stage_count(2),
                    paid: stage_count(3),
                    mature: cohort_is_mature(&month, scope.as_of),
                    qualification_rate_bp: rate_basis_points(qualified, accounts.len() as u64),
                }
            })
            .collect()
    }

    fn scope_wide() -> QueryScope {
        QueryScope::operator_wide(
            UtcMillis::parse_rfc3339("2026-10-15T09:10:00.000Z").expect("as-of"),
        )
    }

    fn month_time(i: usize) -> UtcMillis {
        let month = 1 + (i % 12);
        let year = 2025 + (i % 24) / 12;
        let day = 1 + (i % 28);
        UtcMillis::parse_rfc3339(&format!("{year}-{month:02}-{day:02}T09:00:00.000Z"))
            .expect("month time")
    }

    #[test]
    fn fee_rows_fold_equals_the_naive_report_day_scan() {
        let scope = scope();
        let events: Vec<DomainEvent> = (0..N)
            .map(|i| {
                let period = if i % 3 == 0 { "2026-08" } else { "2026-09" };
                let publisher = if i % 17 == 0 { "other" } else { "northstar" };
                fee_event(i, publisher, period)
            })
            .collect();
        // 400 report days: several publishers' provisional and final rows,
        // plus out-of-range and short-day rows the naive scan must also see.
        let days: Vec<ReportDayRow> = (0..400)
            .map(|i| {
                let publisher = match i % 3 {
                    0 => "northstar",
                    1 => "other",
                    _ => "third",
                };
                let finality = if i % 5 == 0 { "final" } else { "provisional" };
                let day = match i % 7 {
                    0 => "2026-08-31".to_string(),
                    1 => "2026-09-01".to_string(),
                    2 => "2026-10-14".to_string(),
                    3 => "bad".to_string(),
                    _ => format!("2026-09-{:02}", 1 + i % 28),
                };
                report_day(i, publisher, day, finality)
            })
            .collect();
        let optimized = fee_rows(&scope, &events, &days);
        let reference = fee_rows_naive(&scope, &events, &days);
        assert_eq!(optimized, reference);
        assert!(optimized.iter().any(|row| row.state == FeeState::Pending));
    }

    #[test]
    fn data_quality_fold_equals_the_naive_multi_pass_badge() {
        let scope = scope();
        let days: Vec<ReportDayRow> = (0..N)
            .map(|i| {
                let publisher = if i % 5 == 0 { "other" } else { "northstar" };
                let finality = match i % 4 {
                    0 => "final",
                    1 => "provisional",
                    _ => "partial",
                };
                report_day(i, publisher, day_for(i), finality)
            })
            .collect();
        let optimized = data_quality(&scope, &days, Some(7));
        let reference = data_quality_naive(&scope, &days, Some(7));
        assert_eq!(optimized, reference);
        // The no-range path reports the honest unavailable reason on both.
        let wide = QueryScope::operator_wide(scope.as_of);
        assert_eq!(
            data_quality(&wide, &days, None),
            data_quality_naive(&wide, &days, None)
        );
    }

    fn filter_types(events: &[DomainEvent], event_types: &[EventType]) -> Vec<DomainEvent> {
        events
            .iter()
            .filter(|event| event_types.contains(&event.event_type))
            .cloned()
            .collect()
    }

    /// Deterministic pseudo-random events covering every catalog type and a
    /// range of months. Used only to prove that the type lists a reader
    /// filters at the SQL layer are exactly the types its fold reads.
    fn randomized_events(count: usize) -> Vec<DomainEvent> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        (0..count)
            .map(|index| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let event_type = EventType::ALL[(state as usize) % EventType::ALL.len()];
                let year = 2025 + ((state >> 8) % 2) as i64;
                let month = 1 + ((state >> 16) % 12) as u32;
                let day = 1 + ((state >> 24) % 28) as u32;
                let occurred =
                    UtcMillis::parse_rfc3339(&format!("{year}-{month:02}-{day:02}T09:00:00.000Z"))
                        .expect("randomized time");
                valid_event_builder(event_type)
                    .with_event_id(format!("rand_{index:05}"))
                    .with_occurred_at(occurred)
                    .with_received_at(occurred)
                    .build()
                    .expect("randomized event")
            })
            .collect()
    }

    #[test]
    fn type_scoped_lists_match_the_unfiltered_fold_over_randomized_events() {
        use crate::queries::{
            EXPERIMENT_EFFECTIVENESS_EVENT_TYPES, EXPERIMENT_LANDSCAPE_EVENT_TYPES,
            FEE_EVENT_TYPES, GROWTH_EVENT_TYPES, LIFT_EVENT_TYPES, PAGE_REQUEST_EVENT_TYPES,
        };

        let events = randomized_events(600);
        let days: Vec<ReportDayRow> = (0..60)
            .map(|index| {
                let publisher = if index % 4 == 0 { "other" } else { "northstar" };
                let finality = if index % 3 == 0 {
                    "final"
                } else {
                    "provisional"
                };
                report_day(index, publisher, day_for(index), finality)
            })
            .collect();
        let scope = scope();

        assert_eq!(
            lift_rows(&scope, &events),
            lift_rows(&scope, &filter_types(&events, LIFT_EVENT_TYPES))
        );
        assert_eq!(
            fee_rows(&scope, &events, &days),
            fee_rows(&scope, &filter_types(&events, FEE_EVENT_TYPES), &days)
        );
        assert_eq!(
            growth_cohorts(&scope, &events),
            growth_cohorts(&scope, &filter_types(&events, GROWTH_EVENT_TYPES))
        );
        for month in ["2025-01", "2025-12", "2026-01", "2026-12"] {
            assert_eq!(
                activation_funnel(&scope, &events, month),
                activation_funnel(&scope, &filter_types(&events, GROWTH_EVENT_TYPES), month)
            );
            assert_eq!(
                experiment_effectiveness(&scope, &events, month),
                experiment_effectiveness(
                    &scope,
                    &filter_types(&events, EXPERIMENT_EFFECTIVENESS_EVENT_TYPES),
                    month
                )
            );
        }
        assert_eq!(
            experiment_landscape(&scope, &events),
            experiment_landscape(
                &scope,
                &filter_types(&events, EXPERIMENT_LANDSCAPE_EVENT_TYPES)
            )
        );
        assert_eq!(
            page_requests_daily(&events, "2025-01-01", "2026-12-31"),
            page_requests_daily(
                &filter_types(&events, PAGE_REQUEST_EVENT_TYPES),
                "2025-01-01",
                "2026-12-31"
            )
        );
    }

    #[test]
    fn growth_cohorts_compare_integer_month_keys_across_year_boundaries() {
        let scope = scope_wide();
        let boundary_times = [
            "2025-12-31T23:59:59.999Z",
            "2026-01-01T00:00:00.000Z",
            "2026-01-31T23:59:59.999Z",
            "2026-02-01T00:00:00.000Z",
        ];
        let mut events = Vec::new();
        for (index, time) in boundary_times.iter().enumerate() {
            let occurred = UtcMillis::parse_rfc3339(time).expect("boundary time");
            let mut payload = payload_for(EventType::AuditRequestAccepted);
            payload.lead_id = Some(format!("lead_{index}"));
            payload.account_id = Some(format!("acct_{index}"));
            events.push(
                valid_event_builder(EventType::AuditRequestAccepted)
                    .with_event_id(format!("boundary_request_{index}"))
                    .with_occurred_at(occurred)
                    .with_received_at(occurred)
                    .with_object("account", format!("acct_{index}"))
                    .with_payload(payload)
                    .build()
                    .expect("boundary request"),
            );
        }
        let rows = growth_cohorts(&scope, &events);
        let labels: Vec<&str> = rows.iter().map(|row| row.cohort_month.as_str()).collect();
        assert_eq!(labels, vec!["2025-12", "2026-01", "2026-02"]);
        assert_eq!(
            rows,
            growth_cohorts_naive(&scope, &events),
            "the integer key fold equals the per-month string rescan"
        );
        let funnel = activation_funnel(&scope, &events, "2026-01");
        assert_eq!(funnel.stages[0].accounts, 2);
        assert_eq!(funnel.cohort_month, "2026-01");
        assert!(
            activation_funnel(&scope, &events, "2026-1").stages[0].accounts == 0,
            "an invalid label matches no event"
        );
        let invalid = experiment_effectiveness(&scope, &events, "2026-1");
        assert_eq!(invalid.started, 0);
        assert_eq!(invalid.valid_settled, 0);
        assert!(!invalid.mature);
    }

    #[test]
    fn growth_and_funnel_folds_reach_the_250k_event_cap_by_reference() {
        use crate::queries::clickhouse::OPERATOR_QUEUE_EVENT_LIMIT;

        let mut payload = payload_for(EventType::AuditRequestAccepted);
        payload.lead_id = Some("lead_cap".into());
        payload.account_id = Some("acct_cap".into());
        let event = valid_event_builder(EventType::AuditRequestAccepted)
            .with_event_id("cap_event")
            .with_payload(payload)
            .build()
            .expect("cap event");
        let scope = scope_wide();
        let events = std::iter::repeat_n(&event, OPERATOR_QUEUE_EVENT_LIMIT);

        let rows = growth_cohorts_from(&scope, events.clone());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].accepted_accounts, 1);
        assert_eq!(rows[0].accepted, 1);
        let funnel = activation_funnel_from(&scope, events, "2026-10");
        assert_eq!(funnel.stages[0].accounts, 1);
        assert_eq!(funnel.stages[1].accounts, 0);
    }

    #[test]
    fn growth_cohorts_fold_equals_the_naive_per_month_rescan() {
        let scope = scope_wide();
        let events: Vec<DomainEvent> = (0..N)
            .map(|i| {
                let mut payload = payload_for(EventType::AuditRequestAccepted);
                payload.lead_id = Some(format!("lead_{i:07}"));
                valid_event_builder(EventType::AuditRequestAccepted)
                    .with_event_id(format!("acc_{i:07}"))
                    .with_occurred_at(month_time(i))
                    .with_received_at(month_time(i))
                    .with_object("account", format!("acct_{i:07}"))
                    .with_payload(payload)
                    .build()
                    .expect("accepted event")
            })
            .collect();
        let optimized = growth_cohorts(&scope, &events);
        let reference = growth_cohorts_naive(&scope, &events);
        assert_eq!(optimized, reference);
        assert!(optimized.len() > 1, "the fixture spans several months");
    }

    #[test]
    fn a_stage_move_after_the_cohort_month_still_belongs_to_the_cohort() {
        // Accepted on the last day of August, qualified in September: the
        // account is an August cohort member and its September progression
        // must be visible in both the cohort row and the funnel. The old
        // same-month fold reported zero for both.
        let accepted = {
            let mut payload = payload_for(EventType::AuditRequestAccepted);
            payload.lead_id = Some("lead_aug".into());
            payload.account_id = Some("acct_aug".into());
            valid_event_builder(EventType::AuditRequestAccepted)
                .with_event_id("accepted_aug")
                .with_occurred_at(UtcMillis::parse_rfc3339("2026-08-31T20:00:00Z").expect("time"))
                .with_received_at(UtcMillis::parse_rfc3339("2026-08-31T20:00:00Z").expect("time"))
                .with_object("account", "acct_aug")
                .with_payload(payload)
                .build()
                .expect("accepted")
        };
        let qualified = {
            let mut payload = payload_for(EventType::AccountStageChanged);
            payload.account_id = Some("acct_aug".into());
            payload.previous_stage = Some("audit_requested".into());
            payload.next_stage = Some("qualified".into());
            payload.owner = Some("lina".into());
            payload.effective_at = Some("2026-09-02".into());
            valid_event_builder(EventType::AccountStageChanged)
                .with_event_id("qualified_sep")
                .with_occurred_at(UtcMillis::parse_rfc3339("2026-09-02T09:00:00Z").expect("time"))
                .with_received_at(UtcMillis::parse_rfc3339("2026-09-02T09:00:00Z").expect("time"))
                .with_reason_code("stage_move")
                .with_object("account", "acct_aug")
                .with_payload(payload)
                .build()
                .expect("qualified")
        };
        let events = vec![accepted, qualified];
        let cohorts = growth_cohorts(&scope_wide(), &events);
        let august = cohorts
            .iter()
            .find(|row| row.cohort_month == "2026-08")
            .expect("August cohort");
        assert_eq!(august.accepted_accounts, 1);
        assert_eq!(august.qualified, 1, "September qualification counts");
        let funnel = activation_funnel(&scope_wide(), &events, "2026-08");
        assert_eq!(funnel.stages[0].accounts, 1, "requested");
        assert_eq!(funnel.stages[1].accounts, 1, "qualified");
    }
}
