'use client';

import Link from 'next/link';
import { CheckCircle, AlertTriangle, Box, FileCode, Layers, Zap } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function ContainersPage() {
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
            <span className="text-gray-900">Container Formats</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              Container Formats: TS &amp; CMAF
            </h1>
            <p className="text-xl text-gray-600">
              ApexMediation SSAI supports both MPEG Transport Stream (TS) and 
              Common Media Application Format (CMAF/fMP4) containers for maximum 
              device compatibility.
            </p>
          </div>

          {/* Overview */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Box className="w-6 h-6 text-emerald-600" />
              Container Comparison
            </h2>
            
            <p className="text-gray-600 mb-4">
              The <code>ContainerHandler</code> manages both formats with correct MIME types 
              and file conventions per HLS/DASH specifications.
            </p>

            <div className="overflow-x-auto mb-6">
              <table className="w-full min-w-full text-sm border-collapse">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Aspect</th>
                    <th className="text-left p-3 font-semibold text-gray-900">TS (.ts)</th>
                    <th className="text-left p-3 font-semibold text-gray-900">CMAF (.m4s)</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">Full Name</td>
                    <td className="p-3 text-gray-600">MPEG Transport Stream</td>
                    <td className="p-3 text-gray-600">Common Media Application Format</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Extensions</td>
                    <td className="p-3 text-gray-600"><code>.ts</code>, <code>.m2ts</code>, <code>.mts</code></td>
                    <td className="p-3 text-gray-600"><code>.m4s</code>, <code>.mp4</code>, <code>.m4a</code>, <code>.m4v</code></td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Video MIME</td>
                    <td className="p-3 text-gray-600"><code>video/MP2T</code></td>
                    <td className="p-3 text-gray-600"><code>video/mp4</code></td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Audio MIME</td>
                    <td className="p-3 text-gray-600"><code>audio/MP2T</code></td>
                    <td className="p-3 text-gray-600"><code>audio/mp4</code></td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Init Segment</td>
                    <td className="p-3 text-gray-600">Not required (self-contained)</td>
                    <td className="p-3 text-gray-600">Required (separate .mp4/.m4s)</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">HLS Support</td>
                    <td className="p-3 text-emerald-600">✓ Full</td>
                    <td className="p-3 text-emerald-600">✓ Full (HLS v6+)</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">DASH Support</td>
                    <td className="p-3 text-amber-600">△ Limited</td>
                    <td className="p-3 text-emerald-600">✓ Full</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          {/* TS Format */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <FileCode className="w-6 h-6 text-emerald-600" />
              MPEG Transport Stream (TS)
            </h2>
            
            <p className="text-gray-600 mb-4">
              Transport Stream is the legacy container format, widely supported across 
              all devices. Each segment is self-contained with headers.
            </p>

            <div className="grid md:grid-cols-2 gap-6 mb-6">
              <div className="bg-emerald-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Advantages</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Universal device compatibility</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Self-contained segments (no init needed)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Simple error recovery (each segment independent)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Legacy encoder support</span>
                  </li>
                </ul>
              </div>
              
              <div className="bg-amber-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Disadvantages</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-600 flex-shrink-0 mt-0.5" />
                    <span>~15% overhead per segment (repeated headers)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-600 flex-shrink-0 mt-0.5" />
                    <span>Limited DASH support (requires player-specific config)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-600 flex-shrink-0 mt-0.5" />
                    <span>No native browser support (requires hls.js)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-600 flex-shrink-0 mt-0.5" />
                    <span>Limited DRM options</span>
                  </li>
                </ul>
              </div>
            </div>

            <div className="bg-gray-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">TS Detection</h3>
              <p className="text-sm text-gray-600 mb-2">
                From <code>ContainerHandler.detectContainerType()</code>:
              </p>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`// Extensions recognized as TS
private static readonly TS_EXTENSIONS = ['.ts', '.m2ts', '.mts'];

// MIME types
private static readonly MIME_TYPES = {
  ts: {
    video: 'video/MP2T',
    audio: 'audio/MP2T',
  },
  // ...
};`}
              </pre>
            </div>
          </section>

          {/* CMAF Format */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Layers className="w-6 h-6 text-emerald-600" />
              Common Media Application Format (CMAF)
            </h2>
            
            <p className="text-gray-600 mb-4">
              CMAF (also known as fMP4 - fragmented MP4) is the modern container format 
              designed for efficient streaming. It&apos;s the preferred format for new 
              deployments.
            </p>

            <div className="grid md:grid-cols-2 gap-6 mb-6">
              <div className="bg-emerald-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Advantages</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Lower overhead (shared init segment)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Full HLS + DASH support</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Native browser support (MSE)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Full DRM support (Widevine, FairPlay, PlayReady)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Low-latency streaming capable</span>
                  </li>
                </ul>
              </div>
              
              <div className="bg-amber-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Considerations</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-600 flex-shrink-0 mt-0.5" />
                    <span>Init segment required before media segments</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-600 flex-shrink-0 mt-0.5" />
                    <span>Init segment must match at discontinuities</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-600 flex-shrink-0 mt-0.5" />
                    <span>Older devices may lack support</span>
                  </li>
                </ul>
              </div>
            </div>

            <div className="bg-gray-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">CMAF Detection &amp; Init Segments</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`// Extensions recognized as CMAF
private static readonly CMAF_EXTENSIONS = ['.m4s', '.mp4', '.m4a', '.m4v'];

// MIME types
private static readonly MIME_TYPES = {
  cmaf: {
    video: 'video/mp4',
    audio: 'audio/mp4',
    init: 'video/mp4',
  },
  // ...
};

// HLS playlist with init segment
#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.000,
segment1.m4s
#EXTINF:6.000,
segment2.m4s`}
              </pre>
            </div>
          </section>

          {/* SSAI Considerations */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Zap className="w-6 h-6 text-emerald-600" />
              SSAI Container Requirements
            </h2>
            
            <p className="text-gray-600 mb-4">
              When stitching ads into content, the container format must match between 
              content and ads to ensure seamless playback.
            </p>

            <div className="space-y-4">
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Format Matching</h3>
                <p className="text-gray-600 text-sm">
                  HLS/TS content requires HLS/TS ads. DASH/CMAF content requires DASH/CMAF ads. 
                  The pod decisioning engine filters ads by container format compatibility.
                </p>
              </div>
              
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Init Segment Handling (CMAF)</h3>
                <p className="text-gray-600 text-sm">
                  At content-to-ad transitions, the SSAI stitcher inserts the ad init segment 
                  via <code>#EXT-X-MAP</code> or DASH <code>Initialization</code>. At ad-to-content 
                  transitions, the content init segment is restored.
                </p>
              </div>
              
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Codec Consistency</h3>
                <p className="text-gray-600 text-sm">
                  Beyond container format, codecs must be compatible. The system validates 
                  codec strings (H.264: <code>avc1.*</code>, H.265: <code>hvc1.*</code>, 
                  AAC: <code>mp4a.40.*</code>) between content and ads.
                </p>
              </div>
            </div>

            <div className="bg-gray-50 rounded-lg p-6 mt-6">
              <h3 className="font-semibold text-gray-900 mb-3">Codec Patterns</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`// From ContainerHandler
private static readonly CODEC_PATTERNS = {
  h264: /avc[1-4]\\.[0-9A-Fa-f]+/,      // H.264/AVC
  h265: /hvc1\\.[0-9A-Za-z.]+|hev1\\.[0-9A-Za-z.]+/, // H.265/HEVC
  aac: /mp4a\\.40\\.[0-9]+/,             // AAC
  ac3: /ac-3/,                          // Dolby Digital
  eac3: /ec-3/,                         // Dolby Digital Plus
};`}
              </pre>
            </div>
          </section>

          {/* Protocol/Container Matrix */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">Protocol/Container Matrix</h2>
            
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Protocol</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Container</th>
                    <th className="text-center p-3 font-semibold text-gray-900">Live</th>
                    <th className="text-center p-3 font-semibold text-gray-900">VOD</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Players</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">HLS</td>
                    <td className="p-3 text-gray-600">TS</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-gray-600">AVPlayer, hls.js, ExoPlayer</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">HLS</td>
                    <td className="p-3 text-gray-600">CMAF</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-gray-600">AVPlayer, hls.js, ExoPlayer, Shaka</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">DASH</td>
                    <td className="p-3 text-gray-600">TS</td>
                    <td className="p-3 text-center text-amber-600">△</td>
                    <td className="p-3 text-center text-amber-600">△</td>
                    <td className="p-3 text-gray-600">Limited ExoPlayer support</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">DASH</td>
                    <td className="p-3 text-gray-600">CMAF</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-gray-600">Shaka, dash.js, ExoPlayer</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          {/* Recommendations */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">Recommendations</h2>
            
            <div className="space-y-4">
              <div className="bg-emerald-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2">For New Deployments</h3>
                <p className="text-sm text-gray-600">
                  Use <strong>CMAF/fMP4</strong> as your primary container. It provides the best 
                  efficiency, DRM support, and works with both HLS and DASH protocols.
                </p>
              </div>
              
              <div className="bg-blue-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2">For Maximum Compatibility</h3>
                <p className="text-sm text-gray-600">
                  Offer both <strong>HLS/TS</strong> and <strong>HLS/CMAF</strong>. Let the SDK 
                  select based on device capabilities. This covers legacy and modern devices.
                </p>
              </div>
              
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-2">For CTV Focus</h3>
                <p className="text-sm text-gray-600">
                  CTV devices (Apple TV, Roku, Fire TV, Android TV) all support CMAF. 
                  Use <strong>HLS/CMAF</strong> for Apple TV (AVPlayer) and <strong>DASH/CMAF</strong> 
                  for Android TV (ExoPlayer).
                </p>
              </div>
            </div>
          </section>

          {/* Next Steps */}
          <section className="bg-emerald-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/drm" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">DRM Support →</h3>
                <p className="text-sm text-gray-600">Widevine, FairPlay, PlayReady integration</p>
              </Link>
              <Link href="/docs/ssai/protocols/hls" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">HLS Protocol →</h3>
                <p className="text-sm text-gray-600">RFC 8216 compliance, DATERANGE</p>
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
