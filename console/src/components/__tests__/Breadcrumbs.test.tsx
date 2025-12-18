import { buildBreadcrumbsFromPath } from '../Breadcrumbs'

describe('buildBreadcrumbsFromPath', () => {
  it('localizes known route segments', () => {
    const breadcrumbs = buildBreadcrumbsFromPath('/settings/billing/invoices')
    expect(breadcrumbs).toEqual([
      { label: 'Settings', href: '/settings' },
      { label: 'Billing', href: '/settings/billing' },
      { label: 'Invoices', href: '/settings/billing/invoices' },
    ])
  })

  it('masks invoice identifiers to avoid leaking full values', () => {
    const breadcrumbs = buildBreadcrumbsFromPath('/settings/billing/invoices/1234567890abcdef')
    const lastCrumb = breadcrumbs[breadcrumbs.length - 1]
    expect(lastCrumb).toEqual({
      label: 'Invoice #12345678…',
      href: '/settings/billing/invoices/1234567890abcdef',
    })
  })

  it('formats UUID-like segments as localized identifiers', () => {
    const breadcrumbs = buildBreadcrumbsFromPath('/transparency/auctions/123e4567-e89b-12d3-a456-426614174000')
    const lastCrumb = breadcrumbs[breadcrumbs.length - 1]
    expect(lastCrumb).toEqual({
      label: 'Auction 123e4567…',
      href: '/transparency/auctions/123e4567-e89b-12d3-a456-426614174000',
    })
  })

  it('humanizes hyphenated segments when no translation exists', () => {
    const breadcrumbs = buildBreadcrumbsFromPath('/custom/feature-toggle')
    expect(breadcrumbs[0]).toEqual({ label: 'Custom', href: '/custom' })
    expect(breadcrumbs[1]).toEqual({ label: 'Feature Toggle', href: '/custom/feature-toggle' })
  })
})
