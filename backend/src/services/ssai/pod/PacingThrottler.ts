/**
 * Pacing Throttler
 *
 * Implements per-partner pacing and per-tenant throttling
 * to ensure fair distribution of ad opportunities and prevent
 * partner overload.
 */

export interface PacingConfig {
  /** Target requests per second per partner */
  partnerRpsLimit: number;

  /** Target requests per second per tenant */
  tenantRpsLimit: number;

  /** Window size for rate calculation (ms) */
  windowSizeMs: number;

  /** Burst allowance multiplier */
  burstMultiplier: number;

  /** Smoothing factor for exponential moving average */
  smoothingFactor: number;
}

export interface PacingState {
  partnerId: string;
  requestCount: number;
  windowStart: number;
  currentRps: number;
  throttled: boolean;
  throttledUntil?: number;
}

export interface ThrottleResult {
  allowed: boolean;
  waitMs?: number;
  reason?: string;
  currentRps: number;
  limit: number;
}

export interface PacingMetrics {
  totalRequests: number;
  throttledRequests: number;
  averageRps: number;
  peakRps: number;
  partnerMetrics: Map<string, PartnerPacingMetrics>;
}

export interface PartnerPacingMetrics {
  partnerId: string;
  totalRequests: number;
  throttledRequests: number;
  averageRps: number;
  peakRps: number;
}

/**
 * Pacing Throttler for controlling request rates to partners and tenants.
 */
export class PacingThrottler {
  private config: PacingConfig;
  private partnerStates: Map<string, PacingState> = new Map();
  private tenantStates: Map<string, PacingState> = new Map();
  private metrics: PacingMetrics;

  constructor(config?: Partial<PacingConfig>) {
    this.config = {
      partnerRpsLimit: 100,
      tenantRpsLimit: 1000,
      windowSizeMs: 1000,
      burstMultiplier: 1.5,
      smoothingFactor: 0.3,
      ...config,
    };

    this.metrics = {
      totalRequests: 0,
      throttledRequests: 0,
      averageRps: 0,
      peakRps: 0,
      partnerMetrics: new Map(),
    };
  }

  /**
   * Check if a request to a partner should be allowed.
   */
  checkPartnerPacing(partnerId: string): ThrottleResult {
    const now = Date.now();
    let state = this.partnerStates.get(partnerId);

    if (!state) {
      state = {
        partnerId,
        requestCount: 0,
        windowStart: now,
        currentRps: 0,
        throttled: false,
      };
      this.partnerStates.set(partnerId, state);
    }

    // Check if we're in a new window
    if (now - state.windowStart >= this.config.windowSizeMs) {
      // Calculate RPS for the completed window
      const windowDurationSec = (now - state.windowStart) / 1000;
      const rps = state.requestCount / windowDurationSec;

      // Update with exponential moving average
      state.currentRps =
        this.config.smoothingFactor * rps +
        (1 - this.config.smoothingFactor) * state.currentRps;

      // Reset window
      state.requestCount = 0;
      state.windowStart = now;
      state.throttled = false;
    }

    // Check if throttled
    if (state.throttled && state.throttledUntil && now < state.throttledUntil) {
      return {
        allowed: false,
        waitMs: state.throttledUntil - now,
        reason: `Partner ${partnerId} is throttled`,
        currentRps: state.currentRps,
        limit: this.config.partnerRpsLimit,
      };
    }

    // Calculate instantaneous RPS
    const windowDurationSec = Math.max((now - state.windowStart) / 1000, 0.001);
    const instantRps = state.requestCount / windowDurationSec;

    // Check against limit with burst allowance
    const effectiveLimit = this.config.partnerRpsLimit * this.config.burstMultiplier;

    if (instantRps >= effectiveLimit) {
      // Throttle for the remainder of the window
      state.throttled = true;
      state.throttledUntil = state.windowStart + this.config.windowSizeMs;

      this.metrics.throttledRequests++;

      return {
        allowed: false,
        waitMs: state.throttledUntil - now,
        reason: `Partner ${partnerId} rate limit exceeded`,
        currentRps: instantRps,
        limit: this.config.partnerRpsLimit,
      };
    }

    // Allow request
    state.requestCount++;
    this.metrics.totalRequests++;

    // Update partner metrics
    this.updatePartnerMetrics(partnerId, instantRps);

    return {
      allowed: true,
      currentRps: instantRps,
      limit: this.config.partnerRpsLimit,
    };
  }

