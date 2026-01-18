/**
 * SSAI API Router
 * Provides REST endpoints for SSAI management, debugging, and operability
 */

import { Router, Request, Response } from 'express';
import { SessionService } from '../services/ssai/session/SessionService';
import { sessionStore } from '../services/ssai/session/SessionStore';
import { SSAIHealthCheck } from '../services/ssai/operability/SSAIHealthCheck';
import { SSAIMetrics } from '../services/ssai/operability/SSAIMetrics';
import { SSAIDebugger } from '../services/ssai/operability/SSAIDebugger';
import { operatorSessionOnly } from '../middleware/operatorSession';
import { appApiKeyAuth } from '../middleware/appApiKeyAuth';
import type {
  ConsentPolicySnapshot,
  DeviceCapabilityProfile,
  SSAISession,
  SSAISessionConfig,
} from '../services/ssai/session/types';

const router = Router();

// Services
const sessionService = new SessionService();
const healthCheck = new SSAIHealthCheck();
const metrics = new SSAIMetrics();
const debugger_ = new SSAIDebugger();

const requireOperator = operatorSessionOnly();

// Feature flags for rollouts
const featureFlags = new Map<
  string,
  {
    enabled: boolean;
    percentage: number;
    tenants: string[];
  }
>();

// Kill switches
const killSwitches = new Map<
  string,
  {
    active: boolean;
    reason: string;
    activatedAt: number;
    activatedBy: string;
  }
>();

// Safe mode state
let safeModeActive = false;
let safeModeReason = '';

function buildDefaultConsent(): ConsentPolicySnapshot {
  return {
    gdprApplies: false,
    coppa: false,
    capturedAt: new Date().toISOString(),
    targeting: {
      personalized: false,
      contextual: true,
      crossDevice: false,
    },
  };
}

function buildDefaultDeviceProfile(protocol: 'hls' | 'dash'): DeviceCapabilityProfile {
  return {
    deviceType: 'web',
    protocol,
    container: protocol === 'hls' ? 'ts' : 'cmaf',
    maxResolution: '1080p',
    codecs: {
      video: ['avc1'],
      audio: ['aac'],
    },
    drm: { supported: false, systems: [] },
    hdr: { supported: false, types: [] },
    captions: { supported: false, formats: [] },
  };
}

function formatHourLabel(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-US', {
    hour: '2-digit',
    minute: '2-digit',
  });
}

function deviceLabel(deviceType: SSAISession['config']['deviceProfile']['deviceType']): string {
  switch (deviceType) {
    case 'ctv':
      return 'CTV';
    case 'mobile':
      return 'Mobile';
    case 'stb':
      return 'Set-Top Box';
    case 'web':
    default:
      return 'Web';
  }
}

// ==================
// Session Endpoints
// ==================

/**
 * Create new SSAI session (SDK-authenticated)
 */
router.post('/sessions', appApiKeyAuth, async (req: Request, res: Response) => {
  try {
    const body = (req.body ?? {}) as any;

    // Preferred payload: { config: SSAISessionConfig, ... }
    let config: SSAISessionConfig | null = null;
    if (body.config && typeof body.config === 'object') {
      config = body.config as SSAISessionConfig;
    }

    // Backward-compatible payload: { tenantId, assetId, protocol, options }
    if (!config) {
      const tenantId = typeof body.tenantId === 'string' ? body.tenantId : null;
      const assetId = typeof body.assetId === 'string' ? body.assetId : null;
      const protocolRaw = typeof body.protocol === 'string' ? body.protocol : null;
      const protocol = protocolRaw === 'dash' ? 'dash' : 'hls';
      const options = body.options && typeof body.options === 'object' ? body.options : {};

      if (!tenantId) {
        return res.status(400).json({ error: 'Missing required field: tenantId' });
      }

      const sdkTenant = (req as any).sdk?.publisherId;
      if (sdkTenant && tenantId !== sdkTenant) {
        return res.status(403).json({ error: 'tenantId does not match SDK key tenant' });
      }

      config = {
        tenantId: sdkTenant || tenantId,
        channelId: typeof options.channelId === 'string' ? options.channelId : assetId || 'unknown',
        assetId: assetId || undefined,
        streamType:
          typeof options.streamType === 'string' ? options.streamType : assetId ? 'vod' : 'live',
        deviceProfile:
          (options.deviceProfile as DeviceCapabilityProfile) || buildDefaultDeviceProfile(protocol),
        consent: (options.consent as ConsentPolicySnapshot) || buildDefaultConsent(),
        ttlSeconds: typeof options.ttlSeconds === 'number' ? options.ttlSeconds : undefined,
        metadata: options.metadata && typeof options.metadata === 'object' ? options.metadata : undefined,
      };
    }

    const idempotencyKey = (req.header('idempotency-key') || req.header('x-idempotency-key') || '')
      .toString()
      .trim();

    const session = await sessionService.createSession({
      config,
      clientIp: req.ip,
      userAgent: req.header('user-agent') || undefined,
      idempotencyKey: idempotencyKey || undefined,
    });

    if (session.created) {
      metrics.incrementCounter('ssai_sessions_created', {
        protocol: config.deviceProfile.protocol,
      });
    }
    metrics.setGauge('ssai_active_sessions', sessionStore.getActiveSessions().length);

    res.status(201).json(session);
  } catch {
    res.status(500).json({ error: 'Failed to create session' });
  }
});

