// services/sales-automation/DomainHealthGuard.ts
// REVOLUTIONARY: Proactive domain health protection - prevents damage BEFORE it happens
// OVERCOMES: "6-7% bounce rate = inbox placement issues, domain tanks"
// OVERCOMES: "Domains getting flagged fast" - Instantly/Smartlead problem
// OVERCOMES: "Warmup hurting reputation more than helping" - Smartlead problem
// Based on Reddit: "2-3 inboxes max per domain"
// Based on Reddit: "Bounce rate >6-7% = problems"
// Zero-cost: Track metrics locally, apply protective rules automatically

import { Pool } from 'pg';

interface DomainHealth {
  domain: string;
  mailboxes: string[];
  healthScore: number; // 0-100
  status: 'healthy' | 'warning' | 'critical' | 'quarantined';
  metrics: DomainMetrics;
  alerts: HealthAlert[];
  recommendations: string[];
}

interface DomainMetrics {
  totalSent: number;
  bounceRate: number;
  spamRate: number;
  openRate: number;
  replyRate: number;
  unsubscribeRate: number;
  sendingVelocity: number; // emails per day
  warmupComplete: boolean;
  daysSinceFirstSend: number;
  consecutiveSpamReports: number;
}

interface HealthAlert {
  type: 'bounce_rate' | 'spam_report' | 'velocity' | 'warmup' | 'mailbox_limit' | 'reputation';
  severity: 'info' | 'warning' | 'critical';
  message: string;
  triggeredAt: Date;
  resolved: boolean;
}

interface SendDecision {
  allowed: boolean;
  reason?: string;
  recommendedDelay?: number; // minutes
  alternativeDomain?: string;
}

// Thresholds for health scoring
const THRESHOLDS = {
  bounceRate: {
    healthy: 3,    // <3% is healthy
    warning: 5,    // 3-5% is warning
    critical: 7,   // >7% is critical - STOP IMMEDIATELY
  },
  spamRate: {
    healthy: 0.1,  // <0.1% is healthy
    warning: 0.3,  // 0.1-0.3% is warning
    critical: 0.5, // >0.5% is critical
  },
  openRate: {
    healthy: 20,   // >20% is healthy
    warning: 10,   // 10-20% is warning
    critical: 5,   // <5% is critical (likely going to spam)
  },
  dailyLimit: {
    warmup: 20,    // During warmup (first 14 days)
    new: 50,       // After warmup but <30 days
    established: 100, // After 30 days
    max: 200,      // Hard cap
  },
  mailboxesPerDomain: 3, // Maximum mailboxes per domain
};

// Warmup schedule (days -> max emails per day per mailbox)
const WARMUP_SCHEDULE: Record<number, number> = {
  1: 5, 2: 7, 3: 10, 4: 15, 5: 20,
  6: 25, 7: 30, 8: 40, 9: 50, 10: 60,
  11: 75, 12: 90, 13: 100, 14: 100,
};

/**
 * DomainHealthGuard - Proactive domain reputation protection
 * REVOLUTIONARY: Automatically prevents actions that damage sender reputation
 */
