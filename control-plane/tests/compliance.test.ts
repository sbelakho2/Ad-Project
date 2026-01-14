import { 
  getVatReports, 
  generateAnnualReport, 
  generateTsdReport, 
  getComplianceStatus,
  generateKmdInfReport,
  getTaxForecast,
  COMPANY_INFO 
} from '../src/services/estonian-compliance';
import { getReadOnlyPool, getOperatorPool } from '../src/database';
import { logger } from '../src/logger';

// Mock the database module
jest.mock('../src/database', () => ({
  getReadOnlyPool: jest.fn(),
  getOperatorPool: jest.fn(),
}));

// Mock the logger
jest.mock('../src/logger', () => ({
  logger: {
    info: jest.fn(),
    error: jest.fn(),
    warn: jest.fn(),
  },
}));

describe('Estonian Compliance Service', () => {
  let mockQuery: jest.Mock;

  beforeEach(() => {
    mockQuery = jest.fn();
    (getReadOnlyPool as jest.Mock).mockReturnValue({
      query: mockQuery,
    });
    (getOperatorPool as jest.Mock).mockReturnValue({
      query: mockQuery,
    });
  });

  afterEach(() => {
    jest.clearAllMocks();
  });

  describe('getVatReports', () => {
    it('should return mapped VAT reports', async () => {
      const mockDbRows = [
        {
          id: '123',
          year: 2025,
          quarter: 4,
          start_date: '2025-10-01',
          end_date: '2025-12-31',
          total_sales_cents: 10000,
          vat_collected_cents: 2000,
          total_purchases_cents: 5000,
          vat_paid_cents: 1000,
          net_vat_payable_cents: 1000,
          status: 'draft',
          due_date: '2026-01-20',
          submitted_at: null,
        },
      ];

      mockQuery.mockResolvedValue({ rows: mockDbRows });

      const reports = await getVatReports();

      expect(reports).toHaveLength(1);
      expect(reports[0]).toEqual({
        id: '123',
        year: 2025,
        quarter: 4,
        startDate: '2025-10-01',
        endDate: '2025-12-31',
        totalSalesEur: 100,
        vatCollectedEur: 20,
        totalPurchasesEur: 50,
        vatPaidEur: 10,
        netVatPayableEur: 10,
        status: 'draft',
        dueDate: '2026-01-20',
        submittedAt: null,
        companyInfo: COMPANY_INFO,
        generatedAt: expect.any(String),
      });
    });

    it('should handle empty results', async () => {
      mockQuery.mockResolvedValue({ rows: [] });
      const reports = await getVatReports();
      expect(reports).toEqual([]);
    });
  });

  describe('generateAnnualReport', () => {
    it('should calculate financials correctly from account balances', async () => {
      // Mock account_balances query
      mockQuery.mockResolvedValueOnce({
        rows: [
          { account_code: '4000', account_type: 'revenue', balance_cents: '5000000' },
          { account_code: '5000', account_type: 'expense', balance_cents: '3000000' },
          { account_code: '1000', account_type: 'asset', balance_cents: '10000000' },
          { account_code: '2000', account_type: 'liability', balance_cents: '2000000' },
          { account_code: '3000', account_type: 'equity', balance_cents: '8000000' },
        ]
      });

      const report = await generateAnnualReport(2025);

      expect(report.year).toBe(2025);
      expect(report.financials).toEqual({
        revenueEur: 50000,
        expensesEur: 30000,
        profitEur: 20000,
        assetsEur: 100000,
        liabilitiesEur: 20000,
        equityEur: 80000,
      });
      expect(report.companyInfo).toEqual(COMPANY_INFO);
    });

    it('should archive the report when operator is provided', async () => {
      mockQuery.mockResolvedValueOnce({ rows: [] }); // balances
      mockQuery.mockResolvedValueOnce({ rows: [] }); // insert archive

      await generateAnnualReport(2025, 'admin@apexmediation.ee');

      // First query: balances, Second: insert document (including subquery for operator)
      expect(mockQuery).toHaveBeenCalledTimes(2);
      expect(mockQuery.mock.calls[1][0]).toContain('INSERT INTO financial_documents');
    });
  });

  describe('generateTsdReport', () => {
    it('should calculate TSD report from ledger payouts', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [{ total: '1000000', employees: '5' }]
      });

      const report = await generateTsdReport(2025, 1);
      
      expect(report).toEqual({
        year: 2025,
        month: 1,
        companyInfo: COMPANY_INFO,
        generatedAt: expect.any(String),
        employees: 5,
        payrollEur: 10000,
        dividendsEur: 0,
        taxPayableEur: 2500,
      });
    });
  });

  describe('getComplianceStatus', () => {
    it('should return status with pending reports', async () => {
      const mockDbRows = [
        {
          id: '1',
          status: 'draft',
          due_date: '2026-01-20',
          // ... other fields needed for getVatReports mapping
          year: 2025, quarter: 4, start_date: '2025-10-01', end_date: '2025-12-31',
          total_sales_cents: 0, vat_collected_cents: 0, total_purchases_cents: 0,
          vat_paid_cents: 0, net_vat_payable_cents: 0, submitted_at: null
        }
      ];
      mockQuery.mockResolvedValue({ rows: mockDbRows });

      const status = await getComplianceStatus();

      expect(status.vatStatus.pendingReports).toBe(1);
      expect(status.vatStatus.nextDeadline).toBe('2026-01-20');
      expect(status.companyInfo).toEqual(COMPANY_INFO);
    });

    it('should return status with no pending reports', async () => {
      mockQuery.mockResolvedValue({ rows: [] });

      const status = await getComplianceStatus();

      expect(status.vatStatus.pendingReports).toBe(0);
      expect(status.vatStatus.nextDeadline).toBeNull();
    });
  });

  describe('generateKmdInfReport', () => {
    it('should return transactions over 1000 EUR', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [
          { partner_name: 'Big Corp', registry_code: '123', invoice_number: 'INV-1', issue_date: '2025-12-01', subtotal_cents: 150000, vat_cents: 30000 }
        ]
      });

      const report = await generateKmdInfReport(2025, 12);

      expect(report.transactions).toHaveLength(1);
      expect(report.transactions[0].sumWithoutVat).toBe(1500);
    });
  });

  describe('getTaxForecast', () => {
    it('should return 3 months of forecasts', async () => {
      const forecast = await getTaxForecast();
      expect(forecast).toHaveLength(3);
      expect(forecast[0].totalTaxLiabilityEur).toBeGreaterThan(0);
    });
  });
});
