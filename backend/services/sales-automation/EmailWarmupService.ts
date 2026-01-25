// services/sales-automation/EmailWarmupService.ts
// Email warmup service to establish sender reputation before cold outreach
// Critical for deliverability - new domains need 2-4 weeks warmup

import { Pool } from 'pg';
import { Resend } from 'resend';

interface WarmupConfig {
  // Warmup phases (daily send limits)
  phases: WarmupPhase[];
  // Internal emails to warm up with (your own addresses)
  warmupEmails: string[];
  // Days before allowing cold outreach
  minimumWarmupDays: number;
}

interface WarmupPhase {
  day: number;      // Starting day
  dailyLimit: number;
  coldAllowed: boolean;
}

interface MailboxStatus {
  mailbox: string;
  warmupStartDate: Date;
  currentDay: number;
  phase: number;
  dailySent: number;
  dailyLimit: number;
  reputation: 'warming' | 'warm' | 'cold' | 'damaged';
  coldOutreachAllowed: boolean;
  metrics: {
    totalSent: number;
    bounceRate: number;
    spamRate: number;
    openRate: number;
  };
}

// Industry-standard warmup schedule
const DEFAULT_WARMUP_CONFIG: WarmupConfig = {
  phases: [
    // Week 1: Very low volume, internal only
    { day: 1, dailyLimit: 5, coldAllowed: false },
    { day: 2, dailyLimit: 8, coldAllowed: false },
    { day: 3, dailyLimit: 12, coldAllowed: false },
    { day: 4, dailyLimit: 15, coldAllowed: false },
    { day: 5, dailyLimit: 20, coldAllowed: false },
    { day: 6, dailyLimit: 25, coldAllowed: false },
    { day: 7, dailyLimit: 30, coldAllowed: false },
    // Week 2: Gradually increase, still internal focused
    { day: 8, dailyLimit: 35, coldAllowed: false },
    { day: 9, dailyLimit: 40, coldAllowed: false },
    { day: 10, dailyLimit: 45, coldAllowed: false },
    { day: 11, dailyLimit: 50, coldAllowed: true },  // Start allowing cold
    { day: 12, dailyLimit: 55, coldAllowed: true },
    { day: 13, dailyLimit: 60, coldAllowed: true },
    { day: 14, dailyLimit: 65, coldAllowed: true },
    // Week 3-4: Ramp to full capacity
    { day: 15, dailyLimit: 70, coldAllowed: true },
    { day: 21, dailyLimit: 80, coldAllowed: true },
    { day: 28, dailyLimit: 100, coldAllowed: true },  // Full capacity
  ],
  warmupEmails: [
    // These should be real emails you control that will open/reply
     'hello@apexmediation.ee',
     'apexmediation@proton.me',
  ],
  minimumWarmupDays: 11,
};

// Warmup email content templates (look like real emails)
const WARMUP_TEMPLATES = [
  {
    subject: 'Quick question about the dashboard',
    body: `Hey,

Just wanted to follow up on the dashboard metrics we discussed. Can you take a look when you get a chance?

Thanks!`,
  },
  {
    subject: 'Re: Project update',
    body: `Hi,

Got your message. I'll review the changes and get back to you by EOD.

Best,`,
  },
  {
    subject: 'Meeting notes from yesterday',
    body: `Hi team,

Attaching the notes from our sync yesterday. Let me know if I missed anything.

Cheers,`,
  },
  {
    subject: 'Fwd: Interesting article',
    body: `Thought you might find this relevant to what we're working on.

Let me know your thoughts!`,
  },
  {
    subject: 'Quick sync?',
    body: `Hey,

Do you have 15 mins this week to chat about the upcoming release?

Thanks`,
  },
  {
    subject: 'Re: Thanks!',
    body: `No problem at all! Happy to help.

Let me know if you need anything else.`,
  },
];

export class EmailWarmupService {
  private pool: Pool;
  private resend: Resend | null = null;
  private config: WarmupConfig;

  constructor(databaseUrl: string, config?: Partial<WarmupConfig>) {
    this.pool = new Pool({ connectionString: databaseUrl });
    this.config = { ...DEFAULT_WARMUP_CONFIG, ...config };

    const resendApiKey = process.env.RESEND_API_KEY;
    if (resendApiKey) {
      this.resend = new Resend(resendApiKey);
    }
  }

