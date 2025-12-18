import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { configureAxe } from 'jest-axe'
import BillingSettingsPage from './page'

const ORIGINAL_ENV = process.env

function mockGetBillingSettings() {
  const globalKey = '__billingSettingsMock'
  const existing = (globalThis as Record<string, unknown>)[globalKey]
  if (existing) {
    return existing as {
      plan: {
        name: string
        type: 'starter'
        platform_fee_rate: number
        platform_fee_label: string
        revenue_band: string
        sample_fee_note?: string
        currency: string
        included_impressions: number
        included_api_calls: number
        included_data_transfer_gb: number
      }
      billing_email: string
      receipt_preferences: {
        send_receipts: boolean
        send_invoices: boolean
        send_usage_alerts: boolean
      }
      stripe_customer_id: string
    }
  }

  const settings = Object.freeze({
    plan: {
      name: 'Tier 0 — Starter',
      type: 'starter' as const,
      platform_fee_rate: 0,
      platform_fee_label: '0% platform fee',
      revenue_band: 'Covers up to $10k mediated revenue per app per month',
      sample_fee_note: 'Upgrade automatically once you exceed $10k/mo for any app',
      currency: 'usd',
      included_impressions: 1000000,
      included_api_calls: 100000,
      included_data_transfer_gb: 50,
    },
    billing_email: 'billing@example.com',
    receipt_preferences: Object.freeze({
      send_receipts: true,
      send_invoices: true,
      send_usage_alerts: true,
    }),
    stripe_customer_id: 'cus_123',
  })

  ;(globalThis as Record<string, unknown>)[globalKey] = settings
  return settings
}

jest.mock('@/lib/api-client', () => ({
  apiClient: {
    get: jest.fn().mockResolvedValue({
      data: mockGetBillingSettings(),
    }),
    put: jest.fn().mockResolvedValue({ data: {} }),
    post: jest.fn().mockResolvedValue({ data: { url: 'https://portal.stripe.com' } }),
  },
}))

jest.mock('@/lib/useSession', () => ({
  useSession: () => ({
    user: {
      userId: 'user-1',
      publisherId: 'pub-1',
      email: 'billing@test.dev',
      role: 'admin',
      permissions: ['billing:view', 'billing:manage'],
    },
    isLoading: false,
  }),
}))

jest.mock('@tanstack/react-query', () => ({
  useQuery: jest.fn(() => ({
    data: mockGetBillingSettings(),
    isLoading: false,
  })),
  useMutation: jest.fn(() => ({
    mutate: jest.fn(),
    isPending: false,
  })),
  useQueryClient: jest.fn(() => ({
    invalidateQueries: jest.fn(),
  })),
}))

const axe = configureAxe({
  rules: {
    'color-contrast': { enabled: true },
  },
})

beforeEach(() => {
  process.env = { ...ORIGINAL_ENV }
  delete process.env.NEXT_PUBLIC_ENABLE_BILLING_MIGRATION
})

afterAll(() => {
  process.env = ORIGINAL_ENV
})

describe('BillingSettingsPage Accessibility', () => {
  it('should have no accessibility violations', async () => {
    const { container } = render(<BillingSettingsPage />)

    await waitFor(() => {
      expect(screen.getByRole('heading', { level: 1, name: /billing settings/i })).toBeInTheDocument()
    })

    const results = await axe(container, { resultTypes: ['violations'] })
    expect(results).toHaveNoViolations()
  })

  it('should have proper form labels', () => {
    const { container } = render(<BillingSettingsPage />)
    
    const inputs = container.querySelectorAll('input[type="email"], input[type="checkbox"]')
    inputs.forEach(input => {
      const id = input.getAttribute('id')
      if (id) {
        const label = container.querySelector(`label[for="${id}"]`)
        expect(label).toBeInTheDocument()
      }
    })
  })

  it('should have accessible checkboxes with descriptions', () => {
    const { container } = render(<BillingSettingsPage />)
    
    const checkboxes = container.querySelectorAll('input[type="checkbox"]')
    expect(checkboxes.length).toBeGreaterThan(0)
    
    checkboxes.forEach(checkbox => {
      const label = checkbox.closest('label')
      expect(label).toBeInTheDocument()
    })
  })

  it('should have accessible buttons', () => {
    const { getAllByRole } = render(<BillingSettingsPage />)
    
    const buttons = getAllByRole('button')
    buttons.forEach(button => {
      expect(button).toHaveAttribute('type')
      expect(button.textContent).toBeTruthy()
    })
  })

  it('should have proper heading hierarchy', () => {
    const { container } = render(<BillingSettingsPage />)
    
    const h1 = container.querySelector('h1')
    expect(h1).toBeInTheDocument()
    
    const h2s = container.querySelectorAll('h2')
    expect(h2s.length).toBeGreaterThan(0)
  })

  it('should have accessible external link to Stripe Portal', async () => {
    render(<BillingSettingsPage />)

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /manage via stripe portal/i })).toBeInTheDocument()
    })
  })

  it('should surface contextual help with accessible messaging', async () => {
    render(<BillingSettingsPage />)

    await waitFor(() => {
      expect(screen.getByText(/Need to upgrade or change your plan/i)).toBeInTheDocument()
    })

    const salesLink = screen.getByRole('link', { name: /contact sales/i })
    expect(salesLink).toHaveAttribute('href', 'mailto:billing@apexmediation.ee')
  })

  it('supports keyboard navigation for primary actions', async () => {
    render(<BillingSettingsPage />)

    await waitFor(() => {
      expect(screen.getByRole('link', { name: /view usage/i })).toBeInTheDocument()
    })

    const user = userEvent.setup()

    await user.tab()
    expect(screen.getByRole('link', { name: /contact sales/i })).toHaveFocus()

    await user.tab()
    expect(screen.getByRole('link', { name: /view usage/i })).toHaveFocus()

    await user.tab()
    expect(screen.getByRole('button', { name: /manage via stripe portal/i })).toHaveFocus()

    await user.tab()
    const emailInput = screen.getByLabelText(/email address/i)
    expect(emailInput).toHaveFocus()

    await user.clear(emailInput)
    await user.type(emailInput, 'new-billing@example.com')

    await user.tab()
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /update/i, hidden: false })).toHaveFocus()
    })
  })

  it('reveals the migration assistant when the feature flag is enabled', async () => {
    process.env.NEXT_PUBLIC_ENABLE_BILLING_MIGRATION = 'true'

    render(<BillingSettingsPage />)

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: /migration assistant/i })).toBeInTheDocument()
    })
  })
})
