// services/sales-automation/EngagementBranchingService.ts
// Dynamic sequence branching based on prospect engagement
// If they open but don't reply, try different approach

import { Pool } from 'pg';

interface EngagementState {
  emailsSent: number;
  emailsOpened: number;
  emailsClicked: number;
  emailsReplied: number;
  lastOpenAt?: Date;
  lastClickAt?: Date;
  openRate: number;
  engagementLevel: 'cold' | 'warm' | 'hot' | 'unresponsive';
}

interface BranchAction {
  action: 'continue' | 'branch' | 'pause' | 'stop' | 'escalate';
  branchToSequence?: string;
  reason: string;
  nextStepDelay?: number;  // Days
}

// Engagement thresholds
const THRESHOLDS = {
  // Opens without reply after X emails = warm but hesitant
  warmHesitant: { opens: 2, replies: 0, emails: 3 },
  // Multiple clicks = very interested but not ready
  hotNotReady: { clicks: 2, replies: 0 },
  // No engagement after X emails = unresponsive
  unresponsive: { opens: 0, emails: 4 },
  // High engagement = hot lead
  hot: { openRate: 0.75, clicks: 1 },
};

// Branch sequence templates
// FIXED (Jan 2026): Removed unsupported variables (competitor_app, current_mediation, 
// estimated_revenue, net_benefit, app_category, ad_format, similar_app, app_name).
// These variables have no reliable data source and would render as empty strings.
// Now using only: first_name, company_name (from contact data)
const BRANCH_SEQUENCES = {
  // For warm but hesitant (opens but no reply)
  warmHesitant: {
    name: 'Warm Hesitant - Social Proof',
    description: 'For prospects who open but don\'t reply - add social proof and reduce ask',
    steps: [
      {
        delay: 2,
        subject: 'Quick thought for {{company_name}}',
        body: `Hi {{first_name}},

I noticed you've been checking out my previous emails (thanks for reading! 🙂).

I wanted to share a quick win - a publisher similar to {{company_name}} saw a meaningful revenue increase in their first month after optimizing their ad stack.

No pressure at all - but if you're even slightly curious, I'd be happy to share exactly what worked for them in a quick 10-minute call.

What do you think?

Best,
Aaron

P.S. Reply NO to opt out.`,
      },
      {
        delay: 5,
        subject: 'Last note for {{company_name}}',
        body: `Hey {{first_name}},

Last email from me on this (I promise 😅).

Since you seemed at least somewhat interested, I thought I'd offer to put together a quick analysis of how ApexMediation could help {{company_name}} specifically.

Want me to send over the details? Takes 2 minutes to read.

Cheers,
Aaron

P.S. Reply NO to opt out.`,
      },
    ],
  },
  
  // For hot but not ready (clicks but no reply)
  hotNotReady: {
    name: 'Hot Not Ready - Case Study',
    description: 'For prospects clicking links but not replying - detailed case study approach',
    steps: [
      {
        delay: 1,
        subject: 'Case study for {{company_name}}',
        body: `Hi {{first_name}},

I noticed you clicked through to learn more (awesome!).

Since you're clearly researching options, I thought you might find this useful - here's how a similar publisher improved their results:

**The Challenge:**
- Using a single ad network
- Fill rates dropping, eCPM inconsistent
- Limited visibility into performance

**What Changed:**
- Added mediation with transparent auction
- Could see exactly which demand sources performed best
- Optimized based on real data

**Results:**
- Meaningful revenue increase
- Better fill rates
- Full transparency into every auction

Would you like me to do a similar analysis for {{company_name}}? I can usually turn it around in 24 hours.

Best,
Aaron

P.S. Reply NO to opt out.`,
      },
    ],
  },
  
  // For unresponsive - breakup email
  breakup: {
    name: 'Breakup Sequence',
    description: 'Final attempt for unresponsive prospects',
    steps: [
      {
        delay: 7,
        subject: 'Should I stop reaching out?',
        body: `Hi {{first_name}},

I've sent a few emails about how ApexMediation could help {{company_name}} with ad revenue, but haven't heard back.

Totally get it - you're busy, and maybe the timing isn't right.

I just want to make sure I'm not clogging your inbox if you're not interested. Should I:

a) Check back in 3 months?
b) Remove you from my list entirely?
c) Actually, let's chat - hit reply and suggest a time

No hard feelings either way!

Cheers,
Aaron

P.S. If you do want to chat later, you can always reach us at sales@apexmediation.ee`,
      },
    ],
  },
  
  // For re-engagement after long silence
  reEngagement: {
    name: 'Re-engagement Sequence',
    description: 'For contacts who went cold after initial engagement',
    steps: [
      {
        delay: 30,
        subject: 'Quick update for {{company_name}}',
        body: `Hey {{first_name}},

It's been a while! I wanted to reach out because we've made some updates that might be relevant for {{company_name}}:

**What's new at ApexMediation:**
- New AI-powered bidfloor optimization
- Expanded demand partner integrations
- Simplified SDK integration (now takes <30 mins)

Since you showed interest before, I thought you might want to take another look.

Want to see a quick demo of the new features?

Best,
Aaron

P.S. Reply NO to opt out.`,
      },
    ],
  },
};

