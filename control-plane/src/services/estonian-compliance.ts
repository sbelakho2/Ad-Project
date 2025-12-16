import { getReadOnlyPool } from '../database';
import { logger } from '../logger';

export interface CompanyInfo {
  name: string;
  registryCode: string;
  address: string;
  vatNumber: string;
  email: string;
  phone: string;
  logoUrl: string;
}

export const COMPANY_INFO: CompanyInfo = {
  name: 'Bel Consulting OÜ',
  registryCode: '16736399',
  address: 'Harju maakond, Tallinn, Kesklinna linnaosa, Ahtri tn 12, 10151', // Placeholder address, should be verified
  vatNumber: 'EE102695468', // Placeholder, should be verified
  email: 'hello@apexmediation.ee',
  phone: '+372 5555 5555', // Placeholder
  logoUrl: 'https://apexmediation.ee/logo.png', // Placeholder
};

export interface VatReport {
  id: string;
  year: number;
  quarter: number;
  startDate: string;
  endDate: string;
  totalSalesEur: number;
  vatCollectedEur: number;
  totalPurchasesEur: number;
  vatPaidEur: number;
  netVatPayableEur: number;
  status: string;
  dueDate: string;
  submittedAt: string | null;
  companyInfo: CompanyInfo;
  generatedAt: string;
}

export interface AnnualReport {
  year: number;
  companyInfo: CompanyInfo;
  generatedAt: string;
  status: string;
  financials: {
    revenueEur: number;
    expensesEur: number;
    profitEur: number;
    assetsEur: number;
    liabilitiesEur: number;
    equityEur: number;
  };
}

export interface TsdReport {
  year: number;
  month?: number; // If monthly
  companyInfo: CompanyInfo;
  generatedAt: string;
  employees: number;
  payrollEur: number;
  dividendsEur: number;
  taxPayableEur: number;
}

export async function getVatReports(): Promise<VatReport[]> {
  const pool = getReadOnlyPool();
  const result = await pool.query(`
    SELECT 
      id, year, quarter, start_date, end_date,
      total_sales_cents, vat_collected_cents,
      total_purchases_cents, vat_paid_cents,
      net_vat_payable_cents,
      status, due_date, submitted_at
    FROM vat_reports
    ORDER BY year DESC, quarter DESC
  `);

  return result.rows.map(row => ({
    id: row.id,
    year: row.year,
    quarter: row.quarter,
    startDate: row.start_date,
    endDate: row.end_date,
    totalSalesEur: row.total_sales_cents / 100,
    vatCollectedEur: row.vat_collected_cents / 100,
    totalPurchasesEur: row.total_purchases_cents / 100,
    vatPaidEur: row.vat_paid_cents / 100,
    netVatPayableEur: row.net_vat_payable_cents / 100,
    status: row.status,
    dueDate: row.due_date,
    submittedAt: row.submitted_at,
    companyInfo: COMPANY_INFO,
    generatedAt: new Date().toISOString(),
  }));
}

export async function generateAnnualReport(year: number): Promise<AnnualReport> {
  // In a real implementation, this would aggregate data from the ledger
  // For now, we'll aggregate from invoices and expenses as a proxy
  const pool = getReadOnlyPool();
  
  const revenueResult = await pool.query(`
    SELECT SUM(subtotal_cents) as total
    FROM invoices
    WHERE EXTRACT(YEAR FROM issue_date) = $1
    AND status = 'paid'
  `, [year]);

  const expensesResult = await pool.query(`
    SELECT SUM(amount_cents) as total
    FROM expenses
    WHERE EXTRACT(YEAR FROM expense_date) = $1
    AND status = 'paid'
  `, [year]);

  const revenueCents = parseInt(revenueResult.rows[0].total || '0');
  const expensesCents = parseInt(expensesResult.rows[0].total || '0');
  const profitCents = revenueCents - expensesCents;

  return {
    year,
    companyInfo: COMPANY_INFO,
    generatedAt: new Date().toISOString(),
    status: 'draft',
    financials: {
      revenueEur: revenueCents / 100,
      expensesEur: expensesCents / 100,
      profitEur: profitCents / 100,
      assetsEur: 0, // Placeholder
      liabilitiesEur: 0, // Placeholder
      equityEur: 0, // Placeholder
    }
  };
}

export async function generateTsdReport(year: number, month?: number): Promise<TsdReport> {
  // Placeholder implementation
  // In reality, this would query payroll and dividend data
  
  return {
    year,
    month,
    companyInfo: COMPANY_INFO,
    generatedAt: new Date().toISOString(),
    employees: 0,
    payrollEur: 0,
    dividendsEur: 0,
    taxPayableEur: 0,
  };
}

export async function getComplianceStatus() {
  const vatReports = await getVatReports();
  const pendingVat = vatReports.filter(r => r.status === 'draft' || r.status === 'submitted'); // Assuming 'paid' is final
  
  return {
    companyInfo: COMPANY_INFO,
    vatStatus: {
      pendingReports: pendingVat.length,
      nextDeadline: pendingVat.length > 0 ? pendingVat[0].dueDate : null,
    },
    annualReportStatus: {
      lastSubmittedYear: 2024, // Placeholder
      nextDeadline: '2026-06-30',
    }
  };
}
