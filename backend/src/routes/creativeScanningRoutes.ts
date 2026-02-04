import { Router } from 'express';
import { z } from 'zod';
import { query } from '../utils/postgres';
import { redis } from '../utils/redis';
import logger from '../utils/logger';
import crypto from 'crypto';

const router = Router();

// MARK: - Types

const CreativeScanRequestSchema = z.object({
  creativeUrl: z.string().url(),
  creativeType: z.enum(['image', 'video', 'html', 'vast']),
  advertiserDomain: z.string().optional(),
  categories: z.array(z.string()).optional(),
  metadata: z.record(z.string()).optional(),
});

const BulkScanRequestSchema = z.object({
  creatives: z.array(CreativeScanRequestSchema).max(100),
  callbackUrl: z.string().url().optional(),
});

interface ScanResult {
  creativeId: string;
  status: 'approved' | 'rejected' | 'pending' | 'error';
  brandSafetyScore: number; // 0-100
  categories: string[];
  flags: ScanFlag[];
  malwareDetected: boolean;
  redirectChain: string[];
  landingPageUrl?: string;
  scanDuration: number;
  scannedAt: Date;
}

interface ScanFlag {
  type: string;
  severity: 'low' | 'medium' | 'high' | 'critical';
  description: string;
  location?: string;
}

// MARK: - Constants

const BLOCKED_CATEGORIES = [
  'adult',
  'gambling',
  'weapons',
  'drugs',
  'malware',
  'phishing',
  'hate_speech',
  'violence',
  'deceptive',
];

const CACHE_TTL = 60 * 60 * 24; // 24 hours

// MARK: - Routes

/**
 * POST /api/creative-scanning/scan
 * Scan a single creative for brand safety
 */
router.post('/scan', async (req, res) => {
  try {
    const body = CreativeScanRequestSchema.parse(req.body);
    const creativeId = generateCreativeId(body.creativeUrl);
    
    // Check cache first
    const cached = await getCachedResult(creativeId);
    if (cached) {
      return res.json({
        success: true,
        result: cached,
        cached: true,
      });
    }
    
    // Perform scan
    const result = await scanCreative(body);
    
    // Cache result
    await cacheResult(creativeId, result);
    
    // Store in database
    await storeScanResult(result);
    
    res.json({
      success: true,
      result,
      cached: false,
    });
  } catch (error) {
    logger.error('Creative scan failed:', error);
    res.status(400).json({
      success: false,
      error: error instanceof Error ? error.message : 'Scan failed',
    });
  }
});

/**
 * POST /api/creative-scanning/scan/bulk
 * Scan multiple creatives asynchronously
 */
router.post('/scan/bulk', async (req, res) => {
  try {
    const { creatives, callbackUrl } = BulkScanRequestSchema.parse(req.body);
    
    const batchId = crypto.randomUUID();
    
    // Queue batch job
    await redis.set(
      `creative_scan:batch:${batchId}`,
      {
        status: 'processing',
        total: creatives.length,
        completed: 0,
        results: [],
      },
      60 * 60 // 1 hour TTL
    );
    
    // Process asynchronously
    processBulkScan(batchId, creatives, callbackUrl).catch((err) => {
      logger.error(`Bulk scan batch ${batchId} failed:`, err);
    });
    
    res.json({
      success: true,
      batchId,
      statusUrl: `/api/creative-scanning/batch/${batchId}`,
    });
  } catch (error) {
    logger.error('Bulk creative scan failed:', error);
    res.status(400).json({
      success: false,
      error: error instanceof Error ? error.message : 'Bulk scan failed',
    });
  }
});

/**
 * GET /api/creative-scanning/batch/:batchId
 * Get status of a bulk scan batch
 */
router.get('/batch/:batchId', async (req, res) => {
  try {
    const { batchId } = req.params;
    
    const batchData = await redis.get<{
      status: string;
      total: number;
      completed: number;
      results: ScanResult[];
    }>(`creative_scan:batch:${batchId}`);
    
    if (!batchData) {
      return res.status(404).json({
        success: false,
        error: 'Batch not found',
      });
    }
    
    res.json({
      success: true,
      batch: batchData,
    });
  } catch (error) {
    res.status(500).json({
      success: false,
      error: 'Failed to get batch status',
    });
  }
});

