import dotenv from 'dotenv';
import path from 'path';

// Load environment variables
dotenv.config({ path: path.resolve(__dirname, '../.env') });

export interface Config {
  database: {
    url: string;
    poolSize: number;
  };
  api: {
    host: string;
    port: number;
    allowedIps: string[];
  };
  scheduler: {
    healthCheckIntervalSeconds: number;
    partitionCheckIntervalHours: number;
    retentionCleanupHour: number;
    aggregationRunMinutes: number;
  };
  alerts: {
    webhookUrl: string | null;
    slackChannel: string | null;
  };
  logging: {
    level: string;
    format: string;
  };
}

export const config: Config = {
  database: {
    url: process.env.DATABASE_URL || 'postgresql://postgres:postgres@localhost:5432/adproject',
    poolSize: parseInt(process.env.DATABASE_POOL_SIZE || '5', 10),
  },
  api: {
    host: process.env.API_HOST || '0.0.0.0',
    port: parseInt(process.env.API_PORT || '8090', 10),
    allowedIps: (process.env.ALLOWED_IPS || '127.0.0.1').split(',').map(ip => ip.trim()),
  },
  scheduler: {
    healthCheckIntervalSeconds: parseInt(process.env.HEALTH_CHECK_INTERVAL_SECONDS || '60', 10),
    partitionCheckIntervalHours: parseInt(process.env.PARTITION_CHECK_INTERVAL_HOURS || '6', 10),
    retentionCleanupHour: parseInt(process.env.RETENTION_CLEANUP_HOUR || '3', 10),
    aggregationRunMinutes: parseInt(process.env.AGGREGATION_RUN_MINUTES || '5', 10),
  },
  alerts: {
    webhookUrl: process.env.ALERT_WEBHOOK_URL || null,
    slackChannel: process.env.ALERT_SLACK_CHANNEL || null,
  },
  logging: {
    level: process.env.LOG_LEVEL || 'info',
    format: process.env.LOG_FORMAT || 'json',
  },
};
