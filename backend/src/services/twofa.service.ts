import crypto from 'node:crypto';
import { authenticator } from 'otplib';
import bcrypt from 'bcryptjs';
import * as qrcode from 'qrcode';
import { Repository } from 'typeorm';
import logger from '../utils/logger';
import { AppDataSource } from '../database';
import { TwoFactorAuth } from '../database/entities/twoFactorAuth.entity';
import { User } from '../database/entities/user.entity';
import { AuditTwofa } from '../database/entities/auditTwofa.entity';
import { aesGcmEncrypt, aesGcmDecrypt, md5_16 } from '../utils/crypto';

// Security: Require encryption in production
const IS_PRODUCTION = process.env.NODE_ENV === 'production';
const ALLOW_PLAINTEXT_2FA = process.env.ALLOW_PLAINTEXT_2FA === 'true'; // Explicit opt-in for dev/test

type TwofaRepositories = {
  tfaRepository: Repository<TwoFactorAuth>;
  userRepository: Repository<User>;
  auditRepository: Repository<AuditTwofa>;
};

let repoWarningEmitted = false;

const getRepositories = (): TwofaRepositories | null => {
  if (!AppDataSource.isInitialized) {
    if (!repoWarningEmitted) {
      logger.warn('Two-factor auth store is not initialized; falling back to safe defaults.');
      repoWarningEmitted = true;
    }
    return null;
  }

  repoWarningEmitted = false;

  return {
    tfaRepository: AppDataSource.getRepository(TwoFactorAuth),
    userRepository: AppDataSource.getRepository(User),
    auditRepository: AppDataSource.getRepository(AuditTwofa),
  };
};

const requireRepositories = (): TwofaRepositories => {
  const repositories = getRepositories();
  if (!repositories) {
    throw new Error('Two-factor auth data source is unavailable');
  }
  return repositories;
};

type TwofaRow = {
  enabled: boolean;
  secret: string | null;
  secretCiphertext: string | null;
  backupCodes: string | null;
};

type TwofaSchemaStyle = 'camel' | 'snake';
let cachedTwofaSchemaStyle: TwofaSchemaStyle | null = null;

const detectTwofaSchemaStyle = async (): Promise<TwofaSchemaStyle> => {
  if (cachedTwofaSchemaStyle) return cachedTwofaSchemaStyle;
  if (!AppDataSource.isInitialized) return 'camel';

  try {
    const rows = (await AppDataSource.query(
      `SELECT 1
       FROM information_schema.columns
       WHERE table_schema = 'public'
         AND table_name = 'two_factor_auth'
         AND column_name = 'userId'
       LIMIT 1`
    )) as unknown[];
    cachedTwofaSchemaStyle = rows?.length ? 'camel' : 'snake';
  } catch {
    cachedTwofaSchemaStyle = 'camel';
  }

  return cachedTwofaSchemaStyle;
};

const getTwofaRow = async (userId: string): Promise<TwofaRow | null> => {
  if (!AppDataSource.isInitialized) return null;

  const style = await detectTwofaSchemaStyle();

  const sql = style === 'camel'
    ? `SELECT enabled,
             secret,
             "secretCiphertext" AS "secretCiphertext",
             "backupCodes" AS "backupCodes"
       FROM two_factor_auth
       WHERE "userId" = $1
       LIMIT 1`
    : `SELECT enabled,
             secret,
             secret_ciphertext AS "secretCiphertext",
             backup_codes AS "backupCodes"
       FROM two_factor_auth
       WHERE user_id = $1
       LIMIT 1`;

  // Use raw SQL here to avoid any runtime entity/metadata mismatch issues.
  const rows = (await AppDataSource.query(sql, [userId])) as Array<TwofaRow>;

  return rows?.[0] ?? null;
};

const parseBackupCodes = (raw: string | null): string[] => {
  if (!raw) return [];
  // TypeORM simple-array stores as comma-separated string.
  return raw
    .split(',')
    .map((c) => c.trim())
    .filter(Boolean);
};

const maskSecret = (secret: string) => secret.replace(/.(?=.{4})/g, '*');

/**
 * Securely retrieve the 2FA secret from a record
 * 
 * SECURITY: This function enforces encryption in production.
 * - If ciphertext exists: decrypt it (throws on failure)
 * - If plaintext exists: only allowed in dev/test with explicit opt-in
 * - Otherwise: throws an error
 */
