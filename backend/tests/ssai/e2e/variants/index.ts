/**
 * SSAI E2E Variant Test Runner Index
 * 
 * Exports all 8 variant test runners
 */

export { TEST_VARIANTS, TestVariant, TestConfig, TestResult, AssertionResult, getVariantConfig } from './types';

export { HLSLiveTSRunner, runHLSLiveTSTests } from './hls-live-ts.test';
export { HLSLiveCMAFRunner, runHLSLiveCMAFTests } from './hls-live-cmaf.test';
export { HLSVODTSRunner, runHLSVODTSTests } from './hls-vod-ts.test';
export { HLSVODCMAFRunner, runHLSVODCMAFTests } from './hls-vod-cmaf.test';
export { DASHLiveTSRunner, runDASHLiveTSTests } from './dash-live-ts.test';
export { DASHLiveCMAFRunner, runDASHLiveCMAFTests } from './dash-live-cmaf.test';
export { DASHVODTSRunner, runDASHVODTSTests } from './dash-vod-ts.test';
export { DASHVODCMAFRunner, runDASHVODCMAFTests } from './dash-vod-cmaf.test';

import { TestResult, TEST_VARIANTS } from './types';
import { runHLSLiveTSTests } from './hls-live-ts.test';
import { runHLSLiveCMAFTests } from './hls-live-cmaf.test';
import { runHLSVODTSTests } from './hls-vod-ts.test';
import { runHLSVODCMAFTests } from './hls-vod-cmaf.test';
import { runDASHLiveTSTests } from './dash-live-ts.test';
import { runDASHLiveCMAFTests } from './dash-live-cmaf.test';
import { runDASHVODTSTests } from './dash-vod-ts.test';
import { runDASHVODCMAFTests } from './dash-vod-cmaf.test';

/**
 * Run all 8 variant tests
 */
export async function runAllVariantTests(): Promise<TestResult[]> {
  const results: TestResult[] = [];

  console.log('Running all 8 SSAI E2E variant tests...\n');

  // Run all tests in sequence
  const runners = [
    { name: 'HLS-Live-TS', run: runHLSLiveTSTests },
    { name: 'HLS-Live-CMAF', run: runHLSLiveCMAFTests },
    { name: 'HLS-VOD-TS', run: runHLSVODTSTests },
    { name: 'HLS-VOD-CMAF', run: runHLSVODCMAFTests },
    { name: 'DASH-Live-TS', run: runDASHLiveTSTests },
    { name: 'DASH-Live-CMAF', run: runDASHLiveCMAFTests },
    { name: 'DASH-VOD-TS', run: runDASHVODTSTests },
    { name: 'DASH-VOD-CMAF', run: runDASHVODCMAFTests },
  ];

  for (const { name, run } of runners) {
    console.log(`Running ${name}...`);
    const result = await run();
    results.push(result);
    
    const status = result.passed ? '✅ PASSED' : '❌ FAILED';
    const assertions = `${result.assertions.filter(a => a.passed).length}/${result.assertions.length} assertions`;
    console.log(`  ${status} (${assertions}, ${result.duration}ms)`);
    
    if (result.errors.length > 0) {
      console.log(`  Errors: ${result.errors.join(', ')}`);
    }
  }

  // Summary
  console.log('\n========== SUMMARY ==========');
  const passed = results.filter(r => r.passed).length;
  const failed = results.length - passed;
  console.log(`Passed: ${passed}/${results.length}`);
  console.log(`Failed: ${failed}/${results.length}`);
  
  const totalAssertions = results.reduce((sum, r) => sum + r.assertions.length, 0);
  const passedAssertions = results.reduce((sum, r) => sum + r.assertions.filter(a => a.passed).length, 0);
  console.log(`Assertions: ${passedAssertions}/${totalAssertions}`);
  
  const totalTime = results.reduce((sum, r) => sum + r.duration, 0);
  console.log(`Total time: ${totalTime}ms`);

  return results;
}

/**
 * Run a specific variant by name
 */
export async function runVariantByName(variantName: string): Promise<TestResult | null> {
  const runnerMap: Record<string, () => Promise<TestResult>> = {
    'HLS-Live-TS': runHLSLiveTSTests,
    'HLS-Live-CMAF': runHLSLiveCMAFTests,
    'HLS-VOD-TS': runHLSVODTSTests,
    'HLS-VOD-CMAF': runHLSVODCMAFTests,
    'DASH-Live-TS': runDASHLiveTSTests,
    'DASH-Live-CMAF': runDASHLiveCMAFTests,
    'DASH-VOD-TS': runDASHVODTSTests,
    'DASH-VOD-CMAF': runDASHVODCMAFTests,
  };

  const runner = runnerMap[variantName];
  if (!runner) {
    console.error(`Unknown variant: ${variantName}`);
    console.log(`Available variants: ${Object.keys(runnerMap).join(', ')}`);
    return null;
  }

  return runner();
}

/**
 * Get all available variant names
 */
export function getAvailableVariants(): string[] {
  return TEST_VARIANTS.map(v => v.name);
}
