import crypto from 'node:crypto';
import logger from '../../utils/logger';
import { query, insertMany } from '../../utils/postgres';
import { vraProofsCoveragePct, vraProofsIssuanceDurationSeconds, vraProofsVerifyFailuresTotal } from '../../utils/prometheus';

type CHExpectedRow = { receipt_hash?: string; request_id: string };
type CHDailyRootRow = { day: string; merkle_root: string };

/**
 * Key rotation configuration
 * 
 * Environment variables:
 * - PROOFS_SIGNING_PRIVATE_KEY: Current active signing key (PEM format)
 * - PROOFS_SIGNING_PUBLIC_KEY: Current active public key (PEM format)
 * - PROOFS_SIGNING_KEY_ID: Key ID/version (e.g., "v1", "2024-01")
 * - PROOFS_SIGNING_PREVIOUS_PUBLIC_KEYS: JSON array of previous public keys for verification
 *   Format: [{"keyId": "v0", "publicKey": "-----BEGIN PUBLIC KEY-----\n..."}]
 */
interface SigningKey {
  keyId: string;
  privateKey?: string;  // PEM format, only for signing
  publicKey: string;    // PEM format, for verification
  validFrom?: string;   // ISO date
  validUntil?: string;  // ISO date
}

function getCurrentKeyId(): string {
  return process.env.PROOFS_SIGNING_KEY_ID || 'v1';
}

function getSigningKeys(): { current: SigningKey | null; previous: SigningKey[] } {
  const current: SigningKey | null = process.env.PROOFS_SIGNING_PRIVATE_KEY && process.env.PROOFS_SIGNING_PUBLIC_KEY
    ? {
        keyId: getCurrentKeyId(),
        privateKey: process.env.PROOFS_SIGNING_PRIVATE_KEY,
        publicKey: process.env.PROOFS_SIGNING_PUBLIC_KEY,
      }
    : null;

  let previous: SigningKey[] = [];
  try {
    const prevKeysJson = process.env.PROOFS_SIGNING_PREVIOUS_PUBLIC_KEYS;
    if (prevKeysJson) {
      const parsed = JSON.parse(prevKeysJson);
      if (Array.isArray(parsed)) {
        previous = parsed.filter(k => k.keyId && k.publicKey);
      }
    }
  } catch (e) {
    logger.warn('VRA Proofs: Failed to parse PROOFS_SIGNING_PREVIOUS_PUBLIC_KEYS', { error: (e as Error).message });
  }

  return { current, previous };
}

export function sha256Hex(buf: Buffer | string): string {
  const b = Buffer.isBuffer(buf) ? buf : Buffer.from(buf);
  return crypto.createHash('sha256').update(b).digest('hex');
}

export function computeMerkleRoot(hashes: string[]): string {
  // Normalize: lowercase hex, filter invalid
  const clean = hashes
    .map((h) => String(h || '').trim().toLowerCase())
    .filter((h) => /^[0-9a-f]+$/.test(h));
  if (clean.length === 0) {
    // Root of empty set — define as sha256('') for determinism
    return sha256Hex('');
  }
  let layer = clean.slice().sort();
  while (layer.length > 1) {
    const next: string[] = [];
    if (layer.length % 2 === 1) {
      // duplicate last to make even
      layer.push(layer[layer.length - 1]);
    }
    for (let i = 0; i < layer.length; i += 2) {
      const left = Buffer.from(layer[i], 'hex');
      const right = Buffer.from(layer[i + 1], 'hex');
      const combined = Buffer.concat([left, right]);
      next.push(sha256Hex(combined));
    }
    layer = next;
  }
  return layer[0];
}

/**
 * Sign a message with the current key and return signature + key ID
 */
function signWithCurrentKey(messageHex: string): { sig: string; keyId: string } | null {
  const { current } = getSigningKeys();
  if (!current || !current.privateKey) {
    logger.warn('VRA Proofs: No signing key configured');
    return null;
  }
  
  try {
    const key = crypto.createPrivateKey(current.privateKey);
    const sig = crypto.sign(null, Buffer.from(messageHex, 'hex'), key);
    return { sig: sig.toString('hex'), keyId: current.keyId };
  } catch (e) {
    logger.warn('VRA Proofs: signing failed', { error: (e as Error).message, keyId: current.keyId });
    return null;
  }
}

/**
 * Verify a signature, trying current and previous keys
 * Returns the keyId that verified the signature, or null if no key worked
 */