export class DomainHealthGuard {
  private pool: Pool;
  private domainCache: Map<string, DomainHealth> = new Map();
  private sendCounts: Map<string, number> = new Map(); // domain -> today's sends
  private lastReset: Date = new Date();

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * Initialize required tables
   */
  async initialize(): Promise<void> {
    await this.pool.query(`
      CREATE TABLE IF NOT EXISTS domain_health (
        domain VARCHAR(255) PRIMARY KEY,
        mailboxes TEXT[],
        health_score INTEGER DEFAULT 100,
        status VARCHAR(50) DEFAULT 'healthy',
        first_send_date TIMESTAMPTZ,
        warmup_complete BOOLEAN DEFAULT FALSE,
        quarantined_until TIMESTAMPTZ,
        created_at TIMESTAMPTZ DEFAULT NOW(),
        updated_at TIMESTAMPTZ DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS domain_health_alerts (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        domain VARCHAR(255) REFERENCES domain_health(domain),
        alert_type VARCHAR(50) NOT NULL,
        severity VARCHAR(20) NOT NULL,
        message TEXT,
        triggered_at TIMESTAMPTZ DEFAULT NOW(),
        resolved_at TIMESTAMPTZ,
        resolved BOOLEAN DEFAULT FALSE
      );

      CREATE TABLE IF NOT EXISTS domain_daily_stats (
        domain VARCHAR(255),
        date DATE,
        emails_sent INTEGER DEFAULT 0,
        bounces INTEGER DEFAULT 0,
        spam_reports INTEGER DEFAULT 0,
        opens INTEGER DEFAULT 0,
        replies INTEGER DEFAULT 0,
        unsubscribes INTEGER DEFAULT 0,
        PRIMARY KEY (domain, date)
      );

      CREATE INDEX IF NOT EXISTS idx_domain_alerts ON domain_health_alerts(domain, resolved);
      CREATE INDEX IF NOT EXISTS idx_domain_stats_date ON domain_daily_stats(date DESC);
    `);
  }

  /**
   * MAIN: Check if we can send from this domain
   * Call this BEFORE every email send
   */
  async canSend(fromEmail: string): Promise<SendDecision> {
    const domain = this.extractDomain(fromEmail);
    const health = await this.getDomainHealth(domain);

    // 1. Check if quarantined
    if (health.status === 'quarantined') {
      const altDomain = await this.findHealthyAlternative();
      return {
        allowed: false,
        reason: `Domain ${domain} is quarantined due to poor reputation`,
        alternativeDomain: altDomain || undefined,
      };
    }

    // 2. Check if critical health
    if (health.status === 'critical') {
      return {
        allowed: false,
        reason: `Domain ${domain} health is critical (score: ${health.healthScore}). Fix issues before sending.`,
        recommendedDelay: 60 * 24, // 24 hours
      };
    }

    // 3. Check daily limit
    const todaySent = await this.getTodaySendCount(domain);
    const dailyLimit = this.getDailyLimit(health);
    
    if (todaySent >= dailyLimit) {
      return {
        allowed: false,
        reason: `Daily limit reached (${todaySent}/${dailyLimit})`,
        recommendedDelay: this.minutesUntilMidnight(),
      };
    }

    // 4. Check warmup schedule
    if (!health.metrics.warmupComplete) {
      const warmupDay = health.metrics.daysSinceFirstSend || 1;
      const warmupLimit = WARMUP_SCHEDULE[Math.min(warmupDay, 14)] || 100;
      
      if (todaySent >= warmupLimit) {
        return {
          allowed: false,
          reason: `Warmup limit reached (day ${warmupDay}: ${todaySent}/${warmupLimit})`,
          recommendedDelay: this.minutesUntilMidnight(),
        };
      }
    }

    // 5. Check sending velocity (rate limiting)
    const minutesSinceLastSend = await this.getMinutesSinceLastSend(domain);
    const minInterval = health.status === 'warning' ? 3 : 1; // Slow down if warning
    
    if (minutesSinceLastSend < minInterval) {
      return {
        allowed: false,
        reason: 'Sending too fast',
        recommendedDelay: minInterval - minutesSinceLastSend,
      };
    }

    // ✅ All checks passed
    return { allowed: true };
  }

  /**
   * Record that an email was sent
   */
  async recordSend(fromEmail: string): Promise<void> {
    const domain = this.extractDomain(fromEmail);
    const today = new Date().toISOString().split('T')[0];

    // Upsert domain
    await this.pool.query(`
      INSERT INTO domain_health (domain, mailboxes, first_send_date)
      VALUES ($1, ARRAY[$2], NOW())
      ON CONFLICT (domain) DO UPDATE SET
        mailboxes = CASE 
          WHEN NOT ($2 = ANY(domain_health.mailboxes)) 
          THEN array_append(domain_health.mailboxes, $2)
          ELSE domain_health.mailboxes 
        END,
        updated_at = NOW()
    `, [domain, fromEmail]);

    // Update daily stats
    await this.pool.query(`
      INSERT INTO domain_daily_stats (domain, date, emails_sent)
      VALUES ($1, $2, 1)
      ON CONFLICT (domain, date) DO UPDATE SET
        emails_sent = domain_daily_stats.emails_sent + 1
    `, [domain, today]);

    // Check mailbox limit
    await this.checkMailboxLimit(domain);
  }

