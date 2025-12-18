'use client'

import { FormEvent, useEffect, useMemo, useState } from 'react'
import Link from 'next/link'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiClient } from '@/lib/api-client'
import { useSession } from '@/lib/useSession'
import {
  CreditCard,
  Mail,
  FileText,
  ExternalLink,
  AlertCircle,
  CheckCircle2,
  Loader2,
  Settings as SettingsIcon,
  GitCompare,
} from 'lucide-react'
import { isFeatureEnabled } from '@/lib/featureFlags'

interface BillingSettings {
  plan: {
    name: string
    type: 'starter' | 'growth' | 'scale' | 'enterprise'
    platform_fee_rate: number
    platform_fee_label: string
    revenue_band: string
    sample_fee_note?: string
    currency: string
    included_impressions: number
    included_api_calls: number
    included_data_transfer_gb: number
  }
  billing_email: string
  receipt_preferences: {
    send_receipts: boolean
    send_invoices: boolean
    send_usage_alerts: boolean
  }
  stripe_customer_id: string | null
}

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
const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/
const MIGRATION_NOTES_MIN_LENGTH = 20

export default function BillingSettingsPage() {
  const queryClient = useQueryClient()
  const { user, isLoading: sessionLoading } = useSession({ redirectOnUnauthorized: true })

  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)
  const [billingEmail, setBillingEmail] = useState('')
  const [emailError, setEmailError] = useState<string | null>(null)
  const [preferences, setPreferences] = useState<BillingSettings['receipt_preferences']>({
    send_receipts: true,
    send_invoices: true,
    send_usage_alerts: true,
  })
  const [initialPreferences, setInitialPreferences] = useState<BillingSettings['receipt_preferences']>({
    send_receipts: true,
    send_invoices: true,
    send_usage_alerts: true,
  })
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

  // Fetch settings
  const {
    data: settings,
    isLoading,
    isError: settingsError,
    error: settingsErrorDetails,
    refetch: refetchSettings,
  } = useQuery<BillingSettings>({
    queryKey: ['billing', 'settings'],
    queryFn: async () => {
      const response = await apiClient.get('/billing/settings')
      return response.data
    },
    enabled: !!user,
  })

  const { data: billingPolicy } = useQuery<BillingPolicy>({
    queryKey: ['billing', 'policy'],
    queryFn: async () => {
      const response = await apiClient.get('/billing/policy')
      return response.data?.data
    },
    enabled: !!user,
  })

  const preferencesDirty = useMemo(() => {
    return (
      preferences.send_receipts !== initialPreferences.send_receipts ||
      preferences.send_invoices !== initialPreferences.send_invoices ||
      preferences.send_usage_alerts !== initialPreferences.send_usage_alerts
    )
  }, [initialPreferences, preferences])

  const showLoading = sessionLoading || (!!user && isLoading)
  const settingsErrorMessage =
    settingsErrorDetails instanceof Error ? settingsErrorDetails.message : 'Failed to load billing settings.'

  const validateEmail = (value: string) => EMAIL_PATTERN.test(value.trim())
  const handleEmailSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (billingEmail === settings?.billing_email) return
    if (!validateEmail(billingEmail)) {
      setEmailError('Enter a valid billing email address')
      setMessage({ type: 'error', text: 'Enter a valid billing email before saving.' })
      return
    }
    emailMutation.mutate(billingEmail.trim())
  }
  const handlePreferencesSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!preferencesDirty) return
    preferencesMutation.mutate(preferences)
  }
  const handleResetPreferences = () => {
    setPreferences(initialPreferences)
  }

  // Update billing email
  const emailMutation = useMutation({
    mutationFn: async (email: string) => {
      await apiClient.put('/billing/settings/email', { email })
    },
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ['billing', 'settings'] })
      setMessage({ type: 'success', text: 'Billing email updated successfully' })
      setEmailError(null)
      setBillingEmail(variables)
    },
    onError: () => {
      setMessage({ type: 'error', text: 'Failed to update billing email' })
    },
  })

  // Update receipt preferences
  const preferencesMutation = useMutation({
    mutationFn: async (preferences: BillingSettings['receipt_preferences']) => {
      await apiClient.put('/billing/settings/preferences', preferences)
    },
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ['billing', 'settings'] })
      setInitialPreferences(variables)
      setMessage({ type: 'success', text: 'Receipt preferences updated' })
    },
    onError: () => {
      setMessage({ type: 'error', text: 'Failed to update preferences' })
    },
  })

  // Create Stripe Portal session
  const portalMutation = useMutation({
    mutationFn: async () => {
      const response = await apiClient.post('/billing/portal')
      return response.data.url
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
      queryClient.invalidateQueries({ queryKey: ['billing', 'settings'] })
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

  // Update local state when data loads
  useEffect(() => {
    if (settings) {
      setBillingEmail(settings.billing_email)
      setPreferences(settings.receipt_preferences)
      setInitialPreferences(settings.receipt_preferences)
    }
  }, [settings])

  if (!sessionLoading && !user) {
    return <UnauthorizedState />
  }

  if (showLoading) {
    return (
      <div className="flex items-center justify-center min-h-[400px]">
        <Loader2 className="h-8 w-8 animate-spin text-primary-600" />
      </div>
    )
  }

  if (settingsError) {
    return (
      <PageErrorState
        title="We couldn’t fetch your billing settings"
        description={settingsErrorMessage}
        onRetry={() => refetchSettings()}
      />
    )
  }

  if (!settings) {
    return (
      <PageErrorState
        title="Billing settings unavailable"
        description="We couldn’t load your billing configuration. Please try again."
        onRetry={() => refetchSettings()}
      />
    )
  }

  const disableEmailSubmit =
    emailMutation.isPending ||
    !billingEmail ||
    billingEmail.trim() === settings.billing_email ||
    !!emailError
  const preferencesSubmitDisabled = !preferencesDirty || preferencesMutation.isPending
  const isStarterPlan = settings.plan.type === 'starter'
  const autopayEligibleMethods = billingPolicy?.upgradePath?.acceptedPaymentMethods?.filter(
    (method) => method.autopayEligible && !method.enterpriseOnly
  ) ?? []
  const enterpriseOnlyMethods = billingPolicy?.upgradePath?.acceptedPaymentMethods?.filter(
    (method) => method.enterpriseOnly
  ) ?? []
  const preChargeNotification = billingPolicy?.billingCycle?.notifications?.find((notification) =>
    notification.timing.toLowerCase().includes('before')
  )
  const starterRevenueCapLabel = billingPolicy?.starterExperience
    ? formatUsd(billingPolicy.starterExperience.revenueCapUsd)
    : null
  const starterCapabilities = billingPolicy?.starterExperience?.capabilities ?? []
  const starterMessaging = billingPolicy?.starterExperience?.messaging
  const autopayDetails = billingPolicy?.upgradePath?.autopay
  const autopayDefaultBehavior =
    autopayDetails?.defaultBehavior ?? 'Stripe auto-charges the primary payment method when invoices finalize.'
  const autopayEncouragement =
    autopayDetails?.encouragement ?? 'Cards, ACH, or SEPA autopay keeps mediation running with zero manual steps.'
  const enterpriseOverrideCopy = autopayDetails?.enterpriseOverride
  const showPolicySection = Boolean(billingPolicy?.starterExperience && billingPolicy?.upgradePath)
  const showAutopayCard = Boolean(autopayDetails)
  const transparencyCopy = billingPolicy?.transparencyCommitments
  const upgradeTriggers = billingPolicy?.upgradePath?.triggers ?? []

  return (
    <div className="bg-gray-50 min-h-screen">
      {/* Header */}
      <header className="bg-white border-b border-gray-200">
        <div className="max-w-4xl mx-auto px-6 py-6">
          <div className="flex items-center gap-3">
            <div className="h-12 w-12 rounded-lg bg-primary-100 flex items-center justify-center">
              <SettingsIcon className="h-6 w-6 text-primary-600" aria-hidden="true" />
            </div>
            <div>
              <h1 className="text-2xl font-bold text-gray-900">Billing Settings</h1>
              <p className="text-sm text-gray-600">
                Manage your platform tier, payment method, and billing preferences
              </p>
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-4xl mx-auto px-6 py-8">
        {/* Status Message */}
        {message && (
          <div
            className={`mb-6 p-4 rounded-lg border flex items-start gap-3 ${
              message.type === 'success'
                ? 'bg-green-50 border-green-200 text-green-800'
                : 'bg-red-50 border-red-200 text-red-800'
            }`}
            role="alert"
          >
            {message.type === 'success' ? (
              <CheckCircle2 className="h-5 w-5 mt-0.5 flex-shrink-0" />
            ) : (
              <AlertCircle className="h-5 w-5 mt-0.5 flex-shrink-0" />
            )}
            <p className="text-sm font-medium flex-1">{message.text}</p>
            <button
              type="button"
              className="text-sm font-semibold underline"
              onClick={dismissMessage}
            >
              Dismiss
            </button>
          </div>
        )}

        <div className="space-y-6">
          <section className="bg-primary-50 border border-primary-100 text-primary-900 rounded-lg p-4" aria-live="polite">
            <h2 className="text-sm font-semibold uppercase tracking-wide">
              Need to upgrade or change your plan?
            </h2>
            <p className="text-sm mt-2">
              Start an upgrade from this page or
              <a href="mailto:billing@apexmediation.ee" className="underline font-medium mx-1" aria-label="Contact sales">
                contact sales
              </a>
              if you need custom terms. Growth, Scale, and Enterprise plans auto-charge at each billing period unless your
              account is flagged for manual invoices.
            </p>
          </section>

          <section className="bg-white rounded-lg shadow-sm border border-gray-200 p-6" aria-live="polite">
            <h2 className="text-lg font-semibold text-gray-900">What each tier unlocks</h2>
            <div className="mt-3 grid gap-4 sm:grid-cols-2">
              <div className="border border-gray-100 rounded-lg p-4">
                <p className="text-sm font-semibold text-gray-900">Starter (0% to $10k/app/mo)</p>
                <ul className="mt-2 text-sm text-gray-700 list-disc list-inside space-y-1">
                  <li>Basic revenue + latency dashboards</li>
                  <li>SDKs and Migration Studio</li>
                  <li className="text-amber-700">No Transparency auctions, VRA reconciliation, fraud dashboards, or A/B tools</li>
                </ul>
              </div>
              <div className="border border-gray-100 rounded-lg p-4">
                <p className="text-sm font-semibold text-gray-900">Growth (2.5% platform fee)</p>
                <ul className="mt-2 text-sm text-gray-700 list-disc list-inside space-y-1">
                  <li>Everything in Starter</li>
                  <li>Transparency auctions + VRA reconciliation exports</li>
                  <li>Fraud/quality dashboards and A/B experiments</li>
                  <li>Priority support and uptime SLA</li>
                </ul>
              </div>
              <div className="border border-gray-100 rounded-lg p-4">
                <p className="text-sm font-semibold text-gray-900">Scale (2.0% → 1.25% with annual prepay)</p>
                <ul className="mt-2 text-sm text-gray-700 list-disc list-inside space-y-1">
                  <li>Everything in Growth</li>
                  <li>Custom roles/RBAC and BI exports</li>
                  <li>Dedicated migration + integration support</li>
                  <li>Joint roadmap sessions</li>
                </ul>
              </div>
              <div className="border border-gray-100 rounded-lg p-4">
                <p className="text-sm font-semibold text-gray-900">Enterprise (1.0–1.5% custom)</p>
                <ul className="mt-2 text-sm text-gray-700 list-disc list-inside space-y-1">
                  <li>Everything in Scale</li>
                  <li>Custom SLAs and white-label options</li>
                  <li>Bespoke data feeds and compliance support</li>
                  <li>Invoice + wire after finance review (autopay default)</li>
                </ul>
              </div>
            </div>
          </section>

          {showPolicySection && (
            <section className="bg-white rounded-lg shadow-sm border border-primary-100 p-6" aria-live="polite">
              <div className="flex flex-col gap-2">
                <h2 className="text-lg font-semibold text-gray-900">Starter tier & upgrade policy</h2>
                <p className="text-sm text-gray-600">
                  {billingPolicy?.summary ?? 'Stripe autopay is the default once you leave Starter.'}
                </p>
              </div>
              <div className="mt-4 grid grid-cols-1 gap-4 md:grid-cols-2">
                <div className="rounded-lg border border-emerald-100 bg-emerald-50/80 p-4">
                  <p className="text-sm font-semibold text-emerald-900">Starter stays free</p>
                  <p className="text-sm text-emerald-900 mt-1">
                    {starterRevenueCapLabel
                      ? `Stay on Starter with no payment method until you cross ${starterRevenueCapLabel} in mediated revenue per month.`
                      : 'Stay on Starter with no payment method until you hit the published revenue cap.'}
                  </p>
                  <ul className="mt-3 text-sm text-emerald-900 list-disc list-inside space-y-1">
                    {starterCapabilities.map((capability) => (
                      <li key={capability}>{capability}</li>
                    ))}
                  </ul>
                  {starterMessaging && <p className="mt-3 text-xs text-emerald-800">{starterMessaging}</p>}
                </div>
                <div className="rounded-lg border border-primary-100 p-4">
                  <p className="text-sm font-semibold text-gray-900">Upgrade requirements</p>
                  <p className="text-sm text-gray-700 mt-1">{autopayDefaultBehavior}</p>
                  {autopayEligibleMethods.length > 0 && (
                    <div className="mt-3">
                      <p className="text-xs font-semibold text-gray-500 uppercase tracking-wide">Autopay rails</p>
                      <div className="flex flex-wrap gap-2 mt-2">
                        {autopayEligibleMethods.map((method) => (
                          <span
                            key={method.id}
                            className="px-2 py-1 text-xs font-medium rounded-full bg-primary-50 text-primary-700 border border-primary-100"
                          >
                            {method.label}
                          </span>
                        ))}
                      </div>
                    </div>
                  )}
                  <ul className="mt-3 text-sm text-gray-600 list-disc list-inside space-y-1">
                    {upgradeTriggers.map((trigger) => (
                      <li key={trigger}>{trigger}</li>
                    ))}
                  </ul>
                  <p className="mt-3 text-xs text-gray-500">{autopayEncouragement}</p>
                  {enterpriseOnlyMethods.length > 0 && enterpriseOverrideCopy && (
                    <p className="mt-2 text-xs text-gray-500">{enterpriseOverrideCopy}</p>
                  )}
                </div>
              </div>
            </section>
          )}
          {/* Current Platform Tier */}
          <section className="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
            <div className="flex items-start justify-between">
              <div>
                <h2 className="text-lg font-semibold text-gray-900 mb-1">Current Platform Tier</h2>
                <p className="text-sm text-gray-600 mb-4">
                  Your platform fee, revenue band, and included usage guardrails
                </p>
                {settings && (
                  <div className="space-y-3">
                    <div className="flex items-baseline gap-3">
                      <div className="flex flex-col gap-1">
                        <span className="text-3xl font-bold text-gray-900">{settings.plan.name}</span>
                        <span className="text-xl text-gray-600">{settings.plan.platform_fee_label}</span>
                      </div>
                    </div>
                    <p className="text-sm text-gray-600">{settings.plan.revenue_band}</p>
                    {settings.plan.sample_fee_note ? (
                      <p className="text-xs text-gray-500">{settings.plan.sample_fee_note}</p>
                    ) : null}
                    <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 pt-4 border-t border-gray-100">
                      <div>
                        <p className="text-xs font-medium text-gray-500 uppercase">Impressions</p>
                        <p className="text-lg font-semibold text-gray-900">
                          {(settings.plan.included_impressions / 1_000_000).toFixed(1)}M
                        </p>
                      </div>
                      <div>
                        <p className="text-xs font-medium text-gray-500 uppercase">API Calls</p>
                        <p className="text-lg font-semibold text-gray-900">
                          {(settings.plan.included_api_calls / 1_000).toFixed(0)}K
                        </p>
                      </div>
                      <div>
                        <p className="text-xs font-medium text-gray-500 uppercase">Data Transfer</p>
                        <p className="text-lg font-semibold text-gray-900">
                          {settings.plan.included_data_transfer_gb}GB
                        </p>
                      </div>
                    </div>
                  </div>
                )}
              </div>
              <Link
                href="/settings/billing/usage"
                className="px-4 py-2 text-sm font-medium text-primary-700 bg-primary-50 rounded-lg hover:bg-primary-100 transition"
              >
                View Usage
              </Link>
            </div>
          </section>

          {/* Payment Method & Stripe Portal */}
          <section className="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
            <div className="flex items-start justify-between mb-4">
              <div>
                <h2 className="text-lg font-semibold text-gray-900 mb-1">Payment Method</h2>
                <p className="text-sm text-gray-600">
                  Manage payment methods, billing address, and invoicing preferences via Stripe
                </p>
              </div>
              <CreditCard className="h-6 w-6 text-gray-400" aria-hidden="true" />
            </div>
            <div className="pt-4 border-t border-gray-100">
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
                    Manage via Stripe Portal
                  </>
                )}
              </button>
              {!settings?.stripe_customer_id && (
                <p className="text-sm text-amber-600 mt-3 flex items-start gap-2">
                  <AlertCircle className="h-4 w-4 mt-0.5 flex-shrink-0" />
                  {isStarterPlan
                    ? 'Starter tier accounts can stay card-free until they exceed the revenue cap. Add a payment method early to ensure autopay works the moment you upgrade.'
                    : 'No Stripe customer linked yet. Open the Stripe Portal to add a payment method and enable autopay.'}
                </p>
              )}
              {showAutopayCard && (
                <div className="mt-4 rounded-lg border border-primary-100 bg-primary-50/60 p-4 text-sm text-primary-900">
                  <p className="font-semibold">Automatic charges</p>
                  <p className="mt-1">{autopayDefaultBehavior}</p>
                  <p className="mt-2 text-xs text-primary-800">{autopayEncouragement}</p>
                  {preChargeNotification && (
                    <p className="mt-2 text-xs text-primary-800">{preChargeNotification.content}</p>
                  )}
                </div>
              )}
            </div>
          </section>

          {/* Billing Email */}
          <section className="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
            <div className="flex items-start justify-between mb-4">
              <div>
                <h2 className="text-lg font-semibold text-gray-900 mb-1">Billing Email</h2>
                <p className="text-sm text-gray-600">
                  Where we send invoices, receipts, and billing notifications
                </p>
              </div>
              <Mail className="h-6 w-6 text-gray-400" aria-hidden="true" />
            </div>
            <form onSubmit={handleEmailSubmit} className="pt-4 border-t border-gray-100">
              <div className="flex items-end gap-3">
                <div className="flex-1">
                  <label htmlFor="billing-email" className="block text-sm font-medium text-gray-700 mb-2">
                    Email Address
                  </label>
                  <input
                    id="billing-email"
                    type="email"
                    required
                    value={billingEmail}
                    onChange={(e) => {
                      const value = e.target.value
                      setBillingEmail(value)
                      if (emailError) {
                        setEmailError(validateEmail(value) ? null : 'Enter a valid billing email address')
                      }
                    }}
                    onBlur={() => {
                      if (billingEmail && !validateEmail(billingEmail)) {
                        setEmailError('Enter a valid billing email address')
                      } else {
                        setEmailError(null)
                      }
                    }}
                    aria-invalid={emailError ? 'true' : 'false'}
                    className="w-full px-4 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500"
                    placeholder="billing@example.com"
                  />
                  {emailError && <p className="text-sm text-red-600 mt-1">{emailError}</p>}
                </div>
                <button
                  type="submit"
                  disabled={disableEmailSubmit}
                  className="px-4 py-2 bg-primary-600 text-white text-sm font-medium rounded-lg hover:bg-primary-700 disabled:opacity-50 disabled:cursor-not-allowed transition"
                >
                  {emailMutation.isPending ? 'Updating...' : 'Update'}
                </button>
              </div>
            </form>
          </section>

          {/* Receipt Preferences */}
          <section className="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
            <div className="flex items-start justify-between mb-4">
              <div>
                <h2 className="text-lg font-semibold text-gray-900 mb-1">Email Preferences</h2>
                <p className="text-sm text-gray-600">Control which billing emails you receive</p>
              </div>
              <FileText className="h-6 w-6 text-gray-400" aria-hidden="true" />
            </div>
            <form onSubmit={handlePreferencesSubmit} className="pt-4 border-t border-gray-100 space-y-4">
              <label className="flex items-start gap-3 cursor-pointer" aria-label="Toggle payment receipt emails">
                <input
                  type="checkbox"
                  checked={preferences.send_receipts}
                  onChange={(e) =>
                    setPreferences((prev) => ({ ...prev, send_receipts: e.target.checked }))
                  }
                  className="mt-1 h-4 w-4 text-primary-600 border-gray-300 rounded focus:ring-primary-500"
                />
                <div>
                  <p className="text-sm font-medium text-gray-900">Payment Receipts</p>
                  <p className="text-xs text-gray-600">Receive confirmation emails when payments are processed</p>
                </div>
              </label>

              <label className="flex items-start gap-3 cursor-pointer" aria-label="Toggle monthly invoice emails">
                <input
                  type="checkbox"
                  checked={preferences.send_invoices}
                  onChange={(e) =>
                    setPreferences((prev) => ({ ...prev, send_invoices: e.target.checked }))
                  }
                  className="mt-1 h-4 w-4 text-primary-600 border-gray-300 rounded focus:ring-primary-500"
                />
                <div>
                  <p className="text-sm font-medium text-gray-900">Monthly Invoices</p>
                  <p className="text-xs text-gray-600">
                    Get invoices emailed automatically when they&apos;re generated
                  </p>
                </div>
              </label>

              <label className="flex items-start gap-3 cursor-pointer" aria-label="Toggle usage alert emails">
                <input
                  type="checkbox"
                  checked={preferences.send_usage_alerts}
                  onChange={(e) =>
                    setPreferences((prev) => ({ ...prev, send_usage_alerts: e.target.checked }))
                  }
                  className="mt-1 h-4 w-4 text-primary-600 border-gray-300 rounded focus:ring-primary-500"
                />
                <div>
                  <p className="text-sm font-medium text-gray-900">Usage Alerts</p>
                  <p className="text-xs text-gray-600">Notify me when approaching or exceeding tier guardrails</p>
                </div>
              </label>

              <div className="flex items-center justify-end gap-3 pt-2 border-t border-gray-100 mt-6">
                <button
                  type="button"
                  className="px-4 py-2 text-sm font-medium rounded-lg border border-gray-300 text-gray-700 hover:bg-gray-50 disabled:opacity-50 disabled:cursor-not-allowed"
                  onClick={handleResetPreferences}
                  disabled={!preferencesDirty || preferencesMutation.isPending}
                >
                  Discard Changes
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 bg-primary-600 text-white text-sm font-medium rounded-lg hover:bg-primary-700 disabled:opacity-50 disabled:cursor-not-allowed transition"
                  disabled={preferencesSubmitDisabled}
                >
                  {preferencesMutation.isPending ? 'Saving...' : 'Save Changes'}
                </button>
              </div>
            </form>
          </section>

          {/* Migration Assistant */}
          {billingMigrationEnabled && (
            <section className="bg-white rounded-lg shadow-sm border border-dashed border-purple-200 p-6">
              <div className="flex items-start justify-between mb-4">
                <div>
                  <h2 className="text-lg font-semibold text-gray-900 mb-1">Migration Assistant</h2>
                  <p className="text-sm text-gray-600">
                    Pilot tools for migrating legacy billing accounts into the unified workspace. Submit context and our
                    ops team will provision the sandbox or production pipeline you select.
                  </p>
                </div>
                <GitCompare className="h-6 w-6 text-purple-500" aria-hidden="true" />
              </div>
              <form onSubmit={handleMigrationSubmit} className="pt-4 border-t border-gray-100 space-y-4">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div>
                    <label htmlFor="migration-channel" className="block text-sm font-medium text-gray-700 mb-2">
                      Target pipeline
                    </label>
                    <select
                      id="migration-channel"
                      value={migrationChannel}
                      onChange={(event) => setMigrationChannel(event.target.value as 'sandbox' | 'production')}
                      className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-purple-500 focus:border-purple-500"
                    >
                      <option value="sandbox">Sandbox validation</option>
                      <option value="production">Production switchover</option>
                    </select>
                    <p className="text-xs text-gray-500 mt-1">
                      Sandbox migrations let you test invoices before touching live data.
                    </p>
                  </div>
                  <div>
                    <label htmlFor="migration-sla" className="block text-sm font-medium text-gray-700 mb-2">
                      Desired timeline
                    </label>
                    <select
                      id="migration-sla"
                      className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-purple-500 focus:border-purple-500"
                      defaultValue="flexible"
                      disabled
                    >
                      <option value="flexible">Flexible (default)</option>
                    </select>
                    <p className="text-xs text-gray-500 mt-1">SLA selection is coming soon.</p>
                  </div>
                </div>
                <div>
                  <label htmlFor="migration-notes" className="block text-sm font-medium text-gray-700 mb-2">
                    Migration context
                  </label>
                  <textarea
                    id="migration-notes"
                    value={migrationNotes}
                    onChange={(event) => setMigrationNotes(event.target.value)}
                    className="w-full min-h-[120px] resize-y px-3 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-purple-500 focus:border-purple-500"
                    placeholder="List existing billing IDs, expected cutover date, and any blockers."
                  />
                  <p className="text-xs text-gray-500 mt-1">
                    Share enough detail for ops to verify your request. Minimum {MIGRATION_NOTES_MIN_LENGTH} characters. Current:{' '}
                    <span data-testid="migration-notes-length">{trimmedMigrationNotes.length}</span>
                  </p>
                </div>
                <div className="flex items-center justify-end gap-3 border-t border-gray-100 pt-4">
                  <button
                    type="button"
                    className="px-4 py-2 text-sm font-medium rounded-lg border border-gray-300 text-gray-700 hover:bg-gray-50 disabled:opacity-50"
                    onClick={() => setMigrationNotes('')}
                    disabled={migrationMutation.isPending || trimmedMigrationNotes.length === 0}
                  >
                    Clear
                  </button>
                  <button
                    type="submit"
                    className="px-4 py-2 bg-purple-600 text-white text-sm font-medium rounded-lg hover:bg-purple-700 disabled:opacity-50 disabled:cursor-not-allowed"
                    disabled={migrationSubmitDisabled}
                  >
                    {migrationMutation.isPending ? 'Sending…' : 'Request migration support'}
                  </button>
                </div>
              </form>
            </section>
          )}

          {/* Help Text */}
          <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
            <div className="flex items-start gap-3">
              <AlertCircle className="h-5 w-5 text-blue-600 mt-0.5 flex-shrink-0" />
              <div className="text-sm text-blue-900">
                  <p className="font-medium mb-1">Need to adjust your platform tier?</p>
                  <p>
                    Contact our billing team at{' '}
                    <a href="mailto:billing@apexmediation.ee" className="underline font-medium">
                      billing@apexmediation.ee
                    </a>{' '}
                    to review usage, negotiate contracts, or request Enterprise onboarding support.
                  </p>
                  {transparencyCopy && (
                    <p className="mt-2 text-xs text-blue-900/80">
                      {transparencyCopy.weDontTouchPayouts} {transparencyCopy.platformFeeOnly}
                    </p>
                  )}
              </div>
            </div>
          </div>
        </div>
      </main>
    </div>
  )
}

function PageErrorState({
  title,
  description,
  onRetry,
}: {
  title: string
  description: string
  onRetry: () => void
}) {
  return (
    <div className="min-h-screen bg-gray-50 flex items-center justify-center px-6">
      <div className="card max-w-md text-center space-y-4">
        <h2 className="text-xl font-semibold text-gray-900">{title}</h2>
        <p className="text-sm text-gray-600">{description}</p>
        <button type="button" className="btn btn-primary" onClick={onRetry}>
          Retry loading settings
        </button>
      </div>
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
