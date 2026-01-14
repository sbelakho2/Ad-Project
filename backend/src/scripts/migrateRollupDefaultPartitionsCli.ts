import dotenv from 'dotenv';
import fs from 'node:fs';
import path from 'node:path';
import pool from '../utils/postgres';
import logger from '../utils/logger';

const rollupTables = [
  'analytics_impression_rollups',
  'analytics_click_rollups',
  'analytics_revenue_rollups',
  'analytics_metrics_rollups',
] as const;

type RollupTable = (typeof rollupTables)[number];

const assertSafeIdentifier = (identifier: string): void => {
  if (!/^[a-z0-9_]+$/i.test(identifier)) {
    throw new Error(`Unsafe SQL identifier: ${identifier}`);
  }
};

// Load .env for scripts executed from /opt/adproject/backend.
// Prefer repo-root .env (../.env) when cwd is backend/.
// Never override existing env (e.g. if DATABASE_URL already provided).
if (!process.env.DATABASE_URL) {
  const cwd = process.cwd();
  const cwdBase = path.basename(cwd);
  const candidates =
    cwdBase === 'backend'
      ? [path.resolve(cwd, '../.env'), path.resolve(cwd, '.env')]
      : [path.resolve(cwd, '.env'), path.resolve(cwd, '../.env')];

  const envPath = [process.env.ENV_FILE, ...candidates].find((p) => !!p && fs.existsSync(p));
  if (envPath) {
    dotenv.config({ path: envPath });
  } else {
    dotenv.config();
  }

  // Safe diagnostics for production ops; never prints credentials.
  try {
    if (process.env.DATABASE_URL) {
      const u = new URL(process.env.DATABASE_URL);
      logger.info('Migrate rollup DEFAULT partitions CLI: env loaded', {
        cwd,
        envPath: envPath ?? '<default>',
        dbHost: u.hostname,
        dbPort: u.port,
        dbName: (u.pathname || '').replace(/^\//, ''),
        sslmode: u.searchParams.get('sslmode') ?? undefined,
      });
    } else {
      logger.warn('Migrate rollup DEFAULT partitions CLI: DATABASE_URL still missing after dotenv load', { cwd, envPath: envPath ?? '<default>' });
    }
  } catch {
    logger.warn('Migrate rollup DEFAULT partitions CLI: could not parse DATABASE_URL after dotenv load');
  }
}

const usage = (): void => {
  // eslint-disable-next-line no-console
  console.log(
    [
      'Usage:',
      '  node dist/src/scripts/migrateRollupDefaultPartitionsCli.js [--from YYYY-MM] [--to YYYY-MM] [--tables t1,t2] [--execute]',
      '',
      'Notes:',
      '  - Scans DEFAULT partitions ("*_p_default") for rollup tables and moves rows into the correct monthly partitions.',
      '  - This is a one-off remediation when historical rollup rows were inserted into DEFAULT.',
      '  - By default this is a DRY RUN. Pass --execute to actually move data.',
      '  - --from is inclusive; --to is exclusive. Example: --from 2025-12 --to 2026-02 (moves Dec 2025 + Jan 2026).',
    ].join('\n')
  );
};

const parseArg = (name: string): string | undefined => {
  const argv = process.argv.slice(2);
  const idx = argv.indexOf(name);
  if (idx === -1) return undefined;
  return argv[idx + 1];
};

const hasFlag = (name: string): boolean => process.argv.slice(2).includes(name);

const parseMonth = (s: string): { y: number; m: number } => {
  if (!/^\d{4}-\d{2}$/.test(s)) {
    throw new Error(`Invalid month format: ${s} (expected YYYY-MM)`);
  }
  const [y, m] = s.split('-').map(Number);
  if (!Number.isFinite(y) || !Number.isFinite(m) || m < 1 || m > 12) {
    throw new Error(`Invalid month value: ${s}`);
  }
  return { y, m };
};

const monthStartYmd = (y: number, m: number): string => `${String(y).padStart(4, '0')}-${String(m).padStart(2, '0')}-01`;

const addMonths = ({ y, m }: { y: number; m: number }, delta: number): { y: number; m: number } => {
  const total = (y * 12 + (m - 1)) + delta;
  const ny = Math.floor(total / 12);
  const nm = (total % 12) + 1;
  return { y: ny, m: nm };
};

const monthSuffix = ({ y, m }: { y: number; m: number }): string => `${String(y).padStart(4, '0')}_${String(m).padStart(2, '0')}`;

const monthKey = ({ y, m }: { y: number; m: number }): string => `${String(y).padStart(4, '0')}-${String(m).padStart(2, '0')}`;

const withinRange = (month: { y: number; m: number }, from?: { y: number; m: number }, to?: { y: number; m: number }): boolean => {
  const k = month.y * 12 + (month.m - 1);
  if (from) {
    const f = from.y * 12 + (from.m - 1);
    if (k < f) return false;
  }
  if (to) {
    const t = to.y * 12 + (to.m - 1);
    if (k >= t) return false;
  }
  return true;
};

async function migrateOneMonth(opts: {
  client: { query: (sql: string, params?: unknown[]) => Promise<{ rows: any[] }> };
  parent: RollupTable;
  month: { y: number; m: number };
  execute: boolean;
}): Promise<void> {
  const { client, parent, month, execute } = opts;
  const defaultPartition = `${parent}_p_default`;
  const suffix = monthSuffix(month);
  const partition = `${parent}_${suffix}`;

  assertSafeIdentifier(parent);
  assertSafeIdentifier(defaultPartition);
  assertSafeIdentifier(partition);

  const from = monthStartYmd(month.y, month.m);
  const next = addMonths(month, 1);
  const to = monthStartYmd(next.y, next.m);

  const countRes = await client.query(
    `SELECT count(*)::bigint AS rows FROM ONLY public.${defaultPartition} WHERE bucket_date >= $1::date AND bucket_date < $2::date`,
    [from, to]
  );
  const rowsInDefault = BigInt(countRes.rows[0]?.rows ?? 0);

  if (rowsInDefault === 0n) {
    logger.info('Rollup DEFAULT migration: nothing to move', { parent, month: monthKey(month), from, to });
    return;
  }

  logger.info('Rollup DEFAULT migration: month has rows in DEFAULT', {
    parent,
    month: monthKey(month),
    from,
    to,
    rowsInDefault: rowsInDefault.toString(),
    mode: execute ? 'execute' : 'dry-run',
  });

  if (!execute) return;

  await client.query('BEGIN');
  try {
    await client.query("SET LOCAL lock_timeout = '30s'");
    await client.query("SET LOCAL statement_timeout = '0'");

    await client.query(`CREATE TEMP TABLE tmp_rollup_move (LIKE public.${parent} INCLUDING ALL) ON COMMIT DROP`);
    await client.query(
      `INSERT INTO tmp_rollup_move SELECT * FROM ONLY public.${defaultPartition} WHERE bucket_date >= $1::date AND bucket_date < $2::date`,
      [from, to]
    );

    const tmpCountRes = await client.query(`SELECT count(*)::bigint AS rows FROM tmp_rollup_move`);
    const tmpRows = BigInt(tmpCountRes.rows[0]?.rows ?? 0);

    await client.query(
      `DELETE FROM ONLY public.${defaultPartition} WHERE bucket_date >= $1::date AND bucket_date < $2::date`,
      [from, to]
    );

    // Create the partition after removing rows from DEFAULT for that range.
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
              '${from}',
              '${to}'
            );
          END IF;
        END IF;
      END $$;
    `);

    const insertRes = await client.query(
      `WITH ins AS (
         INSERT INTO public.${parent}
         SELECT * FROM tmp_rollup_move
         ON CONFLICT DO NOTHING
         RETURNING 1
       )
       SELECT count(*)::bigint AS inserted FROM ins`
    );

    const inserted = BigInt(insertRes.rows[0]?.inserted ?? 0);

    await client.query('COMMIT');

    logger.info('Rollup DEFAULT migration: moved rows', {
      parent,
      month: monthKey(month),
      tmpRows: tmpRows.toString(),
      inserted: inserted.toString(),
      droppedFromDefault: tmpRows.toString(),
    });
  } catch (err) {
    await client.query('ROLLBACK');
    throw err;
  }
}

async function main(): Promise<void> {
  if (hasFlag('--help') || hasFlag('-h')) {
    usage();
    process.exit(0);
  }

  const fromArg = parseArg('--from');
  const toArg = parseArg('--to');
  const tablesArg = parseArg('--tables');
  const execute = hasFlag('--execute');

  const from = fromArg ? parseMonth(fromArg) : undefined;
  const to = toArg ? parseMonth(toArg) : undefined;

  const selectedTables: RollupTable[] = (() => {
    if (!tablesArg) return [...rollupTables];
    const parts = tablesArg.split(',').map((s) => s.trim()).filter(Boolean);
    const set = new Set(parts);
    const filtered = rollupTables.filter((t) => set.has(t));
    if (filtered.length === 0) {
      throw new Error(`No valid --tables specified. Allowed: ${rollupTables.join(', ')}`);
    }
    return filtered;
  })();

  logger.info('Rollup DEFAULT migration: starting', {
    execute,
    from: fromArg ?? undefined,
    to: toArg ?? undefined,
    tables: selectedTables,
  });

  const client = await pool.connect();
  try {
    for (const parent of selectedTables) {
      const defaultPartition = `${parent}_p_default`;
      assertSafeIdentifier(parent);
      assertSafeIdentifier(defaultPartition);

      const monthsRes = await client.query<{ month_key: string; rows: string }>(
        `SELECT to_char(date_trunc('month', bucket_date)::date, 'YYYY-MM') AS month_key, count(*)::bigint AS rows
         FROM ONLY public.${defaultPartition}
         GROUP BY 1
         ORDER BY 1`
      );

      if (!monthsRes.rows.length) {
        logger.info('Rollup DEFAULT migration: no rows in DEFAULT', { parent });
        continue;
      }

      for (const row of monthsRes.rows) {
        const monthKeyStr = String(row.month_key);
        const month = parseMonth(monthKeyStr);
        if (!withinRange(month, from, to)) continue;

        await migrateOneMonth({ client, parent, month, execute });
      }
    }
  } finally {
    client.release();
  }

  logger.info('Rollup DEFAULT migration: completed');
}

main().catch((err) => {
  logger.error('Rollup DEFAULT migration: failed', { error: (err as Error).message });
  process.exit(1);
});
