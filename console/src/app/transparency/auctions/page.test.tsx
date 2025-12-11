import React from 'react'
import { act, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import userEvent from '@testing-library/user-event'
import { useRouter, useSearchParams, usePathname } from 'next/navigation'
import { AuctionsClient } from './AuctionsClient'
import type { TransparencyAuction } from '@/lib/transparency'
import type { AuctionsResponse } from './filterUtils'

const mockIsFeatureEnabled = jest.fn((flag: string) => {
  if (flag === 'transparencyRefresh') return true
  return true
})

jest.mock('@/lib/featureFlags', () => ({
  isFeatureEnabled: (flag: string) => mockIsFeatureEnabled(flag),
}))

jest.mock('@/lib/usePlanAccess', () => ({
  usePlanAccess: () => ({ allowed: true, isLoading: false, plan: 'growth', error: null }),
}))

jest.mock('next/navigation', () => ({
  useRouter: jest.fn(),
  useSearchParams: jest.fn(),
  usePathname: jest.fn(),
}))

const mockTransparencyApi = {
  list: jest.fn(),
  verify: jest.fn(),
}

jest.mock('../../../lib/transparency', () => ({
  transparencyApi: {
    list: (...args: any[]) => mockTransparencyApi.list(...args),
    verify: (...args: any[]) => mockTransparencyApi.verify(...args),
  },
}))

const originalClipboardDescriptor = Object.getOwnPropertyDescriptor(navigator, 'clipboard')
const originalSecureContextDescriptor = Object.getOwnPropertyDescriptor(window, 'isSecureContext')

type MockListResponse = AuctionsResponse

const baseAuction: TransparencyAuction = {
  auction_id: 'auc-123',
  timestamp: new Date('2025-11-10T12:00:00.000Z').toISOString(),
  publisher_id: 'pub-1',
  app_or_site_id: 'app-1',
  placement_id: 'pl-4567890',
  surface_type: 'mobile_app',
  device_context: { os: 'ios', geo: 'US', att: 'authorized', tc_string_sha256: '0'.repeat(64) },
  candidates: [],
  winner: { source: 'alpha', bid_ecpm: 1.23, gross_price: 1.11, currency: 'USD', reason: 'highest_bid' },
  fees: { aletheia_fee_bp: 150, effective_publisher_share: 0.985 },
  integrity: { signature: 'sig', algo: 'ed25519', key_id: 'key-1' },
}

const buildMockResponse = (overrides: Partial<MockListResponse> = {}): MockListResponse => ({
  page: 1,
  limit: 25,
  count: 1,
  data: [baseAuction],
  ...overrides,
})

const mockResponse: MockListResponse = buildMockResponse()

describe('Transparency Auctions Page', () => {
  let routerPush: jest.Mock
  let routerReplace: jest.Mock
  let clipboardWriteMock: jest.Mock

  beforeEach(() => {
    routerPush = jest.fn()
    routerReplace = jest.fn()
    clipboardWriteMock = jest.fn().mockResolvedValue(undefined)

    if (originalSecureContextDescriptor?.configurable !== false) {
      Object.defineProperty(window, 'isSecureContext', {
        configurable: true,
        value: true,
      })
    }

    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      get: () => ({ writeText: clipboardWriteMock }),
    })

  ;(useRouter as jest.Mock).mockReturnValue({ push: routerPush, replace: routerReplace })
    ;(useSearchParams as jest.Mock).mockReturnValue(new URLSearchParams())
    ;(usePathname as jest.Mock).mockReturnValue('/transparency/auctions')

    mockTransparencyApi.list.mockResolvedValue(mockResponse)
    mockTransparencyApi.verify.mockResolvedValue({ status: 'pass', key_id: 'key-1' })
    mockIsFeatureEnabled.mockImplementation((flag: string) => {
      if (flag === 'transparencyRefresh') return true
      return true
    })
  })

  afterEach(() => {
    jest.clearAllMocks()
    jest.useRealTimers()
  })

  afterAll(() => {
    if (originalClipboardDescriptor) {
      Object.defineProperty(navigator, 'clipboard', originalClipboardDescriptor)
    } else {
      delete (navigator as any).clipboard
    }

    if (originalSecureContextDescriptor) {
      Object.defineProperty(window, 'isSecureContext', originalSecureContextDescriptor)
    } else {
      delete (window as any).isSecureContext
    }
  })

  const renderClient = async ({ waitForData = true, initialData = mockResponse, initialError = null }: { waitForData?: boolean; initialData?: MockListResponse | null; initialError?: string | null } = {}) => {
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false, gcTime: 0 },
      },
    })

    let utils: ReturnType<typeof render>

    await act(async () => {
      utils = render(
        <QueryClientProvider client={queryClient}>
          <AuctionsClient
            initialPage={1}
            initialFilters={{}}
            initialData={initialData}
            initialError={initialError}
          />
        </QueryClientProvider>
      )

      if (waitForData) {
        await Promise.resolve()
      }
    })

    return utils!
  }

  it('renders the page heading and filters once data loads', async () => {
  await renderClient()

    await waitFor(() => expect(mockTransparencyApi.list).toHaveBeenCalled())

    expect(screen.getByRole('heading', { name: /transparency — auctions/i })).toBeInTheDocument()
    expect(screen.getByLabelText(/from date/i)).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: /placement id/i })).toBeInTheDocument()
  })

  it('shows a skeleton table while data is loading', async () => {
  mockTransparencyApi.list.mockImplementation(() => new Promise(() => {}))
  await renderClient({ waitForData: false, initialData: null })

    expect(screen.getByRole('table')).toBeInTheDocument()
  })

  it('renders empty state messaging when no auctions exist', async () => {
  mockTransparencyApi.list.mockResolvedValue({ ...mockResponse, count: 0, data: [] })

  await renderClient({ initialData: null })

    await waitFor(() => expect(screen.getByText(/no auctions found/i)).toBeInTheDocument())
  })

  it('renders error state when the API call fails', async () => {
    mockTransparencyApi.list.mockRejectedValue(new Error('Network error'))

    await renderClient({ initialData: null })

    await waitFor(() => {
      expect(screen.getByText(/unable to refresh auctions/i)).toBeInTheDocument()
      expect(screen.getByText(/network error/i)).toBeInTheDocument()
    })
  })

  it('copies the full auction identifier using the clipboard API', async () => {
    const user = userEvent.setup()
  await renderClient()

    const copyButton = await screen.findByRole('button', { name: /copy auction id auc-123/i })
    await user.click(copyButton)

    await waitFor(() => {
      const statusRegion = screen.getByRole('status', { hidden: true })
      expect(statusRegion).toHaveTextContent(/copied/i)
    })
  })

  it('verifies signed auctions on demand via the Verify badge control', async () => {
  const user = userEvent.setup()
  await renderClient()

    const verifyButton = await screen.findByRole('button', { name: /verify auction auc-123/i })
    await user.click(verifyButton)

    await waitFor(() => expect(mockTransparencyApi.verify).toHaveBeenCalledWith('auc-123', expect.anything()))
    expect(screen.getByText(/pass/i)).toBeInTheDocument()
  })

  it('syncs placement filter changes to the URL after the debounce interval', async () => {
  jest.useFakeTimers()
  const user = userEvent.setup({ advanceTimers: jest.advanceTimersByTime })
  await renderClient()

    await waitFor(() => expect(mockTransparencyApi.list).toHaveBeenCalled())
    const placementInput = screen.getByRole('textbox', { name: /placement id/i })
    await user.type(placementInput, 'rewarded-home')

    act(() => {
      jest.advanceTimersByTime(350)
    })

    expect(routerReplace).toHaveBeenCalledWith('/transparency/auctions?placement_id=rewarded-home', { scroll: false })
  })

  it('exposes accessible auction links with the full identifier as the label', async () => {
  await renderClient()

    const auctionLink = await screen.findByRole('link', { name: /view auction auc-123/i })
    expect(auctionLink).toHaveAttribute('href', '/transparency/auctions/auc-123')
  })

  it('hides the manual refresh button when the feature flag is disabled', async () => {
    mockIsFeatureEnabled.mockImplementationOnce(() => false)

    await renderClient()

    expect(screen.queryByRole('button', { name: /refresh auctions/i })).not.toBeInTheDocument()
    expect(screen.getByText(/manual refresh disabled/i)).toBeInTheDocument()
  })
})