  /**
   * Initialize warmup for a new mailbox
   */
  async initializeMailboxWarmup(mailbox: string): Promise<void> {
    await this.pool.query(`
      INSERT INTO email_mailbox_warmup (
        mailbox, warmup_start_date, current_phase, reputation_status,
        total_sent, bounce_count, spam_count, open_count
      ) VALUES ($1, CURRENT_DATE, 1, 'warming', 0, 0, 0, 0)
      ON CONFLICT (mailbox) DO NOTHING
    `, [mailbox]);

    console.log(`[Warmup] Initialized warmup for mailbox: ${mailbox}`);
  }

  /**
   * Get current status of a mailbox
   */
  async getMailboxStatus(mailbox: string): Promise<MailboxStatus | null> {
    const result = await this.pool.query(`
      SELECT * FROM email_mailbox_warmup WHERE mailbox = $1
    `, [mailbox]);

    if (result.rows.length === 0) return null;

    const row = result.rows[0];
    const warmupStartDate = new Date(row.warmup_start_date);
    const currentDay = Math.floor((Date.now() - warmupStartDate.getTime()) / (24 * 60 * 60 * 1000)) + 1;
    
    // Find current phase
    const phase = this.config.phases.reduce((acc, p, idx) => 
      currentDay >= p.day ? idx : acc, 0
    );
    const currentPhase = this.config.phases[phase];

    // Check if cold outreach is allowed
    const coldOutreachAllowed = currentDay >= this.config.minimumWarmupDays && 
      row.reputation_status !== 'damaged' &&
      row.bounce_count / Math.max(row.total_sent, 1) < 0.05 &&  // <5% bounce rate
      row.spam_count / Math.max(row.total_sent, 1) < 0.01;      // <1% spam rate

    return {
      mailbox,
      warmupStartDate,
      currentDay,
      phase,
      dailySent: row.daily_sent || 0,
      dailyLimit: currentPhase?.dailyLimit || 5,
      reputation: row.reputation_status,
      coldOutreachAllowed,
      metrics: {
        totalSent: row.total_sent,
        bounceRate: row.total_sent > 0 ? (row.bounce_count / row.total_sent) * 100 : 0,
        spamRate: row.total_sent > 0 ? (row.spam_count / row.total_sent) * 100 : 0,
        openRate: row.total_sent > 0 ? (row.open_count / row.total_sent) * 100 : 0,
      },
    };
  }

  /**
   * Run daily warmup cycle
   * Sends warmup emails between internal addresses
   */
  async runWarmupCycle(mailbox: string): Promise<{ sent: number; errors: number }> {
    if (!this.resend) {
      console.log('[Warmup] Resend not configured, skipping warmup');
      return { sent: 0, errors: 0 };
    }

    const status = await this.getMailboxStatus(mailbox);
    if (!status) {
      await this.initializeMailboxWarmup(mailbox);
      return { sent: 0, errors: 0 };
    }

    console.log(`[Warmup] Running warmup for ${mailbox} (Day ${status.currentDay}, Limit: ${status.dailyLimit})`);

    // Calculate how many warmup emails to send
    const warmupEmailsToSend = Math.min(
      status.dailyLimit - status.dailySent,
      Math.ceil(status.dailyLimit * 0.3)  // 30% of limit for warmup
    );

    if (warmupEmailsToSend <= 0) {
      console.log('[Warmup] Daily limit reached or no warmup emails needed');
      return { sent: 0, errors: 0 };
    }

    let sent = 0;
    let errors = 0;

    // Send warmup emails to internal addresses
    for (let i = 0; i < warmupEmailsToSend; i++) {
      const template = WARMUP_TEMPLATES[Math.floor(Math.random() * WARMUP_TEMPLATES.length)];
      const recipient = this.config.warmupEmails[Math.floor(Math.random() * this.config.warmupEmails.length)];

      try {
        await this.resend.emails.send({
          from: mailbox,
          to: recipient,
          subject: template.subject,
          text: template.body,
        });

        sent++;
        
        // Simulate reply (in production, this would be done by the receiving mailbox)
        // This helps establish reputation as emails are "engaged with"
        
        await this.delay(this.randomDelay(30, 120) * 1000);  // 30-120 second delay
      } catch (error) {
        console.error(`[Warmup] Error sending warmup email:`, error);
        errors++;
      }
    }

    // Update daily sent count
    await this.pool.query(`
      UPDATE email_mailbox_warmup
      SET daily_sent = daily_sent + $2,
          total_sent = total_sent + $2,
          last_warmup_at = CURRENT_TIMESTAMP,
          updated_at = CURRENT_TIMESTAMP
      WHERE mailbox = $1
    `, [mailbox, sent]);

    console.log(`[Warmup] Sent ${sent} warmup emails for ${mailbox}`);
    return { sent, errors };
  }

