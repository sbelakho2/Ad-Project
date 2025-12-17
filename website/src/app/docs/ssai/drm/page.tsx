'use client';

import Link from 'next/link';
import { Shield, Key, Lock, AlertTriangle, CheckCircle, Settings } from 'lucide-react';
import SiteHeader from '@/components/SiteHeader';
import Footer from '@/components/Footer';

export default function DRMPage() {
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
            <span className="text-gray-900">DRM Integration</span>
          </div>

          {/* Header */}
          <div className="mb-12">
            <h1 className="text-4xl font-bold text-gray-900 mb-4">
              DRM Integration
            </h1>
            <p className="text-xl text-gray-600">
              ApexMediation SSAI supports SSAI workflows with Widevine, FairPlay, and PlayReady 
              when your streams are packaged in compatible formats (e.g., CMAF) and DRM licenses/policies 
              are provided. Policies cover clear vs encrypted ads, key continuity, and mixed-scheme handling.
            </p>
          </div>

          {/* Supported Systems */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Shield className="w-6 h-6 text-emerald-600" />
              Supported DRM Systems
            </h2>
            
            <p className="text-gray-600 mb-4">
              The <code>DRMPolicyHandler</code> manages content protection policies across 
              all major DRM schemes.
            </p>

            <div className="overflow-x-auto mb-6">
              <table className="w-full min-w-full text-sm border-collapse">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">DRM Scheme</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Platforms</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Container</th>
                    <th className="text-left p-3 font-semibold text-gray-900">Protocol</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900 font-medium">Widevine</td>
                    <td className="p-3 text-gray-600">Chrome, Firefox, Android, Android TV</td>
                    <td className="p-3 text-gray-600">CMAF</td>
                    <td className="p-3 text-gray-600">DASH, HLS</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900 font-medium">FairPlay</td>
                    <td className="p-3 text-gray-600">Safari, iOS, tvOS, macOS</td>
                    <td className="p-3 text-gray-600">CMAF, TS</td>
                    <td className="p-3 text-gray-600">HLS</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900 font-medium">PlayReady</td>
                    <td className="p-3 text-gray-600">Edge, Xbox, Windows, Smart TVs</td>
                    <td className="p-3 text-gray-600">CMAF</td>
                    <td className="p-3 text-gray-600">DASH, HLS</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900 font-medium">ClearKey</td>
                    <td className="p-3 text-gray-600">All modern browsers</td>
                    <td className="p-3 text-gray-600">CMAF</td>
                    <td className="p-3 text-gray-600">DASH, HLS</td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div className="bg-gray-50 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-3">DRM Type Definition</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`// From DRMPolicyHandler.ts
export type DRMScheme = 'widevine' | 'fairplay' | 'playready' | 'clearkey' | 'none';

export interface DRMInfo {
  scheme: DRMScheme;
  keyId?: string;
  licenseUrl?: string;
  initData?: string;
  isEncrypted: boolean;
}`}
              </pre>
            </div>
          </section>

          {/* Policy Configuration */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Settings className="w-6 h-6 text-emerald-600" />
              Policy Configuration
            </h2>
            
            <p className="text-gray-600 mb-4">
              DRM policy is configurable per-integration to match your security requirements 
              and content licensing terms.
            </p>

            <div className="bg-gray-50 rounded-lg p-6 mb-6">
              <h3 className="font-semibold text-gray-900 mb-3">Default Configuration</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`// DRMPolicyConfig with defaults
{
  allowClearAdsInEncryptedContent: true,   // Clear ads OK in DRM content
  allowEncryptedAdsInClearContent: false,  // No encrypted ads in clear
  requireSameScheme: false,                // Mix schemes allowed
  requireKeyContinuity: true,              // Key changes tracked
  supportedSchemes: ['widevine', 'fairplay', 'playready', 'clearkey', 'none'],
  defaultScheme: 'none'
}`}
              </pre>
            </div>

            <div className="grid md:grid-cols-2 gap-6">
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <CheckCircle className="w-5 h-5 text-emerald-600" />
                  allowClearAdsInEncryptedContent
                </h3>
                <p className="text-sm text-gray-600 mb-2">
                  <strong>Default: true</strong>
                </p>
                <p className="text-sm text-gray-600">
                  When enabled, clear (unencrypted) ads can be inserted into DRM-protected 
                  content. This is common for programmatic ads which typically don&apos;t 
                  require encryption.
                </p>
              </div>
              
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <AlertTriangle className="w-5 h-5 text-amber-600" />
                  allowEncryptedAdsInClearContent
                </h3>
                <p className="text-sm text-gray-600 mb-2">
                  <strong>Default: false</strong>
                </p>
                <p className="text-sm text-gray-600">
                  When disabled (default), encrypted ads cannot be inserted into clear content. 
                  This prevents DRM licensing issues with unprotected content.
                </p>
              </div>
              
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <Lock className="w-5 h-5 text-emerald-600" />
                  requireSameScheme
                </h3>
                <p className="text-sm text-gray-600 mb-2">
                  <strong>Default: false</strong>
                </p>
                <p className="text-sm text-gray-600">
                  When enabled, encrypted content and ads must use the same DRM scheme 
                  (e.g., both Widevine). Useful for strict licensing requirements.
                </p>
              </div>
              
              <div className="border rounded-lg p-6">
                <h3 className="font-semibold text-gray-900 mb-3 flex items-center gap-2">
                  <Key className="w-5 h-5 text-emerald-600" />
                  requireKeyContinuity
                </h3>
                <p className="text-sm text-gray-600 mb-2">
                  <strong>Default: true</strong>
                </p>
                <p className="text-sm text-gray-600">
                  When enabled, key changes at splice boundaries are tracked and warnings 
                  generated. Key ID changes may cause decryption delays on some players.
                </p>
              </div>
            </div>
          </section>

          {/* Compatibility Validation */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <CheckCircle className="w-6 h-6 text-emerald-600" />
              Compatibility Validation
            </h2>
            
            <p className="text-gray-600 mb-4">
              Before inserting ads, the system validates DRM compatibility between content 
              and ad creatives.
            </p>

            <div className="bg-gray-50 rounded-lg p-6 mb-6">
              <h3 className="font-semibold text-gray-900 mb-3">Validation Result Interface</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`export interface DRMValidationResult {
  valid: boolean;       // No errors found
  compatible: boolean;  // Can be stitched
  warnings: string[];   // Non-blocking issues
  errors: string[];     // Blocking issues
  recommendation?: string; // Suggested fix
}`}
              </pre>
            </div>

            <h3 className="font-semibold text-gray-900 mb-3">Validation Checks</h3>
            <div className="space-y-3">
              <div className="flex items-start gap-3 p-4 bg-emerald-50 rounded-lg">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium text-gray-900">Scheme Support</p>
                  <p className="text-sm text-gray-600">
                    Validates both content and ad DRM schemes are in <code>supportedSchemes</code>
                  </p>
                </div>
              </div>
              
              <div className="flex items-start gap-3 p-4 bg-emerald-50 rounded-lg">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium text-gray-900">Clear/Encrypted Mixing</p>
                  <p className="text-sm text-gray-600">
                    Checks if clear ads in encrypted content (or vice versa) are allowed by policy
                  </p>
                </div>
              </div>
              
              <div className="flex items-start gap-3 p-4 bg-emerald-50 rounded-lg">
                <CheckCircle className="w-5 h-5 text-emerald-600 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium text-gray-900">Scheme Matching</p>
                  <p className="text-sm text-gray-600">
                    If <code>requireSameScheme</code> is enabled, verifies both use the same DRM
                  </p>
                </div>
              </div>
            </div>
          </section>

          {/* Key Continuity */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4 flex items-center gap-2">
              <Key className="w-6 h-6 text-emerald-600" />
              Key Continuity
            </h2>
            
            <p className="text-gray-600 mb-4">
              Key continuity refers to maintaining consistent encryption keys across splice 
              boundaries to prevent decryption stalls.
            </p>

            <div className="bg-gray-50 rounded-lg p-6 mb-6">
              <h3 className="font-semibold text-gray-900 mb-3">Key Continuity Check</h3>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`// From DRMPolicyHandler.checkKeyContinuity()
checkKeyContinuity(contentDrm: DRMInfo, adDrm: DRMInfo): {
  continuous: boolean;
  issue?: string;
}

// Cases:
// 1. Both clear → continuous: true
// 2. Mixed clear/encrypted → continuous: false
//    issue: "Key continuity broken: clear/encrypted boundary"
// 3. Both encrypted, same keyId → continuous: true
// 4. Both encrypted, different keyId → continuous: false
//    issue: "Key ID change: abc123 → def456"`}
              </pre>
            </div>

            <div className="bg-amber-50 border border-amber-200 rounded-lg p-6">
              <h3 className="font-semibold text-gray-900 mb-2 flex items-center gap-2">
                <AlertTriangle className="w-5 h-5 text-amber-600" />
                Key Rotation Handling
              </h3>
              <p className="text-sm text-gray-600 mb-3">
                When key rotation is detected during ad insertion, the system tracks:
              </p>
              <pre className="bg-gray-900 rounded-lg p-4 text-sm text-gray-300 overflow-x-auto">
{`export interface KeyRotationInfo {
  currentKeyId: string;
  nextKeyId?: string;
  rotationTime?: number;  // Position in stream
  requiresRekey: boolean;
}`}
              </pre>
            </div>
          </section>

          {/* Best Practices */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">Best Practices</h2>
            
            <div className="space-y-4">
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Use Clear Ads by Default</h3>
                <p className="text-gray-600 text-sm">
                  Most programmatic ad sources deliver clear (unencrypted) creatives. The default 
                  configuration allows this while still protecting premium content with DRM.
                </p>
              </div>
              
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Match Container Formats</h3>
                <p className="text-gray-600 text-sm">
                  DRM only works with CMAF containers. If using DRM, ensure both content and ads 
                  are packaged as CMAF/fMP4. TS containers have limited DRM support (FairPlay only).
                </p>
              </div>
              
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Monitor Key Changes</h3>
                <p className="text-gray-600 text-sm">
                  Enable key continuity tracking to identify ads that cause decryption delays. 
                  Key changes at splice boundaries may cause player stalls on some devices.
                </p>
              </div>
              
              <div className="border-l-4 border-emerald-500 pl-4">
                <h3 className="font-semibold text-gray-900">Test Multi-DRM Scenarios</h3>
                <p className="text-gray-600 text-sm">
                  If serving to multiple platforms, test DRM on each: Widevine for Chrome/Android, 
                  FairPlay for Safari/iOS/tvOS, PlayReady for Edge/Xbox.
                </p>
              </div>
            </div>
          </section>

          {/* Platform Coverage */}
          <section className="mb-12">
            <h2 className="text-2xl font-bold text-gray-900 mb-4">Platform DRM Coverage</h2>
            
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="text-left p-3 font-semibold text-gray-900">Platform</th>
                    <th className="text-center p-3 font-semibold text-gray-900">Widevine</th>
                    <th className="text-center p-3 font-semibold text-gray-900">FairPlay</th>
                    <th className="text-center p-3 font-semibold text-gray-900">PlayReady</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200">
                  <tr>
                    <td className="p-3 text-gray-900">Chrome/Firefox</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Safari/macOS</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">iOS/tvOS</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Android/Android TV</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Edge/Windows</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Xbox</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Fire TV</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                  <tr>
                    <td className="p-3 text-gray-900">Roku</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                    <td className="p-3 text-center text-gray-400">—</td>
                    <td className="p-3 text-center text-emerald-600">✓</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          {/* Related Documentation */}
          <section className="bg-emerald-50 rounded-lg p-8">
            <h2 className="text-xl font-bold text-gray-900 mb-4">Related Documentation</h2>
            <div className="grid md:grid-cols-2 gap-4">
              <Link href="/docs/ssai/containers" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">Container Formats →</h3>
                <p className="text-sm text-gray-600">TS vs CMAF for DRM content</p>
              </Link>
              <Link href="/docs/ssai/protocols/dash" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">DASH Protocol →</h3>
                <p className="text-sm text-gray-600">ContentProtection elements in MPD</p>
              </Link>
              <Link href="/docs/ssai/protocols/hls" className="bg-white p-4 rounded-lg border border-emerald-200 hover:border-emerald-400 transition-colors">
                <h3 className="font-semibold text-gray-900">HLS Protocol →</h3>
                <p className="text-sm text-gray-600">EXT-X-KEY and EXT-X-SESSION-KEY</p>
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
