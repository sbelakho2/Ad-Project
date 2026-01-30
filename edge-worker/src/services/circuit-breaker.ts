/**
 * CIRCUIT BREAKER
 * 
 * Implements the circuit breaker pattern to prevent cascading failures
 * when the Control Plane or other backend services are unavailable.
 * 
 * States:
 * - CLOSED: Normal operation, requests flow through
 * - OPEN: Service is down, fail fast without making requests
 * - HALF_OPEN: Testing if service has recovered
 * 
 * Transitions:
 * - CLOSED → OPEN: When failure threshold is exceeded in the window
 * - OPEN → HALF_OPEN: After reset timeout expires
 * - HALF_OPEN → CLOSED: After success threshold is met
 * - HALF_OPEN → OPEN: On any failure in half-open state
 */

import { Logger } from 'winston';

// ============================================================================
// TYPES
// ============================================================================

export type CircuitState = 'closed' | 'open' | 'half-open';

export interface CircuitBreakerConfig {
  /** Name for logging/metrics */
  name: string;
  /** Number of failures before opening circuit (default: 5) */
  failureThreshold?: number;
  /** Number of successes in half-open to close circuit (default: 2) */
  successThreshold?: number;
  /** Time in ms before transitioning from open to half-open (default: 30000) */
  resetTimeMs?: number;
  /** Sliding window size in ms for counting failures (default: 60000) */
  windowSizeMs?: number;
}

export interface CircuitBreakerStats {
  state: CircuitState;
  name: string;
  failures: number;
  successes: number;
  lastFailure: number | null;
  lastSuccess: number | null;
  openedAt: number | null;
  halfOpenSuccesses: number;
}

export class CircuitOpenError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CircuitOpenError';
  }
}

// ============================================================================
// CIRCUIT BREAKER IMPLEMENTATION
// ============================================================================

export class CircuitBreaker {
  private state: CircuitState = 'closed';
  private failures: number[] = []; // Timestamps of failures within window
  private halfOpenSuccesses = 0;
  private openedAt = 0;
  private lastFailure: number | null = null;
  private lastSuccess: number | null = null;
  
  // Configuration
  private readonly name: string;
  private readonly failureThreshold: number;
  private readonly successThreshold: number;
  private readonly resetTimeMs: number;
  private readonly windowSizeMs: number;
  private readonly logger?: Logger;
  
  constructor(config: CircuitBreakerConfig, logger?: Logger) {
    this.name = config.name;
    this.failureThreshold = config.failureThreshold ?? 5;
    this.successThreshold = config.successThreshold ?? 2;
    this.resetTimeMs = config.resetTimeMs ?? 30000; // 30 seconds
    this.windowSizeMs = config.windowSizeMs ?? 60000; // 1 minute
    this.logger = logger;
  }
  
  /**
   * Get current circuit state
   */
  getState(): CircuitState {
    this.updateState();
    return this.state;
  }
  
  /**
   * Get detailed stats for monitoring
   */
  getStats(): CircuitBreakerStats {
    this.updateState();
    return {
      state: this.state,
      name: this.name,
      failures: this.failures.length,
      successes: this.halfOpenSuccesses,
      lastFailure: this.lastFailure,
      lastSuccess: this.lastSuccess,
      openedAt: this.state === 'closed' ? null : this.openedAt,
      halfOpenSuccesses: this.halfOpenSuccesses,
    };
  }
  
  /**
   * Check if a request can be executed
   * Returns true if circuit is closed or half-open
   */
  canExecute(): boolean {
    this.updateState();
    return this.state !== 'open';
  }
  
  /**
   * Record a successful request
   */
  recordSuccess(): void {
    this.lastSuccess = Date.now();
    
    if (this.state === 'half-open') {
      this.halfOpenSuccesses++;
      
      if (this.halfOpenSuccesses >= this.successThreshold) {
        this.logger?.info(`Circuit ${this.name} CLOSED after ${this.halfOpenSuccesses} successful requests`);
        this.state = 'closed';
        this.failures = [];
        this.halfOpenSuccesses = 0;
        this.openedAt = 0;
      }
    }
  }
  
  /**
   * Record a failed request
   */
  recordFailure(): void {
    const now = Date.now();
    this.lastFailure = now;
    
    if (this.state === 'half-open') {
      // Any failure in half-open immediately opens the circuit
      this.logger?.warn(`Circuit ${this.name} re-OPENED after failure in half-open state`);
      this.state = 'open';
      this.openedAt = now;
      this.halfOpenSuccesses = 0;
      return;
    }
    
    // Add failure to the sliding window
    this.failures.push(now);
    this.cleanOldFailures();
    
    if (this.state === 'closed' && this.failures.length >= this.failureThreshold) {
      this.logger?.warn(`Circuit ${this.name} OPENED after ${this.failures.length} failures in ${this.windowSizeMs}ms window`);
      this.state = 'open';
      this.openedAt = now;
    }
  }
  
  /**
   * Execute a function with circuit breaker protection
   */
  async execute<T>(fn: () => Promise<T>): Promise<T> {
    if (!this.canExecute()) {
      throw new CircuitOpenError(`Circuit ${this.name} is open - failing fast`);
    }
    
    try {
      const result = await fn();
      this.recordSuccess();
      return result;
    } catch (error) {
      this.recordFailure();
      throw error;
    }
  }
  
  /**
   * Force the circuit to a specific state (for testing or manual intervention)
   */
  forceState(state: CircuitState): void {
    this.logger?.warn(`Circuit ${this.name} manually set to ${state}`);
    this.state = state;
    
    if (state === 'closed') {
      this.failures = [];
      this.halfOpenSuccesses = 0;
      this.openedAt = 0;
    } else if (state === 'open') {
      this.openedAt = Date.now();
      this.halfOpenSuccesses = 0;
    } else if (state === 'half-open') {
      this.halfOpenSuccesses = 0;
    }
  }
  
  // ============================================================================
  // PRIVATE METHODS
  // ============================================================================
  
  /**
   * Update state based on time (open → half-open transition)
   */
  private updateState(): void {
    if (this.state === 'open') {
      const now = Date.now();
      if (now - this.openedAt >= this.resetTimeMs) {
        this.logger?.info(`Circuit ${this.name} transitioning to HALF-OPEN after ${this.resetTimeMs}ms`);
        this.state = 'half-open';
        this.halfOpenSuccesses = 0;
      }
    }
    
    // Clean old failures when checking state
    if (this.state === 'closed') {
      this.cleanOldFailures();
    }
  }
  
  /**
   * Remove failures outside the sliding window
   */
  private cleanOldFailures(): void {
    const cutoff = Date.now() - this.windowSizeMs;
    this.failures = this.failures.filter(ts => ts > cutoff);
  }
}

// ============================================================================
// FACTORY FUNCTION
// ============================================================================

/**
 * Create a circuit breaker with default configuration
 */
export function createCircuitBreaker(
  name: string,
  logger?: Logger,
  overrides?: Partial<CircuitBreakerConfig>
): CircuitBreaker {
  return new CircuitBreaker(
    {
      name,
      failureThreshold: 5,
      successThreshold: 2,
      resetTimeMs: 30000,
      windowSizeMs: 60000,
      ...overrides,
    },
    logger
  );
}
