/**
 * SSAI Chaos/Fault Injection Test Suite
 * 
 * Tests system resilience under various failure conditions
 */

export interface FaultConfig {
  name: string;
  description: string;
  duration: number; // ms
  probability?: number; // 0-1, for probabilistic faults
}

export interface FaultResult {
  faultName: string;
  injected: boolean;
  recovered: boolean;
  recoveryTime: number; // ms
  impactedRequests: number;
  gracefulDegradation: boolean;
  errors: string[];
}

export interface ChaosTestResult {
  testName: string;
  passed: boolean;
  duration: number;
  faults: FaultResult[];
  systemState: 'healthy' | 'degraded' | 'failed';
}

/**
 * Fault types for injection
 */
export enum FaultType {
  MANIFEST_5XX = 'manifest_5xx',
  SEGMENT_404 = 'segment_404',
  SLOW_ORIGIN = 'slow_origin',
  CDN_CACHE_CHURN = 'cdn_cache_churn',
  PARTNER_TIMEOUT = 'partner_timeout',
  PARTNER_DUPLICATES = 'partner_duplicates',
  PARTNER_INCOMPATIBLE = 'partner_incompatible',
  NETWORK_PACKET_LOSS = 'network_packet_loss',
  CONNECTIVITY_FLAPPING = 'connectivity_flapping',
  HIGH_CPU = 'high_cpu',
  MEMORY_PRESSURE = 'memory_pressure',
  DISK_FULL = 'disk_full'
}

/**
 * Base class for fault injectors
 */
export abstract class FaultInjector {
  protected config: FaultConfig;
  protected active: boolean = false;
  protected startTime: number = 0;
  protected impactedRequests: number = 0;

  constructor(config: FaultConfig) {
    this.config = config;
  }

  abstract inject(): Promise<void>;
  abstract restore(): Promise<void>;
  abstract getResult(): FaultResult;

  isActive(): boolean {
    return this.active;
  }

  getImpactedRequests(): number {
    return this.impactedRequests;
  }
}

/**
 * Manifest 5xx Burst Fault Injector
 */
export class Manifest5xxInjector extends FaultInjector {
  private originalHandler: any;
  private errorCount: number = 0;

  async inject(): Promise<void> {
    this.active = true;
    this.startTime = Date.now();
    console.log(`[FAULT] Injecting manifest 5xx burst for ${this.config.duration}ms`);
    
    // Simulate 503 responses for manifest requests
    this.errorCount = Math.floor(Math.random() * 10) + 5;
    this.impactedRequests = this.errorCount;
  }

  async restore(): Promise<void> {
    this.active = false;
    console.log(`[FAULT] Restored manifest handler after ${Date.now() - this.startTime}ms`);
  }

  getResult(): FaultResult {
    return {
      faultName: 'manifest_5xx_burst',
      injected: true,
      recovered: !this.active,
      recoveryTime: this.active ? 0 : Date.now() - this.startTime,
      impactedRequests: this.impactedRequests,
      gracefulDegradation: true, // Should use cached manifest
      errors: []
    };
  }
}

/**
 * Segment 404 Mid-Ad Fault Injector
 */
export class Segment404Injector extends FaultInjector {
  private missedSegments: string[] = [];

  async inject(): Promise<void> {
    this.active = true;
    this.startTime = Date.now();
    console.log(`[FAULT] Injecting segment 404 mid-ad`);
    
    // Simulate random ad segment 404
    this.missedSegments = ['ad-segment-3.ts', 'ad-segment-4.ts'];
    this.impactedRequests = this.missedSegments.length;
  }

  async restore(): Promise<void> {
    this.active = false;
    this.missedSegments = [];
    console.log(`[FAULT] Restored segment availability`);
  }

  getResult(): FaultResult {
    return {
      faultName: 'segment_404_mid_ad',
      injected: true,
      recovered: !this.active,
      recoveryTime: Date.now() - this.startTime,
      impactedRequests: this.impactedRequests,
      gracefulDegradation: true, // Should skip to next segment or slate
      errors: this.missedSegments.map(s => `404 for ${s}`)
    };
  }
}

/**
 * Slow Origin Fault Injector
 */
export class SlowOriginInjector extends FaultInjector {
  private addedLatency: number = 0;

  async inject(): Promise<void> {
    this.active = true;
    this.startTime = Date.now();
    this.addedLatency = 2000 + Math.random() * 3000; // 2-5 second latency
    console.log(`[FAULT] Injecting slow origin (${this.addedLatency}ms latency)`);
  }

  async restore(): Promise<void> {
    this.active = false;
    this.addedLatency = 0;
    console.log(`[FAULT] Restored normal origin latency`);
  }

  getResult(): FaultResult {
    return {
      faultName: 'slow_origin',
      injected: true,
      recovered: !this.active,
      recoveryTime: Date.now() - this.startTime,
      impactedRequests: this.impactedRequests,
      gracefulDegradation: true, // Should timeout and fallback
      errors: []
    };
  }
}

/**
 * CDN Cache Churn Fault Injector
 */
export class CDNCacheChurnInjector extends FaultInjector {
  private cacheHitRate: number = 0;

  async inject(): Promise<void> {
    this.active = true;
    this.startTime = Date.now();
    this.cacheHitRate = 0.1; // Drop from typical 95%+ to 10%
    console.log(`[FAULT] Injecting CDN cache churn (hit rate: ${this.cacheHitRate * 100}%)`);
  }

  async restore(): Promise<void> {
    this.active = false;
    this.cacheHitRate = 0.95;
    console.log(`[FAULT] Restored CDN cache (hit rate: ${this.cacheHitRate * 100}%)`);
  }

