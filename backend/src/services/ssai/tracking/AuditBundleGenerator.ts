/**
 * SSAI Audit Bundle Generator
 *
 * Generates cryptographically signed audit bundles for billing reconciliation
 * and dispute resolution. Bundles contain complete decision and delivery proof.
 */

import { AuditBundle, TrackingEvent, TrackingEventType } from './types';
import { createHmac, randomBytes } from 'crypto';

interface DecisionRecord {
  breakId: string;
  cueReceived: Date;
  podPlanned: Date;
  slotsRequested: number;
  slotsFilled: number;
  decisionTimeMs: number;
}

interface CreativeRecord {
  slotId: string;
  creativeId: string;
  partnerId: string;
  bidPrice?: number;
  reason: string;
}

interface CueRecord {
  type: string;
  raw?: string;
  normalized: unknown;
}

export interface AuditBundleGeneratorConfig {
  /** Secret key for HMAC signing */
  signingSecret: string;

  /** Include raw cue data */
  includeRawCue: boolean;

  /** Maximum events per bundle */
  maxEventsPerBundle: number;

  /** Bundle retention period (days) */
  retentionDays: number;
}

const DEFAULT_CONFIG: AuditBundleGeneratorConfig = {
  signingSecret: 'default-signing-secret-change-in-production',
  includeRawCue: true,
  maxEventsPerBundle: 1000,
  retentionDays: 90,
};

interface BundleData {
  sessionId: string;
  podId: string;
  decision?: DecisionRecord;
  manifestVersions: string[];
  cueDetails?: CueRecord;
  creatives: CreativeRecord[];
  events: TrackingEvent[];
}

export class AuditBundleGenerator {
  private config: AuditBundleGeneratorConfig;
  private bundles: Map<string, BundleData> = new Map();
  private generatedBundles: Map<string, AuditBundle> = new Map();

