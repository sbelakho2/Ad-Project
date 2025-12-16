'use client';

import Link from 'next/link';
import { Shield, AlertTriangle, Clock, Zap, Server, RefreshCw, CheckCircle, XCircle } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function OperationalGuaranteesPage() {
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
            <span className="text-gray-900">Operational Guarantees</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              Operational Guarantees &amp; Failure Modes
            </h1>
            <p className="text-xl text-gray-600">
              How ApexMediation SSAI handles failures, timeouts, and edge cases. 
              This page documents exact fallback behavior for Live vs VOD modes.
            </p>
          </div>

          {/* Core Principle */}
          <div className="bg-emerald-50 border border-emerald-200 rounded-lg p-6 mb-12">
            <h2 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
              <Shield className="w-5 h-5 text-emerald-600" />
              Core Principle: Never Black Screen
            </h2>
            <p className="text-gray-600 text-sm">
              The system is designed to always deliver watchable content. When ad delivery fails, 
              we fall back to slate (neutral filler) or pass through content—never a black screen or player stall.
            </p>
          </div>

          {/* SLA Metrics */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Clock className="w-6 h-6 text-emerald-600" />
              SLA Metrics
            </h2>
            
            <div className="overflow-x-auto mb-6">
              <table className="w-full text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Metric</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Target</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Notes</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">Pod Decision Latency (p99)</td>
                    <td className="p-3 text-gray-600">&lt; 200ms</td>
                    <td className="p-3 text-gray-600">From cue detection to filled pod</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Manifest Generation (p99)</td>
                    <td className="p-3 text-gray-600">&lt; 50ms</td>
                    <td className="p-3 text-gray-600">Stitched manifest response time</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Session Availability</td>
                    <td className="p-3 text-gray-600">99.9%</td>
                    <td className="p-3 text-gray-600">Active sessions recoverable after incident</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Tracking Event Delivery</td>
                    <td className="p-3 text-gray-600">99.95%</td>
                    <td className="p-3 text-gray-600">Exactly-once delivery with retry</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Pod Deadline Miss Rate</td>
                    <td className="p-3 text-gray-600">&lt; 5%</td>
                    <td className="p-3 text-gray-600">Percent of breaks requiring fallback</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          {/* Failure Scenarios */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <AlertTriangle className="w-6 h-6 text-amber-600" />
              Failure Scenarios &amp; Fallbacks
            </h2>

            <div className="space-y-6">
              {/* Partner Timeout */}
              <div className="border rounded-lg overflow-hidden">
                <div className="bg-gray-50 px-4 py-3 border-b">
                  <h3 className="font-semibold text-gray-900">Partner Timeout</h3>
                </div>
                <div className="p-4">
                  <p className="text-sm text-gray-600 mb-4">
                    Demand partner doesn&apos;t respond within the bid deadline.
                  </p>
                  <div className="grid md:grid-cols-2 gap-4">
                    <div className="bg-blue-50 rounded p-3">
                      <h4 className="font-medium text-gray-900 text-sm mb-2">Live Mode</h4>
                      <ul className="text-sm text-gray-600 space-y-1">
                        <li>• Partner excluded from current pod</li>
                        <li>• If no ads available: serve slate</li>
                        <li>• Circuit breaker trips after 3 consecutive timeouts</li>
                        <li>• Auto-heal after 60s cooldown</li>
                      </ul>
                    </div>
                    <div className="bg-purple-50 rounded p-3">
                      <h4 className="font-medium text-gray-900 text-sm mb-2">VOD Mode</h4>
                      <ul className="text-sm text-gray-600 space-y-1">
                        <li>• Partner excluded, try next partner</li>
                        <li>• Longer timeout budget (500ms vs 200ms)</li>
                        <li>• If no ads: skip break, content continues</li>
                        <li>• Retry on next session for same asset</li>
                      </ul>
                    </div>
                  </div>
                </div>
              </div>

              {/* Cue Drift */}
              <div className="border rounded-lg overflow-hidden">
                <div className="bg-gray-50 px-4 py-3 border-b">
                  <h3 className="font-semibold text-gray-900">Cue Drift (Late/Early Cue)</h3>
                </div>
                <div className="p-4">
                  <p className="text-sm text-gray-600 mb-4">
                    SCTE-35 cue arrives after expected position or with unexpected timing.
                  </p>
                  <div className="grid md:grid-cols-2 gap-4">
                    <div className="bg-blue-50 rounded p-3">
                      <h4 className="font-medium text-gray-900 text-sm mb-2">Live Mode</h4>
                      <ul className="text-sm text-gray-600 space-y-1">
                        <li>• Late cue (&lt;2s): align to next segment boundary</li>
                        <li>• Very late cue (&gt;2s): skip break, log warning</li>
                        <li>• Early cue: buffer until segment boundary</li>
                        <li>• Metrics track drift distribution</li>
                      </ul>
                    </div>
                    <div className="bg-purple-50 rounded p-3">
                      <h4 className="font-medium text-gray-900 text-sm mb-2">VOD Mode</h4>
                      <ul className="text-sm text-gray-600 space-y-1">
                        <li>• Cue positions fixed at manifest generation</li>
                        <li>• No runtime drift (pre-stitched)</li>
                        <li>• Original cue positions preserved in metadata</li>
                      </ul>
                    </div>
                  </div>
                </div>
              </div>

              {/* Missing Segments */}
              <div className="border rounded-lg overflow-hidden">
                <div className="bg-gray-50 px-4 py-3 border-b">
                  <h3 className="font-semibold text-gray-900">Missing Segments (404 Mid-Ad)</h3>
                </div>
                <div className="p-4">
                  <p className="text-sm text-gray-600 mb-4">
                    Ad segment returns 404 or times out during playback.
                  </p>
                  <div className="grid md:grid-cols-2 gap-4">
                    <div className="bg-blue-50 rounded p-3">
                      <h4 className="font-medium text-gray-900 text-sm mb-2">Live Mode</h4>
                      <ul className="text-sm text-gray-600 space-y-1">
                        <li>• Substitute slate segment at same duration</li>
                        <li>• Creative flagged, excluded from future pods</li>
                        <li>• Tracking event: error with reason</li>
                        <li>• Alert to ops if pattern detected</li>
                      </ul>
                    </div>
                    <div className="bg-purple-50 rounded p-3">
                      <h4 className="font-medium text-gray-900 text-sm mb-2">VOD Mode</h4>
                      <ul className="text-sm text-gray-600 space-y-1">
                        <li>• Manifest pre-validated before serving</li>
                        <li>• All segment URLs verified reachable</li>
                        <li>• Stale ads auto-expired from cache</li>
                        <li>• Player sees discontinuity if unavoidable</li>
                      </ul>
                    </div>
                  </div>
                </div>
              </div>

              {/* Encoder Restart */}
              <div className="border rounded-lg overflow-hidden">
                <div className="bg-gray-50 px-4 py-3 border-b">
                  <h3 className="font-semibold text-gray-900">Encoder/Origin Restart</h3>
                </div>
                <div className="p-4">
                  <p className="text-sm text-gray-600 mb-4">
                    Origin stream resets with new sequence numbers or timestamp discontinuity.
                  </p>
                  <div className="grid md:grid-cols-2 gap-4">
                    <div className="bg-blue-50 rounded p-3">
                      <h4 className="font-medium text-gray-900 text-sm mb-2">Live Mode</h4>
                      <ul className="text-sm text-gray-600 space-y-1">
                        <li>• Detect via media sequence jump</li>
                        <li>• Insert <code>#EXT-X-DISCONTINUITY</code></li>
                        <li>• Abort in-flight pod if mid-break</li>
                        <li>• Session continues (no reconnect needed)</li>
                      </ul>
                    </div>
                    <div className="bg-purple-50 rounded p-3">
                      <h4 className="font-medium text-gray-900 text-sm mb-2">VOD Mode</h4>
                      <ul className="text-sm text-gray-600 space-y-1">
                        <li>• N/A (VOD origin is immutable)</li>
                        <li>• Asset version tracking prevents stale refs</li>
                      </ul>
                    </div>
                  </div>
                </div>
              </div>

              {/* Safe Mode Trigger */}
              <div className="border rounded-lg overflow-hidden">
                <div className="bg-gray-50 px-4 py-3 border-b">
                  <h3 className="font-semibold text-gray-900">Safe Mode Activation</h3>
                </div>
                <div className="p-4">
                  <p className="text-sm text-gray-600 mb-4">
                    System detects elevated error rates or latency.
                  </p>
                  <div className="bg-amber-50 border border-amber-200 rounded p-4 mb-4">
                    <h4 className="font-medium text-gray-900 text-sm mb-2">Thresholds (from SafeModeManager)</h4>
                    <ul className="text-sm text-gray-600 space-y-1">
                      <li>• Error rate threshold: <strong>10%</strong> of requests</li>
                      <li>• Latency threshold: <strong>500ms</strong> p95</li>
                      <li>• Minimum samples: <strong>100</strong> before triggering</li>
                    </ul>
                  </div>
                  <div className="space-y-2">
                    <div className="flex items-start gap-2">
                      <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                      <span className="text-sm text-gray-600"><strong>Reduced fanout:</strong> Only healthy partners called</span>
                    </div>
                    <div className="flex items-start gap-2">
                      <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                      <span className="text-sm text-gray-600"><strong>Reduced timeout:</strong> 100ms hard deadline</span>
                    </div>
                    <div className="flex items-start gap-2">
                      <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                      <span className="text-sm text-gray-600"><strong>Slate fallback:</strong> Enabled by default</span>
                    </div>
                    <div className="flex items-start gap-2">
                      <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                      <span className="text-sm text-gray-600"><strong>Config freeze:</strong> No config changes during incident</span>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </section>

          {/* Fallback Ladder */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <RefreshCw className="w-6 h-6 text-emerald-600" />
              Fallback Ladder
            </h2>
            
            <p className="text-gray-600 mb-4">
              When primary ad delivery fails, the system cascades through fallback options:
            </p>

            <ol className="space-y-4 list-decimal pl-5">
              <li className="pl-2">
                <div className="pb-4">
                  <h3 className="font-semibold text-gray-900">Try Alternate Partners</h3>
                  <p className="text-sm text-gray-600">If primary partner times out, immediately try secondary/tertiary partners if time permits.</p>
                </div>
              </li>
              
              <li className="pl-2">
                <div className="pb-4">
                  <h3 className="font-semibold text-gray-900">Serve House Ads</h3>
                  <p className="text-sm text-gray-600">Publisher-provided house ads (promos, PSAs) configured as zero-cost fallback.</p>
                </div>
              </li>
              
              <li className="pl-2">
                <div className="pb-4">
                  <h3 className="font-semibold text-gray-900">Serve Slate</h3>
                  <p className="text-sm text-gray-600">Generic slate content (spinning logo, &quot;We&apos;ll be right back&quot;) for the break duration.</p>
                </div>
              </li>
              
              <li className="pl-2">
                <div className="pb-4">
                  <h3 className="font-semibold text-gray-900">Skip Break (VOD only)</h3>
                  <p className="text-sm text-gray-600">For VOD, if all options exhausted, break is omitted and content continues seamlessly.</p>
                </div>
              </li>
            </ol>
          </section>

          {/* Circuit Breakers */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Zap className="w-6 h-6 text-emerald-600" />
              Circuit Breakers
            </h2>
            
            <p className="text-gray-600 mb-4">
              Per-partner circuit breakers prevent cascading failures:
            </p>

            <div className="bg-gray-50 rounded-lg p-6">
              <pre className="text-sm text-gray-700 overflow-x-auto">
{`// From SSAICircuitBreaker.ts
export const CircuitBreakerConfig = {
  // Trip conditions
  errorRateThreshold: 0.5,    // 50% error rate trips breaker
  latencyThresholdMs: 500,    // p95 > 500ms trips breaker
  minRequestsForTrip: 10,     // Need 10 requests before evaluation
  
  // Recovery
  cooldownMs: 60000,          // 60s cooldown before retry
  halfOpenRequests: 3,        // 3 test requests in half-open state
  recoveryThreshold: 0.8,     // 80% success to fully recover
};`}
              </pre>
            </div>

            <div className="mt-4 grid md:grid-cols-3 gap-4">
              <div className="bg-emerald-50 rounded p-4 text-center">
                <div className="text-2xl font-bold text-emerald-700">CLOSED</div>
                <p className="text-sm text-gray-600 mt-1">Normal operation</p>
              </div>
              <div className="bg-red-50 rounded p-4 text-center">
                <div className="text-2xl font-bold text-red-700">OPEN</div>
                <p className="text-sm text-gray-600 mt-1">Partner bypassed</p>
              </div>
              <div className="bg-amber-50 rounded p-4 text-center">
                <div className="text-2xl font-bold text-amber-700">HALF-OPEN</div>
                <p className="text-sm text-gray-600 mt-1">Testing recovery</p>
              </div>
            </div>
          </section>

          {/* Incident Response */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Server className="w-6 h-6 text-emerald-600" />
              Incident Response
            </h2>
            
            <p className="text-gray-600 mb-4">
              During major incidents, the system provides:
            </p>

            <div className="space-y-3">
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <h3 className="font-semibold text-gray-900">Incident Bundle</h3>
                  <p className="text-sm text-gray-600">Exportable evidence bundle with decision traces, manifest versions, partner timings, and error logs.</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <h3 className="font-semibold text-gray-900">Auto-Rollback</h3>
                  <p className="text-sm text-gray-600">If new config causes degradation, automatic rollback to last-known-good state.</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <h3 className="font-semibold text-gray-900">Partner Quarantine</h3>
                  <p className="text-sm text-gray-600">Problematic partners automatically isolated until manual review.</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <h3 className="font-semibold text-gray-900">Real-time Alerting</h3>
                  <p className="text-sm text-gray-600">PagerDuty/Slack integration for p1 incidents with runbook links.</p>
                </div>
              </div>
            </div>
          </section>

          {/* Related Documentation */}
          <section className="bg-emerald-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/modes/live" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Live Mode →</h3>
                <p className="text-sm text-gray-600">Latency budgets and sliding window</p>
              </Link>
              <Link href="/docs/ssai/modes/vod" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">VOD Mode →</h3>
                <p className="text-sm text-gray-600">Pre-stitched manifest handling</p>
              </Link>
              <Link href="/docs/ssai/measurement" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Measurement →</h3>
                <p className="text-sm text-gray-600">Tracking and exactly-once semantics</p>
              </Link>
              <Link href="/slo" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">SLO Dashboard →</h3>
                <p className="text-sm text-gray-600">Real-time service level objectives</p>
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
