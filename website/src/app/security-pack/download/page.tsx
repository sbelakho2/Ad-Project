'use client';

import { useEffect } from 'react';
import { Shield, Lock, FileText, Server, Key, FileCheck } from 'lucide-react';

/**
 * Security Pack Download Page
 * 
 * This page renders a print-friendly, comprehensive version of the Security Pack.
 * When opened, it automatically triggers the browser's print dialog,
 * allowing users to save as PDF or print.
 * 
 * All claims are verified against actual implementation in the codebase.
 */

const securityPackContents = [
  {
    title: 'Architecture Overview',
    description: 'Two-server production topology (App + DB), Caddy TLS termination, Redis caching, PostgreSQL 16 persistence.',
    icon: Server,
  },
  {
    title: 'Encryption Standards',
    description: 'TLS via Caddy (Let\'s Encrypt auto-provisioning), AES-256-GCM for secrets at rest with 12-byte IV and auth tags.',
    icon: Key,
  },
  {
    title: 'Access Control (RBAC)',
    description: 'Three-tier customer roles (admin, publisher, readonly) with separate operator authentication for system access.',
    icon: Lock,
  },
  {
    title: 'Audit Logging',
    description: 'Structured logging via Winston, separate audit stream for compliance, configurable retention (30 days–1 year by plan).',
    icon: FileText,
  },
  {
    title: 'Threat Model',
    description: 'Defense-in-depth: cache controls, circuit breakers, dedupe layers, Ed25519-signed auction logs, replay protection.',
    icon: Shield,
  },
  {
    title: 'Privacy & Compliance',
    description: 'GDPR/CCPA-aligned data handling, consent signal propagation, DSAR workflow, data minimization principles.',
    icon: FileCheck,
  },
];

const architectureDetails = [
  {
    component: 'App Server',
    description: 'Hosts Backend API (Node.js/Express on port 8080), Console (Next.js on port 3000), Website (Next.js/PM2 on port 3002), Redis (system service), Prometheus/Grafana monitoring.',
  },
  {
    component: 'DB Server',
    description: 'Dedicated PostgreSQL 16 instance with 139+ tables, accessible only from App Server IP, daily backups configured.',
  },
  {
    component: 'TLS Termination',
    description: 'Caddy reverse proxy with automatic Let\'s Encrypt certificate provisioning and renewal. HTTP/1.1 and HTTP/2 support.',
  },
  {
    component: 'Network Security',
    description: 'SSH key-based access only, UFW firewall (ports 22, 80, 443), PostgreSQL restricted to App Server IP.',
  },
];

const encryptionDetails = [
  {
    layer: 'In Transit',
    implementation: 'Caddy auto-provisions TLS certificates via Let\'s Encrypt. All public endpoints served over HTTPS.',
    evidence: 'infrastructure/caddy/Caddyfile',
  },
  {
    layer: 'At Rest (Credentials)',
    implementation: 'AES-256-GCM encryption with 12-byte random IV and authentication tag. Key derived from APP_KMS_KEY environment variable.',
    evidence: 'backend/src/utils/crypto.ts (aesGcmEncrypt, aesGcmDecrypt)',
  },
  {
    layer: 'API Keys',
    implementation: 'Only bcrypt hash stored; SHA-256 digest for constant-time lookup. Full secret shown once at creation, never stored.',
    evidence: 'backend/src/middleware/apiKeyAuth.ts',
  },
  {
    layer: '2FA Secrets',
    implementation: 'TOTP secrets encrypted with AES-256-GCM before database storage. Legacy plaintext secrets auto-migrated.',
    evidence: 'backend/src/services/twofa.service.ts',
  },
];

const rbacDetails = [
  {
    role: 'admin',
    capabilities: 'Full organization access: billing, team management, settings, all publisher features.',
  },
  {
    role: 'publisher',
    capabilities: 'Manage placements, view analytics, access developer tools, read-only billing access.',
  },
  {
    role: 'readonly',
    capabilities: 'View-only access to dashboards, reports, and analytics. No configuration changes.',
  },
  {
    role: 'operator (separate)',
    capabilities: 'System-level access to Control Plane (/admin/*). Requires separate authentication (not customer RBAC).',
  },
];

