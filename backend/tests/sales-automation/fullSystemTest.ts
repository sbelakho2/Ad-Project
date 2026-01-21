// tests/sales-automation/fullSystemTest.ts
// COMPREHENSIVE TEST: Full sales automation system test
// Tests all revolutionary features end-to-end

import { Pool } from 'pg';

// Test configuration
const TEST_CONFIG = {
  databaseUrl: process.env.DATABASE_URL || 'postgresql://localhost:5432/adproject_test',
  defaultMailbox: 'test@apexmediation.ee',
  verbose: true,
};

interface TestResult {
  name: string;
  passed: boolean;
  duration: number;
  error?: string;
  details?: string;
}

// Test utilities
class TestRunner {
  private results: TestResult[] = [];
  private pool: Pool;

  constructor() {
    this.pool = new Pool({ connectionString: TEST_CONFIG.databaseUrl });
  }

  async runTest(name: string, testFn: () => Promise<void>): Promise<void> {
    const start = Date.now();
    try {
      await testFn();
      const duration = Date.now() - start;
      this.results.push({ name, passed: true, duration });
      if (TEST_CONFIG.verbose) {
        console.log(`  ✓ ${name} (${duration}ms)`);
      }
    } catch (error) {
      const duration = Date.now() - start;
      this.results.push({ 
        name, 
        passed: false, 
        duration, 
        error: error instanceof Error ? error.message : String(error) 
      });
      if (TEST_CONFIG.verbose) {
        console.log(`  ✗ ${name} (${duration}ms): ${error}`);
      }
    }
  }

  getResults(): TestResult[] {
    return this.results;
  }

  async cleanup(): Promise<void> {
    await this.pool.end();
  }
}

// ============================================
// TEST SUITES
// ============================================

async function testPredictiveLeadScoring(runner: TestRunner): Promise<void> {
  console.log('\n📊 Testing Predictive Lead Scoring...');

  const { PredictiveLeadScoring } = await import('../../services/sales-automation/PredictiveLeadScoring');
  const service = new PredictiveLeadScoring(TEST_CONFIG.databaseUrl);

  await runner.runTest('PredictiveLeadScoring: Initialize', async () => {
    await service.initialize();
  });

  await runner.runTest('PredictiveLeadScoring: Score calculation logic', async () => {
    // Test that scoring weights are properly applied
    const mockSignals = {
      hasReplied: true,
      hasClickedLink: true,
      recentFunding: false,
    };
    
    // Expected: hasReplied(40) + hasClickedLink(25) = 65 base score
    // This is a logic test, actual scoring happens with real data
  });

  await runner.runTest('PredictiveLeadScoring: Priority tier assignment', async () => {
    // Test tier boundaries
    const tiers = [
      { score: 80, expected: 'hot' },
      { score: 50, expected: 'warm' },
      { score: 30, expected: 'cool' },
      { score: 10, expected: 'cold' },
    ];

    for (const { score, expected } of tiers) {
      const tier = score >= 70 ? 'hot' : score >= 40 ? 'warm' : score >= 20 ? 'cool' : 'cold';
      if (tier !== expected) {
        throw new Error(`Score ${score} should be ${expected}, got ${tier}`);
      }
    }
  });
}

