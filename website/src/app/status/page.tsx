import Footer from '@/components/Footer'
import CookieBanner from '@/components/CookieBanner'
import { getLKG, saveLKG } from '@/lib/lkg'

export const metadata = {
  title: 'Status — ApexMediation',
  description: 'Service status for ApexMediation: website, API, and console.',
}

type LiveStatus = {
  status: 'operational' | 'degraded' | 'down' | 'unknown'
  service?: string
  timestamp?: string
}

const SERVICES = [
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

function resolveBadge(ls: LiveStatus | null) {
  const status = (ls?.status ?? 'unknown') as LiveStatus['status']
  const lastChecked = typeof ls?.timestamp === 'string' ? ls!.timestamp : null

  if (status === 'operational') {
    return { className: 'bg-green-50 border-green-100 text-green-700', label: 'Operational', dot: 'bg-green-500', lastChecked }
  }
  if (status === 'degraded') {
    return { className: 'bg-amber-50 border-amber-100 text-amber-700', label: 'Degraded', dot: 'bg-amber-500', lastChecked }
  }
  if (status === 'down') {
    return { className: 'bg-red-50 border-red-100 text-red-700', label: 'Down', dot: 'bg-red-500', lastChecked }
  }
  return { className: 'bg-gray-50 border-gray-200 text-gray-700', label: 'Unknown', dot: 'bg-gray-400', lastChecked }
}

export default async function StatusPage() {
  const key = 'status:rollup'
  let live: LiveStatus | null = null
  let usedLkg = false

  try {
    const res = await fetch(`${process.env.NEXT_PUBLIC_BASE_URL ?? ''}/api/public/status`, {
      // Don’t cache HTML errors, and allow ISR at route-handler layer
      cache: 'no-store',
      headers: { accept: 'application/json' },
    })
    if (res.ok) {
      const data = (await res.json()) as LiveStatus
      live = data
      saveLKG(key, data)
    } else {
      const snap = getLKG<LiveStatus>(key)
      if (snap) {
        live = { ...(snap.payload || null) } as LiveStatus
        usedLkg = true
      }
    }
  } catch {
    const snap = getLKG<LiveStatus>(key)
    if (snap) {
      live = { ...(snap.payload || null) } as LiveStatus
      usedLkg = true
    }
  }

  const badge = resolveBadge(live)

  return (
    <>
      <main className="min-h-screen bg-white">
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <div className={`inline-flex items-center gap-2 rounded-full border px-4 py-2 text-sm ${badge.className}`}>
                <span className="font-mono">STATUS</span>
                <span className="text-gray-400">/</span>
                <span>Backend: {badge.label}</span>
                <span className={`ml-1 h-2 w-2 rounded-full ${badge.dot}`} aria-hidden="true" />
              </div>

              <h1 className="mt-6 text-4xl md:text-5xl font-bold text-gray-900">Status</h1>
              <p className="mt-4 text-lg text-gray-600 max-w-xl">
                Current platform availability and where to check deeper diagnostics.
              </p>
              {badge.lastChecked ? (
                <p className="mt-3 text-sm text-gray-500 font-mono">Last checked: {badge.lastChecked}{usedLkg ? ' (cached)' : ''}</p>
              ) : usedLkg ? (
                <p className="mt-3 text-sm text-gray-500 font-mono">Temporarily unavailable — showing last known good snapshot.</p>
              ) : (
                <p className="mt-3 text-sm text-gray-500 font-mono">Temporarily unavailable. See incidents or contact support.</p>
              )}

              <div className="mt-10 grid gap-4">
                {SERVICES.map((svc) => {
                  const dot = badge.label === 'Operational' ? 'bg-green-500' : badge.label === 'Degraded' ? 'bg-amber-500' : badge.label === 'Down' ? 'bg-red-500' : 'bg-gray-400'
                  const pill = dot === 'bg-green-500' ? 'text-green-700 bg-green-50 border border-green-100' : dot === 'bg-amber-500' ? 'text-amber-700 bg-amber-50 border border-amber-100' : dot === 'bg-red-500' ? 'text-red-700 bg-red-50 border border-red-100' : 'text-gray-700 bg-gray-50 border border-gray-200'
                  const label = dot === 'bg-green-500' ? 'Operational' : dot === 'bg-amber-500' ? 'Degraded' : dot === 'bg-red-500' ? 'Down' : 'Unknown'
                  return (
                    <div key={svc.name} className="rounded-xl border border-gray-100 bg-white p-6 shadow-sm">
                      <div className="flex flex-wrap items-center justify-between gap-4">
                        <div>
                          <h2 className="text-lg font-semibold text-gray-900">{svc.name}</h2>
                          <p className="mt-1 text-sm text-gray-600">{svc.description}</p>
                        </div>
                        <div className="flex items-center gap-2">
                          <span className={`h-2 w-2 rounded-full ${dot}`} aria-hidden="true" />
                          <span className={`inline-flex items-center rounded-full px-3 py-1 text-sm font-medium ${pill}`}>{label}</span>
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
