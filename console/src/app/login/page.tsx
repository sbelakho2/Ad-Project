'use client'

import { useEffect, useState } from 'react'
import Link from 'next/link'
import { signIn, useSession } from 'next-auth/react'
import { useRouter } from 'next/navigation'
import Image from 'next/image'
import axios from 'axios'
import { apiClient, handleApiError } from '@/lib/api-client'

const MAX_ATTEMPTS_BEFORE_COOLDOWN = 5
const COOLDOWN_MS = 30_000
const DEMO_LOGIN_ENABLED = process.env.NEXT_PUBLIC_DEMO_LOGIN === 'true'
const IS_PRODUCTION_BUILD = process.env.NODE_ENV === 'production'
const CAN_SHOW_DEMO_MODE = DEMO_LOGIN_ENABLED && !IS_PRODUCTION_BUILD

// Check if we're on the admin subdomain
function isAdminSubdomain(): boolean {
  if (typeof window === 'undefined') return false
  return window.location.hostname.startsWith('admin.')
}

function getControlPlaneOrigin(): string {
  return (process.env.NEXT_PUBLIC_CONTROL_PLANE_URL || 'https://admin.apexmediation.ee').trim()
}

export default function LoginPage() {
  const router = useRouter()
  const { status } = useSession()
  // If already authenticated, go straight to dashboard
  if (typeof window !== 'undefined' && status === 'authenticated') {
    router.replace('/dashboard')
  }
  
  // Redirect admin subdomain users to operator login
  useEffect(() => {
    if (isAdminSubdomain()) {
      const origin = getControlPlaneOrigin()
      window.location.replace(new URL('/admin/login', origin).toString())
    }
  }, [router])
  const [email, setEmail] = useState(() => (CAN_SHOW_DEMO_MODE ? 'demo@apexmediation.ee' : ''))
  const [password, setPassword] = useState(() => (CAN_SHOW_DEMO_MODE ? 'demo' : ''))
  const [isLoading, setIsLoading] = useState(false)
  const [error, setError] = useState('')
  const [csrfToken, setCsrfToken] = useState<string | null>(null)
  const [csrfError, setCsrfError] = useState('')
  const [failedAttempts, setFailedAttempts] = useState(0)
  const [cooldownUntil, setCooldownUntil] = useState<number | null>(null)
  const [humanCheck, setHumanCheck] = useState(false)
  const [cooldownRemaining, setCooldownRemaining] = useState(0)
  const [twofaToken, setTwofaToken] = useState<string | null>(null)
  const [twofaCode, setTwofaCode] = useState('')
  const [twofaError, setTwofaError] = useState('')

  useEffect(() => {
    if (!cooldownUntil) return
    const updateRemaining = () => {
      const seconds = Math.max(0, Math.ceil((cooldownUntil - Date.now()) / 1000))
      setCooldownRemaining(seconds)
      if (seconds <= 0) {
        setCooldownUntil(null)
      }
    }
    updateRemaining()
    const interval = setInterval(updateRemaining, 1000)
    return () => clearInterval(interval)
  }, [cooldownUntil])

  useEffect(() => {
    if (!cooldownUntil) {
      setCooldownRemaining(0)
    }
  }, [cooldownUntil])

  useEffect(() => {
    const controller = new AbortController()

    const fetchCsrf = async () => {
      try {
        const response = await fetch('/api/auth/csrf', { signal: controller.signal })
        if (!response.ok) throw new Error('Failed to fetch CSRF token')
        const data = await response.json()
        setCsrfToken(data?.csrfToken || data?.token || null)
      } catch (err) {
        if ((err as Error).name === 'AbortError') return
        setCsrfError('Unable to secure login form. Refresh or contact support if this persists.')
      }
    }

    fetchCsrf()

    return () => controller.abort()
  }, [])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (twofaToken) {
      setError('Complete two-factor verification below before re-submitting credentials.')
      return
    }
    if (cooldownRemaining > 0) {
      setError('Login temporarily locked after repeated attempts. Please wait and try again.')
      return
    }
    if (!csrfToken) {
      setCsrfError('CSRF token missing. Please refresh before submitting.')
      return
    }
    if (!humanCheck && failedAttempts >= 2) {
      setError('Please confirm you are not a bot before continuing.')
      return
    }

    setIsLoading(true)
    setError('')

    try {
      const response = await apiClient.post('/auth/login', { email, password })
      const payload = response.data?.data ?? response.data

      if (payload?.twofaRequired) {
        setTwofaToken(payload?.tempToken || null)
        setTwofaCode('')
        setTwofaError('')
        setError(
          'Two-factor authentication required. Enter your 6-digit code or a backup code to continue.'
        )
        return
      }

      const token = payload?.token
      if (!token) {
        setError('An unexpected error occurred.')
        return
      }

      const result = await signIn('credentials', {
        accessToken: token,
        csrfToken,
        redirect: false,
      })

      if (result?.error) {
        setError('Unable to complete sign-in. Please try again.')
        return
      }

      setFailedAttempts(0)
      setHumanCheck(false)
      router.replace('/dashboard')
      router.refresh()
    } catch (err) {
      if (axios.isAxiosError(err)) {
        const status = err.response?.status
        if (status === 401) {
          setError('Invalid credentials. Please try again.')
          setFailedAttempts((prev) => {
            const next = prev + 1
            if (next >= MAX_ATTEMPTS_BEFORE_COOLDOWN) {
              setCooldownUntil(Date.now() + COOLDOWN_MS)
              return 0
            }
            return next
          })
          return
        }
        if (status === 503) {
          setError('Authentication temporarily unavailable. Please try again in a moment.')
          return
        }
      }

      setError(handleApiError(err))
    } finally {
      setIsLoading(false)
    }
  }

  const handleTwofaSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!twofaToken) {
      setError('Please submit your credentials first.')
      return
    }
    if (!twofaCode.trim()) {
      setTwofaError('Enter the 6-digit code from your authenticator or a backup code.')
      return
    }

    setIsLoading(true)
    setTwofaError('')
    setError('')

    try {
      const response = await apiClient.post('/auth/login/2fa', {
        tempToken: twofaToken,
        code: twofaCode.trim(),
      })
      const payload = response.data?.data ?? response.data
      const token = payload?.token
      if (!token) {
        setTwofaError('Unable to complete verification. Please try again.')
        return
      }

      const result = await signIn('credentials', {
        accessToken: token,
        csrfToken,
        redirect: false,
      })

      if (result?.error) {
        setTwofaError('Unable to complete verification. Please try again.')
        return
      }

      router.replace('/dashboard')
      router.refresh()
    } catch (err) {
      if (axios.isAxiosError(err) && err.response?.status === 401) {
        setTwofaError('Invalid or expired code. Try again or use a backup code.')
        return
      }
      setTwofaError(handleApiError(err))
    } finally {
      setIsLoading(false)
    }
  }

  const resetTwofa = () => {
    setTwofaToken(null)
    setTwofaCode('')
    setTwofaError('')
    setError('')
  }

  return (
    <div className="min-h-screen bg-gradient-to-br from-primary-50 via-white to-accent-50 flex items-center justify-center p-4">
      <div className="w-full max-w-md">
        {/* Logo/Header */}
        <div className="text-center mb-8">
          <div className="inline-flex items-center justify-center h-24 w-24 rounded-2xl overflow-hidden mb-4 shadow-lg">
            <Image
              src="/logo.jpg"
              alt="ApexMediation"
              width={96}
              height={96}
              className="h-full w-full object-cover"
              priority
            />
          </div>
          <h1 className="text-3xl font-bold text-gray-900 mb-2">ApexMediation</h1>
          <p className="text-gray-600">Sign in to your publisher console</p>
        </div>

        {/* Login Card */}
        <div className="bg-white rounded-2xl shadow-xl border border-gray-100 p-8">
          {!twofaToken ? (
            <form onSubmit={handleSubmit} className="space-y-6" autoComplete="on" noValidate>
              {error && (
                <div className="p-4 bg-danger-50 border border-danger-200 text-danger-700 rounded-lg text-sm" role="alert" aria-live="assertive">
                  {error}
                </div>
              )}

              {csrfError && (
                <div className="p-4 bg-warning-50 border border-warning-200 text-warning-800 rounded-lg text-sm" role="alert">
                  {csrfError}
                </div>
              )}

              {/* Demo Notice */}
              {CAN_SHOW_DEMO_MODE && (
                <div className="p-4 bg-blue-50 border border-blue-200 text-blue-700 rounded-lg text-sm">
                  <p className="font-medium mb-1">Demo Mode</p>
                  <p>
                    Use <strong>demo@apexmediation.ee</strong> with any password to login in sandbox
                    environments.
                  </p>
                </div>
              )}

              {!CAN_SHOW_DEMO_MODE && DEMO_LOGIN_ENABLED && IS_PRODUCTION_BUILD && (
                <div className="p-4 bg-warning-50 border border-warning-200 text-warning-800 rounded-lg text-sm">
                  <p className="font-medium mb-1">Demo login disabled</p>
                  <p>
                    Demo credentials are blocked in production builds even if the flag is set. Contact an
                    administrator if you need sandbox access.
                  </p>
                </div>
              )}

              <div>
                <label htmlFor="email" className="block text-sm font-medium text-gray-700 mb-2">
                  Email
                </label>
                <input
                  id="email"
                  type="email"
                  name="email"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  className="input w-full"
                  placeholder="you@example.com"
                  required
                  autoComplete="email"
                  inputMode="email"
                />
              </div>

              <div>
                <label htmlFor="password" className="block text-sm font-medium text-gray-700 mb-2">
                  Password
                </label>
                <input
                  id="password"
                  type="password"
                  name="current-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="input w-full"
                  placeholder="Enter your password"
                  required
                  autoComplete="current-password"
                  spellCheck={false}
                />
              </div>

              {failedAttempts >= 2 && (
                <div className="flex items-center gap-2 rounded-lg border border-gray-200 px-3 py-2">
                  <input
                    id="human-check"
                    type="checkbox"
                    checked={humanCheck}
                    onChange={(event) => setHumanCheck(event.target.checked)}
                    className="h-4 w-4"
                    required
                  />
                  <label htmlFor="human-check" className="text-sm text-gray-700">
                    I&apos;m not a bot
                  </label>
                </div>
              )}

              {cooldownRemaining > 0 && (
                <p className="text-sm text-warning-700 bg-warning-50 border border-warning-200 rounded-lg p-3">
                  Too many attempts. Please wait {cooldownRemaining} seconds before trying again.
                </p>
              )}

              <button
                type="submit"
                disabled={
                  isLoading || cooldownRemaining > 0 || !!csrfError || !csrfToken || (failedAttempts >= 2 && !humanCheck)
                }
                className="btn-primary w-full disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {isLoading ? 'Signing in...' : 'Sign In'}
              </button>
            </form>
          ) : (
            <form onSubmit={handleTwofaSubmit} className="space-y-6" autoComplete="one-time-code" noValidate>
              <div className="p-4 bg-primary-50 border border-primary-200 text-primary-900 rounded-lg text-sm" role="alert" aria-live="assertive">
                <p className="font-semibold">Two-factor verification required</p>
                <p className="mt-1 text-primary-800">
                  Enter the 6-digit code from your authenticator app or a backup code to finish signing in.
                </p>
              </div>

              {twofaError && (
                <div className="p-4 bg-danger-50 border border-danger-200 text-danger-700 rounded-lg text-sm" role="alert" aria-live="assertive">
                  {twofaError}
                </div>
              )}

              <div>
                <label htmlFor="twofa" className="block text-sm font-medium text-gray-700 mb-2">
                  2FA Code or Backup Code
                </label>
                <input
                  id="twofa"
                  type="text"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  value={twofaCode}
                  onChange={(e) => setTwofaCode(e.target.value)}
                  className="input w-full tracking-[0.3em]"
                  placeholder="123456 or backup code"
                  required
                />
                <p className="mt-2 text-xs text-gray-500">Lost access? Use one of your backup codes to continue.</p>
              </div>

              <div className="flex items-center gap-3">
                <button
                  type="submit"
                  disabled={isLoading || !!csrfError || !csrfToken}
                  className="btn-primary flex-1 disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  {isLoading ? 'Verifying...' : 'Verify and continue'}
                </button>
                <button
                  type="button"
                  onClick={resetTwofa}
                  className="flex-1 px-4 py-2 text-sm font-medium text-gray-700 border border-gray-300 rounded-lg hover:bg-gray-50"
                >
                  Back to credentials
                </button>
              </div>
            </form>
          )}

          <div className="mt-6 text-center text-sm text-gray-600">
            <p>
              Don&apos;t have an account?{' '}
              <Link href="/signup" className="text-primary-600 hover:text-primary-700 font-medium">
                Create one
              </Link>
              {' '}or{' '}
              <a href="mailto:sales@apexmediation.ee" className="text-primary-600 hover:text-primary-700 font-medium">
                contact sales
              </a>
            </p>
          </div>
        </div>

        {/* Footer */}
        <div className="text-center mt-8 text-sm text-gray-600">
          <p>© 2025 Bel Consulting OÜ. ApexMediation is a brand of Bel Consulting OÜ.</p>
          {(cooldownRemaining > 0 || failedAttempts > 0) && (
            <p className="mt-1 text-amber-600" aria-live="polite">
              {cooldownRemaining > 0
                ? `Login locked for ${cooldownRemaining}s due to repeated failures.`
                : `${Math.max(0, MAX_ATTEMPTS_BEFORE_COOLDOWN - failedAttempts)} attempts remaining before lockout.`}
            </p>
          )}
        </div>
      </div>
    </div>
  )
}