/**
 * Get session by ID (token required)
 */
router.get('/sessions/:sessionId', async (req: Request, res: Response) => {
  try {
    const token = typeof req.query.token === 'string' ? req.query.token : undefined;
    if (!token) {
      return res.status(401).json({ error: 'Missing session token' });
    }

    const session = await sessionService.getSession(req.params.sessionId, token);

    if (!session) {
      return res.status(404).json({ error: 'Session not found' });
    }

    res.json(session);
  } catch {
    res.status(500).json({ error: 'Failed to get session' });
  }
});

/**
 * Delete session (operator only)
 */
router.delete('/sessions/:sessionId', requireOperator, async (req: Request, res: Response) => {
  try {
    const success = await sessionService.deleteSession(req.params.sessionId);

    if (!success) {
      return res.status(404).json({ error: 'Session not found' });
    }

    res.status(204).send();
    metrics.setGauge('ssai_active_sessions', sessionStore.getActiveSessions().length);
  } catch {
    res.status(500).json({ error: 'Failed to delete session' });
  }
});

// ==================
// Explain Endpoints
// ==================

router.get('/explain/pod/:podId', requireOperator, async (req: Request, res: Response) => {
  try {
    const explanation = await debugger_.explainPod(req.params.podId);
    res.json(explanation);
  } catch {
    res.status(500).json({ error: 'Failed to explain pod' });
  }
});

router.get('/explain/no-fill/:requestId', requireOperator, async (req: Request, res: Response) => {
  try {
    const explanation = await debugger_.explainNoFill(req.params.requestId);
    res.json(explanation);
  } catch {
    res.status(500).json({ error: 'Failed to explain no-fill' });
  }
});

router.get('/explain/trace/:sessionId/:podId', requireOperator, async (req: Request, res: Response) => {
  try {
    const trace = await debugger_.getDecisionTrace(req.params.sessionId, req.params.podId);
    res.json(trace);
  } catch {
    res.status(500).json({ error: 'Failed to get decision trace' });
  }
});

// ==================
// Kill Switch Endpoints
// ==================

router.get('/admin/kill-switches', requireOperator, (_req: Request, res: Response) => {
  const switches = Array.from(killSwitches.entries()).map(([key, value]) => ({ key, ...value }));
  res.json({ killSwitches: switches });
});

