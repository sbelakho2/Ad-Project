/**
 * SSAI Security Handler
 *
 * Handles session token generation, validation, rate limiting,
 * and SSRF-safe URL fetching for SSAI operations.
 */

import crypto from 'crypto';
import dns from 'dns/promises';

export interface SessionToken {
  sessionId: string;
  tenantId: string;
  channelId?: string;
  assetId?: string;
  issuedAt: number;
  expiresAt: number;
  signature: string;
}

export interface RateLimitConfig {
  windowMs: number;
  maxRequests: number;
  keyPrefix: string;
}

export interface RateLimitResult {
  allowed: boolean;
  remaining: number;
  resetAt: Date;
  retryAfter?: number;
}

export interface SSRFConfig {
  allowedDomains: string[];
  blockedIpRanges: string[];
  allowPrivateNetworks: boolean;
  maxRedirects: number;
  timeoutMs: number;
}

export interface FetchResult {
  success: boolean;
  status?: number;
  body?: Buffer | string;
  headers?: Record<string, string>;
  error?: string;
}

// Default blocked IP ranges (private networks, localhost, link-local)
const DEFAULT_BLOCKED_IP_RANGES = [
  '10.0.0.0/8',
  '172.16.0.0/12',
  '192.168.0.0/16',
  '127.0.0.0/8',
  '169.254.0.0/16',
  '::1/128',
  'fc00::/7',
  'fe80::/10',
];

/**
 * Security Handler for SSAI operations.
 *
 * Responsibilities:
 * 1. Generate and validate signed session tokens
 * 2. Rate limiting per tenant, session, and IP
 * 3. SSRF-safe URL fetching for external resources
 * 4. Anti-tamper validation for manifest URLs
 */
export class SecurityHandler {
  private tokenSecret: string;
  private rateLimitStore: Map<string, { count: number; resetAt: number }>;
  private ssrfConfig: SSRFConfig;

  constructor(
    tokenSecret?: string,
    ssrfConfig?: Partial<SSRFConfig>
  ) {
    this.tokenSecret = tokenSecret || process.env.SSAI_TOKEN_SECRET || 'ssai-default-secret';
    this.rateLimitStore = new Map();
    this.ssrfConfig = {
      allowedDomains: [],
      blockedIpRanges: DEFAULT_BLOCKED_IP_RANGES,
      allowPrivateNetworks: false,
      maxRedirects: 3,
      timeoutMs: 10000,
      ...ssrfConfig,
    };
  }

  // ==================== Token Management ====================

  /**
   * Generate a signed session token.
   */
  generateSessionToken(
    sessionId: string,
    tenantId: string,
    options?: {
      channelId?: string;
      assetId?: string;
      ttlSeconds?: number;
    }
  ): string {
    const now = Math.floor(Date.now() / 1000);
    const ttl = options?.ttlSeconds || 3600; // Default 1 hour

    const tokenData: Omit<SessionToken, 'signature'> = {
      sessionId,
      tenantId,
      channelId: options?.channelId,
      assetId: options?.assetId,
      issuedAt: now,
      expiresAt: now + ttl,
    };

    const signature = this.signTokenData(tokenData);
    const token: SessionToken = { ...tokenData, signature };

    // Encode as base64url
    return Buffer.from(JSON.stringify(token)).toString('base64url');
  }

  /**
   * Validate and decode a session token.
   */
  validateSessionToken(token: string): {
    valid: boolean;
    decoded?: SessionToken;
    error?: string;
  } {
    try {
      // Decode base64url
      const decoded = JSON.parse(
        Buffer.from(token, 'base64url').toString('utf8')
      ) as SessionToken;

      // Check expiry
      const now = Math.floor(Date.now() / 1000);
      if (decoded.expiresAt < now) {
        return { valid: false, error: 'Token expired' };
      }

      // Verify signature
      const expectedSignature = this.signTokenData({
        sessionId: decoded.sessionId,
        tenantId: decoded.tenantId,
        channelId: decoded.channelId,
        assetId: decoded.assetId,
        issuedAt: decoded.issuedAt,
        expiresAt: decoded.expiresAt,
      });

      if (decoded.signature !== expectedSignature) {
        return { valid: false, error: 'Invalid signature' };
      }

      return { valid: true, decoded };
    } catch {
      return { valid: false, error: 'Invalid token format' };
    }
  }

