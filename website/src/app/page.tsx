'use client';

import { motion } from 'framer-motion';
import {
  Box,
  CheckCircle,
  Download,
  FileText,
  GitBranch,
  Key,
  Lock,
  Search,
  ShieldCheck,
  Target,
  Users2,
  Zap,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import { MediationFlowCard } from '@/components/VisualCard';
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';
import { sloConfig } from '@/config/legal';

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
    title: 'Keep your ad network relationships.',
    description:
      'Your existing contracts, accounts, and payouts stay exactly as they are. ApexMediation optimizes how your networks compete—you keep 100% of what they pay you.',
  },
  {
    icon: Lock,
    title: 'See exactly what happened.',
    description:
      'Every auction decision is logged and verifiable. When you need to reconcile with networks or understand a revenue change, you have the data—not just a dashboard.',
  },
  {
    icon: Zap,
    title: 'Reliable SDKs that just work.',
    description:
      'Built for performance: background-only network calls, safe config updates, and automatic recovery from adapter issues. Supports Unity, Android, iOS, tvOS, and Android TV.',
  },
  {
    icon: GitBranch,
    title: 'Migrate without disruption.',
    description:
      'Migration Studio imports your existing setup, lets you test before switching, and enables per-placement rollout with instant rollback if needed.',
  },
];

const howItWorksSteps = [
  {
    step: 1,
    title: 'Connect your ad networks',
    description:
      'Link your existing AdMob, AppLovin, Unity, Meta, and other ad network accounts. Your contracts and payouts stay exactly as they are.',
  },
  {
    step: 2,
    title: 'Add the SDK',
    description:
      'Install the ApexMediation SDK for your platform (Android, iOS, Unity, tvOS, or Web). It\'s lightweight, background-threaded, and includes a debug panel.',
  },
  {
    step: 3,
    title: 'Watch revenue grow',
    description:
      'Our ML optimizes your waterfall automatically. Monitor performance in real-time, run A/B tests, and export detailed analytics to your data tools.',
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
    title: 'Growing studios',
    description:
      'You\'re running multiple ad networks and want to maximize revenue without the complexity. ApexMediation gives you one dashboard, smart optimization, and zero disruption to your existing setup.',
  },
  {
    icon: Search,
    title: 'Data-driven teams',
    description:
      'You need real analytics, not just dashboards. Get log-level exports, A/B testing, and verifiable data you can trust for reconciliation and BI.',
  },
  {
    icon: Key,
    title: 'Control-conscious publishers',
    description:
      'You want the benefits of mediation without vendor lock-in. Keep your ad network relationships, export your data anytime, and switch providers if you ever need to.',
  },
];

// ============================================================================
// COMPONENTS
// ============================================================================

function HeroSection() {
  return (
    <PageHero variant="dark">
      <div className="grid lg:grid-cols-2 gap-8 lg:gap-12 items-center">
        {/* Left: Copy */}
        <motion.div {...getRevealProps(0)} className="max-w-3xl">
          <p className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-brand-600/20 text-brand-300 text-sm font-medium mb-6">
            ✓ Free to start · No credit card required
          </p>
          <h1 className="text-4xl md:text-5xl font-bold leading-tight text-white mb-6">
            Ad mediation that maximizes your revenue—transparently.
          </h1>
          <p className="text-xl leading-relaxed text-slate-200/90 mb-6 max-w-3xl">
            ApexMediation optimizes every ad request across your networks with ML-powered waterfalls, real-time analytics, and verifiable auction logs.
          </p>
          <div className="flex flex-col sm:flex-row items-start gap-4 mb-4">
            <a
              href="/signup"
              className="bg-brand-600 hover:bg-brand-700 text-white px-8 py-4 text-lg font-semibold rounded-lg transition-colors"
            >
              Get started free
            </a>
            <a
              href="/contact"
              className="bg-white/10 hover:bg-white/15 text-white border border-white/20 px-8 py-4 text-lg font-semibold rounded-lg transition-colors"
            >
              Talk with our team
            </a>
          </div>
          <p className="text-sm text-slate-300/80">
            Built for mobile and CTV publishers who demand auditability and control.
          </p>
          <p className="text-sm text-slate-300/80 mt-2">
            <a href="/security-pack" className="text-brand-300 hover:text-brand-200 underline">
              Download the Security Pack →
            </a>
          </p>
        </motion.div>
        {/* Right: Visual Card */}
        <motion.div {...getRevealProps(0.1)} className="hidden lg:block">
          <MediationFlowCard className="ml-auto max-w-md" />
        </motion.div>
      </div>
    </PageHero>
  );
}

