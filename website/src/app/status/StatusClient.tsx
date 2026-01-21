/* eslint-disable react/no-unescaped-entities */
'use client'

import Footer from '@/components/Footer'
import CookieBanner from '@/components/CookieBanner'
import { useCallback, useEffect, useMemo, useState } from 'react'

type ServiceStatus = 'operational' | 'degraded' | 'down' | 'checking'

type ServiceCheck = {
  name: string
  status: 'operational' | 'degraded' | 'down'
  latencyMs: number | null
  error?: string
}

type StatusService = {
  name: string
  description: string
  expected: string
  check: string
  openUrl?: string
}

const SERVICES: StatusService[] = [
  {
    name: 'Website',
    description: 'Marketing site and docs pages.',
    expected: 'https://apexmediation.ee',
    openUrl: 'https://apexmediation.ee',
    check: '/api/health',
  },
  {
    name: 'API',
    description: 'Core platform APIs and auction endpoints.',
    expected: 'https://api.apexmediation.ee',
    openUrl: 'https://api.apexmediation.ee/health',
    check: '/health',
  },
  {
    name: 'Console',
    description: 'Customer console and operator control plane.',
    expected: 'https://console.apexmediation.ee',
    openUrl: 'https://console.apexmediation.ee',
    check: '/api/health',
  },
]