  /**
   * Refresh a session token (extend expiry).
   */
  refreshSessionToken(
    token: string,
    newTtlSeconds?: number
  ): string | null {
    const validation = this.validateSessionToken(token);
    if (!validation.valid || !validation.decoded) {
      return null;
    }

    return this.generateSessionToken(
      validation.decoded.sessionId,
      validation.decoded.tenantId,
      {
        channelId: validation.decoded.channelId,
        assetId: validation.decoded.assetId,
        ttlSeconds: newTtlSeconds || 3600,
      }
    );
  }

  /**
   * Sign token data with HMAC-SHA256.
   */
  private signTokenData(data: Omit<SessionToken, 'signature'>): string {
    const payload = JSON.stringify(data);
    return crypto
      .createHmac('sha256', this.tokenSecret)
      .update(payload)
      .digest('hex');
  }

  // ==================== Rate Limiting ====================

  /**
   * Check rate limit for a given key.
   */
  checkRateLimit(
    key: string,
    config: RateLimitConfig
  ): RateLimitResult {
    const now = Date.now();
    const fullKey = `${config.keyPrefix}:${key}`;
    const entry = this.rateLimitStore.get(fullKey);

    // Clean expired entries periodically
    if (Math.random() < 0.01) {
      this.cleanupExpiredRateLimits();
    }

    // No entry or expired - create new window
    if (!entry || entry.resetAt <= now) {
      this.rateLimitStore.set(fullKey, {
        count: 1,
        resetAt: now + config.windowMs,
      });

      return {
        allowed: true,
        remaining: config.maxRequests - 1,
        resetAt: new Date(now + config.windowMs),
      };
    }

    // Check if limit exceeded
    if (entry.count >= config.maxRequests) {
      const retryAfter = Math.ceil((entry.resetAt - now) / 1000);
      return {
        allowed: false,
        remaining: 0,
        resetAt: new Date(entry.resetAt),
        retryAfter,
      };
    }

    // Increment counter
    entry.count++;
    this.rateLimitStore.set(fullKey, entry);

    return {
      allowed: true,
      remaining: config.maxRequests - entry.count,
      resetAt: new Date(entry.resetAt),
    };
  }

  /**
   * Rate limit by tenant ID.
   */
  checkTenantRateLimit(
    tenantId: string,
    maxRequests = 1000,
    windowMs = 60000
  ): RateLimitResult {
    return this.checkRateLimit(tenantId, {
      keyPrefix: 'tenant',
      maxRequests,
      windowMs,
    });
  }

  /**
   * Rate limit by session ID.
   */
  checkSessionRateLimit(
    sessionId: string,
    maxRequests = 60,
    windowMs = 60000
  ): RateLimitResult {
    return this.checkRateLimit(sessionId, {
      keyPrefix: 'session',
      maxRequests,
      windowMs,
    });
  }

  /**
   * Rate limit by IP address.
   */
  checkIpRateLimit(
    ip: string,
    maxRequests = 100,
    windowMs = 60000
  ): RateLimitResult {
    // Normalize IP (handle IPv6 mapped IPv4)
    const normalizedIp = this.normalizeIp(ip);
    return this.checkRateLimit(normalizedIp, {
      keyPrefix: 'ip',
      maxRequests,
      windowMs,
    });
  }

  /**
   * Clean up expired rate limit entries.
   */
  private cleanupExpiredRateLimits(): void {
    const now = Date.now();
    for (const [key, entry] of this.rateLimitStore.entries()) {
      if (entry.resetAt <= now) {
        this.rateLimitStore.delete(key);
      }
    }
  }

  // ==================== SSRF Protection ====================

