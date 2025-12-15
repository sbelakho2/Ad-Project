/**
 * SSAI Dedupe Layer
 *
 * Provides exactly-once semantics for tracking events.
 * Handles deduplication across client and server-side events.
 */

import { TrackingEvent, TrackingEventType, DedupeConfig } from './types';

const DEFAULT_CONFIG: DedupeConfig = {
  windowSeconds: 300, // 5 minutes
  maxEventsPerKey: 1000,
};

interface DedupeEntry {
  eventId: string;
  timestamp: Date;
  source: 'client' | 'server' | 'inferred';
  hash: string;
}

interface DedupeKey {
  sessionId: string;
  slotId: string;
  eventType: TrackingEventType;
}

export interface DedupeResult {
  /** Whether the event is a duplicate */
  isDuplicate: boolean;

  /** Original event (if duplicate) */
  originalEvent?: DedupeEntry;

  /** Source of the original event */
  originalSource?: 'client' | 'server' | 'inferred';

  /** Event hash */
  hash: string;
}

export class DedupeLayer {
  private config: DedupeConfig;
  private eventStore: Map<string, DedupeEntry[]> = new Map();
  private cleanupInterval: NodeJS.Timeout | null = null;

  constructor(config: Partial<DedupeConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.startCleanup();
  }

  /**
   * Generate a deduplication key
   */
  private generateKey(key: DedupeKey): string {
    return `${key.sessionId}:${key.slotId}:${key.eventType}`;
  }

  /**
   * Generate a content hash for an event
   */
  private generateHash(event: TrackingEvent): string {
    // Simple hash based on event content
    const content = [
      event.sessionId,
      event.slotId,
      event.creativeId,
      event.type,
      Math.floor(event.mediaTime / 0.5), // 500ms buckets
    ].join(':');

    // Simple string hash (in production, use crypto)
    let hash = 0;
    for (let i = 0; i < content.length; i++) {
      const char = content.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash; // Convert to 32bit integer
    }
    return hash.toString(36);
  }

  /**
   * Check if an event is a duplicate
   */
  check(event: TrackingEvent): DedupeResult {
    const key = this.generateKey({
      sessionId: event.sessionId,
      slotId: event.slotId,
      eventType: event.type,
    });

    const hash = this.generateHash(event);
    const entries = this.eventStore.get(key) || [];

    // Check for exact duplicate by event ID
    const exactDupe = entries.find((e) => e.eventId === event.eventId);
    if (exactDupe) {
      return {
        isDuplicate: true,
        originalEvent: exactDupe,
        originalSource: exactDupe.source,
        hash,
      };
    }

    // Check for content duplicate by hash
    const hashDupe = entries.find((e) => e.hash === hash);
    if (hashDupe) {
      return {
        isDuplicate: true,
        originalEvent: hashDupe,
        originalSource: hashDupe.source,
        hash,
      };
    }

    // Check for timing-based duplicate (same event type within short window)
    const timingThreshold = this.getTimingThreshold(event.type);
    const now = event.timestamp.getTime();
    const timingDupe = entries.find((e) => {
      const diff = Math.abs(now - e.timestamp.getTime());
      return diff < timingThreshold;
    });

    if (timingDupe) {
      return {
        isDuplicate: true,
        originalEvent: timingDupe,
        originalSource: timingDupe.source,
        hash,
      };
    }

    return {
      isDuplicate: false,
      hash,
    };
  }

  /**
   * Record an event (must be called after check passes)
   */
  record(event: TrackingEvent): void {
    const key = this.generateKey({
      sessionId: event.sessionId,
      slotId: event.slotId,
      eventType: event.type,
    });

    const hash = this.generateHash(event);
    const entry: DedupeEntry = {
      eventId: event.eventId,
      timestamp: event.timestamp,
      source: event.source,
      hash,
    };

    let entries = this.eventStore.get(key);
    if (!entries) {
      entries = [];
      this.eventStore.set(key, entries);
    }

    entries.push(entry);

    // Trim if over limit
    if (entries.length > this.config.maxEventsPerKey) {
      entries.shift();
    }
  }

  /**
   * Check and record in one operation
   */
  checkAndRecord(event: TrackingEvent): DedupeResult {
    const result = this.check(event);
    if (!result.isDuplicate) {
      this.record(event);
    }
    return result;
  }

