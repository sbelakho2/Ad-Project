'use client'

import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { ChevronRight, Home } from 'lucide-react'
import { t } from '@/i18n'

export interface BreadcrumbItem {
  label: string
  href: string
}

export interface BreadcrumbsProps {
  items?: BreadcrumbItem[]
  className?: string
}

export function Breadcrumbs({ items, className = '' }: BreadcrumbsProps) {
  const pathname = usePathname()

  // Auto-generate breadcrumbs from pathname if not provided
  const breadcrumbs = items || buildBreadcrumbsFromPath(pathname)

  if (breadcrumbs.length === 0) {
    return null
  }

  return (
    <nav aria-label="Breadcrumb" className={`flex items-center space-x-2 text-sm ${className}`}>
      <Link
        href="/"
        className="text-gray-500 hover:text-gray-700 transition-colors"
        aria-label="Home"
      >
        <Home className="h-4 w-4" />
      </Link>

      {breadcrumbs.map((item, index) => {
        const isLast = index === breadcrumbs.length - 1

        return (
          <div key={item.href} className="flex items-center space-x-2">
            <ChevronRight className="h-4 w-4 text-gray-400" aria-hidden="true" />
            {isLast ? (
              <span className="font-medium text-gray-900" aria-current="page">
                {item.label}
              </span>
            ) : (
              <Link
                href={item.href}
                className="text-gray-500 hover:text-gray-700 transition-colors"
              >
                {item.label}
              </Link>
            )}
          </div>
        )
      })}
    </nav>
  )
}

/**
 * Generate breadcrumbs from pathname
 * Examples:
 *   /billing/usage -> [{ label: 'Billing', href: '/billing' }, { label: 'Usage', href: '/billing/usage' }]
 *   /billing/invoices/123 -> [{ label: 'Billing', href: '/billing' }, { label: 'Invoices', href: '/billing/invoices' }, { label: 'Invoice #123…', href: '/billing/invoices/123' }]
 */
export function buildBreadcrumbsFromPath(pathname: string): BreadcrumbItem[] {
  if (!pathname || pathname === '/') {
    return []
  }

  const segments = pathname.split('/').filter(Boolean)
  const breadcrumbs: BreadcrumbItem[] = []

  let currentPath = ''
  for (let i = 0; i < segments.length; i++) {
    const segment = segments[i]
    currentPath += `/${segment}`

    const label = resolveSegmentLabel({
      segment,
      index: i,
      segments,
    })

    breadcrumbs.push({
      label,
      href: currentPath,
    })
  }

  return breadcrumbs
}

const FRIENDLY_SEGMENT_KEYS: Record<string, string> = {
  dashboard: 'breadcrumbs.labels.dashboard',
  placements: 'breadcrumbs.labels.placements',
  adapters: 'breadcrumbs.labels.adapters',
  analytics: 'breadcrumbs.labels.analytics',
  fraud: 'breadcrumbs.labels.fraud',
  settings: 'breadcrumbs.labels.settings',
  team: 'breadcrumbs.labels.settingsTeam',
  compliance: 'breadcrumbs.labels.settingsCompliance',
  notifications: 'breadcrumbs.labels.settingsNotifications',
  billing: 'breadcrumbs.labels.settingsBilling',
  usage: 'breadcrumbs.labels.billingUsage',
  invoices: 'breadcrumbs.labels.billingInvoices',
  transparency: 'breadcrumbs.labels.transparency',
  auctions: 'breadcrumbs.labels.transparencyAuctions',
  summary: 'breadcrumbs.labels.transparencySummary',
  'migration-studio': 'breadcrumbs.labels.migrationStudio',
  admin: 'breadcrumbs.labels.admin',
  health: 'breadcrumbs.labels.adminHealth',
  'value-multipliers': 'breadcrumbs.labels.adminValueMultipliers',
  'sales-automation': 'breadcrumbs.labels.adminSalesAutomation',
}

const IDENTIFIER_PARENT_KEYS: Record<string, string> = {
  invoices: 'breadcrumbs.identifiers.invoice',
  auctions: 'breadcrumbs.identifiers.auction',
  placements: 'breadcrumbs.identifiers.placement',
  adapters: 'breadcrumbs.identifiers.adapter',
  team: 'breadcrumbs.identifiers.member',
}

const UUID_REGEX = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const GENERIC_ID_REGEX = /^[a-z0-9_-]{8,}$/i

type SegmentResolutionInput = {
  segment: string
  index: number
  segments: string[]
}

function resolveSegmentLabel({ segment, index, segments }: SegmentResolutionInput): string {
  const normalized = segment.toLowerCase()
  const translationKey = FRIENDLY_SEGMENT_KEYS[normalized]
  if (translationKey) {
    const translated = t(translationKey)
    if (translated !== translationKey) {
      return translated
    }
  }

  const prevSegment = index > 0 ? segments[index - 1].toLowerCase() : null
  if (looksLikeIdentifier(segment)) {
    return formatIdentifierLabel(segment, prevSegment)
  }

  return humanizeSegment(segment)
}

function looksLikeIdentifier(segment: string): boolean {
  if (!segment) return false
  if (UUID_REGEX.test(segment)) return true
  return GENERIC_ID_REGEX.test(segment) && /\d/.test(segment)
}

function formatIdentifierLabel(segment: string, parentSegment: string | null): string {
  const masked = maskIdentifier(segment)
  if (parentSegment) {
    const identifierKey = IDENTIFIER_PARENT_KEYS[parentSegment]
    if (identifierKey) {
      const translated = t(identifierKey, { id: masked })
      if (translated !== identifierKey) {
        return translated
      }
    }
  }
  const fallback = t('breadcrumbs.identifiers.record', { id: masked })
  return fallback === 'breadcrumbs.identifiers.record' ? masked : fallback
}

function maskIdentifier(value: string): string {
  if (!value) return ''
  const safeValue = value.trim()
  if (safeValue.length <= 8) {
    return safeValue
  }
  return `${safeValue.slice(0, 8)}…`
}

function humanizeSegment(segment: string): string {
  if (!segment) return ''
  const words = segment
    .split(/[-_]/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
  return words.join(' ')
}
