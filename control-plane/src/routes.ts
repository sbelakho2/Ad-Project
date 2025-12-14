/**
 * CONTROL PLANE API ROUTES
 * 
 * All routes require:
 * 1. IP allowlist check (handled at app level)
 * 2. Operator JWT authentication
 * 
 * Route groups:
 * - /health - Basic healthcheck (no auth for load balancer)
 * - /metrics - Prometheus metrics (no auth for scraping)
 * - /api/v1/* - All operator routes (require auth)
 */

import { Router, Request, Response, NextFunction } from 'express';
import { requireOperator, authenticateOperator } from './auth';

// Telemetry adapters
import {
  getTopQueries, 
  getActiveSessions, 
  getBlockingSessions,
  getLongTransactions,
  getDatabaseStats,
  getWALStats,
  getTableMaintenanceStats,
  getReplicationStatus,
} from './adapters/postgres-telemetry';

import { getHostTelemetry } from './adapters/host-telemetry';
import { getAutopilotHealth, getMigrationStudioMetrics, getVRAExperimentStatus } from './adapters/autopilot';
import { getAppTelemetry, getSlowEndpoints, getSaturatedPools, getHighErrorEndpoints } from './adapters/app-telemetry';

// Partition management
import { 
  getAllPartitionStatus, 
  getPartitionStatus, 
  createFuturePartitions,
  dropOldPartitions,
  checkPartitionHealth,
  autoMaintainPartitions,
  getAllRetentionPolicies,
} from './adapters/partition-manager';

// Action gateway
import {
  captureIncidentSnapshot,
  runDeepHealthcheck,
  enterDegradedMode,
  exitDegradedMode,
  terminateSession,
  dropOldPartition,
} from './adapters/action-gateway';

const router = Router();

/**
 * Error wrapper for async routes
 */
function asyncHandler(fn: (req: Request, res: Response, next: NextFunction) => Promise<void>) {
  return (req: Request, res: Response, next: NextFunction) => {
    Promise.resolve(fn(req, res, next)).catch(next);
  };
}

// ============================================
// PUBLIC ROUTES (no auth, for infrastructure)
// ============================================

/**
 * Health check - for load balancer
 */
router.get('/health', (req: Request, res: Response) => {
  res.json({ 
    status: 'ok', 
    service: 'control-plane',
    timestamp: new Date().toISOString(),
  });
});

/**
 * Ready check - for kubernetes
 */
router.get('/ready', asyncHandler(async (req: Request, res: Response) => {
  try {
    const { getReadOnlyPool } = await import('./database');
    const pool = getReadOnlyPool();
    await pool.query('SELECT 1');
    res.json({ status: 'ready' });
  } catch (err) {
    res.status(503).json({ status: 'not ready', error: (err as Error).message });
  }
}));

// ============================================
// AUTHENTICATION
// ============================================

/**
 * Operator login
 */
router.post('/api/v1/auth/login', asyncHandler(async (req: Request, res: Response) => {
  const { username, password } = req.body;
  const ip = req.ip || req.socket.remoteAddress || 'unknown';
  const userAgent = req.get('User-Agent');
  
  if (!username || !password) {
    res.status(400).json({ error: 'Username and password required' });
    return;
  }
  
  const result = await authenticateOperator(username, password, ip, userAgent);
  
  if (result) {
    res.json({ token: result.token, expiresIn: `${result.expiresIn}s` });
  } else {
    res.status(401).json({ error: 'Authentication failed' });
  }
}));

/**
 * Token refresh
 */
router.post('/api/v1/auth/refresh', requireOperator, asyncHandler(async (req: Request, res: Response) => {
  // Refresh not implemented - operator must re-authenticate
  // This is intentional for security - no long-lived sessions
  res.status(501).json({ error: 'Token refresh not supported. Please re-authenticate.' });
}));

// ============================================
// PROTECTED ROUTES (require operator auth)
// ============================================

// Apply auth to all /api/v1/* routes except /auth/*
router.use('/api/v1', (req: Request, res: Response, next: NextFunction) => {
  if (req.path.startsWith('/auth/')) {
    return next();
  }
  return requireOperator(req, res, next);
});

