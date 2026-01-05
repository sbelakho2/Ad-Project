'use client'

import { useEffect } from 'react'
import { useRouter } from 'next/navigation'
import Link from 'next/link'
import {
  ShieldAlert,
  ShieldCheck,
  CreditCard,
  Users,
  Globe,
  BellRing,
  ArrowRight,
  Key,
  Hash,
  Shield,
  Receipt,
} from 'lucide-react'
import { hasRole, type Role } from '@/lib/rbac'
import { useSession } from '@/lib/useSession'

const sections = [
  {
    title: 'Security & 2FA',
    description: 'Manage two-factor authentication and account security.',
    href: '/settings/security',
    icon: ShieldCheck,
  },
  {
    title: 'SDK API Keys',
    description: 'Manage per-app API keys for SDK authentication.',
    href: '/settings/keys',
    icon: Shield,
  },
  {
    title: 'Fraud Protection',
    description: 'Configure thresholds, alerts, and automated blocking rules.',
    href: '/settings/fraud',
    icon: ShieldAlert,
  },
  {
    title: 'Billing & Payments',
    description: 'Manage your Stripe payment method and billing preferences.',
    href: '/settings/billing',
    icon: CreditCard,
  },
  {
    title: 'Usage & Invoices',
    description: 'View platform usage, invoices, and billing history.',
    href: '/settings/billing/usage',
    icon: Receipt,
  },
  {
    title: 'Network Credentials',
    description: 'Store ad network credentials for automatic revenue tracking.',
    href: '/settings/credentials',
    icon: Key,
  },
  {
    title: 'Team Access',
    description: 'Invite teammates and assign roles for collaboration.',
    href: '/settings/team',
    icon: Users,
  },
  {
    title: 'Notifications',
    description: 'Control email, Slack, and webhook alerts for key events.',
    href: '/settings/notifications',
    icon: BellRing,
  },
  {
    title: 'Regional Compliance',
    description: 'Set consent requirements and regional privacy enforcement.',
    href: '/settings/compliance',
    icon: Globe,
  },
  {
    title: 'SDK Configuration',
    description: 'Verify SDK config parity and debug configuration sync issues.',
    href: '/settings/sdk',
    icon: Hash,
  },
]

const SETTINGS_ROUTE = '/settings'
const ALLOWED_ROLES: Role[] = ['admin', 'publisher']

export default function SettingsPage() {
  const router = useRouter()
  const { user, isLoading } = useSession({ redirectOnUnauthorized: true })

  useEffect(() => {
    if (isLoading) return

    if (!user) {
      router.replace(`/login?next=${encodeURIComponent(SETTINGS_ROUTE)}`)
      return
    }

    const role = user.role ?? null
    const publisherId = user.publisherId ?? null
    if (!hasRole(role, ALLOWED_ROLES) || !publisherId) {
      router.replace('/403')
    }
  }, [isLoading, router, user])

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-6xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <p className="text-sm font-medium text-primary-600">Control Center</p>
          <h1 className="text-3xl font-semibold text-gray-900 tracking-tight">Settings</h1>
          <p className="text-sm text-gray-600 mt-1 max-w-3xl">
            Configure fraud defenses, billing preferences, notifications, and compliance policies to tailor the platform to your organization.
          </p>
        </div>
      </header>

      <main className="max-w-6xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-6">
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          {sections.map((section) => (
            <Link key={section.href} href={section.href} className="card group h-full">
              <div className="flex items-start gap-4">
                <div className="h-12 w-12 rounded-lg bg-primary-50 text-primary-600 flex items-center justify-center">
                  <section.icon className="h-6 w-6" aria-hidden={true} />
                </div>
                <div>
                  <h2 className="text-xl font-semibold text-gray-900 flex items-center gap-2">
                    {section.title}
                    <ArrowRight className="h-4 w-4 text-primary-400 opacity-0 transition-opacity duration-200 group-hover:opacity-100" aria-hidden={true} />
                  </h2>
                  <p className="text-sm text-gray-600 mt-2">{section.description}</p>
                </div>
              </div>
            </Link>
          ))}
        </div>
      </main>
    </div>
  )
}
