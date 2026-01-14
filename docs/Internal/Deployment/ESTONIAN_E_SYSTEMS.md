# Estonian E-Systems Integration Guide

_Last updated: 2025-11-18_  
_Company: Bel Consulting OÜ_  
_Registry Code: 16736399_  
_Owner: Finance / Legal / Platform Engineering_  
_Review Cycle: Annually or when Estonian regulations change (next review: 2026-11-18)_  
_Status: Requires legal review for jurisdiction-specific compliance_

> **FIX-10 governance:** This guide covers Estonia-specific e-government integration. Legal review required before implementing X-Road integrations. For deployment status, see `docs/Internal/Deployment/PROJECT_STATUS.md`.

This document provides comprehensive information about Estonian government e-systems and how to integrate with them for business compliance and automation.

---

## Table of Contents

1. [Overview of Estonian E-Systems](#overview)
2. [X-Road Data Exchange Layer](#x-road)
3. [e-MTA (Tax and Customs Board)](#e-mta)
4. [e-Business Register](#e-business-register)
5. [Digital Signatures](#digital-signatures)
6. [e-Invoicing](#e-invoicing)
7. [Authentication Methods](#authentication-methods)
8. [Integration Architecture](#integration-architecture)
9. [Legal Requirements & Deadlines](#legal-requirements)
10. [Testing & Development](#testing)

---

## 1. Overview of Estonian E-Systems {#overview}

Estonia operates a fully digital government infrastructure. All business interactions with the government are conducted electronically.

### Core E-Government Systems

| System | Purpose | URL | API Access |
|--------|---------|-----|------------|
| **e-MTA** | Tax filing, VAT returns, declarations | https://www.emta.ee | X-Road |
| **e-Business Register** | Company registration, annual reports | https://ariregister.rik.ee | X-Road |
| **e-Residency** | Digital identity for non-residents | https://e-resident.gov.ee | Web portal |
| **e-Tax** | Tax payments, tax account | https://www.emta.ee | X-Road |
| **VIES** | EU VAT number validation | https://ec.europa.eu/vies | REST API |
| **Company Registration Portal** | RIK (Registrite ja Infosüsteemide Keskus) | https://www.rik.ee | X-Road |

### Key Principles

1. **Digital-First:** No paper documents required
2. **Once-Only:** Government systems share data automatically
3. **Secure:** All communications use digital signatures and encryption
4. **Transparent:** You can see who accessed your data and when
5. **Automated:** Many processes happen without human intervention

---

## 2. X-Road Data Exchange Layer {#x-road}

X-Road is Estonia's secure data exchange infrastructure connecting all government and private sector systems.

### What is X-Road?

- **Distributed system:** No central database
- **Secure:** TLS encryption + digital signatures
- **Standardized:** SOAP-based messaging protocol
- **Auditable:** All queries are logged
- **International:** Used by Estonia, Finland, Iceland, and others

### X-Road Architecture

```
┌─────────────────┐         ┌──────────────────┐         ┌─────────────────┐
│  Your System    │◄───────►│  Security Server │◄───────►│  e-MTA System   │
│  (RivalApexMediation) │         │  (X-Road Node)   │         │  (Tax Board)    │
└─────────────────┘         └──────────────────┘         └─────────────────┘
                                     │
                                     │ X-Road Network
                                     │
                            ┌────────▼─────────┐
                            │  Central Server  │
                            │  (Address Book)  │
                            └──────────────────┘
```

### X-Road Integration Options

#### Option A: Self-Hosted Security Server

**Pros:**
- Full control
- No recurring fees for service provider
- Direct connection to X-Road

**Cons:**
- Requires server infrastructure (VM or physical)
- IT expertise for setup and maintenance
- Application process with RIA (Riigi Infosüsteemi Amet)
- Time to setup: 4-8 weeks

**Requirements:**
- Server (min 2 CPU, 4GB RAM, 10GB disk)
- Static IP address
- Domain name
- TLS certificates
- Registration as X-Road member

**Process:**
1. Apply for X-Road membership: https://www.ria.ee/x-tee/liitumine.html
2. Install X-Road Security Server software (Ubuntu/RHEL packages)
3. Configure TLS certificates
4. Register your subsystem
5. Request access to e-MTA services
6. Test in X-Road test environment
7. Get approval for production

**Documentation:**
- Installation Guide: https://github.com/nordic-institute/X-Road/blob/develop/doc/Manuals/ig-ss_x-road_v6_security_server_installation_guide.md
- User Guide: https://github.com/nordic-institute/X-Road/blob/develop/doc/Manuals/ug-ss_x-road_6_security_server_user_guide.md

#### Option B: X-Road Service Provider (RECOMMENDED for Solo Operations)

**Pros:**
- No infrastructure management
- Faster setup (1-2 weeks)
- Professional support
- Managed security updates
- SLA guarantees

**Cons:**
- Monthly fee (~€100-300/month)
- Dependency on provider

**Recommended Providers:**

1. **Aktors OÜ**
   - Website: https://www.aktors.ee
   - Email: info@aktors.ee
   - Services: X-Road hosting, integration development
   - Pricing: From €150/month
   - Support: Estonian and English

2. **Cybernetica AS**
   - Website: https://cyber.ee
   - Email: info@cyber.ee
   - Services: X-Road solutions, custom development
   - Pricing: Custom quotes
   - Note: Developer of X-Road software

3. **Nortal AS**
   - Website: https://nortal.com
   - Email: info@nortal.com
   - Services: Full e-government integration
   - Pricing: Enterprise-focused

**What They Provide:**
- Hosted X-Road Security Server
- REST API gateway (no SOAP required)
- Authentication handling
- Monitoring and logging
- Technical support

#### Option C: Manual Web Portal (Current Approach)

**Pros:**
- Zero setup cost
- No technical integration needed
- Immediate availability

**Cons:**
- Manual work (violates automation goal)
- Prone to human error
- Not scalable
- Time-consuming (~30 min per quarterly VAT return)

**When to Use:**
- Development phase
- Very low transaction volume
- Before X-Road integration is ready

---

## 3. e-MTA (Tax and Customs Board) {#e-mta}

Maksu- ja Tolliamet (MTA) is Estonia's Tax and Customs Board.

### e-MTA System: https://www.emta.ee

### Required Filings for OÜ (Private Limited Company)

| Filing | Frequency | Deadline | Submission Method |
|--------|-----------|----------|-------------------|
| **VAT Return (KMD)** | Quarterly | 20th of 2nd month after quarter | e-MTA portal or X-Road |
| **VAT Payment** | Quarterly | 20th of 2nd month after quarter | Bank transfer |
| **TSD (Tax Return)** | Annual | 31st March | e-MTA portal or X-Road |
| **Income Tax (TuM)** | Monthly (if dividends) | 10th of next month | e-MTA portal or X-Road |
| **Social Tax** | Monthly (if employees) | 10th of next month | e-MTA portal or X-Road |

### VAT Return (Käibemaksdeklaratsioon - KMD)

**Requirement:** All companies registered for VAT must file quarterly returns.

**When You Must Register for VAT:**
- Taxable turnover exceeds €40,000 in 12 months
- OR voluntarily register earlier

**VAT Rates:**
- Standard: 20%
- Reduced: 9% (books, hotels, pharmaceuticals)
- Zero-rated: 0% (intra-EU supply with valid VIES number, exports)

**KMD Form Sections:**

```
Section 1: Sales
├── 1.1: Domestic sales at 20% VAT
├── 1.2: Domestic sales at 9% VAT
├── 1.3: Domestic sales at 0% VAT
├── 1.4: Intra-EU supplies (B2B)
├── 1.5: Exports outside EU
└── 1.6: VAT collected

Section 2: Purchases
├── 2.1: Domestic purchases with VAT
├── 2.2: Intra-EU acquisitions
├── 2.3: Imports
└── 2.4: Deductible VAT

Section 3: Summary
├── 3.1: VAT payable (1.6 - 2.4)
└── 3.2: VAT refund (if 2.4 > 1.6)
```

**Filing Process:**

1. **Manual (Current):**
   - Login to https://www.emta.ee with ID card or Mobile-ID
   - Navigate: E-teenused → Deklaratsioonid → KMD
   - Fill form with data from `vat_reports` table
   - Submit (digital signature applied automatically)
   - Note submission reference
   - Pay VAT by bank transfer if amount > 0

2. **Automated (Future with X-Road):**
   ```typescript
   // Generate report
   const reportId = await vatService.generateQuarterlyReport(2025, 4);
   
   // Submit to e-MTA via X-Road
   await vatService.submitToEMTA(reportId);
   
   // Payment still requires manual bank transfer
   // (No API for direct debits from business accounts)
   ```

**X-Road Service:**
- **Service Code:** `EE/GOV/70000740/emta/kmd`
- **Method:** `submitKMD`
- **Protocol:** SOAP 1.1 with digital signature
- **Documentation:** Available after X-Road registration

### VIES VAT Number Validation

**Purpose:** Verify EU customer VAT numbers for 0% reverse charge

**API:** https://ec.europa.eu/taxation_customs/vies/services/checkVatService

**Usage:**
```typescript
import axios from 'axios';
import { XMLParser } from 'fast-xml-parser';

async function validateEUVAT(vatNumber: string): Promise<boolean> {
  const countryCode = vatNumber.substring(0, 2);
  const vatNumberOnly = vatNumber.substring(2);
  
  const soapRequest = `
    <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" 
                      xmlns:urn="urn:ec.europa.eu:taxud:vies:services:checkVat:types">
      <soapenv:Header/>
      <soapenv:Body>
        <urn:checkVat>
          <urn:countryCode>${countryCode}</urn:countryCode>
          <urn:vatNumber>${vatNumberOnly}</urn:vatNumber>
        </urn:checkVat>
      </soapenv:Body>
    </soapenv:Envelope>
  `;
  
  const response = await axios.post(
    'https://ec.europa.eu/taxation_customs/vies/services/checkVatService',
    soapRequest,
    { headers: { 'Content-Type': 'text/xml' } }
  );
  
  const parser = new XMLParser();
  const data = parser.parse(response.data);
  
  return data['soap:Envelope']['soap:Body']['checkVatResponse']['valid'] === 'true';
}
```

**Important:** Always validate VAT numbers before applying 0% rate!

---

## 4. e-Business Register {#e-business-register}

Äriregister (Business Register) managed by RIK (Centre of Registers and Information Systems).

### Portal: https://ariregister.rik.ee

### Annual Report Requirement

**Legal Basis:** Accounting Act § 24

**Deadline:**
- **Small companies:** 6 months after fiscal year end
- For calendar year companies: **30th June**

**Report Components:**

1. **Annual Report (Majandusaasta aruanne)**
   - Balance Sheet (Bilanss)
   - Income Statement (Kasumiaruanne)
   - Cash Flow Statement (if applicable)
   - Notes to Financial Statements

2. **XBRL Format Required**
   - EU standardized format
   - Machine-readable
   - Must validate against taxonomy

3. **Digital Signature Required**
   - Board member must sign
   - ID card or Mobile-ID

**Submission Process:**

1. **Manual (Current):**
   - Prepare financial statements (accountant or software)
   - Convert to XBRL format
   - Login to https://ariregister.rik.ee
   - Upload XBRL file
   - Sign digitally
   - Submit

2. **Automated (Future):**
   ```typescript
   // Generate annual report from ledger data
   const xbrlReport = await accountingService.generateAnnualReport(2025);
   
   // Submit via X-Road
   await businessRegisterService.submitAnnualReport(xbrlReport);
   
   // Board member signs via Mobile-ID
   await businessRegisterService.signWithMobileID('+372XXXXXXXX');
   ```

**XBRL Taxonomy:**
- Estonian taxonomy: https://www.rik.ee/xbrl
- Based on International Financial Reporting Standards (IFRS)
- Validation tools: https://www.rik.ee/xbrl/validator

**X-Road Service:**
- **Service Code:** `EE/GOV/70001490/rik/ariregister`
- **Method:** `submitAnnualReport`
- **Documentation:** https://www.rik.ee/x-tee

---

## 5. Digital Signatures {#digital-signatures}

All official documents submitted to Estonian government must be digitally signed.

### Signature Methods

#### 1. ID Card (Isikutunnistus)

**For e-Residents:**
- Physical ID card with chip
- Card reader required (~€10-20)
- Desktop software: DigiDoc4 (https://www.id.ee/en/article/install-id-software/)

**Technical Implementation:**
```typescript
// Using Web eID library
import { authenticate, sign } from '@web-eid/web-eid';

// Sign document
const signature = await sign({
  format: 'xml', // or 'pdf', 'bdoc'
  data: documentHash,
  certificateLevel: 'qualified'
});
```

**Pros:**
- Most secure
- Works offline
- Qualified electronic signature (highest legal level)

**Cons:**
- Requires physical card reader
- Not practical for server-side automation

#### 2. Mobile-ID (Mobiil-ID)

**Requirements:**
- Estonian SIM card from Telia, Tele2, or Elisa
- Mobile-ID subscription (~€2-3/month)
- Phone with SIM card

**Technical Implementation:**
```typescript
import axios from 'axios';

// Initiate signing
const response = await axios.post('https://mid.sk.ee/mid-api/signature/hash', {
  relyingPartyUUID: 'YOUR_UUID',
  relyingPartyName: 'Bel Consulting OÜ',
  phoneNumber: '+372XXXXXXXX',
  nationalIdentityNumber: 'PERSONAL_CODE',
  hash: documentHash,
  hashType: 'SHA256',
  language: 'ENG'
});

const sessionId = response.data.sessionID;

// Poll for signature completion
let status = 'RUNNING';
while (status === 'RUNNING') {
  await new Promise(resolve => setTimeout(resolve, 3000));
  const statusResponse = await axios.get(
    `https://mid.sk.ee/mid-api/signature/session/${sessionId}`
  );
  status = statusResponse.data.state;
}

const signature = statusResponse.data.signature.value;
```

**Pros:**
- No card reader needed
- Can be automated
- Works from anywhere

**Cons:**
- Requires Estonian phone number
- Monthly subscription fee
- User must enter PIN on phone

#### 3. Smart-ID

**Requirements:**
- Smart-ID app on smartphone
- One-time registration

**Technical Implementation:**
- Similar to Mobile-ID
- API: https://github.com/SK-EID/smart-id-documentation

**Pros:**
- No physical devices
- Free for users
- Works internationally

**Cons:**
- Lower legal status than ID card
- Not always accepted for business documents

### DigiDoc Format

Estonia uses **DigiDoc** container format for signed documents:

- **BDOC:** Binary container (ZIP-based)
- **ASICE:** Advanced Electronic Signature Container
- **Supports:** Multiple signatures, multiple files

**Libraries:**
- Python: `pydigidoc`
- Node.js: `digidoc-node`
- Java: `digidoc4j`

**Example:**
```typescript
import { Container, Signature } from 'digidoc-node';

// Create container
const container = new Container();
container.addFile('invoice.pdf', invoiceBuffer);

// Sign with Mobile-ID
const signature = await container.signWithMobileID('+372XXXXXXXX', 'PERSONAL_CODE');

// Save
const signedContainer = container.save(); // BDOC file
```

---

## 6. e-Invoicing {#e-invoicing}

### EU E-Invoicing Standards

**Peppol BIS Billing 3.0** (Pan-European Public Procurement OnLine)
- Based on EN 16931 (European Standard)
- Format: UBL 2.1 XML or UN/CEFACT Cross-Industry Invoice
- Required for B2G (Business to Government) in many EU countries
- Increasingly required for B2B

### UBL 2.1 Format

Our `InvoiceGeneratorService.ts` already generates UBL 2.1 compliant XML.

**Key Elements:**
```xml
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2">
  <CustomizationID>urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0</CustomizationID>
  <ProfileID>urn:fdc:peppol.eu:2017:poacc:billing:01:1.0</ProfileID>
  <ID>INV-000001</ID>
  <IssueDate>2025-11-03</IssueDate>
  <DueDate>2025-11-17</DueDate>
  <InvoiceTypeCode>380</InvoiceTypeCode>
  <DocumentCurrencyCode>EUR</DocumentCurrencyCode>
  <!-- ... -->
</Invoice>
```

### Peppol Network

**What is Peppol?**
- Network for exchanging business documents
- Used across EU for e-invoicing
- Requires Peppol Access Point (service provider)

**Peppol Access Point Providers:**
1. **Omniva** (Estonian Post)
   - Website: https://www.omniva.ee
   - Pricing: ~€20-50/month
   
2. **Pagero**
   - Website: https://www.pagero.com
   - International coverage
   
3. **Basware**
   - Website: https://www.basware.com
   - Enterprise-focused

**When to Use Peppol:**
- Selling to large enterprises
- Government contracts
- Countries requiring e-invoicing (Italy, France mandatory)

**Alternative for B2C:**
- Email PDF invoices (current approach)
- Attach UBL XML for accounting software compatibility

---

## 7. Authentication Methods {#authentication-methods}

### Accessing E-Government Services

All Estonian e-government portals require strong authentication:

1. **ID Card**
   - Insert card in reader
   - Enter PIN1 (4-12 digits)
   - Browser certificate authentication

2. **Mobile-ID**
   - Enter phone number
   - Enter PIN1 on phone
   - Wait for confirmation (15 seconds)

3. **Smart-ID**
   - Enter personal code
   - Confirm on smartphone app
   - PIN or biometric

### Web Application Integration

For your application to use Estonian authentication:

**Option A: TARA (Authentication Service)**
- Government service for authentication
- Free for Estonian companies
- OpenID Connect protocol
- Apply at: https://www.ria.ee/tara

**Implementation:**
```typescript
// Redirect to TARA
window.location.href = 
  'https://tara.ria.ee/oidc/authorize?' +
  'client_id=YOUR_CLIENT_ID&' +
  'redirect_uri=https://yourapp.com/callback&' +
  'scope=openid+idcard&' +
  'response_type=code';

// Handle callback
const tokenResponse = await axios.post('https://tara.ria.ee/oidc/token', {
  grant_type: 'authorization_code',
  code: authCode,
  client_id: 'YOUR_CLIENT_ID',
  client_secret: 'YOUR_SECRET'
});

const idToken = jwt.decode(tokenResponse.data.id_token);
// Contains: personal code, name, authentication method
```

**Option B: Web eID**
- JavaScript library for ID card
- Works in browser
- No backend needed
- https://github.com/web-eid/web-eid.js

---

## 8. Integration Architecture {#integration-architecture}

### Recommended Architecture for Solo Operation

```
┌─────────────────────────────────────────────────────────────────┐
│                        RivalApexMediation Backend                      │
│                                                                   │
│  ┌──────────────────┐      ┌─────────────────────────────┐      │
│  │  Accounting      │      │  Payment Reconciliation     │      │
│  │  Services        │◄─────┤  Service                    │      │
│  └────────┬─────────┘      └─────────────────────────────┘      │
│           │                                                       │
│           │ Generate Reports                                     │
│           ▼                                                       │
│  ┌──────────────────┐                                           │
│  │  VAT Reporting   │                                           │
│  │  Service         │                                           │
│  └────────┬─────────┘                                           │
│           │                                                       │
└───────────┼───────────────────────────────────────────────────────┘
            │
            │ Option A: Manual (Current)
            │ Generate PDF → User downloads → User uploads to e-MTA
            │
            │ Option B: Service Provider (Recommended)
            ▼
   ┌────────────────────┐
   │  X-Road Service    │  (e.g., Aktors OÜ)
   │  Provider          │
   │  REST API Gateway  │
   └─────────┬──────────┘
             │
             │ SOAP over X-Road
             ▼
   ┌────────────────────┐
   │  e-MTA System      │
   │  (Tax Board)       │
   └────────────────────┘
```

### Implementation Phases

#### Phase 1: Foundation (COMPLETE)
- ✅ Database schema (007_accounting_system.sql)
- ✅ Payment reconciliation (backend & control-plane)
- ✅ Invoice generation (PDF + XML)
- ✅ VAT calculation logic
- ✅ Automated reporting engine (control-plane)

**Status:** PDF generation for VAT, Annual Reports, and TSD is fully automated. Ready for manual upload to e-MTA/Business Register.

#### Phase 2: Semi-Automated (COMPLETE)
- ✅ Generate VAT reports automatically (control-plane Compliance view)
- ✅ Email notification system (mailer.ts in control-plane)
- ✅ One-click download of pre-filled PDF/reports
- ✅ Manual signature and submission (using Mobile-ID/ID-Card)

**Status:** High-level operator dashboard allows one-click generation and archival.

#### Phase 3: Fully Automated (In Progress)
- ⏳ X-Road service provider contract
- ⏳ REST API integration
- ✅ Mobile-ID signing (planned/scaffolded)
- ⏳ Automated submission via X-Road REST gateway
- ✅ Audit logging of all generated reports (7-year retention verified)

---

## 9. Legal Requirements & Deadlines {#legal-requirements}

### Estonian OÜ Compliance Checklist

| Requirement | Frequency | Deadline | Our System |
|-------------|-----------|----------|------------|
| **Accounting Records** | Continuous | Daily/Weekly | ✅ Automated |
| **Invoice Issuance** | Per transaction | Within 5 days | ✅ Automated |
| **VAT Return (KMD)** | Quarterly | 20th of 2nd month | ⏳ Semi-automated |
| **VAT Payment** | Quarterly | 20th of 2nd month | ⚠️ Manual |
| **Annual Report** | Annual | 6 months after FY | ⏳ TODO |
| **Tax Return (TSD)** | Annual | 31st March | ⏳ TODO |
| **Board Decisions** | As needed | N/A | ⚠️ Manual |
| **General Meeting** | Annual | Within 6 months | ⚠️ Manual |

### 2026 Deadlines (Calendar Year Company)

| Date | Task |
|------|------|
| **2026-01-20** | Q4 2025 VAT return + payment |
| **2026-03-31** | 2025 Tax return (TSD) |
| **2026-04-20** | Q1 2026 VAT return + payment |
| **2026-06-30** | 2025 Annual report |
| **2026-07-20** | Q2 2026 VAT return + payment |
| **2026-10-20** | Q3 2026 VAT return + payment |

### Document Retention Requirements

**Estonian Accounting Act § 13:**
- **7 years** for all accounting documents
- **Permanent** for founding documents
- **10 years** for personnel documents

**Our Implementation:**
- S3 Object Lock (COMPLIANCE mode)
- 7-year retention on all `financial_documents`
- Automatic lifecycle management
- SHA256 integrity verification

---

## 10. Testing & Development {#testing}

### e-MTA Test Environment

**Test Portal:** https://test-www.emta.ee

**Test Access:**
- Register with test ID card or test Mobile-ID
- Available from DigiDoc: https://www.id.ee/en/article/test-environment/

**Test VAT Numbers:**
- Format: EE1234567XX (XX = test variant)
- Always returns valid for testing

### X-Road Test Environment

**Test Instance:** `ee-test`

**Access:**
1. Register test member
2. Install test Security Server
3. Connect to test instance
4. Test services available

**Documentation:**
- https://www.ria.ee/x-tee/testkeskkond.html

### VIES Test

**Test VAT Numbers:**
- Valid: `DE123456789`
- Invalid: `DE000000000`
- Service unavailable: `DE111111111`

**Test Endpoint:** Same as production (always available)

---

## Summary: Status and Next Steps

### Current Status (Jan 2026)

1. **✅ Automated Reporting Engine** (estonian-compliance.ts)
2. **✅ Real Ledger Integration** (account_balances view)
3. **✅ PDF Generation** (pdf-generator.ts using pdfkit)
4. **✅ Report Archival** (7-year retention in financial_documents)
5. **✅ Operator Dashboard** (Compliance view in control-plane)

### Short-term (Q1 2026)

1. **Submit Q4 2025 VAT report manually**
   - Use generated PDF from control-plane.
   - Submit via e-MTA portal by Jan 20th.

2. **Evaluate X-Road service providers**
   - Contact Aktors OÜ for REST gateway quotes.

3. **Complete S3 Upload integration**
   - Move from local buffer to S3 archival with Object Lock.

### Medium-term (6-12 Months)

1. **X-Road Integration**
   - Sign contract with service provider
   - Implement REST API client
   - Test in X-Road test environment
   - Go live with automated VAT submission

2. **Annual Report Automation**
   - Implement XBRL generation
   - Connect to e-Business Register via X-Road
   - Test annual report submission

3. **Mobile-ID Integration**
   - Implement Mobile-ID signing in application
   - Allow automated signing of submissions
   - Test end-to-end flow

### Long-term (12+ Months)

1. **Full Compliance Automation**
   - Automated tax calculations
   - Automated payments (if API available)
   - Quarterly/annual reports fully automated
   - Zero manual intervention

2. **Monitoring & Alerts**
   - Deadline reminders
   - Failed submission alerts
   - Payment confirmation tracking
   - Compliance dashboard

---

## Resources

### Official Estonian Resources

- **RIA (Information System Authority):** https://www.ria.ee
- **e-Estonia:** https://e-estonia.com
- **X-Road:** https://x-road.global
- **DigiDoc:** https://www.id.ee/en
- **e-MTA:** https://www.emta.ee
- **Business Register:** https://ariregister.rik.ee

### Developer Resources

- **X-Road GitHub:** https://github.com/nordic-institute/X-Road
- **Web eID:** https://github.com/web-eid
- **DigiDoc4j:** https://github.com/open-eid/digidoc4j
- **TARA:** https://e-gov.github.io/TARA-Doku/

### Legal References

- **Accounting Act:** https://www.riigiteataja.ee/en/eli/512012014003/consolide
- **VAT Act:** https://www.riigiteataja.ee/en/eli/ee/Riigikogu/act/520012015006/consolide
- **Commercial Code:** https://www.riigiteataja.ee/en/eli/509012023002/consolide

### Support

- **e-MTA Help:** info@emta.ee, tel 880 0811
- **RIA Help:** help@ria.ee, tel 663 0200
- **X-Road Support:** xtee@ria.ee

---

**Document Status:** ✅ Comprehensive  
**Next Review:** After first quarterly VAT filing (January 2026)  
**Owner:** Sabel Akhoua (Bel Consulting OÜ)
