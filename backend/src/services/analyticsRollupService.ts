import pool from '../utils/postgres';
import logger from '../utils/logger';

export interface RunAnalyticsRollupsParams {
  startInclusive: Date;
  endExclusive: Date;
}

const formatUtcDate = (d: Date): string => d.toISOString().slice(0, 10);

const rollupTables = [
  'analytics_impression_rollups',
  'analytics_click_rollups',
  'analytics_revenue_rollups',
  'analytics_metrics_rollups',
] as const;

async function rollupTablesExist(): Promise<boolean> {
  const res = await pool.query<{
    table_name: string;
  }>(
    `
      SELECT table_name
      FROM information_schema.tables
      WHERE table_schema = 'public'
        AND table_name = ANY($1::text[])
    `,
    [rollupTables]
  );

  const found = new Set(res.rows.map(r => r.table_name));
  const missing = rollupTables.filter(t => !found.has(t));
  if (missing.length > 0) {
    logger.warn('Analytics rollup tables missing; rollups disabled until migrations are applied', { missing });
    return false;
  }
  return true;
}

export async function runAnalyticsRollups({ startInclusive, endExclusive }: RunAnalyticsRollupsParams): Promise<void> {
  const startDay = formatUtcDate(startInclusive);
  const endDay = formatUtcDate(endExclusive);

  const ready = await rollupTablesExist();
  if (!ready) return;

  logger.info('Analytics rollups: starting', { startDay, endDay });

  const client = await pool.connect();
  try {
    await client.query('BEGIN');
    await client.query("SET LOCAL statement_timeout = '120s'");
    await client.query("SET LOCAL lock_timeout = '5s'");

    // 1) Impression rollups
    await client.query(
      `
      INSERT INTO analytics_impression_rollups (
        bucket_date,
        publisher_id,
        app_id,
        ad_unit_id,
        adapter_id,
        country_code,
        impression_count,
        filled_count,
        updated_at
      )
      SELECT
        (i.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
        i.publisher_id::text AS publisher_id,
        COALESCE(i.app_id::text, '') AS app_id,
        COALESCE(NULLIF(i.ad_unit_id::text, ''), NULLIF(i.meta->>'ad_unit_id', ''), COALESCE(i.placement_id::text, '')) AS ad_unit_id,
        COALESCE(i.adapter_id, '') AS adapter_id,
        LEFT(COALESCE(NULLIF(i.country_code::text, ''), NULLIF(i.meta->>'country_code', ''), 'ZZ'), 2)::char(2) AS country_code,
        COUNT(*)::bigint AS impression_count,
        COUNT(*) FILTER (WHERE i.filled IS TRUE)::bigint AS filled_count,
        now() AS updated_at
      FROM analytics_impressions i
      WHERE i.is_test_mode = false
        AND i.observed_at >= $1::date
        AND i.observed_at < $2::date
      GROUP BY 1,2,3,4,5,6
      ON CONFLICT (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
      DO UPDATE SET
        impression_count = EXCLUDED.impression_count,
        filled_count = EXCLUDED.filled_count,
        updated_at = now()
      `,
      [startDay, endDay]
    );

    // 2) Click rollups (derived from analytics_clicks; align ad_unit_id with impressions when possible)
    await client.query(
      `
      INSERT INTO analytics_click_rollups (
        bucket_date,
        publisher_id,
        app_id,
        ad_unit_id,
        adapter_id,
        country_code,
        click_count,
        updated_at
      )
      SELECT
        (c.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
        c.publisher_id::text AS publisher_id,
        COALESCE(c.app_id::text, '') AS app_id,
        COALESCE(
          NULLIF(i.ad_unit_id::text, ''),
          NULLIF(i.meta->>'ad_unit_id', ''),
          NULLIF(c.meta->>'ad_unit_id', ''),
          COALESCE(c.placement_id::text, '')
        ) AS ad_unit_id,
        COALESCE(c.adapter_id, '') AS adapter_id,
        LEFT(
          COALESCE(
            NULLIF(i.country_code::text, ''),
            NULLIF(i.meta->>'country_code', ''),
            NULLIF(c.country_code::text, ''),
            NULLIF(c.meta->>'country_code', ''),
            'ZZ'
          ),
          2
        )::char(2) AS country_code,
        COUNT(*)::bigint AS click_count,
        now() AS updated_at
      FROM analytics_clicks c
      LEFT JOIN analytics_impressions i
        ON i.event_id = c.impression_id
      WHERE c.is_test_mode = false
        AND c.observed_at >= $1::date
        AND c.observed_at < $2::date
      GROUP BY 1,2,3,4,5,6
      ON CONFLICT (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
      DO UPDATE SET
        click_count = EXCLUDED.click_count,
        updated_at = now()
      `,
      [startDay, endDay]
    );

    // 3) Revenue rollups
    await client.query(
      `
      INSERT INTO analytics_revenue_rollups (
        bucket_date,
        publisher_id,
        app_id,
        ad_unit_id,
        adapter_id,
        country_code,
        total_revenue,
        avg_revenue,
        max_revenue,
        revenue_event_count,
        updated_at
      )
      SELECT
        (r.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
        r.publisher_id::text AS publisher_id,
        COALESCE(r.app_id::text, '') AS app_id,
        COALESCE(NULLIF(r.metadata->>'ad_unit_id', ''), COALESCE(r.placement_id::text, '')) AS ad_unit_id,
        COALESCE(r.adapter_id, '') AS adapter_id,
        LEFT(COALESCE(NULLIF(r.country_code::text, ''), 'ZZ'), 2)::char(2) AS country_code,
        COALESCE(SUM(r.revenue_usd), 0) AS total_revenue,
        COALESCE(AVG(r.revenue_usd), 0) AS avg_revenue,
        COALESCE(MAX(r.revenue_usd), 0) AS max_revenue,
        COUNT(*)::bigint AS revenue_event_count,
        now() AS updated_at
      FROM analytics_revenue_events r
      WHERE r.is_test_mode = false
        AND r.observed_at >= $1::date
        AND r.observed_at < $2::date
      GROUP BY 1,2,3,4,5,6
      ON CONFLICT (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
      DO UPDATE SET
        total_revenue = EXCLUDED.total_revenue,
        avg_revenue = EXCLUDED.avg_revenue,
        max_revenue = EXCLUDED.max_revenue,
        revenue_event_count = EXCLUDED.revenue_event_count,
        updated_at = now()
      `,
      [startDay, endDay]
    );

    // 4) Metrics rollups (combine impressions + clicks + revenue)
    await client.query(
      `
      WITH imp AS (
        SELECT
          (i.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
          i.publisher_id::text AS publisher_id,
          COALESCE(i.app_id::text, '') AS app_id,
          COALESCE(NULLIF(i.ad_unit_id::text, ''), NULLIF(i.meta->>'ad_unit_id', ''), COALESCE(i.placement_id::text, '')) AS ad_unit_id,
          COALESCE(i.adapter_id, '') AS adapter_id,
          LEFT(COALESCE(NULLIF(i.country_code::text, ''), NULLIF(i.meta->>'country_code', ''), 'ZZ'), 2)::char(2) AS country_code,
          COUNT(*)::bigint AS impression_count,
          COUNT(*) FILTER (WHERE i.filled IS TRUE)::bigint AS filled_count
        FROM analytics_impressions i
        WHERE i.is_test_mode = false
          AND i.observed_at >= $1::date
          AND i.observed_at < $2::date
        GROUP BY 1,2,3,4,5,6
      ),
      clk AS (
        SELECT
          (c.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
          c.publisher_id::text AS publisher_id,
          COALESCE(c.app_id::text, '') AS app_id,
          COALESCE(
            NULLIF(i.ad_unit_id::text, ''),
            NULLIF(i.meta->>'ad_unit_id', ''),
            NULLIF(c.meta->>'ad_unit_id', ''),
            COALESCE(c.placement_id::text, '')
          ) AS ad_unit_id,
          COALESCE(c.adapter_id, '') AS adapter_id,
          LEFT(
            COALESCE(
              NULLIF(i.country_code::text, ''),
              NULLIF(i.meta->>'country_code', ''),
              NULLIF(c.country_code::text, ''),
              NULLIF(c.meta->>'country_code', ''),
              'ZZ'
            ),
            2
          )::char(2) AS country_code,
          COUNT(*)::bigint AS click_count
        FROM analytics_clicks c
        LEFT JOIN analytics_impressions i
          ON i.event_id = c.impression_id
        WHERE c.is_test_mode = false
          AND c.observed_at >= $1::date
          AND c.observed_at < $2::date
        GROUP BY 1,2,3,4,5,6
      ),
      rev AS (
        SELECT
          (r.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
          r.publisher_id::text AS publisher_id,
          COALESCE(r.app_id::text, '') AS app_id,
          COALESCE(NULLIF(r.metadata->>'ad_unit_id', ''), COALESCE(r.placement_id::text, '')) AS ad_unit_id,
          COALESCE(r.adapter_id, '') AS adapter_id,
          LEFT(COALESCE(NULLIF(r.country_code::text, ''), 'ZZ'), 2)::char(2) AS country_code,
          COALESCE(SUM(r.revenue_usd), 0) AS total_revenue
        FROM analytics_revenue_events r
        WHERE r.is_test_mode = false
          AND r.observed_at >= $1::date
          AND r.observed_at < $2::date
        GROUP BY 1,2,3,4,5,6
      ),
      dims AS (
        SELECT bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code FROM imp
        UNION
        SELECT bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code FROM clk
        UNION
        SELECT bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code FROM rev
      )
      INSERT INTO analytics_metrics_rollups (
        bucket_date,
        publisher_id,
        app_id,
        ad_unit_id,
        adapter_id,
        country_code,
        impression_count,
        filled_count,
        click_count,
        total_revenue,
        ctr,
        fill_rate,
        ecpm,
        updated_at
      )
      SELECT
        d.bucket_date,
        d.publisher_id,
        d.app_id,
        d.ad_unit_id,
        d.adapter_id,
        d.country_code,
        COALESCE(imp.impression_count, 0) AS impression_count,
        COALESCE(imp.filled_count, 0) AS filled_count,
        COALESCE(clk.click_count, 0) AS click_count,
        COALESCE(rev.total_revenue, 0) AS total_revenue,
         CASE WHEN COALESCE(imp.impression_count, 0) = 0 THEN 0
           ELSE (COALESCE(clk.click_count, 0)::numeric / imp.impression_count::numeric) * 100
        END AS ctr,
         CASE WHEN COALESCE(imp.impression_count, 0) = 0 THEN 0
           ELSE (COALESCE(imp.filled_count, 0)::numeric / imp.impression_count::numeric) * 100
        END AS fill_rate,
        CASE WHEN COALESCE(imp.impression_count, 0) = 0 THEN 0
             ELSE (COALESCE(rev.total_revenue, 0) / imp.impression_count::numeric) * 1000
        END AS ecpm,
        now() AS updated_at
      FROM dims d
      LEFT JOIN imp ON imp.bucket_date = d.bucket_date
        AND imp.publisher_id = d.publisher_id
        AND imp.app_id = d.app_id
        AND imp.ad_unit_id = d.ad_unit_id
        AND imp.adapter_id = d.adapter_id
        AND imp.country_code = d.country_code
      LEFT JOIN clk ON clk.bucket_date = d.bucket_date
        AND clk.publisher_id = d.publisher_id
        AND clk.app_id = d.app_id
        AND clk.ad_unit_id = d.ad_unit_id
        AND clk.adapter_id = d.adapter_id
        AND clk.country_code = d.country_code
      LEFT JOIN rev ON rev.bucket_date = d.bucket_date
        AND rev.publisher_id = d.publisher_id
        AND rev.app_id = d.app_id
        AND rev.ad_unit_id = d.ad_unit_id
        AND rev.adapter_id = d.adapter_id
        AND rev.country_code = d.country_code
      ON CONFLICT (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
      DO UPDATE SET
        impression_count = EXCLUDED.impression_count,
        filled_count = EXCLUDED.filled_count,
        click_count = EXCLUDED.click_count,
        total_revenue = EXCLUDED.total_revenue,
        ctr = EXCLUDED.ctr,
        fill_rate = EXCLUDED.fill_rate,
        ecpm = EXCLUDED.ecpm,
        updated_at = now()
      `,
      [startDay, endDay]
    );

    await client.query('COMMIT');
    logger.info('Analytics rollups: completed', { startDay, endDay });
  } catch (error) {
    await client.query('ROLLBACK');
    logger.error('Analytics rollups: failed', { startDay, endDay, error: (error as Error).message });
    throw error;
  } finally {
    client.release();
  }
}
