/**
 * Data Export Jobs
 * 
 * Background jobs for exporting analytics data
 */

import { Job } from 'bullmq';
import { createWriteStream } from 'fs';
import { join } from 'path';
import { format as formatCsv } from '@fast-csv/format';
import logger from '../../utils/logger';
import pool, { query } from '../../utils/postgres';
import { DataExportJob } from '../queueManager';

type AnalyticsExportRow = {
  date: string;
  publisher_id: string;
  app_id: string;
  ad_unit_id: string;
  adapter_id: string;
  country: string;
  impressions: number;
  filled_impressions: number;
  clicks: number;
  revenue: number;
  ctr: number;
  fill_rate: number;
  ecpm: number;
};

async function tableExists(tableName: string): Promise<boolean> {
  const sql = `
    SELECT EXISTS(
      SELECT 1
      FROM information_schema.tables
      WHERE table_schema = 'public'
        AND table_name = $1
    ) AS exists
  `;

  const { rows } = await query<{ exists: boolean }>(sql, [tableName], { label: 'data_export_schema_check', replica: true });
  return Boolean(rows?.[0]?.exists);
}

/**
 * Process data export job
 */
export async function processDataExport(job: Job<DataExportJob>): Promise<void> {
  const { jobId, publisherId, format, startDate, endDate, filters } = job.data;

  logger.info('Processing data export', {
    jobId: job.id,
    exportJobId: jobId,
    publisherId,
    format,
    startDate,
    endDate,
  });

  try {
    // Update job status to processing
    await updateJobStatus(jobId, 'processing');
    await job.updateProgress(10);

    // Fetch data from Postgres rollups
    const data = await fetchExportData(publisherId, startDate, endDate, filters);
    await job.updateProgress(50);

    // Generate file
    const filePath = await generateExportFile(jobId, data, format);
    await job.updateProgress(80);

    // Update job with file info
    await updateJobWithFile(jobId, filePath);
    await job.updateProgress(100);

    logger.info('Data export completed', {
      jobId: job.id,
      exportJobId: jobId,
      filePath,
    });
  } catch (error) {
    logger.error('Data export failed', {
      jobId: job.id,
      exportJobId: jobId,
      error,
    });

    await updateJobStatus(jobId, 'failed', error instanceof Error ? error.message : 'Unknown error');
    throw error;
  }
}

/**
 * Fetch data for export
 */
