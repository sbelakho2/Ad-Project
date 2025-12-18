/* eslint-disable react/no-unescaped-entities */
'use client'

import Footer from '@/components/Footer'
import CookieBanner from '@/components/CookieBanner'
import { useEffect, useMemo, useState } from 'react'

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
    check: '/healthz (edge)',
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
    check: '/ (web)',
  },
]

export default function StatusClient() {
  const [liveStatus, setLiveStatus] = useState<any | null>(null)

  useEffect(() => {
    const controller = new AbortController()

    fetch('/api/public/status', {
      signal: controller.signal,
      headers: { accept: 'application/json' },
    })
      .then((res) => res.json())
      .then((data) => {
        if (data && typeof data === 'object') {
          setLiveStatus(data)
        }
      })
      .catch(() => {
        // Fail open; page should render even without live status.
      })

    return () => controller.abort()
  }, [])

  const liveBadge = useMemo(() => {
    const status = (liveStatus?.status || 'unknown') as string
    const lastChecked = typeof liveStatus?.timestamp === 'string' ? liveStatus.timestamp : null

    if (status === 'operational') {
      return { className: 'bg-success/10 border-success/20 text-success', label: 'Operational', dot: 'bg-success', lastChecked }
    }
    if (status === 'degraded') {
      return { className: 'bg-warning/10 border-warning/20 text-warning', label: 'Degraded', dot: 'bg-warning', lastChecked }
    }
    if (status === 'down') {
      return { className: 'bg-danger/10 border-danger/20 text-danger', label: 'Down', dot: 'bg-danger', lastChecked }
    }
    return { className: 'bg-gray-50 border-gray-200 text-inkMuted', label: liveStatus ? 'Unknown' : 'Loading…', dot: 'bg-gray-400', lastChecked }
  }, [liveStatus])

  const getServiceBadge = (svcName: string) => {
    if (svcName !== 'API') {
      return { dot: 'bg-success', pill: 'text-success bg-success/10 border border-success/20', label: 'Operational' }
    }

    if (liveBadge.label === 'Operational') {
      return { dot: 'bg-success', pill: 'text-success bg-success/10 border border-success/20', label: 'Operational' }
    }
    if (liveBadge.label === 'Degraded') {
      return { dot: 'bg-warning', pill: 'text-warning bg-warning/10 border border-warning/20', label: 'Degraded' }
    }
    if (liveBadge.label === 'Down') {
      return { dot: 'bg-danger', pill: 'text-danger bg-danger/10 border border-danger/20', label: 'Down' }
    }

    return { dot: 'bg-gray-400', pill: 'text-inkMuted bg-gray-50 border border-gray-200', label: 'Unknown' }
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
                <span>Backend: {liveBadge.label}</span>
                <span className={`ml-1 h-2 w-2 rounded-full ${liveBadge.dot}`} aria-hidden="true" />
              </div>

              <h1 className="mt-6 text-4xl md:text-5xl font-bold text-gray-900">Status</h1>
              <p className="mt-4 text-lg text-gray-600 max-w-xl">
                Current platform availability and where to check deeper diagnostics.
              </p>
              {liveBadge.lastChecked ? (
                <p className="mt-3 text-sm text-gray-500 font-mono">Last checked: {liveBadge.lastChecked}</p>
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
                        </div>
                      </div>

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
