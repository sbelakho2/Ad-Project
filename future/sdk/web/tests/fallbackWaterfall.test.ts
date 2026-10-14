/**
 * FallbackWaterfall tests
 * Tests the waterfall mediation strategy for ad loading
 */
import {
  FallbackWaterfall,
  createWaterfall,
  success,
  noFill,
  error,
  timeout,
  DEFAULT_TIMEOUT_MS,
  WaterfallSource
} from '../src/fallbackWaterfall';

describe('FallbackWaterfall', () => {
  describe('Constants', () => {
    it('should have correct default timeout', () => {
      expect(DEFAULT_TIMEOUT_MS).toBe(5000);
    });
  });
  
  describe('Result helpers', () => {
    it('should create success result', () => {
      const result = success('test-data');
      expect(result.type).toBe('success');
      expect((result as any).data).toBe('test-data');
    });
    
    it('should create noFill result', () => {
      const result = noFill('No ads');
      expect(result.type).toBe('noFill');
      expect((result as any).reason).toBe('No ads');
    });
    
    it('should create error result', () => {
      const err = new Error('Test error');
      const result = error(err);
      expect(result.type).toBe('error');
      expect((result as any).error).toBe(err);
    });
    
    it('should create timeout result', () => {
      const result = timeout();
      expect(result.type).toBe('timeout');
    });
  });
  
  describe('Factory function', () => {
    it('should create waterfall with default timeout', () => {
      const waterfall = createWaterfall<string>();
      expect(waterfall).toBeInstanceOf(FallbackWaterfall);
    });
    
    it('should create waterfall with custom timeout', () => {
      const waterfall = createWaterfall<string>(3000);
      expect(waterfall).toBeInstanceOf(FallbackWaterfall);
    });
  });
  
  describe('execute()', () => {
    it('should return success from first source', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => success('ad-data')),
        waterfall.createSource('source2', 2, async () => success('backup-ad'))
      ];
      
      const result = await waterfall.execute(sources);
      
      expect(result.result.type).toBe('success');
      expect(result.sourceId).toBe('source1');
      expect(result.attemptsCount).toBe(1);
    });
    
    it('should fallback to second source on no-fill', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => noFill('No inventory')),
        waterfall.createSource('source2', 2, async () => success('backup-ad'))
      ];
      
      const result = await waterfall.execute(sources);
      
      expect(result.result.type).toBe('success');
      expect(result.sourceId).toBe('source2');
      expect(result.attemptsCount).toBe(2);
    });
    
    it('should fallback on error', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => {
          throw new Error('Network error');
        }),
        waterfall.createSource('source2', 2, async () => success('backup-ad'))
      ];
      
      const result = await waterfall.execute(sources);
      
      expect(result.result.type).toBe('success');
      expect(result.sourceId).toBe('source2');
      expect(result.attemptsCount).toBe(2);
      expect(result.attemptDetails[0].result).toBe('error');
    });
    
    it('should respect priority ordering', async () => {
      const waterfall = new FallbackWaterfall<string>();
      const callOrder: string[] = [];
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('low-priority', 10, async () => {
          callOrder.push('low-priority');
          return success('low');
        }),
        waterfall.createSource('high-priority', 1, async () => {
          callOrder.push('high-priority');
          return noFill('No fill');
        }),
        waterfall.createSource('medium-priority', 5, async () => {
          callOrder.push('medium-priority');
          return success('medium');
        })
      ];
      
      const result = await waterfall.execute(sources);
      
      expect(callOrder).toEqual(['high-priority', 'medium-priority']);
      expect(result.sourceId).toBe('medium-priority');
    });
    
    it('should return no-fill when all sources exhausted', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => noFill('No ads')),
        waterfall.createSource('source2', 2, async () => noFill('No inventory'))
      ];
      
      const result = await waterfall.execute(sources);
      
      expect(result.result.type).toBe('noFill');
      expect(result.attemptsCount).toBe(2);
      expect((result.result as any).reason).toContain('All 2 sources exhausted');
    });
    
    it('should handle timeout correctly', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource(
          'slow-source',
          1,
          async (signal) => {
            // This source takes too long and should timeout
            return new Promise((resolve, reject) => {
              const timeoutId = setTimeout(() => resolve(success('slow-ad')), 200);
              signal.addEventListener('abort', () => {
                clearTimeout(timeoutId);
                reject(new DOMException('Aborted', 'AbortError'));
              });
            });
          },
          50 // 50ms timeout
        ),
        waterfall.createSource('fast-source', 2, async () => success('fast-ad'))
      ];
      
      const result = await waterfall.execute(sources);
      
      expect(result.result.type).toBe('success');
      expect(result.sourceId).toBe('fast-source');
      expect(result.attemptDetails[0].result).toBe('timeout');
    });
    
    it('should track attempt details', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => noFill('No fill')),
        waterfall.createSource('source2', 2, async () => {
          throw new Error('Error');
        }),
        waterfall.createSource('source3', 3, async () => success('ad'))
      ];
      
      const result = await waterfall.execute(sources);
      
      expect(result.attemptDetails).toHaveLength(3);
      expect(result.attemptDetails[0]).toMatchObject({
        sourceId: 'source1',
        priority: 1,
        result: 'noFill'
      });
      expect(result.attemptDetails[1]).toMatchObject({
        sourceId: 'source2',
        priority: 2,
        result: 'error'
      });
      expect(result.attemptDetails[2]).toMatchObject({
        sourceId: 'source3',
        priority: 3,
        result: 'success'
      });
    });
    
    it('should track total duration', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => {
          await new Promise(resolve => setTimeout(resolve, 20));
          return noFill('No fill');
        }),
        waterfall.createSource('source2', 2, async () => {
          await new Promise(resolve => setTimeout(resolve, 20));
          return success('ad');
        })
      ];
      
      const result = await waterfall.execute(sources);
      
      expect(result.totalDurationMs).toBeGreaterThanOrEqual(40);
    });
    
    it('should handle empty sources array', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const result = await waterfall.execute([]);
      
      expect(result.result.type).toBe('noFill');
      expect(result.attemptsCount).toBe(0);
      expect(result.sourceId).toBe('none');
    });
    
    it('should handle abort signal during execution', async () => {
      const waterfall = new FallbackWaterfall<string>();
      const controller = new AbortController();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async (signal) => {
          return new Promise((resolve, reject) => {
            const timeoutId = setTimeout(() => resolve(success('ad')), 200);
            signal.addEventListener('abort', () => {
              clearTimeout(timeoutId);
              reject(new DOMException('Aborted', 'AbortError'));
            });
          });
        }, 500) // High timeout so abort happens first
      ];
      
      // Abort quickly
      setTimeout(() => controller.abort(), 20);
      
      // The loader itself throws, so we get an error result
      const result = await waterfall.execute(sources, controller.signal);
      
      // When aborted, the source throws and we get error/timeout
      expect(result.attemptsCount).toBe(1);
      expect(['error', 'timeout']).toContain(result.attemptDetails[0].result);
    });
  });
  
  describe('executeWithParallelPreload()', () => {
    it('should preload lower-priority sources', async () => {
      const waterfall = new FallbackWaterfall<string>();
      const startTimes: Record<string, number> = {};
      const start = Date.now();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => {
          startTimes['source1'] = Date.now() - start;
          await new Promise(resolve => setTimeout(resolve, 50));
          return noFill('No fill');
        }),
        waterfall.createSource('source2', 2, async () => {
          startTimes['source2'] = Date.now() - start;
          await new Promise(resolve => setTimeout(resolve, 30));
          return success('preloaded-ad');
        })
      ];
      
      const result = await waterfall.executeWithParallelPreload(sources, 1);
      
      expect(result.result.type).toBe('success');
      // source2 should have started while source1 was still running
      expect(startTimes['source2']).toBeLessThan(50);
    });
    
    it('should still respect priority order', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => success('first-ad')),
        waterfall.createSource('source2', 2, async () => success('second-ad'))
      ];
      
      const result = await waterfall.executeWithParallelPreload(sources, 1);
      
      expect(result.sourceId).toBe('source1');
    });
  });
  
  describe('Statistics tracking', () => {
    it('should record success statistics', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => success('ad'))
      ];
      
      await waterfall.execute(sources);
      
      const stats = waterfall.getStats('source1');
      expect(stats).toBeDefined();
      expect(stats!.successCount).toBe(1);
      expect(stats!.totalAttempts).toBe(1);
      expect(stats!.successRate).toBe(1);
    });
    
    it('should record failure statistics', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => noFill('No fill')),
        waterfall.createSource('source2', 2, async () => {
          throw new Error('Error');
        }),
        waterfall.createSource('source3', 3, async () => success('ad'))
      ];
      
      await waterfall.execute(sources);
      
      expect(waterfall.getStats('source1')!.noFillCount).toBe(1);
      expect(waterfall.getStats('source2')!.failureCount).toBe(1);
      expect(waterfall.getStats('source3')!.successCount).toBe(1);
    });
    
    it('should accumulate statistics across calls', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      let callCount = 0;
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => {
          callCount++;
          return callCount <= 2 ? noFill('No fill') : success('ad');
        })
      ];
      
      await waterfall.execute(sources);
      await waterfall.execute(sources);
      await waterfall.execute(sources);
      
      const stats = waterfall.getStats('source1');
      expect(stats!.noFillCount).toBe(2);
      expect(stats!.successCount).toBe(1);
      expect(stats!.totalAttempts).toBe(3);
    });
    
    it('should track average latency', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => {
          await new Promise(resolve => setTimeout(resolve, 20));
          return success('ad');
        })
      ];
      
      await waterfall.execute(sources);
      await waterfall.execute(sources);
      
      const stats = waterfall.getStats('source1');
      expect(stats!.averageLatencyMs).toBeGreaterThanOrEqual(20);
    });
    
    it('should return all stats', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => noFill('No fill')),
        waterfall.createSource('source2', 2, async () => success('ad'))
      ];
      
      await waterfall.execute(sources);
      
      const allStats = waterfall.getAllStats();
      expect(allStats.size).toBe(2);
      expect(allStats.has('source1')).toBe(true);
      expect(allStats.has('source2')).toBe(true);
    });
    
    it('should clear stats', async () => {
      const waterfall = new FallbackWaterfall<string>();
      
      const sources: WaterfallSource<string>[] = [
        waterfall.createSource('source1', 1, async () => success('ad'))
      ];
      
      await waterfall.execute(sources);
      expect(waterfall.getStats('source1')).toBeDefined();
      
      waterfall.clearStats();
      expect(waterfall.getStats('source1')).toBeUndefined();
    });
    
    it('should return undefined for unknown source', () => {
      const waterfall = new FallbackWaterfall<string>();
      expect(waterfall.getStats('unknown')).toBeUndefined();
    });
  });
  
  describe('createSource()', () => {
    it('should create source with default timeout', () => {
      const waterfall = new FallbackWaterfall<string>(3000);
      const source = waterfall.createSource('test', 1, async () => success('ad'));
      
      expect(source.id).toBe('test');
      expect(source.priority).toBe(1);
      expect(source.timeoutMs).toBe(3000);
    });
    
    it('should create source with custom timeout', () => {
      const waterfall = new FallbackWaterfall<string>(3000);
      const source = waterfall.createSource('test', 1, async () => success('ad'), 1000);
      
      expect(source.timeoutMs).toBe(1000);
    });
  });
});
