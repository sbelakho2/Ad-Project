// Send email to Starz Electronics - with cleanup
const fetch = require('node-fetch').default;
const BASE_URL = 'https://admin.apexmediation.ee';

async function main() {
  console.log('1. Logging in...');
  const loginResp = await fetch(`${BASE_URL}/api/v1/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username: 'operator', password: '&&Pw20354491', deviceId: 'cli-starz2', deviceLabel: 'CLI' }),
  });
  const { token } = await loginResp.json();
  const headers = { 'Content-Type': 'application/json', 'Authorization': `Bearer ${token}` };
  console.log('   ✓ Logged in');

  // Delete existing contact if exists
  console.log('2. Checking for existing contact...');
  const existingResp = await fetch(`${BASE_URL}/api/v1/sales/contacts?search=contact@starzelectronics.com`, { headers });
  const existingData = await existingResp.json();
  const existing = existingData.contacts?.find(c => c.email === 'contact@starzelectronics.com');
  if (existing) {
    console.log('   Deleting existing contact:', existing.id);
    await fetch(`${BASE_URL}/api/v1/sales/contacts/${existing.id}`, { method: 'DELETE', headers });
  }

  console.log('3. Creating fresh contact...');
  const contactResp = await fetch(`${BASE_URL}/api/v1/sales/contacts`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      email: 'contact@starzelectronics.com',
      first_name: 'Starz',
      company_name: 'Starz Electronics',
      source: 'outreach',
    }),
  });
  const contactData = await contactResp.json();
  const contactId = contactData.contact?.id;
  if (!contactId) throw new Error('Failed: ' + JSON.stringify(contactData));
  console.log('   ✓ Contact:', contactId);

  console.log('4. Getting sequence...');
  const seqResp = await fetch(`${BASE_URL}/api/v1/sales/sequences`, { headers });
  const seqData = await seqResp.json();
  const seq = seqData.sequences?.find(s => s.name === 'Simple Introduction') || seqData.sequences?.[0];
  console.log('   ✓ Using:', seq.name);

  console.log('5. Enabling + enrolling...');
  await fetch(`${BASE_URL}/api/v1/sales/toggle`, { method: 'POST', headers, body: JSON.stringify({ enabled: true }) });
  const enrollResp = await fetch(`${BASE_URL}/api/v1/sales/enrollments/enroll`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      contactIds: [contactId],
      sequenceId: seq.id,
      personalization: { first_name: 'Starz', company_name: 'Starz Electronics' },
    }),
  });
  console.log('   ✓ Enrolled:', await enrollResp.json());

  console.log('6. Sending...');
  const runResp = await fetch(`${BASE_URL}/api/v1/sales/run`, { method: 'POST', headers, body: JSON.stringify({ dryRun: false }) });
  const result = await runResp.json();
  console.log('   Result:', JSON.stringify(result, null, 2));

  if (result.sent > 0) {
    console.log('\n✅ Email sent to contact@starzelectronics.com!');
  }
}

main().catch(e => { console.error('Error:', e.message); process.exit(1); });
