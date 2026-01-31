/**
 * Redis-backed SSAI Session Store
 *
 * Provides distributed session storage for high availability.
 * Sessions are automatically expired via Redis TTL.
 */

import crypto from 'crypto';
import { redis } from '../../../utils/redis';
import logger from '../../../utils/logger';
import {
  SSAISession,
  SSAISessionConfig,
  SessionLookupOptions,
  SessionTTLConfig,
  DEFAULT_TTL_CONFIG,
} from './types';
import { ISessionStore } from './SessionStore';

// Redis key prefixes for SSAI sessions
const SESSION_KEY_PREFIX = 'ssai:session:';
const IDEMPOTENCY_KEY_PREFIX = 'ssai:idempotency:';
const TENANT_SESSIONS_PREFIX = 'ssai:tenant:';

/**
 * Redis-backed session store for production use
 * Provides distributed session storage with automatic TTL expiration
 */
export class RedisSessionStore implements ISessionStore {
  private ttlConfig: SessionTTLConfig;

  constructor(ttlConfig: SessionTTLConfig = DEFAULT_TTL_CONFIG) {
    this.ttlConfig = ttlConfig;
  }

  /**
   * Generate a unique session ID
   */
  private generateSessionId(config: SSAISessionConfig): string {
    const components = [
      config.tenantId,
      config.channelId,
      config.assetId || '',
      config.deviceProfile.deviceType,
      config.deviceProfile.protocol,
      config.deviceProfile.container,
      Date.now().toString(),
      crypto.randomBytes(8).toString('hex'),
    ];

    const hash = crypto
      .createHash('sha256')
      .update(components.join(':'))
      .digest('hex');

    return `ssai_${hash.substring(0, 32)}`;
  }

  /**
   * Generate a signed session token
   */
  private generateToken(sessionId: string): string {
    // SECURITY: Require SSAI_TOKEN_SECRET in production (P1 Fix #1)
    const secret = process.env.SSAI_TOKEN_SECRET;
    if (!secret && process.env.NODE_ENV === 'production') {
      throw new Error('SSAI_TOKEN_SECRET must be set in production');
    }
    const signingSecret = secret || 'dev-secret-not-for-production';

    const payload = {
      sid: sessionId,
      iat: Date.now(),
      exp: Date.now() + this.ttlConfig.tokenTtlSeconds * 1000,
    };

    const payloadStr = JSON.stringify(payload);
    const signature = crypto
      .createHmac('sha256', signingSecret)
      .update(payloadStr)
      .digest('hex');

    return Buffer.from(`${payloadStr}.${signature}`).toString('base64url');
  }

  /**
   * Calculate TTL based on stream type
   */
  private getTtlSeconds(streamType: 'live' | 'vod' | 'event'): number {
    switch (streamType) {
      case 'live':
        return this.ttlConfig.liveTtlSeconds;
      case 'vod':
        return this.ttlConfig.vodTtlSeconds;
      case 'event':
        return this.ttlConfig.eventTtlSeconds;
      default:
        return this.ttlConfig.liveTtlSeconds;
    }
  }

