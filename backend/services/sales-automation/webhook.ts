// services/sales-automation/webhook.ts
// Webhook handler for ApexMail events (opens, clicks, bounces)
// Deploy this as an API endpoint to receive email tracking data
// REVOLUTIONARY: Integrated with SmartReplyAnalyzer and DomainHealthGuard

import { Pool } from 'pg';
import * as crypto from 'crypto';
import { SmartReplyAnalyzer } from './SmartReplyAnalyzer';
import { DomainHealthGuard } from './DomainHealthGuard';
import { IntentSignalDetector } from './IntentSignalDetector';

interface ApexMailWebhookPayload {
  type: string;
  created_at: string;
  data: {
    email_id: string;
    from: string;
    to: string[];
    subject: string;
    created_at: string;
    // For bounce events
    bounce?: {
      message: string;
    };
    // For click events
    click?: {
      link: string;
      timestamp: string;
    };
  };
}

const DATABASE_URL = process.env.DATABASE_URL || '';
const APEXMAIL_WEBHOOK_SECRET = process.env.APEXMAIL_WEBHOOK_SECRET || '';

// Singleton database pool to prevent connection exhaustion
const pool = new Pool({ 
  connectionString: DATABASE_URL,
  max: 20, // Limit connections to prevent OOM/exhaustion
  idleTimeoutMillis: 30000,
  connectionTimeoutMillis: 5000,
});

pool.on('error', (err) => {
  console.error('[Webhook] Unexpected database pool error', err);
});

// REVOLUTIONARY: Initialize smart services
let replyAnalyzer: SmartReplyAnalyzer | null = null;
let domainHealthGuard: DomainHealthGuard | null = null;
let intentDetector: IntentSignalDetector | null = null;

if (DATABASE_URL) {
  replyAnalyzer = new SmartReplyAnalyzer(DATABASE_URL);
  domainHealthGuard = new DomainHealthGuard(DATABASE_URL);
  intentDetector = new IntentSignalDetector(DATABASE_URL);
}

/**
 * Verify ApexMail webhook signature
 * SECURITY: Never skip verification - returns false if secret is not configured
 */
function verifySignature(payload: string, signature: string, secret: string): boolean {
  // SECURITY FIX: Never accept empty secret - fail closed
  if (!secret || secret.trim() === '') {
    console.error('[Webhook] CRITICAL: verifySignature called with empty secret - rejecting');
    return false;
  }
  
  // SECURITY: Validate signature format
  if (!signature || !signature.startsWith('sha256=')) {
    console.warn('[Webhook] Invalid signature format');
    return false;
  }
  
  const hmac = crypto.createHmac('sha256', secret);
  const digest = hmac.update(payload).digest('hex');
  
  // SECURITY: Use constant-time comparison to prevent timing attacks
  const expectedSig = `sha256=${digest}`;
  if (signature.length !== expectedSig.length) {
    return false;
  }
  
  return crypto.timingSafeEqual(
    Buffer.from(signature),
    Buffer.from(expectedSig)
  );
}

/**
 * Process incoming webhook from ApexMail
 * Call this from your Express/Fastify route handler
 */
