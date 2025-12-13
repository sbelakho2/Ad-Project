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
    badge: 'No credit card',
    icon: Coins,
    description: 'Perfect for getting started and testing.',
    features: [
      'Up to $10k/month mediated revenue per app',
      'All SDKs & dashboard access',
      'Real-time analytics',
      'Email support',
      '30-day log retention',
    ],
    cta: 'Get started free',
    ctaHref: '/signup',
    accent: true,
  },
  {
    name: 'Growth',
    price: '2.5%',
    badge: 'Most popular',
    icon: Gauge,
    description: 'For teams ready to scale their ad revenue.',
    features: [
      'Everything in Starter',
      'Unlimited mediated revenue',
      'A/B testing & experiments',
      '90-day log retention',
      'Priority Slack support',
    ],
    cta: 'Talk to sales',
    ctaHref: '/contact',
    accent: false,
  },
  {
    name: 'Scale',
    price: '2.0%',
    badge: '1.25% with annual',
    icon: Layers,
    description: 'For publishers with advanced analytics needs.',
    features: [
      'Everything in Growth',
      'Data warehouse exports',
      'Dedicated integration help',
      'Custom SLOs',
      'Compliance review support',
    ],
    cta: 'Book a consult',
    ctaHref: '/contact',
    accent: false,
  },
  {
    name: 'Enterprise',
    price: 'Custom',
    badge: 'Volume pricing',
    icon: ShieldCheck,
    description: 'For large publishers with specific requirements.',
    features: [
      'Everything in Scale',
      'Custom SLAs & data residency',
      'Dedicated support engineer',
      'Roadmap input',
      'Quarterly business reviews',
    ],
    cta: 'Contact us',
    ctaHref: '/contact',
    accent: false,
  },
];

const faqs = [
  {
    question: 'Do you take a cut of my ad revenue?',
    answer: 'No. Your ad networks pay you directly—100% of your ad revenue is yours. We charge a simple platform fee for the mediation layer.',
  },
  {
    question: 'What happens if I exceed the Starter limit?',
    answer: 'We\'ll let you know before you hit the limit and discuss upgrade options. No surprise charges.',
  },
  {
    question: 'Can I export my data and leave?',
    answer: 'Absolutely. Your data is yours. Export logs, analytics, and configurations anytime.',
  },
  {
    question: 'Do I need to change my ad network contracts?',
    answer: 'No. Your existing ad network relationships stay exactly as they are. We work alongside them.',
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
