'use client'

import { useEffect, useState } from 'react'
import Link from 'next/link'
import { useForm, useFieldArray } from 'react-hook-form'
import { z } from 'zod'
import { zodResolver } from '@hookform/resolvers/zod'
import { useMutation, useQuery } from '@tanstack/react-query'
import { settingsApi } from '@/lib/api'
import { ArrowLeft, BellRing, Save, Plus, Trash2, AlertCircle, CheckCircle } from 'lucide-react'
import type { NotificationSettings, WebhookEndpoint } from '@/types'

const emailListToArray = (value: string) => value.split(',').map((email) => email.trim()).filter(Boolean)

const notificationSchema = z
  .object({
    emailEnabled: z.boolean(),
    emailRecipients: z.string(),
    emailEvents: z.array(z.string()),
    slackEnabled: z.boolean(),
    slackWebhook: z.string().url('Enter a valid Slack webhook URL').optional().or(z.literal('')),
    slackChannel: z.string().optional().or(z.literal('')),
    slackEvents: z.array(z.string()),
    webhooksEnabled: z.boolean(),
    webhookEndpoints: z
      .array(
        z.object({
          id: z.string(),
          url: z.string().url('Enter a valid webhook URL'),
          events: z.array(z.string()).min(1, 'Select at least one event'),
          secret: z.string().optional(),
          active: z.boolean().optional(),
          createdAt: z.string().optional(),
        })
      )
      .default([]),
    digestEnabled: z.boolean(),
    digestFrequency: z.enum(['daily', 'weekly', 'monthly']),
    digestRecipients: z.string(),
  })
  .superRefine((values, ctx) => {
    if (values.emailEnabled && emailListToArray(values.emailRecipients).length === 0) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Provide at least one email recipient.',
        path: ['emailRecipients'],
      })
    }

    if (values.digestEnabled && emailListToArray(values.digestRecipients).length === 0) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Provide at least one digest recipient.',
        path: ['digestRecipients'],
      })
    }

    if (values.slackEnabled && !values.slackWebhook) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Slack webhook URL is required when Slack integration is enabled.',
        path: ['slackWebhook'],
      })
    }

    if (values.webhooksEnabled && values.webhookEndpoints.length === 0) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Add at least one webhook endpoint or disable custom webhooks.',
        path: ['webhookEndpoints'],
      })
    }
  })

type NotificationFormValues = z.infer<typeof notificationSchema>
type FormWebhookEndpoint = NotificationFormValues['webhookEndpoints'][number]

const normalizeWebhookEndpoint = (endpoint: FormWebhookEndpoint): WebhookEndpoint => ({
  id: endpoint.id,
  url: endpoint.url,
  events: endpoint.events,
  secret: endpoint.secret ?? '••••••••',
  active: endpoint.active ?? true,
  createdAt: endpoint.createdAt ?? new Date().toISOString(),
})

const availableEvents = [
  { id: 'revenue_milestone', label: 'Revenue Milestones', description: 'Daily/weekly revenue targets' },
  { id: 'fraud_alert', label: 'Fraud Alerts', description: 'Suspicious traffic detected' },
  { id: 'placement_pause', label: 'Placement Paused', description: 'Auto-pause due to quality issues' },
  { id: 'payout_ready', label: 'Payout Ready', description: 'Payment scheduled or completed' },
  { id: 'adapter_failure', label: 'Adapter Failures', description: 'Network integration errors' },
  { id: 'team_changes', label: 'Team Changes', description: 'Member invites and removals' },
]

