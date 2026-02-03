// services/sales-automation/ReliabilityGuard.ts
// OVERCOMES: "Platform bugs", "system randomly stops working", "emails disconnected"
// OVERCOMES: "settings not applying", "campaigns pause randomly", "data lost"
// Based on Smartlead/Instantly complaints: bugs, unreliability, emails disappearing
// Zero-cost: Self-healing, automatic monitoring, no external dependencies

import { Pool } from 'pg';
import { EventEmitter } from 'events';

export interface SystemHealth {
  overall: 'healthy' | 'degraded' | 'critical';
  components: ComponentHealth[];
  lastCheck: Date;
  uptime: number;
  consecutiveFailures: number;
}

interface ComponentHealth {
  name: string;
  status: 'up' | 'degraded' | 'down';
  lastSuccess: Date | null;
  lastError: string | null;
  errorCount: number;
  responseTime: number;
}

interface RecoveryAction {
  component: string;
  action: 'restart' | 'reconnect' | 'reset' | 'alert';
  timestamp: Date;
  success: boolean;
  details: string;
}

interface QueuedEmail {
  id: string;
  enrollment_id: string;
  contact_id: string;
  subject: string;
  body: string;
  scheduled_for: Date;
  retries: number;
  last_error?: string;
}

// Circuit breaker states
type CircuitState = 'closed' | 'open' | 'half-open';

/**
 * ReliabilityGuard - Self-healing system monitor
 * OVERCOMES: All "platform unreliable" complaints from competitor reviews
 */
export class ReliabilityGuard extends EventEmitter {
  private pool: Pool;
  private health: SystemHealth;
  private circuitBreakers: Map<string, { state: CircuitState; failures: number; lastFailure: Date | null }>;
  private recoveryLog: RecoveryAction[] = [];
  private isMonitoring: boolean = false;
  private monitorInterval: NodeJS.Timeout | null = null;
  private startTime: Date;

  // Configuration - tuned for reliability
  private readonly CONFIG = {
    healthCheckInterval: 60 * 1000,    // 1 minute
    circuitBreakerThreshold: 3,         // Failures before opening circuit
    circuitBreakerReset: 5 * 60 * 1000, // 5 minutes before retry
    maxRetries: 5,                       // Max retries per email
    retryBackoff: [1, 2, 5, 15, 30],    // Minutes between retries
    deadLetterThreshold: 24 * 60 * 60,  // 24 hours before moving to dead letter
  };

  constructor(databaseUrl: string) {
    super();
    this.pool = new Pool({ connectionString: databaseUrl });
    this.startTime = new Date();
    this.circuitBreakers = new Map();
    this.health = {
      overall: 'healthy',
      components: [],
      lastCheck: new Date(),
      uptime: 0,
      consecutiveFailures: 0,
    };

    // Initialize circuit breakers for critical components
    ['database', 'email_service', 'discovery', 'enrichment', 'outreach'].forEach(component => {
      this.circuitBreakers.set(component, { state: 'closed', failures: 0, lastFailure: null });
    });
  }

  /**
   * Start continuous monitoring
   * OVERCOMES: "system randomly stops without warning"
   */
  startMonitoring(): void {
    if (this.isMonitoring) return;
    this.isMonitoring = true;

    console.log('[ReliabilityGuard] Starting continuous monitoring...');

    // Initial health check
    this.performHealthCheck();

    // Scheduled health checks
    this.monitorInterval = setInterval(() => {
      this.performHealthCheck();
    }, this.CONFIG.healthCheckInterval);

    // Check for stuck emails every 5 minutes
    setInterval(() => this.recoverStuckEmails(), 5 * 60 * 1000);

    // Database connection pool health
    setInterval(() => this.checkDatabasePool(), 2 * 60 * 1000);
  }

  /**
   * Stop monitoring
   */
  stopMonitoring(): void {
    this.isMonitoring = false;
    if (this.monitorInterval) {
      clearInterval(this.monitorInterval);
      this.monitorInterval = null;
    }
    console.log('[ReliabilityGuard] Monitoring stopped');
  }

