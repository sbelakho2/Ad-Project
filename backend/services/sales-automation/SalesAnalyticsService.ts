// services/sales-automation/SalesAnalyticsService.ts
// Tracks sales metrics, pipeline, and optimizes outreach
// No paid APIs - uses internal data only

import { Pool } from 'pg';

export interface PipelineMetrics {
  total_prospects: number;
  total_qualified: number;
  total_contacted: number;
  total_responded: number;
  total_converted: number;
  conversion_rate: number;
  pipeline_value_cents: number;
}

export interface SequencePerformance {
  sequence_id: string;
  sequence_name: string;
  total_enrolled: number;
  open_rate: number;
  reply_rate: number;
  conversion_rate: number;
}

export interface DailyMetrics {
  date: string;
  prospects_discovered: number;
  prospects_qualified: number;
  emails_sent: number;
  emails_opened: number;
  emails_replied: number;
  trials_started: number;
  deals_won: number;
}

export class SalesAnalyticsService {
  private pool: Pool;

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * Run daily analytics roll-up
   */
  async runDailyRollup(): Promise<void> {
    console.log('[SalesAnalytics] Running daily metrics roll-up...');

    try {
      // 1. Update sequence performance metrics
      await this.updateSequenceMetrics();

      // 2. Calculate daily metrics
      await this.calculateDailyMetrics();

      // 3. Update prospect scores
      await this.recalculateProspectScores();

      // 4. Identify optimization opportunities
      await this.identifyOptimizations();

      // 5. Clean up stale data
      await this.cleanupStaleData();

      console.log('[SalesAnalytics] Daily roll-up complete');
    } catch (error) {
      console.error('[SalesAnalytics] Error in daily roll-up:', error);
      throw error;
    }
  }

  /**
   * Get current pipeline metrics
   */
  async getPipelineMetrics(): Promise<PipelineMetrics> {
    const result = await this.pool.query(`
      SELECT 
        COUNT(*) as total_prospects,
        SUM(CASE WHEN status = 'qualified' THEN 1 ELSE 0 END) as total_qualified,
        SUM(CASE WHEN status = 'contacted' THEN 1 ELSE 0 END) as total_contacted,
        SUM(CASE WHEN status IN ('responded', 'demo_scheduled', 'trial_started') THEN 1 ELSE 0 END) as total_responded,
        SUM(CASE WHEN status = 'converted' THEN 1 ELSE 0 END) as total_converted
      FROM sales_prospects
      WHERE created_at > CURRENT_DATE - INTERVAL '90 days'
    `);

    const metrics = result.rows[0];
    const conversionRate = metrics.total_contacted > 0 
      ? (metrics.total_converted / metrics.total_contacted) * 100 
      : 0;

    // Calculate pipeline value
    const pipelineValue = await this.pool.query(`
      SELECT COALESCE(SUM(estimated_mrr_cents), 0) as value
      FROM sales_deals
      WHERE stage NOT IN ('closed_won', 'closed_lost')
    `);

    return {
      total_prospects: parseInt(metrics.total_prospects, 10),
      total_qualified: parseInt(metrics.total_qualified, 10),
      total_contacted: parseInt(metrics.total_contacted, 10),
      total_responded: parseInt(metrics.total_responded, 10),
      total_converted: parseInt(metrics.total_converted, 10),
      conversion_rate: parseFloat(conversionRate.toFixed(2)),
      pipeline_value_cents: parseInt(pipelineValue.rows[0]?.value || '0', 10),
    };
  }

