/**
 * SSAI Kill Switch Manager
 *
 * Provides instant-off capabilities for any component or partner.
 * Used for incident response and controlled degradation.
 */

import {
  KillSwitchConfig,
  KillSwitchScope,
  KillSwitchResult,
} from './types';

export interface KillSwitchManagerConfig {
  /** Default fallback action */
  defaultFallback: 'slate' | 'passthrough' | 'reject';

  /** Check frequency for remote config (ms) */
  refreshIntervalMs: number;

  /** Enable local caching */
  enableCaching: boolean;
}

const DEFAULT_CONFIG: KillSwitchManagerConfig = {
  defaultFallback: 'slate',
  refreshIntervalMs: 30000,
  enableCaching: true,
};

export class KillSwitchManager {
  private config: KillSwitchManagerConfig;
  private killSwitches: Map<string, KillSwitchConfig> = new Map();
  private refreshTimer: NodeJS.Timeout | null = null;

  constructor(config: Partial<KillSwitchManagerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Start the manager (load config and start refresh)
   */
  async start(): Promise<void> {
    await this.loadConfig();

    if (this.config.enableCaching && this.config.refreshIntervalMs > 0) {
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
   * Load configuration (override in production for remote config)
   */
  private async loadConfig(): Promise<void> {
    // In production, this would fetch from Redis/remote config
    // For now, use local state
  }

  /**
   * Create a kill switch
   */
  createKillSwitch(
    id: string,
    name: string,
    scope: KillSwitchScope,
    reason: string,
    options: {
      enabled?: boolean;
      autoRestoreMinutes?: number;
      updatedBy?: string;
    } = {}
  ): KillSwitchConfig {
    const killSwitch: KillSwitchConfig = {
      id,
      name,
      enabled: options.enabled ?? true,
      scope,
      lastUpdated: new Date(),
      updatedBy: options.updatedBy || 'system',
      reason,
      autoRestoreAt: options.autoRestoreMinutes
        ? new Date(Date.now() + options.autoRestoreMinutes * 60 * 1000)
        : undefined,
    };

    this.killSwitches.set(id, killSwitch);
    return killSwitch;
  }

  /**
   * Activate a kill switch
   */
  activate(
    id: string,
    reason: string,
    options: {
      autoRestoreMinutes?: number;
      updatedBy?: string;
    } = {}
  ): boolean {
    const killSwitch = this.killSwitches.get(id);
    if (!killSwitch) return false;

    killSwitch.enabled = true;
    killSwitch.reason = reason;
    killSwitch.lastUpdated = new Date();
    killSwitch.updatedBy = options.updatedBy || 'system';
    killSwitch.autoRestoreAt = options.autoRestoreMinutes
      ? new Date(Date.now() + options.autoRestoreMinutes * 60 * 1000)
      : undefined;

    return true;
  }

  /**
   * Deactivate a kill switch
   */
  deactivate(id: string, reason: string, updatedBy?: string): boolean {
    const killSwitch = this.killSwitches.get(id);
    if (!killSwitch) return false;

    killSwitch.enabled = false;
    killSwitch.reason = reason;
    killSwitch.lastUpdated = new Date();
    killSwitch.updatedBy = updatedBy || 'system';
    killSwitch.autoRestoreAt = undefined;

    return true;
  }

  /**
   * Check if operation should proceed
   */
  check(context: {
    tenantId?: string;
    partnerId?: string;
    creativeId?: string;
    regionCode?: string;
  }): KillSwitchResult {
    const now = Date.now();
    const activeKillSwitches: string[] = [];
    let fallbackAction: 'slate' | 'passthrough' | 'reject' | undefined;

    // Check auto-restore
    for (const [id, ks] of this.killSwitches) {
      if (ks.enabled && ks.autoRestoreAt && ks.autoRestoreAt.getTime() <= now) {
        ks.enabled = false;
        ks.reason = 'Auto-restored after timeout';
        ks.lastUpdated = new Date();
        ks.updatedBy = 'system';
        ks.autoRestoreAt = undefined;
      }
    }

    // Check each kill switch
    for (const [id, ks] of this.killSwitches) {
      if (!ks.enabled) continue;

      if (this.matchesScope(ks.scope, context)) {
        activeKillSwitches.push(id);
        fallbackAction = fallbackAction || this.config.defaultFallback;
      }
    }

    return {
      proceed: activeKillSwitches.length === 0,
      activeKillSwitches,
      fallbackAction,
    };
  }

  /**
   * Check if a scope matches the context
   */
  private matchesScope(
    scope: KillSwitchScope,
    context: {
      tenantId?: string;
      partnerId?: string;
      creativeId?: string;
      regionCode?: string;
    }
  ): boolean {
    switch (scope.type) {
      case 'global':
        return true;
      case 'tenant':
        return context.tenantId === scope.tenantId;
      case 'partner':
        return context.partnerId === scope.partnerId;
      case 'creative':
        return context.creativeId === scope.creativeId;
      case 'region':
        return context.regionCode === scope.regionCode;
      default:
        return false;
    }
  }

  /**
   * Get all kill switches
   */
  getAll(): KillSwitchConfig[] {
    return Array.from(this.killSwitches.values());
  }

  /**
   * Get active kill switches
   */
  getActive(): KillSwitchConfig[] {
    return Array.from(this.killSwitches.values()).filter((ks) => ks.enabled);
  }

  /**
   * Get a specific kill switch
   */
  get(id: string): KillSwitchConfig | null {
    return this.killSwitches.get(id) || null;
  }

  /**
   * Delete a kill switch
   */
  delete(id: string): boolean {
    return this.killSwitches.delete(id);
  }

  /**
   * Clear all kill switches (for testing)
   */
  clear(): void {
    this.killSwitches.clear();
  }
}
