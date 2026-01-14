import bcrypt from 'bcryptjs'
import { randomBytes } from 'crypto'
import { query } from '../utils/postgres'
import { sha256Hex, md5_16 } from '../utils/crypto'
import { redis } from '../utils/redis'

export type AppApiKeyRow = {
  id: string
  publisher_id: string
  app_id: string
  environment: string
  secret: string
  secret_digest: string | null
  prefix: string
  last4: string
  created_at: string
  updated_at: string
  last_used_at: string | null
  revoked_at: string | null
}

export type RedactedAppApiKey = {
  id: string
  appId: string
  environment: string
  prefix: string
  last4: string
  createdAt: string
  lastUsedAt: string | null
  revokedAt: string | null
}

const redact = (k: AppApiKeyRow): RedactedAppApiKey => ({
  id: k.id,
  appId: k.app_id,
  environment: k.environment,
  prefix: k.prefix,
  last4: k.last4,
  createdAt: k.created_at,
  lastUsedAt: k.last_used_at,
  revokedAt: k.revoked_at,
})

const genSecret = (prefix: 'ak_live' | 'ak_test') => `${prefix}_${randomBytes(24).toString('hex')}`

export async function listByApp(publisherId: string, appId: string): Promise<RedactedAppApiKey[]> {
  const res = await query<AppApiKeyRow>(
    `SELECT k.*
       FROM app_api_keys k
       JOIN apps a ON a.id = k.app_id
      WHERE k.app_id = $1
        AND a.publisher_id = $2
      ORDER BY k.created_at DESC`,
    [appId, publisherId]
  )
  return res.rows.map(redact)
}

export async function createForApp(
  publisherId: string,
  appId: string,
  input?: { live?: boolean; environment?: 'production' | 'staging' }
): Promise<{ key: RedactedAppApiKey; secret: string } | null> {
  const live = Boolean(input?.live)
  const env = (input?.environment ?? 'production') as 'production' | 'staging'
  const prefix: 'ak_live' | 'ak_test' = live ? 'ak_live' : 'ak_test'

  const secret = genSecret(prefix)
  const secretHash = await bcrypt.hash(secret, 10)
  const secretDigest = sha256Hex(secret)

  const res = await query<AppApiKeyRow>(
    `INSERT INTO app_api_keys (publisher_id, app_id, environment, secret, secret_digest, prefix, last4)
     SELECT $1, $2, $3, $4, $5, $6, $7
      WHERE EXISTS (SELECT 1 FROM apps WHERE id = $2 AND publisher_id = $1)
     RETURNING *`,
    [publisherId, appId, env, secretHash, secretDigest, prefix, secret.slice(-4)]
  )

  const row = res.rows[0]
  if (!row) return null

  return { key: redact(row), secret }
}

export async function rotate(
  publisherId: string,
  appId: string,
  keyId: string
): Promise<{ key: RedactedAppApiKey; secret: string } | null> {
  // Revoke old key (scoped to publisher + app)
  const revoked = await query<{ id: string; prefix: string }>(
    `UPDATE app_api_keys k
        SET revoked_at = NOW(), updated_at = NOW()
       FROM apps a
      WHERE k.id = $1
        AND k.app_id = $2
        AND a.id = k.app_id
        AND a.publisher_id = $3
        AND k.revoked_at IS NULL
    RETURNING k.id, k.prefix`,
    [keyId, appId, publisherId]
  )

  const old = revoked.rows[0]
  if (!old) return null

  const live = old.prefix === 'ak_live'
  return createForApp(publisherId, appId, { live, environment: 'production' })
}

export async function revoke(publisherId: string, appId: string, keyId: string): Promise<boolean> {
  const res = await query(
    `UPDATE app_api_keys k
        SET revoked_at = NOW(), updated_at = NOW()
       FROM apps a
      WHERE k.id = $1
        AND k.app_id = $2
        AND a.id = k.app_id
        AND a.publisher_id = $3
        AND k.revoked_at IS NULL`,
    [keyId, appId, publisherId]
  )
  return (res.rowCount ?? 0) > 0
}

export async function findActiveByDigest(digest: string): Promise<Pick<AppApiKeyRow, 'id' | 'publisher_id' | 'app_id' | 'environment' | 'secret' | 'revoked_at'> | null> {
  const res = await query<Pick<AppApiKeyRow, 'id' | 'publisher_id' | 'app_id' | 'environment' | 'secret' | 'revoked_at'>>(
    `SELECT id, publisher_id, app_id, environment, secret, revoked_at
       FROM app_api_keys
      WHERE secret_digest = $1
      LIMIT 1`,
    [digest]
  )
  return res.rows[0] || null
}

export async function touchLastUsed(id: string): Promise<void> {
  const ttlSeconds = 5 * 60
  const redisKey = `app_api_key:last_used:${id}`

  // If Redis is available, gate to at most one DB UPDATE per TTL window per key.
  const gate = await redis.setIfNotExists(redisKey, '1', ttlSeconds)
  if (gate === false) {
    return
  }

  // Always keep a DB-side staleness predicate so that even if Redis is down,
  // we avoid rewriting the row on every request.
  await query(
    `UPDATE app_api_keys
        SET last_used_at = NOW(), updated_at = NOW()
      WHERE id = $1
        AND (last_used_at IS NULL OR last_used_at < NOW() - interval '5 minutes')`,
    [id]
  )
}

export async function recordUsage(
  keyId: string | null,
  req: { baseUrl?: string; path?: string; method?: string; ip?: string; get?: (name: string) => string | undefined },
  status: string
): Promise<void> {
  try {
    const ip = req.ip || ''
    const ua = (req.get ? req.get('user-agent') : '') || ''
    const route = `${req.baseUrl || ''}${req.path || ''}`.slice(0, 255)
    const method = String(req.method || '').slice(0, 10)

    await query(
      `INSERT INTO app_api_key_usages (api_key_id, route, method, ip_hash, ua_hash, status)
       VALUES ($1, $2, $3, $4, $5, $6)`,
      [
        keyId,
        route,
        method,
        ip ? md5_16(ip) : null,
        ua ? md5_16(ua) : null,
        status.slice(0, 3),
      ]
    )
  } catch {
    // best-effort only
  }
}
