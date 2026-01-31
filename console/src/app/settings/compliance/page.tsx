'use client'

import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import Link from 'next/link'
import { useForm, useWatch } from 'react-hook-form'
import { z } from 'zod'
import { zodResolver } from '@hookform/resolvers/zod'
import { useMutation, useQuery } from '@tanstack/react-query'
import { settingsApi } from '@/lib/api'
import {
  ArrowLeft,
  Globe,
  Save,
  AlertCircle,
  CheckCircle,
  Eye,
  EyeOff,
  X,
} from 'lucide-react'
import type { ComplianceSettings } from '@/types'
import { useDebouncedValue } from '@/lib/hooks'
import {
  COMMON_SENSITIVE_CATEGORY_SUGGESTIONS,
  ISO_COUNTRY_CODES,
  ISO_COUNTRY_LABELS,
  ISO_COUNTRY_SET,
  type IsoCountryCode,
} from '@/constants/geo'

type EncryptedConsentPayload =
  | {
      mode: 'encrypted'
      algorithm: 'AES-GCM'
      cipherText: string
      iv: string
      keyId: string
    }
  | {
      mode: 'fallback'
      value: string
    }

const toggleFieldNames = ['gdprEnabled', 'ccpaEnabled', 'coppaMode', 'autoBlock', 'euTrafficOnly'] as const

const slugifyCategory = (value: string) =>
  value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')

const formatCategoryLabel = (slug: string) =>
  slug
    .split('-')
    .filter(Boolean)
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
    .join(' ') || slug.toUpperCase()

/**
 * Encrypt consent value via server-side API
 * 
 * Security: The encryption key is never exposed to the client.
 * The API route at /api/consent/encrypt handles encryption server-side.
 */
const encryptConsentValue = async (value: string): Promise<EncryptedConsentPayload> => {
  if (!value) {
    return { mode: 'fallback', value: '' }
  }

  try {
    const response = await fetch('/api/consent/encrypt', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ value }),
    })

    if (!response.ok) {
      console.warn('Server-side encryption failed, using fallback')
      return { mode: 'fallback', value }
    }

    const result = await response.json()
    return result as EncryptedConsentPayload
  } catch (error) {
    console.warn('Failed to encrypt consent string via API. Falling back to plaintext.', error)
    return { mode: 'fallback', value }
  }
}

const isoCountryCodeSchema = z
  .string()
  .trim()
  .transform((value) => value.toUpperCase())
  .refine((value) => /^[A-Z]{2}$/.test(value), {
    message: 'Use ISO 3166-1 alpha-2 codes (e.g., US, DE).',
  })
  .refine((value) => ISO_COUNTRY_SET.has(value as IsoCountryCode), {
    message: 'Unknown ISO country code.',
  })

const categorySlugSchema = z
  .string()
  .trim()
  .min(2, 'Enter at least two characters.')
  .max(60, 'Category labels must be 60 characters or fewer.')
  .transform((value) => slugifyCategory(value))
  .refine((value) => value.length >= 2, { message: 'Category slugs must contain letters or numbers.' })

const complianceSchema = z.object({
  gdprEnabled: z.boolean(),
  ccpaEnabled: z.boolean(),
  coppaMode: z.boolean(),
  consentProvider: z.enum(['custom', 'onetrust', 'cookiebot']).optional().or(z.literal('')),
  autoBlock: z.boolean(),
  consentString: z.string().optional().or(z.literal('')),
  rawEventsDays: z.coerce.number().min(1).max(365),
  aggregatedDataDays: z.coerce.number().min(1).max(730),
  userDataDays: z.coerce.number().min(1).max(90),
  euTrafficOnly: z.boolean(),
  blockedCountries: z.preprocess((val) => {
    // Allow the UI to send either a comma-separated string or an array.
    if (typeof val === 'string') {
      return (val as string)
        .split(',')
        .map((c) => (c || '').trim().toUpperCase())
        .filter(Boolean)
    }
    return val
  }, z
    .array(isoCountryCodeSchema)
    .max(250, 'Limit blocked countries to 250 entries.')
    .refine((codes) => new Set(codes).size === codes.length, { message: 'Duplicate country codes are not allowed.' })),
  sensitiveCategories: z.preprocess((val) => {
    if (typeof val === 'string') {
      return (val as string)
        .split(',')
        .map((c) => slugifyCategory(c || ''))
        .filter(Boolean)
    }
    return val
  }, z
    .array(categorySlugSchema)
    .max(200, 'Limit blocked categories to 200 entries.')
    .refine((values) => new Set(values).size === values.length, { message: 'Duplicate categories are not allowed.' })),
})
type ComplianceFormValues = z.infer<typeof complianceSchema>

