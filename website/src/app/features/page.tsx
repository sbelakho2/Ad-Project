'use client';

import { useState } from 'react';
import { motion } from 'framer-motion';
import {
  Activity,
  ArrowRightLeft,
  BarChart3,
  Brain,
  CheckCircle,
  Code,
  FileSearch,
  GitBranch,
  Layers,
  Lock,
  Monitor,
  Search,
  Shield,
  ShieldCheck,
  Smartphone,
  Sparkles,
  Tv,
  Zap,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import { VisualCard } from '@/components/VisualCard';
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

const infrastructureFeatures = [
  {
    title: 'Circuit Breakers',
    description: 'Slow networks get isolated. One bad adapter never stalls your auction.',
    icon: Zap,
  },
  {
    title: 'OTA-Safe Rollouts',
    description: 'Staged traffic routing with automatic rollback. Zero app updates needed.',
    icon: GitBranch,
  },
  {
    title: 'Kill Switches',
    description: 'Disable any adapter instantly from the console. Take back control.',
    icon: Activity,
  },
  {
    title: 'Debug Panel',
    description: 'On-device waterfall tracing. See exactly what happened and why.',
    icon: FileSearch,
  },
];

const observabilityStack = [
  {
    title: 'Prometheus + Grafana',
    description: '24/7 monitoring with pre-built dashboards.',
    icon: BarChart3,
  },
  {
    title: 'Structured Logs',
    description: 'PII-redacted, audit-ready, compliance-friendly.',
    icon: Layers,
  },
  {
    title: 'Smart Alerts',
    description: 'Coverage drops and latency spikes surface quickly via metrics and alerting policies.',
    icon: Activity,
  },
  {
    title: 'Request Tracing',
    description: 'SDK to network response. Debug in minutes.',
    icon: Search,
  },
];

const sdkFeatures = [
  {
    platform: 'iOS',
    language: 'Swift',
    features: ['SwiftUI ready', 'Privacy Manifest', 'SKAdNetwork 4.0'],
  },
  {
    platform: 'Android',
    language: 'Kotlin',
    features: ['Jetpack Compose', 'Privacy Sandbox', 'Topics API'],
  },
  {
    platform: 'Web',
    language: 'TypeScript',
    features: ['Tree-shakeable', 'SSR compatible', 'TCF 2.2'],
  },
  {
    platform: 'Unity',
    language: 'C#',
    features: ['Cross-platform', 'Rewarded ads', 'Callbacks'],
  },
];

// ASCII code snippet components
function SDKInitCodeCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Android SDK - Initialize" className={className}>
{`// Application.onCreate()
ApexMediation.initialize(
    context = this,
    appId = "your-app-id"
)

// That's it. You're live.
// No XML. No manifest edits. No callbacks.`}
    </VisualCard>
  );
}

function SDKInterstitialCodeCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Android SDK - Show Interstitial" className={className}>
{`// Load an interstitial
val interstitial = ApexInterstitial(placementId)

interstitial.load()

// Show when ready
interstitial.show(activity) { result ->
    when (result) {
        is AdResult.Shown   -> // Ad displayed
        is AdResult.Clicked -> // User engaged
        is AdResult.Closed  -> // Continue app flow
    }
}`}
    </VisualCard>
  );
}

function ConsentCodeCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Android SDK - Consent" className={className}>
{`// Option 1: One-liner with Google UMP
ApexMediation.requestGoogleConsentForm(activity, callback = object : UmpConsentClient.Callback {
  override fun onConsentUpdated() {
    // (Optional) read IAB strings and pass to the SDK
    val state = ConsentManager.fromIabStorage(activity)
    ApexMediation.setConsent(
      tcString = state.tcfString,
      usPrivacy = state.usPrivacy,
      gdprApplies = state.gdprApplies
    )
  }

  override fun onConsentUpdateFailed(error: Throwable) {
    // Handle consent error
  }
})

// Option 2: Pass your own consent strings
ApexMediation.setConsent(
    tcString = "CPXxRfAPXxR...",  // IAB TCF v2.2
    usPrivacy = "1YNN",           // CCPA/CPRA
    gdprApplies = true
)

// Option 3: Read from CMP storage
val state = ConsentManager.fromIabStorage(context)
ApexMediation.setConsent(
    tcString = state.tcfString,
    usPrivacy = state.usPrivacy,
    gdprApplies = state.gdprApplies
)

// Consent signals are included in auction metadata and passed to adapters.
// Your app remains the consent source of truth.`}
    </VisualCard>
  );
}

