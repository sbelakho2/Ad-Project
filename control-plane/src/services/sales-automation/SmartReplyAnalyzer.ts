// services/sales-automation/SmartReplyAnalyzer.ts
// REVOLUTIONARY: Automatically categorize and prioritize replies
// OVERCOMES: "Fastest replies converted way better" - need to respond fast to interested leads
// OVERCOMES: "163 positive replies out of 523 total" - need to filter signal from noise
// Based on Reddit: "Speed mattered more than copy"
// Zero-cost: Pattern matching and keyword analysis, no AI APIs

import { Pool } from 'pg';

type ReplyCategory = 
  | 'interested'        // Ready to talk/buy
  | 'question'          // Has questions, needs nurturing
  | 'objection'         // Price, timing, competitor concerns
  | 'referral'          // "Talk to X instead"
  | 'not_interested'    // Clear no
  | 'out_of_office'     // Auto-reply
  | 'unsubscribe'       // Wants off list
  | 'wrong_person'      // Not the right contact
  | 'bounced'           // Delivery failure
  | 'neutral';          // Unclear intent

interface ReplyAnalysis {
  emailId: string;
  contactId: string;
  category: ReplyCategory;
  sentiment: 'positive' | 'negative' | 'neutral';
  urgency: 'high' | 'medium' | 'low';
  confidence: number;
  suggestedAction: string;
  keyPhrases: string[];
  responseDeadline?: Date;
}

interface ReplyStats {
  total: number;
  byCategory: Record<ReplyCategory, number>;
  avgResponseTime: number;
  positiveRate: number;
  interestRate: number;
}

// Pattern definitions for each category
const CATEGORY_PATTERNS: Record<ReplyCategory, {
  positive: string[];
  negative: string[];
  weight: number;
}> = {
  interested: {
    positive: [
      'yes', 'interested', 'tell me more', 'send me', 'schedule', 'let\'s talk',
      'set up a call', 'book a demo', 'sounds good', 'i\'d like to', 'i\'m interested',
      'love to learn', 'happy to chat', 'let\'s connect', 'when are you free',
      'this is relevant', 'perfect timing', 'we need this', 'been looking for',
    ],
    negative: [],
    weight: 10,
  },
  question: {
    positive: [
      'how does', 'what is', 'can you explain', 'wondering', 'curious about',
      'tell me about', 'how much', 'what\'s the cost', 'pricing', 'how long',
      'do you support', 'does it work with', 'integration', 'what results',
    ],
    negative: [],
    weight: 7,
  },
  objection: {
    positive: [
      'too expensive', 'not in budget', 'too busy', 'bad timing', 'already using',
      'using competitor', 'happy with current', 'not a priority', 'maybe later',
      'reach out next quarter', 'check back in', 'not right now', 'can\'t commit',
    ],
    negative: [],
    weight: 5,
  },
  referral: {
    positive: [
      'talk to', 'contact', 'reach out to', 'cc\'ing', 'copying', 'forwarding to',
      'the right person is', 'you should speak with', 'i\'ll introduce you to',
      'let me connect you', 'better suited to talk', 'handles this',
    ],
    negative: [],
    weight: 8,
  },
  not_interested: {
    positive: [
      'not interested', 'no thanks', 'no thank you', 'pass', 'not for us',
      'don\'t need', 'not relevant', 'doesn\'t apply', 'wrong fit', 'not a fit',
      'please remove', 'don\'t contact', 'stop emailing', 'not looking',
    ],
    negative: [],
    weight: 3,
  },
  out_of_office: {
    positive: [
      'out of office', 'out of the office', 'ooo', 'away from', 'on vacation',
      'on leave', 'limited access', 'automatic reply', 'auto-reply', 'autoresponder',
      'return on', 'back on', 'will reply when', 'delayed response',
    ],
    negative: [],
    weight: 2,
  },
  unsubscribe: {
    positive: [
      'unsubscribe', 'remove me', 'opt out', 'opt-out', 'stop sending',
      'take me off', 'remove from list', 'don\'t email', 'spam', 'reported',
    ],
    negative: [],
    weight: 1,
  },
  wrong_person: {
    positive: [
      'wrong person', 'not me', 'not my area', 'not my department', 
      'don\'t handle', 'i\'m not the', 'no longer at', 'left the company',
      'not responsible for', 'different team',
    ],
    negative: [],
    weight: 4,
  },
  bounced: {
    positive: [
      'undeliverable', 'delivery failed', 'address not found', 'user unknown',
      'mailbox not found', 'does not exist', 'rejected', 'permanent failure',
    ],
    negative: [],
    weight: 0,
  },
  neutral: {
    positive: [],
    negative: [],
    weight: 5,
  },
};

