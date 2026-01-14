#!/usr/bin/env node
require('dotenv/config');
const { Pool } = require('pg');
const { buildSslConfig } = require('./dbSsl');
const jwt = require('jsonwebtoken');

async function main() {
  const email = process.env.EMAIL || process.argv[2];
  if (!email) {
    console.error('Usage: EMAIL=user@example.com node backend/scripts/mint_token.js');
    process.exit(2);
  }

  const connectionString = process.env.DATABASE_URL;
  if (!connectionString) {
    console.error('DATABASE_URL is required');
    process.exit(2);
  }
  const ssl = buildSslConfig();
  const jwtSecret = process.env.JWT_SECRET;
  if (!jwtSecret) {
    console.error('JWT_SECRET is required in container env');
    process.exit(2);
  }

  const pool = new Pool({ connectionString, ssl });
  try {
    const r = await pool.query('SELECT id, publisher_id FROM users WHERE email = $1 LIMIT 1', [email]);
    if (!r.rows.length) {
      console.error('User not found: ' + email);
      process.exit(2);
    }
    const u = r.rows[0];
    const token = jwt.sign(
      { userId: u.id, publisherId: u.publisher_id, email, tokenType: 'access' },
      jwtSecret,
      { expiresIn: '7d' }
    );
    process.stdout.write(token);
  } finally {
    await pool.end();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
