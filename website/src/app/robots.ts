import type { MetadataRoute } from 'next'

function getBaseUrl() {
  const env = process.env.NEXT_PUBLIC_SITE_URL || process.env.SITE_URL
  if (env) return env.replace(/\/$/, '')
  // Default to https in production if host header is available at runtime; fallback localhost
  return process.env.NODE_ENV === 'production' ? 'https://apexmediation.ee' : 'http://localhost:3000'
}

export default function robots(): MetadataRoute.Robots {
  const base = getBaseUrl()
  const isProd = process.env.NODE_ENV === 'production'
  const isStaging = /staging|preview/i.test(base)

  const commonDisallows = [
    '/dashboard',
    '/api/internal',
    '/api/auth',
  ]

  const rules: MetadataRoute.Robots['rules'] = []

  if (!isProd || isStaging) {
    rules.push({ userAgent: '*', disallow: '/' })
  } else {
    rules.push({ userAgent: '*', allow: '/', disallow: commonDisallows })
  }

  return {
    rules,
    sitemap: `${base}/sitemap.xml`,
    host: base,
  }
}
