/**
 * Enhanced Redaction Service for VRA outputs (CSV/JSON evidence).
 * 
 * Comprehensive patterns to remove potential PII/secrets from free-form fields.
 * 
 * Coverage includes:
 * - Email addresses (plain and URL-encoded)
 * - Phone numbers (US/international formats)
 * - Social Security Numbers
 * - Credit/debit card numbers (Luhn-validated)
 * - Authorization/Bearer tokens
 * - JWT tokens
 * - OAuth tokens in query strings
 * - API keys (Stripe, AWS, Google, GitHub, etc.)
 * - IP addresses (IPv4 and IPv6)
 * - Long hex secrets
 * - Database connection strings
 * - Webhook URLs with embedded tokens
 * - Basic auth credentials
 * - Private keys (PEM format markers)
 */

/**
 * Validates a numeric string using the Luhn algorithm (credit card checksum).
 * Used to reduce false positives on card-like numeric sequences.
 */
function passesLuhnCheck(digits: string): boolean {
  const nums = digits.split('').map(Number);
  let sum = 0;
  let isSecond = false;
  for (let i = nums.length - 1; i >= 0; i--) {
    let d = nums[i];
    if (isSecond) {
      d *= 2;
      if (d > 9) d -= 9;
    }
    sum += d;
    isSecond = !isSecond;
  }
  return sum % 10 === 0;
}

/**
 * Redaction configuration options
 */
export interface RedactionOptions {
  /** If true, validate card-like numbers with Luhn before redacting */
  luhnValidate?: boolean;
  /** If true, redact IPv4 addresses */
  redactIPs?: boolean;
  /** If true, redact phone numbers */
  redactPhones?: boolean;
  /** Custom patterns to apply (array of { pattern: RegExp, replacement: string }) */
  customPatterns?: Array<{ pattern: RegExp; replacement: string }>;
}

const DEFAULT_OPTIONS: RedactionOptions = {
  luhnValidate: true,
  redactIPs: true,
  redactPhones: true,
};

/**
 * Core redaction patterns - applied in order (more specific first)
 */
