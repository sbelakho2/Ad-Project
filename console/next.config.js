/** @type {import('next').NextConfig} */
const ignoreBuildErrors = process.env.NEXT_IGNORE_TS_ERRORS === '1'
const isProd = process.env.NODE_ENV === 'production'

const nextConfig = {
  reactStrictMode: true,
  swcMinify: true,
  images: {
    // Allow images from local dev and production domains
    remotePatterns: [
      { protocol: 'http', hostname: 'localhost' },
      { protocol: 'https', hostname: 'apexmediation.ee' },
      { protocol: 'https', hostname: 'console.apexmediation.ee' },
      { protocol: 'https', hostname: '*.apexmediation.ee' },
    ],
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

  // Security headers
  async headers() {
    // CSP script-src:
    // - Production: No 'unsafe-eval' to prevent XSS via eval()
    // - Development: Allow 'unsafe-eval' for hot-reload/debugging
    // Note: 'unsafe-inline' needed for Next.js hydration and Recharts inline styles
    // Future: implement nonce-based CSP via middleware for full security
    const scriptSrc = isProd
      ? ["'self'", "'unsafe-inline'"]
      : ["'self'", "'unsafe-inline'", "'unsafe-eval'"]

    const cspDirectives = [
      "default-src 'self'",
      "base-uri 'self'",
      "frame-ancestors 'self'",  // Allow embedding in same origin (dashboard iframes)
      "object-src 'none'",
      "img-src 'self' data: https:",
      "style-src 'self' 'unsafe-inline'",  // Required for Recharts, Tailwind JIT
      `script-src ${scriptSrc.join(' ')}`,
      "connect-src 'self' https://api.apexmediation.ee wss:",  // API + WebSocket
      "font-src 'self' data:",
      "form-action 'self'",
    ]

    // Upgrade to HTTPS in production
    if (isProd) {
      cspDirectives.push('upgrade-insecure-requests')
    }

    const csp = cspDirectives.join('; ')

    return [
      {
        source: '/:path*',
        headers: [
          // Security headers
          { key: 'Strict-Transport-Security', value: 'max-age=63072000; includeSubDomains; preload' },
          { key: 'X-Content-Type-Options', value: 'nosniff' },
          { key: 'X-Frame-Options', value: 'SAMEORIGIN' },  // Allow same-origin iframe embeds
          { key: 'Referrer-Policy', value: 'strict-origin-when-cross-origin' },
          { key: 'Permissions-Policy', value: 'camera=(), microphone=(), geolocation=()' },
          { key: 'Content-Security-Policy', value: csp },
        ],
      },
    ]
  },
}

module.exports = nextConfig
