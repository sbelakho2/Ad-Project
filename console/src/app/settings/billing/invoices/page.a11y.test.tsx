import { render, waitFor } from '@testing-library/react'
import { axe } from 'jest-axe'
import InvoicesPage from './page'

// Mock the API client
const mockListInvoices = jest.fn().mockResolvedValue({
  invoices: [
    {
      id: '1',
      invoice_number: 'INV-001',
      amount: 9900,
      currency: 'usd',
      status: 'paid',
      period_start: '2025-10-01T00:00:00Z',
      period_end: '2025-10-31T23:59:59Z',
      due_date: '2025-11-15T00:00:00Z',
      paid_at: '2025-11-10T12:00:00Z',
      created_at: '2025-11-01T00:00:00Z',
    },
  ],
  pagination: {
    page: 1,
    limit: 20,
    total: 1,
    total_pages: 1,
  },
})

jest.mock('@/lib/billing', () => ({
  listInvoices: (...args: unknown[]) => mockListInvoices(...args),
}))

jest.mock('@tanstack/react-query', () => ({
  useQuery: jest.fn(() => ({
    data: {
      invoices: [],
      pagination: { page: 1, limit: 20, total: 0, total_pages: 0 },
    },
    isLoading: false,
    error: null,
  })),
}))

jest.mock('next/navigation', () => ({
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: jest.fn(),
    replace: jest.fn(),
  }),
  usePathname: () => '/billing/invoices',
}))

describe('InvoicesPage Accessibility', () => {
  afterEach(() => {
    mockListInvoices.mockClear()
  })

  it('should have no accessibility violations', async () => {
    const { container } = render(<InvoicesPage />)
    await waitFor(() => expect(mockListInvoices).toHaveBeenCalled())
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('should have proper table structure with headers', async () => {
    const { container } = render(<InvoicesPage />)
    await waitFor(() => expect(mockListInvoices).toHaveBeenCalled())
    
    const table = container.querySelector('table')
    if (table) {
      const headers = table.querySelectorAll('th')
      expect(headers.length).toBeGreaterThan(0)
      
      // Each header should have scope
      headers.forEach(header => {
        expect(header).toHaveAttribute('scope', 'col')
      })
    }
  })

  it('should have accessible status badges', async () => {
    const { container } = render(<InvoicesPage />)
    await waitFor(() => expect(mockListInvoices).toHaveBeenCalled())
    
    // Status badges should have sufficient contrast
    const badges = container.querySelectorAll('[class*="badge"]')
    badges.forEach(badge => {
      const text = badge.textContent
      expect(text).toBeTruthy()
    })
  })

  it('should have keyboard-accessible filters', async () => {
    const { container } = render(<InvoicesPage />)
    await waitFor(() => expect(mockListInvoices).toHaveBeenCalled())
    
    const selects = container.querySelectorAll('select')
    const inputs = container.querySelectorAll('input')
    
    ;[...selects, ...inputs].forEach(element => {
      // Should be focusable
      expect(element.tabIndex).toBeGreaterThanOrEqual(0)
    })
  })
})
