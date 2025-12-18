'use client';
export const dynamic = 'force-dynamic';

// Reference: Design.md § "Dashboard Pages" & WEBSITE_DESIGN.md § "Settings Page"
// Settings page for account, payment, and notification preferences

import { BanknotesIcon, BellIcon, CreditCardIcon, ShieldCheckIcon, UserCircleIcon } from '@heroicons/react/24/outline';
import { useSearchParams } from 'next/navigation';
import { FormEvent, Suspense, useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

function SettingsPageInner() {
  const searchParams = useSearchParams();
  const tabParam = searchParams.get('tab');
  const initialTab = useMemo(() => {
    if (!tabParam) return 'profile';
    if (['profile', 'payment', 'notifications', 'security'].includes(tabParam)) {
      return tabParam as 'profile' | 'payment' | 'notifications' | 'security';
    }
    if (tabParam === 'payouts') return 'payment';
    return 'profile';
  }, [tabParam]);

  const [activeTab, setActiveTab] = useState<'profile' | 'payment' | 'notifications' | 'security'>(initialTab);

  useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab]);

  return (
    <Section>
      <Container className="space-y-6">
        {/* Page Header */}
        <div>
          <h1 className="text-h2-sm md:text-h2-md lg:text-h2 font-semibold text-gray-900">
            Settings
          </h1>
          <p className="text-sm text-gray-600 mt-1">
            Manage your account, payment methods, and preferences
          </p>
        </div>

        {/* Tabs */}
        <div className="border-b-2 border-gray-200">
          <div className="flex gap-1 overflow-x-auto">
            <TabButton
              active={activeTab === 'profile'}
              onClick={() => setActiveTab('profile')}
              icon={UserCircleIcon}
              label="Profile"
            />
            <TabButton
              active={activeTab === 'payment'}
              onClick={() => setActiveTab('payment')}
              icon={CreditCardIcon}
              label="Payment Methods"
            />
            <TabButton
              active={activeTab === 'notifications'}
              onClick={() => setActiveTab('notifications')}
              icon={BellIcon}
              label="Notifications"
            />
            <TabButton
              active={activeTab === 'security'}
              onClick={() => setActiveTab('security')}
              icon={ShieldCheckIcon}
              label="Security"
            />
          </div>
        </div>

        {/* Tab Content */}
        {activeTab === 'profile' && <ProfileTab />}
        {activeTab === 'payment' && <PaymentTab />}
        {activeTab === 'notifications' && <NotificationsTab />}
        {activeTab === 'security' && <SecurityTab />}
      </Container>
    </Section>
  );
}

export default function SettingsPage() {
  return (
    <Suspense fallback={<div className="container py-10"><div className="space-y-3" aria-hidden="true">{[...Array(3)].map((_, i) => (<div key={i} className="h-10 bg-gray-100 animate-pulse rounded" />))}</div></div>}>
      <SettingsPageInner />
    </Suspense>
  );
}

interface TabButtonProps {
  active: boolean;
  onClick: () => void;
  icon: React.ComponentType<{ className?: string }>;
  label: string;
}

function TabButton({ active, onClick, icon: Icon, label }: TabButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={`flex items-center gap-2 px-5 py-3 font-semibold text-sm transition-colors border-b-2 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))] ${
        active
          ? 'border-brand-500 text-brand-700'
          : 'border-transparent text-gray-600 hover:text-brand-700 hover:border-brand-200'
      }`}
    >
      <Icon className="w-5 h-5" />
      {label}
    </button>
  );
}