/**
 * GET /api/creative-scanning/result/:creativeId
 * Get scan result for a creative
 */
router.get('/result/:creativeId', async (req, res) => {
  try {
    const { creativeId } = req.params;
    
    // Check cache
    const cached = await getCachedResult(creativeId);
    if (cached) {
      return res.json({
        success: true,
        result: cached,
      });
    }
    
    // Check database
    const dbResult = await query<{
      creative_id: string;
      status: string;
      brand_safety_score: number;
      categories: string[];
      flags: ScanFlag[];
      malware_detected: boolean;
      redirect_chain: string[];
      landing_page_url: string | null;
      scan_duration: number;
      scanned_at: Date;
    }>(`SELECT * FROM creative_scans WHERE creative_id = $1`, [creativeId]);
    
    if (!dbResult.rows.length) {
      return res.status(404).json({
        success: false,
        error: 'Creative not found',
      });
    }
    
    const row = dbResult.rows[0];
    const result: ScanResult = {
      creativeId: row.creative_id,
      status: row.status as ScanResult['status'],
      brandSafetyScore: row.brand_safety_score,
      categories: row.categories,
      flags: row.flags,
      malwareDetected: row.malware_detected,
      redirectChain: row.redirect_chain,
      landingPageUrl: row.landing_page_url || undefined,
      scanDuration: row.scan_duration,
      scannedAt: row.scanned_at,
    };
    
    res.json({
      success: true,
      result,
    });
  } catch (error) {
    res.status(500).json({
      success: false,
      error: 'Failed to get result',
    });
  }
});

/**
 * POST /api/creative-scanning/validate
 * Quick validation check (returns approve/reject only)
 */
router.post('/validate', async (req, res) => {
  try {
    const body = CreativeScanRequestSchema.parse(req.body);
    const creativeId = generateCreativeId(body.creativeUrl);
    
    // Check cache
    const cached = await getCachedResult(creativeId);
    if (cached) {
      return res.json({
        approved: cached.status === 'approved',
        reason: cached.status === 'rejected' ? cached.flags[0]?.description : undefined,
      });
    }
    
    // Quick scan
    const result = await scanCreative(body);
    await cacheResult(creativeId, result);
    
    res.json({
      approved: result.status === 'approved',
      reason: result.status === 'rejected' ? result.flags[0]?.description : undefined,
    });
  } catch (error) {
    res.status(400).json({
      approved: false,
      reason: 'Validation failed',
    });
  }
});

// MARK: - Scan Implementation

