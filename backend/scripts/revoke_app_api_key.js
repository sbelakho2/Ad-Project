#!/usr/bin/env node
// Revoke an app API key by plaintext secret or digest.
// Usage:
//   DATABASE_URL=postgres://... node backend/scripts/revoke_app_api_key.js --secret ak_live_...
//   DATABASE_URL=postgres://... node backend/scripts/revoke_app_api_key.js --digest <sha256>

require('dotenv/config');
const { createHash } = require('crypto');
const { Pool } = require('pg');

function arg(name, def) {
  const idx = process.argv.indexOf(`--${name}`);
  if (idx !== -1 && process.argv[idx + 1]) return process.argv[idx + 1];
  return def;
}

const secret = arg('secret', undefined);
const digestArg = arg('digest', undefined);

if (!process.env.DATABASE_URL) {
  console.error('[revoke_app_api_key] DATABASE_URL is required');
  process.exit(2);
}
if (!secret && !digestArg) {
  console.error('[revoke_app_api_key] Provide --secret or --digest');
  process.exit(2);
}

const digest = digestArg || createHash('sha256').update(secret).digest('hex');
const pool = new Pool({ connectionString: process.env.DATABASE_URL });

async function main() {
  const client = await pool.connect();
  try {
    const res = await client.query(
      'UPDATE app_api_keys SET revoked_at = NOW(), updated_at = NOW() WHERE secret_digest = $1 AND revoked_at IS NULL RETURNING id, app_id, publisher_id',
      [digest]
    );
    if (res.rows.length === 0) {
      console.log('REVOKE_RESULT=not_found');
      return;
    }
    console.log('REVOKE_RESULT=revoked');
    console.log(`KEY_ID=${res.rows[0].id}`);
    console.log(`APP_ID=${res.rows[0].app_id}`);
    console.log(`PUBLISHER_ID=${res.rows[0].publisher_id}`);
  } finally {
    client.release();
    await pool.end();
  }
}

main().catch((err) => {
  console.error('[revoke_app_api_key] Error:', err.message || err);
  process.exit(1);
});
