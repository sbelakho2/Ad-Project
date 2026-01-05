import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { configureAxe } from 'jest-axe'
import BillingSettingsPage from './page'

const ORIGINAL_ENV = process.env

function mockGetBillingPolicy() {
  const globalKey = '__billingPolicyMock'
  const existing = (globalThis as Record<string, unknown>)[globalKey]
  if (existing) return existing

  const policy = Object.freeze({
    summary: 'Stripe-first billing with transparent platform fees.',
    starterExperience: {
      paymentMethodRequired: false,
      revenueCapUsd: 10000,
      capabilities: ['Basic dashboards', 'SDKs', 'Migration Studio'],
      messaging: 'Upgrade once you exceed the revenue cap.',
    },
    upgradePath: {
      triggers: ['Exceed Starter revenue cap'],
      acceptedPaymentMethods: [],
      autopay: {
        defaultBehavior: 'Stripe auto-charges the primary method.',
        encouragement: 'Enable autopay to avoid downtime.',
        enterpriseOverride: 'Enterprise may request invoices.',
      },
    },
    billingCycle: {
      cadence: 'monthly',
      notifications: [],
    },
    transparencyCommitments: {
      weDontTouchPayouts: 'We do not touch payouts.',
      platformFeeOnly: 'Platform fee only.',
    },
  })

  ;(globalThis as Record<string, unknown>)[globalKey] = policy
  return policy
}

jest.mock('@/lib/api-client', () => ({
  apiClient: {
    get: jest.fn(async (url: string) => {
      if (url === '/billing/policy') {
        return { data: mockGetBillingPolicy() }
      }
      return { data: {} }
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
    data: mockGetBillingPolicy(),
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
      expect(screen.getByRole('heading', { level: 1, name: /billing/i })).toBeInTheDocument()
    })

    const results = await axe(container, { resultTypes: ['violations'] })
    expect(results).toHaveNoViolations()
  })

  it('should have accessible primary actions', async () => {
    render(<BillingSettingsPage />)

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /open stripe portal/i })).toBeInTheDocument()
    })

    expect(screen.getByRole('link', { name: /view usage/i })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /view invoices/i })).toBeInTheDocument()
  })

  it('should have proper heading hierarchy', () => {
    const { container } = render(<BillingSettingsPage />)
    
    const h1 = container.querySelector('h1')
    expect(h1).toBeInTheDocument()
    
    const h2s = container.querySelectorAll('h2')
    expect(h2s.length).toBeGreaterThan(0)
  })

  it('supports keyboard navigation for primary actions', async () => {
    render(<BillingSettingsPage />)

    await waitFor(() => {
      expect(screen.getByRole('link', { name: /view usage/i })).toBeInTheDocument()
    })

    const user = userEvent.setup()

    await user.tab()
    expect(screen.getByRole('button', { name: /open stripe portal/i })).toHaveFocus()

    await user.tab()
    expect(screen.getByRole('link', { name: /view usage/i })).toHaveFocus()

    await user.tab()
    expect(screen.getByRole('link', { name: /view invoices/i })).toHaveFocus()
  })

  it('reveals the migration assistant when the feature flag is enabled', async () => {
    process.env.NEXT_PUBLIC_ENABLE_BILLING_MIGRATION = 'true'

    render(<BillingSettingsPage />)

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: /billing migration assistant/i })).toBeInTheDocument()
    })

    expect(screen.getByLabelText(/target channel/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/notes/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /submit request/i })).toBeInTheDocument()
  })
})