  /**
   * Record a bounce
   */
  async recordBounce(fromEmail: string): Promise<void> {
    const domain = this.extractDomain(fromEmail);
    const today = new Date().toISOString().split('T')[0];

    await this.pool.query(`
      INSERT INTO domain_daily_stats (domain, date, bounces)
      VALUES ($1, $2, 1)
      ON CONFLICT (domain, date) DO UPDATE SET
        bounces = domain_daily_stats.bounces + 1
    `, [domain, today]);

    // Check bounce rate and potentially quarantine
    const health = await this.getDomainHealth(domain, true);
    if (health.metrics.bounceRate > THRESHOLDS.bounceRate.critical) {
      await this.quarantineDomain(domain, 'Critical bounce rate exceeded');
    }
  }

  /**
   * Record a spam report
   */
  async recordSpamReport(fromEmail: string): Promise<void> {
    const domain = this.extractDomain(fromEmail);
    const today = new Date().toISOString().split('T')[0];

    await this.pool.query(`
      INSERT INTO domain_daily_stats (domain, date, spam_reports)
      VALUES ($1, $2, 1)
      ON CONFLICT (domain, date) DO UPDATE SET
        spam_reports = domain_daily_stats.spam_reports + 1
    `, [domain, today]);

    // Check consecutive spam reports
    const health = await this.getDomainHealth(domain, true);
    if (health.metrics.consecutiveSpamReports >= 3) {
      await this.quarantineDomain(domain, 'Multiple consecutive spam reports');
    }
  }

  /**
   * Record an email open
   */
  async recordOpen(fromEmail: string): Promise<void> {
    const domain = this.extractDomain(fromEmail);
    const today = new Date().toISOString().split('T')[0];

    await this.pool.query(`
      INSERT INTO domain_daily_stats (domain, date, opens)
      VALUES ($1, $2, 1)
      ON CONFLICT (domain, date) DO UPDATE SET
        opens = domain_daily_stats.opens + 1
    `, [domain, today]);
  }

  /**
   * Record a reply
   */
  async recordReply(fromEmail: string): Promise<void> {
    const domain = this.extractDomain(fromEmail);
    const today = new Date().toISOString().split('T')[0];

    await this.pool.query(`
      INSERT INTO domain_daily_stats (domain, date, replies)
      VALUES ($1, $2, 1)
      ON CONFLICT (domain, date) DO UPDATE SET
        replies = domain_daily_stats.replies + 1
    `, [domain, today]);
  }

  /**
   * Record an unsubscribe
   */
  async recordUnsubscribe(fromEmail: string): Promise<void> {
    const domain = this.extractDomain(fromEmail);
    const today = new Date().toISOString().split('T')[0];

    await this.pool.query(`
      INSERT INTO domain_daily_stats (domain, date, unsubscribes)
      VALUES ($1, $2, 1)
      ON CONFLICT (domain, date) DO UPDATE SET
        unsubscribes = domain_daily_stats.unsubscribes + 1
    `, [domain, today]);
  }

