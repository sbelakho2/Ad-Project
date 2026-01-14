#!/usr/bin/env node
/*
 * Sandbox bootstrapper:
 * - Creates owner/dev/finance logins for Apex Sandbox Studio
 * - Ensures FakeNetworkA/B/C adapters, configs, and waterfall priorities exist
 * - (Optional) provisions a Stripe test customer with card/ACH/SEPA sources
 */
require('dotenv/config');
const { randomUUID } = require('node:crypto');
const bcrypt = require('bcryptjs');
const { Pool } = require('pg');
const { buildSslConfig } = require('./dbSsl');
const Stripe = require('stripe');
const {
  SANDBOX_PUBLISHER_ID,
  SANDBOX_COMPANY_NAME,
  SANDBOX_USERS,
  DEFAULT_SANDBOX_PASSWORD,
  FAKE_ADAPTERS,
} = require('./sandboxConstants');

const DATABASE_URL = process.env.DATABASE_URL;
if (!DATABASE_URL) {
  console.error('DATABASE_URL is required');
  process.exit(1);
}

const pool = new Pool({
  connectionString: DATABASE_URL,
  ssl: buildSslConfig(),
});

const STRIPE_KEY = process.env.STRIPE_SECRET_KEY;
const stripe = STRIPE_KEY ? new Stripe(STRIPE_KEY, { apiVersion: '2023-10-16' }) : null;

const DEFAULT_PASSWORD = process.env.SANDBOX_USER_PASSWORD || DEFAULT_SANDBOX_PASSWORD;
const PASSWORD_HASH = bcrypt.hashSync(DEFAULT_PASSWORD, 10);

async function main() {
  const client = await pool.connect();
  try {
    await client.query('BEGIN');

    const publisher = await client.query('SELECT id FROM publishers WHERE id = $1', [SANDBOX_PUBLISHER_ID]);
    if (publisher.rowCount === 0) {
      throw new Error(`Publisher ${SANDBOX_COMPANY_NAME} (${SANDBOX_PUBLISHER_ID}) not found.`);
    }

    const userRecords = await ensureUsers(client);
    await ensureAdapters(client);
    await ensureAdapterConfigs(client);
    await ensureWaterfalls(client, userRecords.ownerId);

    await client.query('COMMIT');
    console.log('✅ Sandbox adapters, configs, and waterfalls ensured.');

    if (stripe && userRecords.ownerId) {
      await ensureStripeCustomer(userRecords.ownerId);
    } else if (!stripe) {
      console.warn('⚠️ STRIPE_SECRET_KEY missing — skipped Stripe provisioning.');
    }
  } catch (error) {
    await client.query('ROLLBACK');
    console.error('❌ Failed to set up sandbox assets:', error.message);
    process.exitCode = 1;
  } finally {
    client.release();
    await pool.end();
  }
}

async function ensureUsers(client) {
  const records = {};
  for (const user of SANDBOX_USERS) {
    const result = await client.query(
      `INSERT INTO users (id, publisher_id, email, password_hash, role, created_at)
       VALUES ($1, $2, $3, $4, $5, NOW())
       ON CONFLICT (email)
       DO UPDATE SET publisher_id = EXCLUDED.publisher_id,
                     role = EXCLUDED.role,
                     password_hash = EXCLUDED.password_hash
       RETURNING id`,
      [user.id, SANDBOX_PUBLISHER_ID, user.email, PASSWORD_HASH, user.role]
    );
    const id = result.rows[0]?.id || (await client.query('SELECT id FROM users WHERE email = $1', [user.email])).rows[0].id;
    records[user.role === 'owner' ? 'ownerId' : user.role === 'developer' ? 'developerId' : 'financeId'] = id;
    console.log(`• ensured user ${user.email} (${user.role})`);
  }
  console.log(`   default password: ${DEFAULT_PASSWORD}`);
  return records;
}

async function ensureAdapters(client) {
  for (const adapter of FAKE_ADAPTERS) {
    await client.query(
      `INSERT INTO adapters (id, name, enabled)
       VALUES ($1, $2, true)
       ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, enabled = true`,
      [adapter.id, adapter.name]
    );
    console.log(`• adapter ${adapter.name} online`);
  }
}

