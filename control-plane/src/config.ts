/**
 * CONTROL PLANE CONFIGURATION
 * 
 * CRITICAL: This is the operator control plane - completely separate from customer API.
 * No customer tokens, no shared secrets, no customer routes.
 */

import { z } from 'zod';
import fs from 'node:fs';

const envSchema = z.object({
  PORT: z.string().default('4000'),
  NODE_ENV: z.enum(['development', 'production', 'test']).default('production'),
  
  // Operator Auth (SEPARATE REALM)
  OPERATOR_JWT_SECRET: z.string().min(32),
  OPERATOR_SESSION_TTL_HOURS: z.string().default('4'),
  OPERATOR_CREDENTIALS: z.string().optional(), // email:hash pairs
  
  // IP Allowlist (CIDR supported, comma-separated)
  // Example: "10.0.0.0/8,192.168.1.100,203.0.113.50"
  OPERATOR_IP_ALLOWLIST: z.string().optional(),

  // Device approval (2FA for new devices)
  DEVICE_APPROVAL_ENABLED: z.string().default('true'),
  DEVICE_APPROVAL_EMAIL_TO: z.string().default('contact@apexmediation.ee'),
  PUBLIC_ORIGIN: z.string().default('https://admin.apexmediation.ee'),
  DEVICE_CHALLENGE_TTL_MINUTES: z.string().default('30'),
  DEVICE_CHALLENGE_SECRET: z.string().optional(),

  // ApexMail - Self-hosted email service (replaces Resend)
  SMTP_HOST: z.string().optional(),
  SMTP_PORT: z.string().default('587'),
  SMTP_SECURE: z.string().default('false'),
  SMTP_USER: z.string().optional(),
  SMTP_PASS: z.string().optional(),
  EMAIL_FROM: z.string().default('contact@apexmediation.ee'),
  EMAIL_FROM_NAME: z.string().default('ApexMediation'),
  EMAIL_REPLY_TO: z.string().optional(),
  APEXMAIL_API_KEY: z.string().optional(), // Internal API key for cross-service calls
  
  // Database - Read-Only (monitoring)
  DB_HOST: z.string(),
  DB_PORT: z.string().default('5432'),
  DB_NAME: z.string(),
  DB_USER: z.string(),
  DB_PASSWORD: z.string(),
  DB_SSL: z.string().default('true'),
  DB_SSL_CA: z.string().optional(),
  DB_SSL_REJECT_UNAUTHORIZED: z.string().optional(),
  
  // Database - Operator (safe actions)
  DB_OPERATOR_USER: z.string().optional(),
  DB_OPERATOR_PASSWORD: z.string().optional(),
  
  // Database - Break-Glass (emergency)
  DB_BREAKGLASS_USER: z.string().optional(),
  DB_BREAKGLASS_PASSWORD: z.string().optional(),
  
  // Host Telemetry
  HOST_TELEMETRY_ENDPOINT: z.string().optional(),
  HOST_TELEMETRY_ENDPOINT_APP: z.string().optional(),
  HOST_TELEMETRY_ENDPOINT_DB: z.string().optional(),
  S3_STORAGE_ENDPOINT: z.string().optional(),
  S3_STORAGE_BUCKET: z.string().optional(),
  AWS_ACCESS_KEY_ID: z.string().optional(),
  AWS_SECRET_ACCESS_KEY: z.string().optional(),
  AWS_DEFAULT_REGION: z.string().optional(),
  S3_ENDPOINT: z.string().optional(),
  
  // Hetzner Cloud
  HETZNER_API_TOKEN: z.string().optional(),
  HETZNER_ENABLED: z.string().default('false'),
  
  // Audit
  AUDIT_RETENTION_YEARS: z.string().default('7'),
  
  // Compliance
  COMPLIANCE_ENABLED: z.string().default('false'),
  
  // CORS
  CORS_ORIGIN: z.string().optional(),
  
  // Backend metrics endpoint
  BACKEND_METRICS_URL: z.string().optional(),
  
  // Node exporter URL
  NODE_EXPORTER_URL: z.string().optional(),
});

export type EnvConfig = z.infer<typeof envSchema>;

