// services/sales-automation/FollowUpOptimizer.ts
// REVOLUTIONARY: Follow-ups are more valuable than first emails - optimize them
// OVERCOMES: "Most leads came from follow-ups, not first email"
// OVERCOMES: "163 positive replies... 40% of these came from follow-up #2-4"
// OVERCOMES: "Leaned way too hard on automation and AI" - thoughtful sequencing
// Based on Reddit: "Follow-ups generated more qualified replies than initial send"
// Zero-cost: Pattern analysis from historical data, no external APIs

import { Pool } from 'pg';

interface FollowUpAnalysis {
  sequenceId: string;
  stepNumber: number;
  performance: {
    replyRate: number;
    positiveReplyRate: number;
    conversionRate: number;
    avgResponseTime: number; // hours
  };
  comparison: {
    vsSequenceAvg: number; // percentage better/worse
    vsPreviousStep: number;
    vsIndustryBenchmark: number;
  };
  recommendations: string[];
}

interface OptimalFollowUpConfig {
  stepNumber: number;
  delayDays: number;
  delayHours: number;
  bestDayOfWeek: number; // 0-6
  bestTimeOfDay: number; // hour
  contentType: 'value' | 'social_proof' | 'pain_point' | 'urgency' | 'breakup';
  lengthRecommendation: 'shorter' | 'same' | 'longer';
  confidence: number;
}

interface SequenceGap {
  sequenceId: string;
  sequenceName: string;
  gapType: 'missing_follow_up' | 'too_long_gap' | 'too_short_gap' | 'premature_end' | 'weak_step';
  stepNumber: number;
  description: string;
  recommendation: string;
  potentialLift: number; // estimated % improvement
}

// Industry benchmarks for follow-ups
const BENCHMARKS = {
  replyRate: {
    step1: 2.5, step2: 3.5, step3: 3.0, step4: 2.0, step5: 1.5,
  },
  optimalDelay: {
    step2: { min: 2, max: 4, optimal: 3 },
    step3: { min: 3, max: 5, optimal: 4 },
    step4: { min: 4, max: 7, optimal: 5 },
    step5: { min: 5, max: 10, optimal: 7 },
  },
  totalSteps: { min: 4, optimal: 5, max: 7 },
};

/**
 * FollowUpOptimizer - Make follow-ups as valuable as they should be
 * REVOLUTIONARY: Treats follow-ups as the main event, not afterthought
 */
export class FollowUpOptimizer {
  private pool: Pool;

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * Initialize required tables
   */
  async initialize(): Promise<void> {
    await this.pool.query(`
      CREATE TABLE IF NOT EXISTS followup_analytics (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        sequence_id UUID REFERENCES outreach_sequences(id),
        step_number INTEGER NOT NULL,
        period_start DATE NOT NULL,
        period_end DATE NOT NULL,
        emails_sent INTEGER DEFAULT 0,
        opens INTEGER DEFAULT 0,
        clicks INTEGER DEFAULT 0,
        replies INTEGER DEFAULT 0,
        positive_replies INTEGER DEFAULT 0,
        conversions INTEGER DEFAULT 0,
        avg_response_time_hours FLOAT,
        created_at TIMESTAMPTZ DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS followup_recommendations (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        sequence_id UUID REFERENCES outreach_sequences(id),
        step_number INTEGER NOT NULL,
        recommendation_type VARCHAR(50),
        recommendation TEXT,
        potential_lift FLOAT,
        applied BOOLEAN DEFAULT FALSE,
        applied_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ DEFAULT NOW()
      );

      CREATE INDEX IF NOT EXISTS idx_followup_analytics_seq ON followup_analytics(sequence_id, step_number);
    `);
  }

