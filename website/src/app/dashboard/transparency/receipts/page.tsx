import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

export const dynamic = 'force-static';

export default function TransparencyReceiptsPage() {
  return (
    <Section>
      <Container className="space-y-6">
        <header className="flex items-center justify-between">
          <div>
            <h1 className="text-xl font-semibold">Transparency — Receipts</h1>
            <p className="text-sm text-gray-600">Browse signed request receipts. Filter by date range, placement, adapter, and status.</p>
          </div>
          <div className="flex items-center gap-2">
            <button className="btn-secondary">Export CSV</button>
          </div>
        </header>

        {/* Bulk actions bar placeholder (appears when rows are selected) */}
        <div className="rounded-[12px] border border-gray-200 bg-gray-50 p-3 text-sm text-gray-600" aria-hidden>
          <div className="flex items-center justify-between">
            <span>Bulk actions: 0 selected</span>
            <div className="flex items-center gap-2">
              <button className="btn-ghost" disabled aria-disabled>
                Export
              </button>
              <button className="btn-ghost" disabled aria-disabled>
                Mark as Verified
              </button>
              <button className="btn-ghost text-danger" disabled aria-disabled>
                Delete
              </button>
            </div>
          </div>
        </div>

        {/* Filters */}
        <div className="card-v2">
          <div className="card-v2-body grid grid-cols-1 gap-3 md:grid-cols-4">
            <input className="input-v2" placeholder="Date range" aria-label="Date range" />
            <input className="input-v2" placeholder="Placement" aria-label="Placement" />
            <input className="input-v2" placeholder="Adapter" aria-label="Adapter" />
            <select className="input-v2" aria-label="Status">
              <option value="">Status</option>
              <option value="ok">OK</option>
              <option value="timeout">Timeout</option>
              <option value="no_fill">No fill</option>
              <option value="error">Error</option>
            </select>
          </div>
        </div>

        {/* Table skeleton */}
        <div className="card-v2">
          <div className="card-v2-header">
            <h2 className="text-sm font-semibold">Receipts</h2>
            <div className="text-xs text-gray-500">Most recent 100</div>
          </div>
          <div className="card-v2-body minh-table">
            <div className="text-sm text-gray-600">Table loads here…</div>
          </div>
        </div>
      </Container>
    </Section>
  );
}
