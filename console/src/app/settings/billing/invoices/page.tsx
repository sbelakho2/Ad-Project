'use client'

import { useCallback, useEffect, useState } from 'react'
import { listInvoices, type InvoicesListResponse, type InvoicesQueryParams } from '@/lib/billing'
import Link from 'next/link'
import {
  FileText,
  Download,
  AlertCircle,
} from 'lucide-react'
import StatusBadge from '@/components/ui/StatusBadge'
import Pagination from '@/components/ui/Pagination'
import Filters, { type InvoiceStatusFilter } from '@/components/ui/Filters'
import { formatCurrency, formatDate } from '@/lib/utils'

type InvoiceStatusParam = NonNullable<InvoicesQueryParams['status']>
const ALLOWED_STATUS_VALUES: InvoiceStatusParam[] = ['draft', 'open', 'paid', 'void', 'uncollectible']

const isInvoiceStatusParam = (value: InvoiceStatusFilter): value is InvoiceStatusParam => {
  return typeof value === 'string' && value !== 'all' && (ALLOWED_STATUS_VALUES as readonly string[]).includes(value)
}

export default function InvoicesListPage() {
  const [data, setData] = useState<InvoicesListResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [page, setPage] = useState(1)
  const [statusFilter, setStatusFilter] = useState<InvoiceStatusFilter>('all')

  const loadInvoices = useCallback(async (signal?: AbortSignal) => {
    try {
      setLoading(true)
      setError(null)
      const params: InvoicesQueryParams = { page, limit: 20 }
      if (isInvoiceStatusParam(statusFilter)) {
        params.status = statusFilter
      }
      const result = await listInvoices(params, { signal })
      if (signal?.aborted) return
      setData(result)
    } catch (err) {
      if (signal?.aborted) return
      setError(err instanceof Error ? err.message : 'Failed to load invoices')
    } finally {
      if (!signal?.aborted) {
        setLoading(false)
      }
    }
  }, [page, statusFilter])

  useEffect(() => {
    const controller = new AbortController()
    loadInvoices(controller.signal)
    return () => {
      controller.abort()
    }
  }, [loadInvoices])

  // Status visuals provided by StatusBadge component

  if (loading && !data) {
    return (
      <div className="p-8">
        <div className="max-w-6xl mx-auto">
          <div className="animate-pulse space-y-6">
            <div className="h-8 w-48 bg-gray-200 rounded"></div>
            <div className="space-y-4">
              {[1, 2, 3, 4, 5].map((i) => (
                <div key={i} className="h-20 bg-gray-200 rounded-lg"></div>
              ))}
            </div>
          </div>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-8">
        <div className="max-w-6xl mx-auto">
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 flex items-start gap-3">
            <AlertCircle className="h-5 w-5 text-red-600 mt-0.5" />
            <div>
              <h3 className="font-semibold text-red-900">Error Loading Invoices</h3>
              <p className="text-sm text-red-700 mt-1">{error}</p>
              <button
                onClick={() => loadInvoices()}
                className="mt-3 text-sm font-medium text-red-700 hover:text-red-800"
              >
                Try Again
              </button>
            </div>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="p-8">
      <div className="max-w-6xl mx-auto space-y-6">
        {/* Header */}
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-3xl font-bold text-gray-900">Invoices</h1>
            <p className="text-gray-600 mt-2">View and download your billing invoices</p>
          </div>
        </div>

        {/* Filters */}
        <div className="bg-white rounded-lg border border-gray-200 p-4">
          <Filters
            status={statusFilter}
            onStatusChange={(s) => {
              setStatusFilter(s)
              setPage(1)
            }}
          />
        </div>

        {/* Invoices List */}
        {!data || data.invoices.length === 0 ? (
          <div className="bg-white rounded-lg border border-gray-200 p-12 text-center">
            <FileText className="h-12 w-12 text-gray-400 mx-auto mb-4" />
            <h3 className="text-lg font-semibold text-gray-900">No Invoices Found</h3>
            <p className="text-gray-600 mt-2">
              {statusFilter !== 'all'
                ? `No invoices with status "${statusFilter}"`
                : 'No invoices have been generated yet'}
            </p>
          </div>
        ) : (
          <>
            <div className="bg-white rounded-lg border border-gray-200 divide-y divide-gray-200">
              {data.invoices.map((invoice) => (
                <div
                  key={invoice.id}
                  className="p-6 hover:bg-gray-50 transition-colors"
                >
                  <div className="flex items-center justify-between gap-4">
                    <div className="flex items-center gap-4 flex-1 min-w-0">
                      <div className="flex-shrink-0">
                        <FileText className="h-5 w-5 text-gray-600" aria-hidden={true} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-3 mb-1">
                          <Link
                            href={`/settings/billing/invoices/${invoice.id}`}
                            className="font-semibold text-gray-900 hover:text-primary-600 transition"
                          >
                            {invoice.invoice_number}
                          </Link>
                          <StatusBadge status={invoice.status as any} />
                        </div>
                        <div className="flex items-center gap-4 text-sm text-gray-600">
                          <span>
                            Period: {formatDate(invoice.period_start)} - {formatDate(invoice.period_end)}
                          </span>
                          {invoice.due_date && <span>Due: {formatDate(invoice.due_date)}</span>}
                          {invoice.paid_at && <span>Paid: {formatDate(invoice.paid_at)}</span>}
                        </div>
                      </div>
                    </div>
                    <div className="flex items-center gap-4 flex-shrink-0">
                      <div className="text-right">
                        <p className="text-xl font-bold text-gray-900">
                          {formatCurrency(invoice.amount)}
                        </p>
                        <p className="text-xs text-gray-600">{invoice.currency.toUpperCase()}</p>
                      </div>
                      <Link
                        href={`/settings/billing/invoices/${invoice.id}`}
                        className="px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700 transition font-medium text-sm flex items-center gap-2"
                      >
                        <Download className="h-4 w-4" />
                        View
                      </Link>
                    </div>
                  </div>
                </div>
              ))}
            </div>

            {/* Pagination */}
            {data.pagination.total_pages > 1 && (
              <div className="bg-white rounded-lg border border-gray-200 p-4">
                <div className="flex items-center justify-between">
                  <p className="text-sm text-gray-600">
                    Page {data.pagination.page} of {data.pagination.total_pages} ({data.pagination.total} total invoices)
                  </p>
                  <Pagination
                    page={page}
                    totalPages={data.pagination.total_pages}
                    onPageChange={(next) => setPage(next)}
                  />
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  )
}