function verifyWithAnyKey(messageHex: string, signatureHex: string, preferKeyId?: string): { ok: boolean; keyId?: string } {
  const { current, previous } = getSigningKeys();
  
  // Build list of keys to try, with preferred key first
  const keysToTry: SigningKey[] = [];
  
  if (preferKeyId && current && current.keyId === preferKeyId) {
    keysToTry.push(current);
  } else if (preferKeyId) {
    const preferred = previous.find(k => k.keyId === preferKeyId);
    if (preferred) keysToTry.push(preferred);
  }
  
  if (current && !keysToTry.includes(current)) keysToTry.push(current);
  for (const k of previous) {
    if (!keysToTry.includes(k)) keysToTry.push(k);
  }
  
  for (const k of keysToTry) {
    try {
      const key = crypto.createPublicKey(k.publicKey);
      const ok = crypto.verify(null, Buffer.from(messageHex, 'hex'), key, Buffer.from(signatureHex, 'hex'));
      if (ok) {
        return { ok: true, keyId: k.keyId };
      }
    } catch {
      // Try next key
    }
  }
  
  return { ok: false };
}

// Legacy functions for backward compatibility
function tryEd25519SignHex(messageHex: string): string | null {
  const result = signWithCurrentKey(messageHex);
  return result?.sig || null;
}

function tryEd25519VerifyHex(messageHex: string, signatureHex: string): boolean {
  const pub = process.env.PROOFS_SIGNING_PUBLIC_KEY;
  if (!pub) return false;
  try {
    const key = crypto.createPublicKey(pub);
    return crypto.verify(null, Buffer.from(messageHex, 'hex'), key, Buffer.from(signatureHex, 'hex'));
  } catch {
    return false;
  }
}

export async function issueDailyRoot(day: string, opts?: { dryRun?: boolean }): Promise<{ day: string; root: string; sig?: string; keyId?: string; coveragePct: number; written: boolean }>
{
  const end = vraProofsIssuanceDurationSeconds.startTimer();
  const dryRun = opts?.dryRun === true;
  try {
    // Fetch all recon_expected rows for the given day
    const rowsResult = await query<CHExpectedRow>(
      `SELECT receipt_hash, request_id FROM recon_expected WHERE ts::date = $1::date`,
      [day]
    );
    const rows = rowsResult.rows;
    const total = rows.length;
    // Convert to hashes — prefer receipt_hash, fallback to sha256(request_id)
    const hashes = rows.map((r) => (r.receipt_hash && /^[0-9a-fA-F]+$/.test(r.receipt_hash) ? r.receipt_hash : sha256Hex(r.request_id)));
    const included = hashes.length;
    const coveragePct = total > 0 ? (included / total) * 100 : 0;
    const root = computeMerkleRoot(hashes);
    const messageHex = sha256Hex(Buffer.from(`daily_root:${day}:${root}`));
    const signResult = signWithCurrentKey(messageHex);
    const sig = signResult?.sig || '';
    const keyId = signResult?.keyId;

    try { vraProofsCoveragePct.set({ day }, Number(coveragePct.toFixed(2))); } catch {}

    if (!dryRun) {
      // Insert with key_id for rotation tracking (column may not exist in older schemas)
      try {
        await insertMany(
          'proofs_daily_roots',
          ['day', 'merkle_root', 'sig', 'key_id'],
          [[day, root, sig, keyId || '']],
          { onConflictColumns: ['day'], ignoreConflicts: true }
        );
      } catch {
        // Fallback to legacy columns if key_id doesn't exist
        await insertMany(
          'proofs_daily_roots',
          ['day', 'merkle_root', 'sig'],
          [[day, root, sig]],
          { onConflictColumns: ['day'], ignoreConflicts: true }
        );
      }
    }
    end();
    return { day, root, sig, keyId, coveragePct, written: !dryRun };
  } catch (e) {
    end();
    logger.warn('VRA Proofs: failed to issue daily root', { day, error: (e as Error).message });
    return { day, root: sha256Hex(''), coveragePct: 0, written: false } as any;
  }
}

