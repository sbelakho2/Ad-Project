'use client';

import { motion } from 'framer-motion';
import {
  CheckCircle,
  Download,
  FileCheck,
  FileText,
  Key,
  Lock,
  Server,
  Shield,
  ShieldCheck,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';
import PageHero from '@/components/PageHero';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

const securityPackContents = [
  {
    title: 'Infrastructure Architecture',
    description: 'Two-server topology (App + DB), Caddy TLS termination, Redis caching, PostgreSQL 16 persistence, network isolation.',
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

const threatModels = [
  {
    threat: 'Cache Poisoning',
    control: 'Session manifests use Cache-Control: private, no-store. CDN keys include session token hash.',
    status: 'mitigated',
  },
  {
    threat: 'Token Leakage',
    control: 'JWT tokens with configurable expiry. Short-lived BYO tokens (5-15 min). httpOnly cookies.',
    status: 'mitigated',
  },
  {
    threat: 'SSRF Attacks',
    control: 'Outbound requests constrained to configured partner domains. CORS allowlist enforced.',
    status: 'mitigated',
  },
  {
    threat: 'Replay Attacks',
    control: 'Composite dedupe keys (session, pod, slot, event, creative). DedupeLayer rejects duplicates.',
    status: 'mitigated',
  },
  {
    threat: 'Config Rollback Abuse',
    control: 'All config changes versioned with timestamps. Rollback creates new version, preserving history.',
    status: 'mitigated',
  },
  {
    threat: 'Partner Timeout Abuse',
    control: 'Per-partner circuit breakers with configurable thresholds. Automatic quarantine on trip.',
    status: 'mitigated',
  },
  {
    threat: 'Auction Tampering',
    control: 'Ed25519 signatures on auction logs. Public key available for independent verification.',
    status: 'mitigated',
  },
  {
    threat: 'Credential Exposure',
    control: 'Network credentials never exposed to SDKs. Short-lived JWT tokens generated on demand.',
    status: 'mitigated',
  },
];

const dataProcessing = [
  {
    category: 'Session Data',
    data: 'Device type, app bundle ID, content ID, consent signals (TC string if provided)',
    retention: '30 days–1 year (by plan)',
    deletion: 'Automatic TTL expiry + on-demand DSAR',
  },
  {
    category: 'Auction Logs',
    data: 'Bid requests/responses, decision traces, timing metrics, Ed25519 signatures',
    retention: '30 days–1 year (by plan)',
    deletion: 'Automatic partition pruning + on-demand DSAR',
  },
  {
    category: 'Tracking Events',
    data: 'Impressions, quartiles, completions, click events (no direct PII)',
    retention: '30 days–1 year (by plan)',
    deletion: 'Automatic expiry + on-demand DSAR',
  },
  {
    category: 'Console Users',
    data: 'Email, display name, role, 2FA status, audit trail',
    retention: 'Account lifetime + 30 days',
    deletion: 'On account deletion request',
  },
  {
    category: 'API Keys',
    data: 'Key prefix, last4 digits, bcrypt hash (never full secret)',
    retention: 'Until revocation',
    deletion: 'Soft delete with audit retention',
  },
];

export default function SecurityPackPage() {
  return (
    <>
      <main className="relative bg-white text-slate-900">
        <header className="bg-white border-b border-slate-200/60">
          <div className="container mx-auto px-4 py-6">
            <HomeNav />
          </div>
        </header>

        {/* Hero */}
        <PageHero className="bg-gradient-to-b from-slate-50 to-white border-b border-slate-200/60">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <div className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-brand-100 text-brand-700 text-sm font-medium mb-6">
                <ShieldCheck className="w-4 h-4" />
                Security Documentation
              </div>
              <h1 className="text-4xl md:text-5xl font-bold text-slate-900 leading-tight mb-6">
                Security Pack
              </h1>
              <p className="text-xl text-slate-600 mb-8">
                Everything your security team needs to evaluate ApexMediation: architecture, threat model, compliance, and data handling.
              </p>
              <a
                href="/security-pack/download"
                className="btn-primary inline-flex items-center gap-2"
              >
                <Download className="w-5 h-5" />
                Printable Security Pack (HTML)
              </a>
            </motion.div>
          </div>
        </PageHero>

        {/* What's Included */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                What&apos;s Included
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                The Security Pack is a comprehensive document covering all aspects of our security posture.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-6 max-w-5xl mx-auto">
              {securityPackContents.map((item, index) => (
                <motion.div
                  key={item.title}
                  {...getRevealProps(index * 0.05)}
                  className="bg-gray-50 rounded-xl p-6"
                >
                  <div className="w-12 h-12 bg-brand-100 rounded-xl flex items-center justify-center mb-4">
                    <item.icon className="w-6 h-6 text-brand-600" />
                  </div>
                  <h3 className="text-lg font-semibold text-gray-900 mb-2">{item.title}</h3>
                  <p className="text-gray-600 text-sm">{item.description}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Threat Model Summary */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Threat Model Overview
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                Key threats we&apos;ve identified and the controls in place.
              </p>
            </motion.div>

            <div className="max-w-4xl mx-auto space-y-4">
              {threatModels.map((item, index) => (
                <motion.div
                  key={item.threat}
                  {...getRevealProps(index * 0.05)}
                  className="bg-white rounded-xl p-6 border border-gray-100"
                >
                  <div className="flex items-start gap-4">
                    <div className="flex-1">
                      <div className="flex items-center gap-3 mb-2">
                        <h3 className="text-lg font-semibold text-gray-900">{item.threat}</h3>
                        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-green-100 text-green-700 text-xs font-medium">
                          <CheckCircle className="w-3 h-3" />
                          {item.status}
                        </span>
                      </div>
                      <p className="text-gray-600 text-sm">{item.control}</p>
                    </div>
                  </div>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Data Processing Summary */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Data Processing Summary
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                What data we process, how long we keep it, and how to delete it.
              </p>
            </motion.div>

            <motion.div {...getRevealProps(0.1)} className="max-w-4xl mx-auto overflow-x-auto">
              <table className="w-full text-left border-collapse">
                <thead>
                  <tr className="border-b border-gray-200">
                    <th className="py-4 px-4 text-sm font-semibold text-gray-900">Category</th>
                    <th className="py-4 px-4 text-sm font-semibold text-gray-900">Data Collected</th>
                    <th className="py-4 px-4 text-sm font-semibold text-gray-900">Retention</th>
                    <th className="py-4 px-4 text-sm font-semibold text-gray-900">Deletion</th>
                  </tr>
                </thead>
                <tbody>
                  {dataProcessing.map((row) => (
                    <tr key={row.category} className="border-b border-gray-100">
                      <td className="py-4 px-4 text-sm font-medium text-gray-900">{row.category}</td>
                      <td className="py-4 px-4 text-sm text-gray-600">{row.data}</td>
                      <td className="py-4 px-4 text-sm text-gray-600">{row.retention}</td>
                      <td className="py-4 px-4 text-sm text-gray-600">{row.deletion}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </motion.div>
          </div>
        </section>

        {/* DSAR Workflow */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                  Data Subject Access Request (DSAR) Workflow
                </h2>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="bg-gray-900 rounded-xl p-6 font-mono text-sm overflow-x-auto">
                <pre className="text-gray-300">
{`# DSAR Process Flow
┌─────────────────────────────────────────────────────────────────┐
│                     DSAR Request Handling                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. Request Received                                             │
│     └─▶ Email: privacy@apexmediation.ee                         │
│     └─▶ Console: Settings → Privacy → Submit DSAR               │
│                                                                  │
│  2. Identity Verification (within 48 hours)                      │
│     └─▶ Verify requester is data subject or authorized agent    │
│                                                                  │
│  3. Data Compilation (within 15 days)                           │
│     └─▶ Export all personal data in structured format           │
│     └─▶ Include processing purposes and recipients              │
│                                                                  │
│  4. Delivery (within 30 days of request)                        │
│     └─▶ Secure download link or encrypted email                 │
│     └─▶ Deletion confirmation if requested                      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘`}
                </pre>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Compliance */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-5xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                  Compliance Status
                </h2>
              </motion.div>

              <div className="grid md:grid-cols-3 gap-6">
                <motion.div {...getRevealProps(0.1)} className="bg-green-50 rounded-xl p-6 text-center">
                  <CheckCircle className="w-10 h-10 text-green-600 mx-auto mb-3" />
                  <h3 className="font-semibold text-gray-900 mb-1">GDPR</h3>
                  <p className="text-sm text-gray-600">Aligned</p>
                </motion.div>
                <motion.div {...getRevealProps(0.15)} className="bg-green-50 rounded-xl p-6 text-center">
                  <CheckCircle className="w-10 h-10 text-green-600 mx-auto mb-3" />
                  <h3 className="font-semibold text-gray-900 mb-1">CCPA</h3>
                  <p className="text-sm text-gray-600">Aligned</p>
                </motion.div>
                <motion.div {...getRevealProps(0.2)} className="bg-blue-50 rounded-xl p-6 text-center">
                  <Shield className="w-10 h-10 text-blue-600 mx-auto mb-3" />
                  <h3 className="font-semibold text-gray-900 mb-1">IAB TCF 2.2</h3>
                  <p className="text-sm text-gray-600">Consent Propagation</p>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* CTA */}
        <section className="py-16 bg-gray-900 text-white">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <h2 className="text-3xl md:text-4xl font-bold mb-6">
                Questions? Let&apos;s talk security.
              </h2>
              <p className="text-lg text-gray-300 mb-8">
                Our team is available to discuss security requirements, conduct walkthroughs, and answer procurement questions.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                <a
                  href="/contact"
                  className="inline-flex items-center gap-2 px-6 py-3 bg-white text-gray-900 rounded-lg font-semibold hover:bg-gray-100 transition-colors"
                >
                  Schedule Security Review
                </a>
                <a
                  href="/security-pack/download"
                  className="inline-flex items-center gap-2 px-6 py-3 bg-white/10 text-white border border-white/20 rounded-lg font-semibold hover:bg-white/20 transition-colors"
                >
                  <Download className="w-5 h-5" />
                  Download Pack
                </a>
              </div>
            </motion.div>
          </div>
        </section>

        <Footer />
      </main>
      <CookieBanner />
    </>
  );
}