  /**
   * SSRF-safe URL fetch.
   *
   * Validates URL against allowed domains and blocked IP ranges
   * before fetching to prevent SSRF attacks.
   * 
   * SECURITY: Resolves DNS and validates resolved IPs to prevent DNS rebinding attacks.
   */
  async safeFetch(
    url: string,
    options?: {
      method?: string;
      headers?: Record<string, string>;
      body?: string | Buffer;
    }
  ): Promise<FetchResult> {
    // Validate URL before fetching
    const validation = this.validateUrl(url);
    if (!validation.safe) {
      return {
        success: false,
        error: validation.reason || 'URL validation failed',
      };
    }

    // SECURITY: Resolve DNS and validate resolved IP to prevent DNS rebinding attacks
    const dnsValidation = await this.validateDnsResolution(url);
    if (!dnsValidation.safe) {
      return {
        success: false,
        error: dnsValidation.reason || 'DNS validation failed',
      };
    }

    try {
      const controller = new AbortController();
      const timeout = setTimeout(
        () => controller.abort(),
        this.ssrfConfig.timeoutMs
      );

      const response = await fetch(url, {
        method: options?.method || 'GET',
        headers: options?.headers,
        body: options?.body,
        signal: controller.signal,
        redirect: 'manual', // Handle redirects manually for safety
      });

      clearTimeout(timeout);

      // Handle redirects safely
      if (response.status >= 300 && response.status < 400) {
        const location = response.headers.get('location');
        if (location) {
          return this.handleRedirect(location, options, 1);
        }
      }

      const body = await response.arrayBuffer();

      return {
        success: response.ok,
        status: response.status,
        body: Buffer.from(body),
        headers: Object.fromEntries(response.headers.entries()),
      };
    } catch (error) {
      return {
        success: false,
        error: error instanceof Error ? error.message : 'Fetch failed',
      };
    }
  }

  /**
   * Validate DNS resolution to prevent DNS rebinding attacks.
   * Resolves the hostname and checks if any resolved IP is in private ranges.
   */
  private async validateDnsResolution(url: string): Promise<{ safe: boolean; reason?: string }> {
    try {
      const parsed = new URL(url);
      const hostname = parsed.hostname;

      // Skip DNS lookup for IP addresses
      if (this.isIpAddress(hostname)) {
        return { safe: true };
      }

      // Resolve hostname to IP addresses
      let addresses: string[] = [];
      try {
        const ipv4Addresses = await dns.resolve4(hostname).catch(() => [] as string[]);
        const ipv6Addresses = await dns.resolve6(hostname).catch(() => [] as string[]);
        addresses = [...ipv4Addresses, ...ipv6Addresses];
      } catch {
        // DNS resolution failed - could be a non-existent domain, allow fetch to fail naturally
        return { safe: true };
      }

      // Check if any resolved IP is private
      for (const ip of addresses) {
        const normalizedIp = this.normalizeIp(ip);
        if (this.isPrivateIp(normalizedIp)) {
          return { 
            safe: false, 
            reason: `DNS rebinding attack detected: ${hostname} resolves to private IP ${ip}` 
          };
        }

        // Also check against blocked IP ranges
        for (const range of this.ssrfConfig.blockedIpRanges) {
          if (this.isIpInRange(normalizedIp, range)) {
            return { 
              safe: false, 
              reason: `DNS rebinding attack detected: ${hostname} resolves to blocked IP ${ip}` 
            };
          }
        }
      }

      return { safe: true };
    } catch {
      return { safe: false, reason: 'Failed to validate DNS resolution' };
    }
  }

  /**
   * Handle redirects safely with depth limit.
   */
  private async handleRedirect(
    location: string,
    options?: {
      method?: string;
      headers?: Record<string, string>;
      body?: string | Buffer;
    },
    depth: number = 0
  ): Promise<FetchResult> {
    if (depth >= this.ssrfConfig.maxRedirects) {
      return {
        success: false,
        error: 'Too many redirects',
      };
    }

    // Validate redirect URL
    const validation = this.validateUrl(location);
    if (!validation.safe) {
      return {
        success: false,
        error: `Unsafe redirect target: ${validation.reason}`,
      };
    }

    try {
      const controller = new AbortController();
      const timeout = setTimeout(
        () => controller.abort(),
        this.ssrfConfig.timeoutMs
      );

      const response = await fetch(location, {
        method: options?.method || 'GET',
        headers: options?.headers,
        body: options?.body,
        signal: controller.signal,
        redirect: 'manual',
      });

      clearTimeout(timeout);

      // Check for further redirects
      if (response.status >= 300 && response.status < 400) {
        const nextLocation = response.headers.get('location');
        if (nextLocation) {
          return this.handleRedirect(nextLocation, options, depth + 1);
        }
      }

      const body = await response.arrayBuffer();

      return {
        success: response.ok,
        status: response.status,
        body: Buffer.from(body),
        headers: Object.fromEntries(response.headers.entries()),
      };
    } catch (error) {
      return {
        success: false,
        error: error instanceof Error ? error.message : 'Redirect fetch failed',
      };
    }
  }

