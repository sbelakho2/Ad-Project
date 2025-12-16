import { 
  getVatReports, 
  generateAnnualReport, 
  generateTsdReport, 
  getComplianceStatus,
  COMPANY_INFO 
} from '../src/services/estonian-compliance';
import { getReadOnlyPool } from '../src/database';

// Mock the database module
jest.mock('../src/database', () => ({
  getReadOnlyPool: jest.fn(),
}));

// Mock the logger to avoid cluttering output
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
    it('should calculate financials correctly', async () => {
      // Mock revenue query
      mockQuery.mockResolvedValueOnce({
        rows: [{ total: '5000000' }] // 50,000 EUR
      });

      // Mock expenses query
      mockQuery.mockResolvedValueOnce({
        rows: [{ total: '3000000' }] // 30,000 EUR
      });

      const report = await generateAnnualReport(2025);

      expect(report.year).toBe(2025);
      expect(report.financials).toEqual({
        revenueEur: 50000,
        expensesEur: 30000,
        profitEur: 20000,
        assetsEur: 0,
        liabilitiesEur: 0,
        equityEur: 0,
      });
      expect(report.companyInfo).toEqual(COMPANY_INFO);
    });

    it('should handle null values from DB', async () => {
      mockQuery.mockResolvedValueOnce({ rows: [{ total: null }] });
      mockQuery.mockResolvedValueOnce({ rows: [{ total: null }] });

      const report = await generateAnnualReport(2025);

      expect(report.financials.revenueEur).toBe(0);
      expect(report.financials.expensesEur).toBe(0);
      expect(report.financials.profitEur).toBe(0);
    });
  });

  describe('generateTsdReport', () => {
    it('should return a placeholder TSD report', async () => {
      const report = await generateTsdReport(2025, 1);
      
      expect(report).toEqual({
        year: 2025,
        month: 1,
        companyInfo: COMPANY_INFO,
        generatedAt: expect.any(String),
        employees: 0,
        payrollEur: 0,
        dividendsEur: 0,
        taxPayableEur: 0,
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
});
