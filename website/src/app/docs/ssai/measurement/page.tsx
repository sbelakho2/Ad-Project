'use client';

import Link from 'next/link';
import { BarChart3, CheckCircle, Clock, Shield, AlertTriangle, Zap, RefreshCw, Target } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function MeasurementSemanticsPage() {
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
            <span className="text-gray-900">Measurement Semantics</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              Measurement &amp; Tracking Semantics
            </h1>
            <p className="text-xl text-gray-600">
              How ApexMediation SSAI tracks impressions, quartiles, and completion events.
              Definitions, exactly-once semantics, and reconciliation guidance.
            </p>
          </div>

          {/* Core Guarantee */}
          <div className="bg-brand-50 border border-brand-200 rounded-lg p-6 mb-12">
            <h2 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
              <Shield className="w-5 h-5 text-brand-600" />
              Core Guarantee: Exactly-Once Delivery
            </h2>
            <p className="text-gray-600 text-sm">
              Each tracking event is fired exactly once per ad view. Retries are deduplicated 
              server-side. Late events (beyond the session TTL) are logged but not delivered.
            </p>
          </div>

          {/* Event Definitions */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <BarChart3 className="w-6 h-6 text-brand-600" />
              Event Definitions
            </h2>

            <div className="overflow-x-auto mb-6 border border-gray-200 rounded-lg">
              <table className="w-full min-w-[800px] text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900 whitespace-nowrap">Event</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Trigger Condition</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Live Mode</th>
                    <th className="text-left p-3 font-semibold text-gray-900">VOD Mode</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 font-medium text-gray-900 whitespace-nowrap">Impression</td>
                    <td className="p-3 text-gray-600">First segment of ad loaded</td>
                    <td className="p-3 text-gray-600">Segment fetch from CDN</td>
                    <td className="p-3 text-gray-600">Segment fetch from CDN</td>
                  </tr>
                  <tr>
                    <td className="p-3 font-medium text-gray-900 whitespace-nowrap">Start</td>
                    <td className="p-3 text-gray-600">Playback begins (first byte)</td>
                    <td className="p-3 text-gray-600">Player reports play event</td>
                    <td className="p-3 text-gray-600">Player reports play event</td>
                  </tr>
                  <tr>
                    <td className="p-3 font-medium text-gray-900 whitespace-nowrap">First Quartile</td>
                    <td className="p-3 text-gray-600">25% of ad duration played</td>
                    <td className="p-3 text-gray-600">Time-based (segment count)</td>
                    <td className="p-3 text-gray-600">Time-based (segment count)</td>
                  </tr>
                  <tr>
                    <td className="p-3 font-medium text-gray-900 whitespace-nowrap">Midpoint</td>
                    <td className="p-3 text-gray-600">50% of ad duration played</td>
                    <td className="p-3 text-gray-600">Time-based</td>
                    <td className="p-3 text-gray-600">Time-based</td>
                  </tr>
                  <tr>
                    <td className="p-3 font-medium text-gray-900 whitespace-nowrap">Third Quartile</td>
                    <td className="p-3 text-gray-600">75% of ad duration played</td>
                    <td className="p-3 text-gray-600">Time-based</td>
                    <td className="p-3 text-gray-600">Time-based</td>
                  </tr>
                  <tr>
                    <td className="p-3 font-medium text-gray-900 whitespace-nowrap">Complete</td>
                    <td className="p-3 text-gray-600">100% of ad duration played</td>
                    <td className="p-3 text-gray-600">Last segment fetched</td>
                    <td className="p-3 text-gray-600">Last segment fetched</td>
                  </tr>
                  <tr>
                    <td className="p-3 font-medium text-gray-900 whitespace-nowrap">Skip</td>
                    <td className="p-3 text-gray-600">User skips (if enabled)</td>
                    <td className="p-3 text-gray-600">N/A (SSAI ads not skippable)</td>
                    <td className="p-3 text-gray-600">Optional feature</td>
                  </tr>
                  <tr>
                    <td className="p-3 font-medium text-gray-900 whitespace-nowrap">Error</td>
                    <td className="p-3 text-gray-600">Playback failure</td>
                    <td className="p-3 text-gray-600">Segment 404 or timeout</td>
                    <td className="p-3 text-gray-600">Segment 404 or timeout</td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div className="bg-info/10 border border-info/20 rounded-lg p-4">
              <h3 className="font-semibold text-gray-900 text-sm mb-2">Note on Segment-Based Tracking</h3>
              <p className="text-sm text-gray-600">
                SSAI tracks based on segment fetches, not player-reported time positions. This is more 
                reliable because segment requests are observable server-side. For a 30-second ad with 
                2-second segments, quartile triggers occur at segments 4, 8, 12, and 15.
              </p>
            </div>
          </section>

          {/* Deduplication */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Target className="w-6 h-6 text-brand-600" />
              Deduplication Keys
            </h2>

            <p className="text-gray-600 mb-4">
              Each tracking event is uniquely identified by a composite key that ensures exactly-once delivery:
            </p>

            <div className="bg-gray-50 rounded-lg p-6 mb-4">
              <pre className="text-sm text-gray-700 overflow-x-auto">
{`// Dedupe key format
{session_id}:{ad_id}:{position_in_pod}:{event_type}

// Example
"sess_a1b2c3:ad_xyz789:0:firstQuartile"

// Components:
//   session_id      - Unique viewer session (UUID)
//   ad_id           - Creative identifier from demand partner
//   position_in_pod - 0-indexed position of ad in the break
//   event_type      - impression|start|firstQuartile|midpoint|thirdQuartile|complete|error`}
              </pre>
            </div>

            <div className="grid md:grid-cols-2 gap-4">
              <div className="border rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 text-sm mb-2 flex items-center gap-2">
                  <CheckCircle className="w-4 h-4 text-success" />
                  Dedupe Behavior
                </h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• First event with key → delivered</li>
                  <li>• Duplicate within TTL → silently dropped</li>
                  <li>• Duplicate after TTL → logged as &quot;late&quot;</li>
                  <li>• TTL: 24 hours for VOD, 4 hours for Live</li>
                </ul>
              </div>
              <div className="border rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 text-sm mb-2 flex items-center gap-2">
                  <RefreshCw className="w-4 h-4 text-brand-600" />
                  Retry Behavior
                </h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Initial delivery → immediate</li>
                  <li>• First retry → 500ms delay</li>
                  <li>• Second retry → 2s delay</li>
                  <li>• Third retry → 10s delay (final)</li>
                  <li>• Max retries: 3 per event</li>
                </ul>
              </div>
            </div>
          </section>

          {/* Live vs VOD Differences */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Clock className="w-6 h-6 text-brand-600" />
              Live vs VOD Tracking Differences
            </h2>

            <div className="space-y-6">
              <div className="border rounded-lg overflow-hidden">
                <div className="bg-info/10 px-4 py-3 border-b border-info/20">
                  <h3 className="font-semibold text-info">Live Mode</h3>
                </div>
                <div className="p-4">
                  <ul className="space-y-2 text-sm text-gray-600">
                    <li className="flex items-start gap-2">
                      <span className="text-info mt-1">●</span>
                      <span><strong>Real-time tracking:</strong> Events fired as segments are fetched (sub-second latency)</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-info mt-1">●</span>
                      <span><strong>Session TTL:</strong> 4 hours (covers typical live viewing session)</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-info mt-1">●</span>
                      <span><strong>Seek handling:</strong> Seeks within ad break don&apos;t re-fire events (dedupe)</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-info mt-1">●</span>
                      <span><strong>DVR playback:</strong> Events fire on DVR catch-up if within TTL</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-info mt-1">●</span>
                      <span><strong>Pause handling:</strong> Paused session still valid, events fire on resume</span>
                    </li>
                  </ul>
                </div>
              </div>

              <div className="border rounded-lg overflow-hidden">
                <div className="bg-brand-50 px-4 py-3 border-b border-brand-200">
                  <h3 className="font-semibold text-brand-800">VOD Mode</h3>
                </div>
                <div className="p-4">
                  <ul className="space-y-2 text-sm text-gray-600">
                    <li className="flex items-start gap-2">
                      <span className="text-brand-700 mt-1">●</span>
                      <span><strong>Segment-based tracking:</strong> Events mapped to pre-computed segment positions</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-brand-700 mt-1">●</span>
                      <span><strong>Session TTL:</strong> 24 hours (covers multi-day binge watching)</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-brand-700 mt-1">●</span>
                      <span><strong>Resume position:</strong> Tracked; events only fire for unwatched segments</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-brand-700 mt-1">●</span>
                      <span><strong>Re-watch:</strong> Same session → no duplicate events. New session → full event set.</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-brand-700 mt-1">●</span>
                      <span><strong>Fast-forward:</strong> Skipped segments don&apos;t trigger quartile events</span>
                    </li>
                  </ul>
                </div>
              </div>
            </div>
          </section>

          {/* Late Events */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <AlertTriangle className="w-6 h-6 text-warning" />
              Late Event Handling
            </h2>

            <p className="text-gray-600 mb-4">
              Events that arrive after the session TTL are considered &quot;late&quot; and handled specially:
            </p>

            <div className="bg-warning/10 border border-warning/20 rounded-lg p-6 mb-4">
              <h3 className="font-semibold text-gray-900 mb-3">Late Event Policy</h3>
              <div className="grid md:grid-cols-2 gap-4">
                <div>
                  <h4 className="font-medium text-gray-900 text-sm mb-2">What happens:</h4>
                  <ul className="text-sm text-gray-600 space-y-1">
                    <li>• Event logged to <code>late_events</code> table</li>
                    <li>• Not delivered to demand partners</li>
                    <li>• Not counted in primary metrics</li>
                    <li>• Available in reconciliation reports</li>
                  </ul>
                </div>
                <div>
                  <h4 className="font-medium text-gray-900 text-sm mb-2">Common causes:</h4>
                  <ul className="text-sm text-gray-600 space-y-1">
                    <li>• Client offline then reconnects</li>
                    <li>• Aggressive browser sleep mode</li>
                    <li>• Network outage during playback</li>
                    <li>• SDK retry after long delay</li>
                  </ul>
                </div>
              </div>
            </div>

            <div className="text-sm text-gray-600">
              <strong>Typical late event rate:</strong> &lt;0.1% of total events. Spikes indicate 
              client SDK issues or network problems.
            </div>
          </section>

          {/* Reconciliation */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Zap className="w-6 h-6 text-brand-600" />
              Reconciliation &amp; Reporting
            </h2>

            <p className="text-gray-600 mb-4">
              For billing and partner reconciliation, we provide detailed event exports:
            </p>

            <div className="bg-gray-50 rounded-lg p-6 mb-4">
              <h3 className="font-semibold text-gray-900 mb-3">Export Fields</h3>
              <pre className="text-sm text-gray-700 overflow-x-auto">
{`{
  "event_id": "evt_abc123",
  "session_id": "sess_xyz789",
  "ad_id": "creative_456",
  "demand_partner": "partner_a",
  "event_type": "complete",
  "timestamp": "2024-01-15T14:30:00Z",
  "delivered_at": "2024-01-15T14:30:00.150Z",
  "delivery_latency_ms": 150,
  "retry_count": 0,
  "dedupe_status": "first",  // first | duplicate | late
  "client_ip_hash": "sha256:...",
  "user_agent_category": "ctv_roku",
  "geo_country": "US",
  "geo_region": "CA"
}`}
              </pre>
            </div>

            <div className="grid md:grid-cols-3 gap-4">
              <div className="border rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 text-sm mb-2">Daily Export</h3>
                <p className="text-sm text-gray-600">Full event log for previous day. Available by 06:00 UTC.</p>
              </div>
              <div className="border rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 text-sm mb-2">Real-time API</h3>
                <p className="text-sm text-gray-600">Query events by session, ad, or time range. 15-minute delay.</p>
              </div>
              <div className="border rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 text-sm mb-2">Discrepancy Report</h3>
                <p className="text-sm text-gray-600">Partner-side vs our-side event diff. Weekly summary.</p>
              </div>
            </div>
          </section>

          {/* Common Discrepancy Causes */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">Common Discrepancy Causes</h2>

            <div className="space-y-4">
              <div className="border rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 text-sm mb-2">Our count higher than partner</h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Partner filtering by viewability (we count segment fetch)</li>
                  <li>• Partner has stricter fraud filtering</li>
                  <li>• Timezone mismatch in daily rollup</li>
                </ul>
              </div>
              <div className="border rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 text-sm mb-2">Our count lower than partner</h3>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Partner counting client-side (double-fires)</li>
                  <li>• Partner not deduplicating retries</li>
                  <li>• Test traffic included in partner count</li>
                </ul>
              </div>
              <div className="border rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 text-sm mb-2">Expected variance</h3>
                <p className="text-sm text-gray-600">
                  ±2% variance is normal due to timing differences, timezone rollups, and edge cases. 
                  Variance &gt;5% triggers automatic investigation.
                </p>
              </div>
            </div>
          </section>

          {/* Related Documentation */}
          <section className="bg-brand-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/modes/live" className="bg-white p-4 rounded-lg border border-brand-200 hover:border-brand-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Live Mode →</h3>
                <p className="text-sm text-gray-600">Real-time ad insertion flow</p>
              </Link>
              <Link href="/docs/ssai/modes/vod" className="bg-white p-4 rounded-lg border border-brand-200 hover:border-brand-400 transition-colors">
                <h3 className="font-semibold text-gray-900">VOD Mode →</h3>
                <p className="text-sm text-gray-600">Pre-stitched manifest tracking</p>
              </Link>
              <Link href="/docs/ssai/operations" className="bg-white p-4 rounded-lg border border-brand-200 hover:border-brand-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Operations →</h3>
                <p className="text-sm text-gray-600">Failure modes and SLAs</p>
              </Link>
              <Link href="/reporting" className="bg-white p-4 rounded-lg border border-brand-200 hover:border-brand-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Reporting Dashboard →</h3>
                <p className="text-sm text-gray-600">Real-time metrics and exports</p>
              </Link>
            </div>
            
            <div className="mt-6 pt-6 border-t border-brand-200">
              <Link href="/docs/ssai" className="inline-flex items-center text-brand-700 font-medium hover:text-brand-800">
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
