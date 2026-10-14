/**
 * @file Mediation Debugger
 * @description Debug and trace mediation waterfall execution with sanitized output
 * @module @rivalapex/web-sdk
 */

/**
 * Event types for mediation debugging
 */
export enum MediationEventType {
  WATERFALL_START = 'waterfall_start',
  WATERFALL_END = 'waterfall_end',
  ADAPTER_REQUEST = 'adapter_request',
  ADAPTER_RESPONSE = 'adapter_response',
  BID_RECEIVED = 'bid_received',
  NO_FILL = 'no_fill',
  TIMEOUT = 'timeout',
  ERROR = 'error',
  AD_RENDERED = 'ad_rendered',
  AD_CLICKED = 'ad_clicked',
  AD_CLOSED = 'ad_closed',
  CIRCUIT_BREAKER_OPEN = 'circuit_breaker_open',
  CIRCUIT_BREAKER_CLOSE = 'circuit_breaker_close',
}

/**
 * Mediation event structure
 */
export interface MediationEvent {
  type: MediationEventType;
  adapterId?: string;
  timestamp: number;
  latency?: number;
  success?: boolean;
  timeout?: number;
  error?: string;
  bidAmount?: number | string;
  metadata?: Record<string, unknown>;
}

/**
 * Debug session for tracking mediation
 */
export interface DebugSession {
  id: string;
  placementId: string;
  startTime: number;
  endTime?: number;
  events: MediationEvent[];
}

/**
 * Snapshot of debug session
 */
export interface DebugSnapshot {
  sessionId: string;
  placementId: string;
  startTime: number;
  events: MediationEvent[];
  summary?: {
    totalDuration: number;
    adapterCount: number;
    winningAdapter: string | null;
  };
}

/**
 * Waterfall timeline visualization
 */
export interface WaterfallTimeline {
  totalDuration: number;
  adapters: AdapterTimeline[];
  winningAdapter: string | null;
}

/**
 * Adapter timeline entry
 */
export interface AdapterTimeline {
  adapterId: string;
  startOffset: number;
  duration: number;
  result: 'success' | 'no-fill' | 'timeout' | 'error';
}

/**
 * Performance statistics for an adapter
 */
export interface PerformanceStats {
  avgLatency: number;
  minLatency: number;
  maxLatency: number;
  successRate: number;
  timeoutCount: number;
  totalRequests: number;
}

/**
 * Configuration for the debugger
 */
export interface DebuggerConfig {
  maxEventsPerSession?: number;
  sessionTimeoutMs?: number;
  sanitizeByDefault?: boolean;
  enableRealtime?: boolean;
}

/**
 * Snapshot options
 */
export interface SnapshotOptions {
  sanitize?: boolean;
}

/**
 * Event subscriber callback
 */
type EventSubscriber = (event: MediationEvent) => void;

/**
 * Sensitive field patterns for sanitization
 */
const SENSITIVE_PATTERNS: { key: RegExp; sanitizer: (value: unknown) => unknown }[] = [
  { key: /userId|user_id/i, sanitizer: (v) => typeof v === 'string' ? `user-${'*'.repeat(8)}` : v },
  { key: /deviceId|device_id/i, sanitizer: (v) => typeof v === 'string' ? `device-${'*'.repeat(8)}` : v },
  { key: /email/i, sanitizer: () => '[EMAIL REDACTED]' },
  { key: /ip|clientIp|client_ip/i, sanitizer: () => '***.***.***.***' },
  { key: /idfa|gaid|advertisingId/i, sanitizer: (v) => typeof v === 'string' ? `${'*'.repeat(8)}` : v },
  { key: /token|apiKey|api_key|secret/i, sanitizer: () => '[REDACTED]' },
];

/**
 * Generate a unique ID
 */
function generateId(): string {
  return `debug-${Date.now()}-${Math.random().toString(36).substring(2, 11)}`;
}

/**
 * Mediation debugger for tracing and analyzing waterfall execution
 */
