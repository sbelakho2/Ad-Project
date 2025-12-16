import type { MetadataRoute } from 'next'

/**
 * Next.js app router sitemap generator.
 * Ensures About and Contact are included alongside core marketing routes.
 */
export default function sitemap(): MetadataRoute.Sitemap {
  // Prefer explicit env; otherwise default to production canonical host
  const base = process.env.NEXT_PUBLIC_SITE_URL?.replace(/\/$/, '') || 'https://apexmediation.ee'
  const now = new Date().toISOString()

  // Expand coverage to include quiz and other key marketing routes
  const routes = [
    '/',
    '/pricing',
    '/docs',
    '/about',
    '/contact',
    '/quiz',
    '/privacy',
    '/terms',
    '/cookies',
    '/gdpr',
    '/dpa',
  ]

  return routes.map((path) => ({
    url: `${base}${path}`,
    lastModified: now,
    changeFrequency: 'weekly',
    priority: path === '/' ? 1 : 0.7,
  }))
}
