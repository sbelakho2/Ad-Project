/**
 * EDGE WORKER - Main Entry Point
 * 
 * This service runs on the Test Server (157.90.126.205) and provides:
 *   - LLM-based icebreaker generation (Flan-T5-Small)
 *   - Hybrid email classification (NB + DistilBERT)
 *   - Playwright-based web scraping
 *   - Reputation monitoring analysis
 * 
 * Architecture:
 *   - Test Server (this worker) handles CPU-intensive ML inference
 *   - App Server (Control Plane) coordinates and stores data
 *   - Communication via REST API with shared secret
 * 
 * Endpoints:
 *   POST /icebreaker/generate   - Generate personalized icebreaker
 *   POST /classify/hybrid       - Two-stage email classification
 *   POST /scrape/google-play    - Scrape Google Play category
 *   POST /scrape/app-details    - Scrape single app details
 *   POST /reputation/analyze    - Analyze bounce message
 *   GET  /health                - Health check with model status
 *   POST /warmup                - Pre-warm ML models
 */

import express, { Request, Response, NextFunction } from 'express';
import { createLogger, format, transports } from 'winston';
import crypto from 'crypto';

// Import services
import { IcebreakerEngine } from './services/icebreaker-engine.js';
import { HybridClassifier } from './services/hybrid-classifier.js';
import { ReputationAnalyzer } from './services/reputation-analyzer.js';
import { PlaywrightScraper } from './services/playwright-scraper.js';

// ============================================================================
// CONFIGURATION
// ============================================================================

const PORT = parseInt(process.env.EDGE_WORKER_PORT || '3500');
const IS_PRODUCTION = process.env.NODE_ENV === 'production';

// SECURITY: Require explicit secret in production, fail-fast if missing
const API_SECRET = process.env.EDGE_WORKER_SECRET;
if (!API_SECRET) {
  if (IS_PRODUCTION) {
    console.error('FATAL: EDGE_WORKER_SECRET must be set in production');
    process.exit(1);
  }
  console.warn('WARNING: EDGE_WORKER_SECRET not set - using insecure default (dev only)');
}
const EFFECTIVE_SECRET = API_SECRET || 'dev-secret-change-me';

const CONTROL_PLANE_URL = process.env.CONTROL_PLANE_URL || 'http://46.62.252.96:4000';

// ============================================================================
// LOGGER
// ============================================================================

const logger = createLogger({
  level: process.env.LOG_LEVEL || 'info',
  format: format.combine(
    format.timestamp(),
    format.errors({ stack: true }),
    format.json()
  ),
  defaultMeta: { service: 'edge-worker' },
  transports: [
    new transports.Console({
      format: format.combine(
        format.colorize(),
        format.simple()
      ),
    }),
    new transports.File({ filename: 'logs/edge-worker.log' }),
    new transports.File({ filename: 'logs/edge-worker-error.log', level: 'error' }),
  ],
});

// ============================================================================
// EXPRESS APP
// ============================================================================

const app = express();
app.use(express.json({ limit: '10mb' }));

// ============================================================================
// AUTHENTICATION MIDDLEWARE
// ============================================================================

function authenticate(req: Request, res: Response, next: NextFunction): void {
  const secret = req.headers['x-edge-secret'] as string;
  
  // Use timing-safe comparison to prevent timing attacks
  if (!secret) {
    logger.warn('Unauthorized request - missing secret', { ip: req.ip, path: req.path });
    res.status(401).json({ error: 'Unauthorized' });
    return;
  }
  
  const secretBuf = Buffer.from(secret);
  const expectedBuf = Buffer.from(EFFECTIVE_SECRET);
  
  if (secretBuf.length !== expectedBuf.length || !crypto.timingSafeEqual(secretBuf, expectedBuf)) {
    logger.warn('Unauthorized request - invalid secret', { ip: req.ip, path: req.path });
    res.status(401).json({ error: 'Unauthorized' });
    return;
  }
  
  next();
}

// Apply auth to all routes except health
app.use((req, res, next) => {
  if (req.path === '/health') {
    next();
    return;
  }
  authenticate(req, res, next);
});

