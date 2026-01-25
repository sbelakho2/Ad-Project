// services/sales-automation/TransparentLogging.ts
// OVERCOMES: "no visibility into what's happening", "emails disappear without explanation"
// OVERCOMES: "system randomly stops and no one knows why", "can't troubleshoot issues"
// OVERCOMES: "customer support can't explain what went wrong" (we provide self-service debugging)
// Based on Smartlead/Instantly complaints: "I don't know why my emails aren't sending"
// Zero-cost: Comprehensive logging to PostgreSQL, no external services

import { Pool } from 'pg';
import { EventEmitter } from 'events';

type LogLevel = 'debug' | 'info' | 'warn' | 'error' | 'critical';
type LogCategory = 
  | 'email_send'
  | 'email_delivery'
  | 'warmup'
  | 'bounce'
  | 'spam'
  | 'enrichment'
  | 'discovery'
  | 'scheduling'
  | 'sequence'
  | 'system'
  | 'automation';

interface LogEntry {
  id?: string;
  timestamp: Date;
  level: LogLevel;
  category: LogCategory;
  message: string;
  details?: Record<string, unknown>;
  contactId?: string;
  sequenceId?: string;
  emailId?: string;
  actionTaken?: string;
  resolution?: string;
}

interface EmailJourney {
  contactId: string;
  contactEmail: string;
  contactName: string;
  events: JourneyEvent[];
  currentStatus: string;
  recommendations: string[];
}

interface JourneyEvent {
  timestamp: Date;
  event: string;
  details: string;
  outcome: 'success' | 'warning' | 'error' | 'info';
}

interface SystemHealth {
  component: string;
  status: 'healthy' | 'degraded' | 'down';
  lastCheck: Date;
  message: string;
  metrics?: Record<string, number>;
}

// Human-readable explanations for common issues
const ISSUE_EXPLANATIONS: Record<string, { cause: string; solution: string }> = {
  'bounce_hard': {
    cause: 'The email address does not exist or the domain is invalid.',
    solution: 'This contact has been automatically paused. Verify the email address is correct.',
  },
  'bounce_soft': {
    cause: 'The recipient\'s mailbox is full or temporarily unavailable.',
    solution: 'We\'ll retry sending in 24 hours. If it continues, the contact will be paused.',
  },
  'spam_block': {
    cause: 'The email was blocked by the recipient\'s spam filter.',
    solution: 'Review the email content for spam triggers. Our spam prevention scored this email.',
  },
  'warmup_in_progress': {
    cause: 'Your email account is still building reputation.',
    solution: 'Normal sending is limited during warmup. Current phase allows X emails/day.',
  },
  'daily_limit_reached': {
    cause: 'You\'ve reached the daily sending limit to protect deliverability.',
    solution: 'Emails will resume tomorrow. This limit protects your sender reputation.',
  },
  'contact_paused': {
    cause: 'This contact has been paused due to bounces or unsubscribe request.',
    solution: 'Review the contact\'s engagement history before re-enabling.',
  },
  'sequence_paused': {
    cause: 'The sequence is paused, possibly due to high bounce rate.',
    solution: 'Check sequence health metrics before resuming.',
  },
  'enrichment_failed': {
    cause: 'We couldn\'t find additional information about this contact.',
    solution: 'The contact will still receive emails but with less personalization.',
  },
  'send_failed': {
    cause: 'The email could not be sent due to a technical issue.',
    solution: 'Our system will automatically retry. If persists, check mailbox connection.',
  },
};

/**
 * TransparentLogging - Full visibility into system operations
 * OVERCOMES: "I don't know what's happening" - major Smartlead/Instantly complaint
 */
export class TransparentLogging extends EventEmitter {
  private pool: Pool;
  private logBuffer: LogEntry[] = [];
  private flushInterval: NodeJS.Timeout | null = null;

  constructor(databaseUrl: string) {
    super();
    this.pool = new Pool({ connectionString: databaseUrl });
    this.startBufferFlush();
  }

