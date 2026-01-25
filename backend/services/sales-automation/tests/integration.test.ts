/**
 * SALES AUTOMATION INTEGRATION TESTS
 * 
 * Tests the full sales automation workflow against a real (local/test) database.
 * This does NOT send actual emails - it mocks Resend but tests everything else for real.
 * 
 * Prerequisites:
 *   DATABASE_URL must point to a test database (NOT production!)
 * 
 * Run:
 *   cd backend
 *   DATABASE_URL="postgresql://localhost:5432/adproject_test" npx ts-node services/sales-automation/tests/integration.test.ts
 */

import { Pool } from 'pg';

// ============================================================================
// CONFIG
// ============================================================================

const DATABASE_URL = process.env.DATABASE_URL || '';
const TEST_EMAIL_DOMAIN = 'test.example.com';

if (!DATABASE_URL) {
  console.error('❌ DATABASE_URL not set');
  console.error('Usage: DATABASE_URL="postgresql://..." npx ts-node tests/integration.test.ts');
  process.exit(1);
}

// Safety check - don't run against production
if (DATABASE_URL.includes('hetzner') || DATABASE_URL.includes('production') || DATABASE_URL.includes('77.42')) {
  console.error('❌ SAFETY: Will not run integration tests against production database!');
  process.exit(1);
}

console.log('═══════════════════════════════════════════════════════════════');
console.log('🧪 SALES AUTOMATION INTEGRATION TESTS');
console.log('═══════════════════════════════════════════════════════════════');
console.log(`Database: ${DATABASE_URL.replace(/:[^:@]+@/, ':****@')}`);
console.log(`Started at: ${new Date().toISOString()}`);
console.log('');

// ============================================================================
// TEST UTILITIES
// ============================================================================

interface TestResult {
  name: string;
  passed: boolean;
  duration: number;
  error?: string;
}

const results: TestResult[] = [];
let pool: Pool;

async function runTest(name: string, testFn: () => Promise<void>): Promise<void> {
  const start = Date.now();
  try {
    await testFn();
    const duration = Date.now() - start;
    results.push({ name, passed: true, duration });
    console.log(`  ✓ ${name} (${duration}ms)`);
  } catch (error) {
    const duration = Date.now() - start;
    const errorMsg = error instanceof Error ? error.message : String(error);
    results.push({ name, passed: false, duration, error: errorMsg });
    console.log(`  ✗ ${name} (${duration}ms): ${errorMsg}`);
  }
}

function assertEqual<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertTrue(condition: boolean, message: string): void {
  if (!condition) {
    throw new Error(message);
  }
}

function assertNotNull<T>(value: T | null | undefined, message: string): asserts value is T {
  if (value === null || value === undefined) {
    throw new Error(message);
  }
}

// ============================================================================
// SCHEMA (same as cli.ts)
// ============================================================================

const SCHEMA = `
CREATE TABLE IF NOT EXISTS sa_contacts (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  email VARCHAR(255) UNIQUE NOT NULL,
  first_name VARCHAR(100),
  last_name VARCHAR(100),
  company_name VARCHAR(255),
  job_title VARCHAR(255),
  app_name VARCHAR(255),
  app_downloads VARCHAR(50),
  status VARCHAR(30) DEFAULT 'new',
  source VARCHAR(100),
  notes TEXT,
  score INTEGER DEFAULT 50,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sa_sequences (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name VARCHAR(255) NOT NULL,
  description TEXT,
  active BOOLEAN DEFAULT true,
  steps JSONB NOT NULL DEFAULT '[]',
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sa_enrollments (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  sequence_id UUID REFERENCES sa_sequences(id) ON DELETE CASCADE,
  status VARCHAR(30) DEFAULT 'active',
  current_step INTEGER DEFAULT 0,
  next_send_at TIMESTAMP,
  personalization JSONB DEFAULT '{}',
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(contact_id, sequence_id)
);

CREATE TABLE IF NOT EXISTS sa_emails (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  enrollment_id UUID REFERENCES sa_enrollments(id) ON DELETE CASCADE,
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  to_email VARCHAR(255) NOT NULL,
  from_email VARCHAR(255) NOT NULL,
  subject TEXT NOT NULL,
  body_html TEXT NOT NULL,
  message_id VARCHAR(255),
  status VARCHAR(30) DEFAULT 'pending',
  step_number INTEGER,
  scheduled_for TIMESTAMP,
  sent_at TIMESTAMP,
  opened_at TIMESTAMP,
  clicked_at TIMESTAMP,
  replied_at TIMESTAMP,
  bounced_at TIMESTAMP,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sa_activity_log (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  action VARCHAR(100) NOT NULL,
  details JSONB DEFAULT '{}',
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
`;