  /**
   * Get comprehensive domain health
   */
  async getDomainHealth(domain: string, forceRefresh = false): Promise<DomainHealth> {
    if (!forceRefresh && this.domainCache.has(domain)) {
      const cached = this.domainCache.get(domain)!;
      // Cache for 5 minutes
      if (Date.now() - cached.alerts[0]?.triggeredAt.getTime() < 5 * 60 * 1000) {
        return cached;
      }
    }

    // Get basic info
    const domainResult = await this.pool.query(`
      SELECT * FROM domain_health WHERE domain = $1
    `, [domain]);

    // Get 30-day stats
    const statsResult = await this.pool.query(`
      SELECT 
        COALESCE(SUM(emails_sent), 0) as total_sent,
        COALESCE(SUM(bounces), 0) as total_bounces,
        COALESCE(SUM(spam_reports), 0) as total_spam,
        COALESCE(SUM(opens), 0) as total_opens,
        COALESCE(SUM(replies), 0) as total_replies,
        COALESCE(SUM(unsubscribes), 0) as total_unsubscribes
      FROM domain_daily_stats
      WHERE domain = $1 AND date > NOW() - INTERVAL '30 days'
    `, [domain]);

    // Get recent consecutive spam days
    const spamResult = await this.pool.query(`
      SELECT COUNT(*) as spam_days FROM (
        SELECT date FROM domain_daily_stats
        WHERE domain = $1 AND spam_reports > 0
        ORDER BY date DESC
        LIMIT 7
      ) recent
      WHERE date > NOW() - INTERVAL '7 days'
    `, [domain]);

    // Get alerts
    const alertsResult = await this.pool.query(`
      SELECT * FROM domain_health_alerts
      WHERE domain = $1 AND resolved = FALSE
      ORDER BY triggered_at DESC
    `, [domain]);

    const stats = statsResult.rows[0];
    const domainData = domainResult.rows[0] || {
      domain,
      mailboxes: [],
      first_send_date: null,
      warmup_complete: false,
      quarantined_until: null,
    };

    const totalSent = parseInt(stats.total_sent) || 0;
    const metrics: DomainMetrics = {
      totalSent,
      bounceRate: totalSent > 0 ? (parseInt(stats.total_bounces) / totalSent) * 100 : 0,
      spamRate: totalSent > 0 ? (parseInt(stats.total_spam) / totalSent) * 100 : 0,
      openRate: totalSent > 0 ? (parseInt(stats.total_opens) / totalSent) * 100 : 0,
      replyRate: totalSent > 0 ? (parseInt(stats.total_replies) / totalSent) * 100 : 0,
      unsubscribeRate: totalSent > 0 ? (parseInt(stats.total_unsubscribes) / totalSent) * 100 : 0,
      sendingVelocity: totalSent / 30,
      warmupComplete: domainData.warmup_complete,
      daysSinceFirstSend: domainData.first_send_date 
        ? Math.floor((Date.now() - new Date(domainData.first_send_date).getTime()) / (24 * 60 * 60 * 1000))
        : 0,
      consecutiveSpamReports: parseInt(spamResult.rows[0]?.spam_days) || 0,
    };

    // Calculate health score
    const { healthScore, status } = this.calculateHealthScore(metrics, domainData.quarantined_until);

    // Get recommendations
    const recommendations = this.generateRecommendations(metrics, status);

    const health: DomainHealth = {
      domain,
      mailboxes: domainData.mailboxes || [],
      healthScore,
      status,
      metrics,
      alerts: alertsResult.rows.map(row => ({
        type: row.alert_type,
        severity: row.severity,
        message: row.message,
        triggeredAt: row.triggered_at,
        resolved: row.resolved,
      })),
      recommendations,
    };

    this.domainCache.set(domain, health);
    return health;
  }

  /**
   * Get health dashboard for all domains
   */
  async getAllDomainHealth(): Promise<DomainHealth[]> {
    const result = await this.pool.query(`
      SELECT domain FROM domain_health ORDER BY updated_at DESC
    `);

    const healthReports: DomainHealth[] = [];
    for (const row of result.rows) {
      healthReports.push(await this.getDomainHealth(row.domain));
    }

    return healthReports;
  }

  /**
   * Quarantine a domain
   */
  async quarantineDomain(domain: string, reason: string): Promise<void> {
    const quarantineUntil = new Date(Date.now() + 24 * 60 * 60 * 1000); // 24 hours

    await this.pool.query(`
      UPDATE domain_health 
      SET 
        status = 'quarantined',
        quarantined_until = $1,
        updated_at = NOW()
      WHERE domain = $2
    `, [quarantineUntil, domain]);

    await this.createAlert(domain, 'reputation', 'critical', 
      `Domain quarantined: ${reason}. Will be reviewed at ${quarantineUntil.toISOString()}`);

    console.error(`[DomainHealthGuard] 🚫 QUARANTINED ${domain}: ${reason}`);
  }