// Urgency indicators
const URGENCY_PATTERNS = {
  high: ['asap', 'urgent', 'immediately', 'right now', 'today', 'this week', 'soon'],
  low: ['maybe someday', 'in the future', 'down the road', 'eventually', 'no rush'],
};

/**
 * SmartReplyAnalyzer - Instant reply categorization for fast follow-up
 * REVOLUTIONARY: Auto-prioritize inbox so you respond to hot leads first
 */
export class SmartReplyAnalyzer {
  private pool: Pool;

  constructor(databaseUrlOrPool: string | Pool) {
    if (typeof databaseUrlOrPool === 'string') {
      this.pool = new Pool({ connectionString: databaseUrlOrPool });
    } else {
      this.pool = databaseUrlOrPool;
    }
  }

  /**
   * Initialize required tables
   */
  async initialize(): Promise<void> {
    await this.pool.query(`
      CREATE TABLE IF NOT EXISTS reply_analysis (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        email_id UUID,
        contact_id UUID REFERENCES sales_contacts(id),
        category VARCHAR(50) NOT NULL,
        sentiment VARCHAR(20) NOT NULL,
        urgency VARCHAR(20) NOT NULL,
        confidence INTEGER NOT NULL,
        suggested_action TEXT,
        key_phrases TEXT[],
        response_deadline TIMESTAMPTZ,
        analyzed_at TIMESTAMPTZ DEFAULT NOW(),
        responded_at TIMESTAMPTZ,
        response_time_minutes INTEGER
      );

      CREATE INDEX IF NOT EXISTS idx_reply_analysis_contact ON reply_analysis(contact_id);
      CREATE INDEX IF NOT EXISTS idx_reply_analysis_category ON reply_analysis(category, analyzed_at DESC);
      CREATE INDEX IF NOT EXISTS idx_reply_analysis_urgency ON reply_analysis(urgency, responded_at);
    `);
  }

  /**
   * MAIN: Analyze a reply and categorize it
   * This should be called immediately when a reply is received
   */
  analyzeReply(
    emailId: string,
    contactId: string,
    subject: string,
    body: string
  ): ReplyAnalysis {
    const normalizedBody = body.toLowerCase();
    const normalizedSubject = subject.toLowerCase();
    const fullText = `${normalizedSubject} ${normalizedBody}`;

    // 1. Detect category
    const categoryScores = this.calculateCategoryScores(fullText);
    const category = this.determineCategory(categoryScores);

    // 2. Detect sentiment
    const sentiment = this.analyzeSentiment(fullText, category);

    // 3. Detect urgency
    const urgency = this.detectUrgency(fullText, category);

    // 4. Extract key phrases
    const keyPhrases = this.extractKeyPhrases(body);

    // 5. Calculate confidence
    const confidence = this.calculateConfidence(categoryScores, category);

    // 6. Generate suggested action
    const suggestedAction = this.getSuggestedAction(category, sentiment, urgency);

    // 7. Set response deadline
    const responseDeadline = this.calculateResponseDeadline(urgency, category);

    return {
      emailId,
      contactId,
      category,
      sentiment,
      urgency,
      confidence,
      suggestedAction,
      keyPhrases,
      responseDeadline,
    };
  }

  /**
   * Analyze and store a reply
   */
  async analyzeAndStore(
    emailId: string,
    contactId: string,
    subject: string,
    body: string
  ): Promise<ReplyAnalysis> {
    const analysis = this.analyzeReply(emailId, contactId, subject, body);

    await this.pool.query(`
      INSERT INTO reply_analysis 
        (email_id, contact_id, category, sentiment, urgency, confidence, suggested_action, key_phrases, response_deadline)
      VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
    `, [
      analysis.emailId,
      analysis.contactId,
      analysis.category,
      analysis.sentiment,
      analysis.urgency,
      analysis.confidence,
      analysis.suggestedAction,
      analysis.keyPhrases,
      analysis.responseDeadline,
    ]);

    // Update contact status based on analysis
    await this.updateContactStatus(contactId, analysis);

    return analysis;
  }

