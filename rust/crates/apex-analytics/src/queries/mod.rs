//! Read models: tenant-scoped, typed, and gated by the staged projection
//! publish.
//!
//! Section 05: "Project validated events into tenant-scoped daily summaries
//! and object timelines" (page 73); "A customer cannot change publisher
//! filters to read another account" (page 87, Authorization and export);
//! "the UI displays engine results, it does not recompute financial truth"
//! (page 77); and every summary carries population, period and as-of time
//! (page 84, Mandatory context).
//!
//! Every method takes a [`QueryScope`] that carries the tenant and the
//! as-of time. Both backends run the same pure computations
//! ([`model`]) over scoped facts, so a MemoryStore fixture and the real
//! ClickHouse server return identical DTOs; the ClickHouse reader fetches
//! with `{name:Type}` bound parameters only, never interpolated input.
//! Money read models read the projection tables pinned to the highest
//! PUBLISHED as-of version: a staged rebuild is invisible until
//! [`crate::projections::publish`] runs after a clean compare.

mod clickhouse;
mod dto;
mod memory;
mod model;

pub use clickhouse::ClickHouseReader;
pub use dto::{
    ActivationFunnel, BreakdownDimension, DataQuality, ExperimentEffectiveness,
    ExperimentLandscapeRow, FeeDetail, FeeRow, FeeState, FreshnessStatus, FunnelStageRow,
    GrowthCohortRow, IngestionHealthRow, LiftRow, Money, ObjectTimelineEntry, OperatorQueues,
    QueryScope, RevenueBreakdownRow, RevenueDailyRow, StaleSource,
};
pub use memory::MemoryReader;
pub use model::{
    attributed_increment_micros, fee_before_tax_micros, rate_basis_points, COHORT_MATURITY_DAYS,
    FRESHNESS_STALE_DAYS, FUNNEL_STAGES,
};

use async_trait::async_trait;

use crate::store::StoreError;

/// The tenant-scoped read models the console and exports require.
#[async_trait]
pub trait AnalyticsReader: Send + Sync + std::fmt::Debug {
    /// Daily platform revenue for the scope.
    async fn revenue_daily(&self, scope: &QueryScope) -> Result<Vec<RevenueDailyRow>, StoreError>;
    /// Revenue broken down by one dimension.
    async fn revenue_breakdown(
        &self,
        scope: &QueryScope,
        dimension: BreakdownDimension,
    ) -> Result<Vec<RevenueBreakdownRow>, StoreError>;
    /// Experiment decisions as lift rows in basis points.
    async fn lift_rows(&self, scope: &QueryScope) -> Result<Vec<LiftRow>, StoreError>;
    /// Fee lines with states and invoice references.
    async fn fee_rows(&self, scope: &QueryScope) -> Result<Vec<FeeRow>, StoreError>;
    /// The reproducible fee calculation of one change.
    async fn fee_detail(
        &self,
        scope: &QueryScope,
        change_id: &str,
    ) -> Result<Option<FeeDetail>, StoreError>;
    /// The data-quality badge for the scope.
    async fn data_quality(&self, scope: &QueryScope) -> Result<DataQuality, StoreError>;
    /// Timeline of one object, merged by occurrence time.
    async fn object_timeline(
        &self,
        scope: &QueryScope,
        object_type: &str,
        object_id: &str,
    ) -> Result<Vec<ObjectTimelineEntry>, StoreError>;
    /// The five operator queues.
    async fn operator_queues(&self, scope: &QueryScope) -> Result<OperatorQueues, StoreError>;
    /// Freshness per publisher/platform/account.
    async fn ingestion_health(
        &self,
        scope: &QueryScope,
    ) -> Result<Vec<IngestionHealthRow>, StoreError>;
    /// Growth cohorts by request month.
    async fn growth_cohorts(&self, scope: &QueryScope) -> Result<Vec<GrowthCohortRow>, StoreError>;
    /// The activation funnel of one cohort.
    async fn activation_funnel(
        &self,
        scope: &QueryScope,
        cohort_month: &str,
    ) -> Result<ActivationFunnel, StoreError>;
    /// The experiment fleet landscape.
    async fn experiment_landscape(
        &self,
        scope: &QueryScope,
    ) -> Result<Vec<ExperimentLandscapeRow>, StoreError>;
    /// Effectiveness of a mature experiment start cohort.
    async fn experiment_effectiveness(
        &self,
        scope: &QueryScope,
        cohort_month: &str,
    ) -> Result<ExperimentEffectiveness, StoreError>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_known_value_arithmetic_is_exact() {
        // Acceptance fixture: $51,500 at a 3 percent bound is $1,500; at the
        // agreed 20 percent rate the fee is $300 before tax.
        let increment = attributed_increment_micros(51_500_000_000, 300).expect("increment");
        assert_eq!(increment, 1_500_000_000);
        let fee = fee_before_tax_micros(increment, 2_000).expect("fee");
        assert_eq!(fee, 300_000_000);
    }