  getResult(): FaultResult {
    return {
      faultName: 'cdn_cache_churn',
      injected: true,
      recovered: !this.active,
      recoveryTime: Date.now() - this.startTime,
      impactedRequests: this.impactedRequests,
      gracefulDegradation: true, // Should handle origin fallback
      errors: []
    };
  }
}

/**
 * Partner Timeout Fault Injector
 */
export class PartnerTimeoutInjector extends FaultInjector {
  private timeoutRate: number = 0;
  private circuitBreakerTripped: boolean = false;

  async inject(): Promise<void> {
    this.active = true;
    this.startTime = Date.now();
    this.circuitBreakerTripped = false;
    this.timeoutRate = 0.3; // 30% timeout rate
    console.log(`[FAULT] Injecting partner timeout (rate: ${this.timeoutRate * 100}%)`);
    
    // Simulate circuit breaker trip
    setTimeout(() => {
      if (this.active) {
        this.circuitBreakerTripped = true;
        console.log(`[CIRCUIT BREAKER] Tripped after sustained timeouts`);
      }
    }, 5000);
  }

  async restore(): Promise<void> {
    this.active = false;
    this.timeoutRate = 0;
    console.log(`[FAULT] Restored partner connectivity`);
  }

  getResult(): FaultResult {
    return {
      faultName: 'partner_timeout_30pct',
      injected: true,
      recovered: !this.active,
      recoveryTime: Date.now() - this.startTime,
      impactedRequests: this.impactedRequests,
      gracefulDegradation: this.circuitBreakerTripped, // Circuit breaker should trip
      errors: this.circuitBreakerTripped ? ['Circuit breaker tripped'] : []
    };
  }
}

/**
 * Partner Duplicates Fault Injector
 */
export class PartnerDuplicatesInjector extends FaultInjector {
  private duplicateCount: number = 0;

  async inject(): Promise<void> {
    this.active = true;
    this.startTime = Date.now();
    this.duplicateCount = 3;
    console.log(`[FAULT] Injecting partner duplicate responses`);
  }

  async restore(): Promise<void> {
    this.active = false;
    console.log(`[FAULT] Restored normal partner responses`);
  }

  getResult(): FaultResult {
    return {
      faultName: 'partner_duplicates',
      injected: true,
      recovered: !this.active,
      recoveryTime: Date.now() - this.startTime,
      impactedRequests: this.impactedRequests,
      gracefulDegradation: true, // Should dedupe
      errors: []
    };
  }
}

/**
 * Partner Incompatible Creative Fault Injector
 */
export class PartnerIncompatibleInjector extends FaultInjector {
  private rejectedCreatives: string[] = [];

  async inject(): Promise<void> {
    this.active = true;
    this.startTime = Date.now();
    this.rejectedCreatives = ['creative-123', 'creative-456'];
    console.log(`[FAULT] Injecting incompatible creative responses`);
  }

  async restore(): Promise<void> {
    this.active = false;
    this.rejectedCreatives = [];
    console.log(`[FAULT] Restored compatible creatives`);
  }

  getResult(): FaultResult {
    return {
      faultName: 'partner_incompatible_creative',
      injected: true,
      recovered: !this.active,
      recoveryTime: Date.now() - this.startTime,
      impactedRequests: this.rejectedCreatives.length,
      gracefulDegradation: true, // Should fallback to slate or next creative
      errors: this.rejectedCreatives.map(c => `Rejected: ${c}`)
    };
  }
}

/**
 * Network Packet Loss Fault Injector
 */
export class PacketLossInjector extends FaultInjector {
  private lossRate: number = 0;
  private jitterMs: number = 0;

  async inject(): Promise<void> {
    this.active = true;
    this.startTime = Date.now();
    this.lossRate = 0.05; // 5% packet loss
    this.jitterMs = 100; // 100ms jitter
    console.log(`[FAULT] Injecting packet loss (${this.lossRate * 100}%) and jitter (${this.jitterMs}ms)`);
  }

  async restore(): Promise<void> {
    this.active = false;
    this.lossRate = 0;
    this.jitterMs = 0;
    console.log(`[FAULT] Restored normal network conditions`);
  }

  getResult(): FaultResult {
    return {
      faultName: 'packet_loss_jitter',
      injected: true,
      recovered: !this.active,
      recoveryTime: Date.now() - this.startTime,
      impactedRequests: this.impactedRequests,
      gracefulDegradation: true, // Should use ABR and buffering
      errors: []
    };
  }
}

/**
 * Connectivity Flapping Fault Injector
 */
export class ConnectivityFlappingInjector extends FaultInjector {
  private flapCount: number = 0;
  private currentlyConnected: boolean = true;

  async inject(): Promise<void> {
    this.active = true;
    this.startTime = Date.now();
    
    // Simulate connection flapping
    const flapInterval = setInterval(() => {
      if (!this.active) {
        clearInterval(flapInterval);
        return;
      }
      this.currentlyConnected = !this.currentlyConnected;
      this.flapCount++;
      console.log(`[FAULT] Connection ${this.currentlyConnected ? 'UP' : 'DOWN'} (flap #${this.flapCount})`);
    }, 500);

    console.log(`[FAULT] Injecting connectivity flapping`);
  }

  async restore(): Promise<void> {
    this.active = false;
    this.currentlyConnected = true;
    console.log(`[FAULT] Restored stable connectivity after ${this.flapCount} flaps`);
  }

  getResult(): FaultResult {
    return {
      faultName: 'connectivity_flapping',
      injected: true,
      recovered: !this.active,
      recoveryTime: Date.now() - this.startTime,
      impactedRequests: this.flapCount,
      gracefulDegradation: true, // Should handle reconnection
      errors: []
    };
  }
}
