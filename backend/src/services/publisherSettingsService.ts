import { query } from '../utils/postgres';
import logger from '../utils/logger';

export interface FraudSettings {
  enabled: boolean;
  sensitivity: 'low' | 'medium' | 'high';
  autoBlock: boolean;
  autoBlockThreshold: number;
  alertThreshold: number;
  excludedNetworks: string[];
  excludedPlacements: string[];
  customRules: any[];
}

const DEFAULT_FRAUD_SETTINGS: FraudSettings = {
  enabled: true,
  sensitivity: 'medium',
  autoBlock: false,
  autoBlockThreshold: 0.9,
  alertThreshold: 0.7,
  excludedNetworks: [],
  excludedPlacements: [],
  customRules: [],
};

// Simple in-memory cache to avoid DB hits on every auction
const settingsCache = new Map<string, { settings: FraudSettings; expiresAt: number }>();
const CACHE_TTL_MS = 60000; // 1 minute

export async function getFraudSettings(publisherId: string): Promise<FraudSettings> {
  const now = Date.now();
  const cached = settingsCache.get(publisherId);
  if (cached && cached.expiresAt > now) {
    return cached.settings;
  }

  try {
    const result = await query(
      'SELECT settings FROM publisher_settings WHERE publisher_id = $1 AND category = $2',
      [publisherId, 'fraud']
    );

    let settings = DEFAULT_FRAUD_SETTINGS;
    if (result.rows.length > 0) {
      settings = { ...DEFAULT_FRAUD_SETTINGS, ...result.rows[0].settings };
    }

    settingsCache.set(publisherId, {
      settings,
      expiresAt: now + CACHE_TTL_MS,
    });

    return settings;
  } catch (error) {
    logger.error('Error fetching fraud settings for auction:', { publisherId, error });
    return DEFAULT_FRAUD_SETTINGS;
  }
}