export default function NotificationsSettingsPage() {
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)
  const [newWebhook, setNewWebhook] = useState({ url: '', events: [] as string[] })
  const [webhookError, setWebhookError] = useState<string | null>(null)

  const { data, isLoading } = useQuery({
    queryKey: ['settings', 'notifications'],
    queryFn: async () => {
      const { data } = await settingsApi.getNotificationSettings()
      return data
    },
  })

  const form = useForm<NotificationFormValues>({
    resolver: zodResolver(notificationSchema),
    defaultValues: {
      emailEnabled: true,
      emailRecipients: '',
      emailEvents: ['fraud_alert', 'payout_ready'],
      slackEnabled: false,
      slackWebhook: '',
      slackChannel: '',
      slackEvents: [],
      webhooksEnabled: false,
      webhookEndpoints: [],
      digestEnabled: true,
      digestFrequency: 'weekly',
      digestRecipients: '',
    },
  })

  const {
    fields: webhookFields,
    append: appendWebhookField,
    remove: removeWebhookField,
    replace: replaceWebhookFields,
  } = useFieldArray({
    control: form.control,
    name: 'webhookEndpoints',
  })

  const webhookEndpointsErrorMessage =
    form.formState.errors.webhookEndpoints && !Array.isArray(form.formState.errors.webhookEndpoints)
      ? form.formState.errors.webhookEndpoints.message
      : undefined

  useEffect(() => {
    if (data) {
      const emailAlerts = data.emailAlerts ?? { enabled: false, recipients: [], events: [] }
      const slackIntegration = data.slackIntegration ?? { enabled: false, webhookUrl: undefined, channel: undefined, events: [] }
      const webhooks = data.webhooks ?? { enabled: false, endpoints: [] }
      const digest = data.digest ?? { enabled: false, frequency: 'weekly' as const, recipients: [] }

      form.reset({
        emailEnabled: Boolean(emailAlerts.enabled),
        emailRecipients: (Array.isArray(emailAlerts.recipients) ? emailAlerts.recipients : []).join(', '),
        emailEvents: Array.isArray(emailAlerts.events) ? emailAlerts.events : [],
        slackEnabled: Boolean(slackIntegration.enabled),
        slackWebhook: slackIntegration.webhookUrl ?? '',
        slackChannel: slackIntegration.channel ?? '',
        slackEvents: Array.isArray(slackIntegration.events) ? slackIntegration.events : [],
        webhooksEnabled: Boolean(webhooks.enabled),
        webhookEndpoints: Array.isArray(webhooks.endpoints) ? webhooks.endpoints : [],
        digestEnabled: Boolean(digest.enabled),
        digestFrequency: digest.frequency ?? 'weekly',
        digestRecipients: (Array.isArray(digest.recipients) ? digest.recipients : []).join(', '),
      })
    }
  }, [data, form])

  const updateMutation = useMutation({
    mutationFn: async (values: NotificationFormValues) => {
      await settingsApi.updateNotificationSettings({
        emailAlerts: {
          enabled: values.emailEnabled,
          recipients: values.emailRecipients.split(',').map((e) => e.trim()).filter(Boolean),
          events: values.emailEvents,
        },
        slackIntegration: {
          enabled: values.slackEnabled,
          webhookUrl: values.slackWebhook || undefined,
          channel: values.slackChannel || undefined,
          events: values.slackEvents,
        },
        webhooks: {
          enabled: values.webhooksEnabled,
          endpoints: values.webhookEndpoints.map(normalizeWebhookEndpoint),
        },
        digest: {
          enabled: values.digestEnabled,
          frequency: values.digestFrequency,
          recipients: values.digestRecipients.split(',').map((e) => e.trim()).filter(Boolean),
        },
      })
    },
    onMutate: () => setMessage(null),
    onSuccess: () => setMessage({ type: 'success', text: 'Notification settings updated successfully.' }),
    onError: () => setMessage({ type: 'error', text: 'Failed to update notification settings. Please try again.' }),
  })

  const persistWebhooks = async (endpoints: FormWebhookEndpoint[]) => {
    const normalized = endpoints.map(normalizeWebhookEndpoint)
    try {
      await settingsApi.updateNotificationSettings({
        webhooks: {
          enabled: form.getValues('webhooksEnabled'),
          endpoints: normalized,
        },
      })
      setMessage({ type: 'success', text: 'Webhook endpoints updated.' })
    } catch (error) {
      console.error('Failed to update webhooks', error)
      setMessage({ type: 'error', text: 'Failed to update webhooks. Please try again.' })
      throw error
    }
  }

  const addWebhook = async () => {
    setWebhookError(null)
    const trimmedUrl = newWebhook.url.trim()
    if (!trimmedUrl) {
      setWebhookError('Webhook URL is required.')
      return
    }
    if (newWebhook.events.length === 0) {
      setWebhookError('Select at least one event for the webhook.')
      return
    }

    const currentEndpoints = form.getValues('webhookEndpoints') as FormWebhookEndpoint[]
    if (currentEndpoints.some((endpoint) => endpoint.url.toLowerCase() === trimmedUrl.toLowerCase())) {
      setWebhookError('Webhook URL must be unique.')
      return
    }

    const endpoint: FormWebhookEndpoint = {
      id: `wh_${Date.now()}`,
      url: trimmedUrl,
      events: [...newWebhook.events],
      secret: '••••••••',
      active: true,
      createdAt: new Date().toISOString(),
    }

    const updated = [...currentEndpoints, endpoint]
    appendWebhookField(endpoint)
    setNewWebhook({ url: '', events: [] })

    try {
      await persistWebhooks(updated)
    } catch (error) {
      removeWebhookField(updated.length - 1)
    }
  }

  const handleRemoveWebhook = async (index: number) => {
    if (!window.confirm('Remove this webhook endpoint?')) return
    const currentEndpoints = form.getValues('webhookEndpoints') as FormWebhookEndpoint[]
    const updated = currentEndpoints.filter((_, idx) => idx !== index)
    removeWebhookField(index)
    try {
      await persistWebhooks(updated)
    } catch (error) {
      replaceWebhookFields(currentEndpoints)
    }
  }

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
              <BellRing className="h-6 w-6" aria-hidden={true} />
            </div>
            <div>
              <p className="text-sm font-medium text-primary-600">Alerts & Channels</p>
              <h1 className="text-3xl font-semibold text-gray-900 tracking-tight">Notifications</h1>
              <p className="text-sm text-gray-600 mt-1">
                Configure email, Slack, and webhook alerts for critical events.
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
            <div className="flex items-center justify-between">
              <div>
                <h2 className="text-lg font-semibold text-gray-900">Email Alerts</h2>
                <p className="text-sm text-gray-600">Receive critical notifications via email.</p>
              </div>
              <input
                type="checkbox"
                className="h-5 w-5 accent-primary-600"
                {...form.register('emailEnabled')}
              />
            </div>
            {form.watch('emailEnabled') && (
              <>
                <div>
                  <label htmlFor="emailRecipients" className="label">
                    Recipients
                  </label>
                  <input
                    id="emailRecipients"
                    type="text"
                    className="input"
                    placeholder="ops@example.com, finance@example.com"
                    {...form.register('emailRecipients')}
                  />
                  <p className="text-xs text-gray-500 mt-1">Separate multiple emails with commas.</p>
                </div>
                <div>
                  <p className="label">Events to Monitor</p>
                  <div className="space-y-2">
                    {availableEvents.map((event) => (
                      <label
                        key={event.id}
                        className="flex items-start gap-3 p-3 border rounded hover:bg-gray-50 cursor-pointer"
                        aria-label={`Toggle ${event.label} email alert`}
                      >
                        <input
                          type="checkbox"
                          value={event.id}
                          className="mt-1"
                          {...form.register('emailEvents')}
                        />
                        <div>
                          <p className="text-sm font-medium text-gray-900">{event.label}</p>
                          <p className="text-xs text-gray-600">{event.description}</p>
                        </div>
                      </label>
                    ))}
                  </div>
                </div>
              </>
            )}
          </section>

          <section className="card space-y-6">
            <div className="flex items-center justify-between">
              <div>
                <h2 className="text-lg font-semibold text-gray-900">Slack Integration</h2>
                <p className="text-sm text-gray-600">Push alerts directly to your Slack workspace.</p>
              </div>
              <input
                type="checkbox"
                className="h-5 w-5 accent-primary-600"
                {...form.register('slackEnabled')}
              />
            </div>
            {form.watch('slackEnabled') && (
              <>
                <div>
                  <label htmlFor="slackWebhook" className="label">
                    Webhook URL
                  </label>
                  <input
                    id="slackWebhook"
                    type="url"
                    className="input"
                    placeholder="https://hooks.slack.com/services/..."
                    {...form.register('slackWebhook')}
                  />
                  {form.formState.errors.slackWebhook && (
                    <p className="text-sm text-danger-600 mt-1">{form.formState.errors.slackWebhook.message}</p>
                  )}
                </div>
                <div>
                  <label htmlFor="slackChannel" className="label">
                    Channel (Optional)
                  </label>
                  <input
                    id="slackChannel"
                    type="text"
                    className="input"
                    placeholder="#ad-alerts"
                    {...form.register('slackChannel')}
                  />
                </div>
                <div>
                  <p className="label">Events to Monitor</p>
                  <div className="space-y-2">
                    {availableEvents.map((event) => (
                      <label
                        key={event.id}
                        className="flex items-start gap-3 p-3 border rounded hover:bg-gray-50 cursor-pointer"
                        aria-label={`Toggle ${event.label} Slack alert`}
                      >
                        <input
                          type="checkbox"
                          value={event.id}
                          className="mt-1"
                          {...form.register('slackEvents')}
                        />
                        <div>
                          <p className="text-sm font-medium text-gray-900">{event.label}</p>
                          <p className="text-xs text-gray-600">{event.description}</p>
                        </div>
                      </label>
                    ))}
                  </div>
                </div>
              </>
            )}
          </section>

          <section className="card space-y-6">
            <div className="flex items-center justify-between">
              <div>
                <h2 className="text-lg font-semibold text-gray-900">Custom Webhooks</h2>
                <p className="text-sm text-gray-600">Send event payloads to your own endpoints.</p>
              </div>
              <input
                type="checkbox"
                className="h-5 w-5 accent-primary-600"
                {...form.register('webhooksEnabled')}
              />
            </div>
            {form.watch('webhooksEnabled') && (
              <>
                <div className="space-y-4">
                  {webhookFields.map((wh, index) => (
                    <div key={wh.id} className="flex items-center gap-4 p-4 border rounded-lg">
                      <div className="flex-1">
                        <p className="text-sm font-medium text-gray-900">{wh.url}</p>
                        <p className="text-xs text-gray-600 mt-1">{wh.events.join(', ')}</p>
                      </div>
                      <button
                        type="button"
                        onClick={() => handleRemoveWebhook(index)}
                        className="p-2 text-danger-600 hover:bg-danger-50 rounded"
                        aria-label="Remove webhook"
                      >
                        <Trash2 className="h-4 w-4" aria-hidden={true} />
                      </button>
                    </div>
                  ))}
                </div>
                {webhookEndpointsErrorMessage && (
                  <p className="text-sm text-danger-600">{webhookEndpointsErrorMessage}</p>
                )}
                <div className="border-t pt-4">
                  <p className="text-sm font-medium text-gray-900 mb-3">Add New Webhook</p>
                  <div className="space-y-3">
                    <input
                      type="url"
                      className="input"
                      placeholder="https://api.example.com/webhooks/ads"
                      value={newWebhook.url}
                      onChange={(e) => setNewWebhook({ ...newWebhook, url: e.target.value })}
                    />
                    {webhookError && <p className="text-sm text-danger-600">{webhookError}</p>}
                    <div className="grid grid-cols-2 gap-2">
                      {availableEvents.map((event) => (
                        <label key={event.id} className="flex items-center gap-2 text-sm">
                          <input
                            type="checkbox"
                            checked={newWebhook.events.includes(event.id)}
                            onChange={(e) => {
                              if (e.target.checked) {
                                setNewWebhook({ ...newWebhook, events: [...newWebhook.events, event.id] })
                              } else {
                                setNewWebhook({
                                  ...newWebhook,
                                  events: newWebhook.events.filter((ev) => ev !== event.id),
                                })
                              }
                            }}
                          />
                          {event.label}
                        </label>
                      ))}
                    </div>
                    <button type="button" onClick={addWebhook} className="btn btn-outline flex items-center gap-2">
                      <Plus className="h-4 w-4" aria-hidden={true} />
                      Add Webhook
                    </button>
                  </div>
                </div>
              </>
            )}
          </section>

          <section className="card space-y-6">
            <div className="flex items-center justify-between">
              <div>
                <h2 className="text-lg font-semibold text-gray-900">Performance Digest</h2>
                <p className="text-sm text-gray-600">Scheduled summary reports of key metrics.</p>
              </div>
              <input
                type="checkbox"
                className="h-5 w-5 accent-primary-600"
                {...form.register('digestEnabled')}
              />
            </div>
            {form.watch('digestEnabled') && (
              <>
                <div>
                  <label htmlFor="digestFrequency" className="label">
                    Frequency
                  </label>
                  <select id="digestFrequency" className="input" {...form.register('digestFrequency')}>
                    <option value="daily">Daily</option>
                    <option value="weekly">Weekly</option>
                    <option value="monthly">Monthly</option>
                  </select>
                </div>
                <div>
                  <label htmlFor="digestRecipients" className="label">
                    Recipients
                  </label>
                  <input
                    id="digestRecipients"
                    type="text"
                    className="input"
                    placeholder="team@example.com"
                    {...form.register('digestRecipients')}
                  />
                </div>
              </>
            )}
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
