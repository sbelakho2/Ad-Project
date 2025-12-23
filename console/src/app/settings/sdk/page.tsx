'use client'

import { useState, useEffect, useCallback } from 'react'
import {
  Hash,
  RefreshCw,
  CheckCircle,
  AlertTriangle,
  Copy,
  ExternalLink,
  Smartphone,
  Tv,
  Monitor,
  Gamepad2,
  Globe,
} from 'lucide-react'

interface SDKConfig {
  appId: string
  version: number
  floors: Record<string, number>
  caps: Record<string, { daily: number; hourly: number }>
  pacing: {
    enabled: boolean
    minIntervalMs: number
  }
  networks: string[]
  features: {
    bidding: boolean
    waterfall: boolean
    bannerRefresh: boolean
    bannerRefreshIntervalMs: number
  }
  compliance: {
    gdprApplies: boolean
    ccpaApplies: boolean
    coppaApplies: boolean
  }
  updatedAt: string
}

interface SDKConfigResponse {
  config: SDKConfig
  hash: string
}

interface ValidationResult {
  valid: boolean
  serverHash: string
  mismatch?: {
    field: string
    serverValue: unknown
    clientValue?: unknown
  }[]
}

const SDK_PLATFORMS = [
  { id: 'android', name: 'Android', icon: Smartphone },
  { id: 'ios', name: 'iOS', icon: Smartphone },
  { id: 'android-tv', name: 'Android TV', icon: Tv },
  { id: 'tvos', name: 'tvOS', icon: Tv },
  { id: 'unity', name: 'Unity', icon: Gamepad2 },
  { id: 'web', name: 'Web', icon: Globe },
] as const

