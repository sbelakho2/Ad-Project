/**
 * HYBRID CLASSIFIER (Edge Worker Proxy)
 * 
 * Combines fast Naive Bayes with deep ONNX-based semantic classification.
 * The actual ML inference runs on the Edge Worker (Test Server).
 * 
 * Pipeline:
 *   1. Fast Pass: Naive Bayes classifier (< 1ms) - runs locally
 *   2. Confidence Check: If confidence > 0.85, accept result
 *   3. Deep Pass: ONNX DistilBERT via Edge Worker (~500ms)
 *   4. Consensus: If both agree, high confidence. If disagree, flag for review.
 * 
 * Architecture:
 *   - Control Plane: Fast Naive Bayes, orchestration, audit logging
 *   - Edge Worker: Deep ONNX inference
 */

import { logger } from '../../logger.js';
import { getEdgeWorkerClient, isEdgeWorkerAvailable } from '../edge-worker-client.js';
import type { MessageCategory, ClassificationResult } from './clerk.js';

// ============================================================================
// TYPES
// ============================================================================

export interface HybridClassificationResult extends ClassificationResult {
  fastPassCategory: MessageCategory;
  fastPassConfidence: number;
  deepPassCategory?: MessageCategory;
  deepPassConfidence?: number;
  consensusReached: boolean;
  processingMode: 'fast' | 'deep' | 'hybrid' | 'edge-worker';
  latencyMs: {
    fastPass: number;
    deepPass?: number;
    total: number;
  };
}

// ============================================================================
// LOCAL NAIVE BAYES (Fast Pass)
// ============================================================================

interface NaiveBayesLike {
  classify: (subject: string, body: string) => ClassificationResult;
  learn: (text: string, category: MessageCategory) => void;
}

// ============================================================================
// HYBRID CLASSIFIER
// ============================================================================

export class HybridClassifier {
  private fastClassifier: NaiveBayesLike;
  private confidenceThreshold: number;
  private deepPassEnabled: boolean;
  
  constructor(
    fastClassifier: NaiveBayesLike,
    options?: {
      confidenceThreshold?: number;
      deepPassEnabled?: boolean;
    }
  ) {
    this.fastClassifier = fastClassifier;
    this.confidenceThreshold = options?.confidenceThreshold ?? 0.85;
    this.deepPassEnabled = options?.deepPassEnabled ?? true;
  }
  
  /**
   * Classify an email using hybrid pipeline
   */
  async classify(subject: string, body: string): Promise<HybridClassificationResult> {
    const totalStartTime = Date.now();
    
    // ==================================
    // Stage 1: Fast Pass (Local Naive Bayes)
    // ==================================
    const fastStartTime = Date.now();
    const fastResult = this.fastClassifier.classify(subject, body);
    const fastLatency = Date.now() - fastStartTime;
    
    // High confidence bypass
    if (fastResult.confidence >= this.confidenceThreshold) {
      return {
        ...fastResult,
        fastPassCategory: fastResult.category,
        fastPassConfidence: fastResult.confidence,
        consensusReached: true,
        processingMode: 'fast',
        latencyMs: {
          fastPass: fastLatency,
          total: Date.now() - totalStartTime,
        },
      };
    }
    
    // Rule-based bypass for obvious categories
    const textLower = `${subject} ${body}`.toLowerCase();
    if (this.isObviousCategory(textLower)) {
      return {
        ...fastResult,
        fastPassCategory: fastResult.category,
        fastPassConfidence: 0.95, // High confidence for rule-based
        consensusReached: true,
        processingMode: 'fast',
        latencyMs: {
          fastPass: fastLatency,
          total: Date.now() - totalStartTime,
        },
      };
    }
    
    // ==================================
    // Stage 2: Deep Pass (Edge Worker)
    // ==================================
    if (!this.deepPassEnabled || !isEdgeWorkerAvailable()) {
      // No deep pass available - use fast result with review flag
      return {
        ...fastResult,
        fastPassCategory: fastResult.category,
        fastPassConfidence: fastResult.confidence,
        requiresReview: fastResult.confidence < 0.7,
        consensusReached: false,
        processingMode: 'fast',
        latencyMs: {
          fastPass: fastLatency,
          total: Date.now() - totalStartTime,
        },
      };
    }
    
    const deepStartTime = Date.now();
    let deepCategory: MessageCategory | undefined;
    let deepConfidence: number | undefined;
    
    try {
      const client = getEdgeWorkerClient();
      const deepResult = await client.classifyMessage(subject, body);
      
      deepCategory = deepResult.category;
      deepConfidence = deepResult.confidence;
    } catch (err) {
      logger.warn('[HybridClassifier] Edge Worker deep pass failed', {
        error: (err as Error).message,
      });
    }
    
    const deepLatency = Date.now() - deepStartTime;
    
    // ==================================
    // Stage 3: Consensus
    // ==================================
    let finalCategory = fastResult.category;
    let finalConfidence = fastResult.confidence;
    let consensusReached = false;
    let requiresReview = false;
    
    if (deepCategory && deepConfidence !== undefined) {
      if (fastResult.category === deepCategory) {
        // Both agree
        consensusReached = true;
        finalConfidence = Math.max(fastResult.confidence, deepConfidence);
        requiresReview = false;
      } else {
        // Disagree - use higher confidence but flag for review
        consensusReached = false;
        requiresReview = true;
        
        if (deepConfidence > fastResult.confidence) {
          finalCategory = deepCategory;
          finalConfidence = deepConfidence;
        }
      }
    } else {
      // Deep pass failed - use fast result with review flag
      requiresReview = fastResult.confidence < 0.7;
    }
    
    return {
      category: finalCategory,
      confidence: finalConfidence,
      probabilities: fastResult.probabilities, // Keep fast pass probabilities
      matchedRules: fastResult.matchedRules,   // Keep fast pass matched rules
      requiresReview,
      fastPassCategory: fastResult.category,
      fastPassConfidence: fastResult.confidence,
      deepPassCategory: deepCategory,
      deepPassConfidence: deepConfidence,
      consensusReached,
      processingMode: deepCategory ? 'hybrid' : 'fast',
      latencyMs: {
        fastPass: fastLatency,
        deepPass: deepLatency,
        total: Date.now() - totalStartTime,
      },
    };
  }
  
