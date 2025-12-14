/**
 * PARTITION MANAGER
 * 
 * Manages table partitions for time-series data.
 * - Creates future partitions ahead of time
 * - Drops old partitions per retention policy
 * - All operations are audited
 */

import { queryReadOnly, queryOperator } from '../database';
import { logger, logAudit } from '../logger';

export interface PartitionInfo {
  partition_name: string;
  parent_table: string;
  range_start: string;
  range_end: string;
  size_bytes: number;
  row_count_estimate: number;
}

export interface PartitionStatus {
  table_name: string;
  total_partitions: number;
  oldest_partition: string;
  newest_partition: string;
  months_ahead: number;
  total_size_gb: number;
  partitions: PartitionInfo[];
}

export interface RetentionPolicy {
  table_name: string;
  retention_days: number;
  min_partitions: number;
  description: string;
}

// Default retention policies
const RETENTION_POLICIES: RetentionPolicy[] = [
  { table_name: 'ad_events', retention_days: 90, min_partitions: 3, description: 'Ad impression/click events' },
  { table_name: 'bid_requests', retention_days: 30, min_partitions: 2, description: 'RTB bid request logs' },
  { table_name: 'bid_responses', retention_days: 30, min_partitions: 2, description: 'RTB bid response logs' },
  { table_name: 'sdk_logs', retention_days: 14, min_partitions: 1, description: 'SDK debug logs' },
  { table_name: 'audit_logs', retention_days: 2555, min_partitions: 12, description: '7-year financial audit' },
  { table_name: 'billing_events', retention_days: 2555, min_partitions: 12, description: '7-year billing records' },
  { table_name: 'api_requests', retention_days: 7, min_partitions: 1, description: 'API request logs' },
  { table_name: 'vra_impressions_daily', retention_days: 365, min_partitions: 6, description: 'VRA daily aggregates' },
];

/**
 * Get all partitions for a table
 */
export async function getPartitions(tableName: string): Promise<PartitionInfo[]> {
  const rows = await queryReadOnly<{
    partition_name: string;
    parent_table: string;
    partition_expr: string;
    size_bytes: string;
    reltuples: string;
  }>(`
    SELECT 
      c.relname as partition_name,
      parent.relname as parent_table,
      pg_get_expr(c.relpartbound, c.oid) as partition_expr,
      pg_total_relation_size(c.oid)::text as size_bytes,
      c.reltuples::text as reltuples
    FROM pg_class c
    JOIN pg_inherits i ON c.oid = i.inhrelid
    JOIN pg_class parent ON i.inhparent = parent.oid
    WHERE parent.relname = $1
    ORDER BY c.relname
  `, [tableName]);
  
  return rows.map(r => {
    const rangeMatch = r.partition_expr?.match(/FROM \('([^']+)'\) TO \('([^']+)'\)/);
    return {
      partition_name: r.partition_name,
      parent_table: r.parent_table,
      range_start: rangeMatch?.[1] || 'unknown',
      range_end: rangeMatch?.[2] || 'unknown',
      size_bytes: parseInt(r.size_bytes, 10),
      row_count_estimate: parseFloat(r.reltuples),
    };
  });
}

/**
 * Get partition status for a table
 */
export async function getPartitionStatus(tableName: string): Promise<PartitionStatus> {
  const partitions = await getPartitions(tableName);
  
  if (partitions.length === 0) {
    return {
      table_name: tableName,
      total_partitions: 0,
      oldest_partition: 'none',
      newest_partition: 'none',
      months_ahead: 0,
      total_size_gb: 0,
      partitions: [],
    };
  }
  
  const sortedPartitions = partitions.sort((a, b) => 
    a.range_start.localeCompare(b.range_start)
  );
  
  const newestPartition = sortedPartitions[sortedPartitions.length - 1];
  const newestDate = new Date(newestPartition.range_end);
  const now = new Date();
  const monthsAhead = (newestDate.getFullYear() - now.getFullYear()) * 12 + 
                      (newestDate.getMonth() - now.getMonth());
  
  const totalSizeBytes = partitions.reduce((sum, p) => sum + p.size_bytes, 0);
  
  return {
    table_name: tableName,
    total_partitions: partitions.length,
    oldest_partition: sortedPartitions[0].partition_name,
    newest_partition: newestPartition.partition_name,
    months_ahead: Math.max(0, monthsAhead),
    total_size_gb: parseFloat((totalSizeBytes / (1024 * 1024 * 1024)).toFixed(2)),
    partitions: sortedPartitions,
  };
}

