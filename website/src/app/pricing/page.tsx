'use client';

import { motion } from 'framer-motion';
import {
  CheckCircle,
  Coins,
  Gauge,
  HelpCircle,
  Layers,
  ShieldCheck,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

const pricingTiers = [
  {
    name: 'Starter',
    price: 'Free',
    badge: 'Free tier',
    icon: Coins,
    description: 'For teams integrating and testing.',
    features: [
      'Up to 5M impressions / month',
      'Up to 200K API calls / month',
      'Up to 100 GB data transfer / month',
      'BYO networks only',
      'Email support',
      'Full SDKs & console access',
      'Cryptographic auction logs (30-day retention)',
    ],
    cta: 'Get started free',
    ctaHref: '/signup',
    accent: true,
  },
  {
    name: 'Growth',
    price: '2.5% platform fee',
    badge: 'Most popular',
    icon: Gauge,
    description: 'For teams monetizing at scale with multiple networks.',
    features: [
      'Everything in Starter, plus:',
      'Higher usage limits (custom per contract)',
      '90-day log retention',
      'Role-based access control',
      'Priority support (Slack/Discord channel)',
    ],
    cta: 'Talk to sales',
    ctaHref: '/contact',
    accent: false,
  },
  {
    name: 'Scale',
    price: '2.0% platform fee',
    badge: '1.25% with annual prepay',
    icon: Layers,
    description: 'For mid-sized publishers with serious UA & BI needs.',
    features: [
      'Everything in Growth, plus:',
      'BI exports & pre-built warehouse schemas',
      'Dedicated integration help',
      'Custom SLOs and escalation paths',
      'Security & compliance review support',
    ],
    cta: 'Book a consult',
    ctaHref: '/contact',
    accent: false,
  },
  {
    name: 'Enterprise',
    price: 'Custom',
    badge: '1.0–1.5% + minimum',
    icon: ShieldCheck,
    description: 'For large publishers with strict compliance or hybrid stacks.',
    features: [
      'Custom SLAs, regions, and data residency',
      'Dedicated support engineer',
      'Custom features and roadmap input',
      'All Scale features plus contractual SLAs',
      'Dedicated Slack channel + quarterly reviews',
    ],
    cta: 'Contact us',
    ctaHref: '/contact',
    accent: false,
  },
];

const faqs = [
  {
    question: 'Do you take a cut of what networks pay me?',
    answer: 'No. Networks pay you directly. Our fee is for platform usage only.',
  },
  {
    question: 'What happens if I exceed my included usage?',
    answer: 'We\'ll notify you early with clear overage pricing and give you time to adjust.',
  },
  {
    question: 'Can I leave and take my data?',
    answer: 'Yes. Your data exports and logs are yours to keep.',
  },
];

export default function PricingPage() {
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
                Transparent pricing for a transparent mediation layer.
              </h1>
              <p className="text-xl md:text-2xl text-gray-600 max-w-3xl mx-auto">
                Networks pay you directly. We charge a predictable platform fee based on your usage.
              </p>
            </motion.div>
          </div>
        </section>

        {/* Pricing Tiers */}
        <section className="py-20 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="grid md:grid-cols-2 lg:grid-cols-4 gap-8 max-w-7xl mx-auto">
              {pricingTiers.map((tier, index) => (
                <motion.div
                  key={tier.name}
                  {...getRevealProps(index * 0.1)}
                  className={`rounded-xl p-6 ${
                    tier.accent
                      ? 'bg-brand-600 text-white'
                      : 'bg-white border border-gray-200'
                  }`}
                >
                  <div className="flex items-center justify-between mb-4">
                    <tier.icon className={`w-8 h-8 ${tier.accent ? 'text-white' : 'text-brand-600'}`} />
                    {tier.badge && (
                      <span className={`text-xs font-semibold px-2 py-1 rounded ${
                        tier.accent ? 'bg-white text-brand-600' : 'bg-brand-100 text-brand-600'
                      }`}>
                        {tier.badge}
                      </span>
                    )}
                  </div>
                  <h3 className={`text-2xl font-bold mb-2 ${tier.accent ? 'text-white' : 'text-gray-900'}`}>
                    {tier.name}
                  </h3>
                  <p className={`text-lg font-semibold mb-4 ${tier.accent ? 'text-brand-100' : 'text-brand-600'}`}>
                    {tier.price}
                  </p>
                  <p className={`text-sm mb-6 ${tier.accent ? 'text-brand-100' : 'text-gray-600'}`}>
                    {tier.description}
                  </p>
                  <ul className="space-y-3 mb-8">
                    {tier.features.map((feature, i) => (
                      <li key={i} className="flex items-start gap-2 text-sm">
                        <CheckCircle className={`w-4 h-4 flex-shrink-0 mt-0.5 ${
                          tier.accent ? 'text-brand-200' : 'text-brand-600'
                        }`} />
                        <span className={tier.accent ? 'text-brand-100' : 'text-gray-700'}>
                          {feature}
                        </span>
                      </li>
                    ))}
                  </ul>
                  <a
                    href={tier.ctaHref}
                    className={`block w-full text-center py-3 rounded-lg font-semibold transition-colors ${
                      tier.accent
                        ? 'bg-white text-brand-600 hover:bg-brand-50'
                        : 'bg-brand-600 text-white hover:bg-brand-700'
                    }`}
                  >
                    {tier.cta}
                  </a>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* FAQ */}
        <section className="py-20">
          <div className="container mx-auto px-4">
            <div className="max-w-3xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900">
                  Frequently asked questions
                </h2>
              </motion.div>
              <div className="space-y-6">
                {faqs.map((faq, index) => (
                  <motion.div
                    key={faq.question}
                    {...getRevealProps(index * 0.1)}
                    className="bg-gray-50 rounded-xl p-6"
                  >
                    <div className="flex items-start gap-3">
                      <HelpCircle className="w-6 h-6 text-brand-600 flex-shrink-0 mt-0.5" />
                      <div>
                        <h3 className="text-lg font-semibold text-gray-900 mb-2">
                          {faq.question}
                        </h3>
                        <p className="text-gray-600">
                          {faq.answer}
                        </p>
                      </div>
                    </div>
                  </motion.div>
                ))}
              </div>
            </div>
          </div>
        </section>
      </main>

      <Footer />
      <CookieBanner />
    </>
  );
}
