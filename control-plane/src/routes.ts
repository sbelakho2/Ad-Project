/**
 * CONTROL PLANE API ROUTES
 * 
 * All routes require:
 * 1. Operator JWT authentication
 * 
 * Route groups:
 * - /health - Basic healthcheck (no auth for load balancer)
 * - /metrics - Prometheus metrics (no auth for scraping)
 * - /api/v1/* - All operator routes (require auth)
 */

import { Router, Request, Response, NextFunction } from 'express';
import { requireOperator, verifyOperatorCredentials, issueOperatorToken } from './auth';
import { logAudit } from './logger';
import { createChallengeToken, hashDeviceId, isTrustedDevice, countDeviceHistory, touchDeviceSeen, trustDevice, createDeviceChallenge, confirmDeviceByToken, listTrustedDevices, revokeTrustedDeviceById } from './devices';
import { getConfig } from './config';
import { sendDeviceApprovalEmail } from './mailer';

function sendConfirmHtml(res: Response, params: { title: string; heading: string; message: string; ok: boolean }): void {
  res.setHeader('content-type', 'text/html; charset=utf-8');
  res.status(params.ok ? 200 : 400).send(`<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${params.title}</title>
    <style>
      :root { color-scheme: light; }
      body { margin: 0; font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial; background: #f8fafc; color: #0f172a; }
      .wrap { min-height: 100vh; display: grid; place-items: center; padding: 24px; }
      .card { width: 100%; max-width: 620px; background: #ffffff; border: 1px solid #e2e8f0; border-radius: 14px; padding: 20px; }
      h1 { margin: 0 0 8px; font-size: 18px; }
      p { margin: 0 0 14px; color: #334155; line-height: 1.45; }
      a.btn { display:inline-block; background: #2563eb; color: white; text-decoration:none; padding: 10px 14px; border-radius: 10px; font-weight: 600; }
      .muted { color: #64748b; font-size: 12px; }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="card">
        <h1>${params.heading}</h1>
        <p>${params.message}</p>
        <p><a class="btn" href="/admin/login">Return to login</a></p>
        <p class="muted">ApexMediation Control Plane — operator access</p>
      </div>
    </div>
  </body>
</html>`);
}

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