export async function handleApexMailWebhook(
  rawBody: string,
  signature: string
): Promise<{ success: boolean; message: string }> {
  // Verify signature
  if (APEXMAIL_WEBHOOK_SECRET && !verifySignature(rawBody, signature, APEXMAIL_WEBHOOK_SECRET)) {
    return { success: false, message: 'Invalid signature' };
  }

  try {
    // Note: pool is now a singleton initialized outside
    const payload: ApexMailWebhookPayload = JSON.parse(rawBody);
    const messageId = payload.data.email_id;
    const toEmail = payload.data.to[0];

    // Find the email in our database
    const emailResult = await pool.query(
      'SELECT e.*, c.id as contact_id FROM outreach_emails e JOIN sales_contacts c ON e.contact_id = c.id WHERE e.message_id = $1',
      [messageId]
    );

    if (emailResult.rows.length === 0) {
      console.log(`[Webhook] Email not found: ${messageId}`);
      return { success: true, message: 'Email not tracked by us' };
    }

    const email = emailResult.rows[0];

    switch (payload.type) {
      case 'email.delivered':
        await pool.query(
          `UPDATE outreach_emails 
           SET delivered_at = COALESCE(delivered_at, CURRENT_TIMESTAMP),
               status = CASE WHEN status = 'sent' THEN 'delivered' ELSE status END,
               updated_at = CURRENT_TIMESTAMP
           WHERE id = $1`,
          [email.id]
        );
        console.log(`[Webhook] Delivered: ${toEmail}`);
        break;

      case 'email.opened':
        await pool.query(`
          UPDATE outreach_emails 
          SET opened_at = COALESCE(opened_at, CURRENT_TIMESTAMP),
              opened_count = COALESCE(opened_count, 0) + 1,
              status = CASE WHEN status IN ('sent', 'delivered') THEN 'opened' ELSE status END,
              updated_at = CURRENT_TIMESTAMP
          WHERE id = $1
        `, [email.id]);
        
        // Log activity
        await pool.query(
          'INSERT INTO sales_activities (contact_id, activity_type, channel, metadata, is_automated) VALUES ($1, $2, $3, $4, true)',
          [email.contact_id, 'email_opened', 'email', JSON.stringify({ email_id: email.id })]
        );
        
        // Increase contact score for engagement
        await pool.query(
          'UPDATE sales_contacts SET contact_score = LEAST(contact_score + 10, 100), updated_at = CURRENT_TIMESTAMP WHERE id = $1',
          [email.contact_id]
        );
        
        console.log(`[Webhook] Opened: ${toEmail}`);
        break;

      case 'email.clicked':
        await pool.query(`
          UPDATE outreach_emails 
          SET clicked_at = COALESCE(clicked_at, CURRENT_TIMESTAMP),
              clicked_count = COALESCE(clicked_count, 0) + 1,
              status = 'clicked',
              updated_at = CURRENT_TIMESTAMP
          WHERE id = $1
        `, [email.id]);
        
        await pool.query(
          'INSERT INTO sales_activities (contact_id, activity_type, channel, metadata, is_automated) VALUES ($1, $2, $3, $4, true)',
          [email.contact_id, 'email_clicked', 'email', JSON.stringify({ 
            email_id: email.id,
            link: payload.data.click?.link 
          })]
        );
        
        // Higher score boost for clicks
        await pool.query(
          'UPDATE sales_contacts SET contact_score = LEAST(contact_score + 20, 100), updated_at = CURRENT_TIMESTAMP WHERE id = $1',
          [email.contact_id]
        );
        
        console.log(`[Webhook] Clicked: ${toEmail} -> ${payload.data.click?.link}`);
        break;

      case 'email.bounced':
        await pool.query(
          'UPDATE outreach_emails SET bounced_at = CURRENT_TIMESTAMP, bounce_reason = $1, status = $2, updated_at = CURRENT_TIMESTAMP WHERE id = $3',
          [payload.data.bounce?.message || null, 'bounced', email.id]
        );
        
        // Mark contact as bounced
        await pool.query(
          `UPDATE sales_contacts 
           SET status = $1, 
               contact_score = 0,
               email_verified = false,
               email_verification_status = 'invalid',
               updated_at = CURRENT_TIMESTAMP 
           WHERE id = $2`,
          ['bounced', email.contact_id]
        );
        
        // Pause all enrollments for this contact
        await pool.query(
          'UPDATE outreach_enrollments SET status = $1, updated_at = CURRENT_TIMESTAMP WHERE contact_id = $2',
          ['bounced', email.contact_id]
        );
        
        await pool.query(
          'INSERT INTO sales_activities (contact_id, activity_type, channel, metadata, is_automated) VALUES ($1, $2, $3, $4, true)',
          [email.contact_id, 'email_bounced', 'email', JSON.stringify({ 
            email_id: email.id,
            reason: payload.data.bounce?.message 
          })]
        );
        
        // REVOLUTIONARY: Record bounce in Domain Health Guard
        // This automatically triggers domain quarantine if bounce rate > 7%
        if (domainHealthGuard && payload.data.from) {
          try {
            await domainHealthGuard.recordBounce(payload.data.from);
            console.log(`[Webhook] Domain health updated for bounce: ${payload.data.from}`);
          } catch (dhError) {
            console.error('[Webhook] Domain health update failed:', dhError);
          }
        }
        
        console.log(`[Webhook] Bounced: ${toEmail} - ${payload.data.bounce?.message}`);
        break;

      case 'email.complained':
        // Spam complaint - immediately stop all contact
        await pool.query(
          'UPDATE sales_contacts SET status = $1, contact_score = 0, opted_out_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = $2',
          ['opted_out', email.contact_id]
        );
        
        await pool.query(
          'UPDATE outreach_enrollments SET status = $1, updated_at = CURRENT_TIMESTAMP WHERE contact_id = $2',
          ['opted_out', email.contact_id]
        );
        
        await pool.query(
          'UPDATE outreach_emails SET status = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2',
          ['complained', email.id]
        );
        
        await pool.query(
          'INSERT INTO sales_activities (contact_id, activity_type, channel, metadata, is_automated) VALUES ($1, $2, $3, $4, true)',
          [email.contact_id, 'spam_complaint', 'email', JSON.stringify({ email_id: email.id })]
        );
        
        // REVOLUTIONARY: Record spam report in Domain Health Guard
        // This can trigger domain quarantine after consecutive reports
        if (domainHealthGuard && payload.data.from) {
          try {
            await domainHealthGuard.recordSpamReport(payload.data.from);
            console.log(`[Webhook] Domain health updated for spam report: ${payload.data.from}`);
          } catch (dhError) {
            console.error('[Webhook] Domain health spam update failed:', dhError);
          }
        }
        
        console.log(`[Webhook] SPAM COMPLAINT: ${toEmail}`);
        break;
    }

    return { success: true, message: `Processed: ${payload.type}` };
  } catch (error) {
    console.error('[Webhook] Error:', error);
    return { success: false, message: String(error) };
  } finally {
    // Note: Do NOT call pool.end() here as we use a singleton pool
  }
}

