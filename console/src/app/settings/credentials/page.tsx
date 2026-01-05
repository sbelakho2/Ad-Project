'use client'

import { useState, useEffect, useMemo } from 'react'
import { Plus, Key, Trash2, RotateCw, Check, AlertCircle, Upload, Play } from 'lucide-react'
import { byoApi } from '@/lib/api'
import type { NetworkCredential, NetworkIngestionResult } from '@/types'

const SUPPORTED_NETWORKS = [
  { id: 'admob', name: 'AdMob', fields: ['accountId', 'apiKey'] },
  { id: 'unity', name: 'Unity Ads', fields: ['organizationId', 'projectId', 'apiKey'] },
  { id: 'applovin', name: 'AppLovin', fields: ['sdkKey', 'reportKey'] },
  { id: 'ironsource', name: 'ironSource', fields: ['secretKey', 'refreshToken'] },
  { id: 'mintegral', name: 'Mintegral', fields: ['apiKey', 'secretKey'] },
  { id: 'facebook', name: 'Facebook Audience Network', fields: ['appId', 'appSecret'] },
]

const API_NETWORKS = ['admob', 'unity'] as const
type ApiNetwork = (typeof API_NETWORKS)[number]
const CSV_NETWORKS = ['admob'] as const
type CsvNetwork = (typeof CSV_NETWORKS)[number]

type IngestionStatus = {
  loading: boolean
  result: NetworkIngestionResult | null
  error: string | null
}

const MANUAL_INGESTION_CONFIG: Record<string, { csv?: boolean; api?: boolean; helperLabel: string }> = {
  admob: {
    csv: true,
    api: true,
    helperLabel: 'Trigger AdMob reporting imports when automated jobs are paused.',
  },
  unity: {
    api: true,
    helperLabel: 'Kick off Unity monetization syncs with stored credentials.',
  },
}

const formatDate = (date: Date) => date.toISOString().slice(0, 10)

const buildDefaultRange = () => {
  const end = new Date()
  const start = new Date()
  start.setDate(end.getDate() - 6)
  return { startDate: formatDate(start), endDate: formatDate(end) }
}

