/**
 * SSAI API Router
 * Provides REST endpoints for SSAI management, debugging, and operability
 */

import { Router, Request, Response, NextFunction } from 'express';
import { SessionService } from '../services/ssai/session/SessionService';
import { SSAIHealthCheck } from '../services/ssai/operability/SSAIHealthCheck';
import { SSAIMetrics } from '../services/ssai/operability/SSAIMetrics';
import { SSAIDebugger } from '../services/ssai/operability/SSAIDebugger';

const router = Router();

// Services
const sessionService = new SessionService();
const healthCheck = new SSAIHealthCheck();
const metrics = new SSAIMetrics();
const debugger_ = new SSAIDebugger();

// Feature flags for rollouts
const featureFlags = new Map<string, {
  enabled: boolean;
  percentage: number;
  tenants: string[];
}>();

// Kill switches
const killSwitches = new Map<string, {
  active: boolean;
  reason: string;
  activatedAt: number;
  activatedBy: string;
}>();

// Safe mode state
let safeModeActive = false;
let safeModeReason = '';

/**
 * Authentication middleware for admin endpoints
 */
function requireAdmin(req: Request, res: Response, next: NextFunction) {
  const authHeader = req.headers.authorization;
  if (!authHeader || !authHeader.startsWith('Bearer ')) {
    return res.status(401).json({ error: 'Unauthorized' });
  }
  // In production, validate the token properly
  next();
}

// ==================
// Session Endpoints
// ==================

/**
 * Create new SSAI session
 */
router.post('/sessions', async (req: Request, res: Response) => {
  try {
    const { tenantId, assetId, protocol, options } = req.body;
    
    if (!tenantId || !assetId || !protocol) {
      return res.status(400).json({
        error: 'Missing required fields: tenantId, assetId, protocol'
      });
    }

    const session = await sessionService.createSession({
      tenantId,
      assetId,
      protocol,
      ...options,
    });

    res.status(201).json(session);
  } catch (error) {
    res.status(500).json({ error: 'Failed to create session' });
  }
});

/**
 * Get session by ID
 */
router.get('/sessions/:sessionId', async (req: Request, res: Response) => {
  try {
    const session = await sessionService.getSession(req.params.sessionId);
    
    if (!session) {
      return res.status(404).json({ error: 'Session not found' });
    }

    res.json(session);
  } catch (error) {
    res.status(500).json({ error: 'Failed to get session' });
  }
});

/**
 * Delete session
 */
router.delete('/sessions/:sessionId', async (req: Request, res: Response) => {
  try {
    const success = await sessionService.deleteSession(req.params.sessionId);
    
    if (!success) {
      return res.status(404).json({ error: 'Session not found' });
    }

    res.status(204).send();
  } catch (error) {
    res.status(500).json({ error: 'Failed to delete session' });
  }
});

// ==================
// Explain Endpoints
// ==================

/**
 * Explain pod decisioning
 */
router.get('/explain/pod/:podId', requireAdmin, async (req: Request, res: Response) => {
  try {
    const explanation = await debugger_.explainPod(req.params.podId);
    res.json(explanation);
  } catch (error) {
    res.status(500).json({ error: 'Failed to explain pod' });
  }
});

/**
 * Explain no-fill
 */
router.get('/explain/no-fill/:requestId', requireAdmin, async (req: Request, res: Response) => {
  try {
    const explanation = await debugger_.explainNoFill(req.params.requestId);
    res.json(explanation);
  } catch (error) {
    res.status(500).json({ error: 'Failed to explain no-fill' });
  }
});

/**
 * Get decision trace
 */
router.get('/explain/trace/:sessionId/:podId', requireAdmin, async (req: Request, res: Response) => {
  try {
    const trace = await debugger_.getDecisionTrace(
      req.params.sessionId,
      req.params.podId
    );
    res.json(trace);
  } catch (error) {
    res.status(500).json({ error: 'Failed to get decision trace' });
  }
});

// ==================
// Kill Switch Endpoints
// ==================

/**
 * Get all kill switches
 */
router.get('/admin/kill-switches', requireAdmin, (req: Request, res: Response) => {
  const switches = Array.from(killSwitches.entries()).map(([key, value]) => ({
    key,
    ...value,
  }));
  res.json({ killSwitches: switches });
});

/**
 * Activate kill switch
 */
router.post('/admin/kill-switches/:key', requireAdmin, (req: Request, res: Response) => {
  const { key } = req.params;
  const { reason } = req.body;
  const activatedBy = req.headers['x-user-id'] as string || 'unknown';

  killSwitches.set(key, {
    active: true,
    reason: reason || 'No reason provided',
    activatedAt: Date.now(),
    activatedBy,
  });

  metrics.incrementCounter('ssai_kill_switch_activated', { key });

  res.json({
    key,
    active: true,
    message: `Kill switch ${key} activated`,
  });
});

/**
 * Deactivate kill switch
 */
router.delete('/admin/kill-switches/:key', requireAdmin, (req: Request, res: Response) => {
  const { key } = req.params;
  
  if (!killSwitches.has(key)) {
    return res.status(404).json({ error: 'Kill switch not found' });
  }

  killSwitches.delete(key);
  metrics.incrementCounter('ssai_kill_switch_deactivated', { key });

  res.json({
    key,
    active: false,
    message: `Kill switch ${key} deactivated`,
  });
});

