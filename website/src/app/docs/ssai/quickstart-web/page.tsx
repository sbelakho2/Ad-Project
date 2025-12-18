'use client';

import Link from 'next/link';
import { CheckCircle, Code, Play, Settings, Zap } from 'lucide-react';

import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function SSAIQuickstartWebPage() {
  return (
    <div className="min-h-screen bg-white">
      <SiteHeader />

      <main className="pt-24 pb-16">
        <div className="max-w-4xl mx-auto px-6">
          {/* Breadcrumb */}
          <div className="flex items-center gap-2 text-sm text-gray-500 mb-8">
            <Link href="/docs" className="hover:text-brand-600">Docs</Link>
            <span>/</span>
            <Link href="/docs/ssai" className="hover:text-brand-600">SSAI</Link>
            <span>/</span>
            <span className="text-gray-900">Web Video Quickstart</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">Web Video SSAI Quickstart</h1>
            <p className="text-xl text-gray-600">
              Get SSAI working end-to-end in a browser player. This guide focuses on the fastest path to a stitched manifest you can play with hls.js, Shaka, dash.js, or Video.js.
            </p>
          </div>

          {/* Summary Box */}
          <div className="bg-brand-50 border border-brand-200 rounded-lg p-6 mb-12">
            <h2 className="font-semibold text-gray-900 mb-2">Quick Summary</h2>
            <p className="text-gray-600 text-sm">
              You start with an origin stream (HLS or DASH) and ad break signals. ApexMediation returns a session-specific stitched manifest URL that your web player loads like a normal stream.
            </p>
          </div>

          {/* Steps */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Zap className="w-6 h-6 text-brand-600" />
              Steps
            </h2>

            <div className="space-y-6">
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                  <span className="bg-brand-100 text-brand-700 text-xs px-2 py-1 rounded">1</span>
                  Confirm prerequisites
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Verify your origin manifests, break signals, and segmenting match the SSAI requirements.
                </p>
                <Link href="/docs/ssai/prerequisites" className="inline-flex items-center gap-2 text-brand-700 hover:underline">
                  <CheckCircle className="w-4 h-4" />
                  Publisher prerequisites
                </Link>
              </div>

              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                  <span className="bg-brand-100 text-brand-700 text-xs px-2 py-1 rounded">2</span>
                  Choose protocol and player
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Pick HLS or DASH based on your distribution setup and player support.
                </p>
                <div className="grid sm:grid-cols-2 gap-3">
                  <Link href="/docs/ssai/protocols/hls" className="p-4 border border-gray-200 rounded-lg hover:border-brand-500 transition-colors">
                    <div className="flex items-center gap-2 mb-1">
                      <Play className="w-4 h-4 text-brand-600" />
                      <span className="font-semibold text-gray-900">HLS</span>
                    </div>
                    <p className="text-sm text-gray-600">Play with hls.js, Video.js, Safari</p>
                  </Link>
                  <Link href="/docs/ssai/protocols/dash" className="p-4 border border-gray-200 rounded-lg hover:border-brand-500 transition-colors">
                    <div className="flex items-center gap-2 mb-1">
                      <Play className="w-4 h-4 text-brand-600" />
                      <span className="font-semibold text-gray-900">DASH</span>
                    </div>
                    <p className="text-sm text-gray-600">Play with Shaka or dash.js</p>
                  </Link>
                </div>
              </div>

              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                  <span className="bg-brand-100 text-brand-700 text-xs px-2 py-1 rounded">3</span>
                  Initialize a session and load the stitched manifest
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Use the SSAI session init flow to obtain a per-session manifest URL. Then configure your player to load that URL.
                </p>
                <div className="bg-gray-50 rounded-lg p-4">
                  <code className="text-sm text-gray-800">POST /v1/session/init</code>
                </div>
                <div className="mt-4 flex flex-wrap gap-3">
                  <Link href="/docs/ssai" className="inline-flex items-center gap-2 text-brand-700 hover:underline">
                    <Settings className="w-4 h-4" />
                    SSAI overview (request flow)
                  </Link>
                  <Link href="/docs/ssai/measurement" className="inline-flex items-center gap-2 text-brand-700 hover:underline">
                    <Code className="w-4 h-4" />
                    Measurement and reconciliation
                  </Link>
                </div>
              </div>

              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                  <span className="bg-brand-100 text-brand-700 text-xs px-2 py-1 rounded">4</span>
                  Validate compatibility
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Confirm your player/protocol/container combo is certified and understand any limitations.
                </p>
                <Link href="/docs/ssai/coverage" className="inline-flex items-center gap-2 text-brand-700 hover:underline">
                  <CheckCircle className="w-4 h-4" />
                  Certified playback matrix
                </Link>
              </div>
            </div>
          </section>

          <div className="border-t border-gray-200 pt-8">
            <p className="text-sm text-gray-600">
              Next: if you need CDN guidance for cache behavior and headers, see{' '}
              <Link href="/docs/ssai/cdn" className="text-brand-700 hover:underline">CDN guidance</Link>.
            </p>
          </div>
        </div>
      </main>

      <Footer />
    </div>
  );
}
