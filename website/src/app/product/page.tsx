'use client';

import { motion } from 'framer-motion';
import {
  Box,
  CheckCircle,
  GitBranch,
  Lock,
  Monitor,
  Search,
  Server,
  ShieldCheck,
  Smartphone,
  Tv,
  Zap,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import {
  SignatureChainCard,
  MigrationPipelineCard,
  CircuitBreakerCard,
} from '@/components/VisualCard';
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

const byoKeyPoints = [
  'You own all ad network accounts, contracts and payouts.',
  'ApexMediation never arbitrages or resells your demand.',
  'Our adapters and S2S integrations are built to be network-agnostic.',
  'You can swap networks in and out without touching our commercial terms.',
];

const transparencySubsections = [
  {
    title: 'Verifiable auction logs',
    description:
      'Export log-level auctions to your data warehouse and tie each impression back to a signed record.',
  },
  {
    title: 'Dispute-ready evidence',
    description:
      "When reconciling with networks or partners, you have more than screenshots. You have cryptographically signed logs showing what your mediation layer actually did.",
  },
  {
    title: 'Data-warehouse friendly',
    description:
      'Streaming schema is documented and stable, so you can wire ApexMediation into BigQuery or export to object storage (S3/GCS) for your existing tools.',
  },
];

const migrationCapabilities = [
  'Import your existing placements and waterfalls.',
  'Simulate auctions with your current rules vs ApexMediation.',
  'Enable ApexMediation per-placement, with instant rollback.',
  'Track results over time with side-by-side reports.',
];

const sdkPlatforms = [
  {
    platform: 'Android & Android TV',
    icon: Smartphone,
    features: [
      'Background-only network I/O and disk access',
      'StrictMode fatal in debug builds to catch violations early',
      'Circuit breakers and timeouts per adapter',
      'Lightweight footprint and well-documented ProGuard/R8 rules',
    ],
  },
  {
    platform: 'iOS & tvOS',
    icon: Tv,
    features: [
      'async/await-based API surface',
      'MainActor limited to UI work only',
      'Config signature verification and safe fallbacks',
      'Extension-safe guards',
    ],
  },
  {
    platform: 'Unity',
    icon: Monitor,
    features: [
      'IL2CPP and Mono support',
      'Clean C# API with native bridges hidden away',
      'Debug overlay for placements and adapter status',
    ],
  },
  {
    platform: 'Web',
    icon: Server,
    features: [
      'TypeScript SDK with strict types',
      'No surprises in how events are fired or batched',
    ],
  },
];

const observabilityFeatures = [
  'Per-adapter latency, fill, error and timeout metrics.',
  'SLOs and alerts for auction deadlines and error rates.',
  'Mediation debugger with sanitized request/response timeline.',
  'Supply chain tools: app-ads.txt and sellers.json inspector.',
  'Fraud and quality dashboard (shadow-mode ML, if enabled).',
];

export default function ProductPage() {
  return (
    <>
      <main className="relative bg-white text-slate-900">
        <SiteHeader />

        {/* Hero (ONLY dark block on the page) */}
        <PageHero variant="dark">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto">
              <h1 className="text-4xl md:text-5xl font-bold leading-tight text-white mb-6">
                One mediation layer. Your networks. Your rules.
              </h1>
              <p className="text-xl leading-relaxed text-slate-200/90">
                ApexMediation is the neutral orchestration layer between your apps and your ad networks,
                with SDKs tuned for reliability and a console built for serious teams.
              </p>
              <p className="mt-6 text-sm text-slate-300/80">
                Shipping SSAI for CTV or web video?{' '}
                <a
                  href="/docs/ssai"
                  className="text-white underline underline-offset-4 hover:text-slate-100"
                >
                  Read the SSAI docs
                </a>
                .
              </p>
            </motion.div>
          </div>
        </PageHero>

        {/* BYO Model */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="rounded-2xl border border-gray-200 bg-gray-50 p-8 md:p-10">
                <div className="grid lg:grid-cols-2 gap-12 items-start">
                  <motion.div {...getRevealProps(0)}>
                    <div className="flex items-center gap-3 mb-4">
                      <Box className="w-8 h-8 text-brand-600" />
                      <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                        BYO: Bring Your Own demand.
                      </h2>
                    </div>
                    <p className="text-lg text-gray-600 mb-6 max-w-prose">
                      Managed demand bundles networks under one vendor. BYO mediation keeps you in charge
                      of your ad relationships—ApexMediation handles the infrastructure.
                    </p>
                    <ul className="space-y-3">
                      {byoKeyPoints.map((point, index) => (
                        <li key={index} className="flex items-start gap-3">
                          <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-1" />
                          <span className="text-gray-700">{point}</span>
                        </li>
                      ))}
                    </ul>
                  </motion.div>

                  <motion.div {...getRevealProps(0.1)} className="hidden lg:block">
                    <div className="ml-auto max-w-sm">
                      <CircuitBreakerCard className="shadow-lg" />
                    </div>
                  </motion.div>
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* Divider */}
        <div className="h-px w-full bg-gray-200/70" />

        {/* Cryptographic Transparency */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid lg:grid-cols-2 gap-12 items-start">
                <motion.div {...getRevealProps(0)} className="hidden lg:block">
                  <div className="max-w-sm">
                    <SignatureChainCard className="shadow-lg" />
                  </div>
                </motion.div>

                <motion.div {...getRevealProps(0.1)}>
                  <div className="flex items-center gap-3 mb-4">
                    <Lock className="w-8 h-8 text-brand-600" />
                    <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                      Cryptographic transparency.
                    </h2>
                  </div>
                  <p className="text-lg text-gray-600 mb-6 max-w-prose">
                    Each auction produces a tamper-evident record: bids, waterfall order, floors, and the
                    winning decision. These records are hashed and signed.
                  </p>

                  <div className="space-y-4">
                    {transparencySubsections.map((item) => (
                      <div
                        key={item.title}
                        className="bg-gray-50 rounded-xl p-5 border border-gray-200"
                      >
                        <h3 className="font-semibold text-gray-900 mb-1">{item.title}</h3>
                        <p className="text-gray-600 text-sm">{item.description}</p>
                      </div>
                    ))}
                  </div>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Divider */}
        <div className="h-px w-full bg-gray-200/70" />

        {/* Migration Studio */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="rounded-2xl border border-gray-200 bg-gray-50 p-8 md:p-10">
                <div className="grid lg:grid-cols-2 gap-12 items-start">
                  <motion.div {...getRevealProps(0)}>
                    <div className="flex items-center gap-3 mb-4">
                      <GitBranch className="w-8 h-8 text-brand-600" />
                      <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                        Migration Studio.
                      </h2>
                    </div>
                    <p className="text-lg text-gray-600 mb-6 max-w-prose">
                      Most teams stay on their existing mediation for one reason: fear. Migration Studio
                      is built to reduce that fear.
                    </p>
                    <ul className="space-y-3 mb-6">
                      {migrationCapabilities.map((cap, index) => (
                        <li key={index} className="flex items-start gap-3">
                          <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-1" />
                          <span className="text-gray-700">{cap}</span>
                        </li>
                      ))}
                    </ul>
                    <p className="text-gray-900 font-semibold">
                      Result: you never have to &quot;flip the big switch&quot; all at once.
                    </p>
                  </motion.div>

                  <motion.div {...getRevealProps(0.1)} className="hidden lg:block">
                    <div className="ml-auto max-w-sm">
                      <MigrationPipelineCard className="shadow-lg" />
                    </div>
                  </motion.div>
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* Comparison Table */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                  ApexMediation vs. legacy mediators
                </h2>
                <p className="text-lg text-gray-600">
                  See how we compare on the things that matter.
                </p>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="overflow-x-auto">
                <div className="rounded-2xl border border-gray-200 overflow-hidden bg-white">
                  <table className="w-full border-collapse">
                    <thead>
                      <tr className="border-b border-gray-200">
                        <th className="text-left py-4 px-4 font-semibold text-gray-900">
                          Feature
                        </th>
                        <th className="text-center py-4 px-4 font-semibold text-brand-700 bg-brand-50">
                          ApexMediation
                        </th>
                        <th className="text-center py-4 px-4 font-semibold text-gray-600">
                          Legacy Mediators
                        </th>
                      </tr>
                    </thead>
                    <tbody className="text-sm">
                      {[
                        ['Keep your ad network contracts', true, 'Sometimes'],
                        ['Signed, verifiable auction logs', true, 'No'],
                        ['Per-placement gradual migration', true, 'Rarely'],
                        ['Per-adapter circuit breakers', true, 'No'],
                        ['Data warehouse export', true, 'Limited'],
                        ['No hidden take rate (transparent platform fee)', true, 'Hidden fees common'],
                      ].map(([label, apex, legacy], i) => (
                        <tr key={String(label)} className={i === 5 ? '' : 'border-b border-gray-100'}>
                          <td className="py-3 px-4 text-gray-700">{label as string}</td>
                          <td className="py-3 px-4 text-center bg-brand-50">
                            {apex ? (
                              <CheckCircle className="w-5 h-5 text-brand-600 mx-auto" />
                            ) : (
                              <span className="text-gray-400">—</span>
                            )}
                          </td>
                          <td className="py-3 px-4 text-center text-gray-500">{legacy as string}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </motion.div>
            </div>
          </div>
        </section>

        {/* SDKs */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-5xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <div className="flex items-center justify-center gap-3 mb-4">
                  <Zap className="w-8 h-8 text-brand-600" />
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                    SDKs that just work.
                  </h2>
                </div>
                <p className="text-lg text-gray-600 max-w-prose mx-auto">
                  We design the ApexMediation SDKs to be predictable, well-behaved citizens of your app.
                </p>
              </motion.div>

              <div className="grid md:grid-cols-2 gap-8">
                {sdkPlatforms.map((platform, index) => (
                  <motion.div
                    key={platform.platform}
                    {...getRevealProps(index * 0.1)}
                    className="bg-white rounded-2xl p-6 border border-gray-200 shadow-sm"
                  >
                    <div className="flex items-center gap-3 mb-4">
                      <platform.icon className="w-6 h-6 text-brand-600" />
                      <h3 className="text-xl font-semibold text-gray-900">
                        {platform.platform}
                      </h3>
                    </div>
                    <ul className="space-y-2">
                      {platform.features.map((feature, i) => (
                        <li key={i} className="flex items-start gap-2 text-sm text-gray-600">
                          <CheckCircle className="w-4 h-4 text-brand-600 flex-shrink-0 mt-0.5" />
                          {feature}
                        </li>
                      ))}
                    </ul>
                  </motion.div>
                ))}
              </div>
            </div>
          </div>
        </section>

        {/* CTV & Web Video SSAI */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-5xl mx-auto">
              <motion.div {...getRevealProps(0)}>
                <div className="flex items-center gap-3 mb-4">
                  <Tv className="w-8 h-8 text-brand-600" />
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                    For CTV & Web Video: SSAI that&apos;s debuggable
                  </h2>
                </div>
                <p className="text-lg text-gray-600 mb-6 max-w-prose">
                  Server-side ad insertion should be deterministic and auditable. ApexMediation provides
                  full visibility into every ad decision.
                </p>

                <div className="grid md:grid-cols-2 gap-8 mb-8">
                  {[
                    {
                      title: 'Deterministic behavior',
                      desc: 'Same inputs produce same outputs. Reproduce any ad decision with the exact context.',
                    },
                    {
                      title: 'Explainable traces',
                      desc: 'Every SSAI decision includes a full trace: waterfall order, bid responses, winner selection.',
                    },
                    {
                      title: 'Signed evidence',
                      desc: 'Cryptographically signed logs for every insertion. Dispute-ready documentation.',
                    },
                    {
                      title: 'Safe-mode & rollback',
                      desc: 'Automatic fallback to safe configurations. Instant rollback if guardrails trigger.',
                    },
                  ].map((card) => (
                    <div
                      key={card.title}
                      className="bg-gray-50 rounded-2xl p-6 border border-gray-200"
                    >
                      <h3 className="font-semibold text-gray-900 mb-3">{card.title}</h3>
                      <p className="text-gray-600 text-sm">{card.desc}</p>
                    </div>
                  ))}
                </div>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Observability */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-prose mx-auto">
              <motion.div {...getRevealProps(0)}>
                <div className="flex items-center gap-3 mb-4">
                  <Search className="w-8 h-8 text-brand-600" />
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                    See exactly what your mediation is doing.
                  </h2>
                </div>

                <div className="rounded-2xl border border-gray-200 bg-gray-50 p-6">
                  <ul className="space-y-3">
                    {observabilityFeatures.map((feature, index) => (
                      <li key={index} className="flex items-start gap-3">
                        <ShieldCheck className="w-5 h-5 text-brand-600 flex-shrink-0 mt-1" />
                        <span className="text-gray-700">{feature}</span>
                      </li>
                    ))}
                  </ul>

                  <div className="mt-8 flex flex-wrap gap-4">
                    <a href="/developers" className="btn-primary px-6 py-3">
                      Explore SDKs
                    </a>
                    <a href="/docs/ssai/prerequisites" className="btn-secondary px-6 py-3">
                      SSAI Buyer Checklist
                    </a>
                    <a href="/docs/playback-matrix" className="btn-secondary px-6 py-3">
                      Playback Matrix
                    </a>
                  </div>
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