/**
 * Get all partitioned tables and their status
 */
export async function getAllPartitionStatus(): Promise<PartitionStatus[]> {
  const tables = await queryReadOnly<{ table_name: string }>(`
    SELECT DISTINCT parent.relname as table_name
    FROM pg_class c
    JOIN pg_inherits i ON c.oid = i.inhrelid
    JOIN pg_class parent ON i.inhparent = parent.oid
    WHERE parent.relkind = 'p'
    ORDER BY parent.relname
  `);
  
  const statuses: PartitionStatus[] = [];
  for (const { table_name } of tables) {
    statuses.push(await getPartitionStatus(table_name));
  }
  
  return statuses;
}

/**
 * Create future partitions for a table
 */
export async function createFuturePartitions(
  tableName: string,
  monthsAhead: number,
  actor: string
): Promise<{ created: string[]; skipped: string[] }> {
  if (monthsAhead > 12) {
    throw new Error('Cannot create more than 12 months of partitions at once');
  }
  
  const created: string[] = [];
  const skipped: string[] = [];
  
  const now = new Date();
  
  for (let i = 1; i <= monthsAhead; i++) {
    const targetDate = new Date(now.getFullYear(), now.getMonth() + i, 1);
    const year = targetDate.getFullYear();
    const month = String(targetDate.getMonth() + 1).padStart(2, '0');
    
    const partitionName = `${tableName}_${year}_${month}`;
    const rangeStart = `${year}-${month}-01`;
    
    const nextMonth = new Date(year, targetDate.getMonth() + 1, 1);
    const rangeEnd = nextMonth.toISOString().split('T')[0];
    
    // Check if partition already exists
    const existing = await queryReadOnly<{ relname: string }>(`
      SELECT relname FROM pg_class WHERE relname = $1
    `, [partitionName]);
    
    if (existing.length > 0) {
      skipped.push(partitionName);
      continue;
    }
    
    // Create partition
    await queryOperator(
      `CREATE TABLE IF NOT EXISTS ${partitionName} 
       PARTITION OF ${tableName}
       FOR VALUES FROM ('${rangeStart}') TO ('${rangeEnd}')`,
      [],
      actor,
      `Creating partition ${partitionName} for ${tableName}`
    );
    
    created.push(partitionName);
    logger.info(`Created partition ${partitionName}`, { actor, tableName });
  }
  
  logAudit({
    actor,
    action: 'create_partitions',
    target: tableName,
    preState: { monthsAhead },
    postState: { created, skipped },
    result: 'success',
  });
  
  return { created, skipped };
}

/**
 * Drop old partitions based on retention policy
 */
