"use client"

import Link from 'next/link'
import { useParams, useRouter } from 'next/navigation'
import { useEffect, useMemo, useRef, useState, type InputHTMLAttributes } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  AlertTriangle,
  Clock,
  Code2,
  Copy,
  CheckCircle2,
  ChevronLeft,
  Download,
  Link2,
  Loader2,
  Minus,
  PauseCircle,
  Play,
  RefreshCcw,
  Rocket,
  RotateCcw,
  Save,
  Trash2,
  TrendingDown,
  TrendingUp,
} from 'lucide-react'
import { migrationApi } from '@/lib/api'
import type {
  MigrationExperiment,
  MigrationExperimentMetric,
  MigrationExperimentMetricTimeseries,
  MigrationExperimentTimeseriesPoint,
  MigrationGuardrails,
  MigrationMetricUnit,
  MigrationExperimentShareLink,
  MigrationSdkIntegration,
} from '@/types'
import { formatCurrency, formatDate, formatDateRange, formatNumber, formatPercentage, t } from '@/i18n'
import { Spinner } from '@/components/ui/Spinner'
import { Area, AreaChart, CartesianGrid, Legend, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import { handleApiError } from '@/lib/api-client'

const statusBadgeStyles: Record<MigrationExperiment['status'], string> = {
  draft: 'bg-slate-100 text-slate-700',
  active: 'bg-emerald-100 text-emerald-700',
  paused: 'bg-amber-100 text-amber-700',
  completed: 'bg-sky-100 text-sky-700',
  archived: 'bg-slate-100 text-slate-400',
}

const copyTextWithFallback = async (text: string) => {
  if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text)
    return
  }

  if (typeof document === 'undefined') {
    throw new Error('ClipboardUnavailable')
  }

  const textArea = document.createElement('textarea')
  textArea.value = text
  textArea.setAttribute('readonly', '')
  textArea.style.position = 'fixed'
  textArea.style.left = '-9999px'
  textArea.style.opacity = '0'
  document.body.appendChild(textArea)
  textArea.select()

  try {
    const successful = document.execCommand('copy')
    if (!successful) {
      throw new Error('ClipboardFallbackFailed')
    }
  } finally {
    document.body.removeChild(textArea)
  }
}

const guardrailKeys: (keyof MigrationGuardrails)[] = [
  'latency_budget_ms',
  'revenue_floor_percent',
  'max_error_rate_percent',
  'min_impressions',
]

type GuardrailKey = (typeof guardrailKeys)[number]

type GuardrailFieldState = {
  input: string
  value: number | null
}

type GuardrailFormState = Record<GuardrailKey, GuardrailFieldState>

const createGuardrailFieldState = (initialValue?: number | null): GuardrailFieldState => {
  if (typeof initialValue === 'number' && Number.isFinite(initialValue)) {
    return {
      input: `${initialValue}`,
      value: initialValue,
    }
  }

  return { input: '', value: null }
}

const createGuardrailFormState = (guardrails?: Partial<MigrationGuardrails>): GuardrailFormState => ({
  latency_budget_ms: createGuardrailFieldState(guardrails?.latency_budget_ms ?? null),
  revenue_floor_percent: createGuardrailFieldState(guardrails?.revenue_floor_percent ?? null),
  max_error_rate_percent: createGuardrailFieldState(guardrails?.max_error_rate_percent ?? null),
  min_impressions: createGuardrailFieldState(guardrails?.min_impressions ?? null),
})

const guardrailFieldFromInput = (input: string): GuardrailFieldState => {
  if (input.trim() === '') {
    return { input: '', value: null }
  }

  const numericValue = Number(input)
  if (!Number.isFinite(numericValue)) {
    return { input, value: null }
  }

  return {
    input,
    value: numericValue,
  }
}

const CONTROL_SERIES_COLOR = '#2563eb'
const TEST_SERIES_COLOR = '#10b981'

