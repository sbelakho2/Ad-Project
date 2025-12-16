'use client';

import Link from 'next/link';
import { CheckCircle, AlertTriangle, FileText, Clock, Rewind, BarChart3, Play, FastForward } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function VODModePage() {
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
            <span className="text-gray-900">VOD Mode</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              VOD SSAI Mode
            </h1>
            <p className="text-xl text-gray-600">
              Video-on-demand ad insertion with full seeking support, pre-stitched manifests, 
              and deterministic quartile tracking for accurate measurement.
            </p>
          </div>

          {/* Overview */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Play className="w-6 h-6 text-emerald-600" />
              VOD vs Live SSAI
            </h2>
            
            <p className="text-gray-600 mb-4">
              VOD SSAI differs fundamentally from live: all ad decisions are made at session 
              creation time, manifests are stable throughout playback, and seeking requires 
              special handling for accurate measurement.
            </p>

            <div className="overflow-x-auto mb-6">
              <table className="w-full min-w-full text-sm border-collapse">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Aspect</th>
                    <th className="text-left p-3 font-semibold text-gray-900">VOD</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Live</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">Ad Decisioning</td>
                    <td className="p-3 text-gray-600">At session creation (all breaks)</td>
                    <td className="p-3 text-gray-600">Just-in-time (per break)</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Manifest Stability</td>
                    <td className="p-3 text-gray-600">Immutable after creation</td>
                    <td className="p-3 text-gray-600">Sliding window updates</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Seeking</td>
                    <td className="p-3 text-gray-600">Full support required</td>
                    <td className="p-3 text-gray-600">DVR window only</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Quartile Tracking</td>
                    <td className="p-3 text-gray-600">Deterministic (seek-aware)</td>
                    <td className="p-3 text-gray-600">Sequential</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Session TTL</td>
                    <td className="p-3 text-gray-600">Fixed (content duration + buffer)</td>
                    <td className="p-3 text-gray-600">Refresh-on-access</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          {/* Marker Processing */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <FileText className="w-6 h-6 text-emerald-600" />
              VOD Marker Processing
            </h2>
            
            <p className="text-gray-600 mb-4">
              VOD content often originates from harvested live streams that contain preserved 
              ad markers. The <code>VODMarkerProcessor</code> detects and normalizes these 
              markers from multiple sources:
            </p>

            <div className="grid md:grid-cols-2 gap-4 mb-6">
              <div className="bg-gray-50 rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 mb-2">HLS Marker Types</h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• <code>#EXT-X-DATERANGE</code> - Apple interstitials</li>
                  <li>• <code>#EXT-X-CUE-OUT</code> / <code>#EXT-X-CUE-IN</code> - Standard cues</li>
                  <li>• <code>#EXT-OATCLS-SCTE35</code> - SCTE-35 in comments</li>
                  <li>• <code>#EXT-X-CHAPTER</code> - Chapter markers</li>
                </ul>
              </div>
              <div className="bg-gray-50 rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 mb-2">DASH Marker Types</h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• <code>EventStream</code> with SCTE-35</li>
                  <li>• <code>EventStream</code> with DVB-DASH cues</li>
                  <li>• Period boundaries</li>
                  <li>• Custom event schemes</li>
                </ul>
              </div>
            </div>

            <div className="bg-gray-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">Processing Pipeline</h3>
              <ol className="space-y-2 text-sm text-gray-600 list-decimal pl-5">
                <li className="pl-1"><strong>Parse Origin Manifest:</strong> Extract all ad markers with timeline positions</li>
                <li className="pl-1"><strong>Normalize Markers:</strong> Convert all marker types to unified <code>AdBreakCue</code> format</li>
                <li className="pl-1"><strong>Resolve Pairs:</strong> Match start/end markers, calculate break durations</li>
                <li className="pl-1"><strong>Validate Positions:</strong> Ensure markers don&apos;t overlap, are within duration</li>
                <li className="pl-1"><strong>Generate Breaks:</strong> Create ad break list with positions and durations</li>
              </ol>
            </div>
          </section>

          {/* Manifest Stability */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <FileText className="w-6 h-6 text-emerald-600" />
              Manifest Stability
            </h2>
            
            <p className="text-gray-600 mb-4">
              VOD manifests are generated once at session creation and remain immutable 
              throughout playback. This is critical for seeking and progress tracking.
            </p>

            <div className="grid md:grid-cols-2 gap-6 mb-6">
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">HLS VOD Requirements</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>#EXT-X-ENDLIST</code> tag required at end</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>#EXT-X-PLAYLIST-TYPE:VOD</code> present</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>All segments listed from start to end</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Complete DATERANGE markers for all ad breaks</span>
                  </li>
                </ul>
              </div>
              
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">DASH VOD Requirements</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>type=&quot;static&quot;</code> MPD</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>mediaPresentationDuration</code> set</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>All periods present from start</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Complete EventStream for ad metadata</span>
                  </li>
                </ul>
              </div>
            </div>

            <div className="bg-amber-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                <AlertTriangle className="w-5 h-5 text-amber-600" />
                Common Issue: Missing ENDLIST
              </h3>
              <p className="text-sm text-gray-600">
                VOD playlists without <code>#EXT-X-ENDLIST</code> cause players to poll for updates 
                indefinitely. The SSAI system enforces ENDLIST presence for VOD mode and validates 
                during manifest generation.
              </p>
            </div>
          </section>

          {/* Seeking Behavior */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <FastForward className="w-6 h-6 text-emerald-600" />
              Seeking Behavior
            </h2>
            
            <p className="text-gray-600 mb-4">
              VOD SSAI must handle seeking correctly. Users may seek forward through ads, 
              seek backward to rewatch content, or jump directly to specific points.
            </p>

            <div className="space-y-4">
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Seek Into Ad Break</h3>
                <p className="text-gray-600 text-sm">
                  When seeking into an ad break, playback starts from the ad at that position. 
                  Impression is credited when at least 2 seconds of the ad are viewed.
                </p>
              </div>
              
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Seek Past Ad Break</h3>
                <p className="text-gray-600 text-sm">
                  When seeking past an ad break (skipping), the ads are not credited. 
                  Optional: force ad viewing on first pass (requires SDK integration).
                </p>
              </div>
              
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Seek Backward</h3>
                <p className="text-gray-600 text-sm">
                  Seeking backward does not re-fire already-credited impressions. 
                  Quartiles are deduplicated per slot per session.
                </p>
              </div>
            </div>

            <div className="bg-gray-900 rounded-lg p-4 mt-6">
              <h3 className="font-semibold text-white mb-2">Timeline Mapping</h3>
              <pre className="text-sm text-gray-300 overflow-x-auto">
{`Original Timeline (with ad markers):
[Content: 0-30m] [Break @ 30m, 30s] [Content: 30m-60m] [Break @ 60m, 30s] [Content: 60m-90m]

Stitched Timeline (player sees):
[Content: 0-30m] [Ad Pod 1: 30s] [Content: 30m30s-60m30s] [Ad Pod 2: 30s] [Content: 61m-91m]

Total Duration: 90m content + 1m ads = 91m

Seek to 45m → Player position 45m30s (after first ad break)
Seek to 70m → Player position 71m (after both ad breaks)`}
              </pre>
            </div>
          </section>

          {/* Quartile Tracking */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <BarChart3 className="w-6 h-6 text-emerald-600" />
              Deterministic Quartile Tracking
            </h2>
            
            <p className="text-gray-600 mb-4">
              The <code>VODReconciler</code> handles quartile tracking for seekable content 
              with configurable behavior for missed quartiles.
            </p>

            <div className="bg-gray-50 rounded-lg p-6 mb-6">
              <h3 className="font-semibold text-gray-900 mb-3">Quartile Positions</h3>
              <p className="text-sm text-gray-600 mb-3">
                Calculated by <code>calculateQuartilePositions(duration)</code>:
              </p>
              <div className="overflow-x-auto">
                <table className="w-full min-w-full text-sm border-collapse">
                  <thead>
                    <tr className="border-b border-gray-200">
                      <th className="text-left py-2 pr-4 font-semibold text-gray-900">Event</th>
                      <th className="text-left py-2 pr-4 font-semibold text-gray-900">Position</th>
                      <th className="text-left py-2 font-semibold text-gray-900">Progress</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-gray-100">
                    <tr>
                      <td className="py-2 pr-4 text-gray-700">Impression</td>
                      <td className="py-2 pr-4 text-gray-600">0s (start)</td>
                      <td className="py-2 text-gray-600">0%</td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700">firstQuartile</td>
                      <td className="py-2 pr-4 text-gray-600">duration × 0.25</td>
                      <td className="py-2 text-gray-600">25%</td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700">midpoint</td>
                      <td className="py-2 pr-4 text-gray-600">duration × 0.50</td>
                      <td className="py-2 text-gray-600">50%</td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700">thirdQuartile</td>
                      <td className="py-2 pr-4 text-gray-600">duration × 0.75</td>
                      <td className="py-2 text-gray-600">75%</td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700">complete</td>
                      <td className="py-2 pr-4 text-gray-600">duration (end)</td>
                      <td className="py-2 text-gray-600">100%</td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>

            <div className="grid md:grid-cols-2 gap-6">
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Reconciler Config</h3>
                <p className="text-sm text-gray-600 mb-2">
                  From <code>VODReconcilerConfig</code>:
                </p>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• <strong>timingToleranceSeconds:</strong> 0.5s</li>
                  <li>• <strong>fireOnSeek:</strong> true (fire missed on seek)</li>
                  <li>• <strong>maxAgeMinutes:</strong> 120 (2 hour retention)</li>
                </ul>
              </div>
              
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Seek Handling</h3>
                <p className="text-sm text-gray-600 mb-2">
                  When <code>fireOnSeek</code> is enabled:
                </p>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Seek forward: missed quartiles are fired</li>
                  <li>• Seek backward: no duplicate fires</li>
                  <li>• Dedupe via <code>state.fired</code> Set</li>
                </ul>
              </div>
            </div>
          </section>

          {/* Reconciliation */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Clock className="w-6 h-6 text-emerald-600" />
              Session Reconciliation
            </h2>
            
            <p className="text-gray-600 mb-4">
              At session end, the <code>VODReconciler</code> performs final reconciliation 
              to identify discrepancies between expected and actual tracking events.
            </p>

            <div className="bg-gray-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">Reconciliation Output</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`{
  "podId": "pod-abc123",
  "slots": [
    {
      "slotId": "slot-1",
      "creativeId": "cr-xyz",
      "expected": ["impression", "firstQuartile", "midpoint", "thirdQuartile", "complete"],
      "received": ["impression", "firstQuartile", "midpoint"],
      "missing": ["thirdQuartile", "complete"],
      "duplicates": [],
      "status": "partial"
    }
  ],
  "discrepancies": [
    {
      "type": "missing_quartile",
      "slotId": "slot-1",
      "event": "thirdQuartile",
      "reason": "user_seeked_past"
    }
  ]
}`}
              </pre>
            </div>
          </section>

          {/* Pre-roll Handling */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Rewind className="w-6 h-6 text-emerald-600" />
              Pre-roll Handling
            </h2>
            
            <p className="text-gray-600 mb-4">
              VOD pre-rolls have special considerations since they appear before any content:
            </p>

            <div className="space-y-3">
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">Immediate Insertion</span>
                  <p className="text-sm text-gray-600">Pre-roll ads are at position 0 in the stitched manifest</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">Seek Protection</span>
                  <p className="text-sm text-gray-600">Optional: SDK can prevent seeking until pre-roll complete</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">Deep Link Support</span>
                  <p className="text-sm text-gray-600">Deep links to specific content positions still show pre-roll first</p>
                </div>
              </div>
            </div>
          </section>

          {/* Session TTL */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">VOD Session Lifecycle</h2>
            
            <div className="bg-gray-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">Session TTL Calculation</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`// VOD session TTL = content duration + buffer
const sessionTTL = contentDuration + (24 * 60 * 60); // +24 hours buffer

// Example: 90 minute movie
// TTL = 90 minutes + 24 hours = 25.5 hours

// Session expires after TTL regardless of activity
// (Unlike live which refreshes on manifest access)`}
              </pre>
            </div>
          </section>

          {/* Next Steps */}
          <section className="bg-emerald-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/modes/live" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Live Mode →</h3>
                <p className="text-sm text-gray-600">Latency, resilience, operational controls</p>
              </Link>
              <Link href="/docs/ssai/protocols/hls" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">HLS Protocol →</h3>
                <p className="text-sm text-gray-600">RFC 8216 compliance, ENDLIST</p>
              </Link>
              <Link href="/docs/ssai/containers" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Containers →</h3>
                <p className="text-sm text-gray-600">TS and CMAF segment formats</p>
              </Link>
              <Link href="/docs/ssai" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">← Back to SSAI Overview</h3>
                <p className="text-sm text-gray-600">Integration guide and architecture</p>
              </Link>
            </div>
          </section>
        </div>
      </main>
      
      <Footer />
    </div>
  );
}
