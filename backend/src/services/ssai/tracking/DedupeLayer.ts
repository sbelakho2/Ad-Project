/**
 * SSAI Dedupe Layer (test-facing)
 */

export interface DedupeLayerConfig {
  ttlMs: number;
  maxSize: number;
}

type Entry = { key: string; ts: number; slotId?: string };

export class DedupeLayer {
  private readonly config: DedupeLayerConfig;
  private readonly store: Map<string, Entry> = new Map();

  constructor(config: DedupeLayerConfig) {
    this.config = config;
  }

  isDuplicate(eventKey: string): boolean {
    this.evictExpired();
    return this.store.has(eventKey);
  }

  markAsFired(eventKey: string): void {
    this.evictExpired();

    const slotId = this.parseSlotId(eventKey);
    // Refresh insertion order for the key.
    this.store.delete(eventKey);
    this.store.set(eventKey, { key: eventKey, ts: Date.now(), slotId });

    // Evict oldest if max size exceeded.
    while (this.store.size > this.config.maxSize) {
      const oldestKey = this.store.keys().next().value;
      if (!oldestKey) break;
      this.store.delete(oldestKey);
    }
  }

  clearSlot(slotId: string): void {
    for (const [key, entry] of this.store.entries()) {
      if (entry.slotId === slotId) {
        this.store.delete(key);
      }
    }
  }

  reconcile(clientEvents: Array<{ type: string; slotId: string; timestamp: number }>, serverEvents: Array<{ type: string; slotId: string; timestamp: number }>): {
    matched: Array<{ type: string; slotId: string; clientTimestamp: number; serverTimestamp: number }>;
    clientOnly: Array<{ type: string; slotId: string; timestamp: number }>;
    serverOnly: Array<{ type: string; slotId: string; timestamp: number }>;
  } {
    const serverByKey = new Map<string, { type: string; slotId: string; timestamp: number }[]>();

    for (const ev of serverEvents) {
      const k = `${ev.type}:${ev.slotId}`;
      const list = serverByKey.get(k) ?? [];
      list.push(ev);
      serverByKey.set(k, list);
    }

    const matched: Array<{ type: string; slotId: string; clientTimestamp: number; serverTimestamp: number }> = [];
    const clientOnly: Array<{ type: string; slotId: string; timestamp: number }> = [];
    const usedServer = new Set<string>();

    for (const c of clientEvents) {
      const k = `${c.type}:${c.slotId}`;
      const candidates = serverByKey.get(k) ?? [];
      const server = candidates.shift();
      if (server) {
        matched.push({ type: c.type, slotId: c.slotId, clientTimestamp: c.timestamp, serverTimestamp: server.timestamp });
        usedServer.add(`${k}:${server.timestamp}`);
      } else {
        clientOnly.push(c);
      }
    }

    const serverOnly: Array<{ type: string; slotId: string; timestamp: number }> = [];
    for (const s of serverEvents) {
      const k = `${s.type}:${s.slotId}:${s.timestamp}`;
      if (!usedServer.has(k)) serverOnly.push(s);
    }

    return { matched, clientOnly, serverOnly };
  }

  private evictExpired(): void {
    const cutoff = Date.now() - this.config.ttlMs;
    for (const [key, entry] of this.store.entries()) {
      if (entry.ts < cutoff) this.store.delete(key);
    }
  }

  private parseSlotId(eventKey: string): string | undefined {
    // Expected format in tests: type:podId:slotId:creativeId
    const parts = eventKey.split(':');
    return parts.length >= 3 ? parts[2] : undefined;
  }
}
