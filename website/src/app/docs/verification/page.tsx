'use client';

import { motion } from 'framer-motion';
import {
  CheckCircle,
  Code,
  FileCheck,
  Key,
  Lock,
  Search,
  Terminal,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';
import PageHero from '@/components/PageHero';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

export default function VerificationPage() {
  return (
    <>
      <main className="relative bg-white text-slate-900">
        <header className="bg-white border-b border-slate-200/60">
          <div className="container mx-auto px-4 py-6">
            <HomeNav />
          </div>
        </header>

        {/* Hero */}
        <PageHero variant="light" className="bg-gradient-to-b from-slate-50 to-white border-b border-slate-200/60">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <div className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-brand-100 text-brand-700 text-sm font-medium mb-6">
                <Key className="w-4 h-4" />
                Signed Log Verification
              </div>
              <h1 className="text-4xl md:text-5xl font-bold text-slate-900 leading-tight mb-6">
                Verify Any Auction, Pod, or Session
              </h1>
              <p className="text-xl text-slate-600">
                Every decision ApexMediation makes is cryptographically signed. Here&apos;s how to verify it.
              </p>
            </motion.div>
          </div>
        </PageHero>

        {/* Why Signed Logs */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                  Why Signed Logs Matter
                </h2>
                <p className="text-lg text-gray-600">
                  When you&apos;re reconciling with partners or investigating a discrepancy, you need evidence that can&apos;t be disputed. Our signed logs provide:
                </p>
              </motion.div>

              <div className="grid md:grid-cols-3 gap-6 mb-12">
                <motion.div {...getRevealProps(0.1)} className="bg-gray-50 rounded-xl p-6">
                  <Lock className="w-8 h-8 text-brand-600 mb-4" />
                  <h3 className="font-semibold text-gray-900 mb-2">Tamper Evidence</h3>
                  <p className="text-sm text-gray-600">Any modification to the log invalidates the signature.</p>
                </motion.div>
                <motion.div {...getRevealProps(0.15)} className="bg-gray-50 rounded-xl p-6">
                  <FileCheck className="w-8 h-8 text-brand-600 mb-4" />
                  <h3 className="font-semibold text-gray-900 mb-2">Non-Repudiation</h3>
                  <p className="text-sm text-gray-600">We can&apos;t deny what our system decided—it&apos;s signed.</p>
                </motion.div>
                <motion.div {...getRevealProps(0.2)} className="bg-gray-50 rounded-xl p-6">
                  <Search className="w-8 h-8 text-brand-600 mb-4" />
                  <h3 className="font-semibold text-gray-900 mb-2">Independent Verification</h3>
                  <p className="text-sm text-gray-600">You can verify signatures without our servers.</p>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Verification Steps */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                  How to Verify
                </h2>
              </motion.div>

              {/* Step 1: Export */}
              <motion.div {...getRevealProps(0.1)} className="bg-white rounded-xl p-6 mb-6 border border-gray-100">
                <div className="flex items-start gap-4 mb-4">
                  <div className="w-10 h-10 bg-brand-600 text-white rounded-full flex items-center justify-center font-bold flex-shrink-0">
                    1
                  </div>
                  <div>
                    <h3 className="text-xl font-semibold text-gray-900 mb-2">Export the Evidence Bundle</h3>
                    <p className="text-gray-600 mb-4">
                      From the Console, navigate to the session or pod you want to verify and export the signed bundle.
                    </p>
                  </div>
                </div>
                <div className="bg-gray-900 rounded-lg p-4 font-mono text-sm overflow-x-auto">
                  <pre className="text-gray-300">
{`# Console → Sessions → [Session ID] → Export Evidence Bundle
# Or use the CLI:

apex-cli evidence export --session-id sess_abc123xyz \\
  --output ./evidence-bundle.json`}
                  </pre>
                </div>
              </motion.div>

              {/* Step 2: Inspect */}
              <motion.div {...getRevealProps(0.15)} className="bg-white rounded-xl p-6 mb-6 border border-gray-100">
                <div className="flex items-start gap-4 mb-4">
                  <div className="w-10 h-10 bg-brand-600 text-white rounded-full flex items-center justify-center font-bold flex-shrink-0">
                    2
                  </div>
                  <div>
                    <h3 className="text-xl font-semibold text-gray-900 mb-2">Inspect the Bundle Structure</h3>
                    <p className="text-gray-600 mb-4">
                      The evidence bundle contains all components needed for verification.
                    </p>
                  </div>
                </div>
                <div className="bg-gray-900 rounded-lg p-4 font-mono text-sm overflow-x-auto">
                  <pre className="text-gray-300">
{`{
  "bundleId": "bundle_abc123xyz",
  "sessionId": "sess_abc123xyz",
  "timestamp": "2025-12-15T10:30:00Z",
  "components": {
    "cueTimeline": [...],      // All cues received
    "decisions": [...],        // Pod decisions made
    "stitchVersions": [...],   // Manifest versions served
    "trackingEvents": [...]    // Beacons fired
  },
  "signatures": {
    "cueTimeline": "sha256:a1b2c3...",
    "decisions": "sha256:d4e5f6...",
    "stitchVersions": "sha256:g7h8i9...",
    "trackingEvents": "sha256:j0k1l2...",
    "bundle": "sha256:m3n4o5..."  // Root signature
  },
  "publicKeyId": "key_2025Q4",
  "version": "1.0"
}`}
                  </pre>
                </div>
              </motion.div>

              {/* Step 3: Verify */}
              <motion.div {...getRevealProps(0.2)} className="bg-white rounded-xl p-6 mb-6 border border-gray-100">
                <div className="flex items-start gap-4 mb-4">
                  <div className="w-10 h-10 bg-brand-600 text-white rounded-full flex items-center justify-center font-bold flex-shrink-0">
                    3
                  </div>
                  <div>
                    <h3 className="text-xl font-semibold text-gray-900 mb-2">Verify the Signatures</h3>
                    <p className="text-gray-600 mb-4">
                      Use our CLI tool or verify manually with OpenSSL.
                    </p>
                  </div>
                </div>
                <div className="bg-gray-900 rounded-lg p-4 font-mono text-sm overflow-x-auto">
                  <pre className="text-gray-300">
{`# Using the Apex CLI (recommended)
apex-cli evidence verify --bundle ./evidence-bundle.json

# Output:
✓ Fetching public key: key_2025Q4
✓ Verifying cueTimeline signature... VALID
✓ Verifying decisions signature... VALID
✓ Verifying stitchVersions signature... VALID
✓ Verifying trackingEvents signature... VALID
✓ Verifying bundle root signature... VALID

All signatures verified successfully.
Bundle is authentic and unmodified.`}
                  </pre>
                </div>
              </motion.div>

              {/* Step 4: Manual Verification */}
              <motion.div {...getRevealProps(0.25)} className="bg-white rounded-xl p-6 border border-gray-100">
                <div className="flex items-start gap-4 mb-4">
                  <div className="w-10 h-10 bg-brand-600 text-white rounded-full flex items-center justify-center font-bold flex-shrink-0">
                    4
                  </div>
                  <div>
                    <h3 className="text-xl font-semibold text-gray-900 mb-2">Manual Verification (OpenSSL)</h3>
                    <p className="text-gray-600 mb-4">
                      For independent verification without our tools:
                    </p>
                  </div>
                </div>
                <div className="bg-gray-900 rounded-lg p-4 font-mono text-sm overflow-x-auto">
                  <pre className="text-gray-300">
{`# 1. Download our public key (rotated quarterly)
curl https://keys.apexmediation.ee/v1/key_2025Q4.pem -o apex-key.pem

# 2. Extract the component to verify
jq '.components.decisions' evidence-bundle.json > decisions.json

# 3. Extract the signature
jq -r '.signatures.decisions' evidence-bundle.json | \\
  base64 -d > decisions.sig

# 4. Verify with OpenSSL
openssl dgst -sha256 -verify apex-key.pem \\
  -signature decisions.sig decisions.json

# Output: "Verified OK" if valid`}
                  </pre>
                </div>
              </motion.div>
            </div>
          </div>
        </section>

        {/* What Evidence Looks Like */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                  What Evidence Looks Like
                </h2>
                <p className="text-lg text-gray-600">
                  Here&apos;s a sample of what each component contains.
                </p>
              </motion.div>

              <div className="space-y-6">
                <motion.div {...getRevealProps(0.1)} className="bg-gray-50 rounded-xl p-6">
                  <h3 className="font-semibold text-gray-900 mb-4">Cue Timeline</h3>
                  <div className="bg-gray-900 rounded-lg p-4 font-mono text-xs overflow-x-auto">
                    <pre className="text-gray-300">
{`[
  { "timestamp": "2025-12-15T10:30:00.123Z", "cueId": "cue_001", "type": "start", "mediaTime": 120.5 },
  { "timestamp": "2025-12-15T10:30:30.456Z", "cueId": "cue_001", "type": "end", "mediaTime": 150.5 }
]`}
                    </pre>
                  </div>
                </motion.div>

                <motion.div {...getRevealProps(0.15)} className="bg-gray-50 rounded-xl p-6">
                  <h3 className="font-semibold text-gray-900 mb-4">Pod Decision</h3>
                  <div className="bg-gray-900 rounded-lg p-4 font-mono text-xs overflow-x-auto">
                    <pre className="text-gray-300">
{`{
  "podId": "pod_abc123",
  "slots": 2,
  "filled": 2,
  "ads": [
    { "creativeId": "cr_001", "partner": "partner_a", "duration": 15, "bid": 12.50 },
    { "creativeId": "cr_002", "partner": "partner_b", "duration": 15, "bid": 10.25 }
  ],
  "decisionLatencyMs": 45,
  "timestamp": "2025-12-15T10:30:00.200Z"
}`}
                    </pre>
                  </div>
                </motion.div>

                <motion.div {...getRevealProps(0.2)} className="bg-gray-50 rounded-xl p-6">
                  <h3 className="font-semibold text-gray-900 mb-4">Tracking Events</h3>
                  <div className="bg-gray-900 rounded-lg p-4 font-mono text-xs overflow-x-auto">
                    <pre className="text-gray-300">
{`[
  { "eventType": "impression", "adId": "ad_001", "timestamp": "2025-12-15T10:30:01.000Z" },
  { "eventType": "start", "adId": "ad_001", "timestamp": "2025-12-15T10:30:01.050Z" },
  { "eventType": "firstQuartile", "adId": "ad_001", "timestamp": "2025-12-15T10:30:04.750Z" },
  { "eventType": "midpoint", "adId": "ad_001", "timestamp": "2025-12-15T10:30:08.500Z" },
  { "eventType": "thirdQuartile", "adId": "ad_001", "timestamp": "2025-12-15T10:30:12.250Z" },
  { "eventType": "complete", "adId": "ad_001", "timestamp": "2025-12-15T10:30:16.000Z" }
]`}
                    </pre>
                  </div>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Incident Bundle */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="mb-12">
                <h2 className="text-3xl md:text-4xl font-bold text-gray-900 mb-4">
                  Incident Bundle Export
                </h2>
                <p className="text-lg text-gray-600">
                  When something goes wrong, here&apos;s what you (and your partners) receive:
                </p>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="bg-white rounded-xl p-6 border border-gray-100">
                <ul className="space-y-4">
                  <li className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-green-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="font-medium text-gray-900">Complete Trace</span>
                      <p className="text-sm text-gray-600">Request flow from SDK to partners and back, with timestamps.</p>
                    </div>
                  </li>
                  <li className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-green-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="font-medium text-gray-900">Decision Rationale</span>
                      <p className="text-sm text-gray-600">Why each ad was selected or rejected (format, latency, bid, cap).</p>
                    </div>
                  </li>
                  <li className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-green-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="font-medium text-gray-900">Partner Responses</span>
                      <p className="text-sm text-gray-600">What each partner returned (sanitized of sensitive data).</p>
                    </div>
                  </li>
                  <li className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-green-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="font-medium text-gray-900">Cryptographic Signatures</span>
                      <p className="text-sm text-gray-600">All components signed for tamper-evidence.</p>
                    </div>
                  </li>
                  <li className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-green-600 flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="font-medium text-gray-900">Reconciliation Data</span>
                      <p className="text-sm text-gray-600">Event counts, discrepancies, and resolution notes.</p>
                    </div>
                  </li>
                </ul>
              </motion.div>
            </div>
          </div>
        </section>

        {/* CTA */}
        <section className="py-16 bg-gray-900 text-white">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <h2 className="text-3xl md:text-4xl font-bold mb-6">
                Try it yourself
              </h2>
              <p className="text-lg text-gray-300 mb-8">
                Run a synthetic stream in our sandbox and export a real evidence bundle to verify.
              </p>
              <div className="flex flex-wrap justify-center gap-4">
                <a
                  href="/docs/getting-started/sandbox"
                  className="inline-flex items-center gap-2 px-6 py-3 bg-white text-gray-900 rounded-lg font-semibold hover:bg-gray-100 transition-colors"
                >
                  <Terminal className="w-5 h-5" />
                  Run Sandbox
                </a>
                <a
                  href="https://github.com/sbelakho2/apex-mediation-platform/tree/main/tools/cli"
                  className="inline-flex items-center gap-2 px-6 py-3 bg-white/10 text-white border border-white/20 rounded-lg font-semibold hover:bg-white/20 transition-colors"
                >
                  <Code className="w-5 h-5" />
                  Get CLI Tool
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
