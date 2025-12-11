"use client"

import { useQuery } from '@tanstack/react-query'
import { toolsApi } from '@/lib/api'
import type { SupplyChainAppSummary } from '@/types'
import Link from 'next/link'

function StatusPill({ label, tone }: { label: string; tone: 'ok' | 'warn' | 'muted' }) {
  const colors = {
    ok: 'bg-green-50 text-green-700 border-green-200',
    warn: 'bg-amber-50 text-amber-700 border-amber-200',
    muted: 'bg-gray-50 text-gray-700 border-gray-200',
  } as const
  return <span className={`inline-flex items-center px-3 py-1 rounded-full text-xs font-medium border ${colors[tone]}`}>{label}</span>
}

function AppCard({ app }: { app: SupplyChainAppSummary }) {
  const total = app.ok + app.issues + app.missingDomain
  return (
    <div className="card">
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1">
          <div className="flex items-center gap-2 mb-1">
            <span className="font-semibold text-gray-900">App: {app.appId}</span>
            <StatusPill label={`${app.ok} OK`} tone="ok" />
            {app.issues > 0 && <StatusPill label={`${app.issues} Issues`} tone="warn" />}
            {app.missingDomain > 0 && <StatusPill label={`${app.missingDomain} Missing domain`} tone="warn" />}
          </div>
          <div className="text-xs text-gray-500">Placements assessed: {total}</div>
        </div>
      </div>

      <div className="mt-3 overflow-x-auto">
        <table className="min-w-full text-sm">
          <thead className="bg-gray-50">
            <tr>
              <th className="text-left font-medium text-gray-700 p-2 border-b">Placement</th>
              <th className="text-left font-medium text-gray-700 p-2 border-b">Domain</th>
              <th className="text-left font-medium text-gray-700 p-2 border-b">Seller</th>
              <th className="text-left font-medium text-gray-700 p-2 border-b">Status</th>
              <th className="text-left font-medium text-gray-700 p-2 border-b">Reason</th>
            </tr>
          </thead>
          <tbody>
            {app.placements.map((p) => {
              const tone = p.authorized ? 'ok' : 'warn'
              return (
                <tr key={p.placementId} className="border-b last:border-b-0 align-top">
                  <td className="p-2">
                    <div className="font-medium text-gray-900">{p.placementName}</div>
                    <div className="text-xs text-gray-500 font-mono">{p.placementId}</div>
                  </td>
                  <td className="p-2 text-gray-700">{p.domain || '—'}</td>
                  <td className="p-2 text-gray-700">{p.sellerId || '—'}</td>
                  <td className="p-2">
                    <StatusPill label={p.authorized ? 'Authorized' : 'Attention'} tone={tone as any} />
                  </td>
                  <td className="p-2 text-xs text-gray-700 max-w-sm">{p.reason || '—'}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}

export default function SupplyChainPage() {
  const { data, isLoading, error, refetch, isFetching } = useQuery({
    queryKey: ['supply-chain-summary'],
    queryFn: () => toolsApi.getSupplyChainSummary(),
  })

  const apps = data?.summary.apps ?? []

  return (
    <div className="min-h-screen bg-[#f9fafb] px-6 py-10 space-y-6">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-gray-900">Supply Chain</h1>
          <p className="text-sm text-gray-600">Per-app supply chain status from the ingested app-ads.txt corpus.</p>
          {data?.summary.generatedAt && (
            <p className="text-xs text-gray-500">Generated: {new Date(data.summary.generatedAt).toLocaleString()}</p>
          )}
          {data?.persistedAt && (
            <p className="text-xs text-gray-500">Persisted: {new Date(data.persistedAt).toLocaleString()}</p>
          )}
          {data?.snapshotId && (
            <p className="text-xs text-gray-500">Snapshot ID: {data.snapshotId}</p>
          )}
          {data?.snapshotPath && (
            <p className="text-xs text-gray-500">File snapshot: {data.snapshotPath}</p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="btn btn-outline"
            onClick={() => refetch()}
            disabled={isFetching}
          >{isFetching ? 'Refreshing…' : 'Refresh'}</button>
          <Link href="/tools/supply-chain-status" className="btn btn-secondary">Check a domain</Link>
        </div>
      </div>

      {isLoading && <div className="text-sm text-gray-600">Loading supply chain summary…</div>}
      {error && <div className="text-sm text-red-700">We couldn&apos;t load the summary. Retry.</div>}

      {!isLoading && apps.length === 0 && (
        <div className="card text-sm text-gray-700">No supply chain data for this publisher yet.</div>
      )}

      <div className="grid grid-cols-1 gap-4">
        {apps.map((app) => (
          <AppCard key={app.appId} app={app} />
        ))}
      </div>
    </div>
  )
}
