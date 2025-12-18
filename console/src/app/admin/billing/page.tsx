'use client'

import { type ChangeEvent, useCallback, useMemo, useRef, useState } from 'react'
import { operatorReconcileBilling, operatorResendInvoiceEmail } from '@/lib/admin'

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

export default function AdminBillingOpsPage() {
  const [invoiceId, setInvoiceId] = useState('')
  const [email, setEmail] = useState('')
  const [reconStatus, setReconStatus] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [acknowledgedRisk, setAcknowledgedRisk] = useState(false)

  const [resendStatus, setResendStatus] = useState<string | null>(null)
  const [resendError, setResendError] = useState<string | null>(null)
  const [resendLoading, setResendLoading] = useState(false)

  const reconcileKeyRef = useRef<string | null>(null)
  const reconcilingRef = useRef(false)

  const handleInvoiceIdChange = useCallback((event: ChangeEvent<HTMLInputElement>) => {
    setInvoiceId(event.target.value)
  }, [])

  const handleEmailChange = useCallback((event: ChangeEvent<HTMLInputElement>) => {
    setEmail(event.target.value)
  }, [])

  const triggerReconcile = useCallback(async () => {
    if (reconcilingRef.current) return
    reconcilingRef.current = true
    try {
      setLoading(true)
      setError(null)
      setReconStatus(null)

      const key =
        reconcileKeyRef.current ||
        (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
          ? crypto.randomUUID()
          : `admin-reconcile-${Date.now()}`)
      reconcileKeyRef.current = key

      const res = await operatorReconcileBilling(key)
      setReconStatus(res?.success ? 'Reconciliation triggered successfully' : 'Reconciliation completed with discrepancies')
    } catch (e: any) {
      setError(e?.message || 'Failed to trigger reconciliation')
    } finally {
      reconcileKeyRef.current = null
      reconcilingRef.current = false
      setLoading(false)
      setAcknowledgedRisk(false)
    }
  }, [])

  const handleResendInvoice = useCallback(async () => {
    const trimmedInvoiceId = invoiceId.trim()
    const trimmedEmail = email.trim()
    if (!trimmedInvoiceId || !EMAIL_PATTERN.test(trimmedEmail)) return

    try {
      setResendLoading(true)
      setResendError(null)
      setResendStatus(null)
      await operatorResendInvoiceEmail({ invoiceId: trimmedInvoiceId, email: trimmedEmail })
      setResendStatus('Invoice email queued for delivery')
    } catch (e: any) {
      setResendError(e?.message || 'Unable to resend invoice email')
    } finally {
      setResendLoading(false)
    }
  }, [email, invoiceId])

  const canResend = useMemo(() => Boolean(invoiceId.trim()) && EMAIL_PATTERN.test(email.trim()), [email, invoiceId])

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
      <section className="bg-white border rounded-lg p-6">
        <h2 className="text-xl font-semibold text-gray-900">Reconciliation</h2>
        <p className="text-gray-600 mt-2">
          Trigger a manual reconciliation run. Jobs reuse the same Idempotency-Key for duplicate clicks to avoid double execution.
        </p>
        <label className="mt-4 flex items-start gap-2 text-sm text-gray-700">
          <input
            type="checkbox"
            className="mt-1 h-4 w-4"
            checked={acknowledgedRisk}
            onChange={(event) => setAcknowledgedRisk(event.target.checked)}
          />
          <span>I understand this will enqueue a full billing reconciliation and may take several minutes.</span>
        </label>
        <button
          onClick={triggerReconcile}
          disabled={loading || !acknowledgedRisk}
          className="mt-4 px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700 disabled:opacity-50"
        >
          {loading ? 'Triggering…' : 'Reconcile Now'}
        </button>
        {reconStatus && (
          <p className="text-green-700 mt-3" role="status">
            {reconStatus}
          </p>
        )}
        {error && (
          <p className="text-red-600 mt-3" role="alert">
            {error}
          </p>
        )}
      </section>

      <section className="bg-white border rounded-lg p-6">
        <h2 className="text-xl font-semibold text-gray-900">Resend Invoice Email</h2>
        <p className="text-gray-600 mt-2">Provide an invoice ID and destination email to resend the PDF.</p>
        <div className="mt-4 grid grid-cols-1 sm:grid-cols-2 gap-3">
          <input
            className="border rounded px-3 py-2"
            placeholder="Invoice ID"
            value={invoiceId}
            onChange={handleInvoiceIdChange}
            aria-label="Invoice identifier"
          />
          <input
            className="border rounded px-3 py-2"
            placeholder="Email"
            type="email"
            value={email}
            onChange={handleEmailChange}
            aria-label="Billing email"
          />
        </div>
        <button
          className="mt-4 px-4 py-2 border rounded-lg text-sm font-medium text-gray-800 hover:bg-gray-50 disabled:opacity-50"
          onClick={handleResendInvoice}
          disabled={!canResend || resendLoading}
        >
          {resendLoading ? 'Sending…' : 'Resend Invoice Email'}
        </button>
        {!canResend && (
          <p className="mt-2 text-xs text-gray-500">Both invoice ID and a valid email address are required.</p>
        )}
        {resendStatus && (
          <p className="text-green-700 mt-3" role="status">
            {resendStatus}
          </p>
        )}
        {resendError && (
          <p className="text-red-600 mt-3" role="alert">
            {resendError}
          </p>
        )}
      </section>

      <section className="bg-white border rounded-lg p-6 lg:col-span-2">
        <h2 className="text-xl font-semibold text-gray-900">Stripe Portal</h2>
        <p className="text-gray-600 mt-2">Open Stripe Customer Portal for the current organization (if configured).</p>
        <a
          className="inline-block mt-4 px-4 py-2 bg-white border rounded-lg hover:bg-gray-50 text-sm"
          href="/settings/billing/settings"
        >
          Go to Billing Settings
        </a>
      </section>
    </div>
  )
}
