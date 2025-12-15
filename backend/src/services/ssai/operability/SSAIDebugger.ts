/**
 * SSAI Debugger
 * Provides debugging and explanation capabilities for SSAI decisions
 */

export interface PodExplanation {
  podId: string;
  sessionId?: string;
  timestamp: number;
  decision: {
    filledSlots: number;
    totalSlots: number;
    fillRate: number;
  };
  candidates: Array<{
    id: string;
    partnerId: string;
    selected: boolean;
    reason?: string;
  }>;
  timing: {
    totalMs: number;
    phases: Record<string, number>;
  };
}

export interface NoFillExplanation {
  requestId: string;
  timestamp: number;
  reasons: string[];
  candidates: number;
  filtered: Record<string, number>;
}

export interface DecisionTrace {
  sessionId: string;
  podId: string;
  events: Array<{
    timestamp: number;
    type: string;
    data: unknown;
  }>;
}

export class SSAIDebugger {
  private podCache: Map<string, PodExplanation>;
  private noFillCache: Map<string, NoFillExplanation>;
  private traceCache: Map<string, DecisionTrace>;

  constructor() {
    this.podCache = new Map();
    this.noFillCache = new Map();
    this.traceCache = new Map();
  }

  /**
   * Explain pod decision
   */
  async explainPod(podId: string): Promise<PodExplanation> {
    // Check cache first
    const cached = this.podCache.get(podId);
    if (cached) {
      return cached;
    }

    // Generate explanation (in production, would fetch from decision logs)
    const explanation: PodExplanation = {
      podId,
      timestamp: Date.now(),
      decision: {
        filledSlots: 0,
        totalSlots: 0,
        fillRate: 0,
      },
      candidates: [],
      timing: {
        totalMs: 0,
        phases: {},
      },
    };

    return explanation;
  }

  /**
   * Explain no-fill decision
   */
  async explainNoFill(requestId: string): Promise<NoFillExplanation> {
    const cached = this.noFillCache.get(requestId);
    if (cached) {
      return cached;
    }

    return {
      requestId,
      timestamp: Date.now(),
      reasons: ['No candidates available'],
      candidates: 0,
      filtered: {},
    };
  }

  /**
   * Get decision trace
   */
  async getDecisionTrace(sessionId: string, podId: string): Promise<DecisionTrace> {
    const key = `${sessionId}:${podId}`;
    const cached = this.traceCache.get(key);
    if (cached) {
      return cached;
    }

    return {
      sessionId,
      podId,
      events: [],
    };
  }

  /**
   * Record pod explanation
   */
  recordPodExplanation(explanation: PodExplanation): void {
    this.podCache.set(explanation.podId, explanation);
  }

  /**
   * Record no-fill explanation
   */
  recordNoFillExplanation(explanation: NoFillExplanation): void {
    this.noFillCache.set(explanation.requestId, explanation);
  }

  /**
   * Record decision event
   */
  recordDecisionEvent(sessionId: string, podId: string, event: { type: string; data: unknown }): void {
    const key = `${sessionId}:${podId}`;
    
    if (!this.traceCache.has(key)) {
      this.traceCache.set(key, {
        sessionId,
        podId,
        events: [],
      });
    }

    this.traceCache.get(key)!.events.push({
      timestamp: Date.now(),
      ...event,
    });
  }

  /**
   * Clear old entries
   */
  cleanup(maxAgeMs: number = 3600000): void {
    const cutoff = Date.now() - maxAgeMs;

    for (const [key, value] of this.podCache) {
      if (value.timestamp < cutoff) {
        this.podCache.delete(key);
      }
    }

    for (const [key, value] of this.noFillCache) {
      if (value.timestamp < cutoff) {
        this.noFillCache.delete(key);
      }
    }
  }
}

export default SSAIDebugger;
