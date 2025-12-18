'use client'

import { useSession } from 'next-auth/react'
import { useEffect } from 'react'
import { useRouter } from 'next/navigation'

/**
 * Lightweight client gate to avoid showing a blank page if server session check ever fails open.
 * Redirects unauthenticated users to /login.
 */
export default function DashboardGate({ children }: { children: React.ReactNode }) {
  const { status } = useSession()
  const router = useRouter()
  useEffect(() => {
    if (status === 'unauthenticated') {
      router.replace('/login')
    }
  }, [status, router])
  return <>{children}</>
}
