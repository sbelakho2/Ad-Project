// services/sales-automation/OutreachService.ts
// Manages automated email outreach sequences
// Uses Resend free tier (3K emails/month) - existing service

import { Pool } from 'pg';
import { Resend } from 'resend';
import crypto from 'crypto';

interface OutreachConfig {
  fromEmail: string;
  fromName: string;
  replyTo: string;
  dailyLimit: number;
  trackingDomain: string;
}

const DEFAULT_CONFIG: OutreachConfig = {
  fromEmail: 'sales@apexmediation.ee',
  fromName: 'Aaron from ApexMediation',
  replyTo: 'sales@apexmediation.ee',
  dailyLimit: 100, // Resend free tier is 3K/month ≈ 100/day
  trackingDomain: 'apexmediation.ee',
};

export class OutreachService {
  private pool: Pool;
  private resend: Resend | null = null;
  private config: OutreachConfig;

  constructor(databaseUrl: string, config?: Partial<OutreachConfig>) {
    this.pool = new Pool({ connectionString: databaseUrl });
    this.config = { ...DEFAULT_CONFIG, ...config };
    
    // Initialize Resend if API key is available
    const resendApiKey = process.env.RESEND_API_KEY;
    if (resendApiKey) {
      this.resend = new Resend(resendApiKey);
    } else {
      console.warn('[Outreach] RESEND_API_KEY not set - email sending disabled');
    }
  }

  /**
   * Run outreach cycle - sends scheduled emails
   */
  async runOutreachCycle(): Promise<{ sent: number; errors: number }> {
    console.log('[Outreach] Starting outreach cycle...');
    
    let sent = 0;
    let errors = 0;

    if (!this.resend) {
      console.log('[Outreach] Skipping - email service not configured');
      return { sent: 0, errors: 0 };
    }

    try {
      // 1. Check daily limit
      const todaySent = await this.getTodaysSentCount();
      const remaining = this.config.dailyLimit - todaySent;
      
      if (remaining <= 0) {
        console.log('[Outreach] Daily limit reached, skipping');
        return { sent: 0, errors: 0 };
      }

      // 2. Process pending emails (already scheduled)
      const pendingResult = await this.sendPendingEmails(Math.floor(remaining / 2));
      sent += pendingResult.sent;
      errors += pendingResult.errors;

      // 3. Advance enrollments to next step
      const advanceResult = await this.advanceEnrollments(Math.floor(remaining / 2));
      sent += advanceResult.sent;
      errors += advanceResult.errors;

      // 4. Enroll new qualified contacts
      await this.enrollQualifiedContacts();

      // 5. Process any bounces or replies
      await this.processEmailResponses();

      console.log(`[Outreach] Cycle complete: ${sent} sent, ${errors} errors`);
      return { sent, errors };
    } catch (error) {
      console.error('[Outreach] Error in outreach cycle:', error);
      throw error;
    }
  }

  /**
   * Send pending emails that are scheduled for now
   */
  private async sendPendingEmails(limit: number): Promise<{ sent: number; errors: number }> {
    let sent = 0;
    let errors = 0;

    const pendingEmails = await this.pool.query(`
      SELECT e.*, c.email as contact_email, c.first_name, c.last_name
      FROM outreach_emails e
      JOIN sales_contacts c ON e.contact_id = c.id
      WHERE e.status = 'pending'
        AND e.scheduled_for <= CURRENT_TIMESTAMP
        AND c.status NOT IN ('opted_out', 'bounced')
      ORDER BY e.scheduled_for
      LIMIT $1
    `, [limit]);

    for (const email of pendingEmails.rows) {
      try {
        const result = await this.sendEmail(email);
        
        if (result.success) {
          await this.pool.query(`
            UPDATE outreach_emails 
            SET status = 'sent', sent_at = CURRENT_TIMESTAMP, message_id = $2, updated_at = CURRENT_TIMESTAMP
            WHERE id = $1
          `, [email.id, result.messageId]);
          
          // Update enrollment
          await this.pool.query(`
            UPDATE outreach_enrollments 
            SET emails_sent = emails_sent + 1, updated_at = CURRENT_TIMESTAMP
            WHERE id = $1
          `, [email.enrollment_id]);
          
          // Log activity
          await this.logActivity(email.contact_id, 'email_sent', {
            email_id: email.id,
            subject: email.subject,
          });
          
          sent++;
        } else {
          errors++;
        }
        
        // Small delay between emails
        await this.delay(500);
      } catch (error) {
        console.error(`[Outreach] Error sending email ${email.id}:`, error);
        errors++;
      }
    }

    return { sent, errors };
  }