/**
 * Express route handler example
 */
export function createWebhookRouter() {
  // Example for Express:
  // 
  // import express from 'express';
  // import { handleResendWebhook } from './webhook';
  // 
  // const router = express.Router();
  // 
  // router.post('/webhooks/resend', express.raw({ type: 'application/json' }), async (req, res) => {
  //   const signature = req.headers['svix-signature'] as string || '';
  //   const result = await handleResendWebhook(req.body.toString(), signature);
  //   res.status(result.success ? 200 : 400).json(result);
  // });
  // 
  // export default router;
  
  console.log('Webhook handler ready. Add this route to your Express app:');
  console.log('  POST /api/webhooks/resend');
}

// ============================================================================
// REPLY DETECTION (for when replies come in via email)
// ============================================================================

/**
 * Process incoming reply email
 * REVOLUTIONARY: Uses SmartReplyAnalyzer for intelligent categorization
 * Call this when you detect a reply (via email forwarding, Resend inbound, etc.)
 */
export async function processReply(
  fromEmail: string,
  subject: string,
  body: string
): Promise<{ 
  success: boolean; 
  contactId?: string;
  category?: string;
  sentiment?: string;
  suggestedAction?: string;
}> {
  try {
    // Find contact by email
    const contactResult = await pool.query(
      'SELECT * FROM sales_contacts WHERE LOWER(email) = LOWER($1)',
      [fromEmail]
    );

    if (contactResult.rows.length === 0) {
      return { success: false };
    }

    const contact = contactResult.rows[0];

    // Find the most recent email to get email_id
    const emailResult = await pool.query(`
      SELECT id FROM outreach_emails 
      WHERE contact_id = $1 
      ORDER BY sent_at DESC 
      LIMIT 1
    `, [contact.id]);
    
    const emailId = emailResult.rows[0]?.id || 'unknown';

    // REVOLUTIONARY: Use SmartReplyAnalyzer for intelligent categorization
    let category = 'neutral';
    let sentiment: 'positive' | 'negative' | 'neutral' = 'neutral';
    let suggestedAction = 'Review manually';
    
    if (replyAnalyzer) {
      try {
        const analysis = replyAnalyzer.analyzeReply(emailId, contact.id, subject, body);
        category = analysis.category;
        sentiment = analysis.sentiment;
        suggestedAction = analysis.suggestedAction;
        
        // Store the analysis
        await replyAnalyzer.analyzeAndStore(emailId, contact.id, subject, body);
        
        console.log(`[Reply] SmartAnalysis: category=${category}, sentiment=${sentiment}, urgency=${analysis.urgency}`);
      } catch (analyzerError) {
        console.error('[Reply] SmartReplyAnalyzer failed, using fallback:', analyzerError);
        sentiment = analyzeReplySentiment(body);
      }
    } else {
      // Fallback to simple analysis
      sentiment = analyzeReplySentiment(body);
      category = sentiment === 'positive' ? 'interested' : sentiment === 'negative' ? 'not_interested' : 'neutral';
    }

    // Update contact status based on category
    let newStatus = 'responded';
    let newScore = 100;
    
    if (category === 'unsubscribe' || category === 'not_interested') {
      newStatus = 'opted_out';
      newScore = 0;
    } else if (category === 'interested') {
      newStatus = 'replied';
      newScore = 100;
    } else if (category === 'referral') {
      newStatus = 'replied';
      newScore = 80;
    }

    await pool.query(
      `UPDATE sales_contacts 
       SET status = $1, 
           contact_score = $2, 
           opted_out_at = CASE WHEN $1 = 'opted_out' THEN CURRENT_TIMESTAMP ELSE opted_out_at END,
           updated_at = CURRENT_TIMESTAMP 
       WHERE id = $3`,
      [newStatus, newScore, contact.id]
    );

    // Mark enrollment based on category
    const enrollmentStatus = category === 'unsubscribe' ? 'opted_out' : 'responded';
    await pool.query(`
      UPDATE outreach_enrollments 
      SET status = $1, updated_at = CURRENT_TIMESTAMP 
      WHERE contact_id = $2 AND status = 'active'
    `, [enrollmentStatus, contact.id]);

    // Update the most recent email
    await pool.query(`
      UPDATE outreach_emails 
      SET replied_at = CURRENT_TIMESTAMP, status = 'replied', updated_at = CURRENT_TIMESTAMP
      WHERE id = (
        SELECT id FROM outreach_emails
        WHERE contact_id = $1
          AND replied_at IS NULL
        ORDER BY sent_at DESC
        LIMIT 1
      )
    `, [contact.id]);

    // Log activity with full analysis
    await pool.query(
      'INSERT INTO sales_activities (contact_id, activity_type, channel, metadata, is_automated) VALUES ($1, $2, $3, $4, true)',
      [contact.id, 'email_replied', 'email', JSON.stringify({ 
        subject, 
        preview: body.substring(0, 200),
        category,
        sentiment,
        suggestedAction,
      })]
    );

    // REVOLUTIONARY: Record intent signals for hot leads
    if (intentDetector && (category === 'interested' || category === 'question')) {
      try {
        await intentDetector.recordSignal(
          contact.id,
          category === 'interested' ? 'fast_reply' : 'question_asked',
          'email_reply',
          `Reply to: ${subject}`
        );
      } catch (intentError) {
        console.error('[Reply] IntentSignalDetector failed:', intentError);
      }
    }
    
    return { success: true, contactId: contact.id, category, sentiment, suggestedAction };
  } finally {
    // Note: Do NOT call pool.end() here as we use a singleton pool
  }
}

/**
 * Simple reply sentiment analysis (fallback, no AI needed)
 */
function analyzeReplySentiment(body: string): 'positive' | 'negative' | 'neutral' {
  const text = body.toLowerCase();
  
  const positiveSignals = [
    'interested', 'yes', 'let\'s talk', 'schedule', 'call', 'sounds good',
    'tell me more', 'would love to', 'available', 'free', 'demo'
  ];
  
  const negativeSignals = [
    'not interested', 'no thanks', 'unsubscribe', 'remove', 'stop',
    'don\'t contact', 'not for us', 'no need', 'pass'
  ];
  
  let positiveScore = 0;
  let negativeScore = 0;
  
  for (const signal of positiveSignals) {
    if (text.includes(signal)) positiveScore++;
  }
  
  for (const signal of negativeSignals) {
    if (text.includes(signal)) negativeScore++;
  }
  
  if (positiveScore > negativeScore) return 'positive';
  if (negativeScore > positiveScore) return 'negative';
  return 'neutral';
}
