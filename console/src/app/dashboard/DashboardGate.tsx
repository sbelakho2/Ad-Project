'use client'

import { useEffect } from 'react'
import { useRouter } from 'next/navigation'
import { useSession } from '@/lib/useSession'

/**
 * Lightweight client gate to avoid showing a blank page if server session check ever fails open.
 * Redirects unauthenticated users to /login.
 */
export default function DashboardGate({ children }: { children: React.ReactNode }) {
  const { user, isLoading } = useSession()
  const router = useRouter()
  useEffect(() => {
    if (!isLoading && !user) {
      router.replace('/login')
    }
  }, [isLoading, router, user])
  return <>{children}</>
}