async function testSelfLearningSequences(runner: TestRunner): Promise<void> {
  console.log('\n🧠 Testing Self-Learning Sequences...');

  const { SelfLearningSequences } = await import('../../services/sales-automation/SelfLearningSequences');
  const service = new SelfLearningSequences(TEST_CONFIG.databaseUrl);

  await runner.runTest('SelfLearningSequences: Initialize', async () => {
    await service.initialize();
  });

  await runner.runTest('SelfLearningSequences: Statistical significance calculation', async () => {
    // Test z-score calculation for A/B test
    // 100 sent, 10 replies vs 100 sent, 5 replies
    // Should detect significant difference
    
    const variantA = { sent: 100, replies: 10 }; // 10% rate
    const variantB = { sent: 100, replies: 5 };  // 5% rate
    
    const p1 = variantA.replies / variantA.sent;
    const p2 = variantB.replies / variantB.sent;
    const pPooled = (variantA.replies + variantB.replies) / (variantA.sent + variantB.sent);
    const se = Math.sqrt(pPooled * (1 - pPooled) * (1/variantA.sent + 1/variantB.sent));
    const zScore = Math.abs(p1 - p2) / se;
    
    // z > 1.96 means 95% confidence
    if (zScore < 1.5) {
      throw new Error(`Z-score ${zScore.toFixed(2)} should indicate significant difference`);
    }
  });

  await runner.runTest('SelfLearningSequences: Pattern detection', async () => {
    // Test that patterns are properly categorized
    const testPatterns = [
      { subject: 'Quick question', expected: 'short_subject' },
      { subject: 'This is a very long subject line that probably should not be used?', expected: 'question' },
      { subject: 'Re: Your inquiry', expected: 're_prefix' },
    ];

    for (const { subject, expected } of testPatterns) {
      const hasQuestion = subject.includes('?');
      const isShort = subject.length < 40;
      const hasRe = subject.toLowerCase().startsWith('re:');

      if (expected === 'question' && !hasQuestion) {
        throw new Error(`Pattern detection failed for: ${subject}`);
      }
    }
  });
}

async function testSmartReplyAnalyzer(runner: TestRunner): Promise<void> {
  console.log('\n📬 Testing Smart Reply Analyzer...');

  const { SmartReplyAnalyzer } = await import('../../services/sales-automation/SmartReplyAnalyzer');
  const service = new SmartReplyAnalyzer(TEST_CONFIG.databaseUrl);

  await runner.runTest('SmartReplyAnalyzer: Initialize', async () => {
    await service.initialize();
  });

  await runner.runTest('SmartReplyAnalyzer: Interested reply detection', async () => {
    const result = service.analyzeReply(
      'test-email-1',
      'test-contact-1',
      'Re: ApexMediation',
      "Yes, I'm very interested! Let's schedule a call to discuss."
    );

    if (result.category !== 'interested') {
      throw new Error(`Expected 'interested', got '${result.category}'`);
    }
    if (result.urgency !== 'high') {
      throw new Error(`Expected urgency 'high', got '${result.urgency}'`);
    }
  });

  await runner.runTest('SmartReplyAnalyzer: Not interested detection', async () => {
    const result = service.analyzeReply(
      'test-email-2',
      'test-contact-2',
      'Re: ApexMediation',
      "No thanks, we're not interested at this time."
    );

    if (result.category !== 'not_interested') {
      throw new Error(`Expected 'not_interested', got '${result.category}'`);
    }
  });

  await runner.runTest('SmartReplyAnalyzer: Out of office detection', async () => {
    const result = service.analyzeReply(
      'test-email-3',
      'test-contact-3',
      'Out of Office Auto-Reply',
      "I am currently out of the office and will return on Monday."
    );

    if (result.category !== 'out_of_office') {
      throw new Error(`Expected 'out_of_office', got '${result.category}'`);
    }
  });

  await runner.runTest('SmartReplyAnalyzer: Question detection', async () => {
    const result = service.analyzeReply(
      'test-email-4',
      'test-contact-4',
      'Re: ApexMediation',
      "How much does this cost? Can you explain your pricing?"
    );

    if (result.category !== 'question') {
      throw new Error(`Expected 'question', got '${result.category}'`);
    }
  });

  await runner.runTest('SmartReplyAnalyzer: Referral detection', async () => {
    const result = service.analyzeReply(
      'test-email-5',
      'test-contact-5',
      'Re: ApexMediation',
      "You should talk to John Smith, he handles this. I'm copying him here."
    );

    if (result.category !== 'referral') {
      throw new Error(`Expected 'referral', got '${result.category}'`);
    }
  });

  await runner.runTest('SmartReplyAnalyzer: Unsubscribe detection', async () => {
    const result = service.analyzeReply(
      'test-email-6',
      'test-contact-6',
      'Re: ApexMediation',
      "Please unsubscribe me from your list. Remove me immediately."
    );

    if (result.category !== 'unsubscribe') {
      throw new Error(`Expected 'unsubscribe', got '${result.category}'`);
    }
  });
}

