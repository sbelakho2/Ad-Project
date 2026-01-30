import crypto from 'crypto';
import { logger } from '../utils/logger';
import { Counter, Gauge } from 'prom-client';

export interface AlertOptions {
  severity: 'low' | 'medium' | 'high' | 'critical';
  title: string;
  message: string;
  metadata?: Record<string, any>;
  component: string;
  /** Override default deduplication window (ms). Set to 0 to disable dedup for this alert. */
  deduplicationWindowMs?: number;
}

interface AlertCacheEntry {
  fingerprint: string;
  firstSeen: number;
  lastSeen: number;
  count: number;
  options: AlertOptions;
}

// Default deduplication window: 5 minutes
const DEFAULT_DEDUP_WINDOW_MS = parseInt(process.env.ALERT_DEDUP_WINDOW_MS || '300000', 10);

// Cache cleanup interval: 1 minute
const CACHE_CLEANUP_INTERVAL_MS = 60000;

// In-memory alert cache for deduplication
const alertCache = new Map<string, AlertCacheEntry>();

// Prometheus metrics for alert deduplication
const alertsSentTotal = new Counter({
  name: 'alerts_sent_total',
  help: 'Total number of alerts actually sent (after deduplication)',
  labelNames: ['severity', 'component'],
});

const alertsSuppressedTotal = new Counter({
  name: 'alerts_suppressed_total',
  help: 'Total number of alerts suppressed by deduplication',
  labelNames: ['severity', 'component'],
});

const alertCacheSize = new Gauge({
  name: 'alert_cache_size',
  help: 'Current number of alerts in deduplication cache',
});

/**
 * Generate a fingerprint for an alert to identify duplicates.
 * Uses: severity + component + title (message/metadata may vary slightly for same issue)
 */
function generateAlertFingerprint(options: AlertOptions): string {
  const data = `${options.severity}:${options.component}:${options.title}`;
  return crypto.createHash('sha256').update(data).digest('hex').substring(0, 16);
}

/**
 * Check if alert should be deduplicated (suppressed)
 * Returns true if alert is a duplicate and should NOT be sent
 */
function shouldSuppressAlert(fingerprint: string, options: AlertOptions): boolean {
  const dedupWindow = options.deduplicationWindowMs ?? DEFAULT_DEDUP_WINDOW_MS;
  
  // Deduplication disabled for this alert
  if (dedupWindow === 0) {
    return false;
  }
  
  const now = Date.now();
  const cached = alertCache.get(fingerprint);
  
  if (cached) {
    const timeSinceFirst = now - cached.firstSeen;
    
    if (timeSinceFirst < dedupWindow) {
      // Update the cache entry
      cached.lastSeen = now;
      cached.count++;
      return true; // Suppress this duplicate
    }
    
    // Window expired, remove old entry
    alertCache.delete(fingerprint);
  }
  
  // New alert, add to cache
  alertCache.set(fingerprint, {
    fingerprint,
    firstSeen: now,
    lastSeen: now,
    count: 1,
    options,
  });
  
  alertCacheSize.set(alertCache.size);
  return false; // Don't suppress, send the alert
}

/**
 * Cleanup expired entries from the cache
 */
function cleanupAlertCache(): void {
  const now = Date.now();
  let cleaned = 0;
  
  for (const [fingerprint, entry] of alertCache.entries()) {
    const dedupWindow = entry.options.deduplicationWindowMs ?? DEFAULT_DEDUP_WINDOW_MS;
    const timeSinceFirst = now - entry.firstSeen;
    
    if (timeSinceFirst >= dedupWindow) {
      // Log summary of suppressed alerts before removing
      if (entry.count > 1) {
        logger.info('Alert deduplication summary', {
          fingerprint,
          title: entry.options.title,
          component: entry.options.component,
          severity: entry.options.severity,
          totalOccurrences: entry.count,
          suppressedCount: entry.count - 1,
          windowMs: dedupWindow,
        });
      }
      alertCache.delete(fingerprint);
      cleaned++;
    }
  }
  
  if (cleaned > 0) {
    alertCacheSize.set(alertCache.size);
  }
}

