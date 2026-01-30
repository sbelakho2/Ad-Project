import { vraService } from '../vraService';

// Mock Postgres helper
jest.mock('../../../utils/postgres', () => ({
  query: jest.fn(),
}));

const { query } = jest.requireMock('../../../utils/postgres');

describe('VRA Service — Intraday Buffer', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    // Mock empty results for these tests (we're testing date logic, not data)
    (query as jest.Mock).mockResolvedValue({ rows: [] });
    
    // Reset env vars
    delete process.env.VRA_INTRADAY_BUFFER_HOURS;
  });

  afterEach(() => {
    delete process.env.VRA_INTRADAY_BUFFER_HOURS;
  });

  it('uses default 24h buffer when no dates provided', async () => {
    const now = new Date();
    const expectedEndMs = now.getTime() - 24 * 60 * 60 * 1000;
    
    await vraService.getOverview({ publisherId: 'test-pub' });
    
    // Check that the query was called with dates that exclude today
    const call = (query as jest.Mock).mock.calls[0];
    const params = call[1] as string[];
    
    // The 'to' param should be at midnight UTC of yesterday (24h ago)
    const toDate = new Date(params[1]);
    expect(toDate.getUTCHours()).toBe(0);
    expect(toDate.getUTCMinutes()).toBe(0);
    expect(toDate.getUTCSeconds()).toBe(0);
    
    // Should be approximately 24h ago from now (within 1 hour tolerance for test timing)
    const toMs = toDate.getTime();
    const tolerance = 60 * 60 * 1000; // 1 hour
    expect(Math.abs(toMs - expectedEndMs)).toBeLessThan(tolerance + 24 * 60 * 60 * 1000); // midnight alignment
    
    // 'from' should be 7 days before 'to'
    const fromDate = new Date(params[0]);
    const daysDiff = (toDate.getTime() - fromDate.getTime()) / (24 * 60 * 60 * 1000);
    expect(daysDiff).toBeCloseTo(7, 0);
  });

  it('respects explicit from/to dates (no buffer applied)', async () => {
    const from = '2025-11-01T00:00:00Z';
    const to = '2025-11-08T00:00:00Z';
    
    await vraService.getOverview({ publisherId: 'test-pub', from, to });
    
    const call = (query as jest.Mock).mock.calls[0];
    const params = call[1] as string[];
    
    // Should use the exact dates provided
    expect(params[0]).toBe(from);
    expect(params[1]).toBe(to);
  });

  it('default window ends at midnight UTC', async () => {
    await vraService.getOverview({ publisherId: 'test-pub' });
    
    const call = (query as jest.Mock).mock.calls[0];
    const params = call[1] as string[];
    
    const toDate = new Date(params[1]);
    const fromDate = new Date(params[0]);
    
    // Both dates should be at midnight UTC
    expect(toDate.getUTCHours()).toBe(0);
    expect(toDate.getUTCMinutes()).toBe(0);
    expect(fromDate.getUTCHours()).toBe(0);
    expect(fromDate.getUTCMinutes()).toBe(0);
  });

  it('default window excludes today to avoid false Underpay flags', async () => {
    const now = new Date();
    const todayMidnight = new Date(now);
    todayMidnight.setUTCHours(0, 0, 0, 0);
    
    await vraService.getOverview({ publisherId: 'test-pub' });
    
    const call = (query as jest.Mock).mock.calls[0];
    const params = call[1] as string[];
    
    const toDate = new Date(params[1]);
    
    // The 'to' date should be before today's midnight (i.e., yesterday or earlier)
    expect(toDate.getTime()).toBeLessThan(todayMidnight.getTime());
  });
});
