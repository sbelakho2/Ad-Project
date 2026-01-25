'use client'

import { useEffect, useMemo, useState } from 'react'

const DEFAULT_CONTROL_PLANE_ORIGIN = 'https://admin.apexmediation.ee'

export default function AdminLayout({ children }: { children: React.ReactNode }) {
  const origin = (process.env.NEXT_PUBLIC_CONTROL_PLANE_URL || DEFAULT_CONTROL_PLANE_ORIGIN).trim()
  const [redirecting, setRedirecting] = useState(false)

  const target = useMemo(() => {
    if (typeof window === 'undefined') return `${origin}/admin/`
    const path = `${window.location.pathname}${window.location.search}${window.location.hash}`
    return new URL(path, origin).toString()
  }, [origin])

  useEffect(() => {
    if (typeof window === 'undefined') return
    const resolvedOrigin = origin.startsWith('http') ? origin : window.location.origin
    const shouldRedirect = new URL(resolvedOrigin).origin !== window.location.origin
    if (!shouldRedirect) return

    setRedirecting(true)
    window.location.replace(target)
  }, [origin, target])

  if (redirecting) {
    return (
      <div className="control-plane min-h-screen relative flex items-center justify-center px-6">
        <div className="control-plane__bezel" aria-hidden="true" />
        {['top-2 left-2', 'top-2 right-2', 'bottom-10 left-2', 'bottom-10 right-2'].map((pos) => (
          <div
            key={pos}
            className={`control-plane__screw hidden md:flex absolute ${pos} items-center justify-center`}
            aria-hidden="true"
          >
            <svg width="12" height="12" viewBox="0 0 12 12">
              <circle cx="6" cy="6" r="5" fill="none" stroke="currentColor" strokeWidth="1" />
              <path d="M3 6L9 6M6 3L6 9" stroke="currentColor" strokeWidth="1" />
            </svg>
          </div>
        ))}

        <div className="control-plane__module max-w-xl w-full border">
          <div className="control-plane__module-header border-b">
            <div className="control-plane__label">Operator Access Node</div>
            <h1 className="control-plane__title">Control Plane Redirect</h1>
          </div>

          <div className="p-4 space-y-4">
            <p className="text-sm">
              This console does not host the operator surface. Continue to the dedicated control plane station.
            </p>

            <div className="grid gap-3">
              <a href={target} className="btn btn-primary">
                Proceed to Control Plane
              </a>
              <div className="control-plane__label text-xs">Console: https://console.apexmediation.ee</div>
            </div>
          </div>
        </div>

        <div className="control-plane__statusbar hidden md:flex">
          <div className="flex gap-6">
            <span>STATION: SENSEI-ALPHA-01</span>
            <span>OS_VER: 3.0.1-RAMS</span>
          </div>
          <div className="flex gap-6">
            <span>INTEGRITY: OPERATIONAL</span>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="control-plane min-h-screen">
      <div className="control-plane__bezel" aria-hidden="true" />
      {['top-2 left-2', 'top-2 right-2', 'bottom-10 left-2', 'bottom-10 right-2'].map((pos) => (
        <div
          key={pos}
          className={`control-plane__screw hidden md:flex absolute ${pos} items-center justify-center`}
          aria-hidden="true"
        >
          <svg width="12" height="12" viewBox="0 0 12 12">
            <circle cx="6" cy="6" r="5" fill="none" stroke="currentColor" strokeWidth="1" />
            <path d="M3 6L9 6M6 3L6 9" stroke="currentColor" strokeWidth="1" />
          </svg>
        </div>
      ))}

      <div className="control-plane__content">
        {children}
      </div>

      <div className="control-plane__statusbar hidden md:flex">
        <div className="flex gap-6">
          <span>STATION: SENSEI-ALPHA-01</span>
          <span>OS_VER: 3.0.1-RAMS</span>
        </div>
        <div className="flex gap-6">
          <span>INTEGRITY: OPERATIONAL</span>
        </div>
      </div>
    </div>
  )
}
