import { Request, Response, NextFunction } from 'express';
import { randomBytes } from 'crypto';
import bcrypt from 'bcryptjs';
import logger from '../utils/logger';
import { AppDataSource } from '../database';
import { ApiKey } from '../database/entities/apiKey.entity';
import { User } from '../database/entities/user.entity';
import { sha256Hex } from '../utils/crypto';

// Lazy-load repositories to avoid calling getRepository before AppDataSource is initialized
const getApiKeyRepository = () => AppDataSource.getRepository(ApiKey);
const getUserRepository = () => AppDataSource.getRepository(User);

const redact = (k: ApiKey) => ({
  id: k.id,
  prefix: k.prefix,
  last4: k.last4,
  createdAt: k.createdAt,
  lastUsedAt: k.lastUsedAt ?? null,
  revokedAt: k.revokedAt ?? null,
});

const genSecret = (prefix: 'sk_live' | 'sk_test') => `${prefix}_${randomBytes(24).toString('hex')}`;

export async function listKeys(req: Request, res: Response, _next: NextFunction) {
  const userId = req.user?.userId;
  if (!userId) return res.status(401).json({ error: 'Unauthorized' });

  const keys = await getApiKeyRepository().find({ where: { user: { id: userId } } });
  return res.json({ data: { keys: keys.map(redact) } });
}

export async function createKey(req: Request, res: Response, _next: NextFunction) {
  const userId = req.user?.userId;
  if (!userId) return res.status(401).json({ error: 'Unauthorized' });

  const user = await getUserRepository().findOneBy({ id: userId });
  if (!user) return res.status(404).json({ error: 'User not found' });

  const live = Boolean(req.body?.live);
  const prefix = live ? 'sk_live' : 'sk_test';
  const secret = genSecret(prefix as any);
  const secretHash = await bcrypt.hash(secret, 10);
  const secretDigest = sha256Hex(secret);

  const apiKeyRepo = getApiKeyRepository();
  const newKey = apiKeyRepo.create({
    user,
    secret: secretHash,
    secretDigest,
    prefix,
    last4: secret.slice(-4),
  });

  await apiKeyRepo.save(newKey);

  logger.info('API key created', { userId, prefix, last4: newKey.last4, id: newKey.id });
  return res.status(201).json({ data: { id: newKey.id, secret, prefix: newKey.prefix, last4: newKey.last4 } });
}

export async function rotateKey(req: Request, res: Response, _next: NextFunction) {
  const userId = req.user?.userId;
  if (!userId) return res.status(401).json({ error: 'Unauthorized' });

  const apiKeyRepo = getApiKeyRepository();
  const { id } = req.params;
  const key = await apiKeyRepo.findOne({ where: { id, user: { id: userId } } });
  if (!key) return res.status(404).json({ error: 'Not found' });

  key.revokedAt = new Date();
  await apiKeyRepo.save(key);

  const live = key.prefix === 'sk_live';
  const prefix = live ? 'sk_live' : 'sk_test';
  const secret = genSecret(prefix as any);
  const secretHash = await bcrypt.hash(secret, 10);
  const secretDigest = sha256Hex(secret);

  const newKey = apiKeyRepo.create({
    user: key.user,
    secret: secretHash,
    secretDigest,
    prefix,
    last4: secret.slice(-4),
  });

  await apiKeyRepo.save(newKey);
  
  logger.info('API key rotated', { userId, idOld: key.id, idNew: newKey.id, prefix: newKey.prefix, last4: newKey.last4 });
  return res.json({ data: { id: newKey.id, secret, prefix: newKey.prefix, last4: newKey.last4 } });
}

export async function deleteKey(req: Request, res: Response, _next: NextFunction) {
  const userId = req.user?.userId;
  if (!userId) return res.status(401).json({ error: 'Unauthorized' });
  
  const apiKeyRepo = getApiKeyRepository();
  const { id } = req.params;
  const key = await apiKeyRepo.findOne({ where: { id, user: { id: userId } } });
  if (!key) return res.status(404).json({ error: 'Not found' });

  key.revokedAt = new Date();
  await apiKeyRepo.save(key);

  logger.info('API key deleted', { userId, id });
  return res.status(204).send();
}

