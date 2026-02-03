import { ensureEnvLoaded } from '../utils/envLoader';

// Use centralized env loading (idempotent - safe to call multiple times)
ensureEnvLoaded();

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
