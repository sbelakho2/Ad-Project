'use client'

import { useState, useCallback } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { 
  RefreshCcw, Power, Settings, Mail, Users, CheckCircle, XCircle, 
  AlertTriangle, Play, Eye, Send, Shield, Clock, BarChart3,
  FileText, ChevronDown, ChevronUp
} from 'lucide-react'
import {
  getSalesStatus,
  getSalesConfig,
  updateSalesConfig,
  toggleSalesSystem,
  getSalesSequences,
  getSalesContacts,
  getDraftEmails,
  approveDraftEmail,
  rejectDraftEmail,
  bulkApproveDrafts,
  runSalesOutreach,
  type SalesAutomationConfig,
  type SalesAutomationStatus,
  type SalesSequence,
  type SalesContact,
  type SalesEmail,
} from '@/lib/admin'
import { Section, Container } from '@/components/ui'

export default function SalesControlPanel() {
  const queryClient = useQueryClient()
  const [activeTab, setActiveTab] = useState<'overview' | 'config' | 'sequences' | 'drafts' | 'contacts'>('overview')

  const { data: status, isLoading, refetch } = useQuery({
    queryKey: ['sales-status'],
    queryFn: ({ signal }) => getSalesStatus(signal),
    refetchInterval: 30000,
  })

  if (isLoading) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50">
        <div className="text-sm text-gray-600">Loading sales control panel…</div>
      </div>
    )
  }

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="border-b bg-white">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div>
              <p className="text-sm font-medium text-primary-600">Operations</p>
              <h1 className="text-2xl font-semibold text-gray-900">Sales Automation Control</h1>
              <p className="text-sm text-gray-600 mt-1">
                Manage sender settings, review emails, and control automation behavior
              </p>
            </div>
            <div className="flex items-center gap-3">
              <SystemToggle status={status} onRefresh={() => refetch()} />
            </div>
          </div>
        </div>

        {/* Tabs */}
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <nav className="flex gap-6 -mb-px">
            {[
              { id: 'overview', label: 'Overview', icon: BarChart3 },
              { id: 'config', label: 'Configuration', icon: Settings },
              { id: 'sequences', label: 'Sequences', icon: Mail },
              { id: 'drafts', label: 'Review Queue', icon: FileText },
              { id: 'contacts', label: 'Contacts', icon: Users },
            ].map(tab => (
              <button
                key={tab.id}
                onClick={() => setActiveTab(tab.id as typeof activeTab)}
                className={`flex items-center gap-2 px-1 py-3 text-sm font-medium border-b-2 transition ${
                  activeTab === tab.id
                    ? 'border-primary-600 text-primary-600'
                    : 'border-transparent text-gray-500 hover:text-gray-700'
                }`}
              >
                <tab.icon className="h-4 w-4" />
                {tab.label}
              </button>
            ))}
          </nav>
        </div>
      </header>

      <Section>
        <Container className="py-6">
          {activeTab === 'overview' && <OverviewTab status={status} />}
          {activeTab === 'config' && <ConfigTab />}
          {activeTab === 'sequences' && <SequencesTab />}
          {activeTab === 'drafts' && <DraftsTab />}
          {activeTab === 'contacts' && <ContactsTab />}
        </Container>
      </Section>
    </div>
  )
}

// ============================================================================
// SYSTEM TOGGLE
// ============================================================================

function SystemToggle({ status, onRefresh }: { status?: SalesAutomationStatus; onRefresh: () => void }) {
  const queryClient = useQueryClient()
  
  const toggleMutation = useMutation({
    mutationFn: (enabled: boolean) => toggleSalesSystem(enabled),
    onMutate: async (enabled: boolean) => {
      await queryClient.cancelQueries({ queryKey: ['sales-status'] })
      const previous = queryClient.getQueryData(['sales-status'])
      queryClient.setQueryData(['sales-status'], (current: SalesAutomationStatus | undefined) => {
        if (!current) return current
        return { ...current, enabled }
      })
      return { previous }
    },
    onError: (_err, _enabled, context) => {
      if (context?.previous) {
        queryClient.setQueryData(['sales-status'], context.previous)
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['sales-status'] })
      onRefresh()
    },
  })

  const isEnabled = status?.enabled ?? false

  return (
    <div className="flex items-center gap-4">
      <button
        onClick={onRefresh}
        className="p-2 text-gray-500 hover:text-gray-700 rounded-md border hover:bg-gray-50"
        title="Refresh"
      >
        <RefreshCcw className="h-4 w-4" />
      </button>
      <button
        onClick={() => toggleMutation.mutate(!isEnabled)}
        disabled={toggleMutation.isPending}
        className={`flex items-center gap-2 px-4 py-2 rounded-md text-sm font-medium transition ${
          isEnabled
            ? 'bg-green-100 text-green-700 hover:bg-green-200'
            : 'bg-red-100 text-red-700 hover:bg-red-200'
        }`}
      >
        <Power className="h-4 w-4" />
        {isEnabled ? 'System ON' : 'System OFF'}
      </button>
    </div>
  )
}