// ============================================================================
// URL VALIDATION FOR SCRAPING (SSRF PROTECTION)
// ============================================================================

/**
 * Validate URL for safe scraping.
 * SECURITY: Prevents SSRF via protocol validation and hostname checks.
 */
function validateScrapingUrl(url: string): { valid: boolean; reason?: string } {
  try {
    const parsed = new URL(url);
    
    // Only allow HTTP/HTTPS protocols
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      return { valid: false, reason: `Invalid protocol: ${parsed.protocol}` };
    }
    
    // Block localhost and internal hostnames
    const hostname = parsed.hostname.toLowerCase();
    if (
      hostname === 'localhost' ||
      hostname === '127.0.0.1' ||
      hostname === '::1' ||
      hostname === '[::1]' ||
      hostname.endsWith('.local') ||
      hostname.endsWith('.internal') ||
      hostname.endsWith('.localhost') ||
      hostname === '0.0.0.0'
    ) {
      return { valid: false, reason: 'Internal hostname not allowed' };
    }
    
    // Block private IP ranges
    if (/^(10\.|172\.(1[6-9]|2[0-9]|3[01])\.|192\.168\.|169\.254\.|fc00:|fe80:|::1)/.test(hostname)) {
      return { valid: false, reason: 'Private IP not allowed' };
    }
    
    // Block metadata endpoints (cloud provider)
    if (hostname === '169.254.169.254' || hostname === 'metadata.google.internal') {
      return { valid: false, reason: 'Cloud metadata endpoints not allowed' };
    }
    
    return { valid: true };
  } catch {
    return { valid: false, reason: 'Invalid URL format' };
  }
}

// ============================================================================
// SERVICE INSTANCES (Lazy-loaded)
// ============================================================================

let icebreakerEngine: IcebreakerEngine | null = null;
let hybridClassifier: HybridClassifier | null = null;
let reputationAnalyzer: ReputationAnalyzer | null = null;
let playwrightScraper: PlaywrightScraper | null = null;

async function getIcebreakerEngine(): Promise<IcebreakerEngine> {
  if (!icebreakerEngine) {
    icebreakerEngine = new IcebreakerEngine();
    await icebreakerEngine.initialize();
  }
  return icebreakerEngine;
}

async function getHybridClassifier(): Promise<HybridClassifier> {
  if (!hybridClassifier) {
    hybridClassifier = new HybridClassifier();
    await hybridClassifier.initialize();
  }
  return hybridClassifier;
}

async function getReputationAnalyzer(): Promise<ReputationAnalyzer> {
  if (!reputationAnalyzer) {
    reputationAnalyzer = new ReputationAnalyzer();
  }
  return reputationAnalyzer;
}

async function getPlaywrightScraper(): Promise<PlaywrightScraper> {
  if (!playwrightScraper) {
    playwrightScraper = new PlaywrightScraper();
    await playwrightScraper.initialize();
  }
  return playwrightScraper;
}

// ============================================================================
// HEALTH CHECK
// ============================================================================

app.get('/health', async (_req: Request, res: Response) => {
  const status = {
    status: 'healthy',
    service: 'edge-worker',
    version: '1.0.0',
    timestamp: new Date().toISOString(),
    models: {
      icebreaker: icebreakerEngine?.getStatus() || { loaded: false, loading: false },
      classifier: hybridClassifier?.getStatus() || { loaded: false, loading: false },
    },
    scraper: {
      browserReady: playwrightScraper?.isInitialized() || false,
    },
  };
  
  res.json(status);
});

// ============================================================================
// WARMUP ENDPOINT
// ============================================================================

