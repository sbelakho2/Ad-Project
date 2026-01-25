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
    
    // CLOSED LOOP: If positive response, record success for the arm
    if (classification.category === 'INTERESTED' || classification.category === 'MEETING') {
      // Find the contact by email
      const contacts = await queryReadOnly<{ id: string }>(`
        SELECT id FROM sa_contacts WHERE LOWER(email) = $1 LIMIT 1
      `, [fromEmail]);
      
      if (contacts.length > 0) {
        // Find the original outbound message
        const outbound = await queryReadOnly<{ subject_arm_id: string }>(`
          SELECT subject_arm_id FROM sa_outbound_messages 
          WHERE contact_id = $1 AND subject_arm_id IS NOT NULL
          ORDER BY sent_at DESC LIMIT 1
        `, [contacts[0].id]);
        
        if (outbound.length > 0 && outbound[0].subject_arm_id) {
          const { recordOutcome } = await import('../services/math/thompson-sampler.js');
          await recordOutcome(outbound[0].subject_arm_id, true);
          
          logger.info('SendGrid webhook: Recorded positive bandit outcome', {
            fromEmail,
            category: classification.category,
            armId: outbound[0].subject_arm_id,
          });
        }
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
    logger.error('Stats fetch failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
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
    version: '2.1.0-distributed',
    architecture: 'control-plane + edge-worker',
    edgeWorker: {
      url: edgeWorkerUrl || null,
      status: edgeWorkerStatus,
      latencyMs: edgeWorkerLatency,
    },
    features: ['icebreaker-llm', 'hybrid-classifier', 'reputation-guard', 'company-enrichment'],
    note: 'ML inference runs on Edge Worker (Test Server), Control Plane handles orchestration',
    timestamp: new Date().toISOString(),
  });
});

export default router;
