import { z } from 'zod';
import dotenv from 'dotenv';
import path from 'path';

// Load .env before validation - use absolute path from compiled location
dotenv.config({ path: path.join(__dirname, '../../../../.env') });

/**
 * Environment variable schema with comprehensive validation
 * Fails fast at startup with helpful error messages
 */
const envSchema = z.object({
  // Server Configuration
  NODE_ENV: z.enum(['development', 'staging', 'production', 'test']).default('development'),
  PORT: z.string().regex(/^\d+$/).transform(Number).default('4000'),
  API_VERSION: z.string().default('v1'),
  
  // Database - PostgreSQL
  DATABASE_URL: z.string().url('DATABASE_URL must be a valid PostgreSQL connection string'),
  DB_POOL_MIN: z.string().regex(/^\d+$/).transform(Number).default('2'),
  DB_POOL_MAX: z.string().regex(/^\d+$/).transform(Number).default('10'),
  
  // Redis
  REDIS_HOST: z.string().default('localhost'),
  REDIS_PORT: z.string().regex(/^\d+$/).transform(Number).default('6379'),
  REDIS_PASSWORD: z.string().optional(),
  REDIS_DB: z.string().regex(/^\d+$/).transform(Number).default('0'),
  
  // Security
  JWT_SECRET: z.string().min(32, 'JWT_SECRET must be at least 32 characters for security'),
  COOKIE_SECRET: z.string().min(32, 'COOKIE_SECRET must be at least 32 characters'),
  CSRF_SECRET: z.string().min(32, 'CSRF_SECRET must be at least 32 characters').optional(),
  
  // CORS Configuration
  CORS_ORIGIN: z.string().default('http://localhost:3000'),
  CORS_ALLOWLIST: z.string().optional(),
  
  // Rate Limiting
  RATE_LIMIT_WINDOW_MS: z.string().regex(/^\d+$/).transform(Number).default('900000'),
  RATE_LIMIT_MAX_REQUESTS: z.string().regex(/^\d+$/).transform(Number).default('100'),
  
  // External Services - Payment
  STRIPE_SECRET_KEY: z.string().startsWith('sk_', 'STRIPE_SECRET_KEY must start with sk_').optional(),
  STRIPE_WEBHOOK_SECRET: z.string().startsWith('whsec_', 'STRIPE_WEBHOOK_SECRET must start with whsec_').optional(),

  // Banking & Settlement
  COMPANY_NAME: z.string().default('Bel Consulting OÜ'),
  SEPA_BANK_NAME: z.string().default('Wise'),
  SEPA_BANK_IBAN: z.string().default('BE41967430490410'),
  SEPA_BANK_BIC: z.string().default('TRWIBEB1XXX'),
  SEPA_BANK_ADDRESS: z.string().default('Wise, Rue du Trône 100, 3rd Floor, 1050 Brussels, Belgium'),
  ACH_BANK_NAME: z.string().default('Community Federal Savings Bank'),
  ACH_ACCOUNT_NUMBER: z.string().default('8312408211'),
  ACH_ROUTING_NUMBER: z.string().default('026073150'),
  ACH_ACCOUNT_TYPE: z.enum(['CHECKING', 'SAVINGS']).default('CHECKING'),
  ACH_SWIFT: z.string().default('CMFGUS33'),
  ACH_BANK_ADDRESS: z.string().default('Community Federal Savings Bank, 89-16 Jamaica Ave, Woodhaven, NY 11421, USA'),
  WISE_ACCOUNT_REFERENCE_PREFIX: z.string().default('APEXM'),
  SEB_BANK_NAME: z.string().optional(),
  SEB_BANK_IBAN: z.string().optional(),
  SEB_BANK_BIC: z.string().optional(),
  SEB_BANK_ADDRESS: z.string().optional(),
  
  // External Services - Email
  RESEND_API_KEY: z.string().startsWith('re_', 'RESEND_API_KEY must start with re_').optional(),
  
  // External Services - AI
  OPENAI_API_KEY: z.string().startsWith('sk-', 'OPENAI_API_KEY must start with sk-').optional(),
  
  // Feature Flags
  TRANSPARENCY_ENABLED: z.string().transform(v => v === 'true' || v === '1').default('false'),
  BILLING_ENABLED: z.string().transform(v => v === 'true' || v === '1').default('false'),
  FRAUD_DETECTION_ENABLED: z.string().transform(v => v === 'true' || v === '1').default('true'),
  AB_TESTING_ENABLED: z.string().transform(v => v === 'true' || v === '1').default('false'),
  FEATURE_KILL_SWITCH: z.string().transform(v => v === 'true' || v === '1').default('false'),
  FEATURE_ENFORCE_2FA: z.string().transform(v => v === 'true' || v === '1').default('false'),
  FEATURE_REQUIRE_EMAIL_VERIFICATION: z.string().transform(v => v === 'true' || v === '1').default('false'),
  FEATURE_DISABLE_NEW_ADAPTERS: z.string().transform(v => v === 'true' || v === '1').default('false'),
  
  // Monitoring
  PROMETHEUS_ENABLED: z.string().transform(v => v === 'true' || v === '1').default('true'),
  LOG_LEVEL: z.enum(['error', 'warn', 'info', 'http', 'verbose', 'debug', 'silly']).default('info'),

  // Analytics rollups (storage optimization)
  ANALYTICS_ROLLUP_ENABLED: z.string().transform(v => v === 'true' || v === '1').default('true'),
  ANALYTICS_ROLLUP_CRON: z.string().default('15 */6 * * *'), // every 6 hours at minute 15 (UTC)
  // Default covers the common 30-day dashboard window (plus a small buffer).
  ANALYTICS_ROLLUP_LOOKBACK_DAYS: z.string().regex(/^\d+$/).transform(Number).default('35'),

  // Migration guardrail scheduler (snapshot aggregation + automated evaluation)
  MIGRATION_GUARDRAIL_ENABLED: z.string().transform(v => v === 'true' || v === '1').default('true'),
  MIGRATION_GUARDRAIL_SNAPSHOT_CRON: z.string().default('*/15 * * * *'), // every 15 minutes
  MIGRATION_GUARDRAIL_EVALUATION_CRON: z.string().default('5,35 * * * *'), // at 5 and 35 past hour
  
  // Security Headers
  ENABLE_CSP: z.string().transform(v => v === '1').default('0'),
  TRUST_PROXY: z.string().transform(v => v === 'true' || v === '1').default('false'),
  
  // AWS Configuration (optional)
  AWS_REGION: z.string().optional(),
  AWS_ACCESS_KEY_ID: z.string().optional(),
  AWS_SECRET_ACCESS_KEY: z.string().optional(),
  AWS_S3_BUCKET: z.string().optional(),
  
  // GCP Configuration (optional)
  GCP_PROJECT_ID: z.string().optional(),
  GCP_BUCKET: z.string().optional(),
});

export type Env = z.infer<typeof envSchema>;

/**
 * Validate and parse environment variables
 * @throws {Error} If validation fails with detailed error messages
 */
export function validateEnv(): Env {
  const result = envSchema.safeParse(process.env);
  
  if (!result.success) {
    console.error('❌ Environment validation failed:');
    console.error('');
    
    const errors = result.error.format();
    Object.entries(errors).forEach(([key, value]) => {
      if (key !== '_errors' && value && typeof value === 'object' && '_errors' in value) {
        const errorMessages = (value as { _errors: string[] })._errors;
        if (errorMessages.length > 0) {
          console.error(`  ${key}: ${errorMessages.join(', ')}`);
        }
      }
    });
    
    console.error('');
    console.error('💡 Tips:');
    console.error('  1. Copy .env.sample to .env and fill in required values');
    console.error('  2. Ensure all required secrets are set (JWT_SECRET, COOKIE_SECRET, etc.)');
    console.error('  3. Check that URLs are properly formatted (DATABASE_URL, etc.)');
    console.error('');
    
    throw new Error('Environment validation failed. See errors above.');
  }
  
  return result.data;
}

/**
 * Validated environment variables
 * Use this instead of process.env for type safety
 */
export const env = validateEnv();
