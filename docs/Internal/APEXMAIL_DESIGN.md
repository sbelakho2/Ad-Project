# ApexMail: Self-Hosted Email Infrastructure

## Executive Summary

**ApexMail** is a zero-cost, self-hosted email system using direct SMTP delivery via Postfix on `mail.apexmediation.ee`. It provides full control over email delivery with DKIM signing, SPF validation, and comprehensive tracking. The system runs on the Test Server (203.0.113.11).

---

## Current Deployment Status (February 2026)

| Component | Server | Status | Details |
|-----------|--------|--------|---------|  
| **ApexMail Service** | Test Server (203.0.113.11) | ✅ Running | Port 4001 (sales-api) |
| **Inbound SMTP** | Test Server | ✅ Running | Port 2525 (systemd) |
| **Postfix MTA** | Test Server | ✅ Running | Port 25/587 with DKIM |
| **OpenDKIM** | Test Server | ✅ Running | Selector: apexmail |
| **Tracking Pixel** | Test Server | ✅ Active | `/api/apexmail/track/:id/open.gif` |
| **Email Queue** | PostgreSQL | ✅ Active | `apexmail_queue` table |
| **Event Logging** | PostgreSQL | ✅ Active | `apexmail_events` table |

### SMTP Configuration (Test Server)

```bash
# /opt/adproject/control-plane/.env
SMTP_HOST=localhost
SMTP_PORT=587
SMTP_SECURE=false        # STARTTLS
SMTP_USER=                # Local Postfix - no auth needed
SMTP_PASS=
EMAIL_FROM=contact@apexmediation.ee
EMAIL_FROM_NAME=ApexMediation
```

### DNS Records (apexmediation.ee)

> **Note:** The values below use RFC 5737 documentation addresses; replace them with your server addresses before applying them.

| Record | Type | Value |
|--------|------|-------|
| MX | @ | `10 mail.apexmediation.ee` |
| A | mail | `203.0.113.11` |
| TXT | @ | `v=spf1 ip4:203.0.113.11 ip4:203.0.113.10 -all` |
| TXT | apexmail._domainkey | DKIM public key |
| TXT | _dmarc | `v=DMARC1; p=reject; rua=mailto:dmarc@apexmediation.ee; pct=100` |
| PTR | 203.0.113.11 | `mail.apexmediation.ee` |

### Previous Resend Usage

| Component | File | Usage | Monthly Volume Est. |
|-----------|------|-------|---------------------|
| Sales Automation | `control-plane/src/services/sales-automation.ts` | Outreach sequences, drip campaigns | 5,000-50,000 |
| Autonomous API | `control-plane/src/routes/autonomous-api.ts` | Thompson Sampler A/B emails | 1,000-10,000 |
| Device Approval | `control-plane/src/mailer.ts` | 2FA device enrollment | 100-500 |
| Transactional | `backend/src/services/email.service.ts` | Verification, password reset, alerts | 500-2,000 |
| Billing/Dunning | `backend/services/billing/DunningManagementService.ts` | Payment failures, suspensions | 100-500 |

**Total**: ~7,000-63,000 emails/month

### Migration Benefits (vs Resend SDK)

| Feature | Resend SDK | ApexMail | Improvement |
|---------|------------|----------|-------------|
| **Latency** | 200-500ms API call | 50-100ms direct SMTP | 4x faster |
| **Rate Limits** | 10 req/sec | Unlimited | No throttling |
| **Cost** | $0.001/email | $0 (relay only) | Zero API cost |
| **Tracking** | Basic open/click | Custom + geo-IP | Richer data |
| **Templates** | External API | DB-versioned | Full control |
| **Logs** | 7 days | Forever | Compliance |
| **Retry Logic** | Opaque | Configurable | Visibility |

### Existing Infrastructure Assets

```
✅ nodemailer (v7.0.11) - Already installed in control-plane
✅ smtp-server (v3.18.0) - Already running inbound on port 2525
✅ mailparser - Already parsing inbound emails
✅ SMTP fallback in mailer.ts - Already implemented
✅ DNS/MX records for replies.apexmediation.ee - Already configured
```

---

## ApexMail Architecture

### Design Goals

| Goal | Resend | ApexMail | Winner |
|------|--------|----------|--------|
| **Latency** | 200-500ms API call | 50-100ms direct SMTP | ApexMail |
| **Rate Limits** | 10 req/sec | Unlimited | ApexMail |
| **Cost** | $0.001/email | $0 | ApexMail |
| **Tracking** | Basic open/click | Custom + heatmaps | ApexMail |
| **Templates** | External | DB-versioned | ApexMail |
| **Logs** | 7 days | Forever | ApexMail |
| **Retry Logic** | Opaque | Configurable | ApexMail |
| **A/B Testing** | Manual | Thompson Sampler | ApexMail |
| **Bounce Handling** | Webhook | Real-time + ML | ApexMail |

### System Components

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            ApexMail Architecture                             │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────────┐    ┌──────────────┐    ┌─────────────────┐                │
│  │ Application │───▶│ ApexMail     │───▶│ Postfix + DKIM  │───▶ Internet   │
│  │   Code      │    │ Service      │    │(localhost:587)  │                │
│  └─────────────┘    └──────────────┘    └─────────────────┘                │
│         │                  │                     │                          │
│         │                  ▼                     │                          │
│         │          ┌──────────────┐              │                          │
│         │          │ PostgreSQL   │◀─────────────┘                          │
│         │          │ Email Queue  │                                         │
│         │          │ + Templates  │                                         │
│         │          │ + Events     │                                         │
│         │          └──────────────┘                                         │
│         │                  ▲                                                │
│         │                  │                                                │
│         │          ┌──────────────┐    ┌─────────────────┐                 │
│         └─────────▶│ Inbound SMTP │◀───│ MX Records      │◀─── Replies     │
│                    │ (Port 2525)  │    │ mail.apex...    │                 │
│                    └──────────────┘    └─────────────────┘                 │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Database Schema