export default function CredentialsPage() {
  const [credentials, setCredentials] = useState<NetworkCredential[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [selectedNetwork, setSelectedNetwork] = useState<string | null>(null)
  const [formData, setFormData] = useState<Record<string, string>>({})
  const [saving, setSaving] = useState(false)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)
  const [ingestionStatus, setIngestionStatus] = useState<Record<string, IngestionStatus>>(() => {
    return Object.keys(MANUAL_INGESTION_CONFIG).reduce<Record<string, IngestionStatus>>((acc, key) => {
      acc[key] = { loading: false, result: null, error: null }
      return acc
    }, {})
  })
  const [manualRanges, setManualRanges] = useState<Record<string, { startDate: string; endDate: string }>>(() => {
    return API_NETWORKS.reduce<Record<string, { startDate: string; endDate: string }>>((acc, key) => {
      acc[key] = buildDefaultRange()
      return acc
    }, {})
  })

  useEffect(() => {
    loadCredentials()
  }, [])

  const hasManualSupport = useMemo(() => {
    return new Set(Object.keys(MANUAL_INGESTION_CONFIG))
  }, [])

  const loadCredentials = async () => {
    try {
      setLoading(true)
      setError(null)
      const response = await byoApi.listCredentials()
      setCredentials(Array.isArray(response.data?.credentials) ? response.data.credentials : [])
    } catch (err: any) {
      setError(err.message || 'Failed to load credentials')
    } finally {
      setLoading(false)
    }
  }

  const handleAddCredentials = (networkId: string) => {
    setSelectedNetwork(networkId)
    const network = SUPPORTED_NETWORKS.find(n => n.id === networkId)
    if (network) {
      const initialData: Record<string, string> = {}
      network.fields.forEach(field => {
        initialData[field] = ''
      })
      setFormData(initialData)
    }
    setError(null)
    setSuccessMessage(null)
  }

  const handleSaveCredentials = async () => {
    if (!selectedNetwork) return

    try {
      setSaving(true)
      setError(null)

      await byoApi.storeCredentials({
        network: selectedNetwork,
        credentials: formData,
      })

      setSuccessMessage('Credentials saved successfully')
      setSelectedNetwork(null)
      setFormData({})
      await loadCredentials()

      setTimeout(() => setSuccessMessage(null), 3000)
    } catch (err: any) {
      setError(err.message || 'Failed to save credentials')
    } finally {
      setSaving(false)
    }
  }

  const handleRotateCredentials = async (network: string) => {
    if (!confirm(`Are you sure you want to rotate credentials for ${network}? The old credentials will be invalidated.`)) {
      return
    }

    const networkConfig = SUPPORTED_NETWORKS.find(n => n.id === network)
    if (!networkConfig) return

    const newCreds: Record<string, string> = {}
    networkConfig.fields.forEach(field => {
      const value = prompt(`Enter new ${field}:`)
      if (value) {
        newCreds[field] = value
      }
    })

    if (Object.keys(newCreds).length === 0) {
      return
    }

    try {
      await byoApi.rotateCredentials(network, newCreds)
      setSuccessMessage('Credentials rotated successfully')
      await loadCredentials()
      setTimeout(() => setSuccessMessage(null), 3000)
    } catch (err: any) {
      setError(err.message || 'Failed to rotate credentials')
    }
  }

  const updateIngestionStatus = (network: string, patch: Partial<IngestionStatus>) => {
    setIngestionStatus((prev) => {
      const current = prev[network] ?? { loading: false, result: null, error: null }
      return {
        ...prev,
        [network]: {
          ...current,
          ...patch,
        },
      }
    })
  }

  const handleCsvIngestion = async (network: CsvNetwork, file: File | null) => {
    if (!file) return
    updateIngestionStatus(network, { loading: true, error: null, result: null })
    try {
      const result = await byoApi.ingestAdmobCsv(file)
      updateIngestionStatus(network, { loading: false, result })
    } catch (err: any) {
      updateIngestionStatus(network, {
        loading: false,
        error: err.message || 'Failed to process CSV report',
      })
    }
  }

  const API_HANDLERS: Record<ApiNetwork, (range: { startDate: string; endDate: string }) => Promise<NetworkIngestionResult>> = {
    admob: (range) => byoApi.ingestAdmobApi(range),
    unity: (range) => byoApi.ingestUnityApi(range),
  }

  const triggerApiIngestion = async (network: ApiNetwork) => {
    const range = manualRanges[network]
    if (!range?.startDate || !range?.endDate) {
      updateIngestionStatus(network, {
        loading: false,
        error: 'Please provide both start and end dates.',
      })
      return
    }

    if (new Date(range.startDate) > new Date(range.endDate)) {
      updateIngestionStatus(network, {
        loading: false,
        error: 'Start date must be before end date.',
      })
      return
    }

    updateIngestionStatus(network, { loading: true, error: null, result: null })
    try {
      const result = await API_HANDLERS[network](range)
      updateIngestionStatus(network, { loading: false, result })
    } catch (err: any) {
      updateIngestionStatus(network, {
        loading: false,
        error: err.message || 'Failed to trigger ingestion run',
      })
    }
  }

  const updateRange = (network: ApiNetwork, field: 'startDate' | 'endDate', value: string) => {
    setManualRanges((prev) => ({
      ...prev,
      [network]: {
        ...prev[network],
        [field]: value,
      },
    }))
  }

  const handleDeleteCredentials = async (network: string) => {
    if (!confirm(`Are you sure you want to delete credentials for ${network}? This action cannot be undone.`)) {
      return
    }

    try {
      await byoApi.deleteCredentials(network)
      setSuccessMessage('Credentials deleted successfully')
      await loadCredentials()
      setTimeout(() => setSuccessMessage(null), 3000)
    } catch (err: any) {
      setError(err.message || 'Failed to delete credentials')
    }
  }

  const hasCredentialsFor = (networkId: string) => {
    return credentials.some(c => c.network === networkId && c.hasCredentials)
  }

  if (loading) {
    return (
      <div className="max-w-4xl mx-auto p-6">
        <div className="animate-pulse space-y-4">
          <div className="h-8 bg-gray-200 rounded w-1/3"></div>
          <div className="h-4 bg-gray-200 rounded w-2/3"></div>
          <div className="space-y-3">
            {[1, 2, 3].map(i => (
              <div key={i} className="h-20 bg-gray-200 rounded"></div>
            ))}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="max-w-4xl mx-auto p-6">
      <div className="mb-8">
        <h1 className="text-3xl font-bold text-gray-900 mb-2">Network Credentials</h1>
        <p className="text-gray-600">
          Securely store your ad network credentials for automatic report ingestion and revenue tracking.
        </p>
      </div>

      {successMessage && (
        <div className="mb-6 p-4 bg-green-50 border border-green-200 rounded-lg flex items-start gap-3">
          <Check className="w-5 h-5 text-green-600 mt-0.5" />
          <p className="text-green-800">{successMessage}</p>
        </div>
      )}

      {error && (
        <div className="mb-6 p-4 bg-red-50 border border-red-200 rounded-lg flex items-start gap-3">
          <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
          <p className="text-red-800">{error}</p>
        </div>
      )}

      <div className="space-y-4">
        {SUPPORTED_NETWORKS.map(network => {
          const hasCredentials = hasCredentialsFor(network.id)
          const credential = credentials.find(c => c.network === network.id)

          return (
            <div
              key={network.id}
              className="bg-white border border-gray-200 rounded-lg p-6 hover:border-gray-300 transition-colors"
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-4">
                  <div className="w-12 h-12 bg-blue-50 rounded-lg flex items-center justify-center">
                    <Key className="w-6 h-6 text-blue-600" />
                  </div>
                  <div>
                    <h3 className="text-lg font-semibold text-gray-900">{network.name}</h3>
                    {hasCredentials && credential && (
                      <p className="text-sm text-gray-500">
                        Last updated: {new Date(credential.updatedAt).toLocaleDateString()}
                      </p>
                    )}
                  </div>
                </div>

                <div className="flex items-center gap-2">
                  {hasCredentials ? (
                    <>
                      <button
                        onClick={() => handleRotateCredentials(network.id)}
                        className="px-4 py-2 text-sm font-medium text-blue-600 hover:text-blue-700 flex items-center gap-2"
                      >
                        <RotateCw className="w-4 h-4" />
                        Rotate
                      </button>
                      <button
                        onClick={() => handleDeleteCredentials(network.id)}
                        className="px-4 py-2 text-sm font-medium text-red-600 hover:text-red-700 flex items-center gap-2"
                      >
                        <Trash2 className="w-4 h-4" />
                        Delete
                      </button>
                    </>
                  ) : (
                    <button
                      onClick={() => handleAddCredentials(network.id)}
                      className="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 flex items-center gap-2"
                    >
                      <Plus className="w-4 h-4" />
                      Add Credentials
                    </button>
                  )}
                </div>
              </div>

              {hasCredentials && hasManualSupport.has(network.id) && (
                <div className="mt-4 border-t border-gray-100 pt-4 space-y-4">
                  <div>
                    <p className="text-sm font-semibold text-gray-900">Manual ingestion helpers</p>
                    <p className="text-xs text-gray-500">
                      {MANUAL_INGESTION_CONFIG[network.id]?.helperLabel}
                    </p>
                  </div>

                  {MANUAL_INGESTION_CONFIG[network.id]?.csv && (
                    <div className="flex flex-col gap-2">
                      <p className="text-sm font-medium text-gray-700">Upload AdMob CSV Export</p>
                      <div className="flex items-center gap-3">
                        <label className="btn btn-outline btn-sm inline-flex items-center gap-2 cursor-pointer">
                          <Upload className="w-4 h-4" />
                          Select CSV
                          <input
                            type="file"
                            accept=".csv"
                            className="sr-only"
                            onChange={(event) => {
                              void handleCsvIngestion('admob', event.target.files?.[0] ?? null)
                              event.target.value = ''
                            }}
                            disabled={ingestionStatus[network.id]?.loading}
                          />
                        </label>
                        {ingestionStatus[network.id]?.loading && (
                          <span className="text-xs text-gray-500">Processing CSV…</span>
                        )}
                      </div>
                    </div>
                  )}

                  {MANUAL_INGESTION_CONFIG[network.id]?.api && (
                    <div className="space-y-2">
                      <p className="text-sm font-medium text-gray-700">Trigger API ingestion window</p>
                      <div className="grid grid-cols-1 md:grid-cols-3 gap-2">
                        <div>
                          <label className="text-xs text-gray-500" htmlFor={`${network.id}-start-date`}>
                            Start Date
                          </label>
                          <input
                            type="date"
                            id={`${network.id}-start-date`}
                            className="input mt-1"
                            value={manualRanges[network.id as ApiNetwork]?.startDate || ''}
                            max={manualRanges[network.id as ApiNetwork]?.endDate}
                            onChange={(e) => updateRange(network.id as ApiNetwork, 'startDate', e.target.value)}
                          />
                        </div>
                        <div>
                          <label className="text-xs text-gray-500" htmlFor={`${network.id}-end-date`}>
                            End Date
                          </label>
                          <input
                            type="date"
                            id={`${network.id}-end-date`}
                            className="input mt-1"
                            value={manualRanges[network.id as ApiNetwork]?.endDate || ''}
                            min={manualRanges[network.id as ApiNetwork]?.startDate}
                            onChange={(e) => updateRange(network.id as ApiNetwork, 'endDate', e.target.value)}
                          />
                        </div>
                        <div className="flex items-end">
                          <button
                            type="button"
                            className="btn btn-primary w-full flex items-center justify-center gap-2"
                            disabled={ingestionStatus[network.id]?.loading}
                            onClick={() => void triggerApiIngestion(network.id as ApiNetwork)}
                          >
                            <Play className="w-4 h-4" />
                            Run Ingestion
                          </button>
                        </div>
                      </div>
                    </div>
                  )}

                  {ingestionStatus[network.id]?.result && (
                    <div className="p-3 bg-green-50 border border-green-200 rounded text-xs text-green-800">
                      Processed {ingestionStatus[network.id]?.result?.rowsProcessed ?? 0} rows between
                      {' '}
                      {ingestionStatus[network.id]?.result?.startDate} – {ingestionStatus[network.id]?.result?.endDate}.{' '}
                      Inserted {ingestionStatus[network.id]?.result?.rowsInserted ?? 0} rows.
                      {ingestionStatus[network.id]?.result?.rowsSkipped
                        ? ` Skipped ${ingestionStatus[network.id]?.result?.rowsSkipped} rows.`
                        : ''}
                      {ingestionStatus[network.id]?.result?.errors.length
                        ? ` Errors: ${ingestionStatus[network.id]?.result?.errors.join(', ')}`
                        : ''}
                    </div>
                  )}

                  {ingestionStatus[network.id]?.error && (
                    <div className="p-3 bg-red-50 border border-red-200 rounded text-xs text-red-700">
                      {ingestionStatus[network.id]?.error}
                    </div>
                  )}
                </div>
              )}
            </div>
          )
        })}
      </div>

      {/* Add/Edit Credentials Modal */}
      {selectedNetwork && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center p-4 z-50">
          <div className="bg-white rounded-lg max-w-md w-full p-6">
            <h2 className="text-xl font-semibold text-gray-900 mb-4">
              Add {SUPPORTED_NETWORKS.find(n => n.id === selectedNetwork)?.name} Credentials
            </h2>

            <div className="space-y-4 mb-6">
              {SUPPORTED_NETWORKS.find(n => n.id === selectedNetwork)?.fields.map(field => (
                <div key={field}>
                  <label className="block text-sm font-medium text-gray-700 mb-1">
                    {field.replace(/([A-Z])/g, ' $1').replace(/^./, str => str.toUpperCase())}
                  </label>
                  <input
                    type="text"
                    value={formData[field] || ''}
                    onChange={e => setFormData({ ...formData, [field]: e.target.value })}
                    className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                    placeholder={`Enter ${field}`}
                  />
                </div>
              ))}
            </div>

            <div className="flex gap-3">
              <button
                onClick={() => {
                  setSelectedNetwork(null)
                  setFormData({})
                  setError(null)
                }}
                disabled={saving}
                className="flex-1 px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 rounded-lg hover:bg-gray-200 disabled:opacity-50"
              >
                Cancel
              </button>
              <button
                onClick={handleSaveCredentials}
                disabled={saving || Object.values(formData).some(v => !v)}
                className="flex-1 px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {saving ? 'Saving...' : 'Save Credentials'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
