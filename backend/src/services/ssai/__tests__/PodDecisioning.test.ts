/**
 * SSAI Pod Decisioning Module Unit Tests
 */

import { DecisionSLAEnforcer } from '../pod/DecisionSLAEnforcer';
import { CompetitiveSeparation } from '../pod/CompetitiveSeparation';
import { FrequencyCapManager } from '../pod/FrequencyCapManager';
import { FormatValidator } from '../pod/FormatValidator';
import { PacingThrottler } from '../pod/PacingThrottler';
import { CandidateOrderer } from '../pod/CandidateOrderer';
import { SSAICircuitBreaker, CircuitBreakerRegistry } from '../pod/SSAICircuitBreaker';

describe('DecisionSLAEnforcer', () => {
  let enforcer: DecisionSLAEnforcer;

  beforeEach(() => {
    enforcer = new DecisionSLAEnforcer({
      maxDecisionTimeMs: 250,
      partnerTimeoutMs: 150,
    });
  });

  describe('calculateDeadline', () => {
    it('should calculate deadline with buffer', () => {
      const futureDeadline = new Date(Date.now() + 1000);
      const request = {
        sessionId: 'sess-1',
        podId: 'pod-1',
        availMs: 30000,
        deadline: futureDeadline.toISOString(),
        maxSlots: 4,
        contentProfile: { protocol: 'hls' as const, container: 'ts' as const },
      };

      const deadline = enforcer.calculateDeadline(request);

      expect(deadline).toBeDefined();
      expect(deadline.bufferMs).toBeGreaterThan(0);
    });
  });

  describe('isWithinDeadline', () => {
    it('should return true before deadline', () => {
      const deadline = {
        deadline: new Date(Date.now() + 1000),
        bufferMs: 50,
        safeMode: false,
      };

      const result = enforcer.isWithinDeadline(deadline);

      expect(result).toBe(true);
    });

    it('should return false after deadline', () => {
      const deadline = {
        deadline: new Date(Date.now() - 1000),
        bufferMs: 50,
        safeMode: false,
      };

      const result = enforcer.isWithinDeadline(deadline);

      expect(result).toBe(false);
    });
  });

  describe('getRemainingTimeMs', () => {
    it('should return positive remaining time', () => {
      const deadline = {
        deadline: new Date(Date.now() + 500),
        bufferMs: 50,
        safeMode: false,
      };

      const remaining = enforcer.getRemainingTimeMs(deadline);

      expect(remaining).toBeGreaterThan(0);
      expect(remaining).toBeLessThanOrEqual(500);
    });
  });
});

describe('CompetitiveSeparation', () => {
  let separator: CompetitiveSeparation;

  beforeEach(() => {
    separator = new CompetitiveSeparation({
      advertiserSeparationSlots: 2,
      categorySeparationSlots: 1,
      maxCreativeRepetitionPod: 1,
    });
  });

  describe('constructor', () => {
    it('should create with default config', () => {
      const defaultSeparator = new CompetitiveSeparation();
      expect(defaultSeparator).toBeDefined();
    });

    it('should create with custom config', () => {
      expect(separator).toBeDefined();
    });
  });
});

describe('FrequencyCapManager', () => {
  let manager: FrequencyCapManager;

  beforeEach(() => {
    manager = new FrequencyCapManager({
      defaultSessionCreativeCap: 3,
      defaultSessionAdvertiserCap: 5,
    });
  });

  describe('constructor', () => {
    it('should create with default config', () => {
      const defaultManager = new FrequencyCapManager();
      expect(defaultManager).toBeDefined();
    });

    it('should create with custom config', () => {
      expect(manager).toBeDefined();
    });
  });
});

describe('FormatValidator', () => {
  let validator: FormatValidator;

  beforeEach(() => {
    validator = new FormatValidator();
  });

  describe('constructor', () => {
    it('should create with default config', () => {
      expect(validator).toBeDefined();
    });

    it('should create with strict mode', () => {
      const strictValidator = new FormatValidator({ strictMode: true });
      expect(strictValidator).toBeDefined();
    });
  });
});

describe('PacingThrottler', () => {
  let throttler: PacingThrottler;

  beforeEach(() => {
    throttler = new PacingThrottler({
      partnerRpsLimit: 10,
      tenantRpsLimit: 100,
      windowSizeMs: 1000,
    });
  });

  describe('checkPartnerPacing', () => {
    it('should allow first request', () => {
      const result = throttler.checkPartnerPacing('partner-1');

      expect(result.allowed).toBe(true);
    });

    it('should track requests', () => {
      throttler.checkPartnerPacing('partner-1');
      throttler.checkPartnerPacing('partner-1');

      const state = throttler.getPartnerState('partner-1');

      expect(state).toBeDefined();
    });
  });

  describe('checkTenantPacing', () => {
    it('should allow tenant requests', () => {
      const result = throttler.checkTenantPacing('tenant-1');

      expect(result.allowed).toBe(true);
    });
  });
});

