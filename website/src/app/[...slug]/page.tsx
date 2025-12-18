import Link from 'next/link';
import { notFound } from 'next/navigation';
import type { Metadata } from 'next';
import type { ReactNode } from 'react';

const RESERVED_SEGMENTS = new Set(['api']);
const SLUG_BASE_URL = 'https://apexmediation.ee';
const DEFAULT_TITLE = 'ApexMediation - Enterprise Ad Mediation Platform';
const DEFAULT_DESCRIPTION =
  'Maximize your mediation revenue with transparent reporting, fraud protection, and developer-first workflows.';

const SEO_MAP: Record<string, { title: string; description: string }> = {
  documentation: {
    title: 'Developer Documentation | ApexMediation',
    description: 'Reference SDK guides, RTB APIs, and integration checklists for every supported platform.',
  },
  pricing: {
    title: 'Transparent Pricing | ApexMediation',
    description: 'See Starter, Growth, Scale, and Enterprise pricing (0–2.5%) plus included support and automation.',
  },
  support: {
    title: 'Support & Implementation Help | ApexMediation',
    description: 'Open support tickets, review SLAs, and contact the developer advocacy team for launch assistance.',
  },
  about: {
    title: 'About Bel Consulting OÜ | ApexMediation',
    description: 'Learn how ApexMediation delivers transparent ad demand while keeping publishers in full control.',
  },
  team: {
    title: 'Leadership & Team | ApexMediation',
    description: 'Meet the revenue, product, and engineering leads building the ApexMediation platform.',
  },
  careers: {
    title: 'Careers at ApexMediation',
    description: 'Join a fully-remote team focused on transparent ad monetization, privacy, and revenue automation.',
  },
  press: {
    title: 'Press & Media Kit | ApexMediation',
    description: 'Download the latest press kit, brand assets, and company stats for coverage or partnerships.',
  },
  guides: {
    title: 'Implementation Guides | ApexMediation',
    description: 'Step-by-step guides for SDK integration, consent tooling, and adapter orchestration.',
  },
  'case-studies': {
    title: 'Case Studies | ApexMediation',
    description: 'See how publishers improve fill, transparency, and billing accuracy with ApexMediation.',
  },
  changelog: {
    title: 'Product Changelog | ApexMediation',
    description: 'Track weekly product improvements across SDKs, console, billing, and transparency tooling.',
  },
  blog: {
    title: 'ApexMediation Blog',
    description: 'Insights on ad monetization strategy, mediation best practices, and transparency-first revenue ops.',
  },
  'blog/monetization-trends-2025': {
    title: '2025 Monetization Trends | ApexMediation',
    description: 'Key 2025 trends across rewarded video, transparency, and mediation economics.',
  },
  privacy: {
    title: 'Privacy Policy | ApexMediation',
    description: 'How Bel Consulting OÜ processes publisher and end-user data with GDPR-first safeguards.',
  },
  terms: {
    title: 'Terms of Service | ApexMediation',
    description: 'Customer agreement covering platform usage, responsibilities, and service guarantees.',
  },
  gdpr: {
    title: 'GDPR & Data Processing | ApexMediation',
    description: 'DPIA guidance, roles, and lawful bases for GDPR-compliant mediation workflows.',
  },
  security: {
    title: 'Security Program | ApexMediation',
    description: 'Learn about encryption, access controls, and incident response SLAs protecting your data.',
  },
  cookies: {
    title: 'Cookie Policy | ApexMediation',
    description: 'Details on analytics, functional, and consent cookies used across apexmediation.ee.',
  },
};

const buildSlugKey = (slug: string[]): string => slug.map((segment) => segment.toLowerCase()).join('/');

export function generateMetadata({ params }: PageProps): Metadata {
  if (!params.slug?.length) {
    return {
      title: DEFAULT_TITLE,
      description: DEFAULT_DESCRIPTION,
    };
  }

  const slugKey = buildSlugKey(params.slug);
  const seo = SEO_MAP[slugKey];
  const path = `/${params.slug.join('/')}`;

  return {
    title: seo?.title ?? DEFAULT_TITLE,
    description: seo?.description ?? DEFAULT_DESCRIPTION,
    alternates: {
      canonical: `${SLUG_BASE_URL}${path}`,
    },
    openGraph: {
      title: seo?.title ?? DEFAULT_TITLE,
      description: seo?.description ?? DEFAULT_DESCRIPTION,
      url: `${SLUG_BASE_URL}${path}`,
      siteName: 'ApexMediation',
      type: 'article',
      images: [
        {
          url: '/og-image.png',
          width: 1200,
          height: 630,
        },
      ],
    },
    twitter: {
      card: 'summary_large_image',
      title: seo?.title ?? DEFAULT_TITLE,
      description: seo?.description ?? DEFAULT_DESCRIPTION,
      images: ['/og-image.png'],
    },
  };
}

type PageProps = {
  params: {
    slug: string[];
  };
};

type Breadcrumb = {
  href: string;
  label: string;
};

export default function InformationalPage({ params }: PageProps) {
  if (!params.slug || params.slug.length === 0) {
    notFound();
  }

  const firstSegment = params.slug[0];
  if (RESERVED_SEGMENTS.has(firstSegment)) {
    notFound();
  }

  const slugKey = buildSlugKey(params.slug);

  const breadcrumbs: Breadcrumb[] = params.slug.map((segment, index) => ({
    href: `/${params.slug.slice(0, index + 1).join('/')}`,
    label: segment.replace(/[-_]/g, ' '),
  }));

  const page = renderContent(slugKey, breadcrumbs);

  if (!page) {
    // For unknown slugs, return a 404 to avoid generic placeholder pages surfacing in prod
    notFound();
  }

  return page;
}

type PageLayoutProps = {
  title: string;
  intro: string;
  heroTag: string;
  breadcrumbs: Breadcrumb[];
  children: ReactNode;
};

function PageLayout({ title, intro, heroTag, breadcrumbs, children }: PageLayoutProps) {
  const lastIndex = breadcrumbs.length - 1;

  return (
    <main className="bg-gray-50">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-8 px-4 pb-16 pt-10 sm:px-6 lg:px-8">
        <nav aria-label="Breadcrumb" className="overflow-x-auto text-sm text-gray-600">
          <ol className="flex flex-wrap items-center gap-2 whitespace-nowrap">
            <li>
              <Link href="/" className="text-gray-500 transition hover:text-gray-900">
                Home
              </Link>
            </li>
            {breadcrumbs.map((crumb, index) => (
              <li key={crumb.href} className="flex items-center gap-2">
                <span aria-hidden="true" className="text-gray-400">
                  /
                </span>
                {index === lastIndex ? (
                  <span className="font-semibold capitalize text-gray-900">{crumb.label}</span>
                ) : (
                  <Link href={crumb.href} className="text-gray-500 transition hover:text-gray-900">
                    {crumb.label}
                  </Link>
                )}
              </li>
            ))}
          </ol>
        </nav>

        <header className="space-y-4 text-center md:text-left">
          <p className="text-xs font-semibold uppercase tracking-[0.35em] text-gray-500">{heroTag}</p>
          <h1 className="text-3xl font-semibold text-gray-900 sm:text-4xl lg:text-5xl">{title}</h1>
          <p className="text-base leading-relaxed text-gray-600 md:text-lg">{intro}</p>
        </header>

        <div className="space-y-10 lg:space-y-12">{children}</div>
      </div>
    </main>
  );
}

