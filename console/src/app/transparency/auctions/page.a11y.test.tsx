import React from 'react'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { axe } from 'jest-axe'
import { useRouter, useSearchParams, usePathname } from 'next/navigation'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import Page from './page'

// Mock Next.js navigation hooks
jest.mock('next/navigation', () => ({
  useRouter: jest.fn(),
  useSearchParams: jest.fn(),
  usePathname: jest.fn(),
}))

jest.mock('../../../lib/hooks', () => {
  const actual = jest.requireActual('../../../lib/hooks')
  return {
    ...actual,
    useDebouncedValue: <T,>(value: T) => value,
  }
})

jest.mock('../../../lib/usePlanAccess', () => ({
  usePlanAccess: () => ({ allowed: true, isLoading: false, plan: 'growth', error: null }),
}))

// Mock API client
const mockTransparencyApi = {
  list: jest.fn(),
}

jest.mock('../../../lib/transparency', () => ({
  transparencyApi: {
    list: (...args: any[]) => mockTransparencyApi.list(...args),
  },
}))

// Mock next/link to avoid act warnings from Link component internals
jest.mock('next/link', () => {
  const MockLink = ({ children, href, ...props }: any) => (
    <a href={href} {...props}>
      {children}
    </a>
  )
  MockLink.displayName = 'MockNextLink'
  return MockLink
})

describe('Transparency Auctions Page — a11y and keyboard navigation', () => {
  beforeEach(() => {
    jest.clearAllMocks()
    ;(useRouter as jest.Mock).mockReturnValue({ push: jest.fn() })
    ;(useSearchParams as jest.Mock).mockReturnValue(new URLSearchParams())
    ;(usePathname as jest.Mock).mockReturnValue('/transparency/auctions')

    const sampleAuction = {
      auction_id: 'auc-123',
      timestamp: new Date('2025-11-10T12:00:00.000Z').toISOString(),
      publisher_id: 'pub-1',
      app_or_site_id: 'app-1',
      placement_id: 'pl-456',
      surface_type: 'mobile_app',
      device_context: { os: 'ios', geo: 'US', att: 'authorized', tc_string_sha256: '0'.repeat(64) },
      candidates: [],
      winner: { source: 'alpha', bid_ecpm: 1.23, gross_price: 1.11, currency: 'USD', reason: 'highest_bid' },
      fees: { aletheia_fee_bp: 150, effective_publisher_share: 0.985 },
      integrity: { signature: 'sig', algo: 'ed25519', key_id: 'key-1' },
    }

    const dataset = Array.from({ length: 25 }, (_, index) => {
      if (index === 0) return sampleAuction
      return {
        ...sampleAuction,
        auction_id: `auc-${(index + 200).toString()}`,
        placement_id: `pl-${index + 500}`,
        timestamp: new Date(Date.now() - index * 60_000).toISOString(),
      }
    })

    mockTransparencyApi.list.mockResolvedValue({
      page: 1,
      limit: 25,
      count: dataset.length,
      data: dataset,
    })
  })

  const renderPage = async () => {
    const tree = await Page({})
    const queryClient = new QueryClient()
    return render(
      <QueryClientProvider client={queryClient}>{tree as React.ReactElement}</QueryClientProvider>
    )
  }

  it('has no axe violations with critical rules enabled', async () => {
    const { container } = await renderPage()
    await waitFor(() => screen.getByRole('table', { name: /Auctions table/i }))
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('supports predictable keyboard traversal through filters and actions', async () => {
    await renderPage()
    await waitFor(() => screen.getByRole('table', { name: /Auctions table/i }))

    const user = userEvent.setup()
    const tabSequence: HTMLElement[] = [
      screen.getAllByLabelText(/From date/i)[0],
      screen.getAllByLabelText(/To date/i)[0],
      screen.getAllByLabelText(/Placement ID/i)[0],
      screen.getAllByLabelText(/Surface/i)[0],
      screen.getAllByLabelText(/Geo/i)[0],
      screen.getAllByRole('link', { name: /View auction auc-123/i })[0],
      screen.getByRole('button', { name: /Verify auction auc-123/i }),
    ]

    const focusNextElement = async (element: HTMLElement) => {
      let guard = 0
      while (document.activeElement !== element && guard < 50) {
        await user.tab()
        guard++
      }
      expect(element).toHaveFocus()
    }

    for (const element of tabSequence) {
      await focusNextElement(element)
    }

    const nextButton = screen.getByRole('button', { name: /Next/i })
    if (!nextButton.hasAttribute('disabled')) {
      await focusNextElement(nextButton)
    } else {
      expect(nextButton).toBeDisabled()
    }

    const verifyButton = screen.getByRole('button', { name: /Verify auction auc-123/i })
    await focusNextElement(verifyButton)
  })
})
