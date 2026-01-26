/**
 * DATA RETENTION CLEANUP JOB
 * 
 * Enforces AUDIT_RETENTION_YEARS setting by deleting old records.
 * 
 * This script should be run via cron job on the Test Server:
 *   0 3 * * * cd /opt/adproject/control-plane && node dist/src/scripts/cleanupRetention.js
 * 
 * IMPORTANT: This is a destructive operation. Always test in staging first.
 * 
 * Tables cleaned:
 *   - sa_emails (outbound email records)
 *   - sa_activity_log (activity tracking)
 *   - sa_outbound_messages (Thompson sampler tracking)
 *   - billing_audit_trail (billing events)
 * 
 * @see docs/DATA_RETENTION_POLICIES.md
 */

import { queryOperator, queryReadOnly } from '../database.js';
import { logger } from '../logger.js';

// Default retention period in years (can be overridden by AUDIT_RETENTION_YEARS env)
const DEFAULT_RETENTION_YEARS = 7;

interface CleanupResult {
  table: string;
  deletedCount: number;
  error?: string;
}

interface CleanupSummary {
  startedAt: Date;
  completedAt: Date;
  retentionYears: number;
  cutoffDate: Date;
  results: CleanupResult[];
  totalDeleted: number;
  errors: string[];
}

/**
 * Get the retention period from environment
 */
function getRetentionYears(): number {
  const envValue = process.env.AUDIT_RETENTION_YEARS;
  if (envValue) {
    const parsed = parseInt(envValue, 10);
    if (!isNaN(parsed) && parsed > 0) {
      return parsed;
    }
  }
  return DEFAULT_RETENTION_YEARS;
}

/**
 * Calculate cutoff date for retention
 */
function getCutoffDate(retentionYears: number): Date {
  const cutoff = new Date();
  cutoff.setFullYear(cutoff.getFullYear() - retentionYears);
  return cutoff;
}

/**
 * Clean up a single table
 */
async function cleanupTable(
  tableName: string, 
  dateColumn: string, 
  cutoffDate: Date
): Promise<CleanupResult> {
  try {
    // First, count how many will be deleted (for logging)
    const countResult = await queryReadOnly<{ count: string }>(`
      SELECT COUNT(*) as count FROM ${tableName} 
      WHERE ${dateColumn} < $1
    `, [cutoffDate]);
    
    const toDelete = parseInt(countResult[0]?.count || '0');
    
    if (toDelete === 0) {
      logger.info(`[Retention] No records to delete from ${tableName}`);
      return { table: tableName, deletedCount: 0 };
    }
    
    logger.info(`[Retention] Deleting ${toDelete} records from ${tableName} older than ${cutoffDate.toISOString()}`);
    
    // Delete in batches to avoid long locks
    const BATCH_SIZE = 10000;
    let totalDeleted = 0;
    
    while (totalDeleted < toDelete) {
      const result = await queryOperator(`
        DELETE FROM ${tableName} 
        WHERE id IN (
          SELECT id FROM ${tableName} 
          WHERE ${dateColumn} < $1 
          LIMIT ${BATCH_SIZE}
        )
      `, [cutoffDate]);
      
      // PostgreSQL returns { rowCount } for DELETE
      const deleted = (result as unknown as { rowCount?: number }).rowCount || 0;
      totalDeleted += deleted;
      
      if (deleted === 0) break; // No more to delete
      
      logger.debug(`[Retention] Batch deleted ${deleted} from ${tableName}, total: ${totalDeleted}`);
      
      // Small delay between batches to reduce DB load
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    
    logger.info(`[Retention] Completed ${tableName}: deleted ${totalDeleted} records`);
    return { table: tableName, deletedCount: totalDeleted };
    
  } catch (err) {
    const errorMsg = (err as Error).message;
    logger.error(`[Retention] Failed to clean ${tableName}`, { error: errorMsg });
    return { table: tableName, deletedCount: 0, error: errorMsg };
  }
}

/**
 * Run the full retention cleanup
 */
export async function runRetentionCleanup(): Promise<CleanupSummary> {
  const startedAt = new Date();
  const retentionYears = getRetentionYears();
  const cutoffDate = getCutoffDate(retentionYears);
  
  logger.info('[Retention] Starting data retention cleanup', {
    retentionYears,
    cutoffDate: cutoffDate.toISOString(),
  });
  
  // Tables to clean with their date columns
  const tablesToClean: Array<{ table: string; dateColumn: string }> = [
    { table: 'sa_emails', dateColumn: 'created_at' },
    { table: 'sa_activity_log', dateColumn: 'created_at' },
    { table: 'sa_outbound_messages', dateColumn: 'created_at' },
    { table: 'sa_message_classifications', dateColumn: 'created_at' },
    { table: 'sa_scrape_jobs', dateColumn: 'created_at' },
    { table: 'billing_audit_trail', dateColumn: 'created_at' },
    { table: 'audit_trail', dateColumn: 'created_at' },
  ];
  
  const results: CleanupResult[] = [];
  
  for (const { table, dateColumn } of tablesToClean) {
    const result = await cleanupTable(table, dateColumn, cutoffDate);
    results.push(result);
  }
  
  const completedAt = new Date();
  const totalDeleted = results.reduce((sum, r) => sum + r.deletedCount, 0);
  const errors = results.filter(r => r.error).map(r => `${r.table}: ${r.error}`);
  
  const summary: CleanupSummary = {
    startedAt,
    completedAt,
    retentionYears,
    cutoffDate,
    results,
    totalDeleted,
    errors,
  };
  
  logger.info('[Retention] Cleanup completed', {
    durationMs: completedAt.getTime() - startedAt.getTime(),
    totalDeleted,
    errorCount: errors.length,
  });
  
  return summary;
}

// CLI execution (CommonJS compatible)
if (require.main === module) {
  runRetentionCleanup()
    .then(summary => {
      console.log('Retention cleanup completed:');
      console.log(`  Retention period: ${summary.retentionYears} years`);
      console.log(`  Cutoff date: ${summary.cutoffDate.toISOString()}`);
      console.log(`  Total deleted: ${summary.totalDeleted}`);
      
      if (summary.errors.length > 0) {
        console.log('  Errors:', summary.errors);
        process.exit(1);
      }
      
      process.exit(0);
    })
    .catch(err => {
      console.error('Retention cleanup failed:', err);
      process.exit(1);
    });
}