  /**
   * Get prioritized reply queue
   * REVOLUTIONARY: Shows hot replies first
   */
  async getPrioritizedReplies(limit: number = 50): Promise<Array<ReplyAnalysis & {
    contactEmail: string;
    contactName: string;
    receivedAt: Date;
    waitingTime: number; // minutes
  }>> {
    const result = await this.pool.query(`
      SELECT 
        ra.*,
        c.email as contact_email,
        c.first_name || ' ' || c.last_name as contact_name,
        e.replied_at as received_at,
        EXTRACT(EPOCH FROM (NOW() - e.replied_at))/60 as waiting_time
      FROM reply_analysis ra
      JOIN sales_contacts c ON ra.contact_id = c.id
      JOIN outreach_emails e ON ra.email_id = e.id
      WHERE ra.responded_at IS NULL
      ORDER BY 
        CASE ra.urgency 
          WHEN 'high' THEN 1 
          WHEN 'medium' THEN 2 
          ELSE 3 
        END,
        CASE ra.category 
          WHEN 'interested' THEN 1 
          WHEN 'referral' THEN 2 
          WHEN 'question' THEN 3 
          ELSE 4 
        END,
        ra.analyzed_at ASC
      LIMIT $1
    `, [limit]);

    return result.rows.map(row => ({
      emailId: row.email_id,
      contactId: row.contact_id,
      category: row.category,
      sentiment: row.sentiment,
      urgency: row.urgency,
      confidence: row.confidence,
      suggestedAction: row.suggested_action,
      keyPhrases: row.key_phrases || [],
      responseDeadline: row.response_deadline,
      contactEmail: row.contact_email,
      contactName: row.contact_name,
      receivedAt: row.received_at,
      waitingTime: Math.round(parseFloat(row.waiting_time) || 0),
    }));
  }

  /**
   * Get hot leads - interested replies needing immediate attention
   */
  async getHotLeads(): Promise<Array<{
    contactId: string;
    contactEmail: string;
    category: ReplyCategory;
    waitingMinutes: number;
    suggestedAction: string;
  }>> {
    const result = await this.pool.query(`
      SELECT 
        ra.contact_id,
        c.email as contact_email,
        ra.category,
        EXTRACT(EPOCH FROM (NOW() - ra.analyzed_at))/60 as waiting_minutes,
        ra.suggested_action
      FROM reply_analysis ra
      JOIN sales_contacts c ON ra.contact_id = c.id
      WHERE ra.responded_at IS NULL
        AND ra.category IN ('interested', 'referral', 'question')
        AND ra.urgency IN ('high', 'medium')
      ORDER BY waiting_minutes DESC
      LIMIT 20
    `);

    return result.rows.map(row => ({
      contactId: row.contact_id,
      contactEmail: row.contact_email,
      category: row.category as ReplyCategory,
      waitingMinutes: Math.round(parseFloat(row.waiting_minutes) || 0),
      suggestedAction: row.suggested_action,
    }));
  }

  /**
   * Mark reply as responded
   */
  async markResponded(emailId: string): Promise<void> {
    const result = await this.pool.query(`
      UPDATE reply_analysis 
      SET 
        responded_at = NOW(),
        response_time_minutes = EXTRACT(EPOCH FROM (NOW() - analyzed_at))/60
      WHERE email_id = $1
      RETURNING response_time_minutes
    `, [emailId]);

    // Log response time for learning
    if (result.rows[0]) {
      const responseTime = result.rows[0].response_time_minutes;
      if (responseTime > 60) {
        console.warn(`[SmartReplyAnalyzer] Response time ${responseTime} minutes - aim for <60 minutes`);
      }
    }
  }

