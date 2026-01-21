#!/usr/bin/env node
// Create a new app API key for a publisher's app and print the plaintext secret.
// Usage:
//   DATABASE_URL=postgres://... node backend/scripts/create_app_api_key.js --email user@example.com
//   DATABASE_URL=postgres://... node backend/scripts/create_app_api_key.js --publisher-id <uuid> --app-id <uuid>
// Optional:
//   --platform android|ios|web|unity|ctv (pick first app matching platform)
//   --environment production|staging (default: production)
//   --live true|false (default: true)
//
// NOTE: Prints the plaintext secret ONCE. Store securely.

require('dotenv/config');
const { randomUUID, createHash, randomBytes } = require('node:crypto');
const bcrypt = require('bcryptjs');
const { Pool } = require('pg');

function arg(name, def) {
  const idx = process.argv.indexOf(`--${name}`);
  if (idx !== -1 && process.argv[idx + 1]) return process.argv[idx + 1];
  return def;
}

const email = arg('email', undefined);
const publisherIdArg = arg('publisher-id', undefined);
const appIdArg = arg('app-id', undefined);
const platform = arg('platform', undefined);
const environment = arg('environment', 'production');
const live = (arg('live', 'true') || '').toLowerCase() !== 'false';

if (!process.env.DATABASE_URL) {
  console.error('[create_app_api_key] DATABASE_URL is required');
  process.exit(2);
}
if (!email && !publisherIdArg) {
  console.error('[create_app_api_key] Provide --email or --publisher-id');
  process.exit(2);
}
if (environment !== 'production' && environment !== 'staging') {
  console.error('[create_app_api_key] environment must be production or staging');
  process.exit(2);
}

const pool = new Pool({ connectionString: process.env.DATABASE_URL });

function generateSecret() {
  return `${live ? 'ak_live' : 'ak_test'}_${randomBytes(24).toString('hex')}`;
}

async function main() {
  const client = await pool.connect();
  try {
    await client.query('BEGIN');

    let publisherId = publisherIdArg;
    if (!publisherId) {
      const user = await client.query('SELECT publisher_id FROM users WHERE email = $1 LIMIT 1', [email]);
      publisherId = user.rows[0]?.publisher_id;
      if (!publisherId) {
        throw new Error(`No publisher found for email ${email}`);
      }
    }

    let appId = appIdArg;
    if (!appId) {
      if (platform) {
        const app = await client.query(
          'SELECT id FROM apps WHERE publisher_id = $1 AND platform = $2 ORDER BY created_at ASC LIMIT 1',
          [publisherId, platform]
        );
        appId = app.rows[0]?.id;
      } else {
        const app = await client.query(
          'SELECT id FROM apps WHERE publisher_id = $1 ORDER BY created_at ASC LIMIT 1',
          [publisherId]
        );
        appId = app.rows[0]?.id;
      }
      if (!appId) {
        throw new Error('No app found for publisher (create an app first)');
      }
    }

    const secret = generateSecret();
    const secretDigest = createHash('sha256').update(secret).digest('hex');
    const secretHash = await bcrypt.hash(secret, 10);
    const last4 = secret.slice(-4);
    const prefix = live ? 'ak_live' : 'ak_test';

    await client.query(
      `INSERT INTO app_api_keys (id, app_id, publisher_id, environment, secret, secret_digest, prefix, last4, created_at, updated_at)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW(), NOW())`,
      [randomUUID(), appId, publisherId, environment, secretHash, secretDigest, prefix, last4]
    );

    await client.query('COMMIT');

    console.log('APP_API_KEY_CREATED');
    console.log(`PUBLISHER_ID=${publisherId}`);
    console.log(`APP_ID=${appId}`);
    console.log(`ENVIRONMENT=${environment}`);
    console.log(`API_KEY=${secret}`);
  } catch (err) {
    await client.query('ROLLBACK');
    console.error('[create_app_api_key] Error:', err.message || err);
    process.exit(1);
  } finally {
    client.release();
    await pool.end();
  }
}

main();
