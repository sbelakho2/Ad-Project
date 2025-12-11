'use client'

import { useState, useEffect, useCallback } from 'react'
import { useSession } from 'next-auth/react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import {
  Key,
  Plus,
  RefreshCw,
  Trash2,
  Copy,
  CheckCircle,
  AlertTriangle,
  Eye,
  EyeOff,
  Shield,
  Smartphone,
  Globe,
  Tv,
  Gamepad2,
} from 'lucide-react'
import { appsApi, appKeysApi } from '@/lib/api'
import type { App, AppApiKey, AppApiKeyWithSecret } from '@/types'

const PLATFORM_ICONS: Record<string, React.FC<{ className?: string }>> = {
  android: Smartphone,
  ios: Smartphone,
  web: Globe,
  tvos: Tv,
  'android-tv': Tv,
  unity: Gamepad2,
}

export default function AppKeysPage() {
  const { data: session } = useSession()
  const queryClient = useQueryClient()
  const [selectedApp, setSelectedApp] = useState<string | null>(null)
  const [newKeySecret, setNewKeySecret] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const [showSecret, setShowSecret] = useState(false)
  const [confirmRevoke, setConfirmRevoke] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const { data: apps, isLoading: appsLoading } = useQuery({
    queryKey: ['apps'],
    queryFn: appsApi.list,
    enabled: !!session,
  })

  const { data: keys, isLoading: keysLoading, refetch: refetchKeys } = useQuery({
    queryKey: ['app-keys', selectedApp],
    queryFn: () => (selectedApp ? appKeysApi.list(selectedApp) : Promise.resolve([])),
    enabled: !!selectedApp,
  })

  const createKeyMutation = useMutation({
    mutationFn: ({ appId, live }: { appId: string; live: boolean }) =>
      appKeysApi.create(appId, { live }),
    onSuccess: (data) => {
      setNewKeySecret(data.secret)
      queryClient.invalidateQueries({ queryKey: ['app-keys', selectedApp] })
    },
    onError: (err: Error) => {
      setError(err.message || 'Failed to create key')
    },
  })

  const rotateKeyMutation = useMutation({
    mutationFn: ({ appId, keyId }: { appId: string; keyId: string }) =>
      appKeysApi.rotate(appId, keyId),
    onSuccess: (data) => {
      setNewKeySecret(data.secret)
      queryClient.invalidateQueries({ queryKey: ['app-keys', selectedApp] })
    },
    onError: (err: Error) => {
      setError(err.message || 'Failed to rotate key')
    },
  })

  const revokeKeyMutation = useMutation({
    mutationFn: ({ appId, keyId }: { appId: string; keyId: string }) =>
      appKeysApi.revoke(appId, keyId),
    onSuccess: () => {
      setConfirmRevoke(null)
      queryClient.invalidateQueries({ queryKey: ['app-keys', selectedApp] })
    },
    onError: (err: Error) => {
      setError(err.message || 'Failed to revoke key')
    },
  })

  // Auto-select first app
  useEffect(() => {
    if (apps && apps.length > 0 && !selectedApp) {
      setSelectedApp(apps[0].id)
    }
  }, [apps, selectedApp])

  const handleCopy = useCallback(async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      setError('Failed to copy to clipboard')
    }
  }, [])

  const handleCreateKey = (live: boolean) => {
    if (!selectedApp) return
    setError(null)
    setNewKeySecret(null)
    createKeyMutation.mutate({ appId: selectedApp, live })
  }

  const handleRotateKey = (keyId: string) => {
    if (!selectedApp) return
    setError(null)
    setNewKeySecret(null)
    rotateKeyMutation.mutate({ appId: selectedApp, keyId })
  }

  const handleRevokeKey = (keyId: string) => {
    if (!selectedApp) return
    setError(null)
    revokeKeyMutation.mutate({ appId: selectedApp, keyId })
  }

  const selectedAppData = apps?.find((a) => a.id === selectedApp)
  const activeKeys = keys?.filter((k) => !k.revokedAt) ?? []
  const revokedKeys = keys?.filter((k) => k.revokedAt) ?? []

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <div className="flex items-center gap-3">
            <Key className="h-8 w-8 text-primary-600" />
            <div>
              <p className="text-sm font-medium text-primary-600">Settings</p>
              <h1 className="text-3xl font-semibold text-gray-900 tracking-tight">
                SDK API Keys
              </h1>
              <p className="text-sm text-gray-600 mt-1">
                Manage API keys for SDK authentication. Each app has its own keys.
              </p>
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        {/* App Selector */}
        <div className="card mb-6">
          <label htmlFor="app-select" className="block text-sm font-medium text-gray-700 mb-2">
            Select App
          </label>
          {appsLoading ? (
            <div className="h-10 bg-gray-100 rounded animate-pulse" />
          ) : apps && apps.length > 0 ? (
            <select
              id="app-select"
              value={selectedApp ?? ''}
              onChange={(e) => {
                setSelectedApp(e.target.value)
                setNewKeySecret(null)
              }}
              className="input w-full max-w-md"
            >
              {apps.map((app) => {
                const Icon = PLATFORM_ICONS[app.platform] ?? Smartphone
                return (
                  <option key={app.id} value={app.id}>
                    {app.name} ({app.bundleId}) — {app.platform}
                  </option>
                )
              })}
            </select>
          ) : (
            <p className="text-gray-500">No apps found. Create an app first.</p>
          )}
        </div>

        {/* Error Display */}
        {error && (
          <div className="mb-6 p-4 bg-red-50 border border-red-200 rounded-lg flex items-start gap-3">
            <AlertTriangle className="h-5 w-5 text-red-500 flex-shrink-0 mt-0.5" />
            <div>
              <p className="font-medium text-red-800">Error</p>
              <p className="text-sm text-red-700">{error}</p>
            </div>
            <button
              onClick={() => setError(null)}
              className="ml-auto text-red-500 hover:text-red-700"
            >
              ×
            </button>
          </div>
        )}

        {/* New Key Display */}
        {newKeySecret && (
          <div className="mb-6 p-4 bg-green-50 border border-green-200 rounded-lg">
            <div className="flex items-start gap-3">
              <CheckCircle className="h-5 w-5 text-green-500 flex-shrink-0 mt-0.5" />
              <div className="flex-1">
                <p className="font-medium text-green-800">API Key Created</p>
                <p className="text-sm text-green-700 mb-3">
                  Copy this key now. You will not be able to see it again.
                </p>
                <div className="flex items-center gap-2 bg-white border border-green-300 rounded px-3 py-2">
                  <code className="flex-1 font-mono text-sm break-all">
                    {showSecret ? newKeySecret : '•'.repeat(40)}
                  </code>
                  <button
                    onClick={() => setShowSecret(!showSecret)}
                    className="p-1 text-gray-500 hover:text-gray-700"
                    title={showSecret ? 'Hide' : 'Show'}
                  >
                    {showSecret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                  </button>
                  <button
                    onClick={() => handleCopy(newKeySecret)}
                    className="p-1 text-gray-500 hover:text-gray-700"
                    title="Copy"
                  >
                    {copied ? (
                      <CheckCircle className="h-4 w-4 text-green-500" />
                    ) : (
                      <Copy className="h-4 w-4" />
                    )}
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}

        {selectedApp && (
          <>
            {/* Create Key Actions */}
            <div className="card mb-6">
              <h2 className="text-lg font-medium text-gray-900 mb-4">Create New Key</h2>
              <p className="text-sm text-gray-600 mb-4">
                Create sandbox keys for development or live keys for production.
              </p>
              <div className="flex gap-3">
                <button
                  onClick={() => handleCreateKey(false)}
                  disabled={createKeyMutation.isPending}
                  className="btn btn-outline flex items-center gap-2"
                >
                  <Plus className="h-4 w-4" />
                  Create Sandbox Key
                </button>
                <button
                  onClick={() => handleCreateKey(true)}
                  disabled={createKeyMutation.isPending}
                  className="btn btn-primary flex items-center gap-2"
                >
                  <Shield className="h-4 w-4" />
                  Create Live Key
                </button>
              </div>
            </div>

            {/* Active Keys */}
            <div className="card mb-6">
              <div className="flex items-center justify-between mb-4">
                <h2 className="text-lg font-medium text-gray-900">Active Keys</h2>
                <button
                  onClick={() => refetchKeys()}
                  className="text-gray-500 hover:text-gray-700"
                  title="Refresh"
                >
                  <RefreshCw className="h-4 w-4" />
                </button>
              </div>

              {keysLoading ? (
                <div className="space-y-3">
                  {[1, 2].map((i) => (
                    <div key={i} className="h-16 bg-gray-100 rounded animate-pulse" />
                  ))}
                </div>
              ) : activeKeys.length > 0 ? (
                <div className="space-y-3">
                  {activeKeys.map((key) => (
                    <KeyCard
                      key={key.id}
                      apiKey={key}
                      onRotate={() => handleRotateKey(key.id)}
                      onRevoke={() => setConfirmRevoke(key.id)}
                      isRotating={rotateKeyMutation.isPending}
                    />
                  ))}
                </div>
              ) : (
                <p className="text-gray-500 text-center py-8">
                  No active keys. Create one above.
                </p>
              )}
            </div>

            {/* Revoked Keys */}
            {revokedKeys.length > 0 && (
              <div className="card">
                <h2 className="text-lg font-medium text-gray-900 mb-4">Revoked Keys</h2>
                <div className="space-y-3 opacity-60">
                  {revokedKeys.map((key) => (
                    <KeyCard key={key.id} apiKey={key} revoked />
                  ))}
                </div>
              </div>
            )}
          </>
        )}

        {/* Revoke Confirmation Modal */}
        {confirmRevoke && (
          <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
            <div className="bg-white rounded-lg p-6 max-w-md w-full mx-4 shadow-xl">
              <h3 className="text-lg font-medium text-gray-900 mb-2">Revoke API Key?</h3>
              <p className="text-sm text-gray-600 mb-4">
                This will immediately invalidate the key. Any SDK or service using this key will
                stop working. This action cannot be undone.
              </p>
              <div className="flex gap-3 justify-end">
                <button
                  onClick={() => setConfirmRevoke(null)}
                  className="btn btn-outline"
                >
                  Cancel
                </button>
                <button
                  onClick={() => handleRevokeKey(confirmRevoke)}
                  disabled={revokeKeyMutation.isPending}
                  className="btn bg-red-600 text-white hover:bg-red-700"
                >
                  {revokeKeyMutation.isPending ? 'Revoking...' : 'Revoke Key'}
                </button>
              </div>
            </div>
          </div>
        )}
      </main>
    </div>
  )
}

