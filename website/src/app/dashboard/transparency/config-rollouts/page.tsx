import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

export const dynamic = 'force-static';

export default function TransparencyConfigRolloutsPage() {
  return (
    <Section>
      <Container className="space-y-6">
        <header className="flex items-center justify-between">
          <div>
            <h1 className="text-xl font-semibold">Transparency — Config Rollouts</h1>
            <p className="text-sm text-gray-600">View current rollout status, SLO widgets, and manage rollout progression.</p>
          </div>
          <div className="flex items-center gap-2">
            <button className="btn-ghost">Pause</button>
            <button className="btn-secondary">Advance</button>
            <button
              className="btn-primary !bg-[var(--danger)] hover:!bg-[#b91c1c]"
              aria-label="Rollback configuration"
            >
              Rollback
            </button>
          </div>
        </header>

        {/* Current rollout card */}
        <div className="card-v2">
          <div className="card-v2-header">
            <h2 className="text-sm font-semibold">Current Rollout</h2>
            <div className="text-xs text-gray-500">Version 1.0.0 • 35% • 1h 20m remaining</div>
          </div>
          <div className="card-v2-body grid grid-cols-1 gap-4 md:grid-cols-3">
            <div className="rounded-[12px] border border-gray-200 p-3">
              <div className="text-xs text-gray-500">Crash-free</div>
              <div className="text-lg font-semibold text-gray-900">99.9%</div>
            </div>
            <div className="rounded-[12px] border border-gray-200 p-3">
              <div className="text-xs text-gray-500">ANR</div>
              <div className="text-lg font-semibold text-gray-900">0.02%</div>
            </div>
            <div className="rounded-[12px] border border-gray-200 p-3">
              <div className="text-xs text-gray-500">Timeouts</div>
              <div className="text-lg font-semibold text-gray-900">0.8%</div>
            </div>
          </div>
        </div>

        {/* History */}
        <div className="card-v2">
          <div className="card-v2-header">
            <h2 className="text-sm font-semibold">History</h2>
            <div className="text-xs text-gray-500">Last 10 versions</div>
          </div>
          <div className="card-v2-body minh-table">
            <div className="text-sm text-gray-600">History table loads here…</div>
          </div>
        </div>
      </Container>
    </Section>
  );
}