  /**
   * Get timing threshold for an event type
   * Some events (like quartiles) need stricter deduplication
   */
  private getTimingThreshold(eventType: TrackingEventType): number {
    switch (eventType) {
      case 'impression':
      case 'start':
        return 1000; // 1 second
      case 'firstQuartile':
      case 'midpoint':
      case 'thirdQuartile':
        return 2000; // 2 seconds (allows for slight timing differences)
      case 'complete':
        return 3000; // 3 seconds
      case 'error':
        return 500; // 500ms (errors can be rapid)
      default:
        return 1000;
    }
  }

  /**
   * Start cleanup interval
   */
  private startCleanup(): void {
    this.cleanupInterval = setInterval(() => {
      this.cleanup();
    }, 60000); // Cleanup every minute
  }

  /**
   * Stop cleanup interval
   */
  stopCleanup(): void {
    if (this.cleanupInterval) {
      clearInterval(this.cleanupInterval);
      this.cleanupInterval = null;
    }
  }

  /**
   * Clean up expired entries
   */
  cleanup(): void {
    const expireTime = new Date(Date.now() - this.config.windowSeconds * 1000);

    for (const [key, entries] of this.eventStore) {
      const filtered = entries.filter((e) => e.timestamp > expireTime);
      if (filtered.length === 0) {
        this.eventStore.delete(key);
      } else if (filtered.length !== entries.length) {
        this.eventStore.set(key, filtered);
      }
    }
  }

  /**
   * Clear all state (for testing)
   */
  clear(): void {
    this.eventStore.clear();
  }

  /**
   * Get stats for monitoring
   */
  getStats(): {
    totalKeys: number;
    totalEvents: number;
    oldestEvent: Date | null;
  } {
    let totalEvents = 0;
    let oldestEvent: Date | null = null;

    for (const entries of this.eventStore.values()) {
      totalEvents += entries.length;
      for (const entry of entries) {
        if (!oldestEvent || entry.timestamp < oldestEvent) {
          oldestEvent = entry.timestamp;
        }
      }
    }

    return {
      totalKeys: this.eventStore.size,
      totalEvents,
      oldestEvent,
    };
  }
}

/**
 * Cross-source reconciler for client vs server events
 */
export class CrossSourceReconciler {
  private clientEvents: Map<string, TrackingEvent> = new Map();
  private serverEvents: Map<string, TrackingEvent> = new Map();
  private reconciled: Set<string> = new Set();

  /**
   * Record a client-side event
   */
  recordClientEvent(event: TrackingEvent): void {
    const key = `${event.slotId}:${event.type}`;
    this.clientEvents.set(key, event);
    this.tryReconcile(key);
  }

  /**
   * Record a server-side event
   */
  recordServerEvent(event: TrackingEvent): void {
    const key = `${event.slotId}:${event.type}`;
    this.serverEvents.set(key, event);
    this.tryReconcile(key);
  }

  /**
   * Try to reconcile events from both sources
   */
  private tryReconcile(key: string): void {
    if (this.reconciled.has(key)) return;

    const clientEvent = this.clientEvents.get(key);
    const serverEvent = this.serverEvents.get(key);

    if (clientEvent && serverEvent) {
      // Both sides have reported - reconcile
      this.reconciled.add(key);

      // Calculate timing difference
      const timeDiff = Math.abs(
        clientEvent.timestamp.getTime() - serverEvent.timestamp.getTime()
      );

      // Log significant discrepancies
      if (timeDiff > 5000) {
        console.warn(
          `[CrossSourceReconciler] Large timing discrepancy for ${key}: ${timeDiff}ms`
        );
      }
    }
  }

  /**
   * Get reconciliation status
   */
  getStatus(): {
    clientOnly: string[];
    serverOnly: string[];
    reconciled: string[];
  } {
    const clientOnly: string[] = [];
    const serverOnly: string[] = [];

    for (const key of this.clientEvents.keys()) {
      if (!this.serverEvents.has(key) && !this.reconciled.has(key)) {
        clientOnly.push(key);
      }
    }

    for (const key of this.serverEvents.keys()) {
      if (!this.clientEvents.has(key) && !this.reconciled.has(key)) {
        serverOnly.push(key);
      }
    }

    return {
      clientOnly,
      serverOnly,
      reconciled: Array.from(this.reconciled),
    };
  }

  /**
   * Clear state
   */
  clear(): void {
    this.clientEvents.clear();
    this.serverEvents.clear();
    this.reconciled.clear();
  }
}