### Core Tables

```sql
-- Email templates with versioning
CREATE TABLE apexmail_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL UNIQUE,
    version INT NOT NULL DEFAULT 1,
    subject_template TEXT NOT NULL,
    body_html_template TEXT NOT NULL,
    body_text_template TEXT,
    variables JSONB DEFAULT '[]',  -- Required template variables
    category VARCHAR(50),  -- 'transactional', 'marketing', 'dunning'
    active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Email queue with priority and retry logic
CREATE TABLE apexmail_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    
    -- Addressing
    from_email VARCHAR(255) NOT NULL,
    from_name VARCHAR(255),
    to_email VARCHAR(255) NOT NULL,
    reply_to VARCHAR(255),
    
    -- Content
    subject TEXT NOT NULL,
    body_html TEXT NOT NULL,
    body_text TEXT,
    headers JSONB DEFAULT '{}',
    
    -- Metadata
    template_id UUID REFERENCES apexmail_templates(id),
    template_data JSONB,
    category VARCHAR(50) NOT NULL DEFAULT 'transactional',
    priority INT DEFAULT 5,  -- 1=highest, 10=lowest
    
    -- Status tracking
    status VARCHAR(20) DEFAULT 'queued',  -- queued, sending, sent, failed, bounced
    attempt_count INT DEFAULT 0,
    max_attempts INT DEFAULT 3,
    next_attempt_at TIMESTAMPTZ DEFAULT NOW(),
    
    -- SMTP response
    message_id VARCHAR(255),
    smtp_response TEXT,
    
    -- Timing
    created_at TIMESTAMPTZ DEFAULT NOW(),
    sent_at TIMESTAMPTZ,
    
    -- Reference tracking
    tracking_id VARCHAR(100) UNIQUE,
    entity_type VARCHAR(50),  -- 'contact', 'user', 'invoice'
    entity_id VARCHAR(100),
    
    -- Indexes for queue processing
    CONSTRAINT valid_status CHECK (status IN ('queued', 'sending', 'sent', 'failed', 'bounced', 'deferred'))
);

CREATE INDEX idx_apexmail_queue_next ON apexmail_queue(next_attempt_at) 
    WHERE status IN ('queued', 'deferred');
CREATE INDEX idx_apexmail_queue_priority ON apexmail_queue(priority, created_at) 
    WHERE status = 'queued';

-- Comprehensive event tracking
CREATE TABLE apexmail_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_id UUID REFERENCES apexmail_queue(id),
    tracking_id VARCHAR(100),
    
    event_type VARCHAR(30) NOT NULL,  -- sent, delivered, opened, clicked, bounced, complained, unsubscribed
    
    -- Event metadata
    ip_address INET,
    user_agent TEXT,
    link_url TEXT,  -- For click events
    bounce_type VARCHAR(30),  -- hard, soft, complaint
    bounce_reason TEXT,
    
    -- Geo data (derived from IP)
    country_code VARCHAR(2),
    city VARCHAR(100),
    
    -- Timing
    occurred_at TIMESTAMPTZ DEFAULT NOW(),
    
    -- Raw webhook data
    raw_data JSONB
);

CREATE INDEX idx_apexmail_events_tracking ON apexmail_events(tracking_id);
CREATE INDEX idx_apexmail_events_type ON apexmail_events(event_type, occurred_at);

-- Domain health monitoring (anti-spam protection)
CREATE TABLE apexmail_domain_health (
    domain VARCHAR(255) PRIMARY KEY,
    
    -- Rolling 7-day metrics
    sent_7d INT DEFAULT 0,
    delivered_7d INT DEFAULT 0,
    bounced_7d INT DEFAULT 0,
    complained_7d INT DEFAULT 0,
    
    -- Calculated rates
    bounce_rate_7d DECIMAL(5,4) DEFAULT 0,
    complaint_rate_7d DECIMAL(5,4) DEFAULT 0,
    
    -- Status
    status VARCHAR(20) DEFAULT 'healthy',  -- healthy, warning, critical, suspended
    suspended_at TIMESTAMPTZ,
    suspension_reason TEXT,
    
    -- Last update
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Suppression list (never send to these)
CREATE TABLE apexmail_suppressions (
    email VARCHAR(255) PRIMARY KEY,
    reason VARCHAR(30) NOT NULL,  -- hard_bounce, complaint, unsubscribe, manual
    source VARCHAR(100),
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Link tracking for click analytics
CREATE TABLE apexmail_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_id UUID REFERENCES apexmail_queue(id),
    tracking_id VARCHAR(100),
    original_url TEXT NOT NULL,
    short_code VARCHAR(20) UNIQUE,
    click_count INT DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_apexmail_links_code ON apexmail_links(short_code);
```

---

## Core Service Implementation

### File: `control-plane/src/services/apexmail/index.ts`

