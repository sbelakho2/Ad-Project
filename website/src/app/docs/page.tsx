import Footer from '@/components/Footer'
import CookieBanner from '@/components/CookieBanner'

export const metadata = {
  title: 'Docs — ApexMediation',
  description: 'Documentation for ApexMediation: verification and playback compatibility.',
}

const DOCS = [
  {
    title: 'Verification',
    description: 'Verify signed logs, reproduce decisions, and export incident bundles.',
    href: '/docs/verification',
    card: `VERIFY\n-----\n1) Fetch evidence bundle\n2) Verify signature\n3) Recompute decision\n4) Compare hashes`,
  },
  {
    title: 'Playback Matrix',
    description: 'Supported players and devices for Web Video and CTV delivery.',
    href: '/docs/playback-matrix',
    card: `PLAYBACK\n--------\nWeb:  HLS/DASH\nCTV:  tvOS/Android TV\nDRM:  policy-based\nSSAI: supported`,
  },
] as const

export default function DocsLandingPage() {
  return (
    <>
      <main className="min-h-screen bg-white">
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-5xl mx-auto">
              <div className="inline-flex items-center gap-2 rounded-full border border-gray-200 bg-gray-50 px-4 py-2 text-sm text-gray-700">
                <span className="font-mono">DOCS</span>
                <span className="text-gray-400">/</span>
                <span>Getting started</span>
              </div>

              <h1 className="mt-6 text-4xl md:text-5xl font-bold text-gray-900">Documentation</h1>
              <p className="mt-4 text-lg text-gray-600 max-w-xl">
                Practical guides for verification, SSAI, and compatibility.
              </p>

              <div className="mt-10 grid grid-cols-1 md:grid-cols-2 gap-6">
                {DOCS.map((doc) => (
                  <div key={doc.title} className="rounded-2xl border border-gray-200 bg-white p-6">
                    <h2 className="text-lg font-semibold text-gray-900">{doc.title}</h2>
                    <p className="mt-2 text-sm text-gray-600 max-w-prose">{doc.description}</p>
                    <pre className="mt-4 rounded-xl border border-gray-200 bg-gray-50 p-4 text-xs text-gray-800 overflow-auto">
{doc.card}
                    </pre>
                    <a href={doc.href} className="btn-primary mt-5 inline-flex px-5 py-2 text-sm">
                      Open {doc.title}
                    </a>
                  </div>
                ))}
              </div>

              <div className="mt-12 rounded-2xl border border-gray-200 bg-gray-50 p-6">
                <h3 className="text-base font-semibold text-gray-900">Docs URL</h3>
                <p className="mt-2 text-sm text-gray-600 max-w-prose">
                  Documentation is served from <span className="font-mono">apexmediation.ee/docs</span>.
                </p>
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
