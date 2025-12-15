'use client';

import { motion } from 'framer-motion';
import {
  Building2,
  Globe,
  Heart,
  Lock,
  Scale,
  Shield,
  Target,
  Users,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';
import { PrinciplesToProductCard } from '@/components/VisualCard';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

const values = [
  {
    title: 'Transparency',
    description: 'Every auction is verifiable. You never wonder if you\'re being treated fairly.',
    icon: Shield,
  },
  {
    title: 'Publisher control',
    description: 'You bring the networks, you set the rules. We don\'t gatekeep.',
    icon: Target,
  },
  {
    title: 'Privacy-first',
    description: 'No fingerprinting, no secret sharing. Your users\' data stays in your hands.',
    icon: Lock,
  },
  {
    title: 'Reliability',
    description: '99.9% uptime, 50ms p99 latency. We treat downtime as seriously as you do.',
    icon: Scale,
  },
];

const stats = [
  { label: 'Open-source SDKs', value: '6' },
  { label: 'Certified platforms', value: '20+' },
  { label: 'Signed verification', value: '✓' },
  { label: 'Design partner slots', value: '3' },
];

export default function CompanyPage() {
  return (
    <>
      <main className="relative bg-white text-gray-900">
        <div className="container relative z-10 py-6 md:py-10">
          <HomeNav />
        </div>

        {/* Hero */}
        <section className="py-16 md:py-20">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <h1 className="text-4xl md:text-5xl font-bold text-gray-900 leading-tight mb-6">
                Mediation shouldn&apos;t mean compromise.
              </h1>
              <p className="text-xl text-gray-600">
                Apex Mediation was founded by ads engineers who were tired of mediators that weren&apos;t built for publishers.
              </p>
            </motion.div>
          </div>
        </section>

        {/* Story */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-prose mx-auto">
              <motion.div {...getRevealProps(0)}>
                <div className="flex items-start gap-4 mb-8">
                  <div className="w-12 h-12 bg-brand-100 rounded-xl flex items-center justify-center flex-shrink-0">
                    <Heart className="w-6 h-6 text-brand-600" />
                  </div>
                  <div>
                    <h2 className="text-2xl md:text-3xl font-bold text-gray-900 mt-0 mb-4">
                      Our story
                    </h2>
                    <p className="text-gray-600 leading-relaxed mb-4">
                      We built Apex after years of watching legacy mediators erode trust with publishers. 
                      Hidden algorithms. Opaque fee structures. Networks that couldn&apos;t be removed. 
                      Integration tax that made switching feel like rebuilding from scratch.
                    </p>
                    <p className="text-gray-600 leading-relaxed mb-4">
                      We believed there had to be a better way—one where the publisher retains control 
                      of every network relationship, where auctions are auditable, and where the 
                      mediation layer is a tool, not a gatekeeper.
                    </p>
                    <p className="text-gray-600 leading-relaxed">
                      Today, Apex Mediation is being built alongside design-partner studios who demand 
                      transparency, reliability, and the flexibility to run their ad stack their way.
                    </p>
                  </div>
                </div>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Values */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid lg:grid-cols-2 gap-12 items-start">
                <motion.div {...getRevealProps(0)}>
                  <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                    What we believe
                  </h2>
                  <p className="text-lg text-gray-600 mb-8 max-w-prose">
                    Our values aren&apos;t just words on a wall—they&apos;re built into every line of code.
                  </p>
                  <div className="space-y-6">
                    {values.map((value, index) => (
                      <motion.div
                        key={value.title}
                        {...getRevealProps(index * 0.1)}
                        className="flex items-start gap-4"
                      >
                        <div className="w-12 h-12 bg-brand-100 rounded-xl flex items-center justify-center flex-shrink-0">
                          <value.icon className="w-6 h-6 text-brand-600" />
                        </div>
                        <div>
                          <h3 className="text-lg font-semibold text-gray-900 mb-1">{value.title}</h3>
                          <p className="text-gray-600 text-sm">{value.description}</p>
                        </div>
                      </motion.div>
                    ))}
                  </div>
                </motion.div>
                <motion.div {...getRevealProps(0.1)} className="hidden lg:block">
                  <PrinciplesToProductCard className="ml-auto max-w-sm" />
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Stats */}
        <section className="py-16 bg-brand-600">
          <div className="container mx-auto px-4">
            <div className="grid md:grid-cols-4 gap-8 max-w-5xl mx-auto">
              {stats.map((stat, index) => (
                <motion.div
                  key={stat.label}
                  {...getRevealProps(index * 0.1)}
                  className="text-center"
                >
                  <div className="text-4xl md:text-5xl font-bold text-white mb-2">
                    {stat.value}
                  </div>
                  <div className="text-brand-200">{stat.label}</div>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Company Info */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <div className="grid md:grid-cols-2 gap-12">
                <motion.div {...getRevealProps(0)}>
                  <div className="flex items-center gap-3 mb-4">
                    <Building2 className="w-6 h-6 text-brand-600" />
                    <h3 className="text-xl font-bold text-gray-900">Legal entity</h3>
                  </div>
                  <div className="bg-gray-50 rounded-xl p-6">
                    <p className="font-semibold text-gray-900 mb-2">Bel Consulting OÜ</p>
                    <p className="text-gray-600 mb-1">Harju maakond, Tallinn</p>
                    <p className="text-gray-600 mb-1">Kesklinna linnaosa</p>
                    <p className="text-gray-600 mb-1">Narva mnt 5, 10117</p>
                    <p className="text-gray-600">Estonia</p>
                    <div className="mt-4 pt-4 border-t border-gray-200">
                      <p className="text-sm text-gray-500">
                        VAT: EE102589552 • Registry code: 16916571
                      </p>
                    </div>
                  </div>
                </motion.div>

                <motion.div {...getRevealProps(0.1)}>
                  <div className="flex items-center gap-3 mb-4">
                    <Globe className="w-6 h-6 text-brand-600" />
                    <h3 className="text-xl font-bold text-gray-900">Global presence</h3>
                  </div>
                  <div className="bg-gray-50 rounded-xl p-6">
                    <p className="text-gray-600 mb-4">
                      Headquartered in Estonia, serving publishers worldwide with infrastructure 
                      deployed across multiple regions for low-latency ad serving.
                    </p>
                    <ul className="space-y-2 text-gray-600">
                      <li className="flex items-center gap-2">
                        <Users className="w-4 h-4 text-brand-600" />
                        Remote-first team
                      </li>
                      <li className="flex items-center gap-2">
                        <Shield className="w-4 h-4 text-brand-600" />
                        EU GDPR compliant
                      </li>
                      <li className="flex items-center gap-2">
                        <Scale className="w-4 h-4 text-brand-600" />
                        Signed-log verification
                      </li>
                    </ul>
                  </div>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* CTA */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-6">
                Want to learn more?
              </h2>
              <p className="text-xl text-gray-600 mb-8">
                We&apos;d love to hear from you. Reach out to discuss how Apex can help your team.
              </p>
              <a
                href="/contact"
                className="inline-flex items-center gap-2 bg-brand-600 text-white px-8 py-4 rounded-lg font-semibold hover:bg-brand-700 transition-colors"
              >
                Get in touch
              </a>
            </motion.div>
          </div>
        </section>
      </main>

      <Footer />
      <CookieBanner />
    </>
  );
}