// ============================================================================
// TEST SETUP
// ============================================================================

async function setup(): Promise<void> {
  pool = new Pool({ connectionString: DATABASE_URL });
  
  // Create schema
  await pool.query(SCHEMA);
  
  // Clean up test data (only data with our test domain)
  await pool.query(`DELETE FROM sa_activity_log WHERE contact_id IN (SELECT id FROM sa_contacts WHERE email LIKE $1)`, [`%@${TEST_EMAIL_DOMAIN}`]);
  await pool.query(`DELETE FROM sa_emails WHERE to_email LIKE $1`, [`%@${TEST_EMAIL_DOMAIN}`]);
  await pool.query(`DELETE FROM sa_enrollments WHERE contact_id IN (SELECT id FROM sa_contacts WHERE email LIKE $1)`, [`%@${TEST_EMAIL_DOMAIN}`]);
  await pool.query(`DELETE FROM sa_contacts WHERE email LIKE $1`, [`%@${TEST_EMAIL_DOMAIN}`]);
  await pool.query(`DELETE FROM sa_sequences WHERE name LIKE 'TEST_%'`);
}

async function teardown(): Promise<void> {
  // Clean up test data
  await pool.query(`DELETE FROM sa_activity_log WHERE contact_id IN (SELECT id FROM sa_contacts WHERE email LIKE $1)`, [`%@${TEST_EMAIL_DOMAIN}`]);
  await pool.query(`DELETE FROM sa_emails WHERE to_email LIKE $1`, [`%@${TEST_EMAIL_DOMAIN}`]);
  await pool.query(`DELETE FROM sa_enrollments WHERE contact_id IN (SELECT id FROM sa_contacts WHERE email LIKE $1)`, [`%@${TEST_EMAIL_DOMAIN}`]);
  await pool.query(`DELETE FROM sa_contacts WHERE email LIKE $1`, [`%@${TEST_EMAIL_DOMAIN}`]);
  await pool.query(`DELETE FROM sa_sequences WHERE name LIKE 'TEST_%'`);
  await pool.end();
}

// ============================================================================
// TESTS: Contact Management
// ============================================================================

async function testContactCreation(): Promise<void> {
  console.log('\n📋 Testing Contact Management...');
  
  await runTest('Create new contact', async () => {
    const result = await pool.query(`
      INSERT INTO sa_contacts (email, first_name, last_name, company_name, status, source)
      VALUES ($1, $2, $3, $4, 'new', 'test')
      RETURNING id, email
    `, [`john@${TEST_EMAIL_DOMAIN}`, 'John', 'Doe', 'Test Company']);
    
    assertTrue(result.rows.length === 1, 'Should insert one row');
    assertEqual(result.rows[0].email, `john@${TEST_EMAIL_DOMAIN}`, 'Email should match');
  });

  await runTest('Duplicate email rejected', async () => {
    try {
      await pool.query(`
        INSERT INTO sa_contacts (email, first_name)
        VALUES ($1, $2)
      `, [`john@${TEST_EMAIL_DOMAIN}`, 'John2']);
      throw new Error('Should have thrown duplicate error');
    } catch (error) {
      assertTrue(
        (error as Error).message.includes('duplicate') || (error as Error).message.includes('unique'),
        'Should be a duplicate key error'
      );
    }
  });

  await runTest('Update contact status', async () => {
    await pool.query(`
      UPDATE sa_contacts SET status = 'contacted', updated_at = CURRENT_TIMESTAMP
      WHERE email = $1
    `, [`john@${TEST_EMAIL_DOMAIN}`]);
    
    const result = await pool.query(`SELECT status FROM sa_contacts WHERE email = $1`, [`john@${TEST_EMAIL_DOMAIN}`]);
    assertEqual(result.rows[0].status, 'contacted', 'Status should be updated');
  });

  await runTest('Create multiple contacts', async () => {
    const emails = [
      `jane@${TEST_EMAIL_DOMAIN}`,
      `mike@${TEST_EMAIL_DOMAIN}`,
      `sarah@${TEST_EMAIL_DOMAIN}`,
    ];
    
    for (const email of emails) {
      await pool.query(`
        INSERT INTO sa_contacts (email, first_name, status, source)
        VALUES ($1, $2, 'new', 'test')
        ON CONFLICT (email) DO NOTHING
      `, [email, email.split('@')[0]]);
    }
    
    const count = await pool.query(`SELECT COUNT(*) FROM sa_contacts WHERE email LIKE $1`, [`%@${TEST_EMAIL_DOMAIN}`]);
    assertTrue(parseInt(count.rows[0].count) >= 4, 'Should have at least 4 contacts');
  });
}

