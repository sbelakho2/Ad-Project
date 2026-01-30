/**
 * Migration Guardrail Snapshot Service
 * 
 * Aggregates shadow/mirroring outcomes into guardrail snapshots for evaluation.
 * The guardrail evaluation system depends on periodic snapshots in `migration_guardrail_snapshots`.
 * 
 * UNIT CONVENTIONS:
 * - bid_cpm: Cost Per Mille (revenue per 1000 impressions) in dollars
 * - revenue_micros: Revenue in micro-dollars (1 dollar = 1,000,000 micros)
 * - For a single impression: revenue_micros = (bid_cpm / 1000) * 1,000,000 = bid_cpm * 1000
 */

import { Pool } from 'pg';
import logger from '../utils/logger';
import { migrationGuardrailSnapshotsGenerated } from '../utils/prometheus';
import { CPM_DOLLARS_TO_IMPRESSION_MICROS } from '../utils/migrationUnits';

const ROLLING_WINDOW_MINUTES = 60;

export interface SnapshotAggregationResult {
  experimentId: string;
  controlSnapshot?: { impressions: number; fills: number; revenueMicros: number };
  testSnapshot?: { impressions: number; fills: number; revenueMicros: number };
  skipped?: string;
}

export class MigrationGuardrailSnapshotService {
  constructor(private pool: Pool) {}

  /**
   * Aggregate shadow outcomes into guardrail snapshots for a specific experiment
   * 
   * PERFORMANCE FIX: Uses sample_percentile_if_large() function for large datasets
   * to avoid O(n log n) sorting overhead of PERCENTILE_CONT.
   */
  async aggregateForExperiment(experimentId: string, windowMinutes = ROLLING_WINDOW_MINUTES): Promise<SnapshotAggregationResult> {
    const result: SnapshotAggregationResult = { experimentId };

    try {
      // Get aggregated metrics from shadow outcomes grouped by arm
      // CRITICAL: bid_cpm is Cost Per Mille (per 1000 impressions).
      // For a single impression, revenue_micros = bid_cpm * 1000 (not * 1000000!)
      // Formula: (bid_cpm / 1000) * 1,000,000 = bid_cpm * 1000
      //
      // STATISTICS FIX: Also track SUM(bid_cpm²) for proper variance calculation
      // Var(X) = (Σx² - (Σx)²/n) / (n-1)
      //
      // PERFORMANCE FIX: For latency percentiles, we use a two-phase approach:
      // 1. Get counts and sums efficiently (no sorting)
      // 2. Use sample_percentile_if_large() for percentiles (auto-samples for large datasets)
      const aggregationResult = await this.pool.query(
        `WITH counts AS (
           SELECT 
             arm,
             COUNT(*) FILTER (WHERE status = 'win' OR status = 'served') AS impressions,
             COUNT(*) FILTER (WHERE status IN ('win', 'served', 'no_fill', 'bid')) AS fills,
             COALESCE(SUM(CASE WHEN bid_cpm IS NOT NULL THEN bid_cpm * ${CPM_DOLLARS_TO_IMPRESSION_MICROS} ELSE 0 END), 0)::BIGINT AS revenue_micros,
             COALESCE(SUM(CASE WHEN bid_cpm IS NOT NULL THEN (bid_cpm * ${CPM_DOLLARS_TO_IMPRESSION_MICROS}) * (bid_cpm * ${CPM_DOLLARS_TO_IMPRESSION_MICROS}) ELSE 0 END), 0)::NUMERIC AS revenue_sum_squares,
             COALESCE(SUM(CASE WHEN latency_ms IS NOT NULL THEN latency_ms * latency_ms ELSE 0 END), 0)::NUMERIC AS latency_sum_squares,
             ROUND(100.0 * COUNT(*) FILTER (WHERE status = 'error') / NULLIF(COUNT(*), 0), 2) AS error_rate_percent,
             COUNT(*) FILTER (WHERE latency_ms IS NOT NULL) AS latency_sample_count
           FROM migration_shadow_outcomes
           WHERE experiment_id = $1
             AND created_at >= NOW() - ($2::integer || ' minutes')::interval
           GROUP BY arm
         )
         SELECT 
           c.*,
           -- Use sampling for percentiles if dataset is large (>100k rows)
           -- Falls back to exact PERCENTILE_CONT for smaller datasets
           CASE WHEN c.latency_sample_count > 100000 
             THEN sample_percentile_if_large($1, c.arm, 0.95, $2)
             ELSE (SELECT PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY latency_ms)
                   FROM migration_shadow_outcomes m
                   WHERE m.experiment_id = $1 AND m.arm = c.arm
                     AND m.created_at >= NOW() - ($2::integer || ' minutes')::interval)
           END AS latency_p95_ms,
           CASE WHEN c.latency_sample_count > 100000 
             THEN sample_percentile_if_large($1, c.arm, 0.50, $2)
             ELSE (SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_ms)
                   FROM migration_shadow_outcomes m
                   WHERE m.experiment_id = $1 AND m.arm = c.arm
                     AND m.created_at >= NOW() - ($2::integer || ' minutes')::interval)
           END AS latency_p50_ms
         FROM counts c`,
        [experimentId, windowMinutes]
      );

      if (aggregationResult.rows.length === 0) {
        result.skipped = 'no_data';
        return result;
      }

      // Insert snapshots for each arm
      for (const row of aggregationResult.rows) {
        const arm = row.arm;
        const impressions = Number(row.impressions) || 0;
        const fills = Number(row.fills) || 0;
        const revenueMicros = Number(row.revenue_micros) || 0;
        const revenueSumSquares = Number(row.revenue_sum_squares) || 0;
        const latencyP95 = row.latency_p95_ms ? Math.round(Number(row.latency_p95_ms)) : null;
        const latencyP50 = row.latency_p50_ms ? Math.round(Number(row.latency_p50_ms)) : null;
        const latencySumSquares = Number(row.latency_sum_squares) || 0;
        const errorRate = row.error_rate_percent !== null ? Number(row.error_rate_percent) : null;

        await this.pool.query(
          `INSERT INTO migration_guardrail_snapshots (
             experiment_id, arm, impressions, fills, revenue_micros, revenue_sum_squares,
             latency_p95_ms, latency_p50_ms, latency_sum_squares, error_rate_percent, rolling_window_minutes
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)`,
          [
            experimentId,
            arm,
            impressions,
            fills,
            revenueMicros,
            revenueSumSquares,
            latencyP95,
            latencyP50,
            latencySumSquares,
            errorRate,
            windowMinutes,
          ]
        );

        if (arm === 'control') {
          result.controlSnapshot = { impressions, fills, revenueMicros };
        } else if (arm === 'test') {
          result.testSnapshot = { impressions, fills, revenueMicros };
        }

        // Prometheus counter
        if (typeof migrationGuardrailSnapshotsGenerated?.inc === 'function') {
          migrationGuardrailSnapshotsGenerated.inc({ arm });
        }
      }

      logger.debug('Aggregated guardrail snapshot', {
        experimentId,
        windowMinutes,
        control: result.controlSnapshot,
        test: result.testSnapshot,
      });

      return result;
    } catch (error) {
      logger.error('Failed to aggregate guardrail snapshot', { experimentId, error });
      throw error;
    }
  }