// Start cache cleanup interval
setInterval(cleanupAlertCache, CACHE_CLEANUP_INTERVAL_MS).unref();

/**
 * Send an alert with deduplication.
 * Duplicate alerts within the deduplication window are suppressed.
 */
export async function sendAlert(options: AlertOptions): Promise<void> {
  const { severity, title, message, metadata, component } = options;
  
  // Generate fingerprint for deduplication
  const fingerprint = generateAlertFingerprint(options);
  
  // Check if this alert should be suppressed
  if (shouldSuppressAlert(fingerprint, options)) {
    alertsSuppressedTotal.inc({ severity, component });
    logger.debug('Alert suppressed by deduplication', {
      fingerprint,
      title,
      component,
      severity,
    });
    return;
  }
  
  // Track sent alert
  alertsSentTotal.inc({ severity, component });

  // Always log the alert
  const logMethod = severity === 'critical' || severity === 'high' ? 'error' : 'warn';
  
  if (logger && typeof (logger as any)[logMethod] === 'function') {
    (logger as any)[logMethod](`ALERT [${component}]: ${title}`, { message, severity, metadata });
  } else {
    console.error(`ALERT [${component}]: ${title}`, { message, severity, metadata });
  }

  // Slack Webhook integration
  const slackWebhookUrl = process.env.SLACK_WEBHOOK_URL;
  if (slackWebhookUrl) {
    try {
      await fetch(slackWebhookUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          text: `*${severity.toUpperCase()} ALERT* [${component}]: ${title}\n${message}\n\`\`\`${JSON.stringify(metadata, null, 2)}\`\`\``,
        }),
      });
    } catch (error) {
      logger.warn('Failed to send Slack alert', { error });
    }
  }

  // PagerDuty integration (stub)
  const pdRoutingKey = process.env.PAGERDUTY_ROUTING_KEY;
  if (pdRoutingKey && (severity === 'high' || severity === 'critical')) {
    try {
      const pdSeverity =
        severity === 'critical'
          ? 'critical'
          : severity === 'high'
          ? 'error'
          : severity === 'medium'
          ? 'warning'
          : 'info';

      await fetch('https://events.pagerduty.com/v2/enqueue', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          routing_key: pdRoutingKey,
          event_action: 'trigger',
          payload: {
            summary: `[${component}] ${title}`,
            source: component,
            severity: pdSeverity,
            timestamp: new Date().toISOString(),
            custom_details: {
              message,
              metadata,
            },
          },
        }),
      });
    } catch (error) {
      logger.warn('Failed to send PagerDuty alert', { error });
    }
  }
}

/**
 * Get current alert cache statistics for monitoring/debugging
 */
export function getAlertCacheStats(): {
  size: number;
  entries: Array<{
    fingerprint: string;
    title: string;
    component: string;
    count: number;
    firstSeen: Date;
    lastSeen: Date;
  }>;
} {
  const entries = Array.from(alertCache.values()).map(entry => ({
    fingerprint: entry.fingerprint,
    title: entry.options.title,
    component: entry.options.component,
    count: entry.count,
    firstSeen: new Date(entry.firstSeen),
    lastSeen: new Date(entry.lastSeen),
  }));
  
  return {
    size: alertCache.size,
    entries,
  };
}

/**
 * Clear the alert cache (useful for testing)
 */
export function clearAlertCache(): void {
  alertCache.clear();
  alertCacheSize.set(0);
}

/**
 * Force an alert through without deduplication (for critical manual alerts)
 */
export async function sendAlertForced(options: AlertOptions): Promise<void> {
  return sendAlert({ ...options, deduplicationWindowMs: 0 });
}

export default {
  sendAlert,
  sendAlertForced,
  getAlertCacheStats,
  clearAlertCache,
};
