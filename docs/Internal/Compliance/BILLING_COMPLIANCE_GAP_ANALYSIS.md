# Billing & Compliance Gap Analysis Report

**Entity:** Bel Consulting OÜ  
**Registry Code:** 16192499  
**Address:** Sakala 7-2, EE-10141 Tallinn, Estonia  
**Date:** January 21, 2026  
**Jurisdiction:** Estonian company selling worldwide  

---

## Executive Summary

The ApexMediation billing and compliance system is **65% complete**. Strong foundations exist for invoice generation, document retention, and VAT calculation logic. However, **4 critical blockers** must be resolved before the system is legally compliant for Estonian and EU operations.

| Category | Status | Completion |
|----------|--------|------------|
| Accounting System | ⚠️ Partial | 65% |
| Legal Documentation | ✅ Complete | 95% |
| Compliance UI | ✅ Complete | 90% |
| Customer Data Management | ❌ Incomplete | 30% |
| GDPR Automation | ❌ Incomplete | 25% |

---

## ✅ IMPLEMENTED (What's Working)

### 1. Accounting System (65% Complete)

| Component | Status | Evidence |
|-----------|--------|----------|
| Invoice generation (PDF + XML UBL 2.1) | ✅ | `backend/services/accounting/InvoiceGeneratorService.ts` |
| Sequential invoice numbering | ✅ | `generate_invoice_number()` function with DB lock |
| 7-year document retention | ✅ | S3 Object Lock COMPLIANCE mode (eu-north-1) |
| VAT calculation logic | ✅ | 20% EE domestic, 0% EU B2B reverse charge, 0% non-EU |
| Double-entry bookkeeping | ✅ | `backend/database/migrations/007_accounting_system.sql` |
| Payment reconciliation | ✅ | Adyen webhook integration (5 event types) |
| Dunning management | ✅ | `DunningManagementService.ts` - 3 retry sequence (1, 3, 7 days) |
| Refunds table | ✅ | `refunds` table with full/partial types |
| Credits table | ✅ | `credits` table (referral, goodwill, promotional) |
| Expense tracking | ✅ | `expenses` table with category hierarchy |

### 2. Legal Documentation

| Document | Status | Location |
|----------|--------|----------|
| Terms of Service | ✅ | `website/src/app/terms/page.tsx` |
| Privacy Policy | ✅ | GDPR-compliant with data categories |
| Data Processing Addendum (DPA) | ✅ | `website/src/app/dpa/page.tsx` |
| Subprocessor list | ✅ | Hetzner (hosting), Adyen (billing), Resend (email) |
| Legal entity config | ✅ | `website/src/config/legal.ts` - single source of truth |
| DPA Annex 1 (Processing Description) | ✅ | Data categories, subjects, purposes |
| DPA Annex 2 (Security Measures) | ✅ | TLS, access controls, audit logging |
| DPA Annex 3 (Subprocessors) | ✅ | Transfer safeguards (SCCs) |

### 3. Compliance Settings UI

| Feature | Status | Location |
|---------|--------|----------|
| GDPR toggle | ✅ | `console/src/app/settings/compliance/page.tsx` |
| CCPA toggle | ✅ | California compliance mode |
| COPPA toggle | ✅ | Children's privacy protection |
| Data retention config | ✅ | Configurable days for raw/aggregated/user data |
| Consent provider support | ✅ | OneTrust, Cookiebot, custom providers |
| EU traffic only mode | ✅ | Geographic restriction option |
| Sensitive categories blocking | ✅ | Configurable category list |

### 4. Data Retention Policies

| Data Type | Retention Period | Evidence |
|-----------|------------------|----------|
| Raw ad events | 90 days (configurable) | `docs/DATA_RETENTION_POLICIES.md` |
| Aggregated analytics | 2 years | Configurable via compliance UI |
| Invoices & accounting | 7 years | S3 Object Lock COMPLIANCE |
| Audit logs | 7 years | Legal retention requirement |
| User data | Account lifetime + 30 days | Post-deletion cleanup |

---

## ❌ CRITICAL GAPS (Must Fix Before Launch)

### 1. VAT Registration Missing

