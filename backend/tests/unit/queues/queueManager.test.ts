import { describe, it, expect, beforeEach, jest } from '@jest/globals';

// Mock bullmq before importing queueManager
jest.mock('bullmq', () => {
  const mockQueue = {
    add: jest.fn().mockResolvedValue({ id: 'test-job-id' }),
    getJobCounts: jest.fn().mockResolvedValue({
      waiting: 0,
      active: 0,
      delayed: 0,
      failed: 0,
      completed: 0,
    }),
    isPaused: jest.fn().mockResolvedValue(false),
    close: jest.fn().mockResolvedValue(undefined),
  };

  const mockWorker = {
    on: jest.fn(),
    close: jest.fn().mockResolvedValue(undefined),
  };

  return {
    Queue: jest.fn(() => mockQueue),
    Worker: jest.fn(() => mockWorker),
  };
});

// Mock redis
jest.mock('../../../src/utils/redis', () => ({
  redis: {},
}));

// Mock logger
jest.mock('../../../src/utils/logger', () => ({
  __esModule: true,
  default: {
    info: jest.fn(),
    warn: jest.fn(),
    error: jest.fn(),
    debug: jest.fn(),
  },
}));

import { Queue } from 'bullmq';

describe('QueueManager', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('normalizeJobOptions', () => {
    it('should include default retry configuration', async () => {
      // Dynamically import after mocks are set up
      const { queueManager, QueueName } = await import('../../../src/queues/queueManager');
      
      // Initialize the queue manager
      await queueManager.initialize();
      
      // Add a job without options - should use defaults
      await queueManager.addJob(QueueName.ANALYTICS_INGEST, 'test-job', { data: 'test' });
      
      // Get the mock and verify add was called with retry config
      const mockQueueInstance = (Queue as jest.Mock).mock.results[0]?.value;
      expect(mockQueueInstance.add).toHaveBeenCalledWith(
        'test-job',
        { data: 'test' },
        expect.objectContaining({
          attempts: 3,
          backoff: {
            type: 'exponential',
            delay: 2000,
          },
          removeOnComplete: { age: 24 * 3600, count: 1000 },
          removeOnFail: { age: 7 * 24 * 3600 },
        })
      );
    });

    it('should allow custom options to override defaults', async () => {
      const { queueManager, QueueName } = await import('../../../src/queues/queueManager');
      
      await queueManager.initialize();
      
      // Add a job with custom retry options
      await queueManager.addJob(
        QueueName.DATA_EXPORT, 
        'export-job', 
        { exportId: '123' },
        { 
          attempts: 5,
          backoff: { type: 'fixed', delay: 5000 },
        }
      );
      
      const mockQueueInstance = (Queue as jest.Mock).mock.results[0]?.value;
      expect(mockQueueInstance.add).toHaveBeenCalledWith(
        'export-job',
        { exportId: '123' },
        expect.objectContaining({
          attempts: 5,
          backoff: { type: 'fixed', delay: 5000 },
        })
      );
    });

    it('should have attempts >= 3 to prevent immediate DLQ escalation', async () => {
      const { queueManager, QueueName } = await import('../../../src/queues/queueManager');
      
      await queueManager.initialize();
      await queueManager.addJob(QueueName.PRIVACY, 'gdpr-delete', { userId: 'user-1' });
      
      const mockQueueInstance = (Queue as jest.Mock).mock.results[0]?.value;
      const callArgs = mockQueueInstance.add.mock.calls[0];
      const jobOptions = callArgs[2];
      
      // Critical: jobs should have at least 3 attempts to avoid immediate DLQ
      expect(jobOptions.attempts).toBeGreaterThanOrEqual(3);
    });

    it('should have exponential backoff by default', async () => {
      const { queueManager, QueueName } = await import('../../../src/queues/queueManager');
      
      await queueManager.initialize();
      await queueManager.addJob(QueueName.ANALYTICS_AGGREGATION, 'aggregate', {});
      
      const mockQueueInstance = (Queue as jest.Mock).mock.results[0]?.value;
      const callArgs = mockQueueInstance.add.mock.calls[0];
      const jobOptions = callArgs[2];
      
      // Should use exponential backoff for transient failure recovery
      expect(jobOptions.backoff).toEqual({
        type: 'exponential',
        delay: 2000,
      });
    });
  });

  describe('DLQ escalation', () => {
    it('should only move to DLQ after exhausting all retry attempts', async () => {
      // The logic: job.attemptsMade must be >= job.opts.attempts
      // With attempts: 3, a job should fail 3 times before DLQ
      // This test documents the expected behavior
      
      const { queueManager, QueueName } = await import('../../../src/queues/queueManager');
      
      await queueManager.initialize();
      
      // The default config should give jobs 3 attempts
      await queueManager.addJob(QueueName.REPORT_GENERATION, 'monthly-report', {});
      
      const mockQueueInstance = (Queue as jest.Mock).mock.results[0]?.value;
      const jobOptions = mockQueueInstance.add.mock.calls[0][2];
      
      // Job should have 3 attempts, meaning:
      // - Attempt 1 fails (attemptsMade=1) → retry
      // - Attempt 2 fails (attemptsMade=2) → retry
      // - Attempt 3 fails (attemptsMade=3) → DLQ (3 >= 3)
      expect(jobOptions.attempts).toBe(3);
    });
  });
});