  /**
   * Get reply statistics
   */
  async getReplyStats(days: number = 30): Promise<ReplyStats> {
    const since = new Date(Date.now() - days * 24 * 60 * 60 * 1000);

    const result = await this.pool.query(`
      SELECT 
        category,
        COUNT(*) as count,
        AVG(response_time_minutes) as avg_response_time
      FROM reply_analysis
      WHERE analyzed_at > $1
      GROUP BY category
    `, [since]);

    const byCategory: Record<ReplyCategory, number> = {
      interested: 0, question: 0, objection: 0, referral: 0,
      not_interested: 0, out_of_office: 0, unsubscribe: 0,
      wrong_person: 0, bounced: 0, neutral: 0,
    };

    let total = 0;
    let totalResponseTime = 0;

    for (const row of result.rows) {
      byCategory[row.category as ReplyCategory] = parseInt(row.count);
      total += parseInt(row.count);
      if (row.avg_response_time) {
        totalResponseTime += parseFloat(row.avg_response_time) * parseInt(row.count);
      }
    }

    const positiveCount = byCategory.interested + byCategory.question + byCategory.referral;

    return {
      total,
      byCategory,
      avgResponseTime: total > 0 ? totalResponseTime / total : 0,
      positiveRate: total > 0 ? (positiveCount / total) * 100 : 0,
      interestRate: total > 0 ? (byCategory.interested / total) * 100 : 0,
    };
  }

  /**
   * Analyze all unanalyzed replies (catch-up)
   */
  async analyzeUnprocessedReplies(): Promise<number> {
    const result = await this.pool.query(`
      SELECT 
        e.id as email_id,
        e.contact_id,
        e.subject,
        e.reply_body as body
      FROM outreach_emails e
      LEFT JOIN reply_analysis ra ON e.id = ra.email_id
      WHERE e.replied_at IS NOT NULL 
        AND e.reply_body IS NOT NULL
        AND ra.id IS NULL
      LIMIT 100
    `);

    for (const row of result.rows) {
      await this.analyzeAndStore(
        row.email_id,
        row.contact_id,
        row.subject || '',
        row.body || ''
      );
    }

    return result.rows.length;
  }

  // ===== Private Methods =====

  private calculateCategoryScores(text: string): Map<ReplyCategory, number> {
    const scores = new Map<ReplyCategory, number>();

    for (const [category, patterns] of Object.entries(CATEGORY_PATTERNS)) {
      let score = 0;

      for (const pattern of patterns.positive) {
        if (text.includes(pattern.toLowerCase())) {
          score += patterns.weight;
        }
      }

      scores.set(category as ReplyCategory, score);
    }

    return scores;
  }

  private determineCategory(scores: Map<ReplyCategory, number>): ReplyCategory {
    let maxScore = 0;
    let bestCategory: ReplyCategory = 'neutral';

    for (const [category, score] of scores) {
      if (score > maxScore) {
        maxScore = score;
        bestCategory = category;
      }
    }

    return bestCategory;
  }

  private analyzeSentiment(text: string, category: ReplyCategory): 'positive' | 'negative' | 'neutral' {
    // Category-based sentiment
    const positiveCategories: ReplyCategory[] = ['interested', 'question', 'referral'];
    const negativeCategories: ReplyCategory[] = ['not_interested', 'unsubscribe', 'bounced'];

    if (positiveCategories.includes(category)) return 'positive';
    if (negativeCategories.includes(category)) return 'negative';

    // Text-based sentiment for neutral categories
    const positiveWords = ['thanks', 'thank you', 'appreciate', 'great', 'good', 'helpful'];
    const negativeWords = ['unfortunately', 'sorry', 'can\'t', 'won\'t', 'don\'t'];

    const positiveCount = positiveWords.filter(w => text.includes(w)).length;
    const negativeCount = negativeWords.filter(w => text.includes(w)).length;

    if (positiveCount > negativeCount) return 'positive';
    if (negativeCount > positiveCount) return 'negative';
    return 'neutral';
  }

  private detectUrgency(text: string, category: ReplyCategory): 'high' | 'medium' | 'low' {
    // Interested replies are always high urgency
    if (category === 'interested') return 'high';

    // Check for urgency patterns
    for (const pattern of URGENCY_PATTERNS.high) {
      if (text.includes(pattern)) return 'high';
    }

    for (const pattern of URGENCY_PATTERNS.low) {
      if (text.includes(pattern)) return 'low';
    }

    // Questions and referrals are medium urgency
    if (category === 'question' || category === 'referral') return 'medium';

    return 'low';
  }

