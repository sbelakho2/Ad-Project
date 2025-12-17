/**
 * NetworkWarmer - HTTP Preconnect & DNS Cache for Web SDK
 * 
 * Optimizes network performance by:
 * - Using Resource Hints (preconnect, dns-prefetch) for browser optimization
 * - Warming up connections with HEAD requests
 * - Caching DNS resolution times for metrics
 * - Supporting both browser and Node.js environments
 */

export interface NetworkWarmerConfig {
  /** URLs to preconnect/warm up */
  endpoints: string[];
  /** Whether to use browser resource hints (preconnect, dns-prefetch) */
  useResourceHints: boolean;
  /** Connection warmup timeout in milliseconds */
  warmupTimeout: number;
  /** Enable debug logging */
  debug: boolean;
}

export interface WarmupMetrics {
  totalEndpoints: number;
  successfulConnections: number;
  failedConnections: number;
  averageLatencyMs: number;
  warmupDurationMs: number;
  lastWarmupTimestamp: number;
}

interface ConnectionResult {
  endpoint: string;
  success: boolean;
  latencyMs: number;
  error?: string;
}

const DEFAULT_ENDPOINTS = [
  'https://auction.rivalapexmediation.ee',
  'https://api.rivalapexmediation.ee',
  'https://telemetry.rivalapexmediation.ee',
  'https://cdn.rivalapexmediation.ee',
];

const DEFAULT_CONFIG: NetworkWarmerConfig = {
  endpoints: DEFAULT_ENDPOINTS,
  useResourceHints: true,
  warmupTimeout: 5000,
  debug: false,
};

/**
 * NetworkWarmer handles HTTP preconnection and DNS prefetching
 * to reduce latency for ad auction requests.
 */
export class NetworkWarmer {
  private config: NetworkWarmerConfig;
  private metrics: WarmupMetrics;
  private connectionResults: Map<string, ConnectionResult>;
  private resourceHintElements: HTMLLinkElement[];
  private isWarmedUp: boolean;

  constructor(config: Partial<NetworkWarmerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.connectionResults = new Map();
    this.resourceHintElements = [];
    this.isWarmedUp = false;
    this.metrics = {
      totalEndpoints: 0,
      successfulConnections: 0,
      failedConnections: 0,
      averageLatencyMs: 0,
      warmupDurationMs: 0,
      lastWarmupTimestamp: 0,
    };
  }

  /**
   * Warms up connections to all configured endpoints.
   * Uses resource hints in browser, direct connections otherwise.
   */
  async warmup(): Promise<WarmupMetrics> {
    const startTime = performance.now();
    const endpoints = this.config.endpoints;

    this.log(`Starting warmup for ${endpoints.length} endpoints`);

    // Add resource hints in browser environment
    if (this.isBrowser() && this.config.useResourceHints) {
      this.addResourceHints(endpoints);
    }

    // Perform actual connection warmup
    const results = await Promise.allSettled(
      endpoints.map(endpoint => this.warmupConnection(endpoint))
    );

    // Process results
    let totalLatency = 0;
    let successCount = 0;
    let failCount = 0;

    results.forEach((result, index) => {
      const endpoint = endpoints[index];
      if (result.status === 'fulfilled') {
        const connResult = result.value;
        this.connectionResults.set(endpoint, connResult);
        if (connResult.success) {
          successCount++;
          totalLatency += connResult.latencyMs;
        } else {
          failCount++;
        }
      } else {
        failCount++;
        this.connectionResults.set(endpoint, {
          endpoint,
          success: false,
          latencyMs: 0,
          error: result.reason?.message || 'Unknown error',
        });
      }
    });

    const endTime = performance.now();

    this.metrics = {
      totalEndpoints: endpoints.length,
      successfulConnections: successCount,
      failedConnections: failCount,
      averageLatencyMs: successCount > 0 ? totalLatency / successCount : 0,
      warmupDurationMs: endTime - startTime,
      lastWarmupTimestamp: Date.now(),
    };

    this.isWarmedUp = true;

    this.log(`Warmup complete: ${successCount}/${endpoints.length} connections successful`);
    this.log(`Average latency: ${this.metrics.averageLatencyMs.toFixed(2)}ms`);

    return this.metrics;
  }

  /**
   * Warms up a single connection using a HEAD request.
   */
  private async warmupConnection(endpoint: string): Promise<ConnectionResult> {
    const startTime = performance.now();

    const controller = new AbortController();
    let timeoutId: ReturnType<typeof setTimeout> | null = null;

    try {
      timeoutId = setTimeout(() => controller.abort(), this.config.warmupTimeout);

      // Use HEAD request to minimize data transfer while still establishing connection
      const response = await fetch(endpoint, {
        method: 'HEAD',
        mode: 'no-cors', // Allow cross-origin without CORS headers for warmup
        signal: controller.signal,
        cache: 'no-store',
        credentials: 'omit',
      });

      if (timeoutId) {
        clearTimeout(timeoutId);
        timeoutId = null;
      }

      const latencyMs = performance.now() - startTime;

      return {
        endpoint,
        success: true,
        latencyMs,
      };
    } catch (error) {
      const latencyMs = performance.now() - startTime;
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';

      // AbortError means timeout
      if (errorMessage.includes('abort')) {
        return {
          endpoint,
          success: false,
          latencyMs,
          error: 'Connection timeout',
        };
      }

      // For no-cors mode, network errors are expected but connection is still warmed
      // The browser will have established TCP/TLS connection even if we can't read response
      if (errorMessage.includes('NetworkError') || errorMessage.includes('TypeError')) {
        return {
          endpoint,
          success: true, // Connection was attempted, warming happened
          latencyMs,
        };
      }

      return {
        endpoint,
        success: false,
        latencyMs,
        error: errorMessage,
      };
    } finally {
      if (timeoutId) {
        clearTimeout(timeoutId);
      }
    }
  }

