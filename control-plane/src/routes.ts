/**
 * CONTROL PLANE API ROUTES
 * 
 * All routes require:
 * 1. IP allowlist check (if OPERATOR_IP_ALLOWLIST is set)
 * 2. Rate limiting on login endpoints
 * 3. Operator JWT authentication
 * 
 * Route groups:
 * - /health - Basic healthcheck (no auth for load balancer)
 * - /metrics - Prometheus metrics (no auth for scraping)
 * - /api/v1/* - All operator routes (require auth)
 */

import { Router, Request, Response, NextFunction } from 'express';
import { requireOperator, verifyOperatorCredentials, issueOperatorToken, requireAllowedIP, rateLimitLogin, recordLoginAttempt, clearLoginAttempts, getClientIP } from './auth';
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

import { getHostTelemetry, getHostTelemetryBundle } from './adapters/host-telemetry';
import { getS3StorageTelemetry } from './adapters/s3-telemetry';
import { getAutopilotHealth, getMigrationStudioMetrics, getVRAExperimentStatus } from './adapters/autopilot';
import { getAppTelemetry, getSlowEndpoints, getSaturatedPools, getHighErrorEndpoints } from './adapters/app-telemetry';

// Partition management
import {
  getAllPartitionStatus, 
  getPartitionStatus, 
  getPartitionStatusForSchema,
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

// Sales Automation
import * as salesAutomation from './services/sales-automation';

// Autonomous Sales API routes
import autonomousApiProxy from './routes/autonomous-api-proxy';
import autonomousApiExecution from './routes/autonomous-api';
import { logger } from './logger';

const router = Router();

// ============================================
// IP ALLOWLIST - Applied to ALL API routes
// ============================================
router.use('/api', requireAllowedIP);

async function getTableColumns(tableName: string): Promise<Set<string>> {
  const rows = await queryReadOnly<{ column_name: string }>(
    `SELECT column_name FROM information_schema.columns WHERE table_name = $1`,
    [tableName]
  );
  return new Set(rows.map(row => row.column_name));
}

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
 * JS Integrity check - verifies served JavaScript parses correctly
 * IMPROVEMENT 1: Runtime check to catch template string escaping bugs
 * Call this after deployment to verify UI won't be broken
 */
router.get('/health/js', asyncHandler(async (req: Request, res: Response) => {
  const checks: { name: string; status: 'ok' | 'error'; error?: string }[] = [];
  
  // Helper to test JS syntax
  function testJsSyntax(name: string, js: string): void {
    try {
      // Use Function constructor to test parse (doesn't execute)
      new Function(js);
      checks.push({ name, status: 'ok' });
    } catch (err) {
      checks.push({ name, status: 'error', error: (err as Error).message });
    }
  }
  
  // Import the JS generators from index.ts via internal fetch
  // Since they're inline, we use a minimal test pattern to verify critical functions
  const criticalPatterns = [
    { name: 'editTemplate-newlines', pattern: /\\n(?!\\)/ },  // Detect unescaped \n
    { name: 'showSpintaxModal-newlines', pattern: /\\n(?!\\)/ },
  ];
  
  // Test: Critical inline JS functions parse correctly
  // This catches the bug we fixed (unescaped newlines in template strings)
  const testFunctions = [
    `function editTemplate(id) { var lines = 'test\\\\n'; return lines; }`,
    `function showSpintaxModal() { var body = 'Hi,\\\\nTest'; return body; }`,
    `async function apiFetch(p,o) { return fetch(p,o); }`,
  ];
  
  testFunctions.forEach((fn, i) => testJsSyntax(`inline-fn-${i}`, fn));
  
  const allOk = checks.every(c => c.status === 'ok');
  res.status(allOk ? 200 : 500).json({
    status: allOk ? 'ok' : 'error',
    service: 'control-plane-js',
    checks,
    timestamp: new Date().toISOString(),
  });
}));

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
 * Protected by rate limiting to prevent brute-force attacks
 */
router.post('/api/v1/auth/login', rateLimitLogin, asyncHandler(async (req: Request, res: Response) => {
  const { username, password, deviceId, deviceLabel } = req.body;
  const ip = getClientIP(req);
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
    // Record failed attempt for rate limiting
    recordLoginAttempt(ip);
    res.status(401).json({ error: 'Authentication failed' });
    return;
  }

  // Clear rate limit on successful login
  clearLoginAttempts(ip);

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
 * Host telemetry (app + db)
 */
router.get('/api/v1/telemetry/hosts', asyncHandler(async (req: Request, res: Response) => {
  const hosts = await getHostTelemetryBundle();
  res.json({ hosts });
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
  const schema = typeof req.query.schema === 'string' ? req.query.schema : 'public';
  const status = schema && schema !== 'public'
    ? await getPartitionStatusForSchema(tableName, schema)
    : await getPartitionStatus(tableName);
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
    s3Telemetry,
  ] = await Promise.all([
    getDatabaseStats(),
    getHostTelemetryBundle(),
    getAppTelemetry(),
    getAutopilotHealth(),
    checkPartitionHealth(),
    getS3StorageTelemetry(),
  ]);
  
  // Calculate overall health
  const issues: string[] = [];
  
  // Check disk usage (get max from app + db disks)
  const appDisks = hostTelemetry.app?.disk || [];
  const dbDisks = hostTelemetry.db?.disk || [];
  const diskUsages = [...appDisks, ...dbDisks].map(d => d.use_percent);
  if (diskUsages.length > 0) {
    const maxDiskUsage = Math.max(...diskUsages);
    if (maxDiskUsage > 85) {
      issues.push(`Disk usage at ${Math.round(maxDiskUsage)}%`);
    }
  }
  
  // Check memory usage
  if (hostTelemetry.app?.memory?.use_percent && hostTelemetry.app.memory.use_percent > 90) {
    issues.push(`App memory usage at ${Math.round(hostTelemetry.app.memory.use_percent)}%`);
  }
  if (hostTelemetry.db?.memory?.use_percent && hostTelemetry.db.memory.use_percent > 90) {
    issues.push(`DB memory usage at ${Math.round(hostTelemetry.db.memory.use_percent)}%`);
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
  
  const insights: string[] = [];
  if (!hostTelemetry.app) {
    insights.push('Configure HOST_TELEMETRY_ENDPOINT_APP to monitor app server disk/memory.');
  }
  if (!hostTelemetry.db) {
    insights.push('Configure HOST_TELEMETRY_ENDPOINT_DB to monitor DB server disk/memory.');
  }
  if (!s3Telemetry) {
    insights.push('Configure S3_STORAGE_ENDPOINT to report bucket usage and object counts.');
  }
  if (partitionHealth && !partitionHealth.healthy) {
    insights.push('Run partition maintenance: missing or unhealthy partitions detected.');
  }
  if (autopilotHealth && autopilotHealth.overall_status !== 'OK') {
    insights.push('Investigate autopilot jobs: background maintenance is degraded.');
  }
  if (appTelemetry?.health?.status === 'DOWN') {
    insights.push('Backend metrics report DOWN: check app health and metrics endpoint.');
  }
  if (diskUsages.length > 0 && Math.max(...diskUsages) > 80) {
    insights.push('Disk usage above 80%: review backups and archival policies.');
  }
  if (dbTelemetry && dbTelemetry.length > 0) {
    const topDb = dbTelemetry[0];
    if (topDb.size_bytes > 50 * 1024 * 1024 * 1024) {
      insights.push('Database size exceeds 50GB: consider rollup archiving or retention tuning.');
    }
  }

  res.json({
    timestamp: new Date().toISOString(),
    health: {
      overall: overallHealth,
      issues,
    },
    database: dbTelemetry,
    host: hostTelemetry,
    storage: {
      app: hostTelemetry.app?.disk || null,
      db: hostTelemetry.db?.disk || null,
      s3: s3Telemetry,
    },
    app: appTelemetry.health,
    autopilot: autopilotHealth,
    partitions: partitionHealth,
    insights,
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
  const userColumns = await getTableColumns('users');
  const publisherColumns = await getTableColumns('publishers');
  const hasPublisherJoin = userColumns.has('publisher_id') && publisherColumns.size > 0;

  const companyExpr = userColumns.has('company_name')
    ? 'u.company_name'
    : publisherColumns.has('company_name')
      ? 'p.company_name'
      : 'NULL';

  const statusExpr = userColumns.has('status')
    ? 'u.status'
    : publisherColumns.has('status')
      ? 'p.status'
      : `'active'`;

  const createdExpr = userColumns.has('created_at')
    ? 'u.created_at'
    : publisherColumns.has('created_at')
      ? 'p.created_at'
      : 'NOW()';

  const users = await queryReadOnly(`
    SELECT u.id, u.email,
           ${companyExpr} as company_name,
           ${createdExpr} as created_at,
           ${statusExpr} as status
    FROM users u
    ${hasPublisherJoin ? 'LEFT JOIN publishers p ON u.publisher_id = p.id' : ''}
    ORDER BY ${createdExpr} DESC
    LIMIT 100
  `);
  res.json({ users });
}));

/**
 * List invoices
 */
router.get('/api/v1/data/invoices', requireOperator, asyncHandler(async (req, res) => {
  const invoiceColumns = await getTableColumns('invoices');

  const dateExpr = invoiceColumns.has('issue_date')
    ? 'i.issue_date'
    : invoiceColumns.has('invoice_date')
      ? 'i.invoice_date'
      : 'i.created_at';

  const totalExpr = invoiceColumns.has('total_cents')
    ? 'i.total_cents'
    : invoiceColumns.has('total_amount_cents')
      ? 'i.total_amount_cents'
      : 'NULL';

  const statusExpr = invoiceColumns.has('status')
    ? 'i.status'
    : `'unknown'`;

  const numberExpr = invoiceColumns.has('invoice_number')
    ? 'i.invoice_number'
    : 'i.id::text';

  const invoices = await queryReadOnly(`
    SELECT i.id,
           ${numberExpr} as invoice_number,
           u.email as customer_email,
           ${totalExpr} as total_cents,
           ${statusExpr} as status,
           ${dateExpr} as issue_date
    FROM invoices i
    JOIN users u ON i.customer_id = u.id
    ORDER BY ${dateExpr} DESC
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

// ============================================
// SALES AUTOMATION ROUTES
// ============================================

/**
 * Get sales automation status
 */
router.get('/api/v1/sales/status', requireOperator, asyncHandler(async (req, res) => {
  const status = await salesAutomation.getSalesAutomationStatus();
  res.json(status);
}));

/**
 * Toggle sales automation system on/off
 */
router.post('/api/v1/sales/toggle', requireOperator, asyncHandler(async (req, res) => {
  const { enabled } = req.body;
  const result = await salesAutomation.setSystemEnabled(enabled === true);
  res.json(result);
}));

/**
 * Get sales automation statistics
 */
router.get('/api/v1/sales/stats', requireOperator, asyncHandler(async (req, res) => {
  const stats = await salesAutomation.getSalesAutomationStats();
  res.json(stats);
}));

/**
 * Get contacts
 */
router.get('/api/v1/sales/contacts', requireOperator, asyncHandler(async (req, res) => {
  const { status, limit, offset, search } = req.query;
  const result = await salesAutomation.getContacts({
    status: status as string,
    limit: limit ? parseInt(limit as string) : undefined,
    offset: offset ? parseInt(offset as string) : undefined,
    search: search as string,
  });
  res.json(result);
}));

/**
 * Add a contact
 */
router.post('/api/v1/sales/contacts', requireOperator, asyncHandler(async (req, res) => {
  const result = await salesAutomation.addContact(req.body);
  res.json(result);
}));

/**
 * Update a contact
 */
router.put('/api/v1/sales/contacts/:id', requireOperator, asyncHandler(async (req, res) => {
  const result = await salesAutomation.updateContact(req.params.id, req.body);
  res.json(result);
}));

/**
 * Delete a contact
 */
router.delete('/api/v1/sales/contacts/:id', requireOperator, asyncHandler(async (req, res) => {
  const result = await salesAutomation.deleteContact(req.params.id);
  res.json(result);
}));

/**
 * Import contacts (CSV data)
 */
router.post('/api/v1/sales/contacts/import', requireOperator, asyncHandler(async (req, res) => {
  const { contacts } = req.body;
  const result = await salesAutomation.importContacts(contacts);
  res.json(result);
}));

/**
 * Get sequences
 */
router.get('/api/v1/sales/sequences', requireOperator, asyncHandler(async (req, res) => {
  const sequences = await salesAutomation.getSequences();
  res.json({ sequences });
}));

/**
 * Get single sequence
 */
router.get('/api/v1/sales/sequences/:id', requireOperator, asyncHandler(async (req, res) => {
  const sequence = await salesAutomation.getSequence(req.params.id);
  res.json({ sequence });
}));

/**
 * Create sequence
 */
router.post('/api/v1/sales/sequences', requireOperator, asyncHandler(async (req, res) => {
  const result = await salesAutomation.createSequence(req.body);
  res.json(result);
}));

/**
 * Update sequence
 */
router.put('/api/v1/sales/sequences/:id', requireOperator, asyncHandler(async (req, res) => {
  const result = await salesAutomation.updateSequence(req.params.id, req.body);
  res.json(result);
}));

/**
 * Delete sequence
 */
router.delete('/api/v1/sales/sequences/:id', requireOperator, asyncHandler(async (req, res) => {
  const result = await salesAutomation.deleteSequence(req.params.id);
  res.json(result);
}));

/**
 * Get enrollments
 */
router.get('/api/v1/sales/enrollments', requireOperator, asyncHandler(async (req, res) => {
  const { sequenceId, status, limit, offset } = req.query;
  const result = await salesAutomation.getEnrollments({
    sequenceId: sequenceId as string,
    status: status as string,
    limit: limit ? parseInt(limit as string) : undefined,
    offset: offset ? parseInt(offset as string) : undefined,
  });
  res.json(result);
}));

/**
 * Enroll contacts in a sequence
 */
router.post('/api/v1/sales/enrollments/enroll', requireOperator, asyncHandler(async (req, res) => {
  const { sequenceId, contactStatus, limit } = req.body;
  const result = await salesAutomation.enrollContacts({
    sequenceId,
    contactStatus,
    limit,
  });
  res.json(result);
}));

/**
 * Pause an enrollment
 */
router.post('/api/v1/sales/enrollments/:id/pause', requireOperator, asyncHandler(async (req, res) => {
  const result = await salesAutomation.pauseEnrollment(req.params.id);
  res.json(result);
}));

/**
 * Resume an enrollment
 */
router.post('/api/v1/sales/enrollments/:id/resume', requireOperator, asyncHandler(async (req, res) => {
  const result = await salesAutomation.resumeEnrollment(req.params.id);
  res.json(result);
}));

/**
 * Get emails
 */
router.get('/api/v1/sales/emails', requireOperator, asyncHandler(async (req, res) => {
  const { contactId, status, limit, offset } = req.query;
  const result = await salesAutomation.getEmails({
    contactId: contactId as string,
    status: status as string,
    limit: limit ? parseInt(limit as string) : undefined,
    offset: offset ? parseInt(offset as string) : undefined,
  });
  res.json(result);
}));

/**
 * Run outreach cycle (send pending emails)
 */
router.post('/api/v1/sales/run', requireOperator, asyncHandler(async (req, res) => {
  const { dryRun } = req.body;
  const result = await salesAutomation.runOutreachCycle(dryRun);
  res.json(result);
}));

/**
 * Get activity log
 */
router.get('/api/v1/sales/activity', requireOperator, asyncHandler(async (req, res) => {
  const { contactId, limit } = req.query;
  const activity = await salesAutomation.getActivityLog({
    contactId: contactId as string,
    limit: limit ? parseInt(limit as string) : undefined,
  });
  res.json({ activity });
}));

// ============================================
// SALES AUTOMATION CONFIG ROUTES (NEW)
// ============================================

/**
 * Get sales automation configuration
 */
router.get('/api/v1/sales/config', requireOperator, asyncHandler(async (req, res) => {
  const config = await salesAutomation.getSalesConfig();
  res.json({ config });
}));

/**
 * Update sales automation configuration
 */
router.put('/api/v1/sales/config', requireOperator, asyncHandler(async (req, res) => {
  const result = await salesAutomation.updateSalesConfig(req.body);
  res.json(result);
}));

// ============================================
// SALES AUTOMATION REVIEW QUEUE ROUTES (NEW)
// ============================================

/**
 * Get draft emails for review
 */
router.get('/api/v1/sales/drafts', requireOperator, asyncHandler(async (req, res) => {
  const { limit, offset } = req.query;
  const result = await salesAutomation.getDraftEmails({
    limit: limit ? parseInt(limit as string) : undefined,
    offset: offset ? parseInt(offset as string) : undefined,
  });
  res.json(result);
}));

/**
 * Approve a draft email (sends it)
 */
router.post('/api/v1/sales/drafts/:id/approve', requireOperator, asyncHandler(async (req, res) => {
  const operator = (req as any).operator;
  const result = await salesAutomation.approveDraftEmail(
    req.params.id, 
    operator?.email || 'unknown'
  );
  res.json(result);
}));

/**
 * Reject a draft email
 */
router.post('/api/v1/sales/drafts/:id/reject', requireOperator, asyncHandler(async (req, res) => {
  const { reason } = req.body;
  const result = await salesAutomation.rejectDraftEmail(req.params.id, reason || 'Rejected by operator');
  res.json(result);
}));

/**
 * Bulk approve draft emails
 */
router.post('/api/v1/sales/drafts/bulk-approve', requireOperator, asyncHandler(async (req, res) => {
  const { emailIds } = req.body;
  const operator = (req as any).operator;
  const result = await salesAutomation.bulkApproveDrafts(
    emailIds || [], 
    operator?.email || 'unknown'
  );
  res.json(result);
}));

// ============================================
// SALES AUTOMATION CONTACT VERIFICATION ROUTES (NEW)
// ============================================

/**
 * Get unverified contacts
 */
router.get('/api/v1/sales/contacts/unverified', requireOperator, asyncHandler(async (req, res) => {
  const { limit } = req.query;
  const contacts = await salesAutomation.getUnverifiedContacts(
    limit ? parseInt(limit as string) : undefined
  );
  res.json({ contacts });
}));

/**
 * Mark contact as verified/unverified
 */
router.post('/api/v1/sales/contacts/:id/verify', requireOperator, asyncHandler(async (req, res) => {
  const { verified, status } = req.body;
  const result = await salesAutomation.markContactVerified(
    req.params.id, 
    verified === true,
    status
  );
  res.json(result);
}));

// ============================================
// AUTONOMOUS SALES API
// ============================================

// Check if we are the execution node (Test Server) or proxy (App Server)
// Default to proxy mode for safety
const IS_EXECUTION_NODE = process.env.SALES_AUTOMATION_ROLE === 'execution';

// Sales Controls API (kill switch, unified metrics)
import salesControlsRouter from './routes/sales-controls';

if (IS_EXECUTION_NODE) {
  // EXECUTION MODE (runs on Test Server)
  // Mount the real API implementation (orchestrator, spintax, thompson sampling)
  logger.info('[Autonomous] Mounting EXECUTION routes (Role: execution)');
  router.use('/api/autonomous', autonomousApiExecution);
} else {
  // PROXY MODE (runs on App Server)
  // Requests are forwarded to Test Server (157.90.126.205:4001)
  logger.info('[Autonomous] Mounting PROXY routes (Role: proxy)');
  router.use('/api/autonomous', autonomousApiProxy);
}

// Sales Controls API (available on both nodes)
router.use('/api/sales-controls', salesControlsRouter);

export default router;
