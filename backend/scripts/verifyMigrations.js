#!/usr/bin/env node
require('dotenv/config');
const { execFileSync } = require('node:child_process');
const { Client } = require('pg');
const { buildSslConfig } = require('./dbSsl');

function log(msg){ process.stdout.write(`${msg}\n`); }

async function pgCheck() {
  const cs = process.env.DATABASE_URL;
  if (!cs) throw new Error('DATABASE_URL not set');
  const client = new Client({ connectionString: cs, ssl: buildSslConfig() });
  await client.connect();
  try {
    const tables = ['auctions','bids','auction_wins','signing_keys'];
    for (const t of tables) {
      const r = await client.query(`SELECT to_regclass($1) as exists`, [t]);
      if (!r.rows[0].exists) throw new Error(`Missing table: ${t}`);
    }
  } finally {
    await client.end();
  }
}

async function main() {
  log('Applying Postgres migrations (up)');
  execFileSync('node', ['scripts/runMigrationsV2.js'], { stdio: 'inherit', cwd: __dirname + '/..' });

  log('Running smoke checks');
  await pgCheck();

  log('Reverting last PG migration (down)');
  process.env.ALLOW_MIGRATION_DOWN = process.env.ALLOW_MIGRATION_DOWN || '1';
  execFileSync('node', ['scripts/runMigrationsV2.js','--down'], { stdio: 'inherit', cwd: __dirname + '/..' });

  log('Re-applying PG migrations');
  execFileSync('node', ['scripts/runMigrationsV2.js'], { stdio: 'inherit', cwd: __dirname + '/..' });

  log('Verification complete');
}

main().catch((e)=>{ console.error(e); process.exit(1); });