  private extractKeyPhrases(body: string): string[] {
    const phrases: string[] = [];
    
    // Extract potential key phrases (simple extraction)
    const sentences = body.split(/[.!?]+/);
    
    for (const sentence of sentences) {
      const trimmed = sentence.trim();
      if (trimmed.length > 10 && trimmed.length < 100) {
        // Look for sentences with questions or strong statements
        if (trimmed.includes('?') || 
            trimmed.toLowerCase().startsWith('i ') ||
            trimmed.toLowerCase().startsWith('we ') ||
            trimmed.toLowerCase().includes('interested')) {
          phrases.push(trimmed);
        }
      }
    }

    return phrases.slice(0, 3); // Return top 3 phrases
  }

  private calculateConfidence(scores: Map<ReplyCategory, number>, category: ReplyCategory): number {
    const categoryScore = scores.get(category) || 0;
    const totalScore = Array.from(scores.values()).reduce((a, b) => a + b, 0);

    if (totalScore === 0) return 50; // Neutral confidence

    // Confidence is how dominant the winning category is
    const confidence = (categoryScore / totalScore) * 100;
    
    // Minimum confidence based on absolute score
    const minConfidence = Math.min(categoryScore * 5, 30);
    
    return Math.round(Math.max(confidence, minConfidence));
  }

  private getSuggestedAction(
    category: ReplyCategory,
    sentiment: string,
    urgency: string
  ): string {
    const actions: Record<ReplyCategory, string> = {
      interested: '🔥 RESPOND IMMEDIATELY - Schedule call/demo within 24 hours',
      question: 'Answer questions clearly, include relevant case study',
      objection: 'Address objection with specific counter-point or offer alternative',
      referral: 'Thank them, reach out to referred contact mentioning this intro',
      not_interested: 'Mark as closed-lost, add to long-term nurture',
      out_of_office: 'Set reminder to follow up after their return date',
      unsubscribe: 'Remove from all sequences immediately, mark as unsubscribed',
      wrong_person: 'Ask for referral to correct contact, update record',
      bounced: 'Verify email, find alternative address or remove contact',
      neutral: 'Review manually, may need human judgment',
    };

    let action = actions[category];

    // Add urgency context
    if (urgency === 'high' && !action.includes('IMMEDIATELY')) {
      action = `⚡ URGENT: ${action}`;
    }

    return action;
  }

  private calculateResponseDeadline(urgency: string, category: ReplyCategory): Date {
    const now = new Date();
    
    // Deadline in hours based on urgency and category
    const deadlines: Record<string, number> = {
      'interested_high': 2,
      'interested_medium': 4,
      'interested_low': 8,
      'question_high': 4,
      'question_medium': 8,
      'referral_high': 4,
      'referral_medium': 12,
      'default': 24,
    };

    const key = `${category}_${urgency}`;
    const hoursToDeadline = deadlines[key] || deadlines.default;

    return new Date(now.getTime() + hoursToDeadline * 60 * 60 * 1000);
  }

  private async updateContactStatus(contactId: string, analysis: ReplyAnalysis): Promise<void> {
    let newStatus: string | null = null;

    switch (analysis.category) {
      case 'interested':
        newStatus = 'hot_lead';
        break;
      case 'not_interested':
        newStatus = 'closed_lost';
        break;
      case 'unsubscribe':
        newStatus = 'unsubscribed';
        break;
      case 'bounced':
        newStatus = 'invalid_email';
        break;
    }

    if (newStatus) {
      await this.pool.query(`
        UPDATE sales_contacts 
        SET status = $1, updated_at = NOW()
        WHERE id = $2
      `, [newStatus, contactId]);
    }

    // Log activity
    await this.pool.query(`
      INSERT INTO sales_activities (contact_id, activity_type, details, created_at)
      VALUES ($1, 'email_replied', $2, NOW())
    `, [contactId, JSON.stringify({
      category: analysis.category,
      sentiment: analysis.sentiment,
      urgency: analysis.urgency,
    })]);
  }
}
