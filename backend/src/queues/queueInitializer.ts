/**
 * Queue Initializer
 * 
 * Initializes all job queues and registers workers
 */

import { queueManager, QueueName } from './queueManager';
import { processAnalyticsAggregation } from './processors/analyticsAggregation';
import { processAnalyticsIngest } from './processors/analyticsIngest';
import { processDataExport } from './processors/dataExport';
import { processPrivacyJob } from './processors/privacy';
import logger from '../utils/logger';
import { query } from '../utils/postgres';
import { redis } from '../utils/redis';

/**
 * Process report generation jobs
 */
async function processReportGeneration(job: any): Promise<void> {
  logger.info('Processing report generation', { jobId: job.id, data: job.data });
  
  const { publisherId, reportType, startDate, endDate } = job.data || {};
  
  try {
    // Generate report based on type
    switch (reportType) {
      case 'revenue':
        await generateRevenueReport(publisherId, startDate, endDate);
        break;
      case 'performance':
        await generatePerformanceReport(publisherId, startDate, endDate);
        break;
      case 'monthly':
        await generateMonthlyReport(publisherId, startDate);
        break;
      default:
        logger.warn('Unknown report type', { reportType });
    }
    
    logger.info('Report generation completed', { jobId: job.id, reportType });
  } catch (error) {
    logger.error('Report generation failed', { error, jobId: job.id });
    throw error;
  }
}

async function generateRevenueReport(publisherId: string, startDate: string, endDate: string): Promise<void> {
  const sql = `
    SELECT 
      DATE(event_date) as date,
      SUM(revenue) as total_revenue,
      SUM(impressions) as total_impressions,
      COUNT(DISTINCT placement_id) as active_placements
    FROM revenue_events
    WHERE publisher_id = $1 
      AND event_date >= $2 
      AND event_date <= $3
    GROUP BY DATE(event_date)
    ORDER BY date DESC
  `;
  
  const result = await query(sql, [publisherId, startDate, endDate]);
  logger.info('Revenue report generated', { rows: result.rowCount, publisherId });
}

async function generatePerformanceReport(publisherId: string, startDate: string, endDate: string): Promise<void> {
  const sql = `
    SELECT 
      placement_id,
      COUNT(*) as impression_count,
      SUM(revenue) as total_revenue,
      AVG(revenue) as avg_revenue
    FROM revenue_events
    WHERE publisher_id = $1 
      AND event_date >= $2 
      AND event_date <= $3
    GROUP BY placement_id
    ORDER BY total_revenue DESC
    LIMIT 100
  `;
  
  const result = await query(sql, [publisherId, startDate, endDate]);
  logger.info('Performance report generated', { rows: result.rowCount, publisherId });
}

async function generateMonthlyReport(publisherId: string, monthStart: string): Promise<void> {
  const sql = `
    SELECT 
      DATE_TRUNC('month', event_date) as month,
      SUM(revenue) as monthly_revenue,
      SUM(impressions) as monthly_impressions,
      AVG(revenue / NULLIF(impressions, 0) * 1000) as avg_ecpm
    FROM revenue_events
    WHERE publisher_id = $1 
      AND event_date >= DATE_TRUNC('month', $2::date)
      AND event_date < DATE_TRUNC('month', $2::date) + INTERVAL '1 month'
    GROUP BY DATE_TRUNC('month', event_date)
  `;
  
  const result = await query(sql, [publisherId, monthStart]);
  logger.info('Monthly report generated', { rows: result.rowCount, publisherId });
}

/**
 * Process metrics calculation jobs
 */
async function processMetricsCalculation(job: any): Promise<void> {
  logger.info('Processing metrics calculation', { jobId: job.id, data: job.data });
  
  const { publisherId, metricType, period } = job.data || {};
  
  try {
    switch (metricType) {
      case 'performance':
        await calculatePerformanceMetrics(publisherId, period);
        break;
      case 'quality':
        await calculateQualityMetrics(publisherId, period);
        break;
      case 'aggregates':
        await calculateAggregateMetrics(publisherId, period);
        break;
      default:
        logger.warn('Unknown metric type', { metricType });
    }
    
    logger.info('Metrics calculation completed', { jobId: job.id, metricType });
  } catch (error) {
    logger.error('Metrics calculation failed', { error, jobId: job.id });
    throw error;
  }
}