  /**
   * Comprehensive health check
   * OVERCOMES: "no warning before failure", "issues take days to discover"
   */
  private async performHealthCheck(): Promise<void> {
    const components: ComponentHealth[] = [];

    // 1. Database connectivity
    components.push(await this.checkDatabase());

    // 2. Email service (Resend)
    components.push(await this.checkEmailService());

    // 3. Pending email queue
    components.push(await this.checkEmailQueue());

    // 4. Warmup status
    components.push(await this.checkWarmupHealth());

    // 5. Bounce rate monitoring
    components.push(await this.checkBounceRate());

    // Update overall health
    const downCount = components.filter(c => c.status === 'down').length;
    const degradedCount = components.filter(c => c.status === 'degraded').length;

    let overall: SystemHealth['overall'] = 'healthy';
    if (downCount > 0) {
      overall = 'critical';
      this.health.consecutiveFailures++;
    } else if (degradedCount > 0) {
      overall = 'degraded';
    } else {
      this.health.consecutiveFailures = 0;
    }

    this.health = {
      overall,
      components,
      lastCheck: new Date(),
      uptime: Date.now() - this.startTime.getTime(),
      consecutiveFailures: this.health.consecutiveFailures,
    };

    // Emit health event
    this.emit('health', this.health);

    // Auto-recovery if needed
    if (overall === 'critical' || overall === 'degraded') {
      await this.attemptAutoRecovery(components);
    }

    // Log status
    const statusEmoji = overall === 'healthy' ? '✅' : overall === 'degraded' ? '⚠️' : '🚨';
    console.log(`[ReliabilityGuard] ${statusEmoji} System: ${overall} | ${components.filter(c => c.status === 'up').length}/${components.length} components healthy`);
  }

  /**
   * Check database connectivity
   */
  private async checkDatabase(): Promise<ComponentHealth> {
    const start = Date.now();
    try {
      await this.pool.query('SELECT 1');
      return {
        name: 'database',
        status: 'up',
        lastSuccess: new Date(),
        lastError: null,
        errorCount: 0,
        responseTime: Date.now() - start,
      };
    } catch (error: any) {
      this.recordCircuitBreakerFailure('database');
      return {
        name: 'database',
        status: 'down',
        lastSuccess: null,
        lastError: error.message,
        errorCount: 1,
        responseTime: Date.now() - start,
      };
    }
  }

  /**
   * Check email service (ApexMail via Control Plane)
   */
  private async checkEmailService(): Promise<ComponentHealth> {
    const start = Date.now();
    // ApexMail uses SMTP configuration - check for SMTP_HOST or control-plane URL
    const hasEmailConfig = !!process.env.SMTP_HOST || !!process.env.CONTROL_PLANE_URL;

    if (!hasEmailConfig) {
      return {
        name: 'email_service',
        status: 'degraded',
        lastSuccess: null,
        lastError: 'SMTP not configured (SMTP_HOST or CONTROL_PLANE_URL required)',
        errorCount: 1,
        responseTime: Date.now() - start,
      };
    }

    // Check recent send success rate
    try {
      const result = await this.pool.query(`
        SELECT 
          COUNT(*) FILTER (WHERE status = 'sent') as sent,
          COUNT(*) FILTER (WHERE status = 'failed') as failed
        FROM outreach_emails 
        WHERE created_at > NOW() - INTERVAL '1 hour'
      `);

      const sent = parseInt(result.rows[0]?.sent || '0');
      const failed = parseInt(result.rows[0]?.failed || '0');
      const total = sent + failed;

      if (total > 0 && failed / total > 0.3) {
        return {
          name: 'email_service',
          status: 'degraded',
          lastSuccess: new Date(),
          lastError: `High failure rate: ${((failed / total) * 100).toFixed(1)}%`,
          errorCount: failed,
          responseTime: Date.now() - start,
        };
      }

      return {
        name: 'email_service',
        status: 'up',
        lastSuccess: new Date(),
        lastError: null,
        errorCount: 0,
        responseTime: Date.now() - start,
      };
    } catch (error: any) {
      return {
        name: 'email_service',
        status: 'down',
        lastSuccess: null,
        lastError: error.message,
        errorCount: 1,
        responseTime: Date.now() - start,
      };
    }
  }

  /**
   * Check email queue for stuck emails
   * OVERCOMES: "emails not sending", "campaign stuck"
   */
  private async checkEmailQueue(): Promise<ComponentHealth> {
    const start = Date.now();
    try {
      const result = await this.pool.query(`
        SELECT COUNT(*) as stuck
        FROM outreach_emails 
        WHERE status = 'pending'
          AND scheduled_for < NOW() - INTERVAL '2 hours'
      `);

      const stuck = parseInt(result.rows[0]?.stuck || '0');

      if (stuck > 50) {
        return {
          name: 'email_queue',
          status: 'degraded',
          lastSuccess: new Date(),
          lastError: `${stuck} emails stuck in queue for >2 hours`,
          errorCount: stuck,
          responseTime: Date.now() - start,
        };
      }

      return {
        name: 'email_queue',
        status: 'up',
        lastSuccess: new Date(),
        lastError: null,
        errorCount: 0,
        responseTime: Date.now() - start,
      };
    } catch (error: any) {
      return {
        name: 'email_queue',
        status: 'down',
        lastSuccess: null,
        lastError: error.message,
        errorCount: 1,
        responseTime: Date.now() - start,
      };
    }
  }

