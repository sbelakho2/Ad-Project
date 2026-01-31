/**
 * SSAI Cache Policy
 *
 * Implements CDN-safe caching headers for SSAI manifests and segments.
 * Prevents cross-user cache poisoning while maintaining efficiency.
 */

export interface CacheHeaders {
  'Cache-Control': string;
  Vary?: string;
  ETag?: string;
  'Last-Modified'?: string;
  Expires?: string;
  'X-SSAI-Cache-Policy'?: string;
}

export interface CachePolicyConfig {
  /**
   * Default TTL for live manifests in seconds.
   * Should be shorter than segment duration to ensure fresh data.
   */
  liveManifestTtl: number;

  /**
   * Default TTL for VOD manifests in seconds.
   * Can be longer since VOD content is stable.
   */
  vodManifestTtl: number;

  /**
   * Default TTL for segments in seconds.
   * Segments are immutable once created.
   */
  segmentTtl: number;

  /**
   * TTL for init segments (CMAF).
   * Init segments are immutable.
   */
  initSegmentTtl: number;

  /**
   * Whether to include session tokens in cache keys.
   */
  sessionAwareCaching: boolean;

  /**
   * Headers to include in Vary for CDN.
   */
  varyHeaders: string[];
}

export type ResourceType =
  | 'live-manifest'
  | 'vod-manifest'
  | 'segment'
  | 'init-segment'
  | 'session-manifest';

/**
 * Cache Policy Manager for SSAI resources.
 *
 * Key principles:
 * 1. Session manifests MUST NOT be cached publicly (private/no-store)
 * 2. Segments can be cached with long TTL (immutable after creation)
 * 3. Live manifests need short TTL to reflect sliding window
 * 4. VOD manifests can have longer TTL but still need refresh capability
 * 5. Always use proper Vary headers to prevent cross-user poisoning
 */
export class CachePolicy {
  private config: CachePolicyConfig;

  constructor(config?: Partial<CachePolicyConfig>) {
    this.config = {
      liveManifestTtl: 2, // 2 seconds for live
      vodManifestTtl: 300, // 5 minutes for VOD
      segmentTtl: 86400, // 24 hours for segments
      initSegmentTtl: 604800, // 7 days for init segments
      sessionAwareCaching: true,
      varyHeaders: ['Accept', 'Accept-Encoding'],
      ...config,
    };
  }

  /**
   * Get cache headers for a resource type.
   */
  getHeaders(
    resourceType: ResourceType,
    options?: {
      sessionId?: string;
      etag?: string;
      lastModified?: Date;
    }
  ): CacheHeaders {
    switch (resourceType) {
      case 'session-manifest':
        return this.getSessionManifestHeaders(options);

      case 'live-manifest':
        return this.getLiveManifestHeaders(options);

      case 'vod-manifest':
        return this.getVodManifestHeaders(options);

      case 'segment':
        return this.getSegmentHeaders(options);

      case 'init-segment':
        return this.getInitSegmentHeaders(options);

      default:
        return this.getDefaultHeaders();
    }
  }

  /**
   * Session manifests: private, no public caching.
   *
   * These contain session-specific ad decisions and must
   * never be served to other users.
   */
  private getSessionManifestHeaders(options?: {
    etag?: string;
    lastModified?: Date;
  }): CacheHeaders {
    const headers: CacheHeaders = {
      'Cache-Control': 'private, no-store, no-cache, must-revalidate',
      'X-SSAI-Cache-Policy': 'session-manifest',
    };

    // Add Vary to ensure CDN doesn't cache across users
    if (this.config.sessionAwareCaching) {
      headers.Vary = [...this.config.varyHeaders, 'Authorization', 'Cookie'].join(
        ', '
      );
    }

    if (options?.etag) {
      headers.ETag = options.etag;
    }

    if (options?.lastModified) {
      headers['Last-Modified'] = options.lastModified.toUTCString();
    }

    return headers;
  }