app.post('/warmup', async (_req: Request, res: Response) => {
  const startTime = Date.now();
  const results: Record<string, { success: boolean; latencyMs: number; error?: string }> = {};
  
  try {
    // Warmup icebreaker
    const iceStart = Date.now();
    const iceEngine = await getIcebreakerEngine();
    await iceEngine.warmup();
    results.icebreaker = { success: true, latencyMs: Date.now() - iceStart };
  } catch (err) {
    results.icebreaker = { success: false, latencyMs: 0, error: (err as Error).message };
  }
  
  try {
    // Warmup classifier
    const classStart = Date.now();
    const classifier = await getHybridClassifier();
    await classifier.warmup();
    results.classifier = { success: true, latencyMs: Date.now() - classStart };
  } catch (err) {
    results.classifier = { success: false, latencyMs: 0, error: (err as Error).message };
  }
  
  try {
    // Warmup scraper
    const scrapeStart = Date.now();
    const scraper = await getPlaywrightScraper();
    await scraper.initialize();
    results.scraper = { success: true, latencyMs: Date.now() - scrapeStart };
  } catch (err) {
    results.scraper = { success: false, latencyMs: 0, error: (err as Error).message };
  }
  
  res.json({
    success: true,
    totalLatencyMs: Date.now() - startTime,
    results,
  });
});

// ============================================================================
// ICEBREAKER ENDPOINTS
// ============================================================================

