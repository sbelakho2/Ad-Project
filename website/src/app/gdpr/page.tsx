'use client';

import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';
import CookieBanner from '@/components/CookieBanner';

const LAST_UPDATED = '2025-12-12';

export default function GdprPage() {
  return (
    <>
      <main className="relative bg-white text-slate-900">
        <header className="bg-white border-b border-slate-200/60">
          <div className="container mx-auto px-4 py-6">
            <HomeNav />
          </div>
        </header>

        <section className="py-14 md:py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-3xl">
              <h1 className="text-4xl md:text-5xl font-bold text-gray-900 leading-tight">GDPR</h1>
              <p className="mt-4 text-sm text-gray-600">Last updated: {LAST_UPDATED}</p>

              <div className="mt-10 space-y-8">
                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">1) Overview</h2>
                  <p className="text-gray-700">
                    ApexMediation is operated by Bel Consulting OÜ (Estonia). We take GDPR compliance seriously
                    and design the platform to support publishers in meeting their obligations.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">2) Data subject rights</h2>
                  <p className="text-gray-700">Depending on the context, you may have rights including:</p>
                  <ul className="list-disc pl-6 text-gray-700 space-y-2">
                    <li>Access to your personal data</li>
                    <li>Rectification</li>
                    <li>Erasure</li>
                    <li>Restriction and objection</li>
                    <li>Portability</li>
                  </ul>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">3) How to make a request</h2>
                  <p className="text-gray-700">
                    If your request relates to the ApexMediation website or your account as a business contact,
                    email <a className="underline" href="mailto:security@apexmediation.ee">security@apexmediation.ee</a>.
                  </p>
                  <p className="text-gray-700">
                    If your request relates to end-user data in a publisher&apos;s app, contact the publisher first.
                    ApexMediation typically acts as a Processor for such data and will assist the publisher as required.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">4) DPA for customers</h2>
                  <p className="text-gray-700">
                    Customers can review our Data Processing Addendum here: <a className="underline" href="/dpa">/dpa</a>.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">5) More details</h2>
                  <p className="text-gray-700">
                    See our <a className="underline" href="/privacy">Privacy Policy</a> for a complete explanation of
                    data handling and purposes.
                  </p>
                </section>
              </div>

              <div className="mt-10 rounded-xl border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                <p className="font-semibold text-gray-900">Not legal advice</p>
                <p className="mt-1">This page is informational and not legal advice.</p>
              </div>
            </div>
          </div>
        </section>

        <CookieBanner />
      </main>
      <Footer />
    </>
  );
}
