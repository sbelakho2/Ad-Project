import { render, waitFor } from '@testing-library/react'
// @ts-expect-error jest-axe types do not expose this helper yet
import { axe, toHaveNoViolations } from 'jest-axe'
import UsagePage from './page'

expect.extend(toHaveNoViolations)

// Mock the API client
jest.mock('@/lib/billing', () => ({
  getCurrentUsage: jest.fn().mockResolvedValue({
    current_period: {
      start: '2025-11-01T00:00:00Z',
      end: '2025-11-30T23:59:59Z',
      impressions: 750000,
      api_calls: 50000,
      data_transfer_gb: 45,
    },
    overages: {
      impressions: { amount: 0, cost: 0 },
      api_calls: { amount: 0, cost: 0 },
      data_transfer: { amount: 0, cost: 0 },
      total_overage_cost: 0,
    },
    subscription: {
      plan_type: 'starter',
      included_impressions: 1000000,
      included_api_calls: 100000,
      included_data_transfer_gb: 50,
    },
  }),
  getFeatureFlags: jest.fn().mockResolvedValue({ billing: true }),
}))

// Mock useQuery from react-query
jest.mock('@tanstack/react-query', () => ({
  useQuery: jest.fn((options) => ({
    data: Array.isArray(options.queryKey) && options.queryKey.includes('usage') ? {
      current_period: {
        start: '2025-11-01T00:00:00Z',
        end: '2025-11-30T23:59:59Z',
        impressions: 750000,
        api_calls: 50000,
        data_transfer_gb: 45,
      },
      overages: {
        impressions: { amount: 0, cost: 0 },
        api_calls: { amount: 0, cost: 0 },
        data_transfer: { amount: 0, cost: 0 },
        total_overage_cost: 0,
      },
      subscription: {
        plan_type: 'starter',
        included_impressions: 1000000,
        included_api_calls: 100000,
        included_data_transfer_gb: 50,
      },
    } : null,
    isLoading: false,
    error: null,
  })),
}))

describe('UsagePage Accessibility', () => {
  it('should have no accessibility violations', async () => {
    const { container } = render(<UsagePage />)
    await waitFor(() => {
      expect(container.querySelector('h1')).toBeInTheDocument()
    })

    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('should have proper heading hierarchy', async () => {
    const { container } = render(<UsagePage />)

    await waitFor(() => {
      expect(container.querySelector('h1')).toBeInTheDocument()
    })

    const headings = container.querySelectorAll('h1, h2, h3, h4, h5, h6')
    expect(headings.length).toBeGreaterThan(0)

    const h1 = container.querySelector('h1')
    expect(h1).toBeInTheDocument()
  })

  it('should have aria-labels on progress bars', () => {
    const { container } = render(<UsagePage />)

    return waitFor(() => {
      const progress = container.querySelectorAll('progress')
      progress.forEach((bar) => {
        expect(bar).toHaveAttribute('aria-label')
      })
      expect(progress.length).toBeGreaterThan(0)
    })
  })

  it('should have proper contrast for status indicators', async () => {
    const { container } = render(<UsagePage />)
    await waitFor(() => {
      expect(container.querySelector('h1')).toBeInTheDocument()
    })

    // Axe will check color contrast automatically
    const results = await axe(container, {
      rules: {
        'color-contrast': { enabled: true }
      }
    })
    
    expect(results).toHaveNoViolations()
  })
})
