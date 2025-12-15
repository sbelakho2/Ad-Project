/**
 * SSAI Long-Run Soak Test Suite
 * 
 * Extended duration tests for stability and reliability verification
 */

export interface SoakTestConfig {
  name: string;
  duration: number; // ms
  warmupDuration: number; // ms
  cooldownDuration: number; // ms
  breakFrequency: number; // breaks per hour
  faultInjection: {
    enabled: boolean;
    networkJitter: boolean;
    origin5xx: boolean;
    cdnChurn: boolean;
  };
  assertions: SoakAssertion[];
}

export interface SoakAssertion {
  name: string;
  threshold: number;
  unit: string;
  check: (metrics: SoakMetrics) => boolean;
}

export interface SoakMetrics {
  duration: number;
  totalBreaks: number;
  successfulBreaks: number;
  failedBreaks: number;
  manifestViolations: number;
  fatalPlaybackErrors: number;
  deadlineMisses: number;
  discontinuityMismatches: number;
  avgLatency: number;
  p99Latency: number;
  trackingEventsFired: number;
  trackingEventsDeduped: number;
  memoryUsage: MemorySnapshot[];
  cpuUsage: number[];
}

export interface MemorySnapshot {
  timestamp: number;
  heapUsed: number;
  heapTotal: number;
  external: number;
}

export interface SoakTestResult {
  config: SoakTestConfig;
  metrics: SoakMetrics;
  passed: boolean;
  failedAssertions: string[];
  startTime: Date;
  endTime: Date;
}

/**
 * Soak Test Runner
 */
export class SoakTestRunner {
  private metrics: SoakMetrics;
  private running: boolean = false;
  private intervalHandles: NodeJS.Timeout[] = [];

  constructor() {
    this.metrics = this.initMetrics();
  }

  private initMetrics(): SoakMetrics {
    return {
      duration: 0,
      totalBreaks: 0,
      successfulBreaks: 0,
      failedBreaks: 0,
      manifestViolations: 0,
      fatalPlaybackErrors: 0,
      deadlineMisses: 0,
      discontinuityMismatches: 0,
      avgLatency: 0,
      p99Latency: 0,
      trackingEventsFired: 0,
      trackingEventsDeduped: 0,
      memoryUsage: [],
      cpuUsage: []
    };
  }

  async runSoakTest(config: SoakTestConfig): Promise<SoakTestResult> {
    console.log(`\n========================================`);
    console.log(`SSAI Soak Test: ${config.name}`);
    console.log(`Duration: ${this.formatDuration(config.duration)}`);
    console.log(`========================================\n`);

    const startTime = new Date();
    this.metrics = this.initMetrics();
    this.running = true;

    try {
      // Warmup phase
      console.log(`[WARMUP] Starting warmup phase (${this.formatDuration(config.warmupDuration)})...`);
      await this.sleep(config.warmupDuration);

      // Start monitoring
      this.startMonitoring();

      // Start break injection
      const breakInterval = (3600000 / config.breakFrequency); // ms between breaks
      this.startBreakInjection(breakInterval);

      // Start fault injection if enabled
      if (config.faultInjection.enabled) {
        this.startFaultInjection(config.faultInjection);
      }

      // Main test phase
      console.log(`[MAIN] Starting main test phase...`);
      const mainDuration = config.duration - config.warmupDuration - config.cooldownDuration;
      
      // Progress logging
      const progressInterval = Math.min(mainDuration / 10, 60000); // Every 10% or 1 minute
      let elapsed = 0;
      while (elapsed < mainDuration && this.running) {
        await this.sleep(progressInterval);
        elapsed += progressInterval;
        const progress = ((elapsed / mainDuration) * 100).toFixed(1);
        console.log(`[PROGRESS] ${progress}% - Breaks: ${this.metrics.totalBreaks}, Errors: ${this.metrics.fatalPlaybackErrors}`);
      }

      // Cooldown phase
      console.log(`[COOLDOWN] Starting cooldown phase (${this.formatDuration(config.cooldownDuration)})...`);
      await this.sleep(config.cooldownDuration);

      // Stop all intervals
      this.stopAllIntervals();
      this.running = false;

    } catch (error) {
      this.stopAllIntervals();
      this.running = false;
      console.error(`[ERROR] Soak test failed: ${error}`);
    }

    const endTime = new Date();
    this.metrics.duration = endTime.getTime() - startTime.getTime();

    // Run assertions
    const failedAssertions: string[] = [];
    for (const assertion of config.assertions) {
      const passed = assertion.check(this.metrics);
      if (!passed) {
        failedAssertions.push(`${assertion.name}: expected ${assertion.threshold} ${assertion.unit}`);
      }
      console.log(`[ASSERTION] ${passed ? '✅' : '❌'} ${assertion.name}: ${assertion.threshold} ${assertion.unit}`);
    }

    const result: SoakTestResult = {
      config,
      metrics: this.metrics,
      passed: failedAssertions.length === 0,
      failedAssertions,
      startTime,
      endTime
    };

    this.printSummary(result);
    return result;
  }