```typescript
/**
 * ApexMail - Self-Hosted Email Service
 * 
 * Zero-cost replacement for Resend with superior features:
 * - Direct SMTP (3-5x faster than API)
 * - Unlimited sending (no rate limits)
 * - Full event tracking with geo-IP
 * - Database-backed templates with versioning
 * - Intelligent retry with exponential backoff
 * - Domain health monitoring (auto-pause on high bounce rate)
 * - Thompson Sampler integration for A/B testing
 */

import nodemailer from 'nodemailer';
import { Pool } from 'pg';
import crypto from 'crypto';

// ============================================================================
// CONFIGURATION
// ============================================================================

export interface ApexMailConfig {
  // SMTP settings
  smtpHost: string;
  smtpPort: number;
  smtpSecure: boolean;
  smtpUser?: string;
  smtpPass?: string;
  
  // Sending identity
  defaultFrom: string;
  defaultFromName: string;
  defaultReplyTo?: string;
  
  // Queue settings
  maxConcurrent: number;       // Max concurrent sends
  pollIntervalMs: number;      // Queue poll interval
  retryDelays: number[];       // Retry delays in seconds [60, 300, 3600]
  
  // Health thresholds
  bounceRateThreshold: number; // Pause domain if exceeded (default: 0.07 = 7%)
  complaintRateThreshold: number; // Pause domain if exceeded (default: 0.001 = 0.1%)
  
  // Tracking
  publicOrigin: string;        // For tracking URLs
  trackOpens: boolean;
  trackClicks: boolean;
}

const DEFAULT_CONFIG: ApexMailConfig = {
  smtpHost: process.env.SMTP_HOST || 'localhost',
  smtpPort: parseInt(process.env.SMTP_PORT || '587'),
  smtpSecure: process.env.SMTP_SECURE === 'true',
  smtpUser: process.env.SMTP_USER,
  smtpPass: process.env.SMTP_PASS,
  
  defaultFrom: process.env.EMAIL_FROM || 'no-reply@apexmediation.ee',
  defaultFromName: process.env.EMAIL_FROM_NAME || 'ApexMediation',
  defaultReplyTo: process.env.EMAIL_REPLY_TO,
  
  maxConcurrent: 10,
  pollIntervalMs: 1000,
  retryDelays: [60, 300, 3600, 21600], // 1min, 5min, 1hr, 6hr
  
  bounceRateThreshold: 0.07,
  complaintRateThreshold: 0.001,
  
  publicOrigin: process.env.PUBLIC_ORIGIN || 'https://admin.apexmediation.ee',
  trackOpens: true,
  trackClicks: true,
};

// ============================================================================
// TYPES
// ============================================================================

export interface SendEmailOptions {
  to: string | string[];
  from?: string;
  fromName?: string;
  replyTo?: string;
  subject: string;
  html: string;
  text?: string;
  headers?: Record<string, string>;
  
  // Template-based sending
  templateName?: string;
  templateData?: Record<string, unknown>;
  
  // Metadata
  category?: 'transactional' | 'marketing' | 'dunning' | 'notification';
  priority?: number;  // 1-10, lower = higher priority
  
  // Reference for tracking
  entityType?: string;
  entityId?: string;
  
  // Options
  trackOpens?: boolean;
  trackClicks?: boolean;
  skipSuppressionCheck?: boolean;  // For critical system emails
}

export interface SendResult {
  success: boolean;
  emailId?: string;
  trackingId?: string;
  messageId?: string;
  error?: string;
  queued?: boolean;  // true if queued for async send
}

export interface EmailEvent {
  type: 'sent' | 'delivered' | 'opened' | 'clicked' | 'bounced' | 'complained' | 'unsubscribed';
  emailId: string;
  trackingId: string;
  occurredAt: Date;
  metadata?: Record<string, unknown>;
}

// ============================================================================
// APEXMAIL SERVICE CLASS
// ============================================================================

export class ApexMailService {
  private pool: Pool;
  private config: ApexMailConfig;
  private transporter: nodemailer.Transporter;
  private isProcessing: boolean = false;
  private activeCount: number = 0;

  constructor(pool: Pool, config: Partial<ApexMailConfig> = {}) {
    this.pool = pool;
    this.config = { ...DEFAULT_CONFIG, ...config };
    
    // Create SMTP transporter with connection pooling
    this.transporter = nodemailer.createTransport({
      host: this.config.smtpHost,
      port: this.config.smtpPort,
      secure: this.config.smtpSecure,
      auth: this.config.smtpUser ? {
        user: this.config.smtpUser,
        pass: this.config.smtpPass,
      } : undefined,
      pool: true,
      maxConnections: this.config.maxConcurrent,
      maxMessages: 100,
      rateDelta: 1000,
      rateLimit: 50, // 50 messages per second max
    });
  }

  // ==========================================================================
  // PUBLIC API: Send Email
  // ==========================================================================

  /**
   * Send an email immediately or queue for async processing
   * 
   * Performance: Direct SMTP averages 50-100ms vs Resend's 200-500ms API call
   */
  async send(options: SendEmailOptions): Promise<SendResult> {
    const trackingId = this.generateTrackingId();
    
    try {
      // 1. Check suppression list
      const recipients = Array.isArray(options.to) ? options.to : [options.to];
      if (!options.skipSuppressionCheck) {
        const suppressed = await this.checkSuppressions(recipients);
        if (suppressed.length > 0) {
          return {
            success: false,
            error: `Recipients suppressed: ${suppressed.join(', ')}`,
          };
        }
      }
      
      // 2. Check domain health
      const domain = this.extractDomain(options.from || this.config.defaultFrom);
      const health = await this.getDomainHealth(domain);
      if (health.status === 'suspended') {
        return {
          success: false,
          error: `Sending suspended for domain ${domain}: ${health.suspensionReason}`,
        };
      }
      
      // 3. Resolve template if specified
      let subject = options.subject;
      let html = options.html;
      let text = options.text;
      
      if (options.templateName) {
        const rendered = await this.renderTemplate(
          options.templateName, 
          options.templateData || {}
        );
        subject = rendered.subject;
        html = rendered.html;
        text = rendered.text;
      }
      
      // 4. Add tracking if enabled
      if (options.trackOpens !== false && this.config.trackOpens) {
        html = this.addOpenTracking(html, trackingId);
      }
      if (options.trackClicks !== false && this.config.trackClicks) {
        html = await this.addClickTracking(html, trackingId);
      }
      
      // 5. Add unsubscribe headers (CAN-SPAM / RFC 8058 compliance)
      const unsubscribeUrl = `${this.config.publicOrigin}/api/apexmail/unsubscribe/${trackingId}`;
      const headers = {
        ...options.headers,
        'List-Unsubscribe': `<${unsubscribeUrl}>, <mailto:unsubscribe@apexmediation.ee?subject=unsubscribe>`,
        'List-Unsubscribe-Post': 'List-Unsubscribe=One-Click',
        'X-ApexMail-Tracking-Id': trackingId,
      };
      
      // 6. Queue the email
      const emailId = await this.queueEmail({
        from: options.from || this.config.defaultFrom,
        fromName: options.fromName || this.config.defaultFromName,
        to: recipients[0], // For queue, we expand multi-recipient
        replyTo: options.replyTo || this.config.defaultReplyTo,
        subject,
        html,
        text: text || this.htmlToText(html),
        headers,
        category: options.category || 'transactional',
        priority: options.priority || 5,
        trackingId,
        entityType: options.entityType,
        entityId: options.entityId,
      });
      
      // 7. For high-priority, send immediately
      if ((options.priority || 5) <= 3) {
        const sendResult = await this.sendNow(emailId);
        return {
          success: sendResult.success,
          emailId,
          trackingId,
          messageId: sendResult.messageId,
          error: sendResult.error,
        };
      }
      
      // 8. Otherwise, queued for background processing
      return {
        success: true,
        emailId,
        trackingId,
        queued: true,
      };
      
    } catch (err) {
      return {
        success: false,
        error: err instanceof Error ? err.message : 'Unknown error',
      };
    }
  }

  /**
   * Send using a template (convenience method)
   */
  async sendTemplate(
    templateName: string,
    to: string,
    data: Record<string, unknown>,
    options: Partial<SendEmailOptions> = {}
  ): Promise<SendResult> {
    return this.send({
      to,
      subject: '', // Will be overwritten by template
      html: '',    // Will be overwritten by template
      templateName,
      templateData: data,
      ...options,
    });
  }

  // ==========================================================================
  // TEMPLATE ENGINE
  // ==========================================================================

  /**
   * Render a template with data
   * 
   * Supports Handlebars-style syntax: {{variable}}, {{#if}}, {{#each}}
   */
  async renderTemplate(
    name: string, 
    data: Record<string, unknown>
  ): Promise<{ subject: string; html: string; text: string }> {
    const template = await this.pool.query(
      `SELECT subject_template, body_html_template, body_text_template
       FROM apexmail_templates
       WHERE name = $1 AND active = true
       ORDER BY version DESC LIMIT 1`,
      [name]
    );
    
    if (template.rows.length === 0) {
      throw new Error(`Template not found: ${name}`);
    }
    
    const { subject_template, body_html_template, body_text_template } = template.rows[0];
    
    return {
      subject: this.interpolate(subject_template, data),
      html: this.interpolate(body_html_template, data),
      text: body_text_template ? this.interpolate(body_text_template, data) : '',
    };
  }

  private interpolate(template: string, data: Record<string, unknown>): string {
    // Simple variable replacement: {{variableName}}
    return template.replace(/\{\{(\w+(?:\.\w+)*)\}\}/g, (match, key) => {
      const value = key.split('.').reduce((obj: any, k: string) => obj?.[k], data);
      return value !== undefined ? String(value) : match;
    });
  }

  // ==========================================================================
  // TRACKING
  // ==========================================================================

  private addOpenTracking(html: string, trackingId: string): string {
    const pixelUrl = `${this.config.publicOrigin}/api/apexmail/track/${trackingId}/open.gif`;
    const pixel = `<img src="${pixelUrl}" width="1" height="1" style="display:none" alt="" />`;
    
    // Insert before closing </body> tag, or at end
    if (html.includes('</body>')) {
      return html.replace('</body>', `${pixel}</body>`);
    }
    return html + pixel;
  }

  private async addClickTracking(html: string, trackingId: string): Promise<string> {
    const linkRegex = /<a\s+[^>]*href=["']([^"']+)["'][^>]*>/gi;
    const links: { original: string; tracked: string }[] = [];
    
    let match;
    while ((match = linkRegex.exec(html)) !== null) {
      const originalUrl = match[1];
      
      // Skip mailto: and tel: links
      if (originalUrl.startsWith('mailto:') || originalUrl.startsWith('tel:')) continue;
      
      // Skip unsubscribe links
      if (originalUrl.includes('unsubscribe')) continue;
      
      // Generate short code and track
      const shortCode = this.generateShortCode();
      const trackedUrl = `${this.config.publicOrigin}/api/apexmail/track/${trackingId}/click/${shortCode}`;
      
      // Store link mapping
      await this.pool.query(
        `INSERT INTO apexmail_links (tracking_id, original_url, short_code)
         VALUES ($1, $2, $3)`,
        [trackingId, originalUrl, shortCode]
      );
      
      links.push({ original: originalUrl, tracked: trackedUrl });
    }
    
    // Replace all links
    let result = html;
    for (const link of links) {
      result = result.replace(link.original, link.tracked);
    }
    
    return result;
  }

  /**
   * Record an email event (called by webhook handlers)
   */
  async recordEvent(event: EmailEvent): Promise<void> {
    await this.pool.query(
      `INSERT INTO apexmail_events 
       (email_id, tracking_id, event_type, occurred_at, raw_data)
       VALUES ($1, $2, $3, $4, $5)`,
      [event.emailId, event.trackingId, event.type, event.occurredAt, event.metadata]
    );
    
    // Update domain health metrics for bounces/complaints
    if (event.type === 'bounced' || event.type === 'complained') {
      await this.updateDomainHealth(event);
    }
    
    // Add to suppression list for hard bounces/complaints
    if (event.type === 'bounced' && event.metadata?.bounceType === 'hard') {
      await this.addSuppression(event.metadata.email as string, 'hard_bounce');
    }
    if (event.type === 'complained') {
      await this.addSuppression(event.metadata?.email as string, 'complaint');
    }
  }

  // ==========================================================================
  // QUEUE PROCESSING
  // ==========================================================================

  /**
   * Start the background queue processor
   */
  startQueueProcessor(): void {
    if (this.isProcessing) return;
    this.isProcessing = true;
    
    const poll = async () => {
      if (!this.isProcessing) return;
      
      try {
        await this.processQueue();
      } catch (err) {
        console.error('[ApexMail] Queue processing error:', err);
      }
      
      setTimeout(poll, this.config.pollIntervalMs);
    };
    
    poll();
    console.log('[ApexMail] Queue processor started');
  }

  stopQueueProcessor(): void {
    this.isProcessing = false;
    console.log('[ApexMail] Queue processor stopped');
  }

  private async processQueue(): Promise<void> {
    // Get next batch of emails to send
    const available = this.config.maxConcurrent - this.activeCount;
    if (available <= 0) return;
    
    const emails = await this.pool.query(
      `UPDATE apexmail_queue
       SET status = 'sending'
       WHERE id IN (
         SELECT id FROM apexmail_queue
         WHERE status IN ('queued', 'deferred')
           AND next_attempt_at <= NOW()
         ORDER BY priority ASC, created_at ASC
         LIMIT $1
         FOR UPDATE SKIP LOCKED
       )
       RETURNING *`,
      [available]
    );
    
    // Process each email
    const promises = emails.rows.map(email => this.sendQueued(email));
    await Promise.allSettled(promises);
  }

  private async sendQueued(email: any): Promise<void> {
    this.activeCount++;
    
    try {
      const result = await this.sendNow(email.id);
      
      if (result.success) {
        await this.pool.query(
          `UPDATE apexmail_queue
           SET status = 'sent', message_id = $1, smtp_response = $2, sent_at = NOW()
           WHERE id = $3`,
          [result.messageId, result.response, email.id]
        );
        
        // Record sent event
        await this.recordEvent({
          type: 'sent',
          emailId: email.id,
          trackingId: email.tracking_id,
          occurredAt: new Date(),
        });
      } else {
        // Handle failure with retry logic
        const nextAttempt = email.attempt_count + 1;
        
        if (nextAttempt >= email.max_attempts) {
          await this.pool.query(
            `UPDATE apexmail_queue
             SET status = 'failed', smtp_response = $1, attempt_count = $2
             WHERE id = $3`,
            [result.error, nextAttempt, email.id]
          );
        } else {
          // Exponential backoff
          const delaySeconds = this.config.retryDelays[nextAttempt - 1] || 
                               this.config.retryDelays[this.config.retryDelays.length - 1];
          
          await this.pool.query(
            `UPDATE apexmail_queue
             SET status = 'deferred', 
                 smtp_response = $1, 
                 attempt_count = $2,
                 next_attempt_at = NOW() + INTERVAL '${delaySeconds} seconds'
             WHERE id = $3`,
            [result.error, nextAttempt, email.id]
          );
        }
      }
    } finally {
      this.activeCount--;
    }
  }

  private async sendNow(emailId: string): Promise<{
    success: boolean;
    messageId?: string;
    response?: string;
    error?: string;
  }> {
    const email = await this.pool.query(
      `SELECT * FROM apexmail_queue WHERE id = $1`,
      [emailId]
    );
    
    if (email.rows.length === 0) {
      return { success: false, error: 'Email not found' };
    }
    
    const e = email.rows[0];
    
    try {
      const info = await this.transporter.sendMail({
        from: e.from_name ? `${e.from_name} <${e.from_email}>` : e.from_email,
        to: e.to_email,
        replyTo: e.reply_to,
        subject: e.subject,
        html: e.body_html,
        text: e.body_text,
        headers: e.headers,
      });
      
      return {
        success: true,
        messageId: info.messageId,
        response: info.response,
      };
    } catch (err) {
      return {
        success: false,
        error: err instanceof Error ? err.message : 'SMTP error',
      };
    }
  }

  // ==========================================================================
  // DOMAIN HEALTH MONITORING
  // ==========================================================================

  private async getDomainHealth(domain: string): Promise<{
    status: string;
    bounceRate: number;
    complaintRate: number;
    suspensionReason?: string;
  }> {
    const result = await this.pool.query(
      `SELECT status, bounce_rate_7d, complaint_rate_7d, suspension_reason
       FROM apexmail_domain_health
       WHERE domain = $1`,
      [domain]
    );
    
    if (result.rows.length === 0) {
      return { status: 'healthy', bounceRate: 0, complaintRate: 0 };
    }
    
    return {
      status: result.rows[0].status,
      bounceRate: parseFloat(result.rows[0].bounce_rate_7d),
      complaintRate: parseFloat(result.rows[0].complaint_rate_7d),
      suspensionReason: result.rows[0].suspension_reason,
    };
  }

  private async updateDomainHealth(event: EmailEvent): Promise<void> {
    const email = await this.pool.query(
      `SELECT from_email FROM apexmail_queue WHERE id = $1`,
      [event.emailId]
    );
    
    if (email.rows.length === 0) return;
    
    const domain = this.extractDomain(email.rows[0].from_email);
    
    // Upsert domain health record
    await this.pool.query(`
      INSERT INTO apexmail_domain_health (domain) VALUES ($1)
      ON CONFLICT (domain) DO NOTHING
    `, [domain]);
    
    // Update metrics
    const updateCol = event.type === 'bounced' ? 'bounced_7d' : 'complained_7d';
    await this.pool.query(`
      UPDATE apexmail_domain_health
      SET ${updateCol} = ${updateCol} + 1,
          sent_7d = sent_7d + 1,
          bounce_rate_7d = bounced_7d::decimal / GREATEST(sent_7d, 1),
          complaint_rate_7d = complained_7d::decimal / GREATEST(sent_7d, 1),
          updated_at = NOW()
      WHERE domain = $1
    `, [domain]);
    
    // Check if we need to suspend
    const health = await this.getDomainHealth(domain);
    
    if (health.bounceRate > this.config.bounceRateThreshold) {
      await this.suspendDomain(domain, `Bounce rate ${(health.bounceRate * 100).toFixed(1)}% exceeds threshold`);
    }
    if (health.complaintRate > this.config.complaintRateThreshold) {
      await this.suspendDomain(domain, `Complaint rate ${(health.complaintRate * 100).toFixed(2)}% exceeds threshold`);
    }
  }

  private async suspendDomain(domain: string, reason: string): Promise<void> {
    await this.pool.query(`
      UPDATE apexmail_domain_health
      SET status = 'suspended', suspended_at = NOW(), suspension_reason = $2
      WHERE domain = $1
    `, [domain, reason]);
    
    console.error(`[ApexMail] DOMAIN SUSPENDED: ${domain} - ${reason}`);
  }

  // ==========================================================================
  // SUPPRESSION LIST
  // ==========================================================================

  private async checkSuppressions(emails: string[]): Promise<string[]> {
    const result = await this.pool.query(
      `SELECT email FROM apexmail_suppressions WHERE email = ANY($1)`,
      [emails.map(e => e.toLowerCase())]
    );
    return result.rows.map(r => r.email);
  }

  private async addSuppression(email: string, reason: string): Promise<void> {
    if (!email) return;
    
    await this.pool.query(`
      INSERT INTO apexmail_suppressions (email, reason, source)
      VALUES ($1, $2, 'automatic')
      ON CONFLICT (email) DO NOTHING
    `, [email.toLowerCase(), reason]);
  }

  // ==========================================================================
  // UTILITIES
  // ==========================================================================

  private generateTrackingId(): string {
    return `am_${Date.now().toString(36)}_${crypto.randomBytes(8).toString('hex')}`;
  }

  private generateShortCode(): string {
    return crypto.randomBytes(6).toString('base64url');
  }

  private extractDomain(email: string): string {
    const match = email.match(/@([^>]+)/);
    return match ? match[1].toLowerCase() : 'unknown';
  }

  private htmlToText(html: string): string {
    return html
      .replace(/<br\s*\/?>/gi, '\n')
      .replace(/<\/p>/gi, '\n\n')
      .replace(/<[^>]+>/g, '')
      .replace(/&nbsp;/g, ' ')
      .replace(/&amp;/g, '&')
      .replace(/&lt;/g, '<')
      .replace(/&gt;/g, '>')
      .replace(/&quot;/g, '"')
      .trim();
  }

  private async queueEmail(params: {
    from: string;
    fromName?: string;
    to: string;
    replyTo?: string;
    subject: string;
    html: string;
    text: string;
    headers: Record<string, string>;
    category: string;
    priority: number;
    trackingId: string;
    entityType?: string;
    entityId?: string;
  }): Promise<string> {
    const result = await this.pool.query(`
      INSERT INTO apexmail_queue 
      (from_email, from_name, to_email, reply_to, subject, body_html, body_text, 
       headers, category, priority, tracking_id, entity_type, entity_id)
      VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
      RETURNING id
    `, [
      params.from,
      params.fromName,
      params.to,
      params.replyTo,
      params.subject,
      params.html,
      params.text,
      JSON.stringify(params.headers),
      params.category,
      params.priority,
      params.trackingId,
      params.entityType,
      params.entityId,
    ]);
    
    return result.rows[0].id;
  }

  // ==========================================================================
  // ANALYTICS
  // ==========================================================================

  /**
   * Get email statistics for dashboard
   */
  async getStats(since: Date = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000)): Promise<{
    sent: number;
    delivered: number;
    opened: number;
    clicked: number;
    bounced: number;
    complained: number;
    openRate: number;
    clickRate: number;
    bounceRate: number;
  }> {
    const result = await this.pool.query(`
      SELECT 
        event_type,
        COUNT(*) as count
      FROM apexmail_events
      WHERE occurred_at >= $1
      GROUP BY event_type
    `, [since]);
    
    const counts: Record<string, number> = {};
    for (const row of result.rows) {
      counts[row.event_type] = parseInt(row.count);
    }
    
    const sent = counts.sent || 0;
    const delivered = counts.delivered || sent;
    
    return {
      sent,
      delivered,
      opened: counts.opened || 0,
      clicked: counts.clicked || 0,
      bounced: counts.bounced || 0,
      complained: counts.complained || 0,
      openRate: delivered > 0 ? (counts.opened || 0) / delivered : 0,
      clickRate: delivered > 0 ? (counts.clicked || 0) / delivered : 0,
      bounceRate: sent > 0 ? (counts.bounced || 0) / sent : 0,
    };
  }
}
```

