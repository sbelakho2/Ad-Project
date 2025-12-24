/** @type {import('next').NextConfig} */
const ignoreBuildErrors = process.env.NEXT_IGNORE_TS_ERRORS === '1'
const nextConfig = {
  reactStrictMode: true,
  swcMinify: true,
  images: {
    // Allow images from local dev and production domains
    domains: ['localhost', 'apexmediation.ee', 'console.apexmediation.ee'],
  },
  output: 'standalone',
  compress: false, // Disable compression to fix Playwright ERR_CONTENT_DECODING_FAILED
  // Allow gating typechecking/ESLint during container builds for infra verification (Phase 8)
  typescript: {
    ignoreBuildErrors,
  },
  eslint: {
    ignoreDuringBuilds: ignoreBuildErrors,
  },
}

module.exports = nextConfig