app.post('/icebreaker/generate', async (req: Request, res: Response) => {
  try {
    const { appName, appDescription, genre, developerName, installs, rating } = req.body;
    
    if (!appName) {
      res.status(400).json({ error: 'appName required' });
      return;
    }
    
    const engine = await getIcebreakerEngine();
    const result = await engine.generate({
      appName,
      appDescription,
      genre,
      developerName,
      installs,
      rating,
    });
    
    res.json({
      success: true,
      ...result,
    });
  } catch (err) {
    logger.error('Icebreaker generation failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

app.get('/icebreaker/status', async (_req: Request, res: Response) => {
  const engine = await getIcebreakerEngine();
  res.json({
    success: true,
    status: engine.getStatus(),
  });
});

// ============================================================================
// CLASSIFICATION ENDPOINTS
// ============================================================================

app.post('/classify/hybrid', async (req: Request, res: Response) => {
  try {
    const { subject, body } = req.body;
    
    if (!subject || !body) {
      res.status(400).json({ error: 'subject and body required' });
      return;
    }
    
    const classifier = await getHybridClassifier();
    const result = await classifier.classify(subject, body);
    
    res.json({
      success: true,
      classification: result,
    });
  } catch (err) {
    logger.error('Classification failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

app.get('/classify/status', async (_req: Request, res: Response) => {
  const classifier = await getHybridClassifier();
  res.json({
    success: true,
    status: classifier.getStatus(),
  });
});

// ============================================================================
// REPUTATION ANALYSIS ENDPOINTS
// ============================================================================

app.post('/reputation/analyze', async (req: Request, res: Response) => {
  try {
    const { domain, bounces, engagement } = req.body;
    
    if (!domain) {
      res.status(400).json({ error: 'domain required' });
      return;
    }
    
    const analyzer = await getReputationAnalyzer();
    const result = await analyzer.analyze({
      domain,
      bounces,
      engagement,
    });
    
    res.json({
      success: true,
      analysis: result,
    });
  } catch (err) {
    logger.error('Reputation analysis failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// SCRAPING ENDPOINTS
// ============================================================================

app.post('/scrape/linkedin', async (req: Request, res: Response) => {
  try {
    const { url } = req.body;
    
    if (!url) {
      res.status(400).json({ error: 'url required' });
      return;
    }
    
    // SECURITY: Validate URL before scraping
    const validation = validateScrapingUrl(url);
    if (!validation.valid) {
      res.status(400).json({ error: `Invalid URL: ${validation.reason}` });
      return;
    }
    
    const scraper = await getPlaywrightScraper();
    const result = await scraper.scrapeLinkedInProfile(url);
    
    res.json(result);
  } catch (err) {
    logger.error('LinkedIn scrape failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

app.post('/scrape/company', async (req: Request, res: Response) => {
  try {
    const { url } = req.body;
    
    if (!url) {
      res.status(400).json({ error: 'url required' });
      return;
    }
    
    // SECURITY: Validate URL before scraping
    const validation = validateScrapingUrl(url);
    if (!validation.valid) {
      res.status(400).json({ error: `Invalid URL: ${validation.reason}` });
      return;
    }
    
    const scraper = await getPlaywrightScraper();
    const result = await scraper.scrapeCompanyWebsite(url);
    
    res.json(result);
  } catch (err) {
    logger.error('Company scrape failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

app.post('/scrape/generic', async (req: Request, res: Response) => {
  try {
    const { url, selectors } = req.body;
    
    if (!url || !selectors) {
      res.status(400).json({ error: 'url and selectors required' });
      return;
    }
    
    // SECURITY: Validate URL before scraping
    const validation = validateScrapingUrl(url);
    if (!validation.valid) {
      res.status(400).json({ error: `Invalid URL: ${validation.reason}` });
      return;
    }
    
    const scraper = await getPlaywrightScraper();
    const result = await scraper.scrapeGenericPage(url, selectors);
    
    res.json(result);
  } catch (err) {
    logger.error('Generic scrape failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// ENRICHMENT ENDPOINTS (NEW - fixes missing feature)
// ============================================================================

app.post('/enrich/company', async (req: Request, res: Response) => {
  try {
    const { name, website, linkedinUrl } = req.body;
    
    if (!name && !website) {
      res.status(400).json({ error: 'name or website required' });
      return;
    }
    
    const scraper = await getPlaywrightScraper();
    const startTime = Date.now();
    
    let enrichment: Record<string, unknown> = {};
    
    // Try website first
    if (website) {
      const result = await scraper.scrapeCompanyWebsite(website);
      if (result.success && result.data) {
        enrichment = {
          ...enrichment,
          ...result.data,
        };
      }
    }
    
    // Try LinkedIn if provided
    if (linkedinUrl) {
      const result = await scraper.scrapeLinkedInProfile(linkedinUrl);
      if (result.success && result.data) {
        enrichment.linkedin = result.data;
      }
    }
    
    res.json({
      success: true,
      data: enrichment,
      latencyMs: Date.now() - startTime,
    });
  } catch (err) {
    logger.error('Company enrichment failed', { error: (err as Error).message });
    res.status(500).json({ error: (err as Error).message });
  }
});

// ============================================================================
// ERROR HANDLER
// ============================================================================

app.use((err: Error, _req: Request, res: Response, _next: NextFunction) => {
  logger.error('Unhandled error', { error: err.message, stack: err.stack });
  res.status(500).json({ error: 'Internal server error' });
});

// ============================================================================
// STARTUP
// ============================================================================

async function start(): Promise<void> {
  // Create logs directory
  const fs = await import('fs');
  if (!fs.existsSync('logs')) {
    fs.mkdirSync('logs', { recursive: true });
  }
  
  app.listen(PORT, '0.0.0.0', () => {
    logger.info(`Edge Worker started on port ${PORT}`);
    logger.info(`Control Plane URL: ${CONTROL_PLANE_URL}`);
    logger.info('Endpoints available:');
    logger.info('  POST /icebreaker/generate');
    logger.info('  POST /classify/hybrid');
    logger.info('  POST /reputation/analyze');
    logger.info('  POST /scrape/google-play');
    logger.info('  POST /scrape/app-details');
    logger.info('  POST /scrape/app-ads-txt');
    logger.info('  POST /enrich/company');
    logger.info('  POST /warmup');
    logger.info('  GET  /health');
  });
  
  // Warmup models in background (fixes race condition bug)
  setTimeout(async () => {
    logger.info('Starting background model warmup...');
    try {
      const [iceEngine, classifier] = await Promise.all([
        getIcebreakerEngine(),
        getHybridClassifier(),
      ]);
      await Promise.all([
        iceEngine.warmup(),
        classifier.warmup(),
      ]);
      logger.info('Background model warmup complete');
    } catch (err) {
      logger.warn('Background warmup failed (will retry on first request)', {
        error: (err as Error).message,
      });
    }
  }, 1000);
}

start().catch(err => {
  logger.error('Failed to start Edge Worker', { error: err.message });
  process.exit(1);
});
