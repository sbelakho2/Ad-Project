import dotenv from 'dotenv';
import fs from 'node:fs';
import path from 'node:path';
import { Pool, PoolClient, QueryResult, QueryResultRow } from 'pg';
import Cursor from 'pg-cursor';
import logger from './logger';
import { dbQueryDurationSeconds } from './prometheus';

// Prefer an explicitly-provided ENV_FILE, then repo-root .env (one level up from backend/)
// so scripts executed from /opt/adproject/backend pick up /opt/adproject/.env.
// Only load dotenv if DATABASE_URL isn't already set (e.g. PM2-provided env).
if (!process.env.DATABASE_URL) {
  const cwdBase = path.basename(process.cwd());
  const cwdCandidates =
    cwdBase === 'backend'
      ? [
          // Prefer repo-root env when running inside backend/
          path.resolve(process.cwd(), '../.env'),
          path.resolve(process.cwd(), '.env'),
        ]
      : [
          path.resolve(process.cwd(), '.env'),
          path.resolve(process.cwd(), '../.env'),
        ];

  const candidates = [
    process.env.ENV_FILE,
    ...cwdCandidates,
    // Running from transpiled dist/ paths (backend/dist/src/utils)
    path.resolve(__dirname, '../../../../.env'),
    // Running from source paths (backend/src/utils)
    path.resolve(__dirname, '../../..', '.env'),
  ].filter((p): p is string => !!p);

  const envPath = candidates.find((p) => {
    try {
      return fs.existsSync(p);
    } catch {
      return false;
    }
  });

  if (envPath) {
    dotenv.config({ path: envPath });
  } else {
    dotenv.config();
  }
}

const defaultTestConnection = 'postgresql://postgres:postgres@localhost:5432/apexmediation_test';
const connectionString = process.env.DATABASE_URL ?? (process.env.NODE_ENV === 'test' ? defaultTestConnection : undefined);

if (!connectionString) {
  throw new Error('DATABASE_URL environment variable is not configured.');
}

if (!process.env.DATABASE_URL) {
  process.env.DATABASE_URL = connectionString;
}

if (!process.env.TEST_DATABASE_URL && process.env.NODE_ENV === 'test') {
  process.env.TEST_DATABASE_URL = connectionString;
}

const shouldUseSsl = (): boolean => {
  if (process.env.DATABASE_SSL === 'true') return true;
  // Backwards-compatible: some environments use libpq-style sslmode in the URL.
  if (connectionString && /[?&]sslmode=require\b/i.test(connectionString)) return true;

  // Sensible default: connections to non-local hosts should use TLS.
  // This is important for one-off scripts (NODE_ENV may be unset) and for prod.
  if (connectionString) {
    try {
      const u = new URL(connectionString);
      const host = u.hostname;
      const isLocalHost = host === 'localhost' || host === '127.0.0.1' || host === '::1';
      return !isLocalHost;
    } catch {
      return false;
    }
  }
  return false;
};

const useSsl = shouldUseSsl();
const poolMax = parseInt(process.env.DATABASE_POOL_MAX || '20', 10); // Increased from 10 to 20
const poolMin = parseInt(process.env.DATABASE_POOL_MIN || '2', 10); // Minimum connections
const idleTimeout = parseInt(process.env.DATABASE_POOL_IDLE_MS || '30000', 10);
const connectionTimeout = parseInt(process.env.DATABASE_CONNECTION_TIMEOUT_MS || '5000', 10);
const statementTimeout = parseInt(process.env.DATABASE_STATEMENT_TIMEOUT_MS || '30000', 10); // 30 second query timeout

const pool = new Pool({
  connectionString,
  ssl: useSsl ? { rejectUnauthorized: false } : undefined,
  max: poolMax, // Maximum pool size (default: 20)
  min: poolMin, // Minimum pool size (default: 2)
  idleTimeoutMillis: idleTimeout, // How long idle clients stay in pool (default: 30s)
  connectionTimeoutMillis: connectionTimeout, // How long to wait for connection (default: 5s)
  // Do not set statement_timeout here when using PgBouncer
  // statement_timeout: statementTimeout, // Query timeout in ms (default: 30s)
});

pool.on('error', (error: Error) => {
  logger.error('Unexpected database error', {
    error: error.message,
    stack: error.stack,
  });
});

pool.on('connect', () => {
  logger.debug('New database connection established');
});

pool.on('acquire', () => {
  logger.debug('Connection acquired from pool');
});

pool.on('remove', () => {
  logger.debug('Connection removed from pool');
});

const buildReplicaPool = (): Pool | null => {
  const replicaUrl = process.env.REPLICA_DATABASE_URL;
  if (!replicaUrl) return null;
  return new Pool({
    connectionString: replicaUrl,
    ssl: useSsl ? { rejectUnauthorized: false } : undefined,
    max: parseInt(process.env.REPLICA_DATABASE_POOL_MAX || '15', 10),
    min: parseInt(process.env.REPLICA_DATABASE_POOL_MIN || '1', 10),
    idleTimeoutMillis: idleTimeout,
    connectionTimeoutMillis: connectionTimeout,
    // Do not set statement_timeout when using PgBouncer
    // statement_timeout: statementTimeout,
  });
};

const replicaPool = buildReplicaPool();

type QueryOptions = {
  replica?: boolean;
  label?: string;
};

type StreamQueryOptions = QueryOptions & {
  batchSize?: number;
};

