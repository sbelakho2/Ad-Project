/**
 * Frequency Cap Manager
 *
 * Manages per-session, per-device, and per-household frequency caps
 * for SSAI ad serving.
 */

import { SelectedCreative, FrequencyCapState } from './types';

export interface FrequencyCapRule {
  /** Rule ID */
  id: string;

  /** Scope of the cap */
  scope: 'session' | 'device' | 'household' | 'hourly' | 'daily';

  /** What to cap on */
  capOn: 'creative' | 'advertiser' | 'campaign' | 'lineItem';

  /** Maximum impressions allowed */
  maxImpressions: number;

  /** Time window in seconds (for time-based caps) */
  timeWindowSeconds?: number;

  /** Priority (higher = checked first) */
  priority: number;
}

export interface FrequencyCapConfig {
  /** Default session cap per creative */
  defaultSessionCreativeCap: number;

  /** Default session cap per advertiser */
  defaultSessionAdvertiserCap: number;

  /** Default hourly cap per creative */
  defaultHourlyCreativeCap: number;

  /** Default daily cap per creative */
  defaultDailyCreativeCap: number;

  /** Custom rules */
  rules: FrequencyCapRule[];

  /** Enable device-level capping (requires device ID) */
  enableDeviceCapping: boolean;

  /** Enable household-level capping (requires HH ID) */
  enableHouseholdCapping: boolean;

  /** Contextual mode caps (for no-ID scenarios) */
  contextualModeCaps: {
    sessionCreativeCap: number;
    sessionAdvertiserCap: number;
  };
}

export interface FrequencyCapResult {
  allowed: boolean;
  reason?: string;
  violatedRule?: FrequencyCapRule;
  remainingImpressions?: number;
}

/**
 * Frequency Cap Manager tracks and enforces impression caps
 * at various scopes and granularities.
 */
export class FrequencyCapManager {
  private config: FrequencyCapConfig;
  private sessionCaps: Map<string, FrequencyCapState>;
  private hourlyCaps: Map<string, Record<string, number>>;
  private dailyCaps: Map<string, Record<string, number>>;
  private hourlyResetTime: number = 0;
  private dailyResetTime: number = 0;

  constructor(config?: Partial<FrequencyCapConfig>) {
    this.config = {
      defaultSessionCreativeCap: 3,
      defaultSessionAdvertiserCap: 10,
      defaultHourlyCreativeCap: 5,
      defaultDailyCreativeCap: 10,
      rules: [],
      enableDeviceCapping: true,
      enableHouseholdCapping: false,
      contextualModeCaps: {
        sessionCreativeCap: 2,
        sessionAdvertiserCap: 5,
      },
      ...config,
    };

    this.sessionCaps = new Map();
    this.hourlyCaps = new Map();
    this.dailyCaps = new Map();

    this.resetHourlyCaps();
    this.resetDailyCaps();
  }

