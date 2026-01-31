// services/email/EmailAutomationService.ts
// Email automation with ApexMail (self-hosted, replaces Resend)

import { Pool } from 'pg';
import {
  BillingPreviewEmailPayload,
  PaymentFailedEmailPayload,
  PaymentRetryEmailPayload,
  PaymentSucceededEmailPayload,
  SUPPORT_EMAIL_ADDRESS,
  buildBillingPreviewEmail,
  buildPaymentFailedEmail,
  buildPaymentRetryEmail,
  buildPaymentSucceededEmail,
} from './billingEmailBuilders';

// ApexMail API endpoint (via Control Plane)
const APEXMAIL_BASE_URL = process.env.APEXMAIL_URL || 'http://localhost:4000/api/apexmail';
const APEXMAIL_API_KEY = process.env.APEXMAIL_API_KEY;

// ApexMail helper function
async function sendViaApexMail(options: {
  from: string;
  to: string;
  subject: string;
  html: string;
  text?: string;
  replyTo?: string;
  category?: 'transactional' | 'marketing' | 'notification';
}): Promise<{ data?: { id: string }; error?: { message: string } }> {
  try {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };
    if (APEXMAIL_API_KEY) {
      headers['X-ApexMail-Key'] = APEXMAIL_API_KEY;
    }

    const response = await fetch(`${APEXMAIL_BASE_URL}/send`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        to: options.to,
        from: options.from,
        subject: options.subject,
        html: options.html,
        text: options.text || options.html.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ').trim(),
        replyTo: options.replyTo,
        category: options.category || 'transactional',
        priority: 1,
      }),
    });

    if (response.ok) {
      const result = await response.json() as { emailId?: string };
      return { data: { id: result.emailId || 'unknown' } };
    } else {
      const error = await response.text();
      return { error: { message: `ApexMail error: ${response.status} ${error}` } };
    }
  } catch (error) {
    return { error: { message: error instanceof Error ? error.message : 'Unknown error' } };
  }
}

const db = new Pool({
  connectionString: process.env.DATABASE_URL,
});


interface EmailTemplate {
  id: string;
  subject: string;
  from: string;
  replyTo?: string;
  html: string;
  text: string;
}

interface EmailJob {
  to: string;
  template_id: string;
  data: Record<string, any>;
  scheduled_for?: Date;
}


export class EmailAutomationService {
  private readonly FROM_EMAIL = 'ApexMediation Support <support@apexmediation.ee>';
  private readonly SUPPORT_EMAIL = SUPPORT_EMAIL_ADDRESS;
  private readonly FOUNDER_EMAIL = 'ApexMediation Contact <contact@apexmediation.ee>';

  /**
   * Process email event queue (run every minute via cron)
   */
  async processEmailQueue(): Promise<void> {
    // Get pending email events
    const events = await db.query(
      `SELECT id, event_type, data, created_at
       FROM events
       WHERE event_type LIKE 'email.%'
         AND processed_at IS NULL
       ORDER BY created_at ASC
       LIMIT 100`
    );

    for (const event of events.rows) {
      try {
        await this.handleEmailEvent(event.event_type, JSON.parse(event.data));

        // Mark as processed
        await db.query(
          `UPDATE events SET processed_at = NOW() WHERE id = $1`,
          [event.id]
        );
      } catch (error) {
        console.error(`[Email] Error processing event ${event.id}:`, error);
        // Mark as failed
        await db.query(
          `UPDATE events 
           SET processed_at = NOW(), 
               error_message = $2 
           WHERE id = $1`,
          [event.id, (error as Error).message]
        );
      }
    }
  }

