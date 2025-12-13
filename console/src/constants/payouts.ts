import type { LucideIcon } from 'lucide-react'
import { AlertCircle, CheckCircle, Clock, XCircle } from 'lucide-react'
import type { PayoutHistory } from '@/types'

export type PayoutStatus = PayoutHistory['status']
export type PayoutMethod = PayoutHistory['method']

export const PAYOUT_STATUS_META: Record<
  PayoutStatus,
  {
    label: string
    icon: LucideIcon
    subtleClass: string
    solidClass: string
  }
> = {
  pending: {
    label: 'Pending',
    icon: Clock,
    subtleClass: 'text-warning-700 bg-warning-50 border-warning-200',
    solidClass: 'bg-warning-100 text-warning-700',
  },
  processing: {
    label: 'Processing',
    icon: AlertCircle,
    subtleClass: 'text-blue-700 bg-blue-50 border-blue-200',
    solidClass: 'bg-blue-100 text-blue-700',
  },
  completed: {
    label: 'Completed',
    icon: CheckCircle,
    subtleClass: 'text-success-700 bg-success-50 border-success-200',
    solidClass: 'bg-success-100 text-success-700',
  },
  failed: {
    label: 'Failed',
    icon: XCircle,
    subtleClass: 'text-danger-700 bg-danger-50 border-danger-200',
    solidClass: 'bg-danger-100 text-danger-700',
  },
}

export const PAYOUT_METHOD_LABELS: Record<PayoutMethod, string> = {
  stripe: 'Stripe',
  wire: 'Wire Transfer',
}

export const PAYOUTS_PAGE_SIZE = 20

export const PAYOUT_CSV_HEADERS = [
  { key: 'id', label: 'Payout ID' },
  { key: 'scheduledFor', label: 'Scheduled Date' },
  { key: 'processedAt', label: 'Processed Date' },
  { key: 'amount', label: 'Amount' },
  { key: 'status', label: 'Status' },
  { key: 'method', label: 'Method' },
] as const

export type PayoutCsvHeaderKey = (typeof PAYOUT_CSV_HEADERS)[number]['key']