  /**
   * Validate URL for SSRF safety.
   */
  validateUrl(url: string): { safe: boolean; reason?: string } {
    try {
      const parsed = new URL(url);

      // Only allow http/https
      if (!['http:', 'https:'].includes(parsed.protocol)) {
        return { safe: false, reason: 'Invalid protocol' };
      }

      // Check allowed domains if configured
      if (this.ssrfConfig.allowedDomains.length > 0) {
        const domainAllowed = this.ssrfConfig.allowedDomains.some(
          (domain) =>
            parsed.hostname === domain ||
            parsed.hostname.endsWith(`.${domain}`)
        );
        if (!domainAllowed) {
          return { safe: false, reason: 'Domain not in allowlist' };
        }
      }

      // Check for IP address in hostname
      if (this.isIpAddress(parsed.hostname)) {
        if (!this.ssrfConfig.allowPrivateNetworks) {
          if (this.isPrivateIp(parsed.hostname)) {
            return { safe: false, reason: 'Private IP addresses not allowed' };
          }
        }
      }

      // Check blocked IP ranges
      for (const range of this.ssrfConfig.blockedIpRanges) {
        if (this.isIpInRange(parsed.hostname, range)) {
          return { safe: false, reason: 'IP address in blocked range' };
        }
      }

      return { safe: true };
    } catch {
      return { safe: false, reason: 'Invalid URL' };
    }
  }

  /**
   * Check if hostname is an IP address.
   */
  private isIpAddress(hostname: string): boolean {
    // IPv4 pattern
    const ipv4 = /^(\d{1,3}\.){3}\d{1,3}$/;
    // IPv6 pattern (simplified)
    const ipv6 = /^([0-9a-fA-F]{0,4}:){2,7}[0-9a-fA-F]{0,4}$/;

    return ipv4.test(hostname) || ipv6.test(hostname);
  }

  /**
   * Check if IP is in private range.
   */
  private isPrivateIp(ip: string): boolean {
    const privateRanges = [
      '0.0.0.0/8',      // "This" network
      '10.0.0.0/8',
      '172.16.0.0/12',
      '192.168.0.0/16',
      '127.0.0.0/8',
      '169.254.0.0/16', // Link-local
      '100.64.0.0/10',  // Carrier-grade NAT
      '::1/128',        // IPv6 localhost
      'fc00::/7',       // IPv6 ULA
      'fe80::/10',      // IPv6 link-local
    ];

    return privateRanges.some((range) => this.isIpInRange(ip, range));
  }

  /**
   * Check if IP is in CIDR range.
   * SECURITY: Properly validates CIDR ranges to prevent SSRF bypass via edge-case IPs.
   */
  private isIpInRange(ip: string, cidr: string): boolean {
    const [rangeIp, prefixStr] = cidr.split('/');
    const prefix = parseInt(prefixStr, 10);

    // Handle IPv6
    if (rangeIp.includes(':')) {
      return this.isIpv6InRange(ip, rangeIp, prefix);
    }

    // IPv4 CIDR check
    const rangeParts = rangeIp.split('.').map(Number);
    const ipParts = ip.split('.').map(Number);

    if (rangeParts.length !== 4 || ipParts.length !== 4) {
      return false;
    }

    // Validate IP octets
    if (ipParts.some(p => isNaN(p) || p < 0 || p > 255)) {
      return false;
    }

    // Convert to 32-bit integers
    const rangeInt = (rangeParts[0] << 24) | (rangeParts[1] << 16) | (rangeParts[2] << 8) | rangeParts[3];
    const ipInt = (ipParts[0] << 24) | (ipParts[1] << 16) | (ipParts[2] << 8) | ipParts[3];

    // Create mask from prefix
    const mask = prefix === 0 ? 0 : (~0 << (32 - prefix)) >>> 0;

    // Compare masked values (use >>> 0 to ensure unsigned comparison)
    return ((rangeInt & mask) >>> 0) === ((ipInt & mask) >>> 0);
  }