  /**
   * Advance enrollments to next step
   */
  private async advanceEnrollments(limit: number): Promise<{ sent: number; errors: number }> {
    let sent = 0;
    let errors = 0;

    // Get enrollments ready for next step
    const enrollments = await this.pool.query(`
      SELECT 
        e.*,
        c.email as contact_email,
        c.first_name,
        c.full_name,
        p.app_name,
        p.downloads_estimate,
        p.app_category,
        co.name as company_name,
        co.ad_networks
      FROM outreach_enrollments e
      JOIN sales_contacts c ON e.contact_id = c.id
      JOIN outreach_sequences s ON e.sequence_id = s.id
      LEFT JOIN sales_prospects p ON e.prospect_id = p.id
      LEFT JOIN sales_companies co ON c.company_id = co.id
      WHERE e.status = 'active'
        AND e.next_step_at <= CURRENT_TIMESTAMP
        AND c.status NOT IN ('opted_out', 'bounced')
      ORDER BY e.next_step_at
      LIMIT $1
    `, [limit]);

    for (const enrollment of enrollments.rows) {
      try {
        const nextStep = enrollment.current_step + 1;
        
        // Get the next step template
        const stepResult = await this.pool.query(`
          SELECT * FROM outreach_sequence_steps
          WHERE sequence_id = $1 AND step_number = $2
        `, [enrollment.sequence_id, nextStep]);
        
        if (!stepResult.rows[0]) {
          // No more steps - mark as completed
          await this.pool.query(`
            UPDATE outreach_enrollments 
            SET status = 'completed', completed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
            WHERE id = $1
          `, [enrollment.id]);
          continue;
        }
        
        const step = stepResult.rows[0];
        
        // Create personalized email
        const personalizationData = {
          first_name: enrollment.first_name || 'there',
          full_name: enrollment.full_name || enrollment.contact_email.split('@')[0],
          app_name: enrollment.app_name || 'your app',
          company_name: enrollment.company_name || enrollment.contact_email.split('@')[1],
          downloads: enrollment.downloads_estimate || 'growing',
          app_category: enrollment.app_category || 'your category',
          current_mediation: this.detectCurrentMediation(enrollment.ad_networks),
          estimated_revenue: this.estimateRevenue(enrollment.downloads_estimate),
          net_benefit: this.estimateNetBenefit(enrollment.downloads_estimate),
          pain_point: this.generatePainPoint(enrollment.ad_networks),
        };
        
        const subject = this.personalizeContent(step.subject_template, personalizationData);
        const bodyHtml = this.personalizeContent(step.body_template, personalizationData);
        
        // LINT: Catch any unresolved {{placeholders}} before sending
        // This prevents emails with literal "{{variable}}" from reaching recipients
        const subjectUnresolved = this.findUnresolvedPlaceholders(subject);
        const bodyUnresolved = this.findUnresolvedPlaceholders(bodyHtml);
        
        if (subjectUnresolved.length > 0 || bodyUnresolved.length > 0) {
          const allUnresolved = [...new Set([...subjectUnresolved, ...bodyUnresolved])];
          console.warn(`[Outreach] Template lint FAILED for enrollment ${enrollment.id}: unresolved placeholders: ${allUnresolved.join(', ')}`);
          
          // Mark enrollment as errored to prevent future send attempts
          await this.pool.query(`
            UPDATE outreach_enrollments 
            SET status = 'error', 
                updated_at = CURRENT_TIMESTAMP,
                error_message = $2
            WHERE id = $1
          `, [enrollment.id, `Unresolved placeholders: ${allUnresolved.join(', ')}`]);
          
          errors++;
          continue;
        }
        
        const bodyText = this.stripHtml(bodyHtml);
        
        // Add tracking pixel
        const trackingPixelId = crypto.randomUUID();
        const trackedBodyHtml = this.addTrackingPixel(bodyHtml, trackingPixelId);
        
        // Add unsubscribe link
        const finalBodyHtml = this.addUnsubscribeLink(trackedBodyHtml, enrollment.contact_id);
        
        // Schedule the email
        await this.pool.query(`
          INSERT INTO outreach_emails (
            enrollment_id, step_id, contact_id,
            to_email, from_email, subject, body_html, body_text,
            tracking_pixel_id, scheduled_for, status
          ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, CURRENT_TIMESTAMP, 'pending')
        `, [
          enrollment.id,
          step.id,
          enrollment.contact_id,
          enrollment.contact_email,
          this.config.fromEmail,
          subject,
          finalBodyHtml,
          bodyText,
          trackingPixelId,
        ]);
        
        // Calculate next step time
        const nextStepDelay = await this.getNextStepDelay(enrollment.sequence_id, nextStep + 1);
        
        // Update enrollment
        await this.pool.query(`
          UPDATE outreach_enrollments 
          SET current_step = $2,
              next_step_at = $3,
              updated_at = CURRENT_TIMESTAMP
          WHERE id = $1
        `, [
          enrollment.id,
          nextStep,
          nextStepDelay ? new Date(Date.now() + nextStepDelay * 24 * 60 * 60 * 1000) : null,
        ]);
        
        sent++;
      } catch (error) {
        console.error(`[Outreach] Error advancing enrollment ${enrollment.id}:`, error);
        errors++;
      }
    }

    return { sent, errors };
  }