  /**
   * Get sequence performance metrics
   */
  async getSequencePerformance(): Promise<SequencePerformance[]> {
    const result = await this.pool.query(`
      SELECT 
        s.id as sequence_id,
        s.name as sequence_name,
        COUNT(DISTINCT e.id) as total_enrolled,
        CASE WHEN COUNT(DISTINCT e.id) > 0 
          THEN (SUM(e.emails_opened)::FLOAT / NULLIF(SUM(e.emails_sent), 0)) * 100 
          ELSE 0 
        END as open_rate,
        CASE WHEN COUNT(DISTINCT e.id) > 0 
          THEN (SUM(CASE WHEN e.status = 'responded' THEN 1 ELSE 0 END)::FLOAT / COUNT(DISTINCT e.id)) * 100 
          ELSE 0 
        END as reply_rate,
        CASE WHEN COUNT(DISTINCT e.id) > 0 
          THEN (SUM(CASE WHEN e.status = 'converted' THEN 1 ELSE 0 END)::FLOAT / COUNT(DISTINCT e.id)) * 100 
          ELSE 0 
        END as conversion_rate
      FROM outreach_sequences s
      LEFT JOIN outreach_enrollments e ON s.id = e.sequence_id
      WHERE s.active = true
      GROUP BY s.id, s.name
      ORDER BY conversion_rate DESC
    `);

    return result.rows.map((row) => ({
      sequence_id: row.sequence_id,
      sequence_name: row.sequence_name,
      total_enrolled: parseInt(row.total_enrolled, 10),
      open_rate: parseFloat(row.open_rate || '0'),
      reply_rate: parseFloat(row.reply_rate || '0'),
      conversion_rate: parseFloat(row.conversion_rate || '0'),
    }));
  }

  /**
   * Get metrics trend for last N days
   */
  async getMetricsTrend(days: number = 30): Promise<DailyMetrics[]> {
    const result = await this.pool.query(`
      SELECT 
        date::TEXT,
        COALESCE(prospects_discovered, 0) as prospects_discovered,
        COALESCE(prospects_qualified, 0) as prospects_qualified,
        COALESCE(emails_sent, 0) as emails_sent,
        COALESCE(emails_opened, 0) as emails_opened,
        COALESCE(emails_replied, 0) as emails_replied,
        COALESCE(trials_started, 0) as trials_started,
        COALESCE(deals_won, 0) as deals_won
      FROM sales_metrics_daily
      WHERE date > CURRENT_DATE - INTERVAL '1 day' * $1
      ORDER BY date DESC
    `, [days]);

    return result.rows;
  }

  /**
   * Update sequence performance metrics
   */
  private async updateSequenceMetrics(): Promise<void> {
    // Update each sequence's aggregate metrics
    await this.pool.query(`
      UPDATE outreach_sequences s
      SET 
        total_enrolled = stats.enrolled,
        total_opened = stats.opened,
        total_clicked = stats.clicked,
        total_replied = stats.replied,
        total_converted = stats.converted,
        updated_at = CURRENT_TIMESTAMP
      FROM (
        SELECT 
          sequence_id,
          COUNT(*) as enrolled,
          SUM(emails_opened) as opened,
          SUM(emails_clicked) as clicked,
          SUM(CASE WHEN status = 'responded' THEN 1 ELSE 0 END) as replied,
          SUM(CASE WHEN status = 'converted' THEN 1 ELSE 0 END) as converted
        FROM outreach_enrollments
        GROUP BY sequence_id
      ) stats
      WHERE s.id = stats.sequence_id
    `);

    // Update step-level stats
    await this.pool.query(`
      INSERT INTO outreach_sequence_stats (sequence_id, step_number, ab_variant, date, sent, delivered, opened, clicked, replied, bounced, opted_out)
      SELECT 
        e.sequence_id,
        step.step_number,
        COALESCE(e.ab_variant, 'A'),
        CURRENT_DATE,
        COUNT(CASE WHEN oe.status IN ('sent', 'delivered', 'opened', 'clicked', 'replied') THEN 1 END),
        COUNT(CASE WHEN oe.status IN ('delivered', 'opened', 'clicked', 'replied') THEN 1 END),
        COUNT(CASE WHEN oe.status IN ('opened', 'clicked', 'replied') THEN 1 END),
        COUNT(CASE WHEN oe.status IN ('clicked', 'replied') THEN 1 END),
        COUNT(CASE WHEN oe.status = 'replied' THEN 1 END),
        COUNT(CASE WHEN oe.status = 'bounced' THEN 1 END),
        COUNT(CASE WHEN e.status = 'opted_out' THEN 1 END)
      FROM outreach_emails oe
      JOIN outreach_enrollments e ON oe.enrollment_id = e.id
      JOIN outreach_sequence_steps step ON oe.step_id = step.id
      WHERE oe.sent_at >= CURRENT_DATE
      GROUP BY e.sequence_id, step.step_number, e.ab_variant
      ON CONFLICT (sequence_id, step_number, ab_variant, date) DO UPDATE SET
        sent = EXCLUDED.sent,
        delivered = EXCLUDED.delivered,
        opened = EXCLUDED.opened,
        clicked = EXCLUDED.clicked,
        replied = EXCLUDED.replied,
        bounced = EXCLUDED.bounced,
        opted_out = EXCLUDED.opted_out
    `);
  }