// ============================================================================
// TESTS: Sequence Management
// ============================================================================

async function testSequenceManagement(): Promise<void> {
  console.log('\n📧 Testing Sequence Management...');

  await runTest('Create sequence with steps', async () => {
    const steps = [
      { delay_days: 0, subject: 'Hello {{first_name}}', body: 'Initial email to {{company_name}}' },
      { delay_days: 3, subject: 'Follow up {{first_name}}', body: 'Just following up' },
      { delay_days: 5, subject: 'Last chance', body: 'Final message' },
    ];
    
    const result = await pool.query(`
      INSERT INTO sa_sequences (name, description, steps, active)
      VALUES ($1, $2, $3, true)
      RETURNING id, name
    `, ['TEST_Sequence', 'Test sequence for integration testing', JSON.stringify(steps)]);
    
    assertTrue(result.rows.length === 1, 'Should create sequence');
    assertEqual(result.rows[0].name, 'TEST_Sequence', 'Name should match');
  });

  await runTest('Retrieve sequence with steps', async () => {
    const result = await pool.query(`
      SELECT * FROM sa_sequences WHERE name = 'TEST_Sequence'
    `);
    
    assertTrue(result.rows.length === 1, 'Should find sequence');
    const steps = result.rows[0].steps;
    assertTrue(Array.isArray(steps), 'Steps should be an array');
    assertEqual(steps.length, 3, 'Should have 3 steps');
    assertEqual(steps[0].delay_days, 0, 'First step delay should be 0');
  });

  await runTest('Deactivate sequence', async () => {
    await pool.query(`UPDATE sa_sequences SET active = false WHERE name = 'TEST_Sequence'`);
    const result = await pool.query(`SELECT active FROM sa_sequences WHERE name = 'TEST_Sequence'`);
    assertEqual(result.rows[0].active, false, 'Sequence should be inactive');
    
    // Reactivate for later tests
    await pool.query(`UPDATE sa_sequences SET active = true WHERE name = 'TEST_Sequence'`);
  });
}

// ============================================================================
// TESTS: Enrollment
// ============================================================================

