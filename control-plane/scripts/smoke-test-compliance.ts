import fetch from 'node-fetch';

const BASE_URL = process.env.CONTROL_PLANE_URL || 'http://localhost:4000';
const TOKEN = process.env.OPERATOR_TOKEN;

if (!TOKEN) {
  console.error('Error: OPERATOR_TOKEN environment variable is required');
  process.exit(1);
}

async function runSmokeTest() {
  console.log(`Starting Compliance Smoke Test against ${BASE_URL}...`);

  try {
    // 1. Check Compliance Status
    console.log('\n1. Checking Compliance Status...');
    const statusRes = await fetch(`${BASE_URL}/api/v1/compliance/status`, {
      headers: { 'Authorization': `Bearer ${TOKEN}` }
    });
    
    if (statusRes.status !== 200) {
      throw new Error(`Failed to get status: ${statusRes.status} ${statusRes.statusText}`);
    }
    
    const status = await statusRes.json();
    console.log('✅ Status OK');
    console.log('   Company:', status.companyInfo.name);
    console.log('   VAT Pending:', status.vatStatus.pendingReports);
    
    // 2. Check VAT Reports
    console.log('\n2. Checking VAT Reports...');
    const vatRes = await fetch(`${BASE_URL}/api/v1/compliance/vat-reports`, {
      headers: { 'Authorization': `Bearer ${TOKEN}` }
    });
    
    if (vatRes.status !== 200) {
      throw new Error(`Failed to get VAT reports: ${vatRes.status} ${vatRes.statusText}`);
    }
    
    const vatReports = await vatRes.json();
    console.log(`✅ VAT Reports OK (${vatReports.length} reports found)`);
    
    // 3. Generate Annual Report (Preview)
    console.log('\n3. Generating Annual Report Preview (2025)...');
    const annualRes = await fetch(`${BASE_URL}/api/v1/compliance/annual-report/2025`, {
      headers: { 'Authorization': `Bearer ${TOKEN}` }
    });
    
    if (annualRes.status !== 200) {
      throw new Error(`Failed to generate annual report: ${annualRes.status} ${annualRes.statusText}`);
    }
    
    const annualReport = await annualRes.json();
    console.log('✅ Annual Report OK');
    console.log('   Revenue:', annualReport.financials.revenueEur, 'EUR');
    console.log('   Profit:', annualReport.financials.profitEur, 'EUR');
    
    // 4. Generate TSD Report (Preview)
    console.log('\n4. Generating TSD Report Preview (2025)...');
    const tsdRes = await fetch(`${BASE_URL}/api/v1/compliance/tsd-report/2025`, {
      headers: { 'Authorization': `Bearer ${TOKEN}` }
    });
    
    if (tsdRes.status !== 200) {
      throw new Error(`Failed to generate TSD report: ${tsdRes.status} ${tsdRes.statusText}`);
    }
    
    const tsdReport = await tsdRes.json();
    console.log('✅ TSD Report OK');
    console.log('   Employees:', tsdReport.employees);
    
    console.log('\n✨ All Smoke Tests Passed!');
    
  } catch (error) {
    console.error('\n❌ Smoke Test Failed:', error);
    process.exit(1);
  }
}

runSmokeTest();
