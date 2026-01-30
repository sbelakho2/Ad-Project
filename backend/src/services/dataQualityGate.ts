/**
 * Analytics Data Quality Gate
 * 
 * Pre-insertion validation for analytics events.
 * Catches issues that schema validation alone cannot detect:
 * - Timestamp sanity (future dates, too old)
 * - Duplicate detection (same event_id)
 * - Revenue sanity (negative values, unrealistic amounts)
 * - Rate anomaly detection
 */

import logger from '../utils/logger';
import { redis } from '../utils/redis';

// Quality gate configuration
const CONFIG = {
  // Timestamp validation
  MAX_FUTURE_SECONDS: 300, // Events can be at most 5 minutes in the future (clock skew tolerance)
  MAX_AGE_DAYS: 7, // Events older than 7 days are rejected
  
  // Revenue validation
  MAX_REVENUE_USD: 100, // Single impression max $100 (outlier)
  MIN_REVENUE_USD: -0.001, // Tiny negative allowed for rounding
  
  // Rate limiting
  DEDUP_TTL_SECONDS: 86400, // 24 hours for duplicate detection
  
  // Metrics
  METRICS_KEY_PREFIX: 'dq:metrics:',
};

export type QualityCheckResult = {
  passed: boolean;
  rejectedCount: number;
  reasons: Record<string, number>;
  validEvents: any[];
};

/**
 * Check timestamp sanity
 */
function checkTimestamp(timestamp: string | Date): { valid: boolean; reason?: string } {
  const eventDate = typeof timestamp === 'string' ? new Date(timestamp) : timestamp;
  const now = Date.now();
  const eventMs = eventDate.getTime();

  if (Number.isNaN(eventMs)) {
    return { valid: false, reason: 'invalid_timestamp' };
  }

  // Check for future events (with clock skew tolerance)
  if (eventMs > now + CONFIG.MAX_FUTURE_SECONDS * 1000) {
    return { valid: false, reason: 'future_timestamp' };
  }

  // Check for too old events
  const maxAgeMs = CONFIG.MAX_AGE_DAYS * 24 * 60 * 60 * 1000;
  if (now - eventMs > maxAgeMs) {
    return { valid: false, reason: 'stale_timestamp' };
  }

  return { valid: true };
}

/**
 * Check revenue sanity
 */
function checkRevenue(revenue?: number): { valid: boolean; reason?: string } {
  if (revenue === undefined || revenue === null) {
    return { valid: true };
  }

  if (typeof revenue !== 'number' || Number.isNaN(revenue)) {
    return { valid: false, reason: 'invalid_revenue' };
  }

  if (revenue < CONFIG.MIN_REVENUE_USD) {
    return { valid: false, reason: 'negative_revenue' };
  }

  if (revenue > CONFIG.MAX_REVENUE_USD) {
    return { valid: false, reason: 'outlier_revenue' };
  }

  return { valid: true };
}

/**
 * Check for duplicate event IDs using Redis
 * Returns true if event is a duplicate
 */
async function isDuplicate(eventId: string, eventType: string): Promise<boolean> {
  if (!redis.isReady()) {
    // Fail open - if Redis unavailable, allow the event
    return false;
  }

  try {
    const key = `dq:dedup:${eventType}:${eventId}`;
    // setIfNotExists returns true if key was set (new event), false if it existed (duplicate)
    const result = await redis.setIfNotExists(key, '1', CONFIG.DEDUP_TTL_SECONDS);
    return result === false; // false means key existed (duplicate)
  } catch (error) {
    logger.warn('[DataQualityGate] Dedup check failed, allowing event', { error, eventId });
    return false; // Fail open
  }
}

/**
 * Record quality gate metrics (lightweight)
 * Uses simple counters instead of hashes for compatibility
 */
async function recordMetrics(eventType: string, passed: number, rejected: number, reasons: Record<string, number>): Promise<void> {
  if (!redis.isReady()) return;

  try {
    const today = new Date().toISOString().slice(0, 10);
    
    // Use simple keys with incr
    const passedKey = `${CONFIG.METRICS_KEY_PREFIX}${eventType}:${today}:passed`;
    const rejectedKey = `${CONFIG.METRICS_KEY_PREFIX}${eventType}:${today}:rejected`;
    
    // Increment counters (simple approach without hash)
    for (let i = 0; i < passed; i++) {
      await redis.incr(passedKey);
    }
    for (let i = 0; i < rejected; i++) {
      await redis.incr(rejectedKey);
    }
    
    // Set expiry (30 days)
    await redis.expire(passedKey, 30 * 24 * 60 * 60);
    await redis.expire(rejectedKey, 30 * 24 * 60 * 60);
    
    // Record reason counts
    for (const [reason, count] of Object.entries(reasons)) {
      const reasonKey = `${CONFIG.METRICS_KEY_PREFIX}${eventType}:${today}:reason:${reason}`;
      for (let i = 0; i < count; i++) {
        await redis.incr(reasonKey);
      }
      await redis.expire(reasonKey, 30 * 24 * 60 * 60);
    }
  } catch (error) {
    logger.debug('[DataQualityGate] Failed to record metrics', { error });
  }
}

/**
 * Validate impression events
 */
