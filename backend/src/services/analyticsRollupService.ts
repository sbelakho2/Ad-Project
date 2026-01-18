import pool from '../utils/postgres';
import logger from '../utils/logger';

export interface RunAnalyticsRollupsParams {
  startInclusive: Date;
  endExclusive: Date;
}

const formatUtcDate = (d: Date): string => d.toISOString().slice(0, 10);

const formatUtcMonthSuffix = (d: Date): string => {
  const y = d.getUTCFullYear();
  const m = String(d.getUTCMonth() + 1).padStart(2, '0');
  return `${y}_${m}`;
};

const firstOfUtcMonth = (d: Date): Date => new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), 1));

const addUtcMonths = (d: Date, months: number): Date =>
  new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + months, 1));

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

async function ensureMonthlyRollupPartitions(
  client: { query: (sql: string) => Promise<unknown> },
  startInclusive: Date,
  endExclusive: Date
): Promise<void> {
  const isDefaultPartitionConstraintViolation = (err: unknown): boolean => {
    const anyErr = err as { message?: string };
    const msg = (anyErr?.message ?? '').toLowerCase();
    return msg.includes('updated partition constraint for default partition') && msg.includes('would be violated by some row');
  };

  // The intended production design is monthly RANGE partitions on bucket_date.
  // If the ops script/cron isn't running, rollups accumulate in the DEFAULT partition,
  // which grows unbounded and becomes a performance + maintenance risk.
  const startMonth = firstOfUtcMonth(startInclusive);
  const endMonth = firstOfUtcMonth(endExclusive);

  for (let cur = startMonth; cur.getTime() <= endMonth.getTime(); cur = addUtcMonths(cur, 1)) {
    const next = addUtcMonths(cur, 1);
    const fromYmd = formatUtcDate(cur);
    const toYmd = formatUtcDate(next);
    const suffix = formatUtcMonthSuffix(cur);

    for (const parent of rollupTables) {
      const partition = `${parent}_${suffix}`;

      // Hard safety: only allow expected identifier characters.
      if (!/^[a-z0-9_]+$/i.test(partition) || !/^[a-z0-9_]+$/i.test(parent)) {
        throw new Error(`Invalid rollup partition identifier: parent=${parent} partition=${partition}`);
      }

      try {
        await client.query(`
          DO $$
          BEGIN
            IF EXISTS (
              SELECT 1 FROM pg_class c
              JOIN pg_namespace n ON n.oid = c.relnamespace
              WHERE n.nspname='public' AND c.relname = '${parent}'
            ) THEN
              IF NOT EXISTS (
                SELECT 1 FROM pg_tables
                WHERE schemaname='public' AND tablename='${partition}'
              ) THEN
                EXECUTE format(
                  'CREATE TABLE public.%I PARTITION OF public.%I FOR VALUES FROM (%L) TO (%L)',
                  '${partition}',
                  '${parent}',
                  '${fromYmd}',
                  '${toYmd}'
                );
              END IF;
            END IF;
          END $$;
        `);
      } catch (err) {
        // If DEFAULT already contains rows that belong to the new partition range,
        // Postgres refuses to create/attach the partition until those rows are moved.
        // Don't fail rollups here; warn and let ops migrate DEFAULT rows.
        if (isDefaultPartitionConstraintViolation(err)) {
          logger.warn('Analytics rollups: cannot create monthly partition because DEFAULT contains rows in-range; migrate DEFAULT first', {
            parent,
            partition,
            fromYmd,
            toYmd,
            error: (err as Error).message,
          });
          continue;
        }
        throw err;
      }
    }
  }
}