const REDACTION_PATTERNS: Array<{ pattern: RegExp; replacement: string | ((match: string) => string) }> = [
  // ─────────────────────────────────────────────────────────────────────────────
  // API Keys and Secrets (specific formats first)
  // ─────────────────────────────────────────────────────────────────────────────
  
  // Stripe keys (live and test)
  { pattern: /sk_live_[A-Za-z0-9]{6,}/g, replacement: 'sk_live_[REDACTED]' },
  { pattern: /sk_test_[A-Za-z0-9]{6,}/g, replacement: 'sk_test_[REDACTED]' },
  { pattern: /pk_live_[A-Za-z0-9]{6,}/g, replacement: 'pk_live_[REDACTED]' },
  { pattern: /pk_test_[A-Za-z0-9]{6,}/g, replacement: 'pk_test_[REDACTED]' },
  { pattern: /rk_live_[A-Za-z0-9]{6,}/g, replacement: 'rk_live_[REDACTED]' },
  { pattern: /rk_test_[A-Za-z0-9]{6,}/g, replacement: 'rk_test_[REDACTED]' },
  
  // AWS keys
  { pattern: /AKIA[0-9A-Z]{16}/g, replacement: '[REDACTED_AWS_KEY]' },
  { pattern: /(?:aws_secret_access_key|AWS_SECRET_ACCESS_KEY)[=:\s]+["']?[A-Za-z0-9/+=]{40}["']?/gi, replacement: '[REDACTED_AWS_SECRET]' },
  
  // Google Cloud API keys
  { pattern: /AIza[0-9A-Za-z_-]{35}/g, replacement: '[REDACTED_GOOGLE_API_KEY]' },
  
  // GitHub tokens
  { pattern: /gh[pousr]_[A-Za-z0-9_]{36,}/g, replacement: '[REDACTED_GITHUB_TOKEN]' },
  { pattern: /github_pat_[A-Za-z0-9_]{22,}/g, replacement: '[REDACTED_GITHUB_PAT]' },
  
  // Slack tokens
  { pattern: /xox[baprs]-[0-9]{10,}-[A-Za-z0-9-]+/g, replacement: '[REDACTED_SLACK_TOKEN]' },
  
  // SendGrid API keys
  { pattern: /SG\.[A-Za-z0-9_-]{12,}\.[A-Za-z0-9_-]{32,}/g, replacement: '[REDACTED_SENDGRID_KEY]' },
  
  // Twilio keys
  { pattern: /SK[0-9a-f]{32}/g, replacement: '[REDACTED_TWILIO_KEY]' },
  
  // ─────────────────────────────────────────────────────────────────────────────
  // Authentication Tokens
  // ─────────────────────────────────────────────────────────────────────────────
  
  // JWT tokens (base64url encoded, starting with eyJ)
  { pattern: /eyJ[A-Za-z0-9_-]{2,}\.[A-Za-z0-9_-]{2,}\.[A-Za-z0-9_-]{2,}/g, replacement: '[REDACTED_JWT]' },
  { pattern: /eyJ[A-Za-z0-9_%~-]{2,}(?:%2E|\.)[A-Za-z0-9_%~-]{2,}(?:%2E|\.)[A-Za-z0-9_%~-]{2,}/gi, replacement: '[REDACTED_JWT]' },
  
  // Bearer tokens in headers
  { pattern: /(authorization"?\s*:\s*")Bearer\s+[^"\s]+/gi, replacement: '$1Bearer [REDACTED]' },
  { pattern: /Bearer\s+[A-Za-z0-9._~+\/-]{6,}/g, replacement: 'Bearer [REDACTED]' },
  
  // OAuth tokens in query strings
  { pattern: /((?:access_token|id_token|refresh_token|token)=)[A-Za-z0-9._-]+/gi, replacement: '$1[REDACTED]' },
  { pattern: /((?:access_token|id_token|refresh_token|token)=)(?:[A-Za-z0-9._-]|%2E|%2e)+/gi, replacement: '$1[REDACTED]' },
  
  // Basic auth in URLs
  { pattern: /:\/\/[^:]+:[^@]+@/g, replacement: '://[REDACTED_CREDENTIALS]@' },
  
  // ─────────────────────────────────────────────────────────────────────────────
  // Personal Identifiable Information (PII)
  // ─────────────────────────────────────────────────────────────────────────────
  
  // Email addresses (plain)
  { pattern: /[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi, replacement: '[REDACTED_EMAIL]' },
  // Email addresses (URL encoded)
  { pattern: /[A-Z0-9._%+-]+%40[A-Z0-9.-]+(?:(?:%2E|\.)[A-Z]{2,})/gi, replacement: '[REDACTED_EMAIL]' },
  
  // US Social Security Numbers (XXX-XX-XXXX or XXXXXXXXX)
  { pattern: /\b\d{3}[- ]?\d{2}[- ]?\d{4}\b/g, replacement: '[REDACTED_SSN]' },
  
  // US Phone numbers (various formats)
  { pattern: /\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b/g, replacement: '[REDACTED_PHONE]' },
  // International phone with + prefix
  { pattern: /\+\d{1,3}[-.\s]?\d{6,14}/g, replacement: '[REDACTED_PHONE]' },
  
  // ─────────────────────────────────────────────────────────────────────────────
  // Financial Data
  // ─────────────────────────────────────────────────────────────────────────────
  
  // Card numbers (13-19 digits, with optional separators) - Luhn checked separately
  // This is a placeholder; actual card matching uses passesLuhnCheck below
  
  // Bank routing numbers (9 digits starting with specific prefixes)
  { pattern: /\b[0-3]\d{8}\b/g, replacement: '[REDACTED_ROUTING]' },
  
  // ─────────────────────────────────────────────────────────────────────────────
  // Infrastructure Secrets
  // ─────────────────────────────────────────────────────────────────────────────
  
  // Database connection strings
  { pattern: /(?:postgres|postgresql|mysql|mongodb|redis):\/\/[^\s"']+/gi, replacement: '[REDACTED_DB_URL]' },
  
  // Private key markers (PEM format)
  { pattern: /-----BEGIN\s+(?:RSA\s+)?PRIVATE\s+KEY-----[\s\S]*?-----END\s+(?:RSA\s+)?PRIVATE\s+KEY-----/g, replacement: '[REDACTED_PRIVATE_KEY]' },
  { pattern: /-----BEGIN\s+(?:EC\s+)?PRIVATE\s+KEY-----[\s\S]*?-----END\s+(?:EC\s+)?PRIVATE\s+KEY-----/g, replacement: '[REDACTED_PRIVATE_KEY]' },
  
  // Webhook URLs with embedded tokens (common patterns)
  { pattern: /https:\/\/hooks\.slack\.com\/services\/[A-Za-z0-9/]+/g, replacement: '[REDACTED_WEBHOOK_URL]' },
  { pattern: /https:\/\/discord(?:app)?\.com\/api\/webhooks\/\d+\/[A-Za-z0-9_-]+/g, replacement: '[REDACTED_WEBHOOK_URL]' },
  
  // Long hex secrets (32+ hex chars) - likely API keys/tokens
  { pattern: /\b[0-9a-f]{32,}\b/gi, replacement: '[REDACTED_HEX]' },
  
  // Base64 encoded secrets (long base64 strings, 40+ chars)
  { pattern: /\b[A-Za-z0-9+/]{40,}={0,2}\b/g, replacement: '[REDACTED_BASE64]' },
];

/**
 * IPv4 address pattern (separate for optional redaction)
 */
const IPV4_PATTERN = /\b(?:25[0-5]|2[0-4]\d|1\d{2}|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d{2}|[1-9]?\d)){3}\b/g;

/**
 * IPv6 address pattern (simplified - covers most common formats)
 */
const IPV6_PATTERN = /\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b|\b(?:[0-9a-fA-F]{1,4}:){1,7}:\b|\b(?:[0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}\b/g;

/**
 * Credit card number pattern (for Luhn validation)
 */
const CARD_NUMBER_PATTERN = /\b\d{4}[- ]?\d{4}[- ]?\d{4}[- ]?\d{1,7}\b/g;

/**
 * Redact sensitive information from a string.
 * 
 * @param val - The string to redact
 * @param options - Redaction options (defaults to redacting all known patterns)
 * @returns The redacted string
 * 
 * @example
 * ```typescript
 * redactString('Contact john@example.com or call 555-123-4567');
 * // Returns: 'Contact [REDACTED_EMAIL] or call [REDACTED_PHONE]'
 * 
 * redactString('API Key: sk_live_abc123xyz', { luhnValidate: false });
 * // Returns: 'API Key: sk_live_[REDACTED]'
 * ```
 */
export function redactString(val: string, options: RedactionOptions = {}): string {
  if (!val) return '';
  
  const opts = { ...DEFAULT_OPTIONS, ...options };
  let s = String(val);
  
  // Apply all standard patterns
  for (const { pattern, replacement } of REDACTION_PATTERNS) {
    if (typeof replacement === 'function') {
      s = s.replace(pattern, replacement);
    } else {
      // Clone regex to reset lastIndex
      const clonedPattern = new RegExp(pattern.source, pattern.flags);
      s = s.replace(clonedPattern, replacement);
    }
  }
  
  // Apply custom patterns if provided
  if (opts.customPatterns) {
    for (const { pattern, replacement } of opts.customPatterns) {
      s = s.replace(pattern, replacement);
    }
  }
  
  // Credit card numbers with Luhn validation
  if (opts.luhnValidate) {
    s = s.replace(CARD_NUMBER_PATTERN, (match) => {
      const digitsOnly = match.replace(/[- ]/g, '');
      if (digitsOnly.length >= 13 && digitsOnly.length <= 19 && passesLuhnCheck(digitsOnly)) {
        return '[REDACTED_CARD]';
      }
      return match;
    });
  } else {
    // Without Luhn validation, redact any 13-19 digit sequences
    s = s.replace(/\b\d{13,19}\b/g, '[REDACTED_NUMERIC]');
  }
  
  // IP addresses (optional)
  if (opts.redactIPs) {
    s = s.replace(IPV4_PATTERN, '[REDACTED_IP]');
    s = s.replace(IPV6_PATTERN, '[REDACTED_IP]');
  }
  
  return s;
}

/**
 * Redact sensitive fields from an object recursively.
 * 
 * @param obj - The object to redact
 * @param options - Redaction options
 * @returns A new object with sensitive fields redacted
 */
export function redactObject<T>(obj: T, options: RedactionOptions = {}): T {
  if (obj === null || obj === undefined) return obj;
  
  if (typeof obj === 'string') {
    return redactString(obj, options) as T;
  }
  
  if (Array.isArray(obj)) {
    return obj.map(item => redactObject(item, options)) as T;
  }
  
  if (typeof obj === 'object') {
    const result: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(obj)) {
      // Check if key itself is a sensitive field name
      const lowercaseKey = key.toLowerCase();
      if (SENSITIVE_FIELD_NAMES.has(lowercaseKey)) {
        result[key] = '[REDACTED]';
      } else {
        result[key] = redactObject(value, options);
      }
    }
    return result as T;
  }
  
  return obj;
}

/**
 * Field names that should always be fully redacted when found as keys
 */
const SENSITIVE_FIELD_NAMES = new Set([
  'password',
  'passwd',
  'secret',
  'secret_key',
  'secretkey',
  'api_key',
  'apikey',
  'api_secret',
  'apisecret',
  'access_token',
  'accesstoken',
  'refresh_token',
  'refreshtoken',
  'private_key',
  'privatekey',
  'auth_token',
  'authtoken',
  'bearer',
  'authorization',
  'ssn',
  'social_security',
  'socialsecurity',
  'credit_card',
  'creditcard',
  'card_number',
  'cardnumber',
  'cvv',
  'cvc',
  'pin',
  'bank_account',
  'bankaccount',
  'routing_number',
  'routingnumber',
  'session_id',
  'sessionid',
  'session_token',
  'sessiontoken',
]);

/**
 * Check if a string contains any sensitive patterns without modifying it.
 * Useful for validation and auditing.
 * 
 * @param val - The string to check
 * @returns True if the string contains sensitive data
 */
export function containsSensitiveData(val: string): boolean {
  if (!val) return false;
  const redacted = redactString(val);
  return redacted !== val;
}

/**
 * Generate a summary of what types of sensitive data were found.
 * 
 * @param val - The string to analyze
 * @returns Object with counts of each type of sensitive data found
 */
export function analyzeSensitiveData(val: string): Record<string, number> {
  if (!val) return {};
  
  const counts: Record<string, number> = {};
  
  // Count each type of redaction
  const redactionTypes = [
    { pattern: /[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi, type: 'email' },
    { pattern: /eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, type: 'jwt' },
    { pattern: /Bearer\s+[A-Za-z0-9._~+\/-]{20,}/g, type: 'bearer_token' },
    { pattern: /sk_(?:live|test)_[A-Za-z0-9]+/g, type: 'stripe_key' },
    { pattern: /AKIA[0-9A-Z]{16}/g, type: 'aws_key' },
    { pattern: /\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b/g, type: 'phone' },
    { pattern: /\b\d{3}[- ]?\d{2}[- ]?\d{4}\b/g, type: 'ssn' },
    { pattern: IPV4_PATTERN, type: 'ipv4' },
  ];
  
  for (const { pattern, type } of redactionTypes) {
    const matches = val.match(pattern);
    if (matches && matches.length > 0) {
      counts[type] = matches.length;
    }
  }
  
  return counts;
}

export default { 
  redactString, 
  redactObject, 
  containsSensitiveData, 
  analyzeSensitiveData,
  passesLuhnCheck,
};