async function calculatePerformanceMetrics(publisherId: string, period: string): Promise<void> {
  const sql = `
    INSERT INTO metrics_cache (publisher_id, metric_type, metric_value, period, calculated_at)
    SELECT 
      $1 as publisher_id,
      'performance' as metric_type,
      jsonb_build_object(
        'revenue', SUM(revenue),
        'impressions', SUM(impressions),
        'ecpm', AVG(revenue / NULLIF(impressions, 0) * 1000)
      ) as metric_value,
      $2 as period,
      NOW() as calculated_at
    FROM revenue_events
    WHERE publisher_id = $1 
      AND event_date >= NOW() - $2::interval
    ON CONFLICT (publisher_id, metric_type, period) 
    DO UPDATE SET metric_value = EXCLUDED.metric_value, calculated_at = NOW()
  `;
  
  await query(sql, [publisherId, period]);
  logger.info('Performance metrics calculated', { publisherId, period });
}

async function calculateQualityMetrics(publisherId: string, period: string): Promise<void> {
  const sql = `
    SELECT 
      COUNT(DISTINCT placement_id) as active_placements,
      COUNT(*) as total_events,
      SUM(CASE WHEN revenue > 0 THEN 1 ELSE 0 END) as monetized_events
    FROM revenue_events
    WHERE publisher_id = $1 
      AND event_date >= NOW() - $2::interval
  `;
  
  const result = await query(sql, [publisherId, period]);
  logger.info('Quality metrics calculated', { publisherId, metrics: result.rows[0] });
}

async function calculateAggregateMetrics(publisherId: string, period: string): Promise<void> {
  // Calculate daily aggregates for faster dashboard loading
  const sql = `
    INSERT INTO daily_aggregates (publisher_id, date, revenue, impressions, clicks)
    SELECT 
      publisher_id,
      DATE(event_date) as date,
      SUM(revenue) as revenue,
      SUM(impressions) as impressions,
      SUM(clicks) as clicks
    FROM revenue_events
    WHERE publisher_id = $1 
      AND event_date >= NOW() - $2::interval
    GROUP BY publisher_id, DATE(event_date)
    ON CONFLICT (publisher_id, date) 
    DO UPDATE SET 
      revenue = EXCLUDED.revenue,
      impressions = EXCLUDED.impressions,
      clicks = EXCLUDED.clicks
  `;
  
  await query(sql, [publisherId, period]);
  logger.info('Aggregate metrics calculated', { publisherId, period });
}

/**
 * Process cache warming jobs
 */
async function processCacheWarming(job: any): Promise<void> {
  logger.info('Processing cache warming', { jobId: job.id, data: job.data });
  
  const { cacheType, keys } = job.data || {};
  
  try {
    if (!redis.isReady()) {
      logger.warn('Redis not ready, skipping cache warming');
      return;
    }
    
    switch (cacheType) {
      case 'dashboard':
        await warmDashboardCache(keys);
        break;
      case 'placements':
        await warmPlacementsCache(keys);
        break;
      case 'revenue':
        await warmRevenueCache(keys);
        break;
      default:
        logger.warn('Unknown cache type', { cacheType });
    }
    
    logger.info('Cache warming completed', { jobId: job.id, cacheType });
  } catch (error) {
    logger.error('Cache warming failed', { error, jobId: job.id });
    throw error;
  }
}

async function warmDashboardCache(publisherIds: string[]): Promise<void> {
  for (const publisherId of publisherIds || []) {
    const sql = `
      SELECT 
        SUM(revenue) as total_revenue,
        SUM(impressions) as total_impressions,
        SUM(clicks) as total_clicks
      FROM revenue_events
      WHERE publisher_id = $1 
        AND event_date >= NOW() - INTERVAL '30 days'
    `;
    
    const result = await query(sql, [publisherId]);
    const cacheKey = `dashboard:overview:${publisherId}`;
    await (redis as any).set(cacheKey, JSON.stringify(result.rows[0]), 'EX', 3600);
  }
  
  logger.info('Dashboard cache warmed', { count: publisherIds?.length || 0 });
}

