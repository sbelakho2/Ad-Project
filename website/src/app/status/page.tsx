import Footer from '@/components/Footer'
import CookieBanner from '@/components/CookieBanner'

const SERVICES = [
  {
    name: 'Website',
    description: 'Marketing site and docs pages.',
    expected: 'https://apexmediation.ee',
    check: '/healthz (edge)',
  },
  {
    name: 'API',
    description: 'Core platform APIs and auction endpoints.',
    expected: 'https://api.apexmediation.ee',
    check: '/health',
  },
  {
    name: 'Console',
    description: 'Customer console and operator control plane.',
    expected: 'https://console.apexmediation.ee',
    check: '/ (web)',
  },
] as const

export const metadata = {
  title: 'Status — ApexMediation',
  description: 'Service status for ApexMediation: website, API, and console.',
}

export default function StatusPage() {
  return (
    <>
      <main className="min-h-screen bg-white">
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <div className="inline-flex items-center gap-2 rounded-full border border-gray-200 bg-gray-50 px-4 py-2 text-sm text-gray-700">
                <span className="font-mono">STATUS</span>
                <span className="text-gray-400">/</span>
                <span>Service health overview</span>
              </div>

              <h1 className="mt-6 text-4xl md:text-5xl font-bold text-gray-900">Status</h1>
              <p className="mt-4 text-lg text-gray-600 max-w-xl">
                Current platform availability and where to check deeper diagnostics.
              </p>

              <div className="mt-10 grid gap-4">
                {SERVICES.map((svc) => (
                  <div key={svc.name} className="rounded-xl border border-gray-200 bg-white p-6">
                    <div className="flex flex-col md:flex-row md:items-start md:justify-between gap-4">
                      <div className="space-y-2">
                        <div className="flex items-center gap-3">
                          <div className="h-3 w-3 rounded-full bg-green-500" aria-hidden="true" />
                          <h2 className="text-lg font-semibold text-gray-900">{svc.name}</h2>
                          <span className="text-sm text-green-700 bg-green-50 border border-green-100 rounded-full px-3 py-1">
                            Operational
                          </span>
                        </div>
                        <p className="text-sm text-gray-600 max-w-prose">{svc.description}</p>
                        <div className="text-sm text-gray-700">
                          <span className="font-medium">Endpoint:</span> <span className="font-mono">{svc.expected}</span>
                        </div>
                        <div className="text-sm text-gray-500">
                          <span className="font-medium text-gray-600">Check:</span> <span className="font-mono">{svc.check}</span>
                        </div>
                      </div>

                      <div className="shrink-0">
                        <a
                          href={svc.expected}
                          className="btn-secondary text-sm"
                        >
                          Open
                        </a>
                      </div>
                    </div>
                  </div>
                ))}
              </div>

              <div className="mt-10 grid grid-cols-1 md:grid-cols-2 gap-4">
                <div className="rounded-xl border border-gray-200 bg-gray-50 p-6">
                  <h3 className="text-base font-semibold text-gray-900">Verification guide</h3>
                  <p className="mt-2 text-sm text-gray-600 max-w-prose">
                    How to verify signed logs and reproduce decisions.
                  </p>
                  <a href="/docs/verification" className="btn-primary mt-4 inline-flex px-5 py-2 text-sm">
                    View verification docs
                  </a>
                </div>
                <div className="rounded-xl border border-gray-200 bg-gray-50 p-6">
                  <h3 className="text-base font-semibold text-gray-900">SLOs & transparency</h3>
                  <p className="mt-2 text-sm text-gray-600 max-w-prose">
                    What we measure, how we alert, and what we consider an incident.
                  </p>
                  <a href="/slo" className="btn-secondary mt-4 inline-flex px-5 py-2 text-sm">
                    Review SLO page
                  </a>
                </div>
              </div>

              <div className="mt-12 rounded-2xl border border-gray-200 bg-white p-6">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                  <div>
                    <h3 className="text-base font-semibold text-gray-900">Visual Card</h3>
                    <p className="mt-2 text-sm text-gray-600 max-w-prose">
                      A quick mental model for how to validate the full request path.
                    </p>
                  </div>
                  <pre className="rounded-xl border border-gray-200 bg-gray-50 p-4 text-xs text-gray-800 overflow-auto">
{`Client
  |  https://apexmediation.ee
  v
Edge / CDN
  |  /api/*
  v
API  https://api.apexmediation.ee
  |  decision logs
  v
Console  https://console.apexmediation.ee
  |  audit + export
  v
Signed evidence bundle`}
                  </pre>
                </div>
              </div>

              <p className="mt-10 text-xs text-gray-500">
                Note: This page is an overview. For incident comms and postmortems, use the linked docs and SLO pages.
              </p>
            </div>
          </div>
        </section>

        <Footer />
      </main>
      <CookieBanner />
    </>
  )
}