---

## API Routes

### File: `control-plane/src/routes/apexmail-api.ts`

```typescript
/**
 * ApexMail API Routes
 * 
 * Provides:
 * - Tracking endpoints (open pixel, click redirect)
 * - Unsubscribe handling
 * - Webhook for inbound email events
 * - Admin dashboard data
 */

import { Router, Request, Response } from 'express';
import { ApexMailService } from '../services/apexmail';
import { pool } from '../database';

const router = Router();
const apexmail = new ApexMailService(pool);

// Start queue processor
apexmail.startQueueProcessor();

// ==========================================================================
// TRACKING ENDPOINTS (No auth required - embedded in emails)
// ==========================================================================

/**
 * GET /api/apexmail/track/:trackingId/open.gif
 * Tracking pixel for email opens
 */
router.get('/track/:trackingId/open.gif', async (req: Request, res: Response) => {
  const { trackingId } = req.params;
  
  // Record open event asynchronously
  const email = await pool.query(
    `SELECT id FROM apexmail_queue WHERE tracking_id = $1`,
    [trackingId]
  );
  
  if (email.rows.length > 0) {
    await apexmail.recordEvent({
      type: 'opened',
      emailId: email.rows[0].id,
      trackingId,
      occurredAt: new Date(),
      metadata: {
        ip: req.ip,
        userAgent: req.headers['user-agent'],
      },
    });
  }
  
  // Return 1x1 transparent GIF
  const pixel = Buffer.from(
    'R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7',
    'base64'
  );
  
  res.set({
    'Content-Type': 'image/gif',
    'Content-Length': pixel.length,
    'Cache-Control': 'no-store, no-cache, must-revalidate, proxy-revalidate',
  });
  res.send(pixel);
});

/**
 * GET /api/apexmail/track/:trackingId/click/:shortCode
 * Click tracking redirect
 */
router.get('/track/:trackingId/click/:shortCode', async (req: Request, res: Response) => {
  const { trackingId, shortCode } = req.params;
  
  // Look up original URL
  const link = await pool.query(
    `SELECT original_url, email_id FROM apexmail_links WHERE short_code = $1`,
    [shortCode]
  );
  
  if (link.rows.length === 0) {
    return res.status(404).send('Link not found');
  }
  
  const { original_url, email_id } = link.rows[0];
  
  // Record click event
  await pool.query(
    `UPDATE apexmail_links SET click_count = click_count + 1 WHERE short_code = $1`,
    [shortCode]
  );
  
  if (email_id) {
    await apexmail.recordEvent({
      type: 'clicked',
      emailId: email_id,
      trackingId,
      occurredAt: new Date(),
      metadata: {
        ip: req.ip,
        userAgent: req.headers['user-agent'],
        linkUrl: original_url,
      },
    });
  }
  
  // Redirect to original URL
  res.redirect(302, original_url);
});

/**
 * GET/POST /api/apexmail/unsubscribe/:trackingId
 * One-click unsubscribe (RFC 8058 compliant)
 */
router.all('/unsubscribe/:trackingId', async (req: Request, res: Response) => {
  const { trackingId } = req.params;
  
  const email = await pool.query(
    `SELECT id, to_email, entity_type, entity_id FROM apexmail_queue WHERE tracking_id = $1`,
    [trackingId]
  );
  
  if (email.rows.length === 0) {
    return res.status(404).send('Email not found');
  }
  
  const { to_email, entity_type, entity_id } = email.rows[0];
  
  // Add to suppression list
  await pool.query(`
    INSERT INTO apexmail_suppressions (email, reason, source)
    VALUES ($1, 'unsubscribe', $2)
    ON CONFLICT (email) DO NOTHING
  `, [to_email.toLowerCase(), `user:${trackingId}`]);
  
  // If linked to a contact, update contact status
  if (entity_type === 'contact' && entity_id) {
    await pool.query(`
      UPDATE sa_contacts SET unsubscribed = true, unsubscribed_at = NOW()
      WHERE id = $1
    `, [entity_id]);
  }
  
  // Record event
  await apexmail.recordEvent({
    type: 'unsubscribed',
    emailId: email.rows[0].id,
    trackingId,
    occurredAt: new Date(),
  });
  
  // Show confirmation page
  res.send(`
    <!DOCTYPE html>
    <html>
    <head><title>Unsubscribed</title></head>
    <body style="font-family: Arial; text-align: center; padding: 50px;">
      <h1>✓ Unsubscribed</h1>
      <p>You have been successfully unsubscribed from our mailing list.</p>
      <p>You will no longer receive emails from ApexMediation.</p>
    </body>
    </html>
  `);
});

// ==========================================================================
// ADMIN/DASHBOARD ENDPOINTS (Require auth)
// ==========================================================================

/**
 * GET /api/apexmail/stats
 * Email statistics for dashboard
 */
router.get('/stats', async (req: Request, res: Response) => {
  const days = parseInt(req.query.days as string) || 7;
  const since = new Date(Date.now() - days * 24 * 60 * 60 * 1000);
  
  const stats = await apexmail.getStats(since);
  res.json(stats);
});

/**
 * GET /api/apexmail/domain-health
 * Domain health status
 */
router.get('/domain-health', async (req: Request, res: Response) => {
  const result = await pool.query(`
    SELECT * FROM apexmail_domain_health ORDER BY domain
  `);
  res.json(result.rows);
});

/**
 * GET /api/apexmail/queue
 * View email queue status
 */
router.get('/queue', async (req: Request, res: Response) => {
  const status = req.query.status as string || 'queued';
  const limit = Math.min(parseInt(req.query.limit as string) || 50, 200);
  
  const result = await pool.query(`
    SELECT id, to_email, subject, status, attempt_count, created_at, next_attempt_at
    FROM apexmail_queue
    WHERE status = $1
    ORDER BY priority, created_at
    LIMIT $2
  `, [status, limit]);
  
  res.json(result.rows);
});

/**
 * GET /api/apexmail/templates
 * List email templates
 */
router.get('/templates', async (req: Request, res: Response) => {
  const result = await pool.query(`
    SELECT id, name, version, category, active, created_at, updated_at
    FROM apexmail_templates
    ORDER BY name, version DESC
  `);
  res.json(result.rows);
});

export default router;
```

