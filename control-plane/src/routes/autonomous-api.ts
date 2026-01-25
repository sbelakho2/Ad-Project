/**
 * AUTONOMOUS SALES API ROUTES
 * 
 * REST API endpoints for the autonomous sales system.
 * These routes are mounted on the control-plane server.
 * 
 * Endpoints:
 *   POST /api/autonomous/discover    - Run discovery job
 *   POST /api/autonomous/score       - Run scoring job
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
import { getSources } from '../services/discovery/hunter';
import { logger } from '../logger';

const router = Router();

// ============================================================================
// MIDDLEWARE
// ============================================================================

/**
 * API Key authentication for external calls (from app server)
 */
function authenticateApiKey(req: Request, res: Response, next: () => void): void {
  const apiKey = req.headers['x-api-key'] as string;
  const expectedKey = process.env.AUTONOMOUS_API_KEY;
  
  // If no key is configured, allow all requests (dev mode)
  if (!expectedKey) {
    return next();
  }
  
  if (!apiKey || apiKey !== expectedKey) {
    res.status(401).json({ error: 'Unauthorized' });
    return;
  }
  
  next();
}

router.use(authenticateApiKey);

// ============================================================================
// JOB ENDPOINTS
// ============================================================================

/**
 * POST /api/autonomous/discover
 * Run a discovery job to find new leads
 */
router.post('/discover', async (req: Request, res: Response) => {
  try {
    const { batchSize = 3 } = req.body;
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
 */
router.post('/score', async (req: Request, res: Response) => {
  try {
    const { batchSize = 50 } = req.body;
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
// SOURCES
// ============================================================================

/**
 * GET /api/autonomous/sources
 * Get configured lead sources
 */
router.get('/sources', async (_req: Request, res: Response) => {
  try {
    const sources = await getSources();
    
    res.json({
      success: true,
      sources,
    });
  } catch (err) {
    logger.error('Sources fetch failed', { error: (err as Error).message });
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
router.get('/health', (_req: Request, res: Response) => {
  res.json({
    status: 'healthy',
    service: 'autonomous-sales',
    timestamp: new Date().toISOString(),
  });
});

export default router;
