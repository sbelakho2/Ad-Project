
import crypto from 'crypto';

/**
 * DEEP SYSTEM SIMULATION v2: Advanced Failure Modes
 * 
 * This script simulates the advanced architectural risks identified:
 * 1. ML Feature Misalignment (Research vs Production)
 * 2. Quota-Based Fraud Poisoning (TabPFN Contamination)
 * 3. SDK Integration Gap (Experiment Data Loss)
 * 4. Audit Transparency Theater (Forgeable Weak Proofs)
 */

// --- 1. ML FEATURE MISALIGNMENT ---

function simulateMlFeatureMisalignment() {
  console.log('\n--- Scenario 1: ML Feature Misalignment ---');

  const researchFeatures = ['hour_of_day', 'ip_burstiness', 'entropy', 'geo_velocity', 'ctr_anomaly'];
  const productionFeatures = ['impressions_24h', 'clicks_24h'];

  function runOnnxInference(input: number[]) {
    // Model expects researchFeatures.length (5)
    if (input.length !== researchFeatures.length) {
      throw new Error(`ShapeMismatch: Expected ${researchFeatures.length}, got ${input.length}`);
    }
    return 0.99; // Fraud detected
  }

  function productionInference(input: any) {
    const prodInput = [input.impressions_24h, input.clicks_24h];
    try {
      return runOnnxInference(prodInput);
    } catch (e: any) {
      console.log(`[Prod] ONNX Failed: ${e.message}. Falling back to heuristic...`);
      // 4-line Heuristic from services/inference-ml/main.py
      const clickRatio = input.clicks_24h / Math.max(1, input.impressions_24h);
      return 0.2 + clickRatio * 0.6; // Primitive heuristic
    }
  }

  const sample = { impressions_24h: 100, clicks_24h: 80 }; // Highly suspicious activity
  const score = productionInference(sample);

  console.log('Resultant Fraud Score:', score);
  if (score < 0.8) {
    console.log('❌ MISALIGNMENT CONFIRMED: Complex model failed; primitive heuristic under-estimated risk.');
  }
}

// --- 2. QUOTA-BASED FRAUD POISONING ---

function simulateFraudPoisoning() {
  console.log('\n--- Scenario 2: Quota-Based Fraud Poisoning (Contamination) ---');

  const contamination = 0.05; // 5% forced
  const datasetSize = 1000;
  const scores = Array.from({ length: datasetSize }, () => Math.random()); // All random (legit) traffic

  // TabPFN pseudolabeling logic from train_tabpfn_pseudolabels.py
  const threshold = scores.sort((a, b) => a - b)[Math.floor(datasetSize * (1 - contamination))];
  const fraudLabels = scores.filter(s => s >= threshold).length;

  console.log(`Dataset: 100% Legit Traffic. Forced Contamination: ${contamination * 100}%`);
  console.log(`Number of Legit Users labeled as "Fraud": ${fraudLabels}`);

  if (fraudLabels > 0) {
    console.log('❌ QUALITY RISK CONFIRMED: Quota-based labeling ensures 5% false-positive rate even on pure traffic.');
  }
}

// --- 3. SDK INTEGRATION GAP ---

function simulateSdkIntegrationGap() {
  console.log('\n--- Scenario 3: SDK Integration Gap (Data Loss) ---');

  const backendExpects = ['experiment_id', 'arm', 'mode'];
  const sdkSends = ['publisher_id', 'placement_id', 'ad_format']; // Actual fields found in AuctionClient.kt

  console.log('Backend expects migration signals:', backendExpects);
  console.log('SDK actually sends:', sdkSends);

  const isGap = backendExpects.some(field => !sdkSends.includes(field));
  if (isGap) {
    console.log('❌ INTEGRATION GAP CONFIRMED: Backend is running blind; no experiment data being collected.');
  }
}

// --- 4. FORGEABLE WEAK PROOFS ---

function simulateForgeableProofs() {
  console.log('\n--- Scenario 4: Forgeable Weak Proofs (Audit Theater) ---');

  const requestId = 'req-secret-123';
  const receiptHash = null; // Missing from network

  // ProofsIssuer.ts logic: fallback to sha256(request_id)
  const weakProof = crypto.createHash('sha256').update(requestId).digest('hex');

  console.log('Request ID (Platform Controlled):', requestId);
  console.log('Weak Proof (Forgeable):', weakProof);

  // Foresight: Platform can generate a request_id that hashes to a specific target if they want to manipulate the audit.
  console.log('⚠️  AUDIT THEATER CONFIRMED: Weak proofs are derived from platform-controlled values, not network-signed receipts.');
}

async function run() {
  simulateMlFeatureMisalignment();
  simulateFraudPoisoning();
  simulateSdkIntegrationGap();
  simulateForgeableProofs();
}

run().catch(console.error);