function PillarsSection() {
  return (
    <section className="py-16 bg-gray-50">
      <div className="container mx-auto px-4">
        <motion.div {...getRevealProps(0)} className="text-center mb-12">
          <p className="text-sm font-semibold text-brand-600 uppercase tracking-wide mb-2">
            Why publishers choose ApexMediation
          </p>
          <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
            Powerful mediation, full transparency
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

function SSAICoverageSection() {
  return (
    <section className="py-16 bg-white">
      <div className="container mx-auto px-4">
        <motion.div {...getRevealProps(0)} className="text-center mb-12">
          <p className="text-sm font-semibold text-brand-600 uppercase tracking-wide mb-2">
            Web + CTV SSAI (Live & VOD)
          </p>
          <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
            Supported today
          </h2>
          <p className="text-lg text-gray-600 max-w-2xl mx-auto">
            Full SSAI coverage for HLS and DASH across live and VOD streams.
          </p>
        </motion.div>

        <motion.div {...getRevealProps(0.1)} className="max-w-3xl mx-auto">
          <div className="bg-gray-900 rounded-xl p-6 font-mono text-sm">
            <div className="text-gray-400 mb-4 text-center font-semibold">SSAI Coverage Matrix</div>
            <div className="overflow-x-auto">
              <pre className="text-gray-300">
{`       Live  VOD   TS    CMAF
HLS    ✅    ✅    ✅    ✅
DASH   ✅    ✅    ✅    ✅
DRM    ✅ Clear  (Policy-based for DRM streams)`}
              </pre>
            </div>
          </div>
        </motion.div>
      </div>
    </section>
  );
}

function ProofStripSection() {
  const proofItems = [
    {
      icon: Target,
      title: 'Public SLO Targets',
      description: `${sloConfig.availability.target} availability, ${sloConfig.latency.p50} latency`,
      href: '/slo',
    },
    {
      icon: Key,
      title: 'Signed Logs',
      description: 'Cryptographically verifiable auction decisions',
      href: '/docs/verification',
    },
    {
      icon: Download,
      title: 'Security Pack',
      description: 'Architecture, threat model, compliance docs',
      href: '/security-pack',
    },
    {
      icon: FileText,
      title: 'Playback Matrix',
      description: '20+ certified platforms and players',
      href: '/docs/playback-matrix',
    },
  ];

  return (
    <section className="py-12 bg-gray-900">
      <div className="container mx-auto px-4">
        <div className="text-center mb-8">
          <p className="text-sm font-medium text-gray-400 uppercase tracking-wide">
            Verify before you buy
          </p>
        </div>
        <div className="grid md:grid-cols-4 gap-6 max-w-5xl mx-auto">
          {proofItems.map((item, index) => (
            <motion.a
              key={item.title}
              href={item.href}
              {...getRevealProps(index * 0.1)}
              className="bg-gray-800 rounded-xl p-5 text-center hover:bg-gray-700 transition-colors group"
            >
              <item.icon className="w-8 h-8 text-brand-400 mx-auto mb-3 group-hover:text-brand-300" />
              <h3 className="font-semibold text-white mb-1">{item.title}</h3>
              <p className="text-xs text-gray-400">{item.description}</p>
            </motion.a>
          ))}
        </div>
      </div>
    </section>
  );
}

function DesignPartnerSection() {
  const partnerBenefits = [
    {
      icon: Users2,
      title: 'White-glove onboarding',
      description: 'Personal integration support from our engineering team. We\'ll help you set up, migrate, and optimize.',
    },
    {
      icon: GitBranch,
      title: 'Migration Studio access',
      description: 'Test your existing waterfall against Apex side-by-side before switching any live traffic.',
    },
    {
      icon: ShieldCheck,
      title: 'Signed log verification',
      description: 'Every auction is cryptographically signed. Verify any decision independently—no trust required.',
    },
  ];

  return (
    <section className="py-16">
      <div className="container mx-auto px-4">
        <motion.div {...getRevealProps(0)} className="text-center mb-12">
          <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
            Built with design partners, for real problems.
          </h2>
          <p className="text-lg text-gray-600 max-w-prose mx-auto">
            We&apos;re working closely with a small group of studios across mobile games, utilities, and CTV apps. 
            Together we&apos;re solving the issues they actually hit in production: SDK crashes, opaque revenue swings, 
            network disputes, and painful migrations.
          </p>
        </motion.div>
        <div className="grid md:grid-cols-3 gap-8 max-w-5xl mx-auto">
          {partnerBenefits.map((benefit, index) => (
            <motion.div
              key={benefit.title}
              {...getRevealProps(index * 0.1)}
              className="bg-gray-50 rounded-xl p-6 border border-gray-100"
            >
              <div className="w-12 h-12 rounded-lg bg-brand-100 flex items-center justify-center mb-4">
                <benefit.icon className="w-6 h-6 text-brand-600" />
              </div>
              <h3 className="text-lg font-semibold text-gray-900 mb-2">
                {benefit.title}
              </h3>
              <p className="text-gray-600 text-sm leading-relaxed">
                {benefit.description}
              </p>
            </motion.div>
          ))}
        </div>

        <div className="mt-10 max-w-3xl mx-auto bg-white rounded-xl p-6 border border-gray-100 shadow-sm">
          <h3 className="text-xl font-semibold text-gray-900 mb-3">Design Partner Program</h3>
          <ul className="space-y-2 text-gray-700">
            <li className="flex items-start gap-2">
              <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
              <span>White-glove onboarding and integration support</span>
            </li>
            <li className="flex items-start gap-2">
              <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
              <span>Side-by-side migration testing with instant rollback</span>
            </li>
            <li className="flex items-start gap-2">
              <CheckCircle className="w-5 h-5 text-brand-600 flex-shrink-0 mt-0.5" />
              <span>Signed log verification for every auction decision</span>
            </li>
          </ul>
          <div className="mt-5">
            <a href="/contact" className="btn-primary px-6 py-3 text-base font-semibold">
              Apply to the Design Partner Program
            </a>
          </div>
        </div>
      </div>
    </section>
  );
}

function HowItWorksSection() {
  return (
    <section className="py-16 bg-gray-50">
      <div className="container mx-auto px-4">
        <motion.div {...getRevealProps(0)} className="text-center mb-12">
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
    <section className="py-16">
      <div className="container mx-auto px-4">
        <div className="max-w-prose mx-auto">
          <motion.div {...getRevealProps(0)}>
            <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
              Avoid the next &quot;over-the-air SDK crash&quot; headline.
            </h2>
            <p className="text-lg text-gray-600 mb-8">
              Ad SDKs are a common cause of ANRs, crashes and mysterious revenue drops. ApexMediation is built by engineers with deep experience in high-reliability systems and battery-sensitive mobile apps.
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
    <section className="py-16 bg-gray-50">
      <div className="container mx-auto px-4">
        <motion.div {...getRevealProps(0)} className="text-center mb-12">
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
    <section className="py-16">
      <div className="container mx-auto px-4 text-center">
        <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
          <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
            Simple, transparent pricing.
          </h2>
          <p className="text-lg text-gray-600 mb-8">
            Start free with up to $10k/month in mediated revenue per app. As you grow, pay a small platform fee—your ad network payouts stay 100% yours.
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
    <section className="py-16 bg-brand-600">
      <div className="container mx-auto px-4 text-center">
        <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
          <h2 className="text-3xl md:text-4xl font-bold text-white mb-4">
            Ready to maximize your ad revenue?
          </h2>
          <p className="text-lg text-brand-100 mb-8">
            Evaluate ApexMediation in 30 minutes. Signed logs. Public SLOs. No lock-in.
          </p>
          <div className="flex flex-col sm:flex-row items-center justify-center gap-4">
            <a
              href="/signup"
              className="bg-white text-brand-600 px-8 py-4 rounded-lg text-lg font-semibold hover:bg-brand-50 transition-colors"
            >
              Get started free
            </a>
            <a
              href="/contact"
              className="border-2 border-white text-white px-8 py-4 rounded-lg text-lg font-semibold hover:bg-brand-500 transition-colors"
            >
              Schedule a demo
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
      <main className="relative bg-white text-slate-900">
        <SiteHeader />
        
        <HeroSection />
        <PillarsSection />
        <SSAICoverageSection />
        <ProofStripSection />
        <DesignPartnerSection />
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
