/**
 * Unit tests for MigrationGuardrailSnapshotService
 */

import { Pool } from 'pg';
import { MigrationGuardrailSnapshotService } from '../../../src/services/migrationGuardrailSnapshotService';

describe('MigrationGuardrailSnapshotService', () => {
  let mockPool: jest.Mocked<Pool>;
  let service: MigrationGuardrailSnapshotService;

  beforeEach(() => {
    mockPool = {
      query: jest.fn(),
      connect: jest.fn(),
    } as unknown as jest.Mocked<Pool>;
    
    service = new MigrationGuardrailSnapshotService(mockPool);
  });

  describe('aggregateForExperiment', () => {
    const experimentId = 'test-exp-123';

    it('should aggregate shadow outcomes and insert snapshots', async () => {
      const mockAggregation = [
        {
          arm: 'control',
          impressions: '1000',
          fills: '950',
          revenue_micros: '5000000',
          latency_p95_ms: 450,
          latency_p50_ms: 200,
          error_rate_percent: '1.50',
        },
        {
          arm: 'test',
          impressions: '1100',
          fills: '1000',
          revenue_micros: '5500000',
          latency_p95_ms: 420,
          latency_p50_ms: 180,
          error_rate_percent: '1.20',
        },
      ];

      mockPool.query
        .mockResolvedValueOnce({ rows: mockAggregation, rowCount: 2 }) // aggregation query
        .mockResolvedValueOnce({ rows: [], rowCount: 1 }) // insert control
        .mockResolvedValueOnce({ rows: [], rowCount: 1 }); // insert test

      const result = await service.aggregateForExperiment(experimentId);

      expect(result.experimentId).toBe(experimentId);
      expect(result.controlSnapshot).toEqual({
        impressions: 1000,
        fills: 950,
        revenueMicros: 5000000,
      });
      expect(result.testSnapshot).toEqual({
        impressions: 1100,
        fills: 1000,
        revenueMicros: 5500000,
      });
      expect(mockPool.query).toHaveBeenCalledTimes(3);
    });

    it('should skip when no data available', async () => {
      mockPool.query.mockResolvedValueOnce({ rows: [], rowCount: 0 });

      const result = await service.aggregateForExperiment(experimentId);

      expect(result.skipped).toBe('no_data');
      expect(result.controlSnapshot).toBeUndefined();
      expect(result.testSnapshot).toBeUndefined();
    });

    it('should use custom window minutes', async () => {
      mockPool.query.mockResolvedValueOnce({ rows: [], rowCount: 0 });

      await service.aggregateForExperiment(experimentId, 30);

      expect(mockPool.query).toHaveBeenCalledWith(
        expect.any(String),
        [experimentId, 30]
      );
    });
  });

  describe('aggregateAllActive', () => {
    it('should aggregate for all active experiments', async () => {
      const activeExperiments = [
        { id: 'exp-1' },
        { id: 'exp-2' },
      ];

      // Mock getActiveExperiments query
      mockPool.query.mockResolvedValueOnce({ rows: activeExperiments, rowCount: 2 });
      
      // Mock aggregation queries for each experiment (returns empty = skipped)
      mockPool.query.mockResolvedValueOnce({ rows: [], rowCount: 0 });
      mockPool.query.mockResolvedValueOnce({ rows: [], rowCount: 0 });

      const result = await service.aggregateAllActive();

      expect(result.processed).toBe(0);
      expect(result.skipped).toBe(2);
      expect(result.errors).toBe(0);
    });

    it('should count errors correctly', async () => {
      mockPool.query
        .mockResolvedValueOnce({ rows: [{ id: 'exp-1' }], rowCount: 1 })
        .mockRejectedValueOnce(new Error('DB error'));

      const result = await service.aggregateAllActive();

      expect(result.errors).toBe(1);
      expect(result.processed).toBe(0);
    });
  });

  describe('cleanupOldSnapshots', () => {
    it('should delete snapshots older than retention period', async () => {
      mockPool.query.mockResolvedValueOnce({ rows: [], rowCount: 150 });

      const deleted = await service.cleanupOldSnapshots(7);

      expect(deleted).toBe(150);
      expect(mockPool.query).toHaveBeenCalledWith(
        expect.stringContaining('DELETE FROM migration_guardrail_snapshots'),
        [7]
      );
    });

    it('should use default 7 day retention', async () => {
      mockPool.query.mockResolvedValueOnce({ rows: [], rowCount: 0 });

      await service.cleanupOldSnapshots();

      expect(mockPool.query).toHaveBeenCalledWith(
        expect.any(String),
        [7]
      );
    });
  });
});