  /**
   * Check if mailbox can send cold emails
   */
  async canSendColdEmail(mailbox: string): Promise<{ allowed: boolean; reason?: string; limit?: number }> {
    const status = await this.getMailboxStatus(mailbox);

    if (!status) {
      return { allowed: false, reason: 'Mailbox not initialized for warmup' };
    }

    if (status.reputation === 'damaged') {
      return { allowed: false, reason: 'Mailbox reputation is damaged, needs recovery' };
    }

    if (status.currentDay < this.config.minimumWarmupDays) {
      return { 
        allowed: false, 
        reason: `Warmup in progress: Day ${status.currentDay}/${this.config.minimumWarmupDays}`,
      };
    }

    if (status.metrics.bounceRate > 5) {
      return { allowed: false, reason: `Bounce rate too high: ${status.metrics.bounceRate.toFixed(1)}%` };
    }

    if (status.metrics.spamRate > 1) {
      return { allowed: false, reason: `Spam rate too high: ${status.metrics.spamRate.toFixed(1)}%` };
    }

    // Calculate remaining cold email capacity
    const warmupReserve = Math.ceil(status.dailyLimit * 0.1);  // Reserve 10% for warmup
    const remaining = status.dailyLimit - status.dailySent - warmupReserve;

    if (remaining <= 0) {
      return { allowed: false, reason: 'Daily limit reached' };
    }

    return { allowed: true, limit: remaining };
  }

  /**
   * Record email send result for reputation tracking
   */
  async recordSendResult(mailbox: string, result: 'sent' | 'bounced' | 'spam'): Promise<void> {
    const updateField = result === 'bounced' ? 'bounce_count' : 
                        result === 'spam' ? 'spam_count' : 
                        'total_sent';

    await this.pool.query(`
      UPDATE email_mailbox_warmup
      SET ${updateField} = ${updateField} + 1,
          daily_sent = daily_sent + 1,
          updated_at = CURRENT_TIMESTAMP
      WHERE mailbox = $1
    `, [mailbox]);

    // Check if reputation is damaged
    if (result === 'bounced' || result === 'spam') {
      const status = await this.getMailboxStatus(mailbox);
      if (status && (status.metrics.bounceRate > 10 || status.metrics.spamRate > 2)) {
        await this.pool.query(`
          UPDATE email_mailbox_warmup
          SET reputation_status = 'damaged'
          WHERE mailbox = $1
        `, [mailbox]);
        console.warn(`[Warmup] ⚠️ Mailbox ${mailbox} reputation damaged!`);
      }
    }
  }

  /**
   * Record email open for reputation tracking
   */
  async recordOpen(mailbox: string): Promise<void> {
    await this.pool.query(`
      UPDATE email_mailbox_warmup
      SET open_count = open_count + 1,
          updated_at = CURRENT_TIMESTAMP
      WHERE mailbox = $1
    `, [mailbox]);
  }

  /**
   * Reset daily counts (run at midnight)
   */
  async resetDailyCounts(): Promise<void> {
    await this.pool.query(`
      UPDATE email_mailbox_warmup
      SET daily_sent = 0,
          updated_at = CURRENT_TIMESTAMP
    `);
    console.log('[Warmup] Reset daily counts for all mailboxes');
  }

  /**
   * Get warmup progress report
   */
  async getWarmupReport(): Promise<MailboxStatus[]> {
    const result = await this.pool.query(`
      SELECT mailbox FROM email_mailbox_warmup ORDER BY mailbox
    `);

    const statuses: MailboxStatus[] = [];
    for (const row of result.rows) {
      const status = await this.getMailboxStatus(row.mailbox);
      if (status) statuses.push(status);
    }

    return statuses;
  }

  private delay(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  private randomDelay(minSeconds: number, maxSeconds: number): number {
    return Math.floor(Math.random() * (maxSeconds - minSeconds + 1)) + minSeconds;
  }
}

export default EmailWarmupService;