export async function dropOldPartitions(
  tableName: string,
  retentionDays: number,
  actor: string,
  dryRun: boolean = true
): Promise<{ toDrop: string[]; dropped: string[] }> {
  const policy = RETENTION_POLICIES.find(p => p.table_name === tableName);
  const minPartitions = policy?.min_partitions || 2;
  
  const partitions = await getPartitions(tableName);
  if (partitions.length <= minPartitions) {
    logger.info(`Skipping drop for ${tableName}: only ${partitions.length} partitions (min: ${minPartitions})`);
    return { toDrop: [], dropped: [] };
  }
  
  const cutoffDate = new Date();
  cutoffDate.setDate(cutoffDate.getDate() - retentionDays);
  
  const toDrop: string[] = [];
  
  for (const partition of partitions) {
    const partitionEndDate = new Date(partition.range_end);
    if (partitionEndDate < cutoffDate) {
      toDrop.push(partition.partition_name);
    }
  }
  
  // Keep at least minPartitions
  const wouldRemain = partitions.length - toDrop.length;
  if (wouldRemain < minPartitions) {
    const toKeep = minPartitions - wouldRemain;
    toDrop.splice(0, toKeep);
  }
  
  if (dryRun) {
    logger.info(`Dry run: would drop ${toDrop.length} partitions from ${tableName}`, { toDrop });
    return { toDrop, dropped: [] };
  }
  
  const dropped: string[] = [];
  for (const partitionName of toDrop) {
    await queryOperator(
      `DROP TABLE IF EXISTS ${partitionName}`,
      [],
      actor,
      `Dropping old partition ${partitionName}`
    );
    dropped.push(partitionName);
    logger.info(`Dropped partition ${partitionName}`, { actor, tableName });
  }
  
  logAudit({
    actor,
    action: 'drop_partitions',
    target: tableName,
    preState: { retentionDays, dryRun },
    postState: { toDrop, dropped },
    result: 'success',
  });
  
  return { toDrop, dropped };
}

/**
 * Get retention policy for a table
 */
export function getRetentionPolicy(tableName: string): RetentionPolicy | undefined {
  return RETENTION_POLICIES.find(p => p.table_name === tableName);
}

/**
 * Get all retention policies
 */
export function getAllRetentionPolicies(): RetentionPolicy[] {
  return RETENTION_POLICIES;
}

/**
 * Check partition health and return issues
 */
export async function checkPartitionHealth(): Promise<{
  healthy: boolean;
  issues: Array<{
    table: string;
    issue: string;
    severity: 'warning' | 'critical';
  }>;
}> {
  const issues: Array<{ table: string; issue: string; severity: 'warning' | 'critical' }> = [];
  
  for (const policy of RETENTION_POLICIES) {
    try {
      const status = await getPartitionStatus(policy.table_name);
      
      // Check if we have enough future partitions
      if (status.months_ahead < 2) {
        issues.push({
          table: policy.table_name,
          issue: `Only ${status.months_ahead} months of future partitions`,
          severity: status.months_ahead < 1 ? 'critical' : 'warning',
        });
      }
      
      // Check if we have too many old partitions
      if (status.total_partitions > 0) {
        const oldestDate = new Date(status.partitions[0]?.range_start || Date.now());
        const daysOld = Math.floor((Date.now() - oldestDate.getTime()) / (1000 * 60 * 60 * 24));
        
        if (daysOld > policy.retention_days + 30) {
          issues.push({
            table: policy.table_name,
            issue: `Oldest partition is ${daysOld} days old (retention: ${policy.retention_days})`,
            severity: 'warning',
          });
        }
      }
      
    } catch (err) {
      issues.push({
        table: policy.table_name,
        issue: `Failed to check: ${(err as Error).message}`,
        severity: 'critical',
      });
    }
  }
  
  return {
    healthy: issues.filter(i => i.severity === 'critical').length === 0,
    issues,
  };
}

/**
 * Auto-maintain partitions (create future, report old)
 */
export async function autoMaintainPartitions(actor: string): Promise<{
  created: Record<string, string[]>;
  needsDropping: Record<string, string[]>;
}> {
  const created: Record<string, string[]> = {};
  const needsDropping: Record<string, string[]> = {};
  
  for (const policy of RETENTION_POLICIES) {
    try {
      // Create future partitions (3 months ahead)
      const createResult = await createFuturePartitions(policy.table_name, 3, actor);
      if (createResult.created.length > 0) {
        created[policy.table_name] = createResult.created;
      }
      
      // Check what needs dropping (dry run)
      const dropResult = await dropOldPartitions(policy.table_name, policy.retention_days, actor, true);
      if (dropResult.toDrop.length > 0) {
        needsDropping[policy.table_name] = dropResult.toDrop;
      }
    } catch (err) {
      logger.error(`Failed to maintain partitions for ${policy.table_name}`, { 
        error: (err as Error).message 
      });
    }
  }
  
  return { created, needsDropping };
}