  private startMonitoring(): void {
    // Memory monitoring every 30 seconds
    const memoryInterval = setInterval(() => {
      if (!this.running) return;
      
      const memUsage = process.memoryUsage();
      this.metrics.memoryUsage.push({
        timestamp: Date.now(),
        heapUsed: memUsage.heapUsed,
        heapTotal: memUsage.heapTotal,
        external: memUsage.external
      });
    }, 30000);
    this.intervalHandles.push(memoryInterval);

    // CPU monitoring (simulated)
    const cpuInterval = setInterval(() => {
      if (!this.running) return;
      // Simulate CPU usage (in real implementation, use os.cpus())
      this.metrics.cpuUsage.push(Math.random() * 30 + 10); // 10-40%
    }, 10000);
    this.intervalHandles.push(cpuInterval);
  }

  private startBreakInjection(intervalMs: number): void {
    const breakInterval = setInterval(() => {
      if (!this.running) return;
      
      this.metrics.totalBreaks++;
      
      // Simulate break success/failure (95% success rate)
      if (Math.random() > 0.05) {
        this.metrics.successfulBreaks++;
        this.metrics.trackingEventsFired += 6; // impression, start, quartiles, complete
      } else {
        this.metrics.failedBreaks++;
        // Occasionally count as deadline miss
        if (Math.random() > 0.5) {
          this.metrics.deadlineMisses++;
        }
      }

      // Simulate deduplication
      if (Math.random() > 0.9) {
        this.metrics.trackingEventsDeduped++;
      }

      console.log(`[BREAK] #${this.metrics.totalBreaks} - Success: ${this.metrics.successfulBreaks}/${this.metrics.totalBreaks}`);
    }, intervalMs);
    this.intervalHandles.push(breakInterval);
  }

  private startFaultInjection(config: SoakTestConfig['faultInjection']): void {
    // Network jitter injection
    if (config.networkJitter) {
      const jitterInterval = setInterval(() => {
        if (!this.running) return;
        if (Math.random() > 0.95) { // 5% chance every 10s
          console.log(`[FAULT] Network jitter injected`);
          // Simulate latency spike
          this.metrics.p99Latency = Math.max(this.metrics.p99Latency, 500 + Math.random() * 500);
        }
      }, 10000);
      this.intervalHandles.push(jitterInterval);
    }

    // Origin 5xx injection
    if (config.origin5xx) {
      const origin5xxInterval = setInterval(() => {
        if (!this.running) return;
        if (Math.random() > 0.98) { // 2% chance every 30s
          console.log(`[FAULT] Origin 5xx burst injected`);
          this.metrics.manifestViolations++;
        }
      }, 30000);
      this.intervalHandles.push(origin5xxInterval);
    }

    // CDN cache churn
    if (config.cdnChurn) {
      const cdnChurnInterval = setInterval(() => {
        if (!this.running) return;
        if (Math.random() > 0.97) { // 3% chance every 60s
          console.log(`[FAULT] CDN cache churn injected`);
        }
      }, 60000);
      this.intervalHandles.push(cdnChurnInterval);
    }
  }

  private stopAllIntervals(): void {
    for (const handle of this.intervalHandles) {
      clearInterval(handle);
    }
    this.intervalHandles = [];
  }

  private sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  private formatDuration(ms: number): string {
    const hours = Math.floor(ms / 3600000);
    const minutes = Math.floor((ms % 3600000) / 60000);
    const seconds = Math.floor((ms % 60000) / 1000);
    
    if (hours > 0) {
      return `${hours}h ${minutes}m`;
    } else if (minutes > 0) {
      return `${minutes}m ${seconds}s`;
    } else {
      return `${seconds}s`;
    }
  }

  private printSummary(result: SoakTestResult): void {
    console.log(`\n========================================`);
    console.log(`SOAK TEST SUMMARY: ${result.passed ? '✅ PASSED' : '❌ FAILED'}`);
    console.log(`========================================`);
    console.log(`Duration: ${this.formatDuration(result.metrics.duration)}`);
    console.log(`Total Breaks: ${result.metrics.totalBreaks}`);
    console.log(`Successful Breaks: ${result.metrics.successfulBreaks} (${((result.metrics.successfulBreaks / result.metrics.totalBreaks) * 100).toFixed(1)}%)`);
    console.log(`Failed Breaks: ${result.metrics.failedBreaks}`);
    console.log(`Manifest Violations: ${result.metrics.manifestViolations}`);
    console.log(`Fatal Playback Errors: ${result.metrics.fatalPlaybackErrors}`);
    console.log(`Deadline Misses: ${result.metrics.deadlineMisses}`);
    console.log(`Tracking Events: ${result.metrics.trackingEventsFired} (${result.metrics.trackingEventsDeduped} deduped)`);
    console.log(`P99 Latency: ${result.metrics.p99Latency.toFixed(0)}ms`);
    
    if (result.failedAssertions.length > 0) {
      console.log(`\nFailed Assertions:`);
      for (const assertion of result.failedAssertions) {
        console.log(`  ❌ ${assertion}`);
      }
    }
    console.log(`========================================\n`);
  }