export class EngagementBranchingService {
  private pool: Pool;

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * Evaluate engagement and determine next action
   */
  async evaluateAndBranch(enrollmentId: string): Promise<BranchAction> {
    const engagement = await this.getEngagementState(enrollmentId);
    
    // Check for hot leads first
    if (engagement.engagementLevel === 'hot') {
      return {
        action: 'escalate',
        reason: 'High engagement - escalate to personal follow-up',
        nextStepDelay: 1,
      };
    }
    
    // Check for unresponsive
    if (engagement.engagementLevel === 'unresponsive') {
      return {
        action: 'branch',
        branchToSequence: 'breakup',
        reason: 'No engagement after multiple emails - sending breakup sequence',
        nextStepDelay: 0,
      };
    }
    
    // Check for warm but hesitant (opens but no replies)
    if (
      engagement.emailsOpened >= THRESHOLDS.warmHesitant.opens &&
      engagement.emailsReplied === 0 &&
      engagement.emailsSent >= THRESHOLDS.warmHesitant.emails
    ) {
      return {
        action: 'branch',
        branchToSequence: 'warmHesitant',
        reason: 'Multiple opens without reply - switching to social proof approach',
        nextStepDelay: 2,
      };
    }
    
    // Check for hot but not ready (clicks but no replies)
    if (
      engagement.emailsClicked >= THRESHOLDS.hotNotReady.clicks &&
      engagement.emailsReplied === 0
    ) {
      return {
        action: 'branch',
        branchToSequence: 'hotNotReady',
        reason: 'Clicking links but not replying - sending detailed case study',
        nextStepDelay: 1,
      };
    }
    
    // Default: continue with normal sequence
    return {
      action: 'continue',
      reason: 'Continuing normal sequence',
    };
  }

