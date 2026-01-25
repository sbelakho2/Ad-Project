// Send test email script - v2
const fetch = require('node-fetch').default;

const BASE_URL = 'https://admin.apexmediation.ee';
const USERNAME = 'operator';
const PASSWORD = process.env.CP_PASSWORD || '&&Pw20354491';

async function main() {
  console.log('1. Logging in...');
  const loginResp = await fetch(`${BASE_URL}/api/v1/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username: USERNAME, password: PASSWORD, deviceId: 'cli-v3', deviceLabel: 'CLI v3' }),
  });
  const loginData = await loginResp.json();
  const token = loginData.token;
  console.log('   ✓ Logged in');

  const headers = { 'Content-Type': 'application/json', 'Authorization': `Bearer ${token}` };

  console.log('2. Enabling system...');
  await fetch(`${BASE_URL}/api/v1/sales/toggle`, { 
    method: 'POST', 
    headers, 
    body: JSON.stringify({ enabled: true }) 
  });
  console.log('   ✓ System enabled');

  console.log('3. Running outreach (LIVE)...');
  const runResp = await fetch(`${BASE_URL}/api/v1/sales/run`, { 
    method: 'POST', 
    headers, 
    body: JSON.stringify({ dryRun: false }) 
  });
  const runData = await runResp.json();
  console.log('   Result:', JSON.stringify(runData, null, 2));

  if (runData.sent > 0) {
    console.log('\n✅ Email sent! Check hello@apexmediation.ee');
    console.log('   Sender should now be: sales@apexmediation.ee');
  } else {
    console.log('\n⚠️  No new emails sent (contact may have received all steps already)');
  }
}

main().catch(e => { console.error('Error:', e.message); process.exit(1); });
