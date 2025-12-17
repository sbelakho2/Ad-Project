#!/usr/bin/env node
// Create a dummy publisher, user, 5 apps, and per-app SDK keys.
// Usage:
//   DATABASE_URL=postgres://... node backend/scripts/create_dummy_publisher.js \
//     --email sales@apexmediation.ee \
//     --password '&&Pw20354491' \
//     --company "Apex Dummy Publisher"

require('dotenv/config');
const { randomUUID, createHash, randomBytes } = require('node:crypto');
const bcrypt = require('bcryptjs');
const { Pool } = require('pg');

function arg(name, def) {
  const idx = process.argv.indexOf(`--${name}`);
  if (idx !== -1 && process.argv[idx + 1]) return process.argv[idx + 1];
  return def;
}

const email = arg('email', 'sales@apexmediation.ee');
const password = arg('password', '&&Pw20354491');
const company = arg('company', 'Apex Dummy Publisher');

const connectionString = process.env.DATABASE_URL;
if (!connectionString) {
  console.error('[create_dummy_publisher] DATABASE_URL is required');
  process.exit(2);
}

const pool = new Pool({ connectionString });

async function upsertPublisher(client, id, companyName) {
  await client.query(
    'INSERT INTO publishers (id, company_name, created_at) VALUES ($1, $2, NOW()) ON CONFLICT (id) DO UPDATE SET company_name = EXCLUDED.company_name',
    [id, companyName]
  );
}

async function upsertUser(client, id, publisherId, email, password) {
  const hash = await bcrypt.hash(password, 10);
  await client.query(
    `INSERT INTO users (id, publisher_id, email, password_hash, created_at)
     VALUES ($1, $2, $3, $4, NOW())
     ON CONFLICT (email) DO UPDATE SET password_hash = EXCLUDED.password_hash, publisher_id = EXCLUDED.publisher_id`,
    [id, publisherId, email, hash]
  );
}

async function upsertApp(client, id, publisherId, name, platform, bundleId) {
  await client.query(
    `INSERT INTO apps (id, publisher_id, name, platform, bundle_id, created_at)
     VALUES ($1, $2, $3, $4, $5, NOW())
     ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, platform = EXCLUDED.platform, bundle_id = EXCLUDED.bundle_id`,
    [id, publisherId, name, platform, bundleId]
  );
}

function generateKeySecret() {
  return 'ak_test_' + randomBytes(32).toString('hex');
}

async function insertAppApiKey(client, id, appId, publisherId, secret, env = 'sandbox') {
  const digest = createHash('sha256').update(secret).digest('hex');
  const last4 = secret.slice(-4);
  await client.query(
    `INSERT INTO app_api_keys (id, app_id, publisher_id, environment, secret, secret_digest, prefix, last4, created_at, updated_at)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW(), NOW())
     ON CONFLICT (id) DO NOTHING`,
    [id, appId, publisherId, env, secret, digest, 'ak_test_', last4]
  );
}

async function main() {
  const client = await pool.connect();
  try {
    await client.query('BEGIN');

    // If user exists, reuse publisher_id; else create fresh IDs
    const existing = await client.query('SELECT id, publisher_id FROM users WHERE email = $1 LIMIT 1', [email]);

    const publisherId = existing.rows[0]?.publisher_id || randomUUID();
    const userId = existing.rows[0]?.id || randomUUID();

    await upsertPublisher(client, publisherId, company);
    await upsertUser(client, userId, publisherId, email, password);

    // Create 5 apps (idempotent)
    const apps = [
      { key: 'ANDROID', platform: 'android', bundle: 'com.apex.sandbox.android' },
      { key: 'IOS', platform: 'ios', bundle: 'com.apex.sandbox.ios' },
      { key: 'WEB', platform: 'web', bundle: 'web.apex.sandbox' },
      { key: 'UNITY', platform: 'unity', bundle: 'com.apex.sandbox.unity' },
      { key: 'CTV', platform: 'ctv', bundle: 'tv.apex.sandbox' },
    ];

    const appIds = {};
    const appSecrets = {};
    for (const app of apps) {
      // Try to find an existing app for this platform under the publisher
      const existingApp = await client.query(
        'SELECT id FROM apps WHERE publisher_id = $1 AND platform = $2 LIMIT 1',
        [publisherId, app.platform]
      );
      const appId = existingApp.rows[0]?.id || randomUUID();
      appIds[app.key] = appId;
      await upsertApp(client, appId, publisherId, `Sandbox ${app.platform.toUpperCase()}`, app.platform, app.bundle);

      // Ensure an SDK key exists for each app
      const existingKey = await client.query('SELECT id FROM app_api_keys WHERE app_id = $1 LIMIT 1', [appId]);
      if (existingKey.rows.length === 0) {
        const secret = generateKeySecret();
        await insertAppApiKey(client, randomUUID(), appId, publisherId, secret, 'sandbox');
      }
      // Fetch latest secret for this app (prints for operator to place into .env.slo)
      const keyRow = await client.query('SELECT secret FROM app_api_keys WHERE app_id = $1 ORDER BY created_at DESC LIMIT 1', [appId]);
      appSecrets[app.key] = keyRow.rows[0]?.secret || '';
    }

    await client.query('COMMIT');

    // Print out env values
    console.log('--- Dummy Publisher Created/Ensured ---');
    console.log(`PUBLISHER_ID=${publisherId}`);
    for (const [k, v] of Object.entries(appIds)) {
      console.log(`APP_ID_${k}=${v}`);
    }
    console.log('--- SDK Keys (per app) ---');
    for (const [k, v] of Object.entries(appSecrets)) {
      console.log(`SDK_KEY_${k}=${v}`);
    }
  } catch (err) {
    await client.query('ROLLBACK');
    console.error('[create_dummy_publisher] Error:', err);
    process.exit(1);
  } finally {
    client.release();
    await pool.end();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