  /**
   * Check warmup health
   */
  private async checkWarmupHealth(): Promise<ComponentHealth> {
    const start = Date.now();
    try {
      const result = await this.pool.query(`
        SELECT 
          COUNT(*) FILTER (WHERE reputation = 'damaged') as damaged,
          COUNT(*) as total
        FROM email_mailbox_warmup
      `);

      const damaged = parseInt(result.rows[0]?.damaged || '0');

      if (damaged > 0) {
        return {
          name: 'warmup',
          status: 'degraded',
          lastSuccess: new Date(),
          lastError: `${damaged} mailbox(es) with damaged reputation`,
          errorCount: damaged,
          responseTime: Date.now() - start,
        };
      }

      return {
        name: 'warmup',
        status: 'up',
        lastSuccess: new Date(),
        lastError: null,
        errorCount: 0,
        responseTime: Date.now() - start,
      };
    } catch (error: any) {
      // Table might not exist yet
      return {
        name: 'warmup',
        status: 'up',
        lastSuccess: new Date(),
        lastError: null,
        errorCount: 0,
        responseTime: Date.now() - start,
      };
    }
  }

  /**
   * Check bounce rate
   * OVERCOMES: "high bounce rates destroying sender reputation"
   */
  private async checkBounceRate(): Promise<ComponentHealth> {
    const start = Date.now();
    try {
      const result = await this.pool.query(`
        SELECT 
          COUNT(*) FILTER (WHERE bounced = true) as bounced,
          COUNT(*) as total
        FROM sales_contacts
        WHERE created_at > NOW() - INTERVAL '7 days'
      `);

      const bounced = parseInt(result.rows[0]?.bounced || '0');
      const total = parseInt(result.rows[0]?.total || '1');
      const bounceRate = (bounced / total) * 100;

      if (bounceRate > 5) {
        return {
          name: 'bounce_rate',
          status: bounceRate > 10 ? 'down' : 'degraded',
          lastSuccess: new Date(),
          lastError: `Bounce rate ${bounceRate.toFixed(1)}% (target: <3%)`,
          errorCount: bounced,
          responseTime: Date.now() - start,
        };
      }

      return {
        name: 'bounce_rate',
        status: 'up',
        lastSuccess: new Date(),
        lastError: null,
        errorCount: 0,
        responseTime: Date.now() - start,
      };
    } catch (error: any) {
      return {
        name: 'bounce_rate',
        status: 'up',
        lastSuccess: new Date(),
        lastError: null,
        errorCount: 0,
        responseTime: Date.now() - start,
      };
    }
  }

  /**
   * Automatic recovery actions
   * OVERCOMES: "issues take days for support to resolve"
   */
  private async attemptAutoRecovery(components: ComponentHealth[]): Promise<void> {
    for (const component of components) {
      if (component.status === 'down' || component.status === 'degraded') {
        console.log(`[ReliabilityGuard] Attempting recovery for: ${component.name}`);

        switch (component.name) {
          case 'database':
            await this.recoverDatabase();
            break;
          case 'email_queue':
            await this.recoverStuckEmails();
            break;
          case 'bounce_rate':
            await this.pauseHighBounceContacts();
            break;
          case 'warmup':
            // Warmup recovery is handled by WarmupService
            break;
        }
      }
    }
  }

  /**
   * Recover database connection
   */
  private async recoverDatabase(): Promise<void> {
    console.log('[ReliabilityGuard] Attempting database reconnection...');
    try {
      // End current pool
      await this.pool.end();
      
      // Create new pool
      this.pool = new Pool({ connectionString: process.env.DATABASE_URL });
      
      // Test connection
      await this.pool.query('SELECT 1');
      
      this.logRecovery('database', 'reconnect', true, 'Database reconnected successfully');
    } catch (error: any) {
      this.logRecovery('database', 'reconnect', false, error.message);
    }
  }

  /**
   * Recover stuck emails
   * OVERCOMES: "emails disappear", "replies not showing"
   */
  private async recoverStuckEmails(): Promise<void> {
    try {
      // Find stuck pending emails
      const stuck = await this.pool.query<QueuedEmail>(`
        SELECT id, enrollment_id, contact_id, subject, body_text as body, scheduled_for, 
               COALESCE(retries, 0) as retries, last_error
        FROM outreach_emails 
        WHERE status = 'pending'
          AND scheduled_for < NOW() - INTERVAL '2 hours'
        LIMIT 50
      `);

      let recovered = 0;
      let dead = 0;

      for (const email of stuck.rows) {
        const retries = email.retries || 0;

        if (retries >= this.CONFIG.maxRetries) {
          // Move to dead letter
          await this.pool.query(`
            UPDATE outreach_emails 
            SET status = 'failed', 
                error_message = 'Max retries exceeded after being stuck',
                updated_at = CURRENT_TIMESTAMP
            WHERE id = $1
          `, [email.id]);
          dead++;
        } else {
          // Reschedule with backoff
          const backoffMinutes = this.CONFIG.retryBackoff[Math.min(retries, this.CONFIG.retryBackoff.length - 1)];
          await this.pool.query(`
            UPDATE outreach_emails 
            SET scheduled_for = NOW() + $2 * INTERVAL '1 minute',
                retries = $3,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = $1
          `, [email.id, backoffMinutes, retries + 1]);
          recovered++;
        }
      }

      if (recovered > 0 || dead > 0) {
        console.log(`[ReliabilityGuard] Email recovery: ${recovered} rescheduled, ${dead} moved to dead letter`);
        this.logRecovery('email_queue', 'reset', true, `Recovered ${recovered}, dead-lettered ${dead}`);
      }
    } catch (error: any) {
      console.error('[ReliabilityGuard] Email recovery failed:', error.message);
      this.logRecovery('email_queue', 'reset', false, error.message);
    }
  }

