#!/usr/bin/env node
require('dotenv/config');
const path = require('node:path');
const fs = require('node:fs/promises');
const { Pool } = require('pg');
const { buildSslConfig } = require('./dbSsl');

const connectionString = process.env.DATABASE_URL;

if (!connectionString) {
  console.error('DATABASE_URL environment variable is not configured.');
  process.exit(1);
}

const useSsl = process.env.DATABASE_SSL === 'true';
const poolMax = parseInt(process.env.DATABASE_POOL_MAX || '10', 10);
const idleTimeout = parseInt(process.env.DATABASE_POOL_IDLE_MS || '30000', 10);

const pool = new Pool({
  connectionString,
  ssl: buildSslConfig(),
  max: poolMax,
  idleTimeoutMillis: idleTimeout,
});

const MIGRATIONS_TABLE = 'schema_migrations';

async function ensureMigrationsTable(client) {
  await client.query(
    `CREATE TABLE IF NOT EXISTS ${MIGRATIONS_TABLE} (
      id SERIAL PRIMARY KEY,
      version TEXT NOT NULL UNIQUE,
      applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )`
  );
}

async function getAppliedVersions(client) {
  const result = await client.query(`SELECT version FROM ${MIGRATIONS_TABLE}`);
  return new Set(result.rows.map((row) => row.version));
}

async function applyMigration(client, filePath, version, sql) {
  console.log(`Applying migration ${version}...`);
  await client.query('BEGIN');

  try {
    await client.query(sql);
    await client.query(
      `INSERT INTO ${MIGRATIONS_TABLE} (version) VALUES ($1) ON CONFLICT (version) DO NOTHING`,
      [version]
    );
    await client.query('COMMIT');
    console.log(`Migration ${version} applied`);
  } catch (error) {
    await client.query('ROLLBACK');
    console.error(`Failed to apply migration ${version} (${filePath})`);
    throw error;
  }
}

async function connectWithRetry(maxAttempts = 10, delayMs = 2000) {
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      const client = await pool.connect();
      console.log('Database connection established');
      return client;
    } catch (error) {
      if (attempt === maxAttempts) {
        throw error;
      }
      console.log(`Connection attempt ${attempt}/${maxAttempts} failed, retrying in ${delayMs}ms...`);
      await new Promise(resolve => setTimeout(resolve, delayMs));
    }
  }
}

async function run() {
  const client = await connectWithRetry();

  try {
    await ensureMigrationsTable(client);

    const migrationsDir = path.resolve(__dirname, '../migrations');
    const files = (await fs.readdir(migrationsDir))
      .filter((file) => file.endsWith('.sql'))
      .sort();

    const appliedVersions = await getAppliedVersions(client);

    for (const file of files) {
      const version = file;

      if (appliedVersions.has(version)) {
        console.log(`Skipping already applied migration ${version}`);
        continue;
      }

      const filePath = path.join(migrationsDir, file);
      const sql = await fs.readFile(filePath, 'utf8');

      if (!sql.trim()) {
        console.log(`Skipping empty migration file ${file}`);
        continue;
      }

      await applyMigration(client, filePath, version, sql);
    }

    console.log('All migrations applied.');
  } finally {
    client.release();
    await pool.end();
  }
}

run().catch((error) => {
  console.error(error);
  process.exit(1);
});