const threatModels = [
  {
    threat: 'Cache Poisoning',
    control: 'Session manifests use Cache-Control: private, no-store. CDN cache keys include session token hash.',
    status: 'Mitigated',
    evidence: 'SSAI_TODO.md confirms no-store implementation',
  },
  {
    threat: 'Token Leakage',
    control: 'JWT tokens with configurable expiry (default 7 days for session, 5-15 min for BYO credential tokens). httpOnly cookies.',
    status: 'Mitigated',
    evidence: 'website/src/lib/auth.ts, networkCredentialVault.ts',
  },
  {
    threat: 'SSRF Attacks',
    control: 'Outbound requests constrained to configured partner domains. CORS allowlist enforced.',
    status: 'Mitigated',
    evidence: 'docker-compose.prod.yml CORS_ALLOWLIST',
  },
  {
    threat: 'Replay Attacks',
    control: 'Tracking beacons use composite dedupe keys (session_id, pod_id, slot_id, event_type, creative_id). DedupeLayer rejects duplicates.',
    status: 'Mitigated',
    evidence: 'backend/src/services/ssai/tracking/DedupeLayer.ts, SDK dedupe in all clients',
  },
  {
    threat: 'Config Rollback Abuse',
    control: 'All configuration changes are versioned with timestamps. Rollback creates new version entry, preserving full history.',
    status: 'Mitigated',
    evidence: 'Versioned config tables with created_at timestamps',
  },
  {
    threat: 'Partner Timeout Abuse',
    control: 'Per-partner circuit breakers with configurable thresholds (error rate, timeout rate). Automatic quarantine on trip.',
    status: 'Mitigated',
    evidence: 'backend/src/services/ssai/pod/SSAICircuitBreaker.ts, SDK circuit breakers',
  },
  {
    threat: 'Auction Tampering',
    control: 'Ed25519 signatures on auction logs. Canonical JSON serialization. Public key available for independent verification.',
    status: 'Mitigated',
    evidence: 'transparency.controller.ts, data/schemas/postgresql.sql (Ed25519 signature column)',
  },
  {
    threat: 'Credential Exposure',
    control: 'Network credentials never exposed to SDKs. Short-lived JWT tokens (5-15 min) generated on demand.',
    status: 'Mitigated',
    evidence: 'backend/src/services/networkCredentialVault.ts',
  },
];

const dataProcessing = [
  {
    category: 'Session Data',
    data: 'Device type, app bundle ID, content ID, consent signals (TC string if provided)',
    retention: '30 days–1 year (by plan)',
    legal_basis: 'Legitimate interest / Consent',
    deletion: 'Automatic TTL expiry + on-demand DSAR',
  },
  {
    category: 'Auction Logs',
    data: 'Bid requests/responses, decision traces, timing metrics, Ed25519 signatures',
    retention: '30 days–1 year (by plan)',
    legal_basis: 'Legitimate interest (fraud prevention, transparency)',
    deletion: 'Automatic partition pruning + on-demand DSAR',
  },
  {
    category: 'Tracking Events',
    data: 'Impressions, quartiles, completions, click events (no direct PII)',
    retention: '30 days–1 year (by plan)',
    legal_basis: 'Legitimate interest / Contract',
    deletion: 'Automatic expiry + on-demand DSAR',
  },
  {
    category: 'Console Users',
    data: 'Email, display name, role, 2FA enrollment status, audit trail',
    retention: 'Account lifetime + 30 days post-deletion',
    legal_basis: 'Contract performance',
    deletion: 'On account deletion request',
  },
  {
    category: 'API Keys',
    data: 'Key prefix, last4 digits, bcrypt hash, SHA-256 digest (never full secret)',
    retention: 'Until revocation',
    legal_basis: 'Contract performance',
    deletion: 'Soft delete with audit retention',
  },
];