function VRAArchitectureCard({ className }: { className?: string }) {
  return (
    <VisualCard title="VRA Pipeline" className={className}>
{`Network Reports ──┐
                   ├──▶ Normalizer ──▶ Matcher
SDK Paid Events ──┘         │            │
                            ▼            ▼
Crypto Receipts ────▶ Receipts Join ──▶ Reconciler
                                            │
                      ┌─────────────────────┤
                      ▼                     ▼
               Delta Classifier    Proof-of-Revenue
                      │
                      ▼
               Dispute Kit Builder`}
    </VisualCard>
  );
}

function MigrationStudioCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Migration Studio" className={className}>
{`┌──────────────────────────────────────────┐
│  Config v12 → Simulate → Canary → 100%  │
│                    │          │          │
│                    └─ rollback◀┘          │
└──────────────────────────────────────────┘

Shadow Mode:  Log without serving
Mirror Mode:  Test arm + production
Guardrails:   Latency, errors, eCPM
Kill Switch:  Instant revert`}
    </VisualCard>
  );
}

function TransparencyEngineCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Signed Auction Receipt" className={className}>
{`Auction #48291
├─ Timestamp:   2025-01-01T12:00:00Z
├─ Bids:        [A: $2.50, B: $2.45, C: $2.10]
├─ Winner:      Network A @ $2.50
├─ Floor:       $2.00
├─ Waterfall:   [A, B, C] (ML-optimized)
└─ Signature:   Ed25519:AbCd...1234 ✓

Verify:   canonical → verify → trust
Dispute:  cryptographic evidence`}
    </VisualCard>
  );
}

function MLOptimizerCard({ className }: { className?: string }) {
  return (
    <VisualCard title="ML Waterfall Optimizer" className={className}>
{`┌─ Bid Prediction Model ─────────────────┐
│                                         │
│  Context: [geo, time, device, history]  │
│      │                                  │
│      ▼                                  │
│  Predict: P(bid > floor | network)      │
│      │                                  │
│      ▼                                  │
│  Rank: Reorder waterfall by expected    │
│        revenue × fill probability       │
└─────────────────────────────────────────┘

Training:   Continuous from your traffic
Shadow ML:  Test before activating
A/B Test:   Statistical significance`}
    </VisualCard>
  );
}

const platformTabs = [
  {
    id: 'ctv',
    label: 'CTV SSAI',
    icon: Tv,
    outcomes: [
      'Server-side ad insertion for Apple TV, Fire TV, Roku, Android TV',
      'Live & VOD support with HLS and DASH manifests',
      'Signed logs for every insertion decision',
    ],
    diagram: `┌──────────────┐    ┌─────────────┐    ┌──────────────┐
│  CTV App     │───▶│  Apex SSAI  │───▶│  Stitched    │
│  (Player)    │    │  Server     │    │  Stream      │
└──────────────┘    └─────────────┘    └──────────────┘
                           │
                    ┌──────┴──────┐
                    │  Ad Server  │
                    └─────────────┘`,
    docsLink: '/docs/playback-matrix#ctv',
  },
  {
    id: 'web',
    label: 'Web Video',
    icon: Monitor,
    outcomes: [
      'TypeScript SDK for hls.js, dash.js, Video.js, and native players',
      'VAST/VPAID support with waterfall optimization',
      'Real-time analytics and debug panel in browser',
    ],
    diagram: `┌──────────────┐    ┌─────────────┐    ┌──────────────┐
│  Web Player  │───▶│  Apex SDK   │───▶│  Ad Request  │
│  (Browser)   │    │  (TS/JS)    │    │  + Response  │
└──────────────┘    └─────────────┘    └──────────────┘
                           │
                    ┌──────┴──────┐
                    │  Analytics  │
                    └─────────────┘`,
    docsLink: '/docs/playback-matrix#web-video',
  },
  {
    id: 'mobile',
    label: 'Mobile Apps',
    icon: Smartphone,
    outcomes: [
      'Native SDKs for iOS, Android, Unity with async/await APIs',
      'Background-only network calls, never blocks main thread',
      'Circuit breakers and per-adapter timeouts',
    ],
    diagram: `┌──────────────┐    ┌─────────────┐    ┌──────────────┐
│  Mobile App  │───▶│  Apex SDK   │───▶│  Ad Networks │
│  (Native)    │    │  (Native)   │    │  (8+)        │
└──────────────┘    └─────────────┘    └──────────────┘
                           │
                    ┌──────┴──────┐
                    │  Console    │
                    └─────────────┘`,
    docsLink: '/developers',
  },
];

