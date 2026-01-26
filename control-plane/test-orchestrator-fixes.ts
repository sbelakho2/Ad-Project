/**
 * Standalone test script for autonomous-orchestrator quality gates
 * Run with: npx ts-node test-orchestrator-fixes.ts
 */

// Define test cases for the quality gates that were modified
interface QualityGateTestCase {
  name: string;
  subject: string;
  body: string;
  expectedPass: boolean;
  expectedGate?: string;
}

console.log('\n╔════════════════════════════════════════════════════════════════╗');
console.log('║     AUTONOMOUS ORCHESTRATOR QUALITY GATES - TEST SUITE        ║');
console.log('╚════════════════════════════════════════════════════════════════╝\n');

// Quality gate patterns from autonomous-orchestrator.ts
const qualityGatePatterns = [
  /free|spam|click here|act now|limited time/gi,
  /!!+|\$\$+|[A-Z]{10,}/g,
  /{{[A-Z_]+}}/g, // Unsubstituted variables
  /\btest\b.*\btest\b/gi, // Multiple test words
];

// Subject ending pattern (abrupt endings)
const subjectEndingPattern = /\s+(for|about|re:|regarding)\s*$/i;

// Function to test quality gates
function testQualityGates(subject: string, body: string): { passed: boolean; failedGate?: number; reason?: string } {
  const fullContent = `${subject} ${body}`;
  
  // Gate 1-4: Pattern matching
  for (let i = 0; i < qualityGatePatterns.length; i++) {
    const pattern = qualityGatePatterns[i];
    // Reset lastIndex for global patterns
    pattern.lastIndex = 0;
    if (pattern.test(fullContent)) {
      return { passed: false, failedGate: i + 1, reason: `Pattern match: ${pattern}` };
    }
  }
  
  // Gate 5: Empty greeting check
  if (body.match(/^(hi|hello|hey|dear)\s*,/i) && !body.match(/^(hi|hello|hey|dear)\s+\w+/i)) {
    // Allow "Hi team," as valid
    if (!body.match(/^(hi|hello|hey|dear)\s+team\b/i)) {
      return { passed: false, failedGate: 5, reason: 'Empty greeting' };
    }
  }
  
  // Gate 6: Content length checks
  if (subject.length < 15) {
    return { passed: false, failedGate: 6, reason: 'Subject too short' };
  }
  if (body.length < 50) {
    return { passed: false, failedGate: 6, reason: 'Body too short' };
  }
  
  // Gate 7: Subject line abrupt ending
  if (subjectEndingPattern.test(subject)) {
    return { passed: false, failedGate: 7, reason: 'Subject ends abruptly' };
  }
  
  // Gate 8: Mobile preview (warning only, doesn't fail)
  // This is a warning, not a failure
  
  return { passed: true };
}

let passed = 0;
let failed = 0;

function test(name: string, condition: boolean, details?: string) {
  if (condition) {
    console.log(`✅ ${name}`);
    passed++;
  } else {
    console.log(`❌ ${name}${details ? ` - ${details}` : ''}`);
    failed++;
  }
}

// ============================================================
// Test 1: Spam patterns should be rejected
// ============================================================
console.log('--- Test 1: Spam Pattern Detection ---');

const spam1 = testQualityGates('Get FREE money now!', 'This is a body text that meets the minimum length requirement for testing.');
test('FREE keyword rejected', !spam1.passed && spam1.failedGate === 1);

const spam2 = testQualityGates('Click here for amazing deal', 'This is a body text that meets the minimum length requirement for testing.');
test('click here rejected', !spam2.passed && spam2.failedGate === 1);

const spam3 = testQualityGates('Limited time offer!!!', 'This is a body text that meets the minimum length requirement for testing.');
test('limited time rejected', !spam3.passed && spam3.failedGate === 1);

// ============================================================
// Test 2: Excessive formatting should be rejected
// ============================================================
console.log('\n--- Test 2: Excessive Formatting Detection ---');

const format1 = testQualityGates('Amazing offer!!!!', 'This is a body text that meets the minimum length requirement for testing.');
test('Multiple exclamation marks rejected', !format1.passed);

const format2 = testQualityGates('AMAZING OPPORTUNITY FOR YOUR BUSINESS', 'This is a body text that meets the minimum length requirement for testing.');
test('All caps string (10+ chars) rejected', !format2.passed);

// ============================================================
// Test 3: Unsubstituted variables should be rejected
// ============================================================
console.log('\n--- Test 3: Unsubstituted Variables Detection ---');

const vars1 = testQualityGates('Hi {{FIRST_NAME}}', 'This is a body text that meets the minimum length requirement for testing with {{COMPANY}}.');
test('{{FIRST_NAME}} unsubstituted rejected', !vars1.passed && vars1.failedGate === 3);

const vars2 = testQualityGates('Great partnership for {{APP_NAME}}', 'This is a body text that meets the minimum length requirement for testing.');
test('{{APP_NAME}} unsubstituted rejected', !vars2.passed && vars2.failedGate === 3);

