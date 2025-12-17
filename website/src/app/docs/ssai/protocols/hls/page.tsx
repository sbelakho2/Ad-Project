'use client';

import Link from 'next/link';
import { CheckCircle, AlertTriangle, FileText, Clock, Shield, Zap, Code } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function HLSProtocolPage() {
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
            <span className="text-gray-900">HLS Protocol</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              HLS Protocol Support
            </h1>
            <p className="text-xl text-gray-600">
              RFC 8216 compliant HLS manifest generation with proper live sliding window behavior, 
              discontinuity handling, and CDN-safe caching for SSAI.
            </p>
          </div>

          {/* RFC 8216 Compliance */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <FileText className="w-6 h-6 text-emerald-600" />
              RFC 8216 Compliance
            </h2>
            
            <p className="text-gray-600 mb-4">
              ApexMediation generates HLS playlists that fully comply with RFC 8216 (HTTP Live Streaming). 
              Every aspect of the specification is validated during manifest generation.
            </p>

            <div className="bg-gray-50 rounded-lg p-6 mb-6">
              <h3 className="font-semibold text-gray-900 mb-4">Master Playlist Structure</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`#EXTM3U
#EXT-X-VERSION:6
#EXT-X-INDEPENDENT-SEGMENTS

#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aac",NAME="English",DEFAULT=YES,AUTOSELECT=YES,LANGUAGE="en",URI="audio/en/playlist.m3u8"

#EXT-X-STREAM-INF:BANDWIDTH=2800000,CODECS="avc1.64001f,mp4a.40.2",RESOLUTION=1280x720,AUDIO="aac"
video/720p/playlist.m3u8

#EXT-X-STREAM-INF:BANDWIDTH=5000000,CODECS="avc1.640028,mp4a.40.2",RESOLUTION=1920x1080,AUDIO="aac"
video/1080p/playlist.m3u8`}
              </pre>
            </div>

            <h3 className="font-semibold text-gray-900 mb-3">Compliance Checkpoints</h3>
            <div className="space-y-3">
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">#EXTM3U Required</span>
                  <p className="text-sm text-gray-600">Every playlist starts with the required #EXTM3U tag</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">EXT-X-VERSION Consistency</span>
                  <p className="text-sm text-gray-600">Version tag matches features used (v6+ for EXT-X-DATERANGE)</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">EXTINF Precision</span>
                  <p className="text-sm text-gray-600">Every segment has nonzero duration with correct precision</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <span className="font-medium text-gray-900">TARGETDURATION Accuracy</span>
                  <p className="text-sm text-gray-600">EXT-X-TARGETDURATION is ceiling of longest segment (never exceeded)</p>
                </div>
              </div>
            </div>
          </section>

          {/* Live Behavior */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Clock className="w-6 h-6 text-emerald-600" />
              Live Sliding Window Behavior
            </h2>
            
            <p className="text-gray-600 mb-4">
              Live HLS playlists use a sliding window that maintains a fixed duration of content. 
              As new segments are added, old segments are removed. The SSAI system handles this 
              correctly even with ad breaks in the window.
            </p>

            <div className="grid md:grid-cols-2 gap-6 mb-6">
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">✅ Correct Behavior</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span><code>EXT-X-MEDIA-SEQUENCE</code> is monotonically increasing</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Sequence increments by exactly 1 per removed segment</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>No <code>EXT-X-PLAYLIST-TYPE</code> tag in live playlists</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-600 flex-shrink-0 mt-0.5" />
                    <span>Window doesn&apos;t shrink too aggressively (prevents drift)</span>
                  </li>
                </ul>
              </div>
              
              <div className="bg-red-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">❌ Avoided Problems</h3>
                <ul className="space-y-2 text-sm text-gray-600">
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-red-600 flex-shrink-0 mt-0.5" />
                    <span>Media sequence jumping or decreasing (causes player errors)</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-red-600 flex-shrink-0 mt-0.5" />
                    <span>Adding <code>EVENT</code>/<code>VOD</code> type to live streams</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-red-600 flex-shrink-0 mt-0.5" />
                    <span>Discontinuity drift from aggressive window shrinking</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-red-600 flex-shrink-0 mt-0.5" />
                    <span>Segments falling out mid-ad break</span>
                  </li>
                </ul>
              </div>
            </div>

            <div className="bg-gray-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">Live Media Playlist Example</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`#EXTM3U
#EXT-X-VERSION:6
#EXT-X-TARGETDURATION:6
#EXT-X-MEDIA-SEQUENCE:2847
#EXT-X-DISCONTINUITY-SEQUENCE:12
#EXT-X-PROGRAM-DATE-TIME:2025-01-15T10:30:24.000Z

#EXTINF:6.000,
https://cdn.example.com/content/seg2847.ts
#EXTINF:6.000,
https://cdn.example.com/content/seg2848.ts
#EXTINF:6.000,
https://cdn.example.com/content/seg2849.ts

#EXT-X-DATERANGE:ID="ad-pod-1",CLASS="com.apple.hls.interstitial",START-DATE="2025-01-15T10:30:42.000Z",DURATION=30
#EXT-X-DISCONTINUITY

#EXTINF:6.000,
https://ssai.apexmediation.ee/session/abc123/ad/slot1-seg1.ts
#EXTINF:6.000,
https://ssai.apexmediation.ee/session/abc123/ad/slot1-seg2.ts
#EXTINF:6.000,
https://ssai.apexmediation.ee/session/abc123/ad/slot1-seg3.ts
#EXTINF:6.000,
https://ssai.apexmediation.ee/session/abc123/ad/slot1-seg4.ts
#EXTINF:6.000,
https://ssai.apexmediation.ee/session/abc123/ad/slot1-seg5.ts

#EXT-X-DISCONTINUITY

#EXTINF:6.000,
https://cdn.example.com/content/seg2850.ts
#EXTINF:6.000,
https://cdn.example.com/content/seg2851.ts`}
              </pre>
            </div>
          </section>

          {/* EXT-X-DATERANGE */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Code className="w-6 h-6 text-emerald-600" />
              EXT-X-DATERANGE for Ad Markers
            </h2>
            
            <p className="text-gray-600 mb-4">
              Ad breaks are signaled using <code>EXT-X-DATERANGE</code> tags per the Apple HLS 
              interstitials specification. This allows players to identify ad content and provide 
              appropriate UI treatment.
            </p>

            <div className="bg-blue-50 rounded-lg p-6 mb-6">
              <h3 className="font-semibold text-gray-900 mb-3">DATERANGE Attributes</h3>
              <div className="overflow-x-auto">
                <table className="w-full min-w-full text-sm border-collapse">
                  <thead>
                    <tr className="border-b border-blue-200">
                      <th className="text-left py-2 pr-4 font-semibold text-gray-900">Attribute</th>
                      <th className="text-left py-2 pr-4 font-semibold text-gray-900">Description</th>
                      <th className="text-left py-2 font-semibold text-gray-900">Example</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-blue-100">
                    <tr>
                      <td className="py-2 pr-4 text-gray-700"><code>ID</code></td>
                      <td className="py-2 pr-4 text-gray-600">Unique pod identifier</td>
                      <td className="py-2 text-gray-600"><code>&quot;ad-pod-1&quot;</code></td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700"><code>CLASS</code></td>
                      <td className="py-2 pr-4 text-gray-600">Apple interstitial class</td>
                      <td className="py-2 text-gray-600"><code>&quot;com.apple.hls.interstitial&quot;</code></td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700"><code>START-DATE</code></td>
                      <td className="py-2 pr-4 text-gray-600">ISO 8601 start time</td>
                      <td className="py-2 text-gray-600"><code>&quot;2025-01-15T10:30:42.000Z&quot;</code></td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700"><code>DURATION</code></td>
                      <td className="py-2 pr-4 text-gray-600">Pod duration in seconds</td>
                      <td className="py-2 text-gray-600"><code>30</code></td>
                    </tr>
                    <tr>
                      <td className="py-2 pr-4 text-gray-700"><code>X-SLOT-COUNT</code></td>
                      <td className="py-2 pr-4 text-gray-600">Number of ads in pod</td>
                      <td className="py-2 text-gray-600"><code>2</code></td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>

            <h3 className="font-semibold text-gray-900 mb-3">Player Detection</h3>
            <p className="text-gray-600 mb-4">
              The ApexMediation SDK parses DATERANGE tags to identify ad boundaries:
            </p>
            <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto mb-4">
{`// Web SDK (TypeScript)
import { SSAIManifestParser } from '@apexmediation/web-sdk';

const parser = new SSAIManifestParser();
const adBreaks = parser.parseHLSManifest(manifestContent);

adBreaks.forEach(adBreak => {
  console.log('Ad break:', adBreak.id);
  console.log('  Start:', adBreak.startTime);
  console.log('  Duration:', adBreak.duration);
  console.log('  Slots:', adBreak.slotCount);
});`}
            </pre>
          </section>

          {/* Discontinuity */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Zap className="w-6 h-6 text-emerald-600" />
              Discontinuity Handling
            </h2>
            
            <p className="text-gray-600 mb-4">
              Ad content typically has different encoding parameters than program content. The SSAI 
              system correctly handles discontinuities to ensure seamless playback.
            </p>

            <div className="space-y-4">
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">EXT-X-DISCONTINUITY</h3>
                <p className="text-gray-600 text-sm">
                  Inserted at content-to-ad and ad-to-content transitions when encoding parameters 
                  (codec, timescale, resolution) differ.
                </p>
              </div>
              
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">EXT-X-DISCONTINUITY-SEQUENCE</h3>
                <p className="text-gray-600 text-sm">
                  Maintained monotonically across manifest updates. Increments correctly when 
                  discontinuity tags slide out of the live window.
                </p>
              </div>
              
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Cross-Rendition Consistency</h3>
                <p className="text-gray-600 text-sm">
                  All renditions have matching discontinuity counts at equivalent playback positions 
                  to prevent ABR switching issues.
                </p>
              </div>
            </div>

            <div className="bg-amber-50 rounded-lg p-6 mt-6">
              <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                <AlertTriangle className="w-5 h-5 text-amber-600" />
                Common Integration Issues
              </h3>
              <ul className="space-y-2 text-sm text-gray-600">
                <li><strong>hls.js warnings:</strong> Usually caused by discontinuity sequence mismatch or aggressive window shrinking. The SSAI system avoids these patterns.</li>
                <li><strong>Player &quot;reset&quot; at ad boundaries:</strong> Often caused by missing discontinuity tags. All ad transitions include proper markers.</li>
                <li><strong>ABR issues during ads:</strong> Caused by inconsistent discontinuity counts across renditions. Our system ensures cross-rendition consistency.</li>
              </ul>
            </div>
          </section>

          {/* CDN Caching */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Shield className="w-6 h-6 text-emerald-600" />
              CDN Caching Strategy
            </h2>
            
            <p className="text-gray-600 mb-4">
              Session manifests contain personalized ad content and must not be cached at CDN edges. 
              Segments, however, are cacheable.
            </p>

            <div className="grid md:grid-cols-2 gap-6">
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Session Manifests</h3>
                <pre className="bg-gray-900 rounded-lg p-3 text-xs text-gray-300 overflow-x-auto mb-3">
{`Cache-Control: private, no-store, max-age=0
Vary: Accept-Encoding, Cookie`}
                </pre>
                <ul className="space-y-1 text-sm text-gray-600">
                  <li>• <code>private</code>: Not cached by shared caches</li>
                  <li>• <code>no-store</code>: Never cached</li>
                  <li>• Tokenized URLs for additional protection</li>
                </ul>
              </div>
              
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Segment Files</h3>
                <pre className="bg-gray-900 rounded-lg p-3 text-xs text-gray-300 overflow-x-auto mb-3">
{`Cache-Control: public, max-age=31536000, immutable`}
                </pre>
                <ul className="space-y-1 text-sm text-gray-600">
                  <li>• <code>public</code>: Cacheable by all caches</li>
                  <li>• Long TTL with immutable content</li>
                  <li>• Unique URLs per segment version</li>
                </ul>
              </div>
            </div>

            <div className="bg-red-50 rounded-lg p-6 mt-6">
              <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                <AlertTriangle className="w-5 h-5 text-red-600" />
                Cache Poisoning Prevention
              </h3>
              <p className="text-sm text-gray-600">
                Session manifests are protected from cache poisoning attacks. Even if a CDN were 
                misconfigured, the signed session token in the URL helps reduce the risk of cross-user 
                cache poisoning. You should still configure your CDN to bypass caching for session manifests.
              </p>
            </div>
          </section>

          {/* VOD Requirements */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <FileText className="w-6 h-6 text-emerald-600" />
              VOD-Specific Requirements
            </h2>
            
            <div className="space-y-4">
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <CheckCircle className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">EXT-X-ENDLIST Required</h3>
                  <p className="text-gray-600 text-sm">
                    VOD playlists always end with <code>#EXT-X-ENDLIST</code> to signal finite content. 
                    This is critical for player seek behavior and progress UI.
                  </p>
                </div>
              </div>
              
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <CheckCircle className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">EXT-X-PLAYLIST-TYPE:VOD</h3>
                  <p className="text-gray-600 text-sm">
                    VOD playlists include the <code>EXT-X-PLAYLIST-TYPE:VOD</code> tag to indicate 
                    immutable content (no segments will be added or removed).
                  </p>
                </div>
              </div>
              
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <CheckCircle className="w-6 h-6 text-emerald-600 flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Complete Timeline</h3>
                  <p className="text-gray-600 text-sm">
                    VOD manifests include all content and ad segments from the start. Ads are 
                    pre-stitched at session creation time.
                  </p>
                </div>
              </div>
            </div>
          </section>

          {/* Player Compatibility */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">Player Compatibility</h2>
            
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Player</th>
                    <th className="text-center p-3 font-semibold text-gray-900">Live</th>
                    <th className="text-center p-3 font-semibold text-gray-900">VOD</th>
                    <th className="text-center p-3 font-semibold text-gray-900">DATERANGE</th>
                    <th className="text-center p-3 font-semibold text-gray-900">CMAF</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Notes</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">hls.js</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-gray-600">Web, most browsers</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Safari AVPlayer</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-gray-600">Native HLS support</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">tvOS AVPlayer</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-gray-600">Apple TV native</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">ExoPlayer (HLS)</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-gray-600">Android, Fire TV</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Shaka Player (HLS)</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-gray-600">Cross-platform</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          {/* Next Steps */}
          <section className="bg-emerald-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/protocols/dash" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">DASH Protocol →</h3>
                <p className="text-sm text-gray-600">MPD structure, EventStream, multi-period</p>
              </Link>
              <Link href="/docs/ssai/modes/live" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Live Mode →</h3>
                <p className="text-sm text-gray-600">Latency requirements, resilience patterns</p>
              </Link>
              <Link href="/docs/ssai/containers" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Container Formats →</h3>
                <p className="text-sm text-gray-600">TS vs CMAF segment formats</p>
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
