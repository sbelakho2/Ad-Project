import { Router } from 'express';
import revenueRoutes from './revenue.routes';
import placementRoutes from './placement.routes';
import adapterRoutes from './adapter.routes';
import fraudRoutes from './fraud.routes';
import payoutRoutes from './payout.routes';
import analyticsRoutes from './analytics.routes';
import reportingRoutes from './reporting.routes';
import rtbRoutes from './rtb.routes';
import authRoutes from './auth';
import skadnetworkRoutes from './skadnetwork.routes';
import consentRoutes from './consent.routes';
import abTestingRoutes from './abTesting.routes';
import dataExportRoutes from './dataExport.routes';
import queuesRoutes from './queues.routes';
import transparencyRoutes from './transparency.routes';
import metaRoutes from './meta.routes';
import billingRoutes from './billing.routes';
import webhooksRoutes from './webhooks.routes';
import adminRoutes from './admin.routes';
import privacyRoutes from './privacy.routes';
import migrationRoutes from './migration.routes';
import keysRoutes from './keys.routes';
import appsRoutes from './apps.routes';
import integrationsRoutes from './integrations.routes';
import flagsRoutes from './flags.routes';
import dashboardRoutes from './dashboard.routes';
import byoRoutes from './byo.routes';
import toolsRoutes from './tools.routes';
import configRoutes from './config.routes';
import { killSwitchGuard } from '../middleware/featureFlags';
import vraRoutes from './vra.routes';
import vraConnectionsRoutes from './vraConnections.routes';

const router = Router();

// Feature flags kill-switch guard (applies to all /api/v1 routes with allowlist in middleware)
router.use(killSwitchGuard);

// Mount route modules
// NOTE: Route order is sensitive (see FIX-11/694). Keep /migration before /admin to avoid
// unintended matches and ensure OpenAPI grouping remains stable.
router.use('/flags', flagsRoutes);
router.use('/auth', authRoutes);
router.use('/keys', keysRoutes);
router.use('/apps', appsRoutes);
router.use('/integrations', integrationsRoutes);
router.use('/meta', metaRoutes);
router.use('/webhooks', webhooksRoutes);
router.use('/revenue', revenueRoutes);
router.use('/placements', placementRoutes);
router.use('/adapters', adapterRoutes);
router.use('/fraud', fraudRoutes);
router.use('/payouts', payoutRoutes);
router.use('/analytics', analyticsRoutes);
router.use('/reporting', reportingRoutes);
router.use('/dashboard', dashboardRoutes);
router.use('/rtb', rtbRoutes);
router.use('/skadnetwork', skadnetworkRoutes);
router.use('/consent', consentRoutes);
router.use('/ab-testing', abTestingRoutes);
router.use('/data-export', dataExportRoutes);
router.use('/queues', queuesRoutes);
router.use('/transparency', transparencyRoutes);
router.use('/billing', billingRoutes);
router.use('/config', configRoutes);
// Migration before admin (ordering requirement per FIX-11/694)
router.use('/migration', migrationRoutes);
router.use('/admin', adminRoutes);
router.use('/privacy', privacyRoutes);
router.use('/byo', byoRoutes); // BYO model credential vault & transparency
router.use('/tools', toolsRoutes); // Developer tools (e.g., app-ads.txt inspector)
// VRA routes mounted at root to expose /recon/* and /proofs/* under /api/v1
router.use('/', vraRoutes);
// VRA network connection management
router.use('/vra', vraConnectionsRoutes);

export default router;