async function ensureAdapterConfigs(client) {
  for (const adapter of FAKE_ADAPTERS) {
    await client.query(
      `INSERT INTO adapter_configs (id, publisher_id, adapter_id, config)
       VALUES ($1, $2, $3, $4::jsonb)
       ON CONFLICT (id) DO UPDATE SET config = EXCLUDED.config, updated_at = NOW()`,
      [
        adapter.configId,
        SANDBOX_PUBLISHER_ID,
        adapter.id,
        JSON.stringify({
          behavior: adapter.behavior,
          fillRate: adapter.behavior === 'always_fill' ? 1 : adapter.behavior === 'random_fill' ? 0.55 : 0.35,
          timeoutMs: adapter.timeoutMs,
          targetEcpm: adapter.ecpm,
        }),
      ]
    );
  }
  console.log('• adapter configs refreshed');
}

async function ensureWaterfalls(client, ownerUserId) {
  if (!ownerUserId) {
    console.warn('⚠️ owner user missing — skipping waterfall configs');
    return;
  }
  const placements = await client.query(
    `SELECT pl.id, pl.name, pl.type
       FROM placements pl
       JOIN apps a ON a.id = pl.app_id
      WHERE a.publisher_id = $1
      ORDER BY pl.name`,
    [SANDBOX_PUBLISHER_ID]
  );
  if (placements.rowCount === 0) {
    console.warn('⚠️ No placements found for sandbox publisher.');
    return;
  }
  for (const placement of placements.rows) {
    await client.query('DELETE FROM waterfall_configs WHERE placement_id = $1', [placement.id]);
    const adapterPriorities = FAKE_ADAPTERS.map((adapter, index) => ({
      adapter_id: adapter.id,
      priority: index + 1,
      timeout_ms: adapter.timeoutMs,
      notes: adapter.behavior,
    }));
    await client.query(
      `INSERT INTO waterfall_configs (id, customer_id, placement_id, adapter_priorities, floor_price_cents, optimization_strategy, active, created_at, updated_at)
       VALUES ($1, $2, $3, $4::jsonb, $5, 'static', true, NOW(), NOW())`,
      [randomUUID(), ownerUserId, placement.id, JSON.stringify(adapterPriorities), 50]
    );
  }
  console.log(`• waterfall configs rebuilt for ${placements.rowCount} placements`);
}

async function ensureStripeCustomer(ownerUserId) {
  const client = await pool.connect();
  try {
    const owner = await client.query('SELECT email, stripe_customer_id FROM users WHERE id = $1', [ownerUserId]);
    if (owner.rowCount === 0) {
      console.warn('⚠️ Owner user missing, cannot sync Stripe account.');
      return;
    }
    const email = owner.rows[0].email;
    let customerId = owner.rows[0].stripe_customer_id;
    if (!customerId) {
      const existing = await stripe.customers.list({ email, limit: 1 });
      if (existing.data.length) {
        customerId = existing.data[0].id;
      } else {
        const created = await stripe.customers.create({
          email,
          name: 'Apex Sandbox Owner',
          description: 'Sandbox staging org billing contact',
        });
        customerId = created.id;
      }
      await client.query('UPDATE users SET stripe_customer_id = $1 WHERE id = $2', [customerId, ownerUserId]);
    }
    await ensureStripeSources(customerId, email);
    console.log(`• Stripe customer ready: ${customerId}`);
  } finally {
    client.release();
  }
}

async function ensureStripeSources(customerId, email) {
  const card = await ensureCard(customerId, email);
  await ensureBankToken(customerId, 'btok_us_verified', 'ACH');
  await ensureBankToken(customerId, 'btok_sepa_debit', 'SEPA');
  if (card) {
    await stripe.customers.update(customerId, { default_source: card });
  }
}

async function ensureCard(customerId, email) {
  const existing = await stripe.customers.listSources(customerId, { object: 'card', limit: 1 });
  if (existing.data.length) {
    return existing.data[0].id;
  }
  const card = await stripe.customers.createSource(customerId, { source: 'tok_visa' });
  console.log(`• attached test card for ${email}`);
  return card.id;
}

async function ensureBankToken(customerId, tokenId, label) {
  const object = tokenId.includes('sepa') ? 'sepa_debit' : 'bank_account';
  const existing = await stripe.customers.listSources(customerId, { object, limit: 1 });
  if (existing.data.length) {
    return existing.data[0].id;
  }
  if (tokenId.includes('sepa')) {
    console.warn('⚠️ Stripe account lacks SEPA test token support; skipping sepa source.');
    return null;
  }
  const source = await stripe.customers.createSource(customerId, { source: tokenId });
  console.log(`• attached ${label} source (${source.id})`);
  return source.id;
}

main();