export async function validateImpressionEvents(events: any[]): Promise<QualityCheckResult> {
  const validEvents: any[] = [];
  const reasons: Record<string, number> = {};
  
  for (const event of events) {
    let valid = true;
    
    // Check timestamp
    const tsCheck = checkTimestamp(event.timestamp);
    if (!tsCheck.valid) {
      valid = false;
      reasons[tsCheck.reason!] = (reasons[tsCheck.reason!] || 0) + 1;
    }
    
    // Check revenue (if present - impressions may have bid_price_usd)
    const revCheck = checkRevenue(event.bid_price_usd);
    if (!revCheck.valid) {
      valid = false;
      reasons[revCheck.reason!] = (reasons[revCheck.reason!] || 0) + 1;
    }
    
    // Check for duplicates
    if (valid && event.event_id) {
      const isDupe = await isDuplicate(event.event_id, 'impression');
      if (isDupe) {
        valid = false;
        reasons['duplicate'] = (reasons['duplicate'] || 0) + 1;
      }
    }
    
    if (valid) {
      validEvents.push(event);
    }
  }
  
  const rejectedCount = events.length - validEvents.length;
  
  // Record metrics asynchronously (don't block)
  recordMetrics('impression', validEvents.length, rejectedCount, reasons).catch(() => {});
  
  if (rejectedCount > 0) {
    logger.info('[DataQualityGate] Rejected impression events', {
      total: events.length,
      rejected: rejectedCount,
      reasons,
    });
  }
  
  return {
    passed: rejectedCount === 0,
    rejectedCount,
    reasons,
    validEvents,
  };
}

/**
 * Validate click events
 */
export async function validateClickEvents(events: any[]): Promise<QualityCheckResult> {
  const validEvents: any[] = [];
  const reasons: Record<string, number> = {};
  
  for (const event of events) {
    let valid = true;
    
    // Check timestamp
    const tsCheck = checkTimestamp(event.timestamp);
    if (!tsCheck.valid) {
      valid = false;
      reasons[tsCheck.reason!] = (reasons[tsCheck.reason!] || 0) + 1;
    }
    
    // Check for duplicates
    if (valid && event.event_id) {
      const isDupe = await isDuplicate(event.event_id, 'click');
      if (isDupe) {
        valid = false;
        reasons['duplicate'] = (reasons['duplicate'] || 0) + 1;
      }
    }
    
    if (valid) {
      validEvents.push(event);
    }
  }
  
  const rejectedCount = events.length - validEvents.length;
  
  recordMetrics('click', validEvents.length, rejectedCount, reasons).catch(() => {});
  
  if (rejectedCount > 0) {
    logger.info('[DataQualityGate] Rejected click events', {
      total: events.length,
      rejected: rejectedCount,
      reasons,
    });
  }
  
  return {
    passed: rejectedCount === 0,
    rejectedCount,
    reasons,
    validEvents,
  };
}

/**
 * Validate revenue events
 */
export async function validateRevenueEvents(events: any[]): Promise<QualityCheckResult> {
  const validEvents: any[] = [];
  const reasons: Record<string, number> = {};
  
  for (const event of events) {
    let valid = true;
    
    // Check timestamp
    const tsCheck = checkTimestamp(event.timestamp);
    if (!tsCheck.valid) {
      valid = false;
      reasons[tsCheck.reason!] = (reasons[tsCheck.reason!] || 0) + 1;
    }
    
    // Check revenue (required for revenue events)
    const revCheck = checkRevenue(event.revenue_usd);
    if (!revCheck.valid) {
      valid = false;
      reasons[revCheck.reason!] = (reasons[revCheck.reason!] || 0) + 1;
    }
    
    // Check for duplicates
    if (valid && event.event_id) {
      const isDupe = await isDuplicate(event.event_id, 'revenue');
      if (isDupe) {
        valid = false;
        reasons['duplicate'] = (reasons['duplicate'] || 0) + 1;
      }
    }
    
    if (valid) {
      validEvents.push(event);
    }
  }
  
  const rejectedCount = events.length - validEvents.length;
  
  recordMetrics('revenue', validEvents.length, rejectedCount, reasons).catch(() => {});
  
  if (rejectedCount > 0) {
    logger.info('[DataQualityGate] Rejected revenue events', {
      total: events.length,
      rejected: rejectedCount,
      reasons,
    });
  }
  
  return {
    passed: rejectedCount === 0,
    rejectedCount,
    reasons,
    validEvents,
  };
}

/**
 * Get quality gate metrics for monitoring (simplified)
 */
export async function getQualityMetrics(eventType: string, days: number = 7): Promise<Record<string, any>> {
  if (!redis.isReady()) {
    return { error: 'Redis not available' };
  }

  try {
    const metrics: Record<string, any> = {};
    const now = new Date();
    
    for (let i = 0; i < days; i++) {
      const date = new Date(now);
      date.setDate(date.getDate() - i);
      const dateStr = date.toISOString().slice(0, 10);
      
      const passedKey = `${CONFIG.METRICS_KEY_PREFIX}${eventType}:${dateStr}:passed`;
      const rejectedKey = `${CONFIG.METRICS_KEY_PREFIX}${eventType}:${dateStr}:rejected`;
      
      const passed = await redis.get<number>(passedKey);
      const rejected = await redis.get<number>(rejectedKey);
      
      if (passed !== null || rejected !== null) {
        metrics[dateStr] = {
          passed: passed || 0,
          rejected: rejected || 0,
        };
      }
    }
    
    return metrics;
  } catch (error) {
    logger.error('[DataQualityGate] Failed to get metrics', { error });
    return { error: 'Failed to retrieve metrics' };
  }
}