async function fetchExportData(
  publisherId: string,
  startDate: string,
  endDate: string,
  _filters?: Record<string, unknown>
): Promise<AnalyticsExportRow[]> {
  const hasRollups = await tableExists('analytics_metrics_rollups');

  const sql = hasRollups
    ? `
      SELECT
        bucket_date::text AS date,
        publisher_id,
        NULLIF(app_id, '') AS app_id,
        NULLIF(ad_unit_id, '') AS ad_unit_id,
        NULLIF(adapter_id, '') AS adapter_id,
        country_code AS country,
        impression_count::double precision AS impressions,
        filled_count::double precision AS filled_impressions,
        click_count::double precision AS clicks,
        total_revenue::double precision AS revenue,
        ctr::double precision AS ctr,
        fill_rate::double precision AS fill_rate,
        ecpm::double precision AS ecpm
      FROM analytics_metrics_rollups
      WHERE publisher_id = $1
        AND bucket_date >= $2::date
        AND bucket_date < $3::date
      ORDER BY bucket_date DESC, impression_count DESC
    `
    : `
      WITH imp AS (
        SELECT
          (i.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
          i.publisher_id,
          COALESCE(i.app_id::text, '') AS app_id,
          COALESCE(NULLIF(i.ad_unit_id::text, ''), NULLIF(i.meta->>'ad_unit_id', ''), COALESCE(i.placement_id::text, '')) AS ad_unit_id,
          COALESCE(i.adapter_id::text, '') AS adapter_id,
          LEFT(COALESCE(NULLIF(i.country_code::text, ''), NULLIF(i.meta->>'country_code', ''), 'ZZ'), 2)::char(2) AS country_code,
          COUNT(*)::double precision AS impressions,
          COUNT(*) FILTER (WHERE i.filled IS TRUE)::double precision AS filled_impressions
        FROM analytics_impressions i
        WHERE i.publisher_id = $1
          AND i.is_test_mode = false
          AND i.observed_at >= $2::date
          AND i.observed_at < $3::date
        GROUP BY 1,2,3,4,5,6
      ),
      rev AS (
        SELECT
          (r.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
          r.publisher_id,
          COALESCE(r.app_id::text, '') AS app_id,
          COALESCE(NULLIF(r.metadata->>'ad_unit_id', ''), COALESCE(r.placement_id::text, '')) AS ad_unit_id,
          COALESCE(r.adapter_id::text, '') AS adapter_id,
          LEFT(COALESCE(NULLIF(r.country_code::text, ''), 'ZZ'), 2)::char(2) AS country_code,
          COUNT(*) FILTER (WHERE r.revenue_type = 'click')::double precision AS clicks,
          COALESCE(SUM(r.revenue_usd), 0)::double precision AS revenue
        FROM analytics_revenue_events r
        WHERE r.publisher_id = $1
          AND r.is_test_mode = false
          AND r.observed_at >= $2::date
          AND r.observed_at < $3::date
        GROUP BY 1,2,3,4,5,6
      ),
      dims AS (
        SELECT bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code FROM imp
        UNION
        SELECT bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code FROM rev
      )
      SELECT
        d.bucket_date::text AS date,
        d.publisher_id,
        NULLIF(d.app_id, '') AS app_id,
        NULLIF(d.ad_unit_id, '') AS ad_unit_id,
        NULLIF(d.adapter_id, '') AS adapter_id,
        d.country_code AS country,
        COALESCE(imp.impressions, 0) AS impressions,
        COALESCE(imp.filled_impressions, 0) AS filled_impressions,
        COALESCE(rev.clicks, 0) AS clicks,
        COALESCE(rev.revenue, 0) AS revenue,
        CASE WHEN COALESCE(imp.impressions, 0) = 0 THEN 0
             ELSE (COALESCE(rev.clicks, 0) / imp.impressions) * 100
        END AS ctr,
        CASE WHEN COALESCE(imp.impressions, 0) = 0 THEN 0
             ELSE (COALESCE(imp.filled_impressions, 0) / imp.impressions) * 100
        END AS fill_rate,
        CASE WHEN COALESCE(imp.impressions, 0) = 0 THEN 0
             ELSE (COALESCE(rev.revenue, 0) / imp.impressions) * 1000
        END AS ecpm
      FROM dims d
      LEFT JOIN imp ON imp.bucket_date = d.bucket_date
        AND imp.publisher_id = d.publisher_id
        AND imp.app_id = d.app_id
        AND imp.ad_unit_id = d.ad_unit_id
        AND imp.adapter_id = d.adapter_id
        AND imp.country_code = d.country_code
      LEFT JOIN rev ON rev.bucket_date = d.bucket_date
        AND rev.publisher_id = d.publisher_id
        AND rev.app_id = d.app_id
        AND rev.ad_unit_id = d.ad_unit_id
        AND rev.adapter_id = d.adapter_id
        AND rev.country_code = d.country_code
      ORDER BY d.bucket_date DESC, COALESCE(imp.impressions, 0) DESC
    `;

  const { rows } = await query<AnalyticsExportRow>(sql, [publisherId, startDate, endDate], {
    label: hasRollups ? 'analytics_export_rollup' : 'analytics_export_fallback',
    replica: true,
  });

  logger.debug('Export data fetched', {
    publisherId,
    rowCount: rows.length,
  });

  return rows;
}

/**
 * Generate export file
 */
async function generateExportFile(
  jobId: string,
  data: AnalyticsExportRow[],
  format: 'csv' | 'json'
): Promise<string> {
  const exportDir = process.env.EXPORT_DIR || join(process.cwd(), 'exports');
  const fileName = `export_${jobId}.${format}`;
  const filePath = join(exportDir, fileName);

  if (format === 'csv') {
    await generateCsvFile(filePath, data);
  } else {
    await generateJsonFile(filePath, data);
  }

  logger.debug('Export file generated', { filePath });
  return filePath;
}

/**
 * Generate CSV file
 */
async function generateCsvFile(filePath: string, data: AnalyticsExportRow[]): Promise<void> {
  return new Promise((resolve, reject) => {
    const writeStream = createWriteStream(filePath);
    const csvStream = formatCsv({ headers: true });

    csvStream.pipe(writeStream);

    data.forEach((row) => csvStream.write(row));
    csvStream.end();

    writeStream.on('finish', resolve);
    writeStream.on('error', reject);
  });
}

/**
 * Generate JSON file
 */
async function generateJsonFile(filePath: string, data: AnalyticsExportRow[]): Promise<void> {
  return new Promise((resolve, reject) => {
    const writeStream = createWriteStream(filePath);

    writeStream.write(JSON.stringify(data, null, 2));
    writeStream.end();

    writeStream.on('finish', resolve);
    writeStream.on('error', reject);
  });
}

/**
 * Update job status in database
 */
async function updateJobStatus(
  jobId: string,
  status: string,
  error?: string
): Promise<void> {
  const query = `
    UPDATE data_export_jobs
    SET status = $1, error = $2, updated_at = NOW()
    WHERE id = $3
  `;

  await pool.query(query, [status, error || null, jobId]);

  logger.debug('Job status updated', { jobId, status });
}

/**
 * Update job with file information
 */
async function updateJobWithFile(jobId: string, filePath: string): Promise<void> {
  const query = `
    UPDATE data_export_jobs
    SET 
      status = 'completed',
      file_path = $1,
      completed_at = NOW(),
      updated_at = NOW()
    WHERE id = $2
  `;

  await pool.query(query, [filePath, jobId]);

  logger.debug('Job completed with file', { jobId, filePath });
}
