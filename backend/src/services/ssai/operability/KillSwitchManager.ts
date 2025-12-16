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

type KillSwitchAction = 'slate' | 'passthrough' | 'reject';
type KillSwitchScopeKind = 'global' | 'tenant' | 'partner' | 'creative' | 'region';

export interface KillSwitchActivationRequest {
  scope: KillSwitchScopeKind;
  scopeId?: string;
  reason: string;
  action: KillSwitchAction;
  autoRestoreAfterMs?: number;
}

export interface KillSwitchCheckRequestContext {
  tenantId?: string;
  partnerId?: string;
  creativeId?: string;
  region?: string;
}

export interface KillSwitchCheckRequestResult {
  blocked: boolean;
  action?: KillSwitchAction;
  reason?: string;
  scope?: KillSwitchScopeKind;
  scopeId?: string;
}

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
  private actions: Map<string, KillSwitchAction> = new Map();
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
  activate(request: KillSwitchActivationRequest): void;
  activate(
    id: string,
    reason: string,
    options?: {
      autoRestoreMinutes?: number;
      updatedBy?: string;
    }
  ): boolean;
  activate(
    arg1: KillSwitchActivationRequest | string,
    reason?: string,
    options: {
      autoRestoreMinutes?: number;
      updatedBy?: string;
    } = {}
  ): boolean | void {
    // Test-facing API
    if (typeof arg1 !== 'string') {
      const key = this.toKey(arg1.scope, arg1.scopeId);
      const scope = this.toScope(arg1.scope, arg1.scopeId);

      const killSwitch: KillSwitchConfig = {
        id: key,
        name: key,
        enabled: true,
        scope,
        lastUpdated: new Date(),
        updatedBy: 'system',
        reason: arg1.reason,
        autoRestoreAt: arg1.autoRestoreAfterMs
          ? new Date(Date.now() + arg1.autoRestoreAfterMs)
          : undefined,
      };

      this.killSwitches.set(key, killSwitch);
      this.actions.set(key, arg1.action);
      return;
    }

    // Backward-compatible API
    const id = arg1;
    const killSwitch = this.killSwitches.get(id);
    if (!killSwitch) return false;

    killSwitch.enabled = true;
    killSwitch.reason = reason;
    killSwitch.lastUpdated = new Date();
    killSwitch.updatedBy = options.updatedBy || 'system';
    killSwitch.autoRestoreAt = options.autoRestoreMinutes
      ? new Date(Date.now() + options.autoRestoreMinutes * 60 * 1000)
      : undefined;

    if (!this.actions.has(id)) {
      this.actions.set(id, this.config.defaultFallback);
    }

    return true;
  }

  /**
   * Deactivate a kill switch
   */
  deactivate(scope: KillSwitchScopeKind, scopeId?: string): void;
  deactivate(id: string, reason: string, updatedBy?: string): boolean;
  deactivate(
    arg1: string,
    arg2?: string,
    arg3?: string
  ): boolean | void {
    // Test-facing API: deactivate('global') / deactivate('tenant','tenant-123')
    if (
      arg1 === 'global' ||
      arg1 === 'tenant' ||
      arg1 === 'partner' ||
      arg1 === 'creative' ||
      arg1 === 'region'
    ) {
      const key = this.toKey(arg1, arg2);
      const ks = this.killSwitches.get(key);
      if (!ks) return;
      ks.enabled = false;
      ks.lastUpdated = new Date();
      ks.reason = 'Deactivated';
      ks.updatedBy = 'system';
      ks.autoRestoreAt = undefined;
      return;
    }

    // Backward-compatible API
    const id = arg1;
    const reason = arg2;
    const updatedBy = arg3;
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
   * Test-facing API: check if a given scope is active.
   */
  isActive(scope: KillSwitchScopeKind, scopeId?: string): boolean {
    this.autoRestoreExpired();
    const key = this.toKey(scope, scopeId);
    const ks = this.killSwitches.get(key);
    return Boolean(ks?.enabled);
  }

  /**
   * Test-facing API: evaluate whether a request should be blocked.
   */
  checkRequest(context: KillSwitchCheckRequestContext): KillSwitchCheckRequestResult {
    this.autoRestoreExpired();

    // Precedence order: global -> tenant -> partner -> creative -> region
    const checks: Array<{ scope: KillSwitchScopeKind; scopeId?: string }> = [
      { scope: 'global' },
      context.tenantId ? { scope: 'tenant', scopeId: context.tenantId } : (null as any),
      context.partnerId ? { scope: 'partner', scopeId: context.partnerId } : (null as any),
      context.creativeId ? { scope: 'creative', scopeId: context.creativeId } : (null as any),
      context.region ? { scope: 'region', scopeId: context.region } : (null as any),
    ].filter(Boolean);

    for (const check of checks) {
      const key = this.toKey(check.scope, check.scopeId);
      const ks = this.killSwitches.get(key);
      if (!ks?.enabled) continue;

      return {
        blocked: true,
        action: this.actions.get(key) || this.config.defaultFallback,
        reason: ks.reason,
        scope: check.scope,
        scopeId: check.scopeId,
      };
    }

    return { blocked: false };
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
    const activeKillSwitches: string[] = [];
    let fallbackAction: 'slate' | 'passthrough' | 'reject' | undefined;

    this.autoRestoreExpired();

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

  private toKey(scope: KillSwitchScopeKind, scopeId?: string): string {
    return scopeId ? `${scope}:${scopeId}` : scope;
  }

  private toScope(scope: KillSwitchScopeKind, scopeId?: string): KillSwitchScope {
    switch (scope) {
      case 'global':
        return { type: 'global' };
      case 'tenant':
        return { type: 'tenant', tenantId: scopeId || '' };
      case 'partner':
        return { type: 'partner', partnerId: scopeId || '' };
      case 'creative':
        return { type: 'creative', creativeId: scopeId || '' };
      case 'region':
        return { type: 'region', regionCode: scopeId || '' };
    }
  }

  private autoRestoreExpired(): void {
    const now = Date.now();
    for (const ks of this.killSwitches.values()) {
      if (!ks.enabled) continue;
      if (!ks.autoRestoreAt) continue;
      if (ks.autoRestoreAt.getTime() > now) continue;

      ks.enabled = false;
      ks.reason = 'Auto-restored after timeout';
      ks.lastUpdated = new Date();
      ks.updatedBy = 'system';
      ks.autoRestoreAt = undefined;
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
    this.actions.clear();
  }
}