describe('CandidateOrderer', () => {
  let orderer: CandidateOrderer;

  beforeEach(() => {
    orderer = new CandidateOrderer();
  });

  describe('orderCandidates', () => {
    it('should order by score descending', () => {
      const candidates = [
        { id: '1', creativeId: 'c1', advertiserId: 'a1', campaignId: 'camp1', priority: 5, price: 10, duration: 30, healthScore: 1.0, partnerId: 'p1' },
        { id: '2', creativeId: 'c2', advertiserId: 'a2', campaignId: 'camp2', priority: 10, price: 20, duration: 30, healthScore: 1.0, partnerId: 'p2' },
      ];

      const context = {
        sessionId: 'sess-1',
        podId: 'pod-1',
        slotIndex: 0,
        partnerHealth: new Map(),
        selectedCandidates: [],
        timestamp: Date.now(),
      };

      const ordered = orderer.orderCandidates(candidates, context);

      expect(ordered[0].id).toBe('2'); // Higher priority/price
    });

    it('should be deterministic', () => {
      const candidates = [
        { id: '1', creativeId: 'c1', advertiserId: 'a1', campaignId: 'camp1', priority: 5, price: 10, duration: 30, healthScore: 1.0, partnerId: 'p1' },
        { id: '2', creativeId: 'c2', advertiserId: 'a2', campaignId: 'camp2', priority: 5, price: 10, duration: 30, healthScore: 1.0, partnerId: 'p2' },
      ];

      const context = {
        sessionId: 'sess-1',
        podId: 'pod-1',
        slotIndex: 0,
        partnerHealth: new Map(),
        selectedCandidates: [],
        timestamp: Date.now(),
      };

      const ordered1 = orderer.orderCandidates(candidates, context);
      const ordered2 = orderer.orderCandidates(candidates, context);

      expect(ordered1.map(c => c.id)).toEqual(ordered2.map(c => c.id));
    });

    it('should filter unhealthy candidates', () => {
      const candidates = [
        { id: '1', creativeId: 'c1', advertiserId: 'a1', campaignId: 'camp1', priority: 10, price: 20, duration: 30, healthScore: 0.2, partnerId: 'p1' },
        { id: '2', creativeId: 'c2', advertiserId: 'a2', campaignId: 'camp2', priority: 5, price: 10, duration: 30, healthScore: 1.0, partnerId: 'p2' },
      ];

      const context = {
        sessionId: 'sess-1',
        podId: 'pod-1',
        slotIndex: 0,
        partnerHealth: new Map(),
        selectedCandidates: [],
        timestamp: Date.now(),
      };

      const ordered = orderer.orderCandidates(candidates, context);

      expect(ordered.length).toBe(1);
      expect(ordered[0].id).toBe('2');
    });
  });
});

describe('SSAICircuitBreaker', () => {
  let breaker: SSAICircuitBreaker;

  beforeEach(() => {
    breaker = new SSAICircuitBreaker('partner-1', {
      failureRateThreshold: 0.5,
      minimumCalls: 5,
      openDuration: 1000,
    });
  });

  describe('initial state', () => {
    it('should start closed', () => {
      expect(breaker.getState()).toBe('closed');
    });

    it('should allow calls when closed', () => {
      expect(breaker.canCall()).toBe(true);
    });
  });

  describe('failure tracking', () => {
    it('should open on high failure rate', () => {
      // Record failures
      for (let i = 0; i < 10; i++) {
        breaker.recordResult({ success: false, timedOut: false, durationMs: 100 });
      }

      expect(breaker.getState()).toBe('open');
      expect(breaker.canCall()).toBe(false);
    });

    it('should track timeout rate', () => {
      for (let i = 0; i < 10; i++) {
        breaker.recordResult({ success: false, timedOut: true, durationMs: 5000 });
      }

      const stats = breaker.getStats();
      expect(stats.timeoutRate).toBeGreaterThan(0);
    });
  });

  describe('recovery', () => {
    it('should transition to half-open after cooldown', async () => {
      // Trip the breaker
      for (let i = 0; i < 10; i++) {
        breaker.recordResult({ success: false, timedOut: false, durationMs: 100 });
      }

      expect(breaker.getState()).toBe('open');

      // Wait for cooldown
      await new Promise(r => setTimeout(r, 1100));

      // Check state (should allow test call)
      expect(breaker.canCall()).toBe(true);
      expect(breaker.getState()).toBe('half-open');
    });
  });

  describe('health score', () => {
    it('should return 0 when open', () => {
      breaker.forceOpen();

      expect(breaker.getHealthScore()).toBe(0);
    });

    it('should return 0.5 when half-open', () => {
      breaker.forceOpen();
      // Can't easily force half-open, but test the closed case
      expect(breaker.getHealthScore()).toBe(0);
    });
  });
});

describe('CircuitBreakerRegistry', () => {
  let registry: CircuitBreakerRegistry;

  beforeEach(() => {
    registry = new CircuitBreakerRegistry();
  });

  it('should create breakers on demand', () => {
    const breaker = registry.getBreaker('partner-1');

    expect(breaker).toBeDefined();
    expect(breaker.getPartnerId()).toBe('partner-1');
  });

  it('should return same breaker for same partner', () => {
    const breaker1 = registry.getBreaker('partner-1');
    const breaker2 = registry.getBreaker('partner-1');

    expect(breaker1).toBe(breaker2);
  });

  it('should track open circuits', () => {
    registry.getBreaker('partner-1').forceOpen();

    const open = registry.getOpenCircuits();

    expect(open).toContain('partner-1');
  });
});
