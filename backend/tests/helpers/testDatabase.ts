import { Pool } from 'pg';
import pool from '../../src/utils/postgres';

/**
 * Setup and teardown for integration tests
 * Uses the global database pool initialized in setup.ts
 */

export const setupTestDatabase = async (): Promise<Pool> => {
  // Return the global pool (already initialized in setup.ts)
  // Test connection
  await pool.query('SELECT 1');
  
  return pool;
};

export const teardownTestDatabase = async (): Promise<void> => {
  // Don't close the global pool here - Jest will handle cleanup
  // If we close it, other tests might fail
};

export const cleanDatabase = async (testPool: Pool): Promise<void> => {
  // Use TRUNCATE CASCADE to clean all tables efficiently
  // Publishers CASCADE will clean: users, apps, placements, adapter_configs, payout_settings, payouts, fraud_alerts, revenue_events, refresh_tokens
  // Adapters needs separate TRUNCATE as it's independent
  await testPool.query('TRUNCATE TABLE publishers RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE publisher_bank_accounts RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE adapters RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE analytics_revenue_events RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE analytics_impressions RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE analytics_sdk_telemetry RESTART IDENTITY CASCADE');
  
  // Clean A/B testing tables
  await testPool.query('TRUNCATE TABLE ab_events RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE ab_variants RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE ab_experiments RESTART IDENTITY CASCADE');
  
  // Clean data export tables
  await testPool.query('TRUNCATE TABLE export_jobs RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE warehouse_syncs RESTART IDENTITY CASCADE');

  // Clean migration studio tables
  await testPool.query('TRUNCATE TABLE migration_guardrail_snapshots RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE migration_events RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE migration_mappings RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE migration_imports RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE migration_audit RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE migration_shadow_outcomes RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE migration_feature_flags RESTART IDENTITY CASCADE');
  await testPool.query('TRUNCATE TABLE migration_experiments RESTART IDENTITY CASCADE');
};

export const beginTransaction = async (pool: Pool): Promise<void> => {
  await pool.query('BEGIN');
};

export const rollbackTransaction = async (pool: Pool): Promise<void> => {
  await pool.query('ROLLBACK');
};

export const commitTransaction = async (pool: Pool): Promise<void> => {
  await pool.query('COMMIT');
};
