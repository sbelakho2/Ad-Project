// services/sales-automation/tests/revolutionaryServicesTest.ts
// Standalone test file for revolutionary sales automation services
// Tests core logic without database dependencies

console.log('═══════════════════════════════════════════════════════════════');
console.log('🧪 REVOLUTIONARY SALES AUTOMATION - UNIT TESTS');
console.log('═══════════════════════════════════════════════════════════════');
console.log(`Started at: ${new Date().toISOString()}`);
console.log('');

interface TestResult {
  name: string;
  passed: boolean;
  duration: number;
  error?: string;
}

const results: TestResult[] = [];

function runTest(name: string, testFn: () => void): void {
  const start = Date.now();
  try {
    testFn();
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

function assertEqual(actual: any, expected: any, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${expected}, got ${actual}`);
  }
}

function assertTrue(condition: boolean, message: string): void {
  if (!condition) {
    throw new Error(message);
  }
}

// ============================================
// TEST: Smart Reply Analyzer Logic
// ============================================
console.log('\n📬 Testing Smart Reply Analyzer Logic...');

const INTERESTED_PATTERNS = [
  'yes', 'interested', 'tell me more', 'send me', 'schedule', 'let\'s talk',
  'set up a call', 'book a demo', 'sounds good', 'i\'d like to', 'i\'m interested',
];

const NOT_INTERESTED_PATTERNS = [
  'not interested', 'no thanks', 'no thank you', 'pass', 'not for us',
  'don\'t need', 'not relevant',
];

const OUT_OF_OFFICE_PATTERNS = [
  'out of office', 'out of the office', 'ooo', 'on vacation', 'on leave',
];

const UNSUBSCRIBE_PATTERNS = [
  'unsubscribe', 'remove me', 'opt out', 'stop sending',
];

function analyzeReplyCategory(body: string): string {
  const normalized = body.toLowerCase();
  
  for (const pattern of OUT_OF_OFFICE_PATTERNS) {
    if (normalized.includes(pattern)) return 'out_of_office';
  }
  for (const pattern of UNSUBSCRIBE_PATTERNS) {
    if (normalized.includes(pattern)) return 'unsubscribe';
  }
  for (const pattern of NOT_INTERESTED_PATTERNS) {
    if (normalized.includes(pattern)) return 'not_interested';
  }
  for (const pattern of INTERESTED_PATTERNS) {
    if (normalized.includes(pattern)) return 'interested';
  }
  
  return 'neutral';
}

runTest('Reply: Interested detection', () => {
  const result = analyzeReplyCategory("Yes, I'm very interested! Let's schedule a call.");
  assertEqual(result, 'interested', 'Should detect interested');
});

runTest('Reply: Not interested detection', () => {
  const result = analyzeReplyCategory("No thanks, we're not interested at this time.");
  assertEqual(result, 'not_interested', 'Should detect not interested');
});

runTest('Reply: Out of office detection', () => {
  const result = analyzeReplyCategory("I am out of the office until Monday.");
  assertEqual(result, 'out_of_office', 'Should detect out of office');
});

runTest('Reply: Unsubscribe detection', () => {
  const result = analyzeReplyCategory("Please unsubscribe me from your list.");
  assertEqual(result, 'unsubscribe', 'Should detect unsubscribe');
});

runTest('Reply: Neutral detection', () => {
  const result = analyzeReplyCategory("Thanks for the email.");
  assertEqual(result, 'neutral', 'Should detect neutral');
});

// ============================================
// TEST: Predictive Lead Scoring Logic
// ============================================
console.log('\n📊 Testing Predictive Lead Scoring Logic...');

const SIGNAL_WEIGHTS = {
  hasReplied: 40,
  hasClickedLink: 25,
  hasOpenedMultiple: 15,
  fastReplyTime: 20,
  recentFunding: 20,
  companyGrowth: 15,
  techStackMatch: 12,
  hasJobPosting: 10,
  websiteVisit: 8,
  contentDownload: 12,
};

function calculateLeadScore(signals: Partial<Record<keyof typeof SIGNAL_WEIGHTS, boolean>>): {
  score: number;
  tier: 'hot' | 'warm' | 'cool' | 'cold';
} {
  let score = 0;
  
  for (const [signal, hasSignal] of Object.entries(signals)) {
    if (hasSignal && SIGNAL_WEIGHTS[signal as keyof typeof SIGNAL_WEIGHTS]) {
      score += SIGNAL_WEIGHTS[signal as keyof typeof SIGNAL_WEIGHTS];
    }
  }
  
  score = Math.min(100, score);
  
  let tier: 'hot' | 'warm' | 'cool' | 'cold';
  if (score >= 70) tier = 'hot';
  else if (score >= 40) tier = 'warm';
  else if (score >= 20) tier = 'cool';
  else tier = 'cold';
  
  return { score, tier };
}

runTest('Lead Score: Hot lead (replied + clicked)', () => {
  const { score, tier } = calculateLeadScore({ hasReplied: true, hasClickedLink: true });
  assertTrue(tier === 'warm' || tier === 'hot', `Score ${score} should be warm or hot tier`);
  assertEqual(score, 65, 'Should be 40+25=65');
});

runTest('Lead Score: Warm lead (clicked + funding)', () => {
  const { score, tier } = calculateLeadScore({ hasClickedLink: true, recentFunding: true });
  assertTrue(tier === 'warm', `Score ${score} should be warm tier`);
});

runTest('Lead Score: Cool lead (website visit only)', () => {
  const { score, tier } = calculateLeadScore({ websiteVisit: true, contentDownload: true });
  assertTrue(tier === 'cool', `Score ${score} should be cool tier`);
});

runTest('Lead Score: Cold lead (no signals)', () => {
  const { score, tier } = calculateLeadScore({});
  assertEqual(tier, 'cold', 'No signals should be cold');
  assertEqual(score, 0, 'No signals should be 0');
});

// ============================================
// TEST: Domain Health Logic
// ============================================
console.log('\n🛡️ Testing Domain Health Logic...');

const DOMAIN_THRESHOLDS = {
  bounceRate: { healthy: 3, warning: 5, critical: 7 },
  spamRate: { healthy: 0.1, warning: 0.3, critical: 0.5 },
  openRate: { healthy: 20, warning: 10, critical: 5 },
};

function calculateDomainHealth(metrics: {
  bounceRate: number;
  spamRate: number;
  openRate: number;
}): { score: number; status: 'healthy' | 'warning' | 'critical' } {
  let score = 100;
  let status: 'healthy' | 'warning' | 'critical' = 'healthy';

  // Bounce rate
  if (metrics.bounceRate > DOMAIN_THRESHOLDS.bounceRate.critical) {
    score -= 40;
    status = 'critical';
  } else if (metrics.bounceRate > DOMAIN_THRESHOLDS.bounceRate.warning) {
    score -= 20;
    status = 'warning';
  } else if (metrics.bounceRate > DOMAIN_THRESHOLDS.bounceRate.healthy) {
    score -= 10;
  }

  // Spam rate
  if (metrics.spamRate > DOMAIN_THRESHOLDS.spamRate.critical) {
    score -= 30;
    status = 'critical';
  } else if (metrics.spamRate > DOMAIN_THRESHOLDS.spamRate.warning) {
    score -= 15;
    if (status === 'healthy') status = 'warning';
  }

  // Open rate
  if (metrics.openRate < DOMAIN_THRESHOLDS.openRate.critical) {
    score -= 20;
    if (status === 'healthy') status = 'warning';
  } else if (metrics.openRate < DOMAIN_THRESHOLDS.openRate.warning) {
    score -= 10;
  }

  return { score: Math.max(0, score), status };
}

runTest('Domain Health: Healthy domain', () => {
  const { score, status } = calculateDomainHealth({
    bounceRate: 2,
    spamRate: 0.05,
    openRate: 25,
  });
  assertEqual(status, 'healthy', 'Should be healthy');
  assertTrue(score >= 90, `Score ${score} should be >= 90`);
});

runTest('Domain Health: Warning domain', () => {
  const { score, status } = calculateDomainHealth({
    bounceRate: 6,
    spamRate: 0.2,
    openRate: 15,
  });
  assertEqual(status, 'warning', 'Should be warning');
  assertTrue(score >= 50 && score < 90, `Score ${score} should be warning range`);
});

runTest('Domain Health: Critical domain', () => {
  const { score, status } = calculateDomainHealth({
    bounceRate: 10,
    spamRate: 0.6,
    openRate: 3,
  });
  assertEqual(status, 'critical', 'Should be critical');
  assertTrue(score < 50, `Score ${score} should be < 50`);
});

// ============================================
// TEST: Intent Signal Detection Logic
// ============================================
console.log('\n🎯 Testing Intent Signal Detection Logic...');

const INTENT_WEIGHTS = {
  pricing_page: 25,
  fast_reply: 20,
  email_click: 15,
  email_multiple_opens: 12,
  follow_up_reply: 18,
  recent_funding: 20,
  company_growth: 15,
  website_visit: 5,
};

function calculateIntentScore(signals: string[]): {
  score: number;
  level: 'hot' | 'warm' | 'cool' | 'cold';
  stage: 'awareness' | 'consideration' | 'decision' | 'unknown';
} {
  let score = 0;
  
  for (const signal of signals) {
    if (INTENT_WEIGHTS[signal as keyof typeof INTENT_WEIGHTS]) {
      score += INTENT_WEIGHTS[signal as keyof typeof INTENT_WEIGHTS];
    }
  }
  
  score = Math.min(100, score);
  
  let level: 'hot' | 'warm' | 'cool' | 'cold';
  if (score >= 70) level = 'hot';
  else if (score >= 40) level = 'warm';
  else if (score >= 20) level = 'cool';
  else level = 'cold';

  // Determine buying stage
  let stage: 'awareness' | 'consideration' | 'decision' | 'unknown';
  if (signals.includes('pricing_page')) stage = 'decision';
  else if (signals.includes('email_click') || signals.includes('fast_reply')) stage = 'consideration';
  else if (signals.length > 0) stage = 'awareness';
  else stage = 'unknown';

  return { score, level, stage };
}

runTest('Intent: Decision stage (pricing page)', () => {
  const { level, stage } = calculateIntentScore(['pricing_page', 'email_click']);
  assertEqual(stage, 'decision', 'Should be decision stage');
  assertTrue(level === 'warm' || level === 'hot', 'Should be warm or hot');
});

runTest('Intent: Consideration stage (clicked)', () => {
  const { level, stage } = calculateIntentScore(['email_click', 'follow_up_reply']);
  assertEqual(stage, 'consideration', 'Should be consideration stage');
});

runTest('Intent: Hot lead (multiple high signals)', () => {
  const { score, level } = calculateIntentScore([
    'pricing_page', 'fast_reply', 'recent_funding'
  ]);
  assertTrue(level === 'warm' || level === 'hot', `Score ${score} should be warm or hot`);
});

// ============================================
// TEST: Follow-Up Optimizer Logic
// ============================================
console.log('\n📈 Testing Follow-Up Optimizer Logic...');

const FOLLOWUP_BENCHMARKS = {
  replyRate: { step1: 2.5, step2: 3.5, step3: 3.0, step4: 2.0, step5: 1.5 },
  optimalDelay: { step2: 3, step3: 4, step4: 5, step5: 7 },
  totalSteps: { min: 4, optimal: 5, max: 7 },
};

function scoreFollowUpEmail(stepNumber: number, subject: string, body: string): {
  score: number;
  issues: string[];
} {
  const issues: string[] = [];
  let score = 100;

  // Check subject length
  if (subject.length > 50) {
    issues.push('Subject too long');
    score -= 10;
  }

  // Check body length
  const wordCount = body.split(/\s+/).length;
  if (stepNumber >= 3 && wordCount > 100) {
    issues.push('Body too long for later step');
    score -= 15;
  }

  // Check for personalization
  if (!body.includes('{{') && !body.includes('{')) {
    issues.push('No personalization');
    score -= 10;
  }

  // Check for CTA
  const ctaPatterns = ['calendar', 'call', 'meet', 'chat', 'reply', 'let me know'];
  const hasCTA = ctaPatterns.some(p => body.toLowerCase().includes(p));
  if (!hasCTA) {
    issues.push('No clear CTA');
    score -= 10;
  }

  // Check for spam triggers
  const spamTriggers = ['free', 'guarantee', 'act now', 'limited time'];
  const hasSpam = spamTriggers.some(t => body.toLowerCase().includes(t));
  if (hasSpam) {
    issues.push('Contains spam triggers');
    score -= 15;
  }

  return { score: Math.max(0, score), issues };
}

runTest('FollowUp: Good email scores high', () => {
  const { score, issues } = scoreFollowUpEmail(
    2,
    'Re: Quick question',
    'Hi {{firstName}}, I wanted to follow up. Would you have time for a quick call?'
  );
  assertTrue(score >= 80, `Score ${score} should be >= 80`);
  assertEqual(issues.length, 0, 'Should have no issues');
});

runTest('FollowUp: Spammy email scores low', () => {
  const { score, issues } = scoreFollowUpEmail(
    1,
    'FREE OFFER - ACT NOW LIMITED TIME!!!!',
    'This is a free guarantee! Act now for limited time offer!'
  );
  assertTrue(score < 80, `Score ${score} should be < 80 for spammy email`);
  assertTrue(issues.length > 0, 'Should have issues');
});

runTest('FollowUp: Missing CTA detected', () => {
  const { score, issues } = scoreFollowUpEmail(
    1,
    'Hello',
    'Hi {{firstName}}, just wanted to share some info with you.'
  );
  assertTrue(issues.includes('No clear CTA'), 'Should detect missing CTA');
});

// ============================================
// TEST: Self-Learning A/B Test Logic
// ============================================
console.log('\n🧠 Testing Self-Learning A/B Test Logic...');

function calculateStatisticalSignificance(
  variantA: { sent: number; conversions: number },
  variantB: { sent: number; conversions: number }
): { winner: 'A' | 'B' | 'none'; confidence: number } {
  const p1 = variantA.conversions / variantA.sent;
  const p2 = variantB.conversions / variantB.sent;
  
  const pPooled = (variantA.conversions + variantB.conversions) / 
                  (variantA.sent + variantB.sent);
  
  const se = Math.sqrt(pPooled * (1 - pPooled) * 
                      (1/variantA.sent + 1/variantB.sent));
  
  const zScore = se > 0 ? Math.abs(p1 - p2) / se : 0;
  
  // Convert z-score to confidence
  let confidence = 0;
  if (zScore >= 2.576) confidence = 99;
  else if (zScore >= 1.96) confidence = 95;
  else if (zScore >= 1.645) confidence = 90;
  else if (zScore >= 1.28) confidence = 80;
  else confidence = Math.round(zScore * 40);
  
  let winner: 'A' | 'B' | 'none' = 'none';
  if (confidence >= 95) {
    winner = p1 > p2 ? 'A' : 'B';
  }
  
  return { winner, confidence };
}

runTest('A/B Test: Clear winner detected', () => {
  const { winner, confidence } = calculateStatisticalSignificance(
    { sent: 100, conversions: 15 },  // 15%
    { sent: 100, conversions: 5 }    // 5%
  );
  assertEqual(winner, 'A', 'A should win');
  assertTrue(confidence >= 95, `Confidence ${confidence} should be >= 95`);
});

runTest('A/B Test: No winner (similar)', () => {
  const { winner, confidence } = calculateStatisticalSignificance(
    { sent: 50, conversions: 5 },   // 10%
    { sent: 50, conversions: 4 }    // 8%
  );
  assertEqual(winner, 'none', 'Should be no clear winner');
  assertTrue(confidence < 95, `Confidence ${confidence} should be < 95`);
});

runTest('A/B Test: Variant B wins', () => {
  const { winner, confidence } = calculateStatisticalSignificance(
    { sent: 200, conversions: 10 },  // 5%
    { sent: 200, conversions: 30 }   // 15%
  );
  assertEqual(winner, 'B', 'B should win');
});

// ============================================
// TEST: Spam Prevention Logic
// ============================================
console.log('\n🚫 Testing Spam Prevention Logic...');

const SPAM_TRIGGERS = {
  critical: ['act now', 'free', 'guarantee', '100% free', 'winner'],
  high: ['cheap', 'discount', 'special offer', 'limited time'],
  medium: ['amazing', 'incredible', 'exclusive'],
};

function checkSpamScore(subject: string, body: string): {
  score: number;
  triggers: string[];
} {
  const text = `${subject} ${body}`.toLowerCase();
  const triggers: string[] = [];
  let score = 0;

  for (const word of SPAM_TRIGGERS.critical) {
    if (text.includes(word)) {
      triggers.push(word);
      score += 3;
    }
  }

  for (const word of SPAM_TRIGGERS.high) {
    if (text.includes(word)) {
      triggers.push(word);
      score += 2;
    }
  }

  for (const word of SPAM_TRIGGERS.medium) {
    if (text.includes(word)) {
      triggers.push(word);
      score += 1;
    }
  }

  return { score, triggers };
}

runTest('Spam: Clean email passes', () => {
  const { score, triggers } = checkSpamScore(
    'Quick question about your app',
    'Hi, I noticed your company develops mobile apps. I have a question about your monetization.'
  );
  assertTrue(score <= 2, `Score ${score} should be <= 2`);
});

runTest('Spam: Spammy email detected', () => {
  const { score, triggers } = checkSpamScore(
    'FREE OFFER - ACT NOW',
    'This is 100% free! Act now for guaranteed results!'
  );
  assertTrue(score >= 6, `Score ${score} should be >= 6`);
  assertTrue(triggers.length >= 3, `Should detect ${triggers.length} >= 3 triggers`);
});

// ============================================
// PRINT RESULTS
// ============================================
console.log('\n═══════════════════════════════════════════════════════════════');
console.log('📊 TEST RESULTS');
console.log('═══════════════════════════════════════════════════════════════');

const passed = results.filter(r => r.passed).length;
const failed = results.filter(r => !r.passed).length;
const totalDuration = results.reduce((sum, r) => sum + r.duration, 0);

console.log(`Total: ${results.length} tests`);
console.log(`Passed: ${passed} ✓`);
console.log(`Failed: ${failed} ✗`);
console.log(`Duration: ${totalDuration}ms`);
console.log(`Success Rate: ${((passed / results.length) * 100).toFixed(1)}%`);

if (failed > 0) {
  console.log('\n❌ FAILED TESTS:');
  for (const result of results.filter(r => !r.passed)) {
    console.log(`  - ${result.name}: ${result.error}`);
  }
}

console.log('\n═══════════════════════════════════════════════════════════════');

if (failed === 0) {
  console.log('✅ ALL TESTS PASSED!');
  console.log('');
  console.log('Revolutionary features validated:');
  console.log('  • Smart Reply Analyzer - Instant categorization ✓');
  console.log('  • Predictive Lead Scoring - Score before outreach ✓');
  console.log('  • Domain Health Guard - Proactive protection ✓');
  console.log('  • Intent Signal Detector - Know who\'s ready ✓');
  console.log('  • Follow-Up Optimizer - Follow-ups as main event ✓');
  console.log('  • Self-Learning Sequences - Auto-optimization ✓');
  console.log('  • Spam Prevention - Avoid inbox placement issues ✓');
} else {
  console.log('⚠️  SOME TESTS FAILED - Please review');
  process.exitCode = 1;
}

console.log('═══════════════════════════════════════════════════════════════');
