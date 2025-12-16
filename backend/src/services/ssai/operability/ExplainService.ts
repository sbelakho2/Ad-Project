/**
 * SSAI Explain Service
 *
 * Provides detailed explanation of pod decisions for debugging
 * and support. Shows the full decision trace for any session/pod.
 */

import {
  ExplainPodRequest,
  ExplainPodResponse,
  ExplainDecisionStep,
  ExplainSlot,
} from './types';

interface StoredDecision {
  sessionId: string;
  podId: string;
  tenantId: string;
  deviceType: string;
  capabilities: string[];
  breakId: string;
  cueType: string;
  cueTime: Date;
  duration: number;
  steps: ExplainDecisionStep[];
  slots: ExplainSlot[];
  manifestType?: 'HLS' | 'DASH';
  manifestUrl?: string;
  createdAt: Date;
}

export interface ExplainServiceConfig {
  /** Maximum decisions to store */
  maxStoredDecisions: number;

  /** Retention period (minutes) */
  retentionMinutes: number;

  /** Include sensitive data */
  includeSensitiveData: boolean;
}
/**
 * SSAI Explain Service (test-facing)
 */

export interface ExplainStep {
  step: string;
  input: unknown;
  output: unknown;
  durationMs: number;
}

export interface SlotDecision {
  winner: string;
  winningBid: number;
  reason: string;
  alternatives?: Array<{ creativeId: string; bid: number; reason: string }>;
}

export interface ExplainTrace {
  traceId: string;
  sessionId: string;
  podId: string;
  steps: ExplainStep[];
  slotDecisions: Map<string, SlotDecision>;
  totalDurationMs: number;
}

export class ExplainService {
  private traces: Map<string, ExplainTrace> = new Map();

  createTrace(sessionId: string, podId: string): string {
    const traceId = `${sessionId}:${podId}:${Date.now()}:${Math.random().toString(16).slice(2)}`;
    this.traces.set(traceId, {
      traceId,
      sessionId,
      podId,
      steps: [],
      slotDecisions: new Map(),
      totalDurationMs: 0,
    });
    return traceId;
  }

  recordStep(traceId: string, step: ExplainStep): void {
    const trace = this.traces.get(traceId);
    if (!trace) return;
    trace.steps.push(step);
    trace.totalDurationMs += step.durationMs;
  }

  getTrace(traceId: string): ExplainTrace {
    const trace = this.traces.get(traceId);
    if (!trace) {
      return {
        traceId,
        sessionId: '',
        podId: '',
        steps: [],
        slotDecisions: new Map(),
        totalDurationMs: 0,
      };
    }
    return trace;
  }

  recordSlotDecision(traceId: string, slotId: string, decision: SlotDecision): void {
    const trace = this.traces.get(traceId);
    if (!trace) return;
    trace.slotDecisions.set(slotId, decision);
  }

  explainDecision(traceId: string, slotId: string): SlotDecision {
    const trace = this.traces.get(traceId);
    return (
      trace?.slotDecisions.get(slotId) ?? {
        winner: '',
        winningBid: 0,
        reason: 'not_found',
        alternatives: [],
      }
    );
  }
}

