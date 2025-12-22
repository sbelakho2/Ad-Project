'use client'

import { useEffect, useMemo, useRef, useState } from 'react'
import { signOut, useSession } from 'next-auth/react'

const IDLE_MS = 10 * 60 * 1000
const GRACE_MS = 60 * 1000

type Props = {
  enabled?: boolean
}

function nowMs() {
  return Date.now()
}

export function IdleLogoutGuard({ enabled = true }: Props) {
  const { status } = useSession()

  const active = enabled && status === 'authenticated'

  const timers = useRef<{ idle?: number; grace?: number; tick?: number }>({})
  const lastActivityAt = useRef<number>(nowMs())
  const [showPrompt, setShowPrompt] = useState(false)
  const [secondsRemaining, setSecondsRemaining] = useState<number>(Math.ceil(GRACE_MS / 1000))
  const [isSigningOut, setIsSigningOut] = useState(false)

  const activityEvents = useMemo(
    () => ['mousemove', 'mousedown', 'keydown', 'scroll', 'touchstart', 'pointerdown'] as const,
    []
  )

  const clearTimers = () => {
    if (timers.current.idle) window.clearTimeout(timers.current.idle)
    if (timers.current.grace) window.clearTimeout(timers.current.grace)
    if (timers.current.tick) window.clearInterval(timers.current.tick)
    timers.current = {}
  }

  const scheduleIdleCheck = () => {
    clearTimers()
    timers.current.idle = window.setTimeout(() => {
      setShowPrompt(true)
      setSecondsRemaining(Math.ceil(GRACE_MS / 1000))

      const graceEndsAt = nowMs() + GRACE_MS
      timers.current.tick = window.setInterval(() => {
        const remaining = Math.max(0, Math.ceil((graceEndsAt - nowMs()) / 1000))
        setSecondsRemaining(remaining)
      }, 250)

      timers.current.grace = window.setTimeout(async () => {
        await doSignOut()
      }, GRACE_MS)
    }, IDLE_MS)
  }

  const noteActivity = () => {
    // Requirement: user must explicitly click "Stay connected" once the prompt appears.
    // So we stop resetting timers while the prompt is visible.
    if (showPrompt) return

    lastActivityAt.current = nowMs()
    scheduleIdleCheck()
  }

  const doSignOut = async () => {
    if (isSigningOut) return
    setIsSigningOut(true)
    try {
      // NextAuth handles clearing its cookies + redirect.
      await signOut({ callbackUrl: '/login' })
    } finally {
      setIsSigningOut(false)
    }
  }

  useEffect(() => {
    if (!active || typeof window === 'undefined') {
      clearTimers()
      setShowPrompt(false)
      return
    }

    lastActivityAt.current = nowMs()
    scheduleIdleCheck()

    const handler = () => noteActivity()
    for (const eventName of activityEvents) {
      window.addEventListener(eventName, handler, { passive: true })
    }

    return () => {
      for (const eventName of activityEvents) {
        window.removeEventListener(eventName, handler)
      }
      clearTimers()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active])

  // If prompt is visible and the user clicks "Stay connected", reset the idle timer.
  const handleStayConnected = () => {
    if (!active) return
    setShowPrompt(false)
    lastActivityAt.current = nowMs()
    scheduleIdleCheck()
  }

  if (!active || !showPrompt) return null

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
      <div
        className="bg-white rounded-xl shadow-xl max-w-md w-full p-6"
        role="dialog"
        aria-modal="true"
        aria-labelledby="idle-timeout-title"
      >
        <h3 id="idle-timeout-title" className="text-lg font-semibold text-gray-900 mb-2">
          You&apos;ll be logged out soon
        </h3>
        <p className="text-sm text-gray-600 mb-5">
          You&apos;ve been inactive for 10 minutes. You&apos;ll be logged out in{' '}
          <span className="font-medium text-gray-900">{secondsRemaining}s</span> unless you click
          <span className="font-medium text-gray-900"> Stay connected</span>.
        </p>
        <div className="flex gap-3">
          <button
            type="button"
            onClick={doSignOut}
            disabled={isSigningOut}
            className="flex-1 px-4 py-2 border rounded-lg hover:bg-gray-50 disabled:opacity-50"
          >
            Log out
          </button>
          <button
            type="button"
            onClick={handleStayConnected}
            disabled={isSigningOut}
            className="flex-1 px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700 disabled:opacity-50"
          >
            Stay connected
          </button>
        </div>
      </div>
    </div>
  )
}