  /**
   * Check if a creative can be shown.
   */
  checkCap(
    creative: SelectedCreative,
    sessionId: string,
    deviceId?: string,
    householdId?: string,
    isContextualMode = false
  ): FrequencyCapResult {
    // Clean up expired caps
    this.cleanupExpiredCaps();

    // Get or create session state
    const sessionState = this.getOrCreateSessionState(sessionId);

    // Apply contextual mode limits if no ID available
    const effectiveConfig = isContextualMode
      ? {
          ...this.config,
          defaultSessionCreativeCap: this.config.contextualModeCaps.sessionCreativeCap,
          defaultSessionAdvertiserCap:
            this.config.contextualModeCaps.sessionAdvertiserCap,
        }
      : this.config;

    // Check session creative cap
    const sessionCreativeCount = sessionState.session[creative.creativeId] || 0;
    if (sessionCreativeCount >= effectiveConfig.defaultSessionCreativeCap) {
      return {
        allowed: false,
        reason: `Session creative cap exceeded (${sessionCreativeCount}/${effectiveConfig.defaultSessionCreativeCap})`,
        remainingImpressions: 0,
      };
    }

    // Check session advertiser cap
    const sessionAdvertiserCount =
      sessionState.advertiserSession[creative.advertiserId] || 0;
    if (
      sessionAdvertiserCount >= effectiveConfig.defaultSessionAdvertiserCap
    ) {
      return {
        allowed: false,
        reason: `Session advertiser cap exceeded (${sessionAdvertiserCount}/${effectiveConfig.defaultSessionAdvertiserCap})`,
        remainingImpressions: 0,
      };
    }

    // Check hourly cap
    const hourlyKey = this.getHourlyKey(sessionId, deviceId);
    const hourlyState = this.hourlyCaps.get(hourlyKey) || {};
    const hourlyCount = hourlyState[creative.creativeId] || 0;
    if (hourlyCount >= this.config.defaultHourlyCreativeCap) {
      return {
        allowed: false,
        reason: `Hourly creative cap exceeded (${hourlyCount}/${this.config.defaultHourlyCreativeCap})`,
        remainingImpressions: 0,
      };
    }

    // Check daily cap
    const dailyKey = this.getDailyKey(sessionId, deviceId);
    const dailyState = this.dailyCaps.get(dailyKey) || {};
    const dailyCount = dailyState[creative.creativeId] || 0;
    if (dailyCount >= this.config.defaultDailyCreativeCap) {
      return {
        allowed: false,
        reason: `Daily creative cap exceeded (${dailyCount}/${this.config.defaultDailyCreativeCap})`,
        remainingImpressions: 0,
      };
    }

    // Check custom rules
    const customResult = this.checkCustomRules(
      creative,
      sessionId,
      sessionState,
      hourlyState,
      dailyState
    );
    if (!customResult.allowed) {
      return customResult;
    }

    return {
      allowed: true,
      remainingImpressions:
        effectiveConfig.defaultSessionCreativeCap - sessionCreativeCount - 1,
    };
  }

  /**
   * Record an impression.
   */
  recordImpression(
    creative: SelectedCreative,
    sessionId: string,
    deviceId?: string,
    _householdId?: string
  ): void {
    // Update session state
    const sessionState = this.getOrCreateSessionState(sessionId);
    sessionState.session[creative.creativeId] =
      (sessionState.session[creative.creativeId] || 0) + 1;
    sessionState.advertiserSession[creative.advertiserId] =
      (sessionState.advertiserSession[creative.advertiserId] || 0) + 1;

    // Update hourly state
    const hourlyKey = this.getHourlyKey(sessionId, deviceId);
    if (!this.hourlyCaps.has(hourlyKey)) {
      this.hourlyCaps.set(hourlyKey, {});
    }
    const hourlyState = this.hourlyCaps.get(hourlyKey)!;
    hourlyState[creative.creativeId] = (hourlyState[creative.creativeId] || 0) + 1;

    // Update daily state
    const dailyKey = this.getDailyKey(sessionId, deviceId);
    if (!this.dailyCaps.has(dailyKey)) {
      this.dailyCaps.set(dailyKey, {});
    }
    const dailyState = this.dailyCaps.get(dailyKey)!;
    dailyState[creative.creativeId] = (dailyState[creative.creativeId] || 0) + 1;
  }

  /**
   * Get current cap state for a session.
   */
  getSessionState(sessionId: string): FrequencyCapState | undefined {
    return this.sessionCaps.get(sessionId);
  }

  /**
   * Get remaining impressions for a creative.
   */
  getRemainingImpressions(
    creativeId: string,
    sessionId: string,
    _deviceId?: string
  ): number {
    const sessionState = this.sessionCaps.get(sessionId);
    if (!sessionState) {
      return this.config.defaultSessionCreativeCap;
    }

    const used = sessionState.session[creativeId] || 0;
    return Math.max(0, this.config.defaultSessionCreativeCap - used);
  }

