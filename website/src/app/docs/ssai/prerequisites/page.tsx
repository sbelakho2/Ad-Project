'use client';

import Link from 'next/link';
import { CheckCircle, XCircle, AlertTriangle, FileCheck, Settings, Zap, Shield, Clock } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function PrerequisitesPage() {
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
            <span className="text-gray-900">Publisher Prerequisites</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              Publisher Prerequisites
            </h1>
            <p className="text-xl text-gray-600">
              Technical requirements your origin and content must meet for successful SSAI integration. 
              This checklist ensures smooth onboarding and optimal fill rates.
            </p>
          </div>

          {/* Summary Box */}
          <div className="bg-emerald-50 border border-emerald-200 rounded-lg p-6 mb-12">
            <h2 className="font-semibold text-gray-900 mb-2">Quick Summary</h2>
            <p className="text-gray-600 text-sm">
              You provide: origin manifest URL, ad break signals (SCTE-35/DATERANGE), content with keyframe-aligned segments. 
              We handle: manifest rewriting, ad decisioning, stitching, tracking, and reporting.
            </p>
          </div>

          {/* What You Must Provide */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <FileCheck className="w-6 h-6 text-emerald-600" />
              What You Must Provide
            </h2>

            <div className="space-y-6">
              {/* Origin Manifest */}
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <span className="bg-emerald-100 text-emerald-700 text-xs px-2 py-1 rounded">Required</span>
                  Origin Manifest URL
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Your origin HLS master playlist or DASH MPD must be accessible via HTTPS.
                </p>
                <div className="bg-gray-50 rounded p-4 space-y-3">
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span className="text-sm text-gray-600">HTTPS endpoint (no self-signed certs)</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span className="text-sm text-gray-600">Stable URL pattern (or webhook notification for URL changes)</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span className="text-sm text-gray-600">IP allowlisting or token-based access for our manifest fetcher</span>
                  </div>
                </div>
              </div>

              {/* Ad Break Signals */}
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <span className="bg-emerald-100 text-emerald-700 text-xs px-2 py-1 rounded">Required</span>
                  Ad Break Signals
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Ad break opportunities must be signaled via one of these mechanisms:
                </p>
                <div className="grid md:grid-cols-2 gap-4">
                  <div className="bg-gray-50 rounded p-4">
                    <h4 className="font-medium text-gray-900 text-sm mb-2">HLS Options</h4>
                    <ul className="text-sm text-gray-600 space-y-1">
                      <li>• <code>#EXT-X-DATERANGE</code> with SCTE-35 data</li>
                      <li>• <code>#EXT-X-CUE-OUT</code> / <code>#EXT-X-CUE-IN</code></li>
                      <li>• SCTE-35 in-band (ID3 timed metadata)</li>
                    </ul>
                  </div>
                  <div className="bg-gray-50 rounded p-4">
                    <h4 className="font-medium text-gray-900 text-sm mb-2">DASH Options</h4>
                    <ul className="text-sm text-gray-600 space-y-1">
                      <li>• <code>EventStream</code> with SCTE-35 scheme</li>
                      <li>• <code>urn:scte:scte35:2013:xml</code></li>
                      <li>• <code>urn:scte:scte35:2014:xml+bin</code></li>
                    </ul>
                  </div>
                </div>
                <div className="mt-4 p-3 bg-amber-50 border border-amber-200 rounded text-sm text-amber-800">
                  <strong>Important:</strong> Cues with <code>DURATION</code> or <code>PLANNED-DURATION</code> improve fill rates. 
                  Without duration, we wait for the &quot;end&quot; cue before ad selection.
                </div>
              </div>

              {/* Segment Requirements */}
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <span className="bg-emerald-100 text-emerald-700 text-xs px-2 py-1 rounded">Required</span>
                  Segment Requirements
                </h3>
                <div className="space-y-3">
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="text-sm text-gray-900 font-medium">Keyframe alignment at cue points</span>
                      <p className="text-sm text-gray-600">Every ad break cue must land on an IDR frame. Encoder must insert keyframes at signaled positions.</p>
                    </div>
                  </div>
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="text-sm text-gray-900 font-medium">Consistent segment duration</span>
                      <p className="text-sm text-gray-600">Segments should be 2-6 seconds. Recommended: 4s for VOD, 2s for low-latency live.</p>
                    </div>
                  </div>
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="text-sm text-gray-900 font-medium">A/V muxed or aligned</span>
                      <p className="text-sm text-gray-600">Audio and video segment boundaries must align. Separate audio renditions require matching timelines.</p>
                    </div>
                  </div>
                </div>
              </div>

              {/* Codec Ladder */}
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <span className="bg-emerald-100 text-emerald-700 text-xs px-2 py-1 rounded">Required</span>
                  Codec &amp; Resolution Ladder
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  Ads are matched to content quality tiers. Provide your encoding ladder so we can source compatible creatives:
                </p>
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="bg-gray-50">
                        <th className="text-left p-2 font-semibold text-gray-900">Parameter</th>
                        <th className="text-left p-2 font-semibold text-gray-900">Recommended</th>
                        <th className="text-left p-2 font-semibold text-gray-900">Notes</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-gray-200">
                      <tr>
                        <td className="p-2 text-gray-900">Video Codec</td>
                        <td className="p-2 text-gray-600">H.264 (AVC)</td>
                        <td className="p-2 text-gray-600">H.265 (HEVC) supported but reduces ad inventory</td>
                      </tr>
                      <tr>
                        <td className="p-2 text-gray-900">Audio Codec</td>
                        <td className="p-2 text-gray-600">AAC-LC</td>
                        <td className="p-2 text-gray-600">AC-3/E-AC-3 for 5.1 content</td>
                      </tr>
                      <tr>
                        <td className="p-2 text-gray-900">Resolutions</td>
                        <td className="p-2 text-gray-600">1080p, 720p, 480p, 360p</td>
                        <td className="p-2 text-gray-600">4K reduces ad inventory significantly</td>
                      </tr>
                      <tr>
                        <td className="p-2 text-gray-900">Frame Rate</td>
                        <td className="p-2 text-gray-600">24/25/30 fps</td>
                        <td className="p-2 text-gray-600">60fps supported but verify ad availability</td>
                      </tr>
                    </tbody>
                  </table>
                </div>
              </div>

              {/* Caption Format */}
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <span className="bg-blue-100 text-blue-700 text-xs px-2 py-1 rounded">Optional</span>
                  Caption Format
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  If your content includes captions, specify the format for proper timing adjustment:
                </p>
                <div className="grid md:grid-cols-2 gap-4">
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span className="text-sm text-gray-600">CEA-608/708 embedded (recommended)</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span className="text-sm text-gray-600">WebVTT sidecar files</span>
                  </div>
                </div>
              </div>

              {/* DRM Constraints */}
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <span className="bg-blue-100 text-blue-700 text-xs px-2 py-1 rounded">If Applicable</span>
                  DRM Requirements
                </h3>
                <p className="text-gray-600 text-sm mb-4">
                  If your content is DRM-protected, we need:
                </p>
                <div className="space-y-2">
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span className="text-sm text-gray-600">DRM scheme(s) used (Widevine, FairPlay, PlayReady)</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span className="text-sm text-gray-600">Policy: Can ads be clear (unencrypted) in encrypted content? (Default: yes)</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span className="text-sm text-gray-600">License server URL if ads must use same DRM</span>
                  </div>
                </div>
              </div>
            </div>
          </section>

          {/* What We Validate Automatically */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Settings className="w-6 h-6 text-emerald-600" />
              What We Validate Automatically
            </h2>
            
            <p className="text-gray-600 mb-4">
              Before serving ads, our system validates:
            </p>

            <div className="grid md:grid-cols-2 gap-4">
              <div className="bg-gray-50 rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 mb-2">Manifest Validation</h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• HLS RFC 8216 compliance</li>
                  <li>• DASH-IF MPD structure</li>
                  <li>• <code>EXTINF</code> &gt; 0 for all segments</li>
                  <li>• <code>EXT-X-TARGETDURATION</code> accuracy</li>
                  <li>• Discontinuity sequence consistency</li>
                </ul>
              </div>
              <div className="bg-gray-50 rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 mb-2">Cue Validation</h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• SCTE-35 parse correctness</li>
                  <li>• Cue deduplication</li>
                  <li>• Keyframe alignment check</li>
                  <li>• Duration reasonability (0.5s–300s)</li>
                </ul>
              </div>
              <div className="bg-gray-50 rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 mb-2">Format Compatibility</h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Codec string matching (avc1.*, mp4a.40.*)</li>
                  <li>• Container format (TS vs CMAF)</li>
                  <li>• Audio layout compatibility</li>
                  <li>• Resolution tier matching</li>
                </ul>
              </div>
              <div className="bg-gray-50 rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 mb-2">Origin Health</h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Manifest fetch latency</li>
                  <li>• Segment availability (404 detection)</li>
                  <li>• SSL certificate validity</li>
                  <li>• Rate limit headroom</li>
                </ul>
              </div>
            </div>
          </section>

          {/* What Causes Issues */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <XCircle className="w-6 h-6 text-red-600" />
              What Causes Rejects / No-Fill / Slate
            </h2>
            
            <p className="text-gray-600 mb-4">
              These issues result in unfilled ad breaks or fallback to slate content:
            </p>

            <div className="space-y-3">
              <div className="flex items-start gap-3 p-4 border border-red-200 bg-red-50 rounded-lg">
                <XCircle className="w-5 h-5 text-red-600 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium text-gray-900">Cue not at keyframe</p>
                  <p className="text-sm text-gray-600">Splice point mid-GOP causes glitches. We reject or align to nearest safe boundary.</p>
                </div>
              </div>
              <div className="flex items-start gap-3 p-4 border border-red-200 bg-red-50 rounded-lg">
                <XCircle className="w-5 h-5 text-red-600 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium text-gray-900">No compatible ad creative</p>
                  <p className="text-sm text-gray-600">Content codec/resolution has no matching ad inventory. Consider H.264 1080p baseline.</p>
                </div>
              </div>
              <div className="flex items-start gap-3 p-4 border border-red-200 bg-red-50 rounded-lg">
                <XCircle className="w-5 h-5 text-red-600 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium text-gray-900">Partner timeout</p>
                  <p className="text-sm text-gray-600">Demand partner didn&apos;t respond within deadline. System serves slate to avoid black screen.</p>
                </div>
              </div>
              <div className="flex items-start gap-3 p-4 border border-amber-200 bg-amber-50 rounded-lg">
                <AlertTriangle className="w-5 h-5 text-amber-600 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium text-gray-900">Cue without duration</p>
                  <p className="text-sm text-gray-600">Forces wait for end-cue before decisioning, reducing available think time.</p>
                </div>
              </div>
              <div className="flex items-start gap-3 p-4 border border-amber-200 bg-amber-50 rounded-lg">
                <AlertTriangle className="w-5 h-5 text-amber-600 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium text-gray-900">Origin latency spikes</p>
                  <p className="text-sm text-gray-600">Slow manifest fetch reduces time budget for ad selection.</p>
                </div>
              </div>
            </div>
          </section>

          {/* Pre-Integration Checklist */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Zap className="w-6 h-6 text-emerald-600" />
              Pre-Integration Checklist
            </h2>
            
            <div className="bg-gray-50 rounded-lg p-6">
              <ul className="space-y-3">
                <li className="flex items-start gap-3">
                  <div className="mt-1 bg-white rounded border border-gray-300 w-5 h-5 flex items-center justify-center flex-shrink-0">
                    <CheckCircle className="w-3 h-3 text-emerald-600" />
                  </div>
                  <span className="text-gray-600">Origin manifest URL accessible via HTTPS</span>
                </li>
                <li className="flex items-start gap-3">
                  <div className="mt-1 bg-white rounded border border-gray-300 w-5 h-5 flex items-center justify-center flex-shrink-0">
                    <CheckCircle className="w-3 h-3 text-emerald-600" />
                  </div>
                  <span className="text-gray-600">Ad break cues present (SCTE-35, DATERANGE, or CUE-OUT/IN)</span>
                </li>
                <li className="flex items-start gap-3">
                  <div className="mt-1 bg-white rounded border border-gray-300 w-5 h-5 flex items-center justify-center flex-shrink-0">
                    <CheckCircle className="w-3 h-3 text-emerald-600" />
                  </div>
                  <span className="text-gray-600">Cues include DURATION or PLANNED-DURATION</span>
                </li>
                <li className="flex items-start gap-3">
                  <div className="mt-1 bg-white rounded border border-gray-300 w-5 h-5 flex items-center justify-center flex-shrink-0">
                    <CheckCircle className="w-3 h-3 text-emerald-600" />
                  </div>
                  <span className="text-gray-600">Encoder inserts keyframes at cue positions</span>
                </li>
                <li className="flex items-start gap-3">
                  <div className="mt-1 bg-white rounded border border-gray-300 w-5 h-5 flex items-center justify-center flex-shrink-0">
                    <CheckCircle className="w-3 h-3 text-emerald-600" />
                  </div>
                  <span className="text-gray-600">Segments are 2-6 seconds duration</span>
                </li>
                <li className="flex items-start gap-3">
                  <div className="mt-1 bg-white rounded border border-gray-300 w-5 h-5 flex items-center justify-center flex-shrink-0">
                    <CheckCircle className="w-3 h-3 text-emerald-600" />
                  </div>
                  <span className="text-gray-600">Audio and video segment boundaries align</span>
                </li>
                <li className="flex items-start gap-3">
                  <div className="mt-1 bg-white rounded border border-gray-300 w-5 h-5 flex items-center justify-center flex-shrink-0">
                    <CheckCircle className="w-3 h-3 text-emerald-600" />
                  </div>
                  <span className="text-gray-600">Encoding ladder documented (codecs, resolutions, bitrates)</span>
                </li>
                <li className="flex items-start gap-3">
                  <div className="mt-1 bg-white rounded border border-gray-300 w-5 h-5 flex items-center justify-center flex-shrink-0">
                    <CheckCircle className="w-3 h-3 text-emerald-600" />
                  </div>
                  <span className="text-gray-600">DRM requirements documented (if applicable)</span>
                </li>
                <li className="flex items-start gap-3">
                  <div className="mt-1 bg-white rounded border border-gray-300 w-5 h-5 flex items-center justify-center flex-shrink-0">
                    <CheckCircle className="w-3 h-3 text-emerald-600" />
                  </div>
                  <span className="text-gray-600">Our manifest fetcher IPs allowlisted (or token provided)</span>
                </li>
              </ul>
            </div>
          </section>

          {/* Related Documentation */}
          <section className="bg-emerald-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/protocols/hls" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">HLS Protocol →</h3>
                <p className="text-sm text-gray-600">DATERANGE and CUE-OUT/IN signaling</p>
              </Link>
              <Link href="/docs/ssai/protocols/dash" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">DASH Protocol →</h3>
                <p className="text-sm text-gray-600">EventStream and SCTE-35 schemes</p>
              </Link>
              <Link href="/docs/ssai/containers" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Container Formats →</h3>
                <p className="text-sm text-gray-600">TS vs CMAF requirements</p>
              </Link>
              <Link href="/docs/ssai/drm" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">DRM Integration →</h3>
                <p className="text-sm text-gray-600">Widevine, FairPlay, PlayReady policies</p>
              </Link>
            </div>
            
            <div className="mt-6 pt-6 border-t border-emerald-200">
              <Link href="/docs/ssai" className="inline-flex items-center text-emerald-700 font-medium hover:text-emerald-800">
                ← Back to SSAI Overview
              </Link>
            </div>
          </section>
        </div>
      </main>
      
      <Footer />
    </div>
  );
}
