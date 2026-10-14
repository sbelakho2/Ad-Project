# Invoicing & Payments Guide

_Last updated: 2025-11-24_  
_Owner: Finance / Billing Team_  
_Review Cycle: Quarterly or when payment terms change (next review: 2026-02-24)_  
_Status: Active_

> **Billing notice:** Terms described here reflect the current production system and may change with 30 days’ notice. Always review the latest invoice for the definitive amount owed.

Complete guide to payments, invoicing, and financial operations for ApexMediation customers.

## Table of Contents

1. [Payment Schedule](#payment-schedule)
2. [Revenue Sharing](#revenue-sharing)
3. [Invoicing Process](#invoicing-process)
4. [Payment Methods](#payment-methods)
5. [Tax Information](#tax-information)
6. [Currency & Exchange Rates](#currency--exchange-rates)
7. [Disputes & Adjustments](#disputes--adjustments)
8. [Financial Reporting](#financial-reporting)

---

## Payment Schedule

### Monthly Invoice Cycle (NET 30)

We meter your mediated revenue during each calendar month and issue a single platform-fee invoice on the **1st of the following month**. Payment is due **NET 30** regardless of tier.

```
Oct 1–31:       Revenue metered & reconciled
Nov 1:          Invoice INV-2025-11 issued (covers October)
Nov 30:         Payment due (NET 30)
Dec 2–5:        Funds typically reach our Wise account depending on your bank
```

If the balance due for a given period is **below €100**, we automatically roll it forward and display the carried amount on the next invoice. Nothing expires; you simply pay once the cumulative platform fee crosses the threshold.

### Bank & Holiday Considerations

Please initiate wires/ACH transfers early if your due date lands on a holiday or weekend. We treat payments as on-time when your bank confirms the transfer on the first business day following the holiday. Common closures:
- Christmas: Dec 25–26
- New Year: Jan 1
- Easter: Variable (Good Friday, Easter Monday)
- Local/national bank holidays in your jurisdiction

If you anticipate a delay, email **billing@apexmediation.ee** before the due date so we can note the expected settlement date.

---

## Autopay Expectations

- **Starter** — No autopay requirement. Stay on the free tier up to €10k per app per month without adding a payment method. We only prompt for billing details when you opt into Growth or when trailing 30-day revenue crosses the Starter cap.
- **Growth / Scale** — Require an autopay-ready rail stored in Adyen. Accepted rails: credit/debit cards, Apple Pay / Google Pay wallets, ACH direct debit (US), and SEPA direct debit (EU/EEA). We auto-charge on the NET 30 due date, mirroring the Adyen retry schedule (day 0 / 3 / 5 / 7) so mediation never pauses for billing.
- **Enterprise** — Defaults to the same autopay flow. If your procurement rules forbid auto-charges, email billing@apexmediation.ee for an exception; we keep a Adyen payment method on file as a backstop even when invoice + wire terms are approved.

**Notification cadence:**
1. Console banner + email when invoices draft (1st of the month)
2. Email reminder 5 days before autopay (“We will auto-charge €X on DATE unless you update payment details.”)
3. Real-time email + console event when Adyen charges succeed or fail. Failed attempts surface instructions for updating the payment method or requesting Wise wire instructions.

If you prefer to pay via Wise wire every month, contact us to flag the account. We’ll note finance approval and keep the autopay method as a safety net for dunning.

## Revenue Sharing

### Revenue Base Definition

**"Your Monthly Revenue"** = Publisher-recognized, IVT-adjusted revenue after network clawbacks for the period, aggregated at the account level across all apps & platforms (mobile, web, CTV).

**Key points:**
- Invalid traffic (IVT/fraud) is filtered out
- Network clawbacks/adjustments applied
- Negative adjustments carry forward (prevents gaming)
- Aggregated across all your apps and platforms
- Calculated monthly for billing period

### BYO Platform Tiers (Marginal Pricing)

**We operate four Bring-Your-Own (BYO) platform tiers**. Each revenue slice is billed at the rate for its tier, so effective fees fall automatically as you scale. Revenue bands are measured per app per month but consolidated invoices show the blended rate across your organization.

| Tier | Revenue Band (€/mo) | Platform Fee | You Keep | What unlocks |
|------|---------------------|--------------|----------|---------------|
| Starter | €0 – €10,000 | 0% | 100% | Launch checklist, SDK + consent tooling, telemetry starter pack |
| Growth | €10,000 – €100,000 | 2.5% | 97.5% | Migration Studio, debugger replays, observability budgets |
| Scale | €100,000 – €500,000 | 2.0% | 98.0% | Named revenue engineers, fraud tooling, SLO dashboards |
| Enterprise | €500,000+ (plus platform minimum) | Custom 1.0–1.5% | 98.5–99.0% | Dedicated pod, bespoke compliance reviews, contractual SLAs |

**Example at €8,000 revenue (Starter only):**
```
Revenue:                 €8,000.00
Band 1 (€8,000 @ 0%):       -€0.00
────────────────────────────────
Your payment:            €8,000.00
Effective rate:               0%
```

**Example at €25,000 revenue (Starter + partial Growth):**
```
Revenue:                      €25,000.00
Band 1 (€10,000 @ 0%):             -€0.00
Band 2 (€15,000 @ 2.5%):          -€375.00
───────────────────────────────────────
Total ApexMediation fee:           -€375.00
Your payment:                  €24,625.00
Effective rate:                    1.5%
```

**Example at €250,000 revenue (Starter + Growth + Scale):**
```
Revenue:                       €250,000.00
Band 1 (€10,000 @ 0%):               -€0.00
Band 2 (€90,000 @ 2.5%):         -€2,250.00
Band 3 (€150,000 @ 2.0%):        -€3,000.00
────────────────────────────────────────
Total ApexMediation fee:        -€5,250.00
Your payment:                 €244,750.00
Effective rate:                      2.1%
```

**Example at €800,000 revenue (Enterprise add-on):**
```
Revenue:                       €800,000.00
Band 1 (€10,000 @ 0%):               -€0.00
Band 2 (€90,000 @ 2.5%):         -€2,250.00
Band 3 (€400,000 @ 2.0%):        -€8,000.00
Band 4 (€300,000 @ 1.2% custom): -€3,600.00
Platform minimum (if higher): see contract
────────────────────────────────────────
Total ApexMediation fee:       -€13,850.00
Your payment:                 €786,150.00
Effective rate:                      1.73%
```

Enterprise tiers include a negotiated monthly platform minimum to guarantee access to the dedicated pod. We reconcile the custom rate against that minimum automatically on your invoice.

### CTV/Web Video Premium

**CTV/OTT and web video** adds **+2 percentage points** to the BYO rates above because of extra fraud, compliance, and CDN overhead.

| Tier | Standard Rate | CTV/Web Rate |
|------|---------------|--------------|
| Starter | 0% | 2% |
| Growth | 2.5% | 4.5% |
| Scale | 2.0% | 4.0% |
| Enterprise | Custom 1.0–1.5% | Custom + 2.0pp |

**Example at €120,000 CTV revenue:**
```
Revenue:                       €120,000.00
Band 1 (€10,000 @ 2%):             -€200.00
Band 2 (€90,000 @ 4.5%):        -€4,050.00
Band 3 (€20,000 @ 4.0%):          -€800.00
────────────────────────────────────────
Total ApexMediation fee:        -€5,050.00
Your payment:                 €114,950.00
Effective rate:                      4.21%
```

We track CTV/web video revenue automatically. Mixed traffic is segmented so mobile display uses standard rates while CTV/web applies the premium uplift.

---

## Invoicing Process

### Automatic Invoices

**1st of each month:** Invoice generated automatically for previous month's revenue.

**Dashboard → Billing → Invoices**

```
Recent Invoices

┌────────────────┬──────────────┬────────────┬──────────┐
│ Invoice #      │ Period       │ Amount     │ Status   │
├────────────────┼──────────────┼────────────┼──────────┤
│ INV-2025-11    │ Oct 2025     │ $12,345.67 │ Paid ✅  │
│ INV-2025-10    │ Sep 2025     │ $11,567.89 │ Paid ✅  │
│ INV-2025-09    │ Aug 2025     │ $10,894.56 │ Paid ✅  │
│ INV-2025-08    │ Jul 2025     │ $9,456.78  │ Paid ✅  │
└────────────────┴──────────────┴────────────┴──────────┘
```

### Invoice Contents

**Invoice #INV-2025-11-01**

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                   APEX MEDIATION
           Bel Consulting OÜ
       Reg: 16558645 | VAT: EE102569407
         Sepise 4-2, Tallinn, Estonia
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

INVOICE

Invoice #:      INV-2025-11
Issue Date:     2025-11-01
Due Date:       2025-11-30 (NET 30)
Period:         October 1-31, 2025

Bill To:
[Your Company Name]
[Your Company Address]
[VAT Number if applicable]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

REVENUE BREAKDOWN

Ad Revenue (Gross):                    $1,452.42

By Network:
  Google AdMob:        $523.45
  Meta Audience:       $389.67
  Unity Ads:           $312.34
  AppLovin:            $156.78
  Others:              $70.18

By Ad Type:
  Interstitial:        $789.34
  Rewarded Video:      $556.78
  Banner:              $106.30

By Geography:
  United States:       $678.45
  Germany:             $234.56
  United Kingdom:      $189.23
  Canada:              $156.78
  Others:              $193.40

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

CALCULATION (Marginal Tier Pricing)

Gross Ad Revenue:                      $14,523.89

Marginal Tier Breakdown:
  Band 1 ($10,000 @ 0%):                  -$0.00
  Band 2 ($4,523.89 @ 2.5%):            -$113.10

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Total ApexMediation Fee:                      -$113.10
Net Payment:                           $14,410.79
(Effective rate: 0.78%)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

VAT (if applicable):                   $0.00
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
TOTAL DUE:                             $1,234.56
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Payment Status: PAID ✅
Payment Date: 2025-11-30
Payment Method: Default SEPA (Wise Europe SA)
Transaction ID: TXN-202511-ABC123

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Questions? billing@apexmediation.ee
Dashboard: https://console.apexmediation.ee/billing
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### Download Options

**Formats:**
- PDF (default)
- CSV (for accounting software)
- Excel (for manual processing)

**API:**
```bash
# Download invoice PDF
curl "https://api.apexmediation.ee/v1/invoices/INV-2025-11/download?format=pdf" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  > invoice.pdf

# Download invoice CSV
curl "https://api.apexmediation.ee/v1/invoices/INV-2025-11/download?format=csv" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  > invoice.csv
```

---

## Payment Methods

> **Default rails:** Adyen autopay. Growth, Scale, and Enterprise tiers keep a Adyen payment method on file (card/wallet worldwide, ACH in the US, SEPA direct debit in the EU/EEA). We auto-charge on the due date so mediation never pauses for billing. Wise/SEPA/ACH wires are available for finance-approved exceptions and still reference the same invoices.

### Default: Adyen autopay (cards, wallets, ACH, SEPA)

- **Cards & wallets (global):** Visa, Mastercard, AMEX, Apple Pay, Google Pay. Add/update via Console → Billing → Payment Method.
- **ACH direct debit (US):** Provide routing + account number once; Adyen stores it tokenized. Suitable for USD settlements.
- **SEPA direct debit (EU/EEA):** Provide IBAN + mandate. Works for EUR-denominated invoices.
- **Retry cadence:** Adyen attempts immediately on the NET 30 due date, then retries on days 3, 5, and 7 before marking the invoice delinquent. Console + email notifications fire on each attempt.

You can update the autopay rail anytime before the due date. Enterprise customers that prefer manual invoices still keep an autopay method on file as a fallback/dunning rail.

### Manual SEPA wire (Wise Europe SA)

**Best for:** EU/EEA customers with finance policies that forbid autopay. Funds land in hours (SEPA Instant) or 1–2 business days.

```
Beneficiary: Bel Consulting OÜ
Bank: Wise Europe SA
IBAN: EE907700771234567890
BIC/SWIFT: TRWIBEB1XXX
Bank Address: Wise Europe SA, Narva mnt 7, 10117 Tallinn, Estonia
Reference: INV-XXXX (always include your invoice number)
```

### Manual SEB account (EUR)

**Best for:** Public-sector procurement or customers that require local SEB settlement. Enable via support ticket and we will whitelist your organisation.

```
Beneficiary: Bel Consulting OÜ
Bank: SEB Pank AS
IBAN: EE231010010000123456
BIC: EEUHEE2X
Bank Address: Tornimäe 2, 15010 Tallinn, Estonia
Reference: INV-XXXX
```

### Manual ACH (USD)

**Best for:** US entities sending USD. ACH clears in 1–3 US banking days and avoids wire fees.

```
Beneficiary: Bel Consulting OÜ
Bank: Community Federal Savings Bank (via Wise US)
Account #: 9600001234
Routing #: 026073150
Account Type: Checking
SWIFT: CMFGUS33
Reference: INV-XXXX
```

### Secondary: Wise multi-currency link

Use the Wise payment link in your invoice email for instant funding (mid-market FX, 0.41% avg fee). Ideal for CAD/GBP/SGD wires when SEPA/ACH is unavailable.

### Secondary: Adyen hosted payment link

Pay by card (Visa, Mastercard, AMEX) or wallets (Apple Pay, Google Pay). Fees: 2.9% + €0.30 per charge. Access via Console → Billing → “Pay via card/Adyen”.

### Secondary: PayPal

- PayPal email: billing@apexmediation.ee
- Currencies: USD, EUR, GBP
- Fees: 2% network fee (passed through)

PayPal deducts **2.9% + $0.30** from the amount you send. Please **gross up** so that the net amount we receive equals your invoice total.

**Example:**
```
Invoice due:       $1,234.56
PayPal fee:        $36.13 (2.9% + $0.30)
Send this amount:  $1,270.69  (so we net $1,234.56)
```

---

## Tax Information

### VAT (Value Added Tax)

**EU customers with VAT number:**
- **Reverse charge** applies (you pay VAT in your country)
- Provide VAT number in Dashboard → Settings → Billing
- No VAT charged on invoice

**Example invoice (EU B2B):**
```
Net Payment:                $1,234.56
VAT (0% - Reverse Charge):  $0.00
TOTAL:                      $1,234.56

Note: Reverse charge - Article 196 EU VAT Directive
```

**EU customers without VAT number:**
- 20% Estonian VAT applied
- Appears on invoice

**Example invoice (EU B2C):**
```
Net Payment:                $1,234.56
VAT (20%):                  $246.91
TOTAL:                      $1,481.47
```

**Non-EU customers:**
- No VAT charged
- Provide company details for invoice

### Withholding Tax

**Some countries require withholding tax on payments to foreign companies.**

**Your responsibility:**
- Check local tax laws
- Withhold tax if required
- Remit to tax authority
- Provide certificate to ApexMediation

**Our support:**
- We provide W-8BEN-E form (US)
- Tax residence certificate (Estonia)
- Contact billing@apexmediation.ee for tax forms

### Tax Reporting

**Dashboard → Billing → Tax Documents**

**Annual tax summary:**
```
2025 Tax Year Summary

Total Gross Revenue:        $63,456.78
Total ApexMediation Fees:   -$9,518.52
Total Net Payments:         $53,938.26

VAT Charged (if applicable): $0.00

Payments by Quarter:
  Q1: $12,345.67
  Q2: $14,567.89
  Q3: $13,234.56
  Q4: $13,790.14
```

**Export:** CSV, PDF

---

## Currency & Exchange Rates

### Invoice Currency & Reporting

- **Invoices are denominated in EUR**. This keeps accounting simple and matches our Estonian corporate books.
- **Dashboard reporting** can be viewed in EUR, USD, GBP, or a custom currency for planning purposes. Changing the view does **not** change the currency you remit.
- **Exchange rate**: ECB daily mid-market rate captured on the invoice issue date. If you pay in USD, please use your bank’s FX rate when converting so the EUR amount on the invoice is fully settled.

**Example:**
```
Revenue:                 €1,234.56 (October usage)
ECB rate on Nov 1:       1 EUR = 1.09 USD
Invoice displays:        €1,234.56 due (≈ $1,345.67 when converted by your bank)
```

**Dashboard → Settings → Billing → Currency Display** lets you switch the analytics view without changing the amount owed.

### Exchange Rate Transparency

**Dashboard → Billing → Exchange Rates**

```
Recent Exchange Rates

┌─────────────┬──────────┬──────────────┐
│ Date        │ EUR/USD  │ Source       │
├─────────────┼──────────┼──────────────┤
│ 2025-11-04 │ 1.0900   │ ECB          │
│ 2025-11-03 │ 1.0898   │ ECB          │
│ 2025-11-02 │ 1.0905   │ ECB          │
│ 2025-11-01 │ 1.0903   │ ECB          │
└─────────────┴──────────┴──────────────┘
```

**API:**
```bash
curl "https://api.apexmediation.ee/v1/billing/exchange-rates?date=2025-11-04" \
  -H "Authorization: Bearer YOUR_API_KEY"
```

---

## Disputes & Adjustments

### Reporting Discrepancies

**If you notice a discrepancy:**

1. **Check Dashboard:** Dashboard → Analytics → Revenue (compare to invoice)
2. **Email billing@apexmediation.ee** with:
   - Invoice number
   - Expected amount
   - Actual amount
   - Explanation

### Response Time

**Initial response:** Within 24 hours
**Resolution:** Within 5 business days

### Common Reasons for Discrepancies

1. **Invalid Traffic Filtering**
   - We filter fraud/invalid clicks
   - Refunds from ad networks
   - Typically 1-3% of traffic
  - Deducted before platform fee calculation

2. **Currency Exchange**
   - Some networks pay in different currencies
   - Exchange rate fluctuations
   - Check "Revenue Breakdown" in invoice

3. **Revenue Timing**
   - Some networks report delayed
   - Previous week's adjustments
   - Check "Prior Period Adjustments"

4. **Chargebacks & Network Clawbacks**
   - Advertiser disputes charge
   - Network adjustments for invalid traffic
   - Rare (<0.5%)
   - Detailed in invoice notes
   - Negative adjustments carry forward

5. **Channel Mix Classification**
  - Dashboard shows mobile vs. CTV/web video revenue
  - Platform fee applies to total mediated revenue (no separate surcharge)
  - Add-ons, if enabled, are itemized separately

### Adjustments

**If discrepancy confirmed:**

**Credit adjustment:**
```
Invoice #INV-2025-11-02

Net Payment:                $1,234.56
Prior Period Adjustment:    +$265.44
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
TOTAL:                      $1,500.00
```

**Separate payment (large adjustment):**
- Issued within 5 business days
- Separate invoice generated
- Email notification

---

## Financial Reporting

### Revenue Dashboard

**Dashboard → Analytics → Revenue**

```
┌─────────────────────────────────────────────────┐
│  Revenue Overview                               │
├─────────────────────────────────────────────────┤
│  Today:           $45.67                        │
│  This Week:       $234.56                       │
│  This Month:      $4,567.89                     │
│  Last Month:      $4,234.56                     │
│  This Year:       $53,938.26                    │
└─────────────────────────────────────────────────┘

Revenue Trend (Last 30 Days)
  $200 ┤              ╭─╮
  $150 ┤         ╭───╮│ │╭─╮
  $100 ┤    ╭───╮│   ╰╯ ╰╯ ╰╮
   $50 ┤╭───╯   ╰╯           ╰─╮
       └┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─
```

### Revenue Breakdown

**By Network, Geography, Ad Type** - all available in dashboard.

### Export Reports

**Dashboard → Billing → Export Reports**

**Pre-configured reports:**
- Weekly Revenue Summary (CSV)
- Monthly Invoice Summary (Excel)
- Annual Tax Summary (PDF)
- Custom Date Range (Choose format)

**API:**
```bash
curl "https://api.apexmediation.ee/v1/billing/reports?period=30d&format=csv" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  > revenue_report.csv
```

### Accounting Software Integration

**Supported integrations:**
- QuickBooks (via Zapier)
- Xero (via Zapier)
- Sage (CSV import)
- FreshBooks (CSV import)
- Custom (API)

**Setup:** Dashboard → Settings → Integrations

---

## FAQ

### When will I be invoiced?

On the **1st of every month** for the preceding calendar month’s usage. All invoices are **NET 30**, so the due date is the last day of that same month.

### Is there a minimum invoice amount?

Yes—**€100**. If your platform fee for a period is below €100, we roll it forward and show the carryover on the next invoice. Nothing expires.

### Can I change how I pay?

Yes. Go to **Dashboard → Settings → Billing → Payment Method** to request SEPA, ACH, Adyen card, PayPal, or Wise link access. Changes apply to future invoices.

### What if my payment will be late?

Email **billing@apexmediation.ee** before the due date with:
- Invoice number
- Expected settlement date
- Transfer method (SEPA, ACH, etc.)

We’ll note the account to prevent automated reminders.

### Do you charge extra fees?

No hidden fees. You only pay your tier’s platform percentage plus any optional add-ons you explicitly enable. The only surcharge is when you choose PayPal or card rails, where the network fee is passed through.

### Can I download historical invoices?

Yes. **Dashboard → Billing → Invoices** lets you filter by year and download PDF/CSV/Excel copies anytime.

### What happens if I close my account?

We issue a **final invoice** once the last month’s mediation data closes. Pay it via your usual rail, after which we retain billing records for 7 years (per Estonian law) and remove console access.

---

## Support

**Email:** billing@apexmediation.ee
**Dashboard:** https://console.apexmediation.ee/billing
**Phone:** +372 5XXX XXXX (Mon-Fri, 9am-5pm EET)

**Response Times:**
- Payment issues: <24 hours
- Invoice questions: <24 hours
- Tax questions: <48 hours
- General inquiries: <48 hours