  constructor(config: Partial<AuditBundleGeneratorConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Start a new bundle for a session/pod
   */
  startBundle(sessionId: string, podId: string): string {
    const bundleKey = `${sessionId}:${podId}`;

    if (!this.bundles.has(bundleKey)) {
      this.bundles.set(bundleKey, {
        sessionId,
        podId,
        manifestVersions: [],
        creatives: [],
        events: [],
      });
    }

    return bundleKey;
  }

  /**
   * Record decision data
   */
  recordDecision(
    sessionId: string,
    podId: string,
    decision: DecisionRecord
  ): void {
    const bundleKey = `${sessionId}:${podId}`;
    const bundle = this.bundles.get(bundleKey);
    if (bundle) {
      bundle.decision = decision;
    }
  }

  /**
   * Record cue details
   */
  recordCue(
    sessionId: string,
    podId: string,
    cueType: string,
    normalizedCue: unknown,
    rawCue?: string
  ): void {
    const bundleKey = `${sessionId}:${podId}`;
    const bundle = this.bundles.get(bundleKey);
    if (bundle) {
      bundle.cueDetails = {
        type: cueType,
        normalized: normalizedCue,
        raw: this.config.includeRawCue ? rawCue : undefined,
      };
    }
  }

  /**
   * Record manifest version served
   */
  recordManifestVersion(
    sessionId: string,
    podId: string,
    version: string
  ): void {
    const bundleKey = `${sessionId}:${podId}`;
    const bundle = this.bundles.get(bundleKey);
    if (bundle && !bundle.manifestVersions.includes(version)) {
      bundle.manifestVersions.push(version);
    }
  }

  /**
   * Record creative selection
   */
  recordCreative(
    sessionId: string,
    podId: string,
    creative: CreativeRecord
  ): void {
    const bundleKey = `${sessionId}:${podId}`;
    const bundle = this.bundles.get(bundleKey);
    if (bundle) {
      bundle.creatives.push(creative);
    }
  }

  /**
   * Record tracking event
   */
  recordEvent(sessionId: string, podId: string, event: TrackingEvent): void {
    const bundleKey = `${sessionId}:${podId}`;
    const bundle = this.bundles.get(bundleKey);
    if (bundle) {
      if (bundle.events.length < this.config.maxEventsPerBundle) {
        bundle.events.push(event);
      }
    }
  }

  /**
   * Generate the final audit bundle
   */
  generate(sessionId: string, podId: string): AuditBundle {
    const bundleKey = `${sessionId}:${podId}`;
    const bundle = this.bundles.get(bundleKey);

    if (!bundle) {
      throw new Error(`No bundle data found for ${bundleKey}`);
    }

    const bundleId = this.generateBundleId();
    const generatedAt = new Date();

    // Build event timeline
    const eventTimeline = bundle.events
      .sort((a, b) => a.timestamp.getTime() - b.timestamp.getTime())
      .map((e) => ({
        timestamp: e.timestamp,
        eventType: e.type,
        slotId: e.slotId,
        source: e.source,
      }));

    // Create bundle object (without signature)
    const auditBundle: Omit<AuditBundle, 'signature'> = {
      bundleId,
      sessionId,
      podId,
      generatedAt,
      decision: bundle.decision || {
        breakId: podId,
        cueReceived: generatedAt,
        podPlanned: generatedAt,
        slotsRequested: 0,
        slotsFilled: 0,
        decisionTimeMs: 0,
      },
      manifestVersions: bundle.manifestVersions,
      cueDetails: bundle.cueDetails || {
        type: 'unknown',
        normalized: {},
      },
      creativesChosen: bundle.creatives,
      eventTimeline,
    };

    // Generate signature
    const signature = this.signBundle(auditBundle);

    const finalBundle: AuditBundle = {
      ...auditBundle,
      signature,
    };

    // Store generated bundle
    this.generatedBundles.set(bundleId, finalBundle);

    // Clean up working data
    this.bundles.delete(bundleKey);

    return finalBundle;
  }

  /**
   * Generate a unique bundle ID
   */
  private generateBundleId(): string {
    const timestamp = Date.now().toString(36);
    const random = randomBytes(8).toString('hex');
    return `audit-${timestamp}-${random}`;
  }

  /**
   * Sign the bundle using HMAC-SHA256
   */
  private signBundle(bundle: Omit<AuditBundle, 'signature'>): string {
    const content = JSON.stringify({
      bundleId: bundle.bundleId,
      sessionId: bundle.sessionId,
      podId: bundle.podId,
      generatedAt: bundle.generatedAt.toISOString(),
      decision: bundle.decision,
      creativesChosen: bundle.creativesChosen,
      eventTimeline: bundle.eventTimeline.map((e) => ({
        ...e,
        timestamp: e.timestamp.toISOString(),
      })),
    });

    const hmac = createHmac('sha256', this.config.signingSecret);
    hmac.update(content);
    return hmac.digest('hex');
  }

  /**
   * Verify a bundle signature
   */
  verifyBundle(bundle: AuditBundle): boolean {
    const { signature, ...bundleWithoutSignature } = bundle;
    const expectedSignature = this.signBundle(bundleWithoutSignature);
    return signature === expectedSignature;
  }

  /**
   * Get a generated bundle by ID
   */
  getBundle(bundleId: string): AuditBundle | null {
    return this.generatedBundles.get(bundleId) || null;
  }

  /**
   * Export bundle as JSON
   */
  exportAsJson(bundleId: string): string | null {
    const bundle = this.generatedBundles.get(bundleId);
    if (!bundle) return null;

    return JSON.stringify(
      bundle,
      (key, value) => {
        if (value instanceof Date) {
          return value.toISOString();
        }
        return value;
      },
      2
    );
  }

  /**
   * Get summary of a bundle for quick reference
   */
  getSummary(bundleId: string): {
    bundleId: string;
    sessionId: string;
    podId: string;
    slotsRequested: number;
    slotsFilled: number;
    impressions: number;
    completions: number;
    errors: number;
    signatureValid: boolean;
  } | null {
    const bundle = this.generatedBundles.get(bundleId);
    if (!bundle) return null;

    const impressions = bundle.eventTimeline.filter(
      (e) => e.eventType === 'impression'
    ).length;
    const completions = bundle.eventTimeline.filter(
      (e) => e.eventType === 'complete'
    ).length;
    const errors = bundle.eventTimeline.filter(
      (e) => e.eventType === 'error'
    ).length;

    return {
      bundleId: bundle.bundleId,
      sessionId: bundle.sessionId,
      podId: bundle.podId,
      slotsRequested: bundle.decision.slotsRequested,
      slotsFilled: bundle.decision.slotsFilled,
      impressions,
      completions,
      errors,
      signatureValid: this.verifyBundle(bundle),
    };
  }

  /**
   * Clean up old bundles
   */
  cleanup(): void {
    const maxAge = this.config.retentionDays * 24 * 60 * 60 * 1000;
    const now = Date.now();

    for (const [bundleId, bundle] of this.generatedBundles) {
      if (now - bundle.generatedAt.getTime() > maxAge) {
        this.generatedBundles.delete(bundleId);
      }
    }
  }

  /**
   * Clear all state
   */
  clear(): void {
    this.bundles.clear();
    this.generatedBundles.clear();
  }

  /**
   * Get statistics
   */
  getStats(): {
    pendingBundles: number;
    generatedBundles: number;
    oldestBundle: Date | null;
  } {
    let oldestBundle: Date | null = null;
    for (const bundle of this.generatedBundles.values()) {
      if (!oldestBundle || bundle.generatedAt < oldestBundle) {
        oldestBundle = bundle.generatedAt;
      }
    }

    return {
      pendingBundles: this.bundles.size,
      generatedBundles: this.generatedBundles.size,
      oldestBundle,
    };
  }
}

/**
 * Rate limiter for tracking events
 */
export class TrackingRateLimiter {
  private eventCounts: Map<string, { count: number; windowStart: Date }> = new Map();
  private readonly windowMs: number;
  private readonly maxEventsPerWindow: number;

  constructor(windowSeconds: number = 60, maxEventsPerWindow: number = 100) {
    this.windowMs = windowSeconds * 1000;
    this.maxEventsPerWindow = maxEventsPerWindow;
  }

  /**
   * Check if an event is allowed
   */
  isAllowed(key: string): boolean {
    const now = Date.now();
    const entry = this.eventCounts.get(key);

    if (!entry) {
      this.eventCounts.set(key, { count: 1, windowStart: new Date(now) });
      return true;
    }

    // Check if window has expired
    if (now - entry.windowStart.getTime() > this.windowMs) {
      entry.count = 1;
      entry.windowStart = new Date(now);
      return true;
    }

    // Check if under limit
    if (entry.count < this.maxEventsPerWindow) {
      entry.count++;
      return true;
    }

    return false;
  }

  /**
   * Get current rate for a key
   */
  getRate(key: string): number {
    const entry = this.eventCounts.get(key);
    if (!entry) return 0;

    const now = Date.now();
    if (now - entry.windowStart.getTime() > this.windowMs) {
      return 0;
    }

    return entry.count;
  }

  /**
   * Clear state
   */
  clear(): void {
    this.eventCounts.clear();
  }
}
