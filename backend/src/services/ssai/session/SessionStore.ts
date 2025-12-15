/**
 * SSAI Session Store
 *
 * Persistence layer for SSAI sessions. Uses Redis for distributed
 * session storage with TTL support.
 */

import crypto from 'crypto';
import {
  SSAISession,
  SSAISessionConfig,
  SSAISessionState,
  SessionLookupOptions,
  SessionTTLConfig,
  DEFAULT_TTL_CONFIG,
} from './types';

/**
 * Session store interface for abstraction
 */
export interface ISessionStore {
  create(config: SSAISessionConfig, idempotencyKey?: string): Promise<SSAISession>;
  get(sessionId: string, options?: SessionLookupOptions): Promise<SSAISession | null>;
  refresh(sessionId: string): Promise<SSAISession | null>;
  terminate(sessionId: string): Promise<boolean>;
  updateManifestVersion(sessionId: string): Promise<number>;
  addActivePod(sessionId: string, podId: string): Promise<void>;
  completePod(sessionId: string, podId: string): Promise<void>;
  getByIdempotencyKey(key: string): Promise<SSAISession | null>;
}

/**
 * In-memory session store for development/testing
 * In production, use Redis-backed implementation
 */
export class InMemorySessionStore implements ISessionStore {
  private sessions: Map<string, SSAISession> = new Map();
  private idempotencyIndex: Map<string, string> = new Map();
  private ttlConfig: SessionTTLConfig;

  constructor(ttlConfig: SessionTTLConfig = DEFAULT_TTL_CONFIG) {
    this.ttlConfig = ttlConfig;

    // Cleanup expired sessions every minute
    setInterval(() => this.cleanupExpired(), 60000);
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
    const payload = {
      sid: sessionId,
      iat: Date.now(),
      exp: Date.now() + this.ttlConfig.tokenTtlSeconds * 1000,
    };

    const payloadStr = JSON.stringify(payload);
    const signature = crypto
      .createHmac('sha256', process.env.SSAI_TOKEN_SECRET || 'dev-secret')
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

    this.sessions.set(sessionId, session);

    if (idempotencyKey) {
      this.idempotencyIndex.set(idempotencyKey, sessionId);
    }

    return session;
  }

  /**
   * Get a session by ID
   */
  async get(sessionId: string, options?: SessionLookupOptions): Promise<SSAISession | null> {
    const session = this.sessions.get(sessionId);

    if (!session) {
      return null;
    }

    // Check if expired
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
    const session = this.sessions.get(sessionId);

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

    return session;
  }

  /**
   * Terminate a session
   */
  async terminate(sessionId: string): Promise<boolean> {
    const session = this.sessions.get(sessionId);

    if (!session) {
      return false;
    }

    session.state = 'terminated';
    return true;
  }

  /**
   * Update manifest version
   */
  async updateManifestVersion(sessionId: string): Promise<number> {
    const session = this.sessions.get(sessionId);

    if (!session) {
      return -1;
    }

    session.manifestVersion++;
    session.lastAccessedAt = new Date().toISOString();

    return session.manifestVersion;
  }

  /**
   * Add an active pod to the session
   */
  async addActivePod(sessionId: string, podId: string): Promise<void> {
    const session = this.sessions.get(sessionId);

    if (session && !session.activePods.includes(podId)) {
      session.activePods.push(podId);
    }
  }

  /**
   * Complete a pod (move from active to completed)
   */
  async completePod(sessionId: string, podId: string): Promise<void> {
    const session = this.sessions.get(sessionId);

    if (session) {
      session.activePods = session.activePods.filter((id) => id !== podId);
      if (!session.completedPods.includes(podId)) {
        session.completedPods.push(podId);
      }
    }
  }

  /**
   * Get session by idempotency key
   */
  async getByIdempotencyKey(key: string): Promise<SSAISession | null> {
    const sessionId = this.idempotencyIndex.get(key);

    if (!sessionId) {
      return null;
    }

    return this.get(sessionId);
  }

  /**
   * Cleanup expired sessions
   */
  private cleanupExpired(): void {
    const now = new Date();

    for (const [sessionId, session] of this.sessions.entries()) {
      if (new Date(session.expiresAt) < now) {
        session.state = 'expired';
        // Keep for a grace period, then delete
        const gracePeriod = 5 * 60 * 1000; // 5 minutes
        setTimeout(() => {
          if (this.sessions.get(sessionId)?.state === 'expired') {
            this.sessions.delete(sessionId);
          }
        }, gracePeriod);
      }
    }

    // Cleanup old idempotency keys (keep for 1 hour)
    // In production, use Redis TTL for this
  }

  /**
   * Get all active sessions (for metrics)
   */
  getActiveSessions(): SSAISession[] {
    return Array.from(this.sessions.values()).filter(
      (s) => s.state === 'active'
    );
  }

  /**
   * Get session count by tenant
   */
  getSessionCountByTenant(): Record<string, number> {
    const counts: Record<string, number> = {};

    for (const session of this.sessions.values()) {
      if (session.state === 'active') {
        const tenantId = session.config.tenantId;
        counts[tenantId] = (counts[tenantId] || 0) + 1;
      }
    }

    return counts;
  }
}

// Export singleton instance
export const sessionStore = new InMemorySessionStore();
