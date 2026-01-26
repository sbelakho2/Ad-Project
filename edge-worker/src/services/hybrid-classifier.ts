/**
 * HYBRID CLASSIFIER (Two-Stage Email Classification)
 * 
 * Combines fast Naive Bayes with deep ONNX-based semantic analysis.
 * 
 * Pipeline:
 *   1. Rule-based pre-check for obvious categories (OOO, BOUNCE, UNSUBSCRIBE)
 *   2. Fast Pass: Naive Bayes classifier (< 1ms)
 *   3. If confidence > threshold, accept
 *   4. Deep Pass: DistilBERT sentiment + keywords (~500ms)
 *   5. Consensus check: if both agree, high confidence; else flag for review
 * 
 * Fixed bugs:
 *   - Configurable confidence threshold (was hardcoded)
 *   - Model loading synchronization
 *   - Better sentiment-to-category mapping
 */

import type { Pipeline } from '@xenova/transformers';

// ============================================================================
// TYPES
// ============================================================================

export type MessageCategory = 
  | 'INTERESTED'
  | 'MEETING'
  | 'NOT_INTERESTED'
  | 'OOO'
  | 'BOUNCE'
  | 'UNSUBSCRIBE'
  | 'SPAM'
  | 'UNKNOWN';

export interface ClassificationResult {
  category: MessageCategory;
  confidence: number;
  requiresReview: boolean;
  fastPassCategory: MessageCategory;
  fastPassConfidence: number;
  deepPassCategory?: MessageCategory;
  deepPassConfidence?: number;
  consensusReached: boolean;
  processingMode: 'fast' | 'deep' | 'hybrid' | 'rules';
  matchedRules: string[];
  latencyMs: {
    rules?: number;
    fastPass: number;
    deepPass?: number;
    total: number;
  };
}

export interface ClassifierStatus {
  nbTrained: boolean;
  nbCategories: number;
  deepModelLoaded: boolean;
  deepModelLoading: boolean;
  deepModelError: string | null;
  confidenceThreshold: number;
}

// ============================================================================
// NAIVE BAYES CLASSIFIER (Fast Pass)
// ============================================================================

class NaiveBayesClassifier {
  private wordCounts: Map<MessageCategory, Map<string, number>> = new Map();
  private categoryWordTotals: Map<MessageCategory, number> = new Map();
  private categoryCounts: Map<MessageCategory, number> = new Map();
  private totalDocuments = 0;
  private vocabularySize = 0;
  private vocabulary: Set<string> = new Set();
  
  constructor() {
    const categories: MessageCategory[] = [
      'INTERESTED', 'MEETING', 'NOT_INTERESTED', 'OOO',
      'BOUNCE', 'UNSUBSCRIBE', 'SPAM', 'UNKNOWN'
    ];
    
    for (const cat of categories) {
      this.wordCounts.set(cat, new Map());
      this.categoryWordTotals.set(cat, 0);
      this.categoryCounts.set(cat, 0);
    }
  }
  
  private tokenize(text: string): string[] {
    return text
      .toLowerCase()
      .replace(/[^a-z0-9\s]/g, ' ')
      .split(/\s+/)
      .filter(word => word.length > 2 && word.length < 20);
  }
  
  train(text: string, category: MessageCategory): void {
    const words = this.tokenize(text);
    
    this.categoryCounts.set(category, (this.categoryCounts.get(category) || 0) + 1);
    this.totalDocuments++;
    
    const wordCountsForCategory = this.wordCounts.get(category)!;
    
    for (const word of words) {
      this.vocabulary.add(word);
      wordCountsForCategory.set(word, (wordCountsForCategory.get(word) || 0) + 1);
      this.categoryWordTotals.set(category, (this.categoryWordTotals.get(category) || 0) + 1);
    }
    
    this.vocabularySize = this.vocabulary.size;
  }
  