  /**
   * Initialize logging tables
   */
  async initialize(): Promise<void> {
    await this.pool.query(`
      CREATE TABLE IF NOT EXISTS automation_logs (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        timestamp TIMESTAMPTZ DEFAULT NOW(),
        level VARCHAR(20) NOT NULL,
        category VARCHAR(50) NOT NULL,
        message TEXT NOT NULL,
        details JSONB,
        contact_id UUID REFERENCES sales_contacts(id),
        sequence_id UUID REFERENCES outreach_sequences(id),
        email_id UUID,
        action_taken TEXT,
        resolution TEXT,
        created_at TIMESTAMPTZ DEFAULT NOW()
      );

      CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON automation_logs(timestamp DESC);
      CREATE INDEX IF NOT EXISTS idx_logs_level ON automation_logs(level);
      CREATE INDEX IF NOT EXISTS idx_logs_category ON automation_logs(category);
      CREATE INDEX IF NOT EXISTS idx_logs_contact ON automation_logs(contact_id);

      CREATE TABLE IF NOT EXISTS system_health_history (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        component VARCHAR(100) NOT NULL,
        status VARCHAR(20) NOT NULL,
        message TEXT,
        metrics JSONB,
        checked_at TIMESTAMPTZ DEFAULT NOW()
      );

      CREATE INDEX IF NOT EXISTS idx_health_component ON system_health_history(component, checked_at DESC);
    `);
  }

  /**
   * Log an event
   */
  log(entry: Omit<LogEntry, 'id' | 'timestamp'>): void {
    const fullEntry: LogEntry = {
      ...entry,
      timestamp: new Date(),
    };

    this.logBuffer.push(fullEntry);

    // Emit for real-time monitoring
    this.emit('log', fullEntry);

    // Immediately flush critical/error logs
    if (entry.level === 'critical' || entry.level === 'error') {
      this.flushLogs();
    }
  }

  /**
   * Convenient logging methods
   */
  debug(category: LogCategory, message: string, details?: Record<string, unknown>): void {
    this.log({ level: 'debug', category, message, details });
  }

  info(category: LogCategory, message: string, details?: Record<string, unknown>): void {
    this.log({ level: 'info', category, message, details });
  }

  warn(category: LogCategory, message: string, details?: Record<string, unknown>): void {
    this.log({ level: 'warn', category, message, details });
  }

  error(category: LogCategory, message: string, details?: Record<string, unknown>): void {
    this.log({ level: 'error', category, message, details });
  }

  critical(category: LogCategory, message: string, details?: Record<string, unknown>): void {
    this.log({ level: 'critical', category, message, details });
  }

  /**
   * Log an email event with full context
   * OVERCOMES: "I don't know why my email didn't send"
   */
  logEmailEvent(
    event: 'queued' | 'sending' | 'sent' | 'opened' | 'clicked' | 'replied' | 'bounced' | 'failed' | 'spam_blocked',
    emailId: string,
    contactId: string,
    details?: Record<string, unknown>
  ): void {
    const eventMessages: Record<string, { level: LogLevel; message: string }> = {
      queued: { level: 'debug', message: 'Email queued for sending' },
      sending: { level: 'debug', message: 'Email is being sent' },
      sent: { level: 'info', message: 'Email sent successfully' },
      opened: { level: 'info', message: 'Email was opened by recipient' },
      clicked: { level: 'info', message: 'Recipient clicked a link in the email' },
      replied: { level: 'info', message: 'Recipient replied to the email' },
      bounced: { level: 'warn', message: 'Email bounced' },
      failed: { level: 'error', message: 'Email failed to send' },
      spam_blocked: { level: 'warn', message: 'Email blocked by spam filter' },
    };

    const { level, message } = eventMessages[event];

    // Add explanation for issues
    let resolution: string | undefined;
    let actionTaken: string | undefined;

    if (event === 'bounced' && details?.bounceType) {
      const explanation = ISSUE_EXPLANATIONS[`bounce_${details.bounceType}`];
      if (explanation) {
        resolution = explanation.solution;
        actionTaken = 'Contact paused automatically';
      }
    }

    if (event === 'spam_blocked') {
      const explanation = ISSUE_EXPLANATIONS.spam_block;
      resolution = explanation.solution;
    }

    this.log({
      level,
      category: 'email_delivery',
      message,
      details: { event, ...details },
      emailId,
      contactId,
      actionTaken,
      resolution,
    });
  }