function PlatformTabsSection() {
  const [activeTab, setActiveTab] = useState('ctv');
  const activePlatform = platformTabs.find(t => t.id === activeTab) || platformTabs[0];

  return (
    <section className="py-16 bg-gray-50">
      <div className="container mx-auto px-4">
        <div className="max-w-4xl mx-auto">
          {/* Tab buttons */}
          <motion.div {...getRevealProps(0)} className="flex justify-center gap-2 mb-12">
            {platformTabs.map((tab) => (
              <button
                key={tab.id}
                onClick={() => setActiveTab(tab.id)}
                className={`flex items-center gap-2 px-6 py-3 rounded-lg font-semibold transition-colors ${
                  activeTab === tab.id
                    ? 'bg-brand-600 text-white'
                    : 'bg-white text-gray-700 border border-gray-200 hover:bg-gray-50'
                }`}
              >
                <tab.icon className="w-5 h-5" />
                {tab.label}
              </button>
            ))}
          </motion.div>

          {/* Tab content */}
          <motion.div
            key={activeTab}
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.3 }}
            className="bg-white rounded-xl p-8 border border-gray-200"
          >
            <div className="grid md:grid-cols-2 gap-8">
              {/* Outcomes */}
              <div>
                <h3 className="text-xl font-bold text-gray-900 mb-4">What you get</h3>
                <ul className="space-y-3">
                  {activePlatform.outcomes.map((outcome, i) => (
                    <li key={i} className="flex items-start gap-3">
                      <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                      <span className="text-gray-700">{outcome}</span>
                    </li>
                  ))}
                </ul>
                <a
                  href={activePlatform.docsLink}
                  className="inline-flex items-center gap-2 mt-6 text-brand-600 font-semibold hover:underline"
                >
                  How to validate →
                </a>
              </div>

              {/* Diagram */}
              <div>
                <h3 className="text-xl font-bold text-gray-900 mb-4">Architecture</h3>
                <div className="bg-gray-900 rounded-lg p-4 font-mono text-xs text-gray-300 overflow-x-auto">
                  <pre>{activePlatform.diagram}</pre>
                </div>
              </div>
            </div>
          </motion.div>
        </div>
      </div>
    </section>
  );
}

