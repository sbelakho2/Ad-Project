import { queryReadOnly } from '../database';
import { logger } from '../logger';

export interface BusinessMetrics {
  timestamp: string;
  impressions: {
    today: number;
    yesterday: number;
    change_pct: number;
  };
  revenue_usd: {
    today: number;
    yesterday: number;
    change_pct: number;
  };
  active_publishers: number;
  active_experiments: number;
  financials: {
    monthly_inflow_eur: number;
    monthly_outflow_eur: number;
    net_cash_flow_eur: number;
    runway_months: number;
  };
  ceo_metrics: {
    ltv_eur: number;
    churn_rate_pct: number;
    avg_revenue_per_pub_eur: number;
  };
}

export async function getBusinessOverview(): Promise<BusinessMetrics> {
  try {
    const now = new Date();
    const today = now.toISOString().split('T')[0];
    const yesterdayDate = new Date(now);
    yesterdayDate.setDate(yesterdayDate.getDate() - 1);
    const yesterday = yesterdayDate.toISOString().split('T')[0];

    // 1. Core Ad Metrics
    const statsSql = `
      SELECT 
        COUNT(*) FILTER (WHERE observed_at >= $1) as impressions_today,
        COUNT(*) FILTER (WHERE observed_at >= $2 AND observed_at < $1) as impressions_yesterday,
        SUM(revenue_usd) FILTER (WHERE observed_at >= $1) as revenue_today,
        SUM(revenue_usd) FILTER (WHERE observed_at >= $2 AND observed_at < $1) as revenue_yesterday
      FROM analytics_impressions
      WHERE observed_at >= $2
    `;
    
    const statsResult = await queryReadOnly<any>(statsSql, [today, yesterday]);
    const stats = statsResult[0] || {
      impressions_today: 0,
      impressions_yesterday: 0,
      revenue_today: 0,
      revenue_yesterday: 0
    };

    const impToday = parseInt(stats.impressions_today);
    const impYesterday = parseInt(stats.impressions_yesterday);
    const revToday = parseFloat(stats.revenue_today || 0);
    const revYesterday = parseFloat(stats.revenue_yesterday || 0);

    const impChange = impYesterday > 0 ? ((impToday - impYesterday) / impYesterday) * 100 : 0;
    const revChange = revYesterday > 0 ? ((revToday - revYesterday) / revYesterday) * 100 : 0;

    // 2. Publisher/Experiment counts
    const pubResult = await queryReadOnly<any>(`SELECT COUNT(*) as count FROM publishers WHERE status = 'active'`);
    const activePublishers = parseInt(pubResult[0]?.count || 0);

    const expResult = await queryReadOnly<any>(`SELECT COUNT(*) as count FROM vra_experiments WHERE status = 'active'`);
    const activeExperiments = parseInt(expResult[0]?.count || 0);

    // 3. Cash Flow (last 30 days)
    const inflowResult = await queryReadOnly<any>(`
      SELECT SUM(total_cents) as total FROM invoices 
      WHERE status = 'paid' AND paid_at >= NOW() - INTERVAL '30 days'
    `);
    const monthlyInflow = (parseInt(inflowResult[0]?.total || 0) / 100);

    const outflowResult = await queryReadOnly<any>(`
      SELECT SUM(amount_eur_cents) as total FROM expenses 
      WHERE status = 'paid' AND payment_date >= NOW() - INTERVAL '30 days'
    `);
    const monthlyOutflow = (parseInt(outflowResult[0]?.total || 0) / 100);

    // 4. Runway calculation
    const balanceResult = await queryReadOnly<any>(`
      SELECT SUM(balance_cents) as total FROM account_balances 
      WHERE account_type = 'asset' AND account_code LIKE '11%'
    `);
    const currentCash = (parseInt(balanceResult[0]?.total || 0) / 100);
    
    // Average burn (outflow) over last 3 months
    const burnResult = await queryReadOnly<any>(`
      SELECT SUM(amount_eur_cents) / 3 as avg_burn FROM expenses 
      WHERE status = 'paid' AND payment_date >= NOW() - INTERVAL '90 days'
    `);
    const avgBurn = (parseInt(burnResult[0]?.avg_burn || 0) / 100) || 1; // Avoid div by zero
    const runway = currentCash / avgBurn;

    // 5. CEO Metrics (LTV, Churn)
    const ltvResult = await queryReadOnly<any>(`
      SELECT AVG(total_paid_cents) / 100 as avg_ltv FROM customer_lifetime_value
    `);
    const ltv = parseFloat(ltvResult[0]?.avg_ltv || 0);

    const churnResult = await queryReadOnly<any>(`
      WITH monthly_active AS (
        SELECT COUNT(*) as count FROM publishers WHERE status = 'active'
      ),
      recently_inactive AS (
        SELECT COUNT(*) as count FROM publishers 
        WHERE status = 'inactive' AND updated_at >= NOW() - INTERVAL '30 days'
      )
      SELECT 
        (recently_inactive.count::float / NULLIF(monthly_active.count, 0)) * 100 as churn_rate
      FROM monthly_active, recently_inactive
    `);
    const churnRate = parseFloat(churnResult[0]?.churn_rate || 0);

    return {
      timestamp: now.toISOString(),
      impressions: {
        today: impToday,
        yesterday: impYesterday,
        change_pct: parseFloat(impChange.toFixed(1))
      },
      revenue_usd: {
        today: parseFloat(revToday.toFixed(2)),
        yesterday: parseFloat(revYesterday.toFixed(2)),
        change_pct: parseFloat(revChange.toFixed(1))
      },
      active_publishers: activePublishers,
      active_experiments: activeExperiments,
      financials: {
        monthly_inflow_eur: parseFloat(monthlyInflow.toFixed(2)),
        monthly_outflow_eur: parseFloat(monthlyOutflow.toFixed(2)),
        net_cash_flow_eur: parseFloat((monthlyInflow - monthlyOutflow).toFixed(2)),
        runway_months: parseFloat(runway.toFixed(1))
      },
      ceo_metrics: {
        ltv_eur: parseFloat(ltv.toFixed(2)),
        churn_rate_pct: parseFloat(churnRate.toFixed(1)),
        avg_revenue_per_pub_eur: activePublishers > 0 ? parseFloat((monthlyInflow / activePublishers).toFixed(2)) : 0
      }
    };
  } catch (err) {
    logger.error('Failed to get business overview', { error: (err as Error).message });
    throw err;
  }
}
