/**
 * SSAI Circuit Breaker
 *
 * Enhanced circuit breaker specifically designed for SSAI partner calls,
 * with timeout rate thresholds, error rate thresholds, and configurable cooldowns.
 */

export type CircuitState = 'closed' | 'open' | 'half-open';

export interface CircuitBreakerConfig {
  /** Failure rate threshold to open circuit (0-1) */
  failureRateThreshold: number;

  /** Timeout rate threshold to open circuit (0-1) */
  timeoutRateThreshold: number;

  /** Minimum number of calls before evaluating thresholds */
  minimumCalls: number;

  /** Duration to keep circuit open before half-open (ms) */
  openDuration: number;

  /** Number of test calls in half-open state */
  halfOpenCalls: number;

  /** Sliding window size for rate calculation */
  windowSize: number;

  /** Slow call duration threshold (ms) */
  slowCallThreshold: number;

  /** Slow call rate threshold to open circuit (0-1) */
  slowCallRateThreshold: number;
}

export interface CircuitStats {
  totalCalls: number;
  successfulCalls: number;
  failedCalls: number;
  timedOutCalls: number;
  slowCalls: number;
  failureRate: number;
  timeoutRate: number;
  slowCallRate: number;
  lastStateChange: number;
  consecutiveSuccesses: number;
  consecutiveFailures: number;
}

export interface CallResult {
  success: boolean;
  timedOut: boolean;
  durationMs: number;
  error?: string;
}

/**
 * SSAI-specific Circuit Breaker for partner reliability.
 */
export class SSAICircuitBreaker {
  private config: CircuitBreakerConfig;
  private state: CircuitState = 'closed';
  private stats: CircuitStats;
  private callHistory: CallResult[] = [];
  private halfOpenSuccesses = 0;
  private stateChangedAt: number;
  private partnerId: string;

  constructor(partnerId: string, config?: Partial<CircuitBreakerConfig>) {
    this.partnerId = partnerId;
    this.config = {
      failureRateThreshold: 0.5,
      timeoutRateThreshold: 0.3,
      minimumCalls: 10,
      openDuration: 30000, // 30 seconds
      halfOpenCalls: 3,
      windowSize: 100,
      slowCallThreshold: 5000, // 5 seconds
      slowCallRateThreshold: 0.5,
      ...config,
    };

    this.stateChangedAt = Date.now();
    this.stats = this.initStats();
  }

  private initStats(): CircuitStats {
    return {
      totalCalls: 0,
      successfulCalls: 0,
      failedCalls: 0,
      timedOutCalls: 0,
      slowCalls: 0,
      failureRate: 0,
      timeoutRate: 0,
      slowCallRate: 0,
      lastStateChange: Date.now(),
      consecutiveSuccesses: 0,
      consecutiveFailures: 0,
    };
  }

  /**
   * Check if call should be allowed through the circuit.
   */
  canCall(): boolean {
    const now = Date.now();

    switch (this.state) {
      case 'closed':
        return true;

      case 'open':
        // Check if cooldown period has passed
        if (now - this.stateChangedAt >= this.config.openDuration) {
          this.transitionTo('half-open');
          return true;
        }
        return false;

      case 'half-open':
        // Allow limited test calls
        return this.halfOpenSuccesses < this.config.halfOpenCalls;
    }
  }

  /**
   * Record the result of a call.
   */
  recordResult(result: CallResult): void {
    // Add to history
    this.callHistory.push(result);

    // Trim history to window size
    if (this.callHistory.length > this.config.windowSize) {
      this.callHistory.shift();
    }

    // Update stats
    this.updateStats(result);

    // Evaluate state transition
    this.evaluateState(result);
  }

  /**
   * Update statistics based on call result.
   */
  private updateStats(result: CallResult): void {
    this.stats.totalCalls++;

    if (result.success) {
      this.stats.successfulCalls++;
      this.stats.consecutiveSuccesses++;
      this.stats.consecutiveFailures = 0;
    } else {
      this.stats.failedCalls++;
      this.stats.consecutiveFailures++;
      this.stats.consecutiveSuccesses = 0;
    }

    if (result.timedOut) {
      this.stats.timedOutCalls++;
    }

    if (result.durationMs > this.config.slowCallThreshold) {
      this.stats.slowCalls++;
    }

    // Recalculate rates from history
    this.recalculateRates();
  }

  /**
   * Recalculate rates from call history.
   */
  private recalculateRates(): void {
    const total = this.callHistory.length;

    if (total === 0) {
      this.stats.failureRate = 0;
      this.stats.timeoutRate = 0;
      this.stats.slowCallRate = 0;
      return;
    }

    const failures = this.callHistory.filter((c) => !c.success).length;
    const timeouts = this.callHistory.filter((c) => c.timedOut).length;
    const slowCalls = this.callHistory.filter(
      (c) => c.durationMs > this.config.slowCallThreshold
    ).length;

    this.stats.failureRate = failures / total;
    this.stats.timeoutRate = timeouts / total;
    this.stats.slowCallRate = slowCalls / total;
  }