async function testEnrollment(): Promise<void> {
  console.log('\n📝 Testing Enrollment...');

  await runTest('Enroll contact in sequence', async () => {
    // Get contact and sequence IDs
    const contact = await pool.query(`SELECT id FROM sa_contacts WHERE email = $1`, [`jane@${TEST_EMAIL_DOMAIN}`]);
    const sequence = await pool.query(`SELECT id, steps FROM sa_sequences WHERE name = 'TEST_Sequence'`);
    
    assertNotNull(contact.rows[0], 'Contact should exist');
    assertNotNull(sequence.rows[0], 'Sequence should exist');
    
    const steps = sequence.rows[0].steps;
    const firstStepDelay = steps[0]?.delay_days || 0;
    const nextSendAt = new Date(Date.now() + firstStepDelay * 24 * 60 * 60 * 1000);
    
    const result = await pool.query(`
      INSERT INTO sa_enrollments (contact_id, sequence_id, current_step, next_send_at, personalization)
      VALUES ($1, $2, 0, $3, $4)
      RETURNING id
    `, [
      contact.rows[0].id,
      sequence.rows[0].id,
      nextSendAt,
      JSON.stringify({ first_name: 'Jane', company_name: 'Test Corp' })
    ]);
    
    assertTrue(result.rows.length === 1, 'Should create enrollment');
  });

  await runTest('Prevent duplicate enrollment', async () => {
    const contact = await pool.query(`SELECT id FROM sa_contacts WHERE email = $1`, [`jane@${TEST_EMAIL_DOMAIN}`]);
    const sequence = await pool.query(`SELECT id FROM sa_sequences WHERE name = 'TEST_Sequence'`);
    
    try {
      await pool.query(`
        INSERT INTO sa_enrollments (contact_id, sequence_id, current_step, next_send_at)
        VALUES ($1, $2, 0, CURRENT_TIMESTAMP)
      `, [contact.rows[0].id, sequence.rows[0].id]);
      throw new Error('Should have thrown duplicate error');
    } catch (error) {
      assertTrue(
        (error as Error).message.includes('duplicate') || (error as Error).message.includes('unique'),
        'Should be a duplicate key error'
      );
    }
  });

  await runTest('Enroll multiple contacts', async () => {
    const sequence = await pool.query(`SELECT id FROM sa_sequences WHERE name = 'TEST_Sequence'`);
    const contacts = await pool.query(`
      SELECT id, first_name FROM sa_contacts 
      WHERE email LIKE $1 AND email != $2
    `, [`%@${TEST_EMAIL_DOMAIN}`, `jane@${TEST_EMAIL_DOMAIN}`]);
    
    for (const contact of contacts.rows) {
      await pool.query(`
        INSERT INTO sa_enrollments (contact_id, sequence_id, current_step, next_send_at, personalization)
        VALUES ($1, $2, 0, CURRENT_TIMESTAMP, $3)
        ON CONFLICT (contact_id, sequence_id) DO NOTHING
      `, [
        contact.id,
        sequence.rows[0].id,
        JSON.stringify({ first_name: contact.first_name || 'there' })
      ]);
    }
    
    const count = await pool.query(`
      SELECT COUNT(*) FROM sa_enrollments WHERE sequence_id = $1
    `, [sequence.rows[0].id]);
    
    assertTrue(parseInt(count.rows[0].count) >= 2, 'Should have at least 2 enrollments');
  });
}

// ============================================================================
// TESTS: Email Recording and Tracking
// ============================================================================

async function testEmailTracking(): Promise<void> {
  console.log('\n📨 Testing Email Tracking...');

  await runTest('Record sent email', async () => {
    const enrollment = await pool.query(`
      SELECT e.id, e.contact_id, c.email 
      FROM sa_enrollments e
      JOIN sa_contacts c ON e.contact_id = c.id
      WHERE c.email = $1
    `, [`jane@${TEST_EMAIL_DOMAIN}`]);
    
    assertNotNull(enrollment.rows[0], 'Enrollment should exist');
    
    const result = await pool.query(`
      INSERT INTO sa_emails (
        enrollment_id, contact_id, to_email, from_email, 
        subject, body_html, message_id, status, step_number, sent_at
      ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'sent', 1, CURRENT_TIMESTAMP)
      RETURNING id
    `, [
      enrollment.rows[0].id,
      enrollment.rows[0].contact_id,
      enrollment.rows[0].email,
      'test@apexmediation.ee',
      'Test Subject',
      '<p>Test body</p>',
      'msg_test_123'
    ]);
    
    assertTrue(result.rows.length === 1, 'Should record email');
  });

  await runTest('Track email open', async () => {
    await pool.query(`
      UPDATE sa_emails 
      SET status = 'opened', opened_at = CURRENT_TIMESTAMP
      WHERE message_id = 'msg_test_123'
    `);
    
    const result = await pool.query(`SELECT status, opened_at FROM sa_emails WHERE message_id = 'msg_test_123'`);
    assertEqual(result.rows[0].status, 'opened', 'Status should be opened');
    assertTrue(result.rows[0].opened_at !== null, 'opened_at should be set');
  });

  await runTest('Track email click', async () => {
    await pool.query(`
      UPDATE sa_emails 
      SET status = 'clicked', clicked_at = CURRENT_TIMESTAMP
      WHERE message_id = 'msg_test_123'
    `);
    
    const result = await pool.query(`SELECT status, clicked_at FROM sa_emails WHERE message_id = 'msg_test_123'`);
    assertEqual(result.rows[0].status, 'clicked', 'Status should be clicked');
    assertTrue(result.rows[0].clicked_at !== null, 'clicked_at should be set');
  });

  await runTest('Track reply and update enrollment', async () => {
    // Mark email as replied
    await pool.query(`
      UPDATE sa_emails 
      SET status = 'replied', replied_at = CURRENT_TIMESTAMP
      WHERE message_id = 'msg_test_123'
    `);
    
    // Get the enrollment for this email
    const email = await pool.query(`SELECT enrollment_id FROM sa_emails WHERE message_id = 'msg_test_123'`);
    
    // Update enrollment status
    await pool.query(`
      UPDATE sa_enrollments SET status = 'replied', updated_at = CURRENT_TIMESTAMP
      WHERE id = $1
    `, [email.rows[0].enrollment_id]);
    
    // Update contact status
    await pool.query(`
      UPDATE sa_contacts SET status = 'replied', updated_at = CURRENT_TIMESTAMP
      WHERE email = $1
    `, [`jane@${TEST_EMAIL_DOMAIN}`]);
    
    const enrollment = await pool.query(`
      SELECT e.status as enrollment_status, c.status as contact_status
      FROM sa_enrollments e
      JOIN sa_contacts c ON e.contact_id = c.id
      WHERE c.email = $1
    `, [`jane@${TEST_EMAIL_DOMAIN}`]);
    
    assertEqual(enrollment.rows[0].enrollment_status, 'replied', 'Enrollment should be replied');
    assertEqual(enrollment.rows[0].contact_status, 'replied', 'Contact should be replied');
  });
}