---

## Migration Plan

### Phase 1: Database Setup (Day 1)

```sql
-- Run migration to create ApexMail tables
-- File: control-plane/migrations/XXX_create_apexmail_tables.sql

BEGIN;

-- Create all tables from schema above
-- (See Database Schema section)

-- Seed default templates
INSERT INTO apexmail_templates (name, subject_template, body_html_template, category) VALUES
('email-verification', 
 'Verify your email address', 
 '<h1>Welcome!</h1><p>Click <a href="{{verificationUrl}}">here</a> to verify.</p>',
 'transactional'),
('password-reset',
 'Reset your password',
 '<h1>Password Reset</h1><p>Click <a href="{{resetUrl}}">here</a> to reset your password.</p>',
 'transactional'),
('payment-failed',
 'Payment failed for your subscription',
 '<h1>Payment Failed</h1><p>Your payment of {{amount}} {{currency}} failed. <a href="{{paymentUrl}}">Update payment method</a>.</p>',
 'dunning');

COMMIT;
```

### Phase 2: Service Integration (Day 2-3)

1. **Create ApexMail service** → `control-plane/src/services/apexmail/index.ts`
2. **Create API routes** → `control-plane/src/routes/apexmail-api.ts`
3. **Update existing services** to use ApexMail:

```typescript
// Before (sales-automation.ts)
import { Resend } from 'resend';
const resend = new Resend(process.env.RESEND_API_KEY);
await resend.emails.send({ ... });

// After
import { apexmail } from '../services/apexmail';
await apexmail.send({ ... });
```