export default function SDKConfigPage() {
  const [appId, setAppId] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [configResponse, setConfigResponse] = useState<SDKConfigResponse | null>(null)
  const [validationResult, setValidationResult] = useState<ValidationResult | null>(null)
  const [clientHash, setClientHash] = useState('')
  const [copied, setCopied] = useState(false)
  const [selectedPlatform, setSelectedPlatform] = useState<string>('android')

  const fetchConfig = useCallback(async () => {
    if (!appId.trim()) {
      setError('Please enter an App ID')
      return
    }

    setLoading(true)
    setError(null)
    setValidationResult(null)

    try {
      const response = await fetch(`/api/v1/config/sdk/config?appId=${encodeURIComponent(appId)}`)
      
      if (!response.ok) {
        throw new Error(`Failed to fetch config: ${response.statusText}`)
      }

      const data: SDKConfigResponse = await response.json()
      setConfigResponse(data)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to fetch configuration')
    } finally {
      setLoading(false)
    }
  }, [appId])

  const validateHash = useCallback(async () => {
    if (!appId.trim() || !clientHash.trim()) {
      setError('Please enter both App ID and client hash')
      return
    }

    setLoading(true)
    setError(null)

    try {
      const response = await fetch('/api/v1/config/sdk/config/validate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ appId, clientHash }),
      })

      if (!response.ok) {
        throw new Error(`Validation failed: ${response.statusText}`)
      }

      const result: ValidationResult = await response.json()
      setValidationResult(result)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Validation failed')
    } finally {
      setLoading(false)
    }
  }, [appId, clientHash])

  const copyToClipboard = useCallback(async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Fallback for older browsers
      const textarea = document.createElement('textarea')
      textarea.value = text
      document.body.appendChild(textarea)
      textarea.select()
      document.execCommand('copy')
      document.body.removeChild(textarea)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    }
  }, [])

  const generateCodeSnippet = useCallback((platform: string, hash: string) => {
    switch (platform) {
      case 'android':
      case 'android-tv':
        return `// Kotlin - Add to your Application class
val configHash = mediationSDK.getConfigHash()
Log.d("SDK", "Config hash: $configHash")

// Validate against server
if (configHash != "${hash}") {
    Log.w("SDK", "Config mismatch detected!")
}`
      case 'ios':
      case 'tvos':
        return `// Swift - Add to AppDelegate
let configHash = MediationSDK.shared.getConfigHash()
print("Config hash: \\(configHash)")

// Validate against server
if configHash != "${hash}" {
    print("Config mismatch detected!")
}`
      case 'unity':
        return `// C# - Unity
string configHash = MediationSDK.GetConfigHash();
Debug.Log($"Config hash: {configHash}");

// Validate against server
if (configHash != "${hash}") {
    Debug.LogWarning("Config mismatch detected!");
}`
      case 'web':
        return `// JavaScript/TypeScript
const configHash = mediationSDK.getConfigHash();
console.log('Config hash:', configHash);

// Validate against server
if (configHash !== '${hash}') {
    console.warn('Config mismatch detected!');
}`
      default:
        return '// Unknown platform'
    }
  }, [])

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-6xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <div className="flex items-center gap-3">
            <div className="h-10 w-10 rounded-lg bg-primary-50 text-primary-600 flex items-center justify-center">
              <Hash className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm font-medium text-primary-600">Settings</p>
              <h1 className="text-2xl font-semibold text-gray-900 tracking-tight">
                SDK Configuration Parity
              </h1>
            </div>
          </div>
          <p className="text-sm text-gray-600 mt-3 max-w-3xl">
            Verify that SDK configurations are synchronized between the Console and your app.
            Compare config hashes to detect mismatches and ensure consistency.
          </p>
        </div>
      </header>

      <main className="max-w-6xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-8">
        {/* App ID Input */}
        <div className="card">
          <h2 className="text-lg font-semibold text-gray-900 mb-4">Fetch Configuration</h2>
          <div className="flex gap-4">
            <div className="flex-1">
              <label htmlFor="appId" className="block text-sm font-medium text-gray-700 mb-1">
                App ID
              </label>
              <input
                id="appId"
                type="text"
                value={appId}
                onChange={(e) => setAppId(e.target.value)}
                placeholder="com.example.myapp"
                className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500"
              />
            </div>
            <div className="flex items-end">
              <button
                onClick={fetchConfig}
                disabled={loading}
                className="px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700 disabled:opacity-50 flex items-center gap-2"
              >
                {loading ? (
                  <RefreshCw className="h-4 w-4 animate-spin" />
                ) : (
                  <RefreshCw className="h-4 w-4" />
                )}
                Fetch Config
              </button>
            </div>
          </div>

          {error && (
            <div className="mt-4 p-3 bg-red-50 border border-red-200 rounded-lg text-red-700 text-sm">
              {error}
            </div>
          )}
        </div>

        {/* Config Display */}
        {configResponse && (
          <div className="card">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-lg font-semibold text-gray-900">Server Configuration</h2>
              <span className="text-xs text-gray-500">
                Version {configResponse.config.version} • Updated{' '}
                {new Date(configResponse.config.updatedAt).toLocaleString()}
              </span>
            </div>

            {/* Config Hash */}
            <div className="bg-gray-50 border border-gray-200 rounded-lg p-4 mb-6">
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-sm font-medium text-gray-700">Configuration Hash</p>
                  <p className="font-mono text-sm text-gray-900 mt-1 break-all">
                    {configResponse.hash}
                  </p>
                </div>
                <button
                  onClick={() => copyToClipboard(configResponse.hash)}
                  className="p-2 hover:bg-gray-200 rounded-lg transition-colors"
                  title="Copy hash"
                >
                  {copied ? (
                    <CheckCircle className="h-5 w-5 text-green-600" />
                  ) : (
                    <Copy className="h-5 w-5 text-gray-600" />
                  )}
                </button>
              </div>
            </div>

            {/* Config Details */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
              <div>
                <h3 className="text-sm font-medium text-gray-700 mb-2">Networks</h3>
                <div className="flex flex-wrap gap-2">
                  {configResponse.config.networks.map((network) => (
                    <span
                      key={network}
                      className="px-2 py-1 bg-blue-100 text-blue-800 text-xs rounded-full"
                    >
                      {network}
                    </span>
                  ))}
                </div>
              </div>

              <div>
                <h3 className="text-sm font-medium text-gray-700 mb-2">Features</h3>
                <div className="space-y-1 text-sm">
                  <p>
                    Bidding:{' '}
                    <span className={configResponse.config.features.bidding ? 'text-green-600' : 'text-gray-500'}>
                      {configResponse.config.features.bidding ? 'Enabled' : 'Disabled'}
                    </span>
                  </p>
                  <p>
                    Waterfall:{' '}
                    <span className={configResponse.config.features.waterfall ? 'text-green-600' : 'text-gray-500'}>
                      {configResponse.config.features.waterfall ? 'Enabled' : 'Disabled'}
                    </span>
                  </p>
                  <p>
                    Banner Refresh:{' '}
                    <span className={configResponse.config.features.bannerRefresh ? 'text-green-600' : 'text-gray-500'}>
                      {configResponse.config.features.bannerRefresh
                        ? `${configResponse.config.features.bannerRefreshIntervalMs / 1000}s`
                        : 'Disabled'}
                    </span>
                  </p>
                </div>
              </div>

              <div>
                <h3 className="text-sm font-medium text-gray-700 mb-2">Pacing</h3>
                <div className="text-sm">
                  <p>
                    Status:{' '}
                    <span className={configResponse.config.pacing.enabled ? 'text-green-600' : 'text-gray-500'}>
                      {configResponse.config.pacing.enabled ? 'Enabled' : 'Disabled'}
                    </span>
                  </p>
                  {configResponse.config.pacing.enabled && (
                    <p>
                      Min Interval: {configResponse.config.pacing.minIntervalMs / 1000}s
                    </p>
                  )}
                </div>
              </div>

              <div>
                <h3 className="text-sm font-medium text-gray-700 mb-2">Compliance</h3>
                <div className="space-y-1 text-sm">
                  <p>
                    GDPR:{' '}
                    <span className={configResponse.config.compliance.gdprApplies ? 'text-amber-600' : 'text-gray-500'}>
                      {configResponse.config.compliance.gdprApplies ? 'Applies' : 'N/A'}
                    </span>
                  </p>
                  <p>
                    CCPA:{' '}
                    <span className={configResponse.config.compliance.ccpaApplies ? 'text-amber-600' : 'text-gray-500'}>
                      {configResponse.config.compliance.ccpaApplies ? 'Applies' : 'N/A'}
                    </span>
                  </p>
                  <p>
                    COPPA:{' '}
                    <span className={configResponse.config.compliance.coppaApplies ? 'text-amber-600' : 'text-gray-500'}>
                      {configResponse.config.compliance.coppaApplies ? 'Applies' : 'N/A'}
                    </span>
                  </p>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* SDK Integration Code */}
        {configResponse && (
          <div className="card">
            <h2 className="text-lg font-semibold text-gray-900 mb-4">SDK Integration</h2>
            
            {/* Platform Selector */}
            <div className="flex flex-wrap gap-2 mb-4">
              {SDK_PLATFORMS.map((platform) => {
                const Icon = platform.icon
                return (
                  <button
                    key={platform.id}
                    onClick={() => setSelectedPlatform(platform.id)}
                    className={`flex items-center gap-2 px-3 py-2 rounded-lg transition-colors ${
                      selectedPlatform === platform.id
                        ? 'bg-primary-100 text-primary-700 border-2 border-primary-500'
                        : 'bg-gray-100 text-gray-700 border-2 border-transparent hover:bg-gray-200'
                    }`}
                  >
                    <Icon className="h-4 w-4" />
                    {platform.name}
                  </button>
                )
              })}
            </div>

            {/* Code Snippet */}
            <div className="bg-gray-900 rounded-lg p-4 overflow-x-auto">
              <pre className="text-sm text-gray-100 font-mono whitespace-pre">
                {generateCodeSnippet(selectedPlatform, configResponse.hash)}
              </pre>
            </div>
          </div>
        )}

        {/* Hash Validation */}
        <div className="card">
          <h2 className="text-lg font-semibold text-gray-900 mb-4">Validate Client Hash</h2>
          <p className="text-sm text-gray-600 mb-4">
            Paste the config hash from your SDK to verify it matches the server configuration.
          </p>
          
          <div className="flex gap-4">
            <div className="flex-1">
              <label htmlFor="clientHash" className="block text-sm font-medium text-gray-700 mb-1">
                Client Hash
              </label>
              <input
                id="clientHash"
                type="text"
                value={clientHash}
                onChange={(e) => setClientHash(e.target.value)}
                placeholder="v1:abc123..."
                className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500 font-mono text-sm"
              />
            </div>
            <div className="flex items-end">
              <button
                onClick={validateHash}
                disabled={loading || !appId.trim()}
                className="px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700 disabled:opacity-50 flex items-center gap-2"
              >
                Validate
              </button>
            </div>
          </div>

          {/* Validation Result */}
          {validationResult && (
            <div
              className={`mt-4 p-4 rounded-lg border ${
                validationResult.valid
                  ? 'bg-green-50 border-green-200'
                  : 'bg-red-50 border-red-200'
              }`}
            >
              <div className="flex items-center gap-2">
                {validationResult.valid ? (
                  <>
                    <CheckCircle className="h-5 w-5 text-green-600" />
                    <span className="font-medium text-green-700">Configuration matches!</span>
                  </>
                ) : (
                  <>
                    <AlertTriangle className="h-5 w-5 text-red-600" />
                    <span className="font-medium text-red-700">Configuration mismatch detected</span>
                  </>
                )}
              </div>

              {!validationResult.valid && validationResult.mismatch && (
                <div className="mt-3">
                  <p className="text-sm font-medium text-red-700 mb-2">Mismatched fields:</p>
                  <ul className="space-y-1">
                    {validationResult.mismatch.map((m, i) => (
                      <li key={i} className="text-sm text-red-600">
                        <span className="font-mono">{m.field}</span>: Server has{' '}
                        <span className="font-mono">{JSON.stringify(m.serverValue)}</span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              <p className="text-xs text-gray-600 mt-3 font-mono">
                Server hash: {validationResult.serverHash}
              </p>
            </div>
          )}
        </div>

        {/* Documentation Link */}
        <div className="card bg-gradient-to-br from-primary-50 to-blue-50">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="font-semibold text-gray-900">SDK Documentation</h3>
              <p className="text-sm text-gray-600 mt-1">
                Learn more about SDK configuration parity and best practices.
              </p>
            </div>
            <a
              href="/docs/sdk/config-parity"
              className="flex items-center gap-2 px-4 py-2 bg-white border border-gray-200 rounded-lg hover:bg-gray-50 transition-colors"
            >
              <span className="text-sm font-medium">View Docs</span>
              <ExternalLink className="h-4 w-4" />
            </a>
          </div>
        </div>
      </main>
    </div>
  )
}
