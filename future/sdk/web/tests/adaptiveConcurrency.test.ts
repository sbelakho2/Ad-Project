import { AdaptiveConcurrency, getAdaptiveConcurrency } from '../src/adaptiveConcurrency';

describe('AdaptiveConcurrency', () => {
  beforeEach(() => {
    AdaptiveConcurrency.reset();
  });

  afterEach(() => {
    AdaptiveConcurrency.reset();
  });

  describe('getInstance', () => {
    it('should return singleton instance', () => {
      const instance1 = AdaptiveConcurrency.getInstance();
      const instance2 = AdaptiveConcurrency.getInstance();
      expect(instance1).toBe(instance2);
    });

    it('should return new instance after reset', () => {
      const instance1 = AdaptiveConcurrency.getInstance();
      AdaptiveConcurrency.reset();
      const instance2 = AdaptiveConcurrency.getInstance();
      expect(instance1).not.toBe(instance2);
    });

    it('should accept custom config', () => {
      const instance = AdaptiveConcurrency.getInstance({
        maxBackgroundTasks: 10,
        maxNetworkTasks: 20,
        maxComputeTasks: 5,
      });
      
      expect(instance.getBackgroundPoolConfig().maxConcurrency).toBe(10);
      expect(instance.getNetworkPoolConfig().maxConcurrency).toBe(20);
      expect(instance.getComputePoolConfig().maxConcurrency).toBe(5);
    });
  });

  describe('getDeviceInfo', () => {
    it('should return device information', () => {
      const instance = AdaptiveConcurrency.getInstance();
      const info = instance.getDeviceInfo();
      
      expect(info.hardwareConcurrency).toBeGreaterThan(0);
      expect(typeof info.isLowEndDevice).toBe('boolean');
      expect(typeof info.userAgent).toBe('string');
    });

    it('should return a copy of device info', () => {
      const instance = AdaptiveConcurrency.getInstance();
      const info1 = instance.getDeviceInfo();
      const info2 = instance.getDeviceInfo();
      
      expect(info1).not.toBe(info2);
      expect(info1).toEqual(info2);
    });
  });

  describe('pool configs', () => {
    it('should return background pool config', () => {
      const instance = AdaptiveConcurrency.getInstance();
      const config = instance.getBackgroundPoolConfig();
      
      expect(config.maxConcurrency).toBeGreaterThan(0);
    });

    it('should return network pool config', () => {
      const instance = AdaptiveConcurrency.getInstance();
      const config = instance.getNetworkPoolConfig();
      
      expect(config.maxConcurrency).toBeGreaterThan(0);
    });

    it('should return compute pool config', () => {
      const instance = AdaptiveConcurrency.getInstance();
      const config = instance.getComputePoolConfig();
      
      expect(config.maxConcurrency).toBeGreaterThan(0);
    });
  });

  describe('executeBackground', () => {
    it('should execute a task and return result', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      const result = await instance.executeBackground(async () => {
        return 42;
      });
      
      expect(result).toBe(42);
    });

    it('should handle task errors', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      await expect(instance.executeBackground(async () => {
        throw new Error('Test error');
      })).rejects.toThrow('Test error');
    });

    it('should limit concurrency', async () => {
      const instance = AdaptiveConcurrency.getInstance({ maxBackgroundTasks: 2 });
      let concurrent = 0;
      let maxConcurrent = 0;
      
      const tasks = Array(5).fill(null).map(() => 
        instance.executeBackground(async () => {
          concurrent++;
          maxConcurrent = Math.max(maxConcurrent, concurrent);
          await new Promise(resolve => setTimeout(resolve, 10));
          concurrent--;
          return concurrent;
        })
      );
      
      await Promise.all(tasks);
      
      expect(maxConcurrent).toBeLessThanOrEqual(2);
    });
  });

  describe('executeNetwork', () => {
    it('should execute a network task and return result', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      const result = await instance.executeNetwork(async () => {
        return 'network result';
      });
      
      expect(result).toBe('network result');
    });

    it('should handle network task errors', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      await expect(instance.executeNetwork(async () => {
        throw new Error('Network error');
      })).rejects.toThrow('Network error');
    });
  });

  describe('executeCompute', () => {
    it('should execute a compute task and return result', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      const result = await instance.executeCompute(async () => {
        let sum = 0;
        for (let i = 0; i < 100; i++) {
          sum += i;
        }
        return sum;
      });
      
      expect(result).toBe(4950);
    });

    it('should limit compute concurrency', async () => {
      const instance = AdaptiveConcurrency.getInstance({ maxComputeTasks: 1 });
      let concurrent = 0;
      let maxConcurrent = 0;
      
      const tasks = Array(3).fill(null).map(() => 
        instance.executeCompute(async () => {
          concurrent++;
          maxConcurrent = Math.max(maxConcurrent, concurrent);
          await new Promise(resolve => setTimeout(resolve, 10));
          concurrent--;
        })
      );
      
      await Promise.all(tasks);
      
      expect(maxConcurrent).toBe(1);
    });
  });

  describe('executeWhenIdle', () => {
    it('should execute a task during idle time', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      const result = await instance.executeWhenIdle(() => {
        return 'idle result';
      });
      
      expect(result).toBe('idle result');
    });

    it('should handle errors in idle tasks', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      await expect(instance.executeWhenIdle(() => {
        throw new Error('Idle error');
      })).rejects.toThrow('Idle error');
    });
  });

  describe('executeAll', () => {
    it('should execute multiple tasks concurrently', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      const tasks = [
        async () => 1,
        async () => 2,
        async () => 3,
      ];
      
      const results = await instance.executeAll(tasks);
      
      expect(results).toEqual([1, 2, 3]);
    });

    it('should respect queue type', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      const tasks = [
        async () => 'net1',
        async () => 'net2',
      ];
      
      const results = await instance.executeAll(tasks, 'network');
      
      expect(results).toEqual(['net1', 'net2']);
    });
  });

  describe('getStats', () => {
    it('should return current statistics', () => {
      const instance = AdaptiveConcurrency.getInstance();
      const stats = instance.getStats();
      
      expect(stats.deviceInfo).toBeDefined();
      expect(stats.backgroundQueue).toBeDefined();
      expect(stats.networkQueue).toBeDefined();
      expect(stats.computeQueue).toBeDefined();
      expect(stats.totalTasksSubmitted).toBe(0);
      expect(stats.totalTasksCompleted).toBe(0);
    });

    it('should track task counts', async () => {
      const instance = AdaptiveConcurrency.getInstance();
      
      await instance.executeBackground(async () => 'task1');
      await instance.executeNetwork(async () => 'task2');
      
      const stats = instance.getStats();
      
      expect(stats.totalTasksSubmitted).toBe(2);
      expect(stats.totalTasksCompleted).toBe(2);
    });

    it('should show pending tasks', async () => {
      const instance = AdaptiveConcurrency.getInstance({ maxBackgroundTasks: 1 });
      
      let resolveFirst: () => void;
      const firstTask = new Promise<void>(resolve => {
        resolveFirst = resolve;
      });
      
      // Start first task that will block
      const firstPromise = instance.executeBackground(() => firstTask);
      
      // Start second task that will be queued
      const secondPromise = instance.executeBackground(async () => 'second');
      
      // Give time for tasks to be processed
      await new Promise(resolve => setTimeout(resolve, 10));
      
      const stats = instance.getStats();
      expect(stats.backgroundQueue.pending).toBe(1);
      expect(stats.backgroundQueue.running).toBe(1);
      
      // Complete first task
      resolveFirst!();
      await firstPromise;
      await secondPromise;
    });
  });

  describe('getAdaptiveConcurrency helper', () => {
    it('should return the singleton instance', () => {
      const instance = getAdaptiveConcurrency();
      expect(instance).toBe(AdaptiveConcurrency.getInstance());
    });
  });
});
