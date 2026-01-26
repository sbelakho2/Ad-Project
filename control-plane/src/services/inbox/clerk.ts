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
    
    // Ensure category is initialized (handle unknown categories from training data)
    if (!this.wordCounts.has(category)) {
      this.wordCounts.set(category, new Map());
      this.categoryWordTotals.set(category, 0);
      this.categoryCounts.set(category, 0);
    }
    
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
      /on (annual |paid |maternity |paternity )?leave/i,
      /away from (my )?email/i,
      /limited (access|connectivity)/i,
      /automatic reply/i,
      /auto[- ]?reply/i,
      /return(ing)? on/i,
      /back (in the )?office/i,
      /currently (out|away|traveling|on leave|on vacation|on holiday)/i,
      /on (PTO|vacation|holiday)/i,
      /will (respond|reply|return|be back)/i,
      /offsite (retreat|meeting)/i,
      /parental leave/i,
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
      /mailer[- ]?daemon/i,
      /delivery status notification/i,
      /email account does not exist/i,
      /recipient.*rejected/i,
      /DNS.*failed/i,
      /no MX records/i,
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
      /cease (all )?contact/i,
      /marking (this )?as spam/i,
      /never (signed up|gave permission)/i,
    ],
    priority: 90,
  },
  
  // Meeting requests - ENHANCED
  {
    id: 'meeting_request',
    category: 'MEETING',
    patterns: [
      // Scheduling intent
      /let'?s (set up|schedule|book|arrange) a (call|meeting|demo|zoom|chat)/i,
      /can we (chat|talk|meet|schedule|connect|catch up|hop on a call)/i,
      /free for a (quick )?(call|chat|meeting|demo|sync)/i,
      /would (love|like) to (chat|talk|discuss|meet|connect|hop on)/i,
      /when are you (free|available)/i,
      /pick a time/i,
      /book(ed)? (a |some )?time/i,
      
      // Calendar patterns
      /calendar (link|invite)/i,
      /calendly/i,
      /cal\.com/i,
      /doodle/i,
      /send (a |me )?(calendar )?invite/i,
      /schedule (a |the )?demo/i,
      
      // Time availability patterns
      /i'?m free (on |at |this |next )?/i,
      /does (that|this) work/i,
      /what times? (work|are you|do you)/i,
      /how('?s| about) (next|this|monday|tuesday|wednesday|thursday|friday)/i,
      /morning(s)? work (best|for me)/i,
      /afternoon(s)? work (best|for me)/i,
      /(zoom|google meet|teams|webex)/i,
      
      // Meeting request phrases
      /jump on a (quick )?(call|zoom|meeting)/i,
      /let'?s (find|pick) (a )?time/i,
      /happy to (chat|talk|meet|connect|discuss)/i,
      /looking forward to (chatting|talking|meeting|discussing)/i,
      /30 minute (call|meeting|chat)/i,
      /quick (call|chat|sync|demo)/i,
    ],
    priority: 80,
  },
  
  // Interested signals - ENHANCED
  {
    id: 'interested_positive',
    category: 'INTERESTED',
    patterns: [
      // Interest expressions
      /sounds (great|interesting|good|promising|intriguing)/i,
      /tell me more/i,
      /send (me |over )?(more )?info(rmation)?/i,
      /interested in (learning|hearing|knowing|exploring|understanding)/i,
      /this (sounds|looks) (like what|exactly)/i,
      /would (love|like) to (learn|know|hear|understand|explore) more/i,
      /could you (share|send|provide|explain)/i,
      /what (are your|is the) pricing/i,
      
      // Buying signals
      /looking for (a |an )?(solution|partner|platform|alternative)/i,
      /currently (using|evaluating|looking)/i,
      /considering (alternatives|options|switching)/i,
      /actively (evaluating|searching|looking)/i,
      /what makes you (different|better)/i,
      /how does (it|this|your) work/i,
      /can you (do|improve|beat)/i,
      
      // Forwarding/team interest
      /forward(ed|ing) (this )?to (my |our )?team/i,
      /will (share|forward) (with|to)/i,
      /monetization team/i,
      /relevant to (what )?we'?re (doing|building)/i,
      /right time/i,
      /great timing/i,
      
      // Positive curiosity
      /curious about/i,
      /intrigued by/i,
      /want to (learn|know|understand|see)/i,
      /open to (exploring|discussing|learning)/i,
      /definitely (open|interested)/i,
      /case study/i,
      /what (kind of )?results/i,
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
   * Initialize with comprehensive email patterns
   * 
   * IMPORTANT: The Naive Bayes classifier needs sufficient examples per category
   * to build reliable probability distributions. Minimum 15-20 examples per category.
   * These defaults are used when the database is empty.
   */
  private initializeDefaultTraining(): void {
    // =========================================================================
    // OOO (Out of Office) - Auto-reply patterns
    // =========================================================================
    const oooExamples = [
      "I am currently out of the office and will return on Monday.",
      "Thank you for your email. I am on annual leave until next week.",
      "This is an automatic reply. I am away from email.",
      "I'm traveling and have limited access to email.",
      "Out of office: I will be back on the 15th.",
      "I'm currently out of the office with limited email access.",
      "Thank you for your message. I'm on vacation until January 10th.",
      "Auto-reply: Away from my desk. Will respond when I return.",
      "OOO: On parental leave until March. Contact support@company.com",
      "Automatic reply: I'm out sick today. Back tomorrow.",
      "Away message: Attending a conference this week. Back Monday.",
      "I'm at an offsite retreat with no email access. Back in 3 days.",
      "Vacation auto-reply: Will respond after January 2nd.",
      "I'm traveling with limited connectivity. Expect delayed responses.",
      "Currently on holiday. Returning to email Monday the 12th.",
      "I'm on PTO through end of week. Check back next Monday.",
      "This is an automated response. I'm away from the office.",
      "Hi! Thanks for your email. I'm on leave and will reply when back.",
      "I'm attending a training session and won't check email until Friday.",
      "Out of office auto-responder: Limited availability this week.",
    ];
    
    // =========================================================================
    // BOUNCE - Email delivery failures
    // =========================================================================
    const bounceExamples = [
      "Mail delivery failed: returning message to sender.",
      "Your message could not be delivered. User unknown.",
      "Undeliverable: 550 5.1.1 The email account does not exist.",
      "Mailbox unavailable. Message could not be delivered.",
      "Permanent failure: address rejected.",
      "Delivery Status Notification: Failure",
      "The email account does not exist at the organization.",
      "User not found in directory. Message undeliverable.",
      "Remote server returned: 550 Requested action not taken: mailbox unavailable",
      "Your message was not delivered because the address is invalid.",
      "Mailer-Daemon: Undeliverable mail returned to sender.",
      "550 5.1.1 User unknown at destination domain.",
      "The recipient's mailbox is full. Try again later.",
      "This email address is no longer active.",
      "Unable to deliver message to the following recipients.",
      "Message blocked: recipient email disabled.",
      "Your message was rejected by the mail server.",
      "Mailbox not found. Check the email address and try again.",
      "DNS lookup failed for domain. No MX records found.",
      "Connection to mail server failed. Permanent error.",
    ];
    
    // =========================================================================
    // INTERESTED - Positive buying signals
    // =========================================================================
    const interestedExamples = [
      "This sounds really interesting! Can you tell me more?",
      "I'd love to learn more about your platform.",
      "Could you send over some more information?",
      "This looks like exactly what we need. What's the pricing?",
      "Interesting! I'd like to hear more about how this works.",
      "Great timing! We were just discussing ad monetization. Let's talk.",
      "I saw your email and I'm curious about your fill rates.",
      "Yes I'm interested. Our app has 500k DAU and we want to improve ARPU.",
      "Sounds promising. What makes you different from ironSource?",
      "We should definitely talk. Our current eCPM is $2.50 - can you do better?",
      "I've forwarded this to our monetization team. They will be in touch!",
      "This is relevant to what we're building. Tell me more.",
      "We've been looking for a solution like this. How does it work?",
      "Intriguing! What kind of results have you seen with similar apps?",
      "I'm definitely open to exploring this further.",
      "Yes, we're actively evaluating new ad partners.",
      "Our revenue team is interested. Can you send a case study?",
      "Would love to understand more about your technology.",
      "This came at the right time. We're scaling monetization.",
      "Can you provide more details about integration requirements?",
    ];
    
    // =========================================================================
    // MEETING - Scheduling intent
    // =========================================================================
    const meetingExamples = [
      "Let's set up a call to discuss this further.",
      "I'm free Thursday afternoon for a quick chat.",
      "Can we schedule a demo next week?",
      "Would love to chat! Here's my calendar link.",
      "Let's book some time to talk through the details.",
      "I'm free Tuesday at 2pm. Does that work for a call?",
      "Let's set up a Zoom meeting. Send me a calendar invite.",
      "How about next week? I have availability Wednesday or Thursday.",
      "Can we do a 30 minute call? Here's my Calendly link.",
      "I'd like to schedule a demo. What times work on your end?",
      "Yes let's talk. I'm in PST - mornings work best.",
      "Book a meeting on my calendar: cal.com/developer",
      "I can do a quick call Friday. Send an invite.",
      "Let's meet next Monday. 10am works. Google Meet or Zoom?",
      "Happy to jump on a call. Send your meeting link.",
      "Sure, let's find time to connect. How's next Tuesday?",
      "Can you send a calendar invite for 3pm tomorrow?",
      "I've got 15 minutes free this afternoon if you want to chat.",
      "Let's do a quick video call to discuss. Send options.",
      "My assistant can schedule. Email schedule@company.com",
    ];
    
    // =========================================================================
    // NOT_INTERESTED - Polite declines
    // =========================================================================
    const notInterestedExamples = [
      "Thanks, but we're not interested at this time.",
      "We're happy with our current solution.",
      "Not a good fit for us right now.",
      "We'll pass on this, but thanks for reaching out.",
      "We're not in the market for this currently.",
      "Thanks but we're not looking for a new ad partner at this time.",
      "We already have an ad solution that works well for us.",
      "Our app doesn't monetize with ads. We use subscriptions only.",
      "We've evaluated mediation platforms before and decided against it.",
      "Thank you for thinking of us but we'll pass on this opportunity.",
      "Not for us right now. Maybe in the future.",
      "Our publisher team is focused on other priorities at the moment.",
      "Unfortunately this is not relevant for our business model.",
      "We're locked into a contract with our current provider.",
      "Appreciate the outreach but we're set for now.",
      "This doesn't align with our current roadmap.",
      "We've recently gone through this evaluation. Timing is off.",
      "Budget is already allocated elsewhere this quarter.",
      "Not something we're actively pursuing at the moment.",
      "We're focused on growth right now, not monetization changes.",
    ];
    
    // =========================================================================
    // UNSUBSCRIBE - Opt-out requests
    // =========================================================================
    const unsubscribeExamples = [
      "Please remove me from your mailing list.",
      "Unsubscribe me from these emails.",
      "Stop sending me these emails.",
      "Please don't contact me again.",
      "Take me off your list.",
      "UNSUBSCRIBE",
      "Remove my email immediately.",
      "I never signed up for this. Remove me.",
      "Do not contact me again. Remove from list.",
      "I want to opt out of these communications.",
      "How do I unsubscribe? Please stop sending emails.",
      "Remove. Unsubscribe. Do not email again.",
      "This is spam. I never gave you permission. Remove me now.",
      "Stop spamming me.",
      "Please stop. I'm not interested. Remove.",
      "I don't want to receive these messages anymore.",
      "Opt me out of all future communications.",
      "Remove this email address from your system.",
      "I'm marking this as spam unless you stop.",
      "Cease all contact. Unsubscribe.",
    ];
    
    // =========================================================================
    // SPAM - Irrelevant/automated garbage
    // =========================================================================
    const spamExamples = [
      "RE: Get rich quick with cryptocurrency! Limited time offer!",
      "You've won a prize! Click here to claim your reward now!",
      "This email has been blocked by our spam filter.",
      "Automatic response: Your email was marked as spam.",
      "Hot singles in your area want to meet you!",
      "Congratulations! You've been selected for a free iPhone!",
      "Act now! Limited time discount on weight loss pills!",
      "Make $10,000/week working from home!",
      "Your account has been compromised. Click here immediately.",
      "URGENT: Wire transfer needed for inheritance claim.",
      "Binary options trading - guaranteed profits!",
      "You have unclaimed lottery winnings waiting.",
      "Free credit score check! No credit card needed!",
      "Your computer has been infected. Download this now.",
      "Amazing investment opportunity - 500% returns guaranteed!",
      "This is an automated security alert from your bank.",
      "Cheap pharmacy meds shipped to your door!",
      "Enlarge your portfolio with crypto gains!",
      "You've been specially selected for this offer!",
      "Act fast before this opportunity expires!",
    ];
    
    // =========================================================================
    // UNKNOWN - Ambiguous messages needing human review
    // =========================================================================
    const unknownExamples = [
      "Hi",
      "Thanks",
      "Got it",
      "Ok",
      "Who is this?",
      "Wrong person.",
      "I didn't understand your email.",
      "?",
      "I'll think about it.",
      "Let me get back to you.",
      "Hm.",
      "Maybe",
      "I need to check with my team.",
      "Not sure what you're asking.",
      "Can you clarify?",
      "...",
      "K",
      "Will review",
      "Forwarding to the right person",
      "Noted.",
    ];
    
    // Train on all examples
    for (const text of oooExamples) this.classifier.train(text, 'OOO');
    for (const text of bounceExamples) this.classifier.train(text, 'BOUNCE');
    for (const text of interestedExamples) this.classifier.train(text, 'INTERESTED');
    for (const text of meetingExamples) this.classifier.train(text, 'MEETING');
    for (const text of notInterestedExamples) this.classifier.train(text, 'NOT_INTERESTED');
    for (const text of unsubscribeExamples) this.classifier.train(text, 'UNSUBSCRIBE');
    for (const text of spamExamples) this.classifier.train(text, 'SPAM');
    for (const text of unknownExamples) this.classifier.train(text, 'UNKNOWN');
    
    const totalExamples = oooExamples.length + bounceExamples.length + 
                          interestedExamples.length + meetingExamples.length +
                          notInterestedExamples.length + unsubscribeExamples.length +
                          spamExamples.length + unknownExamples.length;
    
    logger.info('Clerk initialized with comprehensive training data', {
      totalExamples,
      categories: {
        ooo: oooExamples.length,
        bounce: bounceExamples.length,
        interested: interestedExamples.length,
        meeting: meetingExamples.length,
        not_interested: notInterestedExamples.length,
        unsubscribe: unsubscribeExamples.length,
        spam: spamExamples.length,
        unknown: unknownExamples.length,
      },
    });
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
 * 
 * FIX: Auto-links to contact by matching fromEmail to sa_contacts.email
 * This enables the bandit learning loop to work properly
 */
export async function saveMessageClassification(params: {
  contactId?: string;
  fromEmail: string;
  subject: string;
  body: string;
  classification: ClassificationResult;
}): Promise<string> {
  // ====== AUTO-LINK: Try to find contact by email if not provided ======
  let resolvedContactId = params.contactId || null;
  
  if (!resolvedContactId && params.fromEmail) {
    try {
      const contactLookup = await queryReadOnly<{ id: string }>(`
        SELECT id FROM sa_contacts 
        WHERE LOWER(email) = LOWER($1)
        LIMIT 1
      `, [params.fromEmail]);
      
      if (contactLookup.length > 0) {
        resolvedContactId = contactLookup[0].id;
        logger.debug('Auto-linked inbox message to contact', { 
          fromEmail: params.fromEmail, 
          contactId: resolvedContactId 
        });
      }
    } catch (err) {
      // Don't fail if lookup fails
      logger.warn('Contact lookup failed', { error: (err as Error).message });
    }
  }
  
  // ====== ALSO: Link to original outbound message if reply ======
  let originalOutboundId: string | null = null;
  if (resolvedContactId) {
    try {
      const outboundLookup = await queryReadOnly<{ id: string }>(`
        SELECT id FROM sa_outbound_messages 
        WHERE contact_id = $1
        ORDER BY sent_at DESC NULLS LAST, created_at DESC
        LIMIT 1
      `, [resolvedContactId]);
      
      if (outboundLookup.length > 0) {
        originalOutboundId = outboundLookup[0].id;
      }
    } catch {
      // Don't fail if lookup fails
    }
  }
  
  const result = await queryOperator<{ id: string }>(`
    INSERT INTO sa_inbox_messages 
    (contact_id, from_email, subject, body_text, classification, classification_confidence, requires_human_review, original_outbound_id)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
    RETURNING id
  `, [
    resolvedContactId,
    params.fromEmail,
    params.subject,
    params.body,
    params.classification.category.toLowerCase(),
    Math.round(params.classification.confidence * 100),
    params.classification.requiresReview,
    originalOutboundId,
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
 * 
 * CLOSED LOOP: When a message is marked as INTERESTED or MEETING,
 * we record a SUCCESS for the subject line arm that generated it.
 * This feeds the Thompson Sampler and makes the system self-optimizing.
 */
export async function submitReview(
  messageId: string, 
  correctCategory: MessageCategory,
  clerk?: Clerk
): Promise<void> {
  // Update the message
  await queryOperator(`
    UPDATE sa_inbox_messages
    SET classification = $1, human_reviewed_at = CURRENT_TIMESTAMP
    WHERE id = $2
  `, [correctCategory.toLowerCase(), messageId]);
  
  // =========================================================================
  // CLOSED LOOP: Feed success back to Thompson Sampler
  // =========================================================================
  // If this was a positive response, record success for the subject line arm
  const positiveCategories: MessageCategory[] = ['INTERESTED', 'MEETING'];
  const negativeCategories: MessageCategory[] = ['NOT_INTERESTED', 'UNSUBSCRIBE', 'SPAM'];
  
  if (positiveCategories.includes(correctCategory) || negativeCategories.includes(correctCategory)) {
    // Look up the original outbound message that triggered this response
    // We join on contact_id AND ensure the outbound was sent BEFORE this inbox message was received
    // This prevents matching against later outbound messages sent to the same contact
    const originalMessages = await queryReadOnly<{
      subject_arm_id: string | null;
      contact_id: string | null;
      outbound_id: string | null;
    }>(`
      SELECT om.subject_arm_id, om.contact_id, om.id as outbound_id
      FROM sa_inbox_messages im
      JOIN sa_outbound_messages om ON im.contact_id = om.contact_id
      WHERE im.id = $1
        AND om.subject_arm_id IS NOT NULL
        AND (om.sent_at IS NULL OR om.sent_at <= im.received_at OR im.received_at IS NULL)
      ORDER BY om.sent_at DESC NULLS LAST, om.created_at DESC
      LIMIT 1
    `, [messageId]);
    
    if (originalMessages.length > 0 && originalMessages[0].subject_arm_id) {
      const { recordOutcome } = await import('../math/thompson-sampler.js');
      const isSuccess = positiveCategories.includes(correctCategory);
      
      await recordOutcome(originalMessages[0].subject_arm_id, isSuccess);
      
      logger.info('Closed loop: Recorded bandit outcome from inbox review', {
        messageId,
        category: correctCategory,
        armId: originalMessages[0].subject_arm_id,
        success: isSuccess,
      });
    }
  }
  
  // Get the message content for training
  const messages = await queryReadOnly<{ subject: string; body_text: string }>(`
    SELECT subject, body_text FROM sa_inbox_messages WHERE id = $1
  `, [messageId]);
  
  if (messages.length > 0) {
    const text = `${messages[0].subject} ${messages[0].body_text}`;
    
    // Save training data (column is 'label' not 'category')
    await queryOperator(`
      INSERT INTO sa_bayes_training (label, text_content, source)
      VALUES ($1, $2, 'manual')
    `, [correctCategory.toLowerCase(), text]);
    
    // If clerk instance provided, train it and persist the updated model
    if (clerk) {
      clerk.learn(text, correctCategory);
      await saveClassifierModel(clerk);
    }
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
 * Save classifier model state to database
 */
export async function saveClassifierModel(clerk: Clerk): Promise<void> {
  const modelState = clerk.exportModel();
  const stats = clerk.getStats();
  
  await queryOperator(`
    UPDATE sa_classifier_models
    SET 
      model_state = $1,
      model_version = model_version + 1,
      total_documents = $2,
      vocabulary_size = $3,
      category_counts = $4,
      trained_at = CURRENT_TIMESTAMP,
      updated_at = CURRENT_TIMESTAMP
    WHERE model_name = 'clerk_naive_bayes'
  `, [
    JSON.stringify(modelState),
    stats.totalDocuments,
    stats.vocabularySize,
    JSON.stringify(stats.categoryCounts),
  ]);
  
  logger.info('Saved Clerk classifier model to database', {
    totalDocuments: stats.totalDocuments,
    vocabularySize: stats.vocabularySize,
  });
}

/**
 * Load classifier model state from database
 */
export async function loadClassifierModel(): Promise<ReturnType<NaiveBayesClassifier['exportModel']> | null> {
  const rows = await queryReadOnly<{
    model_state: string;
    total_documents: number;
  }>(`
    SELECT model_state, total_documents
    FROM sa_classifier_models
    WHERE model_name = 'clerk_naive_bayes'
  `);
  
  if (rows.length === 0 || rows[0].total_documents === 0) {
    return null;
  }
  
  try {
    const state = typeof rows[0].model_state === 'string' 
      ? JSON.parse(rows[0].model_state)
      : rows[0].model_state;
    
    // Validate it has the expected structure
    if (state && state.wordCounts && state.categoryCounts) {
      return state;
    }
    return null;
  } catch {
    logger.warn('Failed to parse stored classifier model');
    return null;
  }
}

/**
 * Initialize Clerk from database
 * First tries to load saved model state, then falls back to training data
 */
export async function initializeClerkFromDatabase(): Promise<Clerk> {
  const clerk = new Clerk();
  
  // Try to load saved model state first (faster, preserves learned weights)
  const savedModel = await loadClassifierModel();
  
  if (savedModel) {
    clerk.importModel(savedModel);
    logger.info('Clerk loaded from saved model state', {
      totalDocuments: savedModel.totalDocuments,
      vocabularySize: savedModel.vocabularySize,
    });
    return clerk;
  }
  
  // Fall back to training from scratch using training data
  const trainingData = await loadTrainingData();
  
  for (const { category, text } of trainingData) {
    clerk.learn(text, category);
  }
  
  logger.info('Clerk trained from training data', { 
    examples: trainingData.length 
  });
  
  // Save the trained model for next time
  if (trainingData.length > 0) {
    await saveClassifierModel(clerk);
  }
  
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