function getSecretFromRecord(secretCiphertext: string | null, secret: string | null): string {
  if (secretCiphertext) {
    // Encrypted secret - decrypt it
    try {
      return aesGcmDecrypt(JSON.parse(secretCiphertext));
    } catch (e) {
      // FAIL-CLOSED: Don't fall back to plaintext on decryption failure
      // This prevents attacks that corrupt ciphertext to force plaintext fallback
      logger.error('2FA secret decryption failed - possible tampering or key rotation', {
        error: (e as Error).message,
      });
      throw new Error('2FA secret unavailable - re-enrollment may be required');
    }
  }
  
  // No ciphertext - check for legacy plaintext
  if (secret) {
    if (IS_PRODUCTION && !ALLOW_PLAINTEXT_2FA) {
      // SECURITY: Reject plaintext secrets in production
      logger.error('SECURITY: Plaintext 2FA secret found in production - this is a security violation');
      throw new Error('2FA configuration error - contact support');
    }
    logger.warn('Using legacy plaintext 2FA secret - migrate to encrypted storage immediately');
    return secret;
  }
  
  throw new Error('2FA secret not found');
}

type AuditCtx = { actorEmail?: string | null; ip?: string | null };

async function writeAudit(userId: string, action: 'enroll' | 'enable' | 'regen' | 'disable', ctx?: AuditCtx) {
  try {
    const repositories = getRepositories();
    if (!repositories) return;

    const row = repositories.auditRepository.create({
      user: { id: userId } as any,
      action,
      actorEmail: ctx?.actorEmail ?? null,
      ipHash: ctx?.ip ? md5_16(ctx.ip) : null,
    });
    await repositories.auditRepository.save(row);
  } catch (e) {
    logger.warn('Failed to write 2FA audit row', { userId, action, error: (e as Error).message });
  }
}

