import dotenv from 'dotenv';

import fs from 'node:fs';
import path from 'node:path';

// Many utility modules import config early (e.g. logger). Avoid loading the wrong
// .env when running from /opt/adproject/backend by preferring repo-root ../.env.
// Never override an already-provided DATABASE_URL (e.g. PM2 env).
if (!process.env.DATABASE_URL) {
  const cwd = process.cwd();
  const cwdBase = path.basename(cwd);
  const candidates =
    cwdBase === 'backend'
      ? [path.resolve(cwd, '../.env'), path.resolve(cwd, '.env')]
      : [path.resolve(cwd, '.env'), path.resolve(cwd, '../.env')];

  const envPath = [process.env.ENV_FILE, ...candidates].find((p) => !!p && fs.existsSync(p));
  if (envPath) {
    dotenv.config({ path: envPath });
  } else {
    dotenv.config();
  }
}

const config = {
  isDevelopment: process.env.NODE_ENV === 'development',
  databaseUrl:
    process.env.DATABASE_URL || 'postgresql://postgres:postgres@postgres:5432/apexmediation',
  // Batch-2 feature flags and knobs
  replicaRequired: process.env.REPLICA_REQUIRED === '1',
  useRedisStreamsForAnalytics: process.env.USE_REDIS_STREAMS_FOR_ANALYTICS === '1',
  useKafkaForAnalytics: process.env.USE_KAFKA_FOR_ANALYTICS === '1',
  adapterRegistryRefreshSec: parseInt(process.env.ADAPTER_REGISTRY_REFRESH_SEC || '60', 10),
  redisBreakersEnabled: process.env.REDIS_BREAKERS_ENABLED === '1',
  logToFiles: process.env.LOG_TO_FILES === '1',
  logRedactionSalt: process.env.OBS_LOG_SALT || '',
  readinessThresholds: {
    dbSlowMs: parseInt(process.env.READINESS_DB_SLOW_MS || '1000', 10),
    replicaLagWarnMs: parseInt(process.env.READINESS_REPLICA_LAG_WARN_MS || '5000', 10),
    replicaLagCriticalMs: parseInt(process.env.READINESS_REPLICA_LAG_CRITICAL_MS || '15000', 10),
    stagingWarnRows: parseInt(process.env.READINESS_STAGING_WARN_ROWS || '250000', 10),
    cacheHitWarnRatio: parseFloat(process.env.READINESS_CACHE_HIT_WARN_RATIO || '0.9'),
  },
};

export default config;