  /**
   * Handle email event
   */
  private async handleEmailEvent(eventType: string, data: any): Promise<void> {
    switch (eventType) {
      case 'email.welcome':
        await this.sendWelcomeEmail(data);
        break;
      case 'email.trial_ending':
        await this.sendTrialEndingEmail(data);
        break;
      case 'email.billing_preview':
        await this.sendBillingPreviewEmail(data);
        break;
      case 'email.payment_failed':
        await this.sendPaymentFailedEmail(data);
        break;
      case 'email.payment_retry':
        await this.sendPaymentRetryEmail(data);
        break;
      case 'email.payment_succeeded_after_retry':
        await this.sendPaymentSucceededEmail(data);
        break;
      case 'email.subscription_suspended':
        await this.sendSubscriptionSuspendedEmail(data);
        break;
      case 'email.usage_alert':
        await this.sendUsageAlertEmail(data);
        break;
      case 'email.monthly_summary':
        await this.sendMonthlySummaryEmail(data);
        break;
      case 'email.sdk_update':
        await this.sendSDKUpdateEmail(data);
        break;
      case 'email.milestone_celebration':
        await this.sendMilestoneCelebrationEmail(data);
        break;
      case 'email.referral_invite':
        await this.sendReferralInviteEmail(data);
        break;
      case 'email.testimonial_request':
        await this.sendTestimonialRequestEmail(data);
        break;
      case 'email.case_study_invite':
        await this.sendCaseStudyInviteEmail(data);
        break;
      case 'email.community_champion_reward':
        await this.sendCommunityChampionRewardEmail(data);
        break;
      default:
        console.log(`[Email] Unknown event type: ${eventType}`);
    }
  }

  /**
   * Welcome email (sent immediately after signup)
   */
  private async sendWelcomeEmail(data: {
    to: string;
    customer_id: string;
    api_key: string;
    plan_type: string;
  }): Promise<void> {
    const { data: result, error } = await sendViaApexMail({
      from: this.FOUNDER_EMAIL,
      to: data.to,
      subject: 'Welcome to ApexMediation! 🚀',
      html: `
        <h1>Welcome to ApexMediation!</h1>
        <p>Hi there,</p>
        <p>Thanks for signing up! Your account is ready to go.</p>
        
        <h2>Your API Key</h2>
        <code style="background: #f4f4f4; padding: 10px; display: block; font-family: monospace;">
          ${data.api_key}
        </code>
        
        <h2>Get Started</h2>
        <ol>
          <li><strong>Download the SDK:</strong> <a href="https://docs.apexmediation.ee/download">docs.apexmediation.ee/download</a></li>
          <li><strong>Follow the integration guide:</strong> <a href="https://docs.apexmediation.ee/quickstart">docs.apexmediation.ee/quickstart</a></li>
          <li><strong>Join our community:</strong> <a href="https://discord.gg/apexmediation">discord.gg/apexmediation</a></li>
        </ol>
        
        <p><strong>You have 14 days of free trial.</strong> No credit card charged until your trial ends.</p>
        
        <p>Need help? Just reply to this email or visit <a href="https://docs.apexmediation.ee">docs.apexmediation.ee</a></p>
        
        <p>Best regards,<br>
        Sabel Akhoua<br>
        Founder, ApexMediation</p>
        
        <hr>
        <p style="font-size: 12px; color: #666;">
          Plan: ${data.plan_type}<br>
          Console: <a href="https://console.apexmediation.ee">console.apexmediation.ee</a>
        </p>
      `,
      replyTo: this.FOUNDER_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send welcome email: ${error.message}`);
    }

    console.log(`[Email] Sent welcome email to ${data.to}`);
  }

  /**
   * Trial ending reminder (sent 7 days, 3 days, 1 day before trial ends)
   */
  private async sendTrialEndingEmail(data: {
    to: string;
    customer_id: string;
    days_remaining: number;
    plan_type: string;
    plan_price: string;
  }): Promise<void> {
    const { data: result, error } = await sendViaApexMail({
      from: this.FROM_EMAIL,
      to: data.to,
      subject: `Your ApexMediation trial ends in ${data.days_remaining} day${data.days_remaining > 1 ? 's' : ''}`,
      html: `
        <h1>Your trial ends in ${data.days_remaining} day${data.days_remaining > 1 ? 's' : ''}</h1>
        <p>Hi there,</p>
        <p>Just a friendly reminder that your ApexMediation trial ends in ${data.days_remaining} day${data.days_remaining > 1 ? 's' : ''}.</p>
        
        <p><strong>Your plan:</strong> ${data.plan_type} (${data.plan_price}/month)</p>
        
        <p>After your trial ends, your payment method will be charged automatically to continue service.</p>
        
        <h2>What happens next?</h2>
        <ul>
          <li>✅ Uninterrupted service - no downtime</li>
          <li>✅ Keep all your data and settings</li>
          <li>✅ Continue using the SDK without changes</li>
        </ul>
        
        <p><strong>Want to change your plan?</strong></p>
        <p><a href="https://console.apexmediation.ee/billing" style="background: #0066cc; color: white; padding: 10px 20px; text-decoration: none; display: inline-block; border-radius: 5px;">Manage Billing</a></p>
        
        <p>Questions? Reply to this email - I read every message personally.</p>
        
        <p>Best regards,<br>
        Sabel Akhoua<br>
        Founder, ApexMediation</p>
      `,
      replyTo: this.FOUNDER_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send trial ending email: ${error.message}`);
    }

    console.log(`[Email] Sent trial ending email to ${data.to} (${data.days_remaining} days)`);
  }

