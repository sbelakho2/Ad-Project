# Accounting System Operations Guide

**Company:** Bel Consulting OÜ  
**System:** RivalApexMediation Automated Accounting  
**Version:** 1.0  
**Last Updated:** 2025-11-03

This guide provides step-by-step instructions for operating the automated accounting system on a day-to-day and quarterly basis.

---

## Table of Contents

1. [Daily Operations](#daily-operations)
2. [Monthly Tasks](#monthly-tasks)
3. [Quarterly VAT Filing](#quarterly-vat-filing)
4. [Annual Tasks](#annual-tasks)
5. [Troubleshooting](#troubleshooting)
6. [Emergency Procedures](#emergency-procedures)

---

## 1. Daily Operations {#daily-operations}

### Automated Tasks (No Action Required)

The following happen automatically:

✅ **Payment Processing**
- Adyen webhooks trigger payment reconciliation
- Payments matched to invoices (±2% tolerance)
- Ledger entries created automatically
- Customer invoices updated to 'paid' status

✅ **Invoice Generation**
- Triggered by subscription billing
- PDF and XML generated automatically
- Uploaded to S3 with 7-year retention
- Customer receives email with invoice

✅ **Double-Entry Bookkeeping**
- Every payment creates balanced ledger entries
- Debit: Bank Account (1110)
- Credit: Revenue (4100)
- Credit: VAT Liability (2200) if applicable

### Daily Checks (5 minutes)

**Morning Routine:**

```bash
# 1. Check for failed payments (if any)
psql -h localhost -U rivalapexmediation -d rivalapexmediation -c \
  "SELECT customer_id, amount_eur_cents, failure_code, failure_message, created_at 
   FROM payments 
   WHERE status = 'failed' 
     AND created_at > NOW() - INTERVAL '24 hours';"
```

**If failures found:**
- Check Adyen dashboard for details
- Contact customer if needed
- Update payment method or retry

```bash
# 2. Verify ledger balance (should always be 0)
psql -h localhost -U rivalapexmediation -d rivalapexmediation -c \
  "SELECT 
     SUM(debit_cents) as total_debits,
     SUM(credit_cents) as total_credits,
     SUM(debit_cents) - SUM(credit_cents) as balance
   FROM ledger_entries
   WHERE DATE(created_at) = CURRENT_DATE;"
```

**Expected:** balance = 0  
**If not 0:** Check latest transactions, review logs

```bash
# 3. Check unreconciled payments
psql -h localhost -U rivalapexmediation -d rivalapexmediation -c \
  "SELECT id, customer_id, amount_eur_cents, payment_processor, created_at
   FROM payments
   WHERE reconciled = false
     AND created_at < NOW() - INTERVAL '1 hour';"
```

**If found:** Investigate why invoice matching failed

---

## 2. Monthly Tasks {#monthly-tasks}

### Revenue Review (30 minutes)

**First day of month:**

```sql
-- Monthly revenue summary
SELECT * FROM monthly_revenue 
WHERE month = DATE_TRUNC('month', NOW() - INTERVAL '1 month')
ORDER BY month DESC;

-- Customer lifetime value
SELECT * FROM customer_lifetime_value
ORDER BY total_revenue_cents DESC
LIMIT 20;

-- Top customers last month
SELECT 
  c.name,
  c.email,
  COUNT(i.id) as invoice_count,
  SUM(i.total_cents) as total_revenue_cents,
  SUM(i.total_cents) / 100.0 as total_revenue_eur
FROM invoices i
JOIN customers c ON i.customer_id = c.id
WHERE i.issue_date >= DATE_TRUNC('month', NOW() - INTERVAL '1 month')
  AND i.issue_date < DATE_TRUNC('month', NOW())
  AND i.status = 'paid'
GROUP BY c.id, c.name, c.email
ORDER BY total_revenue_cents DESC
LIMIT 10;
```

### Expense Recording

**As expenses occur:**

```sql
-- Record business expense
INSERT INTO expenses (
  vendor_name,
  description,
  amount_eur_cents,
  currency,
  expense_date,
  category,
  is_vat_deductible,
  vat_cents,
  payment_method,
  receipt_url,
  account_code,
  status
) VALUES (
  'AWS',
  'Infrastructure costs - October 2025',
  20000,  -- €200.00
  'EUR',
  '2025-10-31',
  'infrastructure',
  true,
  4000,  -- €40 VAT (20%)
  'bank_transfer',
  's3://rivalapexmediation-accounting/receipts/2025/10/aws-invoice.pdf',
  '5100',  -- Server & Hosting Costs
  'paid'
);
```

**Common Categories:**
- `infrastructure` - AWS, servers, domains
- `software` - SaaS subscriptions, tools
- `marketing` - Ads, content, SEO
- `professional_services` - Legal, accounting (B2Baltics)
- `office` - Equipment, supplies
- `travel` - Business trips, conferences

**Common Account Codes:**
- 5100: Server & Hosting Costs
- 5200: Software Subscriptions
- 5300: Marketing & Advertising
- 5400: Professional Services
- 5500: Office Expenses
- 5600: Travel & Conferences

### Adyen Fee Reconciliation

**Monthly:**

```bash
# Download Adyen balance transactions
# Adyen Dashboard → Balance → Download CSV
# Then import fees as expenses

# Or use Adyen API
curl https://api.adyen.com/v1/balance_transactions \
  -u "sk_live_...:" \
  -d "created[gte]=$(date -v-1m +%s)" \
  -d "limit=100" \
  -d "type=stripe_fee"
```

```sql
-- Record Adyen fees as expense
INSERT INTO expenses (
  vendor_name,
  description,
  amount_eur_cents,
  currency,
  expense_date,
  category,
  is_vat_deductible,
  vat_cents,
  payment_method,
  account_code,
  status
) VALUES (
  'Adyen',
  'Payment processing fees - October 2025',
  5000,  -- €50.00
  'EUR',
  '2025-10-31',
  'software',
  false,  -- Adyen fees don't include VAT
  0,
  'automatic_deduction',
  '5200',
  'paid'
);
```

---

## 3. Quarterly VAT Filing {#quarterly-vat-filing}

### Timeline

| Quarter | Period | Due Date |
|---------|--------|----------|
| Q1 | Jan-Mar | May 20 |
| Q2 | Apr-Jun | Aug 20 |
| Q3 | Jul-Sep | Nov 20 |
| Q4 | Oct-Dec | Feb 20 (next year) |

### Process (1 hour total)

#### Step 1: Generate VAT Report (Automated - 2 minutes)

**On 1st of 2nd month after quarter:**

```typescript
// In Node.js console or via API endpoint
import { VATReportingService } from './services/accounting/VATReportingService';
import { Pool } from 'pg';

const pool = new Pool({/* db config */});
const vatService = new VATReportingService(pool);

// Generate Q4 2025 report
const reportId = await vatService.generateQuarterlyReport(2025, 4);
console.log(`Report generated: ${reportId}`);

// Check status
const status = await vatService.getReportStatus(2025, 4);
console.log(status);
```

**Or via SQL:**

```sql
-- Manual calculation if needed
SELECT
  SUM(CASE WHEN vat_rate = 0.20 THEN subtotal_cents ELSE 0 END) as sales_20_percent,
  SUM(CASE WHEN vat_rate = 0.00 THEN subtotal_cents ELSE 0 END) as sales_0_percent,
  SUM(vat_cents) as vat_collected
FROM invoices
WHERE issue_date >= '2025-10-01' 
  AND issue_date <= '2025-12-31'
  AND status = 'paid';
```

#### Step 2: Review Report (10 minutes)

```sql
-- Get latest VAT report
SELECT * FROM vat_reports 
WHERE year = 2025 AND quarter = 4;

-- Download PDF
SELECT report_url FROM vat_reports 
WHERE year = 2025 AND quarter = 4;
```

**Review Checklist:**
- ✅ Total sales match expected revenue
- ✅ VAT collected = sales × 20% (for EE customers)
- ✅ 0% rate only for valid EU VAT IDs
- ✅ Purchases have receipts uploaded
- ✅ Net VAT payable looks correct

**Common Issues:**

| Issue | Cause | Fix |
|-------|-------|-----|
| VAT too high | Applied 20% to EU B2B | Check VIES validation |
| Sales too low | Unpaid invoices excluded | Verify payment status |
| Purchases missing | Forgot to record expenses | Add missing expenses |

#### Step 3: Submit to e-MTA (15 minutes)

**Manual Submission (Current Process):**

1. **Download report PDF:**
   ```bash
   # From S3 or database report_url
   aws s3 cp s3://rivalapexmediation-accounting/vat-reports/2025/Q4-VAT-Report.pdf ./
   ```

2. **Login to e-MTA:**
   - Go to https://www.emta.ee
   - Click "Logi sisse" (Log in)
   - Use ID card, Mobile-ID, or Smart-ID

3. **Navigate to VAT return:**
   - E-teenused → Deklaratsioonid → KMD
   - Select period: 2025 Q4

4. **Fill form:**
   - Section 1: Sales
     - 1.1: Domestic 20% = sales_20_percent_cents / 100
     - 1.4: EU B2B (0%) = sales_0_percent_cents / 100
     - 1.6: VAT collected = vat_collected_cents / 100
   
   - Section 2: Purchases
     - 2.1: Domestic purchases = total_purchases_cents / 100
     - 2.4: Deductible VAT = vat_paid_cents / 100
   
   - Section 3: Summary
     - Automatically calculated
     - Net VAT payable or refund

5. **Submit:**
   - Review all values
   - Click "Esita" (Submit)
   - Digital signature applied automatically
   - Note the submission ID

6. **Update database:**
   ```sql
   UPDATE vat_reports 
   SET status = 'submitted',
       submission_id = 'REF_NUMBER_FROM_EMTA',
       submitted_at = NOW()
   WHERE year = 2025 AND quarter = 4;
   ```

#### Step 4: Make Payment (if VAT > 0) (5 minutes)

**If net VAT payable > 0:**

1. **Get payment details from e-MTA submission**

2. **Make bank transfer:**
   - Recipient: Maksu- ja Tolliamet
   - IBAN: EE76 1010 0101 0101 0101
   - Amount: Net VAT payable (from report)
   - Reference: From e-MTA submission
   - Deadline: Same as filing (20th of 2nd month)

3. **Update database:**
   ```sql
   UPDATE vat_reports 
   SET status = 'paid',
       payment_reference = 'BANK_TRANSFER_REF',
       paid_at = NOW()
   WHERE year = 2025 AND quarter = 4;
   ```

**If net VAT payable < 0 (refund):**
- No payment needed
- Refund will be processed by e-MTA
- Usually credited within 30 days

---

## 4. Annual Tasks {#annual-tasks}

### Annual Report (Outsourced to B2Baltics)

**Deadline:** June 30 (for previous calendar year)

**Your Responsibility:**

1. **Export financial data (January):**
   ```sql
   -- Full year income statement data
   SELECT 
     ac.code,
     ac.name,
     ac.category,
     SUM(le.debit_cents) as total_debits,
     SUM(le.credit_cents) as total_credits,
     SUM(le.debit_cents) - SUM(le.credit_cents) as net_cents
   FROM ledger_entries le
   JOIN chart_of_accounts ac ON le.account_code = ac.code
   WHERE DATE(le.created_at) >= '2025-01-01'
     AND DATE(le.created_at) <= '2025-12-31'
   GROUP BY ac.code, ac.name, ac.category
   ORDER BY ac.code;

   -- Export to CSV
   \copy (SELECT ...) TO '/tmp/2025-ledger.csv' CSV HEADER;
   ```

2. **Provide to B2Baltics:**
   - Full ledger export (CSV)
   - All VAT reports (Q1-Q4)
   - Bank statements (if requested)
   - Major expense receipts
   - Any other supporting documents

3. **Review & approve:**
   - B2Baltics prepares XBRL report
   - Review draft for accuracy
   - Approve final version

4. **Sign & submit:**
   - B2Baltics submits to e-Business Register
   - You sign with ID card/Mobile-ID
   - Confirm submission

**Cost:** ~€300-500/year (B2Baltics fee)

### Tax Return (TSD)

**Deadline:** March 31 (for previous calendar year)

**Process:**
- Similar to annual report
- B2Baltics usually handles this too
- Confirms no personal income tax due (OÜ income tax only on dividends)

---

## 5. Troubleshooting {#troubleshooting}

### Payment Not Reconciled

**Symptoms:**
- Payment appears in Adyen
- No matching invoice in database
- Ledger entries not created

**Diagnosis:**
```sql
-- Check unreconciled payments
SELECT * FROM payments 
WHERE reconciled = false 
ORDER BY created_at DESC;

-- Check if invoice exists
SELECT * FROM invoices 
WHERE customer_id = 'cus_xxx';
```

**Solutions:**

1. **Invoice doesn't exist:**
   ```typescript
   // Generate invoice manually
   const invoiceService = new InvoiceGeneratorService(pool);
   await invoiceService.generateInvoice({
     customerId: 'cus_xxx',
     items: [/* ... */],
   });
   ```

2. **Amount mismatch:**
   ```sql
   -- Update invoice amount or manually reconcile
   UPDATE payments 
   SET invoice_id = 'inv_xxx',
       reconciled = true,
       reconciled_at = NOW()
   WHERE id = 'pmt_xxx';
   
   -- Create ledger entries manually
   -- (Use PaymentReconciliationService.createPaymentLedgerEntries)
   ```

### Ledger Out of Balance

**Symptoms:**
```sql
SELECT SUM(debit_cents) - SUM(credit_cents) as balance
FROM ledger_entries;
-- Result: != 0
```

**Diagnosis:**
```sql
-- Find unbalanced transactions
SELECT 
  transaction_id,
  SUM(debit_cents) as debits,
  SUM(credit_cents) as credits,
  SUM(debit_cents) - SUM(credit_cents) as balance
FROM ledger_entries
GROUP BY transaction_id
HAVING SUM(debit_cents) != SUM(credit_cents);
```

**Solutions:**

1. **Incomplete transaction:**
   - Check application logs
   - Look for database transaction rollback
   - Delete incomplete entries and recreate

2. **Manual entry error:**
   - Review manual entries
   - Correct amounts
   - Ensure debit = credit for each transaction

### Webhook Failures

**Symptoms:**
- Adyen shows webhook attempts failed
- Payments not being processed

**Diagnosis:**
```bash
# Check webhook endpoint logs
tail -f /var/log/rivalapexmediation/backend.log | grep Webhook

# Test webhook endpoint
curl -X POST https://yourdomain.com/api/webhooks/adyen \
  -H "Content-Type: application/json" \
  -d '{"type":"ping"}'
```

**Solutions:**

1. **Endpoint unreachable:**
   - Check server status
   - Verify DNS/routing
   - Check firewall rules

2. **Signature verification failing:**
   - Verify STRIPE_WEBHOOK_SECRET in .env
   - Check Adyen dashboard for correct secret
   - Ensure raw body is passed to webhook handler

3. **Processing errors:**
   - Check application logs
   - Verify database connection
   - Review error messages in Adyen dashboard

---

## 6. Emergency Procedures {#emergency-procedures}

### System Down Before VAT Deadline

**Scenario:** Backend is down, VAT due in 2 days

**Action Plan:**

1. **Generate report manually:**
   ```sql
   -- Connect directly to database
   psql -h production-db -U rivalapexmediation

   -- Run manual queries (see Section 3, Step 1)
   SELECT ...
   ```

2. **Export to spreadsheet:**
   - Copy results to Excel/Google Sheets
   - Calculate totals manually
   - Verify against previous quarters

3. **Submit to e-MTA:**
   - Proceed with manual submission (Section 3, Step 3)
   - No need for system-generated PDF

4. **Update database when system recovers**

### Data Loss / Corruption

**Scenario:** Database corruption, recent backup available

**Action Plan:**

1. **Restore from backup:**
   ```bash
   # Restore PostgreSQL backup
   pg_restore -h localhost -U rivalapexmediation -d rivalapexmediation_recovery \
     /backups/rivalapexmediation-2025-11-02.dump
   ```

2. **Verify data integrity:**
   ```sql
   -- Check ledger balance
   SELECT SUM(debit_cents) - SUM(credit_cents) FROM ledger_entries;
   
   -- Check invoice sequence
   SELECT invoice_number FROM invoices ORDER BY invoice_number;
   
   -- Verify payment counts
   SELECT COUNT(*), status FROM payments GROUP BY status;
   ```

3. **Replay missing transactions:**
   - Check Adyen for payments after backup date
   - Manually trigger webhooks for missing payments
   - Verify all transactions reconciled

4. **Contact B2Baltics if close to deadline**

### Incorrect VAT Filed

**Scenario:** Realized error after submitting to e-MTA

**Action Plan:**

1. **Document the error:**
   - Calculate correct amounts
   - Determine difference
   - Prepare explanation

2. **File correction:**
   - Login to e-MTA
   - Deklaratsioonid → KMD → Parandus (Correction)
   - Submit corrected values
   - Add explanation in notes

3. **If underpaid:**
   - Pay difference immediately
   - Include interest if past deadline
   - Reference original submission

4. **If overpaid:**
   - Request refund via e-MTA
   - Or apply to next quarter

5. **Update database:**
   ```sql
   UPDATE vat_reports 
   SET total_sales_cents = [corrected],
       vat_collected_cents = [corrected],
       notes = 'Corrected on [date]: [reason]'
   WHERE year = 2025 AND quarter = 4;
   ```

---

## Quick Reference Card

### Daily (5 min)
- ✅ Check for failed payments
- ✅ Verify ledger balance
- ✅ Check unreconciled payments

### Monthly (30 min)
- 📊 Review revenue
- 📝 Record expenses
- 💳 Reconcile Adyen fees

### Quarterly (1 hour)
- 📄 Generate VAT report (1st of 2nd month)
- 📋 Review & submit to e-MTA (by 20th)
- 💰 Pay VAT if > 0

### Annually (2 hours)
- 📚 Export ledger for B2Baltics (January)
- 📝 Provide supporting documents
- ✍️ Review & sign annual report (by June 30)

### Key Contacts
- **B2Baltics (Accounting):** [email/phone]
- **Adyen Support:** https://support.adyen.com
- **e-MTA Helpdesk:** 880 0811, info@emta.ee

### Emergency SQL Queries
```sql
-- Check system health
SELECT 
  (SELECT COUNT(*) FROM payments WHERE reconciled = false) as unreconciled,
  (SELECT SUM(debit_cents) - SUM(credit_cents) FROM ledger_entries) as balance,
  (SELECT COUNT(*) FROM invoices WHERE status = 'overdue') as overdue;
```

---

**Document Version:** 1.0  
**Last Updated:** 2025-11-03  
**Next Review:** After first quarterly VAT filing (February 2026)