function renderContent(slugKey: string, breadcrumbs: Breadcrumb[]): React.ReactNode {
  switch (slugKey) {
    case 'documentation':
      return (
        <PageLayout
          title="Developer Documentation"
          intro="Integrate ApexMediation in record time, optimise your monetisation stack, and ship with confidence. Every guide below is production-ready and aligned to the design system you see throughout the product."
          heroTag="Docs"
          breadcrumbs={breadcrumbs}
        >
          <div className="grid gap-8 lg:grid-cols-[260px_1fr]">
            <aside>
              <nav className="sticky top-24 space-y-3 rounded-3xl bg-gray-50 p-6 text-sm font-semibold tracking-wide text-gray-700">
                <span className="block text-gray-500">Contents</span>
                <a href="#getting-started" className="block hover:text-brand-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))]">Getting started</a>
                <a href="#real-time-bidding" className="block hover:text-brand-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))]">Real-time bidding</a>
                <a href="#ml-fraud-detection" className="block hover:text-brand-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))]">ML fraud detection</a>
                <a href="#ab-testing-platform" className="block hover:text-brand-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))]">A/B testing</a>
                <a href="#integration-checklist" className="block hover:text-brand-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))]">Integration checklist</a>
                <a href="#sdk-reference" className="block hover:text-brand-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))]">SDK reference</a>
                <a href="#ssai-documentation" className="block hover:text-brand-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))]">SSAI documentation</a>
              </nav>
            </aside>
            <div className="space-y-8">
              <section id="getting-started" className="rounded-3xl bg-white p-8 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-semibold text-gray-900">1. Getting Started</h2>
                <p className="mt-4 text-body text-gray-700 leading-relaxed">
                  Create an account, add your first app, and invite collaborators. Once your app is connected you can enter your network credentials and start monetizing within minutes.
                </p>
                <ol className="mt-6 space-y-3 text-body text-gray-700">
                  <li className="rounded-2xl border border-gray-200 bg-gray-50 p-4">
                    <span className="font-semibold text-gray-900">Step 1:</span> Go to <Link href="/signup" className="font-semibold underline">Sign Up</Link> and provision your workspace.
                  </li>
                  <li className="rounded-2xl border border-gray-200 bg-gray-50 p-4">
                    <span className="font-semibold text-gray-900">Step 2:</span> In the dashboard, open <Link href="/dashboard/apps" className="font-semibold underline">Apps</Link> and add each platform build (iOS, Android, Unity, Web).
                  </li>
                  <li className="rounded-2xl border border-gray-200 bg-gray-50 p-4">
                    <span className="font-semibold text-gray-900">Step 3:</span> Download the SDK bundle that matches your engine and integrate with the code sample below.
                  </li>
                </ol>
                <pre className="mt-6 overflow-x-auto rounded-2xl bg-gray-900 p-6 text-sm text-white">
{`import { ApexMediation } from '@apexmediation/sdk';

ApexMediation.configure({
  appId: 'YOUR_APP_ID',
  apiKey: 'YOUR_API_KEY',
  testMode: false,
});

ApexMediation.loadInterstitial({ placementId: 'level_complete' });`}
                </pre>
              </section>

              <section id="real-time-bidding" className="rounded-3xl bg-white p-8 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-semibold text-gray-900">2. Real-Time Bidding</h2>
                <p className="mt-4 text-body text-gray-700 leading-relaxed">
                  Every impression is auctioned across the demand sources you configure. Enable header bidding, waterfall mediation, or a hybrid approach from the control center—always using adapters tied directly to your accounts.
                </p>
                <ul className="mt-4 space-y-3 text-body text-gray-700">
                  <li className="flex items-start gap-3">
                    <span className="mt-1 h-2 w-2 rounded-full bg-brand-500" aria-hidden="true" />
                    Real-time logs show bidder response time, CPM, and win rate.
                  </li>
                  <li className="flex items-start gap-3">
                    <span className="mt-1 h-2 w-2 rounded-full bg-brand-500" aria-hidden="true" />
                    Configure floors per country, device, or user cohort.
                  </li>
                  <li className="flex items-start gap-3">
                    <span className="mt-1 h-2 w-2 rounded-full bg-brand-500" aria-hidden="true" />
                    Export raw auction trails to BigQuery or S3 on an hourly cadence.
                  </li>
                </ul>
                <div className="mt-6 rounded-2xl bg-gray-50 p-6 text-sm text-gray-900">
                  <p className="font-semibold">API Endpoint</p>
                  <pre className="mt-2 overflow-x-auto text-xs text-gray-800">
{`POST https://api.apexmediation.ee/v1/auctions
Body: {
  "placement_id": "level_complete",
  "bid_floor": 1.75,
  "user_id": "abc123",
  "geo": "SE"
}`}
                  </pre>
                </div>
              </section>

              <section id="ml-fraud-detection" className="rounded-3xl bg-white p-8 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-semibold text-gray-900">3. ML Fraud Detection</h2>
                <p className="mt-4 text-body text-gray-700 leading-relaxed">
                  Our anomaly detection models are trained on 500k+ installs and refreshed nightly. Suspicious signals trigger automated quarantines and optional manual review flows.
                </p>
                <div className="mt-6 grid gap-4 md:grid-cols-2">
                  <div className="rounded-2xl border border-gray-200 bg-gray-50 p-4">
                    <h3 className="text-sm font-semibold text-gray-900">Signals</h3>
                    <ul className="mt-3 space-y-2 text-sm text-gray-700">
                      <li>Device fingerprint velocity</li>
                      <li>Bid shading anomalies</li>
                      <li>Geo spoofing heuristics</li>
                      <li>Click-to-install delta</li>
                    </ul>
                  </div>
                  <div className="rounded-2xl border border-gray-200 bg-gray-50 p-4">
                    <h3 className="text-sm font-semibold text-gray-900">Actions</h3>
                    <ul className="mt-3 space-y-2 text-sm text-gray-700">
                      <li>Auto-reject suspicious bids</li>
                      <li>Throttle inventory per network</li>
                      <li>Notify assigned analyst via Slack</li>
                      <li>Send evidence packet to partner</li>
                    </ul>
                  </div>
                </div>
              </section>

              <section id="ab-testing-platform" className="rounded-3xl bg-white p-8 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-semibold text-gray-900">4. A/B Testing Platform</h2>
                <p className="mt-4 text-body text-gray-700 leading-relaxed">
                  Launch Bayesian experiments with Thompson Sampling and get directional insights within hours. Experiments can target creative, placement layout, or mediation partner mix.
                </p>
                <div className="mt-6 grid gap-4 md:grid-cols-2">
                  <div className="rounded-2xl bg-brand-600 p-6 text-white">
                    <h3 className="text-sm font-bold uppercase">Primary metrics</h3>
                    <ul className="mt-3 space-y-2 text-sm">
                      <li>Revenue per daily active user</li>
                      <li>Ad impressions per user session</li>
                      <li>Interstitial opt-out rate</li>
                      <li>Average session length</li>
                    </ul>
                  </div>
                  <div className="rounded-2xl border border-gray-200 bg-white p-6">
                    <h3 className="text-sm font-semibold text-gray-900">Rollout guardrails</h3>
                    <p className="mt-3 text-sm text-gray-700">
                      Set minimum win probability thresholds, cap daily risk at the inventory level, and auto-promote winners when results clear 95% significance.
                    </p>
                  </div>
                </div>
              </section>

              <section id="integration-checklist" className="rounded-3xl bg-white p-8 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-semibold text-gray-900">5. Integration Checklist</h2>
                <ul className="mt-4 space-y-3 text-body text-gray-700">
                  <li className="flex items-start gap-3">
                    <span className="mt-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-success/10 text-success">
                      <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}><path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" /></svg>
                    </span>
                    <span>Add the SDK to your project and confirm no build errors.</span>
                  </li>
                  <li className="flex items-start gap-3">
                    <span className="mt-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-success/10 text-success">
                      <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}><path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" /></svg>
                    </span>
                    <span>Verify GDPR/CCPA consent flow for European and Californian users.</span>
                  </li>
                  <li className="flex items-start gap-3">
                    <span className="mt-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-success/10 text-success">
                      <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}><path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" /></svg>
                    </span>
                    <span>Test rewarded and interstitial placements using device test mode.</span>
                  </li>
                  <li className="flex items-start gap-3">
                    <span className="mt-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-success/10 text-success">
                      <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}><path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" /></svg>
                    </span>
                    <span>Enable analytics streaming to your data warehouse of choice.</span>
                  </li>
                  <li className="flex items-start gap-3">
                    <span className="mt-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-success/10 text-success">
                      <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}><path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" /></svg>
                    </span>
                    <span>View payout schedule and invoices (NET 30 payment terms) in <Link href="/dashboard/settings?tab=payment" className="font-bold underline">Settings → Payment Methods</Link>.</span>
                  </li>
                  <li className="flex items-start gap-3">
                    <span className="mt-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-success/10 text-success">
                      <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}><path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" /></svg>
                    </span>
                    <span>Review <Link href="/docs/ssai/prerequisites" className="font-bold underline">SSAI prerequisites</Link> if using server-side ad insertion.</span>
                  </li>
                </ul>
              </section>

              <section id="sdk-reference" className="rounded-3xl bg-white p-8 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-semibold text-gray-900">6. SDK Reference</h2>
                <table className="mt-4 w-full table-auto text-left text-sm text-gray-700">
                  <thead>
                    <tr className="text-gray-900">
                      <th className="border-b-2 border-gray-200 py-3">Method</th>
                      <th className="border-b-2 border-gray-200 py-3">Description</th>
                      <th className="border-b-2 border-gray-200 py-3">Usage</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr className="odd:bg-gray-50">
                      <td className="py-3 font-bold">configure()</td>
                      <td>Bootstraps the SDK with your credentials.</td>
                      <td><code className="text-xs">ApexMediation.configure({`{ appId, apiKey }`})</code></td>
                    </tr>
                    <tr className="odd:bg-gray-50">
                      <td className="py-3 font-bold">loadInterstitial()</td>
                      <td>Preloads an interstitial ad for a placement.</td>
                      <td><code className="text-xs">ApexMediation.loadInterstitial({`{ placementId }`})</code></td>
                    </tr>
                    <tr className="odd:bg-cream/40">
                      <td className="py-3 font-bold">showRewarded()</td>
                      <td>Displays a rewarded video and resolves with reward payload.</td>
                      <td><code className="text-xs">await ApexMediation.showRewarded({`{ placementId }`})</code></td>
                    </tr>
                  </tbody>
                </table>
              </section>

              <section id="ssai-documentation" className="rounded-3xl bg-white p-8 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-semibold text-gray-900">7. SSAI Documentation</h2>
                <p className="mt-4 text-body text-gray-700 leading-relaxed">
                  Complete guides for server-side ad insertion across Live and VOD content. Covers protocols, containers, DRM, and operational best practices.
                </p>
                <div className="mt-6 grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                  <Link href="/docs/ssai" className="group rounded-2xl border border-gray-200 bg-gray-50 p-4 transition-colors hover:border-brand-500 hover:bg-brand-50">
                    <h3 className="font-semibold text-gray-900 group-hover:text-brand-700">SSAI Overview</h3>
                    <p className="mt-1 text-sm text-gray-600">Architecture, protocols, and getting started</p>
                  </Link>
                  <Link href="/docs/ssai/prerequisites" className="group rounded-2xl border border-gray-200 bg-gray-50 p-4 transition-colors hover:border-brand-500 hover:bg-brand-50">
                    <h3 className="font-semibold text-gray-900 group-hover:text-brand-700">Prerequisites</h3>
                    <p className="mt-1 text-sm text-gray-600">Origin, cue types, codec ladder requirements</p>
                  </Link>
                  <Link href="/docs/ssai/operations" className="group rounded-2xl border border-gray-200 bg-gray-50 p-4 transition-colors hover:border-brand-500 hover:bg-brand-50">
                    <h3 className="font-semibold text-gray-900 group-hover:text-brand-700">Operations</h3>
                    <p className="mt-1 text-sm text-gray-600">Failure modes, fallback ladder, SLAs</p>
                  </Link>
                  <Link href="/docs/ssai/cdn" className="group rounded-2xl border border-gray-200 bg-gray-50 p-4 transition-colors hover:border-brand-500 hover:bg-brand-50">
                    <h3 className="font-semibold text-gray-900 group-hover:text-brand-700">CDN Guidance</h3>
                    <p className="mt-1 text-sm text-gray-600">Cache policies, headers, common misconfigs</p>
                  </Link>
                  <Link href="/docs/ssai/measurement" className="group rounded-2xl border border-gray-200 bg-gray-50 p-4 transition-colors hover:border-brand-500 hover:bg-brand-50">
                    <h3 className="font-semibold text-gray-900 group-hover:text-brand-700">Measurement</h3>
                    <p className="mt-1 text-sm text-gray-600">Tracking, exactly-once, reconciliation</p>
                  </Link>
                  <Link href="/docs/playback-matrix" className="group rounded-2xl border border-gray-200 bg-gray-50 p-4 transition-colors hover:border-brand-500 hover:bg-brand-50">
                    <h3 className="font-semibold text-gray-900 group-hover:text-brand-700">Playback Matrix</h3>
                    <p className="mt-1 text-sm text-gray-600">Certified player/platform compatibility</p>
                  </Link>
                </div>
              </section>
            </div>
          </div>
        </PageLayout>
      );

    case 'pricing':
      return (
        <PageLayout
          title="Pricing"
          intro="Simple, transparent pricing. Connect your demand accounts, keep the lion's share of revenue, and pay a straightforward platform fee (0–2.5% for most publishers, custom 1.0–1.5% for Enterprise) that funds SDKs, observability, fraud tooling, and payouts."
          heroTag="Billing"
          breadcrumbs={breadcrumbs}
        >
          <div className="space-y-6">
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-4">
              <h2 className="text-h3 font-bold uppercase">Platform tiers</h2>
              <p className="text-sm text-gray-700">
                Each tier bills only the revenue that lands inside its band, so your effective platform fee drops automatically as you scale. Managed demand is not bundled—you stay in control of networks, credentials, and auctions.
              </p>
              <div className="overflow-x-auto">
                <table className="w-full text-left text-sm text-gray-700">
                  <thead>
                    <tr>
                      <th className="py-2 font-semibold">Tier</th>
                      <th className="py-2 font-semibold">Revenue band (per app monthly)</th>
                      <th className="py-2 font-semibold">Platform fee</th>
                      <th className="py-2 font-semibold">Value pillars</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr>
                      <td className="py-2 align-top">Tier 0 — Starter</td>
                      <td className="py-2 align-top">Up to $10k</td>
                      <td className="py-2 align-top">0% platform fee</td>
                      <td className="py-2 align-top">1M impressions, 100k API calls, and 50GB transfer included while you validate SDKs and consent flows.</td>
                    </tr>
                    <tr>
                      <td className="py-2 align-top">Tier 1 — Growth</td>
                      <td className="py-2 align-top">$10k – $100k</td>
                      <td className="py-2 align-top">2.5% platform fee</td>
                      <td className="py-2 align-top">Advanced observability, Migration Studio, transparency proofs, and billing assistants for ops teams.</td>
                    </tr>
                    <tr>
                      <td className="py-2 align-top">Tier 2 — Scale</td>
                      <td className="py-2 align-top">$100k – $500k</td>
                      <td className="py-2 align-top">2.0% platform fee</td>
                      <td className="py-2 align-top">Named revenue engineers, custom exports, fraud tooling in shadow → block mode, and SLO dashboards.</td>
                    </tr>
                    <tr>
                      <td className="py-2 align-top">Tier 3 — Enterprise</td>
                      <td className="py-2 align-top">$500k+ (plus platform minimum)</td>
                      <td className="py-2 align-top">Custom 1.0–1.5% platform fee</td>
                      <td className="py-2 align-top">Dedicated pod, bespoke compliance + residency reviews, contract SLAs, and white-glove migrations.</td>
                    </tr>
                  </tbody>
                </table>
              </div>
              <p className="text-sm text-gray-700">
                Example: $250k in mediated revenue lands entirely inside Scale, so the platform fee is $5,000 (2.0%) and $245k is paid out.
              </p>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">What the platform fee covers</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Credential vault, adapter orchestration, Config-as-Code, and unified API access to every supported network.</li>
                <li>Observability + fraud stack: telemetry, debugger replays, Migration Studio, redaction-ready reconciliation exports.</li>
                <li>Transparency + compliance: signed proofs, consent tooling (GDPR/TCF, COPPA, US-GPP), and audit-friendly logging.</li>
                <li>Finance automation: tier-aware billing, NET 30 payouts, €/$100 minimums, SEPA/SWIFT wiring, and Stripe portal access.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">Payment terms</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Invoices issued on the 1st of each month for the prior period; payment NET 30.</li>
                <li>€/$100 payout minimum applies. Balances roll over until the threshold is hit.</li>
                <li>Platform fee applies marginally per tier (0%, 2.5%, 2.0%, custom 1.0–1.5% with a negotiated minimum).</li>
                <li>Payment methods: SEPA, SWIFT, and PayPal (2.9% + $0.30 fee). Dashboards show converted local currency for convenience.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-brand-50 p-8 text-gray-900 shadow-xl ring-1 ring-brand-100 space-y-3">
              <h2 className="text-h3 font-bold uppercase">Talk to us</h2>
              <p className="text-sm text-gray-700">
                Non-profits receive 50% off the platform fee. Contact <a href="mailto:sales@apexmediation.ee" className="underline">sales@apexmediation.ee</a> or <Link href="/contact" className="font-bold underline">schedule a demo</Link> to lock in Starter/Growth/Scale pricing (30 days’ notice before changes).
              </p>
            </section>
          </div>
        </PageLayout>
      );

    case 'support':
      return (
        <PageLayout
          title="Support Center"
          intro="We are here around the clock. Explore quick answers below or open a ticket and we will respond within the published SLA."
          heroTag="Support"
          breadcrumbs={breadcrumbs}
        >
          <div className="grid gap-6 md:grid-cols-2">
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200" id="account-access">
              <h2 className="text-h3 font-bold uppercase">Account & Access</h2>
              <ul className="mt-4 space-y-3 text-sm text-gray-700">
                <li>Reset your password via <a href="https://console.apexmediation.ee/login" className="font-bold underline">Sign In</a> → “Forgot password”.</li>
                <li>Invite teammates from <Link href="/dashboard/settings?tab=security" className="font-bold underline">Settings → Security</Link>.</li>
                <li>Contact us at <a href="mailto:support@apexmediation.ee" className="underline">support@apexmediation.ee</a> for account merges.</li>
              </ul>
            </section>
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Monetisation Help</h2>
              <p className="mt-4 text-sm text-gray-700">
                Troubleshoot low fill, enable new demand sources, or schedule a mediation audit. Include app ID, platform, and SDK version for the fastest response.
              </p>
              <a
                href="mailto:support@apexmediation.ee?subject=Monetisation%20support"
                className="mt-6 inline-flex items-center gap-2 btn-primary text-sm"
              >
                Email Monetisation Team →
              </a>
            </section>
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Billing & Payouts</h2>
              <p className="mt-4 text-sm text-gray-700">
                Monthly payouts with NET 30 terms - invoiced on the 1st of each month, payment processed 30 days later. Bank transfers can take 2-5 business days. View invoices and remittance files in <Link href="/dashboard/settings?tab=payment" className="font-bold underline">Settings → Payment Methods</Link>.
              </p>
              <p className="mt-4 text-sm text-gray-700">To update banking info, submit a signed W-9/W-8BEN to <a href="mailto:billing@apexmediation.ee" className="underline">billing@apexmediation.ee</a>.</p>
            </section>
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Live chat & SLA</h2>
              <p className="mt-4 text-sm text-gray-700">
                Chat is available 09:00–23:00 CET, Monday to Saturday. Enterprise customers receive a dedicated Slack channel with 15-minute first-response guarantees.
              </p>
              <p className="mt-4 text-sm text-gray-700">
                Incident hotline (24/7): <a href="tel:+46101234567" className="underline">+46 10 123 45 67</a>
              </p>
            </section>
          </div>
          <section className="mt-10 rounded-3xl bg-white/10 p-8 text-sm text-white/80">
            <h2 className="text-sm font-bold uppercase text-brand-100">Need Help?</h2>
            <p className="mt-3">
              For security questions or to report vulnerabilities, email <a href="mailto:security@apexmediation.ee" className="underline">security@apexmediation.ee</a>. We publish RCA documents within 48 hours of every incident.
            </p>
          </section>
        </PageLayout>
      );

    case 'about':
      return (
        <PageLayout
          title="About ApexMediation"
          intro="ApexMediation is built by a distributed team obsessed with transparency, fairness, and delightful developer experiences."
          heroTag="Company"
          breadcrumbs={breadcrumbs}
        >
          <section className="space-y-8">
            <div className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Our Mission</h2>
              <p className="mt-4 text-body text-gray-700 leading-relaxed">
                We help game studios and app developers regain control over their monetisation stack. No black boxes, no slow payouts, no invasive SDKs—just clean APIs, clear reporting, and a partner you can trust.
              </p>
            </div>
            <div className="grid gap-6 md:grid-cols-3">
              {[
                { label: 'Founded', value: '2021, Tallinn' },
                { label: 'Team', value: '38 people across EU & US' },
                { label: 'Payouts processed', value: '$48M in 2024' },
              ].map((stat) => (
                <div key={stat.label} className="rounded-3xl bg-white p-6 text-gray-900 shadow ring-1 ring-gray-200">
                  <p className="text-xs font-bold uppercase text-brand-700">{stat.label}</p>
                  <p className="mt-2 text-2xl font-extrabold">{stat.value}</p>
                </div>
              ))}
            </div>
          </section>
        </PageLayout>
      );

    case 'team':
      return (
        <PageLayout
          title="Meet the Team"
          intro="A cross-functional crew of ad tech veterans, ML engineers, designers, and former studio operators."
          heroTag="Team"
          breadcrumbs={breadcrumbs}
        >
          <section className="grid gap-6 md:grid-cols-2">
            {[
              { name: 'Sara Blom', role: 'CEO & Co-founder', bio: 'Previously led monetisation strategy at King. Advocates for fair revenue share models.' },
              { name: 'Jonas Lind', role: 'CTO & Co-founder', bio: 'Built programmatic infrastructure at Spotify. Loves Typescript, Go, and fika.' },
              { name: 'Amelia Ortiz', role: 'VP of Product', bio: 'Former Unity Product Manager. Champion of developer-first tooling.' },
              { name: 'Priya Nair', role: 'Head of Machine Learning', bio: 'Scaled fraud detection at Adyen. Focused on explainable AI.' },
            ].map((person) => (
              <article key={person.name} className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-bold uppercase">{person.name}</h2>
                <p className="mt-2 text-sm font-semibold text-gray-700">{person.role}</p>
                <p className="mt-4 text-sm text-gray-700 leading-relaxed">{person.bio}</p>
              </article>
            ))}
          </section>
        </PageLayout>
      );

    case 'careers':
      return (
        <PageLayout
          title="Careers"
          intro="Join a remote-friendly team that ships fast, cares deeply about craft, and keeps work-life balance healthy."
          heroTag="Hiring"
          breadcrumbs={breadcrumbs}
        >
          <section className="space-y-6">
            {[
              {
                title: 'Senior Frontend Engineer',
                location: 'Tallinn or Remote (EU)',
                description: 'Build delightful dashboards and workflows in Next.js, Tailwind, and TypeScript.',
              },
              {
                title: 'Data Scientist, Monetisation',
                location: 'Remote (EU)',
                description: 'Model auction behaviour, design experiments, and ship production models.',
              },
              {
                title: 'Customer Success Manager',
                location: 'New York City, Hybrid',
                description: 'Partner with top-grossing studios to improve retention and ongoing insights.',
              },
            ].map((role) => (
              <article key={role.title} className="rounded-3xl border border-gray-200 bg-white p-6 text-gray-900 shadow">
                <h2 className="text-h3 font-bold uppercase">{role.title}</h2>
                <p className="mt-1 text-xs font-bold uppercase tracking-wide text-brand-700">{role.location}</p>
                <p className="mt-4 text-sm text-gray-700">{role.description}</p>
                <a
                  href="mailto:contact@apexmediation.ee"
                  className="mt-6 inline-flex items-center gap-2 btn-primary text-sm"
                >
                  Apply via Email →
                </a>
              </article>
            ))}
          </section>
        </PageLayout>
      );

    case 'press':
      return (
        <PageLayout
          title="Press"
          intro="Download media assets, read recent headlines, and contact our communications team."
          heroTag="Media"
          breadcrumbs={breadcrumbs}
        >
          <div className="grid gap-6 md:grid-cols-2">
            <section className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Media Resources</h2>
              <ul className="mt-4 space-y-3 text-sm text-gray-700">
                <li><a href="/press/apexmediation-media-kit.zip" className="underline">Media kit (logos, product shots)</a></li>
                <li><a href="/press/fact-sheet.pdf" className="underline">Fact sheet</a></li>
                <li><a href="mailto:sales@apexmediation.ee" className="underline">sales@apexmediation.ee</a></li>
              </ul>
            </section>
            <section className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Latest Headlines</h2>
              <ul className="mt-4 space-y-3 text-sm text-gray-700">
                <li>ApexMediation launches ML-driven fraud firewall across 5M daily impressions.</li>
                <li>Studios adopt ApexMediation Growth plan for unified forecasting.</li>
                <li>ApexMediation secures SOC 2 Type II attestation for enterprise customers.</li>
              </ul>
            </section>
          </div>
        </PageLayout>
      );

    case 'guides':
      return (
        <PageLayout
          title="Guides"
          intro="Deep dives on monetisation strategy, creative optimisation, and analytics best practices."
          heroTag="Guides"
          breadcrumbs={breadcrumbs}
        >
          <div className="space-y-6">
            {[
              {
                title: 'Launch Checklist for New Studios',
                summary: 'Everything you need to configure mediation, verify payouts, and test placements before launch day.',
                href: '/docs',
              },
              {
                title: 'How to Read Bid Landscape Reports',
                summary: 'Interpret auction diagnostics, identify underperforming partners, and tune floors by cohort.',
                href: '/dashboard/analytics',
              },
              {
                title: 'Designing Rewarded Video that Users Love',
                summary: 'Use behavioural cues and optionality to keep opt-in rates above 85% without sacrificing retention.',
                href: '/blog/monetization-trends-2025',
              },
            ].map((guide) => (
              <article key={guide.title} className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-bold uppercase">{guide.title}</h2>
                <p className="mt-3 text-sm text-gray-700">{guide.summary}</p>
                <a href={guide.href} className="mt-4 inline-flex items-center gap-2 text-sm font-bold uppercase text-brand-700 underline">
                  Continue reading →
                </a>
              </article>
            ))}
          </div>
        </PageLayout>
      );

    case 'case-studies':
      return (
        <PageLayout
          title="Case Studies"
          intro="Studios across genres rely on ApexMediation for stable revenue growth and delightful player experiences."
          heroTag="Success Stories"
          breadcrumbs={breadcrumbs}
        >
          <div className="grid gap-6 md:grid-cols-2">
            {[
              {
                title: 'Northvolt Games',
                stat: 'Full rollout in 60 days',
                summary: 'Swapped legacy waterfall mediation for ApexMediation Growth. Weekly lab sessions reduced setup friction and surfaced creative fatigue sooner.',
              },
              {
                title: 'Aurora Interactive',
                stat: 'Reduced fraud by 96%',
                summary: 'Implemented automated quarantines and network-level throttling. Saved $120k per month in invalid spend.',
              },
            ].map((study) => (
              <article key={study.title} className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
                <p className="text-xs font-bold uppercase text-brand-700">{study.stat}</p>
                <h2 className="mt-2 text-h3 font-bold uppercase">{study.title}</h2>
                <p className="mt-4 text-sm text-gray-700">{study.summary}</p>
              </article>
            ))}
          </div>
        </PageLayout>
      );

    case 'changelog':
      return (
        <PageLayout
          title="Changelog"
          intro="We deploy improvements weekly. Track every feature, fix, and maintenance update here."
          heroTag="Product Updates"
          breadcrumbs={breadcrumbs}
        >
          <ul className="space-y-6">
            {[
              {
                date: '2025-10-24',
                title: 'Dashboard refresh + palette update',
                items: ['Updated navigation focus states', 'Mobile sidebar with animated overlay', 'Revamped Settings page with URL-synced tabs'],
              },
              {
                date: '2025-09-30',
                title: 'Fraud firewall GA',
                items: ['Auto-quarantine for click flooding', 'New anomaly alerts via Slack', 'CSV export for appeals'],
              },
              {
                date: '2025-09-05',
                title: 'NET 30 payment terms + in-app billing view',
                items: ['Upload tax forms directly in Settings', 'Real-time payout timeline widget', 'Webhook for payout status changes'],
              },
            ].map((entry) => (
              <li key={entry.date} className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
                <p className="text-xs font-bold uppercase text-brand-700">{entry.date}</p>
                <h2 className="mt-2 text-h3 font-bold uppercase">{entry.title}</h2>
                <ul className="mt-3 list-disc space-y-2 pl-5 text-sm text-gray-700">
                  {entry.items.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </li>
            ))}
          </ul>
        </PageLayout>
      );

    case 'blog':
      return (
        <PageLayout
          title="Blog"
          intro="Insights for monetisation leads, product managers, and growth teams."
          heroTag="Stories"
          breadcrumbs={breadcrumbs}
        >
          <div className="space-y-6">
            {[
              {
                title: 'Monetisation Trends 2025',
                excerpt: 'What we are seeing across 120+ studios—privacy-first measurement, new pricing models, and creative best practices.',
                href: '/blog/monetization-trends-2025',
              },
              {
                title: 'Designing Ethical Rewarded Video',
                excerpt: 'Reward structure experiments that preserve retention while boosting opt-in 14%.',
                href: '/guides',
              },
            ].map((post) => (
              <article key={post.title} className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
                <h2 className="text-h3 font-bold uppercase">{post.title}</h2>
                <p className="mt-3 text-sm text-gray-700">{post.excerpt}</p>
                <a href={post.href} className="mt-4 inline-flex items-center gap-2 text-sm font-bold uppercase text-brand-700 underline">
                  Continue reading →
                </a>
              </article>
            ))}
          </div>
        </PageLayout>
      );

    case 'blog/monetization-trends-2025':
      return (
        <PageLayout
          title="Monetisation Trends 2025"
          intro="Privacy-first measurement, dynamic pricing, and rewarded design tactics we are tracking this year."
          heroTag="Blog"
          breadcrumbs={breadcrumbs}
        >
          <article className="prose prose-lg max-w-none rounded-3xl bg-white p-10 text-gray-900 shadow-xl ring-1 ring-gray-200">
            <h2>Key themes</h2>
            <ul>
              <li>Hybrid monetisation models pair IAP bundles with rewarded funnels.</li>
              <li>Studios allocate 20% more time to creative iteration each sprint.</li>
              <li>First-party data clean rooms unlock precise attribution without privacy debt.</li>
            </ul>
            <h2>What to do now</h2>
            <p>
              Audit the balance between compulsory ads and opt-in experiences. Rewarded video remains the most loved format when you respect session cadence and offer meaningful rewards.
            </p>
            <p>
              Need a partner?{' '}
              <Link href="/support" className="font-bold underline">
                Talk to our monetisation strategists
              </Link>
              .
            </p>
          </article>
        </PageLayout>
      );

    case 'privacy':
      return (
        <PageLayout
          title="Privacy Policy"
          intro="This summary mirrors the Customer-Facing Privacy, GDPR, and Troubleshooting guides so you can see exactly what data we process, why, and for how long."
          heroTag="Legal"
          breadcrumbs={breadcrumbs}
        >
          <div className="space-y-6">
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Data we collect</h2>
              <p className="mt-3 text-sm text-gray-700">
                We act as your data processor and only collect the fields described in the GDPR Compliance guide:
              </p>
              <ul className="mt-4 list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Device identifiers (IDFA, GAID, advertising ID, hashed user IDs)</li>
                <li>Device metadata (model, OS version, locale, screen size, timezone)</li>
                <li>Approximate location derived from IP for geo targeting and fraud controls</li>
                <li>Usage and ad interaction data (session duration, impressions, clicks, revenue)</li>
                <li>Publisher account data necessary for invoicing (company name, billing contact, tax IDs)</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">How we use data</h2>
              <ul className="mt-4 list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Deliver personalised or contextual ads when you obtain valid consent.</li>
                <li>Run fraud detection (device, behavioural, timing, and network signals) with 99.7% precision.</li>
                <li>Power analytics, pacing, and experimentation dashboards described in the Features docs.</li>
                <li>Process payouts under the marginal revenue-share pricing tables (NET 30, €100 minimum).</li>
              </ul>
              <p className="mt-4 text-sm text-gray-700">
                Legal bases include consent (personalised ads/analytics) and legitimate interest (fraud prevention, security, non-personalised ads). We never sell data or ingest special-category data.
              </p>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Storage & retention</h2>
              <p className="text-sm text-gray-700">
                Data resides in EU-West (Frankfurt) with encrypted backups in Dublin. Active users are retained while your app remains installed. Inactive user records auto-delete 90 days after the last event, and account-level data is removed 30 days after closure unless finance or law requires longer retention. Aggregated, anonymised metrics may persist for benchmarking.
              </p>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Your rights & contacts</h2>
              <ul className="mt-4 list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Access, deletion, portability, rectification, and objection rights honoured within 30 days.</li>
                <li>Use the documented `/v1/gdpr/access` and `/v1/gdpr/delete` endpoints or email <a href="mailto:security@apexmediation.ee" className="underline">security@apexmediation.ee</a>.</li>
                <li>Privacy inquiries: <a href="mailto:security@apexmediation.ee" className="underline">security@apexmediation.ee</a>.</li>
                <li>Sub-processors are limited to the ad networks you enable plus audited infrastructure vendors with SOC 2 Type II-aligned controls.</li>
              </ul>
            </section>
          </div>
        </PageLayout>
      );

    case 'terms':
      return (
        <PageLayout
          title="Terms of Service"
          intro="These Terms govern access to apexmediation.ee, the ApexMediation platform, SDKs, and related services operated by Bel Consulting OÜ."
          heroTag="Legal"
          breadcrumbs={breadcrumbs}
        >
          <div className="space-y-6">
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">1. Parties & acceptance</h2>
              <p className="text-sm text-gray-700">
                ApexMediation is a brand of Bel Consulting OÜ (registry code 14955546, Sepapaja 6, 15551 Tallinn, Estonia). By creating an account, integrating our SDKs, or accessing any resource on apexmediation.ee you enter into a binding agreement with Bel Consulting OÜ. If you represent an organisation, you confirm you have authority to accept these Terms on its behalf.
              </p>
              <p className="text-sm text-gray-700">
                These Terms incorporate the Privacy Policy, GDPR playbooks, Billing Policy Snapshot, and any order form or statement of work you sign. Conflicts resolve in the following order: custom order → DPA/SCCs → these Terms → public documentation.
              </p>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">2. Services</h2>
              <p className="text-sm text-gray-700">
                We provide SDKs, APIs, dashboards, reporting, and professional services that allow you to run a bring-your-own demand mediation stack. Feature availability may vary by tier. We may modify non-material features to improve performance or security but will not materially reduce core functionality without 30 days’ notice.
              </p>
              <p className="text-sm text-gray-700">
                Beta or preview features are offered “as is” and may be withdrawn at any time. You acknowledge they should not be relied upon for production workloads until designated GA in writing.
              </p>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">3. Accounts, security & acceptable use</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Maintain accurate profile information, enforce least-privilege roles, and enable MFA for Admin and Finance seats.</li>
                <li>You are responsible for all activity under your credentials. Notify <a href="mailto:security@apexmediation.ee" className="underline">security@apexmediation.ee</a> within 24 hours of any suspected compromise.</li>
                <li>Do not reverse engineer, benchmark for competitive purposes without consent, or use the platform to transmit malware, prohibited content, invalid traffic, or privacy-invasive signals.</li>
                <li>Provide truthful consent signals (GDPR/TCF v2.2, COPPA, US state laws) and honour user opt-outs captured via ApexMediation APIs or our consent helpers.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">4. Fees, billing & taxes</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Pricing tiers: Starter 0%, Growth 2.5%, Scale 2.0%, Enterprise 1.0–1.5% plus negotiated minimums. Fees accrue marginally per mediated revenue band and exclude taxes.</li>
                <li>Invoices are issued on the first business day each month, payable NET 30 by bank transfer, SEPA, ACH, or Stripe autopay. Late amounts accrue 1.5% interest per month or the maximum allowed by law.</li>
                <li>You are responsible for VAT, withholding, and other taxes linked to your use of the services. Provide valid tax IDs to avoid backup withholding.</li>
                <li>We may suspend access for non-payment after 10 days’ written notice. Suspensions do not relieve you of accrued fees.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">5. Data protection & confidentiality</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Our Data Processing Agreement (Article 28 GDPR) and Standard Contractual Clauses form part of these Terms. You remain the Data Controller; ApexMediation acts as Data Processor.</li>
                <li>We process personal data only according to your instructions, maintain SOC 2 Type II-aligned safeguards, and limit sub-processors to listed infrastructure or networks you enable.</li>
                <li>Each party must protect the other’s confidential information with care equal to its own, and at least reasonable care, and only use it for purposes of this agreement.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">6. Intellectual property</h2>
              <p className="text-sm text-gray-700">
                Bel Consulting OÜ retains all rights, title, and interest in the platform, SDKs, documentation, and derivative works. You retain ownership of your apps, creatives, data, and network credentials. We may use aggregated, anonymised analytics to improve the service provided they cannot identify you or your end users.
              </p>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">7. Warranties & disclaimers</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>We warrant that we will provide the services in a professional manner consistent with industry standards and that we hold all rights necessary to grant the licenses herein.</li>
                <li>You warrant that you have obtained all necessary rights, consents, and licenses to use your content and to direct us to process end-user data.</li>
                <li>Except as expressly stated, the services are provided “as is” without other warranties, including implied warranties of merchantability, fitness for a particular purpose, or non-infringement.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">8. Indemnity</h2>
              <p className="text-sm text-gray-700">
                Each party will indemnify, defend, and hold the other harmless against third-party claims alleging infringement or violation of law arising from the indemnifying party’s content or misuse of the services, provided the indemnified party promptly notifies the other and allows control of the defence.
              </p>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">9. Limitation of liability</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Total liability for all claims arising out of these Terms is limited to the fees you paid during the twelve (12) months immediately preceding the claim.</li>
                <li>Neither party is liable for indirect, incidental, consequential, or punitive damages, loss of profit, or business interruption, even if advised of the possibility.</li>
                <li>These caps do not apply to payment obligations, confidentiality breaches, or indemnification duties.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">10. Term, suspension & termination</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>These Terms remain in effect until terminated. Either party may terminate for convenience with 30 days’ written notice.</li>
                <li>We may suspend or terminate immediately for material breach, non-payment, legal requirement, or security risk. We will provide prompt notice with remediation steps when practical.</li>
                <li>Upon termination we will disable access, delete personal data within 30 days (unless law requires longer retention), and make reasonable efforts to assist with data export.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">11. Governing law & disputes</h2>
              <p className="text-sm text-gray-700">
                These Terms are governed by the laws of the Republic of Estonia without regard to conflict-of-law rules. Parties submit to the exclusive jurisdiction of the Harju County Court in Tallinn. We are open to good-faith mediation prior to formal proceedings.
              </p>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-3">
              <h2 className="text-h3 font-bold uppercase">12. Contact</h2>
              <p className="text-sm text-gray-700">
                Questions about these Terms can be sent to <a href="mailto:legal@apexmediation.ee" className="underline">legal@apexmediation.ee</a> or Bel Consulting OÜ, Sepapaja 6, Tallinn 15551, Estonia. Notices are deemed delivered when emailed to your designated billing contact or posted in the dashboard inbox.
              </p>
            </section>
          </div>
        </PageLayout>
      );

    case 'gdpr':
      return (
        <PageLayout
          title="GDPR"
          intro="Directly summarised from the Customer-Facing GDPR Compliance Guide so your legal, product, and finance teams have a single source of truth."
          heroTag="Compliance"
          breadcrumbs={breadcrumbs}
        >
          <div className="space-y-6">
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-4">
              <h2 className="text-h3 font-bold uppercase">Roles & agreements</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>You are the Data Controller; ApexMediation is the Data Processor.</li>
                <li>Execute the Data Processing Agreement (Article 28 GDPR) plus Standard Contractual Clauses directly inside <Link href="/dashboard/settings?tab=security" className="font-bold underline">Settings → Security</Link>.</li>
                <li>Sub-processors are limited to audited infrastructure vendors and the ad networks you explicitly enable.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-4">
              <h2 className="text-h3 font-bold uppercase">Consent & lawful basis</h2>
              <p className="text-sm text-gray-700">
                The SDK exposes helpers to detect when consent is required (IAB TCF) and to store consent objects for up to 13 months. Personalised ads require consent; fraud detection, security, and non-personalised ads operate under legitimate interest. Provide clear privacy notices, separate toggles per purpose, and make opt-out as easy as opt-in.
              </p>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-4">
              <h2 className="text-h3 font-bold uppercase">Data subject requests</h2>
              <p className="text-sm text-gray-700">
                Fulfil Right of Access, Deletion, Rectification, Portability, and Objection within 30 days using the documented APIs:
              </p>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li><code className="rounded bg-gray-100 px-1 py-0.5">POST /v1/gdpr/access</code> – returns JSON or CSV bundles for a device/user identifier.</li>
                <li><code className="rounded bg-gray-100 px-1 py-0.5">POST /v1/gdpr/delete</code> – queues deletion with confirmation when data is purged from hot and cold storage.</li>
                <li>Automations ensure standard requests finish in minutes; complex batches complete well within the 30-day SLA.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-4">
              <h2 className="text-h3 font-bold uppercase">Security & retention</h2>
              <ul className="list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Encryption in transit (TLS 1.3) and at rest (AES-256) with SOC 2 Type II-aligned controls.</li>
                <li>Inactive user data automatically deletes after 90 days; account-level data deletes 30 days post-closure.</li>
                <li>Breaches are reported to you within 24 hours and to authorities within 72 hours when required.</li>
              </ul>
            </section>

            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200 space-y-4">
              <h2 className="text-h3 font-bold uppercase">Need help?</h2>
              <p className="text-sm text-gray-700">
                Contact <a href="mailto:security@apexmediation.ee" className="underline">security@apexmediation.ee</a> for privacy questions or <a href="mailto:support@apexmediation.ee" className="underline">support@apexmediation.ee</a> for integration help. Sample consent copy, DPIA templates, and audit checklists live inside the Customer-Facing GDPR guide.
              </p>
            </section>
          </div>
        </PageLayout>
      );

    case 'security':
      return (
        <PageLayout
          title="Security"
          intro="SOC 2 Type II-aligned controls, quarterly Cure53 penetration tests, and redaction-first telemetry—all documented in the Security playbooks."
          heroTag="Trust"
          breadcrumbs={breadcrumbs}
        >
          <div className="space-y-6">
            <section className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Controls</h2>
              <ul className="mt-4 list-disc space-y-2 pl-6 text-sm text-gray-700">
                <li>Encryption at rest (AES-256) and in transit (TLS 1.3) across EU regions.</li>
                <li>Quarterly penetration tests by Cure53 plus continuous dependency scanning.</li>
                <li>Role-based access, hardware security keys, and background checks for production access.</li>
                <li>Telemetry automatically redacts consent strings, credentials, and user identifiers before export.</li>
              </ul>
            </section>
            <section className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Data handling</h2>
              <ul className="mt-4 list-disc space-y-2 pl-6 text-sm text-gray-700">
                <li>Inactive user data deleted after 90 days; account data deleted 30 days after closure.</li>
                <li>SOC 2 Type II-aligned policies cover incident response, logging, and vendor reviews.</li>
                <li>24-hour breach notification commitment to customers and 72-hour regulator timeline compliance.</li>
              </ul>
            </section>
            <section className="rounded-3xl bg-white p-6 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Report an issue</h2>
              <p className="mt-3 text-sm text-gray-700">
                Email <a href="mailto:security@apexmediation.ee" className="underline">security@apexmediation.ee</a> or open a ticket through the dashboard. We run a responsible disclosure program with cash rewards and publish RCAs within 48 hours of closing incidents.
              </p>
            </section>
          </div>
        </PageLayout>
      );

    case 'cookies':
      return (
        <PageLayout
          title="Cookie Policy"
          intro="Cookie usage mirrors the Privacy and GDPR guides—only essential auth/session cookies are required."
          heroTag="Legal"
          breadcrumbs={breadcrumbs}
        >
          <div className="space-y-6">
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Essential cookies</h2>
              <p className="mt-3 text-sm text-gray-700">
                Required for sign-in sessions, CSRF protection, and consent storage. Names include <code className="rounded bg-gray-100 px-1 py-0.5">apex_session</code>, <code className="rounded bg-gray-100 px-1 py-0.5">apex_locale</code>, and <code className="rounded bg-gray-100 px-1 py-0.5">apex_consent</code>. Expire within 30 days or immediately when you sign out.
              </p>
            </section>
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Optional analytics</h2>
              <p className="mt-3 text-sm text-gray-700">
                We offer anonymised product analytics to improve documentation and onboarding flows. These cookies are disabled by default in the EEA until you opt in through the consent banner. Opt-out at any time via the “Cookie settings” link in the footer.
              </p>
            </section>
            <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
              <h2 className="text-h3 font-bold uppercase">Managing preferences</h2>
              <ul className="mt-4 list-disc space-y-3 pl-6 text-sm text-gray-700">
                <li>Update or withdraw consent from the banner or by visiting <Link href="/cookie-settings" className="font-bold underline">Cookie Settings</Link>.</li>
                <li>Essential cookies can only be removed by clearing your browser storage or closing the account.</li>
                <li>Contact <a href="mailto:security@apexmediation.ee" className="underline">security@apexmediation.ee</a> for detailed cookie inventories or vendor agreements.</li>
              </ul>
            </section>
          </div>
        </PageLayout>
      );

    default:
      return null;
  }
}
