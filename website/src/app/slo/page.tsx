'use client';

import { motion } from 'framer-motion';
import { useEffect, useMemo, useState } from 'react';
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
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';
import { SLO } from '@/lib/slo';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

/**
 * SLO Definitions - CANONICAL SOURCE
 *
 * This page is the authoritative source for all SLO metrics.
 * Other pages (home, security, company, SSAI docs) reference these values
 * via the SLO constants in /lib/slo.ts
 */
const sloDefinitions = [
  {
    name: 'Manifest/API Availability',
    target: SLO.availability.target,
    measurement:
      'Percentage of successful responses (2xx) from manifest and API endpoints over a 30-day rolling window.',
    unit: 'availability',
    icon: Server,
  },
  {
    name: 'Decision Latency (p95)',
    target: SLO.latency.p95,
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
    description:
      'When error rates exceed thresholds, the system automatically enters safe mode: reduced fanout, strict deadlines, and slate fallback for ad breaks.',
    trigger: `Error rate > 5% or latency p95 > ${SLO.latency.p95.replace(/^[<≤]\s*/, '')}`,
    icon: Shield,
  },
  {
    title: 'Automatic Rollback',
    description:
      'Configuration changes that cause SLI regression are automatically rolled back within 60 seconds of detection.',
    trigger: 'SLI regression > 20% from baseline',
    icon: RefreshCw,
  },
  {
    title: 'Partner Quarantine',
    description:
      'Partners with repeated timeouts or errors are temporarily quarantined with exponential backoff cooldown.',
    trigger: '5+ timeouts in 60 seconds',
    icon: AlertTriangle,
  },
  {
    title: 'Circuit Breaker',
    description:
      'Per-partner circuit breakers prevent cascading failures. Trips after threshold breaches, recovers automatically.',
    trigger: 'Failure rate > 50% over 10 requests',
    icon: Zap,
  },
];

