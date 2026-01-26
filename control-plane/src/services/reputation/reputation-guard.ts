/**
 * REPUTATION GUARD (DMARC/SPF Delivery Monitoring)
 * 
 * Monitors email delivery health and protects domain reputation.
 * Analyzes bounce messages for reputation-damaging patterns.
 * 
 * Features:
 *   - Parse bounce messages for DMARC/SPF/DKIM failures
 *   - Track reputation metrics over time
 *   - Auto-pause campaigns when reputation is threatened
 *   - Alert operators of critical issues
 * 
 * Bounce Codes Monitored:
 *   - 5.7.1: Message rejected due to policy (DMARC, reputation)
 *   - 5.7.23: SPF validation failed
 *   - 5.7.25: DKIM validation failed
 *   - 5.7.26: DMARC policy failure
 *   - 550: Generic rejection
 * 
 * Architecture:
 *   - Runs on Control Plane (App Server)
 *   - Analyzes messages classified as BOUNCE by Clerk
 *   - Stores metrics in database
 *   - Triggers alerts via operator notification system
 */

import { queryOperator, queryReadOnly } from '../../database';
import { logger } from '../../logger';

// ============================================================================
// TYPES
// ============================================================================

export type ReputationRisk = 'LOW' | 'MEDIUM' | 'HIGH' | 'CRITICAL';

export interface BounceAnalysis {
  messageId: string;
  bounceType: 'hard' | 'soft' | 'policy';
  errorCode?: string;
  errorMessage?: string;
  isDmarcFailure: boolean;
  isSpfFailure: boolean;
  isDkimFailure: boolean;
  isReputationIssue: boolean;
  isMailboxFull: boolean;
  isInvalidAddress: boolean;
  rawDiagnostic: string | null;
  risk: ReputationRisk;
  suggestedAction: string;
}

export interface ReputationMetrics {
  period: 'hour' | 'day' | 'week';
  totalSent: number;
  totalBounced: number;
  hardBounces: number;
  softBounces: number;
  policyBounces: number;
  dmarcFailures: number;
  spfFailures: number;
  dkimFailures: number;
  bounceRate: number;
  reputationScore: number;
  risk: ReputationRisk;
  timestamp: Date;
}

export interface ReputationAlert {
  id: string;
  severity: 'info' | 'warning' | 'critical';
  type: string;
  message: string;
  details: Record<string, unknown>;
  acknowledged: boolean;
  createdAt: Date;
}

// ============================================================================
// BOUNCE PATTERN MATCHERS
// ============================================================================