export const query = <T extends QueryResultRow = QueryResultRow>(
  text: string,
  params?: ReadonlyArray<unknown>,
  options: QueryOptions = {}
): Promise<QueryResult<T>> => {
  // Add query timeout wrapper
  const timeoutMs = parseInt(process.env.DATABASE_STATEMENT_TIMEOUT_MS || '30000', 10);
  const op = options.label || (text.split(/\s+/)[0] || 'QUERY').toUpperCase();
  const endTimer = dbQueryDurationSeconds.startTimer({ operation: op, replica: options.replica ? 'true' : 'false' });
  
  const targetPool = options.replica && replicaPool ? replicaPool : pool;

  const queryPromise = params && params.length > 0
    ? targetPool.query<T>(text, [...params])
    : targetPool.query<T>(text);

  // Wrap with timeout (and ensure the timeout handle is cleaned up)
  let timeoutHandle: NodeJS.Timeout | null = null;
  const timeoutPromise = new Promise<QueryResult<T>>((_, reject) => {
    timeoutHandle = setTimeout(() => reject(new Error(`Query timeout after ${timeoutMs}ms`)), timeoutMs);
    timeoutHandle.unref?.();
  });

  return Promise.race([queryPromise, timeoutPromise]).finally(() => {
    if (timeoutHandle) {
      clearTimeout(timeoutHandle);
      timeoutHandle = null;
    }
    try {
      endTimer();
    } catch {
      /* noop */
    }
  });
};

const defaultStreamBatchSize = parseInt(process.env.DB_STREAM_BATCH_SIZE || '5000', 10);

export const streamQuery = async function* <T extends QueryResultRow = QueryResultRow>(
  text: string,
  params?: ReadonlyArray<unknown>,
  options: StreamQueryOptions = {}
): AsyncGenerator<T> {
  const batchSize = options.batchSize && options.batchSize > 0 ? options.batchSize : defaultStreamBatchSize;
  const op = options.label || (text.split(/\s+/)[0] || 'QUERY').toUpperCase();
  const endTimer = dbQueryDurationSeconds.startTimer({ operation: op, replica: options.replica ? 'true' : 'false' });

  const targetPool = options.replica && replicaPool ? replicaPool : pool;
  const client = await targetPool.connect();
  let cursor: Cursor<T> | null = null;

  try {
    cursor = client.query(new Cursor(text, params ? [...params] : []));
    while (true) {
      const rows: T[] = await new Promise((resolve, reject) => {
        cursor!.read(batchSize, (error?: Error | null, resultRows?: T[] | null) => {
          if (error) {
            reject(error);
            return;
          }
          resolve(resultRows ?? []);
        });
      });

      if (!rows.length) {
        break;
      }

      for (const row of rows) {
        yield row;
      }
    }
  } finally {
    await new Promise<void>((resolve) => {
      if (!cursor) {
        resolve();
        return;
      }
      cursor.close(() => resolve());
    });
    client.release();
    try {
      endTimer();
    } catch {
      /* noop */
    }
  }
};

export const getClient = (): Promise<PoolClient> => pool.connect();

type InsertManyOptions = {
  /** Column names (or a named constraint) to apply in ON CONFLICT clause */
  onConflictColumns?: ReadonlyArray<string>;
  onConflictConstraint?: string;
  ignoreConflicts?: boolean; // shorthand for DO NOTHING when columns/constraint provided
};

const quoteIdentifier = (identifier: string): string => `"${identifier.replace(/"/g, '""')}"`;

const quoteTableName = (table: string): string =>
  table
    .split('.')
    .map((segment) => quoteIdentifier(segment))
    .join('.');

export const insertMany = async (
  table: string,
  columns: ReadonlyArray<string>,
  rows: ReadonlyArray<ReadonlyArray<unknown>>,
  options?: InsertManyOptions
): Promise<void> => {
  if (!rows.length) return;

  const quotedTable = quoteTableName(table);
  const columnSql = columns.map((c) => quoteIdentifier(c)).join(', ');
  const valuesSql: string[] = [];
  const params: unknown[] = [];

  rows.forEach((row, rowIdx) => {
    if (row.length !== columns.length) {
      throw new Error(`Row ${rowIdx} length (${row.length}) does not match columns length (${columns.length}) for ${table}`);
    }
    const placeholderOffset = rowIdx * columns.length;
    const placeholders = row.map((_, colIdx) => `$${placeholderOffset + colIdx + 1}`);
    valuesSql.push(`(${placeholders.join(', ')})`);
    params.push(...row);
  });

  let conflictSql = '';
  if (options?.ignoreConflicts) {
    if (options.onConflictConstraint) {
      conflictSql = ` ON CONFLICT ON CONSTRAINT ${quoteIdentifier(options.onConflictConstraint)} DO NOTHING`;
    } else if (options.onConflictColumns?.length) {
      const cols = options.onConflictColumns.map((c) => quoteIdentifier(c)).join(', ');
      conflictSql = ` ON CONFLICT (${cols}) DO NOTHING`;
    }
  }

  const sql = `INSERT INTO ${quotedTable} (${columnSql}) VALUES ${valuesSql.join(', ')}${conflictSql}`;
  await query(sql, params);
};

export const initializeDatabase = async (): Promise<void> => {
  const client = await pool.connect();

  try {
    await client.query('SELECT 1');
    logger.info('Successfully connected to PostgreSQL');
  } finally {
    client.release();
  }
};

export default pool;
