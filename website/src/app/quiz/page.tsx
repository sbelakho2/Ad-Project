'use client';

import Link from 'next/link';
import { useMemo, useState } from 'react';

type Option = {
  label: string;
  helper: string;
  score: number;
};

type Question = {
  id: string;
  prompt: string;
  options: Option[];
};

const QUESTIONS: Question[] = [
  {
    id: 'audience-size',
    prompt: 'How many monthly active users do you currently monetise?',
    options: [
      { label: 'Under 50,000', helper: 'Early stage studio or soft launch.', score: 1 },
      { label: '50,000 – 500,000', helper: 'Growing traction and testing new geos.', score: 2 },
      { label: '500,000 – 2 million', helper: 'Established title with diverse demand sources.', score: 3 },
      { label: 'More than 2 million', helper: 'Global franchise with live operations.', score: 4 },
    ],
  },
  {
    id: 'team-size',
    prompt: 'How big is the team responsible for ad monetisation?',
    options: [
      { label: 'It is just me', helper: 'You wear every hat and need automation.', score: 1 },
      { label: 'Small team (2-3 people)', helper: 'Tight feedback loops and direct experiments.', score: 2 },
      { label: 'Squad (4-8 people)', helper: 'Specialists for UA, product, and monetisation.', score: 3 },
      { label: 'Multiple squads / shared service', helper: 'Enterprise process with layered approvals.', score: 4 },
    ],
  },
  {
    id: 'tooling',
    prompt: 'How would you describe your current tooling?',
    options: [
      { label: 'Manual spreadsheets and basic reports', helper: 'You copy data manually into sheets.', score: 1 },
      { label: 'Waterfall mediation with limited automation', helper: 'Rules-based optimisation, few integrations.', score: 2 },
      { label: 'Hybrid mediation with some custom pipelines', helper: 'Mix of partners and internal data warehouse.', score: 3 },
      { label: 'Fully automated auctions with BI integrations', helper: 'Programmatic workflows and bespoke bidding.', score: 4 },
    ],
  },
  {
    id: 'goals',
    prompt: 'Primary goal for the next quarter?',
    options: [
      { label: 'Launch monetisation for a new title', helper: 'Need quick wins and simple setup.', score: 1 },
      { label: 'Keep revenue per user predictable', helper: 'Optimise placements and pricing.', score: 2 },
      { label: 'Expand to new regions and formats', helper: 'More networks, more segmentation.', score: 3 },
      { label: 'Operational excellence & automation', helper: 'SLA-driven, compliance, and advanced controls.', score: 4 },
    ],
  },
  {
    id: 'support',
    prompt: 'What level of support do you require?',
    options: [
      { label: 'Self-serve documentation is fine', helper: 'You will explore and experiment solo.', score: 1 },
      { label: 'Occasional check-ins with specialists', helper: 'Monthly guidance to stay on track.', score: 2 },
      { label: 'Dedicated success manager', helper: 'Weekly strategy sessions and playbooks.', score: 3 },
      { label: '24/7 coverage & shared OKRs', helper: 'You expect enterprise-grade SLAs.', score: 4 },
    ],
  },
];

const RESULTS = [
  {
    minScore: 1,
    maxScore: 1.9,
    label: 'Starter Fast Track',
    summary: 'Perfect for teams spinning up monetisation. Use our step-by-step integration guides and pay nothing until revenue flows—then just a simple marginal fee.',
    cta: { label: 'Read the integration checklist', href: '/docs' },
  },
  {
    minScore: 2,
    maxScore: 2.9,
    label: 'Growth + Guidance',
    summary: 'Ready to scale with scheduled check-ins on fraud prevention, A/B experiments, and revenue reconciliation. We tailor onboarding workshops to your team\'s needs.',
    cta: { label: 'See pricing', href: '/pricing' },
  },
  {
    minScore: 3,
    maxScore: 4,
    label: 'Enterprise Track',
    summary: 'You need automation, custom workflows, and contractual SLAs. Our platform supports all of this—let us map the fee schedule to your approval process and configure tooling with you.',
    cta: { label: 'Talk to our enterprise specialists', href: 'mailto:sales@apexmediation.ee' },
  },
];

