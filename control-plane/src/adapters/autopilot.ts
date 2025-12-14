/**
 * AUTOPILOT ADAPTER
 * 
 * Reads Autopilot health, events, and job ledger from the database.
 * Monitors automation integrity for:
 * - Cron job health
 * - Background job status
 * - Migration Studio throughput
 * - VRA experiment status
 */

import { queryReadOnly } from '../database';
import { logger } from '../logger';

export interface AutopilotJob {
  id: string;
  name: string;
  schedule: string;
  last_run: Date | null;
  last_success: Date | null;
  last_error: string | null;
  run_count: number;
  error_count: number;
  avg_duration_ms: number;
  is_stale: boolean;
}

export interface AutopilotEvent {
  id: string;
  job_name: string;
  event_type: 'started' | 'completed' | 'failed';
  timestamp: Date;
  duration_ms: number | null;
  error: string | null;
  metadata: Record<string, unknown>;
}

export interface AutopilotHealth {
  overall_status: 'OK' | 'DEGRADED' | 'FAIL';
  last_health_check: Date;
  stale_jobs: string[];
  failing_jobs: string[];
  warnings: string[];
}

/**
 * Get Autopilot job status from background_jobs table
 */
export async function getAutopilotJobs(): Promise<AutopilotJob[]> {
  try {
    const sql = `
      SELECT 
        id::text,
        name,
        cron_schedule as schedule,
        last_run_at as last_run,
        last_success_at as last_success,
        last_error,
        run_count,
        error_count,
        COALESCE(avg_duration_ms, 0) as avg_duration_ms,
        CASE 
          WHEN last_run_at IS NULL THEN true
          WHEN last_run_at < NOW() - INTERVAL '2 hours' THEN true
          ELSE false
        END as is_stale
      FROM background_jobs
      ORDER BY last_run_at DESC NULLS LAST
    `;
    return await queryReadOnly<AutopilotJob>(sql);
  } catch (err) {
    logger.warn('background_jobs table not available', { error: (err as Error).message });
    return [];
  }
}

/**
 * Get recent Autopilot events from job_events table
 */
export async function getAutopilotEvents(limit = 100): Promise<AutopilotEvent[]> {
  try {
    const sql = `
      SELECT 
        id::text,
        job_name,
        event_type,
        created_at as timestamp,
        duration_ms,
        error,
        COALESCE(metadata, '{}')::jsonb as metadata
      FROM job_events
      ORDER BY created_at DESC
      LIMIT $1
    `;
    return await queryReadOnly<AutopilotEvent>(sql, [limit]);
  } catch (err) {
    logger.warn('job_events table not available', { error: (err as Error).message });
    return [];
  }
}

/**
 * Calculate Autopilot health status
 */
export async function getAutopilotHealth(): Promise<AutopilotHealth> {
  const jobs = await getAutopilotJobs();
  
  const staleJobs = jobs.filter(j => j.is_stale).map(j => j.name);
  const failingJobs = jobs.filter(j => j.last_error && !j.last_success).map(j => j.name);
  
  const warnings: string[] = [];
  
  // Check for high error rates
  for (const job of jobs) {
    if (job.run_count > 0) {
      const errorRate = job.error_count / job.run_count;
      if (errorRate > 0.1) {
        warnings.push(`${job.name} has ${(errorRate * 100).toFixed(1)}% error rate`);
      }
    }
  }
  
  let overall_status: 'OK' | 'DEGRADED' | 'FAIL' = 'OK';
  
  if (failingJobs.length > 0) {
    overall_status = 'FAIL';
  } else if (staleJobs.length > 0 || warnings.length > 0) {
    overall_status = 'DEGRADED';
  }
  
  return {
    overall_status,
    last_health_check: new Date(),
    stale_jobs: staleJobs,
    failing_jobs: failingJobs,
    warnings,
  };
}

/**
 * Get Migration Studio metrics
 */
export async function getMigrationStudioMetrics(): Promise<{
  active_migrations: number;
  completed_today: number;
  failed_today: number;
  avg_throughput_per_hour: number;
  mismatch_rate: number;
}> {
  try {
    const sql = `
      SELECT 
        COUNT(*) FILTER (WHERE status = 'running') as active_migrations,
        COUNT(*) FILTER (WHERE status = 'completed' AND completed_at >= CURRENT_DATE) as completed_today,
        COUNT(*) FILTER (WHERE status = 'failed' AND completed_at >= CURRENT_DATE) as failed_today,
        COALESCE(AVG(records_processed / NULLIF(EXTRACT(EPOCH FROM (completed_at - started_at)) / 3600, 0)), 0) as avg_throughput_per_hour,
        COALESCE(SUM(mismatch_count)::float / NULLIF(SUM(records_processed), 0), 0) as mismatch_rate
      FROM migration_jobs
      WHERE started_at >= NOW() - INTERVAL '24 hours'
    `;
    const rows = await queryReadOnly<any>(sql);
    return rows[0] || {
      active_migrations: 0,
      completed_today: 0,
      failed_today: 0,
      avg_throughput_per_hour: 0,
      mismatch_rate: 0,
    };
  } catch (err) {
    logger.warn('migration_jobs table not available', { error: (err as Error).message });
    return {
      active_migrations: 0,
      completed_today: 0,
      failed_today: 0,
      avg_throughput_per_hour: 0,
      mismatch_rate: 0,
    };
  }
}

/**
 * Get VRA experiment status
 */
export async function getVRAExperimentStatus(): Promise<{
  active_experiments: number;
  experiments: Array<{
    id: string;
    name: string;
    status: string;
    variant_count: number;
    impressions_today: number;
    created_at: Date;
  }>;
}> {
  try {
    const sql = `
      SELECT 
        e.id::text,
        e.name,
        e.status,
        COUNT(DISTINCT v.id) as variant_count,
        COALESCE(SUM(i.count) FILTER (WHERE i.date = CURRENT_DATE), 0) as impressions_today,
        e.created_at
      FROM vra_experiments e
      LEFT JOIN vra_variants v ON v.experiment_id = e.id
      LEFT JOIN vra_impressions_daily i ON i.experiment_id = e.id
      GROUP BY e.id, e.name, e.status, e.created_at
      ORDER BY e.created_at DESC
      LIMIT 20
    `;
    const experiments = await queryReadOnly<any>(sql);
    return {
      active_experiments: experiments.filter((e: any) => e.status === 'active').length,
      experiments,
    };
  } catch (err) {
    logger.warn('VRA tables not available', { error: (err as Error).message });
    return { active_experiments: 0, experiments: [] };
  }
}