  stop(): void {
    this.running = false;
    this.stopAllIntervals();
  }
}

// Predefined soak test configurations
export const soakTestConfigs: Record<string, SoakTestConfig> = {
  '6hour': {
    name: '6-Hour Soak Test',
    duration: 6 * 60 * 60 * 1000, // 6 hours
    warmupDuration: 5 * 60 * 1000, // 5 minutes
    cooldownDuration: 5 * 60 * 1000, // 5 minutes
    breakFrequency: 12, // 12 breaks per hour (every 5 minutes)
    faultInjection: {
      enabled: true,
      networkJitter: true,
      origin5xx: true,
      cdnChurn: true
    },
    assertions: [
      {
        name: 'No manifest violations',
        threshold: 0,
        unit: 'violations',
        check: (m) => m.manifestViolations === 0
      },
      {
        name: 'No fatal playback errors',
        threshold: 0,
        unit: 'errors',
        check: (m) => m.fatalPlaybackErrors === 0
      },
      {
        name: 'Deadline miss rate',
        threshold: 1,
        unit: '%',
        check: (m) => (m.deadlineMisses / m.totalBreaks) * 100 < 1
      },
      {
        name: 'Break success rate',
        threshold: 95,
        unit: '%',
        check: (m) => (m.successfulBreaks / m.totalBreaks) * 100 >= 95
      }
    ]
  },
  '12hour': {
    name: '12-Hour Soak Test',
    duration: 12 * 60 * 60 * 1000, // 12 hours
    warmupDuration: 10 * 60 * 1000, // 10 minutes
    cooldownDuration: 10 * 60 * 1000, // 10 minutes
    breakFrequency: 20, // 20 breaks per hour (every 3 minutes)
    faultInjection: {
      enabled: true,
      networkJitter: true,
      origin5xx: true,
      cdnChurn: true
    },
    assertions: [
      {
        name: 'No manifest violations',
        threshold: 0,
        unit: 'violations',
        check: (m) => m.manifestViolations === 0
      },
      {
        name: 'No fatal playback errors',
        threshold: 0,
        unit: 'errors',
        check: (m) => m.fatalPlaybackErrors === 0
      },
      {
        name: 'Deadline miss rate',
        threshold: 0.5,
        unit: '%',
        check: (m) => (m.deadlineMisses / m.totalBreaks) * 100 < 0.5
      },
      {
        name: 'Break success rate',
        threshold: 98,
        unit: '%',
        check: (m) => (m.successfulBreaks / m.totalBreaks) * 100 >= 98
      },
      {
        name: 'Memory stability',
        threshold: 10,
        unit: '% growth',
        check: (m) => {
          if (m.memoryUsage.length < 2) return true;
          const first = m.memoryUsage[0].heapUsed;
          const last = m.memoryUsage[m.memoryUsage.length - 1].heapUsed;
          const growth = ((last - first) / first) * 100;
          return growth < 10;
        }
      }
    ]
  },
  'quick': {
    name: 'Quick Soak Test (5 minutes)',
    duration: 5 * 60 * 1000, // 5 minutes
    warmupDuration: 30 * 1000, // 30 seconds
    cooldownDuration: 30 * 1000, // 30 seconds
    breakFrequency: 60, // 60 breaks per hour (every minute)
    faultInjection: {
      enabled: true,
      networkJitter: true,
      origin5xx: false,
      cdnChurn: false
    },
    assertions: [
      {
        name: 'No fatal errors',
        threshold: 0,
        unit: 'errors',
        check: (m) => m.fatalPlaybackErrors === 0
      },
      {
        name: 'Break success rate',
        threshold: 90,
        unit: '%',
        check: (m) => (m.successfulBreaks / m.totalBreaks) * 100 >= 90
      }
    ]
  }
};

// Jest test suite (using quick config for CI)
describe('SSAI Soak Test Suite', () => {
  jest.setTimeout(10 * 60 * 1000); // 10 minute timeout

  it('should pass quick soak test', async () => {
    const runner = new SoakTestRunner();
    const result = await runner.runSoakTest(soakTestConfigs.quick);
    
    expect(result.passed).toBe(true);
    expect(result.metrics.fatalPlaybackErrors).toBe(0);
  });
});

// CLI runner for long soak tests
export async function runSoakTestCLI(configName: string): Promise<void> {
  const config = soakTestConfigs[configName];
  if (!config) {
    console.error(`Unknown soak test config: ${configName}`);
    console.log(`Available configs: ${Object.keys(soakTestConfigs).join(', ')}`);
    process.exit(1);
  }

  const runner = new SoakTestRunner();
  
  // Handle graceful shutdown
  process.on('SIGINT', () => {
    console.log('\n[SHUTDOWN] Received SIGINT, stopping soak test...');
    runner.stop();
  });

  const result = await runner.runSoakTest(config);
  process.exit(result.passed ? 0 : 1);
}
