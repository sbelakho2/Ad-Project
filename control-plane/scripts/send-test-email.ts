import fetch from 'node-fetch';

// SECURITY: Use environment variables for credentials (P1 Fix #1)
const BASE_URL = process.env.CONTROL_PLANE_URL || 'https://admin.apexmediation.ee';
const USERNAME = process.env.OPERATOR_USERNAME;
const PASSWORD = process.env.OPERATOR_PASSWORD;

if (!USERNAME || !PASSWORD) {
  console.error('ERROR: OPERATOR_USERNAME and OPERATOR_PASSWORD environment variables are required');
  console.error('Usage: OPERATOR_USERNAME=xxx OPERATOR_PASSWORD=xxx npx tsx send-test-email.ts');
  process.exit(1);
}

async function main() {
  // Step 1: Login to get token
  console.log('1. Logging in...');
  const loginResp = await fetch(`${BASE_URL}/api/v1/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ 
      username: USERNAME, 
      password: PASSWORD,
      deviceId: 'cli-test-device-v2',
      deviceLabel: 'CLI Test v2'
    }),
  });
  
  if (!loginResp.ok) {
    throw new Error(`Login failed: ${loginResp.status} ${await loginResp.text()}`);
  }
  
  const loginData = await loginResp.json() as { token: string };
  const token = loginData.token;
  console.log('   ✓ Logged in');

  const headers = {
    'Content-Type': 'application/json',
    'Authorization': `Bearer ${token}`,
  };

  // Step 2: Enable system
  console.log('2. Enabling sales automation system...');
  const toggleResp = await fetch(`${BASE_URL}/api/v1/sales/toggle`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ enabled: true }),
  });
  const toggleData = await toggleResp.json();
  console.log('   Toggle result:', toggleData);

  // Step 3: Get sequences to find ID
  console.log('3. Fetching sequences...');
  const seqResp = await fetch(`${BASE_URL}/api/v1/sales/sequences`, { headers });
  const seqData = await seqResp.json() as { sequences: Array<{ id: string; name: string }> };
  console.log('   Sequences:', seqData.sequences?.map(s => `${s.name} (${s.id})`).join(', '));
  
  // Use Simple Introduction sequence
  const simpleSeq = seqData.sequences?.find(s => s.name === 'Simple Introduction');
  const sequenceId = simpleSeq?.id || seqData.sequences?.[0]?.id;
  if (!sequenceId) throw new Error('No sequences found');
  console.log('   Using sequence:', simpleSeq?.name || 'first available');

  // Step 4: Check if contact exists, create if not
  console.log('4. Getting/creating contact hello@apexmediation.ee...');
  const existingResp = await fetch(`${BASE_URL}/api/v1/sales/contacts?search=hello@apexmediation.ee`, { headers });
  const existingData = await existingResp.json() as { contacts: Array<{ id: string; email: string }> };
  
  let contactId = existingData.contacts?.find(c => c.email === 'hello@apexmediation.ee')?.id;
  
  if (!contactId) {
    const contactResp = await fetch(`${BASE_URL}/api/v1/sales/contacts`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        email: 'hello@apexmediation.ee',
        first_name: 'Apex',
        last_name: 'Team',
        company_name: 'ApexMediation',
        job_title: 'Test Contact',
        source: 'internal-test-v2',
      }),
    });
    const contactData = await contactResp.json() as { contact?: { id: string } };
    contactId = contactData.contact?.id;
  }
  
  if (!contactId) throw new Error('Could not get contact ID');
  console.log('   ✓ Contact ID:', contactId);

  // Step 5: Enroll contact in sequence (if not already enrolled)
  console.log('5. Enrolling contact in sequence...');
  const enrollResp = await fetch(`${BASE_URL}/api/v1/sales/enrollments/enroll`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      contactIds: [contactId],
      sequenceId: sequenceId,
      personalization: {
        first_name: 'Apex',
        company_name: 'ApexMediation',
        app_name: 'Test App',
      },
    }),
  });
  const enrollData = await enrollResp.json();
  console.log('   Enroll result:', enrollData);

  // Step 6: Run outreach (NOT dry run)
  console.log('6. Running outreach cycle (LIVE - will send email)...');
  const runResp = await fetch(`${BASE_URL}/api/v1/sales/run`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ dryRun: false }),
  });
  const runData = await runResp.json();
  console.log('   ✓ Outreach result:', JSON.stringify(runData, null, 2));
  
  if ((runData as any).sent > 0) {
    console.log('\n✅ Email sent successfully! Check hello@apexmediation.ee inbox.');
    console.log('   Sender should be: sales@apexmediation.ee');
  } else {
    console.log('\n⚠️  No emails sent. This might be because:');
    console.log('   - Contact already received all sequence steps');
    console.log('   - No active enrollments with pending emails');
  }
}

main().catch(err => {
  console.error('Error:', err.message);
  process.exit(1);
});
