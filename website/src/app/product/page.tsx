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
import HomeNav from '@/components/HomeNav';

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
    description: 'Export log-level auctions to your data warehouse and tie each impression back to a signed record.',
  },
  {
    title: 'Dispute-ready evidence',
    description: 'When reconciling with networks or partners, you have more than screenshots. You have cryptographically signed logs showing what your mediation layer actually did.',
  },
  {
    title: 'Data-warehouse friendly',
    description: 'Streaming schema is documented and stable, so you can wire ApexMediation directly into ClickHouse, BigQuery, Snowflake or your existing tools.',
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
      <main className="relative bg-white text-gray-900">
        <div className="container relative z-10 py-6 md:py-10">
          <HomeNav />
        </div>

        {/* Hero */}
        <section className="py-20 md:py-32">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-4xl mx-auto">
              <h1 className="text-4xl md:text-6xl font-bold text-gray-900 leading-tight mb-6">
                One mediation layer. Your networks. Your rules.
              </h1>
              <p className="text-xl md:text-2xl text-gray-600 max-w-3xl mx-auto">
                ApexMediation is the neutral orchestration layer between your apps and your ad networks, with SDKs tuned for reliability and a console built for serious teams.
              </p>
            </motion.div>
          </div>
        </section>

        {/* BYO Model */}
        <section className="py-20 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)}>
                <div className="flex items-center gap-3 mb-4">
                  <Box className="w-8 h-8 text-brand-600" />
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                    BYO: Bring Your Own demand and keep control.
                  </h2>
                </div>
                <p className="text-lg text-gray-600 mb-8">
                  Managed demand bundles networks and negotiations under one vendor. BYO mediation keeps you in charge of your ad relationships, while ApexMediation focuses on the infrastructure: auctions, SDKs, observability, and safety.
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
            </div>
          </div>
        </section>

        {/* Cryptographic Transparency */}
        <section className="py-20">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)}>
                <div className="flex items-center gap-3 mb-4">
                  <Lock className="w-8 h-8 text-brand-600" />
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                    Cryptographic transparency: prove what ran.
                  </h2>
                </div>
                <p className="text-lg text-gray-600 mb-8">
                  Each auction ApexMediation runs produces a tamper-evident record: bids received, waterfall order, floor prices, and the winning decision. These records are hashed and signed, so they can&apos;t be silently altered after the fact.
                </p>
                <div className="grid md:grid-cols-3 gap-6">
                  {transparencySubsections.map((item, index) => (
                    <motion.div
                      key={item.title}
                      {...getRevealProps(index * 0.1)}
                      className="bg-gray-50 rounded-xl p-6"
                    >
                      <h3 className="text-lg font-semibold text-gray-900 mb-2">
                        {item.title}
                      </h3>
                      <p className="text-gray-600 text-sm">
                        {item.description}
                      </p>
                    </motion.div>
                  ))}
                </div>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Migration Studio */}
        <section className="py-20 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)}>
                <div className="flex items-center gap-3 mb-4">
                  <GitBranch className="w-8 h-8 text-brand-600" />
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                    Migration Studio: change mediation without breaking anything.
                  </h2>
                </div>
                <p className="text-lg text-gray-600 mb-8">
                  Most teams stay on their existing mediation for one reason: fear. Fear that changing will break monetization, crash apps, or tank ARPDAU. Migration Studio is built to reduce that fear.
                </p>
                <ul className="space-y-3 mb-6">
                  {migrationCapabilities.map((cap, index) => (
                    <li key={index} className="flex items-start gap-3">
                      <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-1" />
                      <span className="text-gray-700">{cap}</span>
                    </li>
                  ))}
                </ul>
                <p className="text-gray-700 font-medium">
                  Result: you never have to &quot;flip the big switch&quot; all at once.
                </p>
              </motion.div>
            </div>
          </div>
        </section>

        {/* SDKs */}
        <section className="py-20">
          <div className="container mx-auto px-4">
            <div className="max-w-5xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <div className="flex items-center justify-center gap-3 mb-4">
                  <Zap className="w-8 h-8 text-brand-600" />
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                    SDKs that feel like high-quality framework code—not an afterthought.
                  </h2>
                </div>
                <p className="text-lg text-gray-600 max-w-3xl mx-auto">
                  We design the ApexMediation SDKs to be predictable, well-behaved citizens of your app.
                </p>
              </motion.div>
              <div className="grid md:grid-cols-2 gap-8">
                {sdkPlatforms.map((platform, index) => (
                  <motion.div
                    key={platform.platform}
                    {...getRevealProps(index * 0.1)}
                    className="bg-gray-50 rounded-xl p-6"
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

        {/* Observability */}
        <section className="py-20 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)}>
                <div className="flex items-center gap-3 mb-4">
                  <Search className="w-8 h-8 text-brand-600" />
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                    See exactly what your mediation is doing.
                  </h2>
                </div>
                <ul className="space-y-3">
                  {observabilityFeatures.map((feature, index) => (
                    <li key={index} className="flex items-start gap-3">
                      <ShieldCheck className="w-5 h-5 text-brand-600 flex-shrink-0 mt-1" />
                      <span className="text-gray-700">{feature}</span>
                    </li>
                  ))}
                </ul>
                <div className="mt-8">
                  <a href="/developers" className="btn-primary px-6 py-3">
                    Explore console screenshots →
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
