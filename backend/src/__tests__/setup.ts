import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { Client } from 'pg';
import { jest } from '@jest/globals';

// Default: skip real DB setup to keep CI lightweight. Set FORCE_DB_SETUP=true to enable.
const skipDbSetup = process.env.FORCE_DB_SETUP !== 'true';

let dbPool: import('pg').Pool | null = null;

const ensureDatabaseExists = async (connectionString: string): Promise<void> => {
  const dbUrl = new URL(connectionString);
  const databaseName = dbUrl.pathname.replace('/', '');

  if (!databaseName) {
    throw new Error('Unable to determine database name from connection string.');
  }

  if (!/^[A-Za-z0-9_]+$/.test(databaseName)) {
    throw new Error(`Refusing to create database with unsafe name: ${databaseName}`);
  }

  const adminUrl = new URL(connectionString);
  adminUrl.pathname = '/postgres';

  const client = new Client({ connectionString: adminUrl.toString() });

  try {
    await client.connect();
    const result = await client.query('SELECT 1 FROM pg_database WHERE datname = $1', [databaseName]);
    if (result.rowCount === 0) {
      await client.query(`CREATE DATABASE "${databaseName}"`);
    }
  } finally {
    await client.end();
  }
};

// Ensure critical env vars are present before modules import configuration.
process.env.NODE_ENV = 'test';
process.env.JWT_SECRET = process.env.JWT_SECRET || 'test-access-secret-change-me-please-123456';
process.env.COOKIE_SECRET = process.env.COOKIE_SECRET || 'test-cookie-secret-needs-length-123456';
process.env.DATABASE_URL =
  process.env.DATABASE_URL ||
  process.env.TEST_DATABASE_URL ||
  'postgresql://postgres:postgres@localhost:5432/apexmediation_test';
process.env.JWT_REFRESH_SECRET = process.env.JWT_REFRESH_SECRET || 'test-refresh-secret-change-me-123456';
process.env.JWT_EXPIRES_IN = process.env.JWT_EXPIRES_IN || '1h';
process.env.REFRESH_TOKEN_EXPIRES_IN = process.env.REFRESH_TOKEN_EXPIRES_IN || '7d';

// Initialize database connection before all tests
beforeAll(async () => {
  if (skipDbSetup) {
    return;
  }

  // Ensure both DATABASE_URL and TEST_DATABASE_URL are present before importing the pool
  if (!process.env.DATABASE_URL) {
    throw new Error('DATABASE_URL is required for DB-backed tests');
  }
  const resolvedDbUrl = process.env.DATABASE_URL;
  await ensureDatabaseExists(resolvedDbUrl);
  process.env.TEST_DATABASE_URL = resolvedDbUrl;

  // Some test suites mock ../utils/postgres; DB-backed setup must always use the real module.
  const postgres = jest.requireActual('../utils/postgres') as typeof import('../utils/postgres');

  // Initialize the database connection pool
  await postgres.initializeDatabase();
  dbPool = postgres.default;
  const backendRoot = path.resolve(__dirname, '..', '..');
  // Only run migrations when explicitly enabled; default is off for CI speed and portability.
  const runMigrations = process.env.RUN_MIGRATIONS_IN_TEST === 'true';
  if (runMigrations) {
    execFileSync(process.execPath, ['scripts/runMigrationsV2.js'], {
      stdio: 'inherit',
      cwd: backendRoot,
      env: process.env,
    });
  }
}, 30000); // 30 second timeout for database initialization

// Close database connection after all tests complete
afterAll(async () => {
  if (skipDbSetup || !dbPool) {
    return;
  }
  await dbPool.end();
}, 10000);


