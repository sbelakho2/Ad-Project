import { NextFunction, Request, Response } from 'express';
import bcrypt from 'bcryptjs';
import { AppDataSource } from '../database';
import { ApiKey } from '../database/entities/apiKey.entity';
import { ApiKeyUsage } from '../database/entities/apiKeyUsage.entity';
import { md5_16, sha256Hex } from '../utils/crypto';

/**
 * API Key authentication middleware
 *
 * Looks for API key in `Authorization: Bearer <key>` or `X-Api-Key` header.
 * Matches by sha256 digest, then verifies bcrypt hash.
 * On success, attaches minimal user context to req.user and records usage asynchronously.
 */
export async function apiKeyAuth(req: Request, res: Response, next: NextFunction) {
  try {
    const header = (req.header('authorization') || '').trim();
    let key = '';
    if (header.toLowerCase().startsWith('bearer ')) {
      key = header.slice(7).trim();
    }
    if (!key) {
      key = (req.header('x-api-key') || req.header('X-Api-Key') || '').trim();
    }

    if (!key) {
      return res.status(401).json({ error: 'Missing API key' });
    }

    const digest = sha256Hex(key);
    const apiKeyRepo = AppDataSource.getRepository(ApiKey);
    const usageRepo = AppDataSource.getRepository(ApiKeyUsage);
    const found = await apiKeyRepo.findOne({ where: { secretDigest: digest }, relations: ['user'] });
    if (!found || found.revokedAt) {
      await safeRecordUsage(usageRepo, undefined, req, '401');
      return res.status(401).json({ error: 'Invalid API key' });
    }

    const ok = await bcrypt.compare(key, found.secret);
    if (!ok) {
      await safeRecordUsage(usageRepo, found.id, req, '401');
      return res.status(401).json({ error: 'Invalid API key' });
    }

    // Update last used asynchronously
    void apiKeyRepo.update({ id: found.id }, { lastUsedAt: new Date() });

    // Attach minimal user context.
    // NOTE: `User` entity intentionally omits several production columns (publisher_id, role).
    // When loading `relations: ['user']`, those fields won't be present, so we fetch them explicitly.
    type ApiRole = 'admin' | 'publisher' | 'readonly';

    let publisherId = '';
    let role: ApiRole | undefined;
    let email = (found.user as any)?.email || '';
    try {
      const rows = await AppDataSource.query(
        'SELECT publisher_id AS "publisherId", role, email FROM users WHERE id = $1 LIMIT 1',
        [found.user.id]
      );
      const row = rows?.[0];
      if (row) {
        publisherId = String(row.publisherId || '');
        const maybeRole = String(row.role || '').toLowerCase();
        role = (maybeRole === 'admin' || maybeRole === 'publisher' || maybeRole === 'readonly') ? (maybeRole as ApiRole) : undefined;
        email = String(row.email || email);
      }
    } catch {
      // Best-effort; leave publisherId empty if unavailable.
    }

    req.user = {
      userId: found.user.id,
      publisherId,
      email,
      role,
    };

    // Record usage asynchronously
    void safeRecordUsage(usageRepo, found.id, req, '200');

    return next();
  } catch (e) {
    return next(e);
  }
}

async function safeRecordUsage(repo: ReturnType<typeof AppDataSource.getRepository<ApiKeyUsage>>, keyId: string | undefined, req: Request, status: string) {
  try {
    if (!repo) return;
    const ip = req.ip || '';
    const ua = req.get('user-agent') || '';
    const usage = repo.create({
      // apiKey will be set via id reference; TypeORM allows setting relation by id in save
      apiKey: keyId ? ({ id: keyId } as any) : (undefined as any),
      route: String(req.baseUrl || req.path || '' ).slice(0, 128),
      method: (req.method || '').slice(0, 64),
      ipHash: ip ? md5_16(ip) : null,
      uaHash: ua ? md5_16(ua) : null,
      status: status.slice(0, 16),
    });
    await repo.save(usage);
  } catch {
    // swallow
  }
}

export default apiKeyAuth;
