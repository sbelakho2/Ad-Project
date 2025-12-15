'use client';

import { motion } from 'framer-motion';
import {
  CheckCircle,
  Info,
  Monitor,
  Play,
  Smartphone,
  Tablet,
  Tv,
  XCircle,
} from 'lucide-react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import HomeNav from '@/components/HomeNav';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

// Certified playback matrix data
const playbackMatrix = {
  ctv: {
    label: 'CTV / OTT',
    icon: Tv,
    platforms: [
      { name: 'Roku', version: '≥ 9.4', status: 'certified', notes: 'Native RSG + SceneGraph' },
      { name: 'Fire TV', version: '≥ 5.2.8', status: 'certified', notes: 'ExoPlayer wrapper' },
      { name: 'Apple TV', version: '≥ 15.0', status: 'certified', notes: 'AVPlayer + IMA SDK' },
      { name: 'Android TV', version: '≥ API 21', status: 'certified', notes: 'ExoPlayer + IMA' },
      { name: 'Samsung Tizen', version: '≥ 4.0', status: 'certified', notes: 'AVPlay' },
      { name: 'LG webOS', version: '≥ 4.0', status: 'certified', notes: 'HTMLMediaElement' },
      { name: 'Chromecast', version: '≥ CAF v3', status: 'certified', notes: 'Cast receiver SDK' },
      { name: 'Vizio', version: 'SmartCast 3+', status: 'partial', notes: 'Web runtime only' },
      { name: 'PlayStation', version: '≥ PS4 7.0', status: 'partial', notes: 'WebMAF SDK' },
      { name: 'Xbox', version: '≥ Series S/X', status: 'partial', notes: 'UWP MediaElement' },
    ],
  },
  web: {
    label: 'Web Video',
    icon: Monitor,
    platforms: [
      { name: 'Chrome', version: '≥ 90', status: 'certified', notes: 'HLS.js + Shaka' },
      { name: 'Firefox', version: '≥ 85', status: 'certified', notes: 'HLS.js + Shaka' },
      { name: 'Safari', version: '≥ 14', status: 'certified', notes: 'Native HLS' },
      { name: 'Edge', version: '≥ 90', status: 'certified', notes: 'HLS.js + Shaka' },
      { name: 'iOS Safari', version: '≥ 14', status: 'certified', notes: 'Native HLS' },
      { name: 'Samsung Internet', version: '≥ 14', status: 'certified', notes: 'HLS.js' },
      { name: 'Opera', version: '≥ 75', status: 'certified', notes: 'HLS.js + Shaka' },
    ],
  },
  mobile: {
    label: 'Mobile Apps',
    icon: Smartphone,
    platforms: [
      { name: 'iOS Native', version: '≥ 14.0', status: 'certified', notes: 'AVPlayer + IMA SDK' },
      { name: 'Android Native', version: '≥ API 21', status: 'certified', notes: 'ExoPlayer + IMA' },
      { name: 'React Native', version: '≥ 0.68', status: 'certified', notes: 'Native bridge' },
      { name: 'Flutter', version: '≥ 3.0', status: 'certified', notes: 'Platform channels' },
      { name: 'Cordova/Ionic', version: 'Latest', status: 'partial', notes: 'WebView only' },
    ],
  },
};

const statusConfig = {
  certified: {
    icon: CheckCircle,
    color: 'text-green-600',
    bgColor: 'bg-green-100',
    label: 'Certified',
  },
  partial: {
    icon: Info,
    color: 'text-amber-600',
    bgColor: 'bg-amber-100',
    label: 'Partial Support',
  },
  unsupported: {
    icon: XCircle,
    color: 'text-red-600',
    bgColor: 'bg-red-100',
    label: 'Unsupported',
  },
};

