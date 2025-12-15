import crypto from 'crypto';
import { getConfig } from './config';
import { getOperatorPool, getReadOnlyPool } from './database';

export type TrustedDeviceRow = {
  id: number;
  operator_email: string;
  device_hash: string;
  device_label: string | null;
  user_agent: string | null;
  added_via: 'bootstrap' | 'email';
  added_at: Date;
  last_seen_at: Date;
  revoked_at: Date | null;
  revoked_by: string | null;
  revoked_reason: string | null;
};

function sha256Hex(input: string): string {
  return crypto.createHash('sha256').update(input).digest('hex');
}

function hmacSha256Hex(secret: string, input: string): string {
  return crypto.createHmac('sha256', secret).update(input).digest('hex');
}

export function hashDeviceId(deviceId: string): string {
  return sha256Hex(deviceId);
}

export function createChallengeToken(): string {
  return crypto.randomBytes(32).toString('base64url');
}

function getChallengeSecret(): string {
  const cfg = getConfig();
  return cfg.DEVICE_CHALLENGE_SECRET || cfg.OPERATOR_JWT_SECRET;
}

export async function isTrustedDevice(operatorEmail: string, deviceHash: string): Promise<boolean> {
  const pool = getReadOnlyPool();
  const result = await pool.query(
    `SELECT 1
     FROM control_plane.operator_trusted_devices
     WHERE operator_email = $1
       AND device_hash = $2
       AND revoked_at IS NULL
     LIMIT 1`,
    [operatorEmail, deviceHash]
  );
  return (result.rowCount ?? 0) > 0;
}

export async function countTrustedDevices(operatorEmail: string): Promise<number> {
  const pool = getReadOnlyPool();
  const result = await pool.query(
    `SELECT COUNT(*)::int AS count
     FROM control_plane.operator_trusted_devices
     WHERE operator_email = $1
       AND revoked_at IS NULL`,
    [operatorEmail]
  );
  return result.rows?.[0]?.count ?? 0;
}

export async function countDeviceHistory(operatorEmail: string): Promise<number> {
  const pool = getReadOnlyPool();
  const result = await pool.query(
    `SELECT COUNT(*)::int AS count
     FROM control_plane.operator_trusted_devices
     WHERE operator_email = $1`,
    [operatorEmail]
  );
  return result.rows?.[0]?.count ?? 0;
}

export async function touchDeviceSeen(operatorEmail: string, deviceHash: string): Promise<void> {
  const pool = getOperatorPool();
  await pool.query(
    `UPDATE control_plane.operator_trusted_devices
     SET last_seen_at = NOW()
     WHERE operator_email = $1
       AND device_hash = $2
       AND revoked_at IS NULL`,
    [operatorEmail, deviceHash]
  );
}

export async function trustDevice(params: {
  operatorEmail: string;
  deviceHash: string;
  deviceLabel?: string;
  userAgent?: string;
  addedVia: 'bootstrap' | 'email';
}): Promise<void> {
  const pool = getOperatorPool();
  await pool.query(
    `INSERT INTO control_plane.operator_trusted_devices (
       operator_email,
       device_hash,
       device_label,
       user_agent,
       added_via
     ) VALUES ($1,$2,$3,$4,$5)
     ON CONFLICT DO NOTHING`,
    [
      params.operatorEmail,
      params.deviceHash,
      params.deviceLabel || null,
      params.userAgent || null,
      params.addedVia,
    ]
  );
}

export async function listTrustedDevices(operatorEmail: string): Promise<TrustedDeviceRow[]> {
  const pool = getReadOnlyPool();
  const result = await pool.query(
    `SELECT
       id,
       operator_email,
       device_hash,
       device_label,
       user_agent,
       added_via,
       added_at,
       last_seen_at,
       revoked_at,
       revoked_by,
       revoked_reason
     FROM control_plane.operator_trusted_devices
     WHERE operator_email = $1
     ORDER BY (revoked_at IS NULL) DESC, last_seen_at DESC, added_at DESC`,
    [operatorEmail]
  );
  return result.rows as TrustedDeviceRow[];
}

export async function revokeTrustedDeviceById(params: {
  operatorEmail: string;
  deviceId: number;
  revokedBy: string;
  revokedReason?: string;
}): Promise<boolean> {
  const pool = getOperatorPool();
  const reason = params.revokedReason && params.revokedReason.trim() ? params.revokedReason.trim().slice(0, 250) : null;

  const result = await pool.query(
    `UPDATE control_plane.operator_trusted_devices
     SET revoked_at = NOW(),
         revoked_by = $3,
         revoked_reason = $4
     WHERE id = $1
       AND operator_email = $2
       AND revoked_at IS NULL`,
    [params.deviceId, params.operatorEmail, params.revokedBy, reason]
  );
  return (result.rowCount ?? 0) > 0;
}