  /**
   * Live manifests: short TTL, public cacheable with Vary.
   *
   * Must update frequently to reflect sliding window,
   * but can be cached briefly to reduce origin load.
   */
  private getLiveManifestHeaders(options?: {
    etag?: string;
    lastModified?: Date;
  }): CacheHeaders {
    const headers: CacheHeaders = {
      'Cache-Control': `public, max-age=${this.config.liveManifestTtl}, s-maxage=${this.config.liveManifestTtl}`,
      Vary: this.config.varyHeaders.join(', '),
      'X-SSAI-Cache-Policy': 'live-manifest',
    };

    if (options?.etag) {
      headers.ETag = options.etag;
    }

    if (options?.lastModified) {
      headers['Last-Modified'] = options.lastModified.toUTCString();
    }

    // Set explicit Expires for legacy CDNs
    const expires = new Date(
      Date.now() + this.config.liveManifestTtl * 1000
    );
    headers.Expires = expires.toUTCString();

    return headers;
  }

  /**
   * VOD manifests: medium TTL, publicly cacheable.
   *
   * VOD content is stable, so longer caching is safe.
   * Still need ETag for conditional requests.
   */
  private getVodManifestHeaders(options?: {
    etag?: string;
    lastModified?: Date;
  }): CacheHeaders {
    const headers: CacheHeaders = {
      'Cache-Control': `public, max-age=${this.config.vodManifestTtl}, s-maxage=${this.config.vodManifestTtl}`,
      Vary: this.config.varyHeaders.join(', '),
      'X-SSAI-Cache-Policy': 'vod-manifest',
    };

    if (options?.etag) {
      headers.ETag = options.etag;
    }

    if (options?.lastModified) {
      headers['Last-Modified'] = options.lastModified.toUTCString();
    }

    const expires = new Date(
      Date.now() + this.config.vodManifestTtl * 1000
    );
    headers.Expires = expires.toUTCString();

    return headers;
  }

  /**
   * Segments: long TTL, immutable, publicly cacheable.
   *
   * Once a segment is created, its content never changes.
   * Safe to cache aggressively.
   */
  private getSegmentHeaders(options?: {
    etag?: string;
    lastModified?: Date;
  }): CacheHeaders {
    const headers: CacheHeaders = {
      'Cache-Control': `public, max-age=${this.config.segmentTtl}, s-maxage=${this.config.segmentTtl}, immutable`,
      Vary: this.config.varyHeaders.join(', '),
      'X-SSAI-Cache-Policy': 'segment',
    };

    if (options?.etag) {
      headers.ETag = options.etag;
    }

    if (options?.lastModified) {
      headers['Last-Modified'] = options.lastModified.toUTCString();
    }

    const expires = new Date(
      Date.now() + this.config.segmentTtl * 1000
    );
    headers.Expires = expires.toUTCString();

    return headers;
  }

  /**
   * Init segments: very long TTL, immutable.
   *
   * CMAF init segments never change once created.
   * Cache for as long as possible.
   */
  private getInitSegmentHeaders(options?: {
    etag?: string;
    lastModified?: Date;
  }): CacheHeaders {
    const headers: CacheHeaders = {
      'Cache-Control': `public, max-age=${this.config.initSegmentTtl}, s-maxage=${this.config.initSegmentTtl}, immutable`,
      Vary: this.config.varyHeaders.join(', '),
      'X-SSAI-Cache-Policy': 'init-segment',
    };

    if (options?.etag) {
      headers.ETag = options.etag;
    }

    if (options?.lastModified) {
      headers['Last-Modified'] = options.lastModified.toUTCString();
    }

    const expires = new Date(
      Date.now() + this.config.initSegmentTtl * 1000
    );
    headers.Expires = expires.toUTCString();

    return headers;
  }