async function scanCreative(
  request: z.infer<typeof CreativeScanRequestSchema>
): Promise<ScanResult> {
  const startTime = Date.now();
  const creativeId = generateCreativeId(request.creativeUrl);
  
  const flags: ScanFlag[] = [];
  let brandSafetyScore = 100;
  let malwareDetected = false;
  const redirectChain: string[] = [];
  let landingPageUrl: string | undefined;
  
  try {
    // 1. URL Analysis
    const urlFlags = await analyzeUrl(request.creativeUrl);
    flags.push(...urlFlags);
    
    // 2. Redirect Chain Analysis
    const redirectResult = await analyzeRedirects(request.creativeUrl);
    redirectChain.push(...redirectResult.chain);
    landingPageUrl = redirectResult.finalUrl;
    
    if (redirectResult.suspicious) {
      flags.push({
        type: 'suspicious_redirect',
        severity: 'high',
        description: 'Suspicious redirect chain detected',
      });
    }
    
    // 3. Content Analysis (type-specific)
    switch (request.creativeType) {
      case 'image': {
        const imageFlags = await analyzeImage(request.creativeUrl);
        flags.push(...imageFlags);
        break;
      }
        
      case 'video': {
        const videoFlags = await analyzeVideo(request.creativeUrl);
        flags.push(...videoFlags);
        break;
      }
        
      case 'html': {
        const htmlFlags = await analyzeHtml(request.creativeUrl);
        flags.push(...htmlFlags);
        break;
      }
        
      case 'vast': {
        const vastFlags = await analyzeVast(request.creativeUrl);
        flags.push(...vastFlags);
        break;
      }
    }
    
    // 4. Malware Detection
    malwareDetected = await checkMalware(request.creativeUrl, redirectChain);
    if (malwareDetected) {
      flags.push({
        type: 'malware',
        severity: 'critical',
        description: 'Malware detected in creative or landing page',
      });
    }
    
    // 5. Domain Reputation
    if (request.advertiserDomain) {
      const domainFlags = await checkDomainReputation(request.advertiserDomain);
      flags.push(...domainFlags);
    }
    
    // 6. Category Analysis
    const detectedCategories = await detectCategories(request.creativeUrl);
    const blockedCategories = detectedCategories.filter((c) =>
      BLOCKED_CATEGORIES.includes(c.toLowerCase())
    );
    
    if (blockedCategories.length > 0) {
      flags.push({
        type: 'blocked_category',
        severity: 'high',
        description: `Blocked categories detected: ${blockedCategories.join(', ')}`,
      });
    }
    
    // Calculate brand safety score
    brandSafetyScore = calculateBrandSafetyScore(flags, malwareDetected);
    
    // Determine status
    const status = determineStatus(brandSafetyScore, flags, malwareDetected);
    
    return {
      creativeId,
      status,
      brandSafetyScore,
      categories: detectedCategories,
      flags,
      malwareDetected,
      redirectChain,
      landingPageUrl,
      scanDuration: Date.now() - startTime,
      scannedAt: new Date(),
    };
  } catch (error) {
    logger.error(`Creative scan error for ${creativeId}:`, error);
    
    return {
      creativeId,
      status: 'error',
      brandSafetyScore: 0,
      categories: [],
      flags: [
        {
          type: 'scan_error',
          severity: 'high',
          description: error instanceof Error ? error.message : 'Scan failed',
        },
      ],
      malwareDetected: false,
      redirectChain: [],
      scanDuration: Date.now() - startTime,
      scannedAt: new Date(),
    };
  }
}

// MARK: - Analysis Functions

async function analyzeUrl(url: string): Promise<ScanFlag[]> {
  const flags: ScanFlag[] = [];
  
  try {
    const parsed = new URL(url);
    
    // Check for suspicious patterns
    if (parsed.hostname.includes('bit.ly') || parsed.hostname.includes('goo.gl')) {
      flags.push({
        type: 'url_shortener',
        severity: 'medium',
        description: 'URL shortener detected - may hide destination',
      });
    }
    
    // Check for IP address URLs
    if (/^\d+\.\d+\.\d+\.\d+$/.test(parsed.hostname)) {
      flags.push({
        type: 'ip_address_url',
        severity: 'medium',
        description: 'Direct IP address URL detected',
      });
    }
    
    // Check protocol
    if (parsed.protocol !== 'https:') {
      flags.push({
        type: 'insecure_protocol',
        severity: 'low',
        description: 'Non-HTTPS URL detected',
      });
    }
    
  } catch (error) {
    flags.push({
      type: 'invalid_url',
      severity: 'high',
      description: 'Invalid URL format',
    });
  }
  
  return flags;
}

async function analyzeRedirects(
  url: string
): Promise<{ chain: string[]; finalUrl: string; suspicious: boolean }> {
  const chain: string[] = [url];
  let currentUrl = url;
  let suspicious = false;
  const maxRedirects = 10;
  
  try {
    for (let i = 0; i < maxRedirects; i++) {
      const response = await fetch(currentUrl, {
        method: 'HEAD',
        redirect: 'manual',
      });
      
      if (response.status >= 300 && response.status < 400) {
        const location = response.headers.get('location');
        if (location) {
          currentUrl = new URL(location, currentUrl).href;
          chain.push(currentUrl);
          
          // Check for cross-domain redirects
          const prevDomain = new URL(chain[chain.length - 2]).hostname;
          const newDomain = new URL(currentUrl).hostname;
          if (prevDomain !== newDomain) {
            suspicious = true;
          }
        } else {
          break;
        }
      } else {
        break;
      }
    }
  } catch (error) {
    // Network error during redirect check
  }
  
  // Suspicious if too many redirects
  if (chain.length > 5) {
    suspicious = true;
  }
  
  return {
    chain,
    finalUrl: currentUrl,
    suspicious,
  };
}