export async function createDeviceChallenge(params: {
  operatorEmail: string;
  deviceHash: string;
  deviceLabel?: string;
  userAgent?: string;
  requestedIp?: string;
  token: string;
}): Promise<{ tokenHash: string; expiresAt: Date }> {
  const cfg = getConfig();
  const ttlMinutes = parseInt(cfg.DEVICE_CHALLENGE_TTL_MINUTES, 10);
  const expiresAt = new Date(Date.now() + ttlMinutes * 60 * 1000);
  const tokenHash = hmacSha256Hex(getChallengeSecret(), params.token);

  const pool = getOperatorPool();
  await pool.query(
    `INSERT INTO control_plane.operator_device_challenges (
       operator_email,
       device_hash,
       device_label,
       user_agent,
       token_hash,
       requested_ip,
       expires_at
     ) VALUES ($1,$2,$3,$4,$5,$6,$7)`,
    [
      params.operatorEmail,
      params.deviceHash,
      params.deviceLabel || null,
      params.userAgent || null,
      tokenHash,
      params.requestedIp || null,
      expiresAt,
    ]
  );

  return { tokenHash, expiresAt };
}

export async function confirmDeviceByToken(token: string): Promise<
  | { ok: true; operatorEmail: string; deviceHash: string; deviceLabel?: string; userAgent?: string }
  | { ok: false; reason: 'invalid' | 'expired' | 'consumed' }
> {
  const tokenHash = hmacSha256Hex(getChallengeSecret(), token);

  const pool = getOperatorPool();
  const client = await pool.connect();
  try {
    await client.query('BEGIN');

    const challenge = await client.query(
      `SELECT id, operator_email, device_hash, device_label, user_agent, expires_at, consumed_at
       FROM control_plane.operator_device_challenges
       WHERE token_hash = $1
       LIMIT 1
       FOR UPDATE`,
      [tokenHash]
    );

    if (challenge.rowCount === 0) {
      await client.query('ROLLBACK');
      return { ok: false, reason: 'invalid' };
    }

    const row = challenge.rows[0] as {
      id: number;
      operator_email: string;
      device_hash: string;
      device_label: string | null;
      user_agent: string | null;
      expires_at: Date;
      consumed_at: Date | null;
    };

    if (row.consumed_at) {
      await client.query('ROLLBACK');
      return { ok: false, reason: 'consumed' };
    }

    if (new Date(row.expires_at).getTime() < Date.now()) {
      await client.query('ROLLBACK');
      return { ok: false, reason: 'expired' };
    }

    const deviceLabel = row.device_label || null;
    const userAgent = row.user_agent || null;

    // Prefer updating an existing row (active first, else most recently revoked)
    // so re-approving a revoked device "un-revokes" it rather than creating a new active row.
    const updated = await client.query(
      `WITH target AS (
         SELECT id
         FROM control_plane.operator_trusted_devices
         WHERE operator_email = $1
           AND device_hash = $2
         ORDER BY (revoked_at IS NULL) DESC, revoked_at DESC NULLS LAST, id DESC
         LIMIT 1
       )
       UPDATE control_plane.operator_trusted_devices d
       SET revoked_at = NULL,
           revoked_by = NULL,
           revoked_reason = NULL,
           device_label = COALESCE($3, d.device_label),
           user_agent = COALESCE($4, d.user_agent),
           last_seen_at = NOW()
       WHERE d.id IN (SELECT id FROM target)
       RETURNING d.id`,
      [row.operator_email, row.device_hash, deviceLabel, userAgent]
    );

    if ((updated.rowCount ?? 0) === 0) {
      await client.query(
        `INSERT INTO control_plane.operator_trusted_devices (
           operator_email,
           device_hash,
           device_label,
           user_agent,
           added_via
         ) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT DO NOTHING`,
        [row.operator_email, row.device_hash, deviceLabel, userAgent, 'email']
      );
    }

    await client.query(
      `UPDATE control_plane.operator_device_challenges
       SET consumed_at = NOW()
       WHERE id = $1
         AND consumed_at IS NULL`,
      [row.id]
    );

    await client.query('COMMIT');

    return {
      ok: true,
      operatorEmail: row.operator_email,
      deviceHash: row.device_hash,
      deviceLabel: row.device_label || undefined,
      userAgent: row.user_agent || undefined,
    };
  } catch (err) {
    await client.query('ROLLBACK');
    throw err;
  } finally {
    client.release();
  }
}