  /**
   * Get engagement state for an enrollment
   */
  async getEngagementState(enrollmentId: string): Promise<EngagementState> {
    const result = await this.pool.query(`
      SELECT 
        COUNT(*) as emails_sent,
        COUNT(*) FILTER (WHERE opened_at IS NOT NULL) as emails_opened,
        COUNT(*) FILTER (WHERE clicked_at IS NOT NULL) as emails_clicked,
        COUNT(*) FILTER (WHERE replied_at IS NOT NULL) as emails_replied,
        MAX(opened_at) as last_open_at,
        MAX(clicked_at) as last_click_at
      FROM outreach_emails
      WHERE enrollment_id = $1
        AND status IN ('sent', 'delivered', 'opened', 'clicked', 'replied')
    `, [enrollmentId]);

    const row = result.rows[0];
    const sent = parseInt(row.emails_sent) || 0;
    const opened = parseInt(row.emails_opened) || 0;
    const clicked = parseInt(row.emails_clicked) || 0;
    const replied = parseInt(row.emails_replied) || 0;
    
    const openRate = sent > 0 ? opened / sent : 0;
    
    // Determine engagement level
    let engagementLevel: EngagementState['engagementLevel'];
    
    if (replied > 0) {
      engagementLevel = 'hot';
    } else if (clicked >= THRESHOLDS.hotNotReady.clicks || openRate >= THRESHOLDS.hot.openRate) {
      engagementLevel = 'hot';
    } else if (opened > 0) {
      engagementLevel = 'warm';
    } else if (sent >= THRESHOLDS.unresponsive.emails && opened === 0) {
      engagementLevel = 'unresponsive';
    } else {
      engagementLevel = 'cold';
    }

    return {
      emailsSent: sent,
      emailsOpened: opened,
      emailsClicked: clicked,
      emailsReplied: replied,
      lastOpenAt: row.last_open_at ? new Date(row.last_open_at) : undefined,
      lastClickAt: row.last_click_at ? new Date(row.last_click_at) : undefined,
      openRate,
      engagementLevel,
    };
  }

  /**
   * Execute branch action
   */
  async executeBranch(enrollmentId: string, action: BranchAction): Promise<void> {
    if (action.action === 'continue') {
      return;  // Nothing to do
    }

    if (action.action === 'stop') {
      await this.pool.query(`
        UPDATE outreach_enrollments
        SET status = 'completed',
            completed_at = CURRENT_TIMESTAMP,
            completion_reason = $2,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [enrollmentId, action.reason]);
      return;
    }

    if (action.action === 'pause') {
      await this.pool.query(`
        UPDATE outreach_enrollments
        SET status = 'paused',
            pause_reason = $2,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [enrollmentId, action.reason]);
      return;
    }

    if (action.action === 'escalate') {
      // Create a task for manual follow-up
      await this.pool.query(`
        INSERT INTO sales_activities (
          contact_id, activity_type, subject, notes, metadata
        )
        SELECT contact_id, 'task_created', 'Manual follow-up required', $2,
               jsonb_build_object('enrollment_id', $1, 'urgency', 'high')
        FROM outreach_enrollments WHERE id = $1
      `, [enrollmentId, action.reason]);
      
      await this.pool.query(`
        UPDATE outreach_enrollments
        SET status = 'escalated',
            updated_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [enrollmentId]);
      return;
    }

    if (action.action === 'branch' && action.branchToSequence) {
      // Get branch sequence template
      const branchTemplate = BRANCH_SEQUENCES[action.branchToSequence as keyof typeof BRANCH_SEQUENCES];
      if (!branchTemplate) {
        console.error(`[Branching] Unknown branch sequence: ${action.branchToSequence}`);
        return;
      }

      // Create or get branch sequence
      const sequenceId = await this.getOrCreateBranchSequence(branchTemplate);

      // Update enrollment to use new sequence
      await this.pool.query(`
        UPDATE outreach_enrollments
        SET sequence_id = $2,
            current_step = 0,
            next_step_at = CURRENT_TIMESTAMP + INTERVAL '${action.nextStepDelay || 1} days',
            branch_reason = $3,
            branches_count = branches_count + 1,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [enrollmentId, sequenceId, action.reason]);

      // Log the branch
      await this.pool.query(`
        INSERT INTO sales_activities (
          contact_id, activity_type, subject, notes, metadata
        )
        SELECT contact_id, 'sequence_branched', 'Sequence branched', $2,
               jsonb_build_object('enrollment_id', $1, 'new_sequence', $3)
        FROM outreach_enrollments WHERE id = $1
      `, [enrollmentId, action.reason, action.branchToSequence]);

      console.log(`[Branching] Enrollment ${enrollmentId} branched to ${action.branchToSequence}`);
    }
  }