export default function MigrationExperimentPage() {
  const params = useParams<{ experimentId: string }>()
  const router = useRouter()
  const queryClient = useQueryClient()
  const experimentId = params?.experimentId ?? ''

  const [mirrorPercent, setMirrorPercent] = useState<number>(5)
  const [guardrailState, setGuardrailState] = useState<GuardrailFormState>(() =>
    createGuardrailFormState()
  )
  const [guardrailResult, setGuardrailResult] = useState<{
    shouldPause: boolean
    violations: string[]
  } | null>(null)
  const [showSavedBanner, setShowSavedBanner] = useState<boolean>(false)

  const handleGuardrailFieldChange = (field: GuardrailKey, input: string) => {
    setGuardrailState((current) => ({
      ...current,
      [field]: guardrailFieldFromInput(input),
    }))
  }

  const experimentQuery = useQuery({
    queryKey: ['migration-experiment', experimentId],
    enabled: Boolean(experimentId),
    queryFn: async () => {
      if (!experimentId) return null
      return migrationApi.getExperiment(experimentId)
    },
  })

  const experiment = experimentQuery.data ?? null

  const shareLinksQuery = useQuery<MigrationExperimentShareLink[], Error>({
    queryKey: ['migration-experiment-share-links', experimentId],
    enabled: Boolean(experimentId),
    queryFn: async () => {
      if (!experimentId) return []
      return migrationApi.getExperimentShareLinks(experimentId)
    },
  })

  const shareLinks = shareLinksQuery.data ?? []
  const shareLinksErrorMessage = shareLinksQuery.isError
    ? handleApiError(shareLinksQuery.error)
    : null

  const reportQuery = useQuery({
    queryKey: ['migration-experiment-report', experimentId],
    enabled: Boolean(experimentId),
    queryFn: async () => {
      if (!experimentId) return null
      return migrationApi.getExperimentReport(experimentId, {
        window: 'last_7_days',
        granularity: 'day',
      })
    },
  })

  const report = reportQuery.data ?? null
  const overallMetrics = report?.metrics?.overall ?? []
  const isReportEmpty = overallMetrics.length === 0
  const timeseriesMetrics = useMemo(() => report?.metrics?.timeseries ?? [], [
    report?.metrics?.timeseries,
  ])
  const [selectedTimeseriesId, setSelectedTimeseriesId] = useState<string | null>(null)
  const hasTimeseries = timeseriesMetrics.length > 0
  const [selectedShareExpiry, setSelectedShareExpiry] = useState<string>('168')
  const [shareFeedback, setShareFeedback] = useState<
    { tone: 'success' | 'error'; message: string } | null
  >(null)
  const [downloadFeedback, setDownloadFeedback] = useState<
    { tone: 'error'; message: string } | null
  >(null)
  const [copiedLinkId, setCopiedLinkId] = useState<string | null>(null)
  const copyTimeoutRef = useRef<number | null>(null)

  useEffect(() => {
    if (timeseriesMetrics.length === 0) {
      if (selectedTimeseriesId !== null) {
        setSelectedTimeseriesId(null)
      }
      return
    }

    const exists = timeseriesMetrics.some((metric) => metric.id === selectedTimeseriesId)
    if (!exists) {
      setSelectedTimeseriesId(timeseriesMetrics[0]?.id ?? null)
    }
  }, [selectedTimeseriesId, timeseriesMetrics])

  const activeTimeseries = useMemo(() => {
    if (!selectedTimeseriesId) return null
    return timeseriesMetrics.find((metric) => metric.id === selectedTimeseriesId) ?? null
  }, [selectedTimeseriesId, timeseriesMetrics])

  useEffect(() => {
    if (!experiment) return

    setMirrorPercent(Math.min(20, Math.max(0, experiment.mirror_percent)))
    const guardrails = experiment.guardrails ?? {}
    setGuardrailState(createGuardrailFormState(guardrails))
  }, [experiment])

  const isDraft = experiment?.status === 'draft'
  const isActive = experiment?.status === 'active'
  const isPaused = experiment?.status === 'paused'

  const currentGuardrails = useMemo<Partial<MigrationGuardrails>>(
    () => experiment?.guardrails ?? {},
    [experiment]
  )

  const parsedGuardrails: Partial<MigrationGuardrails> = useMemo(() => {
    const asValue = (field: GuardrailFieldState) => (field.value === null ? undefined : field.value)
    const asPreciseValue = (field: GuardrailFieldState) =>
      field.value === null ? undefined : Number(field.value.toFixed(2))

    return {
      latency_budget_ms: asValue(guardrailState.latency_budget_ms),
      revenue_floor_percent: asPreciseValue(guardrailState.revenue_floor_percent),
      max_error_rate_percent: asPreciseValue(guardrailState.max_error_rate_percent),
      min_impressions: asValue(guardrailState.min_impressions),
    }
  }, [guardrailState])

  const isDirty = useMemo(() => {
    if (!experiment) return false
    const guardrailChanged = guardrailKeys.some((key) => {
      const currentValue = currentGuardrails?.[key]
      const nextValue = parsedGuardrails[key]
      return (currentValue ?? undefined) !== (nextValue ?? undefined)
    })
    return guardrailChanged || experiment.mirror_percent !== mirrorPercent
  }, [currentGuardrails, experiment, mirrorPercent, parsedGuardrails])

  useEffect(() => {
    if (!showSavedBanner) return
    const timeout = window.setTimeout(() => setShowSavedBanner(false), 4000)
    return () => window.clearTimeout(timeout)
  }, [showSavedBanner])

  useEffect(() => {
    if (guardrailResult && guardrailResult.shouldPause && experiment?.status === 'paused') {
      setGuardrailResult(null)
    }
  }, [experiment?.status, guardrailResult])

  const handleCopyLink = async (link: MigrationExperimentShareLink) => {
    try {
      await copyTextWithFallback(link.url)
      if (copyTimeoutRef.current) {
        window.clearTimeout(copyTimeoutRef.current)
      }
      setCopiedLinkId(link.id)
      copyTimeoutRef.current = window.setTimeout(() => setCopiedLinkId(null), 2000)
    } catch (error) {
      const fallbackMessage =
        error instanceof Error &&
        (error.message === 'ClipboardUnavailable' || error.message === 'ClipboardFallbackFailed')
          ? t('migrationStudio.detail.reportAccessCopyUnsupported')
          : handleApiError(error)
      setShareFeedback({
        tone: 'error',
        message: fallbackMessage,
      })
    }
  }

  useEffect(() => {
    return () => {
      if (copyTimeoutRef.current) {
        window.clearTimeout(copyTimeoutRef.current)
      }
    }
  }, [])

  useEffect(() => {
    if (!shareFeedback) return
    const timeout = window.setTimeout(() => setShareFeedback(null), 5000)
    return () => window.clearTimeout(timeout)
  }, [shareFeedback])

  useEffect(() => {
    if (!downloadFeedback) return
    const timeout = window.setTimeout(() => setDownloadFeedback(null), 5000)
    return () => window.clearTimeout(timeout)
  }, [downloadFeedback])

  const createShareLinkMutation = useMutation<MigrationExperimentShareLink, unknown, number>({
    mutationFn: async (expiresInHours) => {
      if (!experiment) {
        throw new Error(t('migrationStudio.detail.reportAccessMissingExperiment'))
      }
      return migrationApi.createExperimentShareLink(experiment.id, {
        expires_in_hours: expiresInHours,
      })
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ['migration-experiment-share-links', experimentId],
      })
      setShareFeedback({
        tone: 'success',
        message: t('migrationStudio.detail.reportAccessShareSuccess'),
      })
      setCopiedLinkId(null)
    },
    onError: (error) => {
      setShareFeedback({
        tone: 'error',
        message: handleApiError(error),
      })
    },
  })

  const revokeShareLinkMutation = useMutation<void, unknown, string>({
    mutationFn: async (shareLinkId) => {
      if (!experiment) {
        throw new Error(t('migrationStudio.detail.reportAccessMissingExperiment'))
      }
      await migrationApi.revokeExperimentShareLink(experiment.id, shareLinkId)
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ['migration-experiment-share-links', experimentId],
      })
      setShareFeedback({
        tone: 'success',
        message: t('migrationStudio.detail.reportAccessRevokeSuccess'),
      })
    },
    onError: (error) => {
      setShareFeedback({
        tone: 'error',
        message: handleApiError(error),
      })
    },
  })

  const downloadReportMutation = useMutation<Blob, unknown, void>({
    mutationFn: async () => {
      if (!experimentId) {
        throw new Error(t('migrationStudio.detail.reportAccessMissingExperiment'))
      }
      return migrationApi.downloadExperimentReport(experimentId)
    },
    onSuccess: (blob) => {
      if (typeof window === 'undefined') return
      const url = window.URL.createObjectURL(blob)
      const anchor = document.createElement('a')
      const timestamp = new Date().toISOString().replace(/[:.]/g, '-')
      const safeExperimentId = experiment?.id ?? experimentId
      const fileName = `migration-report-${safeExperimentId}-${timestamp}.json`
      anchor.href = url
      anchor.download = fileName
      document.body.appendChild(anchor)
      anchor.click()
      document.body.removeChild(anchor)
      window.URL.revokeObjectURL(url)
      setDownloadFeedback(null)
    },
    onError: (error) => {
      setDownloadFeedback({
        tone: 'error',
        message: handleApiError(error),
      })
    },
  })

  const invalidateExperimentQueries = (updated: MigrationExperiment) => {
    queryClient.setQueryData(['migration-experiment', experimentId], updated)
    queryClient.invalidateQueries({ queryKey: ['migration-experiments'] })
  }

  const updateMutation = useMutation({
    mutationFn: async () => {
      if (!experiment) return null
      const payload = {
        mirror_percent: mirrorPercent,
        guardrails: parsedGuardrails,
      }
      const updated = await migrationApi.updateExperiment(experiment.id, payload)
      return updated
    },
    onSuccess: (updated) => {
      if (updated) {
        invalidateExperimentQueries(updated)
        setGuardrailResult(null)
        setShowSavedBanner(true)
      }
    },
  })

  const activateMutation = useMutation({
    mutationFn: async () => {
      if (!experiment) return null
      const updated = await migrationApi.activateExperiment(experiment.id, { mirror_percent: mirrorPercent })
      return updated
    },
    onSuccess: (updated) => {
      if (updated) {
        invalidateExperimentQueries(updated)
        setGuardrailResult(null)
      }
    },
  })

  const pauseMutation = useMutation({
    mutationFn: async () => {
      if (!experiment) return null
      const updated = await migrationApi.pauseExperiment(
        experiment.id,
        t('migrationStudio.detail.pauseReason')
      )
      return updated
    },
    onSuccess: (updated) => {
      if (updated) {
        invalidateExperimentQueries(updated)
      }
    },
  })

  const evaluateMutation = useMutation({
    mutationFn: async () => {
      if (!experiment) return null
      return migrationApi.evaluateGuardrails(experiment.id)
    },
    onSuccess: (result) => {
      if (result) {
        setGuardrailResult(result)
        if (result.shouldPause) {
          void experimentQuery.refetch()
          queryClient.invalidateQueries({ queryKey: ['migration-experiments'] })
        }
      }
    },
  })

  if (!experimentId) {
    return null
  }

  if (experimentQuery.isLoading) {
    return (
      <div className="min-h-screen bg-gray-50">
        <main className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-16">
          <div className="card flex items-center gap-3">
            <Spinner size="md" label={t('migrationStudio.detail.loadingLabel')} />
            <span className="text-sm text-gray-600">{t('migrationStudio.detail.loadingCopy')}</span>
          </div>
        </main>
      </div>
    )
  }

  if (experimentQuery.isError || !experiment) {
    return (
      <div className="min-h-screen bg-gray-50">
        <main className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-16">
          <div className="card border border-rose-200 bg-rose-50 text-rose-800">
            <div className="flex items-start gap-3">
              <AlertTriangle className="h-5 w-5" aria-hidden={true} />
              <div>
                <h1 className="text-lg font-semibold">
                  {t('migrationStudio.detail.loadErrorTitle')}
                </h1>
                <p className="mt-1 text-sm">
                  {t('migrationStudio.detail.loadErrorCopy')}
                </p>
                <button
                  type="button"
                  className="btn btn-outline mt-4"
                  onClick={() => router.push('/migration-studio')}
                >
                  {t('migrationStudio.detail.returnToList')}
                </button>
              </div>
            </div>
          </div>
        </main>
      </div>
    )
  }

  const statusBadgeClass = statusBadgeStyles[experiment.status]

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-6 space-y-4">
          <Link
            href="/migration-studio"
            className="inline-flex items-center gap-2 text-sm text-gray-500 hover:text-gray-700"
          >
            <ChevronLeft className="h-4 w-4" aria-hidden={true} />
            {t('migrationStudio.detail.backToOverview')}
          </Link>
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div>
              <p className="text-sm font-medium uppercase tracking-wide text-primary-600">
                {t('migrationStudio.detail.eyebrow')}
              </p>
              <h1 className="mt-1 text-3xl font-semibold text-gray-900">{experiment.name}</h1>
              {experiment.description && (
                <p className="mt-2 text-sm text-gray-600 max-w-2xl">{experiment.description}</p>
              )}
              <div className="mt-3 flex flex-wrap items-center gap-3 text-sm text-gray-600">
                <span className={`inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium ${statusBadgeClass}`}>
                  {experiment.status}
                </span>
                <span className="uppercase tracking-wide text-xs text-gray-400">
                  {t('migrationStudio.experiments.mirrorPercent')}: {experiment.mirror_percent}%
                </span>
                {experiment.mode && (
                  <span className="uppercase tracking-wide text-xs text-gray-400">
                    {t('migrationStudio.messages.mode', { mode: experiment.mode })}
                  </span>
                )}
              </div>
            </div>
            <div className="text-xs text-gray-500 space-y-1 text-left sm:text-right">
              <div>
                {t('migrationStudio.detail.objectiveLabel')}: {experiment.objective}
              </div>
              <div>
                {t('migrationStudio.detail.seedLabel')}: <span className="font-mono text-gray-600">{experiment.seed}</span>
              </div>
              <div>
                {t('migrationStudio.detail.updatedLabel', { date: formatDate(experiment.updated_at) })}
              </div>
              {experiment.last_guardrail_check && (
                <div>
                  {t('migrationStudio.detail.lastGuardrailCheck', { date: formatDate(experiment.last_guardrail_check) })}
                </div>
              )}
            </div>
          </div>
        </div>
      </header>

      <main className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-10 space-y-8">
        {showSavedBanner && (
          <div
            className="border border-emerald-200 bg-emerald-50 text-emerald-800 rounded-xl px-4 py-3 flex items-start gap-3"
            role="status"
            aria-live="polite"
          >
            <CheckCircle2 className="h-5 w-5" aria-hidden={true} />
            <div>
              <p className="font-medium text-sm">{t('migrationStudio.detail.saveSuccessTitle')}</p>
              <p className="text-xs mt-1">{t('migrationStudio.detail.saveSuccessCopy')}</p>
            </div>
          </div>
        )}

  <section className="card space-y-6">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div>
              <h2 className="text-xl font-semibold text-gray-900">
                {t('migrationStudio.detail.metricsHeading')}
              </h2>
              <p className="mt-1 text-sm text-gray-600 max-w-2xl">
                {t('migrationStudio.detail.metricsDescription')}
              </p>
              {report?.window && (
                <p className="mt-2 text-xs text-gray-500">
                  {t('migrationStudio.detail.metricsWindow', {
                    window: formatDateRange(report.window.start, report.window.end),
                  })}
                </p>
              )}
            </div>
            <div className="flex flex-col items-end gap-2 text-xs text-gray-500">
              {report && (
                <span>
                  {t('migrationStudio.detail.metricsGenerated', {
                    date: formatDate(report.generated_at),
                  })}
                </span>
              )}
              <button
                type="button"
                className="btn btn-ghost text-sm inline-flex items-center gap-2"
                onClick={() => reportQuery.refetch()}
                disabled={reportQuery.isLoading || reportQuery.isFetching}
              >
                {reportQuery.isFetching ? (
                  <Spinner size="sm" label={t('migrationStudio.detail.metricsRefreshing')} />
                ) : (
                  <RefreshCcw className="h-4 w-4" aria-hidden={true} />
                )}
                {t('migrationStudio.detail.metricsRefresh')}
              </button>
            </div>
          </div>

          {reportQuery.isLoading ? (
            <div className="flex items-center gap-3 text-sm text-gray-500">
              <Spinner size="sm" label={t('migrationStudio.detail.metricsLoadingLabel')} />
              <span>{t('migrationStudio.detail.metricsLoadingCopy')}</span>
            </div>
          ) : reportQuery.isError ? (
            <div className="flex gap-3 rounded-lg border border-rose-200 bg-rose-50 px-4 py-3 text-rose-800">
              <AlertTriangle className="h-5 w-5" aria-hidden={true} />
              <div>
                <p className="font-medium text-sm">
                  {t('migrationStudio.detail.metricsErrorTitle')}
                </p>
                <p className="mt-1 text-xs">
                  {t('migrationStudio.detail.metricsErrorCopy')}
                </p>
                <button
                  type="button"
                  className="btn btn-outline btn-sm mt-3 inline-flex items-center gap-2"
                  onClick={() => reportQuery.refetch()}
                >
                  <RefreshCcw className="h-3.5 w-3.5" aria-hidden={true} />
                  {t('migrationStudio.detail.metricsRetryCta')}
                </button>
              </div>
            </div>
          ) : isReportEmpty ? (
            <div className="rounded-lg border border-dashed border-gray-200 bg-gray-50/70 px-4 py-5 text-sm text-gray-600">
              <p className="font-medium text-gray-700">
                {t('migrationStudio.detail.metricsEmptyTitle')}
              </p>
              <p className="mt-1 text-xs text-gray-500">
                {t('migrationStudio.detail.metricsEmptyCopy')}
              </p>
            </div>
          ) : (
            <div className="grid gap-4 md:grid-cols-2">
              {overallMetrics.map((metric) => (
                <MetricComparisonCard key={metric.id} metric={metric} />
              ))}
            </div>
          )}
        </section>

  <section className="card space-y-6">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div>
              <h2 className="text-xl font-semibold text-gray-900">
                {t('migrationStudio.detail.metricsTimeseriesHeading')}
              </h2>
              <p className="mt-1 text-sm text-gray-600 max-w-2xl">
                {t('migrationStudio.detail.metricsTimeseriesDescription')}
              </p>
            </div>
          </div>

          {reportQuery.isLoading ? (
            <div className="flex items-center gap-3 text-sm text-gray-500">
              <Spinner size="sm" label={t('migrationStudio.detail.metricsLoadingLabel')} />
              <span>{t('migrationStudio.detail.metricsLoadingCopy')}</span>
            </div>
          ) : reportQuery.isError ? (
            <div className="flex gap-3 rounded-lg border border-rose-200 bg-rose-50 px-4 py-3 text-rose-800">
              <AlertTriangle className="h-5 w-5" aria-hidden={true} />
              <div>
                <p className="font-medium text-sm">
                  {t('migrationStudio.detail.metricsErrorTitle')}
                </p>
                <p className="mt-1 text-xs">
                  {t('migrationStudio.detail.metricsErrorCopy')}
                </p>
                <button
                  type="button"
                  className="btn btn-outline btn-sm mt-3 inline-flex items-center gap-2"
                  onClick={() => reportQuery.refetch()}
                >
                  <RefreshCcw className="h-3.5 w-3.5" aria-hidden={true} />
                  {t('migrationStudio.detail.metricsRetryCta')}
                </button>
              </div>
            </div>
          ) : timeseriesMetrics.length === 0 || !activeTimeseries ? (
            <div className="rounded-lg border border-dashed border-gray-200 bg-gray-50/70 px-4 py-5 text-sm text-gray-600">
              <p className="font-medium text-gray-700">
                {t('migrationStudio.detail.metricsTimeseriesEmptyTitle')}
              </p>
              <p className="mt-1 text-xs text-gray-500">
                {t('migrationStudio.detail.metricsTimeseriesEmptyCopy')}
              </p>
            </div>
          ) : (
            <div className="space-y-5">
              <div className="flex flex-wrap gap-2">
                {timeseriesMetrics.map((metric) => {
                  const isActive = metric.id === activeTimeseries.id
                  return (
                    <button
                      key={metric.id}
                      type="button"
                      className={`btn btn-sm ${
                        isActive ? 'btn-primary' : 'btn-ghost'
                      }`}
                      aria-pressed={isActive}
                      onClick={() => setSelectedTimeseriesId(metric.id)}
                    >
                      {metric.label}
                    </button>
                  )
                })}
              </div>
              <MetricTimeseriesChart metric={activeTimeseries} />
            </div>
          )}
        </section>

        {!reportQuery.isLoading && !reportQuery.isError && hasTimeseries && (
          <section className="card space-y-6">
            <div>
              <h2 className="text-xl font-semibold text-gray-900">
                {t('migrationStudio.detail.metricsDashboardsHeading')}
              </h2>
              <p className="mt-1 text-sm text-gray-600 max-w-2xl">
                {t('migrationStudio.detail.metricsDashboardsDescription')}
              </p>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              {timeseriesMetrics.map((metric) => (
                <MetricTimeseriesDashboardCard key={`dashboard-${metric.id}`} metric={metric} />
              ))}
            </div>
          </section>
        )}

        <section className="card space-y-6">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div>
              <h2 className="text-xl font-semibold text-gray-900">
                {t('migrationStudio.detail.reportAccessHeading')}
              </h2>
              <p className="mt-1 text-sm text-gray-600 max-w-2xl">
                {t('migrationStudio.detail.reportAccessDescription')}
              </p>
            </div>
          </div>

          {shareFeedback ? (
            <div
              className={`flex items-start gap-2 rounded-lg border px-3 py-2 text-sm ${
                shareFeedback.tone === 'success'
                  ? 'border-emerald-200 bg-emerald-50 text-emerald-700'
                  : 'border-rose-200 bg-rose-50 text-rose-800'
              }`}
              role={shareFeedback.tone === 'success' ? 'status' : 'alert'}
              aria-live="polite"
            >
              {shareFeedback.tone === 'success' ? (
                <CheckCircle2 className="h-4 w-4" aria-hidden={true} />
              ) : (
                <AlertTriangle className="h-4 w-4" aria-hidden={true} />
              )}
              <span>{shareFeedback.message}</span>
            </div>
          ) : null}

          {downloadFeedback ? (
            <div
              className="flex items-start gap-2 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-sm text-rose-800"
              role="alert"
              aria-live="assertive"
            >
              <AlertTriangle className="h-4 w-4" aria-hidden={true} />
              <span>{downloadFeedback.message}</span>
            </div>
          ) : null}

          <div className="space-y-6">
            <div>
              <h3 className="text-lg font-semibold text-gray-900">
                {t('migrationStudio.detail.reportAccessShareHeading')}
              </h3>
              <p className="mt-1 text-sm text-gray-600 max-w-2xl">
                {t('migrationStudio.detail.reportAccessShareDescription')}
              </p>

              <div className="mt-4 space-y-4">
                {shareLinksQuery.isLoading || shareLinksQuery.isFetching ? (
                  <div className="flex items-center gap-3 text-sm text-gray-500">
                    <Spinner size="sm" label={t('migrationStudio.detail.reportAccessSharingLoading')} />
                    <span>{t('migrationStudio.detail.reportAccessSharingLoadingCopy')}</span>
                  </div>
                ) : shareLinksErrorMessage ? (
                  <div className="flex items-start gap-2 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-sm text-rose-800">
                    <AlertTriangle className="h-4 w-4" aria-hidden={true} />
                    <div>
                      <p>{shareLinksErrorMessage}</p>
                      <button
                        type="button"
                        className="btn btn-outline btn-sm mt-3 inline-flex items-center gap-2"
                        onClick={() => shareLinksQuery.refetch()}
                      >
                        <RefreshCcw className="h-3.5 w-3.5" aria-hidden={true} />
                        {t('migrationStudio.detail.reportAccessRetryCta')}
                      </button>
                    </div>
                  </div>
                ) : shareLinks.length === 0 ? (
                  <div className="rounded-lg border border-dashed border-gray-200 bg-gray-50/70 px-4 py-4 text-sm text-gray-600">
                    <p className="font-medium text-gray-700">
                      {t('migrationStudio.detail.reportAccessEmptyTitle')}
                    </p>
                    <p className="mt-1 text-xs text-gray-500">
                      {t('migrationStudio.detail.reportAccessEmptyCopy')}
                    </p>
                  </div>
                ) : (
                  <ul className="space-y-3">
                    {shareLinks.map((link) => (
                      <li
                        key={link.id}
                        className="flex flex-col gap-3 rounded-lg border border-gray-200 bg-white px-4 py-3 md:flex-row md:items-center md:justify-between"
                      >
                        <div className="space-y-1">
                          <div className="flex items-center gap-2 text-sm font-medium text-gray-900 break-all">
                            <Link2 className="h-4 w-4 text-primary-500" aria-hidden={true} />
                            <span>{link.url}</span>
                          </div>
                          <div className="flex flex-wrap items-center gap-3 text-xs text-gray-500">
                            <span className="inline-flex items-center gap-1">
                              <Clock className="h-3.5 w-3.5" aria-hidden={true} />
                              {t('migrationStudio.detail.reportAccessExpires', {
                                date: formatDate(link.expires_at),
                              })}
                            </span>
                            <span>
                              {t('migrationStudio.detail.reportAccessCreated', {
                                date: formatDate(link.created_at),
                              })}
                            </span>
                          </div>
                        </div>
                        <div className="flex flex-wrap items-center gap-2">
                          <button
                            type="button"
                            className="btn btn-ghost btn-sm inline-flex items-center gap-2"
                            onClick={() => void handleCopyLink(link)}
                          >
                            <Copy className="h-4 w-4" aria-hidden={true} />
                            {copiedLinkId === link.id
                              ? t('migrationStudio.detail.reportAccessCopied')
                              : t('migrationStudio.detail.reportAccessCopy')}
                          </button>
                          <button
                            type="button"
                            className="btn btn-outline btn-sm inline-flex items-center gap-2"
                            onClick={() => revokeShareLinkMutation.mutate(link.id)}
                            disabled={revokeShareLinkMutation.isPending}
                          >
                            {revokeShareLinkMutation.isPending ? (
                              <Loader2 className="h-4 w-4 animate-spin" aria-hidden={true} />
                            ) : (
                              <Trash2 className="h-4 w-4" aria-hidden={true} />
                            )}
                            {revokeShareLinkMutation.isPending
                              ? t('migrationStudio.detail.reportAccessRevoking')
                              : t('migrationStudio.detail.reportAccessRevoke')}
                          </button>
                        </div>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            </div>

            <div className="rounded-lg border border-dashed border-gray-200 bg-gray-50/70 px-4 py-4">
              <label htmlFor="share-link-expiry" className="block text-sm font-medium text-gray-700">
                {t('migrationStudio.detail.reportAccessShareSelectLabel')}
              </label>
              <div className="mt-3 flex flex-wrap items-center gap-3">
                <select
                  id="share-link-expiry"
                  className="input w-52"
                  value={selectedShareExpiry}
                  onChange={(event) => setSelectedShareExpiry(event.target.value)}
                  aria-describedby="share-link-hint"
                >
                  <option value="24">{t('migrationStudio.detail.reportAccessShareOption24Hours')}</option>
                  <option value="168">{t('migrationStudio.detail.reportAccessShareOption7Days')}</option>
                  <option value="720">{t('migrationStudio.detail.reportAccessShareOption30Days')}</option>
                </select>
                <button
                  type="button"
                  className="btn btn-primary inline-flex items-center gap-2"
                  onClick={() => createShareLinkMutation.mutate(Number(selectedShareExpiry))}
                  disabled={createShareLinkMutation.isPending}
                >
                  {createShareLinkMutation.isPending ? (
                    <Loader2 className="h-4 w-4 animate-spin" aria-hidden={true} />
                  ) : (
                    <Link2 className="h-4 w-4" aria-hidden={true} />
                  )}
                  {createShareLinkMutation.isPending
                    ? t('migrationStudio.detail.reportAccessGenerating')
                    : t('migrationStudio.detail.reportAccessGenerate')}
                </button>
                {createShareLinkMutation.isPending ? (
                  <span className="text-xs text-gray-500" role="status" aria-live="polite">
                    {t('migrationStudio.detail.reportAccessGenerating')}
                  </span>
                ) : null}
              </div>
              <p className="mt-2 text-xs text-gray-500" id="share-link-hint">
                {t('migrationStudio.detail.reportAccessShareHint')}
              </p>
            </div>

            <div className="pt-4 border-t border-gray-100">
              <h3 className="text-lg font-semibold text-gray-900">
                {t('migrationStudio.detail.reportAccessDownloadHeading')}
              </h3>
              <p className="mt-1 text-sm text-gray-600 max-w-2xl">
                {t('migrationStudio.detail.reportAccessDownloadDescription')}
              </p>
              <div className="mt-4">
                <button
                  type="button"
                  className="btn btn-secondary inline-flex items-center gap-2"
                  onClick={() => downloadReportMutation.mutate()}
                  disabled={downloadReportMutation.isPending}
                >
                  {downloadReportMutation.isPending ? (
                    <Loader2 className="h-4 w-4 animate-spin" aria-hidden={true} />
                  ) : (
                    <Download className="h-4 w-4" aria-hidden={true} />
                  )}
                  {downloadReportMutation.isPending
                    ? t('migrationStudio.detail.reportAccessDownloading')
                    : t('migrationStudio.detail.reportAccessDownloadCta')}
                </button>
                {downloadReportMutation.isPending ? (
                  <p className="mt-2 text-xs text-gray-500" role="status" aria-live="polite">
                    {t('migrationStudio.detail.reportAccessDownloading')}
                  </p>
                ) : null}
              </div>
            </div>
          </div>
        </section>

        <section className="card space-y-6">
          <div className="flex items-center justify-between gap-4">
            <div>
              <h2 className="text-xl font-semibold text-gray-900">{t('migrationStudio.detail.mirrorHeading')}</h2>
              <p className="mt-1 text-sm text-gray-600 max-w-2xl">
                {t('migrationStudio.detail.mirrorDescription')}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              {(isDraft || isPaused) && (
                <button
                  type="button"
                  className="btn btn-primary flex items-center gap-2"
                  onClick={() => activateMutation.mutate()}
                  disabled={activateMutation.isPending}
                >
                  {activateMutation.isPending ? (
                    <Spinner size="sm" label={t('migrationStudio.import.actions.loading')} />
                  ) : (
                    <>
                      <Play className="h-4 w-4" aria-hidden={true} />
                      {isPaused ? t('migrationStudio.actions.resume') : t('migrationStudio.actions.activate')}
                    </>
                  )}
                </button>
              )}
              {isActive && (
                <button
                  type="button"
                  className="btn btn-secondary flex items-center gap-2"
                  onClick={() => pauseMutation.mutate()}
                  disabled={pauseMutation.isPending}
                >
                  {pauseMutation.isPending ? (
                    <Spinner size="sm" label={t('migrationStudio.import.actions.loading')} />
                  ) : (
                    <>
                      <PauseCircle className="h-4 w-4" aria-hidden={true} />
                      {t('migrationStudio.actions.pause')}
                    </>
                  )}
                </button>
              )}
            </div>
          </div>

          <div>
            <label
              htmlFor="mirror-percent"
              className="block text-sm font-medium text-gray-700"
              id="mirror-percent-label"
            >
              {t('migrationStudio.detail.mirrorPercentLabel')}
            </label>
            <div className="mt-3">
              <input
                id="mirror-percent"
                type="range"
                min={0}
                max={20}
                step={1}
                value={mirrorPercent}
                onChange={(event) => setMirrorPercent(Number(event.target.value))}
                className="w-full"
                aria-labelledby="mirror-percent-label"
              />
            </div>
            <div className="mt-3 flex items-center gap-3">
              <input
                type="number"
                min={0}
                max={20}
                value={mirrorPercent}
                onChange={(event) => {
                  const next = Number(event.target.value)
                  if (!Number.isFinite(next)) return
                  setMirrorPercent(Math.min(20, Math.max(0, next)))
                }}
                className="input w-28"
                aria-describedby="mirror-percent-helper"
                aria-labelledby="mirror-percent-label"
              />
              <span className="text-sm text-gray-500" id="mirror-percent-helper">
                {t('migrationStudio.detail.mirrorPercentHelper')}
              </span>
            </div>
          </div>

          <div className="pt-4 border-t border-gray-100">
            <h3 className="text-lg font-semibold text-gray-900">
              {t('migrationStudio.detail.guardrailHeading')}
            </h3>
            <p className="mt-1 text-sm text-gray-600 max-w-2xl">
              {t('migrationStudio.detail.guardrailDescription')}
            </p>

            <div className="mt-6 grid grid-cols-1 gap-6 sm:grid-cols-2">
              <GuardrailField
                id="latency-budget"
                label={t('migrationStudio.detail.latencyLabel')}
                suffix={t('migrationStudio.detail.latencySuffix')}
                value={guardrailState.latency_budget_ms.input}
                onChange={(value) => handleGuardrailFieldChange('latency_budget_ms', value)}
                inputMode="numeric"
                min={0}
              />
              <GuardrailField
                id="revenue-floor"
                label={t('migrationStudio.detail.revenueLabel')}
                suffix={t('migrationStudio.detail.revenueSuffix')}
                value={guardrailState.revenue_floor_percent.input}
                onChange={(value) => handleGuardrailFieldChange('revenue_floor_percent', value)}
                inputMode="decimal"
                min={0}
                step="0.1"
              />
              <GuardrailField
                id="error-rate"
                label={t('migrationStudio.detail.errorRateLabel')}
                suffix={t('migrationStudio.detail.errorRateSuffix')}
                value={guardrailState.max_error_rate_percent.input}
                onChange={(value) => handleGuardrailFieldChange('max_error_rate_percent', value)}
                inputMode="decimal"
                min={0}
                step="0.1"
              />
              <GuardrailField
                id="min-impressions"
                label={t('migrationStudio.detail.minImpressionsLabel')}
                suffix={t('migrationStudio.detail.minImpressionsSuffix')}
                value={guardrailState.min_impressions.input}
                onChange={(value) => handleGuardrailFieldChange('min_impressions', value)}
                inputMode="numeric"
                min={0}
              />
            </div>

            <div className="mt-6 flex flex-wrap items-center gap-3">
              <button
                type="button"
                className="btn btn-primary flex items-center gap-2"
                onClick={() => updateMutation.mutate()}
                disabled={!isDirty || updateMutation.isPending}
              >
                {updateMutation.isPending ? (
                  <Spinner size="sm" label={t('migrationStudio.import.actions.loading')} />
                ) : (
                  <>
                    <Save className="h-4 w-4" aria-hidden={true} />
                    {t('migrationStudio.detail.saveGuardrailsCta')}
                  </>
                )}
              </button>
              <button
                type="button"
                className="btn btn-outline flex items-center gap-2"
                onClick={() => evaluateMutation.mutate()}
                disabled={evaluateMutation.isPending}
              >
                {evaluateMutation.isPending ? (
                  <Loader2 className="h-4 w-4 animate-spin" aria-hidden={true} />
                ) : (
                  <RefreshCcw className="h-4 w-4" aria-hidden={true} />
                )}
                {evaluateMutation.isPending
                  ? t('migrationStudio.actions.evaluateInFlight')
                  : t('migrationStudio.actions.evaluate')}
              </button>
              {guardrailResult && (
                <div
                  className={`flex items-start gap-2 rounded-lg border px-3 py-2 text-sm ${
                    guardrailResult.shouldPause
                      ? 'border-amber-200 bg-amber-50 text-amber-800'
                      : 'border-emerald-200 bg-emerald-50 text-emerald-700'
                  }`}
                  role={guardrailResult.shouldPause ? 'alert' : 'status'}
                  aria-live={guardrailResult.shouldPause ? 'assertive' : 'polite'}
                >
                  {guardrailResult.shouldPause ? (
                    <AlertTriangle className="h-4 w-4" aria-hidden={true} />
                  ) : (
                    <CheckCircle2 className="h-4 w-4" aria-hidden={true} />
                  )}
                  <div>
                    <p className="font-medium">
                      {guardrailResult.shouldPause
                        ? t('migrationStudio.messages.guardrailPaused')
                        : t('migrationStudio.messages.guardrailPassed')}
                    </p>
                    {guardrailResult.violations.length > 0 && (
                      <ul className="mt-2 list-disc pl-4 text-xs">
                        {guardrailResult.violations.map((violation) => (
                          <li key={violation}>{violation}</li>
                        ))}
                      </ul>
                    )}
                  </div>
                </div>
              )}
            </div>
          </div>
        </section>

        <section className="card flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <h2 className="text-lg font-semibold text-gray-900">
              {t('migrationStudio.detail.nextStepsHeading')}
            </h2>
            <p className="mt-1 text-sm text-gray-600">
              {t('migrationStudio.detail.nextStepsCopy')}
            </p>
          </div>
          <Link href="/migration-studio" className="btn btn-outline flex items-center gap-2">
            <ChevronLeft className="h-4 w-4" aria-hidden={true} />
            {t('migrationStudio.detail.backToOverview')}
          </Link>
        </section>

        {/* SDK Integration Section */}
        <SdkIntegrationSection experimentId={experimentId} />

        {/* Complete/Rollback Migration Section */}
        <MigrationActionsSection 
          experiment={experiment} 
          onComplete={async () => {
            const updated = await migrationApi.completeMigration(experiment.id)
            invalidateExperimentQueries(updated)
          }}
          onRollback={async (reason) => {
            const updated = await migrationApi.rollbackMigration(experiment.id, reason)
            invalidateExperimentQueries(updated)
          }}
        />
      </main>
    </div>
  )
}

function SdkIntegrationSection({ experimentId }: { experimentId: string }) {
  const [selectedPlatform, setSelectedPlatform] = useState<string>('ios')
  const [copied, setCopied] = useState(false)

  const { data: sdkIntegration, isLoading, isError } = useQuery({
    queryKey: ['migration-sdk-integration', experimentId],
    queryFn: () => migrationApi.getSdkIntegration(experimentId),
  })

  const handleCopyCode = async () => {
    if (!sdkIntegration?.snippets?.[selectedPlatform]?.code) return
    try {
      await navigator.clipboard.writeText(sdkIntegration.snippets[selectedPlatform].code)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Fallback for older browsers
    }
  }

  if (isLoading) {
    return (
      <section className="card">
        <div className="flex items-center gap-3">
          <Spinner size="sm" label="Loading SDK integration..." />
          <span className="text-sm text-gray-600">Loading SDK integration...</span>
        </div>
      </section>
    )
  }

  if (isError || !sdkIntegration) {
    return null
  }

  const snippet = sdkIntegration.snippets?.[selectedPlatform]
  const platforms = Object.keys(sdkIntegration.snippets || {})

  return (
    <section className="card space-y-6">
      <div>
        <div className="flex items-center gap-2">
          <Code2 className="h-5 w-5 text-primary-600" aria-hidden={true} />
          <h2 className="text-xl font-semibold text-gray-900">SDK Integration</h2>
        </div>
        <p className="mt-1 text-sm text-gray-600 max-w-2xl">
          Integrate the Apex SDK into your app to start routing traffic through the experiment.
        </p>
      </div>

      <div className="bg-slate-50 rounded-lg p-4 space-y-3">
        <div className="flex items-center justify-between">
          <span className="text-sm font-medium text-gray-700">API Endpoint:</span>
          <code className="text-xs bg-white px-2 py-1 rounded border font-mono">
            {sdkIntegration.api_endpoint}
          </code>
        </div>
        <div className="flex items-center justify-between">
          <span className="text-sm font-medium text-gray-700">Placement ID:</span>
          <code className="text-xs bg-white px-2 py-1 rounded border font-mono">
            {sdkIntegration.placement_id}
          </code>
        </div>
        {sdkIntegration.adapters && sdkIntegration.adapters.length > 0 && (
          <div>
            <span className="text-sm font-medium text-gray-700">Configured Adapters:</span>
            <div className="mt-2 flex flex-wrap gap-2">
              {sdkIntegration.adapters.map((adapter: { id: string; name: string }) => (
                <span key={adapter.id} className="inline-flex items-center px-2 py-1 rounded-full text-xs bg-emerald-100 text-emerald-700">
                  {adapter.name}
                </span>
              ))}
            </div>
          </div>
        )}
      </div>

      <div>
        <div className="flex items-center gap-2 mb-4">
          <label htmlFor="platform-select" className="text-sm font-medium text-gray-700">
            Platform:
          </label>
          <select
            id="platform-select"
            value={selectedPlatform}
            onChange={(e) => setSelectedPlatform(e.target.value)}
            className="input w-40"
          >
            {platforms.map((platform) => (
              <option key={platform} value={platform}>
                {platform.charAt(0).toUpperCase() + platform.slice(1)}
              </option>
            ))}
          </select>
        </div>

        {snippet && (
          <>
            <div className="relative">
              <div className="absolute top-2 right-2 z-10">
                <button
                  type="button"
                  className="btn btn-sm btn-ghost flex items-center gap-1"
                  onClick={handleCopyCode}
                >
                  {copied ? (
                    <>
                      <CheckCircle2 className="h-4 w-4 text-emerald-600" />
                      <span className="text-emerald-600">Copied!</span>
                    </>
                  ) : (
                    <>
                      <Copy className="h-4 w-4" />
                      Copy
                    </>
                  )}
                </button>
              </div>
              <pre className="bg-gray-900 text-gray-100 rounded-lg p-4 overflow-x-auto text-xs font-mono leading-relaxed max-h-96">
                <code>{snippet.code}</code>
              </pre>
            </div>

            <div className="mt-4 bg-sky-50 border border-sky-200 rounded-lg p-4">
              <h4 className="text-sm font-semibold text-sky-800">Setup Instructions</h4>
              <p className="mt-2 text-sm text-sky-700 whitespace-pre-line">
                {snippet.instructions}
              </p>
            </div>
          </>
        )}
      </div>
    </section>
  )
}

function MigrationActionsSection({ 
  experiment, 
  onComplete, 
  onRollback 
}: { 
  experiment: MigrationExperiment
  onComplete: () => Promise<void>
  onRollback: (reason?: string) => Promise<void>
}) {
  const [showRollbackConfirm, setShowRollbackConfirm] = useState(false)
  const [showCompleteConfirm, setShowCompleteConfirm] = useState(false)
  const [rollbackReason, setRollbackReason] = useState('')
  const [isCompleting, setIsCompleting] = useState(false)
  const [isRollingBack, setIsRollingBack] = useState(false)

  const isActive = experiment.status === 'active'
  const isCompleted = experiment.status === 'completed'
  const canComplete = isActive && experiment.mirror_percent >= 10
  const canRollback = isActive || isCompleted

  const handleComplete = async () => {
    setIsCompleting(true)
    try {
      await onComplete()
      setShowCompleteConfirm(false)
    } finally {
      setIsCompleting(false)
    }
  }

  const handleRollback = async () => {
    setIsRollingBack(true)
    try {
      await onRollback(rollbackReason || undefined)
      setShowRollbackConfirm(false)
      setRollbackReason('')
    } finally {
      setIsRollingBack(false)
    }
  }

  if (!canComplete && !canRollback) {
    return null
  }

  return (
    <section className="card space-y-6">
      <div>
        <div className="flex items-center gap-2">
          <Rocket className="h-5 w-5 text-primary-600" aria-hidden={true} />
          <h2 className="text-xl font-semibold text-gray-900">Migration Actions</h2>
        </div>
        <p className="mt-1 text-sm text-gray-600 max-w-2xl">
          Complete the migration to route 100% of traffic through Apex, or rollback if needed.
        </p>
      </div>

      <div className="flex flex-wrap gap-4">
        {canComplete && !showCompleteConfirm && (
          <button
            type="button"
            className="btn btn-primary flex items-center gap-2"
            onClick={() => setShowCompleteConfirm(true)}
          >
            <Rocket className="h-4 w-4" aria-hidden={true} />
            Complete Migration (100%)
          </button>
        )}

        {canRollback && !showRollbackConfirm && (
          <button
            type="button"
            className="btn btn-outline text-amber-600 border-amber-300 hover:bg-amber-50 flex items-center gap-2"
            onClick={() => setShowRollbackConfirm(true)}
          >
            <RotateCcw className="h-4 w-4" aria-hidden={true} />
            Rollback Migration
          </button>
        )}
      </div>

      {showCompleteConfirm && (
        <div className="bg-emerald-50 border border-emerald-200 rounded-lg p-4 space-y-4">
          <div className="flex items-start gap-3">
            <CheckCircle2 className="h-5 w-5 text-emerald-600 mt-0.5" aria-hidden={true} />
            <div>
              <h4 className="font-semibold text-emerald-800">Complete Migration?</h4>
              <p className="mt-1 text-sm text-emerald-700">
                This will set traffic to 100% and mark the migration as complete. 
                All ad requests will be routed through Apex adapters.
              </p>
            </div>
          </div>
          <div className="flex gap-3">
            <button
              type="button"
              className="btn btn-primary flex items-center gap-2"
              onClick={handleComplete}
              disabled={isCompleting}
            >
              {isCompleting ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <CheckCircle2 className="h-4 w-4" />
              )}
              {isCompleting ? 'Completing...' : 'Yes, Complete Migration'}
            </button>
            <button
              type="button"
              className="btn btn-ghost"
              onClick={() => setShowCompleteConfirm(false)}
              disabled={isCompleting}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {showRollbackConfirm && (
        <div className="bg-amber-50 border border-amber-200 rounded-lg p-4 space-y-4">
          <div className="flex items-start gap-3">
            <AlertTriangle className="h-5 w-5 text-amber-600 mt-0.5" aria-hidden={true} />
            <div>
              <h4 className="font-semibold text-amber-800">Rollback Migration?</h4>
              <p className="mt-1 text-sm text-amber-700">
                This will pause the experiment and revert to 0% traffic through Apex.
                All ad requests will be routed back through your incumbent mediation.
              </p>
            </div>
          </div>
          <div>
            <label htmlFor="rollback-reason" className="block text-sm font-medium text-amber-800 mb-1">
              Reason (optional):
            </label>
            <input
              id="rollback-reason"
              type="text"
              value={rollbackReason}
              onChange={(e) => setRollbackReason(e.target.value)}
              placeholder="e.g., Revenue dropped unexpectedly"
              className="input w-full"
            />
          </div>
          <div className="flex gap-3">
            <button
              type="button"
              className="btn bg-amber-600 text-white hover:bg-amber-700 flex items-center gap-2"
              onClick={handleRollback}
              disabled={isRollingBack}
            >
              {isRollingBack ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <RotateCcw className="h-4 w-4" />
              )}
              {isRollingBack ? 'Rolling Back...' : 'Yes, Rollback'}
            </button>
            <button
              type="button"
              className="btn btn-ghost"
              onClick={() => {
                setShowRollbackConfirm(false)
                setRollbackReason('')
              }}
              disabled={isRollingBack}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {isCompleted && (
        <div className="bg-sky-50 border border-sky-200 rounded-lg p-4">
          <div className="flex items-start gap-3">
            <CheckCircle2 className="h-5 w-5 text-sky-600 mt-0.5" aria-hidden={true} />
            <div>
              <h4 className="font-semibold text-sky-800">Migration Complete</h4>
              <p className="mt-1 text-sm text-sky-700">
                100% of traffic is now being routed through Apex adapters.
                {experiment.completed_at && (
                  <span className="block mt-1 text-xs">
                    Completed on {formatDate(experiment.completed_at)}
                  </span>
                )}
              </p>
            </div>
          </div>
        </div>
      )}
    </section>
  )
}

function GuardrailField({
  id,
  label,
  suffix,
  value,
  onChange,
  inputMode,
  min,
  step,
}: {
  id: string
  label: string
  suffix: string
  value: string
  onChange: (value: string) => void
  inputMode?: InputHTMLAttributes<HTMLInputElement>['inputMode']
  min?: number
  step?: string
}) {
  const suffixId = `${id}-suffix`

  return (
    <div>
      <label className="block text-sm font-medium text-gray-700" htmlFor={id}>
        {label}
      </label>
      <div className="mt-2 flex items-center gap-2">
        <input
          id={id}
          type="number"
          inputMode={inputMode}
          min={min}
          step={step}
          className="input"
          value={value}
          onChange={(event) => onChange(event.target.value)}
          aria-describedby={suffixId}
        />
        <span className="text-sm text-gray-500" id={suffixId}>
          {suffix}
        </span>
      </div>
    </div>
  )
}

function MetricComparisonCard({ metric }: { metric: MigrationExperimentMetric }) {
  const { label, description, unit, control, test, uplift, sample_size: sampleSize } = metric
  const { Icon, className: upliftClassName, label: upliftLabel } = getUpliftDisplay(uplift)

  return (
    <article className="rounded-xl border border-gray-200 bg-white p-4 shadow-sm">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="text-sm font-semibold text-gray-900">{label}</h3>
          {description && <p className="mt-1 text-xs text-gray-500">{description}</p>}
        </div>
        <div className={`inline-flex items-center gap-1 text-sm font-semibold ${upliftClassName}`}>
          <Icon className="h-4 w-4" aria-hidden={true} />
          <span>{upliftLabel}</span>
        </div>
      </div>

      <dl className="mt-4 grid grid-cols-1 gap-4 text-sm sm:grid-cols-3">
        <div>
          <dt className="text-xs uppercase tracking-wide text-gray-400">
            {t('migrationStudio.detail.metricsControlLabel')}
          </dt>
          <dd className="mt-1 font-medium text-gray-900">{formatMetricValue(control, unit)}</dd>
        </div>
        <div>
          <dt className="text-xs uppercase tracking-wide text-gray-400">
            {t('migrationStudio.detail.metricsTestLabel')}
          </dt>
          <dd className="mt-1 font-medium text-gray-900">{formatMetricValue(test, unit)}</dd>
        </div>
        <div>
          <dt className="text-xs uppercase tracking-wide text-gray-400">
            {t('migrationStudio.detail.metricsUpliftLabel')}
          </dt>
          <dd className={`mt-1 font-semibold ${upliftClassName}`}>{upliftLabel}</dd>
        </div>
      </dl>

      {sampleSize && (sampleSize.control || sampleSize.test) ? (
        <p className="mt-3 text-xs text-gray-500">
          {t('migrationStudio.detail.metricsSample', {
            control: formatNumber(sampleSize.control ?? 0, { maximumFractionDigits: 0 }),
            test: formatNumber(sampleSize.test ?? 0, { maximumFractionDigits: 0 }),
          })}
        </p>
      ) : null}
    </article>
  )
}

function MetricTimeseriesDashboardCard({ metric }: { metric: MigrationExperimentMetricTimeseries }) {
  const controlLabel = t('migrationStudio.detail.metricsTimeseriesControl')
  const testLabel = t('migrationStudio.detail.metricsTimeseriesTest')
  const latestControl = getLatestTimeseriesValue(metric.points, 'control')
  const latestTest = getLatestTimeseriesValue(metric.points, 'test')
  const latestTimestamp = getLatestTimestamp(metric.points)
  const delta = formatTimeseriesDelta(latestControl?.value ?? null, latestTest?.value ?? null, metric.unit)

  return (
    <article className="rounded-xl border border-gray-200 bg-white p-4 shadow-sm space-y-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-gray-900">{metric.label}</h3>
          {latestTimestamp && (
            <p className="mt-1 text-xs text-gray-500">
              {t('migrationStudio.detail.metricsDashboardsLatest', {
                date: formatDate(latestTimestamp),
              })}
            </p>
          )}
        </div>
        {delta ? (
          <span className={`text-xs font-semibold ${delta.className}`}>
            ∆ {delta.label}
          </span>
        ) : null}
      </div>

      <div className="flex flex-wrap gap-4 text-xs text-gray-500">
        <SeriesLegendSwatch
          color={CONTROL_SERIES_COLOR}
          label={controlLabel}
          value={formatMetricValue(latestControl?.value ?? null, metric.unit)}
        />
        <SeriesLegendSwatch
          color={TEST_SERIES_COLOR}
          label={testLabel}
          value={formatMetricValue(latestTest?.value ?? null, metric.unit)}
        />
      </div>

      <MetricTimeseriesChart
        metric={metric}
        height={200}
        showLegend={false}
        showTooltip={false}
        showDots={false}
        dataTestId="metric-timeseries-mini-chart"
      />
    </article>
  )
}

function MetricTimeseriesChart({
  metric,
  height = 320,
  showLegend = true,
  showTooltip = true,
  showDots = true,
  dataTestId = 'metric-timeseries-chart',
}: {
  metric: MigrationExperimentMetricTimeseries
  height?: number
  showLegend?: boolean
  showTooltip?: boolean
  showDots?: boolean
  dataTestId?: string
}) {
  const controlLabel = t('migrationStudio.detail.metricsTimeseriesControl')
  const testLabel = t('migrationStudio.detail.metricsTimeseriesTest')
  const latestControlPoint = getLatestTimeseriesValue(metric.points, 'control')
  const latestTestPoint = getLatestTimeseriesValue(metric.points, 'test')
  const summaryDate = latestControlPoint?.timestamp ?? latestTestPoint?.timestamp ?? null
  const accessibleControl = formatMetricValue(latestControlPoint?.value ?? null, metric.unit)
  const accessibleTest = formatMetricValue(latestTestPoint?.value ?? null, metric.unit)
  const summaryId = `timeseries-summary-${metric.id}-${dataTestId}`
  const summaryCopy = latestControlPoint || latestTestPoint
    ? summaryDate
      ? t('migrationStudio.detail.metricsTimeseriesChartSummary', {
          label: metric.label,
          control: accessibleControl,
          test: accessibleTest,
          date: formatDate(summaryDate),
        })
      : t('migrationStudio.detail.metricsTimeseriesChartSummaryNoDate', {
          label: metric.label,
          control: accessibleControl,
          test: accessibleTest,
        })
    : t('migrationStudio.detail.metricsTimeseriesChartSummaryUnavailable', {
        label: metric.label,
      })

  const chartData = metric.points.map((point) => {
    const normalizedControl = normalizeTimeseriesValue(point.control, metric.unit)
    const normalizedTest = normalizeTimeseriesValue(point.test, metric.unit)
    return {
      timestamp: point.timestamp,
      label: formatDate(point.timestamp, { month: 'short', day: 'numeric' }),
      control: normalizedControl,
      test: normalizedTest,
      rawControl: point.control,
      rawTest: point.test,
    }
  })

  const gradientSuffix = `${metric.id}-${dataTestId}`
  const controlGradientId = `controlGradient-${gradientSuffix}`
  const testGradientId = `testGradient-${gradientSuffix}`

  return (
    <figure aria-labelledby={summaryId} className="w-full">
      <figcaption id={summaryId} className="sr-only">
        {summaryCopy}
      </figcaption>
      <div className="w-full" style={{ height }} data-testid={dataTestId} aria-hidden="true">
        <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={chartData} margin={{ top: 10, right: 24, left: 0, bottom: 0 }}>
          <defs>
            <linearGradient id={controlGradientId} x1="0" y1="0" x2="0" y2="1">
              <stop offset="5%" stopColor={CONTROL_SERIES_COLOR} stopOpacity={0.35} />
              <stop offset="95%" stopColor={CONTROL_SERIES_COLOR} stopOpacity={0} />
            </linearGradient>
            <linearGradient id={testGradientId} x1="0" y1="0" x2="0" y2="1">
              <stop offset="5%" stopColor={TEST_SERIES_COLOR} stopOpacity={0.35} />
              <stop offset="95%" stopColor={TEST_SERIES_COLOR} stopOpacity={0} />
            </linearGradient>
          </defs>
          <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
          <XAxis dataKey="label" stroke="#6b7280" fontSize={12} tickLine={false} />
          <YAxis
            stroke="#6b7280"
            fontSize={12}
            tickLine={false}
            tickFormatter={(value) => formatTimeseriesAxisTick(value as number, metric.unit)}
          />
          {showTooltip ? (
            <Tooltip
              contentStyle={{ backgroundColor: '#fff', border: '1px solid #e5e7eb', borderRadius: '8px' }}
              labelFormatter={(label) => formatDate(label as string)}
              formatter={(value: any, name: any, payload: any) => {
                if (!payload?.payload) {
                  return [value ?? '—', name]
                }
                const rawValue = name === controlLabel ? payload.payload.rawControl : payload.payload.rawTest
                return [formatMetricValue(rawValue ?? null, metric.unit), name]
              }}
            />
          ) : null}
          {showLegend ? <Legend /> : null}
          <Area
            type="monotone"
            dataKey="control"
            name={controlLabel}
            stroke={CONTROL_SERIES_COLOR}
            strokeWidth={2}
            fill={`url(#${controlGradientId})`}
            connectNulls
            dot={showDots ? { r: 3, fill: CONTROL_SERIES_COLOR } : false}
          />
          <Area
            type="monotone"
            dataKey="test"
            name={testLabel}
            stroke={TEST_SERIES_COLOR}
            strokeWidth={2}
            fill={`url(#${testGradientId})`}
            connectNulls
            dot={showDots ? { r: 3, fill: TEST_SERIES_COLOR } : false}
          />
        </AreaChart>
      </ResponsiveContainer>
      </div>
    </figure>
  )
}

function SeriesLegendSwatch({
  color,
  label,
  value,
}: {
  color: string
  label: string
  value: string
}) {
  return (
    <span className="inline-flex items-center gap-2">
      <span className="flex items-center gap-1.5">
        <span className="h-2.5 w-2.5 rounded-full" style={{ backgroundColor: color }} aria-hidden={true} />
        <span className="text-xs font-medium text-gray-500">{label}</span>
      </span>
      <span className="text-xs font-semibold text-gray-900">{value}</span>
    </span>
  )
}

function getLatestTimeseriesValue(
  points: MigrationExperimentTimeseriesPoint[],
  key: 'control' | 'test'
): { value: number; timestamp: string } | null {
  for (let index = points.length - 1; index >= 0; index -= 1) {
    const point = points[index]
    const rawValue = point[key]
    if (rawValue === null || rawValue === undefined || Number.isNaN(rawValue)) {
      continue
    }
    return { value: rawValue, timestamp: point.timestamp }
  }
  return null
}

function getLatestTimestamp(points: MigrationExperimentTimeseriesPoint[]): string | null {
  const fromControl = getLatestTimeseriesValue(points, 'control')
  if (fromControl) return fromControl.timestamp
  const fromTest = getLatestTimeseriesValue(points, 'test')
  return fromTest ? fromTest.timestamp : null
}

function formatTimeseriesDelta(
  control: number | null,
  test: number | null,
  unit: MigrationMetricUnit
): { label: string; className: string } | null {
  if (
    control === null ||
    control === undefined ||
    test === null ||
    test === undefined ||
    Number.isNaN(control) ||
    Number.isNaN(test)
  ) {
    return null
  }

  const diff = test - control
  if (!Number.isFinite(diff)) {
    return null
  }

  if (diff === 0) {
    return {
      label: formatMetricValue(0, unit),
      className: 'text-slate-500',
    }
  }

  const formatted = formatMetricValue(Math.abs(diff), unit)
  const prefix = diff > 0 ? '+' : '-'

  return {
    label: `${prefix}${formatted}`,
    className: diff > 0 ? 'text-emerald-600' : 'text-rose-600',
  }
}

function formatMetricValue(value: number | null, unit: MigrationMetricUnit): string {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return '—'
  }

  switch (unit) {
    case 'currency_cents':
      return formatCurrency(value, 'USD', { fromMinorUnits: true })
    case 'percent':
      return formatPercentage(value, 1)
    case 'milliseconds':
      return `${formatNumber(value, { maximumFractionDigits: 0 })} ms`
    case 'ratio':
      return formatNumber(value, { minimumFractionDigits: 2, maximumFractionDigits: 4 })
    case 'count':
      return formatNumber(value, { maximumFractionDigits: 0 })
    default:
      return formatNumber(value)
  }
}

function normalizeTimeseriesValue(value: number | null, unit: MigrationMetricUnit): number | null {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return null
  }

  switch (unit) {
    case 'currency_cents':
      return value / 100
    default:
      return value
  }
}

function formatTimeseriesAxisTick(value: number | null, unit: MigrationMetricUnit): string {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return '—'
  }

  switch (unit) {
    case 'currency_cents':
      return formatCurrency(value)
    case 'percent':
      return formatPercentage(value, 0)
    case 'milliseconds':
      return `${formatNumber(value, { maximumFractionDigits: 0 })}`
    case 'ratio':
      return formatNumber(value, { minimumFractionDigits: 2, maximumFractionDigits: 3 })
    case 'count':
      return formatNumber(value, { maximumFractionDigits: 0 })
    default:
      return formatNumber(value)
  }
}

function getUpliftDisplay(uplift: number | null): {
  label: string
  className: string
  Icon: typeof TrendingUp
} {
  if (uplift === null || uplift === undefined || Number.isNaN(uplift)) {
    return { label: '—', className: 'text-slate-500', Icon: Minus }
  }

  if (uplift > 0) {
    return {
      label: `+${formatPercentage(uplift, 1)}`,
      className: 'text-emerald-600',
      Icon: TrendingUp,
    }
  }

  if (uplift < 0) {
    return {
      label: `-${formatPercentage(Math.abs(uplift), 1)}`,
      className: 'text-rose-600',
      Icon: TrendingDown,
    }
  }

  return {
    label: formatPercentage(0, 1),
    className: 'text-slate-500',
    Icon: Minus,
  }
}
