'use client'

import { useEffect, useMemo } from 'react'

const DEFAULT_CONTROL_PLANE_ORIGIN = 'https://admin.apexmediation.ee'

export default function AdminLayout({ children }: { children: React.ReactNode }) {
  const origin = (process.env.NEXT_PUBLIC_CONTROL_PLANE_URL || DEFAULT_CONTROL_PLANE_ORIGIN).trim()

  const target = useMemo(() => {
    if (typeof window === 'undefined') return `${origin}/admin/`
    const path = `${window.location.pathname}${window.location.search}${window.location.hash}`
    return new URL(path, origin).toString()
  }, [origin])

  useEffect(() => {
    if (typeof window === 'undefined') return
    // Hard separation: the operator control plane is not served by the customer console.
    window.location.replace(target)
  }, [target])

  // Render a small fallback in case redirects are blocked.
  return (
    <div className="min-h-screen bg-gray-900 flex items-center justify-center px-6">
      <div className="max-w-md w-full rounded-xl border border-gray-700 bg-gray-800 p-6">
        <h1 className="text-xl font-semibold text-white">Operator Control Plane</h1>
        <p className="mt-2 text-sm text-gray-300">
          This route is no longer served by the customer console. Continue on the dedicated operator hostname.
        </p>
        <a
          href={target}
          className="mt-4 inline-flex items-center justify-center w-full rounded-lg bg-primary-600 px-4 py-2 text-sm font-medium text-white hover:bg-primary-700"
        >
          Go to Control Plane
        </a>
        <p className="mt-3 text-xs text-gray-500">
          If you expected the customer console, use https://console.apexmediation.ee
        </p>
      </div>
      {/* Keep children unused to avoid accidental operator UI rendering in console */}
      <div style={{ display: 'none' }}>{children}</div>
    </div>
  )
}
