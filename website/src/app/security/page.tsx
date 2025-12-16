'use client';

import { motion } from 'framer-motion';
import {
  CheckCircle,
  Clock,
  Database,
  Eye,
  FileCheck,
  Globe,
  Key,
  Lock,
  RefreshCw,
  Server,
  Shield,
  ShieldCheck,
  UserX,
  Zap,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';
import { sloConfig, tcfConfig } from '@/config/legal';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

const securityPillars = [
  {
    title: 'Cryptographic Transparency',
    description: 'Every auction produces a signed, tamper-evident log. You can verify any decision we make.',
    icon: Key,
    details: [
      'SHA-256 signed auction logs',
      'Exportable for independent verification',
      'Immutable audit trail',
    ],
  },
  {
    title: 'Data Minimization',
    description: 'We collect only what\'s necessary for mediation. No fingerprinting, no cross-app tracking.',
    icon: Database,
    details: [
      'No device fingerprinting',
      'No cross-publisher data sharing',
      'Configurable data retention',
    ],
  },
  {
    title: 'Publisher Control',
    description: 'You decide which networks participate, what data they receive, and how long logs are retained.',
    icon: Shield,
    details: [
      'Per-network data policies',
      'Granular consent handling',
      'Self-service data deletion',
    ],
  },
];

const privacyFeatures = [
  {
    title: 'No fingerprinting',
    description: 'We don\'t collect or infer device identifiers beyond what you explicitly pass.',
    icon: UserX,
  },
  {
    title: 'No cross-publisher sharing',
    description: 'Your user data is never shared with other publishers on our platform.',
    icon: Globe,
  },
  {
    title: 'GDPR & CCPA ready',
    description: 'Built-in consent handling and data subject request support.',
    icon: FileCheck,
  },
  {
    title: 'Configurable retention',
    description: 'Set your own retention periods. Delete data on demand.',
    icon: Clock,
  },
];

const reliabilityMetrics = [
  { label: 'Uptime SLA', value: sloConfig.availability.target, icon: CheckCircle },
  { label: 'Latency (p50)', value: sloConfig.latency.p50, icon: Zap },
  { label: 'Regions', value: sloConfig.regions.map(r => `${r.code} (${r.name})`).join(', '), icon: Server },
  { label: 'Auto-failover', value: sloConfig.failover.target, icon: RefreshCw },
];

const complianceBadges = [
  { name: 'GDPR Compliant', description: 'Full EU data protection compliance', status: 'live' },
  { name: 'CCPA Ready', description: 'California Consumer Privacy Act support', status: 'live' },
  { name: `IAB TCF ${tcfConfig.version}`, description: 'Transparency & Consent Framework', status: 'live' },
  { name: 'COPPA Compliant', description: 'Children\'s privacy protection', status: 'live' },
];

export default function SecurityPage() {
  return (
    <>
      <main className="relative bg-white text-slate-900">
        <SiteHeader />

        {/* Hero */}
        <PageHero variant="light" className="bg-slate-50 border-b border-slate-200/60">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto">
              <div className="w-16 h-16 bg-brand-600 rounded-2xl flex items-center justify-center mx-auto mb-6">
                <ShieldCheck className="w-8 h-8 text-white" />
              </div>
              <h1 className="text-4xl md:text-5xl font-bold leading-tight text-slate-900 mb-6">
                Security that scales with your trust.
              </h1>
              <p className="text-xl leading-relaxed text-slate-600">
                We believe mediation should be verifiable, not just trustworthy.
                Every auction is cryptographically signed. Every decision is auditable.
              </p>
            </motion.div>
          </div>
        </PageHero>

        {/* Security Pillars */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Security by design
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                Security isn&apos;t a feature we added—it&apos;s how we built the platform from day one.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-3 gap-8 max-w-6xl mx-auto">
              {securityPillars.map((pillar, index) => (
                <motion.div
                  key={pillar.title}
                  {...getRevealProps(index * 0.1)}
                  className="bg-gray-50 rounded-xl p-8"
                >
                  <div className="w-14 h-14 bg-brand-600 rounded-xl flex items-center justify-center mb-6">
                    <pillar.icon className="w-7 h-7 text-white" />
                  </div>
                  <h3 className="text-xl font-bold text-gray-900 mb-3">{pillar.title}</h3>
                  <p className="text-gray-600 mb-6">{pillar.description}</p>
                  <ul className="space-y-2">
                    {pillar.details.map((detail) => (
                      <li key={detail} className="flex items-center gap-2 text-sm text-gray-700">
                        <CheckCircle className="w-4 h-4 text-brand-600 flex-shrink-0" />
                        {detail}
                      </li>
                    ))}
                  </ul>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Privacy */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Privacy-first architecture
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                We believe privacy and monetization can coexist. 
                Your users&apos; data stays in your hands.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-2 lg:grid-cols-4 gap-6 max-w-6xl mx-auto">
              {privacyFeatures.map((feature, index) => (
                <motion.div
                  key={feature.title}
                  {...getRevealProps(index * 0.1)}
                  className="bg-white rounded-xl p-6 border border-gray-200"
                >
                  <feature.icon className="w-8 h-8 text-brand-600 mb-4" />
                  <h3 className="font-bold text-gray-900 mb-2">{feature.title}</h3>
                  <p className="text-sm text-gray-600">{feature.description}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Reliability */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid md:grid-cols-2 gap-12 items-center">
                <motion.div {...getRevealProps(0)}>
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-6">
                    Reliability you can count on
                  </h2>
                  <p className="text-lg text-gray-600 mb-6">
                    We treat downtime as seriously as you do. Our infrastructure is designed 
                    for high availability with automatic failover and multi-region redundancy.
                  </p>
                  <div className="bg-brand-50 border border-brand-200 rounded-xl p-4 flex items-start gap-3">
                    <ShieldCheck className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <p className="font-semibold text-brand-800">SLO Dashboard</p>
                      <p className="text-sm text-brand-700">
                        View our public SLO commitments at{' '}
                        <a href="/slo" className="underline">
                          apexmediation.ee/slo
                        </a>
                      </p>
                    </div>
                  </div>
                </motion.div>

                <motion.div {...getRevealProps(0.1)}>
                  <div className="grid grid-cols-2 gap-4">
                    {reliabilityMetrics.map((metric) => (
                      <div key={metric.label} className="bg-gray-50 rounded-xl p-6 text-center">
                        <metric.icon className="w-8 h-8 text-brand-600 mx-auto mb-3" />
                        <div className="text-2xl font-bold text-gray-900 mb-1">
                          {metric.value}
                        </div>
                        <div className="text-sm text-gray-600">{metric.label}</div>
                      </div>
                    ))}
                  </div>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Compliance */}
        <section className="py-16 bg-gray-900">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-white mb-4">
                Compliance & certifications
              </h2>
              <p className="text-lg text-gray-400 max-w-prose mx-auto">
                We meet the standards your legal and security teams require.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-4 gap-6 max-w-5xl mx-auto">
              {complianceBadges.map((badge, index) => (
                <motion.div
                  key={badge.name}
                  {...getRevealProps(index * 0.1)}
                  className="bg-gray-800 rounded-xl p-6 text-center relative"
                >
                  <Lock className="w-8 h-8 text-brand-400 mx-auto mb-3" />
                  <h3 className="font-bold text-white mb-1">{badge.name}</h3>
                  <p className="text-sm text-gray-400">{badge.description}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Audit & Verification */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <Eye className="w-12 h-12 text-brand-600 mx-auto mb-6" />
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-6">
                  Don&apos;t trust us. Verify.
                </h2>
                <p className="text-lg text-gray-600 mb-8">
                  Every auction produces a cryptographically signed log. Export it, verify it, 
                  audit it independently. We built Apex for publishers who demand proof, not promises.
                </p>
              </motion.div>

              {/* Security Pack Contents */}
              <motion.div {...getRevealProps(0.1)} className="bg-gray-50 rounded-xl p-8 mb-8">
                <h3 className="text-xl font-bold text-gray-900 mb-4 text-center">
                  Security Pack Contents
                </h3>
                <div className="grid md:grid-cols-2 gap-4">
                  <div className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                    <span className="text-gray-700">System architecture diagram</span>
                  </div>
                  <div className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                    <span className="text-gray-700">Threat model & mitigations</span>
                  </div>
                  <div className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                    <span className="text-gray-700">Data flow documentation</span>
                  </div>
                  <div className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                    <span className="text-gray-700">Encryption & key management</span>
                  </div>
                  <div className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                    <span className="text-gray-700">GDPR/CCPA compliance checklist</span>
                  </div>
                  <div className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                    <span className="text-gray-700">Penetration test summary</span>
                  </div>
                  <div className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                    <span className="text-gray-700">Signed log verification guide</span>
                  </div>
                  <div className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                    <span className="text-gray-700">Vendor security questionnaire</span>
                  </div>
                </div>
              </motion.div>

              <motion.div {...getRevealProps(0.2)} className="flex flex-wrap justify-center gap-4">
                <a
                  href="/security-pack"
                  className="btn-primary"
                >
                  Download Security Pack
                </a>
                <a
                  href="/docs/verification"
                  className="btn-secondary"
                >
                  Learn about signed logs
                </a>
              </motion.div>
            </div>
          </div>
        </section>
      </main>

      <Footer />
      <CookieBanner />
    </>
  );
}