  /**
   * Check if IPv6 address is in CIDR range.
   */
  private isIpv6InRange(ip: string, rangeIp: string, prefix: number): boolean {
    // Normalize IPv6 addresses
    try {
      const ipExpanded = this.expandIpv6(ip);
      const rangeExpanded = this.expandIpv6(rangeIp);

      if (!ipExpanded || !rangeExpanded) return false;

      // Compare prefix bits
      const ipBits = ipExpanded.split(':').map(h => parseInt(h, 16));
      const rangeBits = rangeExpanded.split(':').map(h => parseInt(h, 16));

      let bitsToCheck = prefix;
      for (let i = 0; i < 8 && bitsToCheck > 0; i++) {
        const segmentBits = Math.min(16, bitsToCheck);
        const mask = segmentBits === 16 ? 0xFFFF : ((1 << segmentBits) - 1) << (16 - segmentBits);
        if ((ipBits[i] & mask) !== (rangeBits[i] & mask)) {
          return false;
        }
        bitsToCheck -= 16;
      }
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Expand shortened IPv6 address to full form.
   */
  private expandIpv6(ip: string): string | null {
    // Handle IPv4-mapped IPv6
    if (ip.includes('.')) {
      const parts = ip.split(':');
      const ipv4Part = parts.pop()!;
      const ipv4Octets = ipv4Part.split('.').map(Number);
      if (ipv4Octets.length !== 4) return null;
      const hex1 = ((ipv4Octets[0] << 8) | ipv4Octets[1]).toString(16).padStart(4, '0');
      const hex2 = ((ipv4Octets[2] << 8) | ipv4Octets[3]).toString(16).padStart(4, '0');
      ip = parts.join(':') + ':' + hex1 + ':' + hex2;
    }

    // Expand :: notation
    if (ip.includes('::')) {
      const sides = ip.split('::');
      const left = sides[0] ? sides[0].split(':') : [];
      const right = sides[1] ? sides[1].split(':') : [];
      const missing = 8 - left.length - right.length;
      const middle = Array(missing).fill('0000');
      ip = [...left, ...middle, ...right].join(':');
    }

    const segments = ip.split(':');
    if (segments.length !== 8) return null;
    return segments.map(s => s.padStart(4, '0')).join(':');
  }

  /**
   * Normalize IP address.
   */
  private normalizeIp(ip: string): string {
    // Handle IPv6-mapped IPv4 (::ffff:192.168.1.1)
    if (ip.startsWith('::ffff:')) {
      return ip.substring(7);
    }
    return ip;
  }

  // ==================== Anti-Tamper ====================

  /**
   * Generate manifest URL with anti-tamper signature.
   */
  signManifestUrl(
    baseUrl: string,
    sessionId: string,
    params: Record<string, string> = {}
  ): string {
    const url = new URL(baseUrl);

    // Add session ID
    url.searchParams.set('sid', sessionId);

    // Add custom params
    for (const [key, value] of Object.entries(params)) {
      url.searchParams.set(key, value);
    }

    // Add timestamp
    const timestamp = Math.floor(Date.now() / 1000).toString();
    url.searchParams.set('ts', timestamp);

    // Generate signature
    const signatureData = url.pathname + url.search;
    const signature = crypto
      .createHmac('sha256', this.tokenSecret)
      .update(signatureData)
      .digest('hex')
      .substring(0, 16);

    url.searchParams.set('sig', signature);

    return url.toString();
  }

  /**
   * Validate signed manifest URL.
   */
  validateManifestUrl(url: string): { valid: boolean; error?: string } {
    try {
      const parsed = new URL(url);
      const signature = parsed.searchParams.get('sig');
      const timestamp = parsed.searchParams.get('ts');

      if (!signature || !timestamp) {
        return { valid: false, error: 'Missing signature or timestamp' };
      }

      // Check timestamp (allow 5 minute window)
      const ts = parseInt(timestamp, 10);
      const now = Math.floor(Date.now() / 1000);
      if (Math.abs(now - ts) > 300) {
        return { valid: false, error: 'URL expired' };
      }

      // Remove signature and verify
      parsed.searchParams.delete('sig');
      const signatureData = parsed.pathname + parsed.search;
      const expectedSignature = crypto
        .createHmac('sha256', this.tokenSecret)
        .update(signatureData)
        .digest('hex')
        .substring(0, 16);

      if (signature !== expectedSignature) {
        return { valid: false, error: 'Invalid signature' };
      }

      return { valid: true };
    } catch {
      return { valid: false, error: 'Invalid URL format' };
    }
  }

  /**
   * Update SSRF configuration.
   */
  updateSSRFConfig(config: Partial<SSRFConfig>): void {
    this.ssrfConfig = { ...this.ssrfConfig, ...config };
  }
}

export default SecurityHandler;