// ============================================================================
// OVERVIEW TAB
// ============================================================================

function OverviewTab({ status }: { status?: SalesAutomationStatus }) {
  const runMutation = useMutation({
    mutationFn: (dryRun: boolean) => runSalesOutreach(dryRun),
  })

  if (!status) return null

  return (
    <div className="space-y-6">
      {/* Status Cards */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
        <StatusCard
          title="System Status"
          value={status.enabled ? 'Enabled' : 'Disabled'}
          icon={Power}
          color={status.enabled ? 'green' : 'red'}
        />
        <StatusCard
          title="Today Sent"
          value={`${status.todaySent} / ${status.dailyLimit}`}
          icon={Send}
          color="blue"
          subtitle={`${status.todayRemaining} remaining`}
        />
        <StatusCard
          title="Resend API"
          value={status.resendConfigured ? 'Connected' : 'Not Configured'}
          icon={Mail}
          color={status.resendConfigured ? 'green' : 'yellow'}
        />
        <StatusCard
          title="Review Queue"
          value={status.config.enableReviewQueue ? 'Active' : 'Disabled'}
          icon={Eye}
          color={status.config.enableReviewQueue ? 'purple' : 'gray'}
        />
      </div>

      {/* Quick Actions */}
      <div className="bg-white rounded-xl border p-6">
        <h3 className="text-lg font-semibold text-gray-900 mb-4">Quick Actions</h3>
        <div className="flex flex-wrap gap-3">
          <button
            onClick={() => runMutation.mutate(true)}
            disabled={runMutation.isPending}
            className="flex items-center gap-2 px-4 py-2 bg-blue-50 text-blue-700 rounded-md hover:bg-blue-100 text-sm font-medium"
          >
            <Eye className="h-4 w-4" />
            Dry Run (Preview)
          </button>
          <button
            onClick={() => runMutation.mutate(false)}
            disabled={runMutation.isPending || !status.enabled}
            className="flex items-center gap-2 px-4 py-2 bg-green-50 text-green-700 rounded-md hover:bg-green-100 text-sm font-medium disabled:opacity-50"
          >
            <Play className="h-4 w-4" />
            Run Outreach Cycle
          </button>
        </div>

        {runMutation.data && (
          <div className="mt-4 p-4 bg-gray-50 rounded-lg">
            <p className="text-sm font-medium text-gray-900">
              {runMutation.data.dryRun ? 'Dry Run' : 'Outreach'} Results:
            </p>
            <ul className="mt-2 text-sm text-gray-600 space-y-1">
              <li>Processed: {runMutation.data.processed}</li>
              <li>Sent: {runMutation.data.sent}</li>
              <li>Skipped (Spam): {runMutation.data.skippedSpam}</li>
              <li>Domain Health: {runMutation.data.domainHealth.status}</li>
              {runMutation.data.errors.length > 0 && (
                <li className="text-red-600">Errors: {runMutation.data.errors.join(', ')}</li>
              )}
            </ul>
          </div>
        )}
      </div>

      {/* Current Config Summary */}
      <div className="bg-white rounded-xl border p-6">
        <h3 className="text-lg font-semibold text-gray-900 mb-4">Current Configuration</h3>
        <dl className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
          <div>
            <dt className="text-gray-500">Sender Name</dt>
            <dd className="font-medium text-gray-900">{status.config.fromName}</dd>
          </div>
          <div>
            <dt className="text-gray-500">Sender Email</dt>
            <dd className="font-medium text-gray-900">{status.config.fromEmail}</dd>
          </div>
          <div>
            <dt className="text-gray-500">Daily Limit</dt>
            <dd className="font-medium text-gray-900">{status.config.dailyLimit}</dd>
          </div>
          <div>
            <dt className="text-gray-500">Require Verification</dt>
            <dd className="font-medium text-gray-900">{status.config.requireEmailVerification ? 'Yes' : 'No'}</dd>
          </div>
        </dl>
      </div>
    </div>
  )
}

function StatusCard({ title, value, icon: Icon, color, subtitle }: {
  title: string
  value: string
  icon: React.ElementType
  color: 'green' | 'red' | 'blue' | 'yellow' | 'purple' | 'gray'
  subtitle?: string
}) {
  const colors = {
    green: 'bg-green-50 text-green-700',
    red: 'bg-red-50 text-red-700',
    blue: 'bg-blue-50 text-blue-700',
    yellow: 'bg-yellow-50 text-yellow-700',
    purple: 'bg-purple-50 text-purple-700',
    gray: 'bg-gray-50 text-gray-700',
  }

  return (
    <div className={`rounded-xl border p-4 ${colors[color]}`}>
      <div className="flex items-center gap-2 mb-2">
        <Icon className="h-4 w-4" />
        <span className="text-xs font-medium uppercase tracking-wide">{title}</span>
      </div>
      <p className="text-xl font-semibold">{value}</p>
      {subtitle && <p className="text-xs mt-1 opacity-75">{subtitle}</p>}
    </div>
  )
}

// ============================================================================
// CONFIG TAB
// ============================================================================

function ConfigTab() {
  const queryClient = useQueryClient()
  const [formData, setFormData] = useState<Partial<SalesAutomationConfig>>({})

  const { data: config, isLoading } = useQuery({
    queryKey: ['sales-config'],
    queryFn: ({ signal }) => getSalesConfig(signal),
  })

  const updateMutation = useMutation({
    mutationFn: (updates: Partial<SalesAutomationConfig>) => updateSalesConfig(updates),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['sales-config'] })
      queryClient.invalidateQueries({ queryKey: ['sales-status'] })
      setFormData({})
    },
  })

  if (isLoading || !config) {
    return <div className="text-sm text-gray-500">Loading configuration…</div>
  }

  const hasChanges = Object.keys(formData).length > 0

  return (
    <div className="space-y-6">
      {/* Sender Settings */}
      <div className="bg-white rounded-xl border p-6">
        <h3 className="text-lg font-semibold text-gray-900 mb-4 flex items-center gap-2">
          <Mail className="h-5 w-5" />
          Sender Settings
        </h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div>
            <label htmlFor="sa-from-name" className="block text-sm font-medium text-gray-700 mb-1">Sender Name</label>
            <input
              id="sa-from-name"
              type="text"
              value={formData.fromName ?? config.fromName}
              onChange={e => setFormData(prev => ({ ...prev, fromName: e.target.value }))}
              className="w-full px-3 py-2 border rounded-md text-sm"
              placeholder="Aaron from ApexMediation"
            />
          </div>
          <div>
            <label htmlFor="sa-from-email" className="block text-sm font-medium text-gray-700 mb-1">Sender Email</label>
            <input
              id="sa-from-email"
              type="email"
              value={formData.fromEmail ?? config.fromEmail}
              onChange={e => setFormData(prev => ({ ...prev, fromEmail: e.target.value }))}
              className="w-full px-3 py-2 border rounded-md text-sm"
              placeholder="sales@apexmediation.ee"
            />
          </div>
          <div>
            <label htmlFor="sa-reply-to" className="block text-sm font-medium text-gray-700 mb-1">Reply-To Email</label>
            <input
              id="sa-reply-to"
              type="email"
              value={formData.replyTo ?? config.replyTo}
              onChange={e => setFormData(prev => ({ ...prev, replyTo: e.target.value }))}
              className="w-full px-3 py-2 border rounded-md text-sm"
              placeholder="sales@apexmediation.ee"
            />
          </div>
          <div>
            <label htmlFor="sa-daily-limit" className="block text-sm font-medium text-gray-700 mb-1">Daily Send Limit</label>
            <input
              id="sa-daily-limit"
              type="number"
              value={formData.dailyLimit ?? config.dailyLimit}
              onChange={e => setFormData(prev => ({ ...prev, dailyLimit: parseInt(e.target.value) }))}
              className="w-full px-3 py-2 border rounded-md text-sm"
              min={1}
              max={500}
            />
          </div>
        </div>
      </div>

      {/* Safety Settings */}
      <div className="bg-white rounded-xl border p-6">
        <h3 className="text-lg font-semibold text-gray-900 mb-4 flex items-center gap-2">
          <Shield className="h-5 w-5" />
          Safety Settings
        </h3>
        <div className="space-y-4">
          <div className="flex items-center gap-3">
            <input
              id="sa-require-verify"
              type="checkbox"
              checked={formData.requireEmailVerification ?? config.requireEmailVerification}
              onChange={e => setFormData(prev => ({ ...prev, requireEmailVerification: e.target.checked }))}
              className="h-4 w-4 text-primary-600 rounded"
            />
            <label htmlFor="sa-require-verify">
              <span className="text-sm font-medium text-gray-900">Require Email Verification</span>
              <p className="text-xs text-gray-500">Only send to contacts with verified emails</p>
            </label>
          </div>
          <div className="flex items-center gap-3">
            <input
              id="sa-review-queue"
              type="checkbox"
              checked={formData.enableReviewQueue ?? config.enableReviewQueue}
              onChange={e => setFormData(prev => ({ ...prev, enableReviewQueue: e.target.checked }))}
              className="h-4 w-4 text-primary-600 rounded"
            />
            <label htmlFor="sa-review-queue">
              <span className="text-sm font-medium text-gray-900">Enable Review Queue</span>
              <p className="text-xs text-gray-500">Create drafts for manual approval before sending</p>
            </label>
          </div>
          <div className="flex items-center gap-3">
            <input
              id="sa-pause-critical"
              type="checkbox"
              checked={formData.pauseOnCriticalHealth ?? config.pauseOnCriticalHealth}
              onChange={e => setFormData(prev => ({ ...prev, pauseOnCriticalHealth: e.target.checked }))}
              className="h-4 w-4 text-primary-600 rounded"
            />
            <label htmlFor="sa-pause-critical">
              <span className="text-sm font-medium text-gray-900">Auto-Pause on Critical Health</span>
              <p className="text-xs text-gray-500">Stop sending when domain health becomes critical</p>
            </label>
          </div>
          <div className="grid grid-cols-2 gap-4 mt-4">
            <div>
              <label htmlFor="sa-spam-threshold" className="block text-sm font-medium text-gray-700 mb-1">Spam Score Threshold (0-100)</label>
              <input
                id="sa-spam-threshold"
                type="number"
                value={formData.spamScoreThreshold ?? config.spamScoreThreshold}
                onChange={e => setFormData(prev => ({ ...prev, spamScoreThreshold: parseInt(e.target.value) }))}
                className="w-full px-3 py-2 border rounded-md text-sm"
                min={0}
                max={100}
              />
              <p className="text-xs text-gray-500 mt-1">Emails below this score will be blocked</p>
            </div>
            <div>
              <label htmlFor="sa-bounce-threshold" className="block text-sm font-medium text-gray-700 mb-1">Bounce Rate Threshold (%)</label>
              <input
                id="sa-bounce-threshold"
                type="number"
                value={formData.bounceRateThreshold ?? config.bounceRateThreshold}
                onChange={e => setFormData(prev => ({ ...prev, bounceRateThreshold: parseInt(e.target.value) }))}
                className="w-full px-3 py-2 border rounded-md text-sm"
                min={1}
                max={20}
              />
              <p className="text-xs text-gray-500 mt-1">Sending pauses above this bounce rate</p>
            </div>
          </div>
        </div>
      </div>

      {/* Save Button */}
      {hasChanges && (
        <div className="flex justify-end gap-3">
          <button
            onClick={() => setFormData({})}
            className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border rounded-md hover:bg-gray-50"
          >
            Cancel
          </button>
          <button
            onClick={() => updateMutation.mutate(formData)}
            disabled={updateMutation.isPending}
            className="px-4 py-2 text-sm font-medium text-white bg-primary-600 rounded-md hover:bg-primary-500 disabled:opacity-50"
          >
            {updateMutation.isPending ? 'Saving…' : 'Save Changes'}
          </button>
        </div>
      )}

      {updateMutation.isSuccess && (
        <div className="p-4 bg-green-50 text-green-700 rounded-lg text-sm">
          Configuration updated successfully!
        </div>
      )}
    </div>
  )
}