  /**
   * Calculate and store daily metrics
   */
  private async calculateDailyMetrics(): Promise<void> {
    await this.pool.query(`
      INSERT INTO sales_metrics_daily (date, 
        prospects_discovered, prospects_enriched, prospects_qualified, prospects_disqualified,
        contacts_discovered, contacts_verified,
        emails_scheduled, emails_sent, emails_delivered, emails_opened, emails_clicked, emails_replied, emails_bounced,
        deals_created, deals_qualified, demos_scheduled, trials_started, deals_won, deals_lost,
        pipeline_value_cents, won_value_cents
      )
      SELECT 
        CURRENT_DATE,
        (SELECT COUNT(*) FROM sales_prospects WHERE discovery_date::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_prospects WHERE status = 'enriched' AND updated_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_prospects WHERE status = 'qualified' AND updated_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_prospects WHERE status = 'disqualified' AND disqualified_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_contacts WHERE discovered_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_contacts WHERE email_verified = true AND updated_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM outreach_emails WHERE scheduled_for::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM outreach_emails WHERE sent_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM outreach_emails WHERE delivered_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM outreach_emails WHERE opened_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM outreach_emails WHERE clicked_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM outreach_emails WHERE replied_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM outreach_emails WHERE bounced_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_deals WHERE created_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_deals WHERE stage = 'qualified' AND updated_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_deals WHERE stage = 'demo_scheduled' AND updated_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_deals WHERE stage = 'trial_started' AND updated_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_deals WHERE stage = 'closed_won' AND closed_at::DATE = CURRENT_DATE),
        (SELECT COUNT(*) FROM sales_deals WHERE stage = 'closed_lost' AND closed_at::DATE = CURRENT_DATE),
        (SELECT COALESCE(SUM(estimated_mrr_cents), 0) FROM sales_deals WHERE stage NOT IN ('closed_won', 'closed_lost')),
        (SELECT COALESCE(SUM(estimated_mrr_cents), 0) FROM sales_deals WHERE stage = 'closed_won' AND closed_at::DATE = CURRENT_DATE)
      ON CONFLICT (date) DO UPDATE SET
        prospects_discovered = EXCLUDED.prospects_discovered,
        prospects_enriched = EXCLUDED.prospects_enriched,
        prospects_qualified = EXCLUDED.prospects_qualified,
        prospects_disqualified = EXCLUDED.prospects_disqualified,
        contacts_discovered = EXCLUDED.contacts_discovered,
        contacts_verified = EXCLUDED.contacts_verified,
        emails_scheduled = EXCLUDED.emails_scheduled,
        emails_sent = EXCLUDED.emails_sent,
        emails_delivered = EXCLUDED.emails_delivered,
        emails_opened = EXCLUDED.emails_opened,
        emails_clicked = EXCLUDED.emails_clicked,
        emails_replied = EXCLUDED.emails_replied,
        emails_bounced = EXCLUDED.emails_bounced,
        deals_created = EXCLUDED.deals_created,
        deals_qualified = EXCLUDED.deals_qualified,
        demos_scheduled = EXCLUDED.demos_scheduled,
        trials_started = EXCLUDED.trials_started,
        deals_won = EXCLUDED.deals_won,
        deals_lost = EXCLUDED.deals_lost,
        pipeline_value_cents = EXCLUDED.pipeline_value_cents,
        won_value_cents = EXCLUDED.won_value_cents,
        updated_at = CURRENT_TIMESTAMP
    `);
  }

