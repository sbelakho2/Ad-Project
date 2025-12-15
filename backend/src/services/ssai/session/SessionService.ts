/**
 * SSAI Session Service
 *
 * Main service for managing SSAI sessions. Handles session creation,
 * validation, rate limiting, and lifecycle management.
 */

import crypto from 'crypto';
import {
  SSAISession,
  SSAISessionConfig,
  CreateSessionRequest,
  CreateSessionResponse,
  RefreshSessionRequest,
  SessionRateLimitConfig,
  DEFAULT_RATE_LIMIT_CONFIG,
  SessionMetrics,
} from './types';
import { sessionStore, ISessionStore } from './SessionStore';

/**
 * Rate limiter entry
 */
interface RateLimitEntry {
  count: number;
  windowStart: number;
}

/**
 * SSAI Session Service
 */
export class SessionService {
  private store: ISessionStore;
  private rateLimitConfig: SessionRateLimitConfig;

  // In-memory rate limiters (use Redis in production)
  private tenantRateLimits: Map<string, RateLimitEntry> = new Map();
  private ipRateLimits: Map<string, RateLimitEntry> = new Map();
  private sessionRequestRateLimits: Map<string, number[]> = new Map();

  constructor(
    store: ISessionStore = sessionStore,
    rateLimitConfig: SessionRateLimitConfig = DEFAULT_RATE_LIMIT_CONFIG
  ) {
    this.store = store;
    this.rateLimitConfig = rateLimitConfig;

    // Cleanup rate limit entries every minute
    setInterval(() => this.cleanupRateLimits(), 60000);
  }

  /**
   * Create a new SSAI session
   */
  async createSession(request: CreateSessionRequest): Promise<CreateSessionResponse> {
    const { config, clientIp, idempotencyKey } = request;

    // Rate limit check - tenant
    if (!this.checkTenantRateLimit(config.tenantId)) {
      throw new SessionRateLimitError(
        `Tenant ${config.tenantId} has exceeded session creation rate limit`,
        'tenant_rate_limit'
      );
    }

    // Rate limit check - IP
    if (clientIp && !this.checkIpRateLimit(clientIp)) {
      throw new SessionRateLimitError(
        `IP ${clientIp} has exceeded session creation rate limit`,
        'ip_rate_limit'
      );
    }

    // Check for existing session via idempotency key
    if (idempotencyKey) {
      const existing = await this.store.getByIdempotencyKey(idempotencyKey);
      if (existing && existing.state === 'active') {
        return {
          created: false,
          session: existing,
          manifestUrls: this.generateManifestUrls(existing),
        };
      }
    }

    // Create new session
    const session = await this.store.create(config, idempotencyKey);

    return {
      created: true,
      session,
      manifestUrls: this.generateManifestUrls(session),
    };
  }

  /**
   * Get a session by ID
   */
  async getSession(sessionId: string, token?: string): Promise<SSAISession | null> {
    const session = await this.store.get(sessionId);

    if (!session) {
      return null;
    }

    // Validate token if provided
    if (token && !this.validateToken(session, token)) {
      throw new SessionAuthError('Invalid session token');
    }

    return session;
  }

  /**
   * Refresh a session (called on manifest request)
   */
  async refreshSession(request: RefreshSessionRequest): Promise<SSAISession | null> {
    const { sessionId, token } = request;

    const session = await this.store.get(sessionId);

    if (!session) {
      return null;
    }

    // Validate token
    if (!this.validateToken(session, token)) {
      throw new SessionAuthError('Invalid session token');
    }

    // Check for retry storm
    if (this.isRetryStorm(sessionId)) {
      throw new SessionRateLimitError(
        'Retry storm detected - too many manifest requests',
        'retry_storm'
      );
    }

    // Record request for retry storm detection
    this.recordManifestRequest(sessionId);

    // Refresh session
    return this.store.refresh(sessionId);
  }

  /**
   * Terminate a session
   */
  async terminateSession(sessionId: string, token: string): Promise<boolean> {
    const session = await this.store.get(sessionId);

    if (!session) {
      return false;
    }

    if (!this.validateToken(session, token)) {
      throw new SessionAuthError('Invalid session token');
    }

    return this.store.terminate(sessionId);
  }

  /**
   * Delete a session (admin operation - no token required)
   */
  async deleteSession(sessionId: string): Promise<boolean> {
    const session = await this.store.get(sessionId);

    if (!session) {
      return false;
    }

    return this.store.terminate(sessionId);
  }

  /**
   * Update manifest version (called when manifest changes)
   */
  async updateManifestVersion(sessionId: string): Promise<number> {
    return this.store.updateManifestVersion(sessionId);
  }

  /**
   * Add an active pod to session
   */
  async addPod(sessionId: string, podId: string): Promise<void> {
    return this.store.addActivePod(sessionId, podId);
  }

  /**
   * Complete a pod in session
   */
  async completePod(sessionId: string, podId: string): Promise<void> {
    return this.store.completePod(sessionId, podId);
  }