export class MediationDebugger {
  private sessions: Map<string, DebugSession> = new Map();
  private placementToSession: Map<string, string> = new Map();
  private subscribers: Map<string, Set<EventSubscriber>> = new Map();
  private config: Required<DebuggerConfig>;
  private cleanupTimer: ReturnType<typeof setInterval> | null = null;

  constructor(config: DebuggerConfig = {}) {
    this.config = {
      maxEventsPerSession: config.maxEventsPerSession ?? 1000,
      sessionTimeoutMs: config.sessionTimeoutMs ?? 300000, // 5 minutes
      sanitizeByDefault: config.sanitizeByDefault ?? true,
      enableRealtime: config.enableRealtime ?? true,
    };

    // Start cleanup timer
    this.cleanupTimer = setInterval(() => this.cleanup(), 60000);
  }

  /**
   * Start a new debug session
   */
  startSession(placementId: string): DebugSession {
    const session: DebugSession = {
      id: generateId(),
      placementId,
      startTime: Date.now(),
      events: [],
    };

    this.sessions.set(session.id, session);
    this.placementToSession.set(placementId, session.id);

    return session;
  }

  /**
   * Stop a debug session
   */
  stopSession(sessionId: string): void {
    const session = this.sessions.get(sessionId);
    if (session) {
      session.endTime = Date.now();
      this.sessions.delete(sessionId);
      this.placementToSession.delete(session.placementId);
      this.subscribers.delete(sessionId);
    }
  }

  /**
   * Get a debug session
   */
  getSession(sessionId: string): DebugSession | null {
    return this.sessions.get(sessionId) ?? null;
  }

  /**
   * Get session for a placement
   */
  getSessionForPlacement(placementId: string): DebugSession | null {
    const sessionId = this.placementToSession.get(placementId);
    if (!sessionId) return null;
    return this.sessions.get(sessionId) ?? null;
  }

  /**
   * Record a mediation event
   */
  recordEvent(sessionId: string, event: MediationEvent): void {
    const session = this.sessions.get(sessionId);
    if (!session) return;

    session.events.push(event);

    // Enforce max events limit (keep most recent)
    if (session.events.length > this.config.maxEventsPerSession) {
      session.events = session.events.slice(-this.config.maxEventsPerSession);
    }

    // Notify subscribers
    if (this.config.enableRealtime) {
      const subs = this.subscribers.get(sessionId);
      if (subs) {
        subs.forEach(callback => {
          try {
            callback(event);
          } catch (e) {
            // Ignore subscriber errors
          }
        });
      }
    }
  }

  /**
   * Get events for a session
   */
  getEvents(sessionId: string): MediationEvent[] {
    const session = this.sessions.get(sessionId);
    return session?.events ?? [];
  }

  /**
   * Get a sanitized snapshot of the session
   */
  getSnapshot(sessionId: string, options?: SnapshotOptions): DebugSnapshot | null {
    const session = this.sessions.get(sessionId);
    if (!session) return null;

    const sanitize = options?.sanitize ?? this.config.sanitizeByDefault;
    const events = sanitize ? this.sanitizeEvents(session.events) : [...session.events];

    const timeline = this.calculateTimeline(session);

    return {
      sessionId: session.id,
      placementId: session.placementId,
      startTime: session.startTime,
      events,
      summary: timeline ? {
        totalDuration: timeline.totalDuration,
        adapterCount: timeline.adapters.length,
        winningAdapter: timeline.winningAdapter,
      } : undefined,
    };
  }

  /**
   * Sanitize events to remove sensitive data
   */
  private sanitizeEvents(events: MediationEvent[]): MediationEvent[] {
    return events.map(event => this.sanitizeEvent(event));
  }

  /**
   * Sanitize a single event
   */
  private sanitizeEvent(event: MediationEvent): MediationEvent {
    const sanitized: MediationEvent = { ...event };

    // Sanitize bid amount
    if (sanitized.bidAmount !== undefined) {
      sanitized.bidAmount = '[REDACTED]';
    }

    // Sanitize metadata
    if (sanitized.metadata) {
      sanitized.metadata = this.sanitizeMetadata(sanitized.metadata);
    }

    return sanitized;
  }

