import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

export const dynamic = 'force-static';

export default function ReconciliationPage() {
  return (
    <Section>
      <Container className="space-y-6">
        {/* Header */}
        <header className="flex items-center justify-between">
          <div>
            <h1 className="text-xl font-semibold">Reconciliation</h1>
            <p className="text-sm text-gray-600">
              Import third-party reports, map fields, validate, and reconcile differences.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button className="btn-secondary" aria-label="View help">Runbook</button>
            <button className="btn-primary" aria-label="Start importer wizard">New Import</button>
          </div>
        </header>

        {/* Static step indicator */}
        <nav aria-label="Wizard steps" className="rounded-[12px] border border-gray-200 bg-gray-50 p-3">
          <ol className="flex flex-wrap items-center gap-2 text-xs font-semibold text-gray-700">
            <li className="inline-flex items-center gap-2 rounded-full bg-white px-3 py-1 ring-1 ring-gray-200">
              <span className="inline-block h-2 w-2 rounded-full bg-brand-500" aria-hidden /> Upload CSV
            </li>
            <li className="inline-flex items-center gap-2 rounded-full bg-white px-3 py-1 ring-1 ring-gray-200">
              <span className="inline-block h-2 w-2 rounded-full bg-gray-300" aria-hidden /> Map Columns
            </li>
            <li className="inline-flex items-center gap-2 rounded-full bg-white px-3 py-1 ring-1 ring-gray-200">
              <span className="inline-block h-2 w-2 rounded-full bg-gray-300" aria-hidden /> Validate Preview
            </li>
            <li className="inline-flex items-center gap-2 rounded-full bg-white px-3 py-1 ring-1 ring-gray-200">
              <span className="inline-block h-2 w-2 rounded-full bg-gray-300" aria-hidden /> Commit
            </li>
          </ol>
        </nav>

        {/* Importer Wizard Skeleton */}
        <div className="card-v2">
          <div className="card-v2-header">
            <h2 className="text-sm font-semibold">Importer Wizard</h2>
            <div className="text-xs text-gray-500">Upload → Map Columns → Validate → Commit</div>
          </div>
          <div className="card-v2-body grid grid-cols-1 gap-4 md:grid-cols-2">
            <div className="space-y-3">
              <label className="block text-sm font-medium text-gray-700">CSV File</label>
              <input type="file" className="input-v2" aria-label="Upload CSV file" />
              <p className="text-xs text-gray-500">Maximum 10MB. CSV only.</p>
            </div>
            <div className="space-y-3">
              <label className="block text-sm font-medium text-gray-700">Preset</label>
              <select className="input-v2" aria-label="Select connector preset">
                <option value="">Select connector</option>
                <option>AppLovin</option>
                <option>ironSource</option>
                <option>Unity</option>
                <option>Custom</option>
              </select>
            </div>
            <div className="col-span-1 md:col-span-2 flex items-center justify-end gap-2">
              <button className="btn-ghost" disabled aria-disabled>
                Back
              </button>
              <button className="btn-secondary" disabled aria-disabled>
                Next
              </button>
              <button className="btn-primary" disabled aria-disabled>
                Commit
              </button>
            </div>
          </div>
        </div>

        {/* Discrepancies Table Skeleton */}
        <div className="card-v2">
          <div className="card-v2-header">
            <h2 className="text-sm font-semibold">Discrepancies</h2>
            <div className="text-xs text-gray-500">Recent 50 entries</div>
          </div>
          <div className="card-v2-body minh-table">
            <div className="w-full overflow-x-auto">
              <table className="min-w-full text-left text-sm">
                <thead className="text-gray-600">
                  <tr className="border-b border-gray-200">
                    <th className="py-2 pr-4">Date</th>
                    <th className="py-2 pr-4">Placement</th>
                    <th className="py-2 pr-4">Network</th>
                    <th className="py-2 pr-4">Reported</th>
                    <th className="py-2 pr-4">Observed</th>
                    <th className="py-2 pr-4">Δ</th>
                    <th className="py-2 pr-4">Status</th>
                    <th className="py-2">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td className="py-3 pr-4 text-gray-500" colSpan={8}>
                      Table loads here…
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </Container>
    </Section>
  );
}