export default function StatusClient() {
  const [serviceStatuses, setServiceStatuses] = useState<Record<string, { status: ServiceStatus; latencyMs?: number | null; error?: string }>>({
    Website: { status: 'checking' },
    API: { status: 'checking' },
    Console: { status: 'checking' },
  })
  const [lastChecked, setLastChecked] = useState<string | null>(null)

  const checkAllServices = useCallback(async () => {
    try {
      const res = await fetch('/api/public/services', {
        cache: 'no-store',
        headers: { accept: 'application/json' },
      })

      if (res.ok) {
        const data = await res.json()
        const newStatuses: Record<string, { status: ServiceStatus; latencyMs?: number | null; error?: string }> = {}

        data.services?.forEach((svc: ServiceCheck) => {
          newStatuses[svc.name] = {
            status: svc.status,
            latencyMs: svc.latencyMs,
            error: svc.error,
          }
        })

        setServiceStatuses(newStatuses)
        setLastChecked(data.timestamp || new Date().toISOString())
      }
    } catch {
      // Keep existing statuses on error
    }
  }, [])

  useEffect(() => {
    checkAllServices()

    // Refresh every 30 seconds
    const interval = setInterval(checkAllServices, 30000)
    return () => clearInterval(interval)
  }, [checkAllServices])

  const overallStatus = useMemo(() => {
    const statuses = Object.values(serviceStatuses).map((s) => s.status)
    if (statuses.some((s) => s === 'checking')) return 'checking'
    if (statuses.every((s) => s === 'operational')) return 'operational'
    if (statuses.some((s) => s === 'down')) return 'down'
    return 'degraded'
  }, [serviceStatuses])

  const liveBadge = useMemo(() => {
    if (overallStatus === 'checking') {
      return { className: 'bg-gray-50 border-gray-200 text-inkMuted', label: 'Checking…', dot: 'bg-gray-400', lastChecked }
    }
    if (overallStatus === 'operational') {
      return { className: 'bg-success/10 border-success/20 text-success', label: 'All Systems Operational', dot: 'bg-success', lastChecked }
    }
    if (overallStatus === 'degraded') {
      return { className: 'bg-warning/10 border-warning/20 text-warning', label: 'Partial Outage', dot: 'bg-warning', lastChecked }
    }
    return { className: 'bg-danger/10 border-danger/20 text-danger', label: 'Major Outage', dot: 'bg-danger', lastChecked }
  }, [overallStatus, lastChecked])

  const getServiceBadge = (svcName: string) => {
    const svcState = serviceStatuses[svcName]
    const status = svcState?.status || 'checking'

    if (status === 'checking') {
      return { dot: 'bg-gray-400 animate-pulse', pill: 'text-inkMuted bg-gray-50 border border-gray-200', label: 'Checking…', latencyMs: null, error: null }
    }
    if (status === 'operational') {
      return { dot: 'bg-success', pill: 'text-success bg-success/10 border border-success/20', label: 'Operational', latencyMs: svcState?.latencyMs, error: null }
    }
    if (status === 'degraded') {
      return { dot: 'bg-warning', pill: 'text-warning bg-warning/10 border border-warning/20', label: 'Degraded', latencyMs: svcState?.latencyMs, error: svcState?.error }
    }
    return { dot: 'bg-danger', pill: 'text-danger bg-danger/10 border border-danger/20', label: 'Down', latencyMs: svcState?.latencyMs, error: svcState?.error }
  }

  return (
    <>
      <main className="min-h-screen bg-white">
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <div className={`inline-flex items-center gap-2 rounded-full border px-4 py-2 text-sm ${liveBadge.className}`}>
                <span className="font-mono">STATUS</span>
                <span className="text-gray-400">/</span>
                <span>{liveBadge.label}</span>
                <span className={`ml-1 h-2 w-2 rounded-full ${liveBadge.dot}`} aria-hidden="true" />
              </div>

              <h1 className="mt-6 text-4xl md:text-5xl font-bold text-gray-900">System Status</h1>
              <p className="mt-4 text-lg text-gray-600 max-w-xl">
                Real-time status of ApexMediation services. Checks refresh automatically every 30 seconds.
              </p>
              {lastChecked ? (
                <p className="mt-3 text-sm text-gray-500 font-mono">
                  Last checked: {new Date(lastChecked).toLocaleTimeString()} 
                  <button 
                    onClick={checkAllServices}
                    className="ml-3 text-brand hover:underline"
                  >
                    Refresh now
                  </button>
                </p>
              ) : null}

              <div className="mt-10 grid gap-4">
                {SERVICES.map((svc) => {
                  const badge = getServiceBadge(svc.name)
                  return (
                    <div key={svc.name} className="rounded-xl border border-gray-100 bg-white p-6 shadow-sm">
                      <div className="flex flex-wrap items-center justify-between gap-4">
                        <div>
                          <h2 className="text-lg font-semibold text-gray-900">{svc.name}</h2>
                          <p className="mt-1 text-sm text-gray-600">{svc.description}</p>
                        </div>
                        <div className="flex items-center gap-2">
                          <span className={`h-2 w-2 rounded-full ${badge.dot}`} aria-hidden="true" />
                          <span className={`inline-flex items-center rounded-full px-3 py-1 text-sm font-medium ${badge.pill}`}>{badge.label}</span>
                          {badge.latencyMs !== null && badge.latencyMs !== undefined && (
                            <span className="text-xs text-gray-400 font-mono">{badge.latencyMs}ms</span>
                          )}
                        </div>
                      </div>

                      {badge.error && (
                        <div className="mt-3 rounded-lg bg-danger/5 border border-danger/20 px-3 py-2 text-sm text-danger">
                          {badge.error}
                        </div>
                      )}

                      <div className="mt-4 grid gap-2 text-sm text-gray-600">
                        <p>
                          <span className="font-mono text-gray-500">Expected:</span> {svc.expected}
                        </p>
                        <p>
                          <span className="font-mono text-gray-500">Check:</span> {svc.check}
                        </p>
                      </div>

                      {svc.openUrl ? (
                        <div className="mt-4">
                          <a
                            className="inline-flex items-center rounded-lg border border-gray-200 bg-white px-3 py-2 text-sm font-medium text-gray-900 hover:bg-gray-50"
                            href={svc.openUrl}
                            target="_blank"
                            rel="noreferrer"
                          >
                            Open
                          </a>
                        </div>
                      ) : null}
                    </div>
                  )
                })}
              </div>
            </div>
          </div>
        </section>

        <Footer />
      </main>

      <CookieBanner />
    </>
  )
}
