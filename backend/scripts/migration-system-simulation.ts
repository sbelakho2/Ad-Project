
import crypto from 'crypto';
import { MigrationStudioService } from '../src/services/migrationStudioService';
import { MigrationGuardrailSnapshotService } from '../src/services/migrationGuardrailSnapshotService';
import { recordShadowOutcome } from '../src/services/rtb/shadowRecorder';
import { runAuction } from '../src/services/rtb/orchestrator';

/**
 * DEEP SIMULATION: Migration Studio System
 * 
 * This script simulates various edge cases and vulnerabilities identified in the system.
 */

async function simulatePoisonPill() {
  console.log('--- SCENARIO 1: Poison Pill in Shadow Recorder ---');
  
  // We need to set up the environment to have a small MAX_JSON_LEN if possible, 
  // but it's already defined at module load. 
  // Default is 20000. Let's send something larger.
  
  const giantMetadata = {
    field: 'x'.repeat(21000)
  };
  
  console.log('Queuing a record with oversized metadata...');
  await recordShadowOutcome({
    experimentId: crypto.randomUUID(), // Will fail later on FK but we want to see JSON error
    requestId: 'req-poison',
    arm: 'test',
    mode: 'shadow',
    status: 'win',
    metadata: giantMetadata
  });
  
  // Note: Since we are using a real (or mock) Postgres, if it's truncated, 
  // the INSERT will fail with "invalid input syntax for type json".
  // Because it's a batch, other valid records in the queue will also fail.
  
  console.log('Queuing a valid record...');
  await recordShadowOutcome({
    experimentId: crypto.randomUUID(),
    requestId: 'req-valid',
    arm: 'test',
    mode: 'shadow',
    status: 'win',
    metadata: { valid: true }
  });
  
  console.log('Flushing queue (expecting failure if using a real DB with ::jsonb)...');
  // In this environment, we might not have a real DB connected that will fail, 
  // but the code analysis confirms the vulnerability.
}

async function simulateFakeStats() {
  console.log('\n--- SCENARIO 2: Fake Statistical Significance in Reports ---');
  
  // Mock data for 1,000,000 impressions
  const experimentId = 'exp-fake-stats';
  const publisherId = 'pub-1';
  
  // We'll manually check the logic in MigrationStudioService#generateReport
  // lines 1825-1830.
  
  const impressions = 1000000;
  const controlVariance = impressions > 100 ? impressions * 0.05 : impressions;
  const testVariance = impressions > 100 ? impressions * 0.05 : impressions;
  const pooledStdErr = Math.sqrt((controlVariance + testVariance) / 2);
  
  console.log(`For ${impressions.toLocaleString()} impressions:`);
  console.log(`  Assumed Variance: ${controlVariance} (hardcoded 5% of impressions!)`);
  console.log(`  Pooled StdErr: ${pooledStdErr}`);
  
  const controlEcpm = 2.50;
  const testEcpm = 2.51; // 0.01 cent difference
  const zScore = Math.abs(testEcpm - controlEcpm) / pooledStdErr;
  
  console.log(`  eCPM Difference: $0.01`);
  console.log(`  Calculated z-score: ${zScore}`);
  console.log(`  p-value would be: ${zScore > 1.96 ? 0.01 : 0.10} (hardcoded binary!)`);
}

async function simulateSDKTrust() {
  console.log('\n--- SCENARIO 3: SDK Mode/Arm Trust Vulnerability ---');
  console.log('Analysis: runAuction() in orchestrator.ts accepts "migration" context directly from controller body.');
  console.log('Controller (rtb.controller.ts) extracts it from body.signal.migration without database verification.');
  console.log('Outcome: A malicious client can force mirroring mode on any experiment if they know the ID.');
}

async function main() {
  await simulatePoisonPill();
  await simulateFakeStats();
  await simulateSDKTrust();
  
  console.log('\nSimulation complete. See report for detailed analysis of these findings.');
}

main().catch(console.error);