export default function PlaybackMatrixPage() {
  return (
    <>
      <main className="relative bg-white text-gray-900">
        <div className="container relative z-10 py-6 md:py-10">
          <HomeNav />
        </div>

        {/* Hero */}
        <section className="py-16 md:py-16 bg-gradient-to-b from-gray-50 to-white">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <div className="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-brand-100 text-brand-700 text-sm font-medium mb-6">
                <Play className="w-4 h-4" />
                SSAI Compatibility
              </div>
              <h1 className="text-4xl md:text-5xl font-bold text-gray-900 leading-tight mb-6">
                Certified Playback Matrix
              </h1>
              <p className="text-xl text-gray-600">
                Our SSAI solution has been tested and certified across 20+ platforms and players. Here&apos;s the complete compatibility matrix.
              </p>
            </motion.div>
          </div>
        </section>

        {/* Legend */}
        <section className="py-8 border-b border-gray-100">
          <div className="container mx-auto px-4">
            <div className="flex flex-wrap justify-center gap-6">
              {Object.entries(statusConfig).map(([key, config]) => {
                const Icon = config.icon;
                return (
                  <div key={key} className="flex items-center gap-2">
                    <Icon className={`w-5 h-5 ${config.color}`} />
                    <span className="text-sm text-gray-600">{config.label}</span>
                  </div>
                );
              })}
            </div>
          </div>
        </section>

        {/* CTV Matrix */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="flex items-center gap-3 mb-8">
                <div className="w-12 h-12 bg-brand-100 rounded-lg flex items-center justify-center">
                  <Tv className="w-6 h-6 text-brand-600" />
                </div>
                <div>
                  <h2 className="text-2xl font-bold text-gray-900">CTV / OTT Platforms</h2>
                  <p className="text-gray-600">Living room devices and streaming platforms</p>
                </div>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="overflow-x-auto">
                <table className="w-full text-left">
                  <thead>
                    <tr className="bg-gray-50 border-b border-gray-200">
                      <th className="px-4 py-3 font-semibold text-gray-900">Platform</th>
                      <th className="px-4 py-3 font-semibold text-gray-900">Min Version</th>
                      <th className="px-4 py-3 font-semibold text-gray-900">Status</th>
                      <th className="px-4 py-3 font-semibold text-gray-900">Notes</th>
                    </tr>
                  </thead>
                  <tbody>
                    {playbackMatrix.ctv.platforms.map((platform, idx) => {
                      const status = statusConfig[platform.status as keyof typeof statusConfig];
                      const StatusIcon = status.icon;
                      return (
                        <tr key={platform.name} className={idx % 2 === 0 ? 'bg-white' : 'bg-gray-50'}>
                          <td className="px-4 py-3 font-medium text-gray-900">{platform.name}</td>
                          <td className="px-4 py-3 text-gray-600 font-mono text-sm">{platform.version}</td>
                          <td className="px-4 py-3">
                            <span className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium ${status.bgColor} ${status.color}`}>
                              <StatusIcon className="w-3.5 h-3.5" />
                              {status.label}
                            </span>
                          </td>
                          <td className="px-4 py-3 text-gray-600 text-sm">{platform.notes}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Web Matrix */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="flex items-center gap-3 mb-8">
                <div className="w-12 h-12 bg-brand-100 rounded-lg flex items-center justify-center">
                  <Monitor className="w-6 h-6 text-brand-600" />
                </div>
                <div>
                  <h2 className="text-2xl font-bold text-gray-900">Web Video</h2>
                  <p className="text-gray-600">Browser-based playback</p>
                </div>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="overflow-x-auto">
                <table className="w-full text-left bg-white rounded-xl overflow-hidden">
                  <thead>
                    <tr className="bg-gray-100 border-b border-gray-200">
                      <th className="px-4 py-3 font-semibold text-gray-900">Browser</th>
                      <th className="px-4 py-3 font-semibold text-gray-900">Min Version</th>
                      <th className="px-4 py-3 font-semibold text-gray-900">Status</th>
                      <th className="px-4 py-3 font-semibold text-gray-900">Player Support</th>
                    </tr>
                  </thead>
                  <tbody>
                    {playbackMatrix.web.platforms.map((platform, idx) => {
                      const status = statusConfig[platform.status as keyof typeof statusConfig];
                      const StatusIcon = status.icon;
                      return (
                        <tr key={platform.name} className={idx % 2 === 0 ? 'bg-white' : 'bg-gray-50'}>
                          <td className="px-4 py-3 font-medium text-gray-900">{platform.name}</td>
                          <td className="px-4 py-3 text-gray-600 font-mono text-sm">{platform.version}</td>
                          <td className="px-4 py-3">
                            <span className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium ${status.bgColor} ${status.color}`}>
                              <StatusIcon className="w-3.5 h-3.5" />
                              {status.label}
                            </span>
                          </td>
                          <td className="px-4 py-3 text-gray-600 text-sm">{platform.notes}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Mobile Matrix */}
        <section className="py-16">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="flex items-center gap-3 mb-8">
                <div className="w-12 h-12 bg-brand-100 rounded-lg flex items-center justify-center">
                  <Smartphone className="w-6 h-6 text-brand-600" />
                </div>
                <div>
                  <h2 className="text-2xl font-bold text-gray-900">Mobile Apps</h2>
                  <p className="text-gray-600">Native and hybrid mobile applications</p>
                </div>
              </motion.div>

              <motion.div {...getRevealProps(0.1)} className="overflow-x-auto">
                <table className="w-full text-left">
                  <thead>
                    <tr className="bg-gray-50 border-b border-gray-200">
                      <th className="px-4 py-3 font-semibold text-gray-900">Platform</th>
                      <th className="px-4 py-3 font-semibold text-gray-900">Min Version</th>
                      <th className="px-4 py-3 font-semibold text-gray-900">Status</th>
                      <th className="px-4 py-3 font-semibold text-gray-900">Integration</th>
                    </tr>
                  </thead>
                  <tbody>
                    {playbackMatrix.mobile.platforms.map((platform, idx) => {
                      const status = statusConfig[platform.status as keyof typeof statusConfig];
                      const StatusIcon = status.icon;
                      return (
                        <tr key={platform.name} className={idx % 2 === 0 ? 'bg-white' : 'bg-gray-50'}>
                          <td className="px-4 py-3 font-medium text-gray-900">{platform.name}</td>
                          <td className="px-4 py-3 text-gray-600 font-mono text-sm">{platform.version}</td>
                          <td className="px-4 py-3">
                            <span className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium ${status.bgColor} ${status.color}`}>
                              <StatusIcon className="w-3.5 h-3.5" />
                              {status.label}
                            </span>
                          </td>
                          <td className="px-4 py-3 text-gray-600 text-sm">{platform.notes}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </motion.div>
            </div>
          </div>
        </section>

        {/* Certification Process */}
        <section className="py-16 bg-gray-50">
          <div className="container mx-auto px-4">
            <div className="max-w-4xl mx-auto">
              <motion.div {...getRevealProps(0)} className="text-center mb-12">
                <h2 className="text-3xl font-bold text-gray-900 mb-4">
                  How We Certify
                </h2>
                <p className="text-lg text-gray-600">
                  Every platform goes through our comprehensive certification process.
                </p>
              </motion.div>

              <div className="grid md:grid-cols-4 gap-6">
                <motion.div {...getRevealProps(0.1)} className="bg-white rounded-xl p-6 border border-gray-100">
                  <div className="w-10 h-10 bg-brand-100 rounded-full flex items-center justify-center text-brand-600 font-bold mb-4">
                    1
                  </div>
                  <h3 className="font-semibold text-gray-900 mb-2">Integration</h3>
                  <p className="text-sm text-gray-600">SDK integration with platform-specific player</p>
                </motion.div>

                <motion.div {...getRevealProps(0.15)} className="bg-white rounded-xl p-6 border border-gray-100">
                  <div className="w-10 h-10 bg-brand-100 rounded-full flex items-center justify-center text-brand-600 font-bold mb-4">
                    2
                  </div>
                  <h3 className="font-semibold text-gray-900 mb-2">Automated Tests</h3>
                  <p className="text-sm text-gray-600">200+ test cases covering edge cases</p>
                </motion.div>

                <motion.div {...getRevealProps(0.2)} className="bg-white rounded-xl p-6 border border-gray-100">
                  <div className="w-10 h-10 bg-brand-100 rounded-full flex items-center justify-center text-brand-600 font-bold mb-4">
                    3
                  </div>
                  <h3 className="font-semibold text-gray-900 mb-2">Soak Testing</h3>
                  <p className="text-sm text-gray-600">24+ hour continuous playback testing</p>
                </motion.div>

                <motion.div {...getRevealProps(0.25)} className="bg-white rounded-xl p-6 border border-gray-100">
                  <div className="w-10 h-10 bg-brand-100 rounded-full flex items-center justify-center text-brand-600 font-bold mb-4">
                    4
                  </div>
                  <h3 className="font-semibold text-gray-900 mb-2">Partner Validation</h3>
                  <p className="text-sm text-gray-600">End-to-end testing with demand partners</p>
                </motion.div>
              </div>
            </div>
          </div>
        </section>

        {/* Request Certification */}
        <section className="py-16 bg-gray-900 text-white">
          <div className="container mx-auto px-4 text-center">
            <motion.div {...getRevealProps(0)} className="max-w-prose mx-auto">
              <h2 className="text-3xl md:text-4xl font-bold mb-6">
                Don&apos;t see your platform?
              </h2>
              <p className="text-lg text-gray-300 mb-8">
                We&apos;re constantly expanding our certified platforms. Contact us to request certification for your specific player or device.
              </p>
              <a
                href="mailto:partnerships@apexmediation.ee?subject=Platform%20Certification%20Request"
                className="inline-flex items-center gap-2 px-6 py-3 bg-white text-gray-900 rounded-lg font-semibold hover:bg-gray-100 transition-colors"
              >
                Request Certification
              </a>
            </motion.div>
          </div>
        </section>

        <Footer />
      </main>
      <CookieBanner />
    </>
  );
}
