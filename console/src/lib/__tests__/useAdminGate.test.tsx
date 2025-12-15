import { renderHook, waitFor } from '@testing-library/react'
import { useAdminGate } from '../useAdminGate'

const mockIsFeatureEnabled = jest.fn((flag?: string) => true)

jest.mock('@/lib/featureFlags', () => ({
  isFeatureEnabled: (flag: string) => mockIsFeatureEnabled(flag),
}))

const mockRouterReplace = jest.fn()
let mockPathname = '/admin/health'

jest.mock('next/navigation', () => ({
  useRouter: () => ({
    replace: mockRouterReplace,
  }),
  usePathname: () => mockPathname,
}))

const mockFetch = jest.fn()

beforeAll(() => {
  ;(global as any).fetch = mockFetch
})

describe('useAdminGate', () => {
  beforeEach(() => {
    jest.clearAllMocks()
    mockPathname = '/admin/health'
    window.history.replaceState({}, 'Test', mockPathname)
    mockIsFeatureEnabled.mockImplementation(() => true)
    mockFetch.mockResolvedValue({ ok: false, status: 401 })
  })

  it('redirects non-operators to 403 once', async () => {
    renderHook(() => useAdminGate())

    await waitFor(() => {
      expect(mockRouterReplace).toHaveBeenCalledTimes(1)
      expect(mockRouterReplace).toHaveBeenCalledWith('/403')
    })

    mockRouterReplace.mockClear()

    await waitFor(() => {
      expect(mockRouterReplace).not.toHaveBeenCalled()
    })
  })

  it('does not redirect operators and returns gate status', async () => {
    mockFetch.mockResolvedValue({
      ok: true,
      json: async () => ({ ok: true, identifier: 'operator@apexmediation.ee' }),
    })

    const { result } = renderHook(() => useAdminGate())

    await waitFor(() => {
      expect(result.current.isOperator).toBe(true)
      expect(result.current.isAdmin).toBe(true)
      expect(result.current.user?.email).toBe('operator@apexmediation.ee')
    })

    expect(mockRouterReplace).not.toHaveBeenCalled()
  })

  it('can disable redirects for custom handling', async () => {
    renderHook(() => useAdminGate({ disableRedirects: true }))

    await waitFor(() => {
      expect(mockRouterReplace).not.toHaveBeenCalled()
    })
  })

  it('respects the requireAdminGuard flag when disabled', async () => {
    mockIsFeatureEnabled.mockImplementation(() => false)

    renderHook(() => useAdminGate())

    await waitFor(() => {
      expect(mockRouterReplace).not.toHaveBeenCalled()
    })
  })
})