// ============================================================================
// SEQUENCES TAB
// ============================================================================

function SequencesTab() {
  const { data: sequences, isLoading } = useQuery({
    queryKey: ['sales-sequences'],
    queryFn: ({ signal }) => getSalesSequences(signal),
  })

  const [expandedId, setExpandedId] = useState<string | null>(null)

  if (isLoading) {
    return <div className="text-sm text-gray-500">Loading sequences…</div>
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold text-gray-900">Email Sequences</h3>
        <p className="text-sm text-gray-500">{sequences?.length || 0} sequences</p>
      </div>

      {sequences?.map(seq => (
        <div key={seq.id} className="bg-white rounded-xl border">
          <button
            onClick={() => setExpandedId(expandedId === seq.id ? null : seq.id)}
            className="w-full p-4 flex items-center justify-between text-left"
          >
            <div>
              <h4 className="font-medium text-gray-900">{seq.name}</h4>
              <p className="text-sm text-gray-500">{seq.description}</p>
              <div className="flex gap-4 mt-2 text-xs text-gray-500">
                <span>{seq.steps?.length || 0} steps</span>
                <span>{seq.enrolled_count || 0} enrolled</span>
                <span>{seq.active_count || 0} active</span>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <span className={`px-2 py-1 text-xs rounded-full ${seq.active ? 'bg-green-100 text-green-700' : 'bg-gray-100 text-gray-700'}`}>
                {seq.active ? 'Active' : 'Inactive'}
              </span>
              {expandedId === seq.id ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </div>
          </button>

          {expandedId === seq.id && (
            <div className="border-t p-4 space-y-4">
              {(Array.isArray(seq.steps) ? seq.steps : []).map((step: { delay_days: number; subject: string; body: string }, idx: number) => (
                <div key={idx} className="p-4 bg-gray-50 rounded-lg">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-xs font-medium text-gray-500">Step {idx + 1}</span>
                    <span className="text-xs text-gray-400">• Day {step.delay_days}</span>
                  </div>
                  <p className="font-medium text-gray-900 mb-2">{step.subject}</p>
                  <pre className="text-sm text-gray-600 whitespace-pre-wrap font-sans">{step.body}</pre>
                </div>
              ))}
            </div>
          )}
        </div>
      ))}
    </div>
  )
}

// ============================================================================
// DRAFTS TAB (Review Queue)
// ============================================================================

function DraftsTab() {
  const queryClient = useQueryClient()
  const [selectedIds, setSelectedIds] = useState<string[]>([])

  const { data, isLoading, refetch } = useQuery({
    queryKey: ['sales-drafts'],
    queryFn: ({ signal }) => getDraftEmails({ signal }),
  })

  const approveMutation = useMutation({
    mutationFn: (id: string) => approveDraftEmail(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['sales-drafts'] })
      refetch()
    },
  })

  const rejectMutation = useMutation({
    mutationFn: (id: string) => rejectDraftEmail(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['sales-drafts'] })
      refetch()
    },
  })

  const bulkApproveMutation = useMutation({
    mutationFn: (ids: string[]) => bulkApproveDrafts(ids),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['sales-drafts'] })
      setSelectedIds([])
      refetch()
    },
  })

  if (isLoading) {
    return <div className="text-sm text-gray-500">Loading drafts…</div>
  }

  const drafts = data?.emails || []

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold text-gray-900">Review Queue</h3>
        <div className="flex items-center gap-3">
          <span className="text-sm text-gray-500">{data?.total || 0} drafts pending</span>
          {selectedIds.length > 0 && (
            <button
              onClick={() => bulkApproveMutation.mutate(selectedIds)}
              disabled={bulkApproveMutation.isPending}
              className="px-3 py-1.5 text-sm font-medium text-white bg-green-600 rounded-md hover:bg-green-500"
            >
              Approve Selected ({selectedIds.length})
            </button>
          )}
        </div>
      </div>

      {drafts.length === 0 ? (
        <div className="bg-white rounded-xl border p-8 text-center">
          <FileText className="h-12 w-12 text-gray-300 mx-auto mb-4" />
          <p className="text-gray-600">No drafts awaiting review</p>
          <p className="text-sm text-gray-500 mt-1">Enable review queue in settings to review emails before sending</p>
        </div>
      ) : (
        <div className="space-y-3">
          {drafts.map(draft => (
            <div key={draft.id} className="bg-white rounded-xl border p-4">
              <div className="flex items-start gap-4">
                <input
                  type="checkbox"
                  checked={selectedIds.includes(draft.id)}
                  onChange={e => {
                    if (e.target.checked) {
                      setSelectedIds(prev => [...prev, draft.id])
                    } else {
                      setSelectedIds(prev => prev.filter(id => id !== draft.id))
                    }
                  }}
                  className="mt-1 h-4 w-4 text-primary-600 rounded"
                />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <span className="text-sm font-medium text-gray-900">{draft.to_email}</span>
                    <span className="text-xs text-gray-500">• Step {draft.step_number}</span>
                  </div>
                  <p className="font-medium text-gray-900">{draft.subject}</p>
                  <p className="text-sm text-gray-600 mt-1 line-clamp-2">{draft.body_html}</p>
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={() => approveMutation.mutate(draft.id)}
                    disabled={approveMutation.isPending}
                    className="p-2 text-green-600 hover:bg-green-50 rounded-md"
                    title="Approve & Send"
                  >
                    <CheckCircle className="h-5 w-5" />
                  </button>
                  <button
                    onClick={() => rejectMutation.mutate(draft.id)}
                    disabled={rejectMutation.isPending}
                    className="p-2 text-red-600 hover:bg-red-50 rounded-md"
                    title="Reject"
                  >
                    <XCircle className="h-5 w-5" />
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ============================================================================
// CONTACTS TAB
// ============================================================================

function ContactsTab() {
  const [search, setSearch] = useState('')
  const [statusFilter, setStatusFilter] = useState<string>('')

  const { data, isLoading } = useQuery({
    queryKey: ['sales-contacts', search, statusFilter],
    queryFn: ({ signal }) => getSalesContacts({ search, status: statusFilter || undefined, limit: 50, signal }),
  })

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-4">
        <input
          type="text"
          value={search}
          onChange={e => setSearch(e.target.value)}
          placeholder="Search contacts…"
          className="flex-1 px-3 py-2 border rounded-md text-sm"
        />
        <select
          value={statusFilter}
          onChange={e => setStatusFilter(e.target.value)}
          className="px-3 py-2 border rounded-md text-sm"
        >
          <option value="">All statuses</option>
          <option value="new">New</option>
          <option value="contacted">Contacted</option>
          <option value="replied">Replied</option>
          <option value="converted">Converted</option>
          <option value="bounced">Bounced</option>
          <option value="opted_out">Opted Out</option>
        </select>
      </div>

      {isLoading ? (
        <div className="text-sm text-gray-500">Loading contacts…</div>
      ) : (
        <div className="bg-white rounded-xl border overflow-hidden">
          <table className="min-w-full text-sm">
            <thead className="bg-gray-50">
              <tr>
                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Email</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Name</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Company</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Status</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Verified</th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {data?.contacts.map(contact => (
                <tr key={contact.id}>
                  <td className="px-4 py-3 text-gray-900">{contact.email}</td>
                  <td className="px-4 py-3 text-gray-600">{contact.first_name} {contact.last_name}</td>
                  <td className="px-4 py-3 text-gray-600">{contact.company_name || '—'}</td>
                  <td className="px-4 py-3">
                    <span className={`inline-flex px-2 py-1 text-xs rounded-full ${
                      contact.status === 'new' ? 'bg-blue-100 text-blue-700' :
                      contact.status === 'contacted' ? 'bg-yellow-100 text-yellow-700' :
                      contact.status === 'replied' ? 'bg-green-100 text-green-700' :
                      contact.status === 'bounced' ? 'bg-red-100 text-red-700' :
                      'bg-gray-100 text-gray-700'
                    }`}>
                      {contact.status}
                    </span>
                  </td>
                  <td className="px-4 py-3">
                    {contact.email_verified ? (
                      <CheckCircle className="h-4 w-4 text-green-600" />
                    ) : (
                      <XCircle className="h-4 w-4 text-gray-400" />
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {data?.contacts.length === 0 && (
            <div className="p-8 text-center text-gray-500">No contacts found</div>
          )}
        </div>
      )}
    </div>
  )
}