  /**
   * MAIN: Analyze follow-up performance for a sequence
   */
  async analyzeSequenceFollowUps(sequenceId: string): Promise<FollowUpAnalysis[]> {
    // Get steps for this sequence
    const stepsResult = await this.pool.query(`
      SELECT 
        ss.step_number,
        COUNT(oe.id) as sent,
        COUNT(oe.opened_at) as opens,
        COUNT(oe.clicked_at) as clicks,
        COUNT(oe.replied_at) as replies,
        COUNT(CASE WHEN ra.sentiment = 'positive' THEN 1 END) as positive_replies,
        AVG(EXTRACT(EPOCH FROM (oe.replied_at - oe.sent_at))/3600) as avg_response_hours
      FROM sequence_steps ss
      LEFT JOIN outreach_emails oe ON ss.sequence_id = oe.sequence_id 
        AND ss.step_number = oe.step_number
        AND oe.sent_at > NOW() - INTERVAL '30 days'
      LEFT JOIN reply_analysis ra ON oe.id = ra.email_id
      WHERE ss.sequence_id = $1
      GROUP BY ss.step_number
      ORDER BY ss.step_number
    `, [sequenceId]);

    const analyses: FollowUpAnalysis[] = [];
    const steps = stepsResult.rows;

    // Calculate sequence averages
    const totalSent = steps.reduce((sum, s) => sum + parseInt(s.sent || 0), 0);
    const totalReplies = steps.reduce((sum, s) => sum + parseInt(s.replies || 0), 0);
    const sequenceAvgReplyRate = totalSent > 0 ? (totalReplies / totalSent) * 100 : 0;

    for (let i = 0; i < steps.length; i++) {
      const step = steps[i];
      const sent = parseInt(step.sent || 0);
      const replies = parseInt(step.replies || 0);
      const positiveReplies = parseInt(step.positive_replies || 0);

      const replyRate = sent > 0 ? (replies / sent) * 100 : 0;
      const positiveReplyRate = sent > 0 ? (positiveReplies / sent) * 100 : 0;
      
      // Compare to previous step
      const prevStep = i > 0 ? steps[i - 1] : null;
      const prevReplyRate = prevStep && parseInt(prevStep.sent) > 0 
        ? (parseInt(prevStep.replies) / parseInt(prevStep.sent)) * 100 
        : 0;

      // Get benchmark for this step
      const stepKey = `step${step.step_number}` as keyof typeof BENCHMARKS.replyRate;
      const benchmark = BENCHMARKS.replyRate[stepKey] || 2;

      const analysis: FollowUpAnalysis = {
        sequenceId,
        stepNumber: step.step_number,
        performance: {
          replyRate,
          positiveReplyRate,
          conversionRate: 0, // Would need conversion tracking
          avgResponseTime: parseFloat(step.avg_response_hours) || 0,
        },
        comparison: {
          vsSequenceAvg: sequenceAvgReplyRate > 0 
            ? ((replyRate - sequenceAvgReplyRate) / sequenceAvgReplyRate) * 100 
            : 0,
          vsPreviousStep: prevReplyRate > 0 
            ? ((replyRate - prevReplyRate) / prevReplyRate) * 100 
            : 0,
          vsIndustryBenchmark: benchmark > 0 
            ? ((replyRate - benchmark) / benchmark) * 100 
            : 0,
        },
        recommendations: this.generateStepRecommendations(
          step.step_number,
          replyRate,
          positiveReplyRate,
          benchmark,
          sent
        ),
      };

      analyses.push(analysis);
    }

    return analyses;
  }

