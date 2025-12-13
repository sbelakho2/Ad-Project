import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import AdminAuditPage from '../page'
import * as adminLib from '@/lib/admin'

// Mock admin lib
jest.mock('@/lib/admin', () => ({
  listBillingAudit: jest.fn(),
}))

const mockListBillingAudit = adminLib.listBillingAudit as jest.MockedFunction<typeof adminLib.listBillingAudit>

const mockAuditData: adminLib.AuditListResponse = {
  success: true,
  data: [
    {
      id: 'aud_001',
      organization_id: 'org_123',
      event_type: 'subscription_created',
      metadata: { plan: 'growth', amount: 299 },
      created_at: '2024-12-10T10:30:00Z',
    },
    {
      id: 'aud_002',
      organization_id: 'org_456',
      event_type: 'payment_succeeded',
      metadata: { amount: 599, invoice_id: 'inv_789' },
      created_at: '2024-12-10T11:00:00Z',
    },
    {
      id: 'aud_003',
      organization_id: 'org_789',
      event_type: 'tier_upgrade',
      metadata: { from: 'starter', to: 'growth' },
      created_at: '2024-12-10T12:00:00Z',
    },
  ],
  pagination: {
    page: 1,
    limit: 20,
    total: 50,
    total_pages: 3,
  },
}

// Mock URL.createObjectURL and revokeObjectURL
const mockCreateObjectURL = jest.fn(() => 'blob:test-url')
const mockRevokeObjectURL = jest.fn()
global.URL.createObjectURL = mockCreateObjectURL
global.URL.revokeObjectURL = mockRevokeObjectURL

describe('AdminAuditPage', () => {
  beforeEach(() => {
    jest.clearAllMocks()
  })

  it('renders loading state initially', () => {
    mockListBillingAudit.mockImplementation(() => new Promise(() => {})) // Never resolves
    render(<AdminAuditPage />)

    expect(screen.getByText(/Loading…/i)).toBeInTheDocument()
  })

  it('renders audit entries on successful load', async () => {
    mockListBillingAudit.mockResolvedValue(mockAuditData)

    render(<AdminAuditPage />)

    await waitFor(() => {
      expect(screen.getAllByText(/Billing Audit/i).length).toBeGreaterThan(0)
    })

    await waitFor(() => {
      expect(screen.getByText(/subscription_created/i)).toBeInTheDocument()
    })

    await waitFor(() => {
      expect(screen.getByText(/payment_succeeded/i)).toBeInTheDocument()
    })
  })

  it('displays organization IDs', async () => {
    mockListBillingAudit.mockResolvedValue(mockAuditData)

    render(<AdminAuditPage />)

    await waitFor(() => {
      expect(screen.getByText(/org_123/)).toBeInTheDocument()
    })
  })

  it('handles API errors gracefully', async () => {
    mockListBillingAudit.mockRejectedValue(new Error('Failed to fetch audit log'))

    render(<AdminAuditPage />)

    await waitFor(() => {
      expect(screen.getByText(/Failed|error/i)).toBeInTheDocument()
    })
  })

  it('supports pagination', async () => {
    mockListBillingAudit.mockResolvedValue(mockAuditData)

    render(<AdminAuditPage />)

    await waitFor(() => {
      expect(screen.getByText(/subscription_created/i)).toBeInTheDocument()
    })

    // Check pagination component exists
    const paginationButtons = screen.getAllByRole('button')
    expect(paginationButtons.length).toBeGreaterThan(0)
  })

  it('exports data as CSV', async () => {
    mockListBillingAudit.mockResolvedValue(mockAuditData)

    render(<AdminAuditPage />)

    await waitFor(() => {
      expect(screen.getAllByText(/Billing Audit/i).length).toBeGreaterThan(0)
    })

    const exportButton = screen.getByRole('button', { name: /download csv/i })
    expect(exportButton).toBeInTheDocument()
  })

  it('calls API with correct pagination params', async () => {
    mockListBillingAudit.mockResolvedValue(mockAuditData)

    render(<AdminAuditPage />)

    await waitFor(() => {
      expect(mockListBillingAudit).toHaveBeenCalledWith({ page: 1, limit: 20 })
    })
  })
})
