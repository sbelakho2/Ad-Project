import React from 'react'
import { render, screen } from '@testing-library/react'
import Navigation from './Navigation'

const mockUsePathname = jest.fn()
jest.mock('next/navigation', () => ({
  usePathname: () => mockUsePathname(),
}))

jest.mock('next/link', () => {
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const React = require('react')
  const MockLink = ({ href, children, ...props }: any) =>
    React.createElement('a', { href, ...props }, children)
  MockLink.displayName = 'MockNextLink'
  return MockLink
})

const mockUseSession = jest.fn()
jest.mock('@/lib/useSession', () => ({
  useSession: () => mockUseSession(),
}))

jest.mock('@/lib/usePlanAccess', () => ({
  usePlanAccess: () => ({ allowed: true, isLoading: false, plan: 'growth', error: null }),
}))

const mockUseFeatures = jest.fn()
jest.mock('@/lib/useFeatures', () => ({
  useFeatures: () => mockUseFeatures(),
}))

const renderNavigation = () =>
  render(
    <Navigation>
      <main>children</main>
    </Navigation>
  )

const buildFeaturesState = (features: Record<string, boolean> = {}) => ({
  features,
  loading: false,
  error: null,
  refresh: jest.fn(),
})

describe('Navigation feature flags', () => {
  beforeEach(() => {
    mockUsePathname.mockReturnValue('/dashboard')
    mockUseSession.mockReturnValue({
      user: { email: 'demo@example.com', role: 'publisher' },
      isLoading: false,
    })
    mockUseFeatures.mockReturnValue(buildFeaturesState())
  })

  afterEach(() => {
    jest.clearAllMocks()
  })

  it('shows Migration Studio link when feature is enabled', () => {
    mockUseFeatures.mockReturnValue(buildFeaturesState({ migrationStudio: true }))
    renderNavigation()

    expect(screen.getByRole('link', { name: /Migration Studio/i })).toBeInTheDocument()
  })

  it('hides Migration Studio link when feature is disabled', () => {
    renderNavigation()

    expect(screen.queryByRole('link', { name: /Migration Studio/i })).not.toBeInTheDocument()
  })

  it('shows transparency and billing links when features enabled', () => {
    mockUseFeatures.mockReturnValue(buildFeaturesState({ transparency: true, billing: true }))
    renderNavigation()

    expect(screen.getByRole('link', { name: /Transparency/i })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Billing/i })).toBeInTheDocument()
  })

  it('hides transparency and billing links when features disabled', () => {
    mockUseFeatures.mockReturnValue(buildFeaturesState({ transparency: false, billing: false }))
    renderNavigation()

    expect(screen.queryByRole('link', { name: /Transparency/i })).not.toBeInTheDocument()
    expect(screen.queryByRole('link', { name: /Billing/i })).not.toBeInTheDocument()
  })

  it('renders admin link only for admin role', () => {
    mockUseSession.mockReturnValue({
      user: { email: 'publisher@example.com', role: 'publisher' },
      isLoading: false,
    })
    renderNavigation()
    expect(screen.queryByRole('link', { name: /Admin/i })).not.toBeInTheDocument()

    mockUseSession.mockReturnValue({
      user: { email: 'admin@example.com', role: 'admin' },
      isLoading: false,
    })
    renderNavigation()

    expect(screen.getByRole('link', { name: /Admin/i })).toBeInTheDocument()
  })
})
