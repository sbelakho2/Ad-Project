/**
 * SSAI Rollout Manager
 *
 * Manages feature rollouts with percentage-based and targeted deployment.
 * Supports canary releases, regional rollouts, and A/B testing.
 */

import { RolloutConfig, RolloutDecision, RolloutStrategy } from './types';
import { createHash } from 'crypto';

export interface RolloutManagerConfig {
  /** Default percentage for new features */
  defaultPercentage: number;

  /** Enable sticky assignment */
  enableStickyAssignment: boolean;

  /** Refresh interval for remote config (ms) */
  refreshIntervalMs: number;
}

const DEFAULT_CONFIG: RolloutManagerConfig = {
  defaultPercentage: 0,
  enableStickyAssignment: true,
  refreshIntervalMs: 60000,
};

export class RolloutManager {
  private config: RolloutManagerConfig;
  private features: Map<string, RolloutConfig> = new Map();
  private allowlists: Map<string, Set<string>> = new Map();
  private denylists: Map<string, Set<string>> = new Map();
  private refreshTimer: NodeJS.Timeout | null = null;

  constructor(config: Partial<RolloutManagerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Start the manager
   */
  async start(): Promise<void> {
    await this.loadConfig();

    if (this.config.refreshIntervalMs > 0) {
      this.refreshTimer = setInterval(() => {
        this.loadConfig().catch(console.error);
      }, this.config.refreshIntervalMs);
    }
  }

  /**
   * Stop the manager
   */
  stop(): void {
    if (this.refreshTimer) {
      clearInterval(this.refreshTimer);
      this.refreshTimer = null;
    }
  }

  /**
   * Load configuration (override for remote config)
   */
  private async loadConfig(): Promise<void> {
    // In production, fetch from remote config service
  }

  /**
   * Register a feature with rollout config
   */
  registerFeature(featureName: string, config: Partial<RolloutConfig>): void {
    const rolloutConfig: RolloutConfig = {
      featureName,
      percentage: config.percentage ?? this.config.defaultPercentage,
      strategy: config.strategy ?? 'percentage',
      stickyKey: config.stickyKey,
      enabledRegions: config.enabledRegions,
      disabledRegions: config.disabledRegions,
      minVersion: config.minVersion,
      lastUpdated: new Date(),
    };

    this.features.set(featureName, rolloutConfig);
  }

  /**
   * Update rollout percentage
   */
  setPercentage(featureName: string, percentage: number): boolean {
    const feature = this.features.get(featureName);
    if (!feature) return false;

    feature.percentage = Math.max(0, Math.min(100, percentage));
    feature.lastUpdated = new Date();
    return true;
  }

  /**
   * Add to allowlist
   */
  addToAllowlist(featureName: string, ids: string[]): void {
    if (!this.allowlists.has(featureName)) {
      this.allowlists.set(featureName, new Set());
    }
    const list = this.allowlists.get(featureName)!;
    for (const id of ids) {
      list.add(id);
    }
  }

  /**
   * Add to denylist
   */
  addToDenylist(featureName: string, ids: string[]): void {
    if (!this.denylists.has(featureName)) {
      this.denylists.set(featureName, new Set());
    }
    const list = this.denylists.get(featureName)!;
    for (const id of ids) {
      list.add(id);
    }
  }

  /**
   * Check if a feature is enabled for given context
   */
  isEnabled(
    featureName: string,
    context: {
      sessionId?: string;
      deviceId?: string;
      tenantId?: string;
      regionCode?: string;
      version?: string;
    }
  ): RolloutDecision {
    const feature = this.features.get(featureName);

    if (!feature) {
      return {
        enabled: false,
        config: {
          featureName,
          percentage: 0,
          strategy: 'percentage',
          lastUpdated: new Date(),
        },
        reason: 'Feature not registered',
      };
    }

    // Check denylist first
    const denylist = this.denylists.get(featureName);
    const contextId = this.getContextId(feature, context);
    if (denylist && contextId && denylist.has(contextId)) {
      return {
        enabled: false,
        config: feature,
        reason: 'On denylist',
      };
    }

    // Check allowlist
    const allowlist = this.allowlists.get(featureName);
    if (allowlist && contextId && allowlist.has(contextId)) {
      return {
        enabled: true,
        config: feature,
        reason: 'On allowlist',
      };
    }

    // Check region restrictions
    if (context.regionCode) {
      if (feature.disabledRegions?.includes(context.regionCode)) {
        return {
          enabled: false,
          config: feature,
          reason: `Disabled in region ${context.regionCode}`,
        };
      }
      if (feature.enabledRegions && !feature.enabledRegions.includes(context.regionCode)) {
        return {
          enabled: false,
          config: feature,
          reason: `Not enabled in region ${context.regionCode}`,
        };
      }
    }

    // Check version requirement
    if (feature.minVersion && context.version) {
      if (!this.meetsVersionRequirement(context.version, feature.minVersion)) {
        return {
          enabled: false,
          config: feature,
          reason: `Version ${context.version} below minimum ${feature.minVersion}`,
        };
      }
    }

    // Apply rollout strategy
    switch (feature.strategy) {
      case 'percentage':
        return this.percentageDecision(feature, context);
      case 'canary':
        return this.canaryDecision(feature, context);
      case 'allowlist':
        return {
          enabled: false,
          config: feature,
          reason: 'Not on allowlist',
        };
      case 'denylist':
        return {
          enabled: true,
          config: feature,
          reason: 'Not on denylist',
        };
      case 'geographic':
        return this.geographicDecision(feature, context);
      default:
        return {
          enabled: false,
          config: feature,
          reason: 'Unknown strategy',
        };
    }
  }

  /**
   * Get context ID based on sticky key
   */
  private getContextId(
    feature: RolloutConfig,
    context: {
      sessionId?: string;
      deviceId?: string;
      tenantId?: string;
    }
  ): string | null {
    switch (feature.stickyKey) {
      case 'session':
        return context.sessionId || null;
      case 'device':
        return context.deviceId || null;
      case 'tenant':
        return context.tenantId || null;
      default:
        return context.sessionId || context.deviceId || context.tenantId || null;
    }
  }

  /**
   * Make percentage-based decision
   */
  private percentageDecision(
    feature: RolloutConfig,
    context: { sessionId?: string; deviceId?: string; tenantId?: string }
  ): RolloutDecision {
    if (feature.percentage >= 100) {
      return {
        enabled: true,
        config: feature,
        reason: '100% rollout',
      };
    }

    if (feature.percentage <= 0) {
      return {
        enabled: false,
        config: feature,
        reason: '0% rollout',
      };
    }

    // Get sticky hash
    const contextId = this.getContextId(feature, context);
    if (!contextId) {
      // Random decision without sticky assignment
      const enabled = Math.random() * 100 < feature.percentage;
      return {
        enabled,
        config: feature,
        reason: enabled ? 'Random selection (enabled)' : 'Random selection (disabled)',
      };
    }

    // Deterministic hash-based decision
    const hash = this.hashToPercentage(feature.featureName, contextId);
    const enabled = hash < feature.percentage;

    return {
      enabled,
      config: feature,
      reason: enabled
        ? `Hash ${hash.toFixed(2)}% < ${feature.percentage}%`
        : `Hash ${hash.toFixed(2)}% >= ${feature.percentage}%`,
    };
  }

  /**
   * Make canary decision (first X% of sessions)
   */
  private canaryDecision(
    feature: RolloutConfig,
    context: { sessionId?: string; deviceId?: string; tenantId?: string }
  ): RolloutDecision {
    // Canary uses same logic as percentage but with canary-specific key
    return this.percentageDecision(feature, context);
  }

  /**
   * Make geographic decision
   */
  private geographicDecision(
    feature: RolloutConfig,
    context: { regionCode?: string }
  ): RolloutDecision {
    if (!context.regionCode) {
      return {
        enabled: false,
        config: feature,
        reason: 'No region code provided',
      };
    }

    if (feature.enabledRegions?.includes(context.regionCode)) {
      return {
        enabled: true,
        config: feature,
        reason: `Enabled in region ${context.regionCode}`,
      };
    }

    return {
      enabled: false,
      config: feature,
      reason: `Not enabled in region ${context.regionCode}`,
    };
  }

  /**
   * Hash a feature+context to a percentage (0-100)
   */
  private hashToPercentage(featureName: string, contextId: string): number {
    const input = `${featureName}:${contextId}`;
    const hash = createHash('md5').update(input).digest();
    // Use first 4 bytes as uint32 and convert to percentage
    const uint32 = hash.readUInt32BE(0);
    return (uint32 / 0xffffffff) * 100;
  }

  /**
   * Check if version meets requirement
   */
  private meetsVersionRequirement(current: string, required: string): boolean {
    const currentParts = current.split('.').map(Number);
    const requiredParts = required.split('.').map(Number);

    for (let i = 0; i < Math.max(currentParts.length, requiredParts.length); i++) {
      const curr = currentParts[i] || 0;
      const req = requiredParts[i] || 0;

      if (curr > req) return true;
      if (curr < req) return false;
    }

    return true;
  }

  /**
   * Get all features
   */
  getAll(): RolloutConfig[] {
    return Array.from(this.features.values());
  }

  /**
   * Get a specific feature
   */
  get(featureName: string): RolloutConfig | null {
    return this.features.get(featureName) || null;
  }

  /**
   * Delete a feature
   */
  delete(featureName: string): boolean {
    this.allowlists.delete(featureName);
    this.denylists.delete(featureName);
    return this.features.delete(featureName);
  }

  /**
   * Clear all features
   */
  clear(): void {
    this.features.clear();
    this.allowlists.clear();
    this.denylists.clear();
  }
}
