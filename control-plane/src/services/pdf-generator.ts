import PDFDocument from 'pdfkit';
import { format } from 'date-fns';
import { CompanyInfo, VatReport, AnnualReport, TsdReport, KmdInfReport } from './estonian-compliance';
import * as fs from 'fs';
import * as path from 'path';

function addLogo(doc: PDFKit.PDFDocument) {
  try {
    const logoPath = path.resolve(__dirname, '../../../logo.jpg');
    if (fs.existsSync(logoPath)) {
      doc.image(logoPath, 450, 50, { width: 100 });
    }
  } catch (e) {
    // Ignore logo if it fails
  }
}

export async function generateVatPDF(report: VatReport): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const doc = new PDFDocument({ size: 'A4', margin: 50 });
    const chunks: Buffer[] = [];

    doc.on('data', (chunk) => chunks.push(chunk));
    doc.on('end', () => resolve(Buffer.concat(chunks)));
    doc.on('error', reject);

    // Layout constants
    const col1 = 50;
    const col2 = 300;

    // Logo & Header
    addLogo(doc);
    doc.fontSize(20).font('Helvetica-Bold').text('VAT DECLARATION (KMD)', col1, 50);
    doc.fontSize(10).font('Helvetica').text(`Generated on: ${format(new Date(), 'dd.MM.yyyy HH:mm')}`, col1, 75);

    // Company Info
    doc.fontSize(12).font('Helvetica-Bold').text('Submitter:', col1, 110);
    doc.fontSize(10).font('Helvetica')
      .text(report.companyInfo.name, col1, 130)
      .text(`Registry Code: ${report.companyInfo.registryCode}`, col1, 145)
      .text(`VAT Number: ${report.companyInfo.vatNumber}`, col1, 160)
      .text(report.companyInfo.address, col1, 175);

    // Report Period
    doc.fontSize(12).font('Helvetica-Bold').text('Tax Period:', col2, 110);
    doc.fontSize(10).font('Helvetica')
      .text(`Year: ${report.year}`, col2, 130)
      .text(`Quarter: ${report.quarter}`, col2, 145)
      .text(`Period: ${format(new Date(report.startDate), 'dd.MM.yyyy')} - ${format(new Date(report.endDate), 'dd.MM.yyyy')}`, col2, 160)
      .text(`Due Date: ${format(new Date(report.dueDate), 'dd.MM.yyyy')}`, col2, 175);

    // Financial Table
    const tableTop = 230;
    doc.fontSize(12).font('Helvetica-Bold').text('Financial Summary', col1, tableTop);
    
    let y = tableTop + 30;
    const rowHeight = 25;

    const rows = [
      ['Total Sales (excl. VAT)', `€${report.totalSalesEur.toLocaleString()}`],
      ['VAT Collected (Output VAT)', `€${report.vatCollectedEur.toLocaleString()}`],
      ['Total Purchases (excl. VAT)', `€${report.totalPurchasesEur.toLocaleString()}`],
      ['VAT Paid (Input VAT)', `€${report.vatPaidEur.toLocaleString()}`],
    ];

    rows.forEach(([label, value]) => {
      doc.fontSize(10).font('Helvetica').text(label, col1, y);
      doc.text(value, col2, y, { align: 'right', width: 200 });
      y += rowHeight;
    });

    // Total
    y += 10;
    doc.moveTo(col1, y).lineTo(500, y).stroke();
    y += 15;
    doc.fontSize(12).font('Helvetica-Bold').text('NET VAT PAYABLE', col1, y);
    doc.text(`€${report.netVatPayableEur.toLocaleString()}`, col2, y, { align: 'right', width: 200 });

    // Footer
    doc.fontSize(8).font('Helvetica-Oblique').fillColor('#666')
      .text('This is an automatically generated document for Estonian Tax and Customs Board compliance.', 50, 750, { align: 'center' });

    doc.end();
  });
}