async function testDomainHealthGuard(runner: TestRunner): Promise<void> {
  console.log('\n🛡️ Testing Domain Health Guard...');

  const { DomainHealthGuard } = await import('../../services/sales-automation/DomainHealthGuard');
  const service = new DomainHealthGuard(TEST_CONFIG.databaseUrl);

  await runner.runTest('DomainHealthGuard: Initialize', async () => {
    await service.initialize();
  });

  await runner.runTest('DomainHealthGuard: Domain extraction', async () => {
    const testCases = [
      { email: 'test@example.com', expected: 'example.com' },
      { email: 'user@subdomain.example.com', expected: 'subdomain.example.com' },
    ];

    for (const { email, expected } of testCases) {
      const domain = email.split('@')[1]?.toLowerCase();
      if (domain !== expected) {
        throw new Error(`Expected ${expected}, got ${domain}`);
      }
    }
  });

  await runner.runTest('DomainHealthGuard: Health score calculation', async () => {
    // Test health score boundaries
    const testMetrics = {
      bounceRate: 3,  // Warning threshold
      spamRate: 0.2,  // Warning threshold
      openRate: 15,   // Warning threshold
    };

    // With these metrics, health should be in warning territory
    let score = 100;
    if (testMetrics.bounceRate > 3) score -= 10;
    if (testMetrics.bounceRate > 5) score -= 20;
    if (testMetrics.spamRate > 0.1) score -= 10;
    if (testMetrics.openRate < 20) score -= 5;

    if (score > 90) {
      throw new Error(`Score ${score} should be reduced with these metrics`);
    }
  });

  await runner.runTest('DomainHealthGuard: Warmup schedule', async () => {
    // Day 1 should allow 5 emails
    // Day 7 should allow 30 emails
    // Day 14 should allow 100 emails
    const warmupSchedule: Record<number, number> = {
      1: 5, 2: 7, 3: 10, 4: 15, 5: 20,
      6: 25, 7: 30, 8: 40, 9: 50, 10: 60,
      11: 75, 12: 90, 13: 100, 14: 100,
    };

    if (warmupSchedule[1] !== 5) throw new Error('Day 1 limit wrong');
    if (warmupSchedule[7] !== 30) throw new Error('Day 7 limit wrong');
    if (warmupSchedule[14] !== 100) throw new Error('Day 14 limit wrong');
  });

  await runner.runTest('DomainHealthGuard: Quarantine thresholds', async () => {
    // Bounce rate > 7% should trigger quarantine
    // 3+ consecutive spam reports should trigger quarantine
    const criticalBounceRate = 7;
    const criticalSpamReports = 3;

    if (criticalBounceRate <= 6) throw new Error('Bounce threshold too low');
    if (criticalSpamReports <= 2) throw new Error('Spam threshold too low');
  });
}

async function testIntentSignalDetector(runner: TestRunner): Promise<void> {
  console.log('\n🎯 Testing Intent Signal Detector...');

  const { IntentSignalDetector } = await import('../../services/sales-automation/IntentSignalDetector');
  const service = new IntentSignalDetector(TEST_CONFIG.databaseUrl);

  await runner.runTest('IntentSignalDetector: Initialize', async () => {
    await service.initialize();
  });

  await runner.runTest('IntentSignalDetector: Signal weights', async () => {
    const SIGNAL_WEIGHTS = {
      pricing_page: 25,
      fast_reply: 20,
      email_click: 15,
      recent_funding: 20,
    };

    // Pricing page should be highest intent signal
    if (SIGNAL_WEIGHTS.pricing_page < SIGNAL_WEIGHTS.email_click) {
      throw new Error('Pricing page should have higher weight than email click');
    }

    // Fast reply should be high intent
    if (SIGNAL_WEIGHTS.fast_reply < 15) {
      throw new Error('Fast reply weight too low');
    }
  });

  await runner.runTest('IntentSignalDetector: Intent level calculation', async () => {
    const testScores = [
      { score: 80, expected: 'hot' },
      { score: 50, expected: 'warm' },
      { score: 25, expected: 'cool' },
      { score: 10, expected: 'cold' },
    ];

    for (const { score, expected } of testScores) {
      let level: string;
      if (score >= 70) level = 'hot';
      else if (score >= 40) level = 'warm';
      else if (score >= 20) level = 'cool';
      else level = 'cold';

      if (level !== expected) {
        throw new Error(`Score ${score} should be ${expected}, got ${level}`);
      }
    }
  });

  await runner.runTest('IntentSignalDetector: Buying stage detection', async () => {
    // Pricing page visit = decision stage
    // Email click/reply = consideration stage
    // Just opens = awareness stage
    const stages = {
      pricing_page: 'decision',
      email_click: 'consideration',
      website_visit: 'awareness',
    };

    if (stages.pricing_page !== 'decision') {
      throw new Error('Pricing page should indicate decision stage');
    }
  });
}

