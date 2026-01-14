import { generateVatPDF, generateAnnualReportPDF, generateTsdPDF, generateKmdInfPDF } from '../src/services/pdf-generator';
import { COMPANY_INFO } from '../src/services/estonian-compliance';

describe('PDF Generator Service', () => {
  it('should generate a VAT report PDF buffer', async () => {
    const mockReport = {
      id: '1',
      year: 2025,
      quarter: 4,
      startDate: '2025-10-01',
      endDate: '2025-12-31',
      totalSalesEur: 1000,
      vatCollectedEur: 200,
      totalPurchasesEur: 500,
      vatPaidEur: 100,
      netVatPayableEur: 100,
      status: 'draft',
      dueDate: '2026-01-20',
      submittedAt: null,
      companyInfo: COMPANY_INFO,
      generatedAt: new Date().toISOString(),
    };

    const buffer = await generateVatPDF(mockReport);
    expect(buffer).toBeInstanceOf(Buffer);
    expect(buffer.length).toBeGreaterThan(0);
    // PDF signature
    expect(buffer.slice(0, 4).toString()).toBe('%PDF');
  });

  it('should generate an Annual Report PDF buffer', async () => {
    const mockReport = {
      year: 2025,
      companyInfo: COMPANY_INFO,
      generatedAt: new Date().toISOString(),
      status: 'draft',
      financials: {
        revenueEur: 10000,
        expensesEur: 5000,
        profitEur: 5000,
        assetsEur: 20000,
        liabilitiesEur: 5000,
        equityEur: 15000,
      },
    };

    const buffer = await generateAnnualReportPDF(mockReport);
    expect(buffer).toBeInstanceOf(Buffer);
    expect(buffer.length).toBeGreaterThan(0);
    expect(buffer.slice(0, 4).toString()).toBe('%PDF');
  });

  it('should generate a TSD report PDF buffer', async () => {
    const mockReport = {
      year: 2025,
      month: 1,
      companyInfo: COMPANY_INFO,
      generatedAt: new Date().toISOString(),
      employees: 5,
      payrollEur: 10000,
      dividendsEur: 0,
      taxPayableEur: 2500,
    };

    const buffer = await generateTsdPDF(mockReport);
    expect(buffer).toBeInstanceOf(Buffer);
    expect(buffer.length).toBeGreaterThan(0);
    expect(buffer.slice(0, 4).toString()).toBe('%PDF');
  });

  it('should generate a KMD INF report PDF buffer', async () => {
    const mockReport = {
      year: 2025,
      month: 12,
      companyInfo: COMPANY_INFO,
      generatedAt: new Date().toISOString(),
      transactions: [
        { partnerName: 'Big Corp', partnerRegCode: '123', invoiceNumber: 'INV-1', invoiceDate: '2025-12-01', sumWithoutVat: 1500, vatAmount: 300 }
      ]
    };

    const buffer = await generateKmdInfPDF(mockReport);
    expect(buffer).toBeInstanceOf(Buffer);
    expect(buffer.length).toBeGreaterThan(0);
    expect(buffer.slice(0, 4).toString()).toBe('%PDF');
  });
});
