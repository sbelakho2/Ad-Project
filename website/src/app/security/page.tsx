'use client';

import { motion } from 'framer-motion';
import {
  AlertTriangle,
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
import HomeNav from '@/components/HomeNav';

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
  { label: 'Uptime SLA', value: '99.9%', icon: CheckCircle },
  { label: 'Median latency', value: '<50ms', icon: Zap },
  { label: 'Multi-region', value: '3 zones', icon: Server },
  { label: 'Auto-failover', value: '<30s', icon: RefreshCw },
];

const complianceBadges = [
  { name: 'GDPR Compliant', description: 'Full EU data protection compliance' },
  { name: 'CCPA Ready', description: 'California Consumer Privacy Act support' },
  { name: 'SOC 2 Type II', description: 'Certification in progress' },
  { name: 'IAB TCF 2.0', description: 'Transparency & Consent Framework' },
];

export default function SecurityPage() {
  return (
    <>
      <main className="relative bg-white text-gray-900">
        <div className="container relative z-10 py-6 md:py-10">
          <HomeNav />
        </div>

        {/* Hero */}
        <section className="py-20 md:py-32 bg-gradient-to-b from-gray-900 to-gray-800">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-4xl mx-auto">
              <div className="w-20 h-20 bg-brand-600 rounded-2xl flex items-center justify-center mx-auto mb-8">
                <ShieldCheck className="w-10 h-10 text-white" />
              </div>
              <h1 className="text-4xl md:text-6xl font-bold text-white leading-tight mb-6">
                Security that scales with your trust.
              </h1>
              <p className="text-xl md:text-2xl text-gray-300 max-w-3xl mx-auto">
                We believe mediation should be verifiable, not just trustworthy. 
                Every auction is cryptographically signed. Every decision is auditable.
              </p>
            </motion.div>
          </div>
        </section>

        {/* Security Pillars */}
        <section className="py-20">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-16">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Security by design
              </h2>
              <p className="text-lg text-gray-600 max-w-2xl mx-auto">
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
        <section className="py-20 bg-gray-50">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-16">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Privacy-first architecture
              </h2>
              <p className="text-lg text-gray-600 max-w-2xl mx-auto">
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
        <section className="py-20">
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
                  <div className="bg-yellow-50 border border-yellow-200 rounded-xl p-4 flex items-start gap-3">
                    <AlertTriangle className="w-5 h-5 text-yellow-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <p className="font-semibold text-yellow-800">Real-time status</p>
                      <p className="text-sm text-yellow-700">
                        Check our status page at{' '}
                        <a href="https://status.apexmediation.ee" className="underline">
                          status.apexmediation.ee
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
        <section className="py-20 bg-gray-900">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-white mb-4">
                Compliance & certifications
              </h2>
              <p className="text-lg text-gray-400 max-w-2xl mx-auto">
                We meet the standards your legal and security teams require.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-4 gap-6 max-w-5xl mx-auto">
              {complianceBadges.map((badge, index) => (
                <motion.div
                  key={badge.name}
                  {...getRevealProps(index * 0.1)}
                  className="bg-gray-800 rounded-xl p-6 text-center"
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
        <section className="py-20">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto text-center">
              <motion.div {...getRevealProps(0)}>
                <Eye className="w-16 h-16 text-brand-600 mx-auto mb-6" />
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-6">
                  Don&apos;t trust us. Verify.
                </h2>
                <p className="text-lg text-gray-600 mb-8 max-w-2xl mx-auto">
                  Every auction produces a cryptographically signed log. Export it, verify it, 
                  audit it independently. We built Apex for publishers who demand proof, not promises.
                </p>
                <div className="flex flex-wrap justify-center gap-4">
                  <a
                    href="/product#transparency"
                    className="inline-flex items-center gap-2 bg-brand-600 text-white px-6 py-3 rounded-lg font-semibold hover:bg-brand-700 transition-colors"
                  >
                    Learn about cryptographic logs
                  </a>
                  <a
                    href="/contact"
                    className="inline-flex items-center gap-2 border border-gray-300 text-gray-700 px-6 py-3 rounded-lg font-semibold hover:bg-gray-50 transition-colors"
                  >
                    Request security review
                  </a>
                </div>
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
