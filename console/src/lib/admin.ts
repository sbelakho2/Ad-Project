"use client"

import { apiClient, handleApiError } from './api-client'
import type { ApiResponse } from '@/types'

export interface AuditEntry {
  id: string
  organization_id: string
  event_type: string
  metadata: unknown
  created_at: string
}

export interface AuditListResponse {
  success: boolean
  data: AuditEntry[]
  pagination: { page: number; limit: number; total: number; total_pages: number }
}

export interface SalesAutomationSummary {
  trialToPaidRate: number
  baselineTrialRate?: number
  avgDealSizeUSD: number
  targetDealSizeUSD?: number
  timeToConvertDays: number
  targetTimeToConvertDays?: number
  activeTrials: number
  trialWeekChange?: number
}

export interface SalesPrincipleStat {
  key: string
  label: string
  description: string
  conversionRate: number
  touches: number
  revenueUSD: number
  icon?: string
  color?: string
}

export interface SalesFunnelStage {
  key: string
  name: string
  count: number
  conversionPercentage: number
  avgDays: number
  engagementScore: number
}

export interface SalesJourneyTouchpoint {
  day: number
  title: string
  principle: string
  sent: number
  openRate: number
  clickRate: number
  impact: 'Low' | 'Medium' | 'High' | 'Very High' | 'Critical'
}

export interface SalesConversionRecord {
  company: string
  mrrUSD: number
  daysInTrial: number
  principle: string
  milestonesCompleted: number
  conversionProbability: number
}

export interface SalesExperimentRecord {
  id: string
  testName: string
  hypothesis: string
  controlRate: number
  testRate: number
  confidence: number
  winner: 'control' | 'test' | 'inconclusive'
  recommendation: string
}

export interface SalesQuickAction {
  id: string
  title: string
  description: string
  icon?: string
  href?: string
}

export interface SalesAutomationOverview {
  summary: SalesAutomationSummary
  principles: SalesPrincipleStat[]
  funnel: SalesFunnelStage[]
  journey: SalesJourneyTouchpoint[]
  conversions: SalesConversionRecord[]
  experiments: SalesExperimentRecord[]
  quickActions: SalesQuickAction[]
  insights: string[]
  updatedAt: string
}

export async function listBillingAudit(params?: {
  page?: number
  limit?: number
  org_id?: string
  action?: string
  from?: string
  to?: string
  signal?: AbortSignal
}): Promise<AuditListResponse> {
  try {
    const { signal, ...rest } = params ?? {}
    const res = await apiClient.get('/admin/billing/audit', { params: rest, signal })
    return res.data as AuditListResponse
  } catch (err) {
    throw new Error(handleApiError(err))
  }
}

export async function getSalesAutomationOverview(params?: { signal?: AbortSignal }) {
  try {
    const res = await apiClient.get<ApiResponse<SalesAutomationOverview>>('/admin/sales-automation', {
      signal: params?.signal,
    })
    return res.data
  } catch (err) {
    throw new Error(handleApiError(err))
  }
}

// ============================================================================
// SALES AUTOMATION CONTROL (NEW - Full UI Control)
// ============================================================================

export interface SalesAutomationConfig {
  fromEmail: string
  fromName: string
  replyTo: string
  dailyLimit: number
  requireEmailVerification: boolean
  enableReviewQueue: boolean
  warmupMode: boolean
  warmupDailyIncrement: number
  systemEnabled: boolean
  spamScoreThreshold: number
  bounceRateThreshold: number
  pauseOnCriticalHealth: boolean
}

export interface SalesAutomationStatus {
  enabled: boolean
  configured: boolean
  smtpConfigured: boolean
  dailyLimit: number
  todaySent: number
  todayRemaining: number
  config: SalesAutomationConfig
}

export interface SalesContact {
  id: string
  email: string
  first_name?: string
  last_name?: string
  company_name?: string
  job_title?: string
  app_name?: string
  app_downloads?: string
  status: string
  score?: number
  source?: string
  notes?: string
  email_verified?: boolean
  email_verification_status?: string
  created_at: string
  updated_at: string
}

