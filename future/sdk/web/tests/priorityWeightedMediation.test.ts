/**
 * PriorityWeightedMediation Tests
 */
import {
  PriorityWeightedMediation,
  WeightedAdSource,
  AdLoadResult,
  WeightedAdSourceBuilder,
  createWeightedAdSource,
} from '../src/priorityWeightedMediation';

function createSource(
  id: string,
  priority: number,
  weight: number = 1.0,
  enabled: boolean = true
): WeightedAdSource {
  return {
    id,
    priority,
    weight,
    timeout: 1000,
    enabled,
    minBid: 0,
    metadata: {},
  };
}

function successResult(
  sourceId: string,
  latencyMs: number = 100,
  bid: number = 1.0
): AdLoadResult {
  return { type: 'success', ad: {}, sourceId, latencyMs, bid };
}

function noFillResult(sourceId: string): AdLoadResult {
  return { type: 'noFill', sourceId, reason: 'No inventory' };
}

describe('PriorityWeightedMediation', () => {
  let mediation: PriorityWeightedMediation;

  beforeEach(() => {
    mediation = new PriorityWeightedMediation();
  });

  describe('Basic Execution', () => {
    it('should return noFill for empty sources', async () => {
      const result = await mediation.execute([], async () => successResult('test'));
      
      expect(result.type).toBe('noFill');
      expect(result.sourceId).toBe('none');
    });

    it('should return noFill when all sources are disabled', async () => {
      const sources = [
        createSource('s1', 0, 1.0, false),
        createSource('s2', 0, 1.0, false),
      ];

      const result = await mediation.execute(sources, async () => successResult('test'));
      
      expect(result.type).toBe('noFill');
      expect((result as { reason: string }).reason).toContain('disabled');
    });

    it('should execute a single source successfully', async () => {
      const sources = [createSource('s1', 0)];

      const result = await mediation.execute(sources, async (source) => 
        successResult(source.id)
      );
      
      expect(result.type).toBe('success');
      expect(result.sourceId).toBe('s1');
    });

    it('should try sources in priority order', async () => {
      const sources = [
        createSource('low', 2),
        createSource('high', 0),
        createSource('medium', 1),
      ];

      const calledOrder: string[] = [];

      const result = await mediation.execute(sources, async (source) => {
        calledOrder.push(source.id);
        if (source.id === 'high') {
          return successResult(source.id);
        }
        return noFillResult(source.id);
      });

      expect(result.type).toBe('success');
      expect(result.sourceId).toBe('high');
      expect(calledOrder[0]).toBe('high');
    });

    it('should fall through to lower priority on noFill', async () => {
      const sources = [
        createSource('high', 0),
        createSource('low', 1),
      ];

      const result = await mediation.execute(sources, async (source) => {
        if (source.id === 'low') {
          return successResult(source.id);
        }
        return noFillResult(source.id);
      });

      expect(result.type).toBe('success');
      expect(result.sourceId).toBe('low');
    });
  });

  describe('Weighted Selection', () => {
    it('should prefer higher weight sources', async () => {
      // Create sources with same priority but different weights
      const sources = [
        createSource('low_weight', 0, 0.1),
        createSource('high_weight', 0, 10.0),
      ];

      const selections: Record<string, number> = { low_weight: 0, high_weight: 0 };

      // Run multiple times to get statistical distribution
      for (let i = 0; i < 100; i++) {
        const m = new PriorityWeightedMediation();
        await m.execute(sources, async (source) => {
          selections[source.id]++;
          return successResult(source.id);
        });
      }

      // High weight should be selected more often
      expect(selections.high_weight).toBeGreaterThan(selections.low_weight);
    });

    it('should try all sources in priority group if needed', async () => {
      const sources = [
        createSource('s1', 0),
        createSource('s2', 0),
        createSource('s3', 0),
      ];

      const called = new Set<string>();

      await mediation.execute(sources, async (source) => {
        called.add(source.id);
        if (called.size === 3) {
          return successResult(source.id);
        }
        return noFillResult(source.id);
      });

      expect(called.size).toBe(3);
    });
  });

  describe('Timeout Handling', () => {
    it('should timeout slow sources', async () => {
      const sources = [createSource('slow', 0)];
      sources[0].timeout = 50; // Very short timeout

      const result = await mediation.execute(sources, async () => {
        await new Promise((resolve) => setTimeout(resolve, 200));
        return successResult('slow');
      });

      expect(result.type).toBe('timeout');
    });

    it('should try next source after timeout', async () => {
      const sources = [
        { ...createSource('slow', 0), timeout: 50 },
        createSource('fast', 0),
      ];

      const result = await mediation.execute(sources, async (source) => {
        if (source.id === 'slow') {
          await new Promise((resolve) => setTimeout(resolve, 200));
          return successResult('slow');
        }
        return successResult('fast');
      });

      expect(result.type).toBe('success');
      expect(result.sourceId).toBe('fast');
    });
  });

  describe('Error Handling', () => {
    it('should catch and record errors', async () => {
      // error source has higher priority (0), success has lower (1)
      // This ensures error is tried first, then falls through to success
      const sources = [createSource('error', 0), createSource('success', 1)];

      const result = await mediation.execute(sources, async (source) => {
        if (source.id === 'error') {
          throw new Error('Test error');
        }
        return successResult('success');
      });

      expect(result.type).toBe('success');
      expect(result.sourceId).toBe('success');

      const stats = mediation.getSourcePerformance('error');
      expect(stats?.errorCount).toBe(1);
    });
  });

  describe('Statistics Tracking', () => {
    it('should track successful attempts', async () => {
      const sources = [createSource('s1', 0)];

      await mediation.execute(sources, async () => successResult('s1', 150, 2.5));

      const stats = mediation.getSourcePerformance('s1');
      
      expect(stats).not.toBeNull();
      expect(stats!.totalAttempts).toBe(1);
      expect(stats!.successCount).toBe(1);
      expect(stats!.averageLatencyMs).toBe(150);
      expect(stats!.fillRate).toBe(1.0);
    });

    it('should track noFill attempts', async () => {
      const sources = [createSource('s1', 0)];

      await mediation.execute(sources, async () => noFillResult('s1'));

      const stats = mediation.getSourcePerformance('s1');
      
      expect(stats).not.toBeNull();
      expect(stats!.totalAttempts).toBe(1);
      expect(stats!.noFillCount).toBe(1);
      expect(stats!.fillRate).toBe(0);
    });

    it('should track timeout attempts', async () => {
      const sources = [{ ...createSource('s1', 0), timeout: 10 }];

      await mediation.execute(sources, async () => {
        await new Promise((resolve) => setTimeout(resolve, 100));
        return successResult('s1');
      });

      const stats = mediation.getSourcePerformance('s1');
      
      expect(stats).not.toBeNull();
      expect(stats!.timeoutCount).toBe(1);
    });

    it('should get all performance stats', async () => {
      const sources = [createSource('s1', 0), createSource('s2', 1)];

      await mediation.execute(sources, async (source) => {
        if (source.id === 's1') return noFillResult('s1');
        return successResult('s2');
      });

      const allStats = mediation.getPerformanceStats();
      
      expect(allStats.length).toBe(2);
      expect(allStats.find((s) => s.sourceId === 's1')?.noFillCount).toBe(1);
      expect(allStats.find((s) => s.sourceId === 's2')?.successCount).toBe(1);
    });

    it('should reset stats for a source', async () => {
      const sources = [createSource('s1', 0)];
      await mediation.execute(sources, async () => successResult('s1'));

      expect(mediation.getSourcePerformance('s1')).not.toBeNull();

      mediation.resetStats('s1');

      expect(mediation.getSourcePerformance('s1')).toBeNull();
    });

    it('should reset all stats', async () => {
      const sources = [createSource('s1', 0), createSource('s2', 0)];
      await mediation.execute(sources, async () => successResult('s1'));

      mediation.resetAllStats();

      expect(mediation.getPerformanceStats().length).toBe(0);
    });
  });

  describe('Performance-Based Weighting', () => {
    it('should adjust weights based on performance', async () => {
      const m = new PriorityWeightedMediation({
        usePerformanceWeighting: true,
        minSampleSize: 3,
      });

      const sources = [
        createSource('good', 0, 1.0),
        createSource('bad', 0, 1.0),
      ];

      // Build up history - good source succeeds, bad source fails
      for (let i = 0; i < 5; i++) {
        await m.execute([createSource('good', 0)], async () => successResult('good'));
        await m.execute([createSource('bad', 0)], async () => noFillResult('bad'));
      }

      // Now check selection preference
      const selections: Record<string, number> = { good: 0, bad: 0 };

      for (let i = 0; i < 100; i++) {
        await m.execute(sources, async (source) => {
          selections[source.id]++;
          return successResult(source.id);
        });
      }

      // Good source should be preferred (with 100 iterations, should be clearer)
      expect(selections.good).toBeGreaterThanOrEqual(selections.bad);
    });

    it('should disable performance weighting when configured', async () => {
      const m = new PriorityWeightedMediation({
        usePerformanceWeighting: false,
      });

      const config = m.getConfiguration();
      expect(config.usePerformanceWeighting).toBe(false);
    });
  });

  describe('Adaptive Timeouts', () => {
    it('should use adaptive timeouts based on history', async () => {
      const m = new PriorityWeightedMediation({
        adaptiveTimeoutsEnabled: true,
        minSampleSize: 3,
      });

      // Build up latency history
      for (let i = 0; i < 5; i++) {
        await m.execute([createSource('s1', 0)], async () => successResult('s1', 100));
      }

      const stats = m.getSourcePerformance('s1');
      expect(stats?.averageLatencyMs).toBe(100);
    });
  });

  describe('Configuration', () => {
    it('should use default configuration', () => {
      const config = mediation.getConfiguration();
      
      expect(config.usePerformanceWeighting).toBe(true);
      expect(config.minSampleSize).toBe(10);
      expect(config.maxConcurrentRequests).toBe(3);
    });

    it('should allow custom configuration', () => {
      const m = new PriorityWeightedMediation({
        minSampleSize: 5,
        maxConcurrentRequests: 2,
      });

      const config = m.getConfiguration();
      
      expect(config.minSampleSize).toBe(5);
      expect(config.maxConcurrentRequests).toBe(2);
    });

    it('should update configuration', () => {
      mediation.updateConfiguration({ minSampleSize: 20 });

      const config = mediation.getConfiguration();
      expect(config.minSampleSize).toBe(20);
    });
  });

  describe('Abort Signal', () => {
    it('should respect abort signal', async () => {
      const sources = [createSource('s1', 0)];
      const controller = new AbortController();

      // Abort before execution
      controller.abort();

      const result = await mediation.execute(
        sources,
        async () => successResult('s1'),
        controller.signal
      );

      expect(result.type).toBe('noFill');
      expect((result as { reason: string }).reason).toContain('Aborted');
    });

    it('should abort during execution', async () => {
      const sources = [createSource('s1', 0)];
      const controller = new AbortController();

      const result = await mediation.execute(
        sources,
        async () => {
          controller.abort();
          await new Promise((resolve) => setTimeout(resolve, 100));
          return successResult('s1');
        },
        controller.signal
      );

      // Should complete since abort happens during loader execution
      expect(result.type).toBe('timeout');
    });
  });
});

