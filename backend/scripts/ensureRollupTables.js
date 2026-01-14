#!/usr/bin/env node
/*
 * Ensures the analytics rollup tables exist in Postgres.
 *
 * Intended to make rollup-table rollout fully automatable:
 * - If tables already exist: exit 0.
 * - If missing: applies ONLY the rollup-table migration (idempotent) via runMigrationsV2.
 *
 * Safe to run repeatedly.
 */

require('dotenv/config');

const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { Pool } = require('pg');
const { buildSslConfig } = require('./dbSsl');

const ROLLUP_MIGRATIONS = [
  '20251201_150500_analytics_rollup_tables',
  '20260108_120000_analytics_rollup_revenue_histograms',
];
const ROLLUP_TABLES = [
  'analytics_impression_rollups',
  'analytics_click_rollups',
  'analytics_revenue_rollups',
  'analytics_metrics_rollups',
];

async function rollupTablesExist(pool) {
  const res = await pool.query(
    `
      SELECT table_name
      FROM information_schema.tables
      WHERE table_schema = 'public'
        AND table_name = ANY($1::text[])
    `,
    [ROLLUP_TABLES]
  );

  const found = new Set(res.rows.map((r) => r.table_name));
  const missing = ROLLUP_TABLES.filter((t) => !found.has(t));

  return { ok: missing.length === 0, missing };
}

async function main() {
  const connectionString = process.env.DATABASE_URL;
  if (!connectionString) {
    console.error('[ensureRollupTables] DATABASE_URL is not configured.');
    process.exit(1);
  }

  const pool = new Pool({
    connectionString,
    ssl: buildSslConfig(),
    max: 2,
    idleTimeoutMillis: 10_000,
  });

  try {
    const { ok, missing } = await rollupTablesExist(pool);
    if (ok) {
      console.log('[ensureRollupTables] Rollup tables already exist.');
      // Still apply any rollup schema evolutions (idempotent migrations).
    }

    if (!ok) {
      console.log('[ensureRollupTables] Rollup tables missing; applying migrations.', {
        migrations: ROLLUP_MIGRATIONS,
        missing,
      });
    } else {
      console.log('[ensureRollupTables] Rollup tables exist; ensuring latest rollup migrations are applied.', {
        migrations: ROLLUP_MIGRATIONS,
      });
    }
  } finally {
    await pool.end();
  }

  const runnerPath = path.resolve(__dirname, 'runMigrationsV2.js');

  for (const migration of ROLLUP_MIGRATIONS) {
    const result = spawnSync(process.execPath, [runnerPath, '--only', migration], {
      stdio: 'inherit',
      env: process.env,
    });
    const code = typeof result.status === 'number' ? result.status : 1;
    if (code !== 0) {
      process.exit(code);
    }
  }
}

main().catch((err) => {
  console.error('[ensureRollupTables] Failed', err);
  process.exit(1);
});
