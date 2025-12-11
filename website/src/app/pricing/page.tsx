import Link from 'next/link';
import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Pricing & BYO Platform Tiers | ApexMediation',
  description:
    'Starter 0% up to $10k per app per month, Growth 2.5%, Scale 2% down to 1.25% with annual prepay, Enterprise custom. BYO only.',
  alternates: {
    canonical: 'https://apexmediation.ee/pricing',
  },
};

const tiers = [
  {
    name: 'Starter',
    headline: '0% platform fee',
    description: 'Free up to $10k per app per month. Built for BYO only.',
    bullets: [
      'No card or bank needed to explore; autopay rail stored only when you opt-in.',
      'Includes SDKs, Migration Studio, and basic revenue/latency dashboards.',
      'Transparency auctions, VRA reconciliation, fraud dashboards, and A/B tools are locked until you upgrade.'
    ],
    badge: 'BYO only',
  },
  {
    name: 'Growth',
    headline: '2.5% platform fee',
    description: 'For teams leaning on experiments, support, and live QA.',
    bullets: [
      'Autopay with Stripe cards, ACH (US), or SEPA (EU/EEA).',
      'Transparency auctions + VRA reconciliation exports, fraud/quality dashboards, and A/B testing.',
      'You still own customer contracts and payments end-to-end.'
    ],
    badge: 'BYO only',
  },
  {
    name: 'Scale',
    headline: '2% → 1.25% with annual prepay',
    description: 'Volume efficiency plus enterprise change control.',
    bullets: [
      'Volume pricing starts at 2%; annual prepay unlocks 1.25%.',
      'Layered approvals, RBAC, BI/export pipelines, and observability tuned for ops.',
      'Zero custody: your ad network payouts stay direct to you.'
    ],
    badge: 'BYO only',
  },
  {
    name: 'Enterprise',
    headline: 'Custom rate card',
    description: 'Contracted SLAs, finance-reviewed rails, and bespoke workflows.',
    bullets: [
      'Invoice + wire after finance review; autopay remains the default.',
      'Joint security, privacy, and observability readiness with your teams.',
      'BYO adapters and contracts stay yours; Apex only bills the platform fee.'
    ],
    badge: 'BYO only',
  },
];

export default function PricingPage() {
  return (
    <main className="bg-gradient-to-b from-[#f3f6ff] via-white to-[#f5f7fb] min-h-screen">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-12 px-4 pb-16 pt-14 sm:px-6 lg:px-8">
        <header className="space-y-4 text-center">
          <p className="text-xs font-semibold uppercase tracking-[0.35em] text-slate-500">BYO Pricing</p>
          <h1 className="text-4xl font-semibold text-gray-900 sm:text-5xl">One pricing model. BYO only.</h1>
          <p className="mx-auto max-w-3xl text-base leading-relaxed text-gray-600 sm:text-lg">
            Your business runs on these facts: Starter: free up to $10k per app per month (built for BYO only). When you opt into Growth or cross the cap, invoices auto-charge via Stripe. Growth is 2.5% platform fee (BYO only). Scale is 2% down to 1.25% with annual prepay (BYO only). Enterprise is custom rate card for BYO only.
          </p>
        </header>

        <section className="grid gap-6 md:grid-cols-2">
          {tiers.map((tier) => (
            <div key={tier.name} className="rounded-2xl bg-white p-6 shadow-lg ring-1 ring-gray-100">
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-xs font-semibold uppercase tracking-wide text-primary-600">{tier.badge}</p>
                  <h2 className="mt-1 text-2xl font-bold text-gray-900">{tier.name}</h2>
                  <p className="text-lg text-gray-700">{tier.headline}</p>
                </div>
              </div>
              <p className="mt-3 text-sm text-gray-600">{tier.description}</p>
              <ul className="mt-4 space-y-2 text-sm text-gray-700 list-disc list-inside">
                {tier.bullets.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
              <div className="mt-6 flex flex-wrap gap-3">
                <Link
                  href="/signup"
                  className="inline-flex items-center justify-center rounded-lg bg-primary-600 px-4 py-2 text-sm font-semibold text-white shadow-sm transition hover:bg-primary-700"
                >
                  Start with {tier.name}
                </Link>
                <Link
                  href="mailto:sales@apexmediation.ee"
                  className="inline-flex items-center justify-center rounded-lg border border-gray-200 px-4 py-2 text-sm font-semibold text-gray-800 hover:bg-gray-50"
                >
                  Talk with us
                </Link>
              </div>
            </div>
          ))}
        </section>

        <section className="grid gap-6 lg:grid-cols-2">
          <div className="rounded-2xl bg-white p-6 shadow-lg ring-1 ring-gray-100">
            <h3 className="text-lg font-semibold text-gray-900">What stays yours</h3>
            <p className="mt-2 text-sm text-gray-700">
              You own your customer and payment with the customer. You never touch publisher contracts or payments—BYO demand, credentials, and payouts remain yours. ApexMediation only bills the platform fee.
            </p>
            <p className="mt-3 text-sm text-gray-700">Starter keeps you in basic dashboards; upgrade to Growth for transparency auctions, VRA exports, fraud dashboards, and experiments. We store autopay rails encrypted and use them only when Stripe finalizes a post-Starter invoice. Enterprise teams can request invoice + wire after finance review; autopay stays the default to keep mediation live.</p>
          </div>
          <div className="rounded-2xl bg-white p-6 shadow-lg ring-1 ring-gray-100">
            <h3 className="text-lg font-semibold text-gray-900">How charging works</h3>
            <ul className="mt-3 space-y-2 text-sm text-gray-700 list-disc list-inside">
              <li>Starter: free up to $10k per app per month (BYO only). Opt into Growth any time.</li>
              <li>Growth: 2.5% platform fee (BYO only) with Stripe autopay at invoice finalization.</li>
              <li>Scale: 2% list, trending to 1.25% with annual prepay and volume (BYO only).</li>
              <li>Enterprise: custom rate card with layered approvals and finance-reviewed rails (BYO only).</li>
            </ul>
            <p className="mt-3 text-xs text-gray-500">Need a contract mapped before going live? <Link href="mailto:sales@apexmediation.ee" className="underline text-primary-600">Email sales</Link> and we will align your paper with the BYO fee schedule.</p>
          </div>
        </section>
      </div>
    </main>
  );
}