function KeyCard({
  apiKey,
  onRotate,
  onRevoke,
  isRotating,
  revoked,
}: {
  apiKey: AppApiKey
  onRotate?: () => void
  onRevoke?: () => void
  isRotating?: boolean
  revoked?: boolean
}) {
  const envColors = {
    sandbox: 'bg-yellow-100 text-yellow-800',
    live: 'bg-green-100 text-green-800',
  }

  return (
    <div className={`border rounded-lg p-4 ${revoked ? 'bg-gray-50' : 'bg-white'}`}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Key className={`h-5 w-5 ${revoked ? 'text-gray-400' : 'text-gray-600'}`} />
          <div>
            <div className="flex items-center gap-2">
              <code className={`font-mono text-sm ${revoked ? 'text-gray-400' : 'text-gray-900'}`}>
                {apiKey.prefix}...{apiKey.last4}
              </code>
              <span className={`text-xs px-2 py-0.5 rounded-full ${envColors[apiKey.environment]}`}>
                {apiKey.environment}
              </span>
              {revoked && (
                <span className="text-xs px-2 py-0.5 rounded-full bg-red-100 text-red-800">
                  Revoked
                </span>
              )}
            </div>
            <p className="text-xs text-gray-500 mt-1">
              Created {new Date(apiKey.createdAt).toLocaleDateString()}
              {apiKey.lastUsedAt && (
                <> • Last used {new Date(apiKey.lastUsedAt).toLocaleDateString()}</>
              )}
              {apiKey.revokedAt && (
                <> • Revoked {new Date(apiKey.revokedAt).toLocaleDateString()}</>
              )}
            </p>
          </div>
        </div>

        {!revoked && (
          <div className="flex items-center gap-2">
            <button
              onClick={onRotate}
              disabled={isRotating}
              className="btn btn-outline btn-sm flex items-center gap-1"
              title="Rotate key"
            >
              <RefreshCw className={`h-3 w-3 ${isRotating ? 'animate-spin' : ''}`} />
              Rotate
            </button>
            <button
              onClick={onRevoke}
              className="btn btn-outline btn-sm text-red-600 hover:bg-red-50 hover:border-red-300"
              title="Revoke key"
            >
              <Trash2 className="h-3 w-3" />
            </button>
          </div>
        )}
      </div>
    </div>
  )
}