  /**
   * Get complete journey for a contact
   * OVERCOMES: "I can't see what happened to my contact"
   */
  async getContactJourney(contactId: string): Promise<EmailJourney> {
    // Get contact info
    const contactResult = await this.pool.query(`
      SELECT email, first_name, last_name, status
      FROM sales_contacts WHERE id = $1
    `, [contactId]);

    const contact = contactResult.rows[0] || { email: 'Unknown', first_name: '', last_name: '' };

    // Get all logs for this contact
    const logsResult = await this.pool.query(`
      SELECT timestamp, level, category, message, details, action_taken, resolution
      FROM automation_logs
      WHERE contact_id = $1
      ORDER BY timestamp ASC
    `, [contactId]);

    const events: JourneyEvent[] = logsResult.rows.map(log => ({
      timestamp: log.timestamp,
      event: log.message,
      details: this.formatLogDetails(log.details, log.action_taken, log.resolution),
      outcome: this.levelToOutcome(log.level),
    }));

    // Generate recommendations based on journey
    const recommendations = this.generateRecommendations(logsResult.rows, contact);

    return {
      contactId,
      contactEmail: contact.email,
      contactName: `${contact.first_name} ${contact.last_name}`.trim(),
      events,
      currentStatus: contact.status || 'unknown',
      recommendations,
    };
  }

  /**
   * Get human-readable explanation for an issue
   */
  explainIssue(issueType: string): { cause: string; solution: string } | null {
    return ISSUE_EXPLANATIONS[issueType] || null;
  }

  /**
   * Get recent system activity summary
   * OVERCOMES: "I don't know if the system is working"
   */
  async getActivitySummary(hours: number = 24): Promise<{
    totalEvents: number;
    byCategory: Record<string, number>;
    byLevel: Record<string, number>;
    recentErrors: LogEntry[];
    systemHealth: SystemHealth[];
  }> {
    const since = new Date(Date.now() - hours * 60 * 60 * 1000);

    // Count by category
    const categoryResult = await this.pool.query(`
      SELECT category, COUNT(*) as count
      FROM automation_logs
      WHERE timestamp > $1
      GROUP BY category
    `, [since]);

    // Count by level
    const levelResult = await this.pool.query(`
      SELECT level, COUNT(*) as count
      FROM automation_logs
      WHERE timestamp > $1
      GROUP BY level
    `, [since]);

    // Recent errors
    const errorsResult = await this.pool.query(`
      SELECT * FROM automation_logs
      WHERE timestamp > $1 AND level IN ('error', 'critical')
      ORDER BY timestamp DESC
      LIMIT 20
    `, [since]);

    // System health
    const healthResult = await this.pool.query(`
      SELECT DISTINCT ON (component) component, status, message, metrics, checked_at
      FROM system_health_history
      ORDER BY component, checked_at DESC
    `);

    const byCategory: Record<string, number> = {};
    categoryResult.rows.forEach(row => {
      byCategory[row.category] = parseInt(row.count);
    });

    const byLevel: Record<string, number> = {};
    levelResult.rows.forEach(row => {
      byLevel[row.level] = parseInt(row.count);
    });

    const totalEvents = Object.values(byCategory).reduce((a, b) => a + b, 0);

    return {
      totalEvents,
      byCategory,
      byLevel,
      recentErrors: errorsResult.rows,
      systemHealth: healthResult.rows.map(row => ({
        component: row.component,
        status: row.status,
        lastCheck: row.checked_at,
        message: row.message,
        metrics: row.metrics,
      })),
    };
  }

