/**
 * AUTONOMOUS SALES API ROUTES
 * 
 * REST API endpoints for the autonomous sales system.
 * These routes are mounted on the control-plane server.
 * 
 * ARCHITECTURE:
 *   - Control Plane (App Server): Provides API, stores data, orchestrates
 *   - Worker (Test Server): Scrapes data, sends results via these APIs
 * 
 * Worker Endpoints (called by test server worker):
 *   GET  /api/autonomous/sources              - Get sources due for scraping
 *   POST /api/autonomous/sources/:id/scraped  - Report scrape results + apps
 *   GET  /api/autonomous/contacts/unscored    - Get contacts needing scoring  
 *   POST /api/autonomous/contacts/:id/score   - Submit lead score
 *   GET  /api/autonomous/health               - Health check
 * 
 * Admin Endpoints (called by console/operators):
 *   POST /api/autonomous/discover    - Trigger discovery (local orchestrator)
 *   POST /api/autonomous/score       - Trigger scoring (local orchestrator)
 *   POST /api/autonomous/compose     - Compose message for contact
 *   POST /api/autonomous/classify    - Classify incoming email
 *   POST /api/autonomous/feedback    - Record email event feedback
 *   GET  /api/autonomous/stats       - Get system statistics
 *   GET  /api/autonomous/arms        - Get Thompson Sampler arms
 *   GET  /api/autonomous/templates   - Get spintax templates
 *   GET  /api/autonomous/inbox       - Get messages needing review
 *   POST /api/autonomous/inbox/:id   - Submit review for message
 */

import { Router, Request, Response } from 'express';
import { getOrchestrator } from '../services/autonomous-orchestrator';
import { getActiveArms, createArm, getBanditStats } from '../services/math/thompson-sampler';
import { getTemplates, createTemplate, previewVariations } from '../services/messaging/spintax-engine';
import { getMessagesForReview, submitReview, getClassificationStats } from '../services/inbox/clerk';
import { getSources, saveDiscoveredApps, getSourcesDueForScrape } from '../services/discovery/hunter';
import { calculateLeadScore, saveLeadScore, getContactsNeedingScoring } from '../services/discovery/analyst';
import { queryOperator, queryReadOnly } from '../database';
import { logger } from '../logger';

const router = Router();

// ============================================================================
// MIDDLEWARE
// ============================================================================

import jwt from 'jsonwebtoken';

/**
 * Dual authentication for autonomous API:
 * - API Key (X-API-Key header): for external calls from worker/edge
 * - JWT Bearer token: for dashboard/operator access
 * 
 * SECURITY: Fail-closed - rejects requests if neither auth method succeeds
 */
function authenticateApiKeyOrJwt(req: Request, res: Response, next: () => void): void {
  const apiKey = req.headers['x-api-key'] as string;
  const expectedKey = process.env.AUTONOMOUS_API_KEY;
  const isProduction = process.env.NODE_ENV === 'production';
  
  // Try API Key authentication first (for worker/edge calls)
  if (apiKey && expectedKey && apiKey === expectedKey) {
    return next();
  }
  
  // Try JWT Bearer token authentication (for dashboard/operator)
  const authHeader = req.headers.authorization;
  if (authHeader?.startsWith('Bearer ')) {
    const token = authHeader.slice(7);
    const jwtSecret = process.env.OPERATOR_JWT_SECRET;
    
    if (jwtSecret) {
      try {
        const decoded = jwt.verify(token, jwtSecret) as { sub: string; role: string };
        if (decoded.role === 'operator') {
          // Valid operator token - allow access
          return next();
        }
      } catch {
        // Invalid JWT - continue to fail
      }
    }
  }
  
  // Dev mode bypass (non-production only)
  if (!expectedKey && !isProduction) {
    console.warn('[DEV] AUTONOMOUS_API_KEY not set - allowing unauthenticated access (non-production only)');
    return next();
  }
  
  res.status(401).json({ error: 'Unauthorized' });
}

// ============================================================================
// PUBLIC TRACKING ENDPOINTS (No auth required - must be before middleware)
// These endpoints are embedded in emails and must work without API keys
// ============================================================================

/**
 * GET /api/autonomous/track/:trackingId/open
 * Tracking pixel endpoint for email opens
 * Returns a 1x1 transparent GIF
 * 
 * NOTE: No auth required - this is hit by email clients loading the image
 */
router.get('/track/:trackingId/open', async (req: Request, res: Response) => {
  const { trackingId } = req.params;
  
  try {
    // Parse tracking ID to get message ID
    const match = trackingId.match(/^track_([^_]+)_/);
    if (match) {
      const messageId = match[1];
      
      // Find the message and update Thompson Sampler
      const messages = await queryReadOnly<{ subject_arm_id: string | null; contact_id: string }>(`
        SELECT subject_arm_id, contact_id FROM sa_outbound_messages WHERE id = $1
      `, [messageId]);
      
      if (messages.length > 0) {
        const { subject_arm_id, contact_id } = messages[0];
        
        // Record open event
        await queryOperator(`
          INSERT INTO sa_email_events 
          (outbound_message_id, contact_id, event_type, arm_id, outcome_recorded, source)
          VALUES ($1, $2, 'open', $3, true, 'tracking_pixel')
          ON CONFLICT DO NOTHING
        `, [messageId, contact_id, subject_arm_id]);
        
        // Update message status
        await queryOperator(`
          UPDATE sa_outbound_messages 
          SET status = 'opened', opened_at = COALESCE(opened_at, CURRENT_TIMESTAMP)
          WHERE id = $1
        `, [messageId]);
        
        // CLOSED LOOP: Record success for Thompson Sampler
        if (subject_arm_id) {
          const { recordOutcome } = await import('../services/math/thompson-sampler.js');
          await recordOutcome(subject_arm_id, true);
          
          logger.info('Email open tracked - Thompson Sampler updated', {
            trackingId,
            messageId,
            armId: subject_arm_id,
          });
        }
      }
    }
  } catch (err) {
    logger.error('Open tracking failed', { trackingId, error: (err as Error).message });
  }
  
  // Always return tracking pixel regardless of errors
  const transparentGif = Buffer.from('R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7', 'base64');
  res.set('Content-Type', 'image/gif');
  res.set('Cache-Control', 'no-cache, no-store, must-revalidate');
  res.send(transparentGif);
});

/**
 * GET /api/autonomous/track/:trackingId/click/:linkId
 * Click tracking endpoint - records click and redirects
 * 
 * NOTE: No auth required - this is hit by users clicking links in emails
 */
router.get('/track/:trackingId/click/:linkId', async (req: Request, res: Response) => {
  const { trackingId, linkId } = req.params;
  const destination = req.query.url as string || 'https://apexmediation.ee';
  
  try {
    const match = trackingId.match(/^track_([^_]+)_/);
    if (match) {
      const messageId = match[1];
      
      const messages = await queryReadOnly<{ subject_arm_id: string | null; contact_id: string }>(`
        SELECT subject_arm_id, contact_id FROM sa_outbound_messages WHERE id = $1
      `, [messageId]);
      
      if (messages.length > 0) {
        const { subject_arm_id, contact_id } = messages[0];
        
        // Record click event
        await queryOperator(`
          INSERT INTO sa_email_events 
          (outbound_message_id, contact_id, event_type, arm_id, outcome_recorded, source, event_data)
          VALUES ($1, $2, 'click', $3, true, 'tracking_link', $4)
        `, [messageId, contact_id, subject_arm_id, JSON.stringify({ linkId, destination })]);
        
        // Update message status
        await queryOperator(`
          UPDATE sa_outbound_messages 
          SET status = 'clicked', clicked_at = COALESCE(clicked_at, CURRENT_TIMESTAMP)
          WHERE id = $1
        `, [messageId]);
        
        // CLOSED LOOP: Record success for Thompson Sampler
        if (subject_arm_id) {
          const { recordOutcome } = await import('../services/math/thompson-sampler.js');
          await recordOutcome(subject_arm_id, true);
          
          logger.info('Email click tracked - Thompson Sampler updated', {
            trackingId,
            messageId,
            linkId,
            armId: subject_arm_id,
          });
        }
      }
    }
  } catch (err) {
    logger.error('Click tracking failed', { trackingId, linkId, error: (err as Error).message });
  }
  
  // Redirect to destination regardless of errors
  res.redirect(302, destination);
});

/**
 * GET /api/autonomous/unsubscribe/:contactId
 * Unsubscribe endpoint for List-Unsubscribe header
 * Marks contact as opted-out
 * 
 * NOTE: No auth required - this is clicked by users from emails
 */
router.get('/unsubscribe/:contactId', async (req: Request, res: Response) => {
  const { contactId } = req.params;
  
  try {
    // Check if contact exists
    const contacts = await queryReadOnly<{ id: string; email: string }>(`
      SELECT id, email FROM sa_contacts WHERE id = $1
    `, [contactId]);
    
    if (contacts.length === 0) {
      res.status(404).send(`
        <!DOCTYPE html>
        <html>
        <head><title>Unsubscribe</title></head>
        <body style="font-family: sans-serif; padding: 40px; text-align: center;">
          <h1>Contact Not Found</h1>
          <p>This unsubscribe link may be invalid or expired.</p>
        </body>
        </html>
      `);
      return;
    }
    
    // Mark contact as opted out
    await queryOperator(`
      UPDATE sa_contacts 
      SET status = 'opted_out', opted_out_at = CURRENT_TIMESTAMP
      WHERE id = $1
    `, [contactId]);
    
    // Log the unsubscribe event
    logger.info('Contact unsubscribed', { contactId, email: contacts[0].email });
    
    res.send(`
      <!DOCTYPE html>
      <html>
      <head><title>Unsubscribed</title></head>
      <body style="font-family: sans-serif; padding: 40px; text-align: center;">
        <h1>You have been unsubscribed</h1>
        <p>You will no longer receive emails from Apex Mediation.</p>
        <p style="color: #666; margin-top: 30px;">If this was a mistake, please contact us.</p>
      </body>
      </html>
    `);
  } catch (err) {
    logger.error('Unsubscribe failed', { contactId, error: (err as Error).message });
    res.status(500).send(`
      <!DOCTYPE html>
      <html>
      <head><title>Error</title></head>
      <body style="font-family: sans-serif; padding: 40px; text-align: center;">
        <h1>Something went wrong</h1>
        <p>Please try again later or contact support.</p>
      </body>
      </html>
    `);
  }
});

/**
 * POST /api/autonomous/unsubscribe/:contactId
 * One-click unsubscribe endpoint for RFC 8058 compliance
 * Handles List-Unsubscribe-Post header one-click unsubscribe
 * 
 * NOTE: No auth required - this is triggered by email clients
 */
router.post('/unsubscribe/:contactId', async (req: Request, res: Response) => {
  const { contactId } = req.params;
  
  try {
    // Check if contact exists
    const contacts = await queryReadOnly<{ id: string; email: string }>(`
      SELECT id, email FROM sa_contacts WHERE id = $1
    `, [contactId]);
    
    if (contacts.length === 0) {
      res.status(404).json({ error: 'Contact not found' });
      return;
    }
    
    // Mark contact as opted out
    await queryOperator(`
      UPDATE sa_contacts 
      SET status = 'opted_out', opted_out_at = CURRENT_TIMESTAMP
      WHERE id = $1
    `, [contactId]);
    
    // Log the unsubscribe event
    logger.info('Contact unsubscribed (one-click)', { contactId, email: contacts[0].email });
    
    // RFC 8058 expects 200 OK for successful unsubscribe
    res.status(200).json({ success: true, message: 'Unsubscribed successfully' });
  } catch (err) {
    logger.error('One-click unsubscribe failed', { contactId, error: (err as Error).message });
    res.status(500).json({ error: 'Unsubscribe failed' });
  }
});

// Apply auth middleware to all remaining routes
router.use(authenticateApiKeyOrJwt);

// ============================================================================
// WORKER ENDPOINTS (Called by Test Server Worker)
// ============================================================================

/**
 * GET /api/autonomous/sources
 * Get sources, optionally filtered by status and due for scraping
 * 
 * Query params:
 *   - status: 'active' | 'paused' | 'all' (default: 'all')
 *   - due: 'true' to only return sources due for scraping
 */