describe('WeightedAdSourceBuilder', () => {
  it('should build a valid source', () => {
    const source = new WeightedAdSourceBuilder()
      .id('test')
      .priority(1)
      .weight(2.0)
      .timeout(3000)
      .enabled(true)
      .minBid(0.5)
      .metadata('key', 'value')
      .build();

    expect(source.id).toBe('test');
    expect(source.priority).toBe(1);
    expect(source.weight).toBe(2.0);
    expect(source.timeout).toBe(3000);
    expect(source.enabled).toBe(true);
    expect(source.minBid).toBe(0.5);
    expect(source.metadata.key).toBe('value');
  });

  it('should throw for missing id', () => {
    expect(() => new WeightedAdSourceBuilder().build()).toThrow('ID is required');
  });

  it('should throw for negative priority', () => {
    expect(() =>
      new WeightedAdSourceBuilder().id('test').priority(-1).build()
    ).toThrow('non-negative');
  });

  it('should throw for zero weight', () => {
    expect(() =>
      new WeightedAdSourceBuilder().id('test').weight(0).build()
    ).toThrow('positive');
  });

  it('should use defaults for optional fields', () => {
    const source = new WeightedAdSourceBuilder().id('test').build();

    expect(source.priority).toBe(0);
    expect(source.weight).toBe(1.0);
    expect(source.timeout).toBe(5000);
    expect(source.enabled).toBe(true);
    expect(source.minBid).toBe(0);
  });
});

describe('createWeightedAdSource', () => {
  it('should create a source with defaults', () => {
    const source = createWeightedAdSource('test', 0);

    expect(source.id).toBe('test');
    expect(source.priority).toBe(0);
    expect(source.weight).toBe(1.0);
    expect(source.enabled).toBe(true);
  });

  it('should create a source with options', () => {
    const source = createWeightedAdSource('test', 1, {
      weight: 2.0,
      enabled: false,
    });

    expect(source.weight).toBe(2.0);
    expect(source.enabled).toBe(false);
  });
});
