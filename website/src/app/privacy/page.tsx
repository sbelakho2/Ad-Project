'use client';

import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';
import CookieBanner from '@/components/CookieBanner';
import { legalEntity } from '@/config/legal';

const LAST_UPDATED = '2025-12-12';

export default function PrivacyPolicyPage() {
  const sections = [
    { id: 'who-we-are', label: '1. Who we are' },
    { id: 'roles', label: '2. Roles (Controller vs Processor)' },
    { id: 'data-collected', label: '3. Data we collect' },
    { id: 'purposes', label: '4. Purposes and legal bases' },
    { id: 'sharing', label: '5. Sharing and subprocessors' },
    { id: 'transfers', label: '6. International transfers' },
    { id: 'security', label: '7. Security' },
    { id: 'retention', label: '8. Retention' },
    { id: 'rights', label: '9. Your rights' },
    { id: 'cookies', label: '10. Cookies' },
    { id: 'changes', label: '11. Changes' },
  ];

  return (
    <>
      <main className="relative bg-white text-slate-900">
        <header className="bg-white border-b border-slate-200/60">
          <div className="container mx-auto px-4 py-6">
            <HomeNav />
          </div>
        </header>

        <section className="py-12 md:py-16">
          <div className="container mx-auto px-4">
            <div className="flex gap-12 max-w-6xl mx-auto">
              {/* Sticky TOC */}
              <aside className="hidden lg:block w-64 flex-shrink-0">
                <nav className="sticky top-24">
                  <p className="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-4">On this page</p>
                  <ul className="space-y-2 text-sm">
                    {sections.map((s) => (
                      <li key={s.id}>
                        <a href={`#${s.id}`} className="text-gray-600 hover:text-brand-600 transition-colors">
                          {s.label}
                        </a>
                      </li>
                    ))}
                  </ul>
                </nav>
              </aside>

              {/* Content */}
              <div className="flex-1 max-w-prose">
                <h1 className="text-4xl md:text-5xl font-bold text-gray-900 leading-tight">
                  Privacy Policy
                </h1>
                <p className="mt-4 text-sm text-gray-600">Last updated: {LAST_UPDATED}</p>

                <div className="mt-10 space-y-8 legal-prose">
                <section id="who-we-are" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">1) Who we are</h2>
                  <p className="text-gray-700">
                    ApexMediation is a brand owned and operated by Bel Consulting OÜ (Estonia) (&quot;we&quot;, &quot;us&quot;).
                    This Privacy Policy explains how we handle personal data when you use our website,
                    developer documentation, Console, SDKs, and APIs (collectively, the &quot;Services&quot;).
                  </p>
                  <div className="rounded-xl border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                    <p className="font-semibold text-gray-900">Controller contact</p>
                    <p className="mt-1">{legalEntity.name} ({legalEntity.registryLine})</p>
                    <p>{legalEntity.fullAddress}</p>
                    <p>
                      Privacy inquiries: <a className="underline" href={`mailto:${legalEntity.contact.privacyEmail}`}>{legalEntity.contact.privacyEmail}</a>
                    </p>
                  </div>
                </section>

                <section id="roles" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">2) Roles under GDPR (Controller vs Processor)</h2>
                  <p className="text-gray-700">
                    Depending on the context, we act as:
                  </p>
                  <ul className="list-disc pl-6 text-gray-700 space-y-1">
                    <li>
                      <span className="font-semibold">Controller</span> for data collected on our website and for
                      account/contract administration (for example, contact requests, account signup,
                      security communications).
                    </li>
                    <li>
                      <span className="font-semibold">Processor</span> for end-user data processed on behalf of our
                      publisher customers when they use ApexMediation in their apps/sites.
                    </li>
                  </ul>
                  <p className="text-gray-700">
                    If you are an end user of a publisher&apos;s app, the publisher is typically the Controller for
                    ad-monetization data. Your primary privacy contact is that publisher.
                  </p>
                </section>

                <section id="data-collected" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">3) Data we collect</h2>
                  <p className="text-gray-700">
                    The data we process depends on how you interact with the Services.
                  </p>

                  <h3 className="text-lg font-semibold text-gray-900 mt-4">Website &amp; sales/contact</h3>
                  <ul className="list-disc pl-6 text-gray-700 space-y-1">
                    <li>Contact details (name, email), company name, message contents.</li>
                    <li>Basic analytics/telemetry about site usage (see Cookies section below).</li>
                  </ul>

                  <h3 className="text-lg font-semibold text-gray-900 mt-4">Console &amp; platform operations</h3>
                  <ul className="list-disc pl-6 text-gray-700 space-y-1">
                    <li>Account identity data: email, name, organization membership, roles/permissions.</li>
                    <li>Billing &amp; finance contacts and records needed to provide the service.</li>
                    <li>Security and audit logs (e.g., sign-in events, API key creation, admin actions).</li>
                  </ul>

                  <h3 className="text-lg font-semibold text-gray-900 mt-4">SDK / ad-transaction data (processed for publishers)</h3>
                  <p className="text-gray-700">
                    When publishers integrate the ApexMediation SDK and backend APIs, we may process data
                    required to run auctions, detect fraud, provide reconciliation, and deliver reporting. The
                    exact fields depend on the publisher&apos;s integration and configuration.
                  </p>
                  <ul className="list-disc pl-6 text-gray-700 space-y-1">
                    <li>App identifiers (publisher app ID, placement ID) and configuration signals.</li>
                    <li>Event telemetry (impressions, clicks, revenue events) and operational diagnostics.</li>
                    <li>IP address and device/network metadata may be processed for fraud prevention, rate limiting, and security.</li>
                  </ul>
                  <p className="text-gray-700">
                    We aim to minimize collection and avoid cross-publisher sharing. Publishers control their
                    own end-user relationships and consent obligations.
                  </p>
                </section>

                <section id="purposes" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">4) Purposes and legal bases</h2>
                  <ul className="list-disc pl-6 text-gray-700 space-y-2">
                    <li>
                      <span className="font-semibold">Contract</span>: to provide the Console, APIs, and support; to
                      administer accounts and billing.
                    </li>
                    <li>
                      <span className="font-semibold">Legitimate interests</span>: to secure the Services, prevent
                      abuse/fraud, improve reliability, and maintain audit logs.
                    </li>
                    <li>
                      <span className="font-semibold">Consent</span>: where required for certain cookies/analytics or
                      marketing communications.
                    </li>
                    <li>
                      <span className="font-semibold">Legal obligations</span>: compliance, accounting, and record retention.
                    </li>
                  </ul>
                </section>

                <section id="sharing" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">5) Sharing and subprocessors</h2>
                  <p className="text-gray-700">
                    We may share data with service providers (subprocessors) that help us host, monitor, and
                    operate the Services (for example: infrastructure providers, email delivery, error tracking).
                    We require subprocessors to protect data and only process it for agreed purposes.
                  </p>
                  <p className="text-gray-700">
                    Where required, we will publish and maintain a subprocessor list and update customers in
                    accordance with our Data Processing Addendum.
                  </p>
                </section>

                <section id="transfers" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">6) International transfers</h2>
                  <p className="text-gray-700">
                    We are based in the EEA (Estonia). Some vendors may process data outside the EEA.
                    When this happens, we use appropriate safeguards (such as Standard Contractual Clauses
                    or equivalent mechanisms) as required under GDPR.
                  </p>
                </section>

                <section id="security" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">7) Security</h2>
                  <p className="text-gray-700">
                    We use technical and organizational measures designed to protect data, including access
                    controls, encryption in transit where supported, and audit logging. No system is perfectly
                    secure; we encourage customers to use strong access controls and protect API keys.
                  </p>
                </section>

                <section id="retention" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">8) Retention</h2>
                  <p className="text-gray-700">
                    We keep personal data only as long as necessary for the purposes described above,
                    including legal, accounting, and security needs. Operational logs and reconciliation
                    evidence may be retained longer where required for fraud prevention, dispute handling,
                    or regulatory obligations.
                  </p>
                </section>

                <section id="rights" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">9) Your rights</h2>
                  <p className="text-gray-700">
                    If we act as Controller for your data, you may have rights under GDPR, including access,
                    rectification, erasure, restriction, portability, and objection. You may also lodge a
                    complaint with your local supervisory authority.
                  </p>
                  <p className="text-gray-700">
                    If we act as Processor (for example, for end-user data in a publisher&apos;s app), we will
                    assist the Controller as required; requests should generally be directed to the publisher.
                  </p>
                </section>

                <section id="cookies" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">10) Cookies</h2>
                  <p className="text-gray-700">
                    We use cookies and similar technologies for basic site functionality and (optionally)
                    analytics. See our <a className="underline" href="/cookies">Cookie Policy</a> for details.
                  </p>
                </section>

                <section id="changes" className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">11) Changes</h2>
                  <p className="text-gray-700">
                    We may update this Policy to reflect changes in our Services or legal requirements.
                    We will post the updated version on this page with a new “Last updated” date.
                  </p>
                </section>
              </div>

              <div className="mt-10 rounded-xl border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                <p className="font-semibold text-gray-900">Not legal advice</p>
                <p className="mt-1">
                  This Privacy Policy is provided for transparency and operational clarity. It is not legal advice.
                  If you need legal guidance for your specific situation, consult a qualified lawyer.
                </p>
              </div>
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
