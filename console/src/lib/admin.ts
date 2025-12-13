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