  /**
   * Enroll qualified contacts into sequences
   */
  async enrollQualifiedContacts(): Promise<number> {
    // Get qualified contacts not yet enrolled
    const contacts = await this.pool.query(`
      SELECT 
        c.*,
        p.app_category,
        p.downloads_estimate,
        p.app_name,
        co.ad_networks
      FROM sales_contacts c
      JOIN sales_prospects p ON c.prospect_id = p.id
      LEFT JOIN sales_companies co ON c.company_id = co.id
      WHERE p.status = 'qualified'
        AND c.status = 'discovered'
        AND c.email_verified IS NOT FALSE
        AND NOT EXISTS (
          SELECT 1 FROM outreach_enrollments oe 
          WHERE oe.contact_id = c.id
        )
      ORDER BY p.fit_score DESC, p.intent_score DESC
      LIMIT 50
    `);

    let enrolled = 0;

    for (const contact of contacts.rows) {
      // Find best matching sequence
      const sequence = await this.findBestSequence(contact);
      
      if (sequence) {
        // Get first step delay
        const firstStepDelay = await this.getNextStepDelay(sequence.id, 1);
        
        // Create enrollment
        await this.pool.query(`
          INSERT INTO outreach_enrollments (
            contact_id, sequence_id, prospect_id,
            status, current_step, next_step_at,
            personalization_data
          ) VALUES ($1, $2, $3, 'active', 0, $4, $5)
        `, [
          contact.id,
          sequence.id,
          contact.prospect_id,
          new Date(Date.now() + (firstStepDelay || 0) * 24 * 60 * 60 * 1000),
          JSON.stringify({
            app_name: contact.app_name,
            downloads: contact.downloads_estimate,
            category: contact.app_category,
            ad_networks: contact.ad_networks,
          }),
        ]);
        
        // Update contact status
        await this.pool.query(`
          UPDATE sales_contacts SET status = 'contacted', updated_at = CURRENT_TIMESTAMP
          WHERE id = $1
        `, [contact.id]);
        
        // Update prospect status
        await this.pool.query(`
          UPDATE sales_prospects SET status = 'contacted', updated_at = CURRENT_TIMESTAMP
          WHERE id = $1
        `, [contact.prospect_id]);
        
        enrolled++;
      }
    }

    console.log(`[Outreach] Enrolled ${enrolled} new contacts`);
    return enrolled;
  }

  /**
   * Find the best sequence for a contact
   */
  private async findBestSequence(contact: any): Promise<{ id: string } | null> {
    // Match by category
    const category = (contact.app_category || '').toLowerCase();
    
    let targetSegment = 'all';
    if (category.includes('game')) targetSegment = 'gaming';
    else if (category.includes('entertainment')) targetSegment = 'entertainment';
    else if (category.includes('tool') || category.includes('util')) targetSegment = 'utility';
    
    const sequence = await this.pool.query(`
      SELECT id FROM outreach_sequences
      WHERE active = true
        AND (target_segment = $1 OR target_segment = 'all')
      ORDER BY 
        CASE WHEN target_segment = $1 THEN 0 ELSE 1 END,
        total_converted DESC
      LIMIT 1
    `, [targetSegment]);
    
    return sequence.rows[0] || null;
  }

  /**
   * Get delay for a step in days
   */
  private async getNextStepDelay(sequenceId: string, stepNumber: number): Promise<number | null> {
    const step = await this.pool.query(`
      SELECT delay_days FROM outreach_sequence_steps
      WHERE sequence_id = $1 AND step_number = $2
    `, [sequenceId, stepNumber]);
    
    return step.rows[0]?.delay_days ?? null;
  }

