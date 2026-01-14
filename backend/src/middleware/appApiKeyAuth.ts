import { NextFunction, Request, Response } from 'express'
import bcrypt from 'bcryptjs'
import { sha256Hex } from '../utils/crypto'
import * as appKeysRepo from '../repositories/appApiKeyRepository'

/**
 * App-scoped API key auth for SDK-facing endpoints.
 *
 * Looks for API key in `Authorization: Bearer <key>` or `X-Api-Key`.
 * Matches by sha256 digest, then verifies bcrypt hash.
 *
 * On success, attaches `{ publisherId, appId, environment }` to `req.sdk`.
 */
export async function appApiKeyAuth(req: Request, res: Response, next: NextFunction) {
  try {
    const authorizationHeader = (req.header('authorization') || '').trim()
    const apiKeyHeader = (req.header('x-api-key') || req.header('X-Api-Key') || '').trim()
    const sdkKeyHeader = (req.header('x-sdk-key') || req.header('X-Sdk-Key') || '').trim()

    let key = ''
    if (authorizationHeader.toLowerCase().startsWith('bearer ')) {
      key = authorizationHeader.slice(7).trim()
    }

    // If the caller sent a JWT-shaped bearer token AND a real SDK/API key header,
    // prefer the explicit SDK/API key header.
    // This prevents accidental 401s when test tooling includes a user JWT for unrelated endpoints.
    const looksLikeJwt = (s: string) => s.startsWith('eyJ') && s.split('.').length === 3
    if (key && looksLikeJwt(key) && (apiKeyHeader || sdkKeyHeader)) {
      key = ''
    }

    if (!key) {
      key = apiKeyHeader
    }
    // Backwards-compat header used by some SDK/test tooling
    if (!key) {
      key = sdkKeyHeader
    }

    if (!key) {
      void appKeysRepo.recordUsage(null, req as any, '401')
      return res.status(401).json({ error: 'Missing API key' })
    }

    const digest = sha256Hex(key)
    const found = await appKeysRepo.findActiveByDigest(digest)
    if (!found || found.revoked_at) {
      void appKeysRepo.recordUsage(found?.id ?? null, req as any, '401')
      return res.status(401).json({ error: 'Invalid API key' })
    }

    const ok = await bcrypt.compare(key, found.secret)
    if (!ok) {
      void appKeysRepo.recordUsage(found.id, req as any, '401')
      return res.status(401).json({ error: 'Invalid API key' })
    }

    void appKeysRepo.touchLastUsed(found.id)
    void appKeysRepo.recordUsage(found.id, req as any, '200')

    ;(req as any).sdk = {
      publisherId: found.publisher_id,
      appId: found.app_id,
      environment: found.environment,
      keyId: found.id,
    }

    return next()
  } catch (e) {
    return next(e)
  }
}

export default appApiKeyAuth
