import { AuctionsClient } from './AuctionsClient'
import {
  TRANSPARENCY_AUCTION_PAGE_SIZE,
  type TransparencyFilterParams,
  sanitizeFilters,
} from './filterUtils'
import { Section, Container, PageHeader } from '@/components/ui'

type PageProps = {
  searchParams?: Promise<Record<string, string | string[] | undefined>>
}

function pickParam(value?: string | string[]): string | undefined {
  if (Array.isArray(value)) return value[0]
  return value ?? undefined
}

export default async function TransparencyAuctionsPage(props: PageProps) {
  const searchParams = (await props.searchParams) ?? {}
  const pageParam = pickParam(searchParams.page)
  const initialPage = Number.parseInt(pageParam || '1', 10)
  const safePage = Number.isFinite(initialPage) && initialPage > 0 ? initialPage : 1

  const rawFilters: TransparencyFilterParams = {
    from: pickParam(searchParams.from),
    to: pickParam(searchParams.to),
    placement_id: pickParam(searchParams.placement_id),
    surface: pickParam(searchParams.surface),
    geo: pickParam(searchParams.geo),
  }

  const sanitizedFilters = sanitizeFilters(rawFilters)

  return (
    <>
      <PageHeader
        kicker="Transparency System"
        title="Auctions"
        subtitle="Live auction samples with verification hooks"
      />
      <Section>
        <Container>
          <AuctionsClient
            initialPage={safePage}
            initialFilters={sanitizedFilters}
            initialData={null}
            initialError={null}
          />
        </Container>
      </Section>
    </>
  )
}