export const twofaService = {
  async isEnabled(userId: string): Promise<boolean> {
    const row = await getTwofaRow(userId);
    return row?.enabled === true;
  },

  async getBackupCodesCount(userId: string): Promise<number> {
    const row = await getTwofaRow(userId);
    return parseBackupCodes(row?.backupCodes ?? null).length;
  },

  async enroll(userId: string, email: string, issuer = 'ApexMediation', auditCtx?: AuditCtx) {
    const { userRepository, tfaRepository } = requireRepositories();
    const user = await userRepository.findOneBy({ id: userId });
    if (!user) throw new Error('User not found');

    let tfaRecord = await tfaRepository.findOne({ where: { user: { id: userId } } });
    if (!tfaRecord) {
      const secret = authenticator.generateSecret();
      try {
        const ct = aesGcmEncrypt(secret);
        tfaRecord = tfaRepository.create({ user, secret: null, secretCiphertext: JSON.stringify(ct), enabled: false, backupCodes: [] });
      } catch (e) {
        // SECURITY: In production, encryption is required - fail hard
        if (IS_PRODUCTION && !ALLOW_PLAINTEXT_2FA) {
          logger.error('SECURITY: 2FA encryption failed in production - APP_KMS_KEY must be configured', {
            error: (e as Error).message,
          });
          throw new Error('2FA enrollment failed - encryption unavailable');
        }
        // Only allow plaintext in dev/test with explicit opt-in
        logger.warn('APP_KMS_KEY not set; storing 2FA secret in legacy plaintext column (dev only).');
        tfaRecord = tfaRepository.create({ user, secret, secretCiphertext: null, enabled: false, backupCodes: [] });
      }
    } else if (!tfaRecord.secretCiphertext && tfaRecord.secret) {
      // Backfill: if legacy plaintext secret exists, attempt to encrypt it now
      try {
        const ct = aesGcmEncrypt(tfaRecord.secret);
        tfaRecord.secret = null;
        tfaRecord.secretCiphertext = JSON.stringify(ct);
        logger.info('2FA secret migrated from plaintext to encrypted storage', { userId });
      } catch (e) {
        // SECURITY: In production, fail the backfill if encryption unavailable
        if (IS_PRODUCTION && !ALLOW_PLAINTEXT_2FA) {
          logger.error('SECURITY: 2FA migration failed - cannot encrypt existing secret', {
            userId,
            error: (e as Error).message,
          });
          throw new Error('2FA enrollment failed - migration required but encryption unavailable');
        }
        // In dev/test, continue with legacy secret but log warning
        logger.warn('2FA encryption unavailable; continuing with legacy plaintext secret for this record.');
      }
    }
    
    await tfaRepository.save(tfaRecord);

    // Use secure secret retrieval
    const secret = getSecretFromRecord(tfaRecord.secretCiphertext, tfaRecord.secret);

    const otpauthUrl = authenticator.keyuri(email, issuer, secret);
    const qrDataUrl = await qrcode.toDataURL(otpauthUrl);
    logger.info('2FA enroll started', { userId, issuer });
    await writeAudit(userId, 'enroll', auditCtx);
    return { otpauthUrl, qrDataUrl, maskedSecret: maskSecret(secret) };
  },

  async verifyAndEnable(userId: string, token: string, auditCtx?: AuditCtx): Promise<{ backupCodes: string[] }> {
    const { tfaRepository } = requireRepositories();
    const tfaRecord = await tfaRepository.findOne({ where: { user: { id: userId } }, relations: ['user'] });
    if (!tfaRecord) throw new Error('No enrollment in progress');

    // Use secure secret retrieval - no plaintext fallback
    const secret = getSecretFromRecord(tfaRecord.secretCiphertext, tfaRecord.secret);
    
    const ok = authenticator.verify({ token, secret });
    if (!ok) throw new Error('Invalid token');

    const codes = Array.from({ length: 10 }, () =>
      crypto.randomBytes(8).toString('base64url').slice(0, 10)
    );
    
    const hashedCodes = await Promise.all(codes.map((c) => bcrypt.hash(c, 10)));

    tfaRecord.enabled = true;
    tfaRecord.backupCodes = hashedCodes;
    await tfaRepository.save(tfaRecord);

    logger.info('2FA enabled', { userId });
    await writeAudit(userId, 'enable', auditCtx);
    return { backupCodes: codes };
  },

  async verifyTokenOrBackupCode(userId: string, code: string): Promise<boolean> {
    const repositories = getRepositories();
    if (!repositories) return false;

    const row = await getTwofaRow(userId);
    if (!row) return false;

    // Use secure secret retrieval - no plaintext fallback on decryption failure
    let secret: string;
    try {
      secret = getSecretFromRecord(row.secretCiphertext, row.secret);
    } catch (e) {
      // If we can't retrieve the secret securely, fail the verification
      logger.error('2FA verification failed - cannot retrieve secret securely', { 
        userId, 
        error: (e as Error).message,
      });
      return false;
    }
    
    if (authenticator.check(code, secret)) return true;

    // Backup codes require a write on success; use the ORM entity for updates.
    const tfaRecord = await repositories.tfaRepository.findOne({ where: { user: { id: userId } } });
    if (!tfaRecord) return false;

    for (let i = 0; i < tfaRecord.backupCodes.length; i++) {
      const hashedCode = tfaRecord.backupCodes[i];
      if (await bcrypt.compare(code, hashedCode)) {
        tfaRecord.backupCodes.splice(i, 1);
        await repositories.tfaRepository.save(tfaRecord);
        logger.info('2FA backup code used', { userId });
        return true;
      }
    }
    return false;
  },

  async regenerateBackupCodes(userId: string, auditCtx?: AuditCtx): Promise<{ backupCodes: string[] }> {
    const { tfaRepository } = requireRepositories();
    const tfaRecord = await tfaRepository.findOne({ where: { user: { id: userId } } });
    if (!tfaRecord || !tfaRecord.enabled) throw new Error('2FA not enabled');

    const codes = Array.from({ length: 10 }, () =>
      crypto.randomBytes(8).toString('base64url').slice(0, 10)
    );
    
    tfaRecord.backupCodes = await Promise.all(codes.map((c) => bcrypt.hash(c, 10)));
    await tfaRepository.save(tfaRecord);

    logger.info('2FA backup codes regenerated', { userId });
    await writeAudit(userId, 'regen', auditCtx);
    return { backupCodes: codes };
  },

  async disable(userId: string, auditCtx?: AuditCtx) {
    const { tfaRepository } = requireRepositories();
    const tfaRecord = await tfaRepository.findOne({ where: { user: { id: userId } } });
    if (!tfaRecord) return;

    tfaRecord.enabled = false;
    tfaRecord.backupCodes = [];
    await tfaRepository.save(tfaRecord);
    
    logger.info('2FA disabled', { userId });
    await writeAudit(userId, 'disable', auditCtx);
  },
};

export default twofaService;
