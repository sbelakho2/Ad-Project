/**
 * CONTROL PLANE CONFIGURATION
 * 
 * CRITICAL: This is the operator control plane - completely separate from customer API.
 * No customer tokens, no shared secrets, no customer routes.
 */

import { z } from 'zod';

const envSchema = z.object({
  PORT: z.string().default('4000'),
  NODE_ENV: z.enum(['development', 'production', 'test']).default('production'),
  
  // Operator Auth (SEPARATE REALM)
  OPERATOR_JWT_SECRET: z.string().min(32),
  OPERATOR_SESSION_TTL_HOURS: z.string().default('4'),
  OPERATOR_CREDENTIALS: z.string().optional(), // email:hash pairs
  
  // IP Allowlist
  ALLOWED_IPS: z.string().default('127.0.0.1/32,::1/128'),
  
  // Database - Read-Only (monitoring)
  DB_HOST: z.string(),
  DB_PORT: z.string().default('5432'),
  DB_NAME: z.string(),
  DB_USER: z.string(),
  DB_PASSWORD: z.string(),
  DB_SSL: z.string().default('true'),
  
  // Database - Operator (safe actions)
  DB_OPERATOR_USER: z.string().optional(),
  DB_OPERATOR_PASSWORD: z.string().optional(),
  
  // Database - Break-Glass (emergency)
  DB_BREAKGLASS_USER: z.string().optional(),
  DB_BREAKGLASS_PASSWORD: z.string().optional(),
  
  // Host Telemetry
  HOST_TELEMETRY_ENDPOINT: z.string().optional(),
  
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
  
  const pairs = cfg.OPERATOR_CREDENTIALS.split(',');
  for (const pair of pairs) {
    const [email, hash] = pair.split(':');
    if (email && hash) {
      creds.set(email.trim().toLowerCase(), hash.trim());
    }
  }
  
  return creds;
}

// Parse allowed IPs from environment
export function getAllowedIPs(): string[] {
  const cfg = getConfig();
  return cfg.ALLOWED_IPS.split(',').map(ip => ip.trim()).filter(Boolean);
}

// Database connection configs
export function getReadOnlyDbConfig() {
  const cfg = getConfig();
  return {
    host: cfg.DB_HOST,
    port: parseInt(cfg.DB_PORT, 10),
    database: cfg.DB_NAME,
    user: cfg.DB_USER,
    password: cfg.DB_PASSWORD,
    ssl: cfg.DB_SSL === 'true' ? { rejectUnauthorized: false } : false,
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
    ssl: cfg.DB_SSL === 'true' ? { rejectUnauthorized: false } : false,
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
    ssl: cfg.DB_SSL === 'true' ? { rejectUnauthorized: false } : false,
  };
}