  /**
   * Default headers: no caching.
   */
  private getDefaultHeaders(): CacheHeaders {
    return {
      'Cache-Control': 'no-store, no-cache, must-revalidate',
      'X-SSAI-Cache-Policy': 'default',
    };
  }

  /**
   * Generate ETag from content.
   */
  generateETag(content: string | Buffer): string {
    const crypto = require('crypto');
    const hash = crypto
      .createHash('md5')
      .update(content)
      .digest('hex');
    return `"${hash}"`;
  }

  /**
   * Generate tokenized URL for CDN edge caching.
   *
   * Adds session token to URL to create unique cache keys
   * per session, preventing cross-user cache poisoning.
   */
  tokenizeUrl(
    baseUrl: string,
    sessionId: string,
    expiresAt?: Date
  ): string {
    const url = new URL(baseUrl);

    // Add session token
    url.searchParams.set('ssai_session', sessionId);

    // Add expiry if provided
    if (expiresAt) {
      url.searchParams.set('ssai_exp', Math.floor(expiresAt.getTime() / 1000).toString());
    }

    // Add signature for tampering prevention
    const signature = this.generateUrlSignature(
      url.pathname,
      sessionId,
      expiresAt
    );
    url.searchParams.set('ssai_sig', signature);

    return url.toString();
  }

  /**
   * Validate tokenized URL.
   */
  validateTokenizedUrl(
    url: string,
    expectedSessionId: string
  ): { valid: boolean; error?: string } {
    try {
      const parsed = new URL(url);
      const sessionId = parsed.searchParams.get('ssai_session');
      const expStr = parsed.searchParams.get('ssai_exp');
      const signature = parsed.searchParams.get('ssai_sig');

      // Check session ID
      if (sessionId !== expectedSessionId) {
        return { valid: false, error: 'Session ID mismatch' };
      }

      // Check expiry
      if (expStr) {
        const expiresAt = new Date(parseInt(expStr, 10) * 1000);
        if (expiresAt < new Date()) {
          return { valid: false, error: 'URL expired' };
        }
      }

      // Verify signature
      const expectedSignature = this.generateUrlSignature(
        parsed.pathname,
        sessionId,
        expStr ? new Date(parseInt(expStr, 10) * 1000) : undefined
      );

      if (signature !== expectedSignature) {
        return { valid: false, error: 'Invalid signature' };
      }

      return { valid: true };
    } catch {
      return { valid: false, error: 'Invalid URL format' };
    }
  }

  /**
   * Generate URL signature for tampering prevention.
   */
  private generateUrlSignature(
    path: string,
    sessionId: string,
    expiresAt?: Date
  ): string {
    const crypto = require('crypto');
    // SECURITY: Require SSAI_URL_SECRET in production (P1 Fix #1)
    const secret = process.env.SSAI_URL_SECRET;
    if (!secret && process.env.NODE_ENV === 'production') {
      throw new Error('SSAI_URL_SECRET must be set in production');
    }
    const signingSecret = secret || 'dev-url-secret-not-for-production';
    
    const data = [
      path,
      sessionId,
      expiresAt ? Math.floor(expiresAt.getTime() / 1000).toString() : '',
    ].join(':');

    return crypto
      .createHmac('sha256', signingSecret)
      .update(data)
      .digest('hex')
      .substring(0, 16); // Truncate for URL friendliness
  }

  /**
   * Check if resource should bypass cache (forced refresh).
   */
  shouldBypassCache(headers: Record<string, string | string[] | undefined>): boolean {
    const cacheControl = headers['cache-control'];
    const pragma = headers['pragma'];

    if (typeof cacheControl === 'string') {
      if (
        cacheControl.includes('no-cache') ||
        cacheControl.includes('no-store')
      ) {
        return true;
      }
    }

    if (pragma === 'no-cache') {
      return true;
    }

    return false;
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<CachePolicyConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Get current configuration.
   */
  getConfig(): CachePolicyConfig {
    return { ...this.config };
  }
}

export default CachePolicy;