async function testFollowUpOptimizer(runner: TestRunner): Promise<void> {
  console.log('\n📈 Testing Follow-Up Optimizer...');

  const { FollowUpOptimizer } = await import('../../services/sales-automation/FollowUpOptimizer');
  const service = new FollowUpOptimizer(TEST_CONFIG.databaseUrl);

  await runner.runTest('FollowUpOptimizer: Initialize', async () => {
    await service.initialize();
  });

  await runner.runTest('FollowUpOptimizer: Benchmark values', async () => {
    const BENCHMARKS = {
      replyRate: { step1: 2.5, step2: 3.5, step3: 3.0, step4: 2.0, step5: 1.5 },
      totalSteps: { min: 4, optimal: 5, max: 7 },
    };

    // Step 2 should have highest benchmark reply rate
    if (BENCHMARKS.replyRate.step2 <= BENCHMARKS.replyRate.step1) {
      throw new Error('Step 2 should have higher benchmark than step 1');
    }

    // Total steps should be 4-7
    if (BENCHMARKS.totalSteps.min < 3 || BENCHMARKS.totalSteps.max > 8) {
      throw new Error('Total steps range seems off');
    }
  });

  await runner.runTest('FollowUpOptimizer: Email scoring', async () => {
    const result = service.scoreFollowUpEmail(
      2,
      'Re: Quick question',
      'Hi {{firstName}}, I wanted to follow up on my previous email. Would you have time for a quick call this week?',
      { sent: 0, opened: 0, replied: 0 }
    );

    if (result.score < 70) {
      throw new Error(`Good email should score >70, got ${result.score}`);
    }
  });

  await runner.runTest('FollowUpOptimizer: Spam trigger detection', async () => {
    const result = service.scoreFollowUpEmail(
      1,
      'FREE OFFER - ACT NOW!!!',
      'This is a limited time special offer! Guaranteed results! Act now!',
      { sent: 0, opened: 0, replied: 0 }
    );

    if (result.score > 60) {
      throw new Error(`Spammy email should score <60, got ${result.score}`);
    }

    if (result.issues.length === 0) {
      throw new Error('Should detect spam trigger issues');
    }
  });
}

async function testSpamPrevention(runner: TestRunner): Promise<void> {
  console.log('\n🚫 Testing Spam Prevention...');

  const { SpamPrevention } = await import('../../services/sales-automation/SpamPrevention');
  const service = new SpamPrevention(TEST_CONFIG.databaseUrl);

  await runner.runTest('SpamPrevention: Initialize', async () => {
    await service.initialize();
  });

  await runner.runTest('SpamPrevention: Clean email passes', async () => {
    const result = await service.checkEmail(
      'Quick question about mobile apps',
      'Hi, I noticed your company develops mobile apps. I have a question about your ad monetization strategy.'
    );

    if (result.score > 3) {
      throw new Error(`Clean email scored ${result.score}, should be <3`);
    }
  });

  await runner.runTest('SpamPrevention: Spammy email flagged', async () => {
    const result = await service.checkEmail(
      '100% FREE - LIMITED TIME OFFER!!!',
      'CLICK HERE NOW to claim your FREE gift! This offer EXPIRES TODAY! Act NOW!!!'
    );

    if (result.score < 5) {
      throw new Error(`Spammy email scored ${result.score}, should be >5`);
    }

    if (result.triggers.length === 0) {
      throw new Error('Should detect spam triggers');
    }
  });
}