export default function FeaturesPage() {
  return (
    <>
      <main className="relative bg-white text-slate-900">
        <SiteHeader />

        {/* Hero */}
        <PageHero variant="dark">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto">
              <div className="w-20 h-20 bg-brand-600 rounded-2xl flex items-center justify-center mx-auto mb-8">
                <Sparkles className="w-10 h-10 text-white" />
              </div>
              <h1 className="text-4xl md:text-5xl font-bold leading-tight text-white mb-6">
                Features that just work.
              </h1>
              <p className="text-xl leading-relaxed text-slate-200/90 max-w-3xl mx-auto">
                Unified API. Privacy handled. All networks, one integration.
                <br className="hidden md:block" />
                Copy, paste, ship.
              </p>
            </motion.div>
          </div>
        </PageHero>

        {/* Platform Tabs */}
        <PlatformTabsSection />

        {/* SDK Integration - TOP FEATURE #1 */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-16">
                <div className="inline-flex items-center gap-2 px-4 py-2 bg-brand-100 text-brand-700 rounded-full font-semibold text-sm mb-6">
                  <Code className="w-4 h-4" />
                  Developer Experience
                </div>
                <h2 className="text-4xl md:text-5xl font-bold text-gray-900 mb-6">
                  Integrate once. Access all networks.
                </h2>
                <p className="text-xl text-gray-600 max-w-3xl mx-auto">
                  Stop juggling 8 SDKs with 8 different APIs. One integration, every ad network.
                  <br className="hidden md:block" />
                  Three lines of Kotlin. That&apos;s it.
                </p>
              </motion.div>

              <div className="grid lg:grid-cols-2 gap-8 mb-12">
                <motion.div {...getRevealProps(0.1)}>
                  <SDKInitCodeCard className="shadow-xl" />
                </motion.div>
                <motion.div {...getRevealProps(0.15)}>
                  <SDKInterstitialCodeCard className="shadow-xl" />
                </motion.div>
              </div>

              <motion.div {...getRevealProps(0.2)} className="bg-gradient-to-r from-brand-50 to-green-50 rounded-2xl p-8 border border-brand-200">
                <div className="grid grid-cols-2 md:grid-cols-4 gap-6 text-center">
                  <div>
                    <div className="text-4xl font-bold text-brand-600 mb-2">3</div>
                    <div className="text-gray-600 font-medium">Lines to integrate</div>
                  </div>
                  <div>
                    <div className="text-4xl font-bold text-brand-600 mb-2">0</div>
                    <div className="text-gray-600 font-medium">XML config files</div>
                  </div>
                  <div>
                    <div className="text-4xl font-bold text-brand-600 mb-2">1</div>
                    <div className="text-gray-600 font-medium">Integration point</div>
                  </div>
                  <div>
                    <div className="text-4xl font-bold text-brand-600 mb-2">8+</div>
                    <div className="text-gray-600 font-medium">Ad networks</div>
                  </div>
                </div>
              </motion.div>

              <motion.div {...getRevealProps(0.25)} className="mt-12 grid md:grid-cols-3 gap-6">
                {[
                  {
                    icon: Smartphone,
                    title: 'Unified API',
                    description: 'Same interface for banners, interstitials, rewarded, native. Learn once, use everywhere.',
                  },
                  {
                    icon: Zap,
                    title: 'No boilerplate',
                    description: 'No lifecycle callbacks to wire. Minimal manifest wiring. No fragile XML plumbing.',
                  },
                  {
                    icon: GitBranch,
                    title: 'Server-controlled',
                    description: 'Change networks, waterfalls, and settings without app updates. OTA configuration.',
                  },
                ].map((item, index) => (
                  <motion.div
                    key={item.title}
                    {...getRevealProps(0.3 + index * 0.05)}
                    className="bg-white rounded-xl p-6 border border-gray-200 shadow-sm"
                  >
                    <item.icon className="w-10 h-10 text-brand-600 mb-4" />
                    <h3 className="font-bold text-gray-900 text-lg mb-2">{item.title}</h3>
                    <p className="text-gray-600">{item.description}</p>
                  </motion.div>
                ))}
              </motion.div>
            </div>
          </div>
        </section>

        {/* Consent Handling - TOP FEATURE #2 */}
        <section className="py-16 bg-gradient-to-b from-gray-50 to-white">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-16">
                <div className="inline-flex items-center gap-2 px-4 py-2 bg-green-100 text-green-700 rounded-full font-semibold text-sm mb-6">
                  <ShieldCheck className="w-4 h-4" />
                  Privacy First
                </div>
                <h2 className="text-4xl md:text-5xl font-bold text-gray-900 mb-6">
                  Consent? Handled.
                </h2>
                <p className="text-xl text-gray-600 max-w-3xl mx-auto">
                  GDPR, CCPA, COPPA, TCF 2.2, US Privacy—all flow through automatically.
                  <br className="hidden md:block" />
                  Three ways to pass consent. All of them easy.
                </p>
              </motion.div>

              <div className="grid lg:grid-cols-2 gap-12 items-start">
                <motion.div {...getRevealProps(0.1)}>
                  <ConsentCodeCard className="shadow-xl" />
                </motion.div>

                <motion.div {...getRevealProps(0.15)} className="space-y-6">
                  <div className="bg-white rounded-xl p-6 border border-gray-200 shadow-sm">
                    <div className="flex items-start gap-4">
                      <div className="w-12 h-12 bg-green-100 rounded-xl flex items-center justify-center flex-shrink-0">
                        <span className="text-2xl font-bold text-green-600">1</span>
                      </div>
                      <div>
                        <h3 className="font-bold text-gray-900 text-lg mb-2">Google UMP integration</h3>
                        <p className="text-gray-600">
                          One-liner to show the Google User Messaging Platform consent form.
                          We handle the callbacks and propagate to all networks.
                        </p>
                      </div>
                    </div>
                  </div>

                  <div className="bg-white rounded-xl p-6 border border-gray-200 shadow-sm">
                    <div className="flex items-start gap-4">
                      <div className="w-12 h-12 bg-green-100 rounded-xl flex items-center justify-center flex-shrink-0">
                        <span className="text-2xl font-bold text-green-600">2</span>
                      </div>
                      <div>
                        <h3 className="font-bold text-gray-900 text-lg mb-2">Direct consent strings</h3>
                        <p className="text-gray-600">
                          Pass TCF v2.2, US Privacy, or custom consent directly.
                          Perfect for apps with existing CMP solutions.
                        </p>
                      </div>
                    </div>
                  </div>

                  <div className="bg-white rounded-xl p-6 border border-gray-200 shadow-sm">
                    <div className="flex items-start gap-4">
                      <div className="w-12 h-12 bg-green-100 rounded-xl flex items-center justify-center flex-shrink-0">
                        <span className="text-2xl font-bold text-green-600">3</span>
                      </div>
                      <div>
                        <h3 className="font-bold text-gray-900 text-lg mb-2">Read IAB-standard consent storage (opt-in)</h3>
                        <p className="text-gray-600">
                          Use the SDK helper to read IAB-standard consent keys (TCF + US Privacy) from
                          app storage, then pass them into the SDK. There is no background scraping.
                        </p>
                      </div>
                    </div>
                  </div>

                  <div className="bg-gradient-to-r from-green-50 to-brand-50 rounded-xl p-6 border border-green-200">
                    <div className="flex items-start gap-3">
                      <CheckCircle className="w-6 h-6 text-green-600 flex-shrink-0 mt-0.5" />
                      <div>
                        <h4 className="font-bold text-gray-900 mb-1">Consistent signal plumbing</h4>
                        <p className="text-gray-600 text-sm">
                          Set consent once. The SDK includes consent signals in auction metadata and makes
                          them available to adapters. Partner behavior depends on each integration.
                        </p>
                      </div>
                    </div>
                  </div>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* VRA Section */}
        <section className="py-16">
          <div className="section-divider mb-16" />
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid lg:grid-cols-2 gap-12 items-start">
                <motion.div {...getRevealProps(0)}>
                  <div className="flex items-center gap-3 mb-4">
                    <div className="w-12 h-12 bg-brand-600 rounded-xl flex items-center justify-center">
                      <Shield className="w-6 h-6 text-white" />
                    </div>
                    <span className="px-2 py-1 bg-green-100 text-green-700 text-xs font-semibold rounded">Live</span>
                  </div>
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                    Verifiable Revenue Auditor
                  </h2>
                  <p className="text-lg text-gray-600 mb-6 max-w-[55ch]">
                    Cross-reference network reports with SDK events and signed auction receipts.
                    Detect discrepancies automatically. Generate dispute kits with confidence scores.
                  </p>
                  <ul className="space-y-3 mb-6">
                    {[
                      'Ingest network statements via API, CSV, or SFTP',
                      'Auto-normalize and match against SDK events',
                      'Classify deltas: underpay, IVT, timing, FX',
                      'Monthly Proof-of-Revenue certificates',
                      'Export to BigQuery, S3, or GCS',
                    ].map((item) => (
                      <li key={item} className="flex items-start gap-2 text-gray-700">
                        <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                        {item}
                      </li>
                    ))}
                  </ul>
                </motion.div>
                <motion.div {...getRevealProps(0.1)}>
                  <VRAArchitectureCard className="shadow-lg" />
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Migration Studio Section */}
        <section className="py-16 section-tinted">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid lg:grid-cols-2 gap-12 items-start">
                <motion.div {...getRevealProps(0.1)} className="order-2 lg:order-1">
                  <MigrationStudioCard className="shadow-lg" />
                </motion.div>
                <motion.div {...getRevealProps(0)} className="order-1 lg:order-2">
                  <div className="flex items-center gap-3 mb-4">
                    <div className="w-12 h-12 bg-brand-600 rounded-xl flex items-center justify-center">
                      <ArrowRightLeft className="w-6 h-6 text-white" />
                    </div>
                    <span className="px-2 py-1 bg-green-100 text-green-700 text-xs font-semibold rounded">Live</span>
                  </div>
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                    Migration Studio
                  </h2>
                  <p className="text-lg text-gray-600 mb-6 max-w-[55ch]">
                    Switch mediators without risking revenue. Shadow test, mirror traffic,
                    and validate eCPM parity before cutting over—with automatic rollback if guardrails trigger.
                  </p>
                  <ul className="space-y-3 mb-6">
                    {[
                      'Shadow mode: log without affecting live serving',
                      'Mirror mode: run test arm alongside production',
                      'Guardrails: latency, error rate, and eCPM thresholds',
                      'Staged rollout: 5% → 25% → 100%',
                      'Kill-switch: instant revert to control',
                    ].map((item) => (
                      <li key={item} className="flex items-start gap-2 text-gray-700">
                        <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                        {item}
                      </li>
                    ))}
                  </ul>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Transparency Engine Section */}
        <section className="py-16">
          <div className="section-divider mb-16" />
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid lg:grid-cols-2 gap-12 items-start">
                <motion.div {...getRevealProps(0)}>
                  <div className="flex items-center gap-3 mb-4">
                    <div className="w-12 h-12 bg-brand-600 rounded-xl flex items-center justify-center">
                      <Lock className="w-6 h-6 text-white" />
                    </div>
                    <span className="px-2 py-1 bg-brand-100 text-brand-700 text-xs font-semibold rounded">Core</span>
                  </div>
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                    Transparency Engine
                  </h2>
                  <p className="text-lg text-gray-600 mb-6 max-w-[55ch]">
                    Every auction produces a tamper-evident record. Bids received, waterfall order,
                    floor prices, winner selection—all Ed25519-signed and exportable.
                  </p>
                  <ul className="space-y-3 mb-6">
                    {[
                      'Immutable audit trail for every decision',
                      'Cryptographic proof of what actually ran',
                      'Export logs for third-party verification',
                      'On-device Debug Panel for real-time inspection',
                      'Dispute-ready evidence, not screenshots',
                    ].map((item) => (
                      <li key={item} className="flex items-start gap-2 text-gray-700">
                        <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                        {item}
                      </li>
                    ))}
                  </ul>
                </motion.div>
                <motion.div {...getRevealProps(0.1)}>
                  <TransparencyEngineCard className="shadow-lg" />
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* ML Optimizer Section */}
        <section className="py-16 section-tinted">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid lg:grid-cols-2 gap-12 items-start">
                <motion.div {...getRevealProps(0.1)} className="order-2 lg:order-1">
                  <MLOptimizerCard className="shadow-lg" />
                </motion.div>
                <motion.div {...getRevealProps(0)} className="order-1 lg:order-2">
                  <div className="flex items-center gap-3 mb-4">
                    <div className="w-12 h-12 bg-brand-600 rounded-xl flex items-center justify-center">
                      <Brain className="w-6 h-6 text-white" />
                    </div>
                    <span className="px-2 py-1 bg-green-100 text-green-700 text-xs font-semibold rounded">Live</span>
                  </div>
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                    ML Waterfall Optimizer
                  </h2>
                  <p className="text-lg text-gray-600 mb-6 max-w-[55ch]">
                    Our models predict bid responses and reorder waterfalls in real-time.
                    Continuously learning from your traffic to maximize fill and eCPM.
                  </p>
                  <ul className="space-y-3 mb-6">
                    {[
                      'Real-time bid prediction per network',
                      'Contextual ranking: geo, time, device, history',
                      'A/B testing with statistical significance',
                      'Shadow ML: test models before activation',
                      'Publisher-configurable optimization goals',
                    ].map((item) => (
                      <li key={item} className="flex items-start gap-2 text-gray-700">
                        <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
                        {item}
                      </li>
                    ))}
                  </ul>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Infrastructure */}
        <section className="py-16">
          <div className="section-divider mb-16" />
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-16">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Reliability infrastructure
              </h2>
              <p className="text-lg text-gray-600 max-w-2xl mx-auto">
                Protections that keep your ads serving when things go wrong.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-2 lg:grid-cols-4 gap-6 max-w-6xl mx-auto">
              {infrastructureFeatures.map((feature, index) => (
                <motion.div
                  key={feature.title}
                  {...getRevealProps(index * 0.1)}
                  className="bg-gray-50 rounded-xl p-6 border border-gray-200"
                >
                  <feature.icon className="w-10 h-10 text-brand-600 mb-4" />
                  <h3 className="font-bold text-gray-900 mb-2">{feature.title}</h3>
                  <p className="text-sm text-gray-600">{feature.description}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Observability */}
        <section className="py-16 section-tinted">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid md:grid-cols-2 gap-12 items-center">
                <motion.div {...getRevealProps(0)}>
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-6">
                    Full-stack observability
                  </h2>
                  <p className="text-lg text-gray-600 mb-8 max-w-[55ch]">
                    Monitor every layer. From SDK to network response,
                    see exactly what&apos;s happening—and set clear alert thresholds.
                  </p>
                  <div className="grid grid-cols-2 gap-4">
                    {observabilityStack.map((item, index) => (
                      <motion.div
                        key={item.title}
                        {...getRevealProps(index * 0.1)}
                        className="bg-white rounded-lg p-4 border border-gray-200"
                      >
                        <item.icon className="w-6 h-6 text-brand-600 mb-2" />
                        <h4 className="font-semibold text-gray-900 text-sm mb-1">{item.title}</h4>
                        <p className="text-xs text-gray-600">{item.description}</p>
                      </motion.div>
                    ))}
                  </div>
                </motion.div>

                <motion.div {...getRevealProps(0.1)}>
                  <div className="bg-gray-900 rounded-xl p-6 text-sm font-mono">
                    <div className="flex items-center gap-2 mb-4">
                      <div className="w-3 h-3 rounded-full bg-red-500" />
                      <div className="w-3 h-3 rounded-full bg-yellow-500" />
                      <div className="w-3 h-3 rounded-full bg-green-500" />
                      <span className="text-gray-400 ml-2">example metrics</span>
                    </div>
                    <pre className="text-green-400 overflow-x-auto text-xs">
{`# VRA reconciliation
vra_coverage_ratio{network="admob"}  0.98
vra_delta_count{kind="underpay"}     3
vra_proof_verified_total             847

# Migration Studio
migration_ecpm_ratio{arm="test"}     1.02
migration_latency_p95_ms             45
migration_guardrail_triggers         0

# Auction health
rtb_auction_p99_ms                   38
rtb_fill_rate{placement="rewarded"}  0.94
rtb_errors{code="timeout"}           2`}
                    </pre>
                  </div>
                </motion.div>
              </div>

              <div className="mt-10 rounded-xl border border-gray-200 bg-gray-50 p-6">
                <h3 className="text-lg font-semibold text-gray-900">Debug Panel</h3>
                <p className="mt-2 text-sm text-gray-600 max-w-prose">
                  A lightweight, on-device view of what the SDK did during an auction—useful for reproducing issues and sharing a sanitized trace with your team.
                </p>
                <ul className="mt-4 space-y-2 text-sm text-gray-700">
                  <li>• Waterfall order and per-adapter outcomes (shown/filled/timeout)</li>
                  <li>• Latency breakdowns and time budget visibility</li>
                  <li>• Consent and policy signals surfaced for debugging (no raw PII)</li>
                  <li>• Export/share a sanitized trace bundle for support</li>
                  <li>• Correlate traces with Prometheus/Grafana metrics</li>
                </ul>
              </div>
            </div>
          </div>
        </section>

        {/* SDKs */}
        <section className="py-16 bg-gray-900">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-white mb-4">
                Native SDKs, native performance
              </h2>
              <p className="text-lg text-gray-400 max-w-2xl mx-auto">
                Built for each platform. Privacy-first. Fully documented.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-4 gap-6 max-w-5xl mx-auto">
              {sdkFeatures.map((sdk, index) => (
                <motion.div
                  key={sdk.platform}
                  {...getRevealProps(index * 0.1)}
                  className="bg-gray-800 rounded-xl p-6"
                >
                  <div className="flex items-center justify-between mb-4">
                    <h3 className="text-xl font-bold text-white">{sdk.platform}</h3>
                    <span className="text-xs text-gray-400 font-mono">{sdk.language}</span>
                  </div>
                  <ul className="space-y-2">
                    {sdk.features.map((feature) => (
                      <li key={feature} className="flex items-center gap-2 text-sm text-gray-300">
                        <Sparkles className="w-3 h-3 text-brand-400" />
                        {feature}
                      </li>
                    ))}
                  </ul>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* CTA */}
        <section className="py-16">
          <div className="section-divider mb-16" />
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto text-center">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-6">
                See the features in action
              </h2>
              <p className="text-lg text-gray-600 mb-8">
                Schedule a technical deep-dive. We&apos;ll walk through SDK integration,
                consent handling, VRA, and Migration Studio.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                <a
                  href="/contact"
                  className="btn-primary"
                >
                  Request a demo
                </a>
                <a
                  href="/developers"
                  className="btn-secondary"
                >
                  Read the docs
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
