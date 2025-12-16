'use client';

import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';
import CookieBanner from '@/components/CookieBanner';

const LAST_UPDATED = '2025-12-12';

export default function CookiePolicyPage() {
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
                Cookie Policy
              </h1>
              <p className="mt-4 text-sm text-gray-600">Last updated: {LAST_UPDATED}</p>

              <div className="mt-10 space-y-8">
                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">1) What cookies are</h2>
                  <p className="text-gray-700">
                    Cookies are small text files stored on your device. They help websites remember
                    preferences, maintain sessions, and understand how users interact with pages.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">2) How we use cookies</h2>
                  <p className="text-gray-700">
                    ApexMediation uses cookies and similar technologies to:
                  </p>
                  <ul className="list-disc pl-6 text-gray-700 space-y-2">
                    <li>Enable essential site functionality (navigation, security protections).</li>
                    <li>Remember cookie preferences where a consent banner is shown.</li>
                    <li>Optionally measure performance and reliability of the website.</li>
                  </ul>

                  <div className="rounded-xl border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                    <p className="font-semibold text-gray-900">Identifiers currently used</p>
                    <ul className="mt-2 list-disc pl-6 space-y-1">
                      <li>
                        <span className="font-semibold">Cookie:</span> <span className="font-mono">session</span> (httpOnly)
                        — used to maintain an authenticated session when you sign in.
                      </li>
                      <li>
                        <span className="font-semibold">Local storage:</span> <span className="font-mono">cookieConsent</span>
                        — stores your cookie banner preference (this is not a cookie).
                      </li>
                    </ul>
                    <p className="mt-2">
                      We do not currently set third-party analytics cookies by default on this site.
                    </p>
                  </div>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">3) Categories</h2>
                  <div className="space-y-4">
                    <div className="rounded-xl border border-gray-200 bg-gray-50 p-4">
                      <p className="font-semibold text-gray-900">Essential</p>
                      <p className="mt-1 text-sm text-gray-700">
                        Required for the website to function. These cannot be disabled in some contexts.
                      </p>
                    </div>
                    <div className="rounded-xl border border-gray-200 bg-gray-50 p-4">
                      <p className="font-semibold text-gray-900">Analytics (optional)</p>
                      <p className="mt-1 text-sm text-gray-700">
                        Help us understand page usage and improve content and reliability. Where required by law,
                        we will request consent before setting analytics cookies.
                      </p>
                    </div>
                  </div>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">4) Managing cookies</h2>
                  <p className="text-gray-700">
                    You can manage cookies through your browser settings. Most browsers let you remove
                    existing cookies and block future cookies. Note that blocking essential cookies may
                    break parts of the site.
                  </p>
                </section>

                <section className="space-y-3">
                  <h2 className="text-2xl font-bold text-gray-900">5) More information</h2>
                  <p className="text-gray-700">
                    For details on personal data, see our <a className="underline" href="/privacy">Privacy Policy</a>.
                  </p>
                </section>
              </div>

              <div className="mt-10 rounded-xl border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                <p className="font-semibold text-gray-900">Not legal advice</p>
                <p className="mt-1">
                  This Cookie Policy is provided for transparency and is not legal advice.
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