  /**
   * Aggregate snapshots for all active experiments
   */
  async aggregateAllActive(): Promise<{ processed: number; skipped: number; errors: number }> {
    const stats = { processed: 0, skipped: 0, errors: 0 };

    try {
      // Get all active experiments
      const activeExperiments = await this.pool.query(
        `SELECT id FROM migration_experiments WHERE status = 'active'`
      );

      logger.info('Running guardrail snapshot aggregation', {
        activeExperiments: activeExperiments.rows.length,
      });

      for (const row of activeExperiments.rows) {
        try {
          const result = await this.aggregateForExperiment(row.id);
          if (result.skipped) {
            stats.skipped++;
          } else {
            stats.processed++;
          }
        } catch (error) {
          stats.errors++;
          logger.warn('Failed to aggregate experiment snapshot', {
            experimentId: row.id,
            error,
          });
        }
      }

      logger.info('Guardrail snapshot aggregation complete', stats);
      return stats;
    } catch (error) {
      logger.error('Critical failure in guardrail snapshot aggregation', { error });
      throw error;
    }
  }

  /**
   * Clean up old snapshots (> 7 days) to manage table size
   */
  async cleanupOldSnapshots(retentionDays = 7): Promise<number> {
    try {
      const result = await this.pool.query(
        `DELETE FROM migration_guardrail_snapshots
         WHERE captured_at < NOW() - ($1::integer || ' days')::interval`,
        [retentionDays]
      );

      const deleted = result.rowCount ?? 0;
      if (deleted > 0) {
        logger.info('Cleaned up old guardrail snapshots', { deleted, retentionDays });
      }
      return deleted;
    } catch (error) {
      logger.error('Failed to cleanup old snapshots', { error });
      throw error;
    }
  }
}

export default MigrationGuardrailSnapshotService;