  /**
   * Unquarantine a domain
   */
  async unquarantineDomain(domain: string): Promise<void> {
    await this.pool.query(`
      UPDATE domain_health 
      SET 
        status = 'warning',
        quarantined_until = NULL,
        updated_at = NOW()
      WHERE domain = $1
    `, [domain]);

    console.log(`[DomainHealthGuard] ✅ Unquarantined ${domain}`);
  }

  /**
   * Check and potentially release quarantined domains
   */
  async checkQuarantines(): Promise<string[]> {
    const result = await this.pool.query(`
      SELECT domain FROM domain_health
      WHERE status = 'quarantined' AND quarantined_until < NOW()
    `);

    const released: string[] = [];
    for (const row of result.rows) {
      await this.unquarantineDomain(row.domain);
      released.push(row.domain);
    }

    return released;
  }

  /**
   * Complete warmup for a domain
   */
  async completeWarmup(domain: string): Promise<void> {
    const health = await this.getDomainHealth(domain);
    
    if (health.metrics.daysSinceFirstSend >= 14 && health.healthScore >= 70) {
      await this.pool.query(`
        UPDATE domain_health 
        SET warmup_complete = TRUE, updated_at = NOW()
        WHERE domain = $1
      `, [domain]);

      console.log(`[DomainHealthGuard] ✅ Warmup complete for ${domain}`);
    }
  }

  // ===== Private Methods =====

  private extractDomain(email: string): string {
    return email.split('@')[1]?.toLowerCase() || email;
  }

  private calculateHealthScore(metrics: DomainMetrics, quarantinedUntil: Date | null): {
    healthScore: number;
    status: 'healthy' | 'warning' | 'critical' | 'quarantined';
  } {
    if (quarantinedUntil && new Date(quarantinedUntil) > new Date()) {
      return { healthScore: 0, status: 'quarantined' };
    }

    let score = 100;
    let status: 'healthy' | 'warning' | 'critical' = 'healthy';

    // Bounce rate (biggest factor)
    if (metrics.bounceRate > THRESHOLDS.bounceRate.critical) {
      score -= 40;
      status = 'critical';
    } else if (metrics.bounceRate > THRESHOLDS.bounceRate.warning) {
      score -= 20;
      status = 'warning';
    } else if (metrics.bounceRate > THRESHOLDS.bounceRate.healthy) {
      score -= 10;
    }

    // Spam rate
    if (metrics.spamRate > THRESHOLDS.spamRate.critical) {
      score -= 30;
      status = 'critical';
    } else if (metrics.spamRate > THRESHOLDS.spamRate.warning) {
      score -= 15;
      if (status === 'healthy') status = 'warning';
    }

    // Open rate (indicator of inbox placement)
    if (metrics.totalSent > 100) { // Only count if enough data
      if (metrics.openRate < THRESHOLDS.openRate.critical) {
        score -= 20;
        if (status === 'healthy') status = 'warning';
      } else if (metrics.openRate < THRESHOLDS.openRate.warning) {
        score -= 10;
      }
    }

    // Consecutive spam reports
    if (metrics.consecutiveSpamReports >= 3) {
      score -= 25;
      status = 'critical';
    } else if (metrics.consecutiveSpamReports >= 2) {
      score -= 10;
    }

    return { healthScore: Math.max(0, score), status };
  }