import { getComplianceStatus, getVatReports, generateAnnualReport, generateTsdReport, generateKmdInfReport, getTaxForecast } from './services/estonian-compliance';
import { generateVatPDF, generateAnnualReportPDF, generateTsdPDF, generateKmdInfPDF } from './services/pdf-generator';
import { getBusinessOverview } from './services/business-analytics';
import { getCohortAnalysis, getRevenueForecast } from './services/advanced-analytics';
import { processBankStatement, confirmReconciliation } from './services/reconciliation';
import { getImpersonationLink } from './adapters/backend-api';
import { queryReadOnly } from './database';

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
    await queryReadOnly('SELECT 1');
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
  const { username, password, deviceId, deviceLabel } = req.body;
  const ip = req.ip || req.socket.remoteAddress || 'unknown';
  const userAgent = req.get('User-Agent');
  const cfg = getConfig();
  const deviceApprovalEnabled = cfg.DEVICE_APPROVAL_ENABLED === 'true';
  
  if (!username || !password) {
    res.status(400).json({ error: 'Username and password required' });
    return;
  }

  if (deviceApprovalEnabled) {
    if (!deviceId || typeof deviceId !== 'string' || deviceId.length < 16 || deviceId.length > 256) {
      res.status(400).json({ error: 'Device ID required' });
      return;
    }
  }

  const safeDeviceLabel = typeof deviceLabel === 'string' && deviceLabel.trim() ? deviceLabel.trim().slice(0, 120) : undefined;
  
  const verified = await verifyOperatorCredentials(username, password, ip, userAgent);

  if (!verified) {
    res.status(401).json({ error: 'Authentication failed' });
    return;
  }

  const operatorEmail = verified.email;
  const deviceHash = deviceId ? hashDeviceId(deviceId) : '';

  if (deviceApprovalEnabled) {
    const isTrusted = await isTrustedDevice(operatorEmail, deviceHash);

    if (!isTrusted) {
      const existing = await countDeviceHistory(operatorEmail);

      // Bootstrap: only when the operator has never had a trusted device.
      // If all devices were revoked, we still require email approval.
      if (existing === 0) {
        await trustDevice({
          operatorEmail,
          deviceHash,
          deviceLabel: safeDeviceLabel,
          userAgent,
          addedVia: 'bootstrap',
        });

        const confirmUrl = `${cfg.PUBLIC_ORIGIN}/api/v1/devices/confirm?token=bootstrap`; // informational only
        await sendDeviceApprovalEmail({
          operatorEmail,
          deviceLabel: safeDeviceLabel,
          userAgent,
          requestedIp: ip,
          confirmUrl,
          kind: 'bootstrap-notice',
        });
      } else {
        // Require email approval for additional devices
        const token = createChallengeToken();
        await createDeviceChallenge({
          operatorEmail,
          deviceHash,
          deviceLabel: safeDeviceLabel,
          userAgent,
          requestedIp: ip,
          token,
        });

        const confirmUrl = `${cfg.PUBLIC_ORIGIN}/api/v1/devices/confirm?token=${encodeURIComponent(token)}`;

        try {
          await sendDeviceApprovalEmail({
            operatorEmail,
            deviceLabel: safeDeviceLabel,
            userAgent,
            requestedIp: ip,
            confirmUrl,
            kind: 'approval-required',
          });
        } catch (err) {
          logAudit({
            actor: operatorEmail,
            action: 'device_approval_email_failed',
            result: 'failure',
            reason: (err as Error).message,
            ip,
            userAgent,
          });
          res.status(503).json({
            error: 'DEVICE_APPROVAL_UNAVAILABLE',
            message: 'Cannot send device approval email. Please contact support.',
          });
          return;
        }

        logAudit({
          actor: operatorEmail,
          action: 'login_attempt',
          result: 'denied',
          reason: 'Device not approved',
          ip,
          userAgent,
        });

        res.status(403).json({
          error: 'DEVICE_CONFIRMATION_REQUIRED',
          message: `New device requires approval. A confirmation email was sent to ${cfg.DEVICE_APPROVAL_EMAIL_TO}.`,
        });
        return;
      }
    }

    await touchDeviceSeen(operatorEmail, deviceHash);
  }

  const issued = issueOperatorToken({ email: operatorEmail, ip, userAgent });
  res.json({ token: issued.token, expiresIn: `${issued.expiresIn}s` });
}));

/**
 * Device confirmation link (no operator auth)
 * Sent to contact@apexmediation.ee
 */
router.get('/api/v1/devices/confirm', asyncHandler(async (req: Request, res: Response) => {
  const token = (req.query.token as string | undefined) || '';
  if (!token || token.length < 10) {
    sendConfirmHtml(res, {
      title: 'Invalid confirmation link',
      heading: 'Invalid confirmation link',
      message: 'This confirmation link is missing or malformed.',
      ok: false,
    });
    return;
  }

  if (token === 'bootstrap') {
    sendConfirmHtml(res, {
      title: 'No action required',
      heading: 'No action required',
      message: 'This device was auto-enrolled as the first trusted device for that operator.',
      ok: true,
    });
    return;
  }

  const result = await confirmDeviceByToken(token);
  if (!result.ok) {
    const msg =
      result.reason === 'expired'
        ? 'This confirmation link has expired.'
        : result.reason === 'consumed'
          ? 'This confirmation link was already used.'
          : 'Invalid confirmation link.';
    sendConfirmHtml(res, {
      title: 'Device approval failed',
      heading: 'Device approval failed',
      message: msg,
      ok: false,
    });
    return;
  }

  sendConfirmHtml(res, {
    title: 'Device approved',
    heading: 'Device approved',
    message: 'This device is now trusted. Return to the login page and sign in from that device.',
    ok: true,
  });
}));

