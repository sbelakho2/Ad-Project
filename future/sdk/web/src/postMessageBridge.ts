/**
 * PostMessageBridge - CSP-Compliant Cross-Origin Communication
 *
 * Provides secure, CSP-compliant postMessage communication between
 * ad iframe and parent publisher page.
 *
 * Features:
 * - Strict origin validation
 * - Request/response pattern with timeouts
 * - Message namespace isolation
 * - No eval or inline scripts (CSP-compliant)
 * - Structured clone compatible payloads
 */

/**
 * Configuration for the PostMessageBridge
 */
export interface PostMessageConfig {
  /** List of allowed origins (exact match or wildcard patterns) */
  allowedOrigins: string[];
  /** Target window for sending messages (e.g., parent, iframe contentWindow) */
  targetWindow?: Window | null;
  /** Callback for incoming messages */
  onMessage?: (message: PostMessagePayload) => void;
  /** Timeout for request/response pattern (ms) */
  timeout?: number;
  /** Enable debug logging */
  enableLogging?: boolean;
}

/**
 * Message payload structure
 */
export interface PostMessagePayload {
  /** Message type identifier */
  type: string;
  /** Unique message ID (namespaced) */
  id: string;
  /** Message timestamp */
  timestamp: number;
  /** Message data (must be structured-clone compatible) */
  data: unknown;
}

/**
 * Pending request tracking
 */
interface PendingRequest {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timeoutId: ReturnType<typeof setTimeout>;
}

/**
 * Statistics for monitoring bridge usage
 */
export interface PostMessageStats {
  messagesSent: number;
  messagesReceived: number;
  messagesRejected: number;
  requestsCompleted: number;
  requestsTimedOut: number;
  errors: number;
}

/**
 * CSP-compliant PostMessage bridge for cross-origin communication
 */
export class PostMessageBridge {
  private readonly config: Required<PostMessageConfig>;
  private readonly pendingRequests: Map<string, PendingRequest> = new Map();
  private messageHandler: ((event: MessageEvent) => void) | null = null;
  private readonly NAMESPACE = 'apex-mediation';
  private stats: PostMessageStats = {
    messagesSent: 0,
    messagesReceived: 0,
    messagesRejected: 0,
    requestsCompleted: 0,
    requestsTimedOut: 0,
    errors: 0,
  };

  constructor(config: PostMessageConfig) {
    this.config = {
      allowedOrigins: config.allowedOrigins,
      targetWindow: config.targetWindow ?? null,
      onMessage: config.onMessage ?? (() => {}),
      timeout: config.timeout ?? 5000,
      enableLogging: config.enableLogging ?? false,
    };
  }