  /**
   * Sanitize metadata object
   */
  private sanitizeMetadata(metadata: Record<string, unknown>): Record<string, unknown> {
    const sanitized: Record<string, unknown> = {};

    for (const [key, value] of Object.entries(metadata)) {
      let sanitizedValue = value;

      for (const pattern of SENSITIVE_PATTERNS) {
        if (pattern.key.test(key)) {
          sanitizedValue = pattern.sanitizer(value);
          break;
        }
      }

      sanitized[key] = sanitizedValue;
    }

    return sanitized;
  }

  /**
   * Get waterfall timeline visualization
   */
  getWaterfallTimeline(sessionId: string): WaterfallTimeline | null {
    const session = this.sessions.get(sessionId);
    if (!session) return null;

    return this.calculateTimeline(session);
  }

  /**
   * Calculate timeline from session events
   */
  private calculateTimeline(session: DebugSession): WaterfallTimeline | null {
    const events = session.events;
    if (events.length === 0) return null;

    const waterfallStart = events.find(e => e.type === MediationEventType.WATERFALL_START);
    const waterfallEnd = events.find(e => e.type === MediationEventType.WATERFALL_END);
    
    const startTime = waterfallStart?.timestamp ?? events[0].timestamp;
    const endTime = waterfallEnd?.timestamp ?? events[events.length - 1].timestamp;
    const totalDuration = endTime - startTime;

    // Group events by adapter
    const adapterEvents: Map<string, MediationEvent[]> = new Map();
    for (const event of events) {
      if (event.adapterId) {
        if (!adapterEvents.has(event.adapterId)) {
          adapterEvents.set(event.adapterId, []);
        }
        adapterEvents.get(event.adapterId)!.push(event);
      }
    }

    // Build adapter timelines
    const adapters: AdapterTimeline[] = [];
    let winningAdapter: string | null = null;

    for (const [adapterId, adapterEvts] of adapterEvents) {
      const request = adapterEvts.find(e => e.type === MediationEventType.ADAPTER_REQUEST);
      const response = adapterEvts.find(e => 
        e.type === MediationEventType.ADAPTER_RESPONSE ||
        e.type === MediationEventType.NO_FILL ||
        e.type === MediationEventType.TIMEOUT ||
        e.type === MediationEventType.ERROR
      );

      if (request && response) {
        let result: AdapterTimeline['result'] = 'error';
        if (response.type === MediationEventType.ADAPTER_RESPONSE && response.success) {
          result = 'success';
        } else if (response.type === MediationEventType.NO_FILL) {
          result = 'no-fill';
        } else if (response.type === MediationEventType.TIMEOUT) {
          result = 'timeout';
        }

        adapters.push({
          adapterId,
          startOffset: request.timestamp - startTime,
          duration: response.timestamp - request.timestamp,
          result,
        });

        // Check for ad rendered (winner)
        const rendered = adapterEvts.find(e => e.type === MediationEventType.AD_RENDERED);
        if (rendered || (response.type === MediationEventType.ADAPTER_RESPONSE && response.success)) {
          winningAdapter = adapterId;
        }
      }
    }

    // Sort by start time
    adapters.sort((a, b) => a.startOffset - b.startOffset);

    return {
      totalDuration,
      adapters,
      winningAdapter,
    };
  }

  /**
   * Get performance statistics for an adapter
   */
  getPerformanceStats(sessionId: string, adapterId: string): PerformanceStats | null {
    const session = this.sessions.get(sessionId);
    if (!session) return null;

    const responses = session.events.filter(
      e => e.adapterId === adapterId && 
           (e.type === MediationEventType.ADAPTER_RESPONSE || e.type === MediationEventType.TIMEOUT)
    );

    if (responses.length === 0) return null;

    const latencies = responses
      .filter(e => e.latency !== undefined)
      .map(e => e.latency!);

    const successCount = responses.filter(
      e => e.type === MediationEventType.ADAPTER_RESPONSE && e.success
    ).length;

    const timeoutCount = responses.filter(
      e => e.type === MediationEventType.TIMEOUT
    ).length;

    return {
      avgLatency: latencies.length > 0 
        ? Math.round(latencies.reduce((a, b) => a + b, 0) / latencies.length)
        : 0,
      minLatency: latencies.length > 0 ? Math.min(...latencies) : 0,
      maxLatency: latencies.length > 0 ? Math.max(...latencies) : 0,
      successRate: responses.length > 0 ? successCount / responses.length : 0,
      timeoutCount,
      totalRequests: responses.length,
    };
  }

