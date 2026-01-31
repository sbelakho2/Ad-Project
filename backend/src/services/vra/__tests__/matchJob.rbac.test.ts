import { runMatchingBatch } from '../matchJob';

// Mock Postgres helpers
jest.mock('../../../utils/postgres', () => {
  const query = jest.fn(async (sql: string) => {
    const q = String(sql).toLowerCase();
    if (q.includes('from recon_statements_norm')) {
      return {
        rows: [
          { event_date: '2025-11-01', app_id: 'app', ad_unit_id: 'unit', country: 'US', format: 'interstitial', paid: '1.00', currency: 'USD', report_id: 'rep', network: 'unity' },
        ],
      };
    }
    if (q.includes('from recon_expected')) {
      return {
        rows: [
          { request_id: 'r1', ts: '2025-11-01T12:00:00Z', expected_value: '1.00' },
        ],
      };
    }
    return { rows: [] };
  });
  return {
    query,
    insertMany: jest.fn(async () => {}),
  };
});

const { insertMany } = jest.requireMock('../../../utils/postgres');

describe('VRA Matching Job — RO posture (dry-run vs write)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('does not write when dryRun=true', async () => {
    const res = await runMatchingBatch({ from: '2025-11-01T00:00:00Z', to: '2025-11-02T00:00:00Z', dryRun: true, publisherId: 'pub-1' });
    expect(res.inserted).toBe(0);
    expect(insertMany).not.toHaveBeenCalled();
  });

  it('writes to recon_match when dryRun=false and auto matches exist', async () => {
    const res = await runMatchingBatch({ from: '2025-11-01T00:00:00Z', to: '2025-11-02T00:00:00Z', dryRun: false, publisherId: 'pub-1' });
    // Depending on scoring config, there should be at least an attempt to insert
    expect(insertMany).toHaveBeenCalled();
  });
});