export async function generateAnnualReportPDF(report: AnnualReport): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const doc = new PDFDocument({ size: 'A4', margin: 50 });
    const chunks: Buffer[] = [];

    doc.on('data', (chunk) => chunks.push(chunk));
    doc.on('end', () => resolve(Buffer.concat(chunks)));
    doc.on('error', reject);

    const col1 = 50;
    const col2 = 300;

    addLogo(doc);
    doc.fontSize(20).font('Helvetica-Bold').text('ANNUAL FINANCIAL REPORT', col1, 50);
    doc.fontSize(10).font('Helvetica').text(`Fiscal Year: ${report.year}`, col1, 75);

    // Company Info
    doc.fontSize(12).font('Helvetica-Bold').text('Entity Details:', col1, 110);
    doc.fontSize(10).font('Helvetica')
      .text(report.companyInfo.name, col1, 130)
      .text(`Registry Code: ${report.companyInfo.registryCode}`, col1, 145)
      .text(report.companyInfo.address, col1, 160);

    // Profit & Loss
    let y = 210;
    doc.fontSize(14).font('Helvetica-Bold').text('Profit and Loss Statement', col1, y);
    y += 30;
    
    const plRows = [
      ['Total Revenue', `€${report.financials.revenueEur.toLocaleString()}`],
      ['Total Expenses', `€${report.financials.expensesEur.toLocaleString()}`],
      ['NET PROFIT/LOSS', `€${report.financials.profitEur.toLocaleString()}`],
    ];

    plRows.forEach(([label, value]) => {
      doc.fontSize(10).font(label.includes('NET') ? 'Helvetica-Bold' : 'Helvetica').text(label, col1, y);
      doc.text(value, col2, y, { align: 'right', width: 200 });
      y += 25;
    });

    // Balance Sheet
    y += 30;
    doc.fontSize(14).font('Helvetica-Bold').text('Balance Sheet Snapshot', col1, y);
    y += 30;

    const bsRows = [
      ['Total Assets', `€${report.financials.assetsEur.toLocaleString()}`],
      ['Total Liabilities', `€${report.financials.liabilitiesEur.toLocaleString()}`],
      ['Total Equity', `€${report.financials.equityEur.toLocaleString()}`],
    ];

    bsRows.forEach(([label, value]) => {
      doc.fontSize(10).font('Helvetica').text(label, col1, y);
      doc.text(value, col2, y, { align: 'right', width: 200 });
      y += 25;
    });

    doc.end();
  });
}

export async function generateTsdPDF(report: TsdReport): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const doc = new PDFDocument({ size: 'A4', margin: 50 });
    const chunks: Buffer[] = [];

    doc.on('data', (chunk) => chunks.push(chunk));
    doc.on('end', () => resolve(Buffer.concat(chunks)));
    doc.on('error', reject);

    const col1 = 50;
    const col2 = 300;

    addLogo(doc);
    doc.fontSize(20).font('Helvetica-Bold').text('TSD REPORT (Payroll & Taxes)', col1, 50);
    doc.fontSize(10).font('Helvetica').text(`Period: ${report.year}${report.month ? ' Month ' + report.month : ''}`, col1, 75);

    doc.fontSize(12).font('Helvetica-Bold').text('Employer:', col1, 110);
    doc.fontSize(10).font('Helvetica')
      .text(report.companyInfo.name, col1, 130)
      .text(`Registry Code: ${report.companyInfo.registryCode}`, col1, 145);

    let y = 200;
    const rows = [
      ['Number of Employees', report.employees.toString()],
      ['Total Gross Payroll', `€${report.payrollEur.toLocaleString()}`],
      ['Dividends Declared', `€${report.dividendsEur.toLocaleString()}`],
      ['Estimated Tax Payable', `€${report.taxPayableEur.toLocaleString()}`],
    ];

    doc.end();
  });
}

export async function generateKmdInfPDF(report: KmdInfReport): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const doc = new PDFDocument({ size: 'A4', margin: 50, layout: 'landscape' });
    const chunks: Buffer[] = [];

    doc.on('data', (chunk) => chunks.push(chunk));
    doc.on('end', () => resolve(Buffer.concat(chunks)));
    doc.on('error', reject);

    addLogo(doc);
    doc.fontSize(20).font('Helvetica-Bold').text('KMD INF (VAT Attachment)', 50, 50);
    doc.fontSize(10).font('Helvetica').text(`Period: ${report.year}-${report.month.toString().padStart(2, '0')}`, 50, 75);

    // Header Table
    const tableTop = 120;
    doc.fontSize(10).font('Helvetica-Bold');
    doc.text('Partner Name', 50, tableTop);
    doc.text('Reg. Code', 200, tableTop);
    doc.text('Invoice #', 300, tableTop);
    doc.text('Date', 450, tableTop);
    doc.text('Sum (excl VAT)', 550, tableTop, { align: 'right', width: 100 });
    doc.text('VAT', 670, tableTop, { align: 'right', width: 80 });

    doc.moveTo(50, tableTop + 15).lineTo(750, tableTop + 15).stroke();

    let y = tableTop + 25;
    doc.font('Helvetica').fontSize(9);

    report.transactions.forEach(t => {
      if (y > 500) {
        doc.addPage({ size: 'A4', layout: 'landscape' });
        y = 50;
      }
      doc.text(t.partnerName, 50, y, { width: 140 });
      doc.text(t.partnerRegCode, 200, y);
      doc.text(t.invoiceNumber, 300, y);
      doc.text(format(new Date(t.invoiceDate), 'dd.MM.yyyy'), 450, y);
      doc.text(`€${t.sumWithoutVat.toLocaleString()}`, 550, y, { align: 'right', width: 100 });
      doc.text(`€${t.vatAmount.toLocaleString()}`, 670, y, { align: 'right', width: 80 });
      y += 20;
    });

    doc.end();
  });
}