  /**
   * Generate manifest URLs for a session
   */
  private generateManifestUrls(session: SSAISession): { master: string; media?: string[] } {
    const baseUrl = process.env.SSAI_MANIFEST_BASE_URL || 'https://ssai.apexmediation.ee';
    const { protocol, container } = session.config.deviceProfile;

    if (protocol === 'hls') {
      const extension = container === 'ts' ? 'm3u8' : 'm3u8';
      return {
        master: `${baseUrl}/v1/session/${session.sessionId}/master.${extension}?token=${session.token}`,
      };
    } else {
      return {
        master: `${baseUrl}/v1/session/${session.sessionId}/manifest.mpd?token=${session.token}`,
      };
    }
  }

  /**
   * Validate session token
   */
  private validateToken(session: SSAISession, token: string): boolean {
    if (session.token !== token) {
      return false;
    }

    if (new Date(session.tokenExpiresAt) < new Date()) {
      return false;
    }

    return true;
  }

  /**
   * Check tenant rate limit
   */
  private checkTenantRateLimit(tenantId: string): boolean {
    const now = Date.now();
    const windowMs = 60000; // 1 minute
    const entry = this.tenantRateLimits.get(tenantId);

    if (!entry || now - entry.windowStart > windowMs) {
      this.tenantRateLimits.set(tenantId, { count: 1, windowStart: now });
      return true;
    }

    if (entry.count >= this.rateLimitConfig.maxSessionsPerTenantPerMinute) {
      return false;
    }

    entry.count++;
    return true;
  }

  /**
   * Check IP rate limit
   */
  private checkIpRateLimit(ip: string): boolean {
    const now = Date.now();
    const windowMs = 60000; // 1 minute
    const entry = this.ipRateLimits.get(ip);

    if (!entry || now - entry.windowStart > windowMs) {
      this.ipRateLimits.set(ip, { count: 1, windowStart: now });
      return true;
    }

    if (entry.count >= this.rateLimitConfig.maxSessionsPerIpPerMinute) {
      return false;
    }

    entry.count++;
    return true;
  }

  /**
   * Record manifest request for retry storm detection
   */
  private recordManifestRequest(sessionId: string): void {
    const now = Date.now();
    const requests = this.sessionRequestRateLimits.get(sessionId) || [];

    // Add current request
    requests.push(now);

    // Keep only requests within window
    const windowStart = now - this.rateLimitConfig.retryStormWindowMs;
    const filtered = requests.filter((t) => t > windowStart);

    this.sessionRequestRateLimits.set(sessionId, filtered);
  }

  /**
   * Check if session is in retry storm
   */
  private isRetryStorm(sessionId: string): boolean {
    const now = Date.now();
    const requests = this.sessionRequestRateLimits.get(sessionId) || [];

    // Count requests in window
    const windowStart = now - this.rateLimitConfig.retryStormWindowMs;
    const recentRequests = requests.filter((t) => t > windowStart);

    return recentRequests.length >= this.rateLimitConfig.retryStormThreshold;
  }

  /**
   * Cleanup old rate limit entries
   */
  private cleanupRateLimits(): void {
    const now = Date.now();
    const windowMs = 60000;

    // Cleanup tenant limits
    for (const [tenantId, entry] of this.tenantRateLimits.entries()) {
      if (now - entry.windowStart > windowMs * 2) {
        this.tenantRateLimits.delete(tenantId);
      }
    }

    // Cleanup IP limits
    for (const [ip, entry] of this.ipRateLimits.entries()) {
      if (now - entry.windowStart > windowMs * 2) {
        this.ipRateLimits.delete(ip);
      }
    }

    // Cleanup session request tracking
    const stormWindow = this.rateLimitConfig.retryStormWindowMs;
    for (const [sessionId, requests] of this.sessionRequestRateLimits.entries()) {
      const filtered = requests.filter((t) => t > now - stormWindow);
      if (filtered.length === 0) {
        this.sessionRequestRateLimits.delete(sessionId);
      } else {
        this.sessionRequestRateLimits.set(sessionId, filtered);
      }
    }
  }

  /**
   * Get session metrics
   */
  async getMetrics(): Promise<SessionMetrics> {
    // This would query the store for metrics
    // Simplified implementation for now
    return {
      activeSessions: 0,
      sessionsByTenant: {},
      sessionsByStreamType: {},
      sessionsCreatedLastMinute: 0,
      sessionCreationRate: 0,
      avgSessionDurationSeconds: 0,
      rateLimitRejections: 0,
    };
  }
}

/**
 * Session rate limit error
 */
export class SessionRateLimitError extends Error {
  public code: string;

  constructor(message: string, code: string) {
    super(message);
    this.name = 'SessionRateLimitError';
    this.code = code;
  }
}

/**
 * Session authentication error
 */
export class SessionAuthError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'SessionAuthError';
  }
}

// Export singleton instance
export const sessionService = new SessionService();
