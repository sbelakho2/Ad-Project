'use client';

import Link from 'next/link';
import { CheckCircle, AlertTriangle, FileText, Clock, Code, Layers, Zap } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function DASHProtocolPage() {
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
            <span className="text-gray-900">DASH Protocol</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              DASH Protocol Support
            </h1>
            <p className="text-xl text-gray-600">
              DASH-IF compliant MPD generation with multi-period ad insertion, EventStream 
              signaling, and seamless period transitions for SSAI.
            </p>
          </div>

          {/* MPD Structure */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Layers className="w-6 h-6 text-brand-600" />
              MPD Structure
            </h2>
            
            <p className="text-gray-600 mb-4">
              DASH manifests (Media Presentation Descriptions) are XML documents that describe 
              the available media streams. ApexMediation supports both single-period origin 
              content and multi-period SSAI output.
            </p>

            <div className="bg-gray-50 rounded-lg p-6 mb-6">
              <h3 className="font-semibold text-gray-900 mb-3">Multi-Period MPD Structure</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011"
     type="dynamic"
     minimumUpdatePeriod="PT2S"
     availabilityStartTime="YYYY-MM-DDTHH:MM:SSZ"
     publishTime="YYYY-MM-DDTHH:MM:SSZ">
  
  <UTCTiming schemeIdUri="urn:mpeg:dash:utc:http-xsdate:2014"
             value="https://time.apexmediation.ee/xsdate"/>
  
  <!-- Content Period -->
  <Period id="content-1" start="PT0S" duration="PT30M24S">
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <Representation id="v1080" bandwidth="5000000" width="1920" height="1080">
        <SegmentTemplate timescale="90000" 
                        initialization="init-$RepresentationID$.m4s"
                        media="seg-$RepresentationID$-$Time$.m4s">
          <SegmentTimeline>
            <S t="0" d="540000" r="303"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
  
  <!-- Ad Period -->
  <Period id="ad-pod-1" start="PT30M24S" duration="PT30S">
    <EventStream schemeIdUri="urn:apex:ssai:2025" timescale="1000">
      <Event presentationTime="0" duration="30000" id="pod-1">
        {"type":"ad-pod","slots":2}
      </Event>
    </EventStream>
    <AdaptationSet mimeType="video/mp4" contentType="video">
      <!-- Ad segments here -->
    </AdaptationSet>
  </Period>
  
  <!-- Content Period (continued) -->
  <Period id="content-2" start="PT30M54S">
    <!-- Content continues -->
  </Period>
  
</MPD>`}
              </pre>
            </div>

            <h3 className="font-semibold text-gray-900 mb-3">Period Types</h3>
            <div className="grid md:grid-cols-2 gap-4">
              <div className="bg-gray-50 p-4 rounded-lg">
                <h4 className="font-medium text-gray-900 mb-2">Content Periods</h4>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Source from origin CMS</li>
                  <li>• Stable segment templates</li>
                  <li>• Long-cacheable segments</li>
                </ul>
              </div>
              <div className="bg-gray-50 p-4 rounded-lg">
                <h4 className="font-medium text-gray-900 mb-2">Ad Periods</h4>
                <ul className="text-sm text-gray-600 space-y-1">
                  <li>• Injected by SSAI service</li>
                  <li>• Per-session segment URLs</li>
                  <li>• EventStream metadata</li>
                </ul>
              </div>
            </div>
          </section>

          {/* Single to Multi-Period Conversion */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Zap className="w-6 h-6 text-brand-600" />
              Single-to-Multi-Period Conversion
            </h2>
            
            <p className="text-gray-600 mb-4">
              Many origins provide single-period MPDs. The SSAI service converts these to 
              multi-period on the fly, splitting at ad break boundaries.
            </p>

            <div className="grid md:grid-cols-2 gap-6 mb-6">
              <div className="bg-gray-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">Origin MPD (Single Period)</h3>
                <pre className="bg-gray-900 rounded-lg p-3 text-xs text-gray-300 overflow-x-auto">
{`<MPD type="dynamic">
  <Period id="main">
    <!-- All content in one period -->
    <EventStream>
      <Event><!-- SCTE-35 marker --></Event>
    </EventStream>
    <AdaptationSet>
      <!-- Segments -->
    </AdaptationSet>
  </Period>
</MPD>`}
                </pre>
              </div>
              
              <div className="bg-brand-50 rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3">SSAI MPD (Multi Period)</h3>
                <pre className="bg-gray-900 rounded-lg p-3 text-xs text-gray-300 overflow-x-auto">
{`<MPD type="dynamic">
  <Period id="content-1">
    <!-- Content before ad -->
  </Period>
  <Period id="ad-pod-1">
    <!-- Inserted ad pod -->
  </Period>
  <Period id="content-2">
    <!-- Content after ad -->
  </Period>
</MPD>`}
                </pre>
              </div>
            </div>

            <div className="bg-info/10 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-2">Conversion Process</h3>
              <ol className="space-y-2 text-sm text-gray-600">
                <li><strong>1. Parse Events:</strong> Detect SCTE-35 or custom ad markers in origin EventStream</li>
                <li><strong>2. Split Timeline:</strong> Create new period boundaries at each ad marker</li>
                <li><strong>3. Inject Ads:</strong> Insert ad periods with decisioned creative segments</li>
                <li><strong>4. Adjust Timing:</strong> Update segment timelines and period start times</li>
              </ol>
            </div>
          </section>

          {/* EventStream */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Code className="w-6 h-6 text-brand-600" />
              EventStream Signaling
            </h2>
            
            <p className="text-gray-600 mb-4">
              DASH uses EventStream elements to signal in-band events. ApexMediation supports 
              both origin-side and SSAI-side event streams.
            </p>

            <h3 className="font-semibold text-gray-900 mb-3">Supported Scheme URIs</h3>
            <div className="overflow-x-auto mb-6">
              <table className="w-full text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Scheme URI</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Purpose</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Direction</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-700"><code>urn:scte:scte35:2014:xml+bin</code></td>
                    <td className="p-3 text-gray-600">SCTE-35 binary markers</td>
                    <td className="p-3 text-gray-600">Origin → SSAI</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-700"><code>urn:dvb:dash:cuepoint:2022</code></td>
                    <td className="p-3 text-gray-600">DVB-DASH cue points</td>
                    <td className="p-3 text-gray-600">Origin → SSAI</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-700"><code>urn:apex:ssai:2025</code></td>
                    <td className="p-3 text-gray-600">SSAI pod metadata</td>
                    <td className="p-3 text-gray-600">SSAI → Player</td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div className="bg-gray-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">SSAI Event Structure</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`<EventStream schemeIdUri="urn:apex:ssai:2025" timescale="1000">
  <Event presentationTime="0" duration="30000" id="pod-abc123">
    {
      "type": "ad-pod",
      "podId": "pod-abc123",
      "breakType": "midroll",
      "slots": [
        {"slotId": "slot-1", "creativeId": "cr-xyz", "duration": 15000},
        {"slotId": "slot-2", "creativeId": "cr-abc", "duration": 15000}
      ],
      "trackingUrl": "https://track.apexmediation.ee/v1/pod/abc123"
    }
  </Event>
</EventStream>`}
              </pre>
            </div>
          </section>

          {/* Live Behavior */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Clock className="w-6 h-6 text-brand-600" />
              Live DASH Behavior
            </h2>
            
            <p className="text-gray-600 mb-4">
              Live DASH streams require careful MPD update handling to ensure smooth playback 
              across ad insertions.
            </p>

            <div className="space-y-4">
              <div className="border-l-4 border-brand-500 pl-4">
                <h3 className="font-semibold text-gray-900">UTCTiming Strategy</h3>
                <p className="text-gray-600 text-sm">
                  Stable <code>UTCTiming</code> elements ensure players sync correctly. We use 
                  <code>urn:mpeg:dash:utc:http-xsdate:2014</code> with our time server.
                </p>
              </div>
              
              <div className="border-l-4 border-brand-500 pl-4">
                <h3 className="font-semibold text-gray-900">Segment Timeline Evolution</h3>
                <p className="text-gray-600 text-sm">
                  Segment timelines evolve predictably. The <code>minimumUpdatePeriod</code> and 
                  <code>timeShiftBufferDepth</code> are maintained consistently.
                </p>
              </div>
              
              <div className="border-l-4 border-brand-500 pl-4">
                <h3 className="font-semibold text-gray-900">Period Continuity</h3>
                <p className="text-gray-600 text-sm">
                  Period IDs are stable across MPD updates. Periods are never removed or reordered 
                  within the availability window.
                </p>
              </div>
            </div>

            <div className="bg-warning/10 rounded-lg p-6 mt-6">
              <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                <AlertTriangle className="w-5 h-5 text-warning" />
                Avoided Issues
              </h3>
              <ul className="space-y-2 text-sm text-gray-600">
                <li><strong>Player &quot;reset&quot; at period transitions:</strong> Caused by inconsistent adaptation sets. Our ad periods match content encoding parameters.</li>
                <li><strong>Timeline gaps:</strong> Caused by incorrect period durations. All period boundaries are precisely calculated.</li>
                <li><strong>UTCTiming drift:</strong> Caused by unreliable time sources. Our time server has &lt;10ms accuracy.</li>
              </ul>
            </div>
          </section>

          {/* Player Support */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <FileText className="w-6 h-6 text-brand-600" />
              Player Compatibility
            </h2>
            
            <p className="text-gray-600 mb-4">
              Different DASH players have varying levels of support for multi-period manifests 
              and EventStream parsing.
            </p>

            <div className="overflow-x-auto mb-6">
              <table className="w-full text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Player</th>
                    <th className="text-center p-3 font-semibold text-gray-900">Multi-Period</th>
                    <th className="text-center p-3 font-semibold text-gray-900">EventStream</th>
                    <th className="text-center p-3 font-semibold text-gray-900">CMAF</th>
                    <th className="text-center p-3 font-semibold text-gray-900">Live</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Notes</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">Shaka Player</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-gray-600">Best cross-platform support</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">dash.js</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-gray-600">Reference implementation</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">ExoPlayer</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-gray-600">Android, Fire TV</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Bitmovin</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-gray-600">Commercial player</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Roku (native)</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-warning">△</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-center text-success">✓</td>
                    <td className="p-3 text-gray-600">EventStream via RAF</td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div className="bg-info/10 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">SDK Integration for Event Handling</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`// Web SDK - Shaka Player integration
import { SSAIManifestParser } from '@apexmediation/web-sdk';

// Shaka event listener
player.addEventListener('timelineregionenter', (event) => {
  const region = event.region;
  if (region.schemeIdUri === 'urn:apex:ssai:2025') {
    const adPod = JSON.parse(region.eventNode.textContent);
    console.log('Entering ad pod:', adPod.podId);
    // SDK handles tracking automatically
  }
});

player.addEventListener('timelineregionexit', (event) => {
  console.log('Exiting region:', event.region.id);
});`}
              </pre>
            </div>
          </section>

          {/* VOD Specifics */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">VOD-Specific Behavior</h2>
            
            <div className="space-y-4">
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <CheckCircle className="w-6 h-6 text-success flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Static MPD Type</h3>
                  <p className="text-gray-600 text-sm">
                    VOD manifests use <code>type=&quot;static&quot;</code> with <code>mediaPresentationDuration</code> 
                    set to the total duration including all ad breaks.
                  </p>
                </div>
              </div>
              
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <CheckCircle className="w-6 h-6 text-success flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Complete Period List</h3>
                  <p className="text-gray-600 text-sm">
                    All periods (content and ads) are present in the initial manifest. 
                    No MPD updates are required during playback.
                  </p>
                </div>
              </div>
              
              <div className="flex items-start gap-4 p-4 bg-gray-50 rounded-lg">
                <CheckCircle className="w-6 h-6 text-success flex-shrink-0 mt-1" />
                <div>
                  <h3 className="font-semibold text-gray-900">Seek Support</h3>
                  <p className="text-gray-600 text-sm">
                    Players can seek across periods seamlessly. Period boundaries are 
                    correctly mapped to seek positions.
                  </p>
                </div>
              </div>
            </div>
          </section>

          {/* Comparison with HLS */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">DASH vs HLS for SSAI</h2>
            
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Feature</th>
                    <th className="text-left p-3 font-semibold text-gray-900">DASH</th>
                    <th className="text-left p-3 font-semibold text-gray-900">HLS</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">Ad Break Structure</td>
                    <td className="p-3 text-gray-600">Separate periods</td>
                    <td className="p-3 text-gray-600">Inline with discontinuity</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Metadata</td>
                    <td className="p-3 text-gray-600">EventStream elements</td>
                    <td className="p-3 text-gray-600">DATERANGE tags</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Encoding Changes</td>
                    <td className="p-3 text-gray-600">Natural at period boundary</td>
                    <td className="p-3 text-gray-600">EXT-X-DISCONTINUITY required</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Safari Support</td>
                    <td className="p-3 text-gray-600">Requires MSE polyfill</td>
                    <td className="p-3 text-gray-600">Native</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Widevine/PlayReady</td>
                    <td className="p-3 text-gray-600">Full support</td>
                    <td className="p-3 text-gray-600">Limited (CMAF only)</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          {/* Next Steps */}
          <section className="bg-brand-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/protocols/hls" className="bg-white p-4 rounded-lg border border-brand-200 hover:border-brand-400 transition-colors">
                <h3 className="font-semibold text-gray-900">HLS Protocol →</h3>
                <p className="text-sm text-gray-600">RFC 8216 compliance, DATERANGE markers</p>
              </Link>
              <Link href="/docs/ssai/modes/live" className="bg-white p-4 rounded-lg border border-brand-200 hover:border-brand-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Live Mode →</h3>
                <p className="text-sm text-gray-600">Latency requirements, resilience patterns</p>
              </Link>
              <Link href="/docs/ssai/drm" className="bg-white p-4 rounded-lg border border-brand-200 hover:border-brand-400 transition-colors">
                <h3 className="font-semibold text-gray-900">DRM Support →</h3>
                <p className="text-sm text-gray-600">Widevine, PlayReady integration</p>
              </Link>
              <Link href="/docs/ssai" className="bg-white p-4 rounded-lg border border-brand-200 hover:border-brand-400 transition-colors">
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
