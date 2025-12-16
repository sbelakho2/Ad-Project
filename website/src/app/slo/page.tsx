'use client';

import { motion } from 'framer-motion';
import {
  Activity,
  AlertTriangle,
  CheckCircle,
  Clock,
  FileCheck,
  RefreshCw,
  Server,
  Shield,
  ShieldCheck,
  Target,
  Zap,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';
import PageHero from '@/components/PageHero';
import { sloConfig } from '@/config/legal';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

/**
 * SLO Definitions - CANONICAL SOURCE
 * 
 * This page is the authoritative source for all SLO metrics.
 * Other pages (home, security, company) reference these values
 * via the sloConfig in /config/legal.ts
 */
const sloDefinitions = [
  {
    name: 'Manifest/API Availability',
    target: sloConfig.availability.target,
    measurement: 'Percentage of successful responses (2xx) from manifest and API endpoints over a 30-day rolling window.',
    unit: 'availability',
    icon: Server,
  },
  {
    name: 'Decision Latency (p95)',
    target: sloConfig.latency.p95,
    measurement: 'Time from cue receipt to pod decision completion, measured at the 95th percentile.',
    unit: 'latency',
    icon: Clock,
  },
  {
    name: 'Pod Deadline Hit Rate',
    target: '> 98%',
    measurement: 'Percentage of ad pods that complete stitching before the playback deadline.',
    unit: 'deadline',
    icon: Target,
  },
  {
    name: 'Invalid Manifest Rate',
    target: '< 0.01%',
    measurement: 'Percentage of manifests that fail HLS/DASH validation or cause player parse errors.',
    unit: 'error',
    icon: FileCheck,
  },
  {
    name: 'Tracking Integrity',
    target: '100%',
    measurement: 'Exactly-once semantics: no duplicates, no missed events. Verified by reconciliation.',
    unit: 'integrity',
    icon: ShieldCheck,
  },
];

const mitigations = [
  {
    title: 'Safe Mode',
    description: 'When error rates exceed thresholds, the system automatically enters safe mode: reduced fanout, strict deadlines, and slate fallback for ad breaks.',
    trigger: `Error rate > 5% or latency p95 > ${sloConfig.latency.p95.replace(/^[<≤]\s*/, '')}`,
    icon: Shield,
  },
  {
    title: 'Automatic Rollback',
    description: 'Configuration changes that cause SLI regression are automatically rolled back within 60 seconds of detection.',
    trigger: 'SLI regression > 20% from baseline',
    icon: RefreshCw,
  },
  {
    title: 'Partner Quarantine',
    description: 'Partners with repeated timeouts or errors are temporarily quarantined with exponential backoff cooldown.',
    trigger: '5+ timeouts in 60 seconds',
    icon: AlertTriangle,
  },
  {
    title: 'Circuit Breaker',
    description: 'Per-partner circuit breakers prevent cascading failures. Trips after threshold breaches, recovers automatically.',
    trigger: 'Failure rate > 50% over 10 requests',
    icon: Zap,
  },
];

const statusHistory = [
  { date: '2025-12-14', status: 'operational', note: 'All systems operational' },
  { date: '2025-12-13', status: 'operational', note: 'All systems operational' },
  { date: '2025-12-12', status: 'operational', note: 'All systems operational' },
  { date: '2025-12-11', status: 'operational', note: 'All systems operational' },
  { date: '2025-12-10', status: 'operational', note: 'All systems operational' },
];

export default function SLOPage() {
  return (
    <>
      <main className="relative bg-white text-slate-900">
        <header className="bg-white border-b border-slate-200/60">
          <div className="container mx-auto px-4 py-6">
            <HomeNav />
          </div>
        </header>

        {/* Hero */}
        <PageHero className="bg-white border-b border-slate-200/60">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <div className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-green-100 text-green-700 text-sm font-medium mb-6">
                <CheckCircle className="w-4 h-4" />
                All Systems Operational
              </div>
              <h1 className="text-4xl md:text-5xl font-bold text-slate-900 leading-tight mb-6">
                SLOs & System Reliability
              </h1>
              <p className="text-xl text-slate-600">
                We publish our service level objectives so you know exactly what to expect. When we miss, here&apos;s what happens.
              </p>
            </motion.div>
          </div>
        </PageHero>

        {/* SLO Definitions */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Our Service Level Objectives
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                These are the targets we hold ourselves to. All measurements are real-time and auditable.
              </p>
            </motion.div>

            <div className="max-w-5xl mx-auto space-y-6">
              {sloDefinitions.map((slo, index) => (
                <motion.div
                  key={slo.name}
                  {...getRevealProps(index * 0.05)}
                  className="bg-white rounded-xl p-6 shadow-sm border border-gray-100"
                >
                  <div className="flex items-start gap-6">
                    <div className="w-12 h-12 bg-brand-100 rounded-xl flex items-center justify-center flex-shrink-0">
                      <slo.icon className="w-6 h-6 text-brand-600" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex flex-wrap items-center gap-4 mb-2">
                        <h3 className="text-lg font-semibold text-gray-900">{slo.name}</h3>
                        <span className="inline-flex items-center px-3 py-1 rounded-full bg-brand-100 text-brand-700 text-sm font-semibold">
                          Target: {slo.target}
                        </span>
                      </div>
                      <p className="text-gray-600 text-sm">{slo.measurement}</p>
                    </div>
                    <div className="hidden md:flex items-center gap-2 text-green-600">
                      <CheckCircle className="w-5 h-5" />
                      <span className="text-sm font-medium">Meeting</span>
                    </div>
                  </div>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* How We Measure */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                  How We Measure
                </h2>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="bg-gray-900 rounded-xl p-6 font-mono text-sm overflow-x-auto">
                <pre className="text-gray-300">
{`# SLI Calculation Pipeline
┌─────────────────────────────────────────────────────────────────┐
│                     Real-Time Metrics Flow                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────┐    ┌─────────────┐    ┌──────────────┐             │
│  │ Request │───▶│ Prometheus  │───▶│  SLI Bucket  │             │
│  │  Logs   │    │   Scrape    │    │  Aggregator  │             │
│  └─────────┘    └─────────────┘    └──────────────┘             │
│                                            │                     │
│                                            ▼                     │
│                     ┌──────────────────────────────┐            │
│                     │    30-Day Rolling Window     │            │
│                     │    ─────────────────────     │            │
│                     │    availability = good/total │            │
│                     │    latency_p95 = histogram   │            │
│                     │    error_rate = errors/total │            │
│                     └──────────────────────────────┘            │
│                                            │                     │
│                     ┌──────────────────────┴───────┐            │
│                     ▼                              ▼            │
│              ┌─────────────┐              ┌─────────────┐       │
│              │  Dashboard  │              │   Alerts    │       │
│              │  (Grafana)  │              │ (PagerDuty) │       │
│              └─────────────┘              └─────────────┘       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘`}
                </pre>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Mitigations */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                What Happens When We Miss
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                Automated mitigations kick in before humans need to act. Here&apos;s the protection stack.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-2 gap-8 max-w-5xl mx-auto">
              {mitigations.map((mitigation, index) => (
                <motion.div
                  key={mitigation.title}
                  {...getRevealProps(index * 0.1)}
                  className="bg-white rounded-xl p-6 shadow-sm border border-gray-100"
                >
                  <div className="flex items-center gap-4 mb-4">
                    <div className="w-12 h-12 bg-amber-100 rounded-xl flex items-center justify-center">
                      <mitigation.icon className="w-6 h-6 text-amber-600" />
                    </div>
                    <h3 className="text-lg font-semibold text-gray-900">{mitigation.title}</h3>
                  </div>
                  <p className="text-gray-600 text-sm mb-4">{mitigation.description}</p>
                  <div className="bg-gray-50 rounded-lg px-4 py-2">
                    <span className="text-xs text-gray-500">Trigger: </span>
                    <span className="text-xs font-mono text-gray-700">{mitigation.trigger}</span>
                  </div>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Status History */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                  Recent Status History
                </h2>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="space-y-3">
                {statusHistory.map((entry) => (
                  <div
                    key={entry.date}
                    className="flex items-center gap-4 p-4 bg-white rounded-lg border border-gray-100"
                  >
                    <CheckCircle className="w-5 h-5 text-green-500 flex-shrink-0" />
                    <span className="text-sm font-mono text-gray-500 w-24">{entry.date}</span>
                    <span className="text-sm text-gray-700">{entry.note}</span>
                  </div>
                ))}
              </motion.div>

              <motion.div {...getRevealProps(0.2)} className="mt-8 text-center">
                <p className="text-sm text-gray-500 mb-4">
                  For real-time status and incident updates, visit our status page.
                </p>
                <a
                  href="/status"
                  className="btn-primary inline-flex items-center gap-2"
                >
                  <Activity className="w-5 h-5" />
                  View Status Page
                </a>
              </motion.div>
            </div>
          </div>
        </section>

        {/* CTA */}
        <section className="py-16 bg-gray-900 text-white">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <h2 className="text-3xl md:text-4xl font-bold mb-6">
                Trust starts with transparency
              </h2>
              <p className="text-lg text-gray-300 mb-8">
                Download our Security Pack for the full picture: architecture, threat model, compliance, and more.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                <a
                  href="/security-pack"
                  className="inline-flex items-center gap-2 px-6 py-3 bg-white text-gray-900 rounded-lg font-semibold hover:bg-gray-100 transition-colors"
                >
                  Download Security Pack
                </a>
                <a
                  href="/docs/verification"
                  className="inline-flex items-center gap-2 px-6 py-3 bg-white/10 text-white border border-white/20 rounded-lg font-semibold hover:bg-white/20 transition-colors"
                >
                  Verification Guide
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
