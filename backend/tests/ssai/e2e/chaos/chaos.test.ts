/**
 * SSAI Chaos Test Suite
 * 
 * Comprehensive fault injection and resilience testing
 */

import {
  FaultInjector,
  FaultResult,
  ChaosTestResult,
  Manifest5xxInjector,
  Segment404Injector,
  SlowOriginInjector,
  CDNCacheChurnInjector,
  PartnerTimeoutInjector,
  PartnerDuplicatesInjector,
  PartnerIncompatibleInjector,
  PacketLossInjector,
  ConnectivityFlappingInjector
} from './fault-injectors';

interface ChaosTestConfig {
  name: string;
  injectors: FaultInjector[];
  duration: number; // ms
  assertions: ChaosAssertion[];
}

interface ChaosAssertion {
  name: string;
  check: (results: FaultResult[]) => boolean;
  message: string;
}

/**
 * Chaos Test Runner
 */
export class ChaosTestRunner {
  private results: ChaosTestResult[] = [];

  async runTest(config: ChaosTestConfig): Promise<ChaosTestResult> {
    console.log(`\n=== Running Chaos Test: ${config.name} ===`);
    const startTime = Date.now();
    const faultResults: FaultResult[] = [];

    try {
      // Inject all faults
      for (const injector of config.injectors) {
        await injector.inject();
      }

      // Let faults run for duration
      await this.sleep(config.duration);

      // Restore all faults
      for (const injector of config.injectors) {
        await injector.restore();
        faultResults.push(injector.getResult());
      }

      // Run assertions
      const assertionsPassed = config.assertions.every(assertion => {
        const passed = assertion.check(faultResults);
        console.log(`  [${passed ? '✓' : '✗'}] ${assertion.name}: ${assertion.message}`);
        return passed;
      });

      const result: ChaosTestResult = {
        testName: config.name,
        passed: assertionsPassed,
        duration: Date.now() - startTime,
        faults: faultResults,
        systemState: assertionsPassed ? 'healthy' : 'degraded'
      };

      this.results.push(result);
      return result;
    } catch (error) {
      const result: ChaosTestResult = {
        testName: config.name,
        passed: false,
        duration: Date.now() - startTime,
        faults: faultResults,
        systemState: 'failed'
      };
      this.results.push(result);
      return result;
    }
  }

  private sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  getResults(): ChaosTestResult[] {
    return this.results;
  }
}