async function analyzeImage(url: string): Promise<ScanFlag[]> {
  const flags: ScanFlag[] = [];
  
  // In production, integrate with image classification API
  // For now, basic checks
  
  try {
    const response = await fetch(url, { method: 'HEAD' });
    const contentType = response.headers.get('content-type');
    
    if (!contentType?.startsWith('image/')) {
      flags.push({
        type: 'invalid_content_type',
        severity: 'high',
        description: `Expected image, got ${contentType}`,
      });
    }
    
    const contentLength = response.headers.get('content-length');
    if (contentLength && parseInt(contentLength) > 5 * 1024 * 1024) {
      flags.push({
        type: 'large_file',
        severity: 'low',
        description: 'Image file exceeds 5MB',
      });
    }
  } catch (error) {
    flags.push({
      type: 'fetch_error',
      severity: 'medium',
      description: 'Failed to fetch image',
    });
  }
  
  return flags;
}

async function analyzeVideo(url: string): Promise<ScanFlag[]> {
  const flags: ScanFlag[] = [];
  
  try {
    const response = await fetch(url, { method: 'HEAD' });
    const contentType = response.headers.get('content-type');
    
    if (!contentType?.startsWith('video/')) {
      flags.push({
        type: 'invalid_content_type',
        severity: 'high',
        description: `Expected video, got ${contentType}`,
      });
    }
  } catch (error) {
    flags.push({
      type: 'fetch_error',
      severity: 'medium',
      description: 'Failed to fetch video',
    });
  }
  
  return flags;
}

async function analyzeHtml(url: string): Promise<ScanFlag[]> {
  const flags: ScanFlag[] = [];
  
  try {
    const response = await fetch(url);
    const html = await response.text();
    
    // Check for suspicious JavaScript
    if (html.includes('eval(') || html.includes('document.write(')) {
      flags.push({
        type: 'suspicious_javascript',
        severity: 'high',
        description: 'Suspicious JavaScript patterns detected',
      });
    }
    
    // Check for external scripts
    const scriptMatches = html.match(/<script[^>]*src=[^>]*>/gi) || [];
    if (scriptMatches.length > 10) {
      flags.push({
        type: 'excessive_scripts',
        severity: 'medium',
        description: 'Excessive external scripts detected',
      });
    }
    
    // Check for iframes
    const iframeMatches = html.match(/<iframe/gi) || [];
    if (iframeMatches.length > 3) {
      flags.push({
        type: 'excessive_iframes',
        severity: 'medium',
        description: 'Excessive iframes detected',
      });
    }
    
  } catch (error) {
    flags.push({
      type: 'fetch_error',
      severity: 'medium',
      description: 'Failed to fetch HTML',
    });
  }
  
  return flags;
}

async function analyzeVast(url: string): Promise<ScanFlag[]> {
  const flags: ScanFlag[] = [];
  
  try {
    const response = await fetch(url);
    const xml = await response.text();
    
    // Basic VAST validation
    if (!xml.includes('<VAST') || !xml.includes('</VAST>')) {
      flags.push({
        type: 'invalid_vast',
        severity: 'high',
        description: 'Invalid VAST XML structure',
      });
    }
    
    // Check for wrapper chains
    const wrapperCount = (xml.match(/<Wrapper>/gi) || []).length;
    if (wrapperCount > 5) {
      flags.push({
        type: 'excessive_wrappers',
        severity: 'medium',
        description: 'Excessive VAST wrapper redirects',
      });
    }
    
  } catch (error) {
    flags.push({
      type: 'fetch_error',
      severity: 'medium',
      description: 'Failed to fetch VAST',
    });
  }
  
  return flags;
}

async function checkMalware(url: string, redirectChain: string[]): Promise<boolean> {
  // In production, integrate with Google Safe Browsing API or similar
  // For now, check against known malware domains
  
  const malwareDomains = [
    'malware.com',
    'phishing.net',
    // Add known malware domains
  ];
  
  const allUrls = [url, ...redirectChain];
  
  for (const checkUrl of allUrls) {
    try {
      const hostname = new URL(checkUrl).hostname;
      if (malwareDomains.some((d) => hostname.includes(d))) {
        return true;
      }
    } catch {
      // Invalid URL
    }
  }
  
  return false;
}