export default function ComplianceSettingsPage() {
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)
  const [showConsent, setShowConsent] = useState(false)

  const { data, isLoading } = useQuery({
    queryKey: ['settings', 'compliance'],
    queryFn: async () => {
      const { data } = await settingsApi.getComplianceSettings()
      return data
    },
  })

  const form = useForm<ComplianceFormValues>({
    resolver: zodResolver(complianceSchema),
    defaultValues: {
      gdprEnabled: false,
      ccpaEnabled: false,
      coppaMode: false,
      consentProvider: '',
      autoBlock: true,
      consentString: '',
      rawEventsDays: 30,
      aggregatedDataDays: 365,
      userDataDays: 30,
      euTrafficOnly: false,
      blockedCountries: [],
      sensitiveCategories: [],
    },
  })

  const watchedBlockedCountries = useWatch({ control: form.control, name: 'blockedCountries' }) as
    | string[]
    | undefined
  const watchedSensitiveCategories = useWatch({ control: form.control, name: 'sensitiveCategories' }) as
    | string[]
    | undefined
  const blockedCountryValues = useMemo(() => watchedBlockedCountries ?? [], [watchedBlockedCountries])
  const sensitiveCategoryValues = useMemo(() => watchedSensitiveCategories ?? [], [watchedSensitiveCategories])
  const toggleValues = useWatch({ control: form.control, name: toggleFieldNames as unknown as any }) as
    | boolean[]
    | undefined

  const [countryInput, setCountryInput] = useState('')
  const [categoryInput, setCategoryInput] = useState('')
  const [countryError, setCountryError] = useState<string | null>(null)
  const [categoryError, setCategoryError] = useState<string | null>(null)

  const debouncedCountryQuery = useDebouncedValue(countryInput, 150)
  const debouncedCategoryQuery = useDebouncedValue(categoryInput, 150)
  const toggleKey = useMemo(() => JSON.stringify(toggleValues ?? []), [toggleValues])
  const debouncedToggleKey = useDebouncedValue(toggleKey, 600)
  const toggleBaselineRef = useRef<string | null>(null)

  const filteredCountrySuggestions = useMemo(() => {
    const query = debouncedCountryQuery.trim()
    const available = ISO_COUNTRY_CODES.filter((code) => !blockedCountryValues.includes(code))
    if (!query) {
      return available.slice(0, 8)
    }
    const lowered = query.toLowerCase()
    return available
      .filter(
        (code) =>
          code.toLowerCase().includes(lowered) ||
          ISO_COUNTRY_LABELS[code as IsoCountryCode].toLowerCase().includes(lowered)
      )
      .slice(0, 8)
  }, [blockedCountryValues, debouncedCountryQuery])

  const filteredCategorySuggestions = useMemo(() => {
    const query = debouncedCategoryQuery.trim().toLowerCase()
    const available = COMMON_SENSITIVE_CATEGORY_SUGGESTIONS.filter(
      (category) => !sensitiveCategoryValues.includes(category)
    )
    if (!query) {
      return available.slice(0, 8)
    }
    return available.filter((category) => category.toLowerCase().includes(query)).slice(0, 8)
  }, [debouncedCategoryQuery, sensitiveCategoryValues])

  const addBlockedCountry = useCallback(
    (input: string) => {
      const normalized = input.trim().toUpperCase()
      if (!normalized) {
        return false
      }
      const parsed = isoCountryCodeSchema.safeParse(normalized)
      if (!parsed.success) {
        setCountryError(parsed.error.issues[0]?.message ?? 'Invalid ISO country code.')
        return false
      }
      if (blockedCountryValues.includes(parsed.data)) {
        setCountryError('Country already blocked.')
        return false
      }
      const next = [...blockedCountryValues, parsed.data]
      form.setValue('blockedCountries', next, { shouldDirty: true, shouldValidate: true })
      setCountryInput('')
      setCountryError(null)
      form.clearErrors('blockedCountries')
      return true
    },
    [blockedCountryValues, form]
  )

  const removeBlockedCountry = useCallback(
    (code: string) => {
      const next = blockedCountryValues.filter((entry) => entry !== code)
      form.setValue('blockedCountries', next, { shouldDirty: true, shouldValidate: true })
    },
    [blockedCountryValues, form]
  )

  const addSensitiveCategory = useCallback(
    (input: string) => {
      const normalized = input.trim()
      if (!normalized) {
        return false
      }
      const parsed = categorySlugSchema.safeParse(normalized)
      if (!parsed.success) {
        setCategoryError(parsed.error.issues[0]?.message ?? 'Invalid category slug.')
        return false
      }
      if (sensitiveCategoryValues.includes(parsed.data)) {
        setCategoryError('Category already blocked.')
        return false
      }
      const next = [...sensitiveCategoryValues, parsed.data]
      form.setValue('sensitiveCategories', next, { shouldDirty: true, shouldValidate: true })
      setCategoryInput('')
      setCategoryError(null)
      form.clearErrors('sensitiveCategories')
      return true
    },
    [form, sensitiveCategoryValues]
  )

  const removeSensitiveCategory = useCallback(
    (value: string) => {
      const next = sensitiveCategoryValues.filter((entry) => entry !== value)
      form.setValue('sensitiveCategories', next, { shouldDirty: true, shouldValidate: true })
    },
    [form, sensitiveCategoryValues]
  )

  const handleCountryInputKeyDown = useCallback(
    (event: KeyboardEvent<HTMLInputElement>) => {
      if (['Enter', ',', 'Tab'].includes(event.key)) {
        if (event.key !== 'Tab') {
          event.preventDefault()
        }
        addBlockedCountry(event.currentTarget.value)
      } else if (event.key === 'Backspace' && !event.currentTarget.value && blockedCountryValues.length) {
        const last = blockedCountryValues[blockedCountryValues.length - 1]
        removeBlockedCountry(last)
      }
    },
    [addBlockedCountry, blockedCountryValues, removeBlockedCountry]
  )

  const handleCategoryInputKeyDown = useCallback(
    (event: KeyboardEvent<HTMLInputElement>) => {
      if (['Enter', ',', 'Tab'].includes(event.key)) {
        if (event.key !== 'Tab') {
          event.preventDefault()
        }
        addSensitiveCategory(event.currentTarget.value)
      } else if (event.key === 'Backspace' && !event.currentTarget.value && sensitiveCategoryValues.length) {
        const last = sensitiveCategoryValues[sensitiveCategoryValues.length - 1]
        removeSensitiveCategory(last)
      }
    },
    [addSensitiveCategory, removeSensitiveCategory, sensitiveCategoryValues]
  )

  useEffect(() => {
    if (data) {
      const consentManagement = data.consentManagement ?? {
        provider: undefined,
        autoBlock: true,
        consentString: undefined,
        encryptedConsent: undefined,
      }
      const dataRetention = data.dataRetention ?? {
        rawEventsDays: 30,
        aggregatedDataDays: 365,
        userDataDays: 30,
      }
      const regionalSettings = data.regionalSettings ?? {
        euTrafficOnly: false,
        blockedCountries: [],
        sensitiveCategories: [],
      }

      form.reset({
        gdprEnabled: Boolean(data.gdprEnabled),
        ccpaEnabled: Boolean(data.ccpaEnabled),
        coppaMode: Boolean(data.coppaMode),
        consentProvider: consentManagement.provider ?? '',
        autoBlock: Boolean(consentManagement.autoBlock),
        // Do NOT pre-fill the consent string to avoid exposing it in the DOM/history
        consentString: '',
        rawEventsDays: dataRetention.rawEventsDays,
        aggregatedDataDays: dataRetention.aggregatedDataDays,
        userDataDays: dataRetention.userDataDays,
        euTrafficOnly: Boolean(regionalSettings.euTrafficOnly),
        blockedCountries: Array.isArray(regionalSettings.blockedCountries) ? regionalSettings.blockedCountries : [],
        sensitiveCategories: Array.isArray(regionalSettings.sensitiveCategories) ? regionalSettings.sensitiveCategories : [],
      })
      toggleBaselineRef.current = JSON.stringify([
        Boolean(data.gdprEnabled),
        Boolean(data.ccpaEnabled),
        Boolean(data.coppaMode),
        Boolean(consentManagement.autoBlock),
        Boolean(regionalSettings.euTrafficOnly),
      ])
    }
  }, [data, form])

  const updateMutation = useMutation({
    mutationFn: async (values: ComplianceFormValues) => {
      const blockedCountries = (Array.isArray(values.blockedCountries)
        ? values.blockedCountries
        : String(values.blockedCountries || '')
            .split(',')
            .map((entry) => entry.trim()))
        .map((code) => code.toUpperCase())
        .filter(Boolean)

      const sensitiveCategories = (Array.isArray(values.sensitiveCategories)
        ? values.sensitiveCategories
        : String(values.sensitiveCategories || '')
            .split(',')
            .map((entry) => entry.trim()))
        .map((category) => slugifyCategory(category))
        .filter(Boolean)

      const trimmedConsent = values.consentString?.trim() ?? ''
      const consentPayload = trimmedConsent ? await encryptConsentValue(trimmedConsent) : null

      await settingsApi.updateComplianceSettings({
        gdprEnabled: values.gdprEnabled,
        ccpaEnabled: values.ccpaEnabled,
        coppaMode: values.coppaMode,
        consentManagement: {
          provider: values.consentProvider || undefined,
          autoBlock: values.autoBlock,
          consentString:
            consentPayload && consentPayload.mode === 'fallback' ? consentPayload.value : undefined,
          encryptedConsent:
            consentPayload && consentPayload.mode === 'encrypted'
              ? {
                  algorithm: consentPayload.algorithm,
                  cipherText: consentPayload.cipherText,
                  iv: consentPayload.iv,
                  keyId: consentPayload.keyId,
                }
              : undefined,
        },
        dataRetention: {
          rawEventsDays: values.rawEventsDays,
          aggregatedDataDays: values.aggregatedDataDays,
          userDataDays: values.userDataDays,
        },
        regionalSettings: {
          euTrafficOnly: values.euTrafficOnly,
          blockedCountries,
          sensitiveCategories,
        },
      })
    },
    onMutate: () => setMessage(null),
    onSuccess: () => {
      setMessage({ type: 'success', text: 'Compliance settings updated successfully.' })
      form.resetField('consentString', { defaultValue: '' })
    },
    onError: () => setMessage({ type: 'error', text: 'Failed to update compliance settings. Please try again.' }),
  })

  useEffect(() => {
    if (!data) return
    if (updateMutation.isPending) return
    if (!debouncedToggleKey) return

    if (!toggleBaselineRef.current) {
      toggleBaselineRef.current = debouncedToggleKey
      return
    }

    if (toggleBaselineRef.current === debouncedToggleKey) {
      return
    }

    toggleBaselineRef.current = debouncedToggleKey
    updateMutation.mutate(form.getValues())
  }, [data, debouncedToggleKey, form, updateMutation])

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <Link
            href="/settings"
            className="inline-flex items-center gap-2 text-sm text-gray-600 hover:text-gray-900 mb-4"
          >
            <ArrowLeft className="h-4 w-4" aria-hidden={true} />
            Back to Settings
          </Link>
          <div className="flex items-center gap-3">
            <div className="h-12 w-12 rounded-lg bg-primary-50 text-primary-600 flex items-center justify-center">
              <Globe className="h-6 w-6" aria-hidden={true} />
            </div>
            <div>
              <p className="text-sm font-medium text-primary-600">Privacy & Legal</p>
              <h1 className="text-3xl font-semibold text-gray-900 tracking-tight">Regional Compliance</h1>
              <p className="text-sm text-gray-600 mt-1">
                Configure GDPR, CCPA, COPPA protections and manage data retention policies.
              </p>
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <form onSubmit={form.handleSubmit((values) => updateMutation.mutate(values))} className="space-y-6">
          {message && (
            <div
              className={`card flex items-start gap-3 ${
                message.type === 'success'
                  ? 'bg-success-50 border-success-200'
                  : 'bg-danger-50 border-danger-200'
              }`}
            >
              {message.type === 'success' ? (
                <CheckCircle className="h-5 w-5 text-success-600 mt-0.5" aria-hidden={true} />
              ) : (
                <AlertCircle className="h-5 w-5 text-danger-600 mt-0.5" aria-hidden={true} />
              )}
              <p className="text-sm text-gray-700">{message.text}</p>
            </div>
          )}

          <section className="card space-y-6">
            <div>
              <h2 className="text-lg font-semibold text-gray-900">Privacy Frameworks</h2>
              <p className="text-sm text-gray-600">
                Enable region-specific privacy laws that govern how traffic is processed.
              </p>
            </div>
            <div className="space-y-4">
              <label className="flex items-start gap-3 p-4 border rounded-lg hover:bg-gray-50 cursor-pointer">
                <input type="checkbox" className="mt-1" {...form.register('gdprEnabled')} />
                <div>
                  <p className="text-sm font-semibold text-gray-900">GDPR (EU)</p>
                  <p className="text-xs text-gray-600 mt-1">
                    General Data Protection Regulation. Requires explicit consent, data portability, and right to deletion.
                  </p>
                </div>
              </label>
              <label className="flex items-start gap-3 p-4 border rounded-lg hover:bg-gray-50 cursor-pointer">
                <input type="checkbox" className="mt-1" {...form.register('ccpaEnabled')} />
                <div>
                  <p className="text-sm font-semibold text-gray-900">CCPA (California)</p>
                  <p className="text-xs text-gray-600 mt-1">
                    California Consumer Privacy Act. Grants users right to opt out of data sales and see collected data.
                  </p>
                </div>
              </label>
              <label className="flex items-start gap-3 p-4 border rounded-lg hover:bg-gray-50 cursor-pointer">
                <input type="checkbox" className="mt-1" {...form.register('coppaMode')} />
                <div>
                  <p className="text-sm font-semibold text-gray-900">COPPA (Children)</p>
                  <p className="text-xs text-gray-600 mt-1">
                    Children&apos;s Online Privacy Protection Act. Restricts ad targeting for users under 13.
                  </p>
                </div>
              </label>
            </div>
          </section>

          <section className="card space-y-6">
            <div>
              <h2 className="text-lg font-semibold text-gray-900">Consent Management</h2>
              <p className="text-sm text-gray-600">
                Integrate with a CMP or handle consent strings manually to comply with privacy laws.
              </p>
            </div>
            <div>
              <label htmlFor="consentProvider" className="label">
                Consent Provider (Optional)
              </label>
              <select id="consentProvider" className="input" {...form.register('consentProvider')}>
                <option value="">Custom / Manual</option>
                <option value="onetrust">OneTrust</option>
                <option value="cookiebot">Cookiebot</option>
              </select>
            </div>
            <div>
              <label htmlFor="consentString" className="label">
                IAB Consent String (Optional)
              </label>
              <div className="relative">
                <input
                  id="consentString"
                  type={showConsent ? 'text' : 'password'}
                  className="input pr-10"
                  placeholder="CPXs..."
                  autoComplete="off"
                  {...form.register('consentString')}
                />
                <button
                  type="button"
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-500"
                  onClick={() => setShowConsent((s) => !s)}
                  aria-label={showConsent ? 'Hide consent string' : 'Show consent string'}
                >
                  {showConsent ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                </button>
              </div>
              <p className="text-xs text-gray-500 mt-1">
                Leave blank if using a CMP that injects the string automatically. The value is masked here for privacy.
              </p>
            </div>
            <div className="flex items-center gap-3">
              <input id="autoBlock" type="checkbox" className="h-4 w-4 accent-primary-600" {...form.register('autoBlock')} />
              <label htmlFor="autoBlock" className="text-sm text-gray-700">
                Automatically block ad requests when consent is not granted
              </label>
            </div>
          </section>

          <section className="card space-y-6">
            <div>
              <h2 className="text-lg font-semibold text-gray-900">Data Retention</h2>
              <p className="text-sm text-gray-600">
                Define how long different data types are retained before automatic deletion.
              </p>
            </div>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <div>
                <label htmlFor="rawEventsDays" className="label">
                  Raw Events (Days)
                </label>
                <input id="rawEventsDays" type="number" className="input" {...form.register('rawEventsDays')} />
                {form.formState.errors.rawEventsDays && (
                  <p className="text-sm text-danger-600 mt-1">{form.formState.errors.rawEventsDays.message}</p>
                )}
              </div>
              <div>
                <label htmlFor="aggregatedDataDays" className="label">
                  Aggregated Data (Days)
                </label>
                <input id="aggregatedDataDays" type="number" className="input" {...form.register('aggregatedDataDays')} />
                {form.formState.errors.aggregatedDataDays && (
                  <p className="text-sm text-danger-600 mt-1">{form.formState.errors.aggregatedDataDays.message}</p>
                )}
              </div>
              <div>
                <label htmlFor="userDataDays" className="label">
                  User Data (Days)
                </label>
                <input id="userDataDays" type="number" className="input" {...form.register('userDataDays')} />
                {form.formState.errors.userDataDays && (
                  <p className="text-sm text-danger-600 mt-1">{form.formState.errors.userDataDays.message}</p>
                )}
              </div>
            </div>
          </section>

          <section className="card space-y-6">
            <div>
              <h2 className="text-lg font-semibold text-gray-900">Regional Restrictions</h2>
              <p className="text-sm text-gray-600">
                Limit which geographies can access your inventory and block sensitive ad categories.
              </p>
            </div>
            <div className="flex items-center gap-3">
              <input id="euTrafficOnly" type="checkbox" className="h-4 w-4 accent-primary-600" {...form.register('euTrafficOnly')} />
              <label htmlFor="euTrafficOnly" className="text-sm text-gray-700">
                Only serve ads to EU/EEA traffic (strict GDPR enforcement)
              </label>
            </div>
            <div>
              <label htmlFor="blockedCountries" className="label">
                Blocked Countries
              </label>
              <div className="space-y-3">
                <div className="relative">
                  <input
                    id="blockedCountries"
                    type="text"
                    className="input pr-24"
                    placeholder="Search ISO code or country name"
                    value={countryInput}
                    onChange={(e) => {
                      setCountryInput(e.target.value.toUpperCase())
                      setCountryError(null)
                    }}
                    onKeyDown={handleCountryInputKeyDown}
                    aria-describedby="blockedCountriesHelp"
                    autoComplete="off"
                  />
                  <button
                    type="button"
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-sm font-semibold text-primary-600 disabled:opacity-40"
                    onClick={() => addBlockedCountry(countryInput)}
                    disabled={!countryInput.trim()}
                  >
                    Add
                  </button>
                </div>
                {countryError && (
                  <p className="text-sm text-danger-600" role="alert">
                    {countryError}
                  </p>
                )}
                {!countryError && form.formState.errors.blockedCountries && (
                  <p className="text-sm text-danger-600" role="alert">
                    {(form.formState.errors.blockedCountries as any).message}
                  </p>
                )}
                {blockedCountryValues.length > 0 && (
                  <div className="flex flex-wrap gap-2">
                    {blockedCountryValues.map((code) => (
                      <span
                        key={code}
                        className="inline-flex items-center gap-2 rounded-full border border-gray-200 bg-gray-100 px-3 py-1 text-xs font-medium text-gray-700"
                      >
                        <span>
                          {code}{' '}
                          <span className="text-gray-500">
                            {ISO_COUNTRY_LABELS[code as IsoCountryCode] ?? 'Unknown'}
                          </span>
                        </span>
                        <button
                          type="button"
                          onClick={() => removeBlockedCountry(code)}
                          className="text-gray-500 hover:text-gray-900"
                        >
                          <X className="h-3 w-3" aria-hidden={true} />
                          <span className="sr-only">Remove {code}</span>
                        </button>
                      </span>
                    ))}
                  </div>
                )}
                {(debouncedCountryQuery || blockedCountryValues.length === 0) &&
                  filteredCountrySuggestions.length > 0 && (
                    <div className="max-h-48 overflow-y-auto rounded-lg border bg-white shadow-sm divide-y">
                      {filteredCountrySuggestions.map((code) => (
                        <button
                          key={code}
                          type="button"
                          className="flex w-full items-center justify-between px-3 py-2 text-sm hover:bg-gray-50"
                          onClick={() => addBlockedCountry(code)}
                        >
                          <span className="font-medium text-gray-900">{code}</span>
                          <span className="text-xs text-gray-500">
                            {ISO_COUNTRY_LABELS[code as IsoCountryCode]}
                          </span>
                        </button>
                      ))}
                    </div>
                  )}
              </div>
              <p id="blockedCountriesHelp" className="text-xs text-gray-500 mt-1">
                Add ISO 3166-1 alpha-2 codes (e.g., US, DE). Traffic from these countries will be rejected.
              </p>
            </div>
            <div>
              <label htmlFor="sensitiveCategories" className="label">
                Blocked Ad Categories
              </label>
              <div className="space-y-3">
                <div className="relative">
                  <input
                    id="sensitiveCategories"
                    type="text"
                    className="input pr-24"
                    placeholder="gambling, alcohol, dating"
                    value={categoryInput}
                    onChange={(e) => {
                      setCategoryInput(e.target.value)
                      setCategoryError(null)
                    }}
                    onKeyDown={handleCategoryInputKeyDown}
                    aria-describedby="sensitiveCategoriesHelp"
                    autoComplete="off"
                  />
                  <button
                    type="button"
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-sm font-semibold text-primary-600 disabled:opacity-40"
                    onClick={() => addSensitiveCategory(categoryInput)}
                    disabled={!categoryInput.trim()}
                  >
                    Add
                  </button>
                </div>
                {categoryError && (
                  <p className="text-sm text-danger-600" role="alert">
                    {categoryError}
                  </p>
                )}
                {!categoryError && form.formState.errors.sensitiveCategories && (
                  <p className="text-sm text-danger-600" role="alert">
                    {(form.formState.errors.sensitiveCategories as any).message}
                  </p>
                )}
                {sensitiveCategoryValues.length > 0 && (
                  <div className="flex flex-wrap gap-2">
                    {sensitiveCategoryValues.map((category) => (
                      <span
                        key={category}
                        className="inline-flex items-center gap-2 rounded-full border border-gray-200 bg-gray-100 px-3 py-1 text-xs font-medium text-gray-700"
                      >
                        <span>{formatCategoryLabel(category)}</span>
                        <button
                          type="button"
                          onClick={() => removeSensitiveCategory(category)}
                          className="text-gray-500 hover:text-gray-900"
                        >
                          <X className="h-3 w-3" aria-hidden={true} />
                          <span className="sr-only">Remove {category}</span>
                        </button>
                      </span>
                    ))}
                  </div>
                )}
                {(debouncedCategoryQuery || sensitiveCategoryValues.length === 0) &&
                  filteredCategorySuggestions.length > 0 && (
                    <div className="max-h-40 overflow-y-auto rounded-lg border bg-white shadow-sm divide-y">
                      {filteredCategorySuggestions.map((category) => (
                        <button
                          key={category}
                          type="button"
                          className="w-full px-3 py-2 text-left text-sm hover:bg-gray-50"
                          onClick={() => addSensitiveCategory(category)}
                        >
                          {formatCategoryLabel(category)}
                        </button>
                      ))}
                    </div>
                  )}
              </div>
              <p id="sensitiveCategoriesHelp" className="text-xs text-gray-500 mt-1">
                Use IAB categories or custom slugs (letters, numbers, and dashes). Ads in these categories are filtered automatically.
              </p>
            </div>
          </section>

          <div className="flex items-center justify-end gap-3">
            <Link href="/settings" className="btn btn-outline">
              Cancel
            </Link>
            <button
              type="submit"
              className="btn btn-primary flex items-center gap-2"
              disabled={updateMutation.isPending || isLoading}
            >
              <Save className="h-4 w-4" aria-hidden={true} />
              {updateMutation.isPending ? 'Saving...' : 'Save Settings'}
            </button>
          </div>
        </form>
      </main>
    </div>
  )
}
