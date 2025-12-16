'use client';

import { useState } from 'react';
import { motion } from 'framer-motion';
import {
  Activity,
  AlertCircle,
  CheckCircle,
  ChevronDown,
  Copy,
  ExternalLink,
  Layers,
  Play,
  RefreshCw,
  Server,
  Terminal,
  Tv,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

// Simulated sandbox session data
const sampleManifest = {
  hlsUrl: 'https://sandbox.apexmediation.ee/hls/demo/master.m3u8',
  dashUrl: 'https://sandbox.apexmediation.ee/dash/demo/manifest.mpd',
  sessionId: 'sandbox_demo_' + Date.now().toString(36),
};

const sampleAuction = {
  sessionId: 'sess_sandbox_demo_001',
  podId: 'pod_mid1_001',
  timestamp: new Date().toISOString(),
  duration: 90,
  slotsRequested: 3,
  slotsFilled: 3,
  decisionsMs: 47,
  partners: [
    { name: 'Partner A', bidMs: 32, bid: 12.50, status: 'won', position: 1 },
    { name: 'Partner B', bidMs: 45, bid: 10.00, status: 'filled', position: 2 },
    { name: 'Partner C', bidMs: 28, bid: 8.75, status: 'filled', position: 3 },
    { name: 'Partner D', bidMs: 67, bid: 0, status: 'timeout', position: null },
  ],
};

const sampleEvents = [
  { time: '00:00:00', event: 'SESSION_INIT', details: 'Session initialized with 3 ad break markers' },
  { time: '00:00:01', event: 'CUE_RECEIVED', details: 'Mid-roll cue at position 300s, duration 90s' },
  { time: '00:00:02', event: 'AUCTION_START', details: 'Pod auction started, 4 partners invited' },
  { time: '00:00:02', event: 'BID_RECEIVED', details: 'Partner C: $8.75 (28ms)' },
  { time: '00:00:02', event: 'BID_RECEIVED', details: 'Partner A: $12.50 (32ms)' },
  { time: '00:00:02', event: 'BID_RECEIVED', details: 'Partner B: $10.00 (45ms)' },
  { time: '00:00:02', event: 'PARTNER_TIMEOUT', details: 'Partner D: exceeded 50ms deadline' },
  { time: '00:00:02', event: 'AUCTION_COMPLETE', details: '3/3 slots filled, total CPM $31.25' },
  { time: '00:00:03', event: 'STITCH_START', details: 'Beginning manifest stitching' },
  { time: '00:00:03', event: 'STITCH_COMPLETE', details: 'Pod ready for playback' },
  { time: '00:05:00', event: 'IMPRESSION', details: 'Slot 1 impression fired' },
  { time: '00:05:30', event: 'QUARTILE_25', details: 'Slot 1 reached 25%' },
  { time: '00:06:00', event: 'QUARTILE_50', details: 'Slot 1 reached 50%' },
  { time: '00:06:30', event: 'QUARTILE_75', details: 'Slot 1 reached 75%' },
  { time: '00:07:00', event: 'COMPLETE', details: 'Slot 1 completed' },
];

const eventColors: Record<string, string> = {
  SESSION_INIT: 'text-blue-600 bg-blue-100',
  CUE_RECEIVED: 'text-purple-600 bg-purple-100',
  AUCTION_START: 'text-indigo-600 bg-indigo-100',
  BID_RECEIVED: 'text-green-600 bg-green-100',
  PARTNER_TIMEOUT: 'text-amber-600 bg-amber-100',
  AUCTION_COMPLETE: 'text-brand-600 bg-brand-100',
  STITCH_START: 'text-cyan-600 bg-cyan-100',
  STITCH_COMPLETE: 'text-teal-600 bg-teal-100',
  IMPRESSION: 'text-emerald-600 bg-emerald-100',
  QUARTILE_25: 'text-lime-600 bg-lime-100',
  QUARTILE_50: 'text-lime-600 bg-lime-100',
  QUARTILE_75: 'text-lime-600 bg-lime-100',
  COMPLETE: 'text-green-700 bg-green-200',
};

export default function SandboxPage() {
  const [isRunning, setIsRunning] = useState(false);
  const [currentEventIndex, setCurrentEventIndex] = useState(-1);
  const [showFullLog, setShowFullLog] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);

  const startDemo = () => {
    setIsRunning(true);
    setCurrentEventIndex(0);
    
    // Simulate events appearing over time
    let index = 0;
    const interval = setInterval(() => {
      index++;
      if (index >= sampleEvents.length) {
        clearInterval(interval);
        setIsRunning(false);
      } else {
        setCurrentEventIndex(index);
      }
    }, 400);
  };

  const resetDemo = () => {
    setIsRunning(false);
    setCurrentEventIndex(-1);
  };

  const copyToClipboard = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopied(id);
    setTimeout(() => setCopied(null), 2000);
  };

  const visibleEvents = currentEventIndex >= 0 
    ? sampleEvents.slice(0, currentEventIndex + 1) 
    : [];

  return (
    <>
      <main className="relative bg-white text-slate-900">
        <header className="bg-white border-b border-slate-200/60">
          <div className="container mx-auto px-4 py-6">
            <HomeNav />
          </div>
        </header>

        {/* Hero */}
        <section className="py-16 md:py-16 bg-gradient-to-b from-gray-900 to-gray-800">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <div className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-brand-600/20 text-brand-400 text-sm font-medium mb-6">
                <Play className="w-4 h-4" />
                Interactive Demo
              </div>
              <h1 className="text-4xl md:text-5xl font-bold text-white leading-tight mb-6">
                SSAI Evaluation Sandbox
              </h1>
              <p className="text-xl text-gray-300 mb-8">
                Run a simulated SSAI session without any credentials. See exactly how ApexMediation handles ad breaks, auctions, and playback.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                {!isRunning && currentEventIndex < 0 ? (
                  <button
                    onClick={startDemo}
                    className="btn-primary inline-flex items-center gap-2"
                  >
                    <Play className="w-5 h-5" />
                    Start Demo Session
                  </button>
                ) : (
                  <>
                    {isRunning && (
                      <div className="inline-flex items-center gap-2 px-6 py-3 bg-brand-600 text-white rounded-lg font-semibold">
                        <RefreshCw className="w-5 h-5 animate-spin" />
                        Running...
                      </div>
                    )}
                    <button
                      onClick={resetDemo}
                      className="inline-flex items-center gap-2 px-6 py-3 bg-white/10 text-white border border-white/20 rounded-lg font-semibold hover:bg-white/20 transition-colors"
                    >
                      Reset Demo
                    </button>
                  </>
                )}
              </div>
            </motion.div>
          </div>
        </section>

        {/* Live Console */}
        <section className="py-12 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <motion.div {...getRevealProps(0)} className="flex items-center gap-3 mb-6">
                <Terminal className="w-6 h-6 text-gray-700" />
                <h2 className="text-2xl font-bold text-gray-900">Live Event Console</h2>
                {isRunning && (
                  <span className="flex items-center gap-2 px-3 py-1 bg-green-100 text-green-700 rounded-full text-sm font-medium">
                    <span className="w-2 h-2 bg-green-500 rounded-full animate-pulse"></span>
                    Live
                  </span>
                )}
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="bg-gray-900 rounded-xl overflow-hidden shadow-xl">
                {/* Console Header */}
                <div className="flex items-center justify-between px-4 py-3 bg-gray-800 border-b border-gray-700">
                  <div className="flex items-center gap-3">
                    <div className="flex gap-2">
                      <span className="w-3 h-3 bg-red-500 rounded-full"></span>
                      <span className="w-3 h-3 bg-yellow-500 rounded-full"></span>
                      <span className="w-3 h-3 bg-green-500 rounded-full"></span>
                    </div>
                    <span className="text-gray-400 text-sm font-mono">apex-sandbox-console</span>
                  </div>
                  <button
                    onClick={() => setShowFullLog(!showFullLog)}
                    className="text-gray-400 hover:text-white text-sm flex items-center gap-1"
                  >
                    {showFullLog ? 'Collapse' : 'Expand'}
                    <ChevronDown className={`w-4 h-4 transition-transform ${showFullLog ? 'rotate-180' : ''}`} />
                  </button>
                </div>

                {/* Console Body */}
                <div className={`font-mono text-sm p-4 overflow-y-auto transition-all ${showFullLog ? 'max-h-[600px]' : 'max-h-[300px]'}`}>
                  {currentEventIndex < 0 ? (
                    <div className="text-gray-500 text-center py-8">
                      Click &quot;Start Demo Session&quot; to begin the simulation
                    </div>
                  ) : (
                    <div className="space-y-2">
                      {visibleEvents.map((event, idx) => (
                        <motion.div
                          key={idx}
                          initial={{ opacity: 0, x: -20 }}
                          animate={{ opacity: 1, x: 0 }}
                          className="flex items-start gap-3"
                        >
                          <span className="text-gray-500 w-20 flex-shrink-0">{event.time}</span>
                          <span className={`px-2 py-0.5 rounded text-xs font-medium flex-shrink-0 ${eventColors[event.event] || 'text-gray-600 bg-gray-700'}`}>
                            {event.event}
                          </span>
                          <span className="text-gray-300">{event.details}</span>
                        </motion.div>
                      ))}
                      {isRunning && (
                        <div className="flex items-center gap-2 text-gray-500">
                          <span className="w-2 h-2 bg-brand-500 rounded-full animate-pulse"></span>
                          Waiting for next event...
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Auction Results */}
        {currentEventIndex >= 7 && (
          <section className="py-12">
            <div className="container mx-auto px-4">
              <div className="max-w-6xl mx-auto">
                <motion.div
                  initial={{ opacity: 0, y: 20 }}
                  animate={{ opacity: 1, y: 0 }}
                  className="mb-6"
                >
                  <div className="flex items-center gap-3 mb-6">
                    <Layers className="w-6 h-6 text-gray-700" />
                    <h2 className="text-2xl font-bold text-gray-900">Auction Results</h2>
                  </div>
                </motion.div>

                <motion.div
                  initial={{ opacity: 0, y: 20 }}
                  animate={{ opacity: 1, y: 0 }}
                  className="grid md:grid-cols-3 gap-6"
                >
                  {/* Pod Summary */}
                  <div className="bg-white rounded-xl p-6 shadow-sm border border-gray-100">
                    <h3 className="font-semibold text-gray-900 mb-4">Pod Summary</h3>
                    <dl className="space-y-3">
                      <div className="flex justify-between">
                        <dt className="text-gray-600">Pod ID</dt>
                        <dd className="font-mono text-sm text-gray-900">{sampleAuction.podId}</dd>
                      </div>
                      <div className="flex justify-between">
                        <dt className="text-gray-600">Duration</dt>
                        <dd className="font-medium text-gray-900">{sampleAuction.duration}s</dd>
                      </div>
                      <div className="flex justify-between">
                        <dt className="text-gray-600">Decision Time</dt>
                        <dd className="font-medium text-brand-600">{sampleAuction.decisionsMs}ms</dd>
                      </div>
                      <div className="flex justify-between">
                        <dt className="text-gray-600">Fill Rate</dt>
                        <dd className="font-medium text-green-600">100% (3/3)</dd>
                      </div>
                    </dl>
                  </div>

                  {/* Partner Bids */}
                  <div className="bg-white rounded-xl p-6 shadow-sm border border-gray-100 md:col-span-2">
                    <h3 className="font-semibold text-gray-900 mb-4">Partner Responses</h3>
                    <div className="overflow-x-auto">
                      <table className="w-full text-sm">
                        <thead>
                          <tr className="text-left text-gray-500 border-b border-gray-100">
                            <th className="pb-2 font-medium">Partner</th>
                            <th className="pb-2 font-medium">Response</th>
                            <th className="pb-2 font-medium">Bid CPM</th>
                            <th className="pb-2 font-medium">Status</th>
                          </tr>
                        </thead>
                        <tbody>
                          {sampleAuction.partners.map((partner) => (
                            <tr key={partner.name} className="border-b border-gray-50">
                              <td className="py-3 font-medium text-gray-900">{partner.name}</td>
                              <td className="py-3 font-mono text-gray-600">{partner.bidMs}ms</td>
                              <td className="py-3 font-medium text-gray-900">
                                {partner.bid > 0 ? `$${partner.bid.toFixed(2)}` : '—'}
                              </td>
                              <td className="py-3">
                                {partner.status === 'won' && (
                                  <span className="inline-flex items-center gap-1 text-green-700 bg-green-100 px-2 py-0.5 rounded-full text-xs font-medium">
                                    <CheckCircle className="w-3 h-3" /> Winner
                                  </span>
                                )}
                                {partner.status === 'filled' && (
                                  <span className="inline-flex items-center gap-1 text-blue-700 bg-blue-100 px-2 py-0.5 rounded-full text-xs font-medium">
                                    <CheckCircle className="w-3 h-3" /> Filled
                                  </span>
                                )}
                                {partner.status === 'timeout' && (
                                  <span className="inline-flex items-center gap-1 text-amber-700 bg-amber-100 px-2 py-0.5 rounded-full text-xs font-medium">
                                    <AlertCircle className="w-3 h-3" /> Timeout
                                  </span>
                                )}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  </div>
                </motion.div>
              </div>
            </div>
          </section>
        )}

        {/* Manifest URLs */}
        <section className="py-12 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <motion.div {...getRevealProps(0)} className="flex items-center gap-3 mb-6">
                <Server className="w-6 h-6 text-gray-700" />
                <h2 className="text-2xl font-bold text-gray-900">Sample Manifest URLs</h2>
              </motion.div>

              <div className="grid md:grid-cols-2 gap-6">
                {/* HLS */}
                <motion.div {...getRevealProps(0.1)} className="bg-white rounded-xl p-6 shadow-sm border border-gray-100">
                  <div className="flex items-center gap-2 mb-4">
                    <div className="w-8 h-8 bg-brand-100 rounded-lg flex items-center justify-center">
                      <Play className="w-4 h-4 text-brand-600" />
                    </div>
                    <h3 className="font-semibold text-gray-900">HLS Manifest</h3>
                  </div>
                  <div className="bg-gray-50 rounded-lg p-3 font-mono text-sm text-gray-700 break-all mb-3">
                    {sampleManifest.hlsUrl}
                  </div>
                  <button
                    onClick={() => copyToClipboard(sampleManifest.hlsUrl, 'hls')}
                    className="inline-flex items-center gap-2 text-sm text-brand-600 hover:text-brand-700"
                  >
                    {copied === 'hls' ? (
                      <>
                        <CheckCircle className="w-4 h-4" />
                        Copied!
                      </>
                    ) : (
                      <>
                        <Copy className="w-4 h-4" />
                        Copy URL
                      </>
                    )}
                  </button>
                </motion.div>

                {/* DASH */}
                <motion.div {...getRevealProps(0.15)} className="bg-white rounded-xl p-6 shadow-sm border border-gray-100">
                  <div className="flex items-center gap-2 mb-4">
                    <div className="w-8 h-8 bg-purple-100 rounded-lg flex items-center justify-center">
                      <Activity className="w-4 h-4 text-purple-600" />
                    </div>
                    <h3 className="font-semibold text-gray-900">DASH Manifest</h3>
                  </div>
                  <div className="bg-gray-50 rounded-lg p-3 font-mono text-sm text-gray-700 break-all mb-3">
                    {sampleManifest.dashUrl}
                  </div>
                  <button
                    onClick={() => copyToClipboard(sampleManifest.dashUrl, 'dash')}
                    className="inline-flex items-center gap-2 text-sm text-brand-600 hover:text-brand-700"
                  >
                    {copied === 'dash' ? (
                      <>
                        <CheckCircle className="w-4 h-4" />
                        Copied!
                      </>
                    ) : (
                      <>
                        <Copy className="w-4 h-4" />
                        Copy URL
                      </>
                    )}
                  </button>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Test on Your Player */}
        <section className="py-12">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <motion.div {...getRevealProps(0)} className="flex items-center gap-3 mb-6">
                <Tv className="w-6 h-6 text-gray-700" />
                <h2 className="text-2xl font-bold text-gray-900">Test with Your Player</h2>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="bg-white rounded-xl p-6 shadow-sm border border-gray-100">
                <p className="text-gray-600 mb-6">
                  Use these sandbox URLs to test SSAI integration with your video player. The sandbox returns pre-stitched manifests with sample ads for evaluation purposes.
                </p>
                
                <div className="grid md:grid-cols-3 gap-6">
                  <div className="bg-gray-50 rounded-lg p-4">
                    <h4 className="font-semibold text-gray-900 mb-2">Web (HLS.js / Shaka)</h4>
                    <p className="text-sm text-gray-600 mb-3">Pass the HLS URL to your web player.</p>
                    <code className="text-xs bg-gray-900 text-gray-300 rounded px-2 py-1 block overflow-x-auto">
                      player.loadSource(hlsUrl);
                    </code>
                  </div>
                  <div className="bg-gray-50 rounded-lg p-4">
                    <h4 className="font-semibold text-gray-900 mb-2">iOS (AVPlayer)</h4>
                    <p className="text-sm text-gray-600 mb-3">Load as AVAsset with the HLS URL.</p>
                    <code className="text-xs bg-gray-900 text-gray-300 rounded px-2 py-1 block overflow-x-auto">
                      AVPlayer(url: hlsUrl)
                    </code>
                  </div>
                  <div className="bg-gray-50 rounded-lg p-4">
                    <h4 className="font-semibold text-gray-900 mb-2">Android (ExoPlayer)</h4>
                    <p className="text-sm text-gray-600 mb-3">Create HLS media source.</p>
                    <code className="text-xs bg-gray-900 text-gray-300 rounded px-2 py-1 block overflow-x-auto">
                      HlsMediaSource.Factory(...)
                    </code>
                  </div>
                </div>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Export Evidence */}
        {currentEventIndex >= sampleEvents.length - 1 && (
          <section className="py-12 bg-gray-900 text-white">
            <div className="container mx-auto px-4">
              <div className="max-w-4xl mx-auto text-center">
                <motion.div
                  initial={{ opacity: 0, y: 20 }}
                  animate={{ opacity: 1, y: 0 }}
                >
                  <CheckCircle className="w-12 h-12 text-green-400 mx-auto mb-6" />
                  <h2 className="text-3xl font-bold mb-4">Session Complete!</h2>
                  <p className="text-gray-300 mb-8">
                    This demo session generated a signed evidence bundle that you can verify independently. In production, every session produces these bundles automatically.
                  </p>
                  <div className="flex flex-wrap justify-center gap-4">
                    <a
                      href="/docs/verification"
                      className="inline-flex items-center gap-2 px-6 py-3 bg-white text-gray-900 rounded-lg font-semibold hover:bg-gray-100 transition-colors"
                    >
                      Learn to Verify Bundles
                    </a>
                    <a
                      href="/security-pack"
                      className="inline-flex items-center gap-2 px-6 py-3 bg-white/10 text-white border border-white/20 rounded-lg font-semibold hover:bg-white/20 transition-colors"
                    >
                      View Security Pack
                    </a>
                  </div>
                </motion.div>
              </div>
            </div>
          </section>
        )}

        {/* CTA */}
        <section className="py-16 bg-white">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-6">
                Ready for a Real Integration?
              </h2>
              <p className="text-lg text-gray-600 mb-8">
                Get a dedicated sandbox environment with your own tenant credentials, or schedule a live demo with our team.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                <a
                  href="/contact"
                  className="btn-primary"
                >
                  Request Full Sandbox Access
                </a>
                <a
                  href="/docs/playback-matrix"
                  className="inline-flex items-center gap-2 px-6 py-3 bg-gray-100 text-gray-900 rounded-lg font-semibold hover:bg-gray-200 transition-colors"
                >
                  View Playback Matrix
                  <ExternalLink className="w-4 h-4" />
                </a>
              </div>
            </motion.div>
          </div>
        </section>

        <Footer />
      </main>
      <CookieBanner />
    </>
  );
}
