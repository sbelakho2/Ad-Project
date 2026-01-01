#!/usr/bin/env node
// Postgres migrations runner with up/down and checksums (SQL-first)
require('dotenv/config');
const path = require('node:path');
const fs = require('node:fs');
const fsp = require('node:fs/promises');
const crypto = require('node:crypto');
const { Pool } = require('pg');

const connectionString = process.env.DATABASE_URL;
if (!connectionString) {
  console.error('DATABASE_URL environment variable is not configured.');
  process.exit(1);
}

const pool = new Pool({
  connectionString,
  ssl: process.env.DATABASE_SSL === 'true' ? { rejectUnauthorized: false } : undefined,
  max: parseInt(process.env.DATABASE_POOL_MAX || '10', 10),
  idleTimeoutMillis: parseInt(process.env.DATABASE_POOL_IDLE_MS || '30000', 10),
});

const MIGRATIONS_TABLE = 'schema_migrations_v2';
const PG_DIR = path.resolve(__dirname, '../migrations/postgres');
const LEGACY_DIR = path.resolve(__dirname, '../migrations');

function sha256(txt) { return crypto.createHash('sha256').update(txt).digest('hex'); }

async function ensureTables(client) {
  await client.query(`CREATE TABLE IF NOT EXISTS ${MIGRATIONS_TABLE} (
    version TEXT PRIMARY KEY,
    checksum TEXT NOT NULL,
    direction TEXT NOT NULL CHECK (direction IN ('up','down')),
    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
  )`);
}

async function listPairedMigrations(dir) {
  if (!fs.existsSync(dir)) return [];
  const files = (await fsp.readdir(dir)).filter(f => f.endsWith('.sql'));
  const map = new Map();
  for (const f of files) {
    const [version, direction, ...rest] = f.split('.sql')[0].split('.').join('').split(/\.(?=up|down)/);
  }
  // Simpler: group by base name before last dot
  const groups = {};
  for (const f of files) {
    const isUp = f.endsWith('.up.sql');
    const isDown = f.endsWith('.down.sql');
    if (!isUp && !isDown) continue;
    const base = f.replace(/\.(up|down)\.sql$/, '');
    groups[base] = groups[base] || { base, up: null, down: null };
    if (isUp) groups[base].up = f; else groups[base].down = f;
  }
  return Object.values(groups).sort((a,b)=> a.base.localeCompare(b.base));
}

async function getApplied(client) {
  const res = await client.query(`SELECT version, checksum, direction FROM ${MIGRATIONS_TABLE}`);
  const map = new Map();
  for (const r of res.rows) map.set(r.version, { checksum: r.checksum, direction: r.direction });
  return map;
}

async function applySql(client, sql) {
  await client.query('BEGIN');
  try {
    // enforce safety timeouts per migration
    await client.query(`SET LOCAL statement_timeout = ${parseInt(process.env.DATABASE_STATEMENT_TIMEOUT_MS || '30000', 10)}`);
    await client.query(sql);
    await client.query('COMMIT');
  } catch (e) {
    await client.query('ROLLBACK');
    throw e;
  }
}