  /**
   * Recalculate prospect scores for recently updated prospects
   */
  private async recalculateProspectScores(): Promise<void> {
    // Get prospects that need score updates
    const prospects = await this.pool.query(`
      SELECT id FROM sales_prospects
      WHERE updated_at > CURRENT_TIMESTAMP - INTERVAL '1 day'
        AND status NOT IN ('converted', 'disqualified')
      LIMIT 1000
    `);

    for (const prospect of prospects.rows) {
      await this.pool.query(`
        UPDATE sales_prospects 
        SET fit_score = calculate_prospect_fit_score($1),
            updated_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [prospect.id]);
    }

    console.log(`[SalesAnalytics] Recalculated scores for ${prospects.rows.length} prospects`);
  }

  /**
   * Identify optimization opportunities
   */
  private async identifyOptimizations(): Promise<void> {
    // 1. Find underperforming sequences (below average reply rate)
    const avgReplyRate = await this.pool.query(`
      SELECT AVG(
        CASE WHEN total_enrolled > 0 
          THEN total_replied::FLOAT / total_enrolled * 100 
          ELSE 0 
        END
      ) as avg_rate
      FROM outreach_sequences
      WHERE total_enrolled >= 10
    `);

    const threshold = parseFloat(avgReplyRate.rows[0]?.avg_rate || '0') * 0.5;

    const underperforming = await this.pool.query(`
      SELECT id, name, total_enrolled, total_replied
      FROM outreach_sequences
      WHERE total_enrolled >= 10
        AND (total_replied::FLOAT / total_enrolled * 100) < $1
    `, [threshold]);

    if (underperforming.rows.length > 0) {
      console.log('[SalesAnalytics] Underperforming sequences:', 
        underperforming.rows.map(s => s.name).join(', ')
      );
    }

    // 2. Find high-value uncontacted prospects
    const uncontactedHigh = await this.pool.query(`
      SELECT COUNT(*) as count
      FROM sales_prospects
      WHERE status = 'qualified'
        AND fit_score >= 70
        AND NOT EXISTS (
          SELECT 1 FROM outreach_enrollments 
          WHERE prospect_id = sales_prospects.id
        )
    `);

    if (parseInt(uncontactedHigh.rows[0]?.count || '0', 10) > 50) {
      console.log(`[SalesAnalytics] ${uncontactedHigh.rows[0].count} high-value prospects not yet contacted`);
    }

    // 3. Find contacts with stale enrollments (stuck)
    const staleEnrollments = await this.pool.query(`
      SELECT COUNT(*) as count
      FROM outreach_enrollments
      WHERE status = 'active'
        AND updated_at < CURRENT_TIMESTAMP - INTERVAL '7 days'
    `);

    if (parseInt(staleEnrollments.rows[0]?.count || '0', 10) > 0) {
      console.log(`[SalesAnalytics] ${staleEnrollments.rows[0].count} stale enrollments need attention`);
    }
  }

  /**
   * Clean up stale data
   */
  private async cleanupStaleData(): Promise<void> {
    // Mark very old prospects as disqualified if never contacted
    await this.pool.query(`
      UPDATE sales_prospects
      SET status = 'disqualified', 
          disqualified_reason = 'stale_no_contact',
          disqualified_at = CURRENT_TIMESTAMP,
          updated_at = CURRENT_TIMESTAMP
      WHERE status IN ('discovered', 'enriched')
        AND created_at < CURRENT_TIMESTAMP - INTERVAL '180 days'
    `);

    // Complete stale enrollments
    await this.pool.query(`
      UPDATE outreach_enrollments
      SET status = 'completed',
          completed_at = CURRENT_TIMESTAMP,
          updated_at = CURRENT_TIMESTAMP
      WHERE status = 'active'
        AND updated_at < CURRENT_TIMESTAMP - INTERVAL '60 days'
    `);

    // Archive old metrics (keep last 365 days)
    // In production, you might move this to a separate archive table
  }

  /**
   * Get funnel conversion rates
   */
  async getFunnelMetrics(): Promise<{
    stage: string;
    count: number;
    conversion_rate: number;
  }[]> {
    const result = await this.pool.query(`
      WITH funnel AS (
        SELECT 
          'discovered' as stage, 
          COUNT(*) as count 
        FROM sales_prospects 
        WHERE created_at > CURRENT_DATE - INTERVAL '90 days'
        UNION ALL
        SELECT 
          'enriched', 
          COUNT(*) 
        FROM sales_prospects 
        WHERE status NOT IN ('discovered') 
          AND created_at > CURRENT_DATE - INTERVAL '90 days'
        UNION ALL
        SELECT 
          'qualified', 
          COUNT(*) 
        FROM sales_prospects 
        WHERE status IN ('qualified', 'contacted', 'converted') 
          AND created_at > CURRENT_DATE - INTERVAL '90 days'
        UNION ALL
        SELECT 
          'contacted', 
          COUNT(*) 
        FROM sales_prospects 
        WHERE status IN ('contacted', 'converted') 
          AND created_at > CURRENT_DATE - INTERVAL '90 days'
        UNION ALL
        SELECT 
          'converted', 
          COUNT(*) 
        FROM sales_prospects 
        WHERE status = 'converted' 
          AND created_at > CURRENT_DATE - INTERVAL '90 days'
      )
      SELECT 
        stage,
        count,
        LAG(count) OVER (ORDER BY 
          CASE stage 
            WHEN 'discovered' THEN 1 
            WHEN 'enriched' THEN 2 
            WHEN 'qualified' THEN 3 
            WHEN 'contacted' THEN 4 
            WHEN 'converted' THEN 5 
          END
        ) as prev_count
      FROM funnel
    `);

    return result.rows.map((row) => ({
      stage: row.stage,
      count: parseInt(row.count, 10),
      conversion_rate: row.prev_count 
        ? (parseInt(row.count, 10) / parseInt(row.prev_count, 10)) * 100 
        : 100,
    }));
  }

  /**
   * Track conversion (when a contact becomes a customer)
   */
  async trackConversion(contactId: string, customerId: string, revenueEstimate?: number): Promise<void> {
    // Update contact
    await this.pool.query(`
      UPDATE sales_contacts 
      SET status = 'converted', updated_at = CURRENT_TIMESTAMP
      WHERE id = $1
    `, [contactId]);

    // Update enrollment
    await this.pool.query(`
      UPDATE outreach_enrollments 
      SET status = 'converted', completed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
      WHERE contact_id = $1 AND status = 'active'
    `, [contactId]);

    // Update prospect
    await this.pool.query(`
      UPDATE sales_prospects 
      SET status = 'converted', updated_at = CURRENT_TIMESTAMP
      WHERE id = (SELECT prospect_id FROM sales_contacts WHERE id = $1)
    `, [contactId]);

    // Create or update deal
    await this.pool.query(`
      INSERT INTO sales_deals (
        contact_id, prospect_id, company_id, customer_id,
        stage, source, estimated_mrr_cents, closed_at
      )
      SELECT 
        c.id, c.prospect_id, c.company_id, $2,
        'closed_won', 'outbound_email', $3, CURRENT_TIMESTAMP
      FROM sales_contacts c
      WHERE c.id = $1
      ON CONFLICT DO NOTHING
    `, [contactId, customerId, revenueEstimate || 0]);

    // Log activity
    await this.pool.query(`
      INSERT INTO sales_activities (contact_id, activity_type, channel, metadata, is_automated)
      VALUES ($1, 'converted', 'signup', $2, false)
    `, [contactId, JSON.stringify({ customer_id: customerId })]);

    console.log(`[SalesAnalytics] Tracked conversion for contact ${contactId}`);
  }

  /**
   * Close database connection
   */
  async close(): Promise<void> {
    await this.pool.end();
  }
}