export default function SecurityPackDownloadPage() {
  useEffect(() => {
    // Auto-trigger print dialog after a short delay
    const timer = setTimeout(() => {
      window.print();
    }, 500);
    return () => clearTimeout(timer);
  }, []);

  const generatedDate = new Date().toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
  });

  return (
    <>
      <style jsx global>{`
        @media print {
          body {
            -webkit-print-color-adjust: exact !important;
            print-color-adjust: exact !important;
          }
          .no-print {
            display: none !important;
          }
          .page-break {
            page-break-before: always;
          }
        }
        @page {
          margin: 0.75in;
          size: letter;
        }
      `}</style>

      <div className="max-w-4xl mx-auto p-8 bg-white text-gray-900">
        {/* Header */}
        <div className="border-b-2 border-gray-900 pb-6 mb-8">
          <div className="flex items-center gap-4 mb-4">
            <img src="/Logo.jpg" alt="ApexMediation" className="w-32 h-32 object-contain" />
            <div>
              <h1 className="text-3xl font-bold">ApexMediation Security Pack</h1>
              <p className="text-gray-600">Comprehensive Security Documentation</p>
            </div>
          </div>
          <div className="flex justify-between text-sm text-gray-500">
            <span>Generated: {generatedDate}</span>
            <span>Document Version: 2.0</span>
          </div>
        </div>

        {/* Table of Contents */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900">Table of Contents</h2>
          <ol className="list-decimal list-inside space-y-1 text-gray-700 text-sm">
            <li>Executive Summary</li>
            <li>Infrastructure Architecture</li>
            <li>Encryption Standards</li>
            <li>Access Control (RBAC)</li>
            <li>Authentication Mechanisms</li>
            <li>Threat Model & Mitigations</li>
            <li>Data Processing & Retention</li>
            <li>Audit Logging</li>
            <li>Privacy & Compliance</li>
            <li>DSAR Workflow</li>
            <li>Incident Response</li>
            <li>Contact Information</li>
          </ol>
        </section>

        {/* Executive Summary */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">1. Executive Summary</h2>
          <p className="text-gray-700 mb-4 text-sm">
            ApexMediation is a server-side ad insertion (SSAI) and mediation platform designed with security, transparency, 
            and privacy as foundational principles. This document provides comprehensive, verifiable documentation of our 
            security architecture, threat model, data handling practices, and compliance posture.
          </p>
          <div className="grid grid-cols-2 gap-3 mb-4">
            {securityPackContents.map((item) => (
              <div key={item.title} className="bg-gray-50 border border-gray-200 rounded p-3 text-xs">
                <div className="flex items-center gap-2 mb-1">
                  <item.icon className="w-4 h-4 text-gray-700" />
                  <span className="font-semibold text-gray-900">{item.title}</span>
                </div>
                <p className="text-gray-600">{item.description}</p>
              </div>
            ))}
          </div>
        </section>

        {/* Infrastructure Architecture */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">2. Infrastructure Architecture</h2>
          <p className="text-gray-700 mb-4 text-sm">
            Production infrastructure runs on dedicated EU-based servers with strict network segmentation.
          </p>
          <table className="w-full text-sm border-collapse mb-4">
            <thead>
              <tr className="bg-gray-100">
                <th className="border border-gray-300 px-3 py-2 text-left font-semibold">Component</th>
                <th className="border border-gray-300 px-3 py-2 text-left font-semibold">Details</th>
              </tr>
            </thead>
            <tbody>
              {architectureDetails.map((item) => (
                <tr key={item.component}>
                  <td className="border border-gray-300 px-3 py-2 font-medium">{item.component}</td>
                  <td className="border border-gray-300 px-3 py-2 text-gray-700">{item.description}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="bg-blue-50 border border-blue-200 rounded p-3 text-sm">
            <strong>Network Diagram:</strong> App Server ↔ (Internal Network) ↔ DB Server. 
            External traffic terminates at Caddy (TLS). Database accessible only from App Server IP.
          </div>
        </section>

        {/* Encryption Standards */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">3. Encryption Standards</h2>
          <table className="w-full text-sm border-collapse">
            <thead>
              <tr className="bg-gray-100">
                <th className="border border-gray-300 px-3 py-2 text-left font-semibold">Layer</th>
                <th className="border border-gray-300 px-3 py-2 text-left font-semibold">Implementation</th>
                <th className="border border-gray-300 px-3 py-2 text-left font-semibold">Evidence</th>
              </tr>
            </thead>
            <tbody>
              {encryptionDetails.map((item) => (
                <tr key={item.layer}>
                  <td className="border border-gray-300 px-3 py-2 font-medium">{item.layer}</td>
                  <td className="border border-gray-300 px-3 py-2 text-gray-700">{item.implementation}</td>
                  <td className="border border-gray-300 px-3 py-2 text-gray-500 text-xs font-mono">{item.evidence}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>

        {/* Page break for print */}
        <div className="page-break"></div>

        {/* Access Control */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">4. Access Control (RBAC)</h2>
          <p className="text-gray-700 mb-4 text-sm">
            Customer Console access is governed by role-based access control. Operator (system) access uses a completely 
            separate authentication mechanism.
          </p>
          <table className="w-full text-sm border-collapse">
            <thead>
              <tr className="bg-gray-100">
                <th className="border border-gray-300 px-3 py-2 text-left font-semibold">Role</th>
                <th className="border border-gray-300 px-3 py-2 text-left font-semibold">Capabilities</th>
              </tr>
            </thead>
            <tbody>
              {rbacDetails.map((item) => (
                <tr key={item.role}>
                  <td className="border border-gray-300 px-3 py-2 font-medium font-mono">{item.role}</td>
                  <td className="border border-gray-300 px-3 py-2 text-gray-700">{item.capabilities}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="text-xs text-gray-500 mt-2 italic">
            Source: console/src/lib/rbac.ts, console/src/lib/useAdminGate.ts
          </p>
        </section>

        {/* Authentication */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">5. Authentication Mechanisms</h2>
          <div className="space-y-4 text-sm">
            <div className="border border-gray-200 rounded p-4">
              <h3 className="font-semibold mb-2">Console Authentication</h3>
              <ul className="list-disc list-inside text-gray-700 space-y-1">
                <li>JWT tokens (HS256) with configurable expiry (default: 7 days)</li>
                <li>Tokens stored in httpOnly cookies to prevent XSS</li>
                <li>Optional TOTP-based 2FA with encrypted secret storage</li>
                <li>Backup codes generated on 2FA enrollment</li>
              </ul>
            </div>
            <div className="border border-gray-200 rounded p-4">
              <h3 className="font-semibold mb-2">API Authentication</h3>
              <ul className="list-disc list-inside text-gray-700 space-y-1">
                <li>API keys: <code className="bg-gray-100 px-1 rounded">sk_*</code> (user) / <code className="bg-gray-100 px-1 rounded">ak_*</code> (app/SDK)</li>
                <li>Keys verified via SHA-256 digest lookup + bcrypt comparison</li>
                <li>Full secret displayed once at creation, then only prefix + last 4 digits</li>
                <li>Usage tracking per key for audit and rate limiting</li>
              </ul>
            </div>
            <div className="border border-gray-200 rounded p-4">
              <h3 className="font-semibold mb-2">BYO Credential Tokens</h3>
              <ul className="list-disc list-inside text-gray-700 space-y-1">
                <li>Short-lived JWT tokens (5-15 minute TTL) for network credentials</li>
                <li>Credentials encrypted with AES-256-GCM, never exposed to SDKs</li>
                <li>Token bound to publisher ID; audit logged on generation</li>
              </ul>
            </div>
          </div>
        </section>

        {/* Threat Model */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">6. Threat Model & Mitigations</h2>
          <table className="w-full text-xs border-collapse">
            <thead>
              <tr className="bg-gray-100">
                <th className="border border-gray-300 px-2 py-2 text-left font-semibold">Threat</th>
                <th className="border border-gray-300 px-2 py-2 text-left font-semibold">Control Implemented</th>
                <th className="border border-gray-300 px-2 py-2 text-left font-semibold">Status</th>
              </tr>
            </thead>
            <tbody>
              {threatModels.map((item) => (
                <tr key={item.threat}>
                  <td className="border border-gray-300 px-2 py-2 font-medium">{item.threat}</td>
                  <td className="border border-gray-300 px-2 py-2 text-gray-700">{item.control}</td>
                  <td className="border border-gray-300 px-2 py-2 text-green-700 font-medium">{item.status}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>

        {/* Page break for print */}
        <div className="page-break"></div>

        {/* Data Processing */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">7. Data Processing & Retention</h2>
          <table className="w-full text-xs border-collapse">
            <thead>
              <tr className="bg-gray-100">
                <th className="border border-gray-300 px-2 py-2 text-left font-semibold">Category</th>
                <th className="border border-gray-300 px-2 py-2 text-left font-semibold">Data Elements</th>
                <th className="border border-gray-300 px-2 py-2 text-left font-semibold">Retention</th>
                <th className="border border-gray-300 px-2 py-2 text-left font-semibold">Legal Basis</th>
                <th className="border border-gray-300 px-2 py-2 text-left font-semibold">Deletion</th>
              </tr>
            </thead>
            <tbody>
              {dataProcessing.map((row) => (
                <tr key={row.category}>
                  <td className="border border-gray-300 px-2 py-2 font-medium">{row.category}</td>
                  <td className="border border-gray-300 px-2 py-2 text-gray-700">{row.data}</td>
                  <td className="border border-gray-300 px-2 py-2 text-gray-700">{row.retention}</td>
                  <td className="border border-gray-300 px-2 py-2 text-gray-700">{row.legal_basis}</td>
                  <td className="border border-gray-300 px-2 py-2 text-gray-700">{row.deletion}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="bg-yellow-50 border border-yellow-200 rounded p-3 mt-4 text-sm">
            <strong>Data Minimization:</strong> We aim to minimize collection. IP addresses may be processed (and in some cases retained) for security, fraud prevention, and rate limiting.
            Device identifiers (IDFA, GAID) are only collected when consent is provided. No direct PII is stored 
            in tracking events.
          </div>
        </section>

        {/* Audit Logging */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">8. Audit Logging</h2>
          <div className="space-y-4 text-sm">
            <div className="border border-gray-200 rounded p-4">
              <h3 className="font-semibold mb-2">What Gets Logged</h3>
              <ul className="list-disc list-inside text-gray-700 space-y-1">
                <li>All authentication events (login, logout, 2FA enrollment/verification)</li>
                <li>API key creation, rotation, and revocation</li>
                <li>Configuration changes (placements, settings, team membership)</li>
                <li>Credential access (BYO token generation, network credential rotation)</li>
                <li>Operator actions via Control Plane (retained per compliance and operational requirements)</li>
              </ul>
            </div>
            <div className="border border-gray-200 rounded p-4">
              <h3 className="font-semibold mb-2">Log Integrity</h3>
              <ul className="list-disc list-inside text-gray-700 space-y-1">
                <li>Structured JSON logs via Winston with consistent schema</li>
                <li>Separate audit stream for compliance-critical events</li>
                <li>Ed25519 signatures on auction decision logs for cryptographic verification</li>
                <li>Log retention is configured by plan (30 days / 90 days / 1 year) with automated pruning</li>
              </ul>
            </div>
          </div>
        </section>

        {/* Compliance Status */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">9. Privacy & Compliance</h2>
          <div className="grid grid-cols-2 gap-4 mb-4">
            <div className="border border-gray-300 rounded p-4">
              <div className="text-xl font-bold text-green-700 mb-1">✓ GDPR</div>
              <div className="text-sm text-gray-600">
                Data subject rights supported (access, rectification, erasure, portability). 
                Privacy by design. Data minimization enforced.
              </div>
            </div>
            <div className="border border-gray-300 rounded p-4">
              <div className="text-xl font-bold text-green-700 mb-1">✓ CCPA</div>
              <div className="text-sm text-gray-600">
                Do Not Sell signals honored. Consumer rights workflow implemented. 
                12-month data disclosure available.
              </div>
            </div>
          </div>
          <div className="border border-gray-300 rounded p-4">
            <div className="text-xl font-bold text-blue-700 mb-1">IAB TCF 2.2</div>
            <div className="text-sm text-gray-600">
              Consent string propagation implemented. TC strings passed through auction pipeline 
              to demand partners. Consent signal encryption available in Console.
            </div>
          </div>
          <div className="bg-gray-50 border border-gray-200 rounded p-3 mt-4 text-sm">
            <strong>Note:</strong> We operate as a data processor. Publisher customers are data controllers 
            responsible for obtaining end-user consent. We provide the technical infrastructure to honor 
            consent signals and data subject requests.
          </div>
        </section>

        {/* DSAR Workflow */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">10. DSAR Workflow</h2>
          <div className="bg-gray-50 border border-gray-300 rounded p-4 text-sm">
            <p className="mb-3"><strong>1. Request Received</strong></p>
            <p className="ml-4 mb-2">• Email: privacy@apexmediation.ee</p>
            <p className="ml-4 mb-4">• Console: Settings → Privacy → Submit DSAR</p>
            
            <p className="mb-3"><strong>2. Identity Verification</strong> (within 48 hours)</p>
            <p className="ml-4 mb-4">• Verify requester is data subject or authorized agent</p>
            
            <p className="mb-3"><strong>3. Data Compilation</strong> (within 15 days)</p>
            <p className="ml-4 mb-2">• Export all personal data in structured JSON/CSV format</p>
            <p className="ml-4 mb-4">• Include processing purposes, categories, and recipients</p>
            
            <p className="mb-3"><strong>4. Delivery</strong> (within 30 days of request)</p>
            <p className="ml-4 mb-2">• Secure download link (time-limited) or encrypted email</p>
            <p className="ml-4">• Deletion confirmation with audit trail if requested</p>
          </div>
        </section>

        {/* Incident Response */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">11. Incident Response</h2>
          <div className="text-sm space-y-3">
            <p className="text-gray-700">
              In the event of a security incident, we follow a structured response process:
            </p>
            <ol className="list-decimal list-inside space-y-2 text-gray-700">
              <li><strong>Detection & Triage:</strong> Automated monitoring alerts + manual review</li>
              <li><strong>Containment:</strong> Isolate affected systems, rotate credentials if needed</li>
              <li><strong>Investigation:</strong> Root cause analysis using audit logs</li>
              <li><strong>Notification:</strong> Affected customers notified within 72 hours (GDPR timeline)</li>
              <li><strong>Remediation:</strong> Fix deployed, post-incident review conducted</li>
            </ol>
            <p className="text-gray-700 mt-3">
              Report security issues to: <strong>security@apexmediation.ee</strong>
            </p>
          </div>
        </section>

        {/* Contact */}
        <section className="mb-8">
          <h2 className="text-xl font-bold mb-4 text-gray-900 border-b border-gray-300 pb-2">12. Contact Information</h2>
          <div className="space-y-2 text-gray-700 text-sm">
            <p><strong>Security Inquiries:</strong> security@apexmediation.ee</p>
            <p><strong>Privacy / DPO:</strong> privacy@apexmediation.ee</p>
            <p><strong>General Contact:</strong> hello@apexmediation.ee</p>
            <p><strong>Website:</strong> https://apexmediation.ee</p>
            <p><strong>Registered Entity:</strong> Bel Consulting OÜ, Estonia</p>
          </div>
        </section>

        {/* Footer */}
        <div className="border-t border-gray-300 pt-4 mt-8 text-center text-xs text-gray-500">
          <p>© {new Date().getFullYear()} Bel Consulting OÜ. All rights reserved.</p>
          <p>This document is confidential and intended for security evaluation purposes.</p>
          <p className="mt-2">Document generated from verified codebase implementation.</p>
        </div>

        {/* Print button (hidden when printing) */}
        <div className="no-print fixed bottom-8 right-8 flex gap-4">
          <button
            onClick={() => window.print()}
            className="px-6 py-3 bg-gray-900 text-white rounded-lg font-semibold shadow-lg hover:bg-gray-800"
          >
            Save as PDF
          </button>
          <a
            href="/security-pack"
            className="px-6 py-3 bg-white text-gray-900 border border-gray-300 rounded-lg font-semibold shadow-lg hover:bg-gray-50"
          >
            Back to Security Pack
          </a>
        </div>
      </div>
    </>
  );
}
