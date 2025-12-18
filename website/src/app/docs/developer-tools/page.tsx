'use client';

import { motion } from 'framer-motion';
import { BookOpen, Play, Terminal } from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';
import { repoPath } from '@/config/links';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

export default function DeveloperToolsDocPage() {
  return (
    <>
      <main className="relative bg-white text-ink">
        <SiteHeader />

        <PageHero className="bg-gradient-to-b from-gray-50 to-white border-b border-gray-200/60">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <div className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-brand-100 text-brand-700 text-sm font-medium mb-6">
                <Terminal className="w-4 h-4" />
                Developer Resources
              </div>
              <h1 className="text-4xl md:text-5xl font-bold text-ink leading-tight mb-6">
                Developer tools
              </h1>
              <p className="text-xl text-inkMuted">
                Practical tools and references for testing, debugging, and validating integrations.
              </p>
            </motion.div>
          </div>
        </PageHero>

        <section className="py-12 md:py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-3xl mx-auto space-y-10">
              <motion.section id="cli" {...getRevealProps(0)} className="scroll-mt-24">
                <div className="flex items-center gap-3 mb-3">
                  <div className="w-10 h-10 bg-brand-100 rounded-lg flex items-center justify-center">
                    <Terminal className="w-5 h-5 text-brand-600" />
                  </div>
                  <h2 className="text-2xl font-bold text-gray-900">CLI</h2>
                </div>
                <p className="text-gray-700">
                  For adapter development and integration validation, we ship a small &quot;adapter dev kit&quot; and runner
                  in the open-source repo.
                </p>
                <ul className="list-disc pl-6 text-gray-700 space-y-2 mt-3">
                  <li>
                    Repo: <a className="underline" href={repoPath('sdk/adapter-dev-kit')}>sdk/adapter-dev-kit</a>
                  </li>
                  <li>
                    iOS runner entrypoint: <a className="underline" href={repoPath('sdk/adapter-dev-kit/ios')}>sdk/adapter-dev-kit/ios</a>
                  </li>
                </ul>
              </motion.section>

              <motion.section id="test-console" {...getRevealProps(0.05)} className="scroll-mt-24">
                <div className="flex items-center gap-3 mb-3">
                  <div className="w-10 h-10 bg-brand-100 rounded-lg flex items-center justify-center">
                    <Play className="w-5 h-5 text-brand-600" />
                  </div>
                  <h2 className="text-2xl font-bold text-gray-900">Test Console</h2>
                </div>
                <p className="text-gray-700">
                  Use sandbox mode to simulate auctions and validate responses before going live.
                </p>
                <p className="text-gray-700 mt-3">
                  Start here: <a className="underline" href="/docs/getting-started/sandbox">Sandbox quickstart</a>
                </p>
              </motion.section>

              <motion.section id="figma-kit" {...getRevealProps(0.1)} className="scroll-mt-24">
                <div className="flex items-center gap-3 mb-3">
                  <div className="w-10 h-10 bg-brand-100 rounded-lg flex items-center justify-center">
                    <BookOpen className="w-5 h-5 text-brand-600" />
                  </div>
                  <h2 className="text-2xl font-bold text-gray-900">Figma Kit</h2>
                </div>
                <p className="text-gray-700">
                  The design kit is shared with customers and partners when needed for custom dashboards and internal tooling.
                </p>
                <p className="text-gray-700 mt-3">
                  Request access: <a className="underline" href="/contact">Contact us</a>
                </p>
              </motion.section>
            </div>
          </div>
        </section>

        <CookieBanner />
      </main>
      <Footer />
    </>
  );
}