  /**
   * Billing preview (sent ~5 days before autopay)
   */
  private async sendBillingPreviewEmail(data: BillingPreviewEmailPayload): Promise<void> {
    const { to, customer_id: _customerId, ...content } = data;
    const { subject, html } = buildBillingPreviewEmail(content);

    const { data: result, error } = await sendViaApexMail({
      from: this.FROM_EMAIL,
      to,
      subject,
      html,
      replyTo: this.SUPPORT_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send billing preview email: ${error.message}`);
    }

    console.log(`[Email] Sent billing preview email to ${data.to}`);
  }

  /**
   * Payment failed notification
   */
  private async sendPaymentFailedEmail(data: PaymentFailedEmailPayload): Promise<void> {
    const { to, customer_id: _customerId, ...content } = data;
    const { subject, html } = buildPaymentFailedEmail(content);

    const { data: result, error } = await sendViaApexMail({
      from: this.FROM_EMAIL,
      to,
      subject,
      html,
      replyTo: this.SUPPORT_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send payment failed email: ${error.message}`);
    }

    console.log(`[Email] Sent payment failed email to ${data.to}`);
  }

  /**
   * Payment retry notification
   */
  private async sendPaymentRetryEmail(data: PaymentRetryEmailPayload): Promise<void> {
    const { to, customer_id: _customerId, ...content } = data;
    const { subject, html } = buildPaymentRetryEmail(content);

    const { data: result, error } = await sendViaApexMail({
      from: this.FROM_EMAIL,
      to,
      subject,
      html,
      replyTo: this.SUPPORT_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send payment retry email: ${error.message}`);
    }

    console.log(`[Email] Sent payment retry email to ${data.to} (attempt ${data.attempt_number})`);
  }

  /**
   * Payment succeeded after retry
   */
  private async sendPaymentSucceededEmail(data: PaymentSucceededEmailPayload): Promise<void> {
    const { to, customer_id: _customerId, ...content } = data;
    const { subject, html } = buildPaymentSucceededEmail(content);

    const { data: result, error } = await sendViaApexMail({
      from: this.FROM_EMAIL,
      to,
      subject,
      html,
      replyTo: this.SUPPORT_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send payment succeeded email: ${error.message}`);
    }

    console.log(`[Email] Sent payment succeeded email to ${data.to}`);
  }

