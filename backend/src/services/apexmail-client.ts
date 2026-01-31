/**
 * ApexMail Client
 * 
 * HTTP client for interacting with ApexMail API (via Control Plane).
 * This provides the backend with a clean interface to send emails via ApexMail.
 */

import logger from '../utils/logger';

// ApexMail API endpoint (via Control Plane on App Server or localhost on Test Server)
const APEXMAIL_BASE_URL = process.env.APEXMAIL_URL || 'http://localhost:4000/api/apexmail';
const APEXMAIL_API_KEY = process.env.APEXMAIL_API_KEY;

export interface ApexMailSendOptions {
  to: string | string[];
  from?: string;
  fromName?: string;
  replyTo?: string;
  subject: string;
  html?: string;
  text?: string;
  headers?: Record<string, string>;
  category?: 'transactional' | 'marketing' | 'dunning' | 'notification';
  priority?: number;
  entityType?: string;
  entityId?: string;
  trackOpens?: boolean;
  trackClicks?: boolean;
}

export interface ApexMailSendResult {
  success: boolean;
  emailId?: string;
  messageId?: string;
  error?: string;
}

export interface ApexMailQueueStats {
  success: boolean;
  smtp: string;
  queue: {
    queued: number;
    sending: number;
    deferred: number;
    failed: number;
    sent24h: number;
  };
}

/**
 * ApexMail HTTP Client
 */
export class ApexMailClient {
  private baseUrl: string;
  private apiKey?: string;

  constructor(baseUrl?: string, apiKey?: string) {
    this.baseUrl = baseUrl || APEXMAIL_BASE_URL;
    this.apiKey = apiKey || APEXMAIL_API_KEY;
  }

  /**
   * Send an email via ApexMail
   */
  async send(options: ApexMailSendOptions): Promise<ApexMailSendResult> {
    try {
      const headers: Record<string, string> = {
        'Content-Type': 'application/json',
      };
      if (this.apiKey) {
        headers['X-ApexMail-Key'] = this.apiKey;
      }

      const response = await fetch(`${this.baseUrl}/send`, {
        method: 'POST',
        headers,
        body: JSON.stringify({
          to: Array.isArray(options.to) ? options.to[0] : options.to,
          from: options.from || process.env.EMAIL_FROM || 'no-reply@apexmediation.ee',
          fromName: options.fromName || process.env.EMAIL_FROM_NAME || 'ApexMediation',
          replyTo: options.replyTo,
          subject: options.subject,
          html: options.html || '',
          text: options.text || (options.html ? options.html.replace(/<[^>]*>/g, ' ').replace(/\\s+/g, ' ').trim() : ''),
          headers: options.headers,
          category: options.category || 'transactional',
          priority: options.priority || 1,
          entityType: options.entityType,
          entityId: options.entityId,
          trackOpens: options.trackOpens,
          trackClicks: options.trackClicks,
        }),
      });

      if (response.ok) {
        const result = (await response.json()) as { emailId?: string; messageId?: string };
        return {
          success: true,
          emailId: result.emailId,
          messageId: result.messageId,
        };
      } else {
        const error = await response.text();
        logger.error('ApexMail send failed', { status: response.status, error });
        return { success: false, error: `HTTP ${response.status}: ${error}` };
      }
    } catch (error) {
      const msg = error instanceof Error ? error.message : 'Unknown error';
      logger.error('ApexMail API error', { error: msg });
      return { success: false, error: msg };
    }
  }

  /**
   * Check ApexMail health/status
   */
  async health(): Promise<ApexMailQueueStats | null> {
    try {
      const headers: Record<string, string> = {};
      if (this.apiKey) {
        headers['X-ApexMail-Key'] = this.apiKey;
      }

      const response = await fetch(`${this.baseUrl}/health`, { headers });
      if (response.ok) {
        return (await response.json()) as ApexMailQueueStats;
      }
      return null;
    } catch {
      return null;
    }
  }

  /**
   * Check if ApexMail is configured and available
   */
  isConfigured(): boolean {
    return !!(this.apiKey || process.env.SMTP_HOST);
  }
}

// Singleton instance
let instance: ApexMailClient | null = null;

export function getApexMailClient(): ApexMailClient {
  if (!instance) {
    instance = new ApexMailClient();
  }
  return instance;
}

export default ApexMailClient;
