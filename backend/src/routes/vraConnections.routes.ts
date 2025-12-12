/**
 * VRA Network Connection Routes
 * 
 * API endpoints for publishers to connect and manage ad network integrations
 */

import { Router, Request, Response } from 'express';
import { Pool } from 'pg';
import { authenticate } from '../middleware/auth';
import { requireFeature } from '../utils/featureFlags';
import { NetworkConnectorService, NetworkType, SUPPORTED_NETWORKS } from '../services/vra/networkConnectorService';
import { VraSyncService } from '../services/vra/vraSyncService';
import logger from '../utils/logger';

const router = Router();

// Middleware - require authentication and VRA feature flag
router.use(authenticate);
router.use(requireFeature('vraEnabled'));

let connectorService: NetworkConnectorService;
let syncService: VraSyncService;

// Initialize services (called from app setup)
export function initVraConnectionRoutes(pool: Pool): void {
  connectorService = new NetworkConnectorService(pool);
  syncService = new VraSyncService(pool);
}

/**
 * GET /api/v1/vra/networks
 * List all supported ad networks and their requirements
 */
router.get('/networks', async (req: Request, res: Response) => {
  try {
    const networks = Object.entries(SUPPORTED_NETWORKS).map(([type, config]) => ({
      type,
      name: config.name,
      authType: config.authType,
      requiredFields: config.requiredFields,
      docsUrl: config.docsUrl,
      hasOAuth: config.authType === 'oauth',
    }));

    res.json({ success: true, networks });
  } catch (err) {
    logger.error('Failed to list networks', { error: (err as Error).message });
    res.status(500).json({ success: false, error: 'Failed to list networks' });
  }
});

/**
 * GET /api/v1/vra/connections
 * List all network connections for the current publisher
 */
router.get('/connections', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Publisher ID required' });
    }

    const connections = await connectorService.getConnections(publisherId);
    
    // Add network metadata to each connection
    const enriched = connections.map(conn => ({
      ...conn,
      networkName: SUPPORTED_NETWORKS[conn.networkType]?.name || conn.networkType,
      docsUrl: SUPPORTED_NETWORKS[conn.networkType]?.docsUrl,
    }));

    res.json({ success: true, connections: enriched });
  } catch (err) {
    logger.error('Failed to list connections', { error: (err as Error).message });
    res.status(500).json({ success: false, error: 'Failed to list connections' });
  }
});

/**
 * POST /api/v1/vra/connections
 * Create a new network connection
 */
router.post('/connections', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Publisher ID required' });
    }

    const { networkType, displayName, credentials, networkAccountId, networkAppIds } = req.body;

    if (!networkType || !SUPPORTED_NETWORKS[networkType as NetworkType]) {
      return res.status(400).json({ 
        success: false, 
        error: `Invalid network type. Supported: ${Object.keys(SUPPORTED_NETWORKS).join(', ')}` 
      });
    }

    if (!credentials || typeof credentials !== 'object') {
      return res.status(400).json({ success: false, error: 'Credentials are required' });
    }

    const connection = await connectorService.createConnection({
      publisherId,
      networkType: networkType as NetworkType,
      displayName,
      credentials,
      networkAccountId,
      networkAppIds,
    });

    res.status(201).json({ 
      success: true, 
      connection,
      message: 'Connection created. Validation in progress...',
    });
  } catch (err) {
    logger.error('Failed to create connection', { error: (err as Error).message });
    res.status(400).json({ success: false, error: (err as Error).message });
  }
});

/**
 * GET /api/v1/vra/connections/:id
 * Get a specific connection
 */
router.get('/connections/:id', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const connection = await connectorService.getConnection(req.params.id);

    if (!connection) {
      return res.status(404).json({ success: false, error: 'Connection not found' });
    }

    if (connection.publisherId !== publisherId) {
      return res.status(403).json({ success: false, error: 'Access denied' });
    }

    res.json({ 
      success: true, 
      connection: {
        ...connection,
        networkName: SUPPORTED_NETWORKS[connection.networkType]?.name,
      },
    });
  } catch (err) {
    logger.error('Failed to get connection', { error: (err as Error).message });
    res.status(500).json({ success: false, error: 'Failed to get connection' });
  }
});

/**
 * DELETE /api/v1/vra/connections/:id
 * Delete a network connection
 */
router.delete('/connections/:id', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const connection = await connectorService.getConnection(req.params.id);

    if (!connection) {
      return res.status(404).json({ success: false, error: 'Connection not found' });
    }

    if (connection.publisherId !== publisherId) {
      return res.status(403).json({ success: false, error: 'Access denied' });
    }

    await connectorService.deleteConnection(req.params.id);
    res.json({ success: true, message: 'Connection deleted' });
  } catch (err) {
    logger.error('Failed to delete connection', { error: (err as Error).message });
    res.status(500).json({ success: false, error: 'Failed to delete connection' });
  }
});

/**
 * POST /api/v1/vra/connections/:id/validate
 * Validate/test a connection
 */