/**
 * List trusted devices for the current operator
 */
router.get('/api/v1/devices', requireOperator, asyncHandler(async (req: Request, res: Response) => {
  const operatorEmail = req.operator!.email;
  const rows = await listTrustedDevices(operatorEmail);

  res.json({
    devices: rows.map((r) => ({
      id: r.id,
      deviceLabel: r.device_label,
      userAgent: r.user_agent,
      addedVia: r.added_via,
      addedAt: r.added_at,
      lastSeenAt: r.last_seen_at,
      revokedAt: r.revoked_at,
      revokedBy: r.revoked_by,
      revokedReason: r.revoked_reason,
    })),
  });
}));

/**
 * Revoke a trusted device by id (current operator only)
 */
router.post('/api/v1/devices/:id/revoke', requireOperator, asyncHandler(async (req: Request, res: Response) => {
  const operatorEmail = req.operator!.email;
  const id = Number(req.params.id);
  if (!Number.isFinite(id) || id <= 0) {
    res.status(400).json({ error: 'Invalid device id' });
    return;
  }

  const reason = typeof req.body?.reason === 'string' ? req.body.reason : undefined;
  const ok = await revokeTrustedDeviceById({
    operatorEmail,
    deviceId: id,
    revokedBy: operatorEmail,
    revokedReason: reason,
  });

  if (!ok) {
    res.status(404).json({ error: 'Device not found or already revoked' });
    return;
  }

  res.json({ ok: true });
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
  if (req.path.startsWith('/devices/confirm')) {
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

// ============================================
// ESTONIAN COMPLIANCE ROUTES
// ============================================

router.get('/api/v1/compliance/status', requireOperator, asyncHandler(async (req, res) => {
  const status = await getComplianceStatus();
  res.json(status);
}));

router.get('/api/v1/compliance/vat-reports', requireOperator, asyncHandler(async (req, res) => {
  const reports = await getVatReports();
  res.json(reports);
}));

router.get('/api/v1/compliance/vat-reports/:id/pdf', requireOperator, asyncHandler(async (req, res) => {
  const reports = await getVatReports();
  const report = reports.find(r => r.id === req.params.id);
  if (!report) {
    res.status(404).json({ error: 'Report not found' });
    return;
  }
  const pdf = await generateVatPDF(report);
  res.setHeader('Content-Type', 'application/pdf');
  res.setHeader('Content-Disposition', `attachment; filename=vat-report-${report.year}-Q${report.quarter}.pdf`);
  res.send(pdf);
}));

router.get('/api/v1/compliance/annual-report/:year', requireOperator, asyncHandler(async (req, res) => {
  const year = parseInt(req.params.year);
  if (isNaN(year)) {
    res.status(400).json({ error: 'Invalid year' });
    return;
  }
  const report = await generateAnnualReport(year, (req as any).operator?.username);
  res.json(report);
}));

router.get('/api/v1/compliance/annual-report/:year/pdf', requireOperator, asyncHandler(async (req, res) => {
  const year = parseInt(req.params.year);
  if (isNaN(year)) {
    res.status(400).json({ error: 'Invalid year' });
    return;
  }
  const report = await generateAnnualReport(year);
  const pdf = await generateAnnualReportPDF(report);
  res.setHeader('Content-Type', 'application/pdf');
  res.setHeader('Content-Disposition', `attachment; filename=annual-report-${year}.pdf`);
  res.send(pdf);
}));

router.get('/api/v1/compliance/tsd-report/:year', requireOperator, asyncHandler(async (req, res) => {
  const year = parseInt(req.params.year);
  const month = req.query.month ? parseInt(req.query.month as string) : undefined;
  
  if (isNaN(year)) {
    res.status(400).json({ error: 'Invalid year' });
    return;
  }
  
  const report = await generateTsdReport(year, month);
  res.json(report);
}));

router.get('/api/v1/compliance/tsd-report/:year/pdf', requireOperator, asyncHandler(async (req, res) => {
  const year = parseInt(req.params.year);
  const month = req.query.month ? parseInt(req.query.month as string) : undefined;
  
  if (isNaN(year)) {
    res.status(400).json({ error: 'Invalid year' });
    return;
  }
  
  const report = await generateTsdReport(year, month);
  const pdf = await generateTsdPDF(report);
  res.setHeader('Content-Type', 'application/pdf');
  res.setHeader('Content-Disposition', `attachment; filename=tsd-report-${year}${month ? '-M' + month : ''}.pdf`);
  res.send(pdf);
}));

router.get('/api/v1/compliance/kmd-inf/:year/:month', requireOperator, asyncHandler(async (req, res) => {
  const year = parseInt(req.params.year);
  const month = parseInt(req.params.month);
  const report = await generateKmdInfReport(year, month);
  res.json(report);
}));

router.get('/api/v1/compliance/kmd-inf/:year/:month/pdf', requireOperator, asyncHandler(async (req, res) => {
  const year = parseInt(req.params.year);
  const month = parseInt(req.params.month);
  const report = await generateKmdInfReport(year, month);
  const pdf = await generateKmdInfPDF(report);
  res.setHeader('Content-Type', 'application/pdf');
  res.setHeader('Content-Disposition', `attachment; filename=kmd-inf-${year}-${month}.pdf`);
  res.send(pdf);
}));

router.get('/api/v1/compliance/forecast', requireOperator, asyncHandler(async (req, res) => {
  const forecast = await getTaxForecast();
  res.json({ forecast });
}));

// ============================================
// DATA EXPLORER ROUTES
// ============================================

/**
 * List users (publishers)
 */
router.get('/api/v1/data/users', requireOperator, asyncHandler(async (req, res) => {
  const users = await queryReadOnly(`
    SELECT id, email, company_name, created_at, status
    FROM users
    ORDER BY created_at DESC
    LIMIT 100
  `);
  res.json({ users });
}));

/**
 * List invoices
 */
router.get('/api/v1/data/invoices', requireOperator, asyncHandler(async (req, res) => {
  const invoices = await queryReadOnly(`
    SELECT i.id, i.invoice_number, u.email as customer_email, i.total_cents, i.status, i.issue_date
    FROM invoices i
    JOIN users u ON i.customer_id = u.id
    ORDER BY i.issue_date DESC
    LIMIT 100
  `);
  res.json({ invoices });
}));

// ============================================
// BUSINESS ANALYTICS ROUTES
// ============================================

/**
 * Get business metrics overview
 */
router.get('/api/v1/analytics/overview', requireOperator, asyncHandler(async (req, res) => {
  const overview = await getBusinessOverview();
  res.json(overview);
}));

/**
 * Advanced Analytics: Cohorts
 */
router.get('/api/v1/analytics/cohorts', requireOperator, asyncHandler(async (req, res) => {
  const cohorts = await getCohortAnalysis();
  res.json({ cohorts });
}));

/**
 * Advanced Analytics: Forecast
 */
router.get('/api/v1/analytics/forecast', requireOperator, asyncHandler(async (req, res) => {
  const forecast = await getRevenueForecast();
  res.json({ forecast });
}));

// ============================================
// RECONCILIATION ROUTES
// ============================================

/**
 * Process bank statement
 */
router.get('/api/v1/reconciliation/process', requireOperator, asyncHandler(async (req, res) => {
  const matches = await processBankStatement((req as any).operator?.username);
  res.json({ matches });
}));

/**
 * Confirm reconciliation
 */
router.post('/api/v1/reconciliation/confirm', requireOperator, asyncHandler(async (req, res) => {
  const { match } = req.body;
  const ok = await confirmReconciliation(match, (req as any).operator?.username);
  res.json({ ok });
}));

/**
 * Get impersonation link for a user
 */
router.post('/api/v1/support/impersonate', requireOperator, asyncHandler(async (req, res) => {
  const { userId } = req.body;
  const operator = (req as any).operator?.username;
  const result = await getImpersonationLink(userId, operator);
  res.json(result);
}));

export default router;
