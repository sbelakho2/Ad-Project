'use client';

import { motion } from 'framer-motion';
import {
  BookOpen,
  Code,
  GitBranch,
  Palette,
  Play,
  Puzzle,
  ShieldCheck,
  Smartphone,
  Terminal,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

const sdks = [
  {
    platform: 'Android',
    icon: Smartphone,
    color: 'bg-green-500',
    repo: 'apex-mediation-platform',
    repoPath: 'sdk/core/android',
    description: 'Kotlin-first SDK for Android apps targeting API 21+.',
    features: ['Compose & View support', 'Kotlin coroutines', 'ProGuard rules included'],
  },
  {
    platform: 'Android TV',
    icon: Smartphone,
    color: 'bg-green-600',
    repo: 'apex-mediation-platform',
    repoPath: 'sdk/core/android',
    description: 'Lean-back optimized SDK for Android TV and Fire TV.',
    features: ['D-pad navigation', 'Large screen layouts', 'Same Kotlin API'],
  },
  {
    platform: 'iOS',
    icon: Smartphone,
    color: 'bg-blue-500',
    repo: 'apex-mediation-platform',
    repoPath: 'sdk/core/ios',
    description: 'Swift SDK for iOS 14+ with full SwiftUI support.',
    features: ['Swift async/await', 'SwiftUI views', 'Privacy manifests included'],
  },
  {
    platform: 'tvOS',
    icon: Smartphone,
    color: 'bg-blue-600',
    repo: 'apex-mediation-platform',
    repoPath: 'sdk/core/ios',
    description: 'Native tvOS SDK for Apple TV apps.',
    features: ['Focus engine support', 'Top Shelf integration', 'Same Swift API'],
  },
  {
    platform: 'Unity',
    icon: Puzzle,
    color: 'bg-purple-500',
    repo: 'apex-mediation-platform',
    repoPath: 'sdk/core/unity',
    description: 'C# plugin for Unity 2021+ with native bridge.',
    features: ['IL2CPP support', 'Editor testing', 'Auto-dependency resolution'],
  },
  {
    platform: 'Web (JS/TS)',
    icon: Code,
    color: 'bg-yellow-500',
    repo: 'apex-mediation-platform',
    repoPath: 'sdk/web',
    description: 'TypeScript SDK for web apps and PWAs.',
    features: ['Tree-shakeable', 'Zero dependencies', 'Server-side rendering safe'],
  },
];

const tools = [
  {
    name: 'CLI',
    description: 'Validate configs, dry-run migrations, and inspect auction traces from your terminal.',
    icon: Terminal,
    href: '/docs/developer-tools#cli',
  },
  {
    name: 'Test Console',
    description: 'Simulate auctions, test adapter integrations, and inspect bid responses in sandbox mode.',
    icon: Play,
    href: '/docs/developer-tools#test-console',
  },
  {
    name: 'Figma Kit',
    description: 'UI components for building custom dashboards and internal tools with our design system.',
    icon: Palette,
    href: '/docs/developer-tools#figma-kit',
  },
];

const codeExamples = {
  android: `// Android (Kotlin)
ApexMediation.initialize(
    context = this,
    appId = "your-app-id",
    config = ApexConfig.Builder()
        .enableTestMode(BuildConfig.DEBUG)
        .setLogLevel(LogLevel.DEBUG)
        .build()
)

// Load and show an interstitial
val interstitial = ApexInterstitial("zone-id")
interstitial.load()
interstitial.show(activity)`,

  ios: `// iOS (Swift)
await ApexMediation.shared.initialize(
    appId: "your-app-id",
    config: ApexConfig(
        testMode: ProcessInfo.isDebug,
        logLevel: .debug
    )
)

// Load and show an interstitial
let interstitial = ApexInterstitial(zoneId: "zone-id")
try await interstitial.load()
interstitial.show(from: self)`,

  unity: `// Unity (C#)
ApexMediation.Initialize(
    appId: "your-app-id",
    config: new ApexConfig {
        TestMode = Debug.isDebugBuild,
        LogLevel = ApexLogLevel.Debug
    }
);

// Load and show an interstitial
var interstitial = new ApexInterstitial("zone-id");
await interstitial.LoadAsync();
interstitial.Show();`,

  web: `// Web (TypeScript)
await ApexMediation.initialize({
  appId: 'your-app-id',
  config: {
    testMode: process.env.NODE_ENV === 'development',
    logLevel: 'debug',
  },
});

// Load and show a banner
const banner = new ApexBanner('zone-id');
await banner.load();
banner.show(document.getElementById('ad-container'));`,
};

export default function DevelopersPage() {
  return (
    <>
      <main className="relative bg-white text-slate-900">
        <SiteHeader />

        {/* Hero */}
        <PageHero variant="light">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto">
              <h1 className="text-4xl md:text-5xl font-bold leading-tight mb-6">
                SDKs built for ad teams who ship.
              </h1>
              <p className="text-xl leading-relaxed mb-8">
                Integrate once. Bring your own networks. Keep full control.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                <a
                  href="https://github.com/sbelakho2/apex-mediation-platform/tree/main/sdk"
                  className="btn-primary inline-flex items-center gap-2"
                >
                  <GitBranch className="w-5 h-5" />
                  View on GitHub
                </a>
                <a
                  href="/docs"
                  className="btn-secondary inline-flex items-center gap-2"
                >
                  <BookOpen className="w-5 h-5" />
                  View Documentation
                </a>
              </div>

              <div className="mt-8 mx-auto max-w-2xl rounded-xl border border-slate-200 bg-slate-50 px-5 py-4 text-left">
                <p className="text-sm text-slate-700 max-w-none">
                  <span className="font-semibold">Naming:</span> Bel Consulting OÜ is the legal entity. ApexMediation is the platform and SDK namespace.
                </p>
              </div>
            </motion.div>
          </div>
        </PageHero>

        {/* Quickstarts */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Quickstarts
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                Get up and running in minutes with platform-specific guides.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-2 gap-6 max-w-4xl mx-auto mb-12">
              <motion.a
                href="/docs/ssai/quickstart-web"
                {...getRevealProps(0.1)}
                className="bg-white rounded-xl p-6 border border-gray-200 hover:shadow-lg transition-shadow group"
              >
                <div className="flex items-center gap-3 mb-3">
                  <div className="w-10 h-10 bg-yellow-100 rounded-lg flex items-center justify-center">
                    <Code className="w-5 h-5 text-yellow-600" />
                  </div>
                  <h3 className="text-lg font-bold text-gray-900 group-hover:text-brand-600">Web Video SSAI Quickstart</h3>
                </div>
                <p className="text-gray-600 text-sm">
                  Integrate SSAI for HLS/DASH streams in your web player. Works with hls.js, dash.js, Video.js, and more.
                </p>
              </motion.a>

              <motion.a
                href="/docs/ssai/quickstart-ctv"
                {...getRevealProps(0.15)}
                className="bg-white rounded-xl p-6 border border-gray-200 hover:shadow-lg transition-shadow group"
              >
                <div className="flex items-center gap-3 mb-3">
                  <div className="w-10 h-10 bg-purple-100 rounded-lg flex items-center justify-center">
                    <Smartphone className="w-5 h-5 text-purple-600" />
                  </div>
                  <h3 className="text-lg font-bold text-gray-900 group-hover:text-brand-600">CTV SSAI Quickstart</h3>
                </div>
                <p className="text-gray-600 text-sm">
                  Set up server-side ad insertion for Apple TV, Fire TV, Android TV, Roku, and smart TVs.
                </p>
              </motion.a>
            </div>
          </div>
        </section>

        {/* SDKs */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                SDKs for every platform
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                All SDKs are fully open-source, consistently designed, and documented.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-2 gap-8 max-w-5xl mx-auto">
              {sdks.map((sdk, index) => (
                <motion.div
                  key={sdk.platform}
                  {...getRevealProps(index * 0.1)}
                  className="bg-gray-50 rounded-xl p-6 hover:shadow-lg transition-shadow"
                >
                  <div className="flex items-start gap-4 mb-4">
                    <div className={`${sdk.color} p-3 rounded-lg`}>
                      <sdk.icon className="w-6 h-6 text-white" />
                    </div>
                    <div>
                      <h3 className="text-xl font-bold text-gray-900">{sdk.platform}</h3>
                      <a
                        href={`https://github.com/sbelakho2/${sdk.repo}/tree/main/${sdk.repoPath}`}
                        className="text-sm text-brand-600 hover:underline"
                      >
                        {sdk.repoPath}
                      </a>
                    </div>
                  </div>
                  <p className="text-gray-600 mb-4">{sdk.description}</p>
                  <ul className="space-y-2">
                    {sdk.features.map((feature) => (
                      <li key={feature} className="flex items-center gap-2 text-sm text-gray-700">
                        <ShieldCheck className="w-4 h-4 text-brand-600" />
                        {feature}
                      </li>
                    ))}
                  </ul>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Quick Start */}
        <section className="py-16 bg-gray-900 text-white">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold mb-4">
                Quick start
              </h2>
              <p className="text-lg text-gray-400 max-w-prose mx-auto">
                Get up and running in minutes with platform-specific examples.
              </p>
            </motion.div>

            <motion.div {...getRevealProps(0.1)} className="max-w-4xl mx-auto">
              <div className="grid md:grid-cols-2 gap-6">
                {Object.entries(codeExamples).map(([platform, code]) => (
                  <div key={platform} className="bg-gray-800 rounded-xl p-4 overflow-hidden">
                    <div className="flex items-center gap-2 mb-3 pb-3 border-b border-gray-700">
                      <div className="flex gap-1.5">
                        <div className="w-3 h-3 rounded-full bg-red-500" />
                        <div className="w-3 h-3 rounded-full bg-yellow-500" />
                        <div className="w-3 h-3 rounded-full bg-green-500" />
                      </div>
                      <span className="text-xs text-gray-500 uppercase ml-2">{platform}</span>
                    </div>
                    <pre className="text-xs text-gray-300 overflow-x-auto">
                      <code>{code}</code>
                    </pre>
                  </div>
                ))}
              </div>
            </motion.div>
          </div>
        </section>

        {/* Developer Tools */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Developer tools
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                Everything you need to build, test, and debug your integration.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-3 gap-8 max-w-5xl mx-auto">
              {tools.map((tool, index) => (
                <motion.a
                  key={tool.name}
                  {...getRevealProps(index * 0.1)}
                  href={tool.href}
                  className="text-center group"
                >
                  <div className="w-16 h-16 bg-brand-100 rounded-2xl flex items-center justify-center mx-auto mb-4">
                    <tool.icon className="w-8 h-8 text-brand-600" />
                  </div>
                  <h3 className="text-xl font-bold text-gray-900 mb-2 group-hover:text-brand-600">{tool.name}</h3>
                  <p className="text-gray-600">{tool.description}</p>
                </motion.a>
              ))}
            </div>
          </div>
        </section>

        {/* Certified Playback Matrix */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Certified playback matrix
              </h2>
              <p className="text-lg text-gray-600 max-w-prose mx-auto">
                20+ platforms and players tested and documented. Know exactly what works before you ship.
              </p>
            </motion.div>

            <motion.div {...getRevealProps(0.1)} className="max-w-4xl mx-auto">
              <div className="bg-white rounded-xl p-6 border border-gray-200">
                <div className="grid md:grid-cols-3 gap-6 text-center">
                  <div>
                    <div className="text-3xl font-bold text-brand-600 mb-2">6</div>
                    <div className="text-gray-600 font-medium">SDK platforms</div>
                    <div className="text-sm text-gray-500">iOS, tvOS, Android, Android TV, Unity, Web</div>
                  </div>
                  <div>
                    <div className="text-3xl font-bold text-brand-600 mb-2">8+</div>
                    <div className="text-gray-600 font-medium">Video players</div>
                    <div className="text-sm text-gray-500">hls.js, dash.js, ExoPlayer, AVPlayer, Video.js</div>
                  </div>
                  <div>
                    <div className="text-3xl font-bold text-brand-600 mb-2">4</div>
                    <div className="text-gray-600 font-medium">Stream formats</div>
                    <div className="text-sm text-gray-500">HLS, DASH, TS, CMAF</div>
                  </div>
                </div>
                <div className="mt-6 text-center">
                  <a
                    href="/docs/playback-matrix"
                    className="inline-flex items-center gap-2 text-brand-600 font-semibold hover:underline"
                  >
                    <BookOpen className="w-4 h-4" />
                    View full playback matrix →
                  </a>
                </div>
              </div>
            </motion.div>
          </div>
        </section>

        {/* CTA */}
        <section className="py-16 bg-brand-600">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <h2 className="text-3xl md:text-4xl font-bold text-white mb-6">
                Ready to integrate?
              </h2>
              <p className="text-xl text-brand-100 mb-8">
                Start with our GitHub repo or reach out if you need help.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                <a
                  href="https://github.com/sbelakho2/apex-mediation-platform"
                  className="inline-flex items-center gap-2 bg-white text-brand-600 px-6 py-3 rounded-lg font-semibold hover:bg-brand-50 transition-colors"
                >
                  View on GitHub
                </a>
                <a
                  href="/contact"
                  className="inline-flex items-center gap-2 border-2 border-white text-white px-6 py-3 rounded-lg font-semibold hover:bg-white/10 transition-colors"
                >
                  Talk to an engineer
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
