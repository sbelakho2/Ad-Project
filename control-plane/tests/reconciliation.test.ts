import { processBankStatement, confirmReconciliation } from '../src/services/reconciliation';
import { getReadOnlyPool, getOperatorPool } from '../src/database';

jest.mock('../src/database', () => ({
  getReadOnlyPool: jest.fn(),
  getOperatorPool: jest.fn(),
}));

jest.mock('../src/audit', () => ({
  logAudit: jest.fn(),
}));

describe('Reconciliation Service', () => {
  let mockQuery: jest.Mock;

  beforeEach(() => {
    mockQuery = jest.fn();
    (getReadOnlyPool as jest.Mock).mockReturnValue({ query: mockQuery });
    (getOperatorPool as jest.Mock).mockReturnValue({ query: mockQuery });
  });

  describe('processBankStatement', () => {
    it('should match incoming bank entries with invoices', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [{ id: 'inv-1', invoice_number: 'INV-000001', total_cents: 50000 }]
      });
      // Mock other lookups as empty
      mockQuery.mockResolvedValue({ rows: [] });

      const matches = await processBankStatement('admin');

      expect(matches).toHaveLength(3);
      expect(matches[0].entry.reference).toBe('INV-000001');
      expect(matches[0].suggestedMatch?.id).toBe('inv-1');
      expect(matches[0].suggestedMatch?.confidence).toBe(1.0);
    });
  });

  describe('confirmReconciliation', () => {
    it('should update invoice status', async () => {
      mockQuery.mockResolvedValue({ rows: [] });

      const ok = await confirmReconciliation(
        { 
          entry: { reference: 'REF1', date: '2025-01-01', amount: 100, currency: 'EUR', counterparty: 'X', type: 'incoming' },
          targetType: 'invoice',
          targetId: 'inv-1'
        },
        'admin'
      );

      expect(ok).toBe(true);
      expect(mockQuery).toHaveBeenCalledWith(
        expect.stringContaining('UPDATE invoices'),
        ['REF1', 'inv-1']
      );
    });
  });
});