export async function runAnalyticsRollups({ startInclusive, endExclusive }: RunAnalyticsRollupsParams): Promise<void> {
  const startDay = formatUtcDate(startInclusive);
  const endDay = formatUtcDate(endExclusive);

  const ready = await rollupTablesExist();
  if (!ready) return;

  logger.info('Analytics rollups: starting', { startDay, endDay });

  const client = await pool.connect();
  try {
    const maxAttempts = 3;
    const isRetryableLockError = (err: unknown): boolean => {
      const anyErr = err as { code?: string; message?: string };
      const code = anyErr?.code;
      const msg = (anyErr?.message ?? '').toLowerCase();
      return code === '55P03' || code === '40P01' || msg.includes('lock timeout') || msg.includes('deadlock');
    };

    const sleepMs = async (ms: number): Promise<void> =>
      new Promise(resolve => {
        setTimeout(resolve, ms);
      });

    for (let attempt = 1; attempt <= maxAttempts; attempt++) {
      try {
        // Apply lock timeout for the session during partition creation too.
        await client.query("SET lock_timeout = '30s'");

        // Ensure monthly rollup partitions exist (keeps rollups bounded and maintenance-friendly).
        await ensureMonthlyRollupPartitions(client, startInclusive, endExclusive);

        await client.query('BEGIN');
        await client.query("SET LOCAL statement_timeout = '120s'");
        // Production reality: rollups compete with reads/writes; 5s is too aggressive and causes wedge.
        await client.query("SET LOCAL lock_timeout = '30s'");

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
        COALESCE(NULLIF(i.meta->>'ad_unit_id', ''), COALESCE(i.placement_id::text, '')) AS ad_unit_id,
        -- Use adapter_name for grouping, not adapter_id (which is a UUID per-request)
        COALESCE(i.adapter_name, '') AS adapter_id,
        LEFT(COALESCE(NULLIF(i.meta->>'country_code', ''), 'ZZ'), 2)::char(2) AS country_code,
        COUNT(*)::bigint AS impression_count,
        COUNT(*) FILTER (WHERE i.filled IS TRUE)::bigint AS filled_count,
        now() AS updated_at
      FROM analytics_impressions i
      WHERE i.is_test_mode = false
        AND i.observed_at >= $1::timestamptz
        AND i.observed_at < $2::timestamptz
      GROUP BY 1,2,3,4,5,6
      ON CONFLICT (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
      DO UPDATE SET
        impression_count = EXCLUDED.impression_count,
        filled_count = EXCLUDED.filled_count,
        updated_at = now()
      `,
      [startInclusive, endExclusive]
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
          NULLIF(i.meta->>'ad_unit_id', ''),
          NULLIF(c.meta->>'ad_unit_id', ''),
          COALESCE(c.placement_id::text, '')
        ) AS ad_unit_id,
        -- Use adapter_name for grouping, not adapter_id (which is a UUID per-request)
        COALESCE(c.adapter_name, '') AS adapter_id,
        LEFT(
          COALESCE(
            NULLIF(i.meta->>'country_code', ''),
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
        AND c.observed_at >= $1::timestamptz
        AND c.observed_at < $2::timestamptz
      GROUP BY 1,2,3,4,5,6
      ON CONFLICT (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
      DO UPDATE SET
        click_count = EXCLUDED.click_count,
        updated_at = now()
      `,
      [startInclusive, endExclusive]
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
        revenue_bucket_lt_1_count,
        revenue_bucket_1_5_count,
        revenue_bucket_gt_5_count,
        revenue_event_count,
        updated_at
      )
      SELECT
        (r.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
        r.publisher_id::text AS publisher_id,
        COALESCE(r.app_id::text, '') AS app_id,
        COALESCE(NULLIF(r.metadata->>'ad_unit_id', ''), COALESCE(r.placement_id::text, '')) AS ad_unit_id,
        -- Use adapter_name for grouping, not adapter_id (which is a UUID per-request)
        COALESCE(r.adapter_name, '') AS adapter_id,
        LEFT(COALESCE(NULLIF(r.country_code::text, ''), 'ZZ'), 2)::char(2) AS country_code,
        COALESCE(SUM(r.revenue_usd), 0) AS total_revenue,
        COALESCE(AVG(r.revenue_usd), 0) AS avg_revenue,
        COALESCE(MAX(r.revenue_usd), 0) AS max_revenue,
        COUNT(*) FILTER (WHERE r.revenue_usd IS NOT NULL AND r.revenue_usd < 1)::bigint AS revenue_bucket_lt_1_count,
        COUNT(*) FILTER (WHERE r.revenue_usd IS NOT NULL AND r.revenue_usd >= 1 AND r.revenue_usd <= 5)::bigint AS revenue_bucket_1_5_count,
        COUNT(*) FILTER (WHERE r.revenue_usd IS NOT NULL AND r.revenue_usd > 5)::bigint AS revenue_bucket_gt_5_count,
        COUNT(*)::bigint AS revenue_event_count,
        now() AS updated_at
      FROM analytics_revenue_events r
      WHERE r.is_test_mode = false
        AND r.observed_at >= $1::timestamptz
        AND r.observed_at < $2::timestamptz
      GROUP BY 1,2,3,4,5,6
      ON CONFLICT (bucket_date, publisher_id, app_id, ad_unit_id, adapter_id, country_code)
      DO UPDATE SET
        total_revenue = EXCLUDED.total_revenue,
        avg_revenue = EXCLUDED.avg_revenue,
        max_revenue = EXCLUDED.max_revenue,
        revenue_bucket_lt_1_count = EXCLUDED.revenue_bucket_lt_1_count,
        revenue_bucket_1_5_count = EXCLUDED.revenue_bucket_1_5_count,
        revenue_bucket_gt_5_count = EXCLUDED.revenue_bucket_gt_5_count,
        revenue_event_count = EXCLUDED.revenue_event_count,
        updated_at = now()
      `,
      [startInclusive, endExclusive]
    );

    // 4) Metrics rollups (combine impressions + clicks + revenue)
    await client.query(
      `
      WITH imp AS (
        SELECT
          (i.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
          i.publisher_id::text AS publisher_id,
          COALESCE(i.app_id::text, '') AS app_id,
          COALESCE(NULLIF(i.meta->>'ad_unit_id', ''), COALESCE(i.placement_id::text, '')) AS ad_unit_id,
          COALESCE(i.adapter_id, '') AS adapter_id,
          LEFT(COALESCE(NULLIF(i.meta->>'country_code', ''), 'ZZ'), 2)::char(2) AS country_code,
          COUNT(*)::bigint AS impression_count,
          COUNT(*) FILTER (WHERE i.filled IS TRUE)::bigint AS filled_count
        FROM analytics_impressions i
        WHERE i.is_test_mode = false
          AND i.observed_at >= $1::timestamptz
          AND i.observed_at < $2::timestamptz
        GROUP BY 1,2,3,4,5,6
      ),
      clk AS (
        SELECT
          (c.observed_at AT TIME ZONE 'UTC')::date AS bucket_date,
          c.publisher_id::text AS publisher_id,
          COALESCE(c.app_id::text, '') AS app_id,
          COALESCE(
            NULLIF(i.meta->>'ad_unit_id', ''),
            NULLIF(c.meta->>'ad_unit_id', ''),
            COALESCE(c.placement_id::text, '')
          ) AS ad_unit_id,
          COALESCE(c.adapter_id, '') AS adapter_id,
          LEFT(
            COALESCE(
              NULLIF(i.meta->>'country_code', ''),
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
          AND c.observed_at >= $1::timestamptz
          AND c.observed_at < $2::timestamptz
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
          AND r.observed_at >= $1::timestamptz
          AND r.observed_at < $2::timestamptz
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
      [startInclusive, endExclusive]
    );

    // Update publisher-level daily summary for fast dashboard queries
    await client.query(
      `
      INSERT INTO analytics_publisher_daily_summary (bucket_date, publisher_id, total_revenue, impression_count, click_count, filled_count)
      SELECT
        bucket_date,
        publisher_id,
        SUM(total_revenue),
        SUM(impression_count),
        SUM(click_count),
        SUM(filled_count)
      FROM analytics_metrics_rollups
      WHERE bucket_date >= $1::date AND bucket_date < $2::date
      GROUP BY bucket_date, publisher_id
      ON CONFLICT (bucket_date, publisher_id)
      DO UPDATE SET
        total_revenue = EXCLUDED.total_revenue,
        impression_count = EXCLUDED.impression_count,
        click_count = EXCLUDED.click_count,
        filled_count = EXCLUDED.filled_count
      `,
      [startInclusive, endExclusive]
    );

        await client.query('COMMIT');
        logger.info('Analytics rollups: completed', { startDay, endDay, attempt });
        return;
      } catch (error) {
        try {
          await client.query('ROLLBACK');
        } catch {
          // ignore
        }

        if (isRetryableLockError(error) && attempt < maxAttempts) {
          const backoffMs = attempt === 1 ? 1000 : attempt === 2 ? 3000 : 10000;
          logger.warn('Analytics rollups: retrying after lock contention', {
            startDay,
            endDay,
            attempt,
            backoffMs,
            error: (error as Error)?.message,
          });
          await sleepMs(backoffMs);
          continue;
        }

        throw error;
      } finally {
        // Clean up any session-level overrides.
        try {
          await client.query('RESET lock_timeout');
        } catch {
          // ignore
        }
      }
    }
  } finally {
    client.release();
  }
}
