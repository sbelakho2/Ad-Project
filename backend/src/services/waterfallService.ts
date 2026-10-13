/**
 * Waterfall Mediation Service
 * 
 * Implements priority-based cascading waterfall when S2S bidding fails.
 * Falls back through adapters by priority with exponential backoff retry logic.
 */

import logger from '../utils/logger';
import { OpenRTBBidRequest } from '../types/openrtb.types';
import { executeAuction, getAdapterConfig, AuctionResult } from './openrtbEngine';

// ========================================
// Waterfall Configuration
// ========================================

interface WaterfallConfig {
  /** Maximum number of waterfall attempts */
  maxAttempts: number;
  /** Initial retry delay in milliseconds */
  initialRetryDelay: number;
  /** Maximum retry delay in milliseconds */
  maxRetryDelay: number;
  /** Exponential backoff multiplier */
  backoffMultiplier: number;
  /** Whether to enable waterfall fallback */
  enabled: boolean;
}

const DEFAULT_WATERFALL_CONFIG: WaterfallConfig = {
  maxAttempts: 15,
  initialRetryDelay: 20,
  maxRetryDelay: 100,
  backoffMultiplier: 1.5,
  enabled: true,
};

interface WaterfallAttempt {
  attemptNumber: number;
  adaptersQueried: string[];
  result: AuctionResult;
  delayMs: number;
  timestamp: string;
}

export interface WaterfallResult {
  success: boolean;
  finalResult: AuctionResult;
  attempts: WaterfallAttempt[];
  totalDuration: number;
  fallbackUsed: boolean;
}

// ========================================
// Waterfall Execution
// ========================================

/**
 * Execute auction with waterfall fallback
 * Tries S2S auction first, falls back through adapters by priority on failure
 */
