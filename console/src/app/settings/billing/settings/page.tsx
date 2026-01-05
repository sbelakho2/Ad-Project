'use client'

import { FormEvent, useEffect, useState } from 'react'
import Link from 'next/link'
import { useMutation, useQuery } from '@tanstack/react-query'
import { apiClient } from '@/lib/api-client'
import { useSession } from '@/lib/useSession'
import {
  CreditCard,
  FileText,
  ExternalLink,
  AlertCircle,
  CheckCircle2,
  Loader2,
  Settings as SettingsIcon,
  GitCompare,
} from 'lucide-react'
import { isFeatureEnabled } from '@/lib/featureFlags'

interface BillingPolicy {
  summary: string
  starterExperience: {
    paymentMethodRequired: boolean
    revenueCapUsd: number
    capabilities: string[]
    messaging: string
  }
  upgradePath: {
    triggers: string[]
    acceptedPaymentMethods: {
      id: string
      label: string
      regions: string[]
      autopayEligible: boolean
      enterpriseOnly?: boolean
    }[]
    autopay: {
      defaultBehavior: string
      encouragement: string
      enterpriseOverride: string
    }
  }
  billingCycle: {
    cadence: string
    notifications: {
      channel: string
      timing: string
      content: string
    }[]
  }
  transparencyCommitments: {
    weDontTouchPayouts: string
    platformFeeOnly: string
  }
}

const formatUsd = (value: number) =>
  new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 0 }).format(value)

const MESSAGE_STORAGE_KEY = 'billing-settings:toast'
const MIGRATION_NOTES_MIN_LENGTH = 20

