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
    const publisherColumns = await getTableColumns('publishers');
    const experimentColumns = await getTableColumns('vra_experiments');
    const invoiceColumns = await getTableColumns('invoices');
    const expenseColumns = await getTableColumns('expenses');

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
    
    const statsResult = await safeQuery<any>(statsSql, [today, yesterday], [{
      impressions_today: 0,
      impressions_yesterday: 0,
      revenue_today: 0,
      revenue_yesterday: 0,
    }]);
    const stats = statsResult[0] || {
      impressions_today: 0,
      impressions_yesterday: 0,
      revenue_today: 0,
      revenue_yesterday: 0,
    };

    const impToday = parseInt(stats.impressions_today);
    const impYesterday = parseInt(stats.impressions_yesterday);
    const revToday = parseFloat(stats.revenue_today || 0);
    const revYesterday = parseFloat(stats.revenue_yesterday || 0);

    const impChange = impYesterday > 0 ? ((impToday - impYesterday) / impYesterday) * 100 : 0;
    const revChange = revYesterday > 0 ? ((revToday - revYesterday) / revYesterday) * 100 : 0;

    // 2. Publisher/Experiment counts
    const pubResult = await safeQuery<any>(
      publisherColumns.has('status')
        ? `SELECT COUNT(*) as count FROM publishers WHERE status = 'active'`
        : `SELECT COUNT(*) as count FROM publishers`,
      [],
      [{ count: 0 }]
    );
    const activePublishers = parseInt(pubResult[0]?.count || 0);

    const expResult = await safeQuery<any>(
      experimentColumns.has('status')
        ? `SELECT COUNT(*) as count FROM vra_experiments WHERE status = 'active'`
        : `SELECT COUNT(*) as count FROM vra_experiments`,
      [],
      [{ count: 0 }]
    );
    const activeExperiments = parseInt(expResult[0]?.count || 0);

    // 3. Cash Flow (last 30 days)
    const inflowAmountColumn = resolveColumn(invoiceColumns, ['total_amount_cents', 'total_cents']);
    const inflowDateColumn = resolveColumn(invoiceColumns, ['paid_at', 'invoice_date', 'issue_date']);
    const inflowFilters: string[] = [];

    if (invoiceColumns.has('status')) {
      inflowFilters.push(`status = 'paid'`);
    }
    if (inflowDateColumn) {
      inflowFilters.push(`${inflowDateColumn} >= NOW() - INTERVAL '30 days'`);
    }

    const inflowSql = inflowAmountColumn
      ? `SELECT SUM(${inflowAmountColumn}) as total FROM invoices${inflowFilters.length ? ` WHERE ${inflowFilters.join(' AND ')}` : ''}`
      : `SELECT 0::bigint as total`;

    const inflowResult = await safeQuery<any>(inflowSql, [], [{ total: 0 }]);
    const monthlyInflow = parseInt(inflowResult[0]?.total || 0) / 100;

    const outflowAmountColumn = resolveColumn(expenseColumns, ['amount_eur_cents', 'amount_cents']);
    const outflowDateColumn = resolveColumn(expenseColumns, ['payment_date', 'expense_date']);
    const outflowFilters: string[] = [];

    if (expenseColumns.has('status')) {
      outflowFilters.push(`status = 'paid'`);
    } else if (expenseColumns.has('approved')) {
      outflowFilters.push(`approved = true`);
    }
    if (outflowDateColumn) {
      outflowFilters.push(`${outflowDateColumn} >= NOW() - INTERVAL '30 days'`);
    }

    const outflowSql = outflowAmountColumn
      ? `SELECT SUM(${outflowAmountColumn}) as total FROM expenses${outflowFilters.length ? ` WHERE ${outflowFilters.join(' AND ')}` : ''}`
      : `SELECT 0::bigint as total`;

    const outflowResult = await safeQuery<any>(outflowSql, [], [{ total: 0 }]);
    const monthlyOutflow = parseInt(outflowResult[0]?.total || 0) / 100;

    // 4. Runway calculation
    const balanceResult = await safeQuery<any>(`
      SELECT SUM(balance_cents) as total FROM account_balances 
      WHERE account_type = 'asset' AND account_code LIKE '11%'
    `, [], [{ total: 0 }]);
    const currentCash = (parseInt(balanceResult[0]?.total || 0) / 100);
    
    // Average burn (outflow) over last 3 months
    const burnFilters: string[] = [];
    if (expenseColumns.has('status')) {
      burnFilters.push(`status = 'paid'`);
    } else if (expenseColumns.has('approved')) {
      burnFilters.push(`approved = true`);
    }
    if (outflowDateColumn) {
      burnFilters.push(`${outflowDateColumn} >= NOW() - INTERVAL '90 days'`);
    }

    const burnSql = outflowAmountColumn
      ? `SELECT SUM(${outflowAmountColumn}) / 3 as avg_burn FROM expenses${burnFilters.length ? ` WHERE ${burnFilters.join(' AND ')}` : ''}`
      : `SELECT 0::bigint as avg_burn`;

    const burnResult = await safeQuery<any>(burnSql, [], [{ avg_burn: 0 }]);
    const avgBurn = (parseInt(burnResult[0]?.avg_burn || 0) / 100) || 1; // Avoid div by zero
    const runway = currentCash / avgBurn;

    // 5. CEO Metrics (LTV, Churn)
    const ltvResult = await safeQuery<any>(`
      SELECT AVG(total_paid_cents) / 100 as avg_ltv FROM customer_lifetime_value
    `, [], [{ avg_ltv: 0 }]);
    const ltv = parseFloat(ltvResult[0]?.avg_ltv || 0);

    const churnResult = await safeQuery<any>(
      publisherColumns.has('status') && publisherColumns.has('updated_at')
        ? `
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
    `
        : `SELECT 0::float as churn_rate`,
      [],
      [{ churn_rate: 0 }]
    );
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

function isMissingRelationError(error: any): boolean {
  if (!error) return false;
  return (
    error.code === '42P01' ||
    error.code === '42703' ||
    String(error.message || '').includes('does not exist')
  );
}

async function getTableColumns(table: string): Promise<Set<string>> {
  const rows = await queryReadOnly<{ column_name: string }>(
    `
    SELECT column_name
    FROM information_schema.columns
    WHERE table_schema = 'public' AND table_name = $1
  `,
    [table]
  );
  return new Set(rows.map((row) => row.column_name));
}

function resolveColumn(columns: Set<string>, candidates: string[]): string | null {
  for (const candidate of candidates) {
    if (columns.has(candidate)) {
      return candidate;
    }
  }
  return null;
}

async function safeQuery<T = unknown>(sql: string, params: unknown[], fallback: T[]): Promise<T[]> {
  try {
    return await queryReadOnly<T>(sql, params);
  } catch (err) {
    if (isMissingRelationError(err)) {
      logger.warn('Business analytics table missing, returning fallback', { error: (err as Error).message });
      return fallback;
    }
    throw err;
  }
}
