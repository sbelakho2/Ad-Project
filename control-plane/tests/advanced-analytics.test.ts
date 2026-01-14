import { getCohortAnalysis, getRevenueForecast } from '../src/services/advanced-analytics';
import { queryReadOnly } from '../src/database';

jest.mock('../src/database', () => ({
  queryReadOnly: jest.fn(),
}));

describe('Advanced Analytics Service', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('getCohortAnalysis', () => {
    it('should return cohort data with retention percentages', async () => {
      // Mock cohorts query
      (queryReadOnly as jest.Mock).mockResolvedValueOnce([
        { cohort_month: '2025-01', size: '10' },
      ]);

      // Mock retention query for 5 months
      (queryReadOnly as jest.Mock).mockResolvedValue([{ active_count: '5' }]);

      const cohorts = await getCohortAnalysis(1);

      expect(cohorts).toHaveLength(1);
      expect(cohorts[0].cohort_month).toBe('2025-01');
      expect(cohorts[0].size).toBe(10);
      expect(cohorts[0].retention).toEqual([100, 50, 50, 50, 50, 50]);
    });
  });

  describe('getRevenueForecast', () => {
    it('should calculate forecast based on historical growth', async () => {
      (queryReadOnly as jest.Mock).mockResolvedValueOnce([
        { month: '2025-01', revenue: '1000' },
        { month: '2025-02', revenue: '1100' },
        { month: '2025-03', revenue: '1210' },
      ]);

      const forecast = await getRevenueForecast();

      // 3 actual + 6 projected
      expect(forecast).toHaveLength(9);
      expect(forecast[0].actual).toBe(1000);
      expect(forecast[3].actual).toBeNull();
      expect(forecast[3].projected).toBeGreaterThan(1210);
    });

    it('should handle insufficient history', async () => {
      (queryReadOnly as jest.Mock).mockResolvedValueOnce([
        { month: '2025-01', revenue: '1000' }
      ]);

      const forecast = await getRevenueForecast();
      expect(forecast).toHaveLength(1);
    });
  });
});