export default function BillingSettingsPage() {
  const { user, isLoading: sessionLoading } = useSession({ redirectOnUnauthorized: true })

  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)
  const [migrationNotes, setMigrationNotes] = useState('')
  const [migrationChannel, setMigrationChannel] = useState<'sandbox' | 'production'>('sandbox')
  const dismissMessage = () => setMessage(null)
  const billingMigrationEnabled = isFeatureEnabled('billingMigration')

  useEffect(() => {
    if (typeof window === 'undefined') return
    try {
      const cached = window.sessionStorage.getItem(MESSAGE_STORAGE_KEY)
      if (cached) {
        const parsed = JSON.parse(cached)
        if (parsed?.type && parsed?.text) {
          setMessage(parsed)
        }
      }
    } catch {
      // ignore malformed cache
    }
  }, [])

  useEffect(() => {
    if (typeof window === 'undefined') return
    if (message) {
      window.sessionStorage.setItem(MESSAGE_STORAGE_KEY, JSON.stringify(message))
    } else {
      window.sessionStorage.removeItem(MESSAGE_STORAGE_KEY)
    }
  }, [message])

  const { data: billingPolicy } = useQuery<BillingPolicy>({
    queryKey: ['billing', 'policy'],
    queryFn: async () => {
      const response = await apiClient.get('/billing/policy')
      const payload: any = response.data
      return payload?.data ?? payload
    },
  })

  // Create Stripe Portal session
  const portalMutation = useMutation({
    mutationFn: async () => {
      const response = await apiClient.post('/billing/portal')
      return (response.data as any)?.url
    },
    onSuccess: (url: string) => {
      window.location.href = url
    },
    onError: () => {
      setMessage({ type: 'error', text: 'Failed to open Stripe Portal' })
    },
  })

  const migrationMutation = useMutation({
    mutationFn: async (payload: { channel: string; notes: string }) => {
      await apiClient.post('/billing/migration/request', payload)
    },
    onSuccess: () => {
      setMessage({ type: 'success', text: 'Migration request submitted. Our team will follow up shortly.' })
      setMigrationNotes('')
    },
    onError: () => {
      setMessage({ type: 'error', text: 'Failed to submit migration request' })
    },
  })

  const trimmedMigrationNotes = migrationNotes.trim()
  const migrationSubmitDisabled =
    migrationMutation.isPending || trimmedMigrationNotes.length < MIGRATION_NOTES_MIN_LENGTH

  const handleMigrationSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (trimmedMigrationNotes.length < MIGRATION_NOTES_MIN_LENGTH) {
      setMessage({
        type: 'error',
        text: `Add at least ${MIGRATION_NOTES_MIN_LENGTH} characters of migration context before submitting.`,
      })
      return
    }
    migrationMutation.mutate({ channel: migrationChannel, notes: trimmedMigrationNotes })
  }

  if (!sessionLoading && !user) {
    return <UnauthorizedState />
  }

  // Minimal billing settings UI.
  // The backend currently supports /billing/policy, /billing/portal, /billing/invoices, /billing/usage/current,
  // but NOT the legacy /billing/settings* endpoints.
  if (sessionLoading) {
    return (
      <div className="min-h-screen bg-gray-50 flex items-center justify-center">
        <Loader2 className="w-8 h-8 animate-spin text-primary-600" />
      </div>
    )
  }

  const policySummary = billingPolicy?.summary || 'Stripe-first billing with transparent platform fees.'
  const starterRevenueCapUsd = billingPolicy?.starterExperience?.revenueCapUsd

  return (
    <div className="bg-gray-50 min-h-screen">
      <header className="bg-white border-b border-gray-200">
        <div className="max-w-4xl mx-auto px-6 py-6">
          <div className="flex items-center gap-3">
            <div className="h-12 w-12 rounded-lg bg-primary-100 flex items-center justify-center">
              <SettingsIcon className="h-6 w-6 text-primary-600" aria-hidden="true" />
            </div>
            <div>
              <h1 className="text-2xl font-bold text-gray-900">Billing</h1>
              <p className="text-sm text-gray-600">Manage invoices and payment methods.</p>
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-4xl mx-auto px-6 py-8 space-y-6">
        {message && (
          <div
            className={`rounded-lg border px-4 py-3 flex items-start gap-3 ${
              message.type === 'success'
                ? 'bg-green-50 border-green-200 text-green-900'
                : 'bg-red-50 border-red-200 text-red-900'
            }`}
          >
            {message.type === 'success' ? (
              <CheckCircle2 className="h-5 w-5 mt-0.5" aria-hidden={true} />
            ) : (
              <AlertCircle className="h-5 w-5 mt-0.5" aria-hidden={true} />
            )}
            <div className="flex-1">
              <p className="text-sm">{message.text}</p>
            </div>
            <button
              type="button"
              onClick={dismissMessage}
              className="text-sm font-medium text-gray-700 hover:text-gray-900"
            >
              Dismiss
            </button>
          </div>
        )}

        <section className="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
          <div className="flex items-start justify-between gap-6">
            <div>
              <h2 className="text-lg font-semibold text-gray-900">Billing policy</h2>
              <p className="text-sm text-gray-600 mt-1">{policySummary}</p>
              {typeof starterRevenueCapUsd === 'number' && (
                <p className="text-sm text-gray-600 mt-3">
                  Starter revenue cap: <span className="font-medium text-gray-900">{formatUsd(starterRevenueCapUsd)}</span>
                </p>
              )}
            </div>
          </div>
        </section>

        <section className="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
          <div className="flex items-start justify-between mb-4">
            <div>
              <h2 className="text-lg font-semibold text-gray-900 mb-1">Invoices & payments</h2>
              <p className="text-sm text-gray-600">Open Stripe Portal for payment methods and invoices.</p>
            </div>
            <CreditCard className="h-6 w-6 text-gray-400" aria-hidden={true} />
          </div>

          <div className="pt-4 border-t border-gray-100 flex flex-wrap gap-3">
            <button
              type="button"
              onClick={() => portalMutation.mutate()}
              disabled={portalMutation.isPending || !user}
              className="inline-flex items-center gap-2 px-4 py-2 bg-primary-600 text-white text-sm font-medium rounded-lg hover:bg-primary-700 disabled:opacity-50 disabled:cursor-not-allowed transition"
            >
              {portalMutation.isPending ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  Opening Portal...
                </>
              ) : (
                <>
                  <ExternalLink className="h-4 w-4" />
                  Open Stripe Portal
                </>
              )}
            </button>
            <Link
              href="/settings/billing/usage"
              className="inline-flex items-center gap-2 px-4 py-2 bg-gray-100 text-gray-900 text-sm font-medium rounded-lg hover:bg-gray-200 transition"
            >
              View usage
            </Link>
            <Link
              href="/settings/billing/invoices"
              className="inline-flex items-center gap-2 px-4 py-2 bg-gray-100 text-gray-900 text-sm font-medium rounded-lg hover:bg-gray-200 transition"
            >
              <FileText className="h-4 w-4" aria-hidden={true} />
              View invoices
            </Link>
          </div>
        </section>

        {billingMigrationEnabled && (
          <section className="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
            <div className="flex items-start justify-between mb-4">
              <div>
                <h2 className="text-lg font-semibold text-gray-900 mb-1">Billing migration assistant</h2>
                <p className="text-sm text-gray-600">Request help migrating billing between sandbox and production.</p>
              </div>
              <GitCompare className="h-6 w-6 text-gray-400" aria-hidden={true} />
            </div>

            <form onSubmit={handleMigrationSubmit} className="pt-4 border-t border-gray-100 space-y-4">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                <div>
                  <label
                    htmlFor="billing-migration-target-channel"
                    className="block text-sm font-medium text-gray-700 mb-2"
                  >
                    Target channel
                  </label>
                  <select
                    id="billing-migration-target-channel"
                    value={migrationChannel}
                    onChange={(e) => setMigrationChannel(e.target.value as any)}
                    className="w-full px-3 py-2 border border-gray-300 rounded-lg"
                  >
                    <option value="sandbox">Sandbox</option>
                    <option value="production">Production</option>
                  </select>
                </div>
                <div className="sm:col-span-2">
                  <label htmlFor="billing-migration-notes" className="block text-sm font-medium text-gray-700 mb-2">
                    Notes
                  </label>
                  <textarea
                    id="billing-migration-notes"
                    value={migrationNotes}
                    onChange={(e) => setMigrationNotes(e.target.value)}
                    rows={4}
                    className="w-full px-3 py-2 border border-gray-300 rounded-lg"
                    placeholder="Describe what you’re migrating and any constraints…"
                  />
                  <p className="text-xs text-gray-500 mt-1">Minimum {MIGRATION_NOTES_MIN_LENGTH} characters.</p>
                </div>
              </div>

              <button
                type="submit"
                className="inline-flex items-center gap-2 px-4 py-2 bg-primary-600 text-white text-sm font-medium rounded-lg hover:bg-primary-700 disabled:opacity-50 disabled:cursor-not-allowed transition"
                disabled={!user || migrationSubmitDisabled}
              >
                {migrationMutation.isPending ? 'Submitting…' : 'Submit request'}
              </button>
            </form>
          </section>
        )}
      </main>
    </div>
  )
}

function UnauthorizedState() {
  return (
    <div className="min-h-screen bg-gray-50 flex items-center justify-center px-6">
      <div className="card max-w-md text-center space-y-4">
        <h2 className="text-xl font-semibold text-gray-900">Sign in required</h2>
        <p className="text-sm text-gray-600">
          You need to be authenticated to manage billing settings. Please sign in and try again.
        </p>
        <Link href="/login" className="btn btn-primary">
          Go to login
        </Link>
      </div>
    </div>
  )
}