export async function issueMonthlyDigest(month: string, opts?: { dryRun?: boolean }): Promise<{ month: string; digest: string; sig?: string; keyId?: string; coveragePct: number; written: boolean }>
{
  const end = vraProofsIssuanceDurationSeconds.startTimer();
  const dryRun = opts?.dryRun === true;
  try {
    const rootsResult = await query<CHDailyRootRow>(
      `SELECT to_char(day, 'YYYY-MM-DD') AS day, merkle_root
         FROM proofs_daily_roots
        WHERE to_char(day, 'YYYY-MM') = $1
        ORDER BY day ASC`,
      [month]
    );
    const roots = rootsResult.rows;
    const rootList = roots.map((r) => r.merkle_root);
    const digest = computeMerkleRoot(rootList);
    const coveragePct = roots.length > 0 ? 100 : 0; // simplistic until we track expected days
    const messageHex = sha256Hex(Buffer.from(`monthly_digest:${month}:${digest}`));
    const signResult = signWithCurrentKey(messageHex);
    const sig = signResult?.sig || '';
    const keyId = signResult?.keyId;
    
    if (!dryRun) {
      // Insert with key_id for rotation tracking (column may not exist in older schemas)
      try {
        await insertMany(
          'proofs_monthly_digest',
          ['month', 'digest', 'sig', 'key_id', 'coverage_pct', 'notes'],
          [[month, digest, sig, keyId || '', Number(coveragePct.toFixed(2)), '']],
          { onConflictColumns: ['month'], ignoreConflicts: true }
        );
      } catch {
        // Fallback to legacy columns if key_id doesn't exist
        await insertMany(
          'proofs_monthly_digest',
          ['month', 'digest', 'sig', 'coverage_pct', 'notes'],
          [[month, digest, sig, Number(coveragePct.toFixed(2)), '']],
          { onConflictColumns: ['month'], ignoreConflicts: true }
        );
      }
    }
    end();
    return { month, digest, sig, keyId, coveragePct, written: !dryRun };
  } catch (e) {
    end();
    logger.warn('VRA Proofs: failed to issue monthly digest', { month, error: (e as Error).message });
    return { month, digest: sha256Hex(''), coveragePct: 0, written: false } as any;
  }
}

export async function verifyMonthlyDigest(month: string): Promise<{ month: string; ok: boolean; keyId?: string; reason?: string }>{
  try {
    // Fetch stored digest (including key_id if available)
    const rowsResult = await query<{ month: string; digest: string; sig: string; key_id?: string }>(
      `SELECT month, digest, sig, key_id FROM proofs_monthly_digest WHERE month = $1 LIMIT 1`,
      [month]
    );
    const rows = rowsResult.rows;
    if (!rows.length) return { month, ok: false, reason: 'not_found' };
    const { digest, sig, key_id } = rows[0];
    // Recompute expected digest from daily roots
    const daysResult = await query<CHDailyRootRow>(
      `SELECT to_char(day, 'YYYY-MM-DD') AS day, merkle_root
         FROM proofs_daily_roots
        WHERE to_char(day, 'YYYY-MM') = $1
        ORDER BY day ASC`,
      [month]
    );
    const days = daysResult.rows;
    const recomputed = computeMerkleRoot(days.map((d) => d.merkle_root));
    if (recomputed !== digest) {
      vraProofsVerifyFailuresTotal.inc();
      return { month, ok: false, reason: 'digest_mismatch' };
    }
    const msgHex = sha256Hex(Buffer.from(`monthly_digest:${month}:${digest}`));
    
    // Use key rotation-aware verification
    const verifyResult = verifyWithAnyKey(msgHex, sig || '', key_id);
    if (!verifyResult.ok) {
      vraProofsVerifyFailuresTotal.inc();
      return { month, ok: false, reason: 'signature_invalid_or_missing' };
    }
    return { month, ok: true, keyId: verifyResult.keyId };
  } catch (e) {
    vraProofsVerifyFailuresTotal.inc();
    return { month, ok: false, reason: 'error' };
  }
}

/**
 * Generate a new Ed25519 keypair for signing proofs
 * Use this during key rotation to create a new key
 */
export function generateSigningKeypair(): { privateKey: string; publicKey: string } {
  const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519');
  return {
    privateKey: privateKey.export({ type: 'pkcs8', format: 'pem' }) as string,
    publicKey: publicKey.export({ type: 'spki', format: 'pem' }) as string,
  };
}

/**
 * Get current signing key status for monitoring
 */
export function getSigningKeyStatus(): { 
  hasCurrentKey: boolean; 
  currentKeyId?: string;
  previousKeysCount: number;
} {
  const { current, previous } = getSigningKeys();
  return {
    hasCurrentKey: !!current,
    currentKeyId: current?.keyId,
    previousKeysCount: previous.length,
  };
}

export default {
  sha256Hex,
  computeMerkleRoot,
  issueDailyRoot,
  issueMonthlyDigest,
  verifyMonthlyDigest,
  generateSigningKeypair,
  getSigningKeyStatus,
};
