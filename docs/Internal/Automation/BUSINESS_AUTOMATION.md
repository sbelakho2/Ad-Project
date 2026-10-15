# Business Automation Strategy - RivalApexMediation

**Context**: One-person operation in Estonia (Bel Consulting OÜ)  
**Objective**: Full automation of business operations to maintain compliance and enable scale  
**Compliance**: Estonian Accounting Act, VAT Act, e-Residency requirements

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Estonian Legal Requirements](#estonian-legal-requirements)
3. [Financial & Accounting Automation](#financial--accounting-automation)
4. [Sales & Customer Automation](#sales--customer-automation)
5. [Platform Maintenance Automation](#platform-maintenance-automation)
6. [Implementation Roadmap](#implementation-roadmap)
7. [Technology Stack](#technology-stack)
8. [Cost Analysis](#cost-analysis)

---

## Executive Summary

### Business Model
- **Target Market**: Mobile game developers seeking Unity alternatives
- **Pricing**: BYO platform fee tiers — Starter 0%, Growth 2.5%, Scale 2.0%, Enterprise 1.0–1.5% (custom floor)
- **Revenue Model**: Platform fee on gross mediated revenue plus optional add-ons (white-label console, extended retention, migration services)
- **Operator**: Solo entrepreneur via Estonian e-Residency

### Automation Imperatives

As a **one-person operation**, manual processes are not sustainable. Automation is required for:

1. **Legal Compliance**: Estonian quarterly VAT reports, annual financial statements
2. **Customer Acquisition**: Self-service signup without human intervention
3. **Revenue Collection**: Automated billing, payment retry, dunning management
4. **Platform Operations**: SDK releases, documentation updates, customer notifications
5. **Support Scalability**: Self-service docs, automated incident responses

**Goal**: Platform operates 24/7 with minimal human intervention while maintaining full regulatory compliance.

---

## Estonian Legal Requirements

### Accounting Act (Raamatupidamise seadus)

**Key Obligations**:
- Maintain double-entry accounting records
- Prepare annual reports within 6 months of fiscal year end
- Retain documents for 7 years (§ 13)
- Submit annual reports to e-Business Register
- Use EUR as base currency

**Compliance Strategy**:
- Automated double-entry ledger with audit trail
- Annual report auto-generation with XBRL format
- S3 storage with 7-year retention policy and WORM configuration
- e-Business Register API integration for electronic filing
- EUR-based accounting with multi-currency transaction logging

### VAT Act (Käibemaksuseadus)

**Key Obligations**:
- Register for VAT if turnover >€40,000 (immediate registration recommended for B2B)
- File quarterly VAT returns (20th of second month following quarter)
- Standard rate: 20%
- EU B2B reverse charge: 0% with valid VAT ID
- Maintain VAT records for 7 years

**Compliance Strategy**:
- Automated VAT calculation based on customer location and VAT ID validation
- Quarterly report auto-generation with e-MTA API submission
- Real-time VAT liability tracking in PostgreSQL
- Adyen/Paddle integration for automatic VAT handling
- VIES (VAT Information Exchange System) integration for EU validation

### E-Residency Digital Requirements

**Advantages**:
- Digital signatures for all official documents (ID card/Mobile-ID)
- E-Tax Board (e-MTA) online portal for all tax filings
- E-Business Register for company registration and annual reports
- Banking via LHV, Wise, or other e-Residency-friendly banks
- 100% remote operations, no physical presence required

**Implementation**:
- DigiDoc integration for document signing
- E-MTA API for automated tax filing
- E-Business Register API for annual reports
- Banking API integration (Wise/LHV) for payment reconciliation

---

## Financial & Accounting Automation

### System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Customer Payments                         │
│              (Adyen, Paddle, Wire Transfer)                 │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│              Payment Reconciliation Service                  │
│  • Webhook processing   • Bank API sync   • Manual entry    │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│                 Accounting Ledger (PostgreSQL)               │
│  • Double-entry bookkeeping   • VAT tracking   • Audit log  │
└───────┬───────────────────────────────┬─────────────────────┘
        │                               │
        ▼                               ▼
┌──────────────────┐          ┌──────────────────────┐
│  Invoice         │          │  Tax Reporting       │
│  Generator       │          │  Service             │
│  • PDF/XML       │          │  • Quarterly VAT     │
│  • e-Invoicing   │          │  • Annual Report     │
└──────┬───────────┘          └──────┬───────────────┘
       │                              │
       ▼                              ▼
┌──────────────────┐          ┌──────────────────────┐
│  Customer Email  │          │  e-MTA API /         │
│  (SendGrid)      │          │  e-Business Register │
└──────────────────┘          └──────────────────────┘
```

### Component Specifications

#### 1. Payment Reconciliation Service

**Purpose**: Automatically match incoming payments with customer invoices.

**Data Sources**:
- Adyen webhooks (`invoice.paid`, `charge.succeeded`, `payment_intent.succeeded`)
- Paddle webhooks (`subscription_payment_succeeded`, `payment_succeeded`)
- Bank API (Wise/LHV for wire transfers)

**Process Flow**:
1. Webhook received → validate signature → parse payload
2. Extract: customer ID, amount, currency, payment date, transaction ID
3. Convert to EUR using ECB exchange rate API (if non-EUR)
4. Match payment to open invoice (by customer ID + amount + ±2% tolerance)
5. Update invoice status in database
6. Create accounting ledger entries (double-entry)
7. Send receipt email to customer
8. Update Adyen metadata for audit trail

**Database Schema**:
```sql
CREATE TABLE payments (
  id UUID PRIMARY KEY,
  customer_id UUID REFERENCES customers(id),
  invoice_id UUID REFERENCES invoices(id),
  adyen_payment_id TEXT,
  paddle_payment_id TEXT,
  amount_cents INTEGER NOT NULL,
  currency TEXT NOT NULL,
  amount_eur_cents INTEGER NOT NULL, -- Converted to EUR
  exchange_rate DECIMAL(10, 6),
  payment_method TEXT, -- card, sepa_debit, wire_transfer
  status TEXT, -- pending, completed, failed, refunded
  metadata JSONB,
  created_at TIMESTAMPTZ DEFAULT NOW(),
  reconciled_at TIMESTAMPTZ
);

CREATE TABLE ledger_entries (
  id UUID PRIMARY KEY,
  payment_id UUID REFERENCES payments(id),
  account TEXT NOT NULL, -- revenue, vat_liability, receivables, etc.
  debit_cents INTEGER,
  credit_cents INTEGER,
  description TEXT,
  created_at TIMESTAMPTZ DEFAULT NOW()
);
```

**Implementation**:
```typescript
// backend/services/accounting/PaymentReconciliationService.ts
import Adyen from 'adyen';
import { PaddleWebhook } from '@paddle/paddle-node-sdk';

export class PaymentReconciliationService {
  async handleAdyenWebhook(event: Adyen.Event) {
    switch (event.type) {
      case 'invoice.paid':
        await this.recordPayment({
          customerId: event.data.object.customer as string,
          amount: event.data.object.amount_paid,
          currency: event.data.object.currency,
          adyenPaymentId: event.data.object.payment_intent as string,
        });
        break;
      // ... other events
    }
  }

  async recordPayment(data: PaymentData) {
    // 1. Convert to EUR if needed
    const amountEur = await this.convertToEur(data.amount, data.currency);
    
    // 2. Find matching invoice
    const invoice = await this.findInvoice(data.customerId, amountEur);
    
    // 3. Create payment record
    const payment = await db.payments.create({
      customerId: data.customerId,
      invoiceId: invoice.id,
      amountCents: data.amount,
      currency: data.currency,
      amountEurCents: amountEur,
      status: 'completed',
    });
    
    // 4. Create ledger entries (double-entry)
    await this.createLedgerEntries(payment, invoice);
    
    // 5. Update invoice status
    await db.invoices.update(invoice.id, { status: 'paid', paidAt: new Date() });
    
    // 6. Send receipt
    await this.emailService.sendReceipt(data.customerId, invoice);
  }

  async createLedgerEntries(payment: Payment, invoice: Invoice) {
    const vatAmount = invoice.amountCents * 0.20; // 20% VAT
    const netAmount = invoice.amountCents - vatAmount;

    // Debit: Bank account (asset increases)
    await db.ledgerEntries.create({
      paymentId: payment.id,
      account: 'bank:adyen',
      debitCents: payment.amountEurCents,
      creditCents: null,
      description: `Payment received from ${invoice.customerName}`,
    });

    // Credit: Revenue (income increases)
    await db.ledgerEntries.create({
      paymentId: payment.id,
      account: 'revenue:subscriptions',
      debitCents: null,
      creditCents: netAmount,
      description: `Subscription revenue - ${invoice.customerName}`,
    });

    // Credit: VAT liability (liability increases)
    await db.ledgerEntries.create({
      paymentId: payment.id,
      account: 'liabilities:vat_payable',
      debitCents: null,
      creditCents: vatAmount,
      description: `VAT collected - ${invoice.customerName}`,
    });
  }
}
```

#### 2. Invoice Generator

**Purpose**: Automatically generate invoices with e-invoicing support.

**Triggers**:
- Subscription renewal (monthly billing cycle)
- Usage overage (impression threshold exceeded)
- Manual invoice creation (Enterprise custom contracts)

**Output Formats**:
- PDF (human-readable, email attachment)
- XML (e-invoicing standard: UBL 2.1 or Peppol BIS Billing 3.0)

**Required Fields** (Estonian e-invoicing):
- Seller: Company name, registration number, VAT number, address
- Buyer: Company name (if B2B), VAT number (if EU B2B), address
- Invoice number (sequential, no gaps)
- Issue date
- Due date (typically 14 days)
- Line items with VAT breakdown
- Payment reference number
- Bank details (IBAN, SWIFT)

**Implementation**:
```typescript
// backend/services/accounting/InvoiceGeneratorService.ts
import PDFDocument from 'pdfkit';
import { XMLBuilder } from 'fast-xml-parser';

export class InvoiceGeneratorService {
  async generateInvoice(subscription: Subscription) {
    // 1. Calculate amounts
    const lineItems = this.calculateLineItems(subscription);
    const subtotal = lineItems.reduce((sum, item) => sum + item.amount, 0);
    const vatRate = this.getVatRate(subscription.customer);
    const vatAmount = subtotal * vatRate;
    const total = subtotal + vatAmount;

    // 2. Create invoice record
    const invoice = await db.invoices.create({
      invoiceNumber: await this.getNextInvoiceNumber(),
      customerId: subscription.customerId,
      subscriptionId: subscription.id,
      issueDate: new Date(),
      dueDate: addDays(new Date(), 14),
      subtotalCents: Math.round(subtotal * 100),
      vatCents: Math.round(vatAmount * 100),
      totalCents: Math.round(total * 100),
      currency: 'EUR',
      status: 'issued',
      lineItems,
    });

    // 3. Generate PDF
    const pdfBuffer = await this.generatePDF(invoice);
    
    // 4. Generate XML (e-invoicing)
    const xmlContent = await this.generateXML(invoice);

    // 5. Store in S3 with 7-year retention
    await this.storageService.upload(`invoices/${invoice.invoiceNumber}.pdf`, pdfBuffer);
    await this.storageService.upload(`invoices/${invoice.invoiceNumber}.xml`, xmlContent);

    // 6. Email to customer
    await this.emailService.sendInvoice(subscription.customer.email, {
      invoiceNumber: invoice.invoiceNumber,
      pdfAttachment: pdfBuffer,
    });

    return invoice;
  }

  getVatRate(customer: Customer): number {
    // Estonia: 20% standard rate
    // EU B2B reverse charge: 0% (if valid VAT ID)
    // Non-EU: 0% (out of scope)
    if (customer.country === 'EE') return 0.20;
    if (customer.country in EU_COUNTRIES && customer.vatId) return 0.00;
    return 0.00; // Non-EU
  }

  async generatePDF(invoice: Invoice): Promise<Buffer> {
    const doc = new PDFDocument({ size: 'A4', margin: 50 });
    const chunks: Buffer[] = [];
    doc.on('data', chunk => chunks.push(chunk));

    // Header
    doc.fontSize(20).text('INVOICE', { align: 'right' });
    doc.fontSize(10).text(`Invoice #: ${invoice.invoiceNumber}`, { align: 'right' });
    doc.text(`Date: ${invoice.issueDate.toLocaleDateString('et-EE')}`, { align: 'right' });

    // Seller info
    doc.fontSize(12).text('Bel Consulting OÜ', 100, 100);
    doc.fontSize(10).text('Registry code: 12345678');
    doc.text('VAT number: EE123456789');
    doc.text('Address: Tallinn, Estonia');

    // Buyer info
    doc.text(`Bill To:`, 100, 200);
    doc.text(invoice.customer.companyName || invoice.customer.name);
    doc.text(invoice.customer.address);
    if (invoice.customer.vatId) doc.text(`VAT ID: ${invoice.customer.vatId}`);

    // Line items table
    // ... (table rendering)

    // Totals
    doc.text(`Subtotal: €${(invoice.subtotalCents / 100).toFixed(2)}`, { align: 'right' });
    doc.text(`VAT (${invoice.vatRate * 100}%): €${(invoice.vatCents / 100).toFixed(2)}`, { align: 'right' });
    doc.fontSize(12).text(`Total: €${(invoice.totalCents / 100).toFixed(2)}`, { align: 'right' });

    // Payment info
    doc.fontSize(10).text(`Payment due: ${invoice.dueDate.toLocaleDateString('et-EE')}`);
    doc.text(`IBAN: EE123456789012345678`);
    doc.text(`Reference: ${invoice.paymentReference}`);

    doc.end();
    return Buffer.concat(await new Promise(resolve => doc.on('end', () => resolve(chunks))));
  }

  async generateXML(invoice: Invoice): Promise<string> {
    // UBL 2.1 format for e-invoicing
    const builder = new XMLBuilder({ format: true });
    return builder.build({
      Invoice: {
        '@_xmlns': 'urn:oasis:names:specification:ubl:schema:xsd:Invoice-2',
        ID: invoice.invoiceNumber,
        IssueDate: invoice.issueDate.toISOString().split('T')[0],
        DueDate: invoice.dueDate.toISOString().split('T')[0],
        AccountingSupplierParty: {
          Party: {
            PartyName: { Name: 'Bel Consulting OÜ' },
            PartyTaxScheme: { CompanyID: 'EE123456789' },
          },
        },
        AccountingCustomerParty: {
          Party: {
            PartyName: { Name: invoice.customer.companyName },
            PartyTaxScheme: { CompanyID: invoice.customer.vatId || '' },
          },
        },
        LegalMonetaryTotal: {
          TaxExclusiveAmount: (invoice.subtotalCents / 100).toFixed(2),
          TaxInclusiveAmount: (invoice.totalCents / 100).toFixed(2),
          PayableAmount: (invoice.totalCents / 100).toFixed(2),
        },
        // ... full UBL structure
      },
    });
  }
}
```

#### 3. Tax Reporting Service

**Purpose**: Automate quarterly VAT reports and annual financial statements.

**Quarterly VAT Report** (Due: 20th of 2nd month after quarter end):
- Total sales (by VAT rate: 0%, 20%)
- Total VAT collected
- Purchases and input VAT (deductible)
- Net VAT payable/refundable

**Annual Financial Statement** (Due: 6 months after fiscal year end):
- Balance sheet (assets, liabilities, equity)
- Income statement (revenue, expenses, profit/loss)
- Cash flow statement
- Notes to financial statements
- XBRL format for e-Business Register submission

**Implementation**:
```typescript
// backend/services/accounting/TaxReportingService.ts
import axios from 'axios';

export class TaxReportingService {
  async generateQuarterlyVATReport(year: number, quarter: number) {
    const startDate = new Date(year, (quarter - 1) * 3, 1);
    const endDate = new Date(year, quarter * 3, 0);

    // Query ledger for VAT transactions
    const vatCollected = await db.ledgerEntries.aggregate({
      account: 'liabilities:vat_payable',
      startDate,
      endDate,
      operation: 'SUM',
      field: 'creditCents',
    });

    const vatPaid = await db.ledgerEntries.aggregate({
      account: 'assets:vat_receivable',
      startDate,
      endDate,
      operation: 'SUM',
      field: 'debitCents',
    });

    const netVAT = vatCollected - vatPaid;

    // Generate report
    const report = {
      year,
      quarter,
      period: `${startDate.toISOString().split('T')[0]} to ${endDate.toISOString().split('T')[0]}`,
      vatCollected: vatCollected / 100,
      vatPaid: vatPaid / 100,
      netVAT: netVAT / 100,
      dueDate: this.calculateVATDueDate(year, quarter),
    };

    // Save to database
    await db.vatReports.create(report);

    // Submit to e-MTA (if enabled)
    if (process.env.AUTO_SUBMIT_VAT === 'true') {
      await this.submitToEMTA(report);
    }

    return report;
  }

  async submitToEMTA(report: VATReport) {
    // e-MTA X-Road API integration
    // Requires Estonian ID card or Mobile-ID for digital signature
    const xRoadClient = new XRoadClient({
      securityServer: 'emta.x-road.eu',
      clientId: 'EE/COM/12345678/rivalapexmediation',
      serviceId: 'EE/GOV/70000740/emta/vat-filing',
      userId: process.env.ESTONIAN_PERSONAL_CODE,
    });

    const response = await xRoadClient.call('submitVATReturn', {
      period: `${report.year}Q${report.quarter}`,
      vatPayable: report.netVAT,
      // ... full report structure per e-MTA API spec
    });

    if (response.success) {
      await db.vatReports.update(report.id, { 
        submittedAt: new Date(),
        submissionId: response.id,
      });
    }
  }

  calculateVATDueDate(year: number, quarter: number): Date {
    // VAT due 20th of 2nd month after quarter end
    const quarterEndMonth = quarter * 3;
    const dueMonth = (quarterEndMonth + 2) % 12 || 12;
    const dueYear = quarterEndMonth + 2 > 12 ? year + 1 : year;
    return new Date(dueYear, dueMonth - 1, 20);
  }

  async generateAnnualReport(year: number) {
    // Balance sheet
    const assets = await this.calculateAssets(year);
    const liabilities = await this.calculateLiabilities(year);
    const equity = assets - liabilities;

    // Income statement
    const revenue = await this.calculateRevenue(year);
    const expenses = await this.calculateExpenses(year);
    const netIncome = revenue - expenses;

    // Generate XBRL
    const xbrl = this.generateXBRL({
      year,
      assets,
      liabilities,
      equity,
      revenue,
      expenses,
      netIncome,
    });

    // Store report
    await this.storageService.upload(`annual-reports/${year}.xbrl`, xbrl);

    // Submit to e-Business Register (manual approval required)
    return { year, assets, liabilities, equity, revenue, expenses, netIncome };
  }
}
```

#### 4. Document Retention System

**Requirements**:
- 7-year retention (§ 13 Estonian Accounting Act)
- Tamper-evident storage
- Audit trail for all document access
- Disaster recovery with cross-region replication

**Implementation**:
```typescript
// AWS S3 with Object Lock (WORM mode)
const s3 = new AWS.S3();

await s3.putObject({
  Bucket: 'rivalapexmediation-accounting-records',
  Key: `invoices/2024/INV-001234.pdf`,
  Body: pdfBuffer,
  ObjectLockMode: 'COMPLIANCE',
  ObjectLockRetainUntilDate: new Date(Date.now() + 7 * 365 * 24 * 60 * 60 * 1000), // 7 years
  Metadata: {
    'document-type': 'invoice',
    'customer-id': customerId,
    'generated-at': new Date().toISOString(),
    'hash': crypto.createHash('sha256').update(pdfBuffer).digest('hex'),
  },
});

// S3 Lifecycle Policy (set on bucket)
// - Transition to Glacier after 1 year (cost savings)
// - Delete after 7 years (compliance)
```

---

## Sales & Customer Automation

### Self-Service Signup Flow

```
┌─────────────────────────────────────────────────────────────┐
│  1. Landing Page → Choose Plan (Starter/Growth/Scale/Enterprise) │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  2. Signup Form: Email, Company, VAT ID (optional)          │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  3. Adyen Checkout: Credit card, billing address           │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  4. Automated Provisioning:                                  │
│     • Create customer record in database                     │
│     • Generate API key (UUID v4)                            │
│     • Send welcome email with SDK download links            │
│     • Redirect to dashboard with onboarding checklist       │
└─────────────────────────────────────────────────────────────┘
```

### Pricing Tiers

| Tier | Gross Mediated Revenue (Monthly) | Platform Fee | Support | Core Inclusions |
|------|----------------------------------|--------------|---------|-----------------|
| Starter | $0 – $10k | 0% (free) | Community / email | All SDKs, core analytics, baseline debugger |
| Growth | $10,001 – $100k | 2.5% | Email + shared Slack channel | Everything in Starter plus Migration Studio, adapter/SLO observability |
| Scale | $100,001 – $500k | 2.0% | Priority Slack, named TAM | Everything in Growth plus custom exports, early fraud/ML features |
| Enterprise | $500k+ | 1.0–1.5% + minimum | Dedicated channel, contractual SLA | Everything in Scale plus bespoke compliance and onboarding |

### Platform-Fee Billing

**Metering**:
- SDKs stream **mediated revenue** events (per network payout) into ClickHouse.
- Usage service aggregates IVT-adjusted revenue per customer per day.
- Nightly job syncs totals into billing so invoices apply the correct platform-fee percentage.

**Fee Calculation**:
- `platform_fee = mediated_revenue * plan.platform_fee_percent` (Enterprise overrides via contract record).
- Negative adjustments (clawbacks) roll forward before applying the fee to prevent gaming.
- Add-ons (white-label console, extended data retention, premium support) show up as separate invoice lines—not bundled percentages.

**Implementation**:
```typescript
// backend/services/billing/RevenueMeteringService.ts
export class RevenueMeteringService {
  async recordRevenue(event: RevenueEvent) {
    await this.clickhouse.insert(
      'INSERT INTO mediated_revenue (customer_id, usd_amount, source_network, ivt_adjusted, occurred_at) VALUES (?, ?, ?, ?, ?)',
      [event.customerId, event.amountUsd, event.network, event.ivtAdjusted, event.occurredAt]
    );
  }

  async syncPlatformFees() {
    const customers = await db.customers.findAll();

    for (const customer of customers) {
      const revenue = await this.getCurrentMonthRevenue(customer.id);
      if (!revenue) continue;

      const { feePercent, minimumFeeUsd } = this.resolvePlan(customer.planTier, customer.enterpriseOverrides);
      const platformFee = Math.max(revenue * feePercent, minimumFeeUsd || 0);

      await this.billingClient.upsertUsageRecord({
        customerId: customer.id,
        amountUsd: revenue,
        platformFeeUsd: platformFee,
        effectiveRate: platformFee / revenue,
      });
    }
  }

  private resolvePlan(tier: PlatformTier, overrides?: EnterpriseOverrides) {
    if (overrides?.customRatePercent) {
      return { feePercent: overrides.customRatePercent, minimumFeeUsd: overrides.minimumFeeUsd };
    }

    return {
      starter: { feePercent: 0, minimumFeeUsd: 0 },
      growth: { feePercent: 0.025, minimumFeeUsd: 0 },
      scale: { feePercent: 0.02, minimumFeeUsd: 0 },
      enterprise: { feePercent: 0.0125, minimumFeeUsd: 5000 },
    }[tier];
  }
}
```

### Dunning Management

**Failed Payment Flow**:
1. Day 0: Payment fails → Adyen sends `invoice.payment_failed` webhook
2. Day 0: Email #1 "Payment failed - please update your card"
3. Day 3: Retry #1 → Email #2 if still failed
4. Day 5: Retry #2 → Email #3 "Your account will be suspended in 2 days"
5. Day 7: Retry #3 → If still failed, suspend account (disable API key)
6. Day 14: Send final email "Your account has been closed due to non-payment"

**Implementation**:
```typescript
// backend/services/billing/DunningService.ts
export class DunningService {
  async handlePaymentFailed(invoice: Adyen.Invoice) {
    const customer = await db.customers.findByAdyenId(invoice.customer as string);

    // Update failed payment count
    await db.customers.update(customer.id, {
      failedPayments: customer.failedPayments + 1,
      lastFailedPaymentAt: new Date(),
    });

    // Schedule retry attempts
    await this.scheduleRetry(customer.id, 3); // Day 3
    await this.scheduleRetry(customer.id, 5); // Day 5
    await this.scheduleRetry(customer.id, 7); // Day 7

    // Send initial email
    await this.emailService.send(customer.email, 'payment-failed', {
      invoiceUrl: invoice.hosted_invoice_url,
      amount: invoice.amount_due / 100,
    });
  }

  async scheduleRetry(customerId: string, daysFromNow: number) {
    await this.queue.add('retry-payment', { customerId }, {
      delay: daysFromNow * 24 * 60 * 60 * 1000,
    });
  }

  async retryPayment(customerId: string) {
    const customer = await db.customers.findById(customerId);
    
    // Attempt to charge via Adyen
    const paymentIntent = await this.adyen.paymentIntents.create({
      customer: customer.adyenCustomerId,
      amount: customer.outstandingBalanceCents,
      currency: 'eur',
      payment_method: customer.defaultPaymentMethod,
      off_session: true,
      confirm: true,
    });

    if (paymentIntent.status === 'succeeded') {
      // Payment successful
      await db.customers.update(customerId, { failedPayments: 0 });
      await this.emailService.send(customer.email, 'payment-successful', {});
    } else {
      // Still failed
      const retryCount = customer.failedPayments;
      if (retryCount >= 3) {
        // Suspend account after 3 failed attempts
        await this.suspendAccount(customerId);
      } else {
        await this.emailService.send(customer.email, 'payment-retry-failed', { retryCount });
      }
    }
  }

  async suspendAccount(customerId: string) {
    await db.customers.update(customerId, { 
      status: 'suspended',
      suspendedAt: new Date(),
    });
    await db.apiKeys.update({ customerId }, { active: false });
    await this.emailService.send(customer.email, 'account-suspended', {});
  }
}
```

---

## Platform Maintenance Automation

### SDK Release Automation

**Goal**: Publish SDK updates to all package managers with zero manual steps.

**Workflow**:
```
Git Tag (v1.2.3) → GitHub Actions → Build → Test → Publish to:
  • Maven Central (Android)
  • CocoaPods (iOS)
  • NPM (Unity/Web)
  • GitHub Releases (Unity package)
```

**Implementation** (`.github/workflows/sdk-release.yml`):
```yaml
name: SDK Release

on:
  push:
    tags:
      - 'sdk-v*.*.*'

jobs:
  publish-android:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-java@v4
        with:
          java-version: '17'
      - name: Build Android SDK
        run: cd sdk/android && ./gradlew build
      - name: Publish to Maven Central
        env:
          OSSRH_USERNAME: ${{ secrets.OSSRH_USERNAME }}
          OSSRH_PASSWORD: ${{ secrets.OSSRH_PASSWORD }}
          SIGNING_KEY: ${{ secrets.MAVEN_SIGNING_KEY }}
        run: cd sdk/android && ./gradlew publishToMavenCentral

  publish-ios:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build iOS SDK
        run: cd sdk/ios && swift build
      - name: Publish to CocoaPods
        env:
          COCOAPODS_TRUNK_TOKEN: ${{ secrets.COCOAPODS_TRUNK_TOKEN }}
        run: |
          cd sdk/ios
          pod trunk push RivalApexMediation.podspec

  publish-unity:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Package Unity SDK
        run: |
          cd sdk/unity
          tar -czf RivalApexMediation-${{ github.ref_name }}.unitypackage .
      - name: Publish to NPM (Unity package)
        env:
          NPM_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: |
          cd sdk/unity
          echo "//registry.npmjs.org/:_authToken=$NPM_TOKEN" > .npmrc
          npm publish
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v1
        with:
          files: sdk/unity/RivalApexMediation-${{ github.ref_name }}.unitypackage

  notify-customers:
    needs: [publish-android, publish-ios, publish-unity]
    runs-on: ubuntu-latest
    steps:
      - name: Generate Release Notes
        id: changelog
        run: |
          CHANGELOG=$(git log $(git describe --tags --abbrev=0 HEAD^)..HEAD --pretty=format:"- %s")
          echo "changelog<<EOF" >> $GITHUB_OUTPUT
          echo "$CHANGELOG" >> $GITHUB_OUTPUT
          echo "EOF" >> $GITHUB_OUTPUT
      - name: Send Email to Customers
        env:
          SENDGRID_API_KEY: ${{ secrets.SENDGRID_API_KEY }}
        run: |
          curl -X POST https://api.sendgrid.com/v3/mail/send \
            -H "Authorization: Bearer $SENDGRID_API_KEY" \
            -H "Content-Type: application/json" \
            -d '{
              "personalizations": [{"to": [{"email": "support@apexmediation.ee"}]}],
              "from": {"email": "contact@apexmediation.ee"},
              "subject": "New SDK Release: ${{ github.ref_name }}",
              "content": [{"type": "text/plain", "value": "${{ steps.changelog.outputs.changelog }}"}]
            }'
```

### Breaking Change Detection

**Goal**: Automatically detect API changes that break backward compatibility.

**Tools**:
- Android: `japicmp` (Gradle plugin)
- iOS: `swift-api-digester`
- TypeScript: `@microsoft/api-extractor`

**Implementation**:
```yaml
# .github/workflows/api-compatibility.yml
name: API Compatibility Check

on:
  pull_request:
    paths:
      - 'sdk/**'

jobs:
  check-android:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - name: Check API compatibility
        run: |
          cd sdk/android
          ./gradlew japicmp
          if grep -q "BINARY_INCOMPATIBLE" build/reports/japicmp.txt; then
            echo "::error::Breaking changes detected in Android SDK"
            exit 1
          fi
```

---

## Implementation Roadmap

### Phase 1: Financial Compliance (4-6 weeks)

**Week 1-2: Payment Infrastructure**
- [ ] Adyen integration (webhook handlers, Customer Portal)
- [ ] Paddle integration (for EU VAT handling)
- [ ] Payment reconciliation service
- [ ] Multi-currency conversion with ECB API

**Week 3-4: Invoicing & Ledger**
- [ ] Invoice generator (PDF + XML)
- [ ] Double-entry ledger implementation
- [ ] VAT calculation engine
- [ ] E-invoicing UBL format support

**Week 5-6: Tax Reporting**
- [ ] Quarterly VAT report automation
- [ ] e-MTA API integration (requires Estonian ID card)
- [ ] Annual report generation (XBRL)
- [ ] Document retention system (S3 with Object Lock)

### Phase 2: Sales Automation (3-4 weeks)

**Week 1-2: Self-Service Signup**
- [ ] Signup flow with Adyen Checkout
- [ ] API key provisioning
- [ ] Welcome email automation
- [ ] Customer dashboard

**Week 3-4: Billing & Dunning**
- [ ] Usage metering service
- [ ] Overage billing automation
- [ ] Dunning management (failed payment retries)
- [ ] Subscription upgrade/downgrade

### Phase 3: Platform Automation (2-3 weeks)

**Week 1-2: SDK Release Automation**
- [ ] GitHub Actions workflow for multi-platform publishing
- [ ] Semantic versioning automation
- [ ] Changelog generation
- [ ] Customer notification emails

**Week 2-3: Monitoring & Alerting**
- [ ] Uptime monitoring (UptimeRobot)
- [ ] Error tracking (Sentry)
- [ ] PagerDuty integration
- [ ] Weekly KPI digest

---

## Technology Stack

### Financial & Accounting
- **Payment Processing**: Adyen (credit cards), Paddle (EU VAT handling)
- **Accounting Ledger**: PostgreSQL with double-entry tables
- **Invoice Generation**: PDFKit (PDF), fast-xml-parser (XML/UBL)
- **Tax Filing**: e-MTA X-Road API, e-Business Register API
- **Document Storage**: AWS S3 with Object Lock (WORM)
- **Currency Conversion**: European Central Bank API

### Sales & Customer Management
- **Customer Portal**: Adyen Customer Portal (hosted)
- **Email Automation**: SendGrid or Postmark
- **Usage Metering**: ClickHouse for fast aggregation
- **Subscription Management**: Adyen Subscriptions API
- **CRM**: Custom-built on PostgreSQL (avoid Salesforce overhead)

### Platform Maintenance
- **SDK Publishing**: GitHub Actions, Maven Central, CocoaPods, NPM
- **Monitoring**: UptimeRobot, Sentry, PagerDuty
- **Analytics**: PostHog (open-source alternative to Mixpanel)
- **Status Page**: Statuspage.io or custom-built

---

## Cost Analysis

### Monthly Operational Costs (Estimated)

| Service | Cost | Purpose |
|---------|------|---------|
| AWS (EKS, S3, RDS) | $500-800 | Infrastructure |
| Adyen | 2.9% + €0.30/txn | Payment processing |
| Paddle | 5% + VAT handling | EU compliance |
| SendGrid | $20-50 | Email automation (10k-50k emails/mo) |
| UptimeRobot | $0 (free tier) | Uptime monitoring |
| Sentry | $26 | Error tracking (team plan) |
| PagerDuty | $21 | On-call alerts (starter) |
| Statuspage.io | $29 | Status page |
| **Total** | **~$600-950/mo** | **Excluding payment fees** |

### Break-Even Analysis

**Assumptions**:
- Avg Growth-tier customer mediates ~$50k/mo → platform fee = $1,250 (2.5%).
- Avg Scale-tier customer mediates ~$250k/mo → platform fee = $5,000 (2.0%).
- Billing/collections overhead is ~0.2% of mediated revenue (bank + FX fees).

**Effective net per Growth customer**: ~$1,150.  
**Effective net per Scale customer**: ~$4,500.

**Break-even**: $950 / $1,150 ≈ **1 Growth customer** or **<1 Scale customer**.  
**Target**: Blend of 10 Growth + 5 Scale customers ≈ $36k/mo platform fees → $35k/mo contribution after costs.

---

## Compliance Checklist

### Estonian Legal Requirements

- [ ] Register for VAT with Estonian Tax Board (if turnover >€40k)
- [ ] Set up e-Tax account for online filing
- [ ] Configure company in e-Business Register
- [ ] Obtain Estonian ID card or Mobile-ID for digital signatures
- [ ] Set up business bank account (LHV, Wise, or similar)
- [ ] Implement 7-year document retention system
- [ ] Create accounting policy document
- [ ] Appoint authorized person for tax matters (can be self)

### GDPR Compliance

- [ ] Privacy Policy published and version-controlled
- [ ] Cookie consent banner (if using analytics cookies)
- [ ] Data Processing Agreement templates for Enterprise customers
- [ ] Data export/deletion automation (respond to requests within 30 days)
- [ ] Data breach notification procedure (within 72 hours)
- [ ] DPO appointment (optional for small businesses, but recommended)

### Payment & Financial

- [ ] PCI DSS compliance (handled by Adyen, but document reliance)
- [ ] Anti-Money Laundering (AML) policy (if accepting large payments)
- [ ] Payment terms clearly stated in Terms of Service
- [ ] Refund policy documented
- [ ] Invoice templates comply with EU invoicing directive

---

## Risk Mitigation

### Single Operator Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Personal illness | High | Automate critical operations, set up emergency contacts |
| Burnout | High | Pre-schedule maintenance windows, limit manual work |
| Key person dependency | High | Document all processes, use infrastructure-as-code |
| Time zone issues | Medium | Set clear SLAs (48h email response), use async communication |

### Technical Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Payment processor outage | High | Dual setup (Adyen + Paddle), monitor uptime |
| AWS outage | High | Multi-AZ deployment, daily backups |
| Security breach | Critical | Automated security scanning, regular audits |
| Data loss | Critical | Automated backups (S3 versioning, cross-region replication) |

### Financial Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Churn | High | Monitor usage patterns, proactive customer outreach |
| Payment failures | Medium | Dunning management, clear communication |
| Currency fluctuation | Low | Price in EUR, hedge with forward contracts if needed |
| Tax penalties | Critical | Automate all filings, use accounting software |

---

## Conclusion

**Automation is not optional** for a one-person SaaS operation. Every manual process is a bottleneck and liability. This implementation plan prioritizes:

1. **Legal Compliance**: Estonian tax obligations fully automated
2. **Revenue Collection**: Self-service sales with zero-touch provisioning
3. **Platform Maintenance**: SDK releases and updates without human intervention
4. **Risk Management**: Monitoring and alerting for early problem detection

**Next Steps**:
1. Implement Phase 1 (Financial Compliance) immediately for legal compliance
2. Launch Phase 2 (Sales Automation) before public launch
3. Deploy Phase 3 (Platform Automation) within first 3 months of operations

**Success Criteria**:
- 95%+ of customer interactions require zero human intervention
- 100% tax filing compliance with automated submissions
- <2 hours/week spent on operational tasks
- Platform runs 24/7 with automated alerting for critical issues

---

**Document Version**: 1.0  
**Last Updated**: 2025-11-03  
**Owner**: Bel Consulting OÜ  
**Review Schedule**: Quarterly