// ==================
// Rollout Endpoints
// ==================

/**
 * Get all rollout flags
 */
router.get('/admin/rollouts', requireAdmin, (req: Request, res: Response) => {
  const flags = Array.from(featureFlags.entries()).map(([key, value]) => ({
    key,
    ...value,
  }));
  res.json({ rollouts: flags });
});

/**
 * Create or update rollout flag
 */
router.put('/admin/rollouts/:key', requireAdmin, (req: Request, res: Response) => {
  const { key } = req.params;
  const { enabled, percentage, tenants } = req.body;

  featureFlags.set(key, {
    enabled: enabled ?? true,
    percentage: percentage ?? 100,
    tenants: tenants ?? [],
  });

  res.json({
    key,
    enabled: featureFlags.get(key)?.enabled,
    percentage: featureFlags.get(key)?.percentage,
    tenants: featureFlags.get(key)?.tenants,
  });
});

/**
 * Delete rollout flag
 */
router.delete('/admin/rollouts/:key', requireAdmin, (req: Request, res: Response) => {
  const { key } = req.params;
  
  if (!featureFlags.has(key)) {
    return res.status(404).json({ error: 'Rollout flag not found' });
  }

  featureFlags.delete(key);

  res.json({
    key,
    deleted: true,
  });
});

/**
 * Check rollout eligibility
 */
router.get('/admin/rollouts/:key/check', requireAdmin, (req: Request, res: Response) => {
  const { key } = req.params;
  const { tenantId, sessionId } = req.query;

  const flag = featureFlags.get(key);
  
  if (!flag) {
    return res.json({ eligible: false, reason: 'Flag not found' });
  }

  if (!flag.enabled) {
    return res.json({ eligible: false, reason: 'Flag disabled' });
  }

  // Check tenant whitelist
  if (flag.tenants.length > 0 && tenantId && !flag.tenants.includes(tenantId as string)) {
    return res.json({ eligible: false, reason: 'Tenant not in allowlist' });
  }

  // Percentage-based rollout
  if (sessionId) {
    const hash = hashCode(sessionId as string);
    const bucket = Math.abs(hash) % 100;
    const eligible = bucket < flag.percentage;
    return res.json({ eligible, bucket, threshold: flag.percentage });
  }

  res.json({ eligible: true });
});

// ==================
// Safe Mode Endpoints
// ==================

/**
 * Get safe mode status
 */
router.get('/admin/safe-mode', requireAdmin, (req: Request, res: Response) => {
  res.json({
    active: safeModeActive,
    reason: safeModeReason,
  });
});

/**
 * Activate safe mode
 */
router.post('/admin/safe-mode', requireAdmin, (req: Request, res: Response) => {
  const { reason } = req.body;
  safeModeActive = true;
  safeModeReason = reason || 'Manual activation';
  
  metrics.incrementCounter('ssai_safe_mode_activated');

  res.json({
    active: true,
    message: 'Safe mode activated',
  });
});

/**
 * Deactivate safe mode
 */
router.delete('/admin/safe-mode', requireAdmin, (req: Request, res: Response) => {
  safeModeActive = false;
  safeModeReason = '';
  
  metrics.incrementCounter('ssai_safe_mode_deactivated');

  res.json({
    active: false,
    message: 'Safe mode deactivated',
  });
});

// ==================
// Health Endpoints
// ==================

/**
 * Liveness probe
 */
router.get('/health/live', (req: Request, res: Response) => {
  const status = healthCheck.liveness();
  res.status(status.alive ? 200 : 503).json(status);
});

/**
 * Readiness probe
 */
router.get('/health/ready', async (req: Request, res: Response) => {
  const status = await healthCheck.readiness();
  res.status(status.ready ? 200 : 503).json(status);
});

/**
 * Detailed health check
 */
router.get('/health', async (req: Request, res: Response) => {
  const status = await healthCheck.check();
  res.status(status.healthy ? 200 : 503).json(status);
});

// ==================
// Metrics Endpoint
// ==================

/**
 * Prometheus metrics
 */
router.get('/metrics', (req: Request, res: Response) => {
  res.set('Content-Type', 'text/plain; version=0.0.4');
  res.send(metrics.exportPrometheus());
});

// ==================
// Helper Functions
// ==================

function hashCode(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i);
    hash = ((hash << 5) - hash) + char;
    hash = hash & hash;
  }
  return hash;
}

// Export functions for testing
export function isKillSwitchActive(key: string): boolean {
  return killSwitches.get(key)?.active ?? false;
}

export function isRolloutEnabled(key: string, tenantId?: string, sessionId?: string): boolean {
  const flag = featureFlags.get(key);
  if (!flag || !flag.enabled) return false;
  
  if (flag.tenants.length > 0 && tenantId && !flag.tenants.includes(tenantId)) {
    return false;
  }
  
  if (sessionId) {
    const hash = hashCode(sessionId);
    const bucket = Math.abs(hash) % 100;
    return bucket < flag.percentage;
  }
  
  return true;
}

export function isSafeModeActive(): boolean {
  return safeModeActive;
}

export default router;