function ProfileTab() {
  const [status, setStatus] = useState<'idle' | 'success'>('idle');

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setStatus('success');
  };

  const handleDelete = () => {
    const confirmed = window.confirm(
      'Are you sure you want to delete your account? This action cannot be undone.'
    );
    if (confirmed) {
      alert('Account deletion scheduled. A confirmation email has been sent.');
    }
  };

  return (
    <div className="space-y-6">
      <form className="card p-6" onSubmit={handleSubmit} aria-labelledby="account-information-heading">
        <h2
          id="account-information-heading"
          className="text-gray-900 font-bold uppercase text-lg mb-6 border-b-2 border-gray-200 pb-2"
        >
          Account Information
        </h2>
        <div className="space-y-4 max-w-2xl">
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2" htmlFor="profile-name">
              Full Name
            </label>
            <input
              id="profile-name"
              name="name"
              type="text"
              className="input-v2 w-full"
              defaultValue="John Developer"
              required
            />
          </div>
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2" htmlFor="profile-email">
              Email Address
            </label>
            <input
              id="profile-email"
              name="email"
              type="email"
              className="input-v2 w-full"
              defaultValue="john@apexmediation.ee"
              required
            />
          </div>
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2" htmlFor="company-name">
              Company Name
            </label>
            <input
              id="company-name"
              name="company"
              type="text"
              className="input-v2 w-full"
              defaultValue="Awesome Games Inc."
            />
          </div>
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2" htmlFor="time-zone">
              Time Zone
            </label>
            <select id="time-zone" name="timezone" className="input-v2 w-full" defaultValue="UTC-8 (Pacific Time)">
              <option>UTC-8 (Pacific Time)</option>
              <option>UTC-5 (Eastern Time)</option>
              <option>UTC+0 (London)</option>
              <option>UTC+1 (Berlin)</option>
            </select>
          </div>
          <div className="pt-4 flex flex-col gap-3">
            <button type="submit" className="btn-primary px-8 py-3">
              Save Changes
            </button>
            {status === 'success' && (
              <span className="text-sm text-success font-semibold" role="status">
                Profile updated successfully.
              </span>
            )}
          </div>
        </div>
      </form>

      <div className="card-v2 p-6 border-2 border-danger/20">
        <h2 className="text-danger font-bold uppercase text-lg mb-4">
          Danger Zone
        </h2>
        <p className="text-gray-700 mb-4">
          Once you delete your account, there is no going back. This will permanently delete all your apps, data, and earnings history.
        </p>
        <button
          type="button"
          onClick={handleDelete}
          className="px-6 py-2 text-sm font-semibold text-white rounded bg-[rgb(var(--danger))] hover:opacity-90 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[rgb(var(--focus))]"
        >
          Delete Account
        </button>
      </div>
    </div>
  );
}

function PaymentTab() {
  return (
    <div className="space-y-6">
      {/* Payout Information */}
      <div className="card-v2 p-6">
        <div className="flex items-start justify-between">
          <div>
            <h2 className="text-gray-900 font-semibold text-lg mb-2">
              Next Payout
            </h2>
            <p className="text-3xl font-semibold text-gray-900 mb-1">$8,934.18</p>
            <p className="text-sm text-gray-600">Monthly payout (NET 30)</p>
          </div>
          <BanknotesIcon className="w-12 h-12 text-brand-600" />
        </div>
      </div>

      {/* Payment Methods */}
      <div className="card-v2 p-6">
        <h2 className="text-gray-900 font-semibold text-lg mb-6 border-b border-gray-200 pb-2">
          Payment Methods
        </h2>
        <div className="space-y-4">
          <PaymentMethodCard
            type="Bank Transfer"
            icon="🏦"
            details="Wells Fargo ****6789"
            primary
          />
          <PaymentMethodCard
            type="PayPal"
            icon="💳"
            details="john@apexmediation.ee"
          />
        </div>
        <button className="mt-4 btn-secondary text-sm">
          + Add Payment Method
        </button>
      </div>

      {/* Payout Settings */}
      <div className="card-v2 p-6">
        <h2 className="text-gray-900 font-semibold text-lg mb-6 border-b border-gray-200 pb-2">
          Payout Settings
        </h2>
        <div className="space-y-4 max-w-2xl">
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2">Payout Schedule</label>
            <select className="input-v2 w-full">
              <option>Monthly (NET 30)</option>
              <option>Monthly (1st of month)</option>
            </select>
          </div>
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2">Minimum Payout</label>
            <select className="input-v2 w-full">
              <option>$0 (Premium accounts only)</option>
              <option>$100 (Standard)</option>
              <option>$500</option>
              <option>$1000</option>
            </select>
            <p className="text-xs text-gray-600 mt-1">
              Premium accounts have no minimum payout threshold
            </p>
          </div>
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2">Currency</label>
            <select className="input-v2 w-full">
              <option>USD - US Dollar</option>
              <option>EUR - Euro</option>
              <option>GBP - British Pound</option>
              <option>CAD - Canadian Dollar</option>
            </select>
          </div>
          <div className="pt-4">
            <button className="btn-primary px-8 py-3">
              Save Settings
            </button>
          </div>
        </div>
      </div>

      {/* Tax Information */}
      <div className="card-v2 p-6">
        <h2 className="text-gray-900 font-semibold text-lg mb-6 border-b border-gray-200 pb-2">
          Tax Information
        </h2>
        <div className="space-y-4">
          <div className="bg-success/10 border border-success/20 rounded p-4">
            <div className="flex items-center gap-2 text-success font-bold mb-2">
              <ShieldCheckIcon className="w-5 h-5" />
              W-9 Form Submitted
            </div>
            <p className="text-sm text-gray-700">
              Your tax information is up to date. Updated on 2025-10-15
            </p>
          </div>
          <button className="btn-secondary text-sm">
            Update Tax Information
          </button>
        </div>
      </div>
    </div>
  );
}

