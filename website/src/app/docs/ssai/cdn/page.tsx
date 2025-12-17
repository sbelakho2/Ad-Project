'use client';

import Link from 'next/link';
import { Globe, CheckCircle, XCircle, AlertTriangle, Shield, Layers } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function CDNGuidancePage() {
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
            <span className="text-gray-900">CDN Guidance</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              CDN Integration Guide
            </h1>
            <p className="text-xl text-gray-600">
              Cache policies, header requirements, and common misconfigurations to avoid 
              when integrating ApexMediation SSAI with your CDN.
            </p>
          </div>

          {/* Critical Warning */}
          <div className="bg-red-50 border border-red-200 rounded-lg p-6 mb-12">
            <h2 className="font-semibold text-red-800 mb-2 flex items-center gap-2">
              <AlertTriangle className="w-5 h-5" />
              Critical: Session Manifests Must Not Be Cached
            </h2>
            <p className="text-red-700 text-sm">
              SSAI session manifests (<code>/v1/session/&#123;id&#125;/manifest.m3u8</code>) are 
              per-viewer personalized. Caching them will serve wrong ads to viewers and break tracking.
              Configure your CDN to bypass cache (or use very low TTL) for session manifest paths.
            </p>
          </div>

          {/* Cache vs No-Cache */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Layers className="w-6 h-6 text-emerald-600" />
              What to Cache vs Never Cache
            </h2>

            <div className="grid md:grid-cols-2 gap-6">
              {/* Cacheable */}
              <div className="border border-emerald-200 rounded-lg">
                <div className="bg-emerald-50 px-4 py-3 border-b border-emerald-200">
                  <h3 className="font-semibold text-emerald-800 flex items-center gap-2">
                    <CheckCircle className="w-5 h-5" />
                    Safe to Cache
                  </h3>
                </div>
                <div className="p-4">
                  <ul className="space-y-3">
                    <li className="flex items-start gap-2">
                      <span className="text-emerald-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">/segments/*.ts</code>
                        <p className="text-sm text-gray-600 mt-1">Media segments are immutable. Cache 24h+</p>
                      </div>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-emerald-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">/segments/*.m4s</code>
                        <p className="text-sm text-gray-600 mt-1">DASH segments. Cache 24h+</p>
                      </div>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-emerald-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">/init/*.mp4</code>
                        <p className="text-sm text-gray-600 mt-1">Init segments. Cache 7d</p>
                      </div>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-emerald-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">/slate/*</code>
                        <p className="text-sm text-gray-600 mt-1">Slate assets. Cache 7d</p>
                      </div>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-emerald-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">/ads/&#123;creative_id&#125;/*</code>
                        <p className="text-sm text-gray-600 mt-1">Transcoded creatives. Cache 1h (with stale-while-revalidate)</p>
                      </div>
                    </li>
                  </ul>
                </div>
              </div>

              {/* Never Cache */}
              <div className="border border-red-200 rounded-lg">
                <div className="bg-red-50 px-4 py-3 border-b border-red-200">
                  <h3 className="font-semibold text-red-800 flex items-center gap-2">
                    <XCircle className="w-5 h-5" />
                    Never Cache
                  </h3>
                </div>
                <div className="p-4">
                  <ul className="space-y-3">
                    <li className="flex items-start gap-2">
                      <span className="text-red-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">/v1/session/*</code>
                        <p className="text-sm text-gray-600 mt-1">Session manifests. Personalized per viewer.</p>
                      </div>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-red-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">/v1/session/init</code>
                        <p className="text-sm text-gray-600 mt-1">Session creation endpoint. Unique per request.</p>
                      </div>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-red-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">/tracking/*</code>
                        <p className="text-sm text-gray-600 mt-1">Tracking pixels. Must hit origin.</p>
                      </div>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-red-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">/live/*.m3u8</code>
                        <p className="text-sm text-gray-600 mt-1">Live manifests. Updates every segment duration.</p>
                      </div>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-red-600 mt-1">●</span>
                      <div>
                        <code className="text-sm bg-gray-100 px-1 rounded">*?_sid=*</code>
                        <p className="text-sm text-gray-600 mt-1">Any URL with session ID param. Personalized.</p>
                      </div>
                    </li>
                  </ul>
                </div>
              </div>
            </div>
          </section>

          {/* Recommended Headers */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Shield className="w-6 h-6 text-emerald-600" />
              Required Response Headers
            </h2>

            <p className="text-gray-600 mb-4">
              ApexMediation returns these headers. Your CDN must respect them:
            </p>

            <div className="bg-gray-50 rounded-lg p-6 overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-gray-300">
                    <th className="text-left py-2 font-semibold text-gray-900">Resource Type</th>
                    <th className="text-left py-2 font-semibold text-gray-900">Cache-Control Header</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="py-3 text-gray-900">Session manifests</td>
                    <td className="py-3"><code className="text-xs bg-gray-200 px-1 rounded">no-store, no-cache, must-revalidate</code></td>
                  </tr>
                  <tr>
                    <td className="py-3 text-gray-900">Live manifests</td>
                    <td className="py-3"><code className="text-xs bg-gray-200 px-1 rounded">max-age=1, stale-while-revalidate=2</code></td>
                  </tr>
                  <tr>
                    <td className="py-3 text-gray-900">VOD manifests</td>
                    <td className="py-3"><code className="text-xs bg-gray-200 px-1 rounded">max-age=300, stale-while-revalidate=60</code></td>
                  </tr>
                  <tr>
                    <td className="py-3 text-gray-900">Media segments</td>
                    <td className="py-3"><code className="text-xs bg-gray-200 px-1 rounded">max-age=86400, immutable</code></td>
                  </tr>
                  <tr>
                    <td className="py-3 text-gray-900">Init segments</td>
                    <td className="py-3"><code className="text-xs bg-gray-200 px-1 rounded">max-age=604800, immutable</code></td>
                  </tr>
                  <tr>
                    <td className="py-3 text-gray-900">Tracking endpoints</td>
                    <td className="py-3"><code className="text-xs bg-gray-200 px-1 rounded">no-store</code></td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div className="mt-4 bg-blue-50 border border-blue-200 rounded-lg p-4">
              <h3 className="font-semibold text-gray-900 text-sm mb-2">CORS Headers</h3>
              <p className="text-sm text-gray-600">
                For cross-origin playback, ensure your CDN forwards our CORS headers:
              </p>
              <pre className="mt-2 text-xs bg-gray-800 text-gray-100 p-3 rounded overflow-x-auto">
{`Access-Control-Allow-Origin: *
Access-Control-Allow-Methods: GET, OPTIONS
Access-Control-Allow-Headers: Range
Access-Control-Expose-Headers: Content-Range, Content-Length`}
              </pre>
            </div>
          </section>

          {/* CDN-Specific Config */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Globe className="w-6 h-6 text-emerald-600" />
              CDN-Specific Configuration
            </h2>

            <div className="space-y-6">
              {/* CloudFront */}
              <div className="border rounded-lg overflow-hidden">
                <div className="bg-gray-50 px-4 py-3 border-b">
                  <h3 className="font-semibold text-gray-900">AWS CloudFront</h3>
                </div>
                <div className="p-4">
                  <pre className="text-xs bg-gray-800 text-gray-100 p-4 rounded overflow-x-auto">
{`# Cache behavior for session manifests
CacheBehavior:
  PathPattern: "/v1/session/*"
  CachePolicyId: "4135ea2d-6df8-44a3-9df3-4b5a84be39ad"  # CachingDisabled
  OriginRequestPolicyId: "216adef6-5c7f-47e4-b989-5492eafa07d3"  # AllViewer

# Cache behavior for segments  
CacheBehavior:
  PathPattern: "/segments/*"
  CachePolicyId: "658327ea-f89d-4fab-a63d-7e88639e58f6"  # CachingOptimized
  Compress: true`}
                  </pre>
                </div>
              </div>

              {/* Cloudflare */}
              <div className="border rounded-lg overflow-hidden">
                <div className="bg-gray-50 px-4 py-3 border-b">
                  <h3 className="font-semibold text-gray-900">Cloudflare</h3>
                </div>
                <div className="p-4">
                  <pre className="text-xs bg-gray-800 text-gray-100 p-4 rounded overflow-x-auto">
{`# Page Rules for session endpoints
Match: *example.com/v1/session/*
Settings:
  Cache Level: Bypass
  Browser Cache TTL: Respect Existing Headers

# Cache Rules for segments
Match: *example.com/segments/*
Settings:
  Cache Level: Cache Everything
  Edge Cache TTL: 1 day
  Browser Cache TTL: 1 day`}
                  </pre>
                </div>
              </div>

              {/* Fastly */}
              <div className="border rounded-lg overflow-hidden">
                <div className="bg-gray-50 px-4 py-3 border-b">
                  <h3 className="font-semibold text-gray-900">Fastly</h3>
                </div>
                <div className="p-4">
                  <pre className="text-xs bg-gray-800 text-gray-100 p-4 rounded overflow-x-auto">
{`# VCL for session bypass
sub vcl_recv {
  if (req.url ~ "^/v1/session/") {
    return(pass);
  }
}

# Segment caching
sub vcl_fetch {
  if (req.url ~ "\\.(ts|m4s|mp4)$") {
    set beresp.ttl = 24h;
    set beresp.stale_if_error = 1d;
  }
}`}
                  </pre>
                </div>
              </div>
            </div>
          </section>

          {/* Common Misconfigurations */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <AlertTriangle className="w-6 h-6 text-amber-600" />
              Common Misconfigurations
            </h2>

            <div className="space-y-4">
              <div className="border border-red-200 rounded-lg p-4 bg-red-50">
                <h3 className="font-semibold text-red-800 mb-2 flex items-center gap-2">
                  <XCircle className="w-5 h-5" />
                  ❌ Caching based on file extension only
                </h3>
                <p className="text-sm text-red-700 mb-2">
                  <strong>Problem:</strong> Caching all <code>.m3u8</code> files ignores that session manifests are personalized.
                </p>
                <p className="text-sm text-gray-600">
                  <strong>Fix:</strong> Cache by path pattern, not extension. <code>/ssai/session/*.m3u8</code> → bypass, 
                  <code>/vod/*.m3u8</code> → cache.
                </p>
              </div>

              <div className="border border-red-200 rounded-lg p-4 bg-red-50">
                <h3 className="font-semibold text-red-800 mb-2 flex items-center gap-2">
                  <XCircle className="w-5 h-5" />
                  ❌ Stripping query parameters
                </h3>
                <p className="text-sm text-red-700 mb-2">
                  <strong>Problem:</strong> CDN removes <code>?_sid=...</code> for cache normalization. Session ID lost.
                </p>
                <p className="text-sm text-gray-600">
                  <strong>Fix:</strong> Preserve query parameters for <code>/ssai/*</code> paths. Only normalize segments.
                </p>
              </div>

              <div className="border border-red-200 rounded-lg p-4 bg-red-50">
                <h3 className="font-semibold text-red-800 mb-2 flex items-center gap-2">
                  <XCircle className="w-5 h-5" />
                  ❌ Ignoring Vary headers
                </h3>
                <p className="text-sm text-red-700 mb-2">
                  <strong>Problem:</strong> CDN caches a single variant, ignoring <code>Vary: Accept-Encoding</code>.
                </p>
                <p className="text-sm text-gray-600">
                  <strong>Fix:</strong> Enable Vary header support and cache per-encoding (gzip, br, identity).
                </p>
              </div>

              <div className="border border-red-200 rounded-lg p-4 bg-red-50">
                <h3 className="font-semibold text-red-800 mb-2 flex items-center gap-2">
                  <XCircle className="w-5 h-5" />
                  ❌ Over-aggressive connection coalescing
                </h3>
                <p className="text-sm text-red-700 mb-2">
                  <strong>Problem:</strong> HTTP/2 connection reuse serves wrong session&apos;s segments.
                </p>
                <p className="text-sm text-gray-600">
                  <strong>Fix:</strong> Disable connection coalescing for <code>/ssai/</code> paths or use separate hostnames.
                </p>
              </div>

              <div className="border border-amber-200 rounded-lg p-4 bg-amber-50">
                <h3 className="font-semibold text-amber-800 mb-2 flex items-center gap-2">
                  <AlertTriangle className="w-5 h-5" />
                  ⚠️ Short TTL on segments
                </h3>
                <p className="text-sm text-amber-700 mb-2">
                  <strong>Problem:</strong> 5-minute TTL on segments causes unnecessary origin load.
                </p>
                <p className="text-sm text-gray-600">
                  <strong>Fix:</strong> Segments are immutable once created. Set 24h+ TTL.
                </p>
              </div>
            </div>
          </section>

          {/* Verification Checklist */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <CheckCircle className="w-6 h-6 text-emerald-600" />
              Verification Checklist
            </h2>

            <p className="text-gray-600 mb-4">
              After CDN configuration, verify with these tests:
            </p>

            <div className="bg-gray-50 rounded-lg p-6">
              <ol className="space-y-4">
                <li className="flex items-start gap-3">
                  <span className="w-6 h-6 bg-emerald-100 rounded-full flex items-center justify-center text-emerald-700 font-semibold text-sm flex-shrink-0">1</span>
                  <div>
                    <h3 className="font-semibold text-gray-900">Session manifest bypass</h3>
                    <code className="text-xs bg-gray-200 px-2 py-1 rounded block mt-1">
                      curl -I https://cdn.example.com/ssai/session/test/manifest.m3u8
                    </code>
                    <p className="text-sm text-gray-600 mt-1">Should show <code>X-Cache: MISS</code> or no cache headers.</p>
                  </div>
                </li>
                <li className="flex items-start gap-3">
                  <span className="w-6 h-6 bg-emerald-100 rounded-full flex items-center justify-center text-emerald-700 font-semibold text-sm flex-shrink-0">2</span>
                  <div>
                    <h3 className="font-semibold text-gray-900">Segment caching</h3>
                    <code className="text-xs bg-gray-200 px-2 py-1 rounded block mt-1">
                      curl -I https://cdn.example.com/segments/1234.ts # twice
                    </code>
                    <p className="text-sm text-gray-600 mt-1">Second request should show <code>X-Cache: HIT</code>.</p>
                  </div>
                </li>
                <li className="flex items-start gap-3">
                  <span className="w-6 h-6 bg-emerald-100 rounded-full flex items-center justify-center text-emerald-700 font-semibold text-sm flex-shrink-0">3</span>
                  <div>
                    <h3 className="font-semibold text-gray-900">Tracking bypass</h3>
                    <code className="text-xs bg-gray-200 px-2 py-1 rounded block mt-1">
                      curl -I https://cdn.example.com/tracking/impression/xyz
                    </code>
                    <p className="text-sm text-gray-600 mt-1">Every request must reach origin (no cache).</p>
                  </div>
                </li>
                <li className="flex items-start gap-3">
                  <span className="w-6 h-6 bg-emerald-100 rounded-full flex items-center justify-center text-emerald-700 font-semibold text-sm flex-shrink-0">4</span>
                  <div>
                    <h3 className="font-semibold text-gray-900">Cross-origin playback</h3>
                    <p className="text-sm text-gray-600">Test from a different origin domain, verify CORS headers present.</p>
                  </div>
                </li>
                <li className="flex items-start gap-3">
                  <span className="w-6 h-6 bg-emerald-100 rounded-full flex items-center justify-center text-emerald-700 font-semibold text-sm flex-shrink-0">5</span>
                  <div>
                    <h3 className="font-semibold text-gray-900">Load test with unique sessions</h3>
                    <p className="text-sm text-gray-600">
                      Simulate 100+ viewers with unique session IDs. Each should see personalized ads.
                    </p>
                  </div>
                </li>
              </ol>
            </div>
          </section>

          {/* Related Documentation */}
          <section className="bg-emerald-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/protocols/hls" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">HLS Details →</h3>
                <p className="text-sm text-gray-600">Manifest structure and segment naming</p>
              </Link>
              <Link href="/docs/ssai/protocols/dash" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">DASH Details →</h3>
                <p className="text-sm text-gray-600">MPD structure and period handling</p>
              </Link>
              <Link href="/docs/ssai/operations" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Operations →</h3>
                <p className="text-sm text-gray-600">Failure modes and fallback behavior</p>
              </Link>
              <Link href="/docs/ssai/prerequisites" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Prerequisites →</h3>
                <p className="text-sm text-gray-600">Origin and encoder requirements</p>
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