  /**
   * Adds preconnect and dns-prefetch resource hints to the document head.
   */
  private addResourceHints(endpoints: string[]): void {
    if (!this.isBrowser()) return;

    // Remove any existing hints we added
    this.removeResourceHints();

    endpoints.forEach(endpoint => {
      try {
        const url = new URL(endpoint);
        const origin = url.origin;

        // Add dns-prefetch hint
        const dnsPrefetch = document.createElement('link');
        dnsPrefetch.rel = 'dns-prefetch';
        dnsPrefetch.href = origin;
        document.head.appendChild(dnsPrefetch);
        this.resourceHintElements.push(dnsPrefetch);

        // Add preconnect hint
        const preconnect = document.createElement('link');
        preconnect.rel = 'preconnect';
        preconnect.href = origin;
        preconnect.crossOrigin = 'anonymous';
        document.head.appendChild(preconnect);
        this.resourceHintElements.push(preconnect);

        this.log(`Added resource hints for ${origin}`);
      } catch (error) {
        this.log(`Failed to add resource hints for ${endpoint}: ${error}`);
      }
    });
  }

  /**
   * Removes previously added resource hints.
   */
  private removeResourceHints(): void {
    this.resourceHintElements.forEach(element => {
      try {
        element.parentNode?.removeChild(element);
      } catch {
        // Element may already be removed
      }
    });
    this.resourceHintElements = [];
  }

  /**
   * Preconnects to a specific endpoint on-demand.
   */
  async preconnect(endpoint: string): Promise<ConnectionResult> {
    // Add resource hint if in browser
    if (this.isBrowser() && this.config.useResourceHints) {
      this.addResourceHints([endpoint]);
    }

    const result = await this.warmupConnection(endpoint);
    this.connectionResults.set(endpoint, result);
    return result;
  }

  /**
   * Gets the connection result for a specific endpoint.
   */
  getConnectionResult(endpoint: string): ConnectionResult | undefined {
    return this.connectionResults.get(endpoint);
  }

  /**
   * Gets all connection results.
   */
  getAllConnectionResults(): Map<string, ConnectionResult> {
    return new Map(this.connectionResults);
  }

  /**
   * Gets the current warmup metrics.
   */
  getMetrics(): WarmupMetrics {
    return { ...this.metrics };
  }

  /**
   * Checks if warmup has been performed.
   */
  isReady(): boolean {
    return this.isWarmedUp;
  }

  /**
   * Resets the warmer state and removes resource hints.
   */
  reset(): void {
    this.removeResourceHints();
    this.connectionResults.clear();
    this.isWarmedUp = false;
    this.metrics = {
      totalEndpoints: 0,
      successfulConnections: 0,
      failedConnections: 0,
      averageLatencyMs: 0,
      warmupDurationMs: 0,
      lastWarmupTimestamp: 0,
    };
    this.log('NetworkWarmer reset');
  }

  /**
   * Updates the endpoints configuration and optionally re-warms.
   */
  async updateEndpoints(endpoints: string[], rewarm: boolean = false): Promise<void> {
    this.config.endpoints = endpoints;
    if (rewarm) {
      await this.warmup();
    }
  }

  /**
   * Checks if running in a browser environment.
   */
  private isBrowser(): boolean {
    return typeof window !== 'undefined' && typeof document !== 'undefined';
  }

  /**
   * Logs a message if debug is enabled.
   */
  private log(message: string): void {
    if (this.config.debug) {
      console.log(`[NetworkWarmer] ${message}`);
    }
  }

  /**
   * Cleans up resources when the warmer is no longer needed.
   */
  destroy(): void {
    this.removeResourceHints();
    this.connectionResults.clear();
    this.isWarmedUp = false;
    this.log('NetworkWarmer destroyed');
  }
}

// Singleton instance for convenient access
let defaultInstance: NetworkWarmer | null = null;

/**
 * Gets or creates the default NetworkWarmer instance.
 */
export function getNetworkWarmer(config?: Partial<NetworkWarmerConfig>): NetworkWarmer {
  if (!defaultInstance) {
    defaultInstance = new NetworkWarmer(config);
  }
  return defaultInstance;
}

/**
 * Resets the default NetworkWarmer instance.
 */
export function resetNetworkWarmer(): void {
  if (defaultInstance) {
    defaultInstance.destroy();
    defaultInstance = null;
  }
}

export default NetworkWarmer;