const BOUNCE_PATTERNS = {
  // DMARC failures
  dmarc: [
    /dmarc/i,
    /5\.7\.26/,
    /dmarc policy/i,
    /dmarc check failed/i,
    /message failed dmarc/i,
  ],
  
  // SPF failures
  spf: [
    /spf/i,
    /5\.7\.23/,
    /sender policy framework/i,
    /spf check failed/i,
    /spf validation failed/i,
  ],
  
  // DKIM failures
  dkim: [
    /dkim/i,
    /5\.7\.25/,
    /domainkeys/i,
    /dkim signature/i,
    /dkim check failed/i,
  ],
  
  // Reputation issues
  reputation: [
    /5\.7\.1/,
    /blocked/i,
    /blacklist/i,
    /spam/i,
    /reputation/i,
    /policy rejection/i,
    /rejected for policy/i,
    /message rejected/i,
    /ip (address )?blocked/i,
    /sender rejected/i,
  ],
  
  // Hard bounces (permanent)
  hardBounce: [
    /5\.[12]\.\d/,
    /user (not found|unknown|doesn'?t exist)/i,
    /mailbox (not found|unavailable|does not exist)/i,
    /no such user/i,
    /recipient rejected/i,
    /address rejected/i,
    /invalid recipient/i,
    /unknown user/i,
  ],
  
  // Soft bounces (temporary)
  softBounce: [
    /4\.\d\.\d/,
    /mailbox full/i,
    /over quota/i,
    /try again later/i,
    /temporarily/i,
    /too many connections/i,
    /rate limit/i,
    /greylisted/i,
  ],
};

// ============================================================================
// REPUTATION GUARD CLASS
// ============================================================================

export class ReputationGuard {
  private alertCallback?: (alert: ReputationAlert) => Promise<void>;
  private pauseCallback?: () => Promise<void>;
  
  // Thresholds for auto-pause
  private readonly criticalBounceRate = 0.10; // 10% bounce rate
  private readonly criticalPolicyBounces = 5;  // 5 policy bounces in a day
  private readonly criticalDmarcFailures = 3;  // 3 DMARC failures
  
  constructor(options?: {
    onAlert?: (alert: ReputationAlert) => Promise<void>;
    onPause?: () => Promise<void>;
  }) {
    this.alertCallback = options?.onAlert;
    this.pauseCallback = options?.onPause;
  }
  
  /**
   * Analyze a bounce message and extract reputation signals
   */
  analyzeBounce(
    subject: string,
    body: string,
    originalMessageId?: string
  ): BounceAnalysis {
    const fullText = `${subject}\n${body}`;
    
    // Check each pattern category
    const isDmarcFailure = BOUNCE_PATTERNS.dmarc.some(p => p.test(fullText));
    const isSpfFailure = BOUNCE_PATTERNS.spf.some(p => p.test(fullText));
    const isDkimFailure = BOUNCE_PATTERNS.dkim.some(p => p.test(fullText));
    const isReputationIssue = BOUNCE_PATTERNS.reputation.some(p => p.test(fullText));
    const isHardBounce = BOUNCE_PATTERNS.hardBounce.some(p => p.test(fullText));
    const isSoftBounce = BOUNCE_PATTERNS.softBounce.some(p => p.test(fullText));
    const isMailboxFull = /mailbox full|over quota/i.test(fullText);
    const isInvalidAddress = /user (not found|unknown)|no such user|invalid/i.test(fullText);
    
    // Extract error code
    const codeMatch = fullText.match(/([45]\.\d\.\d+)/);
    const errorCode = codeMatch?.[1] || undefined;
    
    // Extract diagnostic message
    const diagnosticMatch = fullText.match(/diagnostic[- ]code[:\s]+([^\n]+)/i);
    const rawDiagnostic = diagnosticMatch?.[1]?.trim() || null;
    
    // Determine bounce type
    let bounceType: 'hard' | 'soft' | 'policy';
    if (isDmarcFailure || isSpfFailure || isDkimFailure || isReputationIssue) {
      bounceType = 'policy';
    } else if (isHardBounce || isInvalidAddress) {
      bounceType = 'hard';
    } else {
      bounceType = 'soft';
    }
    
    // Calculate risk level
    let risk: ReputationRisk;
    let suggestedAction: string;
    
    if (isDmarcFailure || isSpfFailure || isDkimFailure) {
      risk = 'CRITICAL';
      suggestedAction = 'PAUSE IMMEDIATELY. Check DNS records (SPF, DKIM, DMARC) for sending domain.';
    } else if (isReputationIssue) {
      risk = 'HIGH';
      suggestedAction = 'Review recent sending patterns. May need to warm up IP or contact blocklist operators.';
    } else if (isHardBounce) {
      risk = 'MEDIUM';
      suggestedAction = 'Remove invalid address from future sends. Check data quality.';
    } else {
      risk = 'LOW';
      suggestedAction = 'Monitor. Soft bounces are usually temporary.';
    }
    
    return {
      messageId: originalMessageId || '',
      bounceType,
      errorCode,
      errorMessage: rawDiagnostic || undefined,
      isDmarcFailure,
      isSpfFailure,
      isDkimFailure,
      isReputationIssue,
      isMailboxFull,
      isInvalidAddress,
      rawDiagnostic,
      risk,
      suggestedAction,
    };
  }
  
  /**
   * Process a bounce and update reputation metrics
   */
  async processBounce(
    messageId: string,
    subject: string,
    body: string,
    contactId?: string
  ): Promise<BounceAnalysis> {
    const analysis = this.analyzeBounce(subject, body, messageId);
    
    // Save bounce record
    await this.saveBounceRecord(analysis, contactId);
    
    // Check if we need to alert or pause
    await this.checkReputationThresholds();
    
    return analysis;
  }
  
  /**
   * Save bounce analysis to database
   */
  private async saveBounceRecord(
    analysis: BounceAnalysis,
    contactId?: string
  ): Promise<void> {
    try {
      await queryOperator(`
        INSERT INTO sa_bounce_records (
          message_id, contact_id, bounce_type, error_code, error_message,
          is_dmarc_failure, is_spf_failure, is_dkim_failure, is_reputation_issue,
          risk_level, raw_diagnostic
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
      `, [
        analysis.messageId || null,
        contactId || null,
        analysis.bounceType,
        analysis.errorCode || null,
        analysis.errorMessage || null,
        analysis.isDmarcFailure,
        analysis.isSpfFailure,
        analysis.isDkimFailure,
        analysis.isReputationIssue,
        analysis.risk,
        analysis.rawDiagnostic,
      ]);
    } catch (err) {
      // Table may not exist yet
      logger.warn('Failed to save bounce record', { error: (err as Error).message });
    }
  }
  
  /**
   * Get reputation metrics for a time period
   */
  async getMetrics(period: 'hour' | 'day' | 'week' = 'day'): Promise<ReputationMetrics> {
    const intervalMap = {
      hour: '1 hour',
      day: '24 hours',
      week: '7 days',
    };
    
    try {
      // Get sent count (from outbound messages)
      const sentResult = await queryReadOnly<{ count: string }>(`
        SELECT COUNT(*) as count FROM sa_outbound_messages
        WHERE created_at > NOW() - INTERVAL '${intervalMap[period]}'
      `);
      const totalSent = parseInt(sentResult[0]?.count || '0');
      
      // Get bounce stats
      const bounceResult = await queryReadOnly<{
        total: string;
        hard: string;
        soft: string;
        policy: string;
        dmarc: string;
        spf: string;
        dkim: string;
      }>(`
        SELECT 
          COUNT(*) as total,
          COUNT(*) FILTER (WHERE bounce_type = 'hard') as hard,
          COUNT(*) FILTER (WHERE bounce_type = 'soft') as soft,
          COUNT(*) FILTER (WHERE bounce_type = 'policy') as policy,
          COUNT(*) FILTER (WHERE is_dmarc_failure = true) as dmarc,
          COUNT(*) FILTER (WHERE is_spf_failure = true) as spf,
          COUNT(*) FILTER (WHERE is_dkim_failure = true) as dkim
        FROM sa_bounce_records
        WHERE created_at > NOW() - INTERVAL '${intervalMap[period]}'
      `);
      
      const stats = bounceResult[0] || {
        total: '0', hard: '0', soft: '0', policy: '0', dmarc: '0', spf: '0', dkim: '0'
      };
      
      const totalBounced = parseInt(stats.total);
      const hardBounces = parseInt(stats.hard);
      const softBounces = parseInt(stats.soft);
      const policyBounces = parseInt(stats.policy);
      const dmarcFailures = parseInt(stats.dmarc);
      const spfFailures = parseInt(stats.spf);
      const dkimFailures = parseInt(stats.dkim);
      
      // Calculate rates
      const bounceRate = totalSent > 0 ? totalBounced / totalSent : 0;
      
      // Calculate reputation score (0-100)
      let reputationScore = 100;
      reputationScore -= bounceRate * 200; // Each 1% bounce = -2 points
      reputationScore -= dmarcFailures * 10; // Each DMARC failure = -10
      reputationScore -= spfFailures * 8;    // Each SPF failure = -8
      reputationScore -= dkimFailures * 8;   // Each DKIM failure = -8
      reputationScore -= policyBounces * 5;  // Each policy bounce = -5
      reputationScore = Math.max(0, Math.min(100, reputationScore));
      
      // Determine risk
      let risk: ReputationRisk;
      if (reputationScore < 50 || dmarcFailures > 0 || bounceRate > this.criticalBounceRate) {
        risk = 'CRITICAL';
      } else if (reputationScore < 70 || policyBounces > 3) {
        risk = 'HIGH';
      } else if (reputationScore < 85 || bounceRate > 0.05) {
        risk = 'MEDIUM';
      } else {
        risk = 'LOW';
      }
      
      return {
        period,
        totalSent,
        totalBounced,
        hardBounces,
        softBounces,
        policyBounces,
        dmarcFailures,
        spfFailures,
        dkimFailures,
        bounceRate,
        reputationScore,
        risk,
        timestamp: new Date(),
      };
      
    } catch (err) {
      // Tables may not exist - return empty metrics
      logger.warn('Failed to get reputation metrics', { error: (err as Error).message });
      return {
        period,
        totalSent: 0,
        totalBounced: 0,
        hardBounces: 0,
        softBounces: 0,
        policyBounces: 0,
        dmarcFailures: 0,
        spfFailures: 0,
        dkimFailures: 0,
        bounceRate: 0,
        reputationScore: 100,
        risk: 'LOW',
        timestamp: new Date(),
      };
    }
  }
  
  /**
   * Check thresholds and trigger alerts/pauses
   */
  async checkReputationThresholds(): Promise<void> {
    const metrics = await this.getMetrics('day');
    
    // CRITICAL: Auto-pause conditions
    if (metrics.dmarcFailures >= this.criticalDmarcFailures) {
      await this.createAlert({
        severity: 'critical',
        type: 'DMARC_FAILURES',
        message: `${metrics.dmarcFailures} DMARC failures detected. Campaign auto-paused.`,
        details: { dmarcFailures: metrics.dmarcFailures },
      });
      
      if (this.pauseCallback) {
        await this.pauseCallback();
      }
      return;
    }
    
    if (metrics.policyBounces >= this.criticalPolicyBounces) {
      await this.createAlert({
        severity: 'critical',
        type: 'POLICY_BOUNCES',
        message: `${metrics.policyBounces} policy bounces detected. Campaign auto-paused.`,
        details: { policyBounces: metrics.policyBounces },
      });
      
      if (this.pauseCallback) {
        await this.pauseCallback();
      }
      return;
    }
    
    if (metrics.bounceRate >= this.criticalBounceRate && metrics.totalSent >= 20) {
      await this.createAlert({
        severity: 'critical',
        type: 'HIGH_BOUNCE_RATE',
        message: `Bounce rate at ${(metrics.bounceRate * 100).toFixed(1)}%. Campaign auto-paused.`,
        details: { bounceRate: metrics.bounceRate, totalSent: metrics.totalSent },
      });
      
      if (this.pauseCallback) {
        await this.pauseCallback();
      }
      return;
    }
    
    // WARNING: Alert but don't pause
    if (metrics.risk === 'HIGH') {
      await this.createAlert({
        severity: 'warning',
        type: 'HIGH_RISK',
        message: `Reputation risk is HIGH. Score: ${metrics.reputationScore.toFixed(0)}/100`,
        details: metrics as unknown as Record<string, unknown>,
      });
    }
  }
  
  /**
   * Create an alert
   */
  private async createAlert(alert: Omit<ReputationAlert, 'id' | 'acknowledged' | 'createdAt'>): Promise<void> {
    const fullAlert: ReputationAlert = {
      id: crypto.randomUUID(),
      ...alert,
      acknowledged: false,
      createdAt: new Date(),
    };
    
    // Save to database
    try {
      await queryOperator(`
        INSERT INTO sa_reputation_alerts (id, severity, type, message, details)
        VALUES ($1, $2, $3, $4, $5)
      `, [fullAlert.id, fullAlert.severity, fullAlert.type, fullAlert.message, JSON.stringify(fullAlert.details)]);
    } catch {
      // Table may not exist
    }
    
    // Notify callback
    if (this.alertCallback) {
      await this.alertCallback(fullAlert);
    }
    
    logger.warn('Reputation alert created', {
      severity: alert.severity,
      type: alert.type,
      message: alert.message,
    });
  }
  
  /**
   * Get recent alerts
   */
  async getRecentAlerts(limit = 10): Promise<ReputationAlert[]> {
    try {
      const result = await queryReadOnly<{
        id: string;
        severity: string;
        type: string;
        message: string;
        details: string;
        acknowledged: boolean;
        created_at: string;
      }>(`
        SELECT id, severity, type, message, details, acknowledged, created_at
        FROM sa_reputation_alerts
        ORDER BY created_at DESC
        LIMIT $1
      `, [limit]);
      
      return result.map(row => ({
        id: row.id,
        severity: row.severity as 'info' | 'warning' | 'critical',
        type: row.type,
        message: row.message,
        details: JSON.parse(row.details || '{}'),
        acknowledged: row.acknowledged,
        createdAt: new Date(row.created_at),
      }));
    } catch {
      return [];
    }
  }
  
  /**
   * Acknowledge an alert
   */
  async acknowledgeAlert(alertId: string): Promise<void> {
    await queryOperator(`
      UPDATE sa_reputation_alerts SET acknowledged = true WHERE id = $1
    `, [alertId]);
  }
  
  /**
   * Get full health report
   */
  async getHealthReport(): Promise<{
    hourly: ReputationMetrics;
    daily: ReputationMetrics;
    weekly: ReputationMetrics;
    recentAlerts: ReputationAlert[];
    isPaused: boolean;
    recommendation: string;
  }> {
    const [hourly, daily, weekly, recentAlerts] = await Promise.all([
      this.getMetrics('hour'),
      this.getMetrics('day'),
      this.getMetrics('week'),
      this.getRecentAlerts(5),
    ]);
    
    // Check if campaign should be paused
    const isPaused = 
      daily.dmarcFailures >= this.criticalDmarcFailures ||
      daily.policyBounces >= this.criticalPolicyBounces ||
      (daily.bounceRate >= this.criticalBounceRate && daily.totalSent >= 20);
    
    // Generate recommendation
    let recommendation: string;
    if (daily.risk === 'CRITICAL') {
      recommendation = 'STOP sending immediately. Review DNS configuration and contact support.';
    } else if (daily.risk === 'HIGH') {
      recommendation = 'Reduce sending volume. Review recent bounces for patterns.';
    } else if (daily.risk === 'MEDIUM') {
      recommendation = 'Monitor closely. Consider cleaning email list.';
    } else {
      recommendation = 'Reputation healthy. Continue normal operations.';
    }
    
    return {
      hourly,
      daily,
      weekly,
      recentAlerts,
      isPaused,
      recommendation,
    };
  }
}

// ============================================================================
// SINGLETON INSTANCE
// ============================================================================

let guardInstance: ReputationGuard | null = null;

export function getReputationGuard(): ReputationGuard {
  if (!guardInstance) {
    guardInstance = new ReputationGuard({
      onAlert: async (alert) => {
        logger.error('[REPUTATION ALERT]', {
          severity: alert.severity,
          type: alert.type,
          message: alert.message,
        });
      },
      onPause: async () => {
        logger.error('[REPUTATION GUARD] Campaign auto-paused due to critical reputation issue');
        // The actual pause logic is handled by the orchestrator
      },
    });
  }
  return guardInstance;
}
