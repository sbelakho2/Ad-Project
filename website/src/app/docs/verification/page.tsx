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
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';
import { repoPath } from '@/config/links';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

export default function VerificationPage() {
  return (
    <>
      <main className="relative bg-white text-ink">
        <SiteHeader />

        {/* Hero */}
        <PageHero className="bg-gradient-to-b from-gray-50 to-white border-b border-gray-200/60">
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <div className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-brand-100 text-brand-700 text-sm font-medium mb-6">
                <Key className="w-4 h-4" />
                Signed Log Verification
              </div>
              <h1 className="text-4xl md:text-5xl font-bold text-ink leading-tight mb-6">
                Verify Any Auction, Pod, or Session
              </h1>
              <p className="text-xl text-inkMuted">
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
                    <h3 className="text-xl font-semibold text-gray-900 mb-2">Export the Auction Receipt</h3>
                    <p className="text-gray-600 mb-4">
                      Export the signed auction receipt from the Console, or fetch it via the Transparency API.
                    </p>
                  </div>
                </div>
                <div className="bg-gray-900 rounded-lg p-4 font-mono text-sm overflow-x-auto">
                  <pre className="text-gray-300">
{`# Fetch a specific auction (requires auth)
# Tip: includeCanonical=1 returns the canonical string to verify.

curl -s \
  -H "Authorization: Bearer $APEX_TOKEN" \
  "https://api.apexmediation.ee/api/v1/transparency/auctions/auc_abc123?includeCanonical=1" | jq . > auction.json`}
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
                      The receipt includes the signature, key id, algorithm, and (optionally) the canonical string.
                    </p>
                  </div>
                </div>
                <div className="bg-gray-900 rounded-lg p-4 font-mono text-sm overflow-x-auto">
                  <pre className="text-gray-300">
{`{
  "auction_id": "auc_abc123",
  "timestamp": "2025-12-15T10:30:00Z",
  "winner": { "source": "network_a", "bid_ecpm": 2.5, "currency": "USD" },
  "integrity": {
    "algo": "ed25519",
    "key_id": "key_2025Q4",
    "signature": "<base64 Ed25519 signature>"
  },
  "canonical": {
    "string": "{...canonical JSON string...}",
    "truncated": false,
    "size_bytes": 1234
  }
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
                      Verify locally (offline) once you have the receipt JSON.
                    </p>
                  </div>
                </div>
                <div className="bg-gray-900 rounded-lg p-4 font-mono text-sm overflow-x-auto">
                  <pre className="text-gray-300">
{`# Extract canonical string + signature from the receipt
jq -r '.canonical.string' auction.json > canonical.txt
jq -r '.integrity.signature' auction.json | base64 -d > signature.bin

# Confirm metadata
jq -r '.integrity.algo' auction.json    # ed25519
jq -r '.integrity.key_id' auction.json  # key_2025Q4`}
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
curl -s https://api.apexmediation.ee/api/v1/transparency/keys | jq .

# If you want a PEM file for a specific key_id:
curl -s https://api.apexmediation.ee/api/v1/transparency/keys | \\
  jq -r '.data[] | select(.key_id=="key_2025Q4") | (.public_key_pem // empty)' > apex-key.pem

# If your response doesn't include public_key_pem, fall back to public_key_base64:
curl -s https://api.apexmediation.ee/api/v1/transparency/keys | \\
  jq -r '.data[] | select(.key_id=="key_2025Q4") | .public_key_base64' | \\
  awk 'BEGIN{print "-----BEGIN PUBLIC KEY-----"} {print} END{print "-----END PUBLIC KEY-----"}' > apex-key.pem

# 2. Verify with OpenSSL (Ed25519 verifies the message directly; no hashing step)
openssl pkeyutl -verify -pubin -inkey apex-key.pem \\
  -sigfile signature.bin -in canonical.txt

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
                    <CheckCircle className="w-5 h-5 text-success flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="font-medium text-gray-900">Complete Trace</span>
                      <p className="text-sm text-gray-600">Request flow from SDK to partners and back, with timestamps.</p>
                    </div>
                  </li>
                  <li className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-success flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="font-medium text-gray-900">Decision Rationale</span>
                      <p className="text-sm text-gray-600">Why each ad was selected or rejected (format, latency, bid, cap).</p>
                    </div>
                  </li>
                  <li className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-success flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="font-medium text-gray-900">Partner Responses</span>
                      <p className="text-sm text-gray-600">What each partner returned (sanitized of sensitive data).</p>
                    </div>
                  </li>
                  <li className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-success flex-shrink-0 mt-0.5" />
                    <div>
                      <span className="font-medium text-gray-900">Cryptographic Signatures</span>
                      <p className="text-sm text-gray-600">All components signed for tamper-evidence.</p>
                    </div>
                  </li>
                  <li className="flex items-start gap-3">
                    <CheckCircle className="w-5 h-5 text-success flex-shrink-0 mt-0.5" />
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
                  href={repoPath('tools/cli')}
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