let _config: EnvConfig | null = null;

// Export for use in index.ts (must be loaded first)
export let config: EnvConfig;

export function loadConfig(): EnvConfig {
  if (_config) return _config;
  
  const result = envSchema.safeParse(process.env);
  if (!result.success) {
    console.error('❌ Invalid control plane configuration:');
    console.error(result.error.format());
    process.exit(1);
  }
  
  _config = result.data;
  config = _config;
  return _config;
}

export function getConfig(): EnvConfig {
  if (!_config) {
    return loadConfig();
  }
  return _config;
}

// Parse operator credentials from environment
export function getOperatorCredentials(): Map<string, string> {
  const cfg = getConfig();
  const creds = new Map<string, string>();
  
  if (!cfg.OPERATOR_CREDENTIALS) return creds;

  const raw = cfg.OPERATOR_CREDENTIALS.trim();

  // Format A (preferred): email:hash,email2:hash2
  // Format B (legacy): {"email":"hash","email2":"hash2"}
  if (raw.startsWith('{')) {
    try {
      const parsed = JSON.parse(raw) as Record<string, unknown>;
      for (const [email, hash] of Object.entries(parsed)) {
        if (typeof email !== 'string') continue;
        if (typeof hash !== 'string') continue;
        const normalizedEmail = email.trim().toLowerCase();
        const normalizedHash = hash.trim();
        if (normalizedEmail && normalizedHash) {
          creds.set(normalizedEmail, normalizedHash);
        }
      }
      return creds;
    } catch {
      // Fall through to CSV parsing
    }
  }

  const pairs = raw.split(',');
  for (const pair of pairs) {
    const idx = pair.indexOf(':');
    if (idx === -1) continue;
    const email = pair.slice(0, idx).trim();
    const hash = pair.slice(idx + 1).trim();
    if (email && hash) {
      creds.set(email.toLowerCase(), hash);
    }
  }
  
  return creds;
}

// Database connection configs
function buildSslConfig(cfg: EnvConfig): false | { rejectUnauthorized: boolean; ca?: string } {
  if (cfg.DB_SSL !== 'true') return false;
  const caPath = process.env.DB_SSL_CA;
  const rejectUnauthorizedEnv = process.env.DB_SSL_REJECT_UNAUTHORIZED;
  const rejectUnauthorized = rejectUnauthorizedEnv
    ? rejectUnauthorizedEnv === 'true'
    : Boolean(caPath);

  const sslConfig: { rejectUnauthorized: boolean; ca?: string } = { rejectUnauthorized };
  if (caPath) {
    sslConfig.ca = fs.readFileSync(caPath, 'utf8');
  }
  return sslConfig;
}

export function getReadOnlyDbConfig() {
  const cfg = getConfig();
  return {
    host: cfg.DB_HOST,
    port: parseInt(cfg.DB_PORT, 10),
    database: cfg.DB_NAME,
    user: cfg.DB_USER,
    password: cfg.DB_PASSWORD,
    ssl: buildSslConfig(cfg),
  };
}

export function getOperatorDbConfig() {
  const cfg = getConfig();
  if (!cfg.DB_OPERATOR_USER || !cfg.DB_OPERATOR_PASSWORD) {
    throw new Error('Operator DB credentials not configured');
  }
  return {
    host: cfg.DB_HOST,
    port: parseInt(cfg.DB_PORT, 10),
    database: cfg.DB_NAME,
    user: cfg.DB_OPERATOR_USER,
    password: cfg.DB_OPERATOR_PASSWORD,
    ssl: buildSslConfig(cfg),
  };
}

export function getBreakGlassDbConfig() {
  const cfg = getConfig();
  if (!cfg.DB_BREAKGLASS_USER || !cfg.DB_BREAKGLASS_PASSWORD) {
    throw new Error('Break-glass DB credentials not configured');
  }
  return {
    host: cfg.DB_HOST,
    port: parseInt(cfg.DB_PORT, 10),
    database: cfg.DB_NAME,
    user: cfg.DB_BREAKGLASS_USER,
    password: cfg.DB_BREAKGLASS_PASSWORD,
    ssl: buildSslConfig(cfg),
  };
}
