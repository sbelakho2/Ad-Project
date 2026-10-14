/**
 * SDK Configuration hash computation for cross-platform parity verification.
 * Uses SHA-256 with deterministic JSON serialization.
 * Hash format: "v1:<hex-digest>"
 */

import type { AuctionClientConfig } from './auctionClient';

export interface SDKConfig {
  appId: string;
  version: number;
  floors: Record<string, number>;
  caps: Record<string, { daily: number; hourly: number }>;
  pacing: {
    enabled: boolean;
    minIntervalMs: number;
  };
  networks: string[];
  features: {
    bidding: boolean;
    waterfall: boolean;
    bannerRefresh: boolean;
    bannerRefreshIntervalMs: number;
  };
  compliance: {
    gdprApplies: boolean;
    ccpaApplies: boolean;
    coppaApplies: boolean;
  };
}

/**
 * Build canonical JSON string from config with sorted keys.
 * This ensures deterministic output for hash computation.
 */
function buildCanonicalConfigJson(config: SDKConfig): string {
  const sortedMap: Record<string, unknown> = {};

  // Add fields in alphabetical order
  sortedMap.appId = config.appId;

  // Caps - sorted by placement ID
  const caps: Record<string, { daily: number; hourly: number }> = {};
  const sortedCapKeys = Object.keys(config.caps).sort();
  for (const placementId of sortedCapKeys) {
    caps[placementId] = config.caps[placementId];
  }
  sortedMap.caps = caps;

  // Compliance - sorted alphabetically
  sortedMap.compliance = {
    ccpaApplies: config.compliance.ccpaApplies,
    coppaApplies: config.compliance.coppaApplies,
    gdprApplies: config.compliance.gdprApplies,
  };

  // Features - sorted alphabetically
  sortedMap.features = {
    bannerRefresh: config.features.bannerRefresh,
    bannerRefreshIntervalMs: config.features.bannerRefreshIntervalMs,
    bidding: config.features.bidding,
    waterfall: config.features.waterfall,
  };

  // Floors - sorted by placement ID
  const floors: Record<string, number> = {};
  const sortedFloorKeys = Object.keys(config.floors).sort();
  for (const placementId of sortedFloorKeys) {
    floors[placementId] = config.floors[placementId];
  }
  sortedMap.floors = floors;

  // Networks - sorted alphabetically
  sortedMap.networks = [...config.networks].sort();

  // Pacing
  sortedMap.pacing = {
    enabled: config.pacing.enabled,
    minIntervalMs: config.pacing.minIntervalMs,
  };

  // Version
  sortedMap.version = config.version;

  return JSON.stringify(sortedMap);
}

/**
 * Convert ArrayBuffer to hex string
 */
function arrayBufferToHex(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

/**
 * Compute SHA-256 hash using Web Crypto API
 */
async function sha256(message: string): Promise<string> {
  const encoder = new TextEncoder();
  const data = encoder.encode(message);

  // Use Web Crypto API (available in browsers and Node.js 15+)
  if (typeof crypto !== 'undefined' && crypto.subtle) {
    const hashBuffer = await crypto.subtle.digest('SHA-256', data);
    return arrayBufferToHex(hashBuffer);
  }

  // Fallback for environments without Web Crypto
  throw new Error('SHA-256 not available: Web Crypto API not found');
}

/**
 * Compute deterministic SHA-256 hash of SDK configuration.
 * Uses sorted JSON serialization to ensure cross-platform parity with server.
 * Hash format: "v1:<hex-digest>"
 *
 * @param config - SDK configuration object
 * @returns Promise resolving to configuration hash string
 */
export async function computeConfigHash(config: SDKConfig): Promise<string> {
  const canonicalJson = buildCanonicalConfigJson(config);
  const hash = await sha256(canonicalJson);
  return `v1:${hash}`;
}

/**
 * Validate that local config hash matches server hash.
 * Useful for debugging configuration sync issues.
 *
 * @param config - Local SDK configuration
 * @param serverHash - Hash returned from /api/v1/config/sdk/config/hash endpoint
 * @returns Promise resolving to true if hashes match, false otherwise
 */
export async function validateConfigHash(
  config: SDKConfig,
  serverHash: string
): Promise<boolean> {
  if (!config || !serverHash) {
    return false;
  }

  const localHash = await computeConfigHash(config);
  return localHash === serverHash;
}

/**
 * Fetch SDK configuration from server and compute its hash.
 *
 * @param endpoint - API endpoint base URL
 * @param appId - Application ID
 * @returns Promise resolving to config and hash
 */
export async function fetchConfigWithHash(
  endpoint: string,
  appId: string
): Promise<{ config: SDKConfig; hash: string }> {
  const url = `${endpoint.replace(/\/$/, '')}/api/v1/config/sdk/config?appId=${encodeURIComponent(appId)}`;

  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to fetch config: ${response.statusText}`);
  }

  const data = (await response.json()) as { config: SDKConfig; hash: string };
  return data;
}

/**
 * Create a default SDKConfig from AuctionClientConfig for hash computation
 * when server config is not available.
 */
export function createDefaultConfig(clientConfig: AuctionClientConfig): SDKConfig {
  return {
    appId: clientConfig.appId ?? '',
    version: 1,
    floors: {},
    caps: {},
    pacing: {
      enabled: false,
      minIntervalMs: 0,
    },
    networks: [],
    features: {
      bidding: true,
      waterfall: true,
      bannerRefresh: false,
      bannerRefreshIntervalMs: 0,
    },
    compliance: {
      gdprApplies: false,
      ccpaApplies: false,
      coppaApplies: false,
    },
  };
}
