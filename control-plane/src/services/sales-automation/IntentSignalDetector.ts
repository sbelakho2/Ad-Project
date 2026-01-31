// services/sales-automation/IntentSignalDetector.ts
// REVOLUTIONARY: Detect buying intent BEFORE outreach - prioritize leads showing signals
// OVERCOMES: "Targeting has gotten lazy" - match ICP but no real urgency
// OVERCOMES: "Most people are guessing where to draw the line"
// Based on Reddit: "Most leads came from follow-ups, not first email"
// Based on Reddit: Finding the right timing is everything
// Zero-cost: Track signals from engagement patterns, no external APIs

import { Pool } from 'pg';

type IntentSignal = 
  | 'website_visit'           // Visited our website
  | 'pricing_page'            // Viewed pricing (high intent)
  | 'multiple_visits'         // Multiple website visits
  | 'content_download'        // Downloaded resource
  | 'email_click'             // Clicked link in email
  | 'email_multiple_opens'    // Opened email multiple times
  | 'fast_reply'              // Replied quickly (<24h)
  | 'follow_up_reply'         // Replied on follow-up (not first email)
  | 'question_asked'          // Asked a question (engagement)
  | 'social_engagement'       // Engaged on social media
  | 'company_growth'          // Company showing growth signals
  | 'recent_funding'          // Recently raised money
  | 'job_posting'             // Hiring for relevant roles
  | 'tech_adoption'           // Recently adopted relevant tech
  | 'competitor_mention'      // Mentioned competitor
  | 'pain_point_mention';     // Mentioned relevant pain point

interface IntentProfile {
  contactId: string;
  signals: SignalRecord[];
  intentScore: number; // 0-100
  intentLevel: 'hot' | 'warm' | 'cool' | 'cold';
  buyingStage: 'awareness' | 'consideration' | 'decision' | 'unknown';
  nextBestAction: string;
  optimalOutreachTime?: Date;
}

interface SignalRecord {
  signal: IntentSignal;
  strength: number; // 1-10
  detectedAt: Date;
  source: string;
  details?: string;
}

// Signal weights for intent scoring
const SIGNAL_WEIGHTS: Record<IntentSignal, number> = {
  pricing_page: 25,
  fast_reply: 20,
  email_click: 15,
  email_multiple_opens: 12,
  follow_up_reply: 18,
  question_asked: 15,
  multiple_visits: 10,
  website_visit: 5,
  content_download: 12,
  recent_funding: 20,
  company_growth: 15,
  job_posting: 10,
  tech_adoption: 8,
  competitor_mention: 15,
  pain_point_mention: 18,
  social_engagement: 5,
};

// Signal decay (signals become less relevant over time)
const SIGNAL_DECAY_DAYS: Record<IntentSignal, number> = {
  pricing_page: 14,
  fast_reply: 7,
  email_click: 14,
  email_multiple_opens: 7,
  follow_up_reply: 14,
  question_asked: 21,
  multiple_visits: 14,
  website_visit: 7,
  content_download: 30,
  recent_funding: 90,
  company_growth: 60,
  job_posting: 30,
  tech_adoption: 60,
  competitor_mention: 30,
  pain_point_mention: 21,
  social_engagement: 7,
};

/**
 * IntentSignalDetector - Detect and score buying intent
 * REVOLUTIONARY: Know who's ready to buy BEFORE you reach out
 */