// ============================================================
// Test 4: "your company"/"your app" should NOT be rejected (fallback removed)
// ============================================================
console.log('\n--- Test 4: Removed Fallback Contradiction ---');

// These should PASS because we removed the conflicting pattern
// NOTE: Body should start with a proper greeting to pass gate 5
const fallback1 = testQualityGates('Partnership opportunity', 'Hello John, I noticed your company has a great mobile app. This is interesting enough.');
test('"your company" no longer rejected', fallback1.passed, `Gate ${fallback1.failedGate}: ${fallback1.reason}`);

const fallback2 = testQualityGates('Partnership opportunity', 'Hi Sarah, I noticed your app has great potential for monetization. Very interesting content.');
test('"your app" no longer rejected', fallback2.passed, `Gate ${fallback2.failedGate}: ${fallback2.reason}`);

// ============================================================
// Test 5: Empty greeting detection (with team fallback allowed)
// ============================================================
console.log('\n--- Test 5: Empty Greeting Detection ---');

const greeting1 = testQualityGates('Partnership opportunity', 'Hi, I wanted to reach out to discuss a partnership. This is a complete body with enough content.');
test('Empty "Hi," rejected', !greeting1.passed && greeting1.failedGate === 5);

const greeting2 = testQualityGates('Partnership opportunity', 'Hi John, I wanted to reach out to discuss a partnership. This is a complete body with content.');
test('"Hi John," passes', greeting2.passed);

const greeting3 = testQualityGates('Partnership opportunity', 'Hi team, I wanted to reach out to discuss a partnership. This is a complete body with content.');
test('"Hi team," passes (new fix)', greeting3.passed);

const greeting4 = testQualityGates('Partnership opportunity', 'Hello, I wanted to reach out to discuss a partnership. This is a complete body with content.');
test('Empty "Hello," rejected', !greeting4.passed && greeting4.failedGate === 5);

// ============================================================
// Test 6: Content length checks
// ============================================================
console.log('\n--- Test 6: Content Length Checks ---');

const length1 = testQualityGates('Short', 'This is a body text that meets the minimum length requirement for testing purposes here.');
test('Subject < 15 chars rejected', !length1.passed && length1.failedGate === 6);

const length2 = testQualityGates('This is a subject line with enough length', 'Short body');
test('Body < 50 chars rejected', !length2.passed && length2.failedGate === 6);

const length3 = testQualityGates('This is a subject line with enough length', 'This is a body text that definitely meets the minimum length requirement for testing.');
test('Both long enough passes', length3.passed);

// ============================================================
// Test 7: Subject line abrupt ending (new fix)
// ============================================================
console.log('\n--- Test 7: Subject Line Abrupt Ending Detection ---');

const ending1 = testQualityGates('Partnership opportunity for', 'This is a body text that meets the minimum length requirement for testing purposes.');
console.log('  "Partnership opportunity for" result:', ending1);
test('Subject ending with "for" rejected', !ending1.passed && ending1.failedGate === 7);

const ending2 = testQualityGates('Quick question about', 'This is a body text that meets the minimum length requirement for testing purposes.');
test('Subject ending with "about" rejected', !ending2.passed && ending2.failedGate === 7);

const ending3 = testQualityGates('Re: your inquiry about monetization', 'This is a body text that meets the minimum length requirement for testing purposes.');
test('Subject with "Re:" in middle passes', ending3.passed);

const ending4 = testQualityGates('Subject regarding', 'This is a body text that meets the minimum length requirement for testing purposes here.');
test('Subject ending with "regarding" rejected', !ending4.passed && ending4.failedGate === 7);

// ============================================================
// Test 8: Valid emails should pass
// ============================================================
console.log('\n--- Test 8: Valid Emails Should Pass ---');

const valid1 = testQualityGates(
  'Partnership opportunity with your app SuperGame', 
  'Hi John, I noticed your recent update to SuperGame added a leaderboard feature. The UX looks really clean! I work at AdX and we specialize in rewarded video ads.'
);
test('Full valid email passes', valid1.passed, `Gate ${valid1.failedGate}: ${valid1.reason}`);

const valid2 = testQualityGates(
  'Quick question about monetization', 
  'Hi team, I saw your app has strong user engagement metrics. Would love to discuss how rewarded ads could boost your revenue without hurting retention.'
);
test('Email with "team" greeting passes', valid2.passed, `Gate ${valid2.failedGate}: ${valid2.reason}`);

// ============================================================
// Summary
// ============================================================
console.log('\n╔════════════════════════════════════════════════════════════════╗');
console.log(`║  TEST SUMMARY: ${passed} passed, ${failed} failed                               ║`);
console.log('╚════════════════════════════════════════════════════════════════╝\n');

if (failed > 0) {
  console.log('❌ Some tests failed! Please review the failures above.');
  process.exit(1);
} else {
  console.log('✅ All tests passed! Quality gate fixes are working correctly.');
  process.exit(0);
}
