/**
 * Publisher Settings Routes
 * 
 * Handles fraud protection, notification, and compliance settings.
 */

import { Router, Request, Response } from 'express';
import { authenticate } from '../middleware/auth';
import { query } from '../utils/postgres';
import logger from '../utils/logger';

const router = Router();

// Require authentication for all settings routes
router.use(authenticate);

// Default settings
const DEFAULT_FRAUD_SETTINGS = {
  enabled: true,
  sensitivity: 'medium',
  autoBlock: false,
  autoBlockThreshold: 0.9,
  alertThreshold: 0.7,
  excludedNetworks: [],
  excludedPlacements: [],
  customRules: [],
};

const DEFAULT_NOTIFICATION_SETTINGS = {
  emailAlerts: {
    enabled: true,
    recipients: [],
    events: ['fraud_detected', 'payout_completed', 'revenue_milestone'],
  },
  slackIntegration: {
    enabled: false,
    webhookUrl: null,
    channel: null,
    events: ['fraud_detected'],
  },
  webhooks: {
    enabled: false,
    endpoints: [],
  },
  digest: {
    enabled: true,
    frequency: 'weekly',
    recipients: [],
  },
};

const DEFAULT_COMPLIANCE_SETTINGS = {
  gdpr: {
    enabled: true,
    dataRetentionDays: 90,
    consentRequired: true,
  },
  ccpa: {
    enabled: false,
    doNotSellEnabled: false,
  },
  coppa: {
    enabled: false,
    childDirectedApps: [],
  },
  regions: {
    restrictedRegions: [],
    enabledRegions: ['US', 'EU', 'APAC'],
  },
};

/**
 * Helper to get or create publisher settings
 */
async function getOrCreateSettings(publisherId: string, category: string, defaults: object) {
  const result = await query(
    'SELECT settings FROM publisher_settings WHERE publisher_id = $1 AND category = $2',
    [publisherId, category]
  );

  if (result.rows.length > 0) {
    return { ...defaults, ...result.rows[0].settings };
  }

  // Create default settings
  await query(
    `INSERT INTO publisher_settings (publisher_id, category, settings, created_at, updated_at)
     VALUES ($1, $2, $3, NOW(), NOW())
     ON CONFLICT (publisher_id, category) DO NOTHING`,
    [publisherId, category, JSON.stringify(defaults)]
  );

  return defaults;
}

/**
 * Helper to update settings
 */
async function updateSettings(publisherId: string, category: string, settings: object) {
  await query(
    `INSERT INTO publisher_settings (publisher_id, category, settings, created_at, updated_at)
     VALUES ($1, $2, $3, NOW(), NOW())
     ON CONFLICT (publisher_id, category) 
     DO UPDATE SET settings = $3, updated_at = NOW()`,
    [publisherId, category, JSON.stringify(settings)]
  );
}

// ============== FRAUD SETTINGS ==============

/**
 * GET /settings/fraud
 * Get fraud protection settings
 */
router.get('/fraud', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const settings = await getOrCreateSettings(publisherId, 'fraud', DEFAULT_FRAUD_SETTINGS);

    res.json({ success: true, data: settings });
  } catch (error) {
    logger.error('Error fetching fraud settings:', error);
    res.status(500).json({ success: false, error: 'Failed to fetch fraud settings' });
  }
});

/**
 * PUT /settings/fraud
 * Update fraud protection settings
 */
router.put('/fraud', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const updates = req.body;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    // Validate sensitivity value
    if (updates.sensitivity && !['low', 'medium', 'high'].includes(updates.sensitivity)) {
      return res.status(400).json({ success: false, error: 'Invalid sensitivity value' });
    }

    // Validate threshold values
    if (updates.autoBlockThreshold !== undefined) {
      const threshold = parseFloat(updates.autoBlockThreshold);
      if (isNaN(threshold) || threshold < 0 || threshold > 1) {
        return res.status(400).json({ success: false, error: 'autoBlockThreshold must be between 0 and 1' });
      }
    }

    if (updates.alertThreshold !== undefined) {
      const threshold = parseFloat(updates.alertThreshold);
      if (isNaN(threshold) || threshold < 0 || threshold > 1) {
        return res.status(400).json({ success: false, error: 'alertThreshold must be between 0 and 1' });
      }
    }

    // Get current settings and merge
    const current = await getOrCreateSettings(publisherId, 'fraud', DEFAULT_FRAUD_SETTINGS);
    const merged = { ...current, ...updates };

    await updateSettings(publisherId, 'fraud', merged);

    res.json({ success: true, data: merged });
  } catch (error) {
    logger.error('Error updating fraud settings:', error);
    res.status(500).json({ success: false, error: 'Failed to update fraud settings' });
  }
});