// Define all chaos test configurations
const chaosTests: ChaosTestConfig[] = [
  {
    name: 'Manifest 5xx Burst',
    injectors: [
      new Manifest5xxInjector({
        name: 'manifest_5xx',
        description: 'Simulate origin returning 503 for manifests',
        duration: 5000
      })
    ],
    duration: 5000,
    assertions: [
      {
        name: 'recovery',
        check: (results) => results.every(r => r.recovered),
        message: 'System should recover from manifest errors'
      },
      {
        name: 'graceful_degradation',
        check: (results) => results.every(r => r.gracefulDegradation),
        message: 'Should use cached manifests during outage'
      }
    ]
  },
  {
    name: 'Segment 404 Mid-Ad',
    injectors: [
      new Segment404Injector({
        name: 'segment_404',
        description: 'Simulate ad segment not found mid-playback',
        duration: 3000
      })
    ],
    duration: 3000,
    assertions: [
      {
        name: 'recovery',
        check: (results) => results.every(r => r.recovered),
        message: 'System should recover from missing segments'
      },
      {
        name: 'graceful_degradation',
        check: (results) => results.every(r => r.gracefulDegradation),
        message: 'Should skip to next segment or use slate'
      }
    ]
  },
  {
    name: 'Slow Origin (High RTT)',
    injectors: [
      new SlowOriginInjector({
        name: 'slow_origin',
        description: 'Simulate high latency to origin',
        duration: 10000
      })
    ],
    duration: 10000,
    assertions: [
      {
        name: 'recovery',
        check: (results) => results.every(r => r.recovered),
        message: 'System should recover when latency normalizes'
      },
      {
        name: 'timeout_handling',
        check: (results) => results.every(r => r.gracefulDegradation),
        message: 'Should timeout and fallback appropriately'
      }
    ]
  },
  {
    name: 'CDN Cache Churn',
    injectors: [
      new CDNCacheChurnInjector({
        name: 'cdn_churn',
        description: 'Simulate CDN cache invalidation',
        duration: 8000
      })
    ],
    duration: 8000,
    assertions: [
      {
        name: 'origin_fallback',
        check: (results) => results.every(r => r.gracefulDegradation),
        message: 'Should fallback to origin on cache miss'
      }
    ]
  },
  {
    name: 'Partner Timeout 30%',
    injectors: [
      new PartnerTimeoutInjector({
        name: 'partner_timeout',
        description: 'Simulate 30% partner timeout rate',
        duration: 15000
      })
    ],
    duration: 15000,
    assertions: [
      {
        name: 'circuit_breaker',
        check: (results) => results.every(r => r.gracefulDegradation),
        message: 'Circuit breaker should trip on sustained timeouts'
      },
      {
        name: 'recovery',
        check: (results) => results.every(r => r.recovered),
        message: 'System should recover after partner stabilizes'
      }
    ]
  },
  {
    name: 'Partner Duplicates',
    injectors: [
      new PartnerDuplicatesInjector({
        name: 'partner_duplicates',
        description: 'Simulate partner returning duplicate ads',
        duration: 5000
      })
    ],
    duration: 5000,
    assertions: [
      {
        name: 'deduplication',
        check: (results) => results.every(r => r.gracefulDegradation),
        message: 'Should deduplicate partner responses'
      }
    ]
  },
  {
    name: 'Partner Incompatible Creative',
    injectors: [
      new PartnerIncompatibleInjector({
        name: 'partner_incompatible',
        description: 'Simulate partner returning incompatible creatives',
        duration: 5000
      })
    ],
    duration: 5000,
    assertions: [
      {
        name: 'creative_rejection',
        check: (results) => results.every(r => r.gracefulDegradation),
        message: 'Should reject incompatible creatives and fallback'
      }
    ]
  },
  {
    name: 'Packet Loss + Jitter',
    injectors: [
      new PacketLossInjector({
        name: 'packet_loss',
        description: 'Simulate network degradation',
        duration: 10000
      })
    ],
    duration: 10000,
    assertions: [
      {
        name: 'abr_adaptation',
        check: (results) => results.every(r => r.gracefulDegradation),
        message: 'Should adapt bitrate and use buffering'
      }
    ]
  },
  {
    name: 'Connectivity Flapping',
    injectors: [
      new ConnectivityFlappingInjector({
        name: 'connectivity_flapping',
        description: 'Simulate unstable network connectivity',
        duration: 10000
      })
    ],
    duration: 10000,
    assertions: [
      {
        name: 'reconnection',
        check: (results) => results.every(r => r.gracefulDegradation),
        message: 'Should handle reconnection gracefully'
      },
      {
        name: 'recovery',
        check: (results) => results.every(r => r.recovered),
        message: 'Should recover when connection stabilizes'
      }
    ]
  }
];

// Jest test suite
describe('SSAI Chaos Test Suite', () => {
  const runner = new ChaosTestRunner();

  describe('Manifest Fault Tests', () => {
    it('should handle manifest 5xx burst', async () => {
      const result = await runner.runTest(chaosTests[0]);
      expect(result.passed).toBe(true);
      expect(result.systemState).toBe('healthy');
    }, 30000);
  });

  describe('Segment Fault Tests', () => {
    it('should handle segment 404 mid-ad', async () => {
      const result = await runner.runTest(chaosTests[1]);
      expect(result.passed).toBe(true);
      expect(result.faults[0].gracefulDegradation).toBe(true);
    }, 30000);
  });

  describe('Origin Fault Tests', () => {
    it('should handle slow origin', async () => {
      const result = await runner.runTest(chaosTests[2]);
      expect(result.passed).toBe(true);
    }, 30000);

    it('should handle CDN cache churn', async () => {
      const result = await runner.runTest(chaosTests[3]);
      expect(result.passed).toBe(true);
    }, 30000);
  });

  describe('Partner Fault Tests', () => {
    it('should handle partner timeout 30%', async () => {
      const result = await runner.runTest(chaosTests[4]);
      expect(result.passed).toBe(true);
      expect(result.faults[0].gracefulDegradation).toBe(true);
    }, 30000);

    it('should handle partner duplicates', async () => {
      const result = await runner.runTest(chaosTests[5]);
      expect(result.passed).toBe(true);
    }, 30000);

    it('should handle partner incompatible creative', async () => {
      const result = await runner.runTest(chaosTests[6]);
      expect(result.passed).toBe(true);
    }, 30000);
  });

  describe('Network Fault Tests', () => {
    it('should handle packet loss and jitter', async () => {
      const result = await runner.runTest(chaosTests[7]);
      expect(result.passed).toBe(true);
    }, 30000);

    it('should handle connectivity flapping', async () => {
      const result = await runner.runTest(chaosTests[8]);
      expect(result.passed).toBe(true);
    }, 30000);
  });
});

// Export for external use
export { ChaosTestRunner, chaosTests };
export type { ChaosTestConfig, ChaosAssertion };
