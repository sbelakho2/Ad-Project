// Send fresh test email - creates new enrollment
const fetch = require('node-fetch').default;

const BASE_URL = 'https://admin.apexmediation.ee';
const USERNAME = 'operator';
const PASSWORD = '&&Pw20354491';

async function main() {
  console.log('1. Logging in...');
  const loginResp = await fetch(`${BASE_URL}/api/v1/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username: USERNAME, password: PASSWORD, deviceId: 'cli-fresh', deviceLabel: 'CLI Fresh' }),
  });
  const loginData = await loginResp.json();
  const token = loginData.token;
  console.log('   ✓ Logged in');

  const headers = { 'Content-Type': 'application/json', 'Authorization': `Bearer ${token}` };

  // Get sequences
  console.log('2. Getting sequences...');
  const seqResp = await fetch(`${BASE_URL}/api/v1/sales/sequences`, { headers });
  const seqData = await seqResp.json();
  console.log('   Sequences:', seqData.sequences?.map(s => s.name).join(', '));
  
  // Find Simple Introduction sequence (the one not used yet or use first)
  const seq = seqData.sequences?.find(s => s.name === 'Simple Introduction') || seqData.sequences?.[0];
  if (!seq) throw new Error('No sequences');
  console.log('   Using:', seq.name);

  // Delete existing contact and create fresh one
  console.log('3. Creating fresh contact...');
  
  // First, get existing contact
  const existingResp = await fetch(`${BASE_URL}/api/v1/sales/contacts?search=hello@apexmediation.ee`, { headers });
  const existingData = await existingResp.json();
  const existingContact = existingData.contacts?.find(c => c.email === 'hello@apexmediation.ee');
  
  if (existingContact) {
    console.log('   Deleting old contact:', existingContact.id);
    await fetch(`${BASE_URL}/api/v1/sales/contacts/${existingContact.id}`, { method: 'DELETE', headers });
  }
  
  // Create new contact
  const contactResp = await fetch(`${BASE_URL}/api/v1/sales/contacts`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      email: 'hello@apexmediation.ee',
      first_name: 'Apex',
      last_name: 'Team',
      company_name: 'ApexMediation',
      source: 'fresh-test',
    }),
  });
  const contactData = await contactResp.json();
  const contactId = contactData.contact?.id;
  if (!contactId) throw new Error('Failed to create contact');
  console.log('   ✓ Created contact:', contactId);

  // Enable system
  console.log('4. Enabling system...');
  await fetch(`${BASE_URL}/api/v1/sales/toggle`, { method: 'POST', headers, body: JSON.stringify({ enabled: true }) });
  console.log('   ✓ Enabled');

  // Enroll in sequence
  console.log('5. Enrolling in sequence...');
  const enrollResp = await fetch(`${BASE_URL}/api/v1/sales/enrollments/enroll`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      contactIds: [contactId],
      sequenceId: seq.id,
      personalization: { first_name: 'Apex', company_name: 'ApexMediation' },
    }),
  });
  const enrollData = await enrollResp.json();
  console.log('   Enroll result:', enrollData);

  // Run outreach
  console.log('6. Running outreach (LIVE)...');
  const runResp = await fetch(`${BASE_URL}/api/v1/sales/run`, { method: 'POST', headers, body: JSON.stringify({ dryRun: false }) });
  const runData = await runResp.json();
  console.log('   Result:', JSON.stringify(runData, null, 2));

  if (runData.sent > 0) {
    console.log('\n✅ Email sent to hello@apexmediation.ee!');
    console.log('   Sender should be: sales@apexmediation.ee');
    console.log('   From name should be: Apexmediation Team');
  } else {
    console.log('\n❌ No emails sent. Check errors above.');
  }
}

main().catch(e => { console.error('Error:', e.message); process.exit(1); });
