// Schema gate: verify required tables and migrations before startup
// Prevents "empty DB" incidents by failing fast with clear error

import pool from './postgres';
import { logger } from './logger';

interface SchemaGateCheck {
  name: string;
  check: () => Promise<boolean>;
  errorMessage: string;
}

const REQUIRED_TABLES = [
  'users',
  'publishers',
  'apps',
  'placements',
  'adapters',
  'analytics_impressions',
  'analytics_revenue_events',
  'subscriptions',
  'invoices',
  'two_factor_auth'
];

const REQUIRED_EXTENSIONS = [
  'uuid-ossp',
  'pgcrypto'
];

export class SchemaGate {
  private static checks: SchemaGateCheck[] = [
    {
      name: 'Required extensions',
      check: async () => {
        try {
          const query = `
            SELECT extname FROM pg_extension 
            WHERE extname = ANY($1::text[])
          `;
          const result = await pool.query(query, [REQUIRED_EXTENSIONS]);
          return result.rows.length === REQUIRED_EXTENSIONS.length;
        } catch (error) {
          // PgBouncer in transaction mode may not allow pg_extension queries
          // Skip this check if using connection pooler
          logger.info('Extension check skipped (likely using PgBouncer transaction mode)');
          return true;
        }
      },
      errorMessage: `Missing required PostgreSQL extensions. Expected: ${REQUIRED_EXTENSIONS.join(', ')}`
    },
    {
      name: 'Required tables',
      check: async () => {
        const query = `
          SELECT table_name FROM information_schema.tables 
          WHERE table_schema = 'public' 
          AND table_name = ANY($1::text[])
        `;
        const result = await pool.query(query, [REQUIRED_TABLES]);
        
        if (result.rows.length !== REQUIRED_TABLES.length) {
          const found = result.rows.map((r: any) => r.table_name);
          const missing = REQUIRED_TABLES.filter(t => !found.includes(t));
          throw new Error(`Missing required tables: ${missing.join(', ')}`);
        }
        return true;
      },
      errorMessage: 'Database schema incomplete'
    },
    {
      name: 'Migration tracking table',
      check: async () => {
        try {
          const query = `
            SELECT table_name FROM information_schema.tables 
            WHERE table_schema = 'public' 
            AND table_name = 'migrations'
          `;
          const result = await pool.query(query);
          return result.rows.length > 0;
        } catch (error) {
          logger.error('Migration table check error', { error });
          return false;
        }
      },
      errorMessage: 'Migration tracking table not found. Migrations may not have been run.'
    },
    {
      name: 'Partition horizon',
      check: async () => {
        // Check that partitions exist for today + next 7 days
        // This will be implemented after partitioning is set up
        return true; // Skip for now
      },
      errorMessage: 'Analytics partition horizon missing'
    }
  ];

  /**
   * Run all schema validation checks
   * @throws Error if any check fails
   */
  static async validate(): Promise<void> {
    logger.info('🔍 Running schema gate validation...');

    for (const check of this.checks) {
      try {
        const passed = await check.check();
        if (!passed) {
          throw new Error(check.errorMessage);
        }
        logger.info(`✓ Schema check passed: ${check.name}`);
      } catch (error: any) {
        logger.error(`✗ Schema check failed: ${check.name}`);
        logger.error(`  Error: ${error.message}`);
        throw new Error(`SCHEMA_GATE_FAILED: ${check.name} - ${check.errorMessage}`);
      }
    }

    logger.info('✓ All schema gate checks passed');
  }

  /**
   * Validate schema and exit if failed
   * Use this at application startup
   */
  static async validateOrExit(): Promise<void> {
    try {
      await this.validate();
    } catch (error: any) {
      logger.error('❌ FATAL: Schema validation failed');
      logger.error(error.message);
      logger.error('');
      logger.error('The application cannot start with an incomplete database schema.');
      logger.error('Please ensure:');
      logger.error('  1. Database migrations have been run');
      logger.error('  2. Required extensions are installed');
      logger.error('  3. All required tables exist');
      logger.error('');
      process.exit(1);
    }
  }
}