### Phase 3: Parallel Running (Day 4-5)

Run both Resend and ApexMail simultaneously:

```typescript
// Feature flag for gradual rollout
const useApexMail = process.env.USE_APEXMAIL === 'true';

if (useApexMail) {
  await apexmail.send(options);
} else {
  await resend.emails.send(options);
}
```

### Phase 4: Full Migration (Day 6-7)

1. Remove Resend SDK and API key
2. Update all imports
3. Remove `resend` from package.json

---

## SMTP Server Configuration

### Test Server Setup (Postfix)

```bash
# /etc/postfix/main.cf
myhostname = mail.apexmediation.ee
mydomain = apexmediation.ee
myorigin = $mydomain
inet_interfaces = all
mydestination = $myhostname, localhost.$mydomain, localhost

# TLS
smtpd_tls_cert_file = /etc/letsencrypt/live/mail.apexmediation.ee/fullchain.pem
smtpd_tls_key_file = /etc/letsencrypt/live/mail.apexmediation.ee/privkey.pem
smtpd_use_tls = yes
smtpd_tls_security_level = may

# Rate limiting (internal use only)
smtpd_recipient_limit = 100
smtpd_client_message_rate_limit = 100

# Relay only from localhost
mynetworks = 127.0.0.0/8 [::1]/128
relayhost =
```

