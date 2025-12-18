'use client';

import { useEffect, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import { usePlanAccess } from '@/lib/usePlanAccess';

interface NetworkConfig {
  type: string;
  name: string;
  authType: 'oauth' | 'api_key';
  requiredFields: string[];
  docsUrl?: string;
  hasOAuth: boolean;
}

interface Connection {
  id: string;
  networkType: string;
  networkName: string;
  status: string;
  lastSyncAt: string | null;
  statusMessage?: string;
}

export default function NetworkSettingsPage() {
  const searchParams = useSearchParams();
  const { allowed, isLoading: planLoading, plan } = usePlanAccess(['growth', 'scale', 'enterprise']);
  const [networks, setNetworks] = useState<NetworkConfig[]>([]);
  const [connections, setConnections] = useState<Connection[]>([]);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<'connected' | 'available'>('connected');
  const [connectingNetwork, setConnectingNetwork] = useState<string | null>(null);
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const [syncing, setSyncing] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  useEffect(() => {
    // Check for OAuth callback results
    const connected = searchParams.get('connected');
    const errorParam = searchParams.get('error');
    
    if (connected) {
      setSuccess(`Successfully connected ${connected}!`);
    }
    if (errorParam) {
      setError(`Connection failed: ${errorParam}`);
    }
  }, [searchParams]);

  useEffect(() => {
    if (planLoading || !allowed) {
      setLoading(false);
      return;
    }
    fetchData();
  }, [planLoading, allowed]);

  async function fetchData() {
    setLoading(true);
    try {
      const [networksRes, connectionsRes] = await Promise.all([
        fetch('/api/v1/vra/networks', { credentials: 'include' }),
        fetch('/api/v1/vra/connections', { credentials: 'include' }),
      ]);

      if (networksRes.ok) {
        const data = await networksRes.json();
        setNetworks(data.networks || []);
      }

      if (connectionsRes.ok) {
        const data = await connectionsRes.json();
        setConnections(data.connections || []);
      }
    } catch (err) {
      setError('Failed to load network data');
    } finally {
      setLoading(false);
    }
  }

  async function startOAuth(networkType: string) {
    window.location.href = `/api/v1/vra/oauth/${networkType}`;
  }

  async function connectWithApiKey(networkType: string) {
    const network = networks.find(n => n.type === networkType);
    if (!network) return;

    const creds: Record<string, string> = {};
    for (const field of network.requiredFields) {
      const value = credentials[`${networkType}_${field}`];
      if (!value) {
        setError(`Please fill in all required fields`);
        return;
      }
      creds[field] = value;
    }

    try {
      const res = await fetch('/api/v1/vra/connections', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({
          networkType,
          credentials: creds,
        }),
      });

      if (res.ok) {
        setSuccess(`Successfully connected ${network.name}!`);
        setConnectingNetwork(null);
        setCredentials({});
        await fetchData();
      } else {
        const data = await res.json();
        setError(data.error || 'Failed to connect');
      }
    } catch (err) {
      setError('Failed to connect network');
    }
  }

  async function disconnectNetwork(connectionId: string) {
    if (!confirm('Are you sure you want to disconnect this network?')) return;

    try {
      const res = await fetch(`/api/v1/vra/connections/${connectionId}`, {
        method: 'DELETE',
        credentials: 'include',
      });

      if (res.ok) {
        setSuccess('Network disconnected');
        await fetchData();
      }
    } catch (err) {
      setError('Failed to disconnect');
    }
  }

  async function syncNetwork(connectionId: string) {
    setSyncing(connectionId);
    try {
      const res = await fetch(`/api/v1/vra/connections/${connectionId}/sync`, {
        method: 'POST',
        credentials: 'include',
      });

      if (res.ok) {
        setSuccess('Sync started');
        await fetchData();
      }
    } catch (err) {
      setError('Sync failed');
    } finally {
      setSyncing(null);
    }
  }

  const connectedNetworkTypes = new Set(connections.map(c => c.networkType));
  const availableNetworks = networks.filter(n => !connectedNetworkTypes.has(n.type));

  if (planLoading) {
    return (
      <div className="p-6">
        <p className="text-sm text-gray-500">Checking your plan…</p>
      </div>
    );
  }

  if (!allowed) {
    return (
      <div className="p-6">
        <h1 className="text-xl font-semibold mb-4">Network Connections</h1>
        <div className="rounded-md border border-amber-200 bg-amber-50 p-4" role="alert">
          <p className="text-sm font-semibold text-amber-900">Upgrade to connect ad networks</p>
          <p className="text-sm text-amber-800 mt-1">
            Network connections are available on Growth plan and above. Current plan: {plan || 'unknown'}.
          </p>
          <div className="mt-3">
            <a href="/settings/billing/settings" className="px-3 py-2 text-sm font-semibold text-primary-700 border border-primary-200 rounded-lg hover:bg-primary-50">
              Open Billing
            </a>
          </div>
        </div>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="p-6">
        <p className="text-sm text-gray-500">Loading…</p>
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6 max-w-4xl">
      <div>
        <h1 className="text-xl font-semibold">Network Connections</h1>
        <p className="text-sm text-gray-600">
          Connect your ad networks to enable revenue reconciliation.
        </p>
      </div>

      {/* Status messages */}
      {error && (
        <div className="rounded-md border border-red-200 bg-red-50 p-4">
          <p className="text-sm text-red-800">{error}</p>
          <button onClick={() => setError(null)} className="text-xs text-red-600 underline mt-1">Dismiss</button>
        </div>
      )}
      {success && (
        <div className="rounded-md border border-green-200 bg-green-50 p-4">
          <p className="text-sm text-green-800">{success}</p>
          <button onClick={() => setSuccess(null)} className="text-xs text-green-600 underline mt-1">Dismiss</button>
        </div>
      )}

      {/* Tabs */}
      <div className="border-b">
        <div className="flex gap-4">
          <button
            onClick={() => setActiveTab('connected')}
            className={`py-2 px-1 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'connected'
                ? 'border-blue-600 text-blue-600'
                : 'border-transparent text-gray-500 hover:text-gray-700'
            }`}
          >
            Connected ({connections.length})
          </button>
          <button
            onClick={() => setActiveTab('available')}
            className={`py-2 px-1 text-sm font-medium border-b-2 -mb-px ${
              activeTab === 'available'
                ? 'border-blue-600 text-blue-600'
                : 'border-transparent text-gray-500 hover:text-gray-700'
            }`}
          >
            Available ({availableNetworks.length})
          </button>
        </div>
      </div>

      {/* Connected Networks */}
      {activeTab === 'connected' && (
        <div className="space-y-4">
          {connections.length === 0 ? (
            <div className="rounded-md border p-8 text-center">
              <p className="text-gray-500 mb-4">No networks connected yet.</p>
              <button
                onClick={() => setActiveTab('available')}
                className="px-4 py-2 bg-blue-600 text-white text-sm font-medium rounded-lg hover:bg-blue-700"
              >
                Connect Your First Network
              </button>
            </div>
          ) : (
            connections.map((conn) => (
              <div key={conn.id} className="rounded-md border p-4 bg-white">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-3">
                    <div className="w-10 h-10 rounded-lg bg-gray-100 flex items-center justify-center text-lg font-semibold">
                      {conn.networkName.charAt(0)}
                    </div>
                    <div>
                      <h3 className="font-medium">{conn.networkName}</h3>
                      <div className="flex items-center gap-2 mt-1">
                        <span className={`inline-flex items-center px-2 py-0.5 text-xs font-medium rounded ${
                          conn.status === 'active' 
                            ? 'bg-green-100 text-green-700' 
                            : conn.status === 'error'
                            ? 'bg-red-100 text-red-700'
                            : 'bg-gray-100 text-gray-700'
                        }`}>
                          {conn.status}
                        </span>
                        {conn.lastSyncAt && (
                          <span className="text-xs text-gray-500">
                            Last sync: {new Date(conn.lastSyncAt).toLocaleDateString()}
                          </span>
                        )}
                      </div>
                      {conn.statusMessage && (
                        <p className="text-xs text-gray-500 mt-1">{conn.statusMessage}</p>
                      )}
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => syncNetwork(conn.id)}
                      disabled={syncing === conn.id}
                      className="px-3 py-1.5 text-sm border rounded-lg hover:bg-gray-50 disabled:opacity-50 flex items-center gap-1"
                    >
                      {syncing === conn.id ? (
                        <svg className="w-4 h-4 animate-spin" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                        </svg>
                      ) : (
                        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                        </svg>
                      )}
                      Sync
                    </button>
                    <button
                      onClick={() => disconnectNetwork(conn.id)}
                      className="px-3 py-1.5 text-sm text-red-600 border border-red-200 rounded-lg hover:bg-red-50"
                    >
                      Disconnect
                    </button>
                  </div>
                </div>
              </div>
            ))
          )}
        </div>
      )}

      {/* Available Networks */}
      {activeTab === 'available' && (
        <div className="space-y-4">
          {availableNetworks.length === 0 ? (
            <div className="rounded-md border p-8 text-center">
              <p className="text-gray-500">All supported networks are connected!</p>
            </div>
          ) : (
            availableNetworks.map((network) => (
              <div key={network.type} className="rounded-md border p-4 bg-white">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-3">
                    <div className="w-10 h-10 rounded-lg bg-gray-100 flex items-center justify-center text-lg font-semibold">
                      {network.name.charAt(0)}
                    </div>
                    <div>
                      <h3 className="font-medium">{network.name}</h3>
                      <p className="text-xs text-gray-500">
                        {network.hasOAuth ? 'OAuth Authentication' : 'API Key Authentication'}
                      </p>
                    </div>
                  </div>
                  {connectingNetwork !== network.type ? (
                    <button
                      onClick={() => {
                        if (network.hasOAuth) {
                          startOAuth(network.type);
                        } else {
                          setConnectingNetwork(network.type);
                        }
                      }}
                      className="px-4 py-2 bg-blue-600 text-white text-sm font-medium rounded-lg hover:bg-blue-700"
                    >
                      {network.hasOAuth ? 'Connect with OAuth' : 'Connect'}
                    </button>
                  ) : (
                    <button
                      onClick={() => setConnectingNetwork(null)}
                      className="px-3 py-1.5 text-sm text-gray-600 border rounded-lg hover:bg-gray-50"
                    >
                      Cancel
                    </button>
                  )}
                </div>

                {/* API Key Form */}
                {connectingNetwork === network.type && !network.hasOAuth && (
                  <div className="mt-4 pt-4 border-t">
                    <div className="space-y-3">
                      {network.requiredFields.map((field) => (
                        <div key={field}>
                          <label className="block text-sm font-medium text-gray-700 mb-1">
                            {field.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase())}
                          </label>
                          <input
                            type={field.includes('secret') || field.includes('key') ? 'password' : 'text'}
                            value={credentials[`${network.type}_${field}`] || ''}
                            onChange={(e) => setCredentials({
                              ...credentials,
                              [`${network.type}_${field}`]: e.target.value
                            })}
                            className="w-full px-3 py-2 border rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                            placeholder={`Enter ${field.replace(/_/g, ' ')}`}
                          />
                        </div>
                      ))}
                      <button
                        onClick={() => connectWithApiKey(network.type)}
                        className="w-full px-4 py-2 bg-blue-600 text-white text-sm font-medium rounded-lg hover:bg-blue-700"
                      >
                        Connect {network.name}
                      </button>
                    </div>
                    {network.docsUrl && (
                      <p className="text-xs text-gray-500 mt-3">
                        Need help? <a href={network.docsUrl} target="_blank" rel="noopener noreferrer" className="text-blue-600 hover:underline">View documentation</a>
                      </p>
                    )}
                  </div>
                )}
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