export interface SalesSequence {
  id: string
  name: string
  description?: string
  steps: Array<{ delay_days: number; subject: string; body: string }>
  active: boolean
  created_at: string
  enrolled_count?: number
  replied_count?: number
  active_count?: number
}

export interface SalesEnrollment {
  id: string
  contact_id: string
  sequence_id: string
  current_step: number
  status: string
  next_send_at?: string
  email?: string
  first_name?: string
  company_name?: string
  sequence_name?: string
  created_at: string
}

export interface SalesEmail {
  id: string
  enrollment_id?: string
  contact_id: string
  to_email: string
  from_email: string
  subject?: string
  body_html?: string
  message_id?: string
  status: string
  sent_at?: string
  opened_at?: string
  clicked_at?: string
  replied_at?: string
  bounced_at?: string
  step_number?: number
  contact_email?: string
  first_name?: string
  approved_at?: string
  approved_by?: string
  rejection_reason?: string
  created_at: string
}

export async function getSalesStatus(signal?: AbortSignal): Promise<SalesAutomationStatus> {
  const res = await apiClient.get<SalesAutomationStatus>('/api/v1/sales/status', { signal })
  return res.data
}

export async function toggleSalesSystem(enabled: boolean): Promise<{ success: boolean; enabled: boolean }> {
  const res = await apiClient.post<{ success: boolean; enabled: boolean }>('/api/v1/sales/toggle', { enabled })
  return res.data
}

export async function getSalesConfig(signal?: AbortSignal): Promise<SalesAutomationConfig> {
  const res = await apiClient.get<{ config: SalesAutomationConfig }>('/api/v1/sales/config', { signal })
  return res.data.config
}

export async function updateSalesConfig(updates: Partial<SalesAutomationConfig>): Promise<{ success: boolean; config?: SalesAutomationConfig; error?: string }> {
  const res = await apiClient.put<{ success: boolean; config?: SalesAutomationConfig; error?: string }>('/api/v1/sales/config', updates)
  return res.data
}

export async function getSalesContacts(params?: { status?: string; limit?: number; offset?: number; search?: string; signal?: AbortSignal }): Promise<{ contacts: SalesContact[]; total: number }> {
  const { signal, ...rest } = params ?? {}
  const res = await apiClient.get<{ contacts: SalesContact[]; total: number }>('/api/v1/sales/contacts', { params: rest, signal })
  return res.data
}

export async function getSalesSequences(signal?: AbortSignal): Promise<SalesSequence[]> {
  const res = await apiClient.get<{ sequences: SalesSequence[] }>('/api/v1/sales/sequences', { signal })
  return res.data.sequences
}

export async function createSalesSequence(data: { name: string; description?: string; steps: Array<{ delay_days: number; subject: string; body: string }> }): Promise<{ success: boolean; sequence?: SalesSequence; error?: string }> {
  const res = await apiClient.post<{ success: boolean; sequence?: SalesSequence; error?: string }>('/api/v1/sales/sequences', data)
  return res.data
}

export async function updateSalesSequence(id: string, data: Partial<{ name: string; description: string; steps: Array<{ delay_days: number; subject: string; body: string }>; active: boolean }>): Promise<{ success: boolean; error?: string }> {
  const res = await apiClient.put<{ success: boolean; error?: string }>(`/api/v1/sales/sequences/${id}`, data)
  return res.data
}

export async function deleteSalesSequence(id: string): Promise<{ success: boolean; error?: string }> {
  const res = await apiClient.delete<{ success: boolean; error?: string }>(`/api/v1/sales/sequences/${id}`)
  return res.data
}

export async function getSalesEnrollments(params?: { sequenceId?: string; status?: string; limit?: number; offset?: number; signal?: AbortSignal }): Promise<{ enrollments: SalesEnrollment[]; total: number }> {
  const { signal, ...rest } = params ?? {}
  const res = await apiClient.get<{ enrollments: SalesEnrollment[]; total: number }>('/api/v1/sales/enrollments', { params: rest, signal })
  return res.data
}

