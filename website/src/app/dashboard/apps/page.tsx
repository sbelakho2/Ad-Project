'use client';

// Reference: Design.md § "Dashboard Pages" & WEBSITE_DESIGN.md § "Apps Page"
// Apps management page with SDK integration status and performance metrics

import { CheckCircleIcon, ExclamationTriangleIcon, PlusCircleIcon } from '@heroicons/react/24/outline';
import { useState } from 'react';
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

interface App {
  id: string;
  name: string;
  platform: 'iOS' | 'Android' | 'Unity' | 'Web';
  status: 'active' | 'paused' | 'error';
  sdkVersion: string;
  dailyRevenue: number;
  dailyImpressions: number;
  ecpm: number;
  users: number;
  lastUpdate: string;
}

export default function AppsPage() {
  const [apps, setApps] = useState<App[]>([
    {
      id: '1',
      name: 'Puzzle Quest Pro',
      platform: 'iOS',
      status: 'active',
      sdkVersion: '4.2.1',
      dailyRevenue: 1247.32,
      dailyImpressions: 89432,
      ecpm: 13.95,
      users: 45678,
      lastUpdate: '5 minutes ago',
    },
    {
      id: '2',
      name: 'Puzzle Quest Pro',
      platform: 'Android',
      status: 'active',
      sdkVersion: '4.2.1',
      dailyRevenue: 1893.45,
      dailyImpressions: 123456,
      ecpm: 15.34,
      users: 78901,
      lastUpdate: '3 minutes ago',
    },
    {
      id: '3',
      name: 'Racing Thunder',
      platform: 'Unity',
      status: 'active',
      sdkVersion: '4.1.8',
      dailyRevenue: 834.21,
      dailyImpressions: 67890,
      ecpm: 12.29,
      users: 34567,
      lastUpdate: '12 minutes ago',
    },
    {
      id: '4',
      name: 'Word Master',
      platform: 'iOS',
      status: 'active',
      sdkVersion: '4.2.1',
      dailyRevenue: 623.78,
      dailyImpressions: 45678,
      ecpm: 13.66,
      users: 23456,
      lastUpdate: '8 minutes ago',
    },
    {
      id: '5',
      name: 'Casual Slots',
      platform: 'Web',
      status: 'paused',
      sdkVersion: '4.0.3',
      dailyRevenue: 0,
      dailyImpressions: 0,
      ecpm: 0,
      users: 12345,
      lastUpdate: '2 days ago',
    },
    {
      id: '6',
      name: 'Adventure RPG',
      platform: 'Android',
      status: 'error',
      sdkVersion: '3.9.2',
      dailyRevenue: 0,
      dailyImpressions: 0,
      ecpm: 0,
      users: 8901,
      lastUpdate: '6 hours ago',
    },
  ]);

  const [modalState, setModalState] = useState<
    | { type: null }
    | { type: 'add' }
    | { type: 'configure' | 'view-stats' | 'fix'; app: App }
  >({ type: null });

  const openModal = (type: 'add' | 'configure' | 'view-stats' | 'fix', app?: App) => {
    if (type === 'add') {
      setModalState({ type });
    } else if (app) {
      setModalState({ type, app });
    }
  };

  const closeModal = () => setModalState({ type: null });

  const handleResumeApp = (id: string) => {
    setApps((prev) =>
      prev.map((app) =>
        app.id === id
          ? {
              ...app,
              status: 'active',
              lastUpdate: 'Just now',
              dailyRevenue: app.dailyRevenue || 432.11,
              dailyImpressions: app.dailyImpressions || 21345,
              ecpm: app.ecpm || 12.45,
            }
          : app
      )
    );
  };

  const handleFixIntegration = (id: string) => {
    setApps((prev) =>
      prev.map((app) =>
        app.id === id
          ? {
              ...app,
              status: 'active',
              sdkVersion: '4.2.1',
              lastUpdate: 'Fixed just now',
              dailyRevenue: 512.34,
              dailyImpressions: 25456,
              ecpm: 14.11,
            }
          : app
      )
    );
  };

  const activeApps = apps.filter((a) => a.status === 'active').length;
  const totalRevenue = apps.reduce((sum, a) => sum + a.dailyRevenue, 0);
  const totalImpressions = apps.reduce((sum, a) => sum + a.dailyImpressions, 0);
  const totalUsers = apps.reduce((sum, a) => sum + a.users, 0);

  return (
    <Section>
      <Container className="space-y-6">
      {/* Page Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-h2-sm md:text-h2-md lg:text-h2 font-semibold text-gray-900">
            Apps Management
          </h1>
          <p className="text-sm text-gray-600 mt-1">
            Manage your apps, monitor performance, and configure SDK settings
          </p>
        </div>
        <button
          className="btn-primary px-6 py-3 flex items-center gap-2"
          onClick={() => openModal('add')}
        >
          <PlusCircleIcon className="w-5 h-5" />
          Add New App
        </button>
      </div>

      {/* Summary Cards */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-6">
        <div className="card-v2 p-6">
          <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">
            Active Apps
          </p>
          <p className="text-3xl font-semibold text-gray-900">{activeApps}</p>
          <p className="text-sm text-gray-600 mt-1">of {apps.length} total</p>
        </div>
        <div className="card-v2 p-6">
          <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">
            Daily Revenue
          </p>
          <p className="text-3xl font-semibold text-gray-900">${totalRevenue.toLocaleString()}</p>
          <p className="text-sm text-gray-600 mt-1">across all apps</p>
        </div>
        <div className="card-v2 p-6">
          <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">
            Daily Impressions
          </p>
          <p className="text-3xl font-semibold text-gray-900">{totalImpressions.toLocaleString()}</p>
          <p className="text-sm text-gray-600 mt-1">today</p>
        </div>
        <div className="card-v2 p-6">
          <p className="text-xs font-semibold uppercase tracking-wide text-brand-600 mb-2">
            Total Users
          </p>
          <p className="text-3xl font-semibold text-gray-900">{totalUsers.toLocaleString()}</p>
          <p className="text-sm text-gray-600 mt-1">daily active</p>
        </div>
      </div>

      {/* Apps Grid */}
      <div className="grid md:grid-cols-2 gap-6">
        {apps.map((app) => (
          <AppCard
            key={app.id}
            app={app}
            onConfigure={() => openModal('configure', app)}
            onViewStats={() => openModal('view-stats', app)}
            onResume={() => handleResumeApp(app.id)}
            onFixIntegration={() => {
              handleFixIntegration(app.id);
              openModal('fix', app);
            }}
          />
        ))}
      </div>

      {/* SDK Integration Guide */}
      <div className="card-v2 p-6">
        <h2 className="text-gray-900 font-bold uppercase text-lg mb-4 border-b-2 border-gray-200 pb-2">
          SDK Integration Guide
        </h2>
        <div className="grid md:grid-cols-3 gap-6">
          <div>
            <h3 className="font-bold text-gray-900 mb-3 flex items-center gap-2">
              <span className="text-2xl">📱</span>
              iOS & Android
            </h3>
            <ol className="space-y-2 text-sm text-gray-700">
              <li>1. Add SDK dependency to your project</li>
              <li>2. Initialize with your App ID</li>
              <li>3. Request ad placements</li>
              <li>4. Handle ad events</li>
              <li>5. Test with test mode enabled</li>
            </ol>
            <a href="/docs" className="text-brand-700 font-bold text-sm underline mt-3 inline-block">
              View Mobile Docs →
            </a>
          </div>
          <div>
            <h3 className="font-bold text-gray-900 mb-3 flex items-center gap-2">
              <span className="text-2xl">🎮</span>
              Unity
            </h3>
            <ol className="space-y-2 text-sm text-gray-700">
              <li>1. Import Unity package</li>
              <li>2. Configure ApexMediation settings</li>
              <li>3. Add ad prefabs to scenes</li>
              <li>4. Script ad loading logic</li>
              <li>5. Build and test on device</li>
            </ol>
            <a href="/docs" className="text-brand-700 font-bold text-sm underline mt-3 inline-block">
              View Unity Docs →
            </a>
          </div>
          <div>
            <h3 className="font-bold text-gray-900 mb-3 flex items-center gap-2">
              <span className="text-2xl">🌐</span>
              Web
            </h3>
            <ol className="space-y-2 text-sm text-gray-700">
              <li>1. Include SDK script tag</li>
              <li>2. Initialize with App ID</li>
              <li>3. Define ad containers</li>
              <li>4. Call displayAd() method</li>
              <li>5. Handle ad callbacks</li>
            </ol>
            <a href="/docs" className="text-brand-700 font-bold text-sm underline mt-3 inline-block">
              View Web Docs →
            </a>
          </div>
        </div>
      </div>

      {/* Latest SDK Version */}
      <div className="card-v2 p-6 bg-brand-600 text-white">
        <div className="flex items-start justify-between">
          <div>
            <h2 className="text-brand-100 font-bold uppercase text-lg mb-2">
              Latest SDK Version: 4.2.1
            </h2>
            <p className="text-white text-body mb-4">
              Released 2025-11-01 • Includes performance improvements and bug fixes
            </p>
            <ul className="space-y-2 text-sm text-white">
              <li className="flex items-start gap-2">
                <span className="text-brand-100">✓</span>
                <span>15% faster ad loading times</span>
              </li>
              <li className="flex items-start gap-2">
                <span className="text-brand-100">✓</span>
                <span>Improved memory management</span>
              </li>
              <li className="flex items-start gap-2">
                <span className="text-brand-100">✓</span>
                <span>Better error reporting</span>
              </li>
              <li className="flex items-start gap-2">
                <span className="text-brand-100">✓</span>
                <span>New rewarded video callbacks</span>
              </li>
            </ul>
          </div>
          <a
            href="/changelog"
            className="btn-primary px-6 py-2 text-sm whitespace-nowrap"
          >
            View Changelog
          </a>
        </div>
      </div>

      <AppModal state={modalState} onClose={closeModal} />
      </Container>
    </Section>
  );
}

interface AppCardProps {
  app: App;
  onConfigure: () => void;
  onViewStats: () => void;
  onResume: () => void;
  onFixIntegration: () => void;
}

function AppCard({ app, onConfigure, onViewStats, onResume, onFixIntegration }: AppCardProps) {
  const statusConfig = {
    active: {
      icon: CheckCircleIcon,
      color: 'text-success',
      bg: 'bg-success/10 text-success border-success/20',
      label: 'Active',
      border: 'border-gray-200',
    },
    paused: {
      icon: ExclamationTriangleIcon,
      color: 'text-warning',
      bg: 'bg-warning/10 text-warning border-warning/20',
      label: 'Paused',
      border: 'border-gray-300',
    },
    error: {
      icon: ExclamationTriangleIcon,
      color: 'text-danger',
      bg: 'bg-danger/10 text-danger border-danger/20',
      label: 'Error',
      border: 'border-danger/20',
    },
  };

  const platformIcons = {
    iOS: '🍎',
    Android: '🤖',
    Unity: '🎮',
    Web: '🌐',
  };

  const config = statusConfig[app.status];
  const StatusIcon = config.icon;

  return (
    <div className={`card-v2 p-6 border ${config.border}`}>
      {/* Header */}
      <div className="flex items-start justify-between mb-4">
        <div className="flex items-start gap-3 flex-1">
          <span className="text-3xl">{platformIcons[app.platform]}</span>
          <div className="flex-1">
            <h3 className="text-lg md:text-xl font-semibold text-gray-900 mb-1">{app.name}</h3>
            <div className="flex items-center gap-3 text-sm text-gray-600">
              <span>{app.platform}</span>
              <span>•</span>
              <span>SDK v{app.sdkVersion}</span>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-3">
          <div className={`flex items-center gap-2 ${config.color}`}>
            <StatusIcon className="w-4 h-4" />
            <span className="text-sm font-bold">{config.label}</span>
          </div>
        </div>
      </div>

      {/* Metrics */}
      {app.status === 'active' ? (
        <>
          <div className="grid grid-cols-2 gap-4 mb-4">
            <div>
              <p className="text-xs text-gray-600 uppercase">Daily Revenue</p>
              <p className="text-lg font-bold text-gray-900">${app.dailyRevenue.toLocaleString()}</p>
            </div>
            <div>
              <p className="text-xs text-gray-600 uppercase">Impressions</p>
              <p className="text-lg font-bold text-gray-900">{app.dailyImpressions.toLocaleString()}</p>
            </div>
            <div>
              <p className="text-xs text-gray-600 uppercase">eCPM</p>
              <p className="text-lg font-bold text-gray-900">${app.ecpm.toFixed(2)}</p>
            </div>
            <div>
              <p className="text-xs text-gray-600 uppercase">DAU</p>
              <p className="text-lg font-bold text-gray-900">{app.users.toLocaleString()}</p>
            </div>
          </div>

          {/* SDK Version Warning */}
          {app.sdkVersion !== '4.2.1' && (
            <div className="bg-warning/10 border border-warning/20 rounded p-3 mb-4">
              <p className="text-sm text-warning font-medium">
                ⚠ SDK update available: v4.2.1 includes performance improvements
              </p>
            </div>
          )}
        </>
      ) : (
        <div className={`${config.bg} rounded p-4 mb-4 border-2`}>
          <p className="text-sm">
            {app.status === 'paused' && 'This app is currently paused. Click "Resume" to start serving ads again.'}
            {app.status === 'error' && 'SDK integration error detected. Check your implementation and credentials.'}
          </p>
        </div>
      )}

      {/* Footer */}
      <div className="flex items-center justify-between pt-4 border-t border-gray-200">
        <span className="text-xs text-gray-500">Updated: {app.lastUpdate}</span>
        <div className="flex gap-2">
          {app.status === 'active' && (
            <>
              <button
                onClick={onConfigure}
                className="btn-outline px-4 py-2 text-sm"
              >
                Configure
              </button>
              <button
                onClick={onViewStats}
                className="btn-primary px-4 py-2 text-sm"
              >
                View Stats
              </button>
            </>
          )}
          {app.status === 'paused' && (
            <button
              onClick={onResume}
              className="px-4 py-2 text-sm font-bold text-white bg-success rounded hover:opacity-90 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary-blue"
            >
              Resume
            </button>
          )}
          {app.status === 'error' && (
            <button
              onClick={onFixIntegration}
              className="px-4 py-2 text-sm font-bold text-white bg-danger rounded hover:opacity-90 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary-blue"
            >
              Fix Integration
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

type ModalState =
  | { type: null }
  | { type: 'add' }
  | { type: 'configure' | 'view-stats' | 'fix'; app: App };

function AppModal({ state, onClose }: { state: ModalState; onClose: () => void }) {
  if (state.type === null) return null;

  const title = (() => {
    if (state.type === 'add') {
      return 'Add New App';
    }
    if (state.type === 'configure') {
      return `Configure ${state.app.name}`;
    }
    if (state.type === 'view-stats') {
      return `Stats for ${state.app.name}`;
    }
    return `Integration Fixed for ${state.app.name}`;
  })();

  const description = (() => {
    if (state.type === 'add') {
      return 'Start integrating a new app into ApexMediation. We will walk you through registering your bundle ID and selecting ad formats.';
    }
    if (state.type === 'configure') {
      return 'Use the SDK key below to configure your app. You can update mediation settings, ad units, and targeting preferences in the detailed settings screen.';
    }
    if (state.type === 'view-stats') {
      return 'View the full analytics dashboard for this app, including revenue, impressions, retention, and funnel metrics.';
    }
    return 'We updated the SDK credentials and enabled monitoring. Please redeploy the latest SDK build to ensure everything stays healthy.';
  })();

  const sdkKey = state.type === 'add' ? null : `app_${state.app.id}_sdk_key_xyz`;

  // Accessibility: focus management & ESC/overlay close
  // We keep a reference to the last focused element to restore focus on close.
  if (typeof window !== 'undefined') {
    // noop - just to satisfy type narrowing when using document in handlers
  }

  const dialogId = 'apps-modal-title';
  const descId = 'apps-modal-description';

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm px-4"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="w-full max-w-xl rounded-lg border-2 border-brand-500 bg-white shadow-xl"
        role="dialog"
        aria-modal="true"
        aria-labelledby={dialogId}
        aria-describedby={descId}
        onKeyDown={(e) => {
          if (e.key === 'Escape') onClose();
        }}
      >
        <div className="flex items-start justify-between border-b-2 border-gray-200 p-6">
          <h2 id={dialogId} className="text-gray-900 font-bold uppercase text-lg">
            {title}
          </h2>
          <button
            onClick={onClose}
            className="text-gray-900 text-2xl leading-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))]"
            aria-label="Close dialog"
          >
            ×
          </button>
        </div>
        <div className="p-6 space-y-4">
          <p id={descId} className="text-sm text-gray-700 leading-relaxed">{description}</p>

          {sdkKey && (
            <div className="bg-gray-50 border-2 border-brand-500 rounded p-4">
              <p className="text-xs uppercase text-brand-700 font-bold">SDK Key</p>
              <code className="text-sm text-gray-800 break-all">{sdkKey}</code>
            </div>
          )}

          {state.type === 'add' && (
            <ol className="list-decimal list-inside space-y-2 text-sm text-gray-700">
              <li>Register your app bundle identifier.</li>
              <li>Choose supported platforms (iOS, Android, Unity, Web).</li>
              <li>Create ad placements for each format.</li>
              <li>Download the SDK and follow the integration guide.</li>
            </ol>
          )}
        </div>
        <div className="flex justify-end gap-3 border-t border-gray-200 p-6">
          <button onClick={onClose} className="btn-outline px-6 py-3 text-sm">
            Close
          </button>
          {state.type === 'add' ? (
            <a href="/docs" className="btn-primary px-6 py-3 text-sm">
              Start Integration
            </a>
          ) : (
            <a href={`/dashboard/apps/${state.app.id}`} className="btn-primary px-6 py-3 text-sm">
              Open details
            </a>
          )}
        </div>
      </div>
    </div>
  );
}
