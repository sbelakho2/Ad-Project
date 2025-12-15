import { describe, it, expect, beforeEach, jest } from '@jest/globals';
import { KillSwitchManager } from '../../../../src/services/ssai/operability/KillSwitchManager';
import { RolloutManager } from '../../../../src/services/ssai/operability/RolloutManager';
import { SafeModeManager } from '../../../../src/services/ssai/operability/SafeModeManager';
import { SLIManager } from '../../../../src/services/ssai/operability/SLIManager';
import { ExplainService } from '../../../../src/services/ssai/operability/ExplainService';
import { SlateManager } from '../../../../src/services/ssai/operability/SlateManager';

describe('SSAI Operability Module', () => {
  describe('KillSwitchManager', () => {
    let manager: KillSwitchManager;

    beforeEach(() => {
      manager = new KillSwitchManager();
    });

    describe('activate/deactivate', () => {
      it('should activate kill switch', () => {
        manager.activate({
          scope: 'global',
          reason: 'Emergency maintenance',
          action: 'slate',
        });

        expect(manager.isActive('global')).toBe(true);
      });

      it('should deactivate kill switch', () => {
        manager.activate({
          scope: 'global',
          reason: 'Emergency',
          action: 'slate',
        });
        
        manager.deactivate('global');

        expect(manager.isActive('global')).toBe(false);
      });

      it('should support scoped kill switches', () => {
        manager.activate({
          scope: 'tenant',
          scopeId: 'tenant-123',
          reason: 'Tenant issue',
          action: 'slate',
        });

        expect(manager.isActive('tenant', 'tenant-123')).toBe(true);
        expect(manager.isActive('tenant', 'tenant-456')).toBe(false);
        expect(manager.isActive('global')).toBe(false);
      });
    });

    describe('checkRequest', () => {
      it('should block requests when global kill switch is active', () => {
        manager.activate({
          scope: 'global',
          reason: 'Maintenance',
          action: 'slate',
        });

        const result = manager.checkRequest({
          tenantId: 'tenant-123',
          region: 'us-east-1',
        });

        expect(result.blocked).toBe(true);
        expect(result.action).toBe('slate');
      });

      it('should allow requests when no kill switch affects them', () => {
        manager.activate({
          scope: 'tenant',
          scopeId: 'tenant-999',
          reason: 'Issue',
          action: 'slate',
        });

        const result = manager.checkRequest({
          tenantId: 'tenant-123',
          region: 'us-east-1',
        });

        expect(result.blocked).toBe(false);
      });
    });

    describe('auto-restore', () => {
      it('should auto-restore after duration', async () => {
        jest.useFakeTimers();

        manager.activate({
          scope: 'global',
          reason: 'Brief issue',
          action: 'slate',
          autoRestoreAfterMs: 1000,
        });

        expect(manager.isActive('global')).toBe(true);

        jest.advanceTimersByTime(1100);

        expect(manager.isActive('global')).toBe(false);

        jest.useRealTimers();
      });
    });
  });

  describe('RolloutManager', () => {
    let manager: RolloutManager;

    beforeEach(() => {
      manager = new RolloutManager();
    });

    describe('percentage rollout', () => {
      it('should enable feature for percentage of users', () => {
        manager.createRollout({
          featureId: 'new-stitcher',
          strategy: 'percentage',
          percentage: 50,
        });

        // Run multiple checks to verify approximate percentage
        let enabled = 0;
        const iterations = 1000;
        
        for (let i = 0; i < iterations; i++) {
          if (manager.isEnabled('new-stitcher', { userId: `user-${i}` })) {
            enabled++;
          }
        }

        // Should be roughly 50% (within 10% tolerance)
        expect(enabled).toBeGreaterThan(400);
        expect(enabled).toBeLessThan(600);
      });

      it('should be sticky for same user', () => {
        manager.createRollout({
          featureId: 'new-stitcher',
          strategy: 'percentage',
          percentage: 50,
        });

        const firstCheck = manager.isEnabled('new-stitcher', { userId: 'user-123' });
        
        // Same user should get same result
        for (let i = 0; i < 10; i++) {
          expect(manager.isEnabled('new-stitcher', { userId: 'user-123' })).toBe(firstCheck);
        }
      });
    });

    describe('allowlist/denylist', () => {
      it('should allow users on allowlist', () => {
        manager.createRollout({
          featureId: 'beta-feature',
          strategy: 'allowlist',
          allowlist: ['user-1', 'user-2', 'user-3'],
        });

        expect(manager.isEnabled('beta-feature', { userId: 'user-1' })).toBe(true);
        expect(manager.isEnabled('beta-feature', { userId: 'user-99' })).toBe(false);
      });

      it('should block users on denylist', () => {
        manager.createRollout({
          featureId: 'stable-feature',
          strategy: 'percentage',
          percentage: 100,
          denylist: ['problematic-user'],
        });

        expect(manager.isEnabled('stable-feature', { userId: 'normal-user' })).toBe(true);
        expect(manager.isEnabled('stable-feature', { userId: 'problematic-user' })).toBe(false);
      });
    });

    describe('canary rollout', () => {
      it('should enable for canary group first', () => {
        manager.createRollout({
          featureId: 'risky-change',
          strategy: 'canary',
          canaryPercentage: 1,
          canaryGroup: 'internal',
        });

        expect(manager.isEnabled('risky-change', { userId: 'external-user' })).toBe(false);
        expect(manager.isEnabled('risky-change', { userId: 'internal-user', group: 'internal' })).toBe(true);
      });
    });

    describe('geographic rollout', () => {
      it('should enable for specific regions', () => {
        manager.createRollout({
          featureId: 'regional-feature',
          strategy: 'geographic',
          regions: ['us-east-1', 'us-west-2'],
        });

        expect(manager.isEnabled('regional-feature', { region: 'us-east-1' })).toBe(true);
        expect(manager.isEnabled('regional-feature', { region: 'eu-west-1' })).toBe(false);
      });
    });
  });

  describe('SafeModeManager', () => {
    let manager: SafeModeManager;

    beforeEach(() => {
      manager = new SafeModeManager({
        errorRateThreshold: 0.1, // 10%
        latencyP99ThresholdMs: 500,
        evaluationWindowMs: 60000,
        autoRecoveryEnabled: true,
      });
    });

    describe('recordMetric', () => {
      it('should activate safe mode on high error rate', () => {
        // Record 15% errors
        for (let i = 0; i < 85; i++) {
          manager.recordMetric({ success: true, latencyMs: 100 });
        }
        for (let i = 0; i < 15; i++) {
          manager.recordMetric({ success: false, latencyMs: 100 });
        }

        expect(manager.isActive()).toBe(true);
      });

      it('should activate safe mode on high latency', () => {
        // Record high latency
        for (let i = 0; i < 100; i++) {
          manager.recordMetric({ success: true, latencyMs: 600 });
        }

        expect(manager.isActive()).toBe(true);
      });

      it('should not activate on good metrics', () => {
        for (let i = 0; i < 100; i++) {
          manager.recordMetric({ success: true, latencyMs: 100 });
        }

        expect(manager.isActive()).toBe(false);
      });
    });

    describe('getDegradedBehavior', () => {
      it('should return degraded behavior when active', () => {
        manager.forceActivate('Testing');

        const behavior = manager.getDegradedBehavior();

        expect(behavior).toBeDefined();
        expect(behavior.skipPodDecisioning).toBe(true);
        expect(behavior.usePrebuiltManifests).toBe(true);
      });
    });
  });

  describe('SLIManager', () => {
    let manager: SLIManager;

    beforeEach(() => {
      manager = new SLIManager({
        sloTarget: 0.999, // 99.9%
        evaluationWindowMs: 3600000, // 1 hour
      });
    });

    describe('recordSLI', () => {
      it('should track decision latency', () => {
        manager.recordSLI('decision_latency_p99', 150);
        manager.recordSLI('decision_latency_p99', 200);
        manager.recordSLI('decision_latency_p99', 180);

        const sli = manager.getSLI('decision_latency_p99');

        expect(sli.current).toBeDefined();
        expect(sli.p99).toBeDefined();
      });

      it('should track fill rate', () => {
        manager.recordSLI('fill_rate', 0.95);
        manager.recordSLI('fill_rate', 0.92);
        manager.recordSLI('fill_rate', 0.98);

        const sli = manager.getSLI('fill_rate');

        expect(sli.average).toBeCloseTo(0.95, 1);
      });
    });

    describe('error budget', () => {
      it('should calculate remaining error budget', () => {
        // 99.9% SLO means 0.1% error budget
        // With 1000 requests and 1 error, budget used = 0.1%
        for (let i = 0; i < 999; i++) {
          manager.recordSLI('error_rate', 0);
        }
        manager.recordSLI('error_rate', 1);

        const budget = manager.getErrorBudget();

        expect(budget.remaining).toBeGreaterThan(0);
        expect(budget.percentUsed).toBeLessThan(100);
      });

      it('should alert when error budget is low', () => {
        const alertCallback = jest.fn();
        manager.onAlert(alertCallback);

        // Exhaust error budget
        for (let i = 0; i < 100; i++) {
          manager.recordSLI('error_rate', 1);
        }

        expect(alertCallback).toHaveBeenCalled();
      });
    });
  });

  describe('ExplainService', () => {
    let service: ExplainService;

    beforeEach(() => {
      service = new ExplainService();
    });

    describe('createTrace', () => {
      it('should create and record trace', () => {
        const traceId = service.createTrace('session-123', 'pod-1');

        service.recordStep(traceId, {
          step: 'bid_request',
          input: { podDuration: 30 },
          output: { bids: 5 },
          durationMs: 50,
        });

        service.recordStep(traceId, {
          step: 'bid_selection',
          input: { bids: 5 },
          output: { selected: 2 },
          durationMs: 10,
        });

        const trace = service.getTrace(traceId);

        expect(trace.steps).toHaveLength(2);
        expect(trace.totalDurationMs).toBe(60);
      });
    });

    describe('explainDecision', () => {
      it('should explain pod decision', () => {
        const traceId = service.createTrace('session-123', 'pod-1');
        
        service.recordSlotDecision(traceId, 'slot-1', {
          winner: 'creative-abc',
          winningBid: 5.50,
          reason: 'highest_bid',
          alternatives: [
            { creativeId: 'creative-xyz', bid: 4.20, reason: 'outbid' },
          ],
        });

        const explanation = service.explainDecision(traceId, 'slot-1');

        expect(explanation.winner).toBe('creative-abc');
        expect(explanation.reason).toBe('highest_bid');
        expect(explanation.alternatives).toHaveLength(1);
      });
    });
  });

  describe('SlateManager', () => {
    let manager: SlateManager;

    beforeEach(() => {
      manager = new SlateManager({
        defaultSlateUrl: 'https://cdn.example.com/slate.ts',
        formats: ['ts', 'cmaf'],
      });
    });

    describe('getSlate', () => {
      it('should return default slate', () => {
        const slate = manager.getSlate({ format: 'ts', duration: 30 });

        expect(slate.url).toBe('https://cdn.example.com/slate.ts');
        expect(slate.duration).toBe(30);
      });

      it('should return tenant-specific slate', () => {
        manager.setTenantSlate('tenant-123', {
          url: 'https://cdn.example.com/tenant-123-slate.ts',
          logoUrl: 'https://cdn.example.com/tenant-123-logo.png',
        });

        const slate = manager.getSlate({
          format: 'ts',
          duration: 30,
          tenantId: 'tenant-123',
        });

        expect(slate.url).toContain('tenant-123');
      });

      it('should loop slate for longer durations', () => {
        const slate = manager.getSlate({
          format: 'ts',
          duration: 90,
          slateDuration: 30,
        });

        expect(slate.loopCount).toBe(3);
      });
    });

    describe('generateSegments', () => {
      it('should generate segment URLs for duration', () => {
        const segments = manager.generateSegments({
          format: 'ts',
          duration: 30,
          segmentDuration: 6,
        });

        expect(segments).toHaveLength(5);
        segments.forEach((segment, i) => {
          expect(segment.index).toBe(i);
          expect(segment.duration).toBe(6);
        });
      });
    });
  });
});