  /**
   * Send an individual email via Resend
   */
  private async sendEmail(email: any): Promise<{ success: boolean; messageId?: string }> {
    if (!this.resend) {
      return { success: false };
    }

    try {
      const result = await this.resend.emails.send({
        from: `${this.config.fromName} <${this.config.fromEmail}>`,
        to: email.to_email,
        subject: email.subject,
        html: email.body_html,
        text: email.body_text,
        replyTo: this.config.replyTo,
        headers: {
          'X-Entity-Ref-ID': email.id,
          'List-Unsubscribe': `<mailto:sales@apexmediation.ee?subject=unsubscribe-${email.contact_id}>`,
        },
      });

      if (result.error) {
        console.error('[Outreach] Resend error:', result.error);
        return { success: false };
      }

      return { success: true, messageId: result.data?.id };
    } catch (error) {
      console.error('[Outreach] Send error:', error);
      return { success: false };
    }
  }

  /**
   * Process email responses (bounces, replies)
   */
  private async processEmailResponses(): Promise<void> {
    // This would typically be done via Resend webhooks
    // For now, check for any bounced emails
    await this.pool.query(`
      UPDATE sales_contacts 
      SET status = 'bounced', updated_at = CURRENT_TIMESTAMP
      WHERE id IN (
        SELECT DISTINCT contact_id FROM outreach_emails
        WHERE status = 'bounced' AND bounced_at > CURRENT_TIMESTAMP - INTERVAL '1 hour'
      )
    `);
    
    // Pause enrollments for bounced contacts
    await this.pool.query(`
      UPDATE outreach_enrollments 
      SET status = 'bounced', updated_at = CURRENT_TIMESTAMP
      WHERE contact_id IN (
        SELECT id FROM sales_contacts WHERE status = 'bounced'
      ) AND status = 'active'
    `);
  }

  /**
   * Get count of emails sent today
   */
  private async getTodaysSentCount(): Promise<number> {
    const result = await this.pool.query(`
      SELECT COUNT(*) as count FROM outreach_emails
      WHERE sent_at >= CURRENT_DATE
        AND status IN ('sent', 'delivered', 'opened', 'clicked')
    `);
    return parseInt(result.rows[0]?.count || '0', 10);
  }

  /**
   * Find unresolved placeholders in a string
   * Returns array of placeholder names that weren't replaced (e.g., ["app_name", "estimated_revenue"])
   * This is the LINT step that prevents sending broken emails
   */
  private findUnresolvedPlaceholders(content: string): string[] {
    const unresolvedPattern = /\{\{([a-zA-Z_][a-zA-Z0-9_]*)\}\}/g;
    const matches: string[] = [];
    let match: RegExpExecArray | null;
    
    while ((match = unresolvedPattern.exec(content)) !== null) {
      matches.push(match[1]);
    }
    
    return matches;
  }

  /**
   * Personalize content with variables
   */
  private personalizeContent(template: string, data: Record<string, any>): string {
    let content = template;
    
    for (const [key, value] of Object.entries(data)) {
      const regex = new RegExp(`\\{\\{${key}\\}\\}`, 'g');
      content = content.replace(regex, value?.toString() || '');
    }
    
    return content;
  }

  /**
   * Add tracking pixel to HTML
   */
  private addTrackingPixel(html: string, pixelId: string): string {
    const pixel = `<img src="https://${this.config.trackingDomain}/track/open/${pixelId}" width="1" height="1" alt="" style="display:none" />`;
    
    // Add before closing body tag
    if (html.includes('</body>')) {
      return html.replace('</body>', `${pixel}</body>`);
    }
    
    return html + pixel;
  }

  /**
   * Add unsubscribe link
   */
  private addUnsubscribeLink(html: string, contactId: string): string {
    const unsubscribeLink = `
      <p style="margin-top: 30px; padding-top: 20px; border-top: 1px solid #eee; font-size: 11px; color: #999; text-align: center;">
        If you no longer wish to receive these emails, 
        <a href="https://${this.config.trackingDomain}/unsubscribe/${contactId}" style="color: #666;">click here to unsubscribe</a>.
      </p>
    `;
    
    if (html.includes('</body>')) {
      return html.replace('</body>', `${unsubscribeLink}</body>`);
    }
    
    return html + unsubscribeLink;
  }

  /**
   * Strip HTML for plain text version
   */
  private stripHtml(html: string): string {
    return html
      .replace(/<style[^>]*>.*?<\/style>/gis, '')
      .replace(/<script[^>]*>.*?<\/script>/gis, '')
      .replace(/<[^>]+>/g, '')
      .replace(/&nbsp;/g, ' ')
      .replace(/&amp;/g, '&')
      .replace(/&lt;/g, '<')
      .replace(/&gt;/g, '>')
      .replace(/\s+/g, ' ')
      .trim();
  }

