/**
 * SDK Config Parity Service
 * 
 * SDK_CHECKS Part 8.1: Console → SDK config parity hash
 * 
 * Computes a deterministic hash of SDK configuration that both Console
 * and SDK can compute independently to verify config parity.
 */

import { createHash } from 'crypto';
import { query } from '../utils/postgres';
import logger from '../utils/logger';
import { getFeatureFlags } from '../config/featureFlags';

// -----------------------------------------------------
// Types
// -----------------------------------------------------

export interface SdkConfigData {
  appId: string;
  appVersion?: string;
  
  // Placements
  placements: Array<{
    id: string;
    name: string;
    type: string;
    floorCpm?: number;
    refreshIntervalMs?: number;
    timeoutMs?: number;
  }>;
  
  // Adapter settings
  adapters: Array<{
    name: string;
    enabled: boolean;
    priority?: number;
    timeoutMs?: number;
  }>;
  
  // Feature flags relevant to SDK
  flags: {
    enableOmSdk?: boolean;
    enableCircuitBreakers?: boolean;
    enablePacing?: boolean;
    enableTelemetry?: boolean;
    debugMode?: boolean;
  };
  
  // Environment settings
  environment: {
    apiBaseUrl: string;
    auctionEndpoint: string;
    telemetryEndpoint: string;
  };
  
  // Version for hash compatibility
  configVersion: string;
}

export interface SdkConfigHash {
  appId: string;
  hash: string;
  shortHash: string; // First 8 chars for display
  generatedAt: string;
  configVersion: string;
}

// -----------------------------------------------------
// Hash Computation
// -----------------------------------------------------

/**
 * Current hash algorithm version.
 * Increment when changing the normalization or hashing algorithm.
 */
const HASH_VERSION = 'v1';

/**
 * Compute a deterministic hash of SDK config.
 * This algorithm must match the SDK-side implementation.
 * 
 * Algorithm:
 * 1. Normalize all config values (sort arrays, lowercase strings)
 * 2. Create canonical JSON representation
 * 3. SHA-256 hash the canonical JSON
 * 4. Prefix with version identifier for forward compatibility
 * 
 * Hash format: "v1:<hex-digest>"
 */
export function computeConfigHash(config: SdkConfigData): string {
  // Normalize config for deterministic hashing
  const normalized = normalizeConfig(config);
  
  // Create canonical JSON (sorted keys)
  const canonical = JSON.stringify(normalized, Object.keys(normalized).sort());
  
  // SHA-256 hash with version prefix (matches Web SDK format)
  const digest = createHash('sha256').update(canonical).digest('hex');
  return `${HASH_VERSION}:${digest}`;
}

/**
 * Normalize config for deterministic comparison
 */
function normalizeConfig(config: SdkConfigData): Record<string, unknown> {
  return {
    v: config.configVersion,
    app: config.appId.toLowerCase(),
    placements: [...config.placements]
      .sort((a, b) => a.id.localeCompare(b.id))
      .map(p => ({
        id: p.id.toLowerCase(),
        name: p.name,
        type: p.type.toLowerCase(),
        floor: p.floorCpm ?? 0,
        refresh: p.refreshIntervalMs ?? 0,
        timeout: p.timeoutMs ?? 0,
      })),
    adapters: [...config.adapters]
      .filter(a => a.enabled)
      .sort((a, b) => a.name.localeCompare(b.name))
      .map(a => ({
        name: a.name.toLowerCase(),
        priority: a.priority ?? 0,
        timeout: a.timeoutMs ?? 0,
      })),
    flags: {
      omsdk: !!config.flags.enableOmSdk,
      breakers: !!config.flags.enableCircuitBreakers,
      pacing: !!config.flags.enablePacing,
      telemetry: !!config.flags.enableTelemetry,
      debug: !!config.flags.debugMode,
    },
    env: {
      api: config.environment.apiBaseUrl.toLowerCase().replace(/\/$/, ''),
      auction: config.environment.auctionEndpoint.toLowerCase().replace(/\/$/, ''),
    },
  };
}

/**
 * Get config hash for an app
 */
export async function getAppConfigHash(appId: string): Promise<SdkConfigHash | null> {
  try {
    const config = await fetchAppConfig(appId);
    if (!config) return null;
    
    const hash = computeConfigHash(config);
    
    // Extract short hash from digest portion (after "v1:" prefix)
    const digestPart = hash.includes(':') ? hash.split(':')[1] : hash;
    
    return {
      appId,
      hash,
      shortHash: digestPart.substring(0, 8),
      generatedAt: new Date().toISOString(),
      configVersion: config.configVersion,
    };
  } catch (error) {
    logger.warn('Failed to compute config hash', { error, appId });
    return null;
  }
}