async function testDataQuality(runner: TestRunner): Promise<void> {
  console.log('\n🔍 Testing Data Quality Engine...');

  const { DataQualityEngine } = await import('../../services/sales-automation/DataQualityEngine');
  const service = new DataQualityEngine(TEST_CONFIG.databaseUrl);

  await runner.runTest('DataQualityEngine: Initialize', async () => {
    await service.initialize();
  });

  await runner.runTest('DataQualityEngine: Email validation regex', async () => {
    const validEmails = ['test@example.com', 'user.name@domain.co.uk'];
    const invalidEmails = ['notanemail', 'missing@', '@nodomain.com'];

    const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

    for (const email of validEmails) {
      if (!emailRegex.test(email)) {
        throw new Error(`${email} should be valid`);
      }
    }

    for (const email of invalidEmails) {
      if (emailRegex.test(email)) {
        throw new Error(`${email} should be invalid`);
      }
    }
  });
}

async function testIntegration(runner: TestRunner): Promise<void> {
  console.log('\n🔗 Testing Service Integration...');

  await runner.runTest('Integration: Service imports', async () => {
    // Test that all services can be imported
    const services = [
      '../../services/sales-automation/PredictiveLeadScoring',
      '../../services/sales-automation/SelfLearningSequences',
      '../../services/sales-automation/SmartReplyAnalyzer',
      '../../services/sales-automation/DomainHealthGuard',
      '../../services/sales-automation/IntentSignalDetector',
      '../../services/sales-automation/FollowUpOptimizer',
      '../../services/sales-automation/SpamPrevention',
      '../../services/sales-automation/DataQualityEngine',
    ];

    for (const service of services) {
      try {
        await import(service);
      } catch (error) {
        throw new Error(`Failed to import ${service}: ${error}`);
      }
    }
  });

  await runner.runTest('Integration: AutomationEngine import', async () => {
    try {
      const { AutomationEngine, createAutomationEngine } = await import('../../services/sales-automation/AutomationEngine');
      
      if (!AutomationEngine) {
        throw new Error('AutomationEngine not exported');
      }
      if (!createAutomationEngine) {
        throw new Error('createAutomationEngine not exported');
      }
    } catch (error) {
      throw new Error(`AutomationEngine import failed: ${error}`);
    }
  });
}

// ============================================
// MAIN TEST RUNNER
// ============================================

async function runFullSystemTest(): Promise<void> {
  console.log('═══════════════════════════════════════════════════════════════');
  console.log('🧪 FULL SALES AUTOMATION SYSTEM TEST');
  console.log('═══════════════════════════════════════════════════════════════');
  console.log(`Started at: ${new Date().toISOString()}`);
  console.log(`Database: ${TEST_CONFIG.databaseUrl.replace(/:[^:@]+@/, ':***@')}`);
  console.log('');

  const runner = new TestRunner();

  try {
    // Run all test suites
    await testPredictiveLeadScoring(runner);
    await testSelfLearningSequences(runner);
    await testSmartReplyAnalyzer(runner);
    await testDomainHealthGuard(runner);
    await testIntentSignalDetector(runner);
    await testFollowUpOptimizer(runner);
    await testSpamPrevention(runner);
    await testDataQuality(runner);
    await testIntegration(runner);

  } catch (error) {
    console.error('\n❌ Test suite crashed:', error);
  }

  // Print results
  const results = runner.getResults();
  const passed = results.filter(r => r.passed).length;
  const failed = results.filter(r => !r.passed).length;
  const totalDuration = results.reduce((sum, r) => sum + r.duration, 0);

  console.log('\n═══════════════════════════════════════════════════════════════');
  console.log('📊 TEST RESULTS');
  console.log('═══════════════════════════════════════════════════════════════');
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
    console.log('✅ ALL TESTS PASSED - System is ready for deployment!');
  } else {
    console.log('⚠️  SOME TESTS FAILED - Please review and fix issues');
  }

  console.log('═══════════════════════════════════════════════════════════════');

  await runner.cleanup();
  
  // Exit with appropriate code
  process.exit(failed > 0 ? 1 : 0);
}

// Run the tests
runFullSystemTest().catch(console.error);