export async function enrollContacts(params: { sequenceId: string; contactStatus?: string; limit?: number }): Promise<{ enrolled: number; errors: string[] }> {
  const res = await apiClient.post<{ enrolled: number; errors: string[] }>('/api/v1/sales/enrollments/enroll', params)
  return res.data
}

export async function getSalesEmails(params?: { contactId?: string; status?: string; limit?: number; offset?: number; signal?: AbortSignal }): Promise<{ emails: SalesEmail[]; total: number }> {
  const { signal, ...rest } = params ?? {}
  const res = await apiClient.get<{ emails: SalesEmail[]; total: number }>('/api/v1/sales/emails', { params: rest, signal })
  return res.data
}

export async function getDraftEmails(params?: { limit?: number; offset?: number; signal?: AbortSignal }): Promise<{ emails: SalesEmail[]; total: number }> {
  const { signal, ...rest } = params ?? {}
  const res = await apiClient.get<{ emails: SalesEmail[]; total: number }>('/api/v1/sales/drafts', { params: rest, signal })
  return res.data
}

export async function approveDraftEmail(emailId: string): Promise<{ success: boolean; error?: string }> {
  const res = await apiClient.post<{ success: boolean; error?: string }>(`/api/v1/sales/drafts/${emailId}/approve`)
  return res.data
}

export async function rejectDraftEmail(emailId: string, reason?: string): Promise<{ success: boolean; error?: string }> {
  const res = await apiClient.post<{ success: boolean; error?: string }>(`/api/v1/sales/drafts/${emailId}/reject`, { reason })
  return res.data
}

export async function bulkApproveDrafts(emailIds: string[]): Promise<{ approved: number; errors: string[] }> {
  const res = await apiClient.post<{ approved: number; errors: string[] }>('/api/v1/sales/drafts/bulk-approve', { emailIds })
  return res.data
}

export async function runSalesOutreach(dryRun: boolean = false): Promise<{ processed: number; sent: number; skippedSpam: number; errors: string[]; dryRun: boolean; domainHealth: { healthy: boolean; status: string } }> {
  const res = await apiClient.post<{ processed: number; sent: number; skippedSpam: number; errors: string[]; dryRun: boolean; domainHealth: { healthy: boolean; status: string } }>('/api/v1/sales/run', { dryRun })
  return res.data
}

export async function verifyContact(contactId: string, verified: boolean, status?: string): Promise<{ success: boolean }> {
  const res = await apiClient.post<{ success: boolean }>(`/api/v1/sales/contacts/${contactId}/verify`, { verified, status })
  return res.data
}

export async function getUnverifiedContacts(limit?: number, signal?: AbortSignal): Promise<SalesContact[]> {
  const res = await apiClient.get<{ contacts: SalesContact[] }>('/api/v1/sales/contacts/unverified', { params: { limit }, signal })
  return res.data.contacts
}

/**
 * Trigger system-wide billing reconciliation (OPERATOR-ONLY)
 * This is different from customer-facing billing - this affects all customers!
 */
export interface ReconciliationResult {
  success: boolean
  data?: {
    processed: number
    discrepancies: number
    errors: string[]
  }
  message?: string
}

export async function operatorReconcileBilling(idempotencyKey: string): Promise<ReconciliationResult> {
  try {
    const res = await apiClient.post<ReconciliationResult>(
      '/admin/billing/reconcile',
      {},
      {
        headers: {
          'Idempotency-Key': idempotencyKey,
        },
      }
    )
    return res.data
  } catch (err) {
    throw new Error(handleApiError(err))
  }
}

/**
 * Resend invoice email to a customer (OPERATOR-ONLY)
 */
export async function operatorResendInvoiceEmail(payload: { invoiceId: string; email: string }): Promise<void> {
  try {
    await apiClient.post('/admin/billing/resend-invoice', payload)
  } catch (err) {
    throw new Error(handleApiError(err))
  }
}