  /**
   * Subscription suspended notification
   */
  private async sendSubscriptionSuspendedEmail(data: {
    to: string;
    customer_id: string;
    reactivation_url: string;
  }): Promise<void> {
    const { data: result, error } = await sendViaApexMail({
      from: this.FROM_EMAIL,
      to: data.to,
      subject: '🚫 Service suspended - payment required',
      html: `
        <h1>Service Suspended</h1>
        <p>Hi there,</p>
        <p>Your ApexMediation service has been suspended due to multiple failed payment attempts.</p>
        
        <h2>What's affected?</h2>
        <ul>
          <li>❌ API access disabled</li>
          <li>❌ SDK functionality limited</li>
          <li>❌ Dashboard access restricted</li>
        </ul>
        
        <h2>How to reactivate</h2>
        <ol>
          <li>Update your payment method</li>
          <li>Pay any outstanding invoices</li>
          <li>Service will be restored immediately</li>
        </ol>
        
        <p><a href="${data.reactivation_url}" style="background: #28a745; color: white; padding: 10px 20px; text-decoration: none; display: inline-block; border-radius: 5px;">Reactivate Service</a></p>
        
        <p><strong>Need help?</strong> We understand payment issues happen. Reply to this email to discuss payment plans or alternatives.</p>
        
        <p>Best regards,<br>
        ApexMediation Team</p>
      `,
      replyTo: this.SUPPORT_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send subscription suspended email: ${error.message}`);
    }

    console.log(`[Email] Sent subscription suspended email to ${data.to}`);
  }

  /**
   * Usage alert email (80%, 90%, 100%, overage)
   */
  private async sendUsageAlertEmail(data: {
    to: string;
    customer_id: string;
    percent_used: number;
    current_usage: number;
    limit: number;
    plan_type: string;
  }): Promise<void> {
    const isOverage = data.percent_used >= 100;
    const overage = isOverage ? data.current_usage - data.limit : 0;

    const { data: result, error } = await sendViaApexMail({
      from: this.FROM_EMAIL,
      to: data.to,
      subject: isOverage
        ? `⚠️ You're over your plan limit`
        : `📊 You've used ${data.percent_used}% of your plan`,
      html: `
        <h1>${isOverage ? 'Plan Limit Exceeded' : `${data.percent_used}% of Plan Used`}</h1>
        <p>Hi there,</p>
        <p>Your current usage: <strong>${data.current_usage.toLocaleString()} impressions</strong></p>
        <p>Your plan includes: <strong>${data.limit.toLocaleString()} impressions</strong></p>
        
        ${
          isOverage
            ? `
          <p style="background: #fff3cd; border-left: 4px solid #ffc107; padding: 15px; margin: 20px 0;">
            <strong>Overage charges apply:</strong> You've exceeded your plan limit by ${overage.toLocaleString()} impressions.
          </p>
          
          <h2>Avoid overage charges</h2>
          <p>Upgrade to a higher plan and save money!</p>
          <p><a href="https://console.apexmediation.ee/billing/upgrade" style="background: #0066cc; color: white; padding: 10px 20px; text-decoration: none; display: inline-block; border-radius: 5px;">Upgrade Plan</a></p>
        `
            : `
          <p>You're approaching your plan limit. Consider upgrading to avoid overage charges.</p>
          <p><a href="https://console.apexmediation.ee/billing/upgrade">View Upgrade Options</a></p>
        `
        }
        
        <p><a href="https://console.apexmediation.ee/usage">View Detailed Usage</a></p>
        
        <p>Best regards,<br>
        ApexMediation Team</p>
      `,
      replyTo: this.SUPPORT_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send usage alert email: ${error.message}`);
    }

    console.log(`[Email] Sent usage alert email to ${data.to} (${data.percent_used}%)`);
  }

  /**
   * Monthly usage summary
   */
  private async sendMonthlySummaryEmail(data: {
    to: string;
    customer_id: string;
    month: string;
    total_impressions: number;
    total_api_calls: number;
    revenue_generated: string;
  }): Promise<void> {
    const { data: result, error } = await sendViaApexMail({
      from: this.FROM_EMAIL,
      to: data.to,
      subject: `📊 Your ApexMediation summary for ${data.month}`,
      html: `
        <h1>Monthly Summary: ${data.month}</h1>
        <p>Hi there,</p>
        <p>Here's your activity summary for ${data.month}:</p>
        
        <table style="border-collapse: collapse; width: 100%; max-width: 500px; margin: 20px 0;">
          <tr style="background: #f4f4f4;">
            <td style="padding: 10px; border: 1px solid #ddd;"><strong>Metric</strong></td>
            <td style="padding: 10px; border: 1px solid #ddd;"><strong>Value</strong></td>
          </tr>
          <tr>
            <td style="padding: 10px; border: 1px solid #ddd;">Ad Impressions</td>
            <td style="padding: 10px; border: 1px solid #ddd;">${data.total_impressions.toLocaleString()}</td>
          </tr>
          <tr>
            <td style="padding: 10px; border: 1px solid #ddd;">API Calls</td>
            <td style="padding: 10px; border: 1px solid #ddd;">${data.total_api_calls.toLocaleString()}</td>
          </tr>
          <tr>
            <td style="padding: 10px; border: 1px solid #ddd;">Estimated Revenue</td>
            <td style="padding: 10px; border: 1px solid #ddd;">$${data.revenue_generated}</td>
          </tr>
        </table>
        
        <p><a href="https://console.apexmediation.ee/analytics">View Detailed Analytics</a></p>
        
        <p>Keep up the great work! 🚀</p>
        
        <p>Best regards,<br>
        ApexMediation Team</p>
      `,
      replyTo: this.SUPPORT_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send monthly summary email: ${error.message}`);
    }

    console.log(`[Email] Sent monthly summary email to ${data.to}`);
  }

