import 'reflect-metadata';
import { DataSource } from 'typeorm';
import { User } from './entities/user.entity';
import { TwoFactorAuth } from './entities/twoFactorAuth.entity';
import { ApiKey } from './entities/apiKey.entity';
import { ApiKeyUsage } from './entities/apiKeyUsage.entity';
import { SkanPostback } from './entities/skanPostback.entity';
import { AdapterConfig } from './entities/adapterConfig.entity';
import { AuditTwofa } from './entities/auditTwofa.entity';
import config from '../config/index';

export const AppDataSource = new DataSource({
  type: 'postgres',
  url: config.databaseUrl,
  entities: [User, TwoFactorAuth, ApiKey, ApiKeyUsage, SkanPostback, AdapterConfig, AuditTwofa],
  migrations: ['src/migrations/*.ts'],
  // Disable synchronize to avoid destructive schema diffs; use migrations instead
  synchronize: false,
  logging: config.isDevelopment,
});

export async function initializeDatabase() {
  const isProd = process.env.NODE_ENV === 'production';
  // In production we prefer to keep the process alive and keep retrying
  // rather than crash-looping under PM2.
  const maxAttempts = Number(process.env.TYPEORM_INIT_MAX_ATTEMPTS || (isProd ? 0 : 5));
  const baseDelayMs = Number(process.env.TYPEORM_INIT_BASE_DELAY_MS || 1000);
  const maxDelayMs = Number(process.env.TYPEORM_INIT_MAX_DELAY_MS || 30_000);
  const logEveryN = Number(process.env.TYPEORM_INIT_LOG_EVERY_N || 5);

  const shouldContinue = (attempt: number) => maxAttempts <= 0 || attempt <= maxAttempts;

  for (let attempt = 1; shouldContinue(attempt); attempt++) {
    try {
      await AppDataSource.initialize();
      console.log('Data Source has been initialized!');
      return;
    } catch (err) {
      const error = err as Error;
      const isVerbose = attempt === 1 || (logEveryN > 0 && attempt % logEveryN === 0);
      if (isVerbose) {
        console.error('Error during Data Source initialization:', err);
      } else {
        console.error(`Error during Data Source initialization: ${error?.message ?? String(err)}`);
      }

      if (maxAttempts > 0 && attempt >= maxAttempts) throw err;

      const expDelayMs = baseDelayMs * Math.pow(2, attempt - 1);
      const delayMs = Math.min(maxDelayMs, expDelayMs);
      const jitterMs = Math.floor(Math.random() * Math.min(250, delayMs));
      const sleepMs = delayMs + jitterMs;

      console.error(
        `Retrying Data Source initialization in ${sleepMs}ms (attempt ${attempt + 1}/${maxAttempts > 0 ? maxAttempts : '∞'})...`
      );
      await new Promise((r) => setTimeout(r, sleepMs));
    }
  }
}
