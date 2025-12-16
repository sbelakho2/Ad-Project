import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import AdminBillingOpsPage from '../page'
import { operatorReconcileBilling, operatorResendInvoiceEmail } from '@/lib/admin'
import type { ReconciliationResult } from '@/lib/admin'

jest.mock('@/lib/admin', () => ({
  operatorReconcileBilling: jest.fn(),
  operatorResendInvoiceEmail: jest.fn(),
}))

const mockedReconcileBilling = operatorReconcileBilling as jest.MockedFunction<typeof operatorReconcileBilling>
const mockedResendInvoiceEmail = operatorResendInvoiceEmail as jest.MockedFunction<typeof operatorResendInvoiceEmail>

const baseReconcileResult: ReconciliationResult = {
  success: true,
  data: {
    processed: 10,
    discrepancies: 0,
    errors: [],
  },
  message: 'Reconciliation completed successfully',
}

describe('AdminBillingOpsPage', () => {
  beforeEach(() => {
    jest.clearAllMocks()
    mockedReconcileBilling.mockResolvedValue(baseReconcileResult)
    mockedResendInvoiceEmail.mockResolvedValue(undefined)
  })

  it('requires acknowledgement before triggering reconciliation', async () => {
    render(<AdminBillingOpsPage />)

    const reconcileButton = screen.getByRole('button', { name: /reconcile now/i })
    const acknowledgementCheckbox = screen.getByRole('checkbox')

    expect(reconcileButton).toBeDisabled()
    expect(acknowledgementCheckbox).not.toBeChecked()

    fireEvent.click(acknowledgementCheckbox)
    expect(reconcileButton).not.toBeDisabled()

    fireEvent.click(reconcileButton)

    await waitFor(() => expect(mockedReconcileBilling).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(screen.getByText(/reconciliation triggered successfully/i)).toBeVisible())

    expect(screen.getByRole('checkbox')).not.toBeChecked()
    expect(screen.getByRole('button', { name: /reconcile now/i })).toBeDisabled()
  })

  it('resends invoice email when valid details are provided', async () => {
    render(<AdminBillingOpsPage />)

    const invoiceInput = screen.getByLabelText(/invoice identifier/i)
    const emailInput = screen.getByLabelText(/billing email/i)
    const resendButton = screen.getByRole('button', { name: /resend invoice email/i })

    expect(resendButton).toBeDisabled()
    expect(screen.getByText(/both invoice id/i)).toBeVisible()

    fireEvent.change(invoiceInput, { target: { value: '  inv_123  ' } })
    fireEvent.change(emailInput, { target: { value: '  billing@example.com  ' } })

    expect(resendButton).not.toBeDisabled()

    fireEvent.click(resendButton)

    await waitFor(() =>
      expect(mockedResendInvoiceEmail).toHaveBeenCalledWith({ invoiceId: 'inv_123', email: 'billing@example.com' })
    )
    await waitFor(() => expect(screen.getByText(/invoice email queued for delivery/i)).toBeVisible())
  })
})
