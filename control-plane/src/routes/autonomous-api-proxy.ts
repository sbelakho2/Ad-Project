/**
 * AUTONOMOUS SALES API PROXY
 * 
 * This module proxies all autonomous sales API requests to the test server.
 * The app server control-plane is STRICTLY A CLIENT - no sales automation
 * logic runs here.
 * 
 * Architecture:
 *   - Test Server (157.90.126.205:4001): Runs sales automation work
 *   - App Server (this): Proxies requests to test server
 * 
 * All endpoints are forwarded to: http://TEST_SERVER_HOST:4001/api/autonomous/*
 */

import { Router, Request, Response } from 'express';
import { logger } from '../logger';

const router = Router();

// Test server configuration
// CRITICAL: Default to 127.0.0.1 (SSH tunnel) NOT the Test Server IP directly!
// Per Hetzner checklist: "Use 127.0.0.1 for SALES_API_HOST on App Server - The tunnel handles forwarding to Test Server"
// The sales-api-tunnel.service on App Server forwards localhost:4001 → Test Server:4001
// Using the Test Server IP (157.90.126.205) directly would bypass the tunnel and fail.
const TEST_SERVER_HOST = process.env.SALES_API_HOST || '127.0.0.1';
const TEST_SERVER_PORT = process.env.SALES_API_PORT || '4001';
const TEST_SERVER_API_KEY = process.env.SALES_API_KEY || process.env.AUTONOMOUS_API_KEY || '';
const SALES_API_BASE = `http://${TEST_SERVER_HOST}:${TEST_SERVER_PORT}`;

// Log configuration at startup (helpful for debugging tunnel issues)
logger.info('Autonomous API Proxy configured', {
  salesApiHost: TEST_SERVER_HOST,
  salesApiPort: TEST_SERVER_PORT,
  usingTunnel: TEST_SERVER_HOST === '127.0.0.1',
  warning: TEST_SERVER_HOST !== '127.0.0.1' ? 'NOT using SSH tunnel - this may fail if direct connection is blocked' : undefined,
});

/**
 * Proxy helper - forwards request to test server
 */
async function proxyToTestServer(
  req: Request, 
  res: Response, 
  path: string,
  options: { method?: string; body?: unknown } = {}
): Promise<void> {
  const url = `${SALES_API_BASE}/api/autonomous${path}`;
  const method = options.method || req.method;
  
  try {
    const fetchOptions: RequestInit = {
      method,
      headers: {
        'Content-Type': 'application/json',
        'X-API-Key': TEST_SERVER_API_KEY,
        // Forward auth header if present
        ...(req.headers.authorization ? { 'Authorization': req.headers.authorization as string } : {}),
      },
    };
    
    // Include body for POST/PUT/PATCH
    if (['POST', 'PUT', 'PATCH'].includes(method) && (options.body || req.body)) {
      fetchOptions.body = JSON.stringify(options.body || req.body);
    }
    
    logger.debug('Proxying to test server', { url, method });
    
    const response = await fetch(url, fetchOptions);
    const data = await response.json();
    
    res.status(response.status).json(data);
  } catch (err) {
    logger.error('Proxy to test server failed', { 
      url, 
      method, 
      error: (err as Error).message 
    });
    res.status(502).json({ 
      error: 'Sales API unavailable',
      detail: (err as Error).message,
    });
  }
}

// ============================================================================
// HEALTH & STATUS
// ============================================================================

router.get('/health', (req, res) => proxyToTestServer(req, res, '/health'));
router.get('/stats', (req, res) => proxyToTestServer(req, res, '/stats'));

// ============================================================================
// WORKER ENDPOINTS (for edge-worker on test server)
// ============================================================================

router.get('/sources', (req, res) => {
  const query = req.url.includes('?') ? req.url.substring(req.url.indexOf('?')) : '';
  proxyToTestServer(req, res, `/sources${query}`);
});

router.post('/sources/:id/scraped', (req, res) => {
  proxyToTestServer(req, res, `/sources/${req.params.id}/scraped`);
});

router.get('/contacts/unscored', (req, res) => {
  const query = req.url.includes('?') ? req.url.substring(req.url.indexOf('?')) : '';
  proxyToTestServer(req, res, `/contacts/unscored${query}`);
});

router.post('/contacts/:id/score', (req, res) => {
  proxyToTestServer(req, res, `/contacts/${req.params.id}/score`);
});

// ============================================================================
// ORCHESTRATION ENDPOINTS
// ============================================================================

router.post('/discover', (req, res) => proxyToTestServer(req, res, '/discover'));
router.post('/score', (req, res) => proxyToTestServer(req, res, '/score'));
router.post('/compose', (req, res) => proxyToTestServer(req, res, '/compose'));
router.post('/classify', (req, res) => proxyToTestServer(req, res, '/classify'));
router.post('/feedback', (req, res) => proxyToTestServer(req, res, '/feedback'));

// ============================================================================
// WEBHOOK ENDPOINTS
// ============================================================================

router.post('/webhook/sendgrid', (req, res) => proxyToTestServer(req, res, '/webhook/sendgrid'));
router.post('/webhook/email-events', (req, res) => proxyToTestServer(req, res, '/webhook/email-events'));

// ============================================================================
// READ ENDPOINTS
// ============================================================================

router.get('/arms', (req, res) => {
  const query = req.url.includes('?') ? req.url.substring(req.url.indexOf('?')) : '';
  proxyToTestServer(req, res, `/arms${query}`);
});

router.post('/arms', (req, res) => proxyToTestServer(req, res, '/arms'));

router.get('/templates', (req, res) => {
  const query = req.url.includes('?') ? req.url.substring(req.url.indexOf('?')) : '';
  proxyToTestServer(req, res, `/templates${query}`);
});

router.post('/templates', (req, res) => proxyToTestServer(req, res, '/templates'));
router.post('/templates/preview', (req, res) => proxyToTestServer(req, res, '/templates/preview'));

router.get('/inbox', (req, res) => {
  const query = req.url.includes('?') ? req.url.substring(req.url.indexOf('?')) : '';
  proxyToTestServer(req, res, `/inbox${query}`);
});

router.post('/inbox/:id', (req, res) => {
  proxyToTestServer(req, res, `/inbox/${req.params.id}`);
});

router.get('/inbox/stats', (req, res) => proxyToTestServer(req, res, '/inbox/stats'));

// ============================================================================
// CATCH-ALL for any other autonomous endpoints
// ============================================================================

router.all('*', (req, res) => {
  const path = req.url;
  logger.info('Proxying catch-all autonomous request', { method: req.method, path });
  proxyToTestServer(req, res, path);
});

export default router;
