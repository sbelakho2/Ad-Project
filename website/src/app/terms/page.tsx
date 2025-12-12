'use client';

import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';
import CookieBanner from '@/components/CookieBanner';

const LAST_UPDATED = '2025-12-12';

export default function TermsOfServicePage() {
  return (
    <>
      <main className="relative bg-white text-gray-900">
        <div className="container relative z-10 py-6 md:py-10">
          <HomeNav />
        </div>

        <section className="py-14 md:py-20">
          <div className="container mx-auto px-4">
            <div className="max-w-3xl">
              <h1 className="text-4xl md:text-5xl font-bold text-gray-900 leading-tight">
                Terms of Service
              </h1>
              <p className="mt-4 text-sm text-gray-600">Last updated: {LAST_UPDATED}</p>

              <div className="mt-10 space-y-8">
                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">1) Agreement</h2>
                  <p className="text-gray-700">
                    These Terms of Service (&quot;Terms&quot;) govern your access to and use of the ApexMediation
                    website, Console, SDKs, APIs, and related services (the &quot;Services&quot;), operated by Bel
                    Consulting OÜ (Estonia) (&quot;ApexMediation&quot;, &quot;we&quot;, &quot;us&quot;).
                  </p>
                  <p className="text-gray-700">
                    By creating an account, integrating our SDK, accessing the Console, or otherwise using
                    the Services, you agree to these Terms.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">2) Who may use the Services</h2>
                  <p className="text-gray-700">
                    The Services are intended for businesses and developers. If you are using the Services
                    on behalf of an organization, you represent that you have authority to bind that
                    organization.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">3) Accounts, authentication, and API keys</h2>
                  <ul className="list-disc pl-6 text-gray-700 space-y-2">
                    <li>You are responsible for keeping credentials and API keys secure.</li>
                    <li>You must promptly notify us of suspected compromise: <a className="underline" href="mailto:security@apexmediation.ee">security@apexmediation.ee</a>.</li>
                    <li>You may only create and use API keys for apps and organizations you own or are authorized to manage.</li>
                  </ul>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">4) Acceptable use</h2>
                  <p className="text-gray-700">You agree not to:</p>
                  <ul className="list-disc pl-6 text-gray-700 space-y-2">
                    <li>Use the Services to break the law or infringe third-party rights.</li>
                    <li>Interfere with, disrupt, or attempt to gain unauthorized access to our systems.</li>
                    <li>Reverse engineer or attempt to extract source code except to the extent permitted by law.</li>
                    <li>Upload malware, run abusive traffic, or bypass rate limits or access controls.</li>
                    <li>Misrepresent auction events, reporting, or payment records in a way that enables fraud.</li>
                  </ul>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">5) Publisher responsibilities</h2>
                  <p className="text-gray-700">
                    You are responsible for your apps/sites, your end users, and your compliance obligations
                    (including consent, notices, and lawful bases for processing) in your use of ad mediation
                    and analytics.
                  </p>
                  <p className="text-gray-700">
                    You are also responsible for your integrations, including ensuring that SDK versions are
                    up-to-date and configured correctly.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">6) Fees and billing</h2>
                  <p className="text-gray-700">
                    Fees, billing cycles, and plan details are described on our pricing pages, documentation,
                    or in an order form. Unless otherwise agreed, fees are due as invoiced. Taxes may apply.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">7) Intellectual property</h2>
                  <p className="text-gray-700">
                    We own the Services, including the Console and our SDKs, except for third-party
                    components and your content. You retain ownership of your apps, data, and trademarks.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">8) Data protection</h2>
                  <p className="text-gray-700">
                    Our <a className="underline" href="/privacy">Privacy Policy</a> describes how we handle personal data.
                    If we process personal data on your behalf as a Processor, the Data Processing Addendum
                    (&quot;DPA&quot;) applies: <a className="underline" href="/dpa">/dpa</a>.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">9) Availability and support</h2>
                  <p className="text-gray-700">
                    We aim to provide a reliable service, but the Services are provided on an “as is” and
                    “as available” basis, subject to applicable law and any written SLA you have with us.
                    Maintenance, outages, and changes may occur.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">10) Limitation of liability</h2>
                  <p className="text-gray-700">
                    To the maximum extent permitted by law, we are not liable for indirect, incidental,
                    special, consequential, or punitive damages, or for loss of profits, revenue, data, or
                    goodwill. Our aggregate liability for any claim relating to the Services will not exceed
                    the amounts paid by you to us for the Services in the 12 months preceding the event
                    giving rise to the claim.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">11) Termination</h2>
                  <p className="text-gray-700">
                    You may stop using the Services at any time. We may suspend or terminate access if you
                    violate these Terms, cause security risks, or use the Services unlawfully.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">12) Governing law</h2>
                  <p className="text-gray-700">
                    These Terms are governed by the laws of Estonia, without regard to conflict of law
                    principles. Where permitted, disputes will be resolved in the courts of Estonia.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">13) Contact</h2>
                  <p className="text-gray-700">
                    Questions about these Terms: <a className="underline" href="mailto:legal@apexmediation.ee">legal@apexmediation.ee</a>
                  </p>
                  <p className="text-gray-700">
                    Bel Consulting OÜ, Narva mnt 5, 10117 Tallinn, Estonia
                  </p>
                </section>
              </div>

              <div className="mt-10 rounded-xl border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                <p className="font-semibold text-gray-900">Not legal advice</p>
                <p className="mt-1">
                  These Terms are provided for operational clarity and are not legal advice.
                  If you need legal guidance, consult a qualified lawyer.
                </p>
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