export async function executeWithWaterfall(
  request: OpenRTBBidRequest,
  config: Partial<WaterfallConfig> = {}
): Promise<WaterfallResult> {
  const waterfallConfig = { ...DEFAULT_WATERFALL_CONFIG, ...config };
  const startTime = Date.now();
  const attempts: WaterfallAttempt[] = [];

  logger.info('Starting waterfall mediation', {
    requestId: request.id,
    maxAttempts: waterfallConfig.maxAttempts,
    enabled: waterfallConfig.enabled,
  });

  // First attempt: Full S2S auction with all adapters
  const firstAttempt = await executeAuction(request);
  
  attempts.push({
    attemptNumber: 1,
    adaptersQueried: ['all_adapters'],
    result: firstAttempt,
    delayMs: 0,
    timestamp: new Date().toISOString(),
  });

  // If successful or waterfall disabled, return immediately
  if (firstAttempt.success || !waterfallConfig.enabled) {
    return {
      success: firstAttempt.success,
      finalResult: firstAttempt,
      attempts,
      totalDuration: Date.now() - startTime,
      fallbackUsed: false,
    };
  }

  logger.info('S2S auction failed, initiating waterfall fallback', {
    requestId: request.id,
    reason: firstAttempt.noBidReason,
  });

  // Get adapters sorted by priority (lower number = higher priority)
  const sortedAdapters = getAdapterConfig()
    .filter(a => a.enabled)
    .sort((a, b) => a.priority - b.priority);

  // Dedupe BEFORE the loop by keeping the first (highest-priority) occurrence of each id.
  // Duplicate ids previously consumed attempt slots (a `continue` skip still burned an
  // iteration against maxAttempts), silently starving distinct adapters later in the list.
  const adapters = sortedAdapters.filter((a, i, arr) => arr.findIndex((x) => x.id === a.id) === i);
  if (adapters.length !== sortedAdapters.length) {
    logger.warn('Waterfall adapter list contained duplicate ids; deduplicated by priority', {
      requestId: request.id,
      before: sortedAdapters.length,
      after: adapters.length,
    });
  }

  // Waterfall through adapters by priority
  let currentDelay = waterfallConfig.initialRetryDelay;

  for (let attemptNum = 2; attemptNum <= waterfallConfig.maxAttempts && attemptNum <= adapters.length + 1; attemptNum++) {
    // Exponential backoff delay
    await sleep(currentDelay);

    // Select adapters for this attempt (higher priority adapters first)
    const adapterId = adapters[attemptNum - 2]?.id;
    
    if (!adapterId) {
      logger.warn('No more adapters available for waterfall', {
        requestId: request.id,
        attemptNumber: attemptNum,
      });
      break;
    }

    logger.info('Waterfall attempt', {
      requestId: request.id,
      attemptNumber: attemptNum,
      adapter: adapterId,
      delayMs: currentDelay,
    });

    // Create filtered request for single adapter
    const filteredRequest: OpenRTBBidRequest = {
      ...request,
      wseat: [adapterId], // Limit to this adapter's seat
    };

    const attemptResult = await executeAuction(filteredRequest);

    attempts.push({
      attemptNumber: attemptNum,
      adaptersQueried: [adapterId],
      result: attemptResult,
      delayMs: currentDelay,
      timestamp: new Date().toISOString(),
    });

    // If this attempt succeeded, return
    if (attemptResult.success) {
      logger.info('Waterfall succeeded', {
        requestId: request.id,
        attemptNumber: attemptNum,
        adapter: adapterId,
        totalDuration: Date.now() - startTime,
      });

      return {
        success: true,
        finalResult: attemptResult,
        attempts,
        totalDuration: Date.now() - startTime,
        fallbackUsed: true,
      };
    }

    // Calculate next delay with exponential backoff
    currentDelay = Math.min(
      currentDelay * waterfallConfig.backoffMultiplier,
      waterfallConfig.maxRetryDelay
    );
  }

  // All attempts failed
  logger.warn('Waterfall exhausted all attempts', {
    requestId: request.id,
    totalAttempts: attempts.length,
    totalDuration: Date.now() - startTime,
  });

  return {
    success: false,
    finalResult: attempts[attempts.length - 1].result,
    attempts,
    totalDuration: Date.now() - startTime,
    fallbackUsed: true,
  };
}

/**
 * Execute auction with priority-based adapter selection
 * Useful for testing specific adapter configurations
 */
export async function executeWithPriority(
  request: OpenRTBBidRequest,
  priorityAdapters: string[]
): Promise<AuctionResult> {
  logger.info('Executing auction with priority adapters', {
    requestId: request.id,
    adapters: priorityAdapters,
  });

  // Filter request to only include priority adapters
  const filteredRequest: OpenRTBBidRequest = {
    ...request,
    wseat: priorityAdapters,
  };

  return executeAuction(filteredRequest);
}

/**
 * Smart waterfall that dynamically adjusts based on adapter performance
 * Uses historical success rates to optimize waterfall order
 */
