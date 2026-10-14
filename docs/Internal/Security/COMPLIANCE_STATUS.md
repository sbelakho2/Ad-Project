# Security Compliance Status

**Last Updated:** January 2026  
**Owner:** Security Team  
**Classification:** Internal

---

## Executive Summary

This document tracks the compliance status of Apex Mediation Platform against major security frameworks and regulations.

---

## 1. SOC 2 Type II Compliance

### Status: **In Progress**

| Control Category | Status | Evidence Location |
|------------------|--------|-------------------|
| Security | ✅ Implemented | `docs/Internal/Security/` |
| Availability | ✅ Implemented | `monitoring/alerts.yml`, SLO page |
| Processing Integrity | ✅ Implemented | VRA system, audit logs |
| Confidentiality | ✅ Implemented | Encryption policies |
| Privacy | ✅ Implemented | GDPR/CCPA handling |

### Key Controls Implemented

1. **Access Control**
   - Role-based access control (RBAC) in Console and Control Plane
   - Two-factor authentication mandatory for operators
   - API key rotation with 90-day expiry

2. **Encryption**
   - Data at rest: AES-256 (PostgreSQL TDE)
   - Data in transit: TLS 1.3 minimum
   - Secrets: HashiCorp Vault / environment encryption

3. **Audit Logging**
   - All admin actions logged with timestamp, actor, and target
   - Transparency ledger for VRA (immutable)
   - 90-day retention minimum

4. **Incident Response**
   - Documented in `docs/runbooks/`
   - 24/7 monitoring via Grafana/Alertmanager
   - Escalation paths defined

---

## 2. GDPR Compliance

### Status: **Compliant**

| Requirement | Implementation | Reference |
|-------------|----------------|-----------|
| Lawful Basis | Consent + Legitimate Interest | SDK consent APIs |
| Data Subject Rights | Self-service + API | Console Settings |
| Data Processing Agreements | Available | `/dpa` page |
| Privacy Policy | Published | `/privacy` page |
| Data Retention | Configurable | `DATA_RETENTION_POLICIES.md` |
| Cross-border Transfers | EU-hosted (Hetzner) | Infrastructure docs |

### Technical Measures

- TCF v2.2 consent string support in all SDKs
- US Privacy String (CCPA) support
- GPP (Global Privacy Platform) support in Web SDK
- COPPA mode for child-directed content
- Data deletion API for RTBF requests

---

## 3. PCI DSS Compliance

### Status: **Out of Scope (Adyen Delegated)**

All payment processing is delegated to Adyen, which is PCI DSS Level 1 certified. Apex Mediation:

- Does NOT store, process, or transmit cardholder data
- Uses Adyen Elements for secure card capture
- Adyen handles all PCI compliance obligations

See: `docs/Internal/Security/PCI_SCOPE.md`

---

## 4. CCPA/CPRA Compliance

### Status: **Compliant**

| Requirement | Implementation |
|-------------|----------------|
| "Do Not Sell" opt-out | US Privacy String honored |
| Data disclosure requests | API + Console support |
| Sensitive data handling | Minimized collection |

---

## 5. Estonian e-Residency & VAT Compliance

### Status: **Compliant**

| Requirement | Implementation | Reference |
|-------------|----------------|-----------|
| VAT Registration | EE VAT ID active | Control Plane |
| TSD Declarations | Automated generation | `estonian-compliance.ts` |
| KMD-INF Reports | Automated generation | `estonian-compliance.ts` |
| Invoice Requirements | PDF generation | `pdf-generator.ts` |

---

## 6. Penetration Testing

### Most Recent Test

| Date | Vendor | Scope | Findings |
|------|--------|-------|----------|
| Q4 2025 | Internal | Full platform | 0 Critical, 2 Medium (resolved) |

### Remediation Summary

| Finding | Severity | Status | Resolution Date |
|---------|----------|--------|-----------------|
| Rate limiting bypass edge case | Medium | ✅ Fixed | Dec 2025 |
| Session fixation potential | Medium | ✅ Fixed | Dec 2025 |

### Next Scheduled Test

- **Date:** Q1 2026
- **Scope:** Full platform + new SSAI features
- **Vendor:** TBD (considering external audit)

---

## 7. Vulnerability Management

### Current Status

| Metric | Value | Target |
|--------|-------|--------|
| Critical CVEs (unpatched) | 0 | 0 |
| High CVEs (unpatched) | 0 | 0 |
| MTTR (Critical) | <24h | <24h |
| MTTR (High) | <7d | <7d |

### Automated Scanning

- **Dependency scanning:** GitHub Dependabot + npm audit (daily)
- **Container scanning:** Trivy in CI/CD pipeline
- **SAST:** ESLint security rules + custom checks

---

## 8. Business Continuity

### Recovery Objectives

| System | RTO | RPO | Backup Method |
|--------|-----|-----|---------------|
| Database | 1h | 24h | PITR + Daily snapshots |
| Application | 15m | N/A | Stateless, multi-instance |
| Redis | 15m | 1h | AOF persistence |

See: `docs/PITR_RECOVERY.md`

---

## 9. Annual Review Schedule

| Review Type | Frequency | Last Completed | Next Due |
|-------------|-----------|----------------|----------|
| Security audit | Annual | Dec 2025 | Dec 2026 |
| Penetration test | Semi-annual | Dec 2025 | Q2 2026 |
| Compliance review | Quarterly | Jan 2026 | Apr 2026 |
| Access review | Quarterly | Dec 2025 | Mar 2026 |

---

## Contact

For compliance inquiries: security@apexmediation.ee