  /**
   * SDK update notification
   */
  private async sendSDKUpdateEmail(data: {
    to: string;
    customer_id: string;
    sdk_version: string;
    platform: string;
    breaking_changes: boolean;
    changelog_url: string;
  }): Promise<void> {
    const { data: result, error } = await sendViaApexMail({
      from: this.FROM_EMAIL,
      to: data.to,
      subject: data.breaking_changes
        ? `🚨 ApexMediation SDK ${data.sdk_version} - Breaking Changes`
        : `🚀 ApexMediation SDK ${data.sdk_version} released`,
      html: `
        <h1>SDK Update: ${data.sdk_version}</h1>
        <p>Hi there,</p>
        <p>We've released a new version of the ApexMediation SDK for ${data.platform}.</p>
        
        ${
          data.breaking_changes
            ? `
          <p style="background: #fff3cd; border-left: 4px solid #ffc107; padding: 15px; margin: 20px 0;">
            <strong>⚠️ Breaking Changes:</strong> This update includes breaking changes. Please review the changelog before updating.
          </p>
        `
            : `
          <p>This update includes bug fixes, performance improvements, and new features.</p>
        `
        }
        
        <p><a href="${data.changelog_url}">View Changelog</a></p>
        
        <p><strong>How to update:</strong></p>
        <pre style="background: #f4f4f4; padding: 10px; border-radius: 5px; overflow-x: auto;">
${
  data.platform === 'iOS'
    ? 'pod update ApexMediation'
    : data.platform === 'Android'
      ? "implementation 'com.apexmediation:sdk:${data.sdk_version}'"
      : 'npm update @apexmediation/sdk'
}
        </pre>
        
        <p>Need help upgrading? <a href="https://docs.apexmediation.ee/migration">View Migration Guide</a></p>
        
        <p>Best regards,<br>
        ApexMediation Team</p>
      `,
      replyTo: this.SUPPORT_EMAIL,
    });

    if (error) {
      throw new Error(`Failed to send SDK update email: ${error.message}`);
    }

    console.log(`[Email] Sent SDK update email to ${data.to} (${data.sdk_version})`);
  }

  /**
   * Schedule trial ending reminders for all active trials
   * Run this daily via cron
   */
  async scheduleTrialReminders(): Promise<void> {
    console.log('[Email] Scheduling trial ending reminders...');

    // Get all active trials ending in 7, 3, or 1 day(s)
    const trials = await db.query(
      `SELECT 
         u.id as customer_id,
         u.email,
         s.plan_type,
         s.base_price_cents / 100.0 as plan_price,
         EXTRACT(DAY FROM s.trial_end_date - NOW()) as days_remaining
       FROM users u
       JOIN subscriptions s ON u.id = s.customer_id
       WHERE s.status = 'trialing'
         AND s.trial_end_date IS NOT NULL
         AND EXTRACT(DAY FROM s.trial_end_date - NOW()) IN (7, 3, 1)`
    );

    for (const trial of trials.rows) {
      // Check if we've already sent this reminder
      const existing = await db.query(
        `SELECT id FROM email_log
         WHERE customer_id = $1
           AND email_type = $2
           AND sent_at >= NOW() - INTERVAL '24 hours'`,
        [trial.customer_id, `trial_ending_${trial.days_remaining}d`]
      );

      if (existing.rows.length > 0) {
        continue; // Already sent
      }

      // Emit email event
      await db.query(
        `INSERT INTO events (event_type, data, created_at)
         VALUES ($1, $2, NOW())`,
        [
          'email.trial_ending',
          JSON.stringify({
            to: trial.email,
            customer_id: trial.customer_id,
            days_remaining: trial.days_remaining,
            plan_type: trial.plan_type,
            plan_price: `$${trial.plan_price}`,
          }),
        ]
      );

      // Log email
      await db.query(
        `INSERT INTO email_log (customer_id, email_type, sent_at)
         VALUES ($1, $2, NOW())`,
        [trial.customer_id, `trial_ending_${trial.days_remaining}d`]
      );

      console.log(
        `[Email] Scheduled trial reminder for ${trial.email} (${trial.days_remaining} days)`
      );
    }

    console.log('[Email] Trial reminders scheduled');
  }