// ============================================
// TELEMETRY ROUTES
// ============================================

/**
 * Database telemetry overview
 */
router.get('/api/v1/telemetry/database', asyncHandler(async (req: Request, res: Response) => {
  const [
    topQueries,
    activeSessions,
    blockingSessions,
    longTransactions,
    databaseStats,
    walStats,
    maintenanceStats,
    replicationStatus,
  ] = await Promise.all([
    getTopQueries(10),
    getActiveSessions(),
    getBlockingSessions(),
    getLongTransactions(5),
    getDatabaseStats(),
    getWALStats(),
    getTableMaintenanceStats(),
    getReplicationStatus(),
  ]);
  
  res.json({
    timestamp: new Date().toISOString(),
    topQueries,
    activeSessions,
    blockingSessions,
    longTransactions,
    databaseStats,
    walStats,
    maintenanceStats,
    replicationStatus,
  });
}));

/**
 * Top queries by total time
 */
router.get('/api/v1/telemetry/database/queries', asyncHandler(async (req: Request, res: Response) => {
  const limit = parseInt(req.query.limit as string) || 20;
  const queries = await getTopQueries(limit);
  res.json({ queries });
}));

/**
 * Active database sessions
 */
router.get('/api/v1/telemetry/database/sessions', asyncHandler(async (req: Request, res: Response) => {
  const [active, blocking, longTx] = await Promise.all([
    getActiveSessions(),
    getBlockingSessions(),
    getLongTransactions(1),
  ]);
  res.json({ active, blocking, longTransactions: longTx });
}));

/**
 * Host telemetry (disk, memory, CPU)
 */
router.get('/api/v1/telemetry/host', asyncHandler(async (req: Request, res: Response) => {
  const telemetry = await getHostTelemetry();
  res.json({ telemetry });
}));

/**
 * App telemetry (latency, pools, errors)
 */
router.get('/api/v1/telemetry/app', asyncHandler(async (req: Request, res: Response) => {
  const telemetry = await getAppTelemetry();
  res.json({ telemetry });
}));

/**
 * Slow endpoints analysis
 */
router.get('/api/v1/telemetry/app/slow', asyncHandler(async (req: Request, res: Response) => {
  const thresholdMs = parseInt(req.query.threshold as string) || 1000;
  const endpoints = await getSlowEndpoints(thresholdMs);
  res.json({ thresholdMs, endpoints });
}));

/**
 * Pool saturation
 */
router.get('/api/v1/telemetry/app/pools', asyncHandler(async (req: Request, res: Response) => {
  const pools = await getSaturatedPools(0);
  res.json({ pools });
}));

/**
 * Error rates
 */
router.get('/api/v1/telemetry/app/errors', asyncHandler(async (req: Request, res: Response) => {
  const errors = await getHighErrorEndpoints(0);
  res.json({ errors });
}));

// ============================================
// AUTOMATION/JOB ROUTES
// ============================================

/**
 * Autopilot health overview
 */
router.get('/api/v1/jobs/health', asyncHandler(async (req: Request, res: Response) => {
  const health = await getAutopilotHealth();
  res.json(health);
}));

/**
 * Migration Studio status
 */
router.get('/api/v1/jobs/migrations', asyncHandler(async (req: Request, res: Response) => {
  const metrics = await getMigrationStudioMetrics();
  res.json(metrics);
}));

/**
 * VRA experiment status
 */
router.get('/api/v1/jobs/vra', asyncHandler(async (req: Request, res: Response) => {
  const experiments = await getVRAExperimentStatus();
  res.json({ experiments });
}));

// ============================================
// PARTITION MANAGEMENT ROUTES
// ============================================

/**
 * Get all partition status
 */
router.get('/api/v1/partitions', asyncHandler(async (req: Request, res: Response) => {
  const statuses = await getAllPartitionStatus();
  const health = await checkPartitionHealth();
  const policies = getAllRetentionPolicies();
  res.json({ statuses, health, policies });
}));

/**
 * Get partition status for a specific table
 */
router.get('/api/v1/partitions/:tableName', asyncHandler(async (req: Request, res: Response) => {
  const { tableName } = req.params;
  const status = await getPartitionStatus(tableName);
  res.json(status);
}));