async function run() {
  const args = process.argv.slice(2);
  const toVersion = args.includes('--to') ? args[args.indexOf('--to')+1] : null;
  const onlyVersion = args.includes('--only') ? args[args.indexOf('--only') + 1] : null;
  const down = args.includes('--down');
  const allowDown = process.env.ALLOW_MIGRATION_DOWN === '1' || process.env.NODE_ENV === 'development' || process.env.NODE_ENV === 'test';
  if (down && !allowDown) {
    console.error('Down migrations are disabled. Set ALLOW_MIGRATION_DOWN=1 to enable in this environment.');
    process.exit(2);
  }

  if (onlyVersion && toVersion) {
    console.error('Use either --only <version> or --to <version>, not both.');
    process.exit(2);
  }

  const client = await pool.connect();
  try {
    await ensureTables(client);

    // Bridge: import legacy applied versions if present and no v2 records exist
    const legacyExists = fs.existsSync(LEGACY_DIR);
    const v2Count = (await client.query(`SELECT count(*) FROM ${MIGRATIONS_TABLE}`)).rows[0].count | 0;
    if (v2Count == 0 && legacyExists) {
      // detect old table
      try {
        const old = await client.query(`SELECT version FROM schema_migrations`);
        for (const row of old.rows) {
          await client.query(`INSERT INTO ${MIGRATIONS_TABLE}(version, checksum, direction) VALUES ($1,$2,'up') ON CONFLICT DO NOTHING`, [row.version, sha256(row.version)]);
        }
        console.log(`Bridged ${old.rowCount} legacy migrations into ${MIGRATIONS_TABLE}`);
      } catch { /* table may not exist */ }
    }

    const groups = await listPairedMigrations(PG_DIR);
    const applied = await getApplied(client);

    if (onlyVersion) {
      const group = groups.find(g => g.base === onlyVersion);
      if (!group) {
        console.error(`Migration not found: ${onlyVersion}`);
        process.exit(2);
      }

      if (down) {
        if (!group.down) {
          console.log(`No down migration file for ${onlyVersion}`);
          return;
        }
        if (!applied.has(onlyVersion)) {
          console.log(`Migration not applied (nothing to revert): ${onlyVersion}`);
          return;
        }
        const sql = await fsp.readFile(path.join(PG_DIR, group.down), 'utf8');
        const checksum = sha256(sql);
        console.log(`Reverting ${onlyVersion}...`);
        await applySql(client, sql);
        await client.query(`DELETE FROM ${MIGRATIONS_TABLE} WHERE version = $1`, [onlyVersion]);
        await client.query(`INSERT INTO ${MIGRATIONS_TABLE}(version, checksum, direction) VALUES ($1,$2,'down')`, [onlyVersion, checksum]);
        console.log(`Reverted ${onlyVersion}`);
        return;
      }

      if (!group.up) {
        console.log(`No up migration file for ${onlyVersion}`);
        return;
      }
      if (applied.has(onlyVersion)) {
        console.log(`Already applied: ${onlyVersion}`);
        return;
      }
      const sql = await fsp.readFile(path.join(PG_DIR, group.up), 'utf8');
      const checksum = sha256(sql);
      console.log(`Applying ${onlyVersion}...`);
      await applySql(client, sql);
      await client.query(`INSERT INTO ${MIGRATIONS_TABLE}(version, checksum, direction) VALUES ($1,$2,'up')`, [onlyVersion, checksum]);
      console.log(`Applied ${onlyVersion}`);
      return;
    }

    if (down) {
      // revert last applied up migration that has a down file
      const appliedVersions = groups.map(g=>g.base).filter(v=> applied.has(v)).sort();
      const last = appliedVersions[appliedVersions.length-1];
      if (!last) { console.log('No migrations to revert.'); return; }
      const group = groups.find(g=>g.base===last);
      if (!group || !group.down) { console.log(`No down migration file for ${last}`); return; }
      const sql = await fsp.readFile(path.join(PG_DIR, group.down), 'utf8');
      const checksum = sha256(sql);
      console.log(`Reverting ${last}...`);
      await applySql(client, sql);
      await client.query(`DELETE FROM ${MIGRATIONS_TABLE} WHERE version = $1`, [last]);
      await client.query(`INSERT INTO ${MIGRATIONS_TABLE}(version, checksum, direction) VALUES ($1,$2,'down')`, [last, checksum]);
      console.log(`Reverted ${last}`);
      return;
    }

    for (const g of groups) {
      if (!g.up) continue;
      if (toVersion && g.base > toVersion) break;
      if (applied.has(g.base)) { continue; }
      const sql = await fsp.readFile(path.join(PG_DIR, g.up), 'utf8');
      const checksum = sha256(sql);
      console.log(`Applying ${g.base}...`);
      await applySql(client, sql);
      await client.query(`INSERT INTO ${MIGRATIONS_TABLE}(version, checksum, direction) VALUES ($1,$2,'up')`, [g.base, checksum]);
      console.log(`Applied ${g.base}`);
    }
    console.log('All migrations applied.');
  } finally {
    client.release();
    await pool.end();
  }
}

run().catch((e)=>{ console.error(e); process.exit(1); });