  /**
   * Milestone celebration email (usage milestones reached)
   */
  private async sendMilestoneCelebrationEmail(data: {
    to: string;
    customer_id: string;
    milestone_type: string;
    total_impressions: number;
    days_since_start: number;
  }): Promise<void> {
    const milestoneEmojis: Record<string, string> = {
      first_100: '🎉',
      first_1k: '🚀',
      first_10k: '⭐',
      first_100k: '🏆',
      first_1m: '💎',
    };

    const milestoneNames: Record<string, string> = {
      first_100: '100 impressions',
      first_1k: '1,000 impressions',
      first_10k: '10,000 impressions',
      first_100k: '100,000 impressions',
      first_1m: '1,000,000 impressions',
    };

    const emoji = milestoneEmojis[data.milestone_type] || '🎉';
    const milestoneName = milestoneNames[data.milestone_type] || data.milestone_type;

    const { error } = await sendViaApexMail({
      from: this.FOUNDER_EMAIL,
      to: data.to,
      subject: `${emoji} Milestone Reached: ${milestoneName}!`,
      html: `
        <h1>${emoji} Congratulations!</h1>
        <p>You just hit <strong>${milestoneName}</strong> in ${data.days_since_start} days!</p>
        
        <h2>Your Progress</h2>
        <ul>
          <li>Total Impressions: <strong>${data.total_impressions.toLocaleString()}</strong></li>
          <li>Time to Milestone: <strong>${data.days_since_start} days</strong></li>
          <li>Average Daily Impressions: <strong>${Math.round(data.total_impressions / data.days_since_start).toLocaleString()}</strong></li>
        </ul>
        
        <h2>Keep Going!</h2>
        <p>Your ad mediation is gaining traction. Check your <a href="https://console.apexmediation.ee/analytics">analytics dashboard</a> for detailed insights.</p>
        
        <p>Need optimization tips? <a href="https://docs.apexmediation.ee/optimization">Visit our optimization guide</a></p>
        
        <p>Best regards,<br>
        Sabel Akhoua<br>
        Founder, ApexMediation</p>
      `,
    });

    if (error) {
      console.error(`[Email] Error sending milestone celebration to ${data.to}:`, error);
      throw error;
    }

    console.log(`[Email] Sent milestone celebration to ${data.to} (${milestoneName})`);
  }

