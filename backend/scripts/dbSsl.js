const fs = require('node:fs');

function buildSslConfig() {
  const connectionString = process.env.DATABASE_URL || '';
  const sslEnabled = process.env.DATABASE_SSL === 'true' || /sslmode=require/i.test(connectionString);
  if (!sslEnabled) return undefined;
  const caPath = process.env.DATABASE_SSL_CA;
  const rejectUnauthorizedEnv = process.env.DATABASE_SSL_REJECT_UNAUTHORIZED;
  const rejectUnauthorized = rejectUnauthorizedEnv
    ? rejectUnauthorizedEnv === 'true'
    : Boolean(caPath);

  const sslConfig = { rejectUnauthorized };
  if (caPath) {
    sslConfig.ca = fs.readFileSync(caPath, 'utf8');
  }
  return sslConfig;
}

module.exports = { buildSslConfig };