  /**
   * Export session to JSON format
   */
  exportToJSON(sessionId: string, sanitize: boolean = true): string | null {
    const snapshot = this.getSnapshot(sessionId, { sanitize });
    if (!snapshot) return null;

    return JSON.stringify(snapshot, null, 2);
  }

  /**
   * Generate text summary of session
   */
  getTextSummary(sessionId: string): string {
    const session = this.sessions.get(sessionId);
    if (!session) return '';

    const lines: string[] = [];
    lines.push(`=== Mediation Debug Session: ${session.id} ===`);
    lines.push(`Placement: ${session.placementId}`);
    lines.push(`Started: ${new Date(session.startTime).toISOString()}`);
    lines.push('');

    for (const event of session.events) {
      const time = event.timestamp - session.startTime;
      let line = `[+${time}ms] ${event.type}`;
      
      if (event.adapterId) {
        line += ` | ${event.adapterId}`;
      }
      if (event.latency !== undefined) {
        line += ` | ${event.latency}ms`;
      }
      if (event.success !== undefined) {
        line += ` | ${event.success ? 'SUCCESS' : 'FAILED'}`;
      }
      if (event.error) {
        line += ` | Error: ${event.error}`;
      }

      lines.push(line);
    }

    return lines.join('\n');
  }

  /**
   * Generate timeline visualization as ASCII art
   */
  getTimelineVisualization(sessionId: string): string {
    const timeline = this.getWaterfallTimeline(sessionId);
    if (!timeline) return '';

    const width = 50;
    const scale = timeline.totalDuration / width;
    const lines: string[] = [];

    lines.push(`Waterfall Timeline (${timeline.totalDuration}ms total)`);
    lines.push('=' .repeat(width + 20));

    for (const adapter of timeline.adapters) {
      const startPos = Math.floor(adapter.startOffset / scale);
      const barLength = Math.max(1, Math.floor(adapter.duration / scale));
      
      const resultChar = {
        'success': '✓',
        'no-fill': '○',
        'timeout': '⏱',
        'error': '✗',
      }[adapter.result];

      const bar = ' '.repeat(startPos) + '█'.repeat(barLength);
      const label = `${adapter.adapterId.padEnd(12)} |${bar}| ${adapter.duration}ms ${resultChar}`;
      lines.push(label);
    }

    lines.push('=' .repeat(width + 20));
    if (timeline.winningAdapter) {
      lines.push(`Winner: ${timeline.winningAdapter}`);
    } else {
      lines.push('No winner (all no-fill)');
    }

    return lines.join('\n');
  }

  /**
   * Subscribe to real-time events
   */
  subscribe(sessionId: string, callback: EventSubscriber): () => void {
    if (!this.subscribers.has(sessionId)) {
      this.subscribers.set(sessionId, new Set());
    }
    this.subscribers.get(sessionId)!.add(callback);

    return () => {
      const subs = this.subscribers.get(sessionId);
      if (subs) {
        subs.delete(callback);
      }
    };
  }

  /**
   * Clean up expired sessions
   */
  cleanup(): void {
    const now = Date.now();
    const expiredIds: string[] = [];

    for (const [id, session] of this.sessions) {
      if (now - session.startTime > this.config.sessionTimeoutMs) {
        expiredIds.push(id);
      }
    }

    for (const id of expiredIds) {
      this.stopSession(id);
    }
  }

  /**
   * Stop the debugger and clean up
   */
  stop(): void {
    if (this.cleanupTimer) {
      clearInterval(this.cleanupTimer);
      this.cleanupTimer = null;
    }
    this.sessions.clear();
    this.placementToSession.clear();
    this.subscribers.clear();
  }
}