router.post('/connections/:id/validate', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const connection = await connectorService.getConnection(req.params.id);

    if (!connection) {
      return res.status(404).json({ success: false, error: 'Connection not found' });
    }

    if (connection.publisherId !== publisherId) {
      return res.status(403).json({ success: false, error: 'Access denied' });
    }

    const result = await connectorService.validateConnection(req.params.id);
    res.json({ success: true, validation: result });
  } catch (err) {
    logger.error('Failed to validate connection', { error: (err as Error).message });
    res.status(500).json({ success: false, error: 'Failed to validate connection' });
  }
});

/**
 * POST /api/v1/vra/connections/:id/sync
 * Manually trigger a sync for a connection
 */
router.post('/connections/:id/sync', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const connection = await connectorService.getConnection(req.params.id);

    if (!connection) {
      return res.status(404).json({ success: false, error: 'Connection not found' });
    }

    if (connection.publisherId !== publisherId) {
      return res.status(403).json({ success: false, error: 'Access denied' });
    }

    const result = await syncService.syncConnection(req.params.id);
    res.json({ success: true, sync: result });
  } catch (err) {
    logger.error('Failed to sync connection', { error: (err as Error).message });
    res.status(500).json({ success: false, error: (err as Error).message });
  }
});

/**
 * GET /api/v1/vra/oauth/:network/url
 * Get OAuth authorization URL for a network
 */
router.get('/oauth/:network/url', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const networkType = req.params.network as NetworkType;

    if (!SUPPORTED_NETWORKS[networkType]) {
      return res.status(400).json({ success: false, error: 'Invalid network type' });
    }

    const redirectUri = `${process.env.API_BASE_URL || 'https://api.apexmediation.ee'}/api/v1/vra/oauth/${networkType}/callback`;
    const url = connectorService.getOAuthUrl(networkType, publisherId!, redirectUri);

    if (!url) {
      return res.status(400).json({ success: false, error: 'This network does not use OAuth' });
    }

    res.json({ success: true, url });
  } catch (err) {
    logger.error('Failed to get OAuth URL', { error: (err as Error).message });
    res.status(500).json({ success: false, error: 'Failed to get OAuth URL' });
  }
});

/**
 * GET /api/v1/vra/oauth/:network/callback
 * OAuth callback handler
 */
router.get('/oauth/:network/callback', async (req: Request, res: Response) => {
  try {
    const networkType = req.params.network as NetworkType;
    const { code, state, error } = req.query;

    if (error) {
      return res.redirect(`${process.env.CONSOLE_URL}/settings/networks?error=${encodeURIComponent(error as string)}`);
    }

    if (!code || !state) {
      return res.redirect(`${process.env.CONSOLE_URL}/settings/networks?error=missing_params`);
    }

    // Decode state to get publisher info
    const stateData = JSON.parse(Buffer.from(state as string, 'base64').toString());
    const { publisherId } = stateData;

    const redirectUri = `${process.env.API_BASE_URL || 'https://api.apexmediation.ee'}/api/v1/vra/oauth/${networkType}/callback`;
    const tokens = await connectorService.handleOAuthCallback(networkType, code as string, redirectUri);

    // Create or update connection with OAuth tokens
    await connectorService.createConnection({
      publisherId,
      networkType,
      credentials: {
        accessToken: tokens.accessToken,
        refreshToken: tokens.refreshToken,
        expiresAt: tokens.expiresAt?.toISOString(),
      },
    });

    res.redirect(`${process.env.CONSOLE_URL}/settings/networks?connected=${networkType}`);
  } catch (err) {
    logger.error('OAuth callback failed', { error: (err as Error).message });
    res.redirect(`${process.env.CONSOLE_URL}/settings/networks?error=${encodeURIComponent((err as Error).message)}`);
  }
});

/**
 * POST /api/v1/vra/reconcile
 * Manually trigger reconciliation for the current publisher
 */
router.post('/reconcile', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Publisher ID required' });
    }

    const { from, to } = req.body;
    const windowEnd = to ? new Date(to) : new Date();
    windowEnd.setHours(0, 0, 0, 0);
    const windowStart = from ? new Date(from) : new Date(windowEnd);
    if (!from) windowStart.setDate(windowStart.getDate() - 7);

    const result = await syncService.runReconciliation(publisherId, windowStart, windowEnd);
    res.json({ success: true, reconciliation: result });
  } catch (err) {
    logger.error('Failed to run reconciliation', { error: (err as Error).message });
    res.status(500).json({ success: false, error: (err as Error).message });
  }
});

/**
 * GET /api/v1/vra/alerts
 * Get alerts for the current publisher
 */
router.get('/alerts', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Publisher ID required' });
    }

    const status = req.query.status as string || 'open';
    const limit = parseInt(req.query.limit as string) || 50;

    const alerts = await syncService.getAlerts(publisherId, status, limit);
    res.json({ success: true, alerts, total: alerts.length });
  } catch (err) {
    logger.error('Failed to get alerts', { error: (err as Error).message });
    res.status(500).json({ success: false, error: 'Failed to get alerts' });
  }
});

export default router;