  private generateRecommendations(metrics: DomainMetrics, status: string): string[] {
    const recommendations: string[] = [];

    if (metrics.bounceRate > THRESHOLDS.bounceRate.warning) {
      recommendations.push('Verify email addresses before sending - use email verification service');
      recommendations.push('Remove contacts that have bounced previously');
    }

    if (metrics.spamRate > THRESHOLDS.spamRate.warning) {
      recommendations.push('Review email content for spam triggers');
      recommendations.push('Ensure clear unsubscribe link in all emails');
      recommendations.push('Use more personalized, less salesy language');
    }

    if (metrics.openRate < THRESHOLDS.openRate.warning && metrics.totalSent > 100) {
      recommendations.push('Emails may be going to spam - check content and sending reputation');
      recommendations.push('Try shorter, more compelling subject lines');
      recommendations.push('Reduce sending volume temporarily');
    }

    if (!metrics.warmupComplete && metrics.daysSinceFirstSend < 14) {
      recommendations.push(`Continue warmup (day ${metrics.daysSinceFirstSend}/14) - follow daily limits`);
    }

    if (status === 'critical') {
      recommendations.push('URGENT: Pause all sending until issues are resolved');
      recommendations.push('Review recent bounces and spam reports');
    }

    return recommendations;
  }

  private getDailyLimit(health: DomainHealth): number {
    if (!health.metrics.warmupComplete) {
      return THRESHOLDS.dailyLimit.warmup;
    }
    
    if (health.metrics.daysSinceFirstSend < 30) {
      return THRESHOLDS.dailyLimit.new;
    }

    if (health.healthScore >= 80) {
      return THRESHOLDS.dailyLimit.established;
    }

    return THRESHOLDS.dailyLimit.new; // Reduced limit for lower health
  }

  private async getTodaySendCount(domain: string): Promise<number> {
    const today = new Date().toISOString().split('T')[0];
    const result = await this.pool.query(`
      SELECT COALESCE(emails_sent, 0) as sent 
      FROM domain_daily_stats 
      WHERE domain = $1 AND date = $2
    `, [domain, today]);

    return parseInt(result.rows[0]?.sent) || 0;
  }

  private async getMinutesSinceLastSend(domain: string): Promise<number> {
    const result = await this.pool.query(`
      SELECT MAX(sent_at) as last_sent
      FROM outreach_emails e
      JOIN sales_contacts c ON e.contact_id = c.id
      WHERE c.email LIKE $1
    `, [`%@${domain}`]);

    if (!result.rows[0]?.last_sent) return 999; // No previous send

    const lastSent = new Date(result.rows[0].last_sent);
    return (Date.now() - lastSent.getTime()) / (60 * 1000);
  }

  private async findHealthyAlternative(): Promise<string | null> {
    const result = await this.pool.query(`
      SELECT domain FROM domain_health
      WHERE status IN ('healthy', 'warning')
      AND (quarantined_until IS NULL OR quarantined_until < NOW())
      ORDER BY health_score DESC
      LIMIT 1
    `);

    return result.rows[0]?.domain || null;
  }

  private async checkMailboxLimit(domain: string): Promise<void> {
    const result = await this.pool.query(`
      SELECT array_length(mailboxes, 1) as count 
      FROM domain_health 
      WHERE domain = $1
    `, [domain]);

    const count = result.rows[0]?.count || 0;

    if (count > THRESHOLDS.mailboxesPerDomain) {
      await this.createAlert(
        domain, 
        'mailbox_limit', 
        'warning',
        `Domain has ${count} mailboxes. Recommended maximum is ${THRESHOLDS.mailboxesPerDomain}.`
      );
    }
  }

  private async createAlert(
    domain: string,
    type: string,
    severity: 'info' | 'warning' | 'critical',
    message: string
  ): Promise<void> {
    // Check if similar unresolved alert exists
    const existing = await this.pool.query(`
      SELECT id FROM domain_health_alerts
      WHERE domain = $1 AND alert_type = $2 AND resolved = FALSE
    `, [domain, type]);

    if (existing.rows.length === 0) {
      await this.pool.query(`
        INSERT INTO domain_health_alerts (domain, alert_type, severity, message)
        VALUES ($1, $2, $3, $4)
      `, [domain, type, severity, message]);
    }
  }

  private minutesUntilMidnight(): number {
    const now = new Date();
    const midnight = new Date(now);
    midnight.setHours(24, 0, 0, 0);
    return Math.ceil((midnight.getTime() - now.getTime()) / (60 * 1000));
  }
}