  /**
   * Search logs with filters
   */
  async searchLogs(filters: {
    level?: LogLevel;
    category?: LogCategory;
    contactId?: string;
    search?: string;
    since?: Date;
    until?: Date;
    limit?: number;
  }): Promise<LogEntry[]> {
    const conditions: string[] = ['1=1'];
    const params: unknown[] = [];
    let paramIndex = 1;

    if (filters.level) {
      conditions.push(`level = $${paramIndex++}`);
      params.push(filters.level);
    }

    if (filters.category) {
      conditions.push(`category = $${paramIndex++}`);
      params.push(filters.category);
    }

    if (filters.contactId) {
      conditions.push(`contact_id = $${paramIndex++}`);
      params.push(filters.contactId);
    }

    if (filters.search) {
      conditions.push(`(message ILIKE $${paramIndex} OR details::text ILIKE $${paramIndex})`);
      params.push(`%${filters.search}%`);
      paramIndex++;
    }

    if (filters.since) {
      conditions.push(`timestamp >= $${paramIndex++}`);
      params.push(filters.since);
    }

    if (filters.until) {
      conditions.push(`timestamp <= $${paramIndex++}`);
      params.push(filters.until);
    }

    const limit = filters.limit || 100;

    const result = await this.pool.query(`
      SELECT * FROM automation_logs
      WHERE ${conditions.join(' AND ')}
      ORDER BY timestamp DESC
      LIMIT ${limit}
    `, params);

    return result.rows;
  }

  /**
   * Record system health check
   */
  async recordHealthCheck(health: SystemHealth): Promise<void> {
    await this.pool.query(`
      INSERT INTO system_health_history (component, status, message, metrics, checked_at)
      VALUES ($1, $2, $3, $4, $5)
    `, [health.component, health.status, health.message, health.metrics || {}, health.lastCheck]);

    // Also log if degraded or down
    if (health.status !== 'healthy') {
      this.warn('system', `${health.component} is ${health.status}: ${health.message}`, health.metrics);
    }
  }

  /**
   * Generate daily summary report
   * OVERCOMES: "I don't know if my campaigns are working"
   */
  async generateDailySummary(date?: Date): Promise<string> {
    const targetDate = date || new Date(Date.now() - 24 * 60 * 60 * 1000);
    const startOfDay = new Date(targetDate);
    startOfDay.setHours(0, 0, 0, 0);
    const endOfDay = new Date(targetDate);
    endOfDay.setHours(23, 59, 59, 999);

    // Get email stats
    const emailStats = await this.pool.query(`
      SELECT 
        COUNT(*) FILTER (WHERE category = 'email_send') as sent,
        COUNT(*) FILTER (WHERE message ILIKE '%opened%') as opened,
        COUNT(*) FILTER (WHERE message ILIKE '%clicked%') as clicked,
        COUNT(*) FILTER (WHERE message ILIKE '%replied%') as replied,
        COUNT(*) FILTER (WHERE message ILIKE '%bounced%') as bounced,
        COUNT(*) FILTER (WHERE level = 'error') as errors
      FROM automation_logs
      WHERE timestamp BETWEEN $1 AND $2
    `, [startOfDay, endOfDay]);

    const stats = emailStats.rows[0];

    // Get any critical issues
    const issues = await this.pool.query(`
      SELECT message, details, resolution
      FROM automation_logs
      WHERE timestamp BETWEEN $1 AND $2 AND level IN ('error', 'critical')
      ORDER BY timestamp DESC
      LIMIT 5
    `, [startOfDay, endOfDay]);

    let summary = `📊 Daily Summary for ${targetDate.toLocaleDateString()}\n\n`;
    summary += `📧 Emails Sent: ${stats.sent}\n`;
    summary += `👁️ Opened: ${stats.opened}\n`;
    summary += `🖱️ Clicked: ${stats.clicked}\n`;
    summary += `💬 Replied: ${stats.replied}\n`;
    summary += `⚠️ Bounced: ${stats.bounced}\n`;
    summary += `❌ Errors: ${stats.errors}\n`;

    if (issues.rows.length > 0) {
      summary += `\n🚨 Issues to Review:\n`;
      issues.rows.forEach((issue, i) => {
        summary += `${i + 1}. ${issue.message}`;
        if (issue.resolution) summary += ` → ${issue.resolution}`;
        summary += '\n';
      });
    }

    return summary;
  }