/**
 * Fetch app config from database
 */
async function fetchAppConfig(appId: string): Promise<SdkConfigData | null> {
  try {
    // Get app info
    const appResult = await query(
      `SELECT id, bundle_id, name, publisher_id FROM apps WHERE id = $1`,
      [appId]
    );
    
    if (appResult.rows.length === 0) return null;
    
    // Get placements
    const placementResult = await query(
      `SELECT id, name, type, 
              (config->>'floorCpm')::numeric as floor_cpm,
              (config->>'refreshIntervalMs')::int as refresh_interval_ms,
              (config->>'timeoutMs')::int as timeout_ms
       FROM placements 
       WHERE app_id = $1 AND status = 'active'`,
      [appId]
    );
    
    // Get adapter configs
    const adapterResult = await query(
      `SELECT adapter_name, enabled, priority, 
              (config->>'timeoutMs')::int as timeout_ms
       FROM adapter_configs 
       WHERE publisher_id = (SELECT publisher_id FROM apps WHERE id = $1)`,
      [appId]
    );
    
    // Get SDK-specific feature flags from environment
    const sdkFlags = {
      enableOmSdk: process.env.SDK_ENABLE_OMSDK === 'true',
      enableCircuitBreakers: process.env.SDK_ENABLE_CIRCUIT_BREAKERS !== 'false',
      enablePacing: process.env.SDK_ENABLE_PACING !== 'false',
      enableTelemetry: process.env.SDK_ENABLE_TELEMETRY !== 'false',
      debugMode: process.env.SDK_DEBUG_MODE === 'true',
    };
    
    const config: SdkConfigData = {
      appId,
      configVersion: '1.0',
      placements: placementResult.rows.map(p => ({
        id: p.id,
        name: p.name,
        type: p.type,
        floorCpm: p.floor_cpm ? parseFloat(p.floor_cpm) : undefined,
        refreshIntervalMs: p.refresh_interval_ms,
        timeoutMs: p.timeout_ms,
      })),
      adapters: adapterResult.rows.map(a => ({
        name: a.adapter_name,
        enabled: a.enabled ?? true,
        priority: a.priority,
        timeoutMs: a.timeout_ms,
      })),
      flags: {
        enableOmSdk: sdkFlags.enableOmSdk,
        enableCircuitBreakers: sdkFlags.enableCircuitBreakers,
        enablePacing: sdkFlags.enablePacing,
        enableTelemetry: sdkFlags.enableTelemetry,
        debugMode: sdkFlags.debugMode,
      },
      environment: {
        apiBaseUrl: process.env.API_BASE_URL || 'https://api.apexmediation.ee',
        auctionEndpoint: process.env.AUCTION_ENDPOINT || 'https://api.apexmediation.ee/v1/auction',
        telemetryEndpoint: process.env.TELEMETRY_ENDPOINT || 'https://api.apexmediation.ee/v1/analytics',
      },
    };
    
    return config;
  } catch (error) {
    logger.warn('Failed to fetch app config', { error, appId });
    return null;
  }
}

/**
 * Compare SDK-reported hash with server-computed hash
 */
export interface ParityCheckResult {
  match: boolean;
  serverHash: string;
  clientHash: string;
  serverShortHash: string;
  clientShortHash: string;
  drift: boolean;
  driftDetails?: {
    fieldsMismatched: string[];
    suggestion: string;
  };
}

export async function checkConfigParity(
  appId: string,
  clientHash: string
): Promise<ParityCheckResult> {
  const serverHashResult = await getAppConfigHash(appId);
  
  // Extract digest portion from client hash (after "v1:" prefix if present)
  const clientDigest = clientHash.includes(':') ? clientHash.split(':')[1] : clientHash;
  
  if (!serverHashResult) {
    return {
      match: false,
      serverHash: '',
      clientHash,
      serverShortHash: '',
      clientShortHash: clientDigest.substring(0, 8),
      drift: true,
      driftDetails: {
        fieldsMismatched: ['app_not_found'],
        suggestion: 'App not found in server. Ensure SDK is initialized with correct appId.',
      },
    };
  }
  
  const match = serverHashResult.hash === clientHash;
  
  return {
    match,
    serverHash: serverHashResult.hash,
    clientHash,
    serverShortHash: serverHashResult.shortHash,
    clientShortHash: clientDigest.substring(0, 8),
    drift: !match,
    driftDetails: match ? undefined : {
      fieldsMismatched: ['unknown'], // Would need full config comparison for details
      suggestion: 'SDK config differs from server. Run console sync or check placement settings.',
    },
  };
}

export default {
  computeConfigHash,
  getAppConfigHash,
  checkConfigParity,
};