async function warmPlacementsCache(publisherIds: string[]): Promise<void> {
  for (const publisherId of publisherIds || []) {
    const sql = `SELECT id, name, type, status FROM placements WHERE publisher_id = $1`;
    const result = await query(sql, [publisherId]);
    const cacheKey = `placements:list:${publisherId}`;
    await (redis as any).set(cacheKey, JSON.stringify(result.rows), 'EX', 1800);
  }
  
  logger.info('Placements cache warmed', { count: publisherIds?.length || 0 });
}

async function warmRevenueCache(publisherIds: string[]): Promise<void> {
  for (const publisherId of publisherIds || []) {
    const sql = `
      SELECT 
        DATE(event_date) as date,
        SUM(revenue) as revenue
      FROM revenue_events
      WHERE publisher_id = $1 
        AND event_date >= NOW() - INTERVAL '7 days'
      GROUP BY DATE(event_date)
      ORDER BY date DESC
    `;
    
    const result = await query(sql, [publisherId]);
    const cacheKey = `revenue:weekly:${publisherId}`;
    await (redis as any).set(cacheKey, JSON.stringify(result.rows), 'EX', 1800);
  }
  
  logger.info('Revenue cache warmed', { count: publisherIds?.length || 0 });
}

/**
 * Process cleanup tasks
 */
async function processCleanupTasks(job: any): Promise<void> {
  logger.info('Processing cleanup tasks', { jobId: job.id, data: job.data });
  
  const { type, olderThan } = job.data || {};
  
  try {
    switch (type) {
      case 'old_logs':
        await cleanupOldLogs(olderThan);
        break;
      case 'expired_jobs':
        await cleanupExpiredJobs(olderThan);
        break;
      case 'stale_cache':
        await cleanupStaleCache();
        break;
      default:
        logger.warn('Unknown cleanup type', { type });
    }
    
    logger.info('Cleanup completed', { jobId: job.id, type });
  } catch (error) {
    logger.error('Cleanup failed', { error, jobId: job.id });
    throw error;
  }
}

async function cleanupOldLogs(olderThan: string): Promise<void> {
  const sql = `
    DELETE FROM system_logs
    WHERE created_at < NOW() - $1::interval
  `;
  
  const result = await query(sql, [olderThan]);
  logger.info('Old logs cleaned up', { deletedCount: result.rowCount });
}

async function cleanupExpiredJobs(olderThan: string): Promise<void> {
  if (!redis.isReady()) {
    logger.warn('Redis not ready, skipping job cleanup');
    return;
  }
  
  // Clean up completed jobs older than specified period
  const cutoffTime = Date.now() - parseDuration(olderThan);
  
  for (const queueName of Object.values(QueueName)) {
    const listKey = `queue:${queueName}:completed`;
    const jobIds = await (redis as any).lRange(listKey, 0, -1);
    
    let cleanedCount = 0;
    for (const jobId of jobIds) {
      const jobKey = `job:${queueName}:${jobId}`;
      const jobData = await (redis as any).get(jobKey);
      
      if (jobData) {
        try {
          const job = JSON.parse(jobData);
          if (job.finishedOn < cutoffTime) {
            await (redis as any).lRem(listKey, 0, jobId);
            await (redis as any).del(jobKey);
            cleanedCount++;
          }
        } catch {
          // Invalid job data, remove it
          await (redis as any).lRem(listKey, 0, jobId);
          await (redis as any).del(jobKey);
          cleanedCount++;
        }
      }
    }
    
    logger.info('Expired jobs cleaned up', { queueName, cleanedCount });
  }
}

async function cleanupStaleCache(): Promise<void> {
  if (!redis.isReady()) {
    logger.warn('Redis not ready, skipping cache cleanup');
    return;
  }
  
  // Redis automatically expires keys with TTL, but we can cleanup orphaned keys
  logger.info('Stale cache cleanup completed (handled by Redis TTL)');
}

function parseDuration(duration: string): number {
  const match = duration.match(/^(\d+)([dhm])$/);
  if (!match) return 86400000; // Default 1 day in ms
  
  const value = parseInt(match[1], 10);
  const unit = match[2];
  
  switch (unit) {
    case 'd': return value * 86400000;
    case 'h': return value * 3600000;
    case 'm': return value * 60000;
    default: return 86400000;
  }
}

/**
 * Initialize all queues and workers
 */