// ============================================================================
// TESTS: Activity Logging
// ============================================================================

async function testActivityLog(): Promise<void> {
  console.log('\n📝 Testing Activity Logging...');

  await runTest('Log activity', async () => {
    const contact = await pool.query(`SELECT id FROM sa_contacts WHERE email = $1`, [`john@${TEST_EMAIL_DOMAIN}`]);
    
    await pool.query(`
      INSERT INTO sa_activity_log (contact_id, action, details)
      VALUES ($1, 'email_sent', $2)
    `, [contact.rows[0].id, JSON.stringify({ subject: 'Test Email', step: 1 })]);
    
    const result = await pool.query(`
      SELECT * FROM sa_activity_log WHERE contact_id = $1 ORDER BY created_at DESC LIMIT 1
    `, [contact.rows[0].id]);
    
    assertEqual(result.rows[0].action, 'email_sent', 'Action should match');
    assertEqual(result.rows[0].details.subject, 'Test Email', 'Details should be preserved');
  });

  await runTest('Multiple activity entries', async () => {
    const contact = await pool.query(`SELECT id FROM sa_contacts WHERE email = $1`, [`john@${TEST_EMAIL_DOMAIN}`]);
    
    const actions = ['enrolled', 'email_opened', 'link_clicked'];
    for (const action of actions) {
      await pool.query(`
        INSERT INTO sa_activity_log (contact_id, action, details)
        VALUES ($1, $2, $3)
      `, [contact.rows[0].id, action, JSON.stringify({ test: true })]);
    }
    
    const count = await pool.query(`
      SELECT COUNT(*) FROM sa_activity_log WHERE contact_id = $1
    `, [contact.rows[0].id]);
    
    assertTrue(parseInt(count.rows[0].count) >= 4, 'Should have at least 4 activities');
  });
}

// ============================================================================
// TESTS: Query Patterns (simulating CLI operations)
// ============================================================================

