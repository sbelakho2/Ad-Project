/**
 * CLERK MODULE (The Inbox Classifier)
 * 
 * Zero-cost email classification using Naive Bayes.
 * Automatically categorizes incoming responses and routes them.
 * 
 * Categories:
 *   - INTERESTED: Positive response, wants to learn more
 *   - MEETING: Requests a meeting or call
 *   - NOT_INTERESTED: Polite decline
 *   - OOO: Out of office auto-reply
 *   - BOUNCE: Email delivery failure
 *   - UNSUBSCRIBE: Opt-out request
 *   - SPAM: Spam/irrelevant response
 *   - UNKNOWN: Needs human review
 * 
 * Implementation:
 *   - Uses Naive Bayes with Laplace smoothing
 *   - Pre-trained on common patterns
 *   - Learns from operator feedback
 *   - Rule-based fallbacks for edge cases
 */

import { queryOperator, queryReadOnly } from '../../database';
import { logger } from '../../logger';

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
  probabilities: Record<MessageCategory, number>;
  matchedRules: string[];
  requiresReview: boolean;
}

export interface InboxMessage {
  id: string;
  contactId?: string;
  fromEmail: string;
  subject: string;
  body: string;
  receivedAt: Date;
  classification?: MessageCategory;
  confidence?: number;
  reviewed: boolean;
}

// ============================================================================
// NAIVE BAYES CLASSIFIER
// ============================================================================

export class NaiveBayesClassifier {
  // Word counts per category
  private wordCounts: Map<MessageCategory, Map<string, number>> = new Map();
  // Total words per category
  private categoryWordTotals: Map<MessageCategory, number> = new Map();
  // Document counts per category
  private categoryCounts: Map<MessageCategory, number> = new Map();
  // Total documents
  private totalDocuments = 0;
  // Vocabulary size (for Laplace smoothing)
  private vocabularySize = 0;
  // All unique words
  private vocabulary: Set<string> = new Set();
  