  classify(text: string): { category: MessageCategory; confidence: number } {
    const words = this.tokenize(text);
    const probabilities: Record<string, number> = {};
    
    for (const [category, docCount] of this.categoryCounts) {
      const prior = Math.log((docCount + 1) / (this.totalDocuments + this.categoryCounts.size));
      
      let logLikelihood = 0;
      const wordCountsForCategory = this.wordCounts.get(category)!;
      const totalWordsInCategory = this.categoryWordTotals.get(category) || 0;
      
      for (const word of words) {
        const wordCount = wordCountsForCategory.get(word) || 0;
        const wordProb = (wordCount + 1) / (totalWordsInCategory + this.vocabularySize + 1);
        logLikelihood += Math.log(wordProb);
      }
      
      probabilities[category] = prior + logLikelihood;
    }
    
    // Softmax normalization
    const maxLogProb = Math.max(...Object.values(probabilities));
    let sumExp = 0;
    
    for (const category of Object.keys(probabilities)) {
      probabilities[category] = Math.exp(probabilities[category] - maxLogProb);
      sumExp += probabilities[category];
    }
    
    for (const category of Object.keys(probabilities)) {
      probabilities[category] /= sumExp;
    }
    
    let bestCategory: MessageCategory = 'UNKNOWN';
    let bestProb = 0;
    
    for (const [category, prob] of Object.entries(probabilities)) {
      if (prob > bestProb) {
        bestProb = prob;
        bestCategory = category as MessageCategory;
      }
    }
    
    return { category: bestCategory, confidence: bestProb };
  }
  
  get isTrained(): boolean {
    return this.totalDocuments > 0;
  }
  
  get categoryCount(): number {
    return this.categoryCounts.size;
  }
}

// ============================================================================
// TRAINING DATA
// ============================================================================

const TRAINING_DATA: Array<{ text: string; category: MessageCategory }> = [
  // INTERESTED
  { text: "This sounds interesting, tell me more about your ad network", category: "INTERESTED" },
  { text: "I'd like to learn more about your monetization platform", category: "INTERESTED" },
  { text: "Can you send me more information about your services?", category: "INTERESTED" },
  { text: "We're always looking for new ad partners, what do you offer?", category: "INTERESTED" },
  { text: "Interesting proposition, what are your rates?", category: "INTERESTED" },
  { text: "Thanks for reaching out, we'd be open to discussing this", category: "INTERESTED" },
  { text: "We're exploring new monetization options right now", category: "INTERESTED" },
  { text: "Your timing is perfect, we're looking for new ad partners", category: "INTERESTED" },
  
  // MEETING
  { text: "Let's schedule a call to discuss this further", category: "MEETING" },
  { text: "Can we set up a meeting next week?", category: "MEETING" },
  { text: "I'm free Tuesday for a demo", category: "MEETING" },
  { text: "Please send me a calendar invite for a call", category: "MEETING" },
  { text: "Would love to chat, when works for you?", category: "MEETING" },
  { text: "Let's jump on a quick call this week", category: "MEETING" },
  { text: "Book a slot on my calendar", category: "MEETING" },
  { text: "Schedule a demo with our team", category: "MEETING" },
  
  // NOT_INTERESTED
  { text: "Thanks but we're not interested at this time", category: "NOT_INTERESTED" },
  { text: "We're happy with our current ad partner", category: "NOT_INTERESTED" },
  { text: "Not a good fit for us right now", category: "NOT_INTERESTED" },
  { text: "We've decided to go in a different direction", category: "NOT_INTERESTED" },
  { text: "No thanks, we're all set", category: "NOT_INTERESTED" },
  { text: "We don't use ads in our app", category: "NOT_INTERESTED" },
  { text: "Pass on this opportunity", category: "NOT_INTERESTED" },
  { text: "Not looking for new ad partners currently", category: "NOT_INTERESTED" },
  
  // OOO
  { text: "I'm out of office until January 15th", category: "OOO" },
  { text: "Thank you for your email. I am currently out of the office", category: "OOO" },
  { text: "I'm on vacation and will return on Monday", category: "OOO" },
  { text: "Auto-reply: I'm away from my desk", category: "OOO" },
  { text: "I'm traveling and have limited access to email", category: "OOO" },
  { text: "Out of office automatic reply", category: "OOO" },
  
  // BOUNCE
  { text: "Message delivery failed permanently", category: "BOUNCE" },
  { text: "The email address could not be found", category: "BOUNCE" },
  { text: "User unknown", category: "BOUNCE" },
  { text: "550 5.1.1 The email account does not exist", category: "BOUNCE" },
  { text: "Undeliverable: your message could not be delivered", category: "BOUNCE" },
  { text: "Mail delivery failed: returning message to sender", category: "BOUNCE" },
  { text: "Address rejected: User not found", category: "BOUNCE" },
  
  // UNSUBSCRIBE
  { text: "Please remove me from your mailing list", category: "UNSUBSCRIBE" },
  { text: "Unsubscribe me from these emails", category: "UNSUBSCRIBE" },
  { text: "Stop sending me emails", category: "UNSUBSCRIBE" },
  { text: "I didn't sign up for this, remove my email", category: "UNSUBSCRIBE" },
  { text: "Take me off your list", category: "UNSUBSCRIBE" },
  { text: "Don't contact me again", category: "UNSUBSCRIBE" },
  
  // SPAM
  { text: "This is spam and I'm reporting it", category: "SPAM" },
  { text: "Reported as junk mail", category: "SPAM" },
  { text: "Marked as spam", category: "SPAM" },
  { text: "This is unsolicited commercial email", category: "SPAM" },
];

