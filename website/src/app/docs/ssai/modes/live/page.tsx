'use client';

import Link from 'next/link';
import { CheckCircle, AlertTriangle, Clock, Zap, Shield, Activity, Settings, Server } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function LiveModePage() {
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
            <span className="text-gray-900">Live Mode</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              Live SSAI Mode
            </h1>
            <p className="text-xl text-gray-600">
              Real-time ad insertion for live streams with strict latency requirements, 
              sliding window management, and resilience patterns.
            </p>
          </div>

          {/* Latency Requirements */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Clock className="w-6 h-6 text-emerald-600" />
              Latency Requirements
            </h2>
            
            <p className="text-gray-600 mb-4">
              Live SSAI operates under strict timing constraints. The system must detect ad break signals, 
              run decisioning, and stitch ad segments before the player requests them.
            </p>

            <div className="bg-gray-50 rounded-lg p-6 mb-6">
              <h3 className="font-semibold text-gray-900 mb-4">Latency Budget (per Ad Break)</h3>
              <div className="overflow-x-auto">
                <table className="w-full min-w-full text-sm border-collapse">
                  <thead>
                    <tr className="border-b border-gray-200">
                      <th className="text-left py-2 pr-4 font-semibold text-gray-900">Stage</th>
                      <th className="text-right py-2 pr-4 font-semibold text-gray-900">Target</th>
                      <th className="text-right py-2 pr-4 font-semibold text-gray-900">p99 Max</th>
                      <th className="text-left py-2 font-semibold text-gray-900">Notes</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-gray-100">
                    <tr>
                      <td className="py-2 pr-4 text-gray-700">Signal Detection</td>
                      <td className="py-2 pr-4 text-right text-gray-600">&lt;10ms</td>
                      <td className="py-2 pr-4 text-right text-gray-600">50ms</td>
                      <td className="py-2 text-gray-600">SCTE-35/DATERANGE parsing</td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700">Pod Decisioning</td>
                      <td className="py-2 pr-4 text-right text-gray-600">&lt;100ms</td>
                      <td className="py-2 pr-4 text-right text-gray-600">200ms</td>
                      <td className="py-2 text-gray-600">Partner requests + filtering</td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700">Manifest Stitching</td>
                      <td className="py-2 pr-4 text-right text-gray-600">&lt;20ms</td>
                      <td className="py-2 pr-4 text-right text-gray-600">50ms</td>
                      <td className="py-2 text-gray-600">Segment list + discontinuities</td>
                    </tr>
                    <tr className="bg-emerald-50">
                      <td className="py-2 pr-4 text-gray-900 font-medium">Total</td>
                      <td className="py-2 pr-4 text-right text-gray-900 font-medium">&lt;130ms</td>
                      <td className="py-2 pr-4 text-right text-gray-900 font-medium">300ms</td>
                      <td className="py-2 text-gray-600">Before break start</td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>

            <div className="border-l-4 border-amber-500 pl-4 bg-amber-50 p-4 rounded-r-lg">
              <h3 className="font-semibold text-gray-900 mb-2">Deadline Enforcement</h3>
              <p className="text-gray-600 text-sm">
                The <code>DecisionSLAEnforcer</code> tracks time remaining until break start. 
                If the deadline approaches (&lt;100ms remaining), the system enters &quot;safe mode&quot;: 
                partner fanout is reduced, and slate (fallback) content is served rather than risking 
                a late decision that causes black screen.
              </p>
            </div>
          </section>

          {/* Sliding Window */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Activity className="w-6 h-6 text-emerald-600" />
              Sliding Window Management
            </h2>
            
            <p className="text-gray-600 mb-4">
              Live manifests maintain a sliding window of segments. As new segments are added, 
              old segments slide out. The SSAI system handles this correctly even when ad breaks 
              span window boundaries.
            </p>

            <div className="grid md:grid-cols-2 gap-6 mb-6">
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">HLS Live Window</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Window size: <code>liveWindowSize</code> segments (default: 6)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>EXT-X-MEDIA-SEQUENCE</code> increments monotonically</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>EXT-X-DISCONTINUITY-SEQUENCE</code> tracks sliding discontinuities</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>No <code>EXT-X-PLAYLIST-TYPE</code> for live streams</span>
                  </li>
                </ul>
              </div>
              
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">DASH Live Window</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>type=&quot;dynamic&quot;</code> MPD</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>timeShiftBufferDepth=&quot;PT5M&quot;</code> (default)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>minimumUpdatePeriod=&quot;PT2S&quot;</code> for manifest refresh</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Stable <code>UTCTiming</code> for player sync</span>
                  </li>
                </ul>
              </div>
            </div>

            <div className="bg-gray-900 rounded-lg p-4 mb-4">
              <h3 className="font-semibold text-white mb-2">Window Behavior During Ad Break</h3>
              <pre className="text-sm text-gray-300 overflow-x-auto">
{`Timeline:
  [C1][C2][C3][C4][C5][C6]   ← Window before break (6 content segments)
       ↓ Break signaled
  [C2][C3][C4][C5][C6][A1]   ← First ad segment enters window
  [C3][C4][C5][C6][A1][A2]   ← Second ad segment, content sliding out
  [C4][C5][C6][A1][A2][A3]   ← Ad break in middle of window
       ↓ Break ends
  [C6][A1][A2][A3][A4][C7]   ← Content resumes, ads sliding out
  [A2][A3][A4][C7][C8][C9]   ← Mixed window (normal)
  [C7][C8][C9][C10][C11][C12] ← Ad break fully out of window`}
              </pre>
            </div>
          </section>

          {/* Resilience */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Shield className="w-6 h-6 text-emerald-600" />
              Resilience Patterns
            </h2>
            
            <p className="text-gray-600 mb-4">
              Live streams require robust fault tolerance. The SSAI system implements multiple 
              resilience patterns based on the <code>SafeModeManager</code> and <code>SSAICircuitBreaker</code>.
            </p>

            <div className="space-y-4">
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <Zap className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Circuit Breaker</h3>
                  <p className="text-gray-600 text-sm mb-2">
                    Per-partner circuit breakers prevent cascading failures. Based on actual 
                    <code>SSAICircuitBreaker</code> implementation:
                  </p>
                  <ul className="text-sm text-gray-600 space-y-1">
                    <li>• <strong>Error threshold:</strong> 50% failure rate triggers open state</li>
                    <li>• <strong>Timeout threshold:</strong> Partners returning &gt;2s are tracked</li>
                    <li>• <strong>Cooldown:</strong> 30s in open state before half-open retry</li>
                    <li>• <strong>Recovery:</strong> 3 consecutive successes to close</li>
                  </ul>
                </div>
              </div>
              
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <Settings className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Safe Mode</h3>
                  <p className="text-gray-600 text-sm mb-2">
                    Automatic degradation when system health deteriorates. From <code>SafeModeManager</code>:
                  </p>
                  <ul className="text-sm text-gray-600 space-y-1">
                    <li>• <strong>Error rate threshold:</strong> 10% triggers safe mode</li>
                    <li>• <strong>Latency threshold:</strong> 500ms p99 triggers safe mode</li>
                    <li>• <strong>Minimum samples:</strong> 100 requests before evaluation</li>
                    <li>• <strong>Behaviors:</strong> Use slate, reduced timeouts (100ms), disable non-critical</li>
                  </ul>
                </div>
              </div>
              
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <Server className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Slate Fallback</h3>
                  <p className="text-gray-600 text-sm">
                    When ads cannot be decisioned in time, pre-encoded slate content fills the break. 
                    The <code>SlateManager</code> maintains ready-to-use slate segments in all 
                    supported formats (HLS/TS, HLS/CMAF, DASH/CMAF).
                  </p>
                </div>
              </div>
            </div>
          </section>

          {/* Operational Controls */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Settings className="w-6 h-6 text-emerald-600" />
              Operational Controls
            </h2>
            
            <p className="text-gray-600 mb-4">
              Live streams require operational controls for incidents and maintenance. 
              Based on actual implementation in the operability module:
            </p>

            <div className="grid md:grid-cols-2 gap-6">
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Kill Switches</h3>
                <p className="text-sm text-gray-600 mb-2">
                  Granular kill switches via <code>KillSwitchManager</code>:
                </p>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Per-tenant disable</li>
                  <li>• Per-channel disable</li>
                  <li>• Per-region disable</li>
                  <li>• Per-partner disable</li>
                  <li>• Per-format disable (HLS/DASH)</li>
                  <li>• Per-protocol disable (TS/CMAF)</li>
                </ul>
              </div>
              
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Rollout Controls</h3>
                <p className="text-sm text-gray-600 mb-2">
                  Gradual rollout via <code>RolloutManager</code>:
                </p>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Percentage-based rollout (0-100%)</li>
                  <li>• Geographic targeting</li>
                  <li>• Device type targeting</li>
                  <li>• Player version targeting</li>
                  <li>• A/B experiment support</li>
                </ul>
              </div>
            </div>
          </section>

          {/* Session Management */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Activity className="w-6 h-6 text-emerald-600" />
              Live Session Management
            </h2>
            
            <p className="text-gray-600 mb-4">
              Live sessions have different lifecycle requirements than VOD. Based on 
              <code>SessionService</code> implementation:
            </p>

            <div className="overflow-x-auto">
              <table className="w-full min-w-full text-sm border-collapse">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Behavior</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Live</th>
                    <th className="text-left p-3 font-semibold text-gray-900">VOD</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">Session TTL</td>
                    <td className="p-3 text-gray-600">Refresh-on-access (sliding window)</td>
                    <td className="p-3 text-gray-600">Fixed TTL (content duration + buffer)</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Manifest Caching</td>
                    <td className="p-3 text-gray-600"><code>private, no-store</code></td>
                    <td className="p-3 text-gray-600"><code>private, no-store</code></td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Ad Decisioning</td>
                    <td className="p-3 text-gray-600">Just-in-time (per break)</td>
                    <td className="p-3 text-gray-600">At session creation (all breaks)</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Retry Storm Protection</td>
                    <td className="p-3 text-gray-600">Critical (active)</td>
                    <td className="p-3 text-gray-600">Less critical</td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div className="bg-amber-50 rounded-lg p-6 mt-6">
              <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                <AlertTriangle className="w-5 h-5 text-amber-600" />
                Retry Storm Prevention
              </h3>
              <p className="text-sm text-gray-600">
                During network flaps, players may request manifests every 250ms. The 
                <code>SessionService</code> detects retry storms via <code>isRetryStorm()</code> 
                and rate limits to prevent service overload. The system remains stable 
                and session counts don&apos;t explode.
              </p>
            </div>
          </section>

          {/* Origin Restart Handling */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">Origin Restart Handling</h2>
            
            <p className="text-gray-600 mb-4">
              Live origins may restart mid-stream, causing discontinuities. The SSAI system 
              handles this gracefully:
            </p>

            <div className="space-y-3">
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">Timeline Continuity</span>
                  <p className="text-sm text-gray-600">Media sequence and discontinuity sequence continue from pre-restart values</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">Break State Preservation</span>
                  <p className="text-sm text-gray-600">Active ad breaks complete even if origin restarts mid-break</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">Automatic Discontinuity</span>
                  <p className="text-sm text-gray-600">Discontinuity tags inserted at origin restart boundary</p>
                </div>
              </div>
            </div>
          </section>

          {/* Metrics */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">Live-Specific Metrics</h2>
            
            <p className="text-gray-600 mb-4">
              The <code>SSAIMetrics</code> module exposes Prometheus metrics for live stream monitoring:
            </p>

            <div className="bg-gray-50 rounded-lg p-6">
              <div className="grid md:grid-cols-2 gap-4 text-sm">
                <div>
                  <h4 className="font-semibold text-gray-900 mb-2">Counters</h4>
                  <ul className="space-y-1 text-gray-600">
                    <li><code>ssai_sessions_active</code> - Active live sessions</li>
                    <li><code>ssai_breaks_total</code> - Total breaks processed</li>
                    <li><code>ssai_breaks_slate</code> - Breaks served with slate</li>
                    <li><code>ssai_safe_mode_triggers</code> - Safe mode activations</li>
                  </ul>
                </div>
                <div>
                  <h4 className="font-semibold text-gray-900 mb-2">Histograms</h4>
                  <ul className="space-y-1 text-gray-600">
                    <li><code>ssai_decision_latency_ms</code> - Pod decision time</li>
                    <li><code>ssai_stitch_latency_ms</code> - Manifest stitch time</li>
                    <li><code>ssai_manifest_latency_ms</code> - Total manifest request time</li>
                    <li><code>ssai_partner_latency_ms</code> - Per-partner response time</li>
                  </ul>
                </div>
              </div>
            </div>
          </section>

          {/* Next Steps */}
          <section className="bg-emerald-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/modes/vod" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">VOD Mode →</h3>
                <p className="text-sm text-gray-600">Seeking, manifest stability, measurement</p>
              </Link>
              <Link href="/docs/ssai/protocols/hls" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">HLS Protocol →</h3>
                <p className="text-sm text-gray-600">RFC 8216 compliance, DATERANGE markers</p>
              </Link>
              <Link href="/docs/ssai/protocols/dash" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">DASH Protocol →</h3>
                <p className="text-sm text-gray-600">MPD structure, EventStream</p>
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