function NotificationsTab() {
  const [slackConnected, setSlackConnected] = useState<boolean | null>(null);
  const [slackLoading, setSlackLoading] = useState(false);
  const [slackError, setSlackError] = useState<string | null>(null);

  useEffect(() => {
    let isMounted = true;
    (async () => {
      try {
        const res = await api.get<{ connected: boolean }>('/integrations/slack/status', { credentials: 'include' });
        if (!isMounted) return;
        if (res.success) setSlackConnected((res.data as any)?.connected === true);
        else setSlackError(res.error || 'Failed to fetch Slack status');
      } catch (e: any) {
        if (!isMounted) return;
        setSlackError(e?.message || 'Failed to fetch Slack status');
      }
    })();
    return () => { isMounted = false; };
  }, []);

  const onConnectSlack = async () => {
    setSlackLoading(true);
    setSlackError(null);
    try {
      const res = await api.get<{ url: string }>('/integrations/slack/connect', { credentials: 'include' });
      if (res.success && (res.data as any)?.url) {
        window.location.href = (res.data as any).url;
      } else {
        setSlackError(res.error || 'Unable to initiate Slack OAuth');
      }
    } catch (e: any) {
      setSlackError(e?.message || 'Unable to initiate Slack OAuth');
    } finally {
      setSlackLoading(false);
    }
  };

  return (
    <div className="space-y-6">
      <div className="card p-6">
        <h2 className="text-gray-900 font-bold uppercase text-lg mb-6 border-b-2 border-gray-200 pb-2">
          Email Notifications
        </h2>
        <div className="space-y-4">
          <NotificationToggle
            title="Revenue Reports"
            description="Monthly revenue summaries and performance insights"
            defaultChecked
          />
          <NotificationToggle
            title="Payout Notifications"
            description="Alerts when payouts are processed"
            defaultChecked
          />
          <NotificationToggle
            title="Fraud Alerts"
            description="Notifications about detected fraud activity"
            defaultChecked
          />
          <NotificationToggle
            title="System Updates"
            description="SDK updates, new features, and maintenance"
            defaultChecked
          />
          <NotificationToggle
            title="Marketing Emails"
            description="Tips, best practices, and product announcements"
          />
          <NotificationToggle
            title="A/B Test Results"
            description="Notifications when tests reach significance"
            defaultChecked
          />
        </div>
      </div>

      <div className="card p-6">
        <h2 className="text-gray-900 font-bold uppercase text-lg mb-6 border-b-2 border-gray-200 pb-2">
          Slack Integration
        </h2>
        <div className="bg-gray-50 p-4 rounded mb-4">
          <p className="text-sm text-gray-700 mb-3">
            Get real-time notifications in your Slack workspace for critical events like fraud detection, revenue milestones, and system alerts.
          </p>
          <div className="flex items-center gap-3">
            <button
              className="btn-primary px-6 py-2 text-sm disabled:opacity-60"
              onClick={onConnectSlack}
              disabled={slackLoading}
            >
              {slackLoading ? 'Connecting…' : (slackConnected ? 'Reconnect Slack' : 'Connect Slack Workspace')}
            </button>
            {slackConnected === true && (
              <span className="text-success bg-success/10 px-2 py-1 rounded text-xs font-bold">CONNECTED</span>
            )}
            {slackError && (
              <span className="text-danger text-xs">{slackError}</span>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

type ApiKey = {
  id: string;
  prefix: string;
  last4: string;
  createdAt: string;
  lastUsedAt?: string | null;
  revokedAt?: string | null;
};

function SecurityTab() {
  const [enrollQR, setEnrollQR] = useState<string | null>(null);
  const [maskedSecret, setMaskedSecret] = useState<string | null>(null);
  const [token, setToken] = useState('');
  const [twofaMsg, setTwofaMsg] = useState<string | null>(null);
  const [backupCodes, setBackupCodes] = useState<string[] | null>(null);
  const [loading2fa, setLoading2fa] = useState(false);

  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [keysLoading, setKeysLoading] = useState(false);
  const [keysError, setKeysError] = useState<string | null>(null);
  const [newKeySecret, setNewKeySecret] = useState<string | null>(null);

  const fetchKeys = async () => {
    setKeysLoading(true);
    setKeysError(null);
    try {
      const res = await api.get<{ keys: ApiKey[] }>('/keys', { credentials: 'include' });
      if (res.success) setKeys((res.data as any)?.keys ?? []);
      else setKeysError(res.error || 'Failed to load API keys');
    } catch (e: any) {
      setKeysError(e?.message || 'Failed to load API keys');
    } finally {
      setKeysLoading(false);
    }
  };

  useEffect(() => {
    fetchKeys();
  }, []);

  const handleEnroll2FA = async () => {
    setLoading2fa(true);
    setTwofaMsg(null);
    try {
      const res = await api.post<{ otpauthUrl: string; qrDataUrl: string; maskedSecret: string }>(
        '/auth/2fa/enroll',
        {},
        { credentials: 'include' }
      );
      if (res.success) {
        const d = res.data as any;
        setEnrollQR(d.qrDataUrl);
        setMaskedSecret(d.maskedSecret);
      } else {
        setTwofaMsg(res.error || 'Unable to start 2FA enrollment');
      }
    } catch (e: any) {
      setTwofaMsg(e?.message || 'Unable to start 2FA enrollment');
    } finally {
      setLoading2fa(false);
    }
  };

  const handleVerify2FA = async () => {
    setLoading2fa(true);
    setTwofaMsg(null);
    setBackupCodes(null);
    try {
      const res = await api.post<{ backupCodes: string[] }>(
        '/auth/2fa/verify',
        { token },
        { credentials: 'include' }
      );
      if (res.success) {
        setTwofaMsg('Two-factor authentication enabled');
        setBackupCodes((res.data as any)?.backupCodes ?? []);
      } else {
        setTwofaMsg(res.error || 'Verification failed');
      }
    } catch (e: any) {
      setTwofaMsg(e?.message || 'Verification failed');
    } finally {
      setLoading2fa(false);
    }
  };

  const createKey = async (live = false) => {
    try {
      const res = await api.post<{ id: string; secret: string; prefix: string; last4: string }>(
        '/keys',
        { live },
        { credentials: 'include' }
      );
      if (res.success) {
        const d = res.data as any;
        setNewKeySecret(d.secret); // ephemeral display only
        await fetchKeys();
      } else {
        setKeysError(res.error || 'Failed to create API key');
      }
    } catch (e: any) {
      setKeysError(e?.message || 'Failed to create API key');
    }
  };

  const rotateKey = async (id: string) => {
    try {
      const res = await api.post<{ id: string; secret: string; prefix: string; last4: string }>(
        `/keys/${id}/rotate`,
        {},
        { credentials: 'include' }
      );
      if (res.success) {
        setNewKeySecret((res.data as any).secret);
        await fetchKeys();
      } else {
        setKeysError(res.error || 'Failed to rotate key');
      }
    } catch (e: any) {
      setKeysError(e?.message || 'Failed to rotate key');
    }
  };

  const revokeKey = async (id: string) => {
    try {
      const res = await api.delete(`/keys/${id}`, { credentials: 'include' });
      if (!res.success && res.error) setKeysError(res.error);
      await fetchKeys();
    } catch (e: any) {
      setKeysError(e?.message || 'Failed to revoke key');
    }
  };

  return (
    <div className="space-y-6">
      <div className="card p-6">
        <h2 className="text-gray-900 font-bold uppercase text-lg mb-6 border-b-2 border-gray-200 pb-2">
          Change Password
        </h2>
        <div className="space-y-4 max-w-2xl">
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2">Current Password</label>
            <input
              type="password"
              className="input-v2 w-full"
            />
          </div>
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2">New Password</label>
            <input
              type="password"
              className="input-v2 w-full"
            />
          </div>
          <div>
            <label className="block text-sm font-semibold text-gray-900 mb-2">Confirm New Password</label>
            <input
              type="password"
              className="input-v2 w-full"
            />
          </div>
          <div className="pt-4">
            <button className="btn-primary px-8 py-3">
              Update Password
            </button>
          </div>
        </div>
      </div>

      <div className="card p-6">
        <h2 className="text-gray-900 font-bold uppercase text-lg mb-6 border-b-2 border-gray-200 pb-2">
          Two-Factor Authentication
        </h2>
        <div className="bg-warning/10 border border-warning rounded p-4 mb-4">
          <p className="text-sm text-gray-700 mb-3">
            Add an extra layer of security to your account. When enabled, you&rsquo;ll need to enter a code from your phone in addition to your password.
          </p>
          <div className="flex items-center gap-3">
            <button className="btn-primary px-6 py-2 text-sm disabled:opacity-60" onClick={handleEnroll2FA} disabled={loading2fa}>
              {loading2fa ? 'Please wait…' : 'Enable 2FA'}
            </button>
            {twofaMsg && <span className="text-sm font-bold text-gray-900">{twofaMsg}</span>}
          </div>
        </div>
        {enrollQR && (
          <div className="bg-white border border-warning rounded p-4 mb-4">
            <p className="text-sm text-gray-700 mb-2">Scan this QR with your authenticator app and enter the 6‑digit code. Secret: <span className="font-mono">{maskedSecret}</span></p>
            <img src={enrollQR} alt="2FA QR" className="w-48 h-48 border" />
            <div className="mt-3 flex items-center gap-2">
              <input
                type="text"
                inputMode="numeric"
                className="input-v2 w-40"
                placeholder="123456"
                value={token}
                onChange={(e) => setToken(e.target.value)}
              />
              <button className="btn-secondary text-sm" onClick={handleVerify2FA} disabled={loading2fa}>
                Verify
              </button>
            </div>
          </div>
        )}
        {backupCodes && backupCodes.length > 0 && (
          <div className="bg-success/10 border border-success/20 rounded p-4">
            <p className="text-sm text-success font-bold mb-2">Save these backup codes in a safe place:</p>
            <ul className="grid grid-cols-2 gap-2 font-mono text-sm">
              {backupCodes.map((c) => (
                <li key={c} className="bg-white border px-2 py-1">{c}</li>
              ))}
            </ul>
          </div>
        )}
      </div>

      <div className="card p-6">
        <h2 className="text-gray-900 font-bold uppercase text-lg mb-6 border-b-2 border-gray-200 pb-2">
          API Keys
        </h2>
        <div className="space-y-3">
          {keysLoading && <p className="text-sm text-gray-600">Loading keys…</p>}
          {keysError && <p className="text-sm text-danger">{keysError}</p>}
          {newKeySecret && (
            <div className="bg-warning/10 border border-warning rounded p-3">
              <p className="text-sm text-gray-800 mb-1 font-bold">New API key (copy now, it will not be shown again):</p>
              <code className="text-xs bg-white border px-2 py-1 inline-block break-all">{newKeySecret}</code>
              <div>
                <button className="mt-2 text-xs text-brand-700 underline" onClick={() => setNewKeySecret(null)}>Dismiss</button>
              </div>
            </div>
          )}
          {keys.map((k) => (
            <div key={k.id} className="border-2 border-gray-300 rounded p-4">
              <div className="flex items-start justify-between mb-2">
                <div>
                  <p className="font-bold text-gray-900">{k.prefix === 'sk_live' ? 'Production' : 'Test'} API Key</p>
                  <p className="text-sm text-gray-600 font-mono">{k.prefix}_••••••••••••••••{k.last4}</p>
                </div>
                <div className="flex items-center gap-3">
                  <button className="text-sm font-bold text-brand-700" onClick={() => rotateKey(k.id)}>Rotate</button>
                  <button className="text-sm font-bold text-danger hover:opacity-90" onClick={() => revokeKey(k.id)}>Revoke</button>
                </div>
              </div>
              <p className="text-xs text-gray-500">Created: {new Date(k.createdAt).toLocaleString()} {k.lastUsedAt ? `• Last used: ${new Date(k.lastUsedAt).toLocaleString()}` : ''}</p>
            </div>
          ))}
        </div>
        <div className="mt-4 flex gap-3">
          <button className="btn-outline px-4 py-2 text-sm" onClick={() => createKey(false)}>
            + Generate Test Key
          </button>
          <button className="btn-outline px-4 py-2 text-sm" onClick={() => createKey(true)}>
            + Generate Production Key
          </button>
        </div>
      </div>

      <div className="card p-6">
        <h2 className="text-gray-900 font-bold uppercase text-lg mb-6 border-b-2 border-gray-200 pb-2">
          Active Sessions
        </h2>
        <div className="space-y-3">
          <SessionCard
            device="MacBook Pro"
            location="San Francisco, CA"
            lastActive="Active now"
            current
          />
          <SessionCard
            device="iPhone 14 Pro"
            location="San Francisco, CA"
            lastActive="2 hours ago"
          />
        </div>
      </div>
    </div>
  );
}

interface PaymentMethodCardProps {
  type: string;
  icon: string;
  details: string;
  primary?: boolean;
}

function PaymentMethodCard({ type, icon, details, primary }: PaymentMethodCardProps) {
  return (
    <div className={`border-2 rounded p-4 ${primary ? 'border-brand-500 bg-brand-50' : 'border-gray-300'}`}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <span className="text-2xl">{icon}</span>
          <div>
            <p className="font-bold text-gray-900">{type}</p>
            <p className="text-sm text-gray-600">{details}</p>
          </div>
        </div>
        {primary && (
          <div className="bg-brand-100 text-brand-700 px-3 py-1 rounded text-xs font-bold border" style={{ borderColor: 'rgb(var(--brand-500))' }}>
            PRIMARY
          </div>
        )}
      </div>
    </div>
  );
}

interface NotificationToggleProps {
  title: string;
  description: string;
  defaultChecked?: boolean;
}

function NotificationToggle({ title, description, defaultChecked }: NotificationToggleProps) {
  return (
    <div className="flex items-start justify-between py-3 border-b border-gray-200 last:border-0">
      <div className="flex-1">
        <p className="font-bold text-gray-900">{title}</p>
        <p className="text-sm text-gray-600">{description}</p>
      </div>
      <label className="relative inline-block w-12 h-6 ml-4">
        <input type="checkbox" className="sr-only peer" defaultChecked={defaultChecked} />
        <div className="w-12 h-6 bg-gray-300 rounded-full peer peer-checked:bg-brand-500 transition-colors cursor-pointer"></div>
        <div className="absolute top-1 left-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-6"></div>
      </label>
    </div>
  );
}

interface SessionCardProps {
  device: string;
  location: string;
  lastActive: string;
  current?: boolean;
}

function SessionCard({ device, location, lastActive, current }: SessionCardProps) {
  return (
    <div className="border-2 border-gray-300 rounded p-4">
      <div className="flex items-start justify-between">
        <div>
          <p className="font-bold text-gray-900">{device}</p>
          <p className="text-sm text-gray-600">{location}</p>
          <p className="text-xs text-gray-500 mt-1">{lastActive}</p>
        </div>
        {current ? (
          <div className="bg-success/10 text-success px-3 py-1 rounded text-xs font-bold">
            CURRENT
          </div>
        ) : (
          <button className="text-sm font-bold text-danger hover:opacity-90">
            Revoke
          </button>
        )}
      </div>
    </div>
  );
}
