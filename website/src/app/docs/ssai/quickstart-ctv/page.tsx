'use client';

import Link from 'next/link';
import { CheckCircle, Monitor, Settings, Zap } from 'lucide-react';

import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function SSAIQuickstartCTVPage() {
  return (
    <div className="min-h-screen bg-white">
      <SiteHeader />

      <main className="pt-24 pb-16">
        <div className="max-w-4xl mx-auto px-6">
          {/* Breadcrumb */}
          <div className="flex items-center gap-2 text-sm text-gray-500 mb-8">
            <Link href="/docs" className="hover:text-emerald-600">Docs</Link>
            <span>/</span>
            <Link href="/docs/ssai" className="hover:text-emerald-600">SSAI</Link>
            <span>/</span>
            <span className="text-gray-900">CTV Quickstart</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">CTV SSAI Quickstart</h1>
            <p className="text-xl text-gray-600">
              Integrate SSAI for connected TV players. This guide focuses on selecting the right protocol/container/DRM policy and validating playback on target devices.
            </p>
          </div>

          {/* Summary Box */}
          <div className="bg-emerald-50 border border-emerald-200 rounded-lg p-6 mb-12">
            <h2 className="font-semibold text-gray-900 mb-2">Quick Summary</h2>
            <p className="text-gray-600 text-sm">
              For CTV, compatibility is driven by your player stack (AVPlayer/ExoPlayer/etc.), protocol (HLS/DASH), container (TS/CMAF), and DRM (if any). Start by aligning those choices, then validate against the playback matrix.
            </p>
          </div>

          {/* Steps */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Zap className="w-6 h-6 text-emerald-600" />
              Steps
            </h2>

            <div className="space-y-6">
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                  <span className="bg-emerald-100 text-emerald-700 text-xs px-2 py-1 rounded">1</span>
                  Confirm prerequisites (origin + break signals)
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Make sure your stream packaging and ad break signaling are suitable for stitched playback.
                </p>
                <Link href="/docs/ssai/prerequisites" className="inline-flex items-center gap-2 text-emerald-600 hover:underline">
                  <CheckCircle className="w-4 h-4" />
                  Publisher prerequisites
                </Link>
              </div>

              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                  <span className="bg-emerald-100 text-emerald-700 text-xs px-2 py-1 rounded">2</span>
                  Choose protocol, container, and DRM policy
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Match your device/player capabilities and DRM requirements to the correct protocol and container.
                </p>
                <div className="flex flex-wrap gap-3">
                  <Link href="/docs/ssai/protocols/hls" className="inline-flex items-center gap-2 text-emerald-600 hover:underline">
                    <Settings className="w-4 h-4" />
                    HLS protocol guidance
                  </Link>
                  <Link href="/docs/ssai/protocols/dash" className="inline-flex items-center gap-2 text-emerald-600 hover:underline">
                    <Settings className="w-4 h-4" />
                    DASH protocol guidance
                  </Link>
                  <Link href="/docs/ssai/containers" className="inline-flex items-center gap-2 text-emerald-600 hover:underline">
                    <Settings className="w-4 h-4" />
                    TS vs CMAF containers
                  </Link>
                  <Link href="/docs/ssai/drm" className="inline-flex items-center gap-2 text-emerald-600 hover:underline">
                    <Settings className="w-4 h-4" />
                    DRM policies
                  </Link>
                </div>
              </div>

              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                  <span className="bg-emerald-100 text-emerald-700 text-xs px-2 py-1 rounded">3</span>
                  Validate playback on your target devices
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Use the certified playback matrix to confirm your target platforms and players have been tested for stitched playback.
                </p>
                <div className="flex flex-wrap gap-3">
                  <Link href="/docs/ssai/coverage" className="inline-flex items-center gap-2 text-emerald-600 hover:underline">
                    <Monitor className="w-4 h-4" />
                    Certified playback matrix
                  </Link>
                  <Link href="/docs/ssai/operations" className="inline-flex items-center gap-2 text-emerald-600 hover:underline">
                    <Monitor className="w-4 h-4" />
                    Operations and failure modes
                  </Link>
                </div>
              </div>

              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                  <span className="bg-emerald-100 text-emerald-700 text-xs px-2 py-1 rounded">4</span>
                  Measure and reconcile
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Ensure measurement events and reconciliation align with your reporting and partner requirements.
                </p>
                <Link href="/docs/ssai/measurement" className="inline-flex items-center gap-2 text-emerald-600 hover:underline">
                  <CheckCircle className="w-4 h-4" />
                  Measurement guide
                </Link>
              </div>
            </div>
          </section>

          <div className="border-t border-gray-200 pt-8">
            <p className="text-sm text-gray-600">
              If you are onboarding a new player platform, start with{' '}
              <Link href="/docs/ssai/coverage" className="text-emerald-600 hover:underline">coverage</Link>{' '}
              and work backward to the protocol/container/DRM choices.
            </p>
          </div>
        </div>
      </main>

      <Footer />
    </div>
  );
}