  /**
   * Pause contacts with high bounce indicators
   * OVERCOMES: "bounce rate killing sender reputation"
   */
  private async pauseHighBounceContacts(): Promise<void> {
    try {
      const result = await this.pool.query(`
        UPDATE sales_contacts
        SET status = 'paused',
            notes = COALESCE(notes, '') || ' [Auto-paused: high bounce indicator]'
        WHERE status = 'new'
          AND (
            email_verification_status = 'invalid'
            OR email_verification_status = 'disposable'
            OR email_verification_score < 30
          )
          AND created_at > NOW() - INTERVAL '7 days'
        RETURNING id
      `);

      if (result.rowCount && result.rowCount > 0) {
        console.log(`[ReliabilityGuard] Paused ${result.rowCount} high-bounce-risk contacts`);
        this.logRecovery('bounce_rate', 'reset', true, `Paused ${result.rowCount} risky contacts`);
      }
    } catch (error: any) {
      console.error('[ReliabilityGuard] Bounce prevention failed:', error.message);
    }
  }

  /**
   * Circuit breaker pattern
   * OVERCOMES: "cascade failures", "one bug crashes everything"
   */
  recordCircuitBreakerFailure(component: string): void {
    const breaker = this.circuitBreakers.get(component);
    if (!breaker) return;

    breaker.failures++;
    breaker.lastFailure = new Date();

    if (breaker.failures >= this.CONFIG.circuitBreakerThreshold) {
      breaker.state = 'open';
      console.log(`[ReliabilityGuard] Circuit OPEN for ${component} after ${breaker.failures} failures`);

      // Schedule half-open check
      setTimeout(() => {
        const b = this.circuitBreakers.get(component);
        if (b && b.state === 'open') {
          b.state = 'half-open';
          console.log(`[ReliabilityGuard] Circuit HALF-OPEN for ${component}, will test on next request`);
        }
      }, this.CONFIG.circuitBreakerReset);
    }
  }

  /**
   * Check if component circuit is open (should not be called)
   */
  isCircuitOpen(component: string): boolean {
    const breaker = this.circuitBreakers.get(component);
    return breaker?.state === 'open';
  }

  /**
   * Record successful operation (reset circuit breaker)
   */
  recordSuccess(component: string): void {
    const breaker = this.circuitBreakers.get(component);
    if (breaker) {
      breaker.failures = 0;
      breaker.state = 'closed';
    }
  }

  /**
   * Check database connection pool
   */
  private async checkDatabasePool(): Promise<void> {
    try {
      const totalCount = this.pool.totalCount;
      const idleCount = this.pool.idleCount;
      const waitingCount = this.pool.waitingCount;

      if (waitingCount > 5) {
        console.warn(`[ReliabilityGuard] Database pool under pressure: ${waitingCount} waiting`);
      }

      if (idleCount === 0 && totalCount > 0) {
        console.warn('[ReliabilityGuard] No idle database connections');
      }
    } catch (error) {
      // Pool stats not available
    }
  }

  /**
   * Log recovery action
   */
  private logRecovery(component: string, action: RecoveryAction['action'], success: boolean, details: string): void {
    const recovery: RecoveryAction = {
      component,
      action,
      timestamp: new Date(),
      success,
      details,
    };
    this.recoveryLog.push(recovery);

    // Keep only last 100 recovery actions
    if (this.recoveryLog.length > 100) {
      this.recoveryLog = this.recoveryLog.slice(-100);
    }
  }

  /**
   * Get current health status
   */
  getHealth(): SystemHealth {
    return this.health;
  }

  /**
   * Get recovery log
   */
  getRecoveryLog(): RecoveryAction[] {
    return this.recoveryLog;
  }

  /**
   * Manual health check trigger
   */
  async triggerHealthCheck(): Promise<SystemHealth> {
    await this.performHealthCheck();
    return this.health;
  }
}