/**
 * Create future partitions
 */
router.post('/api/v1/partitions/:tableName/create', asyncHandler(async (req: Request, res: Response) => {
  const { tableName } = req.params;
  const { monthsAhead = 3 } = req.body;
  const operator = (req as any).operator;
  
  if (monthsAhead > 12) {
    res.status(400).json({ error: 'Cannot create more than 12 months of partitions' });
    return;
  }
  
  const result = await createFuturePartitions(tableName, monthsAhead, operator.username);
  res.json(result);
}));

/**
 * Preview partition drops (dry run)
 */
router.post('/api/v1/partitions/:tableName/preview-drop', asyncHandler(async (req: Request, res: Response) => {
  const { tableName } = req.params;
  const { retentionDays } = req.body;
  const operator = (req as any).operator;
  
  if (!retentionDays || retentionDays < 7) {
    res.status(400).json({ error: 'retentionDays must be >= 7' });
    return;
  }
  
  const result = await dropOldPartitions(tableName, retentionDays, operator.username, true);
  res.json(result);
}));

/**
 * Auto-maintain all partitions
 */
router.post('/api/v1/partitions/auto-maintain', asyncHandler(async (req: Request, res: Response) => {
  const operator = (req as any).operator;
  const result = await autoMaintainPartitions(operator.username);
  res.json(result);
}));

// ============================================
// ACTION GATEWAY ROUTES
// ============================================

/**
 * Tier 0: Capture incident snapshot
 */
router.post('/api/v1/actions/snapshot', asyncHandler(async (req: Request, res: Response) => {
  const operator = (req as any).operator;
  const result = await captureIncidentSnapshot({
    actor: operator.username,
    ip: operator.ip,
    userAgent: req.get('User-Agent'),
  });
  res.json(result);
}));

/**
 * Tier 0: Run deep healthcheck
 */
router.post('/api/v1/actions/healthcheck', asyncHandler(async (req: Request, res: Response) => {
  const operator = (req as any).operator;
  const result = await runDeepHealthcheck({
    actor: operator.username,
    ip: operator.ip,
    userAgent: req.get('User-Agent'),
  });
  res.json(result);
}));

/**
 * Tier 1: Enter degraded mode
 */
router.post('/api/v1/actions/degraded-mode/enter', asyncHandler(async (req: Request, res: Response) => {
  const { reason } = req.body;
  const operator = (req as any).operator;
  
  if (!reason) {
    res.status(400).json({ error: 'Reason required' });
    return;
  }
  
  const result = await enterDegradedMode({
    actor: operator.username,
    ip: operator.ip,
    userAgent: req.get('User-Agent'),
  }, reason);
  
  res.json(result);
}));

/**
 * Tier 1: Exit degraded mode
 */
router.post('/api/v1/actions/degraded-mode/exit', asyncHandler(async (req: Request, res: Response) => {
  const operator = (req as any).operator;
  const result = await exitDegradedMode({
    actor: operator.username,
    ip: operator.ip,
    userAgent: req.get('User-Agent'),
  });
  res.json(result);
}));

/**
 * Tier 2: Terminate database session
 * Requires typed confirmation
 */
router.post('/api/v1/actions/terminate-session', asyncHandler(async (req: Request, res: Response) => {
  const { pid, confirmation } = req.body;
  const operator = (req as any).operator;
  
  if (!pid || typeof pid !== 'number') {
    res.status(400).json({ error: 'Valid pid required' });
    return;
  }
  
  if (!confirmation) {
    res.status(400).json({ 
      error: 'Confirmation required',
      expectedConfirmation: `TERMINATE SESSION ${pid}`,
    });
    return;
  }
  
  const result = await terminateSession({
    actor: operator.username,
    ip: operator.ip,
    userAgent: req.get('User-Agent'),
    confirmation,
  }, pid, confirmation);
  
  res.json(result);
}));

/**
 * Tier 2: Drop old partition
 * Requires typed confirmation
 */