router.post('/admin/kill-switches/:key', requireOperator, (req: Request, res: Response) => {
  const { key } = req.params;
  const { reason } = (req.body ?? {}) as { reason?: string };
  const activatedBy = ((req as any).operator?.identifier as string) || (req.headers['x-user-id'] as string) || 'unknown';

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

router.delete('/admin/kill-switches/:key', requireOperator, (req: Request, res: Response) => {
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

router.get('/admin/rollouts', requireOperator, (_req: Request, res: Response) => {
  const flags = Array.from(featureFlags.entries()).map(([key, value]) => ({ key, ...value }));
  res.json({ rollouts: flags });
});

router.put('/admin/rollouts/:key', requireOperator, (req: Request, res: Response) => {
  const { key } = req.params;
  const { enabled, percentage, tenants } = (req.body ?? {}) as {
    enabled?: boolean;
    percentage?: number;
    tenants?: string[];
  };

  featureFlags.set(key, {
    enabled: enabled ?? true,
    percentage: typeof percentage === 'number' ? percentage : 100,
    tenants: Array.isArray(tenants) ? tenants : [],
  });

  res.json({
    key,
    ...featureFlags.get(key),
  });
});

router.delete('/admin/rollouts/:key', requireOperator, (req: Request, res: Response) => {
  const { key } = req.params;

  if (!featureFlags.has(key)) {
    return res.status(404).json({ error: 'Rollout flag not found' });
  }

  featureFlags.delete(key);
  res.json({ key, deleted: true });
});

router.get('/admin/rollouts/:key/check', requireOperator, (req: Request, res: Response) => {
  const { key } = req.params;
  const tenantId = typeof req.query.tenantId === 'string' ? req.query.tenantId : undefined;
  const sessionId = typeof req.query.sessionId === 'string' ? req.query.sessionId : undefined;

  const flag = featureFlags.get(key);
  if (!flag) return res.json({ eligible: false, reason: 'Flag not found' });
  if (!flag.enabled) return res.json({ eligible: false, reason: 'Flag disabled' });

  if (flag.tenants.length > 0 && tenantId && !flag.tenants.includes(tenantId)) {
    return res.json({ eligible: false, reason: 'Tenant not in allowlist' });
  }

  if (sessionId) {
    const bucket = Math.abs(hashCode(sessionId)) % 100;
    const eligible = bucket < flag.percentage;
    return res.json({ eligible, bucket, threshold: flag.percentage });
  }

  return res.json({ eligible: true });
});

// ==================
// Safe Mode Endpoints
// ==================

router.get('/admin/safe-mode', requireOperator, (_req: Request, res: Response) => {
  res.json({
    active: safeModeActive,
    reason: safeModeReason,
  });
});

router.post('/admin/safe-mode', requireOperator, (req: Request, res: Response) => {
  const { reason } = (req.body ?? {}) as { reason?: string };
  safeModeActive = true;
  safeModeReason = reason || 'Manual activation';

  metrics.incrementCounter('ssai_safe_mode_activated');

  res.json({
    active: true,
    message: 'Safe mode activated',
  });
});

router.delete('/admin/safe-mode', requireOperator, (_req: Request, res: Response) => {
  safeModeActive = false;
  safeModeReason = '';

  metrics.incrementCounter('ssai_safe_mode_deactivated');

  res.json({
    active: false,
    message: 'Safe mode deactivated',
  });
});

// ==================
// Operator Dashboard
// ==================

router.get('/admin/dashboard', requireOperator, (_req: Request, res: Response) => {
  const now = Date.now();
  const last24h = now - 24 * 60 * 60 * 1000;

  const allSessions = sessionStore.getAllSessions();
  const activeSessions = allSessions.filter((s) => s.state === 'active');
  const recentSessions = allSessions.filter((s) => new Date(s.createdAt).getTime() >= last24h);

  const avgSessionDurationSec = (() => {
    if (allSessions.length === 0) return 0;
    const durations = allSessions.map((s) => {
      const start = new Date(s.createdAt).getTime();
      const end = s.state === 'active'
        ? now
        : new Date(s.lastAccessedAt || s.expiresAt).getTime();
      return Math.max(0, Math.floor((end - start) / 1000));
    });
    const sum = durations.reduce((a, b) => a + b, 0);
    return Math.round(sum / durations.length);
  })();

  const fillRateEntries = metrics.getFillRateEntries();
  const latencyEntries = metrics.getDecisionLatencyEntries();
  const adapterMap = new Map<string, {
    name: string;
    status: 'active' | 'degraded' | 'inactive';
    latency_p95_ms: number;
    success_rate: number;
    requests_24h: number;
    revenue_24h: number;
    timeout_rate: number;
  }>();

  for (const entry of fillRateEntries) {
    const successRate = entry.fillRate;
    const status = entry.requested === 0
      ? 'inactive'
      : successRate >= 0.95
        ? 'active'
        : successRate >= 0.8
          ? 'degraded'
          : 'inactive';
    adapterMap.set(entry.partnerId, {
      name: entry.partnerId,
      status,
      latency_p95_ms: 0,
      success_rate: successRate,
      requests_24h: entry.requested,
      revenue_24h: 0,
      timeout_rate: 0,
    });
  }

  for (const latency of latencyEntries) {
    const existing = adapterMap.get(latency.partnerId) || {
      name: latency.partnerId,
      status: 'active' as const,
      latency_p95_ms: 0,
      success_rate: 1,
      requests_24h: 0,
      revenue_24h: 0,
      timeout_rate: 0,
    };
    existing.latency_p95_ms = Math.round(latency.p99Ms);
    adapterMap.set(latency.partnerId, existing);
  }

  const adapters = Array.from(adapterMap.values());

  const platformMetrics = Object.entries(
    recentSessions.reduce<Record<string, { sessions: number }>>((acc, session) => {
      const label = deviceLabel(session.config.deviceProfile.deviceType);
      if (!acc[label]) acc[label] = { sessions: 0 };
      acc[label].sessions += 1;
      return acc;
    }, {})
  ).map(([platform, stats]) => ({
    platform,
    sessions_24h: stats.sessions,
    fill_rate: fillRateEntries.length
      ? fillRateEntries.reduce((sum, e) => sum + e.fillRate, 0) / fillRateEntries.length
      : 0,
    completion_rate: 0,
    avg_latency_ms: latencyEntries.length
      ? Math.round(latencyEntries.reduce((sum, e) => sum + e.p99Ms, 0) / latencyEntries.length)
      : 0,
    revenue: 0,
  }));

  const bucketCounts = new Map<number, number>();
  for (const session of recentSessions) {
    const createdAt = new Date(session.createdAt).getTime();
    const bucket = Math.floor(createdAt / 3600000) * 3600000;
    bucketCounts.set(bucket, (bucketCounts.get(bucket) || 0) + 1);
  }

  const timeSeries = Array.from({ length: 24 }, (_, idx) => {
    const bucketStart = now - (23 - idx) * 3600000;
    const bucket = Math.floor(bucketStart / 3600000) * 3600000;
    return {
      time: formatHourLabel(bucketStart),
      sessions: bucketCounts.get(bucket) || 0,
      fillRate: fillRateEntries.length
        ? fillRateEntries.reduce((sum, e) => sum + e.fillRate, 0) / fillRateEntries.length
        : 0,
      revenue: 0,
    };
  });

  const beaconsFired = metrics.getCounter('ssai_beacons_fired_total');
  const beaconErrors = metrics.getCounter('ssai_beacon_errors_total');

  res.json({
    adapters,
    platformMetrics,
    timeSeries,
    ssai: {
      safeMode: {
        active: safeModeActive,
        reason: safeModeReason,
      },
      killSwitches: Array.from(killSwitches.entries()).map(([key, value]) => ({ key, ...value })),
      rollouts: Array.from(featureFlags.entries()).map(([key, value]) => ({ key, ...value })),
      counters: {
        ssai_safe_mode_activated: metrics.getCounter('ssai_safe_mode_activated'),
        ssai_safe_mode_deactivated: metrics.getCounter('ssai_safe_mode_deactivated'),
      },
      metrics: {
        total_sessions_24h: recentSessions.length,
        active_sessions: activeSessions.length,
        avg_session_duration_sec: avgSessionDurationSec,
        ad_fill_rate: fillRateEntries.length
          ? fillRateEntries.reduce((sum, e) => sum + e.fillRate, 0) / fillRateEntries.length
          : 0,
        beacons_fired_24h: beaconsFired,
        beacon_success_rate: beaconsFired > 0 ? (beaconsFired - beaconErrors) / beaconsFired : 0,
        midroll_completion_rate: 0,
        preroll_completion_rate: 0,
      },
    },
  });
});

// ==================
// Health Endpoints
// ==================

router.get('/health/live', (_req: Request, res: Response) => {
  const status = healthCheck.liveness();
  res.status(status.alive ? 200 : 503).json(status);
});

router.get('/health/ready', async (_req: Request, res: Response) => {
  const status = await healthCheck.readiness();
  res.status(status.ready ? 200 : 503).json(status);
});

router.get('/health', async (_req: Request, res: Response) => {
  const status = await healthCheck.check();
  res.status(status.healthy ? 200 : 503).json(status);
});

// ==================
// Metrics Endpoint
// ==================

router.get('/metrics', (_req: Request, res: Response) => {
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
    hash = (hash << 5) - hash + char;
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
    const bucket = Math.abs(hashCode(sessionId)) % 100;
    return bucket < flag.percentage;
  }

  return true;
}

export function isSafeModeActive(): boolean {
  return safeModeActive;
}

export default router;