  /**
   * Create a new session
   */
  async create(config: SSAISessionConfig, idempotencyKey?: string): Promise<SSAISession> {
    // Check idempotency
    if (idempotencyKey) {
      const existing = await this.getByIdempotencyKey(idempotencyKey);
      if (existing && existing.state === 'active') {
        logger.debug('[RedisSessionStore] Returning existing session via idempotency key', {
          idempotencyKey,
          sessionId: existing.sessionId,
        });
        return existing;
      }
    }

    const sessionId = this.generateSessionId(config);
    const now = new Date().toISOString();
    const ttlSeconds = config.ttlSeconds || this.getTtlSeconds(config.streamType);
    const expiresAt = new Date(Date.now() + ttlSeconds * 1000).toISOString();
    const tokenExpiresAt = new Date(
      Date.now() + this.ttlConfig.tokenTtlSeconds * 1000
    ).toISOString();

    const session: SSAISession = {
      sessionId,
      config,
      state: 'active',
      createdAt: now,
      lastAccessedAt: now,
      expiresAt,
      manifestRequests: 0,
      manifestVersion: 1,
      activePods: [],
      completedPods: [],
      token: this.generateToken(sessionId),
      tokenExpiresAt,
    };

    // Store session in Redis with TTL
    const stored = await redis.set(
      `${SESSION_KEY_PREFIX}${sessionId}`,
      session,
      ttlSeconds
    );

    if (!stored) {
      logger.error('[RedisSessionStore] Failed to store session in Redis', { sessionId });
      throw new Error('Failed to create SSAI session');
    }

    // Store idempotency key with 1 hour TTL
    if (idempotencyKey) {
      await redis.set(
        `${IDEMPOTENCY_KEY_PREFIX}${idempotencyKey}`,
        sessionId,
        3600
      );
    }

    // Track session in tenant set for admin queries
    // Using a simple approach - store sessionId as value (can enhance with SADD later)
    await redis.set(
      `${TENANT_SESSIONS_PREFIX}${config.tenantId}:${sessionId}`,
      { sessionId, createdAt: now },
      ttlSeconds
    );

    logger.info('[RedisSessionStore] Session created', {
      sessionId,
      tenantId: config.tenantId,
      streamType: config.streamType,
      ttlSeconds,
    });

    return session;
  }

  /**
   * Get a session by ID
   */
  async get(sessionId: string, options?: SessionLookupOptions): Promise<SSAISession | null> {
    const session = await redis.get<SSAISession>(`${SESSION_KEY_PREFIX}${sessionId}`);

    if (!session) {
      return null;
    }

    // Check if expired (defensive check - Redis TTL should handle this)
    const isExpired = new Date(session.expiresAt) < new Date();

    if (isExpired && !options?.includeExpired) {
      return null;
    }

    if (isExpired) {
      session.state = 'expired';
    }

    return session;
  }

  /**
   * Refresh a session (extend TTL)
   */
  async refresh(sessionId: string): Promise<SSAISession | null> {
    const session = await this.get(sessionId);

    if (!session || session.state !== 'active') {
      return null;
    }

    const now = new Date();
    const ttlSeconds = this.getTtlSeconds(session.config.streamType);

    session.lastAccessedAt = now.toISOString();
    session.expiresAt = new Date(now.getTime() + ttlSeconds * 1000).toISOString();
    session.manifestRequests++;

    // Refresh token if needed
    const tokenExpiry = new Date(session.tokenExpiresAt);
    const tokenThreshold = new Date(
      now.getTime() + this.ttlConfig.refreshThresholdSeconds * 1000
    );

    if (tokenExpiry < tokenThreshold) {
      session.token = this.generateToken(sessionId);
      session.tokenExpiresAt = new Date(
        now.getTime() + this.ttlConfig.tokenTtlSeconds * 1000
      ).toISOString();
    }

    // Update in Redis with refreshed TTL
    await redis.set(`${SESSION_KEY_PREFIX}${sessionId}`, session, ttlSeconds);

    return session;
  }

  /**
   * Terminate a session
   */
  async terminate(sessionId: string): Promise<boolean> {
    const session = await this.get(sessionId);

    if (!session) {
      return false;
    }

    session.state = 'terminated';
    
    // Keep terminated session for 5 minutes for debugging
    await redis.set(`${SESSION_KEY_PREFIX}${sessionId}`, session, 300);

    logger.info('[RedisSessionStore] Session terminated', { sessionId });
    return true;
  }

