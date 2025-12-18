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
import { PrinciplesToProductCard } from '@/components/VisualCard';
import { CompanyIdentity } from '@/components/CompanyIdentity';
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';
import { SLO } from '@/lib/slo';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

const values = [
  {
    title: 'Transparency',
    description:
      "Every auction is verifiable. You never wonder if you're being treated fairly.",
    icon: Shield,
  },
  {
    title: 'Publisher control',
    description: "You bring the networks, you set the rules. We don't gatekeep.",
    icon: Target,
  },
  {
    title: 'Privacy-first',
    description:
      "No fingerprinting, no secret sharing. Your users' data stays in your hands.",
    icon: Lock,
  },
  {
    title: 'Reliability',
    description: `${SLO.availability.target} uptime with decision latency p95 ${SLO.latency.p95}. We treat downtime as seriously as you do.`,
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
      <main className="relative bg-white text-ink">
        <SiteHeader />

        <PageHero variant="light">
          <motion.div
            {...getRevealProps(0)}
            className="max-w-3xl mx-auto text-center md:text-left"
          >
            <h1 className="text-4xl md:text-5xl font-bold leading-tight text-ink mb-6">
              Mediation shouldn&apos;t mean compromise.
            </h1>
            <p className="text-xl leading-relaxed text-inkMuted">
              We started ApexMediation as ad engineers who were tired of mediation
              platforms that weren’t built for publishers.
            </p>
          </motion.div>
        </PageHero>

        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-prose mx-auto">
              <motion.div {...getRevealProps(0)}>
                <div className="flex items-start gap-4 mb-8">
                  <div className="w-12 h-12 bg-brand-100 rounded-xl flex items-center justify-center flex-shrink-0">
                    <Heart className="w-6 h-6 text-brand-600" />
                  </div>
                  <div>
                    <h2 className="text-2xl md:text-3xl font-bold text-ink mt-0 mb-4">
                      Our story
                    </h2>
                    <p className="text-inkMuted leading-relaxed mb-4">
                      We built ApexMediation after years of watching legacy mediators erode
                      publisher trust: hidden algorithms, opaque fees, and networks that
                      couldn’t be removed. That integration tax made switching feel like
                      rebuilding from scratch.
                    </p>
                    <p className="text-inkMuted leading-relaxed mb-4">
                      We believed there had to be a better way—one where the publisher
                      retains control of every network relationship, where auctions are
                      auditable, and where the mediation layer is a tool, not a gatekeeper.
                    </p>
                    <p className="text-inkMuted leading-relaxed">
                      Today, ApexMediation is built alongside design-partner studios who
                      demand transparency, reliability, and the flexibility to run their ad
                      stack their way.
                    </p>
                  </div>
                </div>
              </motion.div>
            </div>
          </div>
        </section>

        <section className="py-16 bg-white border-t border-gray-200/60">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid lg:grid-cols-2 gap-12 items-start">
                <motion.div {...getRevealProps(0)}>
                  <h2 className="text-3xl md:text-4xl font-bold text-ink mb-4">
                    What we believe
                  </h2>
                  <p className="text-lg text-inkMuted mb-8 max-w-prose">
                    Our values aren&apos;t just words on a wall—they&apos;re built into every
                    line of code.
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
                          <h3 className="text-lg font-semibold text-ink mb-1">
                            {value.title}
                          </h3>
                          <p className="text-inkMuted text-sm">{value.description}</p>
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

        <section className="py-16 bg-brand-600">
          <div className="container mx-auto px-4">
            <div className="grid grid-cols-2 md:grid-cols-4 gap-8 max-w-5xl mx-auto">
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

        <section className="py-16 bg-white">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <div className="grid md:grid-cols-2 gap-12">
                <motion.div {...getRevealProps(0)}>
                  <div className="flex items-center gap-3 mb-4">
                    <Building2 className="w-6 h-6 text-brand-600" />
                    <h3 className="text-xl font-bold text-ink">Legal entity</h3>
                  </div>
                  <CompanyIdentity variant="full" />
                </motion.div>

                <motion.div {...getRevealProps(0.1)}>
                  <div className="flex items-center gap-3 mb-4">
                    <Globe className="w-6 h-6 text-brand-600" />
                    <h3 className="text-xl font-bold text-ink">Global presence</h3>
                  </div>
                  <div className="bg-gray-50 rounded-xl p-6 border border-gray-200/70">
                    <p className="text-inkMuted mb-4">
                      Headquartered in Estonia, serving publishers worldwide with
                      infrastructure designed for regional redundancy. Current regions and
                      reliability targets are published on our SLO page.
                    </p>
                    <p className="text-sm text-inkMuted">
                      <a href="/slo" className="underline">apexmediation.ee/slo</a>
                    </p>
                    <ul className="space-y-2 text-inkMuted">
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

        <section className="py-16 bg-white border-t border-gray-200/60">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <h2 className="text-3xl md:text-4xl font-bold text-ink mb-6">
                Want to learn more?
              </h2>
              <p className="text-xl text-inkMuted mb-8">
                We&apos;d love to hear from you. Reach out to discuss how Apex can help your
                team.
              </p>
              <a href="/contact" className="btn-primary">
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