export class IntentSignalDetector {
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
      CREATE TABLE IF NOT EXISTS intent_signals (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        contact_id UUID REFERENCES sales_contacts(id),
        signal_type VARCHAR(50) NOT NULL,
        strength INTEGER NOT NULL DEFAULT 5,
        source VARCHAR(100),
        details TEXT,
        detected_at TIMESTAMPTZ DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS intent_scores (
        contact_id UUID PRIMARY KEY REFERENCES sales_contacts(id),
        intent_score INTEGER NOT NULL DEFAULT 0,
        intent_level VARCHAR(20) NOT NULL DEFAULT 'cold',
        buying_stage VARCHAR(20) NOT NULL DEFAULT 'unknown',
        last_signal_at TIMESTAMPTZ,
        calculated_at TIMESTAMPTZ DEFAULT NOW()
      );

      CREATE INDEX IF NOT EXISTS idx_intent_signals_contact ON intent_signals(contact_id, detected_at DESC);
      CREATE INDEX IF NOT EXISTS idx_intent_scores_level ON intent_scores(intent_level, intent_score DESC);
    `);
  }

  /**
   * MAIN: Record a new intent signal
   */
  async recordSignal(
    contactId: string,
    signal: IntentSignal,
    source: string,
    details?: string,
    strength: number = 5
  ): Promise<void> {
    await this.pool.query(`
      INSERT INTO intent_signals (contact_id, signal_type, strength, source, details)
      VALUES ($1, $2, $3, $4, $5)
    `, [contactId, signal, strength, source, details]);

    // Recalculate intent score
    await this.calculateIntentScore(contactId);
  }

  /**
   * Detect signals from email engagement
   * Call this after email events (open, click, reply)
   */
  async detectEmailSignals(
    contactId: string,
    event: 'open' | 'click' | 'reply',
    emailNumber: number,
    responseTimeHours?: number
  ): Promise<SignalRecord[]> {
    const signals: SignalRecord[] = [];

    if (event === 'click') {
      const signal: SignalRecord = {
        signal: 'email_click',
        strength: 8,
        detectedAt: new Date(),
        source: 'email_tracking',
        details: `Clicked link in email #${emailNumber}`,
      };
      await this.recordSignal(contactId, 'email_click', 'email_tracking', signal.details);
      signals.push(signal);
    }

    if (event === 'reply') {
      // Fast reply signal
      if (responseTimeHours !== undefined && responseTimeHours < 24) {
        const signal: SignalRecord = {
          signal: 'fast_reply',
          strength: 10,
          detectedAt: new Date(),
          source: 'email_tracking',
          details: `Replied within ${Math.round(responseTimeHours)} hours`,
        };
        await this.recordSignal(contactId, 'fast_reply', 'email_tracking', signal.details, 10);
        signals.push(signal);
      }

      // Follow-up reply signal (not first email)
      if (emailNumber > 1) {
        const signal: SignalRecord = {
          signal: 'follow_up_reply',
          strength: 8,
          detectedAt: new Date(),
          source: 'email_tracking',
          details: `Replied to follow-up email #${emailNumber}`,
        };
        await this.recordSignal(contactId, 'follow_up_reply', 'email_tracking', signal.details);
        signals.push(signal);
      }
    }

    if (event === 'open') {
      // Check for multiple opens
      const openCount = await this.getRecentOpenCount(contactId);
      if (openCount >= 3) {
        const signal: SignalRecord = {
          signal: 'email_multiple_opens',
          strength: 7,
          detectedAt: new Date(),
          source: 'email_tracking',
          details: `Opened emails ${openCount} times in last 7 days`,
        };
        await this.recordSignal(contactId, 'email_multiple_opens', 'email_tracking', signal.details);
        signals.push(signal);
      }
    }

    return signals;
  }

  /**
   * Detect signals from website visits
   * Call this from website tracking
   */
  async detectWebsiteSignals(
    contactId: string,
    page: string,
    visitCount: number
  ): Promise<SignalRecord[]> {
    const signals: SignalRecord[] = [];

    // Basic visit signal
    const visitSignal: SignalRecord = {
      signal: 'website_visit',
      strength: 3,
      detectedAt: new Date(),
      source: 'website_tracking',
      details: `Visited ${page}`,
    };
    await this.recordSignal(contactId, 'website_visit', 'website_tracking', visitSignal.details, 3);
    signals.push(visitSignal);

    // High-intent pricing page
    if (page.toLowerCase().includes('pricing')) {
      const pricingSignal: SignalRecord = {
        signal: 'pricing_page',
        strength: 10,
        detectedAt: new Date(),
        source: 'website_tracking',
        details: 'Viewed pricing page - HIGH INTENT',
      };
      await this.recordSignal(contactId, 'pricing_page', 'website_tracking', pricingSignal.details, 10);
      signals.push(pricingSignal);
    }

    // Multiple visits
    if (visitCount >= 3) {
      const multiVisit: SignalRecord = {
        signal: 'multiple_visits',
        strength: 7,
        detectedAt: new Date(),
        source: 'website_tracking',
        details: `${visitCount} total visits`,
      };
      await this.recordSignal(contactId, 'multiple_visits', 'website_tracking', multiVisit.details, 7);
      signals.push(multiVisit);
    }

    return signals;
  }

  /**
   * Detect company-level signals
   * For enrichment from company data
   */
  async detectCompanySignals(
    contactId: string,
    companyData: {
      recentFunding?: { amount: number; date: Date };
      employeeGrowth?: number; // percentage
      jobPostings?: string[];
      techStack?: string[];
    }
  ): Promise<SignalRecord[]> {
    const signals: SignalRecord[] = [];

    if (companyData.recentFunding) {
      const signal: SignalRecord = {
        signal: 'recent_funding',
        strength: 10,
        detectedAt: new Date(),
        source: 'enrichment',
        details: `Raised $${(companyData.recentFunding.amount / 1000000).toFixed(1)}M`,
      };
      await this.recordSignal(contactId, 'recent_funding', 'enrichment', signal.details, 10);
      signals.push(signal);
    }

    if (companyData.employeeGrowth && companyData.employeeGrowth > 20) {
      const signal: SignalRecord = {
        signal: 'company_growth',
        strength: 7,
        detectedAt: new Date(),
        source: 'enrichment',
        details: `${companyData.employeeGrowth}% employee growth`,
      };
      await this.recordSignal(contactId, 'company_growth', 'enrichment', signal.details, 7);
      signals.push(signal);
    }

    if (companyData.jobPostings && companyData.jobPostings.length > 0) {
      // Check for relevant job postings (marketing, growth, mobile, etc.)
      const relevantKeywords = ['marketing', 'growth', 'mobile', 'advertising', 'monetization'];
      const relevantJobs = companyData.jobPostings.filter(job => 
        relevantKeywords.some(k => job.toLowerCase().includes(k))
      );

      if (relevantJobs.length > 0) {
        const signal: SignalRecord = {
          signal: 'job_posting',
          strength: 6,
          detectedAt: new Date(),
          source: 'enrichment',
          details: `Hiring: ${relevantJobs.slice(0, 3).join(', ')}`,
        };
        await this.recordSignal(contactId, 'job_posting', 'enrichment', signal.details, 6);
        signals.push(signal);
      }
    }

    return signals;
  }

  /**
   * Get intent profile for a contact
   */
  async getIntentProfile(contactId: string): Promise<IntentProfile> {
    // Get all signals
    const signalsResult = await this.pool.query(`
      SELECT * FROM intent_signals
      WHERE contact_id = $1
      ORDER BY detected_at DESC
      LIMIT 50
    `, [contactId]);

    const signals: SignalRecord[] = signalsResult.rows.map(row => ({
      signal: row.signal_type as IntentSignal,
      strength: row.strength,
      detectedAt: row.detected_at,
      source: row.source,
      details: row.details,
    }));

    // Get cached score
    const scoreResult = await this.pool.query(`
      SELECT * FROM intent_scores WHERE contact_id = $1
    `, [contactId]);

    const score = scoreResult.rows[0] || { intent_score: 0, intent_level: 'cold', buying_stage: 'unknown' };

    // Calculate next best action
    const nextBestAction = this.determineNextAction(signals, score.intent_level);

    // Calculate optimal outreach time
    const optimalTime = await this.calculateOptimalOutreachTime(contactId, signals);

    return {
      contactId,
      signals,
      intentScore: score.intent_score,
      intentLevel: score.intent_level,
      buyingStage: score.buying_stage,
      nextBestAction,
      optimalOutreachTime: optimalTime,
    };
  }

  /**
   * Get high-intent contacts ready for outreach
   */
  async getHighIntentContacts(limit: number = 50): Promise<Array<{
    contactId: string;
    email: string;
    name: string;
    intentScore: number;
    intentLevel: string;
    topSignals: string[];
    nextAction: string;
  }>> {
    const result = await this.pool.query(`
      SELECT 
        is2.contact_id,
        c.email,
        c.first_name || ' ' || c.last_name as name,
        is2.intent_score,
        is2.intent_level,
        is2.buying_stage
      FROM intent_scores is2
      JOIN sales_contacts c ON is2.contact_id = c.id
      WHERE is2.intent_level IN ('hot', 'warm')
        AND c.status NOT IN ('unsubscribed', 'bounced', 'closed_lost')
      ORDER BY is2.intent_score DESC
      LIMIT $1
    `, [limit]);

    const contacts: Array<{
      contactId: string;
      email: string;
      name: string;
      intentScore: number;
      intentLevel: string;
      topSignals: string[];
      nextAction: string;
    }> = [];

    for (const row of result.rows) {
      const profile = await this.getIntentProfile(row.contact_id);
      
      contacts.push({
        contactId: row.contact_id,
        email: row.email,
        name: row.name,
        intentScore: row.intent_score,
        intentLevel: row.intent_level,
        topSignals: profile.signals.slice(0, 3).map(s => s.signal),
        nextAction: profile.nextBestAction,
      });
    }

    return contacts;
  }

  /**
   * Get contacts with rising intent (engagement velocity)
   */
  async getRisingIntentContacts(): Promise<Array<{
    contactId: string;
    email: string;
    signalsLast7Days: number;
    signalsPrevious7Days: number;
    velocity: number;
  }>> {
    const result = await this.pool.query(`
      WITH recent AS (
        SELECT contact_id, COUNT(*) as count
        FROM intent_signals
        WHERE detected_at > NOW() - INTERVAL '7 days'
        GROUP BY contact_id
      ),
      previous AS (
        SELECT contact_id, COUNT(*) as count
        FROM intent_signals
        WHERE detected_at > NOW() - INTERVAL '14 days'
          AND detected_at <= NOW() - INTERVAL '7 days'
        GROUP BY contact_id
      )
      SELECT 
        r.contact_id,
        c.email,
        r.count as recent_count,
        COALESCE(p.count, 0) as previous_count,
        (r.count - COALESCE(p.count, 0))::float / GREATEST(COALESCE(p.count, 1), 1) as velocity
      FROM recent r
      LEFT JOIN previous p ON r.contact_id = p.contact_id
      JOIN sales_contacts c ON r.contact_id = c.id
      WHERE r.count > COALESCE(p.count, 0)
      ORDER BY velocity DESC
      LIMIT 20
    `);

    return result.rows.map(row => ({
      contactId: row.contact_id,
      email: row.email,
      signalsLast7Days: parseInt(row.recent_count),
      signalsPrevious7Days: parseInt(row.previous_count),
      velocity: parseFloat(row.velocity),
    }));
  }

  /**
   * Calculate and update intent score for a contact
   */
  private async calculateIntentScore(contactId: string): Promise<void> {
    const signalsResult = await this.pool.query(`
      SELECT signal_type, strength, detected_at
      FROM intent_signals
      WHERE contact_id = $1
    `, [contactId]);

    let totalScore = 0;
    let recentSignals = 0;
    let highIntentSignals = 0;
    const now = Date.now();

    for (const row of signalsResult.rows) {
      const signal = row.signal_type as IntentSignal;
      const baseWeight = SIGNAL_WEIGHTS[signal] || 5;
      const decayDays = SIGNAL_DECAY_DAYS[signal] || 14;
      
      // Calculate decay
      const signalAge = (now - new Date(row.detected_at).getTime()) / (24 * 60 * 60 * 1000);
      const decayFactor = Math.max(0, 1 - (signalAge / decayDays));
      
      const adjustedScore = baseWeight * decayFactor * (row.strength / 5);
      totalScore += adjustedScore;

      if (signalAge < 7) recentSignals++;
      if (['pricing_page', 'fast_reply', 'recent_funding'].includes(signal)) highIntentSignals++;
    }

    // Normalize to 0-100
    const normalizedScore = Math.min(100, Math.round(totalScore));

    // Determine intent level
    let intentLevel: 'hot' | 'warm' | 'cool' | 'cold';
    if (normalizedScore >= 70 || highIntentSignals >= 2) intentLevel = 'hot';
    else if (normalizedScore >= 40) intentLevel = 'warm';
    else if (normalizedScore >= 20) intentLevel = 'cool';
    else intentLevel = 'cold';

    // Determine buying stage
    let buyingStage: 'awareness' | 'consideration' | 'decision' | 'unknown';
    const hasClickedOrReplied = signalsResult.rows.some(r => 
      ['email_click', 'fast_reply', 'follow_up_reply'].includes(r.signal_type)
    );
    const hasPricingVisit = signalsResult.rows.some(r => r.signal_type === 'pricing_page');

    if (hasPricingVisit) buyingStage = 'decision';
    else if (hasClickedOrReplied) buyingStage = 'consideration';
    else if (recentSignals > 0) buyingStage = 'awareness';
    else buyingStage = 'unknown';

    // Upsert score
    await this.pool.query(`
      INSERT INTO intent_scores (contact_id, intent_score, intent_level, buying_stage, last_signal_at, calculated_at)
      VALUES ($1, $2, $3, $4, NOW(), NOW())
      ON CONFLICT (contact_id) DO UPDATE SET
        intent_score = $2,
        intent_level = $3,
        buying_stage = $4,
        last_signal_at = NOW(),
        calculated_at = NOW()
    `, [contactId, normalizedScore, intentLevel, buyingStage]);
  }

  private async getRecentOpenCount(contactId: string): Promise<number> {
    const result = await this.pool.query(`
      SELECT COUNT(*) as count 
      FROM outreach_emails
      WHERE contact_id = $1 
        AND opened_at IS NOT NULL
        AND opened_at > NOW() - INTERVAL '7 days'
    `, [contactId]);

    return parseInt(result.rows[0]?.count) || 0;
  }

  private determineNextAction(signals: SignalRecord[], intentLevel: string): string {
    const hasReplied = signals.some(s => 
      ['fast_reply', 'follow_up_reply', 'question_asked'].includes(s.signal)
    );
    const hasPricingVisit = signals.some(s => s.signal === 'pricing_page');
    const hasClicked = signals.some(s => s.signal === 'email_click');

    if (hasReplied) {
      return 'RESPOND IMMEDIATELY - They engaged, strike while hot';
    }

    if (hasPricingVisit) {
      return 'Send pricing-focused email with ROI calculator or case study';
    }

    if (hasClicked && intentLevel === 'hot') {
      return 'Send follow-up referencing what they clicked on';
    }

    if (intentLevel === 'hot') {
      return 'Call or send personalized video message';
    }

    if (intentLevel === 'warm') {
      return 'Send relevant case study or value-focused content';
    }

    return 'Continue nurture sequence with educational content';
  }

  private async calculateOptimalOutreachTime(
    contactId: string, 
    signals: SignalRecord[]
  ): Promise<Date | undefined> {
    // Look at when signals typically occur for this contact
    const hours = signals
      .filter(s => s.detectedAt)
      .map(s => new Date(s.detectedAt).getHours());

    if (hours.length < 3) return undefined;

    // Find most common hour
    const hourCounts = new Map<number, number>();
    for (const hour of hours) {
      hourCounts.set(hour, (hourCounts.get(hour) || 0) + 1);
    }

    let bestHour = 10; // Default to 10 AM
    let maxCount = 0;
    for (const [hour, count] of hourCounts) {
      if (count > maxCount) {
        maxCount = count;
        bestHour = hour;
      }
    }

    // Return next occurrence of that hour
    const now = new Date();
    const optimal = new Date(now);
    optimal.setHours(bestHour, 0, 0, 0);

    if (optimal <= now) {
      optimal.setDate(optimal.getDate() + 1);
    }

    return optimal;
  }
}