  /**
   * Check custom frequency cap rules.
   */
  private checkCustomRules(
    creative: SelectedCreative,
    _sessionId: string,
    sessionState: FrequencyCapState,
    hourlyState: Record<string, number>,
    dailyState: Record<string, number>
  ): FrequencyCapResult {
    // Sort rules by priority
    const sortedRules = [...this.config.rules].sort(
      (a, b) => b.priority - a.priority
    );

    for (const rule of sortedRules) {
      const capKey = this.getCapKey(creative, rule.capOn);
      if (!capKey) {
        continue;
      }

      let count = 0;

      switch (rule.scope) {
        case 'session':
          count = sessionState.session[capKey] || 0;
          break;
        case 'hourly':
          count = hourlyState[capKey] || 0;
          break;
        case 'daily':
          count = dailyState[capKey] || 0;
          break;
        // Device and household would require additional state
        default:
          continue;
      }

      if (count >= rule.maxImpressions) {
        return {
          allowed: false,
          reason: `Rule ${rule.id} cap exceeded (${count}/${rule.maxImpressions})`,
          violatedRule: rule,
          remainingImpressions: 0,
        };
      }
    }

    return { allowed: true };
  }

  /**
   * Get cap key based on cap target.
   */
  private getCapKey(
    creative: SelectedCreative,
    capOn: FrequencyCapRule['capOn']
  ): string | undefined {
    switch (capOn) {
      case 'creative':
        return creative.creativeId;
      case 'advertiser':
        return creative.advertiserId;
      case 'campaign':
        return creative.campaignId;
      case 'lineItem':
        return creative.lineItemId;
      default:
        return undefined;
    }
  }

  /**
   * Get or create session state.
   */
  private getOrCreateSessionState(sessionId: string): FrequencyCapState {
    if (!this.sessionCaps.has(sessionId)) {
      this.sessionCaps.set(sessionId, {
        session: {},
        advertiserSession: {},
        hourly: {},
        daily: {},
      });
    }
    return this.sessionCaps.get(sessionId)!;
  }

  /**
   * Get hourly cap key.
   */
  private getHourlyKey(sessionId: string, deviceId?: string): string {
    return deviceId || sessionId;
  }

  /**
   * Get daily cap key.
   */
  private getDailyKey(sessionId: string, deviceId?: string): string {
    return deviceId || sessionId;
  }

  /**
   * Clean up expired caps.
   */
  private cleanupExpiredCaps(): void {
    const now = Date.now();

    // Reset hourly caps if hour changed
    if (now - this.hourlyResetTime > 3600000) {
      this.resetHourlyCaps();
    }

    // Reset daily caps if day changed
    if (now - this.dailyResetTime > 86400000) {
      this.resetDailyCaps();
    }

    // Clean up old session caps (sessions older than 4 hours)
    const sessionMaxAge = 4 * 3600000;
    for (const [sessionId] of this.sessionCaps.entries()) {
      // In production, would check session creation time
      // For now, we keep all sessions
    }
  }

  /**
   * Reset hourly caps.
   */
  private resetHourlyCaps(): void {
    this.hourlyCaps.clear();
    this.hourlyResetTime = Date.now();
  }

  /**
   * Reset daily caps.
   */
  private resetDailyCaps(): void {
    this.dailyCaps.clear();
    this.dailyResetTime = Date.now();
  }

  /**
   * Add custom rule.
   */
  addRule(rule: FrequencyCapRule): void {
    this.config.rules.push(rule);
  }

  /**
   * Remove custom rule.
   */
  removeRule(ruleId: string): void {
    this.config.rules = this.config.rules.filter((r) => r.id !== ruleId);
  }

  /**
   * Clear session state.
   */
  clearSession(sessionId: string): void {
    this.sessionCaps.delete(sessionId);
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<FrequencyCapConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Get current configuration.
   */
  getConfig(): FrequencyCapConfig {
    return { ...this.config };
  }
}

export default FrequencyCapManager;