  /**
   * Find gaps in sequences - missing follow-ups or poor timing
   */
  async findSequenceGaps(): Promise<SequenceGap[]> {
    const gaps: SequenceGap[] = [];

    // Get all active sequences with their steps
    const sequencesResult = await this.pool.query(`
      SELECT 
        os.id,
        os.name,
        ss.step_number,
        ss.delay_days,
        COUNT(oe.id) as sent,
        COUNT(oe.replied_at) as replies
      FROM outreach_sequences os
      JOIN sequence_steps ss ON os.id = ss.sequence_id
      LEFT JOIN outreach_emails oe ON ss.sequence_id = oe.sequence_id 
        AND ss.step_number = oe.step_number
        AND oe.sent_at > NOW() - INTERVAL '30 days'
      WHERE os.status = 'active'
      GROUP BY os.id, os.name, ss.step_number, ss.delay_days
      ORDER BY os.id, ss.step_number
    `);

    const sequences = new Map<string, Array<{
      stepNumber: number;
      delayDays: number;
      sent: number;
      replies: number;
    }>>();

    const sequenceNames = new Map<string, string>();

    for (const row of sequencesResult.rows) {
      if (!sequences.has(row.id)) {
        sequences.set(row.id, []);
        sequenceNames.set(row.id, row.name);
      }
      sequences.get(row.id)!.push({
        stepNumber: row.step_number,
        delayDays: row.delay_days,
        sent: parseInt(row.sent) || 0,
        replies: parseInt(row.replies) || 0,
      });
    }

    for (const [seqId, steps] of sequences) {
      const seqName = sequenceNames.get(seqId) || 'Unknown';

      // 1. Check for too few steps (premature end)
      if (steps.length < BENCHMARKS.totalSteps.min) {
        gaps.push({
          sequenceId: seqId,
          sequenceName: seqName,
          gapType: 'premature_end',
          stepNumber: steps.length,
          description: `Only ${steps.length} steps. Most conversions happen in steps 2-4.`,
          recommendation: `Add ${BENCHMARKS.totalSteps.optimal - steps.length} more follow-ups`,
          potentialLift: 25, // Estimated 25% lift
        });
      }

      // 2. Check delays between steps
      for (let i = 1; i < steps.length; i++) {
        const step = steps[i];
        const prevStep = steps[i - 1];
        const delayKey = `step${step.stepNumber}` as keyof typeof BENCHMARKS.optimalDelay;
        const benchmark = BENCHMARKS.optimalDelay[delayKey];

        if (benchmark) {
          if (step.delayDays < benchmark.min) {
            gaps.push({
              sequenceId: seqId,
              sequenceName: seqName,
              gapType: 'too_short_gap',
              stepNumber: step.stepNumber,
              description: `${step.delayDays} day gap before step ${step.stepNumber} is too short`,
              recommendation: `Increase to ${benchmark.optimal} days for better response`,
              potentialLift: 10,
            });
          } else if (step.delayDays > benchmark.max) {
            gaps.push({
              sequenceId: seqId,
              sequenceName: seqName,
              gapType: 'too_long_gap',
              stepNumber: step.stepNumber,
              description: `${step.delayDays} day gap before step ${step.stepNumber} loses momentum`,
              recommendation: `Reduce to ${benchmark.optimal} days to maintain engagement`,
              potentialLift: 15,
            });
          }
        }

        // 3. Check for weak steps (low reply rate)
        if (step.sent >= 50) { // Enough data
          const replyRate = (step.replies / step.sent) * 100;
          const stepBenchmark = BENCHMARKS.replyRate[`step${step.stepNumber}` as keyof typeof BENCHMARKS.replyRate];
          
          if (stepBenchmark && replyRate < stepBenchmark * 0.5) {
            gaps.push({
              sequenceId: seqId,
              sequenceName: seqName,
              gapType: 'weak_step',
              stepNumber: step.stepNumber,
              description: `Step ${step.stepNumber} has ${replyRate.toFixed(1)}% reply rate vs ${stepBenchmark}% benchmark`,
              recommendation: 'A/B test subject lines and content for this step',
              potentialLift: 20,
            });
          }
        }
      }
    }

    return gaps.sort((a, b) => b.potentialLift - a.potentialLift);
  }