router.get('/sources', async (req: Request, res: Response) => {
  try {
    const status = req.query.status as string;
    const due = req.query.due === 'true';
    
    let sources;
    if (due) {
      // Get sources that need to be scraped
      sources = await getSourcesDueForScrape();
    } else {
      // Get all sources (optionally filtered by status)
      sources = await getSources();
      if (status && status !== 'all') {
        sources = sources.filter(s => s.active === (status === 'active'));
      }
    }
    
    res.json({
      success: true,
      sources,
    });
  } catch (err) {
    logger.error('Sources fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/sources/:id/scraped
 * Called by worker to report scrape results and submit discovered apps
 */
router.post('/sources/:id/scraped', async (req: Request, res: Response) => {
  try {
    const { id } = req.params;
    const { apps, appsFound, success, error } = req.body;
    
    logger.info('Worker reported scrape results', { 
      sourceId: id, 
      appsFound, 
      appsSubmitted: apps?.length || 0,
      success 
    });
    
    let inserted = 0;
    let updated = 0;
    
    // Save discovered apps if any
    if (apps && apps.length > 0) {
      const result = await saveDiscoveredApps(apps, id);
      inserted = result.inserted;
      updated = result.updated;
    }
    
    // Update source's last_scraped_at
    await queryOperator(`
      UPDATE sa_sources 
      SET last_scraped_at = CURRENT_TIMESTAMP,
          updated_at = CURRENT_TIMESTAMP
      WHERE id = $1
    `, [id]);
    
    // Record scrape job (for history) - uses apps_found column (not apps_discovered)
    await queryOperator(`
      INSERT INTO sa_scrape_jobs (source_id, status, apps_found, contacts_extracted, error_message, completed_at)
      VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP)
    `, [id, success ? 'completed' : 'failed', appsFound || 0, inserted + updated, error || null]);
    
    res.json({
      success: true,
      inserted,
      updated,
    });
  } catch (err) {
    logger.error('Scrape result processing failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * GET /api/autonomous/contacts/unscored
 * Get contacts that need scoring
 */
router.get('/contacts/unscored', async (req: Request, res: Response) => {
  try {
    const limit = parseInt(req.query.limit as string) || 50;
    const contacts = await getContactsNeedingScoring(limit);
    
    res.json({
      success: true,
      contacts,
    });
  } catch (err) {
    logger.error('Unscored contacts fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/contacts/:id/score
 * Called by worker to submit a lead score
 */
router.post('/contacts/:id/score', async (req: Request, res: Response) => {
  try {
    const { id } = req.params;
    const { totalScore, tier, signals } = req.body;
    
    if (totalScore === undefined || !tier) {
      res.status(400).json({ error: 'totalScore and tier are required' });
      return;
    }
    
    // Save the score
    await saveLeadScore({
      contactId: id,
      totalScore,
      tier,
      signals: signals || [],
      scoredAt: new Date(),
    });
    
    logger.info('Lead score saved', { contactId: id, totalScore, tier });
    
    res.json({ success: true });
  } catch (err) {
    logger.error('Score submission failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// ADMIN JOB ENDPOINTS (Local orchestration)
// ============================================================================

// Maximum batch sizes to prevent HTTP timeout (30s typical)
const MAX_DISCOVER_BATCH = 10;
const MAX_SCORE_BATCH = 100;

/**
 * POST /api/autonomous/discover
 * Run a discovery job to find new leads (triggers local orchestrator)
 * 
 * Note: batchSize is capped at MAX_DISCOVER_BATCH to prevent HTTP timeouts.
 * For large batch processing, use external worker or cron job.
 */
router.post('/discover', async (req: Request, res: Response) => {
  try {
    const requestedBatch = parseInt(req.body.batchSize) || 3;
    const batchSize = Math.min(Math.max(1, requestedBatch), MAX_DISCOVER_BATCH);
    
    if (requestedBatch > MAX_DISCOVER_BATCH) {
      logger.warn('Discovery batch size capped', { requested: requestedBatch, capped: batchSize });
    }
    
    const orchestrator = await getOrchestrator();
    const result = await orchestrator.runDiscoveryJob(batchSize);
    
    res.json({
      success: true,
      job: result,
    });
  } catch (err) {
    logger.error('Discovery job failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/enrich
 * Run an enrichment job to find emails for contacts with websites
 * 
 * Uses the Edge Worker to scrape company websites for contact info.
 */
router.post('/enrich', async (req: Request, res: Response) => {
  try {
    const batchSize = Math.min(Math.max(1, parseInt(req.body.batchSize) || 20), 50);
    
    const orchestrator = await getOrchestrator();
    const result = await orchestrator.runEnrichmentJob(batchSize);
    
    res.json({
      success: true,
      job: result,
    });
  } catch (err) {
    logger.error('Enrichment job failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/train
 * Run a training job to retrain the Naive Bayes classifier
 * 
 * Loads all training data from reviewed messages and updates the classifier model.
 * This creates a closed-loop learning system.
 */
router.post('/train', async (req: Request, res: Response) => {
  try {
    const orchestrator = await getOrchestrator();
    const result = await orchestrator.runTrainingJob();
    
    res.json({
      success: true,
      job: result,
    });
  } catch (err) {
    logger.error('Training job failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/train/seed
 * Seed initial Naive Bayes training data
 * 
 * This bootstraps the classifier with realistic labeled examples so it can
 * start classifying properly from day one. Should be run once on first deployment.
 * 
 * Categories:
 * - interested: Positive responses indicating potential customers
 * - not_interested: Polite rejections
 * - unsubscribe: Explicit opt-out requests  
 * - meeting: Meeting-related responses
 * - spam: Auto-generated/irrelevant responses
 * - out_of_office: Vacation/OOO auto-replies
 */
router.post('/train/seed', async (req: Request, res: Response) => {
  try {
    // Check if already seeded
    const existingData = await queryReadOnly<{ count: string }>(`
      SELECT COUNT(*) as count FROM sa_bayes_training WHERE source = 'seed'
    `);
    
    if (parseInt(existingData[0].count) > 0 && !req.body.force) {
      res.json({
        success: true,
        message: 'Already seeded',
        existingCount: parseInt(existingData[0].count),
        hint: 'Use force=true to reseed',
      });
      return;
    }
    
    // Seed examples for each category
    const seedData: Array<{ label: string; text: string }> = [
      // ===== INTERESTED =====
      { label: 'interested', text: 'Hi, this sounds interesting! Can you tell me more about your ad mediation platform?' },
      { label: 'interested', text: 'Thanks for reaching out. We are looking for a better monetization solution. What are your rates?' },
      { label: 'interested', text: 'I\'d like to learn more about Apex. When can we schedule a call?' },
      { label: 'interested', text: 'This is exactly what we need. We currently use AdMob but looking for alternatives. Please send more details.' },
      { label: 'interested', text: 'Great timing! We were just discussing ad monetization. Let\'s talk.' },
      { label: 'interested', text: 'I saw your email and I\'m curious about your fill rates. We get about 70% with our current solution.' },
      { label: 'interested', text: 'Yes I\'m interested. Our app has 500k DAU and we want to improve ARPU. Can you help?' },
      { label: 'interested', text: 'Sounds promising. What makes Apex different from ironSource or AppLovin?' },
      { label: 'interested', text: 'We should definitely talk. Our current eCPM is $2.50 - can you do better?' },
      { label: 'interested', text: 'I\'ve forwarded this to our monetization team. They will be in touch!' },
      
      // ===== NOT INTERESTED =====
      { label: 'not_interested', text: 'Thanks but we\'re not looking for a new ad partner at this time.' },
      { label: 'not_interested', text: 'We already have an ad solution that works well for us. Good luck!' },
      { label: 'not_interested', text: 'Not interested, please don\'t contact me again.' },
      { label: 'not_interested', text: 'Our app doesn\'t monetize with ads. We use subscriptions only.' },
      { label: 'not_interested', text: 'We\'re happy with our current ad stack. No need for changes.' },
      { label: 'not_interested', text: 'Unfortunately this is not relevant for our business model.' },
      { label: 'not_interested', text: 'We\'ve evaluated mediation platforms before and decided against it.' },
      { label: 'not_interested', text: 'Thank you for thinking of us but we\'ll pass on this opportunity.' },
      { label: 'not_interested', text: 'Not for us right now. Maybe in the future.' },
      { label: 'not_interested', text: 'Our publisher team is focused on other priorities at the moment.' },
      
      // ===== UNSUBSCRIBE =====
      { label: 'unsubscribe', text: 'Please remove me from your mailing list.' },
      { label: 'unsubscribe', text: 'UNSUBSCRIBE' },
      { label: 'unsubscribe', text: 'Stop emailing me. Remove my email immediately.' },
      { label: 'unsubscribe', text: 'Please take me off your list. I never signed up for this.' },
      { label: 'unsubscribe', text: 'Unsubscribe me from all future emails.' },
      { label: 'unsubscribe', text: 'Do not contact me again. Remove from list.' },
      { label: 'unsubscribe', text: 'I want to opt out of these communications.' },
      { label: 'unsubscribe', text: 'How do I unsubscribe? Please stop sending emails.' },
      { label: 'unsubscribe', text: 'Remove. Unsubscribe. Do not email again.' },
      { label: 'unsubscribe', text: 'This is spam. I never gave you permission. Remove me now.' },
      
      // ===== MEETING =====
      { label: 'meeting', text: 'I\'m free Tuesday at 2pm. Does that work for a call?' },
      { label: 'meeting', text: 'Let\'s set up a Zoom meeting. Send me a calendar invite.' },
      { label: 'meeting', text: 'How about next week? I have availability Wednesday or Thursday afternoon.' },
      { label: 'meeting', text: 'Can we do a 30 minute call? Here\'s my Calendly: calendly.com/john' },
      { label: 'meeting', text: 'I\'d like to schedule a demo. What times work on your end?' },
      { label: 'meeting', text: 'Yes let\'s talk. I\'m in PST - mornings work best.' },
      { label: 'meeting', text: 'Book a meeting on my calendar: cal.com/developer-jane' },
      { label: 'meeting', text: 'I can do a quick call Friday. Send an invite to this email.' },
      { label: 'meeting', text: 'Let\'s meet next Monday. 10am works. Google Meet or Zoom?' },
      { label: 'meeting', text: 'Happy to jump on a call. Send your meeting link.' },
      
      // ===== OUT OF OFFICE =====
      { label: 'out_of_office', text: 'I\'m currently out of the office with limited email access. I will respond when I return on January 15th.' },
      { label: 'out_of_office', text: 'Thank you for your email. I am on vacation until next week. For urgent matters contact support@company.com' },
      { label: 'out_of_office', text: 'Auto-reply: I\'m away from my desk. Will respond to emails when I\'m back.' },
      { label: 'out_of_office', text: 'OOO: On parental leave until March. Please contact team@company.com in my absence.' },
      { label: 'out_of_office', text: 'I\'m traveling with limited connectivity. Expect delayed responses.' },
      { label: 'out_of_office', text: 'Automatic reply: I\'m out sick today and will get back to you tomorrow.' },
      { label: 'out_of_office', text: 'Away message: Attending a conference this week. Back Monday.' },
      { label: 'out_of_office', text: 'Hi! I\'m currently on holiday and will return to email on Monday the 12th.' },
      { label: 'out_of_office', text: 'Thanks for your email. I\'m at an offsite retreat with no email. Back in 3 days.' },
      { label: 'out_of_office', text: 'Vacation auto-reply: Will respond after January 2nd. Happy holidays!' },
      
      // ===== SPAM / IRRELEVANT =====
      { label: 'spam', text: 'Mail delivery failed: returning message to sender. Address not found.' },
      { label: 'spam', text: 'Your message was not delivered because the recipient\'s mailbox is full.' },
      { label: 'spam', text: 'Mailer-Daemon: Undeliverable mail returned to sender.' },
      { label: 'spam', text: 'RE: Get rich quick with cryptocurrency! Limited time offer!' },
      { label: 'spam', text: 'You\'ve won a prize! Click here to claim your reward now!' },
      { label: 'spam', text: 'This email has been blocked by our spam filter.' },
      { label: 'spam', text: 'Automatic response: Your email was marked as spam.' },
      { label: 'spam', text: 'Undeliverable: User unknown at destination domain.' },
      { label: 'spam', text: '550 5.1.1 The email account does not exist.' },
      { label: 'spam', text: 'Delivery Status Notification: Failure' },
      
      // ===== UNKNOWN (boundary cases for learning) =====
      { label: 'unknown', text: 'Hi' },
      { label: 'unknown', text: 'Thanks' },
      { label: 'unknown', text: 'Got it' },
      { label: 'unknown', text: 'Ok' },
      { label: 'unknown', text: 'Who is this?' },
      { label: 'unknown', text: 'Wrong person.' },
      { label: 'unknown', text: 'I didn\'t understand your email.' },
      { label: 'unknown', text: '?' },
      { label: 'unknown', text: 'I\'ll think about it.' },
      { label: 'unknown', text: 'Let me get back to you.' },
    ];
    
    // Insert seed data
    let inserted = 0;
    for (const item of seedData) {
      try {
        await queryOperator(`
          INSERT INTO sa_bayes_training (label, text_content, source)
          VALUES ($1, $2, 'seed')
          ON CONFLICT DO NOTHING
        `, [item.label, item.text]);
        inserted++;
      } catch {
        // Continue on conflict
      }
    }
    
    // Retrain the classifier with new data
    try {
      const orchestrator = await getOrchestrator();
      await orchestrator.runTrainingJob();
    } catch (trainErr) {
      logger.warn('Training after seed failed', { error: (trainErr as Error).message });
    }
    
    logger.info('Training data seeded', { inserted, total: seedData.length });
    
    res.json({
      success: true,
      seeded: inserted,
      total: seedData.length,
      categories: {
        interested: seedData.filter(d => d.label === 'interested').length,
        not_interested: seedData.filter(d => d.label === 'not_interested').length,
        unsubscribe: seedData.filter(d => d.label === 'unsubscribe').length,
        meeting: seedData.filter(d => d.label === 'meeting').length,
        out_of_office: seedData.filter(d => d.label === 'out_of_office').length,
        spam: seedData.filter(d => d.label === 'spam').length,
        unknown: seedData.filter(d => d.label === 'unknown').length,
      },
    });
    
  } catch (err) {
    logger.error('Training seed failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/score
 * Run a scoring job to evaluate leads
 * 
 * Note: batchSize is capped at MAX_SCORE_BATCH to prevent HTTP timeouts.
 * For large batch processing, use external worker or cron job.
 */
router.post('/score', async (req: Request, res: Response) => {
  try {
    const requestedBatch = parseInt(req.body.batchSize) || 50;
    const batchSize = Math.min(Math.max(1, requestedBatch), MAX_SCORE_BATCH);
    
    if (requestedBatch > MAX_SCORE_BATCH) {
      logger.warn('Scoring batch size capped', { requested: requestedBatch, capped: batchSize });
    }
    
    const orchestrator = await getOrchestrator();
    const result = await orchestrator.runScoringJob(batchSize);
    
    res.json({
      success: true,
      job: result,
    });
  } catch (err) {
    logger.error('Scoring job failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/compose
 * Compose a personalized message for a contact
 */
router.post('/compose', async (req: Request, res: Response) => {
  try {
    const { contactId } = req.body;
    
    if (!contactId) {
      res.status(400).json({ error: 'contactId is required' });
      return;
    }
    
    const orchestrator = await getOrchestrator();
    const message = await orchestrator.composeMessage(contactId);
    
    if (!message) {
      res.status(404).json({ error: 'Contact not found or no templates available' });
      return;
    }
    
    res.json({
      success: true,
      message,
    });
  } catch (err) {
    logger.error('Compose failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/classify
 * Classify an incoming email
 */
router.post('/classify', async (req: Request, res: Response) => {
  try {
    const { fromEmail, subject, body, contactId } = req.body;
    
    if (!fromEmail || !subject || !body) {
      res.status(400).json({ error: 'fromEmail, subject, and body are required' });
      return;
    }
    
    const orchestrator = await getOrchestrator();
    const result = await orchestrator.classifyEmail({
      fromEmail,
      subject,
      body,
      contactId,
    });
    
    res.json({
      success: true,
      classification: result,
    });
  } catch (err) {
    logger.error('Classification failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/feedback
 * Record email event feedback (open, reply, bounce) for Thompson Sampler
 * 
 * CLOSED LOOP: This is the key endpoint that makes the system self-optimizing.
 * Call this when:
 *   - User opens email (eventType: 'open')
 *   - User clicks link (eventType: 'click')
 *   - User replies (eventType: 'reply')
 *   - Email bounces (eventType: 'bounce')
 *   - User unsubscribes (eventType: 'unsubscribe')
 */
router.post('/feedback', async (req: Request, res: Response) => {
  try {
    const { contactId, eventType, subjectArmId } = req.body;
    
    if (!contactId || !eventType) {
      res.status(400).json({ error: 'contactId and eventType are required' });
      return;
    }
    
    const orchestrator = await getOrchestrator();
    await orchestrator.recordEmailEvent({
      contactId,
      eventType,
      subjectArmId,
    });
    
    res.json({ success: true });
  } catch (err) {
    logger.error('Feedback recording failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/webhook/sendgrid
 * SendGrid Inbound Parse Webhook
 * 
 * Receives incoming emails forwarded by SendGrid's Inbound Parse.
 * Automatically classifies and records feedback for the Thompson Sampler.
 * ALSO: Saves classified replies to sa_bayes_training for Naive Bayes learning.
 * 
 * Setup: Configure SendGrid to forward emails to:
 *   https://admin.apexmediation.ee/api/autonomous/webhook/sendgrid
 */
router.post('/webhook/sendgrid', async (req: Request, res: Response) => {
  try {
    const { from, to, subject, text, html } = req.body;
    
    // Extract email address from "Name <email@domain.com>" format
    const fromMatch = (from || '').match(/<([^>]+)>/) || [null, from];
    const fromEmail = (fromMatch[1] || from || '').toLowerCase().trim();
    
    if (!fromEmail || !subject) {
      logger.warn('SendGrid webhook: missing required fields', { from, subject });
      res.status(200).json({ success: true, skipped: true }); // Don't retry
      return;
    }
    
    const body = text || (html ? html.replace(/<[^>]*>/g, ' ').trim() : '');
    
    logger.info('SendGrid webhook received', { fromEmail, subject });
    
    const orchestrator = await getOrchestrator();
    
    // Classify the incoming email
    const classification = await orchestrator.classifyEmail({
      fromEmail,
      subject,
      body,
    });
    
    // CLOSED LOOP #1: If positive response, record success for the bandit arm
    if (classification.category === 'INTERESTED' || classification.category === 'MEETING') {
      // Find the contact by email
      const contacts = await queryReadOnly<{ id: string }>(`
        SELECT id FROM sa_contacts WHERE LOWER(email) = $1 LIMIT 1
      `, [fromEmail]);
      
      if (contacts.length > 0) {
        // Find the original outbound message
        const outbound = await queryReadOnly<{ id: string; subject_arm_id: string | null }>(`
          SELECT id, subject_arm_id FROM sa_outbound_messages 
          WHERE contact_id = $1 AND subject_arm_id IS NOT NULL
          ORDER BY sent_at DESC LIMIT 1
        `, [contacts[0].id]);
        
        if (outbound.length > 0 && outbound[0].subject_arm_id) {
          const { recordOutcome } = await import('../services/math/thompson-sampler.js');
          await recordOutcome(outbound[0].subject_arm_id, true);
          
          // Update the outbound message with reply timestamp
          await queryOperator(`
            UPDATE sa_outbound_messages 
            SET replied_at = CURRENT_TIMESTAMP, status = 'replied'
            WHERE id = $1
          `, [outbound[0].id]);
          
          logger.info('SendGrid webhook: Recorded positive bandit outcome', {
            fromEmail,
            category: classification.category,
            armId: outbound[0].subject_arm_id,
          });
        }
      }
    }
    
    // CLOSED LOOP #2: Save ALL classified replies to sa_bayes_training for Naive Bayes learning
    // This grows our training corpus organically from real interactions
    const combinedText = `${subject} ${body}`;
    
    if (classification.confidence >= 0.6 || classification.category !== 'UNKNOWN') {
      try {
        await queryOperator(`
          INSERT INTO sa_bayes_training (label, text_content, source)
          VALUES ($1, $2, 'auto_webhook')
          ON CONFLICT DO NOTHING
        `, [classification.category.toLowerCase(), combinedText.substring(0, 4000)]);
        
        logger.info('SendGrid webhook: Saved reply to Naive Bayes training', {
          category: classification.category,
          confidence: classification.confidence,
        });
      } catch (trainingErr) {
        logger.warn('Failed to save training data', { error: (trainingErr as Error).message });
      }
    }
    
    res.json({ success: true, classification: classification.category });
  } catch (err) {
    logger.error('SendGrid webhook failed', { error: (err as Error).message });
    // Return 200 to prevent SendGrid retries on our errors
    res.status(200).json({ success: false, error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/webhook/email-events
 * Generic email events webhook (works with SendGrid, Mailgun, Postmark)
 * 
 * Receives email tracking events (opens, clicks, bounces).
 * Maps events to Thompson Sampler outcomes.
 */
router.post('/webhook/email-events', async (req: Request, res: Response) => {
  try {
    // Normalize event from various providers
    const events = Array.isArray(req.body) ? req.body : [req.body];
    
    for (const event of events) {
      // Extract event type (normalize across providers)
      let eventType = event.event || event.type || event.RecordType || '';
      eventType = eventType.toLowerCase();
      
      // Map to our internal event types
      let internalType: 'open' | 'click' | 'reply' | 'bounce' | 'unsubscribe' | null = null;
      
      if (eventType.includes('open')) internalType = 'open';
      else if (eventType.includes('click')) internalType = 'click';
      else if (eventType.includes('bounce') || eventType.includes('fail')) internalType = 'bounce';
      else if (eventType.includes('unsubscribe') || eventType.includes('unsub')) internalType = 'unsubscribe';
      else if (eventType.includes('reply') || eventType.includes('inbound')) internalType = 'reply';
      
      if (!internalType) continue;
      
      // Extract email from various possible fields
      const email = (
        event.email || 
        event.recipient || 
        event.Recipient || 
        event.to || 
        ''
      ).toLowerCase();
      
      if (!email) continue;
      
      // Find contact and their last outbound message
      const contacts = await queryReadOnly<{ id: string }>(`
        SELECT id FROM sa_contacts WHERE LOWER(email) = $1 LIMIT 1
      `, [email]);
      
      if (contacts.length === 0) continue;
      
      const outbound = await queryReadOnly<{ subject_arm_id: string }>(`
        SELECT subject_arm_id FROM sa_outbound_messages 
        WHERE contact_id = $1 AND subject_arm_id IS NOT NULL
        ORDER BY sent_at DESC LIMIT 1
      `, [contacts[0].id]);
      
      if (outbound.length === 0 || !outbound[0].subject_arm_id) continue;
      
      // Determine if this is a positive outcome
      const positiveEvents = ['open', 'click', 'reply'];
      const negativeEvents = ['bounce', 'unsubscribe'];
      
      // PERSIST EVENT: Store in sa_email_events for audit trail and analytics
      // Find the most recent outbound message for this contact
      const outboundMsg = await queryReadOnly<{ id: string; subject_arm_id: string | null }>(`
        SELECT id, subject_arm_id FROM sa_outbound_messages 
        WHERE contact_id = $1
        ORDER BY created_at DESC LIMIT 1
      `, [contacts[0].id]);
      
      const outboundMsgId = outboundMsg.length > 0 ? outboundMsg[0].id : null;
      const armId = outbound[0].subject_arm_id;
      
      // Save event to sa_email_events table for analytics and audit
      try {
        await queryOperator(`
          INSERT INTO sa_email_events 
          (outbound_message_id, contact_id, event_type, arm_id, outcome_recorded, source, raw_payload)
          VALUES ($1, $2, $3, $4, $5, 'webhook', $6)
        `, [
          outboundMsgId,
          contacts[0].id,
          internalType,
          armId,
          positiveEvents.includes(internalType) || negativeEvents.includes(internalType),
          JSON.stringify(event),
        ]);
      } catch (persistErr) {
        // Don't fail the webhook if event persistence fails
        logger.warn('Failed to persist email event', { error: (persistErr as Error).message });
      }
      
      // Update Thompson Sampler
      const { recordOutcome } = await import('../services/math/thompson-sampler.js');
      
      if (positiveEvents.includes(internalType)) {
        await recordOutcome(armId, true);
        logger.info('Email event webhook: Recorded positive outcome', {
          email, eventType: internalType, armId,
        });
      } else if (negativeEvents.includes(internalType)) {
        await recordOutcome(armId, false);
        logger.info('Email event webhook: Recorded negative outcome', {
          email, eventType: internalType, armId,
        });
      }
    }
    
    res.json({ success: true, processed: events.length });
  } catch (err) {
    logger.error('Email events webhook failed', { error: (err as Error).message });
    res.status(200).json({ success: false, error: (err as Error).message });
  }
});

// ============================================================================
// STATS & MONITORING
// ============================================================================

/**
 * GET /api/autonomous/stats
 * Get overall system statistics
 */
router.get('/stats', async (_req: Request, res: Response) => {
  try {
    const orchestrator = await getOrchestrator();
    const stats = await orchestrator.getSystemStats();
    
    res.json({
      success: true,
      stats,
    });
  } catch (err) {
    const error = err as Error;
    logger.error('Stats fetch failed', { 
      error: error.message,
      stack: error.stack,
    });
    res.status(500).json({ error: error.message, stack: error.stack });
  }
});

// ============================================================================
// THOMPSON SAMPLER MANAGEMENT
// ============================================================================

/**
 * GET /api/autonomous/arms
 * Get all bandit arms for a type
 */
router.get('/arms', async (req: Request, res: Response) => {
  try {
    const armType = (req.query.type as string) || 'subject_line';
    const arms = await getActiveArms(armType);
    const stats = await getBanditStats(armType);
    
    res.json({
      success: true,
      arms,
      stats,
    });
  } catch (err) {
    logger.error('Arms fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/arms
 * Create a new bandit arm
 */
router.post('/arms', async (req: Request, res: Response) => {
  try {
    const { armType, armName, armValue, priorAlpha, priorBeta } = req.body;
    
    if (!armType || !armName || !armValue) {
      res.status(400).json({ error: 'armType, armName, and armValue are required' });
      return;
    }
    
    const armId = await createArm({
      armType,
      armName,
      armValue,
      priorAlpha,
      priorBeta,
    });
    
    res.json({
      success: true,
      armId,
    });
  } catch (err) {
    logger.error('Arm creation failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// SPINTAX TEMPLATES
// ============================================================================

/**
 * GET /api/autonomous/templates
 * Get spintax templates by type
 */
router.get('/templates', async (req: Request, res: Response) => {
  try {
    const templateType = (req.query.type as string) || 'initial_outreach';
    const templates = await getTemplates(templateType);
    
    res.json({
      success: true,
      templates,
    });
  } catch (err) {
    logger.error('Templates fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/templates
 * Create a new spintax template
 */
router.post('/templates', async (req: Request, res: Response) => {
  try {
    const { templateType, name, description, subjectSpintax, bodySpintax, availableVariables } = req.body;
    
    if (!templateType || !name || !subjectSpintax || !bodySpintax) {
      res.status(400).json({ 
        error: 'templateType, name, subjectSpintax, and bodySpintax are required' 
      });
      return;
    }
    
    const templateId = await createTemplate({
      templateType,
      name,
      description,
      subjectSpintax,
      bodySpintax,
      availableVariables,
    });
    
    res.json({
      success: true,
      templateId,
    });
  } catch (err) {
    logger.error('Template creation failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/templates/preview
 * Preview spintax variations
 */
router.post('/templates/preview', async (req: Request, res: Response) => {
  try {
    const { subjectSpintax, bodySpintax, context, count = 5 } = req.body;
    
    if (!subjectSpintax || !bodySpintax) {
      res.status(400).json({ error: 'subjectSpintax and bodySpintax are required' });
      return;
    }
    
    const variations = previewVariations(
      subjectSpintax,
      bodySpintax,
      context || {},
      count
    );
    
    res.json({
      success: true,
      variations,
    });
  } catch (err) {
    logger.error('Preview failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/templates/seed
 * Seed essential email templates (initial outreach + follow-ups)
 * 
 * This ensures the system has a complete email sequence:
 *   - initial_outreach: First contact
 *   - followup_1: Soft reminder (3-5 days after initial)
 *   - followup_2: Value-add follow-up (7-10 days after initial)
 * 
 * Idempotent: Will skip templates that already exist.
 */
router.post('/templates/seed', async (req: Request, res: Response) => {
  try {
    const force = req.body.force === true;
    
    // Define template library
    // IMPROVED (Jan 2026):
    // - Added opt-out line to all templates
    // - Fixed sender consistency (Aaron at ApexMediation)
    // - Softened unverified percentage claims
    // - Removed fake "Re:" prefix from follow-ups (deceptive practice)
    const templateLibrary = [
      {
        name: 'Initial Outreach - Ad Mediation',
        templateType: 'initial_outreach',
        description: 'First touch template for app developers using ad monetization',
        subjectSpintax: '{{app_name}} {monetization|ad revenue|ad strategy} question',
        bodySpintax: `{Hi|Hello|Hey} {{first_name}},

{I noticed|I came across|I saw} {{app_name}} on the {App Store|Play Store|app stores} and {was interested in|wanted to learn more about|was curious about} your {approach to|work on} {{app_genre}}.

{I'm reaching out because|The reason for my email is|I'm writing because} we {provide|offer} {ad mediation services|programmatic ad technology|demand aggregation} for {mobile publishers|app developers|independent studios}.

{We focus on|Our service centers on|What we do is} {connecting apps with multiple demand sources|aggregating demand from major ad networks|simplifying ad operations} - {though results vary by app|every integration is different|outcomes depend on many factors including your existing setup}.

{Would you be open to|Are you free for|Do you have 15 minutes for} a {quick chat|brief call|short conversation} to see if {exploring this makes sense|our approach might fit|there's potential alignment}?

{Best|Cheers|Thanks},
Aaron at ApexMediation

{P.S. Reply NO to opt out - I won't follow up.|P.S. Not interested? Reply NO and I'll remove you from my list.}`,
        availableVariables: ['first_name', 'app_name', 'app_genre', 'company_name'],
      },
      {
        name: 'Follow-up 1 - Gentle Reminder',
        templateType: 'followup_1',
        description: 'First follow-up (3-5 days after initial). Soft reminder with value hook.',
        subjectSpintax: '{Following up:|Checking in -} {{app_name}} monetization',
        bodySpintax: `{Hi|Hey} {{first_name}},

{Wanted to follow up on my previous note|Just circling back on my earlier email|Checking in on my message from last week}.

{I know things get busy|Inboxes fill up quickly|I imagine you're juggling a lot}, so {I wanted to resurface this|thought I'd bump this up|wanted to make sure this didn't get lost}.

{We've been helping|Recently we worked with|We just helped} {several indie studios|a few mobile publishers|apps similar to {{app_name}}} {improve their ad revenue|optimize their fill rates|streamline their monetization stack}. {The common thread is|What we found is|The pattern we see is} {better demand competition|more efficient mediation|smarter waterfall management}.

{If you're curious|If this sounds relevant|If you'd like to learn more}, {I'm happy to|I'd be glad to|I can} {share some specifics|walk you through our approach|show you what we're seeing}.

{No pressure either way|Totally understand if timing isn't right|Feel free to ignore if not a fit}.

{Best|Cheers},
Aaron at ApexMediation

{P.S. Reply NO to opt out.|P.S. Not interested? Just reply NO.}`,
        availableVariables: ['first_name', 'app_name', 'app_genre', 'company_name'],
      },
      {
        name: 'Follow-up 2 - Value Add',
        templateType: 'followup_2',
        description: 'Second follow-up (7-10 days after initial). Provide concrete value/insight.',
        subjectSpintax: '{Quick insight for|Thought you might find this useful -} {{app_name}}',
        bodySpintax: `{Hi|Hey} {{first_name}},

{One more follow-up|Last note from me|Final check-in} - {then I'll leave you be|then I'll step back|no more messages after this}.

{I was looking at|I took a closer look at|I analyzed} {{app_genre}} apps {this week|recently|in our network} and {noticed a trend|saw something interesting|found a pattern} that {might be relevant for|could apply to|you might find useful for} {{app_name}}.

{Apps in your category|Publishers in {{app_genre}}|Similar apps} that {switched to mediation|optimized their demand stack|added multiple demand sources} {often see meaningful improvements|typically report better results|generally experience positive changes} {in fill rates and eCPMs|with more competitive bidding|from better demand competition}.

{I'm not suggesting|This isn't to say|Obviously this doesn't mean} {{app_name}} would see {the same results|identical outcomes|guaranteed gains} - {every app is different|context matters|there are many variables}. But {if you're ever curious|if this is on your radar|if you want to explore}, {happy to share more|I can walk you through the data|the door is open}.

{Either way, best of luck with {{app_name}}|Wishing you continued success|Hope {{app_name}} keeps growing}!

{Best|Cheers|All the best},
Aaron at ApexMediation

{P.S. Reply NO to opt out.|P.S. Not interested? Just reply NO.}`,
        availableVariables: ['first_name', 'app_name', 'app_genre', 'company_name'],
      },
      {
        name: 'Follow-up 3 - Breakup Email',
        templateType: 'followup_3',
        description: 'Final follow-up. Respectful close, leaves door open.',
        subjectSpintax: '{Closing the loop|Last note|Should I close your file?}',
        bodySpintax: `{Hi|Hey} {{first_name}},

{I've reached out a few times|I've sent a few notes|I wanted to touch base one last time} about {{app_name}} and {ad monetization|our mediation services|potential partnership}.

{Since I haven't heard back|Given the silence|Since there's been no response}, I'm {assuming the timing isn't right|guessing this isn't a priority|taking that as a soft no}.

{Totally understood|No worries at all|Completely get it}. {I'll close this out on my end|I'll stop reaching out|I'll remove you from my list}.

{If things ever change|Should priorities shift|If you're ever curious down the road}, {feel free to reach out|my inbox is always open|just drop me a line} at sales@apexmediation.ee.

{Best of luck with {{app_name}}|Wishing you success|All the best}!

{Cheers|Best|Thanks},
Aaron at ApexMediation`,
        availableVariables: ['first_name', 'app_name', 'app_genre', 'company_name'],
      },
    ];
    
    // Check existing templates
    const existingTemplates = await queryReadOnly<{ template_type: string; name: string }>(`
      SELECT template_type, name FROM sa_spintax_templates WHERE active = true
    `);
    
    const existingTypes = new Set(existingTemplates.map(t => t.template_type));
    
    let inserted = 0;
    let skipped = 0;
    const results: Array<{ name: string; status: string }> = [];
    
    for (const template of templateLibrary) {
      // Skip if exists and not forcing
      if (existingTypes.has(template.templateType) && !force) {
        results.push({ name: template.name, status: 'skipped (exists)' });
        skipped++;
        continue;
      }
      
      // Delete existing if forcing
      if (force && existingTypes.has(template.templateType)) {
        await queryOperator(`
          UPDATE sa_spintax_templates SET active = false 
          WHERE template_type = $1
        `, [template.templateType]);
      }
      
      // Create template
      await createTemplate({
        name: template.name,
        templateType: template.templateType,
        description: template.description,
        subjectSpintax: template.subjectSpintax,
        bodySpintax: template.bodySpintax,
        availableVariables: template.availableVariables,
      });
      
      results.push({ name: template.name, status: 'created' });
      inserted++;
    }
    
    logger.info('Template seeding completed', { inserted, skipped });
    
    res.json({
      success: true,
      message: `Seeded ${inserted} templates, skipped ${skipped}`,
      results,
    });
    
  } catch (err) {
    logger.error('Template seeding failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// INBOX REVIEW
// ============================================================================

/**
 * GET /api/autonomous/inbox
 * Get messages needing review
 */
router.get('/inbox', async (req: Request, res: Response) => {
  try {
    const limit = parseInt(req.query.limit as string) || 20;
    const messages = await getMessagesForReview(limit);
    const stats = await getClassificationStats();
    
    res.json({
      success: true,
      messages,
      stats,
    });
  } catch (err) {
    logger.error('Inbox fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/inbox/:id
 * Submit review for a message
 */
router.post('/inbox/:id', async (req: Request, res: Response) => {
  try {
    const { id } = req.params;
    const { category } = req.body;
    
    if (!category) {
      res.status(400).json({ error: 'category is required' });
      return;
    }
    
    await submitReview(id, category);
    
    res.json({ success: true });
  } catch (err) {
    logger.error('Review submission failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// SOURCES (CREATE/DELETE - GET is handled above with ?due param)
// ============================================================================

/**
 * POST /api/autonomous/sources
 * Create a new lead source
 */
router.post('/sources', async (req: Request, res: Response) => {
  try {
    const { sourceName, sourceType, sourceConfig, scrapeIntervalHours } = req.body;
    
    if (!sourceName || !sourceType) {
      res.status(400).json({ error: 'sourceName and sourceType are required' });
      return;
    }
    
    // Extract config fields for schema compatibility
    const searchQuery = sourceConfig?.searchTerm || sourceConfig?.search_term || null;
    const category = sourceConfig?.category || null;
    const region = sourceConfig?.region || null;
    
    const result = await queryOperator<{ id: string }>(`
      INSERT INTO sa_sources (source_name, source_type, search_query, category, region, scrape_interval_hours, status)
      VALUES ($1, $2, $3, $4, $5, $6, 'active')
      RETURNING id
    `, [sourceName, sourceType, searchQuery, category, region, scrapeIntervalHours || 24]);
    
    logger.info('Created lead source', { sourceName, sourceType });
    
    res.json({
      success: true,
      sourceId: result[0]?.id,
    });
  } catch (err) {
    logger.error('Source creation failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * DELETE /api/autonomous/sources/:id
 * Deactivate a lead source (soft delete)
 */
router.delete('/sources/:id', async (req: Request, res: Response) => {
  try {
    const { id } = req.params;
    
    await queryOperator(`UPDATE sa_sources SET status = 'deleted' WHERE id = $1`, [id]);
    
    logger.info('Deleted lead source', { sourceId: id });
    
    res.json({ success: true });
  } catch (err) {
    logger.error('Source deletion failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// HOT LEADS
// ============================================================================

/**
 * GET /api/autonomous/hot-leads
 * Get high-scoring leads ready for outreach
 * 
 * Query params:
 *   - minScore: minimum score threshold (default: 50)
 *   - limit: max results (default: 20)
 *   - status: filter by status (default: 'new')
 */
router.get('/hot-leads', async (req: Request, res: Response) => {
  try {
    const minScore = parseInt(req.query.minScore as string) || 50;
    const limit = parseInt(req.query.limit as string) || 20;
    const status = req.query.status as string || 'new';
    
    const leads = await queryReadOnly<{
      id: string;
      email: string;
      first_name: string | null;
      last_name: string | null;
      company_name: string | null;
      app_name: string | null;
      score: number;
      status: string;
      source: string | null;
      created_at: string;
    }>(`
      SELECT 
        id, email, first_name, last_name, company_name, 
        app_name, score, status, source, created_at
      FROM sa_contacts
      WHERE score >= $1 
        AND ($2 = 'all' OR status = $2)
      ORDER BY score DESC, created_at DESC
      LIMIT $3
    `, [minScore, status, limit]);
    
    res.json({
      success: true,
      leads,
      count: leads.length,
    });
  } catch (err) {
    logger.error('Hot leads fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * GET /api/autonomous/competitor-leads
 * Get leads using specific competitors (THE SNIPER ENDPOINT)
 * 
 * This is the highest-value endpoint. It returns leads that are
 * actively using a competitor network we want to displace.
 * 
 * Query params:
 *   - competitor: filter by competitor name (optional)
 *   - tier: filter by competitor tier 1/2/3 (default: all)
 *   - minScore: minimum overall score (default: 30)
 *   - limit: max results (default: 50)
 */
router.get('/competitor-leads', async (req: Request, res: Response) => {
  try {
    const competitor = req.query.competitor as string;
    const tier = req.query.tier ? parseInt(req.query.tier as string) : null;
    const minScore = parseInt(req.query.minScore as string) || 30;
    const limit = parseInt(req.query.limit as string) || 50;
    
    let query = `
      SELECT 
        c.id, c.email, c.first_name, c.last_name, c.company_name,
        c.app_name, c.app_genre, c.score, c.status,
        c.top_competitor, c.competitor_tier,
        cd.competitor_domain
      FROM sa_contacts c
      JOIN sa_competitor_detections cd ON cd.contact_id = c.id
      WHERE c.score >= $1
        AND c.has_competitor = true
    `;
    const params: (string | number)[] = [minScore];
    let paramIndex = 2;
    
    if (competitor) {
      query += ` AND LOWER(cd.competitor_name) LIKE LOWER($${paramIndex})`;
      params.push(`%${competitor}%`);
      paramIndex++;
    }
    
    if (tier) {
      query += ` AND cd.competitor_tier = $${paramIndex}`;
      params.push(tier);
      paramIndex++;
    }
    
    query += ` ORDER BY cd.competitor_tier ASC, c.score DESC LIMIT $${paramIndex}`;
    params.push(limit);
    
    const leads = await queryReadOnly<{
      id: string;
      email: string;
      first_name: string | null;
      last_name: string | null;
      company_name: string | null;
      app_name: string | null;
      app_genre: string | null;
      score: number;
      status: string;
      top_competitor: string | null;
      competitor_tier: number | null;
      competitor_domain: string;
    }>(query, params);
    
    // Group by competitor for easy targeting
    const byCompetitor: Record<string, typeof leads> = {};
    for (const lead of leads) {
      const comp = lead.top_competitor || 'unknown';
      if (!byCompetitor[comp]) byCompetitor[comp] = [];
      byCompetitor[comp].push(lead);
    }
    
    res.json({
      success: true,
      leads,
      count: leads.length,
      byCompetitor,
      competitorStats: Object.entries(byCompetitor).map(([name, l]) => ({
        competitor: name,
        count: l.length,
        avgScore: Math.round(l.reduce((sum, x) => sum + x.score, 0) / l.length),
      })),
    });
  } catch (err) {
    logger.error('Competitor leads fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/competitor-scan
 * Scan all contacts with app-ads.txt for competitor SDKs
 * 
 * This is a batch job to:
 * 1. Re-scan contacts that have app_ads_txt_content
 * 2. Detect any competitor networks in use
 * 3. Save detections to sa_competitor_detections
 * 
 * Run periodically to ensure competitor data is up-to-date.
 */
router.post('/competitor-scan', async (req: Request, res: Response) => {
  try {
    const limit = parseInt(req.body.limit) || 200;
    
    // Known competitor domains
    const TARGET_COMPETITORS = [
      { domain: 'ironsrc.com', name: 'IronSource', tier: 1 },
      { domain: 'is.com', name: 'IronSource (alt)', tier: 1 },
      { domain: 'unity.com', name: 'Unity Ads', tier: 1 },
      { domain: 'unityads.unity3d.com', name: 'Unity Ads (alt)', tier: 1 },
      { domain: 'applovin.com', name: 'AppLovin', tier: 1 },
      { domain: 'max.com', name: 'AppLovin MAX', tier: 1 },
      { domain: 'mintegral.com', name: 'Mintegral', tier: 2 },
      { domain: 'vungle.com', name: 'Vungle', tier: 2 },
      { domain: 'chartboost.com', name: 'Chartboost', tier: 2 },
      { domain: 'fyber.com', name: 'Fyber', tier: 2 },
      { domain: 'inmobi.com', name: 'InMobi', tier: 2 },
      { domain: 'adcolony.com', name: 'AdColony', tier: 3 },
      { domain: 'tapjoy.com', name: 'Tapjoy', tier: 3 },
      { domain: 'startapp.com', name: 'StartApp', tier: 3 },
      { domain: 'ogury.co', name: 'Ogury', tier: 3 },
    ];
    
    // Get contacts with ads.txt content
    const contacts = await queryReadOnly<{
      id: string;
      app_ads_txt_content: string;
    }>(`
      SELECT id, app_ads_txt_content
      FROM sa_contacts
      WHERE app_ads_txt_content IS NOT NULL
        AND app_ads_txt_content != ''
      ORDER BY score DESC
      LIMIT $1
    `, [limit]);
    
    logger.info('Starting competitor scan', { contactCount: contacts.length });
    
    let scanned = 0;
    let detectionsCreated = 0;
    const competitorCounts: Record<string, number> = {};
    
    for (const contact of contacts) {
      const content = contact.app_ads_txt_content.toLowerCase();
      const detected: typeof TARGET_COMPETITORS = [];
      
      for (const competitor of TARGET_COMPETITORS) {
        if (content.includes(competitor.domain)) {
          detected.push(competitor);
          competitorCounts[competitor.name] = (competitorCounts[competitor.name] || 0) + 1;
        }
      }
      
      if (detected.length > 0) {
        // Save detections
        for (const comp of detected) {
          try {
            await queryOperator(`
              INSERT INTO sa_competitor_detections (contact_id, competitor_domain, competitor_name, competitor_tier)
              VALUES ($1, $2, $3, $4)
              ON CONFLICT (contact_id, competitor_domain) DO UPDATE SET
                competitor_tier = $4,
                created_at = CURRENT_TIMESTAMP
            `, [contact.id, comp.domain, comp.name, comp.tier]);
            detectionsCreated++;
          } catch {
            // Ignore conflicts
          }
        }
        
        // Update contact with best competitor
        const topCompetitor = [...detected].sort((a, b) => a.tier - b.tier)[0];
        await queryOperator(`
          UPDATE sa_contacts 
          SET has_competitor = true, top_competitor = $1, competitor_tier = $2
          WHERE id = $3
        `, [topCompetitor.name, topCompetitor.tier, contact.id]);
      }
      
      scanned++;
    }
    
    logger.info('Competitor scan completed', {
      scanned,
      detectionsCreated,
      competitorCounts,
    });
    
    res.json({
      success: true,
      scanned,
      detectionsCreated,
      competitorCounts,
      topCompetitors: Object.entries(competitorCounts)
        .sort((a, b) => b[1] - a[1])
        .slice(0, 10)
        .map(([name, count]) => ({ name, count })),
    });
    
  } catch (err) {
    logger.error('Competitor scan failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// EDGE INTELLIGENCE V2 ENDPOINTS
// ============================================================================

/**
 * POST /api/autonomous/icebreaker/generate
 * Generate a hyper-personalized icebreaker using local LLM
 * 
 * This runs on-device using ONNX (Flan-T5-Small).
 * Falls back to template-based icebreakers if model unavailable.
 */
router.post('/icebreaker/generate', async (req: Request, res: Response) => {
  try {
    const { appName, appDescription, genre, contactId } = req.body;
    
    // Can generate by contactId or by direct app info
    if (contactId) {
      const { generateIcebreakerForContact } = await import('../services/messaging/icebreaker-engine.js');
      const result = await generateIcebreakerForContact(contactId);
      
      if (!result) {
        res.status(404).json({ error: 'Contact not found or no app info available' });
        return;
      }
      
      res.json({
        success: true,
        icebreaker: result.icebreaker,
        source: result.source,
        latencyMs: result.latencyMs,
        modelUsed: result.modelUsed,
      });
      return;
    }
    
    if (!appName) {
      res.status(400).json({ error: 'appName or contactId required' });
      return;
    }
    
    const { IcebreakerEngine } = await import('../services/messaging/icebreaker-engine.js');
    const engine = await IcebreakerEngine.getInstance();
    
    const result = await engine.generate({
      appName,
      appDescription,
      genre,
    });
    
    res.json({
      success: true,
      icebreaker: result.icebreaker,
      source: result.source,
      latencyMs: result.latencyMs,
      modelUsed: result.modelUsed,
    });
  } catch (err) {
    logger.error('Icebreaker generation failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/icebreaker/batch
 * Batch generate icebreakers for contacts missing them
 */
router.post('/icebreaker/batch', async (req: Request, res: Response) => {
  try {
    const limit = parseInt(req.body.limit) || 10;
    
    const { batchGenerateIcebreakers } = await import('../services/messaging/icebreaker-engine.js');
    const result = await batchGenerateIcebreakers(limit);
    
    res.json({
      success: true,
      ...result,
    });
  } catch (err) {
    logger.error('Batch icebreaker generation failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * GET /api/autonomous/icebreaker/status
 * Get LLM model status
 */
router.get('/icebreaker/status', async (_req: Request, res: Response) => {
  try {
    const { IcebreakerEngine } = await import('../services/messaging/icebreaker-engine.js');
    const engine = await IcebreakerEngine.getInstance();
    const status = engine.getStatus();
    
    res.json({
      success: true,
      status,
    });
  } catch (err) {
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/icebreaker/warmup
 * Pre-warm the LLM model
 */
router.post('/icebreaker/warmup', async (_req: Request, res: Response) => {
  try {
    const { IcebreakerEngine } = await import('../services/messaging/icebreaker-engine.js');
    const engine = await IcebreakerEngine.getInstance();
    const result = await engine.warmup();
    
    res.json(result);
  } catch (err) {
    logger.error('LLM warmup failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/classify/hybrid
 * Classify email using hybrid pipeline (Naive Bayes + ONNX DistilBERT)
 * 
 * This provides defense-in-depth classification:
 *   - Fast pass: Naive Bayes (< 1ms)
 *   - Deep pass: ONNX DistilBERT (~500ms) if confidence low
 */
router.post('/classify/hybrid', async (req: Request, res: Response) => {
  try {
    const { subject, body, fromEmail, contactId } = req.body;
    
    if (!subject || !body) {
      res.status(400).json({ error: 'subject and body required' });
      return;
    }
    
    // Get the clerk instance and wrap with hybrid classifier
    const orchestrator = await getOrchestrator();
    const clerk = (orchestrator as any).clerk;
    
    if (!clerk) {
      res.status(500).json({ error: 'Clerk not initialized' });
      return;
    }
    
    const { createHybridClassifier } = await import('../services/inbox/hybrid-classifier.js');
    const hybridClassifier = createHybridClassifier(clerk);
    
    const result = await hybridClassifier.classify(subject, body);
    
    // Optionally save the classification
    if (fromEmail) {
      const { saveMessageClassification } = await import('../services/inbox/clerk.js');
      await saveMessageClassification({
        contactId,
        fromEmail,
        subject,
        body,
        classification: result,
      });
    }
    
    // Save audit record
    try {
      await queryOperator(`
        INSERT INTO sa_classification_audit 
        (fast_pass_category, fast_pass_confidence, deep_pass_category, deep_pass_confidence,
         final_category, final_confidence, consensus_reached, processing_mode,
         latency_ms_fast, latency_ms_deep, latency_ms_total)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
      `, [
        result.fastPassCategory,
        result.fastPassConfidence,
        result.deepPassCategory || null,
        result.deepPassConfidence || null,
        result.category,
        result.confidence,
        result.consensusReached,
        result.processingMode,
        result.latencyMs.fastPass,
        result.latencyMs.deepPass || null,
        result.latencyMs.total,
      ]);
    } catch {
      // Non-critical
    }
    
    res.json({
      success: true,
      classification: {
        category: result.category,
        confidence: result.confidence,
        requiresReview: result.requiresReview,
        fastPassCategory: result.fastPassCategory,
        fastPassConfidence: result.fastPassConfidence,
        deepPassCategory: result.deepPassCategory,
        deepPassConfidence: result.deepPassConfidence,
        consensusReached: result.consensusReached,
        processingMode: result.processingMode,
        latencyMs: result.latencyMs,
      },
    });
  } catch (err) {
    logger.error('Hybrid classification failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * GET /api/autonomous/classify/hybrid/status
 * Get hybrid classifier status
 */
router.get('/classify/hybrid/status', async (_req: Request, res: Response) => {
  try {
    const orchestrator = await getOrchestrator();
    const clerk = (orchestrator as any).clerk;
    
    if (!clerk) {
      res.json({
        success: true,
        status: {
          clerkInitialized: false,
          deepModelLoaded: false,
          deepModelLoading: false,
          deepModelError: 'Clerk not initialized',
        },
      });
      return;
    }
    
    const { createHybridClassifier } = await import('../services/inbox/hybrid-classifier.js');
    const hybridClassifier = createHybridClassifier(clerk);
    const status = hybridClassifier.getStatus();
    
    res.json({
      success: true,
      status: {
        clerkInitialized: true,
        ...status,
      },
    });
  } catch (err) {
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/classify/hybrid/warmup
 * Pre-warm the ONNX classification model
 */
router.post('/classify/hybrid/warmup', async (_req: Request, res: Response) => {
  try {
    const orchestrator = await getOrchestrator();
    const clerk = (orchestrator as any).clerk;
    
    const { createHybridClassifier } = await import('../services/inbox/hybrid-classifier.js');
    const hybridClassifier = createHybridClassifier(clerk);
    const result = await hybridClassifier.warmup();
    
    res.json({
      success: true,
      ...result,
    });
  } catch (err) {
    logger.error('Hybrid classifier warmup failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * GET /api/autonomous/reputation/health
 * Get reputation health report
 */
router.get('/reputation/health', async (_req: Request, res: Response) => {
  try {
    const { getReputationGuard } = await import('../services/reputation/reputation-guard.js');
    const guard = getReputationGuard();
    const report = await guard.getHealthReport();
    
    res.json({
      success: true,
      report,
    });
  } catch (err) {
    logger.error('Reputation health check failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * GET /api/autonomous/reputation/metrics
 * Get reputation metrics for a period
 */
router.get('/reputation/metrics', async (req: Request, res: Response) => {
  try {
    const period = (req.query.period as 'hour' | 'day' | 'week') || 'day';
    
    const { getReputationGuard } = await import('../services/reputation/reputation-guard.js');
    const guard = getReputationGuard();
    const metrics = await guard.getMetrics(period);
    
    res.json({
      success: true,
      metrics,
    });
  } catch (err) {
    logger.error('Reputation metrics fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/reputation/analyze-bounce
 * Analyze a bounce message for reputation signals
 */
router.post('/reputation/analyze-bounce', async (req: Request, res: Response) => {
  try {
    const { messageId, subject, body, contactId } = req.body;
    
    if (!subject || !body) {
      res.status(400).json({ error: 'subject and body required' });
      return;
    }
    
    const { getReputationGuard } = await import('../services/reputation/reputation-guard.js');
    const guard = getReputationGuard();
    const analysis = await guard.processBounce(messageId, subject, body, contactId);
    
    res.json({
      success: true,
      analysis,
    });
  } catch (err) {
    logger.error('Bounce analysis failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * GET /api/autonomous/reputation/alerts
 * Get recent reputation alerts
 */
router.get('/reputation/alerts', async (req: Request, res: Response) => {
  try {
    const limit = parseInt(req.query.limit as string) || 10;
    
    const { getReputationGuard } = await import('../services/reputation/reputation-guard.js');
    const guard = getReputationGuard();
    const alerts = await guard.getRecentAlerts(limit);
    
    res.json({
      success: true,
      alerts,
    });
  } catch (err) {
    logger.error('Reputation alerts fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/reputation/alerts/:id/acknowledge
 * Acknowledge a reputation alert
 */
router.post('/reputation/alerts/:id/acknowledge', async (req: Request, res: Response) => {
  try {
    const { id } = req.params;
    
    const { getReputationGuard } = await import('../services/reputation/reputation-guard.js');
    const guard = getReputationGuard();
    await guard.acknowledgeAlert(id);
    
    res.json({ success: true });
  } catch (err) {
    logger.error('Alert acknowledgment failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * GET /api/autonomous/edge/status
 * Get combined status of all edge intelligence features
 */
router.get('/edge/status', async (_req: Request, res: Response) => {
  try {
    // Get icebreaker status
    const { IcebreakerEngine } = await import('../services/messaging/icebreaker-engine.js');
    const icebreakerEngine = await IcebreakerEngine.getInstance();
    const icebreakerStatus = icebreakerEngine.getStatus();
    
    // Get hybrid classifier status
    const orchestrator = await getOrchestrator();
    const clerk = (orchestrator as any).clerk;
    let hybridStatus: { deepModelLoaded: boolean; deepModelLoading: boolean; deepModelError: string | null } = 
      { deepModelLoaded: false, deepModelLoading: false, deepModelError: 'Clerk not initialized' };
    
    if (clerk) {
      const { createHybridClassifier } = await import('../services/inbox/hybrid-classifier.js');
      const hybridClassifier = createHybridClassifier(clerk);
      hybridStatus = hybridClassifier.getStatus();
    }
    
    // Get reputation status
    const { getReputationGuard } = await import('../services/reputation/reputation-guard.js');
    const guard = getReputationGuard();
    const reputationMetrics = await guard.getMetrics('day');
    
    res.json({
      success: true,
      edge: {
        icebreaker: {
          name: 'Hyper-Personalization LLM',
          model: 'Xenova/LaMini-Flan-T5-248M',
          loaded: icebreakerStatus.modelLoaded,
          loading: icebreakerStatus.modelLoading,
          error: icebreakerStatus.modelError,
        },
        hybridClassifier: {
          name: 'Hybrid Email Classifier',
          model: 'Xenova/distilbert-base-uncased-finetuned-sst-2-english',
          loaded: hybridStatus.deepModelLoaded,
          loading: hybridStatus.deepModelLoading,
          error: hybridStatus.deepModelError,
        },
        reputationGuard: {
          name: 'DMARC/SPF Reputation Monitor',
          enabled: true,
          riskLevel: reputationMetrics.risk,
          reputationScore: reputationMetrics.reputationScore,
          bounceRate: reputationMetrics.bounceRate,
        },
      },
      timestamp: new Date().toISOString(),
    });
  } catch (err) {
    logger.error('Edge status check failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// EMAIL SENDING (Resend Integration)
// ============================================================================

// Rate limiting state
const sendRateLimiter = {
  lastSendTime: 0,
  minIntervalMs: 3000, // 3 seconds between sends
  dailyLimit: 50, // Configurable via env
  
  async canSend(): Promise<{ allowed: boolean; reason?: string }> {
    // Check time-based rate limit
    const now = Date.now();
    if (now - this.lastSendTime < this.minIntervalMs) {
      return { 
        allowed: false, 
        reason: `Rate limit: wait ${Math.ceil((this.minIntervalMs - (now - this.lastSendTime)) / 1000)}s` 
      };
    }
    
    // Check daily limit
    const [todayCount] = await queryReadOnly<{ count: number }>(`
      SELECT COUNT(*) as count 
      FROM sa_outbound_messages 
      WHERE sent_at >= CURRENT_DATE
    `);
    
    const dailyLimit = parseInt(process.env.DAILY_SEND_LIMIT || '50', 10);
    if ((todayCount?.count || 0) >= dailyLimit) {
      return { 
        allowed: false, 
        reason: `Daily limit reached: ${dailyLimit} emails sent today` 
      };
    }
    
    return { allowed: true };
  },
  
  recordSend(): void {
    this.lastSendTime = Date.now();
  }
};

/**
 * POST /api/autonomous/send
 * Send a composed message via Resend API
 * 
 * This is the CRITICAL missing piece that enables the full funnel:
 *   Discover → Score → Compose → SEND → Track → Learn
 * 
 * RATE LIMITING:
 * - 3 second minimum between sends
 * - Daily limit (default 50, configurable via DAILY_SEND_LIMIT)
 */
router.post('/send', async (req: Request, res: Response) => {
  try {
    const { messageId, contactId, dryRun = false } = req.body;
    
    // ====== RATE LIMIT CHECK ======
    if (!dryRun) {
      const rateCheck = await sendRateLimiter.canSend();
      if (!rateCheck.allowed) {
        res.status(429).json({ 
          error: 'Rate limited', 
          reason: rateCheck.reason,
          retryAfterMs: sendRateLimiter.minIntervalMs,
        });
        return;
      }
    }
    
    // Type definition for message
    interface OutboundMessage {
      id: string; 
      contact_id: string; 
      subject: string; 
      body_text: string;
      subject_arm_id: string | null;
    }
    
    // Fetch message to send
    let message: OutboundMessage | null = null;
    
    if (messageId) {
      const rows = await queryReadOnly<OutboundMessage>(`
        SELECT id, contact_id, subject, body_text, subject_arm_id
        FROM sa_outbound_messages
        WHERE id = $1 AND status = 'pending'
      `, [messageId]);
      message = rows[0] || null;
    } else if (contactId) {
      const rows = await queryReadOnly<OutboundMessage>(`
        SELECT id, contact_id, subject, body_text, subject_arm_id
        FROM sa_outbound_messages
        WHERE contact_id = $1 AND status = 'pending'
        ORDER BY created_at DESC
        LIMIT 1
      `, [contactId]);
      message = rows[0] || null;
    }
    
    if (!message) {
      res.status(404).json({ error: 'No pending message found' });
      return;
    }
    
    // Get contact email
    const contacts = await queryReadOnly<{ email: string; first_name: string | null }>(`
      SELECT email, first_name FROM sa_contacts WHERE id = $1
    `, [message.contact_id]);
    
    if (contacts.length === 0 || !contacts[0].email) {
      res.status(400).json({ error: 'Contact has no email address' });
      return;
    }
    
    const contact = contacts[0];
    
    // Generate tracking ID for open/click tracking
    const trackingId = `track_${message.id}_${Date.now()}`;
    
    // Add tracking pixel to email body
    const trackingPixelUrl = `${process.env.PUBLIC_ORIGIN || 'https://admin.apexmediation.ee'}/api/autonomous/track/${trackingId}/open`;
    const bodyWithTracking = `${message.body_text}\n\n<img src="${trackingPixelUrl}" width="1" height="1" style="display:none;" alt="" />`;
    
    if (dryRun) {
      res.json({
        success: true,
        dryRun: true,
        message: {
          to: contact.email,
          subject: message.subject,
          bodyPreview: message.body_text.substring(0, 200) + '...',
          trackingId,
        },
      });
      return;
    }
    
    // Send via Resend API
    const resendApiKey = process.env.RESEND_API_KEY;
    if (!resendApiKey) {
      res.status(500).json({ error: 'RESEND_API_KEY not configured' });
      return;
    }
    
    const emailFrom = process.env.EMAIL_FROM || 'contact@apexmediation.ee';
    const emailFromName = process.env.EMAIL_FROM_NAME || 'Apex Mediation';
    const replyDomain = process.env.INBOUND_SMTP_DOMAIN || 'replies.apexmediation.ee';
    const publicOrigin = process.env.PUBLIC_ORIGIN || 'https://admin.apexmediation.ee';
    
    // Construct Reply-To for reply capture (inbound-smtp will catch these)
    const replyToAddress = `reply+${message.id}@${replyDomain}`;
    
    // Construct unsubscribe URLs (RFC 8058 compliant)
    const unsubscribeUrl = `${publicOrigin}/api/autonomous/unsubscribe/${message.contact_id}`;
    const unsubscribeMailto = `mailto:unsubscribe+${message.contact_id}@${replyDomain}?subject=Unsubscribe`;
    
    const sendResponse = await fetch('https://api.resend.com/emails', {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${resendApiKey}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        from: `${emailFromName} <${emailFrom}>`,
        reply_to: replyToAddress,
        to: [contact.email],
        subject: message.subject,
        html: bodyWithTracking.replace(/\n/g, '<br>'),
        text: message.body_text,
        headers: {
          'List-Unsubscribe': `<${unsubscribeUrl}>, <${unsubscribeMailto}>`,
          'List-Unsubscribe-Post': 'List-Unsubscribe=One-Click',
        },
        tags: [
          { name: 'tracking_id', value: trackingId },
          { name: 'contact_id', value: message.contact_id },
        ],
      }),
    });
    
    if (!sendResponse.ok) {
      const errorText = await sendResponse.text();
      logger.error('Resend API error', { status: sendResponse.status, error: errorText });
      res.status(500).json({ error: `Resend API error: ${errorText}` });
      return;
    }
    
    const sendResult = await sendResponse.json() as { id: string };
    
    // Record successful send for rate limiting
    sendRateLimiter.recordSend();
    
    // Update message status
    await queryOperator(`
      UPDATE sa_outbound_messages
      SET status = 'sent', sent_at = CURRENT_TIMESTAMP, message_id = $1, provider = 'resend'
      WHERE id = $2
    `, [sendResult.id, message.id]);
    
    // Save tracking record
    await queryOperator(`
      INSERT INTO sa_email_events 
      (outbound_message_id, contact_id, event_type, arm_id, source, event_data)
      VALUES ($1, $2, 'sent', $3, 'resend', $4)
    `, [message.id, message.contact_id, message.subject_arm_id, JSON.stringify({ trackingId, resendId: sendResult.id })]);
    
    logger.info('Email sent successfully', {
      messageId: message.id,
      contactId: message.contact_id,
      to: contact.email,
      resendId: sendResult.id,
    });
    
    res.json({
      success: true,
      messageId: message.id,
      resendId: sendResult.id,
      trackingId,
      to: contact.email,
    });
    
  } catch (err) {
    logger.error('Send email failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/send-batch
 * Send multiple pending messages (with rate limiting)
 */
router.post('/send-batch', async (req: Request, res: Response) => {
  try {
    const limit = Math.min(parseInt(req.body.limit) || 10, 50); // Max 50 per batch
    const delayMs = parseInt(req.body.delayMs) || 2000; // 2 second delay between sends
    
    // Get pending messages with valid contact emails
    const pending = await queryReadOnly<{ id: string; contact_id: string }>(`
      SELECT om.id, om.contact_id
      FROM sa_outbound_messages om
      JOIN sa_contacts c ON c.id = om.contact_id
      WHERE om.status = 'pending'
        AND c.email IS NOT NULL
        AND c.email != ''
      ORDER BY om.created_at ASC
      LIMIT $1
    `, [limit]);
    
    let sent = 0;
    let failed = 0;
    const results: Array<{ messageId: string; success: boolean; error?: string }> = [];
    
    for (const msg of pending) {
      try {
        // Make internal send request
        const sendResult = await fetch(`http://localhost:${process.env.PORT || 4001}/api/autonomous/send`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'X-API-Key': process.env.AUTONOMOUS_API_KEY || '',
          },
          body: JSON.stringify({ messageId: msg.id }),
        });
        
        if (sendResult.ok) {
          sent++;
          results.push({ messageId: msg.id, success: true });
        } else {
          failed++;
          const error = await sendResult.text();
          results.push({ messageId: msg.id, success: false, error });
        }
        
        // Rate limiting delay
        if (pending.indexOf(msg) < pending.length - 1) {
          await new Promise(resolve => setTimeout(resolve, delayMs));
        }
        
      } catch (err) {
        failed++;
        results.push({ messageId: msg.id, success: false, error: (err as Error).message });
      }
    }
    
    res.json({
      success: true,
      sent,
      failed,
      total: pending.length,
      results,
    });
    
  } catch (err) {
    logger.error('Batch send failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// DASHBOARD METRICS (Production Visibility)
// ============================================================================

/**
 * GET /api/autonomous/dashboard
 * Comprehensive dashboard metrics for sales funnel visibility
 */
router.get('/dashboard', async (_req: Request, res: Response) => {
  try {
    // Funnel metrics
    const funnelStats = await queryReadOnly<{
      total_sources: string;
      total_contacts: string;
      contacts_with_email: string;
      contacts_scored: string;
      hot_leads: string;
      warm_leads: string;
      messages_composed: string;
      messages_sent: string;
      messages_opened: string;
      messages_clicked: string;
      messages_replied: string;
    }>(`
      SELECT
        (SELECT COUNT(*) FROM sa_sources WHERE status = 'active') as total_sources,
        (SELECT COUNT(*) FROM sa_contacts) as total_contacts,
        (SELECT COUNT(*) FROM sa_contacts WHERE email IS NOT NULL AND email != '') as contacts_with_email,
        (SELECT COUNT(*) FROM sa_contacts WHERE score > 0) as contacts_scored,
        (SELECT COUNT(*) FROM sa_contacts WHERE score >= 70) as hot_leads,
        (SELECT COUNT(*) FROM sa_contacts WHERE score >= 50 AND score < 70) as warm_leads,
        (SELECT COUNT(*) FROM sa_outbound_messages) as messages_composed,
        (SELECT COUNT(*) FROM sa_outbound_messages WHERE sent_at IS NOT NULL) as messages_sent,
        (SELECT COUNT(*) FROM sa_outbound_messages WHERE opened_at IS NOT NULL) as messages_opened,
        (SELECT COUNT(*) FROM sa_outbound_messages WHERE clicked_at IS NOT NULL) as messages_clicked,
        (SELECT COUNT(*) FROM sa_outbound_messages WHERE replied_at IS NOT NULL) as messages_replied
    `);
    
    const f = funnelStats[0];
    
    // Job health (last 24h)
    const jobHealth = await queryReadOnly<{
      job_type: string;
      completed: string;
      failed: string;
      running: string;
    }>(`
      SELECT 
        job_type,
        COUNT(*) FILTER (WHERE status = 'completed') as completed,
        COUNT(*) FILTER (WHERE status = 'failed') as failed,
        COUNT(*) FILTER (WHERE status = 'running') as running
      FROM sa_autonomous_jobs
      WHERE created_at > NOW() - INTERVAL '24 hours'
      GROUP BY job_type
    `);
    
    // Thompson Sampler health
    const banditHealth = await queryReadOnly<{
      total_arms: string;
      total_trials: string;
      total_successes: string;
    }>(`
      SELECT 
        COUNT(*) as total_arms,
        COALESCE(SUM(total_trials), 0) as total_trials,
        COALESCE(SUM(total_successes), 0) as total_successes
      FROM sa_bandit_arms
      WHERE active = true
    `);
    
    // Naive Bayes training health
    const trainingHealth = await queryReadOnly<{
      total_examples: string;
      categories: string;
    }>(`
      SELECT 
        COUNT(*) as total_examples,
        COUNT(DISTINCT label) as categories
      FROM sa_bayes_training
    `);
    
    // Competitor detection health
    const competitorHealth = await queryReadOnly<{
      total_detections: string;
      unique_competitors: string;
    }>(`
      SELECT 
        COUNT(*) as total_detections,
        COUNT(DISTINCT competitor_name) as unique_competitors
      FROM sa_competitor_detections
    `);
    
    // Calculate conversion rates
    const totalContacts = parseInt(f.total_contacts) || 1;
    const contactsWithEmail = parseInt(f.contacts_with_email);
    const messagesSent = parseInt(f.messages_sent) || 1;
    const messagesOpened = parseInt(f.messages_opened);
    const messagesClicked = parseInt(f.messages_clicked);
    const messagesReplied = parseInt(f.messages_replied);
    
    res.json({
      success: true,
      dashboard: {
        funnel: {
          sources: parseInt(f.total_sources),
          contacts: parseInt(f.total_contacts),
          withEmail: contactsWithEmail,
          scored: parseInt(f.contacts_scored),
          hotLeads: parseInt(f.hot_leads),
          warmLeads: parseInt(f.warm_leads),
          composed: parseInt(f.messages_composed),
          sent: parseInt(f.messages_sent),
          opened: messagesOpened,
          clicked: messagesClicked,
          replied: messagesReplied,
        },
        conversionRates: {
          contactToEmail: `${((contactsWithEmail / totalContacts) * 100).toFixed(1)}%`,
          sentToOpen: `${((messagesOpened / messagesSent) * 100).toFixed(1)}%`,
          openToClick: `${messagesOpened > 0 ? ((messagesClicked / messagesOpened) * 100).toFixed(1) : 0}%`,
          sentToReply: `${((messagesReplied / messagesSent) * 100).toFixed(1)}%`,
        },
        jobs: {
          byType: jobHealth.reduce((acc, j) => {
            acc[j.job_type] = { 
              completed: parseInt(j.completed), 
              failed: parseInt(j.failed),
              running: parseInt(j.running),
            };
            return acc;
          }, {} as Record<string, { completed: number; failed: number; running: number }>),
        },
        learning: {
          thompsonSampler: {
            arms: parseInt(banditHealth[0]?.total_arms || '0'),
            trials: parseInt(banditHealth[0]?.total_trials || '0'),
            successes: parseInt(banditHealth[0]?.total_successes || '0'),
            successRate: banditHealth[0]?.total_trials 
              ? `${((parseInt(banditHealth[0].total_successes) / parseInt(banditHealth[0].total_trials)) * 100).toFixed(1)}%`
              : '0%',
          },
          naiveBayes: {
            trainingExamples: parseInt(trainingHealth[0]?.total_examples || '0'),
            categories: parseInt(trainingHealth[0]?.categories || '0'),
          },
        },
        competitorTargeting: {
          detections: parseInt(competitorHealth[0]?.total_detections || '0'),
          uniqueCompetitors: parseInt(competitorHealth[0]?.unique_competitors || '0'),
        },
      },
      timestamp: new Date().toISOString(),
    });
    
  } catch (err) {
    logger.error('Dashboard fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// STUCK JOB RECOVERY
// ============================================================================

/**
 * POST /api/autonomous/cleanup-emails
 * Clean up malformed emails that got into the database before validation was improved.
 * 
 * Removes emails that match known bad patterns:
 * - UUID local parts (32 hex chars)
 * - Sentry tracking emails
 * - Emails with text after TLD (e.g., domain.comTerms)
 */
router.post('/cleanup-emails', async (req: Request, res: Response) => {
  try {
    const { dryRun = true } = req.body;
    
    // Find malformed emails
    const malformed = await queryReadOnly<{ id: string; email: string; company_name: string | null }>(`
      SELECT id, email, company_name FROM sa_contacts
      WHERE email IS NOT NULL
        AND (
          -- UUID local part (32 hex chars)
          email ~ '^[0-9a-f]{32}@'
          -- Or full UUID format
          OR email ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}@'
          -- Or sentry tracking emails
          OR email ILIKE '%@sentry%'
          -- Or text after TLD (e.g., .comTerms)
          OR email ~ '\\.(com|net|org|io)[A-Za-z]'
        )
    `);
    
    if (dryRun) {
      res.json({
        success: true,
        dryRun: true,
        malformedCount: malformed.length,
        malformed: malformed.map(m => ({
          id: m.id,
          email: m.email,
          company: m.company_name,
        })),
      });
      return;
    }
    
    // Actually remove the malformed emails
    const cleaned = await queryOperator<{ id: string }>(`
      UPDATE sa_contacts
      SET email = NULL, updated_at = CURRENT_TIMESTAMP
      WHERE email IS NOT NULL
        AND (
          email ~ '^[0-9a-f]{32}@'
          OR email ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}@'
          OR email ILIKE '%@sentry%'
          OR email ~ '\\.(com|net|org|io)[A-Za-z]'
        )
      RETURNING id
    `);
    
    logger.info('Cleaned up malformed emails', {
      cleaned: cleaned.length,
    });
    
    res.json({
      success: true,
      cleaned: cleaned.length,
      ids: cleaned.map(c => c.id),
    });
    
  } catch (err) {
    logger.error('Email cleanup failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

/**
 * POST /api/autonomous/jobs/cleanup
 * Clean up stuck jobs and recover system health
 */
router.post('/jobs/cleanup', async (req: Request, res: Response) => {
  try {
    const maxAgeMinutes = parseInt(req.body.maxAgeMinutes) || 30;
    
    // Find and fail stuck running jobs
    const stuckJobs = await queryOperator<{ id: string; job_type: string }>(`
      UPDATE sa_autonomous_jobs
      SET 
        status = 'failed', 
        completed_at = CURRENT_TIMESTAMP,
        output_data = '{"error": "Job timed out - auto-recovered"}'
      WHERE status = 'running'
        AND started_at < NOW() - INTERVAL '${maxAgeMinutes} minutes'
      RETURNING id, job_type
    `);
    
    // Also clean up any orphaned pending jobs older than 1 hour
    const orphanedJobs = await queryOperator<{ id: string }>(`
      UPDATE sa_autonomous_jobs
      SET 
        status = 'failed', 
        completed_at = CURRENT_TIMESTAMP,
        output_data = '{"error": "Orphaned job - auto-recovered"}'
      WHERE status = 'pending'
        AND scheduled_at < NOW() - INTERVAL '1 hour'
      RETURNING id
    `);
    
    logger.info('Job cleanup completed', {
      stuckJobsRecovered: stuckJobs.length,
      orphanedJobsRecovered: orphanedJobs.length,
    });
    
    res.json({
      success: true,
      recovered: {
        stuck: stuckJobs.length,
        orphaned: orphanedJobs.length,
        jobs: stuckJobs.map(j => ({ id: j.id, type: j.job_type })),
      },
    });
    
  } catch (err) {
    logger.error('Job cleanup failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// AUTONOMOUS RUN CYCLE (For Cron Jobs)
// ============================================================================

/**
 * POST /api/autonomous/run-cycle
 * Run a complete automation cycle: cleanup → discover → enrich → score → compose → send
 * 
 * This is the endpoint to call from cron for fully autonomous operation.
 */
router.post('/run-cycle', async (req: Request, res: Response) => {
  try {
    const { 
      discoverBatch = 3, 
      enrichBatch = 10, 
      scoreBatch = 50,
      sendBatch = 5,
      dryRun = false,
    } = req.body;
    
    const results: Record<string, unknown> = {};
    const startTime = Date.now();
    
    // Step 0: Cleanup stuck jobs
    try {
      const cleanupResult = await queryOperator<{ id: string }>(`
        UPDATE sa_autonomous_jobs
        SET status = 'failed', completed_at = CURRENT_TIMESTAMP,
            output_data = '{"error": "Auto-cleanup before cycle"}'
        WHERE status = 'running' AND started_at < NOW() - INTERVAL '30 minutes'
        RETURNING id
      `);
      results.cleanup = { recovered: cleanupResult.length };
    } catch (err) {
      results.cleanup = { error: (err as Error).message };
    }
    
    // Step 1: Discover new leads
    try {
      const orchestrator = await getOrchestrator();
      const discoverResult = await orchestrator.runDiscoveryJob(discoverBatch);
      results.discover = { success: true, processed: discoverResult.processed };
    } catch (err) {
      results.discover = { error: (err as Error).message };
    }
    
    // Step 2: Enrich contacts with emails
    try {
      const orchestrator = await getOrchestrator();
      const enrichResult = await orchestrator.runEnrichmentJob(enrichBatch);
      results.enrich = { success: true, enriched: enrichResult.processed };
    } catch (err) {
      results.enrich = { error: (err as Error).message };
    }
    
    // Step 3: Score leads
    try {
      const orchestrator = await getOrchestrator();
      const scoreResult = await orchestrator.runScoringJob(scoreBatch);
      results.score = { success: true, scored: scoreResult.processed };
    } catch (err) {
      results.score = { error: (err as Error).message };
    }
    
    // Step 4: Compose messages for hot leads without messages
    try {
      const hotLeads = await queryReadOnly<{ id: string }>(`
        SELECT c.id 
        FROM sa_contacts c
        LEFT JOIN sa_outbound_messages om ON om.contact_id = c.id
        WHERE c.score >= 50
          AND c.email IS NOT NULL
          AND c.email != ''
          AND om.id IS NULL
        ORDER BY c.score DESC
        LIMIT $1
      `, [sendBatch]);
      
      let composed = 0;
      const orchestrator = await getOrchestrator();
      
      for (const lead of hotLeads) {
        try {
          await orchestrator.composeMessage(lead.id);
          composed++;
        } catch {
          // Continue with next lead
        }
      }
      
      results.compose = { success: true, composed };
    } catch (err) {
      results.compose = { error: (err as Error).message };
    }
    
    // Step 5: Send pending messages (unless dry run)
    if (!dryRun) {
      try {
        const pending = await queryReadOnly<{ id: string }>(`
          SELECT om.id 
          FROM sa_outbound_messages om
          JOIN sa_contacts c ON c.id = om.contact_id
          WHERE om.status = 'pending'
            AND c.email IS NOT NULL
          ORDER BY om.created_at ASC
          LIMIT $1
        `, [sendBatch]);
        
        let sent = 0;
        let failed = 0;
        
        for (const msg of pending) {
          try {
            // Internal send call
            const response = await fetch(`http://localhost:${process.env.PORT || 4001}/api/autonomous/send`, {
              method: 'POST',
              headers: {
                'Content-Type': 'application/json',
                'X-API-Key': process.env.AUTONOMOUS_API_KEY || '',
              },
              body: JSON.stringify({ messageId: msg.id }),
            });
            
            if (response.ok) {
              sent++;
            } else {
              failed++;
            }
            
            // Rate limit: 2 second delay between sends
            await new Promise(resolve => setTimeout(resolve, 2000));
          } catch {
            failed++;
          }
        }
        
        results.send = { success: true, sent, failed };
      } catch (err) {
        results.send = { error: (err as Error).message };
      }
    } else {
      results.send = { skipped: true, reason: 'dryRun=true' };
    }
    
    const duration = Date.now() - startTime;
    
    logger.info('Automation cycle completed', { results, durationMs: duration });
    
    res.json({
      success: true,
      cycle: results,
      durationMs: duration,
      timestamp: new Date().toISOString(),
    });
    
  } catch (err) {
    logger.error('Automation cycle failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// HEALTH CHECK
// ============================================================================

/**
 * GET /api/autonomous/health
 * Health check endpoint
 */
router.get('/health', async (_req: Request, res: Response) => {
  // Check Edge Worker connectivity
  const edgeWorkerUrl = process.env.EDGE_WORKER_URL;
  let edgeWorkerStatus: 'connected' | 'disconnected' | 'not_configured' = 'not_configured';
  let edgeWorkerLatency: number | null = null;
  
  if (edgeWorkerUrl) {
    try {
      const start = Date.now();
      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), 5000);
      
      const response = await fetch(`${edgeWorkerUrl}/health`, {
        signal: controller.signal,
        headers: { 'X-Edge-Secret': process.env.EDGE_WORKER_SECRET || '' },
      });
      
      clearTimeout(timeout);
      
      if (response.ok) {
        edgeWorkerStatus = 'connected';
        edgeWorkerLatency = Date.now() - start;
      } else {
        edgeWorkerStatus = 'disconnected';
      }
    } catch {
      edgeWorkerStatus = 'disconnected';
    }
  }
  
  res.json({
    status: 'healthy',
    service: 'autonomous-sales',
    version: '2.3.0-scheduled',
    architecture: 'control-plane + edge-worker + scheduler',
    edgeWorker: {
      url: edgeWorkerUrl || null,
      status: edgeWorkerStatus,
      latencyMs: edgeWorkerLatency,
    },
    features: [
      'icebreaker-llm', 
      'hybrid-classifier', 
      'reputation-guard', 
      'company-enrichment',
      'email-sending',
      'open-tracking',
      'click-tracking',
      'dashboard-metrics',
      'stuck-job-recovery',
      'automation-cycle',
      'scheduled-jobs',
      'bandit-learning-loop',
    ],
    note: 'ML inference runs on Edge Worker (Test Server), Control Plane handles orchestration, Scheduler runs automated jobs',
    timestamp: new Date().toISOString(),
  });
});

// ============================================================================
// SCHEDULER MANAGEMENT
// ============================================================================

import { 
  getSchedulerStatus, 
  getAvailableJobs, 
  triggerScheduledJob 
} from '../services/sales-scheduler';

/**
 * GET /api/autonomous/scheduler/status
 * Get the scheduler status
 */
router.get('/scheduler/status', (_req: Request, res: Response) => {
  const status = getSchedulerStatus();
  const availableJobs = getAvailableJobs();
  
  res.json({
    ...status,
    availableJobs,
    mode: process.env.SALES_AUTOMATION_ROLE || 'proxy',
    note: status.running 
      ? 'Scheduler is running automated jobs'
      : 'Scheduler not running (proxy mode or not started)',
  });
});

/**
 * POST /api/autonomous/scheduler/trigger
 * Manually trigger a scheduled job
 */
router.post('/scheduler/trigger', async (req: Request, res: Response) => {
  const { jobName } = req.body;
  
  if (!jobName) {
    res.status(400).json({ error: 'jobName is required' });
    return;
  }
  
  const availableJobs = getAvailableJobs();
  if (!availableJobs.includes(jobName)) {
    res.status(400).json({ 
      error: `Invalid job name. Available jobs: ${availableJobs.join(', ')}` 
    });
    return;
  }
  
  const result = await triggerScheduledJob(jobName);
  
  if (result.success) {
    res.json({ 
      success: true, 
      jobName,
      result: result.result,
      timestamp: new Date().toISOString(),
    });
  } else {
    res.status(500).json({ 
      success: false, 
      jobName,
      error: result.error,
      timestamp: new Date().toISOString(),
    });
  }
});

export default router;