  /**
   * Flush log buffer to database
   */
  private async flushLogs(): Promise<void> {
    if (this.logBuffer.length === 0) return;

    const logs = [...this.logBuffer];
    this.logBuffer = [];

    try {
      const values: unknown[][] = logs.map(log => [
        log.timestamp,
        log.level,
        log.category,
        log.message,
        JSON.stringify(log.details || {}),
        log.contactId || null,
        log.sequenceId || null,
        log.emailId || null,
        log.actionTaken || null,
        log.resolution || null,
      ]);

      // Batch insert
      const placeholders = values.map((_, i) => {
        const offset = i * 10;
        return `($${offset + 1}, $${offset + 2}, $${offset + 3}, $${offset + 4}, $${offset + 5}, $${offset + 6}, $${offset + 7}, $${offset + 8}, $${offset + 9}, $${offset + 10})`;
      }).join(', ');

      await this.pool.query(`
        INSERT INTO automation_logs (timestamp, level, category, message, details, contact_id, sequence_id, email_id, action_taken, resolution)
        VALUES ${placeholders}
      `, values.flat());
    } catch (error) {
      // Put logs back if insert failed
      this.logBuffer = [...logs, ...this.logBuffer];
      console.error('Failed to flush logs:', error);
    }
  }

  /**
   * Start periodic buffer flush
   */
  private startBufferFlush(): void {
    this.flushInterval = setInterval(() => this.flushLogs(), 5000);
  }

  /**
   * Format log details for human reading
   */
  private formatLogDetails(details: Record<string, unknown>, actionTaken?: string, resolution?: string): string {
    let formatted = '';

    if (details && Object.keys(details).length > 0) {
      formatted = Object.entries(details)
        .map(([k, v]) => `${k}: ${v}`)
        .join(', ');
    }

    if (actionTaken) formatted += ` | Action: ${actionTaken}`;
    if (resolution) formatted += ` | Resolution: ${resolution}`;

    return formatted || 'No additional details';
  }

  /**
   * Convert log level to journey outcome
   */
  private levelToOutcome(level: string): 'success' | 'warning' | 'error' | 'info' {
    switch (level) {
      case 'error':
      case 'critical':
        return 'error';
      case 'warn':
        return 'warning';
      case 'info':
        return 'success';
      default:
        return 'info';
    }
  }

  /**
   * Generate recommendations based on contact journey
   */
  private generateRecommendations(logs: LogEntry[], contact: Record<string, unknown>): string[] {
    const recommendations: string[] = [];

    const errorCount = logs.filter(l => l.level === 'error').length;
    const bounceCount = logs.filter(l => l.message?.includes('bounce')).length;

    if (bounceCount >= 2) {
      recommendations.push('Multiple bounces detected. Consider verifying email address or removing from sequence.');
    }

    if (errorCount >= 3) {
      recommendations.push('Multiple errors with this contact. Review technical issues before continuing outreach.');
    }

    const opens = logs.filter(l => l.message?.includes('opened')).length;
    const clicks = logs.filter(l => l.message?.includes('clicked')).length;

    if (opens > 3 && clicks === 0) {
      recommendations.push('Contact opens emails but doesn\'t click. Consider different CTA or content approach.');
    }

    if (contact.status === 'active' && logs.length === 0) {
      recommendations.push('No activity yet. Contact is queued for outreach.');
    }

    if (recommendations.length === 0) {
      recommendations.push('No issues detected. Journey proceeding normally.');
    }

    return recommendations;
  }

  /**
   * Cleanup
   */
  async shutdown(): Promise<void> {
    if (this.flushInterval) {
      clearInterval(this.flushInterval);
    }
    await this.flushLogs();
    await this.pool.end();
  }
}