  /**
   * Get optimal follow-up configuration for a given step
   */
  async getOptimalFollowUpConfig(
    sequenceId: string, 
    stepNumber: number
  ): Promise<OptimalFollowUpConfig> {
    // Analyze historical performance by day/time
    const performanceResult = await this.pool.query(`
      SELECT 
        EXTRACT(DOW FROM oe.sent_at) as day_of_week,
        EXTRACT(HOUR FROM oe.sent_at) as hour,
        COUNT(*) as sent,
        COUNT(oe.replied_at) as replies
      FROM outreach_emails oe
      WHERE oe.sequence_id = $1 
        AND oe.step_number = $2
        AND oe.sent_at > NOW() - INTERVAL '90 days'
      GROUP BY EXTRACT(DOW FROM oe.sent_at), EXTRACT(HOUR FROM oe.sent_at)
      HAVING COUNT(*) >= 10
    `, [sequenceId, stepNumber]);

    // Find best day
    const dayPerformance = new Map<number, { sent: number; replies: number }>();
    const hourPerformance = new Map<number, { sent: number; replies: number }>();

    for (const row of performanceResult.rows) {
      const day = parseInt(row.day_of_week);
      const hour = parseInt(row.hour);
      const sent = parseInt(row.sent);
      const replies = parseInt(row.replies);

      // Aggregate by day
      const dayData = dayPerformance.get(day) || { sent: 0, replies: 0 };
      dayData.sent += sent;
      dayData.replies += replies;
      dayPerformance.set(day, dayData);

      // Aggregate by hour
      const hourData = hourPerformance.get(hour) || { sent: 0, replies: 0 };
      hourData.sent += sent;
      hourData.replies += replies;
      hourPerformance.set(hour, hourData);
    }

    // Find best day
    let bestDay = 2; // Default Tuesday
    let bestDayRate = 0;
    for (const [day, data] of dayPerformance) {
      const rate = data.sent > 0 ? (data.replies / data.sent) : 0;
      if (rate > bestDayRate) {
        bestDayRate = rate;
        bestDay = day;
      }
    }

    // Find best hour
    let bestHour = 10; // Default 10 AM
    let bestHourRate = 0;
    for (const [hour, data] of hourPerformance) {
      const rate = data.sent > 0 ? (data.replies / data.sent) : 0;
      if (rate > bestHourRate) {
        bestHourRate = rate;
        bestHour = hour;
      }
    }

    // Get benchmark delay
    const delayKey = `step${stepNumber}` as keyof typeof BENCHMARKS.optimalDelay;
    const delayBenchmark = BENCHMARKS.optimalDelay[delayKey] || { optimal: 3 };

    // Determine content type based on step number
    const contentTypes: Record<number, OptimalFollowUpConfig['contentType']> = {
      1: 'value',
      2: 'value',
      3: 'social_proof',
      4: 'pain_point',
      5: 'urgency',
      6: 'breakup',
    };

    return {
      stepNumber,
      delayDays: delayBenchmark.optimal,
      delayHours: bestHour - 10, // Offset from 10 AM
      bestDayOfWeek: bestDay,
      bestTimeOfDay: bestHour,
      contentType: contentTypes[stepNumber] || 'value',
      lengthRecommendation: stepNumber <= 2 ? 'shorter' : (stepNumber >= 5 ? 'shorter' : 'same'),
      confidence: Math.min(95, Math.max(50, performanceResult.rows.length * 5)),
    };
  }

  /**
   * Generate optimized follow-up sequence based on historical data
   */
  async generateOptimizedSequence(name: string): Promise<{
    name: string;
    steps: Array<{
      stepNumber: number;
      delayDays: number;
      subject: string;
      contentType: string;
      guidelines: string[];
    }>;
  }> {
    // Analyze what's working across all sequences
    const topPerformers = await this.pool.query(`
      SELECT 
        ss.step_number,
        ss.delay_days,
        ss.subject_template,
        COUNT(oe.replied_at)::float / NULLIF(COUNT(oe.id), 0) * 100 as reply_rate
      FROM sequence_steps ss
      JOIN outreach_emails oe ON ss.sequence_id = oe.sequence_id 
        AND ss.step_number = oe.step_number
      WHERE oe.sent_at > NOW() - INTERVAL '60 days'
      GROUP BY ss.step_number, ss.delay_days, ss.subject_template
      HAVING COUNT(oe.id) >= 30
      ORDER BY ss.step_number, reply_rate DESC
    `);

    // Group by step number, take top performer
    const bestByStep = new Map<number, typeof topPerformers.rows[0]>();
    for (const row of topPerformers.rows) {
      if (!bestByStep.has(row.step_number)) {
        bestByStep.set(row.step_number, row);
      }
    }

    // Build optimized sequence
    const steps: Array<{
      stepNumber: number;
      delayDays: number;
      subject: string;
      contentType: string;
      guidelines: string[];
    }> = [];

    for (let i = 1; i <= BENCHMARKS.totalSteps.optimal; i++) {
      const best = bestByStep.get(i);
      const delayKey = `step${i}` as keyof typeof BENCHMARKS.optimalDelay;
      const delayBenchmark = BENCHMARKS.optimalDelay[delayKey];

      steps.push({
        stepNumber: i,
        delayDays: best?.delay_days || delayBenchmark?.optimal || i * 2,
        subject: best?.subject_template || this.getDefaultSubject(i),
        contentType: this.getContentTypeForStep(i),
        guidelines: this.getGuidelinesForStep(i),
      });
    }

    return { name, steps };
  }

