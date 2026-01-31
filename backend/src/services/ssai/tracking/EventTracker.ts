/**
 * SSAI Event Tracker (test-facing)
 */

export interface EventTrackerConfig {
  requestTimeout: number;
  maxRetries: number;
  parallelRequests: number;
}

export interface PixelResult {
  success: boolean;
  status?: number;
  error?: string;
}

/**
 * SECURITY: Validate URL to prevent SSRF attacks
 * P0 Fix #12: Comprehensive URL validation for pixel firing
 */
function validatePixelUrl(url: string): { valid: boolean; reason?: string } {
  try {
    const parsed = new URL(url);
    
    // Only allow HTTP/HTTPS
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      return { valid: false, reason: 'Invalid protocol' };
    }
    
    const hostname = parsed.hostname.toLowerCase();
    
    // Block internal hostnames
    const blocked = [
      'localhost', '127.0.0.1', '::1', '[::1]', '0.0.0.0',
      '169.254.169.254', 'metadata.google.internal', '100.100.100.200',
      'kubernetes', 'kubernetes.default', 'docker', 'host.docker.internal',
    ];
    if (blocked.includes(hostname)) {
      return { valid: false, reason: 'Internal hostname not allowed' };
    }
    
    // Block internal TLDs
    const blockedSuffixes = ['.local', '.internal', '.localhost', '.lan', '.cluster.local'];
    if (blockedSuffixes.some(suffix => hostname.endsWith(suffix))) {
      return { valid: false, reason: 'Internal TLD not allowed' };
    }
    
    // Block private IP ranges
    const privatePatterns = [
      /^10\./, /^172\.(1[6-9]|2[0-9]|3[01])\./, /^192\.168\./, /^169\.254\./, /^127\./,
    ];
    if (privatePatterns.some(p => p.test(hostname))) {
      return { valid: false, reason: 'Private IP not allowed' };
    }
    
    return { valid: true };
  } catch {
    return { valid: false, reason: 'Invalid URL format' };
  }
}

export class EventTracker {
  private readonly config: EventTrackerConfig;

  constructor(config: EventTrackerConfig) {
    this.config = config;
  }

  calculateQuartile(progressPercent: number):
    | 'start'
    | 'firstQuartile'
    | 'midpoint'
    | 'thirdQuartile'
    | 'complete' {
    if (progressPercent >= 100) return 'complete';
    if (progressPercent >= 75) return 'thirdQuartile';
    if (progressPercent >= 50) return 'midpoint';
    if (progressPercent >= 25) return 'firstQuartile';
    return 'start';
  }

  async firePixel(url: string, macros: Record<string, string> = {}): Promise<PixelResult> {
    const expandedUrl = this.expandMacros(url, macros);

    // SECURITY: Validate URL before fetching (P0 Fix #12)
    const validation = validatePixelUrl(expandedUrl);
    if (!validation.valid) {
      return { success: false, error: `URL validation failed: ${validation.reason}` };
    }

    for (let attempt = 0; attempt <= this.config.maxRetries; attempt++) {
      try {
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), this.config.requestTimeout);

        const res = await fetch(expandedUrl, {
          method: 'GET',
          signal: controller.signal,
          // Avoid cached pixels in test + production.
          cache: 'no-store',
        } as any);

        clearTimeout(timeout);

        if (res?.ok) {
          return { success: true, status: 200 };
        }

        return {
          success: false,
          status: (res as any)?.status,
          error: 'non_ok_response',
        };
      } catch (err: any) {
        if (attempt >= this.config.maxRetries) {
          return { success: false, error: err?.message || 'request_failed' };
        }
      }
    }

    return { success: false, error: 'request_failed' };
  }

  async firePixelBatch(urls: string[], macros: Record<string, string> = {}): Promise<PixelResult[]> {
    const concurrency = Math.max(1, this.config.parallelRequests);
    const results: PixelResult[] = new Array(urls.length);

    let idx = 0;
    const worker = async () => {
      while (true) {
        const current = idx++;
        if (current >= urls.length) return;
        results[current] = await this.firePixel(urls[current], macros);
      }
    };

    const workers = Array.from({ length: Math.min(concurrency, urls.length) }).map(() => worker());
    await Promise.all(workers);

    return results;
  }

  private expandMacros(url: string, macros: Record<string, string>): string {
    let out = url;
    for (const [key, value] of Object.entries(macros)) {
      out = out.split(`[${key}]`).join(encodeURIComponent(value));
    }
    return out;
  }
}