async function checkDomainReputation(domain: string): Promise<ScanFlag[]> {
  const flags: ScanFlag[] = [];
  
  // In production, integrate with domain reputation API
  // Check domain age, SSL certificate, etc.
  
  return flags;
}

async function detectCategories(url: string): Promise<string[]> {
  // In production, integrate with content classification API
  // For now, return empty array
  return [];
}

// MARK: - Helpers

function generateCreativeId(url: string): string {
  return crypto.createHash('sha256').update(url).digest('hex').substring(0, 32);
}

function calculateBrandSafetyScore(flags: ScanFlag[], malwareDetected: boolean): number {
  if (malwareDetected) return 0;
  
  let score = 100;
  
  for (const flag of flags) {
    switch (flag.severity) {
      case 'critical':
        score -= 50;
        break;
      case 'high':
        score -= 25;
        break;
      case 'medium':
        score -= 10;
        break;
      case 'low':
        score -= 5;
        break;
    }
  }
  
  return Math.max(0, score);
}

function determineStatus(
  score: number,
  flags: ScanFlag[],
  malwareDetected: boolean
): ScanResult['status'] {
  if (malwareDetected) return 'rejected';
  if (flags.some((f) => f.severity === 'critical')) return 'rejected';
  if (score >= 70) return 'approved';
  if (score >= 50) return 'pending'; // Manual review
  return 'rejected';
}

async function getCachedResult(creativeId: string): Promise<ScanResult | null> {
  return await redis.get<ScanResult>(`creative_scan:result:${creativeId}`);
}

async function cacheResult(creativeId: string, result: ScanResult): Promise<void> {
  await redis.set(
    `creative_scan:result:${creativeId}`,
    result,
    CACHE_TTL
  );
}

async function storeScanResult(result: ScanResult): Promise<void> {
  try {
    await query(`
      INSERT INTO creative_scans (
        creative_id, status, brand_safety_score, categories, flags,
        malware_detected, redirect_chain, landing_page_url, scan_duration, scanned_at
      ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
      ON CONFLICT (creative_id) DO UPDATE SET
        status = EXCLUDED.status,
        brand_safety_score = EXCLUDED.brand_safety_score,
        categories = EXCLUDED.categories,
        flags = EXCLUDED.flags,
        malware_detected = EXCLUDED.malware_detected,
        redirect_chain = EXCLUDED.redirect_chain,
        landing_page_url = EXCLUDED.landing_page_url,
        scan_duration = EXCLUDED.scan_duration,
        scanned_at = EXCLUDED.scanned_at
    `, [
      result.creativeId,
      result.status,
      result.brandSafetyScore,
      JSON.stringify(result.categories),
      JSON.stringify(result.flags),
      result.malwareDetected,
      JSON.stringify(result.redirectChain),
      result.landingPageUrl,
      result.scanDuration,
      result.scannedAt,
    ]);
  } catch (error) {
    logger.error('Failed to store scan result:', error);
  }
}

async function processBulkScan(
  batchId: string,
  creatives: z.infer<typeof CreativeScanRequestSchema>[],
  callbackUrl?: string
): Promise<void> {
  const results: ScanResult[] = [];
  
  for (let i = 0; i < creatives.length; i++) {
    const result = await scanCreative(creatives[i]);
    results.push(result);
    
    // Update batch status
    await redis.set(
      `creative_scan:batch:${batchId}`,
      {
        status: 'processing',
        total: creatives.length,
        completed: i + 1,
        results,
      },
      60 * 60
    );
    
    // Cache individual result
    await cacheResult(result.creativeId, result);
  }
  
  // Mark batch complete
  await redis.set(
    `creative_scan:batch:${batchId}`,
    {
      status: 'complete',
      total: creatives.length,
      completed: creatives.length,
      results,
    },
    60 * 60
  );
  
  // Send callback if provided
  if (callbackUrl) {
    try {
      await fetch(callbackUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          batchId,
          status: 'complete',
          results,
        }),
      });
    } catch (error) {
      logger.error(`Failed to send callback for batch ${batchId}:`, error);
    }
  }
}

export default router;
