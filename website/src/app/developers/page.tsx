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
import HomeNav from '@/components/HomeNav';

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
    repo: 'apex-mediation-android-sdk',
    description: 'Kotlin-first SDK for Android apps targeting API 21+.',
    features: ['Compose & View support', 'Kotlin coroutines', 'ProGuard rules included'],
  },
  {
    platform: 'iOS',
    icon: Smartphone,
    color: 'bg-blue-500',
    repo: 'apex-mediation-ios-sdk',
    description: 'Swift SDK for iOS 14+ with full SwiftUI support.',
    features: ['Swift async/await', 'SwiftUI views', 'Privacy manifests included'],
  },
  {
    platform: 'Unity',
    icon: Puzzle,
    color: 'bg-purple-500',
    repo: 'apex-mediation-unity-plugin',
    description: 'C# plugin for Unity 2021+ with native bridge.',
    features: ['IL2CPP support', 'Editor testing', 'Auto-dependency resolution'],
  },
  {
    platform: 'Web (JS/TS)',
    icon: Code,
    color: 'bg-yellow-500',
    repo: 'apex-mediation-web-sdk',
    description: 'TypeScript SDK for web apps and PWAs.',
    features: ['Tree-shakeable', 'Zero dependencies', 'Server-side rendering safe'],
  },
];

const tools = [
  {
    name: 'CLI',
    description: 'Validate configs, dry-run migrations, and inspect auction traces from your terminal.',
    icon: Terminal,
  },
  {
    name: 'Test Console',
    description: 'Simulate auctions, test adapter integrations, and inspect bid responses in sandbox mode.',
    icon: Play,
  },
  {
    name: 'Figma Kit',
    description: 'UI components for building custom dashboards and internal tools with our design system.',
    icon: Palette,
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
      <main className="relative bg-white text-gray-900">
        <div className="container relative z-10 py-6 md:py-10">
          <HomeNav />
        </div>

        {/* Hero */}
        <section className="py-20 md:py-32 bg-gradient-to-b from-gray-50 to-white">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-4xl mx-auto">
              <h1 className="text-4xl md:text-6xl font-bold text-gray-900 leading-tight mb-6">
                SDKs built for ad teams who ship.
              </h1>
              <p className="text-xl md:text-2xl text-gray-600 max-w-3xl mx-auto mb-8">
                Integrate once. Bring your own networks. Keep full control.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                <a
                  href="https://docs.apexmediation.ee"
                  className="inline-flex items-center gap-2 bg-brand-600 text-white px-6 py-3 rounded-lg font-semibold hover:bg-brand-700 transition-colors"
                >
                  <BookOpen className="w-5 h-5" />
                  Read the docs
                </a>
                <a
                  href="https://github.com/sbelakho2/Ad-Project"
                  className="inline-flex items-center gap-2 bg-gray-900 text-white px-6 py-3 rounded-lg font-semibold hover:bg-gray-800 transition-colors"
                >
                  <GitBranch className="w-5 h-5" />
                  View on GitHub
                </a>
              </div>
            </motion.div>
          </div>
        </section>

        {/* SDKs */}
        <section className="py-20">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-16">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                SDKs for every platform
              </h2>
              <p className="text-lg text-gray-600 max-w-2xl mx-auto">
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
                        href={`https://github.com/bel-consulting/${sdk.repo}`}
                        className="text-sm text-brand-600 hover:underline"
                      >
                        bel-consulting/{sdk.repo}
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
        <section className="py-20 bg-gray-900 text-white">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-12">
              <h2 className="text-3xl md:text-4xl font-bold mb-4">
                Quick start
              </h2>
              <p className="text-lg text-gray-400 max-w-2xl mx-auto">
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
        <section className="py-20">
          <div className="container mx-auto px-4">
            <motion.div {...getRevealProps(0)} className="text-center mb-16">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                Developer tools
              </h2>
              <p className="text-lg text-gray-600 max-w-2xl mx-auto">
                Everything you need to build, test, and debug your integration.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-3 gap-8 max-w-5xl mx-auto">
              {tools.map((tool, index) => (
                <motion.div
                  key={tool.name}
                  {...getRevealProps(index * 0.1)}
                  className="text-center"
                >
                  <div className="w-16 h-16 bg-brand-100 rounded-2xl flex items-center justify-center mx-auto mb-4">
                    <tool.icon className="w-8 h-8 text-brand-600" />
                  </div>
                  <h3 className="text-xl font-bold text-gray-900 mb-2">{tool.name}</h3>
                  <p className="text-gray-600">{tool.description}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* CTA */}
        <section className="py-20 bg-brand-600">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto">
              <h2 className="text-3xl md:text-4xl font-bold text-white mb-6">
                Ready to integrate?
              </h2>
              <p className="text-xl text-brand-100 mb-8">
                Start with our getting started guide or reach out if you need help.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                <a
                  href="https://docs.apexmediation.ee/quickstart"
                  className="inline-flex items-center gap-2 bg-white text-brand-600 px-6 py-3 rounded-lg font-semibold hover:bg-brand-50 transition-colors"
                >
                  Getting started guide
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