// ============================================================================
// RULE-BASED PATTERNS (Bypass for obvious categories)
// ============================================================================

const RULE_PATTERNS: Array<{ pattern: RegExp; category: MessageCategory; name: string }> = [
  // OOO patterns (high confidence, bypass ML)
  { pattern: /out\s+of\s+(the\s+)?office/i, category: 'OOO', name: 'ooo_explicit' },
  { pattern: /auto[\s-]?reply/i, category: 'OOO', name: 'ooo_auto_reply' },
  { pattern: /i('m| am)\s+(away|traveling|on\s+vacation)/i, category: 'OOO', name: 'ooo_away' },
  { pattern: /will\s+return\s+on/i, category: 'OOO', name: 'ooo_return' },
  
  // BOUNCE patterns (high confidence, bypass ML)
  { pattern: /delivery\s+(status\s+)?notification\s*\(failure\)/i, category: 'BOUNCE', name: 'bounce_dsn' },
  { pattern: /undeliverable/i, category: 'BOUNCE', name: 'bounce_undeliverable' },
  { pattern: /550\s+5\.\d\.\d/i, category: 'BOUNCE', name: 'bounce_550' },
  { pattern: /user\s+(not\s+found|unknown)/i, category: 'BOUNCE', name: 'bounce_user' },
  { pattern: /mailbox\s+(not\s+found|unavailable)/i, category: 'BOUNCE', name: 'bounce_mailbox' },
  
  // UNSUBSCRIBE patterns
  { pattern: /\bunsubscribe\s+me\b/i, category: 'UNSUBSCRIBE', name: 'unsub_explicit' },
  { pattern: /remove\s+(me\s+)?from\s+(your\s+)?(list|mailing)/i, category: 'UNSUBSCRIBE', name: 'unsub_remove' },
  { pattern: /stop\s+sending\s+(me\s+)?emails/i, category: 'UNSUBSCRIBE', name: 'unsub_stop' },
  { pattern: /don'?t\s+contact\s+me/i, category: 'UNSUBSCRIBE', name: 'unsub_no_contact' },
  
  // MEETING patterns (positive signal)
  { pattern: /\b(schedule|book|set\s+up)\s+(a\s+)?(call|meeting|demo)/i, category: 'MEETING', name: 'meeting_schedule' },
  { pattern: /let'?s\s+(jump\s+on|have)\s+(a\s+)?call/i, category: 'MEETING', name: 'meeting_lets' },
  { pattern: /when\s+(are\s+you|works)\s+(free|for\s+you)/i, category: 'MEETING', name: 'meeting_when' },
];

// ============================================================================
// HYBRID CLASSIFIER CLASS
// ============================================================================

const DEEP_MODEL_NAME = 'Xenova/distilbert-base-uncased-finetuned-sst-2-english';

export class HybridClassifier {
  private naiveBayes: NaiveBayesClassifier;
  private deepPipeline: Pipeline | null = null;
  private deepLoading = false;
  private deepLoadError: Error | null = null;
  private deepLoadPromise: Promise<void> | null = null;
  private confidenceThreshold: number;
  private initialized = false;
  
  constructor(options?: { confidenceThreshold?: number }) {
    this.naiveBayes = new NaiveBayesClassifier();
    this.confidenceThreshold = options?.confidenceThreshold ?? 0.85;
  }
  
  async initialize(): Promise<void> {
    if (this.initialized) return;
    
    // Train Naive Bayes on startup
    for (const example of TRAINING_DATA) {
      this.naiveBayes.train(example.text, example.category);
    }
    
    this.initialized = true;
  }
  
  /**
   * Warmup the deep model
   */
  async warmup(): Promise<{ success: boolean; latencyMs: number }> {
    const startTime = Date.now();
    
    try {
      await this.loadDeepModel();
      
      // Test inference
      if (this.deepPipeline) {
        await this.deepPipeline('This is a test');
      }
      
      return { success: true, latencyMs: Date.now() - startTime };
    } catch (err) {
      return { success: false, latencyMs: Date.now() - startTime };
    }
  }
  
  private async loadDeepModel(): Promise<boolean> {
    if (this.deepPipeline) return true;
    if (this.deepLoadError) return false;
    
    if (this.deepLoadPromise) {
      await this.deepLoadPromise;
      return this.deepPipeline !== null;
    }
    
    this.deepLoading = true;
    this.deepLoadPromise = this.doLoadDeepModel();
    
    try {
      await this.deepLoadPromise;
      return this.deepPipeline !== null;
    } finally {
      this.deepLoading = false;
      this.deepLoadPromise = null;
    }
  }
  
  private async doLoadDeepModel(): Promise<void> {
    try {
      console.log(`[HybridClassifier] Loading deep model ${DEEP_MODEL_NAME}...`);
      
      const { pipeline } = await import('@xenova/transformers');
      
      this.deepPipeline = await pipeline('text-classification', DEEP_MODEL_NAME, {
        quantized: true,
      }) as Pipeline;
      
      console.log('[HybridClassifier] Deep model loaded successfully');
    } catch (err) {
      this.deepLoadError = err as Error;
      console.error('[HybridClassifier] Deep model load failed:', (err as Error).message);
      throw err;
    }
  }
  
  getStatus(): ClassifierStatus {
    return {
      nbTrained: this.naiveBayes.isTrained,
      nbCategories: this.naiveBayes.categoryCount,
      deepModelLoaded: this.deepPipeline !== null,
      deepModelLoading: this.deepLoading,
      deepModelError: this.deepLoadError?.message || null,
      confidenceThreshold: this.confidenceThreshold,
    };
  }
  
  /**
   * Classify an email using the hybrid pipeline
   */
  async classify(subject: string, body: string): Promise<ClassificationResult> {
    const totalStartTime = Date.now();
    const fullText = `${subject} ${body}`;
    const matchedRules: string[] = [];
    
    // ========================================
    // Stage 0: Rule-based pre-check
    // ========================================
    const rulesStartTime = Date.now();
    
    for (const rule of RULE_PATTERNS) {
      if (rule.pattern.test(fullText)) {
        matchedRules.push(rule.name);
        
        // For high-confidence rule matches, return immediately
        if (['OOO', 'BOUNCE'].includes(rule.category) || matchedRules.length >= 2) {
          return {
            category: rule.category,
            confidence: 0.95,
            requiresReview: false,
            fastPassCategory: rule.category,
            fastPassConfidence: 0.95,
            consensusReached: true,
            processingMode: 'rules',
            matchedRules,
            latencyMs: {
              rules: Date.now() - rulesStartTime,
              fastPass: 0,
              total: Date.now() - totalStartTime,
            },
          };
        }
      }
    }
    
    // ========================================
    // Stage 1: Fast Pass (Naive Bayes)
    // ========================================
    const fastStartTime = Date.now();
    const fastResult = this.naiveBayes.classify(fullText);
    const fastLatency = Date.now() - fastStartTime;
    
    // High confidence bypass
    if (fastResult.confidence >= this.confidenceThreshold) {
      return {
        category: fastResult.category,
        confidence: fastResult.confidence,
        requiresReview: false,
        fastPassCategory: fastResult.category,
        fastPassConfidence: fastResult.confidence,
        consensusReached: true,
        processingMode: 'fast',
        matchedRules,
        latencyMs: {
          rules: Date.now() - rulesStartTime,
          fastPass: fastLatency,
          total: Date.now() - totalStartTime,
        },
      };
    }
    
    // ========================================
    // Stage 2: Deep Pass (DistilBERT)
    // ========================================
    const deepStartTime = Date.now();
    let deepCategory: MessageCategory | undefined;
    let deepConfidence: number | undefined;
    
    const deepModelReady = await this.loadDeepModel();
    
    if (deepModelReady && this.deepPipeline) {
      try {
        const result = await this.deepPipeline(fullText);
        const output = Array.isArray(result) ? result[0] : result;
        
        // Map sentiment to category
        const mapped = this.mapSentimentToCategory(
          output.label as string,
          output.score as number,
          fullText
        );
        
        deepCategory = mapped.category;
        deepConfidence = mapped.confidence;
      } catch (err) {
        console.warn('[HybridClassifier] Deep pass failed:', (err as Error).message);
      }
    }
    
    const deepLatency = Date.now() - deepStartTime;
    
    // ========================================
    // Stage 3: Consensus
    // ========================================
    let finalCategory: MessageCategory;
    let finalConfidence: number;
    let consensusReached: boolean;
    let requiresReview: boolean;
    
    if (deepCategory && deepConfidence !== undefined) {
      if (fastResult.category === deepCategory) {
        // Both agree
        finalCategory = fastResult.category;
        finalConfidence = Math.max(fastResult.confidence, deepConfidence);
        consensusReached = true;
        requiresReview = false;
      } else {
        // Disagree - use the higher confidence one but flag for review
        if (fastResult.confidence > deepConfidence) {
          finalCategory = fastResult.category;
          finalConfidence = fastResult.confidence;
        } else {
          finalCategory = deepCategory;
          finalConfidence = deepConfidence;
        }
        consensusReached = false;
        requiresReview = true;
      }
    } else {
      // Deep pass unavailable - use fast pass with review flag
      finalCategory = fastResult.category;
      finalConfidence = fastResult.confidence;
      consensusReached = false;
      requiresReview = fastResult.confidence < 0.7;
    }
    
    return {
      category: finalCategory,
      confidence: finalConfidence,
      requiresReview,
      fastPassCategory: fastResult.category,
      fastPassConfidence: fastResult.confidence,
      deepPassCategory: deepCategory,
      deepPassConfidence: deepConfidence,
      consensusReached,
      processingMode: deepCategory ? 'hybrid' : 'fast',
      matchedRules,
      latencyMs: {
        rules: Date.now() - rulesStartTime,
        fastPass: fastLatency,
        deepPass: deepLatency,
        total: Date.now() - totalStartTime,
      },
    };
  }
  
  private mapSentimentToCategory(
    sentiment: string,
    confidence: number,
    text: string
  ): { category: MessageCategory; confidence: number } {
    const lowerText = text.toLowerCase();
    
    if (sentiment === 'POSITIVE' && confidence > 0.6) {
      // Check for meeting signals
      if (/\b(call|meeting|demo|schedule|chat|discuss)\b/.test(lowerText)) {
        return { category: 'MEETING', confidence: confidence * 0.9 };
      }
      // Check for interest signals
      if (/\b(interested|learn|tell me|sounds|would like)\b/.test(lowerText)) {
        return { category: 'INTERESTED', confidence: confidence * 0.9 };
      }
      return { category: 'INTERESTED', confidence: confidence * 0.7 };
    }
    
    if (sentiment === 'NEGATIVE' && confidence > 0.6) {
      if (/\b(unsubscribe|remove|stop|don't contact)\b/.test(lowerText)) {
        return { category: 'UNSUBSCRIBE', confidence: confidence * 0.9 };
      }
      return { category: 'NOT_INTERESTED', confidence: confidence * 0.8 };
    }
    
    return { category: 'UNKNOWN', confidence: confidence * 0.5 };
  }
  
  /**
   * Learn from feedback (online training)
   */
  learn(text: string, category: MessageCategory): void {
    this.naiveBayes.train(text, category);
  }
}