  /**
   * Referral invite email (high-usage customers)
   */
  private async sendReferralInviteEmail(data: {
    to: string;
    customer_id: string;
    referral_code: string;
    reward_amount: string;
  }): Promise<void> {
    const { error } = await sendViaApexMail({
      from: this.FOUNDER_EMAIL,
      to: data.to,
      subject: '🎁 Earn $500 for Every Developer You Refer!',
      html: `
        <h1>🎁 Join Our Referral Program!</h1>
        <p>Thanks for being an awesome ApexMediation customer! We'd love your help spreading the word.</p>
        
        <h2>Your Referral Code</h2>
        <div style="background: #f4f4f4; padding: 20px; text-align: center; border-radius: 8px;">
          <code style="font-size: 24px; font-weight: bold; font-family: monospace;">
            ${data.referral_code}
          </code>
        </div>
        
        <h2>How It Works</h2>
        <ol>
          <li>Share your referral code with fellow developers</li>
          <li>They sign up and mention your code</li>
          <li>After their first paid month, you get <strong>${data.reward_amount}</strong> credit</li>
          <li>They get <strong>$100 off</strong> their first month</li>
        </ol>
        
        <h2>Quick Share Links</h2>
        <p>
          <a href="https://apexmediation.ee/signup?ref=${data.referral_code}" style="background: #4CAF50; color: white; padding: 10px 20px; text-decoration: none; border-radius: 4px; display: inline-block; margin: 5px;">📋 Copy Signup Link</a>
        </p>
        <p style="font-size: 14px; color: #666;">
          Share on: 
          <a href="https://twitter.com/intent/tweet?text=Just%20switched%20to%20ApexMediation%20for%20ad%20mediation%20-%2010min%20setup%2C%20only%2010%25%20take%20rate!%20Use%20code%20${data.referral_code}%20for%20%24100%20off.%20https%3A%2F%2Fapexmediation.ee%2Fsignup%3Fref%3D${data.referral_code}">Twitter</a> | 
          <a href="https://www.linkedin.com/sharing/share-offsite/?url=https%3A%2F%2Fapexmediation.ee%2Fsignup%3Fref%3D${data.referral_code}">LinkedIn</a> | 
          <a href="mailto:?subject=Check%20out%20ApexMediation&body=I%27ve%20been%20using%20ApexMediation%20for%20ad%20mediation%20and%20love%20it.%20Use%20my%20code%20${data.referral_code}%20for%20%24100%20off%3A%20https%3A%2F%2Fapexmediation.ee%2Fsignup%3Fref%3D${data.referral_code}">Email</a>
        </p>
        
        <p><strong>No limits.</strong> Refer as many developers as you want!</p>
        
        <p>Questions? Just reply to this email.</p>
        
        <p>Best regards,<br>
        Sabel Akhoua<br>
        Founder, ApexMediation</p>
      `,
    });

    if (error) {
      console.error(`[Email] Error sending referral invite to ${data.to}:`, error);
      throw error;
    }

    console.log(`[Email] Sent referral invite to ${data.to} (${data.referral_code})`);
  }

  /**
   * Testimonial request email (90+ days, NPS >=9)
   */
  private async sendTestimonialRequestEmail(data: {
    to: string;
    customer_id: string;
    nps_score: number;
    incentive: string;
  }): Promise<void> {
    const { error } = await sendViaApexMail({
      from: this.FOUNDER_EMAIL,
      to: data.to,
      subject: '⭐ Would You Share Your ApexMediation Experience?',
      html: `
        <h1>⭐ We'd Love Your Feedback!</h1>
        <p>You've been using ApexMediation for a while now, and based on your recent feedback (thank you for the ${data.nps_score}/10!), it seems you're happy with the service.</p>
        
        <p>Would you be willing to share your experience in a short testimonial?</p>
        
        <h2>What We're Looking For</h2>
        <ul>
          <li>Your role and company (or "solo developer" if solo)</li>
          <li>What problem ApexMediation solved for you</li>
          <li>Results you've seen (revenue, time saved, etc.)</li>
          <li>Why you chose us over alternatives</li>
        </ul>
        
        <h2>Your Reward</h2>
        <p>As a thank you, we'll give you <strong>${data.incentive}</strong> to your account.</p>
        
        <p style="text-align: center; margin: 30px 0;">
          <a href="https://apexmediation.ee/testimonials/submit?cid=${data.customer_id}" style="background: #4CAF50; color: white; padding: 15px 30px; text-decoration: none; border-radius: 4px; display: inline-block; font-weight: bold;">✍️ Submit Testimonial (2 minutes)</a>
        </p>
        
        <p><strong>Optional:</strong> Want to do a quick 5-minute video testimonial instead? Even better! <a href="https://cal.com/apexmediation/testimonial">Book a time here</a>.</p>
        
        <p>No pressure at all—but if you have 2 minutes, we'd really appreciate it!</p>
        
        <p>Best regards,<br>
        Sabel Akhoua<br>
        Founder, ApexMediation</p>
      `,
    });

    if (error) {
      console.error(`[Email] Error sending testimonial request to ${data.to}:`, error);
      throw error;
    }

    console.log(`[Email] Sent testimonial request to ${data.to} (NPS: ${data.nps_score})`);
  }