  private isObviousCategory(textLower: string): boolean {
    // OOO patterns
    if (/out\s+of\s+(the\s+)?office/i.test(textLower)) return true;
    if (/auto[\s-]?reply/i.test(textLower)) return true;
    
    // Bounce patterns
    if (/undeliverable/i.test(textLower)) return true;
    if (/delivery\s+failed/i.test(textLower)) return true;
    if (/550\s+5\.\d\.\d/i.test(textLower)) return true;
    
    // Unsubscribe patterns
    if (/\bunsubscribe\s+me\b/i.test(textLower)) return true;
    if (/remove\s+(me\s+)?from\s+(your\s+)?list/i.test(textLower)) return true;
    
    return false;
  }
  
  /**
   * Learn from feedback (trains fast classifier only)
   */
  learn(text: string, correctCategory: MessageCategory): void {
    this.fastClassifier.learn(text, correctCategory);
  }
  
  /**
   * Pre-warm the deep model on Edge Worker
   */
  async warmup(): Promise<{ loaded: boolean; latencyMs: number }> {
    const startTime = Date.now();
    
    if (!isEdgeWorkerAvailable()) {
      return { loaded: false, latencyMs: Date.now() - startTime };
    }
    
    try {
      const client = getEdgeWorkerClient();
      const result = await client.warmup();
      
      return {
        loaded: result.success && result.classifier.loaded,
        latencyMs: Date.now() - startTime,
      };
    } catch {
      return { loaded: false, latencyMs: Date.now() - startTime };
    }
  }
  
  /**
   * Get model status
   */
  getStatus(): {
    deepModelLoaded: boolean;
    deepModelLoading: boolean;
    deepModelError: string | null;
    deepPassEnabled: boolean;
    confidenceThreshold: number;
    edgeWorkerAvailable: boolean;
  } {
    return {
      deepModelLoaded: false, // We don't know until we check Edge Worker
      deepModelLoading: false,
      deepModelError: null,
      deepPassEnabled: this.deepPassEnabled,
      confidenceThreshold: this.confidenceThreshold,
      edgeWorkerAvailable: isEdgeWorkerAvailable(),
    };
  }
  
  /**
   * Enable/disable deep pass at runtime
   */
  setDeepPassEnabled(enabled: boolean): void {
    this.deepPassEnabled = enabled;
    logger.info('Hybrid classifier deep pass toggled', { enabled });
  }
}

// ============================================================================
// FACTORY FUNCTION
// ============================================================================

/**
 * Create a hybrid classifier wrapping an existing Clerk instance
 */
export function createHybridClassifier(
  clerk: { classify: (subject: string, body: string) => ClassificationResult; learn: (text: string, category: MessageCategory) => void },
  options?: {
    confidenceThreshold?: number;
    deepPassEnabled?: boolean;
  }
): HybridClassifier {
  return new HybridClassifier(clerk, options);
}