export default function QuizPage() {
  const [currentIndex, setCurrentIndex] = useState(0);
  const [answers, setAnswers] = useState<number[]>([]);
  const [complete, setComplete] = useState(false);

  const currentQuestion = QUESTIONS[currentIndex];

  const averageScore = useMemo(() => {
    if (!complete || answers.length === 0) return 0;
    const total = answers.reduce((sum, value) => sum + value, 0);
    return total / answers.length;
  }, [answers, complete]);

  const result = useMemo(() => {
    if (!complete) return null;
    return RESULTS.find((item) => averageScore >= item.minScore && averageScore <= item.maxScore) ?? RESULTS[1];
  }, [averageScore, complete]);

  const handleSelect = (option: Option) => {
    const nextAnswers = [...answers];
    nextAnswers[currentIndex] = option.score;
    setAnswers(nextAnswers);

    if (currentIndex === QUESTIONS.length - 1) {
      setComplete(true);
    } else {
      setCurrentIndex(currentIndex + 1);
    }
  };

  const handleRestart = () => {
    setAnswers([]);
    setCurrentIndex(0);
    setComplete(false);
  };

  return (
    <main className="min-h-screen bg-brand-600 text-white">
      <div className="container mx-auto max-w-3xl px-4 py-16 space-y-10">
        <header className="space-y-4 text-center">
          <span className="inline-flex items-center rounded-full bg-brand-100 px-3 py-1 text-xs font-bold uppercase tracking-[0.2em] text-brand-700">
            Interactive Quiz
          </span>
          <h1 className="text-h2-sm md:text-h2-md font-bold uppercase text-white">
            What is your Ad Monetisation IQ?
          </h1>
          <p className="text-body text-white/90">
            Answer five quick questions to discover which ApexMediation plan fits your studio and unlock tailored resources.
          </p>
        </header>

        <section className="rounded-3xl bg-white p-8 text-gray-900 shadow-xl ring-1 ring-gray-200">
          <div className="mb-6 flex items-center justify-between text-sm font-bold uppercase text-gray-600">
            <span>Question {complete ? QUESTIONS.length : currentIndex + 1} of {QUESTIONS.length}</span>
            <span>{Math.round(((complete ? QUESTIONS.length : currentIndex + 1) / QUESTIONS.length) * 100)}% complete</span>
          </div>
          <div className="mb-6 h-2 w-full rounded-full bg-gray-200">
            <div
              className="h-full rounded-full bg-brand-500 transition-all"
              style={{ width: `${((complete ? QUESTIONS.length : currentIndex + 1) / QUESTIONS.length) * 100}%` }}
            />
          </div>

          {!complete && currentQuestion && (
            <div className="space-y-6">
              <h2 className="text-h3 font-bold uppercase text-gray-900">{currentQuestion.prompt}</h2>
              <div className="space-y-4">
                {currentQuestion.options.map((option) => (
                  <button
                    key={option.label}
                    type="button"
                    onClick={() => handleSelect(option)}
                    className="w-full rounded-2xl border border-gray-300 bg-white px-5 py-4 text-left transition hover:border-brand-500 hover:shadow-lg focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--focus)]"
                  >
                    <span className="text-sm font-bold uppercase text-gray-900">{option.label}</span>
                    <p className="mt-1 text-sm text-gray-600">{option.helper}</p>
                  </button>
                ))}
              </div>
            </div>
          )}

          {complete && result && (
            <div className="space-y-5 text-center">
              <h2 className="text-h2-sm font-bold uppercase text-gray-900">{result.label}</h2>
              <p className="text-body text-gray-700 leading-relaxed">{result.summary}</p>
              <Link
                href={result.cta.href}
                className="inline-flex items-center justify-center btn-primary px-6 py-3 text-sm font-bold uppercase"
              >
                {result.cta.label}
              </Link>
              <button
                type="button"
                onClick={handleRestart}
                className="block w-full btn-outline px-6 py-3 text-sm font-bold uppercase"
              >
                Retake quiz
              </button>
            </div>
          )}
        </section>

        <footer className="space-y-4 text-center text-sm text-white/80">
          <p>
            Ready to put these insights into action? Explore our <Link href="/docs" className="font-bold underline">documentation</Link> or review the <Link href="/pricing" className="font-bold underline">pricing guide</Link>.
          </p>
        </footer>
      </div>
    </main>
  );
}
