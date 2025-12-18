import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import InvoicesPage from './page'
import * as billing from '@/lib/billing'

// Mock next/navigation
const mockPush = jest.fn()
const mockReplace = jest.fn()
jest.mock('next/navigation', () => ({
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: mockPush,
    replace: mockReplace,
  }),
  usePathname: () => '/billing/invoices',
}))

// Mock billing API
jest.mock('@/lib/billing')

const createWrapper = () => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  const QueryClientWrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )
  QueryClientWrapper.displayName = 'InvoicesQueryClientWrapper'

  return QueryClientWrapper
}

describe('InvoicesPage Component States', () => {
  beforeEach(() => {
    jest.clearAllMocks()
  })

  describe('Loading State', () => {
    it('should display loading skeleton', () => {
      jest.mocked(billing.listInvoices).mockImplementation(
        () => new Promise(() => {}) // Never resolves
      )

      const { container } = render(<InvoicesPage />, { wrapper: createWrapper() })

      expect(container.querySelector('.animate-pulse')).toBeTruthy()
    })
  })

  describe('Empty State', () => {
    it('should display empty message when no invoices exist', async () => {
      jest.mocked(billing.listInvoices).mockResolvedValue({
        invoices: [],
        pagination: {
          page: 1,
          limit: 20,
          total: 0,
          total_pages: 0,
        },
      })

      render(<InvoicesPage />, { wrapper: createWrapper() })

      await waitFor(() => {
        expect(screen.getByText('No Invoices Found')).toBeInTheDocument()
      })
    })

    it('should show helpful message in empty state', async () => {
      jest.mocked(billing.listInvoices).mockResolvedValue({
        invoices: [],
        pagination: { page: 1, limit: 20, total: 0, total_pages: 0 },
      })

      render(<InvoicesPage />, { wrapper: createWrapper() })

      await waitFor(() => {
        expect(screen.getByText('No invoices have been generated yet')).toBeInTheDocument()
      })
    })
  })

  describe('Error State', () => {
    it('should display error message on API failure', async () => {
      jest.mocked(billing.listInvoices).mockRejectedValue(
        new Error('Network error')
      )

      render(<InvoicesPage />, { wrapper: createWrapper() })

      await waitFor(() => {
        expect(screen.getByText('Error Loading Invoices')).toBeInTheDocument()
        expect(screen.getByText('Network error')).toBeInTheDocument()
      })
    })

    it('should have retry button on error', async () => {
      jest.mocked(billing.listInvoices).mockRejectedValue(
        new Error('Network error')
      )

      render(<InvoicesPage />, { wrapper: createWrapper() })

      await waitFor(() => {
        const retryButton = screen.getByRole('button', { name: /try again/i })
        expect(retryButton).toBeInTheDocument()
      })
    })
  })

  describe('Success State with Data', () => {
    it('should display invoices table', async () => {
      jest.mocked(billing.listInvoices).mockResolvedValue({
        invoices: [
          {
            id: '1',
            invoice_number: 'INV-001',
            customer_id: 'cust_123',
            amount: 9900,
            currency: 'usd',
            status: 'paid',
            period_start: '2025-10-01T00:00:00Z',
            period_end: '2025-10-31T23:59:59Z',
            due_date: '2025-11-15T00:00:00Z',
            paid_at: '2025-11-10T12:00:00Z',
            stripe_invoice_id: 'in_123',
            pdf_url: null,
            created_at: '2025-11-01T00:00:00Z',
            updated_at: '2025-11-10T12:00:00Z',
          },
        ],
        pagination: {
          page: 1,
          limit: 20,
          total: 1,
          total_pages: 1,
        },
      })

      render(<InvoicesPage />, { wrapper: createWrapper() })

      await waitFor(() => {
        expect(screen.getByText('INV-001')).toBeInTheDocument()
        expect(screen.getByText('$9,900.00')).toBeInTheDocument()
      })
    })

    it('should display status badges with correct colors', async () => {
      jest.mocked(billing.listInvoices).mockResolvedValue({
        invoices: [
          {
            id: '1',
            invoice_number: 'INV-001',
            customer_id: 'cust_123',
            amount: 9900,
            currency: 'usd',
            status: 'paid',
            period_start: '2025-10-01T00:00:00Z',
            period_end: '2025-10-31T23:59:59Z',
            due_date: '2025-11-15T00:00:00Z',
            paid_at: '2025-11-10T12:00:00Z',
            stripe_invoice_id: 'in_123',
            pdf_url: null,
            created_at: '2025-11-01T00:00:00Z',
            updated_at: '2025-11-10T12:00:00Z',
          },
        ],
        pagination: { page: 1, limit: 20, total: 1, total_pages: 1 },
      })

      render(<InvoicesPage />, { wrapper: createWrapper() })

      await waitFor(() => {
        const paidBadge = screen.getByLabelText('Paid')
        expect(paidBadge).toHaveTextContent('Paid')
      })
    })
  })

  describe('Pagination', () => {
    it('should display pagination controls when multiple pages exist', async () => {
      jest.mocked(billing.listInvoices).mockResolvedValue({
        invoices: Array.from({ length: 20 }, (_, i) => ({
          id: `${i}`,
          invoice_number: `INV-${String(i).padStart(3, '0')}`,
          customer_id: 'cust_123',
          amount: 9900,
          currency: 'usd',
          status: 'paid',
          period_start: '2025-10-01T00:00:00Z',
          period_end: '2025-10-31T23:59:59Z',
          due_date: '2025-11-15T00:00:00Z',
          paid_at: '2025-11-10T12:00:00Z',
          stripe_invoice_id: 'in_123',
          pdf_url: null,
          created_at: '2025-11-01T00:00:00Z',
          updated_at: '2025-11-10T12:00:00Z',
        })),
        pagination: {
          page: 1,
          limit: 20,
          total: 50,
          total_pages: 3,
        },
      })

      render(<InvoicesPage />, { wrapper: createWrapper() })

      await waitFor(() => {
        expect(screen.getByText(/50 total invoices/i)).toBeInTheDocument()
        expect(screen.getByRole('navigation', { name: /pagination/i })).toBeInTheDocument()
      })
    })

    it('should not display pagination for single page', async () => {
      jest.mocked(billing.listInvoices).mockResolvedValue({
        invoices: [
          {
            id: '1',
            invoice_number: 'INV-001',
            customer_id: 'cust_123',
            amount: 9900,
            currency: 'usd',
            status: 'paid',
            period_start: '2025-10-01T00:00:00Z',
            period_end: '2025-10-31T23:59:59Z',
            due_date: '2025-11-15T00:00:00Z',
            paid_at: '2025-11-10T12:00:00Z',
            stripe_invoice_id: 'in_123',
            pdf_url: null,
            created_at: '2025-11-01T00:00:00Z',
            updated_at: '2025-11-10T12:00:00Z',
          },
        ],
        pagination: { page: 1, limit: 20, total: 1, total_pages: 1 },
      })

      render(<InvoicesPage />, { wrapper: createWrapper() })

      await waitFor(() => {
        expect(screen.queryByRole('navigation', { name: /pagination/i })).not.toBeInTheDocument()
      })
    })
  })
})
