'use client';

import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';
import CookieBanner from '@/components/CookieBanner';
import { legalEntity } from '@/config/legal';

const LAST_UPDATED = '2025-12-12';

export default function DpaPage() {
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
              <h1 className="text-4xl md:text-5xl font-bold text-gray-900 leading-tight">
                Data Processing Addendum (DPA)
              </h1>
              <p className="mt-4 text-sm text-gray-600">Last updated: {LAST_UPDATED}</p>

              <div className="mt-10 space-y-8">
                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">1) Parties and scope</h2>
                  <p className="text-gray-700">
                    This Data Processing Addendum (&quot;DPA&quot;) forms part of the agreement between {legalEntity.name}
                    ({legalEntity.address.country}) (&quot;Processor&quot;, &quot;ApexMediation&quot;) and the customer organization that uses the Services
                    (&quot;Controller&quot;, &quot;Customer&quot;).
                  </p>
                  <p className="text-gray-700">
                    This DPA applies when ApexMediation processes personal data on behalf of Customer in
                    connection with the Services.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">2) Definitions</h2>
                  <p className="text-gray-700">
                    Terms such as “personal data”, “processing”, “controller”, “processor”, and “supervisory authority”
                    have the meanings given in the GDPR.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">3) Processing details</h2>
                  <p className="text-gray-700">
                    The subject matter, duration, nature, and purpose of processing are described in Annex 1.
                    Customer determines the purposes and means of processing.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">4) Processor obligations</h2>
                  <ul className="list-disc pl-6 text-gray-700 space-y-2">
                    <li>Process personal data only on documented instructions from Customer.</li>
                    <li>Ensure persons authorized to process personal data are bound by confidentiality.</li>
                    <li>Implement appropriate technical and organizational measures (see Annex 2).</li>
                    <li>Assist Customer with data subject rights requests as required under GDPR.</li>
                    <li>Notify Customer without undue delay after becoming aware of a personal data breach affecting Customer Data.</li>
                    <li>At Customer&apos;s choice, delete or return Customer Data at the end of the Services, unless retention is required by law.</li>
                  </ul>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">5) Subprocessors</h2>
                  <p className="text-gray-700">
                    Customer authorizes ApexMediation to use subprocessors to provide the Services.
                    ApexMediation will impose data protection obligations on subprocessors that are no less
                    protective than those in this DPA.
                  </p>
                  <p className="text-gray-700">
                    Where required, ApexMediation will maintain a subprocessor list and provide a mechanism
                    for customers to receive updates and raise reasonable objections.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">6) International transfers</h2>
                  <p className="text-gray-700">
                    If Customer Data is transferred outside the EEA/UK/Switzerland, the parties will ensure
                    appropriate transfer safeguards are in place (such as Standard Contractual Clauses where applicable).
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">7) Audits</h2>
                  <p className="text-gray-700">
                    Customer may request reasonable information to verify compliance with this DPA. Where required,
                    audits will be conducted at reasonable intervals, during business hours, and subject to confidentiality
                    and security requirements.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">8) Contact</h2>
                  <p className="text-gray-700">
                    DPA questions and requests: <a className="underline" href="mailto:legal@apexmediation.ee">legal@apexmediation.ee</a>
                  </p>
                </section>

                <section className="space-y-4">
                  <h2 className="text-2xl font-bold text-gray-900">Annex 1 — Processing description</h2>
                  <div className="rounded-xl border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700 space-y-2">
                    <p><span className="font-semibold text-gray-900">Subject matter:</span> Operation of ad mediation, analytics, fraud prevention, reconciliation, reporting, and platform administration.</p>
                    <p><span className="font-semibold text-gray-900">Duration:</span> For the term of Customer&apos;s use of the Services, plus any retention period required for security, dispute resolution, or legal obligations.</p>
                    <p><span className="font-semibold text-gray-900">Nature and purpose:</span> Ingestion of SDK/API events, auction execution, delivery of reporting to Customer, and maintaining operational/security logs.</p>
                    <p><span className="font-semibold text-gray-900">Categories of data subjects:</span> Customer end users (app/site users), Customer personnel (account admins), and Customer business contacts.</p>
                    <p><span className="font-semibold text-gray-900">Types of personal data:</span> Account identifiers (email/name), app/device/network metadata as configured by Customer, event telemetry (impressions/clicks/revenue), IP addresses (for security and fraud prevention), and support communications.</p>
                    <p><span className="font-semibold text-gray-900">Special categories:</span> Not intended. Customer must not send special category data unless explicitly agreed in writing with additional safeguards.</p>
                  </div>
                </section>

                <section className="space-y-4">
                  <h2 className="text-2xl font-bold text-gray-900">Annex 2 — Security measures</h2>
                  <div className="rounded-xl border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                    <ul className="list-disc pl-6 space-y-2">
                      <li>Access controls and least privilege for production systems.</li>
                      <li>Encryption in transit where supported (TLS) and secure secret handling.</li>
                      <li>Audit logging for key administrative actions and authentication events.</li>
                      <li>Separation of environments where feasible (development/staging/production).</li>
                      <li>Vulnerability and dependency hygiene (updates and scanning as part of engineering workflow).</li>
                      <li>Incident response process with customer notification for qualifying breaches.</li>
                    </ul>
                  </div>
                </section>
              </div>

              <div className="mt-10 rounded-xl border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                <p className="font-semibold text-gray-900">Not legal advice</p>
                <p className="mt-1">
                  This DPA is a template provided for convenience and is not legal advice.
                  Customers should review it with counsel.
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
