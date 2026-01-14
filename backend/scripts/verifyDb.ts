import 'dotenv/config';
import fs from 'node:fs';
import { Pool } from 'pg';

const buildSslConfig = () => {
  const connectionString = process.env.DATABASE_URL || '';
  const sslEnabled = process.env.DATABASE_SSL === 'true' || /sslmode=require/i.test(connectionString);
  if (!sslEnabled) return undefined;
  const caPath = process.env.DATABASE_SSL_CA;
  const rejectUnauthorizedEnv = process.env.DATABASE_SSL_REJECT_UNAUTHORIZED;
  const rejectUnauthorized = rejectUnauthorizedEnv
    ? rejectUnauthorizedEnv === 'true'
    : Boolean(caPath);

  const sslConfig: { rejectUnauthorized: boolean; ca?: string } = { rejectUnauthorized };
  if (caPath) {
    sslConfig.ca = fs.readFileSync(caPath, 'utf8');
  }
  return sslConfig;
};

async function main() {
  const databaseUrl = process.env.DATABASE_URL;
  if (!databaseUrl) {
    console.error('[verify:db] DATABASE_URL is not set');
    process.exit(2);
  }

  const pool = new Pool({ connectionString: databaseUrl, ssl: buildSslConfig() });
  try {
    const client = await pool.connect();
    try {
      const ping = await client.query('SELECT 1 as ok');
      if (ping.rows[0]?.ok !== 1) throw new Error('Failed basic SELECT 1');

      // Optional: verify expected tables if provided
      const expected = (process.env.VERIFY_DB_EXPECT_TABLES || '')
        .split(',')
        .map(s => s.trim())
        .filter(Boolean);
      for (const tbl of expected) {
        const { rows } = await client.query(
          `SELECT 1 FROM information_schema.tables WHERE table_schema IN ('public') AND table_name = $1 LIMIT 1`,
          [tbl]
        );
        if (rows.length === 0) {
          throw new Error(`Expected table missing: ${tbl}`);
        }
      }

      console.log('[verify:db] OK');
      process.exit(0);
    } finally {
      client.release();
    }
  } catch (err) {
    console.error('[verify:db] FAILED:', err);
    process.exit(1);
  } finally {
    await pool.end();
  }
}

main().catch(err => {
  console.error('[verify:db] Unexpected error:', err);
  process.exit(1);
});