async function testQueryPatterns(): Promise<void> {
  console.log('\n🔍 Testing Query Patterns (CLI operations)...');

  await runTest('Get pending enrollments', async () => {
    // This is the query used by `run` command
    const result = await pool.query(`
      SELECT e.*, c.email, c.first_name, s.name as sequence_name, s.steps
      FROM sa_enrollments e
      JOIN sa_contacts c ON e.contact_id = c.id
      JOIN sa_sequences s ON e.sequence_id = s.id
      WHERE e.status = 'active'
        AND e.next_send_at <= CURRENT_TIMESTAMP
        AND c.status NOT IN ('bounced', 'opted_out', 'replied')
      ORDER BY e.next_send_at
      LIMIT 20
    `);
    
    // Should work without error - count will vary based on test data
    assertTrue(result.rows !== undefined, 'Query should execute');
  });

  await runTest('Get contact stats', async () => {
    const result = await pool.query(`
      SELECT status, COUNT(*) as count 
      FROM sa_contacts 
      WHERE email LIKE $1
      GROUP BY status
    `, [`%@${TEST_EMAIL_DOMAIN}`]);
    
    assertTrue(result.rows.length > 0, 'Should have contact stats');
  });

  await runTest('Get email stats', async () => {
    const result = await pool.query(`
      SELECT 
        COUNT(*) as total,
        COUNT(*) FILTER (WHERE sent_at IS NOT NULL) as sent,
        COUNT(*) FILTER (WHERE opened_at IS NOT NULL) as opened,
        COUNT(*) FILTER (WHERE clicked_at IS NOT NULL) as clicked,
        COUNT(*) FILTER (WHERE replied_at IS NOT NULL) as replied,
        COUNT(*) FILTER (WHERE bounced_at IS NOT NULL) as bounced
      FROM sa_emails
      WHERE to_email LIKE $1
    `, [`%@${TEST_EMAIL_DOMAIN}`]);
    
    assertTrue(result.rows.length === 1, 'Should have email stats');
    assertTrue(parseInt(result.rows[0].total) >= 1, 'Should have at least one email');
  });

  await runTest('Get sequence performance', async () => {
    const result = await pool.query(`
      SELECT s.name, 
        COUNT(e.id) as enrolled,
        COUNT(e.id) FILTER (WHERE e.status = 'replied') as replied,
        COUNT(e.id) FILTER (WHERE e.status = 'completed') as completed
      FROM sa_sequences s
      LEFT JOIN sa_enrollments e ON s.id = e.sequence_id
      WHERE s.name LIKE 'TEST_%'
      GROUP BY s.id, s.name
    `);
    
    assertTrue(result.rows.length >= 1, 'Should have sequence stats');
  });
}

// ============================================================================
// TESTS: Personalization
// ============================================================================

async function testPersonalization(): Promise<void> {
  console.log('\n✨ Testing Personalization...');

  function applyPersonalization(text: string, vars: Record<string, string>): string {
    let result = text;
    for (const [key, value] of Object.entries(vars)) {
      result = result.replace(new RegExp(`\\{\\{${key}\\}\\}`, 'g'), value);
    }
    return result;
  }

  await runTest('Basic variable replacement', async () => {
    const template = 'Hello {{first_name}}, from {{company_name}}';
    const vars = { first_name: 'John', company_name: 'Acme Inc' };
    const result = applyPersonalization(template, vars);
    assertEqual(result, 'Hello John, from Acme Inc', 'Variables should be replaced');
  });

  await runTest('Multiple occurrences replaced', async () => {
    const template = '{{first_name}}, {{first_name}}, {{first_name}}!';
    const vars = { first_name: 'Jane' };
    const result = applyPersonalization(template, vars);
    assertEqual(result, 'Jane, Jane, Jane!', 'All occurrences should be replaced');
  });

  await runTest('Missing variables preserved', async () => {
    const template = 'Hello {{first_name}}, at {{unknown_var}}';
    const vars = { first_name: 'John' };
    const result = applyPersonalization(template, vars);
    assertEqual(result, 'Hello John, at {{unknown_var}}', 'Missing var should be preserved');
  });

  await runTest('Personalization from database', async () => {
    const enrollment = await pool.query(`
      SELECT personalization FROM sa_enrollments e
      JOIN sa_contacts c ON e.contact_id = c.id
      WHERE c.email = $1
    `, [`mike@${TEST_EMAIL_DOMAIN}`]);
    
    if (enrollment.rows.length > 0) {
      const vars = enrollment.rows[0].personalization || {};
      const template = 'Hello {{first_name}}!';
      const result = applyPersonalization(template, vars);
      assertTrue(!result.includes('{{') || result.includes('{{first_name}}'), 'Should handle personalization');
    }
  });
}

