'use client';

import { motion } from 'framer-motion';
import {
  Box,
  GitBranch,
  Key,
  Lock,
  Search,
  ShieldCheck,
  Users2,
  Zap,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';

// Animation helpers - use animate for reliable rendering
const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

// ============================================================================
// DATA
// ============================================================================

const pillars = [
  {
    icon: Box,
    title: 'Keep your deals. We handle the plumbing.',
    description:
      'You stay in control of every ad network account and every contract. ApexMediation sits on top as a neutral BYO orchestration layer—no revenue share with networks, no surprise fees, no hidden optimizations.',
  },
  {
    icon: Lock,
    title: 'Every auction, cryptographically signed.',
    description:
      'Auction decisions are logged with tamper-evident hashes, so you can verify that what ran in production matches what we report. No more "just trust the dashboard".',
  },
  {
    icon: Zap,
    title: 'SDKs built to stay out of your way.',
    description:
      'All network I/O on background threads, strict main-thread guards, OTA-safe config rollouts, and kill-switches for misbehaving adapters. Designed for IL2CPP/Mono, Android, iOS, tvOS and Android TV.',
  },
  {
    icon: GitBranch,
    title: 'Switch without breaking your game.',
    description:
      'Our Migration Studio imports your existing mediation and placement setup, simulates waterfalls, and lets you cut over one placement at a time—with full rollback available.',
  },
];

const howItWorksSteps = [
  {
    step: 1,
    title: 'Connect your networks (BYO)',
    description:
      'You sign in to your existing ad network dashboards (AdMob, MAX, LevelPlay, Pangle, etc.) and add their credentials directly to your console. We never resell your demand; we only orchestrate.',
  },
  {
    step: 2,
    title: 'Drop in the SDKs',
    description:
      "Install the ApexMediation SDKs for Android, iOS, Unity, tvOS, Android TV or Web. SDKs are small, background-threaded, and ship with a Debug Panel so you can see exactly what they're doing.",
  },
  {
    step: 3,
    title: 'Observe, optimize, and verify',
    description:
      'Use the console to inspect waterfalls, see per-adapter latency & fill, run experiments, and export log-level auctions. Every auction has a cryptographic fingerprint, so you can reconcile with your BI stack.',
  },
];

const reliabilityBullets = [
  'OTA-safe config rollouts with staged traffic and automatic rollback',
  'Per-adapter circuit breakers and timeouts, so one slow network can\'t stall your app',
  'Kill-switches for adapters and placements you can flip instantly',
  'Debuggable waterfalls and on-device Debug Panel for fast issue reproduction',
];

const audienceCards = [
  {
    icon: Users2,
    title: 'Mid-sized studios',
    description:
      'You already run multiple ad networks and have real revenue to protect. ApexMediation gives you deeper visibility and a more robust core, without forcing you to re-negotiate your deals.',
  },
  {
    icon: Search,
    title: 'UA-savvy teams',
    description:
      'You care about incrementality, A/B tests, and log-level exports. ApexMediation gives you clean, reconciled data and cryptographic guarantees that your waterfalls match what you see in your BI tools.',
  },
  {
    icon: Key,
    title: 'Infra-conscious founders',
    description:
      'You want to avoid lock-in and opaque "one vendor for everything" setups. BYO mediation lets you keep the benefits of multiple networks, but with the control and transparency of building it yourself.',
  },
];

// ============================================================================
// COMPONENTS
// ============================================================================

function HeroSection() {
  return (
    <section className="relative py-20 md:py-32">
      <div className="container mx-auto px-4 text-center">
        <motion.div {...getRevealProps(0)} className="max-w-4xl mx-auto">
          <h1 className="text-4xl md:text-6xl font-bold text-gray-900 leading-tight mb-6">
            BYO ad mediation with cryptographic transparency.
          </h1>
          <p className="text-xl md:text-2xl text-gray-600 mb-8 max-w-3xl mx-auto">
            ApexMediation lets you keep your existing ad network deals and SDKs—while we handle the hard parts: OTA-safe configs, ANR-minimal SDKs, and verifiable auction logs you can trust.
          </p>
          <div className="flex flex-col sm:flex-row items-center justify-center gap-4 mb-4">
            <a
              href="/signup"
              className="btn-primary px-8 py-4 text-lg font-semibold"
            >
              Start BYO trial
            </a>
            <a
              href="/contact"
              className="btn-secondary px-8 py-4 text-lg font-semibold"
            >
              Talk with an engineer
            </a>
          </div>
          <p className="text-sm text-gray-500">
            For mid-sized mobile and CTV publishers who are done guessing what their mediation layer is doing.
          </p>
        </motion.div>
      </div>
    </section>
  );
}

function PillarsSection() {
  return (
    <section className="py-20 bg-gray-50">
      <div className="container mx-auto px-4">
        <motion.div {...getRevealProps(0)} className="text-center mb-16">
          <p className="text-sm font-semibold text-brand-600 uppercase tracking-wide mb-2">
            Why publishers choose ApexMediation
          </p>
          <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
            BYO, not black box
          </h2>
        </motion.div>
        <div className="grid md:grid-cols-2 lg:grid-cols-4 gap-8">
          {pillars.map((pillar, index) => (
            <motion.div
              key={pillar.title}
              {...getRevealProps(index * 0.1)}
              className="bg-white rounded-xl p-6 shadow-sm border border-gray-100"
            >
              <div className="w-12 h-12 rounded-lg bg-brand-100 flex items-center justify-center mb-4">
                <pillar.icon className="w-6 h-6 text-brand-600" />
              </div>
              <h3 className="text-lg font-semibold text-gray-900 mb-2">
                {pillar.title}
              </h3>
              <p className="text-gray-600 text-sm leading-relaxed">
                {pillar.description}
              </p>
            </motion.div>
          ))}
        </div>
      </div>
    </section>
  );
}

function SocialProofSection() {
  return (
    <section className="py-20">
      <div className="container mx-auto px-4 text-center">
        <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto">
          <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-6">
            Built with real studios, for real problems.
          </h2>
          <p className="text-lg text-gray-600 mb-12">
            ApexMediation is shaped together with a small group of design-partner studios across mobile games, utilities and CTV apps. We prioritize the issues they actually hit in production: SDK crashes, opaque revenue swings, network disputes and painful migrations.
          </p>
          <div className="bg-gray-100 rounded-xl p-8 border-2 border-dashed border-gray-300">
            <img
              src="/api/brand/logo"
              alt="ApexMediation"
              className="h-12 w-auto mx-auto"
            />
            <p className="text-sm text-gray-400 mt-2">
              Use this area later for studio logos, quotes, case studies.
            </p>
          </div>
        </motion.div>
      </div>
    </section>
  );
}

function HowItWorksSection() {
  return (
    <section className="py-20 bg-gray-50">
      <div className="container mx-auto px-4">
        <motion.div {...getRevealProps(0)} className="text-center mb-16">
          <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
            How ApexMediation fits into your stack
          </h2>
        </motion.div>
        <div className="max-w-4xl mx-auto">
          {howItWorksSteps.map((item, index) => (
            <motion.div
              key={item.step}
              {...getRevealProps(index * 0.1)}
              className="flex gap-6 mb-8 last:mb-0"
            >
              <div className="flex-shrink-0 w-12 h-12 rounded-full bg-brand-600 text-white flex items-center justify-center text-xl font-bold">
                {item.step}
              </div>
              <div>
                <h3 className="text-xl font-semibold text-gray-900 mb-2">
                  {item.title}
                </h3>
                <p className="text-gray-600 leading-relaxed">
                  {item.description}
                </p>
              </div>
            </motion.div>
          ))}
        </div>
      </div>
    </section>
  );
}

function ReliabilitySection() {
  return (
    <section className="py-20">
      <div className="container mx-auto px-4">
        <div className="max-w-4xl mx-auto">
          <motion.div {...getRevealProps(0)}>
            <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
              Avoid the next &quot;over-the-air SDK crash&quot; headline.
            </h2>
            <p className="text-lg text-gray-600 mb-8">
              Ad SDKs are a common cause of ANRs, crashes and mysterious revenue drops. ApexMediation is built by engineers with deep experience in high-reliability systems and battery-sensitive mobile apps, and backed by decades of technology work at our parent group.
            </p>
            <ul className="space-y-4">
              {reliabilityBullets.map((bullet, index) => (
                <li key={index} className="flex items-start gap-3">
                  <ShieldCheck className="w-5 h-5 text-brand-600 flex-shrink-0 mt-1" />
                  <span className="text-gray-700">{bullet}</span>
                </li>
              ))}
            </ul>
          </motion.div>
        </div>
      </div>
    </section>
  );
}

function AudienceSection() {
  return (
    <section className="py-20 bg-gray-50">
      <div className="container mx-auto px-4">
        <motion.div {...getRevealProps(0)} className="text-center mb-16">
          <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
            Who gets the most value
          </h2>
        </motion.div>
        <div className="grid md:grid-cols-3 gap-8 max-w-5xl mx-auto">
          {audienceCards.map((card, index) => (
            <motion.div
              key={card.title}
              {...getRevealProps(index * 0.1)}
              className="bg-white rounded-xl p-6 shadow-sm border border-gray-100"
            >
              <div className="w-12 h-12 rounded-lg bg-brand-100 flex items-center justify-center mb-4">
                <card.icon className="w-6 h-6 text-brand-600" />
              </div>
              <h3 className="text-lg font-semibold text-gray-900 mb-2">
                {card.title}
              </h3>
              <p className="text-gray-600 text-sm leading-relaxed">
                {card.description}
              </p>
            </motion.div>
          ))}
        </div>
      </div>
    </section>
  );
}

function PricingTeaserSection() {
  return (
    <section className="py-20">
      <div className="container mx-auto px-4 text-center">
        <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto">
          <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
            Pay for platform value, not your ad revenue.
          </h2>
          <p className="text-lg text-gray-600 mb-8">
            ApexMediation&apos;s BYO model means networks pay you directly. We charge a transparent platform fee based on usage, with a generous free tier so you can integrate, test, and ship before committing.
          </p>
          <a href="/pricing" className="btn-primary px-8 py-4 text-lg font-semibold">
            View pricing
          </a>
        </motion.div>
      </div>
    </section>
  );
}

function FinalCTASection() {
  return (
    <section className="py-20 bg-brand-600">
      <div className="container mx-auto px-4 text-center">
        <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto">
          <h2 className="text-3xl md:text-4xl font-bold text-white mb-4">
            Ready to stop guessing what your mediation layer is doing?
          </h2>
          <p className="text-lg text-brand-100 mb-8">
            Join the early access cohort of studios using ApexMediation to make their ad stack verifiable, resilient and BYO-friendly.
          </p>
          <div className="flex flex-col sm:flex-row items-center justify-center gap-4">
            <a
              href="/signup"
              className="bg-white text-brand-600 px-8 py-4 rounded-lg text-lg font-semibold hover:bg-brand-50 transition-colors"
            >
              Start BYO trial
            </a>
            <a
              href="/contact"
              className="border-2 border-white text-white px-8 py-4 rounded-lg text-lg font-semibold hover:bg-brand-500 transition-colors"
            >
              Book a technical walkthrough
            </a>
          </div>
        </motion.div>
      </div>
    </section>
  );
}

// ============================================================================
// MAIN PAGE
// ============================================================================

export default function HomePage() {
  return (
    <>
      <main className="relative bg-white text-gray-900">
        <div className="container relative z-10 py-6 md:py-10">
          <HomeNav />
        </div>

        <HeroSection />
        <PillarsSection />
        <SocialProofSection />
        <HowItWorksSection />
        <ReliabilitySection />
        <AudienceSection />
        <PricingTeaserSection />
        <FinalCTASection />
      </main>

      <Footer />
      <CookieBanner />
    </>
  );
}