export default function SLOPage() {
  const [liveStatus, setLiveStatus] = useState<any | null>(null);
  const [usedLkg, setUsedLkg] = useState(false);
  const [sloRollup, setSloRollup] = useState<any | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    fetch(`${process.env.NEXT_PUBLIC_BASE_URL ?? ''}/api/public/status`, {
      signal: controller.signal,
      headers: { accept: 'application/json' },
    })
      .then((res) => (res.ok ? res.json() : Promise.reject(new Error('bad status'))))
      .then((data) => setLiveStatus(data))
      .catch(() => {
        setUsedLkg(true);
      });
    return () => controller.abort();
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    fetch(`${process.env.NEXT_PUBLIC_BASE_URL ?? ''}/api/public/slo?days=30`, {
      signal: controller.signal,
      headers: { accept: 'application/json' },
    })
      .then((res) => (res.ok ? res.json() : null))
      .then((data) => setSloRollup(data))
      .catch(() => {});
    return () => controller.abort();
  }, []);

  const targets = (() => {
    const availabilityTargetPct = Number((SLO.availability.target || '').replace('%', ''));
    const latencyTargetMs = Number((SLO.latency.p95 || '').replace(/[^0-9.]/g, ''));
    return {
      availabilityTargetPct: Number.isFinite(availabilityTargetPct) ? availabilityTargetPct : 99.9,
      latencyTargetMs: Number.isFinite(latencyTargetMs) ? latencyTargetMs : 200,
      deadlineTargetPct: 98,
      invalidManifestTargetPct: 0.01,
      trackingIntegrityTargetPct: 100,
    };
  })();

  const computedSlo = useMemo(() => {
    const hasData = sloRollup !== null;

    const availabilityRaw = sloRollup?.slis?.availability?.availabilityPct;
    const availabilityPct =
      availabilityRaw === null || availabilityRaw === undefined ? null : Number(availabilityRaw);

    const latencyP95Ms =
      sloRollup?.slis?.latency?.p95Ms === null || sloRollup?.slis?.latency?.p95Ms === undefined
        ? null
        : Number(sloRollup?.slis?.latency?.p95Ms);

    const deadlineRaw = sloRollup?.slis?.deadlineHitRate?.deadlineHitRatePct;
    const deadlineHitRatePct = deadlineRaw === null || deadlineRaw === undefined ? null : Number(deadlineRaw);

    const invalidManifestRaw = sloRollup?.slis?.invalidManifestRate?.invalidManifestRatePct;
    const invalidManifestRatePct =
      invalidManifestRaw === null || invalidManifestRaw === undefined ? null : Number(invalidManifestRaw);

    const trackingIntegrityRaw = sloRollup?.slis?.trackingIntegrity?.integrityPct;
    const trackingIntegrityPct =
      trackingIntegrityRaw === null || trackingIntegrityRaw === undefined ? null : Number(trackingIntegrityRaw);

    const availabilityOk =
      availabilityPct !== null && Number.isFinite(availabilityPct) ? availabilityPct >= targets.availabilityTargetPct : null;

    const latencyOk =
      typeof latencyP95Ms === 'number' && Number.isFinite(latencyP95Ms) ? latencyP95Ms <= targets.latencyTargetMs : null;

    const deadlineOk =
      deadlineHitRatePct !== null && Number.isFinite(deadlineHitRatePct) ? deadlineHitRatePct >= targets.deadlineTargetPct : null;

    const invalidManifestOk =
      invalidManifestRatePct !== null && Number.isFinite(invalidManifestRatePct)
        ? invalidManifestRatePct <= targets.invalidManifestTargetPct
        : null;

    const trackingIntegrityOk =
      trackingIntegrityPct !== null && Number.isFinite(trackingIntegrityPct) ? trackingIntegrityPct >= targets.trackingIntegrityTargetPct : null;

    return {
      hasData,
      availabilityPct: availabilityPct !== null && Number.isFinite(availabilityPct) ? availabilityPct : null,
      latencyP95Ms,
      deadlineHitRatePct: deadlineHitRatePct !== null && Number.isFinite(deadlineHitRatePct) ? deadlineHitRatePct : null,
      invalidManifestRatePct:
        invalidManifestRatePct !== null && Number.isFinite(invalidManifestRatePct) ? invalidManifestRatePct : null,
      trackingIntegrityPct:
        trackingIntegrityPct !== null && Number.isFinite(trackingIntegrityPct) ? trackingIntegrityPct : null,
      availabilityOk,
      latencyOk,
      deadlineOk,
      invalidManifestOk,
      trackingIntegrityOk,
      windowDays: sloRollup?.window?.days ?? 30,
      windowStart: typeof sloRollup?.window?.start === 'string' ? sloRollup.window.start : null,
      windowEnd: typeof sloRollup?.window?.end === 'string' ? sloRollup.window.end : null,
    };
  }, [
    sloRollup,
    targets.availabilityTargetPct,
    targets.latencyTargetMs,
    targets.deadlineTargetPct,
    targets.invalidManifestTargetPct,
    targets.trackingIntegrityTargetPct,
  ]);

  const badge = useMemo(() => {
    const status = (liveStatus?.status || 'unknown') as string;
    const lastChecked = typeof liveStatus?.timestamp === 'string' ? liveStatus.timestamp : null;

    if (status === 'operational') {
      return {
        className: 'bg-success/10 text-success',
        label: 'Operational',
        icon: CheckCircle,
        lastChecked,
      };
    }

    if (status === 'degraded') {
      return {
        className: 'bg-warning/10 text-warning',
        label: 'Degraded',
        icon: AlertTriangle,
        lastChecked,
      };
    }

    return {
      className: 'bg-gray-100 text-gray-700',
      label: 'Temporarily unavailable',
      icon: Activity,
      lastChecked,
    };
  }, [liveStatus]);

  const renderSloHealth = (unit: string) => {
    const showLoading = !computedSlo.hasData;

    if (unit === 'availability') {
      const current = computedSlo.availabilityPct;
      const ok = computedSlo.availabilityOk;

      if (showLoading) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-inkMuted">
            <span className="text-sm font-medium">Temporarily unavailable</span>
          </div>
        );
      }

      if (ok === null) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-gray-500">
            <span className="text-sm font-medium">No data yet</span>
            <span className="text-xs text-gray-400">Awaiting traffic</span>
          </div>
        );
      }

      return (
        <div className={`hidden md:flex flex-col items-end gap-1 ${ok ? 'text-success' : 'text-danger'}`}>
          <div className="flex items-center gap-2">
            <CheckCircle className={`w-5 h-5 ${ok ? 'text-success' : 'text-danger'}`} />
            <span className="text-sm font-medium">{ok ? 'Meeting' : 'Missed'}</span>
          </div>
          <span className="text-xs font-mono text-gray-500">{current === null ? '—' : `${current.toFixed(3)}%`}</span>
        </div>
      );
    }

    if (unit === 'latency') {
      const current = computedSlo.latencyP95Ms;
      const ok = computedSlo.latencyOk;

      if (showLoading) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-inkMuted">
            <span className="text-sm font-medium">Temporarily unavailable</span>
          </div>
        );
      }

      if (ok === null) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-gray-500">
            <span className="text-sm font-medium">No data yet</span>
            <span className="text-xs text-gray-400">Awaiting traffic</span>
          </div>
        );
      }

      return (
        <div className={`hidden md:flex flex-col items-end gap-1 ${ok ? 'text-success' : 'text-danger'}`}>
          <div className="flex items-center gap-2">
            <CheckCircle className="w-5 h-5" />
            <span className="text-sm font-medium">{ok ? 'Meeting' : 'Missed'}</span>
          </div>
          <span className="text-xs font-mono text-gray-500">{typeof current === 'number' ? `${Math.round(current)}ms` : '—'}</span>
        </div>
      );
    }

    if (unit === 'deadline') {
      const current = computedSlo.deadlineHitRatePct;
      const ok = computedSlo.deadlineOk;

      if (showLoading) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-inkMuted">
            <span className="text-sm font-medium">Temporarily unavailable</span>
          </div>
        );
      }

      if (ok === null) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-gray-500">
            <span className="text-sm font-medium">SSAI metric</span>
            <span className="text-xs text-gray-400">Awaiting SSAI traffic</span>
          </div>
        );
      }

      return (
        <div className={`hidden md:flex flex-col items-end gap-1 ${ok ? 'text-success' : 'text-danger'}`}>
          <div className="flex items-center gap-2">
            <CheckCircle className="w-5 h-5" />
            <span className="text-sm font-medium">{ok ? 'Meeting' : 'Missed'}</span>
          </div>
          <span className="text-xs font-mono text-gray-500">{current === null ? '—' : `${current.toFixed(3)}%`}</span>
        </div>
      );
    }

    if (unit === 'error') {
      const current = computedSlo.invalidManifestRatePct;
      const ok = computedSlo.invalidManifestOk;

      if (showLoading) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-inkMuted">
            <span className="text-sm font-medium">Temporarily unavailable</span>
          </div>
        );
      }

      if (ok === null) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-gray-500">
            <span className="text-sm font-medium">SSAI metric</span>
            <span className="text-xs text-gray-400">Awaiting SSAI traffic</span>
          </div>
        );
      }

      return (
        <div className={`hidden md:flex flex-col items-end gap-1 ${ok ? 'text-success' : 'text-danger'}`}>
          <div className="flex items-center gap-2">
            <CheckCircle className="w-5 h-5" />
            <span className="text-sm font-medium">{ok ? 'Meeting' : 'Missed'}</span>
          </div>
          <span className="text-xs font-mono text-gray-500">{current === null ? '—' : `${current.toFixed(4)}%`}</span>
        </div>
      );
    }

    if (unit === 'integrity') {
      const current = computedSlo.trackingIntegrityPct;
      const ok = computedSlo.trackingIntegrityOk;

      if (showLoading) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-inkMuted">
            <span className="text-sm font-medium">Temporarily unavailable</span>
          </div>
        );
      }

      if (ok === null) {
        return (
          <div className="hidden md:flex flex-col items-end gap-1 text-gray-500">
            <span className="text-sm font-medium">No data yet</span>
            <span className="text-xs text-gray-400">Awaiting traffic</span>
          </div>
        );
      }

      return (
        <div className={`hidden md:flex flex-col items-end gap-1 ${ok ? 'text-success' : 'text-danger'}`}>
          <div className="flex items-center gap-2">
            <CheckCircle className="w-5 h-5" />
            <span className="text-sm font-medium">{ok ? 'Meeting' : 'Missed'}</span>
          </div>
          <span className="text-xs font-mono text-gray-500">{current === null ? '—' : `${current.toFixed(3)}%`}</span>
        </div>
      );
    }

    return (
      <div className="hidden md:flex flex-col items-end gap-1 text-inkMuted">
        <span className="text-sm font-medium">Not published</span>
      </div>
    );
  };

  return (
    <>
      <main className="relative bg-white text-ink">
        <SiteHeader />

        {/* Hero (Rule C: trust/docs => light hero) */}
        <PageHero variant="light" className="border-b border-gray-200/60">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <div
                className={`inline-flex items-center gap-2 px-4 py-2 rounded-full ${badge.className} text-sm font-medium mb-6`}
              >
                <badge.icon className="w-4 h-4" />
                Live Status: {badge.label}
              </div>
              <h1 className="text-4xl md:text-5xl font-bold text-ink leading-tight mb-6">
                SLOs & System Reliability
              </h1>
              <p className="text-xl text-inkMuted">
                We publish our service level objectives so you know exactly what to expect. When we miss,
                here&apos;s what happens.
              </p>
              {badge.lastChecked ? (
                <p className="mt-4 text-sm text-gray-500 font-mono">
                  Last checked: {badge.lastChecked} UTC{usedLkg ? ' (cached)' : ''}
                </p>
              ) : null}
            </motion.div>
          </div>
        </PageHero>

        {/* SLO Definitions (white section + contained surface, avoids “striped flip”) */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">Our Service Level Objectives</h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                These are the targets we hold ourselves to. Live component health is shown above and on the status page.
              </p>
              {computedSlo.windowStart && computedSlo.windowEnd ? (
                <p className="mt-3 text-sm text-gray-500 font-mono">
                  Rolling window: {computedSlo.windowDays}d ({computedSlo.windowStart} → {computedSlo.windowEnd})
                </p>
              ) : null}
            </motion.div>

            <div className="max-w-5xl mx-auto">
              <div className="rounded-2xl border border-gray-200 bg-gray-50 p-6 md:p-8 space-y-6">
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
                      {renderSloHealth(slo.unit)}
                    </div>
                  </motion.div>
                ))}
              </div>
            </div>
          </div>
        </section>

        <div className="h-px w-full bg-gray-200/70" />

        {/* How We Measure */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">How We Measure</h2>
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

        <div className="h-px w-full bg-gray-200/70" />

        {/* Mitigations (white section + contained surface) */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">What Happens When We Miss</h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                Automated mitigations kick in before humans need to act. Here&apos;s the protection stack.
              </p>
            </motion.div>

            <div className="max-w-5xl mx-auto">
              <div className="rounded-2xl border border-gray-200 bg-gray-50 p-6 md:p-8">
                <div className="grid md:grid-cols-2 gap-8">
                  {mitigations.map((mitigation, index) => (
                    <motion.div
                      key={mitigation.title}
                      {...getRevealProps(index * 0.1)}
                      className="bg-white rounded-xl p-6 shadow-sm border border-gray-100"
                    >
                      <div className="flex items-center gap-4 mb-4">
                        <div className="w-12 h-12 bg-warning/10 rounded-xl flex items-center justify-center">
                          <mitigation.icon className="w-6 h-6 text-warning" />
                        </div>
                        <h3 className="text-lg font-semibold text-gray-900">{mitigation.title}</h3>
                      </div>
                      <p className="text-gray-600 text-sm mb-4">{mitigation.description}</p>
                      <div className="bg-gray-50 rounded-lg px-4 py-2 border border-gray-100">
                        <span className="text-xs text-gray-500">Trigger: </span>
                        <span className="text-xs font-mono text-gray-700">{mitigation.trigger}</span>
                      </div>
                    </motion.div>
                  ))}
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* Live Snapshot */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">Live Status Snapshot</h2>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="space-y-3">
                {liveStatus ? (
                  <div className="p-4 bg-white rounded-lg border border-gray-100">
                    <div className="flex items-center justify-between gap-4">
                      <span className="text-sm font-semibold text-gray-900">Backend</span>
                      <span className="text-sm text-gray-600">{badge.label}</span>
                    </div>
                    <div className="mt-3 grid grid-cols-1 md:grid-cols-2 gap-3 text-sm text-gray-700">
                      <div className="flex items-center justify-between gap-4">
                        <span className="text-gray-500">API</span>
                        <span className="font-mono">
                          {liveStatus?.checks?.postgres?.connected
                            ? `${liveStatus?.checks?.postgres?.status ?? 'healthy'} (${liveStatus?.checks?.postgres?.latency_ms ?? '?'}ms)`
                            : '—'}
                        </span>
                      </div>
                      <div className="flex items-center justify-between gap-4">
                        <span className="text-gray-500">Database</span>
                        <span className="font-mono">{liveStatus?.checks?.postgres?.connected ? 'connected' : '—'}</span>
                      </div>
                    </div>
                    {badge.lastChecked && <p className="mt-3 text-xs text-gray-400">Last updated: {badge.lastChecked}</p>}
                  </div>
                ) : (
                  <div className="p-4 bg-gray-50 rounded-lg border border-gray-200 text-center">
                    <p className="text-sm text-gray-600">Status temporarily unavailable</p>
                  </div>
                )}
              </motion.div>

              <motion.div {...getRevealProps(0.2)} className="mt-8 text-center">
                <p className="text-sm text-gray-500 mb-4">For real-time status and incident updates, visit our status page.</p>
                <a href="/status" className="btn-primary inline-flex items-center gap-2">
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
              <h2 className="text-3xl md:text-4xl font-bold mb-6">Trust starts with transparency</h2>
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
      </main>

      <Footer />
      <CookieBanner />
    </>
  );
}
