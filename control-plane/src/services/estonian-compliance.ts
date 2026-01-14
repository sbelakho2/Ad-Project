import { getReadOnlyPool, getOperatorPool } from '../database';
import { logger } from '../logger';
import * as crypto from 'crypto';

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
  registryCode: '16192499',
  address: 'Sakala 7-2, 10141 Tallinn, Estonia',
  vatNumber: 'EE102736890',
  email: 'hello@apexmediation.ee',
  phone: '+372 602 7700',
  logoUrl: 'https://apexmediation.ee/logo.png',
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

export interface KmdInfReport {
  year: number;
  month: number;
  companyInfo: CompanyInfo;
  generatedAt: string;
  transactions: Array<{
    partnerName: string;
    partnerRegCode: string;
    invoiceNumber: string;
    invoiceDate: string;
    sumWithoutVat: number;
    vatAmount: number;
  }>;
}

export interface TaxForecast {
  period: string; // '2026-01'
  estimatedVatEur: number;
  estimatedTsdEur: number;
  totalTaxLiabilityEur: number;
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

async function archiveReport(
  type: 'vat_report' | 'annual_report',
  title: string,
  content: string,
  year: number,
  operator: string
): Promise<string> {
  const pool = getOperatorPool();
  const id = crypto.randomUUID();
  const hash = crypto.createHash('sha256').update(content).digest('hex');
  const retentionUntil = new Date();
  retentionUntil.setFullYear(retentionUntil.getFullYear() + 7);

  // In a real system, we would upload 'content' to S3 here
  const s3Bucket = 'apex-financial-reports';
  const s3Key = `${type}/${year}/${id}.json`;

  await pool.query(
    `INSERT INTO financial_documents (
      id, document_type, title, s3_bucket, s3_key, sha256_hash, 
      retention_until, fiscal_year, created_by
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, (SELECT id FROM users WHERE email = $9 OR username = $9 LIMIT 1))`,
    [id, type, title, s3Bucket, s3Key, hash, retentionUntil, year, operator]
  );

  return id;
}

export async function generateAnnualReport(year: number, operator?: string): Promise<AnnualReport> {
  const pool = getReadOnlyPool();
  
  // Use account_balances view to get real financial data
  const balanceResult = await pool.query(`
    SELECT account_code, account_type, balance_cents
    FROM account_balances
  `);

  const balances = balanceResult.rows;

  const getBalance = (type: string) => {
    return balances
      .filter(b => b.account_type === type)
      .reduce((sum, b) => sum + parseInt(b.balance_cents), 0);
  };

  const revenueCents = getBalance('revenue');
  const expensesCents = getBalance('expense');
  const assetsCents = getBalance('asset');
  const liabilitiesCents = getBalance('liability');
  const equityCents = getBalance('equity');
  
  const profitCents = revenueCents - expensesCents;

  const report: AnnualReport = {
    year,
    companyInfo: COMPANY_INFO,
    generatedAt: new Date().toISOString(),
    status: 'draft',
    financials: {
      revenueEur: revenueCents / 100,
      expensesEur: expensesCents / 100,
      profitEur: profitCents / 100,
      assetsEur: assetsCents / 100,
      liabilitiesEur: liabilitiesCents / 100,
      equityEur: equityCents / 100,
    }
  };

  if (operator) {
    await archiveReport(
      'annual_report',
      `Annual Report ${year}`,
      JSON.stringify(report),
      year,
      operator
    );
  }

  return report;
}

export async function generateTsdReport(year: number, month?: number): Promise<TsdReport> {
  const pool = getReadOnlyPool();
  
  let query = `
    SELECT 
      SUM(amount_cents) as total,
      COUNT(DISTINCT customer_id) as employees
    FROM double_entry_ledger
    WHERE EXTRACT(YEAR FROM transaction_date) = $1
    AND transaction_type = 'payout'
  `;
  const params: any[] = [year];

  if (month) {
    query += ` AND EXTRACT(MONTH FROM transaction_date) = $2`;
    params.push(month);
  }

  const result = await pool.query(query, params);
  const totalCents = parseInt(result.rows[0].total || '0');
  const employees = parseInt(result.rows[0].employees || '0');

  return {
    year,
    month,
    companyInfo: COMPANY_INFO,
    generatedAt: new Date().toISOString(),
    employees,
    payrollEur: totalCents / 100,
    dividendsEur: 0, // Separate ledger account would be needed
    taxPayableEur: (totalCents / 100) * 0.25, // Rough estimate (social tax + income tax)
  };
}

/**
 * Generate KMD INF report (VAT attachment for transactions > 1000 EUR)
 */
export async function generateKmdInfReport(year: number, month: number): Promise<KmdInfReport> {
  const pool = getReadOnlyPool();
  const startDate = `${year}-${month.toString().padStart(2, '0')}-01`;
  const endDate = new Date(year, month, 0).toISOString().split('T')[0];

  const sql = `
    SELECT 
      u.company_name as partner_name,
      u.registry_code as partner_reg_code,
      i.invoice_number,
      i.issue_date,
      i.subtotal_cents,
      i.vat_cents
    FROM invoices i
    JOIN users u ON i.customer_id = u.id
    WHERE i.issue_date >= $1 AND i.issue_date <= $2
    AND i.subtotal_cents >= 100000 -- >= 1000 EUR
  `;
  const result = await pool.query(sql, [startDate, endDate]);

  return {
    year,
    month,
    companyInfo: COMPANY_INFO,
    generatedAt: new Date().toISOString(),
    transactions: result.rows.map(row => ({
      partnerName: row.partner_name,
      partnerRegCode: row.partner_reg_code || 'N/A',
      invoiceNumber: row.invoice_number,
      invoiceDate: row.issue_date,
      sumWithoutVat: row.subtotal_cents / 100,
      vatAmount: row.vat_cents / 100
    }))
  };
}

/**
 * Tax Forecasting
 * Estimates future VAT and TSD liabilities
 */
export async function getTaxForecast(): Promise<TaxForecast[]> {
  const now = new Date();
  const forecasts: TaxForecast[] = [];

  for (let i = 1; i <= 3; i++) {
    const targetDate = new Date(now.getFullYear(), now.getMonth() + i, 1);
    const period = targetDate.toISOString().slice(0, 7);
    
    // In a real system, we'd base this on historical averages + current pipeline
    const estimatedVat = 5000 + (i * 500); // Placeholder trend
    const estimatedTsd = 2500; // Placeholder fixed cost
    
    forecasts.push({
      period,
      estimatedVatEur: estimatedVat,
      estimatedTsdEur: estimatedTsd,
      totalTaxLiabilityEur: estimatedVat + estimatedTsd
    });
  }

  return forecasts;
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