  /**
   * Update manifest version
   */
  async updateManifestVersion(sessionId: string): Promise<number> {
    const session = await this.get(sessionId);

    if (!session) {
      return -1;
    }

    session.manifestVersion++;
    session.lastAccessedAt = new Date().toISOString();

    const ttlSeconds = this.getTtlSeconds(session.config.streamType);
    await redis.set(`${SESSION_KEY_PREFIX}${sessionId}`, session, ttlSeconds);

    return session.manifestVersion;
  }

  /**
   * Add an active pod to the session
   */
  async addActivePod(sessionId: string, podId: string): Promise<void> {
    const session = await this.get(sessionId);

    if (session && !session.activePods.includes(podId)) {
      session.activePods.push(podId);
      const ttlSeconds = this.getTtlSeconds(session.config.streamType);
      await redis.set(`${SESSION_KEY_PREFIX}${sessionId}`, session, ttlSeconds);
    }
  }

  /**
   * Complete a pod (move from active to completed)
   */
  async completePod(sessionId: string, podId: string): Promise<void> {
    const session = await this.get(sessionId);

    if (session) {
      session.activePods = session.activePods.filter((id) => id !== podId);
      if (!session.completedPods.includes(podId)) {
        session.completedPods.push(podId);
      }
      const ttlSeconds = this.getTtlSeconds(session.config.streamType);
      await redis.set(`${SESSION_KEY_PREFIX}${sessionId}`, session, ttlSeconds);
    }
  }

  /**
   * Get session by idempotency key
   */
  async getByIdempotencyKey(key: string): Promise<SSAISession | null> {
    const sessionId = await redis.get<string>(`${IDEMPOTENCY_KEY_PREFIX}${key}`);

    if (!sessionId) {
      return null;
    }

    return this.get(sessionId);
  }

  /**
   * Get all sessions (for metrics/admin)
   * Note: In Redis, this requires SCAN which can be slow for large datasets
   * Consider using sorted sets for production metrics
   */
  getAllSessions(): SSAISession[] {
    // This is a synchronous method in the interface for compatibility
    // For Redis, we return an empty array and recommend using getActiveSessions() async
    logger.warn('[RedisSessionStore] getAllSessions() is not efficient for Redis, use async methods');
    return [];
  }

  /**
   * Get active sessions (for metrics)
   * For Redis, this returns an empty array as it requires async SCAN
   * Use getActiveSessionCount() for metrics instead
   */
  getActiveSessions(): SSAISession[] {
    // This is a synchronous method in the interface for compatibility
    // For Redis, we return an empty array and recommend using async methods
    logger.warn('[RedisSessionStore] getActiveSessions() returns empty for Redis, use getActiveSessionCount() async');
    return [];
  }

  /**
   * Get active sessions count (for metrics)
   * This is more efficient than getAllSessions for Redis
   */
  async getActiveSessionCount(): Promise<number> {
    // Use Redis KEYS with caution - in production, use SCAN or maintain a counter
    // This is a simple implementation for metrics
    if (!redis.isReady()) {
      return 0;
    }

    // For now, return 0 and recommend implementing with Redis SCAN or a counter
    // A full implementation would use: redis.client.scan(0, 'MATCH', SESSION_KEY_PREFIX + '*')
    logger.debug('[RedisSessionStore] getActiveSessionCount not implemented for Redis SCAN');
    return 0;
  }

  /**
   * Check if the Redis store is available
   */
  isReady(): boolean {
    return redis.isReady();
  }
}

// Factory function to get appropriate session store
export function createSessionStore(ttlConfig?: SessionTTLConfig): ISessionStore {
  const useRedis = process.env.SSAI_SESSION_STORE !== 'memory' && redis.isReady();
  
  if (useRedis) {
    logger.info('[SessionStore] Using Redis-backed session store');
    return new RedisSessionStore(ttlConfig);
  } else {
    logger.info('[SessionStore] Using in-memory session store (development/fallback)');
    // Import dynamically to avoid circular dependency issues
    const { InMemorySessionStore } = require('./SessionStore');
    return new InMemorySessionStore(ttlConfig);
  }
}