  /**
   * Check if a request from a tenant should be allowed.
   */
  checkTenantPacing(tenantId: string): ThrottleResult {
    const now = Date.now();
    let state = this.tenantStates.get(tenantId);

    if (!state) {
      state = {
        partnerId: tenantId, // Reusing partnerId field for tenantId
        requestCount: 0,
        windowStart: now,
        currentRps: 0,
        throttled: false,
      };
      this.tenantStates.set(tenantId, state);
    }

    // Check if we're in a new window
    if (now - state.windowStart >= this.config.windowSizeMs) {
      const windowDurationSec = (now - state.windowStart) / 1000;
      const rps = state.requestCount / windowDurationSec;

      state.currentRps =
        this.config.smoothingFactor * rps +
        (1 - this.config.smoothingFactor) * state.currentRps;

      state.requestCount = 0;
      state.windowStart = now;
      state.throttled = false;
    }

    // Check if throttled
    if (state.throttled && state.throttledUntil && now < state.throttledUntil) {
      return {
        allowed: false,
        waitMs: state.throttledUntil - now,
        reason: `Tenant ${tenantId} is throttled`,
        currentRps: state.currentRps,
        limit: this.config.tenantRpsLimit,
      };
    }

    const windowDurationSec = Math.max((now - state.windowStart) / 1000, 0.001);
    const instantRps = state.requestCount / windowDurationSec;
    const effectiveLimit = this.config.tenantRpsLimit * this.config.burstMultiplier;

    if (instantRps >= effectiveLimit) {
      state.throttled = true;
      state.throttledUntil = state.windowStart + this.config.windowSizeMs;

      return {
        allowed: false,
        waitMs: state.throttledUntil - now,
        reason: `Tenant ${tenantId} rate limit exceeded`,
        currentRps: instantRps,
        limit: this.config.tenantRpsLimit,
      };
    }

    state.requestCount++;

    return {
      allowed: true,
      currentRps: instantRps,
      limit: this.config.tenantRpsLimit,
    };
  }

  /**
   * Combined check for both partner and tenant pacing.
   */
  checkPacing(partnerId: string, tenantId: string): ThrottleResult {
    // Check tenant first (broader scope)
    const tenantResult = this.checkTenantPacing(tenantId);
    if (!tenantResult.allowed) {
      return tenantResult;
    }

    // Then check partner
    return this.checkPartnerPacing(partnerId);
  }

  /**
   * Record a completed request (for accurate tracking).
   */
  recordRequest(partnerId: string, tenantId: string, durationMs: number): void {
    // Update partner state
    const partnerState = this.partnerStates.get(partnerId);
    if (partnerState) {
      // Track successful request timing if needed
    }

    // Update tenant state
    const tenantState = this.tenantStates.get(tenantId);
    if (tenantState) {
      // Track successful request timing if needed
    }
  }

  /**
   * Update partner-level metrics.
   */
  private updatePartnerMetrics(partnerId: string, currentRps: number): void {
    let metrics = this.metrics.partnerMetrics.get(partnerId);

    if (!metrics) {
      metrics = {
        partnerId,
        totalRequests: 0,
        throttledRequests: 0,
        averageRps: 0,
        peakRps: 0,
      };
      this.metrics.partnerMetrics.set(partnerId, metrics);
    }

    metrics.totalRequests++;
    metrics.peakRps = Math.max(metrics.peakRps, currentRps);
    metrics.averageRps =
      this.config.smoothingFactor * currentRps +
      (1 - this.config.smoothingFactor) * metrics.averageRps;
  }

  /**
   * Get current pacing metrics.
   */
  getMetrics(): PacingMetrics {
    return { ...this.metrics };
  }

  /**
   * Get pacing state for a specific partner.
   */
  getPartnerState(partnerId: string): PacingState | undefined {
    return this.partnerStates.get(partnerId);
  }

  /**
   * Get pacing state for a specific tenant.
   */
  getTenantState(tenantId: string): PacingState | undefined {
    return this.tenantStates.get(tenantId);
  }

  /**
   * Reset pacing state for a partner.
   */
  resetPartnerPacing(partnerId: string): void {
    this.partnerStates.delete(partnerId);
  }

  /**
   * Reset pacing state for a tenant.
   */
  resetTenantPacing(tenantId: string): void {
    this.tenantStates.delete(tenantId);
  }

  /**
   * Reset all pacing state.
   */
  resetAll(): void {
    this.partnerStates.clear();
    this.tenantStates.clear();
    this.metrics = {
      totalRequests: 0,
      throttledRequests: 0,
      averageRps: 0,
      peakRps: 0,
      partnerMetrics: new Map(),
    };
  }

  /**
   * Update configuration dynamically.
   */
  updateConfig(config: Partial<PacingConfig>): void {
    this.config = { ...this.config, ...config };
  }
}

export default PacingThrottler;
