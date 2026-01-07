/**
 * Migration Guardrail Snapshot Service
 * 
 * Aggregates shadow/mirroring outcomes into guardrail snapshots for evaluation.
 * The guardrail evaluation system depends on periodic snapshots in `migration_guardrail_snapshots`.
 */

import { Pool } from 'pg';
import logger from '../utils/logger';
import { migrationGuardrailSnapshotsGenerated } from '../utils/prometheus';

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
   */
  async aggregateForExperiment(experimentId: string, windowMinutes = ROLLING_WINDOW_MINUTES): Promise<SnapshotAggregationResult> {
    const result: SnapshotAggregationResult = { experimentId };

    try {
      // Get aggregated metrics from shadow outcomes grouped by arm
      const aggregationResult = await this.pool.query(
        `SELECT 
           arm,
           COUNT(*) FILTER (WHERE status = 'win' OR status = 'served') AS impressions,
           COUNT(*) FILTER (WHERE status IN ('win', 'served', 'no_fill', 'bid')) AS fills,
           COALESCE(SUM(CASE WHEN bid_cpm IS NOT NULL THEN bid_cpm * 1000000 ELSE 0 END), 0)::BIGINT AS revenue_micros,
           PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY latency_ms) AS latency_p95_ms,
           PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_ms) AS latency_p50_ms,
           ROUND(100.0 * COUNT(*) FILTER (WHERE status = 'error') / NULLIF(COUNT(*), 0), 2) AS error_rate_percent
         FROM migration_shadow_outcomes
         WHERE experiment_id = $1
           AND created_at >= NOW() - ($2::integer || ' minutes')::interval
         GROUP BY arm`,
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
        const latencyP95 = row.latency_p95_ms ? Math.round(Number(row.latency_p95_ms)) : null;
        const latencyP50 = row.latency_p50_ms ? Math.round(Number(row.latency_p50_ms)) : null;
        const errorRate = row.error_rate_percent !== null ? Number(row.error_rate_percent) : null;

        await this.pool.query(
          `INSERT INTO migration_guardrail_snapshots (
             experiment_id, arm, impressions, fills, revenue_micros,
             latency_p95_ms, latency_p50_ms, error_rate_percent, rolling_window_minutes
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
          [
            experimentId,
            arm,
            impressions,
            fills,
            revenueMicros,
            latencyP95,
            latencyP50,
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