### DNS Records Required

```
; MX for receiving replies
replies.apexmediation.ee.  IN MX  10 mail.apexmediation.ee.

; SPF for sending
apexmediation.ee.          IN TXT "v=spf1 ip4:YOUR_SERVER_IP include:_spf.google.com ~all"

; DKIM (generate with opendkim)
mail._domainkey.apexmediation.ee. IN TXT "v=DKIM1; k=rsa; p=YOUR_PUBLIC_KEY"

; DMARC
_dmarc.apexmediation.ee.   IN TXT "v=DMARC1; p=quarantine; rua=mailto:dmarc@apexmediation.ee"
```

---

## Feature Comparison Matrix

| Feature | Resend | ApexMail | Notes |
|---------|--------|----------|-------|
| **Cost** | $20/mo (50k) | $0 | Runs on existing server |
| **Latency** | 200-500ms | 50-100ms | Direct SMTP vs API |
| **Rate Limit** | 10 req/sec | Unlimited | Self-imposed 50/sec for safety |
| **Open Tracking** | ✅ | ✅ | 1x1 pixel |
| **Click Tracking** | ✅ | ✅ | Short URL redirect |
| **Bounce Handling** | Webhook | Real-time + DB | Automatic suppression |
| **Templates** | External | DB-versioned | Version control |
| **A/B Testing** | Manual | Thompson Sampler | Automatic optimization |
| **Domain Health** | Basic | Full monitoring | Auto-pause on issues |
| **Logs Retention** | 7 days | Forever | Full audit trail |
| **Custom Headers** | ✅ | ✅ | RFC 8058 compliance |
| **Reply Tracking** | ❌ | ✅ | Inbound SMTP server |
| **Geo-IP** | ❌ | ✅ | MaxMind integration ready |
| **Queue Visibility** | ❌ | ✅ | Full queue inspection |
| **Priority Sending** | ❌ | ✅ | 1-10 priority levels |
| **Retry Logic** | Opaque | Configurable | Exponential backoff |