router.post('/api/v1/actions/drop-partition', asyncHandler(async (req: Request, res: Response) => {
  const { partitionName, confirmation } = req.body;
  const operator = (req as any).operator;
  
  if (!partitionName) {
    res.status(400).json({ error: 'partitionName required' });
    return;
  }
  
  if (!confirmation) {
    res.status(400).json({ 
      error: 'Confirmation required',
      expectedConfirmation: `DROP PARTITION ${partitionName}`,
    });
    return;
  }
  
  const result = await dropOldPartition({
    actor: operator.username,
    ip: operator.ip,
    userAgent: req.get('User-Agent'),
    confirmation,
  }, partitionName, confirmation);
  
  res.json(result);
}));

// ============================================
// AUDIT LOG ROUTES
// ============================================

/**
 * Get recent audit logs
 */
router.get('/api/v1/audit', asyncHandler(async (req: Request, res: Response) => {
  const { queryReadOnly } = await import('./database');
  const limit = parseInt(req.query.limit as string) || 100;
  const actor = req.query.actor as string;
  const action = req.query.action as string;
  
  let query = `
    SELECT * FROM audit_logs
    WHERE 1=1
  `;
  const params: any[] = [];
  let paramIndex = 1;
  
  if (actor) {
    query += ` AND actor = $${paramIndex++}`;
    params.push(actor);
  }
  
  if (action) {
    query += ` AND action = $${paramIndex++}`;
    params.push(action);
  }
  
  query += ` ORDER BY timestamp DESC LIMIT $${paramIndex}`;
  params.push(limit);
  
  const logs = await queryReadOnly(query, params);
  res.json({ logs });
}));

// ============================================
// SYSTEM STATE ROUTES
// ============================================

/**
 * Get current system state
 */
router.get('/api/v1/system/state', asyncHandler(async (req: Request, res: Response) => {
  const { queryReadOnly } = await import('./database');
  const rows = await queryReadOnly<{ key: string; value: string; updated_at: string }>(`
    SELECT key, value, updated_at FROM system_state ORDER BY key
  `);
  
  const state = rows.reduce((acc, row) => {
    acc[row.key] = { value: row.value, updatedAt: row.updated_at };
    return acc;
  }, {} as Record<string, { value: string; updatedAt: string }>);
  
  res.json({ state });
}));

/**
 * Get comprehensive dashboard data
 */
router.get('/api/v1/dashboard', asyncHandler(async (req: Request, res: Response) => {
  const [
    dbTelemetry,
    hostTelemetry,
    appTelemetry,
    autopilotHealth,
    partitionHealth,
  ] = await Promise.all([
    getDatabaseStats(),
    getHostTelemetry(),
    getAppTelemetry(),
    getAutopilotHealth(),
    checkPartitionHealth(),
  ]);
  
  // Calculate overall health
  const issues: string[] = [];
  
  // Check disk usage (get max from array of disks)
  if (hostTelemetry && hostTelemetry.disk && hostTelemetry.disk.length > 0) {
    const maxDiskUsage = Math.max(...hostTelemetry.disk.map(d => d.use_percent));
    if (maxDiskUsage > 85) {
      issues.push(`Disk usage at ${maxDiskUsage}%`);
    }
  }
  
  // Check memory usage
  if (hostTelemetry && hostTelemetry.memory && hostTelemetry.memory.use_percent > 90) {
    issues.push(`Memory usage at ${hostTelemetry.memory.use_percent}%`);
  }
  
  // Check autopilot health
  if (autopilotHealth && autopilotHealth.overall_status !== 'OK') {
    issues.push('Autopilot jobs unhealthy');
  }
  
  if (!partitionHealth.healthy) {
    issues.push(`Partition issues: ${partitionHealth.issues.length}`);
  }
  
  const overallHealth = issues.length === 0 ? 'OK' : 
                        issues.length <= 2 ? 'DEGRADED' : 'CRITICAL';
  
  res.json({
    timestamp: new Date().toISOString(),
    health: {
      overall: overallHealth,
      issues,
    },
    database: dbTelemetry,
    host: hostTelemetry,
    app: appTelemetry.health,
    autopilot: autopilotHealth,
    partitions: partitionHealth,
  });
}));

export default router;