  /**
   * Score a follow-up email before sending
   */
  scoreFollowUpEmail(
    stepNumber: number,
    subject: string,
    body: string,
    previousEmailsData: { sent: number; opened: number; replied: number }
  ): {
    score: number;
    issues: string[];
    suggestions: string[];
  } {
    const issues: string[] = [];
    const suggestions: string[] = [];
    let score = 100;

    // 1. Check subject line
    if (subject.length > 50) {
      issues.push('Subject too long - should be under 50 chars');
      score -= 10;
    }

    if (!subject.toLowerCase().includes('re:') && stepNumber > 1) {
      suggestions.push('Consider "Re:" prefix for follow-ups to increase open rate');
    }

    // 2. Check body length
    const wordCount = body.split(/\s+/).length;
    if (stepNumber >= 3 && wordCount > 100) {
      issues.push(`Body too long for step ${stepNumber} (${wordCount} words). Keep under 100.`);
      score -= 15;
    }

    // 3. Check for personalization
    const hasPersonalization = body.includes('{{') || body.includes('{');
    if (!hasPersonalization) {
      issues.push('No personalization tokens found');
      score -= 10;
    }

    // 4. Check for clear CTA
    const ctaPatterns = ['calendar', 'call', 'meet', 'chat', 'reply', 'respond', 'let me know'];
    const hasCTA = ctaPatterns.some(p => body.toLowerCase().includes(p));
    if (!hasCTA) {
      issues.push('No clear call-to-action');
      score -= 10;
    }

    // 5. Check engagement trajectory
    if (previousEmailsData.sent > 0) {
      const openRate = (previousEmailsData.opened / previousEmailsData.sent) * 100;
      if (openRate < 10) {
        suggestions.push('Previous open rate low - try different subject line approach');
      }
      
      if (previousEmailsData.replied === 0 && previousEmailsData.sent > 50) {
        suggestions.push('No replies yet - consider more direct/personalized approach');
      }
    }

    // 6. Check for spam triggers (basic)
    const spamTriggers = ['free', 'guarantee', 'act now', 'limited time', 'special offer'];
    const hasSpamTriggers = spamTriggers.some(t => body.toLowerCase().includes(t));
    if (hasSpamTriggers) {
      issues.push('Contains spam trigger words');
      score -= 15;
    }

    return { score: Math.max(0, score), issues, suggestions };
  }

  // ===== Private Methods =====

  private generateStepRecommendations(
    stepNumber: number,
    replyRate: number,
    positiveReplyRate: number,
    benchmark: number,
    sent: number
  ): string[] {
    const recommendations: string[] = [];

    if (sent < 50) {
      recommendations.push('Need more data - at least 50 emails for reliable analysis');
      return recommendations;
    }

    if (replyRate < benchmark * 0.5) {
      recommendations.push('Reply rate significantly below benchmark - test new subject line');
      recommendations.push('Consider shortening email body');
    }

    if (positiveReplyRate < replyRate * 0.3) {
      recommendations.push('Many negative replies - review messaging for clarity');
    }

    if (stepNumber >= 3 && replyRate > benchmark) {
      recommendations.push('Strong performer - analyze what makes this step work');
    }

    if (stepNumber === 1 && replyRate < 2) {
      recommendations.push('First touch needs stronger value proposition');
      recommendations.push('Try more specific personalization');
    }

    return recommendations;
  }

  private getDefaultSubject(stepNumber: number): string {
    const subjects: Record<number, string> = {
      1: '{{company}} - Quick question',
      2: 'Re: {{company}}',
      3: 'Thought this might help',
      4: 'One more thing',
      5: 'Should I close your file?',
    };
    return subjects[stepNumber] || `Follow-up ${stepNumber}`;
  }

  private getContentTypeForStep(stepNumber: number): string {
    const types: Record<number, string> = {
      1: 'value_proposition',
      2: 'social_proof',
      3: 'pain_point',
      4: 'case_study',
      5: 'breakup',
    };
    return types[stepNumber] || 'value';
  }

  private getGuidelinesForStep(stepNumber: number): string[] {
    const guidelines: Record<number, string[]> = {
      1: [
        'Keep under 150 words',
        'Focus on ONE clear value prop',
        'Ask a question to encourage reply',
        'Don\'t pitch - spark curiosity',
      ],
      2: [
        'Reference first email briefly',
        'Add new angle/value',
        'Include social proof if relevant',
        'Keep even shorter than first',
      ],
      3: [
        'Address potential objections',
        'Share relevant case study',
        'Make reply super easy',
      ],
      4: [
        'Show understanding of their challenges',
        'Offer specific help',
        'Create soft urgency',
      ],
      5: [
        'Respectful "breakup" message',
        'Leave door open',
        'Reverse psychology can work',
      ],
    };
    return guidelines[stepNumber] || ['Keep it short', 'Add value'];
  }
}