  /**
   * Get or create a branch sequence from template
   */
  private async getOrCreateBranchSequence(template: typeof BRANCH_SEQUENCES[keyof typeof BRANCH_SEQUENCES]): Promise<string> {
    // Check if sequence exists
    const existing = await this.pool.query(`
      SELECT id FROM outreach_sequences WHERE name = $1
    `, [template.name]);

    if (existing.rows.length > 0) {
      return existing.rows[0].id;
    }

    // Create new sequence
    const sequenceResult = await this.pool.query(`
      INSERT INTO outreach_sequences (
        name, description, total_steps, target_segment, active
      ) VALUES ($1, $2, $3, 'all', true)
      RETURNING id
    `, [template.name, template.description, template.steps.length]);

    const sequenceId = sequenceResult.rows[0].id;

    // Create steps
    for (let i = 0; i < template.steps.length; i++) {
      const step = template.steps[i];
      await this.pool.query(`
        INSERT INTO outreach_sequence_steps (
          sequence_id, step_number, delay_days, subject_template, body_template
        ) VALUES ($1, $2, $3, $4, $5)
      `, [sequenceId, i + 1, step.delay, step.subject, step.body]);
    }

    return sequenceId;
  }

  /**
   * Run branching evaluation for all active enrollments
   */
  async evaluateAllEnrollments(): Promise<{ evaluated: number; branched: number }> {
    const enrollments = await this.pool.query(`
      SELECT id FROM outreach_enrollments
      WHERE status = 'active'
        AND current_step > 0
        AND branches_count < 2  -- Don't branch more than twice
    `);

    let evaluated = 0;
    let branched = 0;

    for (const enrollment of enrollments.rows) {
      try {
        const action = await this.evaluateAndBranch(enrollment.id);
        evaluated++;

        if (action.action !== 'continue') {
          await this.executeBranch(enrollment.id, action);
          branched++;
        }
      } catch (error) {
        console.error(`[Branching] Error evaluating ${enrollment.id}:`, error);
      }
    }

    console.log(`[Branching] Evaluated ${evaluated} enrollments, branched ${branched}`);
    return { evaluated, branched };
  }

  /**
   * Get engagement analytics
   */
  async getEngagementAnalytics(): Promise<{
    byLevel: { level: string; count: number; percentage: number }[];
    branchingStats: { sequence: string; count: number; conversionRate: number }[];
  }> {
    // By engagement level
    const enrollments = await this.pool.query(`
      SELECT id FROM outreach_enrollments WHERE status = 'active'
    `);

    const levelCounts: Record<string, number> = { cold: 0, warm: 0, hot: 0, unresponsive: 0 };
    
    for (const enrollment of enrollments.rows) {
      const state = await this.getEngagementState(enrollment.id);
      levelCounts[state.engagementLevel]++;
    }

    const total = Object.values(levelCounts).reduce((a, b) => a + b, 0);

    // Branching stats
    const branchStats = await this.pool.query(`
      SELECT 
        s.name as sequence_name,
        COUNT(DISTINCT e.id) as count,
        COUNT(DISTINCT e.id) FILTER (WHERE e.status = 'converted')::float / 
          NULLIF(COUNT(DISTINCT e.id), 0) * 100 as conversion_rate
      FROM outreach_enrollments e
      JOIN outreach_sequences s ON e.sequence_id = s.id
      WHERE e.branches_count > 0
      GROUP BY s.name
    `);

    return {
      byLevel: Object.entries(levelCounts).map(([level, count]) => ({
        level,
        count,
        percentage: total > 0 ? (count / total) * 100 : 0,
      })),
      branchingStats: branchStats.rows.map(row => ({
        sequence: row.sequence_name,
        count: parseInt(row.count),
        conversionRate: parseFloat(row.conversion_rate) || 0,
      })),
    };
  }
}

export default EngagementBranchingService;