    #[test]
    fn attribution_matches_micros_math() {
        // 100.00 at 5 percent: 100 / 1.05 = 95.238095...; increment
        // 4.761904... truncated to integer micros.
        assert_eq!(
            attributed_increment_micros(100_000_000, 500),
            Some(4_761_904)
        );
        // A zero bound yields a real zero increment, not missing data.
        assert_eq!(attributed_increment_micros(100_000_000, 0), Some(0));
        // A bound at or below -100 percent has no denominator.
        assert_eq!(attributed_increment_micros(100_000_000, -10_000), None);
        assert_eq!(attributed_increment_micros(100_000_000, -20_000), None);
        // An unusable bound (i64::MAX) cannot even form its denominator.
        assert_eq!(attributed_increment_micros(i64::MAX, i64::MAX), None);
    }

    #[test]
    fn fees_require_the_contract_rate() {
        let increment = 1_500_000_000;
        assert_eq!(fee_before_tax_micros(increment, 1_500), Some(225_000_000));
        assert_eq!(fee_before_tax_micros(increment, 2_000), Some(300_000_000));
        // Outside 15 to 20 percent there is no reproducible fee.
        assert_eq!(fee_before_tax_micros(increment, 1_499), None);
        assert_eq!(fee_before_tax_micros(increment, 2_001), None);
        // No lift, no fee.
        assert_eq!(fee_before_tax_micros(0, 2_000), Some(0));
    }

    #[test]
    fn rates_need_a_denominator() {
        assert_eq!(rate_basis_points(0, 0), None);
        assert_eq!(rate_basis_points(1, 3), Some(3_333));
        assert_eq!(rate_basis_points(50, 100), Some(5_000));
        assert_eq!(rate_basis_points(0, 10), Some(0));
    }

    #[test]
    fn money_is_micros_plus_currency() {
        let money = Money::new(51_500_000_000, "USD");
        assert_eq!(money.micros, 51_500_000_000);
        assert_eq!(money.currency, "USD");
        let json = serde_json::to_string(&money).expect("json");
        assert_eq!(json, "{\"micros\":51500000000,\"currency\":\"USD\"}");
    }

    #[test]
    fn states_and_dimensions_round_trip() {
        for state in [
            FeeState::Estimated,
            FeeState::Pending,
            FeeState::Held,
            FeeState::Invoiced,
            FeeState::Paid,
            FeeState::Credited,
        ] {
            assert_eq!(FeeState::parse(state.as_str()), Some(state));
            assert_eq!(state.to_string(), state.as_str());
        }
        assert_eq!(FeeState::parse("nope"), None);
        for status in [
            FreshnessStatus::Fresh,
            FreshnessStatus::Stale,
            FreshnessStatus::MissingData,
        ] {
            assert_eq!(status.to_string(), status.as_str());
        }
        for dimension in [
            BreakdownDimension::Platform,
            BreakdownDimension::Format,
            BreakdownDimension::Country,
            BreakdownDimension::AdUnit,
        ] {
            assert_eq!(dimension.to_string(), dimension.as_str());
        }
    }

    #[test]
    fn scopes_are_tenant_scoped_by_default() {
        let now = crate::timestamp::UtcMillis::from_unix_millis(1_000_000);
        let scope = QueryScope::new("northstar", now).with_platform("applovin_max");
        assert_eq!(scope.publisher_id, "northstar");
        let operator = QueryScope::operator_wide(now);
        assert!(operator.publisher_id.is_empty());
        assert_eq!(FeeState::parse("invoiced"), Some(FeeState::Invoiced));
        assert_eq!(FeeState::Estimated.as_str(), "estimated");
        assert_eq!(FreshnessStatus::MissingData.as_str(), "missing_data");
        assert_eq!(BreakdownDimension::AdUnit.as_str(), "ad_unit");
    }
}