  constructor() {
    // Initialize categories
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
  
  /**
   * Tokenize text into words
   */
  private tokenize(text: string): string[] {
    return text
      .toLowerCase()
      .replace(/[^a-z0-9\s]/g, ' ')
      .split(/\s+/)
      .filter(word => word.length > 2 && word.length < 20);
  }
  
  /**
   * Train on a single document
   */
  train(text: string, category: MessageCategory): void {
    const words = this.tokenize(text);
    
    // Increment document count
    this.categoryCounts.set(
      category, 
      (this.categoryCounts.get(category) || 0) + 1
    );
    this.totalDocuments++;
    
    // Count words
    const wordCountsForCategory = this.wordCounts.get(category)!;
    
    for (const word of words) {
      this.vocabulary.add(word);
      wordCountsForCategory.set(
        word, 
        (wordCountsForCategory.get(word) || 0) + 1
      );
      this.categoryWordTotals.set(
        category,
        (this.categoryWordTotals.get(category) || 0) + 1
      );
    }
    
    this.vocabularySize = this.vocabulary.size;
  }
  
  /**
   * Classify a document
   */
  classify(text: string): { category: MessageCategory; confidence: number; probabilities: Record<MessageCategory, number> } {
    const words = this.tokenize(text);
    const probabilities: Record<string, number> = {};
    
    // Calculate log probabilities for each category
    for (const [category, docCount] of this.categoryCounts) {
      // Prior probability P(category)
      const prior = Math.log((docCount + 1) / (this.totalDocuments + this.categoryCounts.size));
      
      // Likelihood P(words|category) using Laplace smoothing
      let logLikelihood = 0;
      const wordCountsForCategory = this.wordCounts.get(category)!;
      const totalWordsInCategory = this.categoryWordTotals.get(category) || 0;
      
      for (const word of words) {
        const wordCount = wordCountsForCategory.get(word) || 0;
        // Laplace smoothing: (count + 1) / (total + vocabulary_size)
        const wordProb = (wordCount + 1) / (totalWordsInCategory + this.vocabularySize + 1);
        logLikelihood += Math.log(wordProb);
      }
      
      probabilities[category] = prior + logLikelihood;
    }
    
    // Convert log probabilities to normalized probabilities
    const maxLogProb = Math.max(...Object.values(probabilities));
    let sumExp = 0;
    
    for (const category of Object.keys(probabilities)) {
      probabilities[category] = Math.exp(probabilities[category] - maxLogProb);
      sumExp += probabilities[category];
    }
    
    // Normalize
    for (const category of Object.keys(probabilities)) {
      probabilities[category] /= sumExp;
    }
    
    // Find best category
    let bestCategory: MessageCategory = 'UNKNOWN';
    let bestProb = 0;
    
    for (const [category, prob] of Object.entries(probabilities)) {
      if (prob > bestProb) {
        bestProb = prob;
        bestCategory = category as MessageCategory;
      }
    }
    
    return {
      category: bestCategory,
      confidence: bestProb,
      probabilities: probabilities as Record<MessageCategory, number>,
    };
  }
  
  /**
   * Export model state for persistence
   */
  exportModel(): {
    wordCounts: Record<string, Record<string, number>>;
    categoryWordTotals: Record<string, number>;
    categoryCounts: Record<string, number>;
    totalDocuments: number;
    vocabularySize: number;
  } {
    const wordCountsObj: Record<string, Record<string, number>> = {};
    for (const [category, counts] of this.wordCounts) {
      wordCountsObj[category] = Object.fromEntries(counts);
    }
    
    return {
      wordCounts: wordCountsObj,
      categoryWordTotals: Object.fromEntries(this.categoryWordTotals),
      categoryCounts: Object.fromEntries(this.categoryCounts),
      totalDocuments: this.totalDocuments,
      vocabularySize: this.vocabularySize,
    };
  }
  
  /**
   * Import model state
   */
  importModel(state: ReturnType<typeof this.exportModel>): void {
    this.totalDocuments = state.totalDocuments;
    this.vocabularySize = state.vocabularySize;
    
    for (const [category, counts] of Object.entries(state.wordCounts)) {
      this.wordCounts.set(category as MessageCategory, new Map(Object.entries(counts)));
    }
    
    this.categoryWordTotals = new Map(Object.entries(state.categoryWordTotals) as [MessageCategory, number][]);
    this.categoryCounts = new Map(Object.entries(state.categoryCounts) as [MessageCategory, number][]);
    
    // Rebuild vocabulary
    this.vocabulary.clear();
    for (const counts of this.wordCounts.values()) {
      for (const word of counts.keys()) {
        this.vocabulary.add(word);
      }
    }
  }
}

// ============================================================================
// RULE-BASED PATTERNS (Fallback/Override)
// ============================================================================

interface ClassificationRule {
  id: string;
  category: MessageCategory;
  patterns: RegExp[];
  priority: number;
}

const BUILT_IN_RULES: ClassificationRule[] = [
  // Out of Office - highest priority
  {
    id: 'ooo_standard',
    category: 'OOO',
    patterns: [
      /out of (the )?office/i,
      /on (annual |paid )?leave/i,
      /away from (my )?email/i,
      /limited access to email/i,
      /automatic reply/i,
      /auto[- ]?reply/i,
      /return(ing)? on/i,
      /back (in the )?office/i,
      /currently (out|away|traveling)/i,
    ],
    priority: 100,
  },
  
  // Bounce - highest priority
  {
    id: 'bounce_delivery',
    category: 'BOUNCE',
    patterns: [
      /delivery (has )?failed/i,
      /undeliverable/i,
      /mail delivery (sub)?system/i,
      /mailbox (not found|unavailable|full)/i,
      /user (not found|unknown|doesn't exist)/i,
      /address rejected/i,
      /550 (5\.1\.1|user unknown)/i,
      /permanent failure/i,
      /no such user/i,
    ],
    priority: 100,
  },
  
  // Unsubscribe requests
  {
    id: 'unsubscribe_request',
    category: 'UNSUBSCRIBE',
    patterns: [
      /unsubscribe/i,
      /remove (me |my email )?from (your |this |the )?list/i,
      /stop (sending|emailing|contacting)/i,
      /don'?t (contact|email|send)/i,
      /opt[- ]?out/i,
      /take me off/i,
    ],
    priority: 90,
  },
  
  // Meeting requests
  {
    id: 'meeting_request',
    category: 'MEETING',
    patterns: [
      /let'?s (set up|schedule|book) a (call|meeting|demo)/i,
      /can we (chat|talk|meet|schedule)/i,
      /free for a (quick )?(call|chat|meeting)/i,
      /would (love|like) to (chat|talk|discuss|meet)/i,
      /calendar link/i,
      /when are you (free|available)/i,
      /pick a time/i,
      /book(ed)? (a |some )?time/i,
    ],
    priority: 80,
  },
  
  // Interested signals
  {
    id: 'interested_positive',
    category: 'INTERESTED',
    patterns: [
      /sounds (great|interesting|good)/i,
      /tell me more/i,
      /send (me |over )?(more )?info/i,
      /interested in (learning|hearing|knowing)/i,
      /this (sounds|looks) (like what|exactly)/i,
      /would (love|like) to (learn|know|hear) more/i,
      /could you (share|send|provide)/i,
      /what (are your|is the) pricing/i,
    ],
    priority: 70,
  },
  
  // Not interested signals
  {
    id: 'not_interested_decline',
    category: 'NOT_INTERESTED',
    patterns: [
      /not (a good fit|interested|looking)/i,
      /no(t| )thanks?/i,
      /pass (on this|for now)/i,
      /not (right now|at this time)/i,
      /already (have|use|work with)/i,
      /happy with (our |my )?current/i,
      /not (in the )?market/i,
      /budget(ed)? for/i,
    ],
    priority: 70,
  },
];

/**
 * Apply rule-based classification
 */
function applyRules(text: string): { category: MessageCategory; matchedRules: string[] } | null {
  const fullText = text.toLowerCase();
  const matches: { rule: ClassificationRule; matchCount: number }[] = [];
  
  for (const rule of BUILT_IN_RULES) {
    let matchCount = 0;
    for (const pattern of rule.patterns) {
      if (pattern.test(fullText)) {
        matchCount++;
      }
    }
    
    if (matchCount > 0) {
      matches.push({ rule, matchCount });
    }
  }
  
  if (matches.length === 0) {
    return null;
  }
  
  // Sort by priority (desc) then match count (desc)
  matches.sort((a, b) => {
    if (a.rule.priority !== b.rule.priority) {
      return b.rule.priority - a.rule.priority;
    }
    return b.matchCount - a.matchCount;
  });
  
  return {
    category: matches[0].rule.category,
    matchedRules: matches.map(m => m.rule.id),
  };
}

// ============================================================================
// CLERK CLASS (Main Classifier)
// ============================================================================

export class Clerk {
  private classifier: NaiveBayesClassifier;
  private confidenceThreshold: number;
  
  constructor(options?: { confidenceThreshold?: number }) {
    this.classifier = new NaiveBayesClassifier();
    this.confidenceThreshold = options?.confidenceThreshold || 0.6;
    
    // Initialize with default training data
    this.initializeDefaultTraining();
  }
  
  /**
   * Initialize with common email patterns
   */
  private initializeDefaultTraining(): void {
    // OOO examples
    const oooExamples = [
      "I am currently out of the office and will return on Monday.",
      "Thank you for your email. I am on annual leave until next week.",
      "This is an automatic reply. I am away from email.",
      "I'm traveling and have limited access to email.",
      "Out of office: I will be back on the 15th.",
    ];
    
    // Bounce examples
    const bounceExamples = [
      "Mail delivery failed: returning message to sender.",
      "Your message could not be delivered. User unknown.",
      "Undeliverable: 550 5.1.1 The email account does not exist.",
      "Mailbox unavailable. Message could not be delivered.",
      "Permanent failure: address rejected.",
    ];
    
    // Interested examples
    const interestedExamples = [
      "This sounds really interesting! Can you tell me more?",
      "I'd love to learn more about your platform.",
      "Could you send over some more information?",
      "This looks like exactly what we need. What's the pricing?",
      "Interesting! I'd like to hear more about how this works.",
    ];
    
    // Meeting examples
    const meetingExamples = [
      "Let's set up a call to discuss this further.",
      "I'm free Thursday afternoon for a quick chat.",
      "Can we schedule a demo next week?",
      "Would love to chat! Here's my calendar link.",
      "Let's book some time to talk through the details.",
    ];
    
    // Not interested examples
    const notInterestedExamples = [
      "Thanks, but we're not interested at this time.",
      "We're happy with our current solution.",
      "Not a good fit for us right now.",
      "We'll pass on this, but thanks for reaching out.",
      "We're not in the market for this currently.",
    ];
    
    // Unsubscribe examples
    const unsubscribeExamples = [
      "Please remove me from your mailing list.",
      "Unsubscribe me from these emails.",
      "Stop sending me these emails.",
      "Please don't contact me again.",
      "Take me off your list.",
    ];
    
    // Train on all examples
    for (const text of oooExamples) this.classifier.train(text, 'OOO');
    for (const text of bounceExamples) this.classifier.train(text, 'BOUNCE');
    for (const text of interestedExamples) this.classifier.train(text, 'INTERESTED');
    for (const text of meetingExamples) this.classifier.train(text, 'MEETING');
    for (const text of notInterestedExamples) this.classifier.train(text, 'NOT_INTERESTED');
    for (const text of unsubscribeExamples) this.classifier.train(text, 'UNSUBSCRIBE');
    
    logger.info('Clerk initialized with default training data');
  }
  
  /**
   * Classify an email
   */
  classify(subject: string, body: string): ClassificationResult {
    const fullText = `${subject} ${body}`;
    
    // First, try rule-based classification
    const ruleResult = applyRules(fullText);
    
    // Then, get Naive Bayes classification
    const bayesResult = this.classifier.classify(fullText);
    
    // Decision logic:
    // 1. If rules match with high-priority categories (OOO, BOUNCE), use rules
    // 2. If Bayes has high confidence, use Bayes
    // 3. If rules match, use rules
    // 4. If Bayes confidence is above threshold, use Bayes
    // 5. Otherwise, mark as UNKNOWN for review
    
    let finalCategory: MessageCategory;
    let finalConfidence: number;
    let matchedRules: string[] = [];
    let requiresReview = false;
    
    if (ruleResult && ['OOO', 'BOUNCE', 'UNSUBSCRIBE'].includes(ruleResult.category)) {
      // High-priority rule matches take precedence
      finalCategory = ruleResult.category;
      finalConfidence = 0.95;
      matchedRules = ruleResult.matchedRules;
    } else if (bayesResult.confidence > 0.8) {
      // High confidence Bayes
      finalCategory = bayesResult.category;
      finalConfidence = bayesResult.confidence;
    } else if (ruleResult) {
      // Rule matches
      finalCategory = ruleResult.category;
      finalConfidence = 0.8;
      matchedRules = ruleResult.matchedRules;
    } else if (bayesResult.confidence >= this.confidenceThreshold) {
      // Above threshold Bayes
      finalCategory = bayesResult.category;
      finalConfidence = bayesResult.confidence;
    } else {
      // Low confidence - needs review
      finalCategory = 'UNKNOWN';
      finalConfidence = bayesResult.confidence;
      requiresReview = true;
    }
    
    return {
      category: finalCategory,
      confidence: finalConfidence,
      probabilities: bayesResult.probabilities,
      matchedRules,
      requiresReview,
    };
  }
  
  /**
   * Learn from feedback
   */
  learn(text: string, correctCategory: MessageCategory): void {
    this.classifier.train(text, correctCategory);
    logger.debug('Clerk learned from feedback', { category: correctCategory });
  }
  
  /**
   * Get classifier stats
   */
  getStats(): {
    totalDocuments: number;
    vocabularySize: number;
    categoryCounts: Record<string, number>;
  } {
    const model = this.classifier.exportModel();
    return {
      totalDocuments: model.totalDocuments,
      vocabularySize: model.vocabularySize,
      categoryCounts: model.categoryCounts,
    };
  }
  
  /**
   * Export model for persistence
   */
  exportModel(): ReturnType<NaiveBayesClassifier['exportModel']> {
    return this.classifier.exportModel();
  }
  
  /**
   * Import model
   */
  importModel(state: ReturnType<NaiveBayesClassifier['exportModel']>): void {
    this.classifier.importModel(state);
  }
}

// ============================================================================
// DATABASE OPERATIONS
// ============================================================================

/**
 * Save message classification
 */
export async function saveMessageClassification(params: {
  contactId?: string;
  fromEmail: string;
  subject: string;
  body: string;
  classification: ClassificationResult;
}): Promise<string> {
  const result = await queryOperator<{ id: string }>(`
    INSERT INTO sa_inbox_messages 
    (contact_id, from_email, subject, body_text, classification, classification_confidence, requires_human_review)
    VALUES ($1, $2, $3, $4, $5, $6, $7)
    RETURNING id
  `, [
    params.contactId || null,
    params.fromEmail,
    params.subject,
    params.body,
    params.classification.category.toLowerCase(),
    Math.round(params.classification.confidence * 100),
    params.classification.requiresReview,
  ]);
  
  return result[0].id;
}

/**
 * Get messages needing review
 */
export async function getMessagesForReview(limit = 20): Promise<InboxMessage[]> {
  const rows = await queryReadOnly<{
    id: string;
    contact_id: string | null;
    from_email: string;
    subject: string;
    body_text: string;
    received_at: string;
    classification: string | null;
    classification_confidence: string | null;
    human_reviewed_at: string | null;
  }>(`
    SELECT id, contact_id, from_email, subject, body_text, received_at,
           classification, classification_confidence, human_reviewed_at
    FROM sa_inbox_messages
    WHERE requires_human_review = true AND human_reviewed_at IS NULL
    ORDER BY received_at DESC
    LIMIT $1
  `, [limit]);
  
  return rows.map(row => ({
    id: row.id,
    contactId: row.contact_id || undefined,
    fromEmail: row.from_email,
    subject: row.subject,
    body: row.body_text,
    receivedAt: new Date(row.received_at),
    classification: row.classification?.toUpperCase() as MessageCategory | undefined,
    confidence: row.classification_confidence ? parseInt(row.classification_confidence) / 100 : undefined,
    reviewed: !!row.human_reviewed_at,
  }));
}

/**
 * Submit review feedback
 */
export async function submitReview(
  messageId: string, 
  correctCategory: MessageCategory
): Promise<void> {
  // Update the message
  await queryOperator(`
    UPDATE sa_inbox_messages
    SET classification = $1, human_reviewed_at = CURRENT_TIMESTAMP
    WHERE id = $2
  `, [correctCategory.toLowerCase(), messageId]);
  
  // Get the message content for training
  const messages = await queryReadOnly<{ subject: string; body_text: string }>(`
    SELECT subject, body_text FROM sa_inbox_messages WHERE id = $1
  `, [messageId]);
  
  if (messages.length > 0) {
    // Save training data (column is 'label' not 'category')
    await queryOperator(`
      INSERT INTO sa_bayes_training (label, text_content, source)
      VALUES ($1, $2, 'manual')
    `, [correctCategory.toLowerCase(), `${messages[0].subject} ${messages[0].body_text}`]);
  }
}

/**
 * Load training data from database
 */
export async function loadTrainingData(): Promise<Array<{ category: MessageCategory; text: string }>> {
  const rows = await queryReadOnly<{
    label: string;
    text_content: string;
  }>(`
    SELECT label, text_content
    FROM sa_bayes_training
    ORDER BY created_at
  `);
  
  return rows.map(row => ({
    category: row.label.toUpperCase() as MessageCategory,
    text: row.text_content,
  }));
}

/**
 * Initialize Clerk from database
 */
export async function initializeClerkFromDatabase(): Promise<Clerk> {
  const clerk = new Clerk();
  
  // Load training data
  const trainingData = await loadTrainingData();
  
  for (const { category, text } of trainingData) {
    clerk.learn(text, category);
  }
  
  logger.info('Clerk loaded training data', { 
    examples: trainingData.length 
  });
  
  return clerk;
}

/**
 * Get classification statistics
 */
export async function getClassificationStats(): Promise<{
  totalMessages: number;
  byCategory: Record<string, number>;
  pendingReview: number;
  averageConfidence: number;
}> {
  const stats = await queryReadOnly<{
    total_messages: string;
    pending_review: string;
    avg_confidence: string;
  }>(`
    SELECT 
      COUNT(*) as total_messages,
      COUNT(*) FILTER (WHERE requires_human_review AND human_reviewed_at IS NULL) as pending_review,
      COALESCE(AVG(classification_confidence), 0) as avg_confidence
    FROM sa_inbox_messages
  `);
  
  const byCategory = await queryReadOnly<{
    classification: string;
    count: string;
  }>(`
    SELECT classification, COUNT(*) as count
    FROM sa_inbox_messages
    WHERE classification IS NOT NULL
    GROUP BY classification
  `);
  
  const byCategoryMap: Record<string, number> = {};
  for (const row of byCategory) {
    byCategoryMap[row.classification.toUpperCase()] = parseInt(row.count);
  }
  
  return {
    totalMessages: parseInt(stats[0].total_messages),
    byCategory: byCategoryMap,
    pendingReview: parseInt(stats[0].pending_review),
    averageConfidence: parseFloat(stats[0].avg_confidence) / 100, // Convert back to 0-1 range
  };
}
