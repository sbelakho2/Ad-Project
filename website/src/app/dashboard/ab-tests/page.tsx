'use client';

// Reference: Design.md § "Dashboard Pages" & WEBSITE_DESIGN.md § "A/B Tests Page"
// A/B testing platform with Bayesian statistics and Thompson Sampling

import {
    BeakerIcon,
    CheckCircleIcon,
    ClockIcon,
    PlayIcon,
    StopIcon,
} from '@heroicons/react/24/outline';
import { useState } from 'react';

interface ABTest {
  id: string;
  name: string;
  status: 'running' | 'completed' | 'draft' | 'paused';
  startDate: string;
  endDate?: string;
  variantA: {
    name: string;
    impressions: number;
    revenue: number;
    ecpm: number;
  };
  variantB: {
    name: string;
    impressions: number;
    revenue: number;
    ecpm: number;
  };
  confidence: number;
  winner?: 'A' | 'B' | null;
}

export default function ABTestsPage() {
  const [tests] = useState<ABTest[]>([
    {
      id: '1',
      name: 'Banner Position Test',
      status: 'running',
      startDate: '2025-11-01',
      variantA: {
        name: 'Top Banner',
        impressions: 45678,
        revenue: 634.89,
        ecpm: 13.90,
      },
      variantB: {
        name: 'Bottom Banner',
        impressions: 45234,
        revenue: 723.45,
        ecpm: 15.99,
      },
      confidence: 87.3,
      winner: 'B',
    },
    {
      id: '2',
      name: 'Rewarded Video Timing',
      status: 'running',
      startDate: '2025-10-28',
      variantA: {
        name: 'Level Start',
        impressions: 23456,
        revenue: 456.78,
        ecpm: 19.47,
      },
      variantB: {
        name: 'Level End',
        impressions: 23890,
        revenue: 512.34,
        ecpm: 21.45,
      },
      confidence: 92.1,
      winner: 'B',
    },
    {
      id: '3',
      name: 'Interstitial Frequency',
      status: 'completed',
      startDate: '2025-10-15',
      endDate: '2025-10-30',
      variantA: {
        name: 'Every 3 levels',
        impressions: 89234,
        revenue: 1234.56,
        ecpm: 13.83,
      },
      variantB: {
        name: 'Every 5 levels',
        impressions: 88901,
        revenue: 1189.23,
        ecpm: 13.38,
      },
      confidence: 95.8,
      winner: 'A',
    },
    {
      id: '4',
      name: 'Ad Network Priority',
      status: 'paused',
      startDate: '2025-10-20',
      variantA: {
        name: 'AdMob First',
        impressions: 12345,
        revenue: 178.90,
        ecpm: 14.49,
      },
      variantB: {
        name: 'Meta First',
        impressions: 12098,
        revenue: 167.34,
        ecpm: 13.83,
      },
      confidence: 45.2,
      winner: null,
    },
  ]);

  const runningTests = tests.filter((t) => t.status === 'running').length;
  const completedTests = tests.filter((t) => t.status === 'completed').length;

  return (
    <div className="p-6 space-y-6">
      {/* Page Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-h2-sm font-bold uppercase text-gray-900 tracking-tight">
            A/B Testing Platform
          </h1>
          <p className="text-sm text-gray-600 mt-1">
            Optimize your ad strategy with Bayesian statistics & Thompson Sampling
          </p>
        </div>
        <button className="btn-primary px-6 py-3 flex items-center gap-2">
          <BeakerIcon className="w-5 h-5" />
          Create New Test
        </button>
      </div>

      {/* Summary Cards */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-6">
        <div className="card-v2 p-6">
          <div className="flex items-center gap-3 mb-2">
            <PlayIcon className="w-6 h-6 text-brand-600" />
            <p className="text-brand-700 font-semibold text-sm">Running Tests</p>
          </div>
          <p className="text-3xl font-semibold text-gray-900">{runningTests}</p>
        </div>
        <div className="card-v2 p-6">
          <div className="flex items-center gap-3 mb-2">
            <CheckCircleIcon className="w-6 h-6 text-success" />
            <p className="text-gray-900 font-semibold text-sm">Completed</p>
          </div>
          <p className="text-3xl font-semibold text-gray-900">{completedTests}</p>
        </div>
        <div className="card-v2 p-6">
          <div className="flex items-center gap-3 mb-2">
            <BeakerIcon className="w-6 h-6 text-brand-600" />
            <p className="text-gray-900 font-semibold text-sm">Avg Lift</p>
          </div>
          <p className="text-3xl font-semibold text-gray-900">+14.2%</p>
        </div>
        <div className="card-v2 p-6">
          <div className="flex items-center gap-3 mb-2">
            <ClockIcon className="w-6 h-6 text-gray-500" />
            <p className="text-gray-900 font-semibold text-sm">Avg Duration</p>
          </div>
          <p className="text-3xl font-semibold text-gray-900">12 days</p>
        </div>
      </div>

      {/* Tests List */}
      <div className="space-y-6">
        {tests.map((test) => (
          <ABTestCard key={test.id} test={test} />
        ))}
      </div>

      {/* Statistical Method Info */}
      <div className="card-v2 p-6">
        <h2 className="text-gray-900 font-semibold text-lg mb-4">
          How Our A/B Testing Works
        </h2>
        <div className="grid md:grid-cols-2 gap-6 text-gray-700">
          <div>
            <h3 className="font-semibold text-gray-900 mb-2">Bayesian Statistics</h3>
            <p className="text-sm leading-relaxed mb-4">
              Unlike frequentist methods, our Bayesian approach provides:
            </p>
            <ul className="space-y-2 text-sm">
              <li className="flex items-start gap-2">
                <span className="text-brand-600">•</span>
                <span>Real-time probability of winning (not just p-values)</span>
              </li>
              <li className="flex items-start gap-2">
                <span className="text-brand-600">•</span>
                <span>Continuous learning as data accumulates</span>
              </li>
              <li className="flex items-start gap-2">
                <span className="text-brand-600">•</span>
                <span>No need for pre-defined sample sizes</span>
              </li>
              <li className="flex items-start gap-2">
                <span className="text-brand-600">•</span>
                <span>Confidence intervals that actually make sense</span>
              </li>
            </ul>
          </div>
          <div>
            <h3 className="font-semibold text-gray-900 mb-2">Thompson Sampling</h3>
            <p className="text-sm leading-relaxed mb-4">
              Our multi-armed bandit algorithm automatically:
            </p>
            <ul className="space-y-2 text-sm">
              <li className="flex items-start gap-2">
                <span className="text-brand-600">•</span>
                <span>Allocates more traffic to winning variants</span>
              </li>
              <li className="flex items-start gap-2">
                <span className="text-brand-600">•</span>
                <span>Minimizes opportunity cost during testing</span>
              </li>
              <li className="flex items-start gap-2">
                <span className="text-brand-600">•</span>
                <span>Balances exploration vs exploitation</span>
              </li>
              <li className="flex items-start gap-2">
                <span className="text-brand-600">•</span>
                <span>Reaches conclusions 30% faster than traditional A/B tests</span>
              </li>
            </ul>
          </div>
        </div>
      </div>
    </div>
  );
}

interface ABTestCardProps {
  test: ABTest;
}

function ABTestCard({ test }: ABTestCardProps) {
  const statusConfig = {
    running: { color: 'border-success/20 bg-success/10', badge: 'bg-success', icon: PlayIcon },
    completed: { color: 'border-info/20 bg-info/10', badge: 'bg-info', icon: CheckCircleIcon },
    draft: { color: 'border-gray-400 bg-gray-50', badge: 'bg-gray-400', icon: ClockIcon },
    paused: { color: 'border-warning/20 bg-warning/10', badge: 'bg-warning', icon: StopIcon },
  };

  const config = statusConfig[test.status];
  const StatusIcon = config.icon;

  const liftB = ((test.variantB.ecpm - test.variantA.ecpm) / test.variantA.ecpm) * 100;

  return (
    <div className="card p-6">
      {/* Header */}
      <div className="flex items-start justify-between mb-6">
        <div className="flex-1">
          <h3 className="text-xl font-bold text-gray-900 mb-2">{test.name}</h3>
          <div className="flex items-center gap-4 text-sm text-gray-600">
            <span>Started: {test.startDate}</span>
            {test.endDate && <span>• Ended: {test.endDate}</span>}
          </div>
        </div>
        <div className="flex items-center gap-3">
          <div className={`${config.badge} text-white px-4 py-2 rounded flex items-center gap-2`}>
            <StatusIcon className="w-4 h-4" />
            <span className="text-sm font-bold uppercase">{test.status}</span>
          </div>
        </div>
      </div>

      {/* Variants Comparison */}
      <div className="grid md:grid-cols-2 gap-6 mb-6">
        {/* Variant A */}
        <div className={`border-2 rounded p-4 ${test.winner === 'A' ? 'border-brand-500 bg-brand-50' : 'border-gray-300'}`}>
          <div className="flex items-center justify-between mb-4">
            <h4 className="font-bold text-gray-900">Variant A: {test.variantA.name}</h4>
            {test.winner === 'A' && (
              <div className="bg-brand-100 text-brand-700 px-3 py-1 rounded text-xs font-bold border" style={{ borderColor: 'rgb(var(--brand-500))' }}>
                WINNER
              </div>
            )}
          </div>
          <div className="space-y-2 text-sm">
            <div className="flex justify-between">
              <span className="text-gray-600">Impressions:</span>
              <span className="font-bold text-gray-900">{test.variantA.impressions.toLocaleString()}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-600">Revenue:</span>
              <span className="font-bold text-gray-900">${test.variantA.revenue.toFixed(2)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-600">eCPM:</span>
              <span className="font-bold text-gray-900">${test.variantA.ecpm.toFixed(2)}</span>
            </div>
          </div>
        </div>

        {/* Variant B */}
        <div className={`border-2 rounded p-4 ${test.winner === 'B' ? 'border-brand-500 bg-brand-50' : 'border-gray-300'}`}>
          <div className="flex items-center justify-between mb-4">
            <h4 className="font-bold text-gray-900">Variant B: {test.variantB.name}</h4>
            {test.winner === 'B' && (
              <div className="bg-brand-100 text-brand-700 px-3 py-1 rounded text-xs font-bold border" style={{ borderColor: 'rgb(var(--brand-500))' }}>
                WINNER
              </div>
            )}
          </div>
          <div className="space-y-2 text-sm">
            <div className="flex justify-between">
              <span className="text-gray-600">Impressions:</span>
              <span className="font-bold text-gray-900">{test.variantB.impressions.toLocaleString()}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-600">Revenue:</span>
              <span className="font-bold text-gray-900">${test.variantB.revenue.toFixed(2)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-600">eCPM:</span>
              <span className="font-bold text-gray-900">${test.variantB.ecpm.toFixed(2)}</span>
            </div>
          </div>
        </div>
      </div>

      {/* Statistical Results */}
      <div className="bg-gray-50 p-4 rounded">
        <div className="flex items-center justify-between mb-3">
          <div>
            <p className="text-sm text-gray-600 mb-1">Statistical Confidence</p>
            <p className="text-2xl font-bold text-gray-900">{test.confidence}%</p>
          </div>
          <div className="text-right">
            <p className="text-sm text-gray-600 mb-1">Variant B Lift</p>
            <p className={`text-2xl font-bold ${liftB > 0 ? 'text-success' : 'text-danger'}`}>
              {liftB > 0 ? '+' : ''}{liftB.toFixed(1)}%
            </p>
          </div>
        </div>
        <div className="w-full bg-gray-300 h-3 rounded-full overflow-hidden">
          <div className="bg-brand-600 h-full" style={{ width: `${test.confidence}%` }} />
        </div>
        <p className="text-xs text-gray-600 mt-2">
          {test.confidence >= 95 ? '✓ Test has reached statistical significance' :
           test.confidence >= 80 ? '⚠ Test is approaching significance' :
           '⏳ Test needs more data to reach significance'}
        </p>
      </div>

      {/* Actions */}
      <div className="flex items-center justify-end gap-3 mt-4 pt-4 border-t border-gray-200">
        {test.status === 'running' && (
          <>
            <button className="btn-secondary text-sm">
              Pause Test
            </button>
            <button className="btn-primary text-sm">
              Stop & Choose Winner
            </button>
          </>
        )}
        {test.status === 'completed' && (
          <button className="btn-outline px-4 py-2 text-sm">
            View Full Report
          </button>
        )}
        {test.status === 'paused' && (
          <>
            <button className="px-4 py-2 text-sm font-bold text-white bg-success rounded hover:opacity-90">
              Resume Test
            </button>
            <button className="px-4 py-2 text-sm font-bold text-danger border border-danger rounded hover:bg-danger/10">
              Delete Test
            </button>
          </>
        )}
      </div>
    </div>
  );
}