export async function initializeQueues(): Promise<void> {
  try {
    logger.info('Initializing job queues...');

    // Initialize queue manager
    await queueManager.initialize();

    // Register workers
    registerWorkers();

    // Schedule recurring jobs
    scheduleRecurringJobs();

    logger.info('Job queues initialized successfully');
  } catch (error) {
    logger.error('Failed to initialize job queues', { error });
    throw error;
  }
}

/**
 * Register all workers
 */
function registerWorkers(): void {
  // Analytics Ingest Worker (batch writes through Postgres staging tables)
  queueManager.registerWorker(
    QueueName.ANALYTICS_INGEST,
    processAnalyticsIngest,
    {
      concurrency: 10,
      limiter: { max: 1000, duration: 1000 }, // up to 1000 jobs/sec across workers
    }
  );

  // Analytics Aggregation Worker
  queueManager.registerWorker(
    QueueName.ANALYTICS_AGGREGATION,
    processAnalyticsAggregation,
    {
      concurrency: 3, // Process up to 3 aggregation jobs concurrently
      limiter: {
        max: 10, // Max 10 jobs
        duration: 60000, // per minute
      },
    }
  );

  // Data Export Worker
  queueManager.registerWorker(
    QueueName.DATA_EXPORT,
    processDataExport,
    {
      concurrency: 2, // Process up to 2 export jobs concurrently
      limiter: {
        max: 20, // Max 20 jobs
        duration: 60000, // per minute
      },
    }
  );

  // Privacy (GDPR/CCPA) Worker
  queueManager.registerWorker(
    QueueName.PRIVACY,
    processPrivacyJob,
    {
      concurrency: 2,
      limiter: {
        max: 10,
        duration: 60000,
      },
    }
  );

  // Report Generation Worker
  if (typeof queueManager.registerWorker === 'function') {
    queueManager.registerWorker(
      QueueName.REPORT_GENERATION,
      processReportGeneration,
      {
        concurrency: 2,
      }
    );
  }

  // Metrics Calculation Worker
  if (typeof queueManager.registerWorker === 'function') {
    queueManager.registerWorker(
      QueueName.METRICS_CALCULATION,
      processMetricsCalculation,
      {
        concurrency: 5,
      }
    );
  }

  // Cache Warming Worker
  if (typeof queueManager.registerWorker === 'function') {
    queueManager.registerWorker(
      QueueName.CACHE_WARMING,
      processCacheWarming,
      {
        concurrency: 3,
      }
    );
  }

  // Cleanup Worker
  if (typeof queueManager.registerWorker === 'function') {
    queueManager.registerWorker(
      QueueName.CLEANUP_TASKS,
      processCleanupTasks,
      {
        concurrency: 1, // Run cleanup jobs sequentially
      }
    );
  }

  logger.info('All workers registered');
}

/**
 * Schedule recurring jobs
 */
function scheduleRecurringJobs(): void {
  // Hourly metrics calculation
  queueManager.scheduleRecurringJob(
    QueueName.METRICS_CALCULATION,
    'hourly-metrics',
    {
      publisherId: 'all',
      metricType: 'performance',
      period: 'last_hour',
    },
    '0 * * * *' // Every hour at minute 0
  );

  // Daily cleanup at 2 AM
  queueManager.scheduleRecurringJob(
    QueueName.CLEANUP_TASKS,
    'daily-cleanup',
    {
      type: 'old_logs',
      olderThan: '30d',
    },
    '0 2 * * *' // Every day at 2 AM
  );

  // Monthly report generation on the 1st at 8 AM
  queueManager.scheduleRecurringJob(
    QueueName.REPORT_GENERATION,
    'monthly-reports',
    {
      publisherId: 'all',
      reportType: 'monthly',
      date: 'last_month',
    },
    '0 8 1 * *' // 1st of every month at 8 AM
  );

  // Cache warming every 5 minutes for popular queries
  queueManager.scheduleRecurringJob(
    QueueName.CACHE_WARMING,
    'popular-cache-warming',
    {
      cacheKeys: ['analytics:popular', 'dashboard:stats'],
      priority: 10,
    },
    '*/5 * * * *' // Every 5 minutes
  );

  logger.info('Recurring jobs scheduled');
}

/**
 * Shutdown all queues
 */
export async function shutdownQueues(): Promise<void> {
  logger.info('Shutting down job queues...');
  await queueManager.shutdown();
  logger.info('Job queues shut down');
}

/**
 * Export queue manager instance
 */
export { queueManager };