**Current State:**
```typescript
// website/src/config/legal.ts line 38
registry: {
  code: '16192499',
  vatNumber: null as string | null,  // ← CRITICAL: Not registered
}
```

**Estonian Law Requirement:**
- Companies with annual turnover > €40,000 MUST register for VAT
- Without registration, you cannot legally charge VAT or issue VAT invoices
- Penalty: Up to €32,000 fine + retroactive VAT liability

**Action Required:**
1. Register for VAT at [emta.ee](https://www.emta.ee/eng)
2. Update `website/src/config/legal.ts` with VAT number (format: `EE123456789`)
3. Update invoice templates to include VAT number

**Priority:** 🔴 CRITICAL  
**Effort:** 1-2 weeks (registration process)

---

### 2. Customer Billing Fields Missing

**Current State** (`backend/migrations/001_initial_schema.sql`):
```sql
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    publisher_id UUID REFERENCES publishers(id),
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ
    -- ❌ NO: billing_address, company_name, tax_id, vat_number, country
);
```

**Required Migration:**
```sql
-- Migration: XXX_add_billing_fields.sql
ALTER TABLE users ADD COLUMN IF NOT EXISTS company_name VARCHAR(255);
ALTER TABLE users ADD COLUMN IF NOT EXISTS billing_address_line1 VARCHAR(255);
ALTER TABLE users ADD COLUMN IF NOT EXISTS billing_address_line2 VARCHAR(255);
ALTER TABLE users ADD COLUMN IF NOT EXISTS billing_city VARCHAR(100);
ALTER TABLE users ADD COLUMN IF NOT EXISTS billing_postal_code VARCHAR(20);
ALTER TABLE users ADD COLUMN IF NOT EXISTS billing_country CHAR(2);  -- ISO 3166-1
ALTER TABLE users ADD COLUMN IF NOT EXISTS vat_number VARCHAR(20);   -- For EU B2B
ALTER TABLE users ADD COLUMN IF NOT EXISTS tax_id VARCHAR(50);       -- For non-EU

-- Index for VAT validation queries
CREATE INDEX IF NOT EXISTS idx_users_vat_number ON users(vat_number) WHERE vat_number IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_users_billing_country ON users(billing_country);
```

**Priority:** 🔴 CRITICAL  
**Effort:** 2-3 days (migration + UI)

---

### 3. No VIES VAT Validation

**Requirement:** For EU B2B reverse charge (0% VAT), Estonian law requires:
1. Customer provides valid EU VAT number
2. Seller verifies against VIES (VAT Information Exchange System)
3. Invoice must state "Reverse charge" per Article 196 VAT Directive

**Current State:** No VIES validation found in codebase

**Required Implementation:**
```typescript
// backend/services/vies/VIESValidationService.ts
import soap from 'soap';

const VIES_WSDL = 'https://ec.europa.eu/taxation_customs/vies/checkVatService.wsdl';

export interface VIESResult {
  valid: boolean;
  name?: string;
  address?: string;
  requestDate: Date;
}

export async function validateVATNumber(
  countryCode: string, 
  vatNumber: string
): Promise<VIESResult> {
  const client = await soap.createClientAsync(VIES_WSDL);
  const result = await client.checkVatAsync({
    countryCode,
    vatNumber: vatNumber.replace(countryCode, ''),
  });
  return {
    valid: result[0].valid,
    name: result[0].name,
    address: result[0].address,
    requestDate: new Date(),
  };
}
```

**Priority:** 🔴 CRITICAL  
**Effort:** 3-5 days (service + caching + UI)

---

### 4. Credit Notes Not Implemented

**Requirement:** For EU partial refunds, you MUST issue a credit note (not just refund the payment).

**Current State:** `refunds` table exists, but NO `credit_notes` table

**Required Schema:**
```sql
CREATE TABLE IF NOT EXISTS credit_notes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    credit_note_number VARCHAR(20) NOT NULL UNIQUE,  -- Sequential: CN-2024-0001
    invoice_id UUID REFERENCES invoices(id),
    customer_id UUID NOT NULL REFERENCES users(id),
    issue_date DATE NOT NULL DEFAULT CURRENT_DATE,
    reason TEXT NOT NULL,  -- 'partial_refund', 'pricing_adjustment', 'service_credit'
    subtotal_cents INTEGER NOT NULL,
    vat_rate NUMERIC(5,4) NOT NULL DEFAULT 0,
    vat_cents INTEGER NOT NULL DEFAULT 0,
    total_cents INTEGER NOT NULL,
    currency CHAR(3) NOT NULL DEFAULT 'EUR',
    pdf_url TEXT,
    xml_url TEXT,  -- UBL credit note
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Sequential numbering function
CREATE OR REPLACE FUNCTION generate_credit_note_number() RETURNS VARCHAR(20) AS $$
DECLARE
  year_part VARCHAR(4);
  seq_num INTEGER;
  result VARCHAR(20);
BEGIN
  year_part := TO_CHAR(CURRENT_DATE, 'YYYY');
  SELECT COALESCE(MAX(
    CAST(SUBSTRING(credit_note_number FROM 'CN-[0-9]{4}-([0-9]+)') AS INTEGER)
  ), 0) + 1
  INTO seq_num
  FROM credit_notes
  WHERE credit_note_number LIKE 'CN-' || year_part || '-%';
  
  result := 'CN-' || year_part || '-' || LPAD(seq_num::TEXT, 4, '0');
  RETURN result;
END;
$$ LANGUAGE plpgsql;
```

**Priority:** 🔴 CRITICAL  
**Effort:** 5-7 days (schema + service + PDF generation)

---

## ⚠️ HIGH PRIORITY GAPS

### 5. GDPR Data Subject Rights - Partial Implementation

| Right | Status | Implementation |
|-------|--------|----------------|
| Right of Access | ⏳ Partial | `/privacy/export` endpoint exists, no UI |
| Right to Rectification | ❌ Missing | No self-service data correction |
| Right to Erasure | ❌ Missing | No deletion automation |
| Right to Portability | ⏳ Partial | Export exists, format not standardized |
| Right to Restrict Processing | ❌ Missing | No processing freeze mechanism |
| Right to Object | ❌ Missing | No objection recording |

**Required:**
- Data Subject Request workflow with 30-day SLA tracking
- Automated data export in machine-readable format (JSON)
- Cascading deletion with audit trail
- Processing restriction flag per user

**Priority:** 🟠 HIGH  
**Effort:** 2-3 weeks

---

### 6. No Contract/Order Acceptance Tracking

**Current State:**
- Terms acceptance UI exists in signup flow
- BUT no database tracking of when/which version accepted

**Required Schema:**
```sql
CREATE TABLE IF NOT EXISTS user_agreements (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id),
    agreement_type VARCHAR(50) NOT NULL,  -- 'terms', 'privacy', 'dpa'
    agreement_version VARCHAR(20) NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ip_address INET,
    user_agent TEXT,
    UNIQUE(user_id, agreement_type, agreement_version)
);

CREATE INDEX idx_user_agreements_user ON user_agreements(user_id);
CREATE INDEX idx_user_agreements_type ON user_agreements(agreement_type, agreement_version);
```

**Priority:** 🟠 HIGH  
**Effort:** 3-5 days

---

### 7. No Proforma Invoices

**Current State:** Only final invoices generated

**Requirement:** For enterprise deals and non-EU customers, proforma invoices expected before payment.

**Priority:** 🟡 MEDIUM  
**Effort:** 2-3 days (extend InvoiceGeneratorService)

---

## 📋 MEDIUM PRIORITY GAPS

### 8. Annual Report Automation: 0%

**Current State:** Manual process (~4 hours/year)

**Requirement:** XBRL format for Estonian Business Register

**Priority:** 🟡 MEDIUM  
**Effort:** 2-3 weeks

---

### 9. X-Road/e-MTA Integration

**Current State:** Not implemented (manual quarterly VAT ~30 min/quarter)

**Priority:** 🟡 MEDIUM  
**Effort:** 3-4 weeks

---

### 10. Mobile-ID/Smart-ID Signing

**Current State:** Not implemented

**Use Case:** Digital contract signing for Estonian customers

**Priority:** 🟢 LOW  
**Effort:** 1-2 weeks

---

## 📊 Compliance Matrix by Market

| Requirement | Estonia | EU B2B | EU B2C | Non-EU |
|-------------|---------|--------|--------|--------|
| VAT Registration | ❌ | ❌ | ❌ | N/A |
| VIES Validation | N/A | ❌ | N/A | N/A |
| Invoice (PDF) | ✅ | ✅ | ✅ | ✅ |
| Invoice (UBL XML) | ✅ | ✅ | ✅ | ✅ |
| Credit Notes | ❌ | ❌ | ❌ | ❌ |
| Customer Tax ID Storage | ❌ | ❌ | ❌ | ❌ |
| Reverse Charge Statement | ⏳ | ⏳ | N/A | N/A |
| 7-Year Retention | ✅ | ✅ | ✅ | ✅ |
| DPA | ✅ | ✅ | N/A | N/A |
| GDPR Rights Automation | ❌ | ❌ | ❌ | N/A |

---

## 🚀 Prioritized Remediation Roadmap

### Phase 1: Critical (Week 1-2)
| Task | Owner | Effort | Blocker? |
|------|-------|--------|----------|
| Register for VAT (emta.ee) | Legal/Finance | 1-2 weeks | Yes |
| Add customer billing fields migration | Backend | 2-3 days | Yes |
| Implement VIES validation service | Backend | 3-5 days | Yes |
| Create credit notes table and service | Backend | 5-7 days | Yes |

### Phase 2: High Priority (Week 3-4)
| Task | Owner | Effort |
|------|-------|--------|
| Terms acceptance tracking | Backend + Frontend | 3-5 days |
| GDPR data subject request workflow | Backend | 1 week |
| Right to erasure implementation | Backend | 1 week |
| Data portability API (JSON format) | Backend | 3-5 days |

### Phase 3: Medium Priority (Month 2)
| Task | Owner | Effort |
|------|-------|--------|
| Proforma invoice support | Backend | 2-3 days |
| Annual report automation (XBRL) | Backend | 2-3 weeks |
| Contract signing integration | Backend | 1-2 weeks |

### Phase 4: Nice-to-Have (Quarter 2)
| Task | Owner | Effort |
|------|-------|--------|
| X-Road integration | Backend | 3-4 weeks |
| Mobile-ID signing | Backend | 1-2 weeks |
| e-MTA automation | Backend | 2-3 weeks |

---

## 💰 Risk Assessment

| Gap | Financial Risk | Legal Risk | Likelihood |
|-----|----------------|------------|------------|
| Missing VAT registration | €32k fine + back-VAT | High | High (if revenue > €40k) |
| No VIES validation | VAT liability on incorrect reverse charge | Medium | Medium |
| No credit notes | Audit findings, manual corrections | Medium | Medium |
| Missing billing fields | Invoice non-compliance | Medium | High |
| GDPR rights automation | €20M or 4% revenue GDPR fine | High | Low-Medium |
| No terms tracking | Contract enforceability issues | Low | Low |

---

## Appendix: Key Files Reference

| Component | File Path |
|-----------|-----------|
| Legal config | `website/src/config/legal.ts` |
| Invoice generator | `backend/services/accounting/InvoiceGeneratorService.ts` |
| VAT reporting | `backend/services/accounting/VATReportingService.ts` |
| Payment reconciliation | `backend/services/accounting/PaymentReconciliationService.ts` |
| Dunning service | `backend/services/billing/DunningManagementService.ts` |
| Accounting schema | `backend/database/migrations/007_accounting_system.sql` |
| Billing compliance schema | `backend/migrations/011_billing_compliance.sql` |
| Transaction logging | `backend/migrations/016_comprehensive_transaction_logging.sql` |
| DPA page | `website/src/app/dpa/page.tsx` |
| Compliance settings | `console/src/app/settings/compliance/page.tsx` |
| Data retention policies | `docs/DATA_RETENTION_POLICIES.md` |
| Accounting status | `docs/Internal/Deployment/ACCOUNTING_IMPLEMENTATION_STATUS.md` |

---

*Report generated: January 21, 2026*  
*Next review: February 21, 2026*
