/**
 * Privacy Utilities for User Identifier Protection
 * 
 * SECURITY FIX: Implements HMAC-SHA256 with rotated secrets for user identifier
 * hashing to prevent rainbow table attacks.
 * 
 * BACKGROUND: Standard identifiers like IDFAs/GAIDs are vulnerable to 
 * de-anonymization via rainbow table attacks if hashed with SHA256 alone.
 * Using HMAC with a system-level secret makes pre-computed tables useless.
 * 
 * KEY ROTATION:
 * - Secrets should be rotated periodically (quarterly recommended)
 * - During rotation, both old and new secrets are active
 * - Hashes include a version prefix for migration support
 */

import { createHmac, randomBytes, timingSafeEqual } from 'crypto';
import logger from './logger';

// Secret management
const SECRET_ENV_VAR = 'USER_IDENTIFIER_HMAC_SECRET';
const PREVIOUS_SECRET_ENV_VAR = 'USER_IDENTIFIER_HMAC_SECRET_PREVIOUS';
const HASH_VERSION = 'v1'; // Increment on algorithm changes

// Minimum secret length (32 bytes = 256 bits)
const MIN_SECRET_LENGTH = 32;

/**
 * Get the current HMAC secret, generating one if not configured
 * 
 * In production, this MUST be set via environment variable.
 * In development, we'll generate a temporary secret and warn.
 */
function getSecret(): Buffer {
  const secretHex = process.env[SECRET_ENV_VAR];
  
  if (!secretHex) {
    // Development fallback - generate ephemeral secret
    if (process.env.NODE_ENV !== 'production') {
      logger.warn(
        `${SECRET_ENV_VAR} not set - using ephemeral secret. ` +
        'Set this in production for consistent hashing!'
      );
      // Use a deterministic development secret (DO NOT use in production)
      return Buffer.from('development-only-secret-do-not-use-in-production!');
    }
    
    throw new Error(
      `${SECRET_ENV_VAR} must be set in production. ` +
      `Generate with: openssl rand -hex 32`
    );
  }
  
  const secret = Buffer.from(secretHex, 'hex');
  
  if (secret.length < MIN_SECRET_LENGTH) {
    throw new Error(
      `${SECRET_ENV_VAR} must be at least ${MIN_SECRET_LENGTH} bytes (${MIN_SECRET_LENGTH * 2} hex chars)`
    );
  }
  
  return secret;
}

/**
 * Get the previous secret for rotation support (optional)
 */
function getPreviousSecret(): Buffer | null {
  const secretHex = process.env[PREVIOUS_SECRET_ENV_VAR];
  if (!secretHex) return null;
  
  return Buffer.from(secretHex, 'hex');
}

/**
 * Hash a user identifier using HMAC-SHA256
 * 
 * Output format: `v1:` + hex(HMAC-SHA256(secret, identifier))
 * 
 * The version prefix allows for algorithm upgrades without breaking existing hashes.
 * 
 * @param userIdentifier - Raw user identifier (IDFA, GAID, etc.)
 * @returns Versioned HMAC hash string
 */
export function hashUserIdentifier(userIdentifier: string): string {
  if (!userIdentifier || typeof userIdentifier !== 'string') {
    // Return a deterministic hash for empty/invalid inputs
    return `${HASH_VERSION}:00000000000000000000000000000000`;
  }
  
  const secret = getSecret();
  const hmac = createHmac('sha256', secret);
  hmac.update(userIdentifier.toLowerCase().trim());
  const hash = hmac.digest('hex');
  
  return `${HASH_VERSION}:${hash}`;
}

/**
 * Verify if a user identifier matches a previously computed hash
 * 
 * Supports key rotation by checking against both current and previous secrets.
 * Uses timing-safe comparison to prevent timing attacks.
 * 
 * @param userIdentifier - Raw user identifier to verify
 * @param storedHash - Previously stored hash value
 * @returns True if the identifier matches
 */
export function verifyUserIdentifier(userIdentifier: string, storedHash: string): boolean {
  if (!storedHash || !storedHash.includes(':')) {
    // Legacy unhashed or invalid format
    return false;
  }
  
  const [version, expectedHash] = storedHash.split(':');
  
  if (version !== HASH_VERSION) {
    // Future: handle hash migration for older versions
    logger.warn('Hash version mismatch', { version, expected: HASH_VERSION });
    return false;
  }
  
  // Check against current secret
  const currentHash = hashUserIdentifier(userIdentifier);
  const currentHashValue = currentHash.split(':')[1];
  
  if (timingSafeEqual(Buffer.from(currentHashValue, 'hex'), Buffer.from(expectedHash, 'hex'))) {
    return true;
  }
  
  // Check against previous secret (rotation support)
  const previousSecret = getPreviousSecret();
  if (previousSecret) {
    const hmac = createHmac('sha256', previousSecret);
    hmac.update(userIdentifier.toLowerCase().trim());
    const previousHashValue = hmac.digest('hex');
    
    if (timingSafeEqual(Buffer.from(previousHashValue, 'hex'), Buffer.from(expectedHash, 'hex'))) {
      return true;
    }
  }
  
  return false;
}

/**
 * Generate a new HMAC secret for rotation
 * 
 * @returns Hex-encoded 32-byte secret
 */
export function generateNewSecret(): string {
  return randomBytes(32).toString('hex');
}

/**
 * Create a deterministic assignment hash for experiment bucketing
 * 
 * This is different from the privacy hash - it's used for consistent
 * user assignment to experiment arms. Uses HMAC with experiment seed
 * to ensure assignments are reproducible but not reversible.
 * 
 * @param userIdentifier - Raw user identifier
 * @param placementId - Placement ID
 * @param experimentSeed - Experiment-specific seed
 * @returns Assignment percentage (0-99)
 */
export function computeAssignmentBucket(
  userIdentifier: string,
  placementId: string,
  experimentSeed: string
): number {
  const input = `${userIdentifier}:${placementId}:${experimentSeed}`;
  const hmac = createHmac('sha256', experimentSeed);
  hmac.update(input);
  const hash = hmac.digest('hex');
  
  // Use first 8 hex chars (32 bits) for assignment
  const hashInt = parseInt(hash.substring(0, 8), 16);
  return hashInt % 100;
}
