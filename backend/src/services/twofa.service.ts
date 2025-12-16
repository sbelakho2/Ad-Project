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

const maskSecret = (secret: string) => secret.replace(/.(?=.{4})/g, '*');

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
    const repositories = getRepositories();
    if (!repositories) return false;

    const tfaRecord = await repositories.tfaRepository.findOne({ where: { user: { id: userId } } });
    return tfaRecord?.enabled === true;
  },

  async getBackupCodesCount(userId: string): Promise<number> {
    const repositories = getRepositories();
    if (!repositories) return 0;

    const tfaRecord = await repositories.tfaRepository.findOne({ where: { user: { id: userId } } });
    return tfaRecord?.backupCodes?.length ?? 0;
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
        // Fallback to legacy plaintext if APP_KMS_KEY is not configured
        logger.warn('APP_KMS_KEY not set; storing 2FA secret in legacy plaintext column (dev only).');
        tfaRecord = tfaRepository.create({ user, secret, secretCiphertext: null, enabled: false, backupCodes: [] });
      }
    } else if (!tfaRecord.secretCiphertext && tfaRecord.secret) {
      // Backfill: if legacy plaintext secret exists, attempt to encrypt it now
      try {
        const ct = aesGcmEncrypt(tfaRecord.secret);
        tfaRecord.secret = null;
        tfaRecord.secretCiphertext = JSON.stringify(ct);
      } catch (e) {
        // Keep legacy secret if encryption unavailable
        logger.warn('2FA encryption unavailable; continuing with legacy plaintext secret for this record.');
      }
    }
    
    await tfaRepository.save(tfaRecord);

    let secret: string;
    try {
      secret = tfaRecord.secretCiphertext
        ? aesGcmDecrypt(JSON.parse(tfaRecord.secretCiphertext))
        : (tfaRecord.secret as string);
    } catch (e) {
      // Decryption failed (likely missing key); fall back to legacy secret if present
      secret = (tfaRecord.secret as string);
    }

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

    let secret: string;
    try {
      secret = tfaRecord.secretCiphertext
        ? aesGcmDecrypt(JSON.parse(tfaRecord.secretCiphertext))
        : (tfaRecord.secret as string);
    } catch (e) {
      secret = (tfaRecord.secret as string);
    }
    const ok = authenticator.verify({ token, secret });
    if (!ok) throw new Error('Invalid token');

    const codes = Array.from({ length: 10 }, () =>
      Math.random().toString(36).replace(/[^a-z0-9]/gi, '').slice(-10)
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

    const tfaRecord = await repositories.tfaRepository.findOne({ where: { user: { id: userId } } });
    if (!tfaRecord) return false;

    let secret: string;
    try {
      secret = tfaRecord.secretCiphertext
        ? aesGcmDecrypt(JSON.parse(tfaRecord.secretCiphertext))
        : (tfaRecord.secret as string);
    } catch (e) {
      secret = (tfaRecord.secret as string);
    }
    if (authenticator.check(code, secret)) return true;

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
      Math.random().toString(36).replace(/[^a-z0-9]/gi, '').slice(-10)
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