// ============== NOTIFICATION SETTINGS ==============

/**
 * GET /settings/notifications
 * Get notification preferences
 */
router.get('/notifications', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const settings = await getOrCreateSettings(publisherId, 'notifications', DEFAULT_NOTIFICATION_SETTINGS);

    // Add current user email as default recipient if empty
    if (settings.emailAlerts && settings.emailAlerts.recipients.length === 0 && req.user?.email) {
      settings.emailAlerts.recipients = [req.user.email];
    }

    if (settings.digest && settings.digest.recipients.length === 0 && req.user?.email) {
      settings.digest.recipients = [req.user.email];
    }

    res.json({ success: true, data: settings });
  } catch (error) {
    logger.error('Error fetching notification settings:', error);
    res.status(500).json({ success: false, error: 'Failed to fetch notification settings' });
  }
});

/**
 * PUT /settings/notifications
 * Update notification preferences
 */
router.put('/notifications', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const updates = req.body;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    // Validate digest frequency
    if (updates.digest?.frequency && !['daily', 'weekly', 'monthly'].includes(updates.digest.frequency)) {
      return res.status(400).json({ success: false, error: 'Invalid digest frequency' });
    }

    // Validate email recipients format
    if (updates.emailAlerts?.recipients) {
      const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
      for (const email of updates.emailAlerts.recipients) {
        if (!emailRegex.test(email)) {
          return res.status(400).json({ success: false, error: `Invalid email: ${email}` });
        }
      }
    }

    // Get current settings and deep merge
    const current = await getOrCreateSettings(publisherId, 'notifications', DEFAULT_NOTIFICATION_SETTINGS);
    const merged = deepMerge(current, updates);

    await updateSettings(publisherId, 'notifications', merged);

    res.json({ success: true, data: merged });
  } catch (error) {
    logger.error('Error updating notification settings:', error);
    res.status(500).json({ success: false, error: 'Failed to update notification settings' });
  }
});

// ============== COMPLIANCE SETTINGS ==============

/**
 * GET /settings/compliance
 * Get regional compliance settings
 */
router.get('/compliance', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const settings = await getOrCreateSettings(publisherId, 'compliance', DEFAULT_COMPLIANCE_SETTINGS);

    res.json({ success: true, data: settings });
  } catch (error) {
    logger.error('Error fetching compliance settings:', error);
    res.status(500).json({ success: false, error: 'Failed to fetch compliance settings' });
  }
});

/**
 * PUT /settings/compliance
 * Update regional compliance settings
 */
router.put('/compliance', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const updates = req.body;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    // Validate data retention days
    if (updates.gdpr?.dataRetentionDays !== undefined) {
      const days = parseInt(updates.gdpr.dataRetentionDays, 10);
      if (isNaN(days) || days < 30 || days > 365) {
        return res.status(400).json({
          success: false,
          error: 'Data retention days must be between 30 and 365',
        });
      }
    }

    // Get current settings and deep merge
    const current = await getOrCreateSettings(publisherId, 'compliance', DEFAULT_COMPLIANCE_SETTINGS);
    const merged = deepMerge(current, updates);

    await updateSettings(publisherId, 'compliance', merged);

    res.json({ success: true, data: merged });
  } catch (error) {
    logger.error('Error updating compliance settings:', error);
    res.status(500).json({ success: false, error: 'Failed to update compliance settings' });
  }
});

/**
 * Deep merge utility for nested settings objects
 */
function deepMerge(target: any, source: any): any {
  const result = { ...target };

  for (const key of Object.keys(source)) {
    if (source[key] && typeof source[key] === 'object' && !Array.isArray(source[key])) {
      result[key] = deepMerge(target[key] || {}, source[key]);
    } else {
      result[key] = source[key];
    }
  }

  return result;
}

export default router;
