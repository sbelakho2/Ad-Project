'use client'

import { useCallback, useEffect, useMemo, useState } from 'react'
import { useParams, useRouter } from 'next/navigation'
import { getInvoice, downloadInvoicePDF, Invoice } from '@/lib/billing'
import Link from 'next/link'
import {
  ArrowLeft,
  Download,
  AlertCircle,
  FileText,
  CheckCircle,
  Clock,
  XCircle,
} from 'lucide-react'
import { useSession } from '@/lib/useSession'
import { useFeatures } from '@/lib/useFeatures'
import { BILLING_FEATURE_FALLBACK, canAccessBilling as canUserAccessBilling } from '../../access'

export default function InvoiceDetailPage() {
  const params = useParams()
  const rawInvoiceId = params?.id
  const invoiceId = useMemo(() => {
    if (typeof rawInvoiceId === 'string') return rawInvoiceId
    if (Array.isArray(rawInvoiceId)) return rawInvoiceId[0]
    return ''
  }, [rawInvoiceId])
  const router = useRouter()
  const { user, isLoading: sessionLoading } = useSession()
  const { features, loading: featureLoading } = useFeatures({ fallback: BILLING_FEATURE_FALLBACK })

  const [invoice, setInvoice] = useState<Invoice | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [downloading, setDownloading] = useState(false)
  const [downloadError, setDownloadError] = useState<string | null>(null)

  const hasBillingFeature = features?.billing ?? BILLING_FEATURE_FALLBACK.billing ?? false
  const canViewInvoices = useMemo(() => canUserAccessBilling(user), [user])

  const loadInvoice = useCallback(
    async (signal?: AbortSignal) => {
      if (!invoiceId) return
      try {
        setLoading(true)
        setError(null)
        const data = await getInvoice(invoiceId, { signal })
        if (signal?.aborted) return
        setInvoice(data)
      } catch (err) {
        if (signal?.aborted) return
        setError(err instanceof Error ? err.message : 'Failed to load invoice')
      } finally {
        if (signal?.aborted) return
        setLoading(false)
      }
    },
    [invoiceId]
  )

  useEffect(() => {
    if (!sessionLoading && !user) {
      router.replace('/login')
    }
  }, [sessionLoading, user, router])

  useEffect(() => {
    if (sessionLoading || featureLoading) return
    if (!hasBillingFeature || !canViewInvoices) {
      router.replace('/403')
    }
  }, [sessionLoading, featureLoading, user, hasBillingFeature, canViewInvoices, router])

  useEffect(() => {
    if (sessionLoading || featureLoading) return
    if (!invoiceId) {
      router.replace('/settings/billing/invoices')
    }
  }, [invoiceId, sessionLoading, featureLoading, router])

  useEffect(() => {
    if (!invoiceId || sessionLoading || featureLoading || !hasBillingFeature || !canViewInvoices) {
      return
    }

    const controller = new AbortController()
    loadInvoice(controller.signal)
    return () => {
      controller.abort()
    }
  }, [invoiceId, sessionLoading, featureLoading, hasBillingFeature, canViewInvoices, loadInvoice])

  const handleDownloadPDF = async () => {
    try {
      if (!invoiceId) {
        setDownloadError('Missing invoice identifier')
        return
      }
      setDownloading(true)
      setDownloadError(null)
      const blobUrl = await downloadInvoicePDF(invoiceId)
      const link = document.createElement('a')
      link.href = blobUrl
      link.download = `invoice-${invoice?.invoice_number || invoiceId}.pdf`
      document.body.appendChild(link)
      link.click()
      document.body.removeChild(link)
      URL.revokeObjectURL(blobUrl)
    } catch (err) {
      setDownloadError(err instanceof Error ? err.message : 'Failed to download PDF')
    } finally {
      setDownloading(false)
    }
  }

  const getStatusIcon = (status: string) => {
    switch (status) {
      case 'paid':
        return <CheckCircle className="h-6 w-6 text-green-600" />
      case 'open':
        return <Clock className="h-6 w-6 text-blue-600" />
      case 'void':
      case 'uncollectible':
        return <XCircle className="h-6 w-6 text-gray-600" />
      default:
        return <FileText className="h-6 w-6 text-gray-600" />
    }
  }

  const getStatusColor = (status: string) => {
    switch (status) {
      case 'paid':
        return 'bg-green-100 text-green-800 border-green-200'
      case 'open':
        return 'bg-blue-100 text-blue-800 border-blue-200'
      case 'void':
      case 'uncollectible':
        return 'bg-gray-100 text-gray-800 border-gray-200'
      default:
        return 'bg-yellow-100 text-yellow-800 border-yellow-200'
    }
  }

  const formatCurrency = (amount: number, currency = 'USD') => {
    const normalizedCurrency = (currency || 'USD').toUpperCase()
    return new Intl.NumberFormat('en-US', { style: 'currency', currency: normalizedCurrency }).format(amount)
  }

  const formatDate = (dateString: string) =>
    new Date(dateString).toLocaleDateString('en-US', {
      year: 'numeric',
      month: 'long',
      day: 'numeric',
    })

  if (sessionLoading || featureLoading) {
    return (
      <div className="p-8">
        <div className="max-w-4xl mx-auto">
          <div className="animate-pulse space-y-6" role="status" aria-live="polite">
            <div className="h-8 w-48 bg-gray-200 rounded" />
            <div className="h-64 bg-gray-200 rounded-lg" />
          </div>
        </div>
      </div>
    )
  }

  if (!hasBillingFeature) {
    return (
      <div className="p-8">
        <div className="max-w-4xl mx-auto">
          <div className="bg-white border border-gray-200 rounded-lg p-6" role="alert">
            <h2 className="text-lg font-semibold text-gray-900">Billing Disabled</h2>
            <p className="text-sm text-gray-700 mt-2">
              Billing tools are unavailable for this environment. Contact support if you believe this is an error.
            </p>
          </div>
        </div>
      </div>
    )
  }

  if (!canViewInvoices) {
    return (
      <div className="p-8">
        <div className="max-w-4xl mx-auto">
          <div className="bg-white border border-gray-200 rounded-lg p-6" role="alert">
            <h2 className="text-lg font-semibold text-gray-900">Access Restricted</h2>
            <p className="text-sm text-gray-700 mt-2">
              You do not have permission to view invoices. Please contact your administrator if you believe
              this is an error.
            </p>
          </div>
        </div>
      </div>
    )
  }

  if (!invoiceId) {
    return (
      <div className="p-8">
        <div className="max-w-4xl mx-auto">
          <div className="bg-white border border-yellow-200 rounded-lg p-6" role="alert">
            <h2 className="text-lg font-semibold text-gray-900">Invoice ID Required</h2>
            <p className="text-sm text-gray-700 mt-2">
              We couldn’t determine which invoice to display. Please return to the invoices list and choose an invoice again.
            </p>
            <Link
              href="/settings/billing/invoices"
              className="inline-flex items-center gap-2 text-sm text-primary-600 hover:text-primary-700 mt-4"
            >
              <ArrowLeft className="h-4 w-4" />
              Back to Invoices
            </Link>
          </div>
        </div>
      </div>
    )
  }

  if (loading) {
    return (
      <div className="p-8">
        <div className="max-w-4xl mx-auto">
          <div className="animate-pulse space-y-6">
            <div className="h-8 w-64 bg-gray-200 rounded"></div>
            <div className="h-64 bg-gray-200 rounded-lg"></div>
          </div>
        </div>
      </div>
    )
  }

  if (error || !invoice) {
    return (
      <div className="p-8">
        <div className="max-w-4xl mx-auto">
          <Link
            href="/settings/billing/invoices"
            className="inline-flex items-center gap-2 text-sm text-gray-600 hover:text-gray-900 mb-6"
          >
            <ArrowLeft className="h-4 w-4" />
            Back to Invoices
          </Link>
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 flex items-start gap-3">
            <AlertCircle className="h-5 w-5 text-red-600 mt-0.5" />
            <div>
              <h3 className="font-semibold text-red-900">Error Loading Invoice</h3>
              <p className="text-sm text-red-700 mt-1">{error || 'Invoice not found'}</p>
              <button
                onClick={() => {
                  void loadInvoice()
                }}
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
      <div className="max-w-4xl mx-auto space-y-6">
        {/* Back Button */}
        <Link
          href="/settings/billing/invoices"
          className="inline-flex items-center gap-2 text-sm text-gray-600 hover:text-gray-900"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to Invoices
        </Link>

        {/* Invoice Header */}
        <div className="bg-white rounded-lg border border-gray-200 p-6">
          <div className="flex items-start justify-between gap-4 mb-6">
            <div className="flex items-center gap-4">
              {getStatusIcon(invoice.status)}
              <div>
                <h1 className="text-2xl font-bold text-gray-900">
                  Invoice {invoice.invoice_number}
                </h1>
                <div className="flex items-center gap-3 mt-2">
                  <span
                    className={`px-3 py-1 text-sm font-medium border rounded-full ${getStatusColor(
                      invoice.status
                    )}`}
                  >
                    {invoice.status.toUpperCase()}
                  </span>
                  <span className="text-sm text-gray-600">
                    {invoice.currency?.toUpperCase()}
                  </span>
                </div>
              </div>
            </div>
            <div className="flex flex-col items-end gap-2" aria-live="assertive">
              <button
                type="button"
                onClick={handleDownloadPDF}
                disabled={downloading}
                className="px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700 transition font-medium text-sm flex items-center gap-2 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                <Download className="h-4 w-4" />
                {downloading ? 'Downloading...' : 'Download PDF'}
              </button>
              {downloadError && (
                <div className="inline-flex items-center gap-2 text-xs text-red-700 bg-red-50 border border-red-200 rounded px-3 py-1" role="alert">
                  <AlertCircle className="h-3 w-3" />
                  {downloadError}
                </div>
              )}
            </div>
          </div>

          {/* Invoice Details Grid */}
          <dl className="grid grid-cols-1 md:grid-cols-2 gap-6 pt-6 border-t border-gray-200">
            <div>
              <dt className="text-sm font-medium text-gray-600">Amount</dt>
              <dd className="text-2xl font-bold text-gray-900 mt-1">
                {formatCurrency(invoice.amount, invoice.currency)}
              </dd>
            </div>
            <div>
              <dt className="text-sm font-medium text-gray-600">Billing Period</dt>
              <dd className="text-gray-900 mt-1">
                {formatDate(invoice.period_start)} - {formatDate(invoice.period_end)}
              </dd>
            </div>
            {invoice.due_date && (
              <div>
                <dt className="text-sm font-medium text-gray-600">Due Date</dt>
                <dd className="text-gray-900 mt-1">{formatDate(invoice.due_date)}</dd>
              </div>
            )}
            {invoice.paid_at && (
              <div>
                <dt className="text-sm font-medium text-gray-600">Paid On</dt>
                <dd className="text-gray-900 mt-1">{formatDate(invoice.paid_at)}</dd>
              </div>
            )}
            <div>
              <dt className="text-sm font-medium text-gray-600">Created</dt>
              <dd className="text-gray-900 mt-1">{formatDate(invoice.created_at)}</dd>
            </div>
            {invoice.stripe_invoice_id && (
              <div>
                <dt className="text-sm font-medium text-gray-600">Stripe Invoice ID</dt>
                <dd className="text-xs text-gray-900 font-mono mt-1">{invoice.stripe_invoice_id}</dd>
              </div>
            )}
          </dl>
        </div>

        {/* Line Items */}
        {invoice.line_items && invoice.line_items.length > 0 && (
          <section aria-labelledby="invoice-items-heading" className="bg-white rounded-lg border border-gray-200">
            <div className="p-6 border-b border-gray-200">
              <h2 id="invoice-items-heading" className="text-lg font-semibold text-gray-900">Invoice Items</h2>
            </div>
            <div className="overflow-x-auto">
              <table className="min-w-full divide-y divide-gray-200" role="table">
                <caption className="sr-only">Breakdown of services billed on this invoice</caption>
                <thead className="bg-gray-50">
                  <tr>
                    <th scope="col" className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                      Description
                    </th>
                    <th scope="col" className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider">
                      Quantity
                    </th>
                    <th scope="col" className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider">
                      Unit Price
                    </th>
                    <th scope="col" className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider">
                      Amount
                    </th>
                  </tr>
                </thead>
                <tbody className="bg-white divide-y divide-gray-200">
                  {invoice.line_items.map((item) => {
                    const rowKey = `${item.description}-${item.quantity}-${item.amount}-${item.unit_amount}`
                    return (
                      <tr key={rowKey}>
                      <td className="px-6 py-4 text-sm font-medium text-gray-900">{item.description}</td>
                      <td className="px-6 py-4 text-sm text-gray-600 text-right">{item.quantity.toLocaleString()}</td>
                      <td className="px-6 py-4 text-sm text-gray-600 text-right">
                        {formatCurrency(item.unit_amount, invoice.currency)}
                      </td>
                      <td className="px-6 py-4 text-sm font-semibold text-gray-900 text-right">
                        {formatCurrency(item.amount, invoice.currency)}
                      </td>
                      </tr>
                    )
                  })}
                </tbody>
                <tfoot className="bg-gray-50">
                  <tr>
                    <th scope="row" colSpan={3} className="px-6 py-4 text-right text-sm font-semibold text-gray-900">
                      Total
                    </th>
                    <td className="px-6 py-4 text-sm font-semibold text-gray-900 text-right">
                      {formatCurrency(invoice.amount, invoice.currency)}
                    </td>
                  </tr>
                </tfoot>
              </table>
            </div>
          </section>
        )}

        {/* Payment Status Message */}
        {invoice.status === 'open' && (
          <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
            <div className="flex items-start gap-3">
              <Clock className="h-5 w-5 text-blue-600 mt-0.5" />
              <div>
                <h3 className="font-semibold text-blue-900">Payment Pending</h3>
                <p className="text-sm text-blue-700 mt-1">
                  This invoice is awaiting payment. Payment will be automatically processed using
                  your default payment method.
                </p>
              </div>
            </div>
          </div>
        )}

        {invoice.status === 'paid' && (
          <div className="bg-green-50 border border-green-200 rounded-lg p-4">
            <div className="flex items-start gap-3">
              <CheckCircle className="h-5 w-5 text-green-600 mt-0.5" />
              <div>
                <h3 className="font-semibold text-green-900">Payment Received</h3>
                <p className="text-sm text-green-700 mt-1">
                  This invoice has been paid in full on {formatDate(invoice.paid_at!)}.
                </p>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