export async function executeSmartWaterfall(
  request: OpenRTBBidRequest,
  performanceData?: Map<string, number> // adapter_id -> success_rate
): Promise<WaterfallResult> {
  const startTime = Date.now();

  // First attempt: Full S2S auction
  const firstAttempt = await executeAuction(request);

  if (firstAttempt.success) {
    return {
      success: true,
      finalResult: firstAttempt,
      attempts: [{
        attemptNumber: 1,
        adaptersQueried: ['all_adapters'],
        result: firstAttempt,
        delayMs: 0,
        timestamp: new Date().toISOString(),
      }],
      totalDuration: Date.now() - startTime,
      fallbackUsed: false,
    };
  }

  // If we have performance data, sort adapters by success rate
  let adapters = getAdapterConfig()
    .filter(a => a.enabled)
    .sort((a, b) => a.priority - b.priority);

  if (performanceData && performanceData.size > 0) {
    adapters = adapters.sort((a, b) => {
      const aRate = performanceData.get(a.id) || 0;
      const bRate = performanceData.get(b.id) || 0;
      return bRate - aRate; // Higher success rate first
    });

    logger.info('Smart waterfall using performance-based ordering', {
      requestId: request.id,
      adapterOrder: adapters.map(a => ({
        id: a.id,
        successRate: performanceData.get(a.id) || 0,
      })),
    });
  }

  // Execute waterfall with optimized adapter order
  const attempts: WaterfallAttempt[] = [{
    attemptNumber: 1,
    adaptersQueried: ['all_adapters'],
    result: firstAttempt,
    delayMs: 0,
    timestamp: new Date().toISOString(),
  }];

  let currentDelay = DEFAULT_WATERFALL_CONFIG.initialRetryDelay;

  for (let i = 0; i < Math.min(adapters.length, DEFAULT_WATERFALL_CONFIG.maxAttempts - 1); i++) {
    await sleep(currentDelay);

    const adapter = adapters[i];
    const filteredRequest: OpenRTBBidRequest = {
      ...request,
      wseat: [adapter.id],
    };

    const attemptResult = await executeAuction(filteredRequest);

    attempts.push({
      attemptNumber: i + 2,
      adaptersQueried: [adapter.id],
      result: attemptResult,
      delayMs: currentDelay,
      timestamp: new Date().toISOString(),
    });

    if (attemptResult.success) {
      return {
        success: true,
        finalResult: attemptResult,
        attempts,
        totalDuration: Date.now() - startTime,
        fallbackUsed: true,
      };
    }

    currentDelay = Math.min(
      currentDelay * DEFAULT_WATERFALL_CONFIG.backoffMultiplier,
      DEFAULT_WATERFALL_CONFIG.maxRetryDelay
    );
  }

  return {
    success: false,
    finalResult: attempts[attempts.length - 1].result,
    attempts,
    totalDuration: Date.now() - startTime,
    fallbackUsed: true,
  };
}

/**
 * Get waterfall statistics for monitoring
 */
export interface WaterfallStats {
  totalRequests: number;
  successfulFirstAttempt: number;
  successfulWithFallback: number;
  failedAllAttempts: number;
  averageAttempts: number;
  averageDuration: number;
}

// In-memory stats (would be persisted in production)
const waterfallStats: WaterfallStats = {
  totalRequests: 0,
  successfulFirstAttempt: 0,
  successfulWithFallback: 0,
  failedAllAttempts: 0,
  averageAttempts: 0,
  averageDuration: 0,
};

export function updateWaterfallStats(result: WaterfallResult): void {
  waterfallStats.totalRequests++;

  if (result.success) {
    if (result.fallbackUsed) {
      waterfallStats.successfulWithFallback++;
    } else {
      waterfallStats.successfulFirstAttempt++;
    }
  } else {
    waterfallStats.failedAllAttempts++;
  }

  // Update running averages
  const n = waterfallStats.totalRequests;
  waterfallStats.averageAttempts = 
    ((waterfallStats.averageAttempts * (n - 1)) + result.attempts.length) / n;
  waterfallStats.averageDuration = 
    ((waterfallStats.averageDuration * (n - 1)) + result.totalDuration) / n;
}

export function getWaterfallStats(): WaterfallStats {
  return { ...waterfallStats };
}

export function resetWaterfallStats(): void {
  waterfallStats.totalRequests = 0;
  waterfallStats.successfulFirstAttempt = 0;
  waterfallStats.successfulWithFallback = 0;
  waterfallStats.failedAllAttempts = 0;
  waterfallStats.averageAttempts = 0;
  waterfallStats.averageDuration = 0;
}

// ========================================
// Utility Functions
// ========================================

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

export default {
  executeWithWaterfall,
  executeWithPriority,
  executeSmartWaterfall,
  updateWaterfallStats,
  getWaterfallStats,
  resetWaterfallStats,
};