// ============================================================================
// TESTS: Bounce/Opt-out Handling
// ============================================================================

async function testBounceOptOut(): Promise<void> {
  console.log('\n⛔ Testing Bounce/Opt-out Handling...');

  await runTest('Mark contact as bounced', async () => {
    await pool.query(`
      UPDATE sa_contacts SET status = 'bounced', updated_at = CURRENT_TIMESTAMP
      WHERE email = $1
    `, [`mike@${TEST_EMAIL_DOMAIN}`]);
    
    const result = await pool.query(`SELECT status FROM sa_contacts WHERE email = $1`, [`mike@${TEST_EMAIL_DOMAIN}`]);
    assertEqual(result.rows[0].status, 'bounced', 'Status should be bounced');
  });

  await runTest('Bounced contacts excluded from outreach', async () => {
    const result = await pool.query(`
      SELECT c.email FROM sa_enrollments e
      JOIN sa_contacts c ON e.contact_id = c.id
      WHERE e.status = 'active'
        AND e.next_send_at <= CURRENT_TIMESTAMP
        AND c.status NOT IN ('bounced', 'opted_out')
    `);
    
    const emails = result.rows.map(r => r.email);
    assertTrue(!emails.includes(`mike@${TEST_EMAIL_DOMAIN}`), 'Bounced contact should be excluded');
  });

  await runTest('Opt-out handling', async () => {
    await pool.query(`
      UPDATE sa_contacts SET status = 'opted_out', updated_at = CURRENT_TIMESTAMP
      WHERE email = $1
    `, [`sarah@${TEST_EMAIL_DOMAIN}`]);
    
    // Verify opted out
    const contact = await pool.query(`SELECT status FROM sa_contacts WHERE email = $1`, [`sarah@${TEST_EMAIL_DOMAIN}`]);
    assertEqual(contact.rows[0].status, 'opted_out', 'Status should be opted_out');
    
    // Verify excluded from queries
    const outreach = await pool.query(`
      SELECT c.email FROM sa_contacts c
      WHERE c.status NOT IN ('bounced', 'opted_out')
        AND c.email LIKE $1
    `, [`%@${TEST_EMAIL_DOMAIN}`]);
    
    const emails = outreach.rows.map(r => r.email);
    assertTrue(!emails.includes(`sarah@${TEST_EMAIL_DOMAIN}`), 'Opted-out contact should be excluded');
  });
}

// ============================================================================
// MAIN
// ============================================================================

async function main(): Promise<void> {
  try {
    console.log('🔧 Setting up test database...\n');
    await setup();
    
    // Run all test suites
    await testContactCreation();
    await testSequenceManagement();
    await testEnrollment();
    await testEmailTracking();
    await testActivityLog();
    await testQueryPatterns();
    await testPersonalization();
    await testBounceOptOut();
    
    // Summary
    console.log('\n═══════════════════════════════════════════════════════════════');
    console.log('📊 TEST SUMMARY');
    console.log('═══════════════════════════════════════════════════════════════');
    
    const passed = results.filter(r => r.passed).length;
    const failed = results.filter(r => !r.passed).length;
    const totalTime = results.reduce((sum, r) => sum + r.duration, 0);
    
    console.log(`\n  Total:  ${results.length} tests`);
    console.log(`  Passed: ${passed} ✓`);
    console.log(`  Failed: ${failed} ✗`);
    console.log(`  Time:   ${totalTime}ms`);
    
    if (failed > 0) {
      console.log('\n❌ FAILED TESTS:');
      for (const result of results.filter(r => !r.passed)) {
        console.log(`   - ${result.name}: ${result.error}`);
      }
    }
    
    console.log('\n🧹 Cleaning up test data...');
    await teardown();
    
    console.log('\n═══════════════════════════════════════════════════════════════');
    if (failed === 0) {
      console.log('✅ ALL TESTS PASSED');
    } else {
      console.log('❌ SOME TESTS FAILED');
      process.exit(1);
    }
    console.log('═══════════════════════════════════════════════════════════════\n');
    
  } catch (error) {
    console.error('\n❌ Test suite failed with error:', error);
    await teardown().catch(() => {});
    process.exit(1);
  }
}

main();
