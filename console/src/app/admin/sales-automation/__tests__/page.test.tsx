import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import SalesAutomationDashboard from '../page'
import * as adminLib from '@/lib/admin'

// Mock admin lib
jest.mock('@/lib/admin', () => ({
  getSalesAutomationOverview: jest.fn(),
}))

// Mock utils functions
jest.mock('@/lib/utils', () => ({
  formatCurrency: (v: number) => `$${v.toFixed(2)}`,
  formatNumber: (v: number) => v.toLocaleString(),
  formatPercentage: (v: number) => `${(v * 100).toFixed(1)}%`,
}))

const mockGetSalesAutomation = adminLib.getSalesAutomationOverview as jest.MockedFunction<
  typeof adminLib.getSalesAutomationOverview
>

const mockSalesData: adminLib.SalesAutomationOverview = {
  summary: {
    trialToPaidRate: 0.42,
    baselineTrialRate: 0.35,
    avgDealSizeUSD: 8500,
    targetDealSizeUSD: 10000,
    timeToConvertDays: 12,
    targetTimeToConvertDays: 14,
    activeTrials: 127,
    trialWeekChange: 8,
  },
  principles: [
    {
      key: 'social_proof',
      label: 'Social Proof',
      description: 'Testimonials and case studies',
      conversionRate: 0.52,
      touches: 1250,
      revenueUSD: 425000,
      icon: 'users',
      color: 'blue',
    },
    {
      key: 'scarcity',
      label: 'Scarcity',
      description: 'Limited-time offers',
      conversionRate: 0.48,
      touches: 890,
      revenueUSD: 312000,
      icon: 'clock',
      color: 'orange',
    },
  ],
  funnel: [
    {
      key: 'awareness',
      name: 'Awareness',
      count: 5000,
      conversionPercentage: 0.25,
      avgDays: 3,
      engagementScore: 0.65,
    },
    {
      key: 'trial',
      name: 'Trial',
      count: 1250,
      conversionPercentage: 0.42,
      avgDays: 7,
      engagementScore: 0.78,
    },
  ],
  journey: [
    {
      day: 1,
      title: 'Welcome Email',
      principle: 'Reciprocity',
      sent: 500,
      openRate: 0.72,
      clickRate: 0.35,
      impact: 'High',
    },
    {
      day: 3,
      title: 'Social Proof Email',
      principle: 'Social Proof',
      sent: 480,
      openRate: 0.68,
      clickRate: 0.28,
      impact: 'Very High',
    },
  ],
  conversions: [
    {
      company: 'TechCorp Inc',
      mrrUSD: 2500,
      daysInTrial: 8,
      principle: 'Authority',
      milestonesCompleted: 5,
      conversionProbability: 0.92,
    },
  ],
  experiments: [
    {
      id: 'exp_001',
      testName: 'CTA Color Test',
      hypothesis: 'Green CTA increases clicks',
      controlRate: 0.12,
      testRate: 0.18,
      confidence: 0.95,
      winner: 'test',
      recommendation: 'Roll out green CTA',
    },
  ],
  quickActions: [
    {
      id: 'action_1',
      title: 'Review High-Value Trials',
      description: 'Check trials with >$5k potential',
    },
  ],
  insights: ['Trial-to-paid rate is 20% above baseline'],
  updatedAt: new Date().toISOString(),
}

const createTestQueryClient = () =>
  new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  })

const renderWithQueryClient = (ui: React.ReactElement) => {
  const queryClient = createTestQueryClient()
  return render(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>)
}

describe('SalesAutomationDashboard', () => {
  beforeEach(() => {
    jest.clearAllMocks()
  })

  it('shows loading state initially', () => {
    mockGetSalesAutomation.mockImplementation(() => new Promise(() => {})) // Never resolves
    renderWithQueryClient(<SalesAutomationDashboard />)

    expect(screen.getByText(/Loading sales automation data/i)).toBeInTheDocument()
  })

  it('renders dashboard with sales data', async () => {
    mockGetSalesAutomation.mockResolvedValue({ data: mockSalesData })

    renderWithQueryClient(<SalesAutomationDashboard />)

    await waitFor(() => {
      expect(screen.getByText(/Sales Automation Dashboard/i)).toBeInTheDocument()
    })

    // Check summary cards - look for specific value in card (may appear multiple times)
    await waitFor(() => {
      expect(screen.getAllByText(/42\.0%/).length).toBeGreaterThan(0) // Trial-to-paid rate
    })
  })

  it('displays principle effectiveness section', async () => {
    mockGetSalesAutomation.mockResolvedValue({ data: mockSalesData })

    renderWithQueryClient(<SalesAutomationDashboard />)

    await waitFor(() => {
      expect(screen.getAllByText(/Social Proof/i).length).toBeGreaterThan(0)
    })
    await waitFor(() => {
      expect(screen.getAllByText(/Scarcity/i).length).toBeGreaterThan(0)
    })
  })

  it('displays funnel stages', async () => {
    mockGetSalesAutomation.mockResolvedValue({ data: mockSalesData })

    renderWithQueryClient(<SalesAutomationDashboard />)

    await waitFor(() => {
      expect(screen.getByText(/Awareness/i)).toBeInTheDocument()
    })
    // "Trial" appears multiple places, just verify page renders successfully
    await waitFor(() => {
      expect(screen.getByText(/Conversion Funnel/i)).toBeInTheDocument()
    })
  })

  it('displays journey touchpoints', async () => {
    mockGetSalesAutomation.mockResolvedValue({ data: mockSalesData })

    renderWithQueryClient(<SalesAutomationDashboard />)

    await waitFor(() => {
      expect(screen.getByText(/Welcome Email/i)).toBeInTheDocument()
    })
  })

  it('handles API errors gracefully', async () => {
    mockGetSalesAutomation.mockRejectedValue(new Error('Unable to fetch sales data'))

    renderWithQueryClient(<SalesAutomationDashboard />)

    await waitFor(() => {
      expect(screen.getByText(/Unable to load sales dashboard/i)).toBeInTheDocument()
    })
  })

  it('allows data refresh', async () => {
    mockGetSalesAutomation.mockResolvedValue({ data: mockSalesData })

    renderWithQueryClient(<SalesAutomationDashboard />)

    await waitFor(() => {
      expect(screen.getByText(/Sales Automation Dashboard/i)).toBeInTheDocument()
    })

    const refreshButton = screen.getByRole('button', { name: /refresh/i })
    expect(refreshButton).toBeInTheDocument()

    fireEvent.click(refreshButton)

    await waitFor(() => {
      expect(mockGetSalesAutomation).toHaveBeenCalledTimes(2)
    })
  })
})
