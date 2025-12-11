import React from 'react'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { axe } from 'jest-axe'
import Navigation from './Navigation'

jest.mock('next/navigation', () => ({
  usePathname: () => '/dashboard',
}))

jest.mock('@/lib/useSession', () => ({
  useSession: () => ({
    user: { email: 'demo@example.com', role: 'publisher' },
    isLoading: false,
    error: null,
    refetch: jest.fn(),
    logout: { mutate: jest.fn() },
  }),
}))

jest.mock('@/lib/usePlanAccess', () => ({
  usePlanAccess: () => ({ allowed: true, isLoading: false, plan: 'growth', error: null }),
}))

const mockUseFeatures = jest.fn()

jest.mock('@/lib/useFeatures', () => ({
  useFeatures: () => mockUseFeatures(),
}))

describe('Navigation component — accessibility', () => {
  beforeEach(() => {
    mockUseFeatures.mockReturnValue({
      features: { transparency: true, billing: true, migrationStudio: true },
      loading: false,
      error: null,
      refresh: jest.fn(),
    })
  })

  it('has no detectable a11y violations with the default feature set', async () => {
    const { container } = render(
      <Navigation>
        <main>Content</main>
      </Navigation>
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('maintains accessibility when all gated features are disabled', async () => {
    mockUseFeatures.mockReturnValue({
      features: { transparency: false, billing: false, migrationStudio: false },
      loading: false,
      error: null,
      refresh: jest.fn(),
    })

    const { container } = render(
      <Navigation>
        <main>Content</main>
      </Navigation>
    )

    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('exposes a keyboard-focusable mobile toggle that updates aria labels', async () => {
    const user = userEvent.setup()
    render(
      <Navigation>
        <main>Content</main>
      </Navigation>
    )

    const toggle = screen.getByRole('button', { name: /open menu/i })
    expect(toggle).toHaveAttribute('aria-expanded', 'false')

    await user.click(toggle)
    expect(toggle).toHaveAttribute('aria-label', 'Close menu')
    expect(toggle).toHaveAttribute('aria-expanded', 'true')
  })

  it('places the mobile toggle first in the tab order for keyboard users', async () => {
    const user = userEvent.setup()
    render(
      <Navigation>
        <main>Content</main>
      </Navigation>
    )

    await user.tab()
    expect(screen.getByRole('button', { name: /open menu/i })).toHaveFocus()
  })
})
