import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import InvoicesPage from './page'
import * as billing from '@/lib/billing'

// Mock next/navigation minimal router
const mockReplace = jest.fn()
jest.mock('next/navigation', () => ({
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({ replace: mockReplace }),
  usePathname: () => '/billing/invoices',
}))

const createWrapper = () => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })

  const QueryClientWrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )
  QueryClientWrapper.displayName = 'InvoicesQueryClientWrapper'

  return QueryClientWrapper
}

describe('InvoicesPage — API error handling', () => {
  afterEach(() => {
    jest.restoreAllMocks()
  })

  const renderWithError = async (message: string) => {
    const error = new Error(message) as any
    error.response = { data: { message } }
    error.isAxiosError = true

    jest.spyOn(billing, 'listInvoices').mockRejectedValueOnce(error)

    render(<InvoicesPage />, { wrapper: createWrapper() })
    await waitFor(() => expect(screen.getByText(/error loading invoices/i)).toBeInTheDocument())
  }

  it('shows 401 Unauthorized error state', async () => {
    await renderWithError('Unauthorized')
    expect(screen.getByText(/unauthorized/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /try again/i })).toBeInTheDocument()
  })

  it('shows 403 Forbidden error state', async () => {
    await renderWithError('Forbidden')
    expect(screen.getByText(/forbidden/i)).toBeInTheDocument()
  })

  it('shows 404 Not Found empty/error state', async () => {
    await renderWithError('Not Found')
    expect(screen.getByText(/not found/i)).toBeInTheDocument()
  })
})