  /**
   * Detect current mediation platform from ad networks
   */
  private detectCurrentMediation(adNetworks: string[] | null): string {
    if (!adNetworks || adNetworks.length === 0) return 'your current platform';
    
    const networks = adNetworks.join(' ').toLowerCase();
    
    if (networks.includes('ironsource') || networks.includes('levelplay')) return 'ironSource/LevelPlay';
    if (networks.includes('max') || networks.includes('applovin')) return 'AppLovin MAX';
    if (networks.includes('admob')) return 'AdMob';
    if (networks.includes('mopub')) return 'MoPub';
    if (networks.includes('unity')) return 'Unity Ads';
    
    return 'your current mediation';
  }

  /**
   * Estimate monthly ad revenue based on downloads
   */
  private estimateRevenue(downloadsEstimate: string | null): string {
    if (!downloadsEstimate) return '$X,XXX';
    
    // Very rough estimates based on industry averages
    const estimates: Record<string, string> = {
      '1K-10K': '$100-$500',
      '10K-50K': '$500-$2,500',
      '50K-100K': '$2,500-$5,000',
      '100K-500K': '$5,000-$25,000',
      '500K-1M': '$25,000-$50,000',
      '1M-5M': '$50,000-$250,000',
      '5M-10M': '$250,000-$500,000',
      '10M-50M': '$500,000+',
    };
    
    return estimates[downloadsEstimate] || '$X,XXX';
  }

  /**
   * Estimate net benefit from switching
   */
  private estimateNetBenefit(downloadsEstimate: string | null): string {
    if (!downloadsEstimate) return '$XXX';
    
    // 15% improvement assumption, minus 2.5% fee
    const estimates: Record<string, string> = {
      '1K-10K': '$12-$60',
      '10K-50K': '$60-$300',
      '50K-100K': '$300-$600',
      '100K-500K': '$600-$3,000',
      '500K-1M': '$3,000-$6,000',
      '1M-5M': '$6,000-$30,000',
      '5M-10M': '$30,000-$60,000',
      '10M-50M': '$60,000+',
    };
    
    return estimates[downloadsEstimate] || '$XXX';
  }

  /**
   * Generate pain point based on ad networks
   */
  private generatePainPoint(adNetworks: string[] | null): string {
    const painPoints = [
      'opaque auction results',
      'unexplained revenue fluctuations',
      'lack of transparency in bid data',
      'difficulty optimizing waterfall',
      "not knowing why certain ads win",
    ];
    
    // Random selection for variety
    return painPoints[Math.floor(Math.random() * painPoints.length)];
  }

  /**
   * Log sales activity
   */
  private async logActivity(contactId: string, type: string, metadata: any): Promise<void> {
    await this.pool.query(`
      INSERT INTO sales_activities (contact_id, activity_type, channel, metadata, is_automated)
      VALUES ($1, $2, 'email', $3, true)
    `, [contactId, type, JSON.stringify(metadata)]);
  }

  /**
   * Handle unsubscribe request
   */
  async handleUnsubscribe(contactId: string): Promise<void> {
    await this.pool.query(`
      UPDATE sales_contacts 
      SET status = 'opted_out', opted_out_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
      WHERE id = $1
    `, [contactId]);
    
    await this.pool.query(`
      UPDATE outreach_enrollments 
      SET status = 'opted_out', updated_at = CURRENT_TIMESTAMP
      WHERE contact_id = $1 AND status = 'active'
    `, [contactId]);
  }

  /**
   * Handle email open (called from tracking endpoint)
   */
  async trackOpen(trackingPixelId: string): Promise<void> {
    await this.pool.query(`
      UPDATE outreach_emails 
      SET status = CASE WHEN status = 'sent' THEN 'opened' ELSE status END,
          opened_at = COALESCE(opened_at, CURRENT_TIMESTAMP),
          opened_count = opened_count + 1,
          updated_at = CURRENT_TIMESTAMP
      WHERE tracking_pixel_id = $1
    `, [trackingPixelId]);
    
    // Update enrollment
    await this.pool.query(`
      UPDATE outreach_enrollments 
      SET emails_opened = emails_opened + 1, updated_at = CURRENT_TIMESTAMP
      WHERE id = (
        SELECT enrollment_id FROM outreach_emails WHERE tracking_pixel_id = $1
      )
    `);
  }

  /**
   * Delay helper
   */
  private delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  /**
   * Close database connection
   */
  async close(): Promise<void> {
    await this.pool.end();
  }
}