  /**
   * Check if an origin is allowed
   */
  isOriginAllowed(origin: string): boolean {
    if (!origin || origin === 'null') {
      return false;
    }

    return this.config.allowedOrigins.some((allowed) => {
      // Wildcard matches any origin
      if (allowed === '*') {
        return true;
      }

      // Subdomain wildcard (e.g., *.example.com)
      if (allowed.startsWith('*.')) {
        const domain = allowed.slice(2);
        const originWithoutProtocol = origin.replace(/^https?:\/\//, '');
        return (
          originWithoutProtocol.endsWith(domain) ||
          originWithoutProtocol.endsWith('.' + domain)
        );
      }

      // Exact match
      return allowed === origin;
    });
  }

  /**
   * Start listening for messages
   */
  start(): void {
    if (this.messageHandler) {
      // Already started, idempotent
      return;
    }

    this.messageHandler = (event: MessageEvent) => {
      this.handleMessage(event);
    };

    if (typeof window !== 'undefined') {
      window.addEventListener('message', this.messageHandler);
      this.log('PostMessageBridge started');
    }
  }

  /**
   * Stop listening and clean up
   */
  stop(): void {
    if (this.messageHandler && typeof window !== 'undefined') {
      window.removeEventListener('message', this.messageHandler);
      this.messageHandler = null;
    }

    // Reject all pending requests
    this.pendingRequests.forEach((pending, id) => {
      clearTimeout(pending.timeoutId);
      pending.reject(new Error('Bridge stopped'));
      this.log(`Cancelled pending request: ${id}`);
    });
    this.pendingRequests.clear();

    this.log('PostMessageBridge stopped');
  }

  /**
   * Send a message to the target window
   */
  send(type: string, data: unknown): string {
    const id = this.generateId();
    const payload: PostMessagePayload = {
      type,
      id,
      timestamp: Date.now(),
      data,
    };

    this.postToTarget(payload);
    this.stats.messagesSent++;

    return id;
  }

  /**
   * Send a request and wait for response
   */
  async request<T>(type: string, data: unknown): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const id = this.send(type, data);

      const timeoutId = setTimeout(() => {
        this.pendingRequests.delete(id);
        this.stats.requestsTimedOut++;
        reject(new Error('Request timed out'));
      }, this.config.timeout);

      this.pendingRequests.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timeoutId,
      });
    });
  }

  /**
   * Send a response to a request
   */
  respond(requestId: string, data: unknown): void {
    const payload: PostMessagePayload = {
      type: 'response',
      id: requestId,
      timestamp: Date.now(),
      data,
    };

    this.postToTarget(payload);
    this.stats.messagesSent++;
  }

  /**
   * Send an error response to a request
   */
  respondError(requestId: string, error: string): void {
    const payload: PostMessagePayload = {
      type: 'error',
      id: requestId,
      timestamp: Date.now(),
      data: error,
    };

    this.postToTarget(payload);
    this.stats.messagesSent++;
    this.stats.errors++;
  }

  /**
   * Get bridge statistics
   */
  getStats(): PostMessageStats {
    return { ...this.stats };
  }

  /**
   * Reset statistics
   */
  resetStats(): void {
    this.stats = {
      messagesSent: 0,
      messagesReceived: 0,
      messagesRejected: 0,
      requestsCompleted: 0,
      requestsTimedOut: 0,
      errors: 0,
    };
  }

  private handleMessage(event: MessageEvent): void {
    // Validate origin
    if (!this.isOriginAllowed(event.origin)) {
      this.stats.messagesRejected++;
      if (this.config.enableLogging) {
        console.warn(
          `[PostMessageBridge] Rejected message from untrusted origin: ${event.origin}`
        );
      }
      return;
    }

    const data = event.data;

    // Validate payload structure
    if (!this.isValidPayload(data)) {
      // Not our message, ignore silently
      return;
    }

    this.stats.messagesReceived++;

    // Handle response/error for pending requests
    if (data.type === 'response' || data.type === 'error') {
      const pending = this.pendingRequests.get(data.id);
      if (pending) {
        clearTimeout(pending.timeoutId);
        this.pendingRequests.delete(data.id);

        if (data.type === 'error') {
          this.stats.errors++;
          pending.reject(new Error(String(data.data)));
        } else {
          this.stats.requestsCompleted++;
          pending.resolve(data.data);
        }
      }
      return;
    }

    // Forward to message handler
    try {
      this.config.onMessage(data);
    } catch (error) {
      this.stats.errors++;
      console.error('[PostMessageBridge] Error in message handler:', error);
    }
  }

  private isValidPayload(data: unknown): data is PostMessagePayload {
    if (typeof data !== 'object' || data === null) {
      return false;
    }

    const payload = data as Record<string, unknown>;
    return (
      typeof payload.type === 'string' &&
      typeof payload.id === 'string' &&
      typeof payload.timestamp === 'number' &&
      payload.id.startsWith(this.NAMESPACE)
    );
  }

  private postToTarget(payload: PostMessagePayload): void {
    const target = this.config.targetWindow;
    if (!target) {
      this.log('No target window configured');
      return;
    }

    // Use first allowed origin as target, or '*' for wildcard
    const targetOrigin =
      this.config.allowedOrigins[0] === '*'
        ? '*'
        : this.config.allowedOrigins[0];

    try {
      target.postMessage(payload, targetOrigin);
      this.log(`Sent message: ${payload.type} -> ${targetOrigin}`);
    } catch (error) {
      this.stats.errors++;
      console.error('[PostMessageBridge] Failed to post message:', error);
    }
  }

  private generateId(): string {
    const random = Math.random().toString(36).slice(2, 11);
    return `${this.NAMESPACE}-${Date.now()}-${random}`;
  }

  private log(message: string): void {
    if (this.config.enableLogging) {
      console.log(`[PostMessageBridge] ${message}`);
    }
  }
}

/**
 * Message types for SDK communication
 */
export const MessageTypes = {
  // Initialization
  INIT: 'init',
  INIT_RESPONSE: 'init.response',

  // Ad lifecycle
  AD_REQUEST: 'ad.request',
  AD_RESPONSE: 'ad.response',
  AD_LOADED: 'ad.loaded',
  AD_FAILED: 'ad.failed',
  AD_IMPRESSION: 'ad.impression',
  AD_CLICK: 'ad.click',
  AD_COMPLETE: 'ad.complete',

  // Consent
  CONSENT_REQUEST: 'consent.request',
  CONSENT_RESPONSE: 'consent.response',
  CONSENT_UPDATE: 'consent.update',

  // MRAID
  MRAID_READY: 'mraid.ready',
  MRAID_EXPAND: 'mraid.expand',
  MRAID_CLOSE: 'mraid.close',
  MRAID_RESIZE: 'mraid.resize',

  // Viewability
  VIEWABILITY_UPDATE: 'viewability.update',
  VIEWABILITY_MRC50: 'viewability.mrc50',

  // Error
  ERROR: 'error',
} as const;

export type MessageType = (typeof MessageTypes)[keyof typeof MessageTypes];

/**
 * Helper function to create a publisher-side bridge
 */
export function createPublisherBridge(config: {
  adIframe: HTMLIFrameElement;
  allowedOrigins: string[];
  onAdEvent?: (event: PostMessagePayload) => void;
  timeout?: number;
}): PostMessageBridge {
  return new PostMessageBridge({
    allowedOrigins: config.allowedOrigins,
    targetWindow: config.adIframe.contentWindow,
    onMessage: config.onAdEvent,
    timeout: config.timeout,
  });
}

/**
 * Helper function to create an ad-side bridge (inside iframe)
 */
export function createAdBridge(config: {
  publisherOrigin: string;
  onPublisherMessage?: (message: PostMessagePayload) => void;
  timeout?: number;
}): PostMessageBridge {
  return new PostMessageBridge({
    allowedOrigins: [config.publisherOrigin],
    targetWindow: typeof window !== 'undefined' ? window.parent : null,
    onMessage: config.onPublisherMessage,
    timeout: config.timeout,
  });
}