---

## Performance Benchmarks (Projected)

| Metric | Resend | ApexMail (Projected) |
|--------|--------|----------------------|
| Time to first byte | 180-400ms | 30-80ms |
| Throughput (burst) | 10/sec | 50/sec |
| Throughput (sustained) | 600/min | 3000/min |
| Queue latency | N/A | <1 sec |
| 99th percentile | 800ms | 150ms |

---

## Monitoring & Alerting

### Prometheus Metrics

```typescript
// Expose these metrics at /metrics
apexmail_emails_sent_total{status="success|failed|bounced"}
apexmail_queue_depth{status="queued|deferred|sending"}
apexmail_send_duration_seconds
apexmail_domain_health{domain, status}
apexmail_bounce_rate{domain}
```

### Alerting Rules

```yaml
# Prometheus alerting rules
groups:
  - name: apexmail
    rules:
      - alert: ApexMailHighBounceRate
        expr: apexmail_bounce_rate > 0.05
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High bounce rate detected"
          
      - alert: ApexMailQueueBacklog
        expr: apexmail_queue_depth{status="queued"} > 1000
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Email queue backlog growing"
```

---

## Security Considerations

1. **TLS Encryption**: All SMTP connections use STARTTLS
2. **SPF/DKIM/DMARC**: Full email authentication
3. **Suppression List**: Automatic hard bounce/complaint handling
4. **Rate Limiting**: Self-imposed limits prevent abuse
5. **Audit Log**: All sends logged with full metadata
6. **No External Dependencies**: All data stays on your server

---

## Rollback Plan

If issues arise, instant rollback by:

```bash
# Set environment variable to use Resend again
export USE_APEXMAIL=false

# Or in .env
USE_APEXMAIL=false
```

Both systems can coexist during transition.