  /**
   * Evaluate and potentially transition state.
   */
  private evaluateState(result: CallResult): void {
    switch (this.state) {
      case 'closed':
        this.evaluateClosedState();
        break;

      case 'half-open':
        this.evaluateHalfOpenState(result);
        break;

      case 'open':
        // No evaluation needed in open state
        break;
    }
  }

  /**
   * Evaluate closed state for potential trip.
   */
  private evaluateClosedState(): void {
    // Need minimum calls before evaluating
    if (this.callHistory.length < this.config.minimumCalls) {
      return;
    }

    // Check thresholds
    const shouldOpen =
      this.stats.failureRate >= this.config.failureRateThreshold ||
      this.stats.timeoutRate >= this.config.timeoutRateThreshold ||
      this.stats.slowCallRate >= this.config.slowCallRateThreshold;

    if (shouldOpen) {
      this.transitionTo('open');
    }
  }

  /**
   * Evaluate half-open state for recovery.
   */
  private evaluateHalfOpenState(result: CallResult): void {
    if (result.success) {
      this.halfOpenSuccesses++;

      // Check if enough successful test calls
      if (this.halfOpenSuccesses >= this.config.halfOpenCalls) {
        this.transitionTo('closed');
      }
    } else {
      // Failure in half-open state → back to open
      this.transitionTo('open');
    }
  }

  /**
   * Transition to a new state.
   */
  private transitionTo(newState: CircuitState): void {
    const oldState = this.state;
    this.state = newState;
    this.stateChangedAt = Date.now();
    this.stats.lastStateChange = this.stateChangedAt;

    if (newState === 'closed') {
      // Reset on recovery
      this.halfOpenSuccesses = 0;
      this.callHistory = [];
      this.stats.consecutiveFailures = 0;
    } else if (newState === 'half-open') {
      this.halfOpenSuccesses = 0;
    }

    // Log state transition
    console.log(
      `[SSAICircuitBreaker] Partner ${this.partnerId}: ${oldState} → ${newState}`
    );
  }

  /**
   * Get current circuit state.
   */
  getState(): CircuitState {
    return this.state;
  }

  /**
   * Get current statistics.
   */
  getStats(): CircuitStats {
    return { ...this.stats };
  }

  /**
   * Get partner ID.
   */
  getPartnerId(): string {
    return this.partnerId;
  }

  /**
   * Force circuit to open state (for manual intervention).
   */
  forceOpen(): void {
    this.transitionTo('open');
  }

  /**
   * Force circuit to closed state (for manual recovery).
   */
  forceClose(): void {
    this.transitionTo('closed');
  }

  /**
   * Reset circuit breaker to initial state.
   */
  reset(): void {
    this.state = 'closed';
    this.stateChangedAt = Date.now();
    this.callHistory = [];
    this.halfOpenSuccesses = 0;
    this.stats = this.initStats();
  }

  /**
   * Check if circuit is healthy.
   */
  isHealthy(): boolean {
    return this.state === 'closed' && this.stats.failureRate < 0.1;
  }

  /**
   * Get health score (0-1).
   */
  getHealthScore(): number {
    if (this.state === 'open') {
      return 0;
    }

    if (this.state === 'half-open') {
      return 0.5;
    }

    // In closed state, health based on failure rate
    return Math.max(0, 1 - this.stats.failureRate * 2);
  }
}

/**
 * Circuit Breaker Registry for managing multiple partner breakers.
 */
export class CircuitBreakerRegistry {
  private breakers: Map<string, SSAICircuitBreaker> = new Map();
  private defaultConfig: Partial<CircuitBreakerConfig>;

  constructor(defaultConfig?: Partial<CircuitBreakerConfig>) {
    this.defaultConfig = defaultConfig || {};
  }

  /**
   * Get or create circuit breaker for a partner.
   */
  getBreaker(partnerId: string): SSAICircuitBreaker {
    let breaker = this.breakers.get(partnerId);

    if (!breaker) {
      breaker = new SSAICircuitBreaker(partnerId, this.defaultConfig);
      this.breakers.set(partnerId, breaker);
    }

    return breaker;
  }

  /**
   * Check if call to partner should be allowed.
   */
  canCall(partnerId: string): boolean {
    return this.getBreaker(partnerId).canCall();
  }

  /**
   * Record call result for partner.
   */
  recordResult(partnerId: string, result: CallResult): void {
    this.getBreaker(partnerId).recordResult(result);
  }

  /**
   * Get all open circuits.
   */
  getOpenCircuits(): string[] {
    const open: string[] = [];
    this.breakers.forEach((breaker, partnerId) => {
      if (breaker.getState() === 'open') {
        open.push(partnerId);
      }
    });
    return open;
  }

  /**
   * Get health summary for all partners.
   */
  getHealthSummary(): Map<string, { state: CircuitState; healthScore: number }> {
    const summary = new Map<string, { state: CircuitState; healthScore: number }>();

    this.breakers.forEach((breaker, partnerId) => {
      summary.set(partnerId, {
        state: breaker.getState(),
        healthScore: breaker.getHealthScore(),
      });
    });

    return summary;
  }

  /**
   * Reset all circuit breakers.
   */
  resetAll(): void {
    this.breakers.forEach((breaker) => breaker.reset());
  }
}

export default SSAICircuitBreaker;