  /**
   * Case study invite email (1M+ impressions, high-value customers)
   */
  private async sendCaseStudyInviteEmail(data: {
    to: string;
    customer_id: string;
    total_impressions: number;
    days_active: number;
    benefits: string[];
  }): Promise<void> {
    const { error } = await sendViaApexMail({
      from: this.FOUNDER_EMAIL,
      to: data.to,
      subject: '📚 Feature Your Success Story?',
      html: `
        <h1>📚 Let's Showcase Your Success!</h1>
        <p>Congrats on <strong>${data.total_impressions.toLocaleString()} impressions</strong> in ${data.days_active} days! Your growth is impressive.</p>
        
        <p>Would you be interested in a <strong>case study</strong> highlighting your success with ApexMediation?</p>
        
        <h2>What's in It for You?</h2>
        <ul>
          ${data.benefits.map((benefit) => `<li>${benefit}</li>`).join('\n          ')}
        </ul>
        
        <h2>What We'll Cover</h2>
        <ul>
          <li>Your app/game and how you use ApexMediation</li>
          <li>Results you've achieved (revenue growth, integration speed, etc.)</li>
          <li>Why you chose us over Unity Ads, AdMob, etc.</li>
          <li>Tips for other developers</li>
        </ul>
        
        <p><strong>Time commitment:</strong> 30-minute interview (Zoom/phone) + quick review of the draft.</p>
        
        <p style="text-align: center; margin: 30px 0;">
          <a href="https://cal.com/apexmediation/case-study?cid=${data.customer_id}" style="background: #2196F3; color: white; padding: 15px 30px; text-decoration: none; border-radius: 4px; display: inline-block; font-weight: bold;">📅 Schedule Interview (30 min)</a>
        </p>
        
        <p>Not interested? No worries—just ignore this email.</p>
        
        <p>Best regards,<br>
        Sabel Akhoua<br>
        Founder, ApexMediation</p>
      `,
    });

    if (error) {
      console.error(`[Email] Error sending case study invite to ${data.to}:`, error);
      throw error;
    }

    console.log(`[Email] Sent case study invite to ${data.to} (${data.total_impressions.toLocaleString()} impressions)`);
  }

  /**
   * Community champion reward email (active contributors)
   */
  private async sendCommunityChampionRewardEmail(data: {
    to: string;
    customer_id: string;
    contributions_count: number;
    reward_amount_cents: number;
    badge_url: string;
  }): Promise<void> {
    const { error } = await sendViaApexMail({
      from: this.FOUNDER_EMAIL,
      to: data.to,
      subject: '🏆 Community Champion Reward: $100 Credit!',
      html: `
        <h1>🏆 You're a Community Champion!</h1>
        <p>Thank you for being such an active and helpful member of the ApexMediation community!</p>
        
        <h2>Your Contributions</h2>
        <p>This month, you've made <strong>${data.contributions_count} contributions</strong> helping other developers in:</p>
        <ul>
          <li>GitHub Discussions</li>
          <li>Discord support channels</li>
          <li>Documentation improvements</li>
          <li>Bug reports</li>
        </ul>
        
        <h2>Your Reward</h2>
        <p>We've added <strong>$${(data.reward_amount_cents / 100).toFixed(2)} credit</strong> to your account. It'll automatically apply to your next invoice.</p>
        
        <div style="text-align: center; margin: 30px 0;">
          <img src="${data.badge_url}" alt="Community Champion Badge" style="max-width: 200px;" />
          <p style="font-weight: bold; margin-top: 10px;">Community Champion Badge</p>
        </div>
        
        <p>Feel free to share your badge on social media! 🎉</p>
        
        <h2>Leaderboard</h2>
        <p>Check out the <a href="https://apexmediation.ee/community/leaderboard">community leaderboard</a> to see where you rank!</p>
        
        <p>Keep up the amazing work—you're making ApexMediation better for everyone.</p>
        
        <p>Best regards,<br>
        Sabel Akhoua<br>
        Founder, ApexMediation</p>
      `,
    });

    if (error) {
      console.error(`[Email] Error sending community champion reward to ${data.to}:`, error);
      throw error;
    }

    console.log(`[Email] Sent community champion reward to ${data.to} ($${data.reward_amount_cents / 100})`);
  }
}

export const emailAutomationService = new EmailAutomationService();
