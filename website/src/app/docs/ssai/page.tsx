'use client';

import Link from 'next/link';
import { ArrowLeft, Play, Server, Zap, Shield, Clock, Eye, Settings, CheckCircle, FileText, AlertTriangle } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function SSAIOverviewPage() {
  return (
    <div className="min-h-screen bg-white">
      <SiteHeader />
      
      <main className="pt-24 pb-16">
        <div className="max-w-4xl mx-auto px-6">
          {/* Breadcrumb */}
          <Link href="/docs" className="inline-flex items-center gap-2 text-emerald-600 hover:text-emerald-700 mb-8">
            <ArrowLeft className="w-4 h-4" />
            Back to Documentation
          </Link>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              Server-Side Ad Insertion (SSAI)
            </h1>
            <p className="text-xl text-gray-600">
              Complete guide to integrating ApexMediation&apos;s SSAI system for seamless ad stitching across Live and VOD content.
            </p>
          </div>

          {/* Quick Navigation */}
          <div className="grid md:grid-cols-3 gap-4 mb-12">
            <Link href="/docs/ssai/protocols/hls" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">HLS Protocol</h3>
              <p className="text-sm text-gray-600">RFC 8216 compliance, DATERANGE, CDN caching</p>
            </Link>
            <Link href="/docs/ssai/protocols/dash" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">DASH Protocol</h3>
              <p className="text-sm text-gray-600">MPD structure, EventStream, multi-period</p>
            </Link>
            <Link href="/docs/ssai/modes/live" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">Live Mode</h3>
              <p className="text-sm text-gray-600">Latency, resilience, operational controls</p>
            </Link>
            <Link href="/docs/ssai/modes/vod" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">VOD Mode</h3>
              <p className="text-sm text-gray-600">Seeking, manifest stability, measurement</p>
            </Link>
            <Link href="/docs/ssai/containers" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">Containers</h3>
              <p className="text-sm text-gray-600">TS and CMAF/fMP4 segment formats</p>
            </Link>
            <Link href="/docs/ssai/drm" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">DRM Support</h3>
              <p className="text-sm text-gray-600">Widevine, FairPlay, PlayReady policies</p>
            </Link>
            <Link href="/docs/ssai/prerequisites" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">Prerequisites</h3>
              <p className="text-sm text-gray-600">Origin, cue types, codec ladder requirements</p>
            </Link>
            <Link href="/docs/ssai/operations" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">Operations</h3>
              <p className="text-sm text-gray-600">Failure modes, fallback ladder, SLAs</p>
            </Link>
            <Link href="/docs/ssai/cdn" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">CDN Guidance</h3>
              <p className="text-sm text-gray-600">Cache policies, headers, misconfigurations</p>
            </Link>
            <Link href="/docs/ssai/measurement" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">Measurement</h3>
              <p className="text-sm text-gray-600">Tracking events, exactly-once, reconciliation</p>
            </Link>
            <Link href="/docs/ssai/coverage" className="p-4 border border-gray-200 rounded-lg hover:border-emerald-500 transition-colors">
              <h3 className="font-semibold text-gray-900">Playback Matrix</h3>
              <p className="text-sm text-gray-600">Certified player/platform compatibility</p>
            </Link>
          </div>

          {/* What is SSAI */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Server className="w-6 h-6 text-emerald-600" />
              What is Server-Side Ad Insertion?
            </h2>
            <p className="text-gray-600 mb-4">
              Server-Side Ad Insertion (SSAI), also known as Dynamic Ad Insertion (DAI), stitches advertisements 
              directly into the video stream at the server level before delivery to the viewer. This approach 
              eliminates ad blocker effectiveness, ensures seamless playback, and provides broadcast-quality 
              ad experiences across all devices.
            </p>
            <div className="bg-gray-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">Key Benefits</h3>
              <ul className="space-y-2">
                <li className="flex items-start gap-2">
                  <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                  <span className="text-gray-600"><strong>Ad blocker resistant:</strong> Ads are part of the content stream, indistinguishable from program content</span>
                </li>
                <li className="flex items-start gap-2">
                  <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                  <span className="text-gray-600"><strong>Seamless experience:</strong> No buffering or player reinitialization between content and ads</span>
                </li>
                <li className="flex items-start gap-2">
                  <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                  <span className="text-gray-600"><strong>Broad compatibility:</strong> Designed to work across modern HLS and DASH players. Exact compatibility depends on protocol, container, DRM policy, and player capabilities—see the <Link href="/docs/ssai/coverage" className="text-emerald-600 hover:underline">Certified Playback Matrix</Link> and protocol pages.</span>
                </li>
                <li className="flex items-start gap-2">
                  <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                  <span className="text-gray-600"><strong>Accurate measurement:</strong> Server-side tracking with exactly-once delivery guarantees</span>
                </li>
              </ul>
            </div>
          </section>

          {/* Architecture */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Settings className="w-6 h-6 text-emerald-600" />
              Architecture Overview
            </h2>
            
            <div className="bg-gray-900 rounded-lg p-6 mb-6">
              <pre className="text-sm text-gray-300 overflow-x-auto">
{`┌─────────────────────────────────────────────────────────────────────┐
│                        Video Player (SDK)                            │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                  │
│  │ HLS.js      │  │ Shaka       │  │ ExoPlayer   │                  │
│  │ AVPlayer    │  │ dash.js     │  │ Native      │                  │
│  └─────┬───────┘  └──────┬──────┘  └──────┬──────┘                  │
└────────┼─────────────────┼────────────────┼─────────────────────────┘
         │                 │                │
         └─────────────────┼────────────────┘
                           │
                    ┌──────▼──────┐
                    │  CDN Edge   │  ← Segments cached at edge
                    └──────┬──────┘
                           │
              ┌────────────▼────────────┐
              │   SSAI Manifest Service │
              │  ┌───────────────────┐  │
              │  │ Session Manager   │  │
              │  │ Pod Decisioning   │  │
              │  │ Manifest Stitcher │  │
              │  │ Tracking Engine   │  │
              │  └───────────────────┘  │
              └────────────┬────────────┘
                           │
         ┌─────────────────┼─────────────────┐
         │                 │                 │
   ┌─────▼─────┐    ┌──────▼──────┐   ┌──────▼──────┐
   │  Origin   │    │  Ad Server  │   │ Demand      │
   │  (CMS)    │    │  (VAST/VMAP)│   │ Partners    │
   └───────────┘    └─────────────┘   └─────────────┘`}
              </pre>
            </div>

            <h3 className="font-semibold text-gray-900 mb-3">Request Flow</h3>
            <ol className="space-y-3 text-gray-600 list-decimal pl-5">
              <li className="pl-2">
                <strong>Session Init:</strong> Player SDK requests a session manifest URL, providing device capabilities, consent signals, and targeting context.
              </li>
              <li className="pl-2">
                <strong>Manifest Fetch:</strong> Player fetches the personalized HLS/DASH manifest from the SSAI service (CDN may cache segments but not the manifest).
              </li>
              <li className="pl-2">
                <strong>Ad Break Detection:</strong> SSAI service detects ad break opportunities from SCTE-35, EXT-X-DATERANGE, or DASH EventStream signals.
              </li>
              <li className="pl-2">
                <strong>Pod Decisioning:</strong> Real-time auction selects ads matching format requirements, competitive separation rules, and frequency caps.
              </li>
              <li className="pl-2">
                <strong>Manifest Stitching:</strong> Ad segments are stitched into the manifest with proper discontinuity markers and timing information.
              </li>
              <li className="pl-2">
                <strong>Segment Delivery:</strong> Player fetches stitched segments from CDN edge (content and ads appear as unified stream).
              </li>
              <li className="pl-2">
                <strong>Tracking:</strong> Impressions, quartiles, and completion events fire server-side with deduplication for exactly-once delivery.
              </li>
            </ol>
          </section>

          {/* Integration */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Zap className="w-6 h-6 text-emerald-600" />
              Integration
            </h2>
            
            <div className="space-y-6">
              <div>
                <h3 className="font-semibold text-gray-900 mb-2">Session Initialization</h3>
                <div className="bg-gray-50 rounded-lg p-4">
                  <code className="text-sm text-gray-800">
                    POST /v1/session/init
                  </code>
                </div>
                <pre className="bg-gray-900 rounded-lg p-4 mt-2 text-sm text-gray-300 overflow-x-auto">
{`{
  "tenant_id": "your-tenant-id",
  "asset": {
    "id": "content-12345",
    "type": "vod",           // "live" | "vod"
    "duration": 3600         // seconds (VOD only)
  },
  "device": {
    "type": "ctv",           // "web" | "mobile" | "ctv" 
    "os": "tvos",
    "model": "Apple TV 4K",
    "player": "avplayer"
  },
  "format": {
    "protocol": "hls",       // "hls" | "dash"
    "container": "cmaf",     // "ts" | "cmaf"
    "drm": null              // "widevine" | "fairplay" | null
  },
  "consent": {
    "gdpr": true,
    "ccpa": false,
    "us_privacy": "1YNN"
  }
}`}
                </pre>
              </div>

              <div>
                <h3 className="font-semibold text-gray-900 mb-2">Response</h3>
                <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`{
  "session_id": "sess_abc123xyz",
  "manifest_url": "https://ssai.apexmediation.ee/v1/session/sess_abc123xyz/master.m3u8",
  "expires_at": "2025-01-15T12:00:00Z",
  "tracking_url": "https://track.apexmediation.ee/v1/session/sess_abc123xyz"
}`}
                </pre>
              </div>
            </div>
          </section>

          {/* Ad Decisioning */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Play className="w-6 h-6 text-emerald-600" />
              Ad Decisioning & Timing
            </h2>
            
            <h3 className="font-semibold text-gray-900 mb-3">Pod Structure</h3>
            <p className="text-gray-600 mb-4">
              Each ad break is organized as a &quot;pod&quot; containing one or more slots. The decisioning engine fills 
              each slot based on format compatibility, competitive separation, and frequency caps.
            </p>
            
            <div className="bg-gray-50 rounded-lg p-6 mb-6">
              <h4 className="font-semibold text-gray-900 mb-3">Slot Types</h4>
              <div className="grid md:grid-cols-3 gap-4">
                <div className="bg-white p-4 rounded border border-gray-200">
                  <h5 className="font-medium text-gray-900">Pre-roll</h5>
                  <p className="text-sm text-gray-600">Before content playback begins</p>
                </div>
                <div className="bg-white p-4 rounded border border-gray-200">
                  <h5 className="font-medium text-gray-900">Mid-roll</h5>
                  <p className="text-sm text-gray-600">During content at signaled break points</p>
                </div>
                <div className="bg-white p-4 rounded border border-gray-200">
                  <h5 className="font-medium text-gray-900">Post-roll</h5>
                  <p className="text-sm text-gray-600">After content completion</p>
                </div>
              </div>
            </div>

            <h3 className="font-semibold text-gray-900 mb-3">SLA Guarantees</h3>
            <ul className="space-y-2 text-gray-600">
              <li className="flex items-start gap-2">
                <Clock className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <span><strong>Decision Deadline:</strong> Pod decisions complete before break start (p99 &lt; 200ms)</span>
              </li>
              <li className="flex items-start gap-2">
                <Shield className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <span><strong>Fallback Behavior:</strong> If deadline approaches, slate (fallback content) is served rather than black screen</span>
              </li>
              <li className="flex items-start gap-2">
                <Settings className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <span><strong>Safe Mode:</strong> During incidents, system reduces fanout and enforces strict deadlines</span>
              </li>
            </ul>
          </section>

          {/* Content Correctness */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <CheckCircle className="w-6 h-6 text-emerald-600" />
              Content Correctness
            </h2>
            
            <p className="text-gray-600 mb-4">
              The SSAI system enforces strict content correctness rules to ensure seamless playback:
            </p>
            
            <div className="space-y-4">
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Format Compatibility</h3>
                <p className="text-gray-600 text-sm">
                  Ads must match content codec, resolution tier, and container format. HLS/TS content receives 
                  HLS/TS ads; DASH/CMAF content receives DASH/CMAF ads.
                </p>
              </div>
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Audio Continuity</h3>
                <p className="text-gray-600 text-sm">
                  Audio layout (stereo/5.1/Atmos) and loudness (EBU R128/ATSC A/85) are validated and normalized 
                  to prevent jarring transitions.
                </p>
              </div>
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Timing Alignment</h3>
                <p className="text-gray-600 text-sm">
                  Segment durations are aligned to GOP boundaries. Discontinuity sequences are maintained 
                  consistently across renditions.
                </p>
              </div>
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Caption Handling</h3>
                <p className="text-gray-600 text-sm">
                  CEA-608/708 captions are preserved through ad breaks. WebVTT timing is adjusted to account 
                  for ad insertion.
                </p>
              </div>
            </div>
          </section>

          {/* Observability */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Eye className="w-6 h-6 text-emerald-600" />
              Observability & Forensics
            </h2>
            
            <p className="text-gray-600 mb-4">
              Every ad decision is fully explainable for debugging and support:
            </p>
            
            <div className="bg-gray-50 rounded-lg p-6 mb-4">
              <h3 className="font-semibold text-gray-900 mb-3">Explain API</h3>
              <code className="text-sm text-gray-800">
                GET /v1/session/&#123;session_id&#125;/pod/&#123;pod_id&#125;/explain
              </code>
              <pre className="bg-gray-900 rounded-lg p-4 mt-2 text-sm text-gray-300 overflow-x-auto">
{`{
  "pod_id": "pod_123",
  "break_type": "midroll",
  "slots_requested": 3,
  "slots_filled": 2,
  "decision_trace": [
    {"step": "partner_request", "partner": "partner_a", "duration_ms": 45},
    {"step": "format_filter", "rejected": 2, "reason": "codec_mismatch"},
    {"step": "competitive_separation", "rejected": 1, "reason": "same_category"},
    {"step": "frequency_cap", "rejected": 0},
    {"step": "final_selection", "selected": 2}
  ],
  "unfilled_reason": "no_compatible_inventory"
}`}
              </pre>
            </div>

            <h3 className="font-semibold text-gray-900 mb-3">Metrics Exposed</h3>
            <ul className="grid md:grid-cols-2 gap-2 text-gray-600">
              <li className="flex items-center gap-2">
                <CheckCircle className="w-4 h-4 text-emerald-600" />
                Session counts by tenant/channel
              </li>
              <li className="flex items-center gap-2">
                <CheckCircle className="w-4 h-4 text-emerald-600" />
                Pod decision latency (p50/p95/p99)
              </li>
              <li className="flex items-center gap-2">
                <CheckCircle className="w-4 h-4 text-emerald-600" />
                Fill rate by break type
              </li>
              <li className="flex items-center gap-2">
                <CheckCircle className="w-4 h-4 text-emerald-600" />
                Format rejection reasons
              </li>
              <li className="flex items-center gap-2">
                <CheckCircle className="w-4 h-4 text-emerald-600" />
                Partner response times
              </li>
              <li className="flex items-center gap-2">
                <CheckCircle className="w-4 h-4 text-emerald-600" />
                Tracking event success rate
              </li>
            </ul>
          </section>

          {/* Security */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Shield className="w-6 h-6 text-emerald-600" />
              Security & Privacy
            </h2>
            
            <div className="space-y-4">
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <Shield className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Signed Session Tokens</h3>
                  <p className="text-gray-600 text-sm">
                    All session manifests are protected by cryptographically signed tokens with TTL expiration. 
                    Tokens are validated on every manifest request.
                  </p>
                </div>
              </div>
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <Shield className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Anti-Tampering</h3>
                  <p className="text-gray-600 text-sm">
                    Session parameters (targeting, consent) cannot be modified after session creation. 
                    Any tampering attempt results in session invalidation.
                  </p>
                </div>
              </div>
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <Shield className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Rate Limiting</h3>
                  <p className="text-gray-600 text-sm">
                    Per-tenant, per-session, and per-IP rate limits prevent abuse and protect against 
                    session storms during player retry loops.
                  </p>
                </div>
              </div>
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <Shield className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Cache Isolation</h3>
                  <p className="text-gray-600 text-sm">
                    Session manifests use <code>private</code>/<code>no-store</code> cache headers to prevent 
                    cross-user manifest poisoning at CDN edges.
                  </p>
                </div>
              </div>
            </div>
          </section>

          {/* Platform Support */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <FileText className="w-6 h-6 text-emerald-600" />
              Platform Coverage Matrix
            </h2>

            {/* Legend */}
            <div className="flex flex-wrap gap-6 mb-4 text-sm">
              <div className="flex items-center gap-2">
                <span className="text-emerald-600">✓</span>
                <span className="text-gray-600">Certified</span>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-amber-600">△</span>
                <span className="text-gray-600">Limited (player-specific config required)</span>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-gray-400">—</span>
                <span className="text-gray-600">Not supported by platform</span>
              </div>
            </div>
            
            <div className="overflow-x-auto">
              <table className="w-full min-w-full text-sm border-collapse">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Platform</th>
                    <th className="text-center p-3 font-semibold text-gray-900">HLS/TS</th>
                    <th className="text-center p-3 font-semibold text-gray-900">HLS/CMAF</th>
                    <th className="text-center p-3 font-semibold text-gray-900">DASH/TS</th>
                    <th className="text-center p-3 font-semibold text-gray-900">DASH/CMAF</th>
                    <th className="text-center p-3 font-semibold text-gray-900">Live</th>
                    <th className="text-center p-3 font-semibold text-gray-900">VOD</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">Web (hls.js)</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Web (Shaka)</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-amber-600" title="Limited: TS with DASH requires player-specific configuration">△</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">tvOS (AVPlayer)</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Android TV (ExoPlayer)</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-amber-600" title="Limited: ExoPlayer TS with DASH requires explicit configuration">△</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Roku</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-amber-600" title="Limited: Roku DASH+TS requires specific firmware">△</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Fire TV</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-amber-600" title="Limited: Fire TV DASH+TS via ExoPlayer workaround">△</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                </tbody>
              </table>
            </div>

            {/* DASH/TS Note */}
            <div className="mt-4 p-4 bg-amber-50 border border-amber-200 rounded-lg flex items-start gap-3">
              <AlertTriangle className="w-5 h-5 text-amber-600 flex-shrink-0 mt-0.5" />
              <div>
                <p className="text-sm text-gray-700">
                  <strong>DASH + TS containers:</strong> While technically supported by some players (Shaka, ExoPlayer), 
                  DASH with TS segments requires player-specific configuration and is not recommended for new deployments. 
                  Use <strong>DASH + CMAF</strong> for full compatibility. See <Link href="/docs/ssai/containers" className="text-emerald-600 hover:underline">Container Formats</Link> for details.
                </p>
              </div>
            </div>
          </section>

          {/* Next Steps */}
          <section className="bg-emerald-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Next Steps</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/protocols/hls" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">HLS Protocol Details →</h3>
                <p className="text-sm text-gray-600">RFC 8216 compliance, DATERANGE markers, live sliding window</p>
              </Link>
              <Link href="/docs/ssai/protocols/dash" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">DASH Protocol Details →</h3>
                <p className="text-sm text-gray-600">MPD structure, EventStream, multi-period ad insertion</p>
              </Link>
              <Link href="/docs/getting-started/sandbox" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Try the Sandbox →</h3>
                <p className="text-sm text-gray-600">Test SSAI integration without credentials</p>
              </Link>
              <Link href="/contact" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Talk to an Expert →</h3>
                <p className="text-sm text-gray-600">Get help planning your SSAI integration</p>
              </Link>
            </div>
          </section>
        </div>
      </main>
      
      <Footer />
    </div>
  );
}
