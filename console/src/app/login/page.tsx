'use client'

import { useEffect, useState } from 'react'
import Link from 'next/link'
import { signIn } from 'next-auth/react'
import { useRouter } from 'next/navigation'
import Image from 'next/image'

const MAX_ATTEMPTS_BEFORE_COOLDOWN = 5
const COOLDOWN_MS = 30_000
const DEMO_LOGIN_ENABLED = process.env.NEXT_PUBLIC_DEMO_LOGIN === 'true'
const IS_PRODUCTION_BUILD = process.env.NODE_ENV === 'production'
const CAN_SHOW_DEMO_MODE = DEMO_LOGIN_ENABLED && !IS_PRODUCTION_BUILD

export default function LoginPage() {
  const router = useRouter()
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
      const result = await signIn('credentials', {
        email,
        password,
        csrfToken,
        redirect: false,
      })

      if (result?.error) {
        try {
          const parsed = JSON.parse(result.error)
          if (parsed?.code === 'TWOFA_REQUIRED') {
            setTwofaToken(parsed.tempToken || null)
            setTwofaCode('')
            setTwofaError('')
            setError('Two-factor authentication required. Enter your 6-digit code or a backup code to continue.')
            return
          }
        } catch (parseError) {
          void parseError
        }
        setError('Invalid credentials. Please try again.')
        setFailedAttempts((prev) => {
          const next = prev + 1
          if (next >= MAX_ATTEMPTS_BEFORE_COOLDOWN) {
            setCooldownUntil(Date.now() + COOLDOWN_MS)
            return 0
          }
          return next
        })
      } else if (result?.ok) {
        setFailedAttempts(0)
        setHumanCheck(false)
        router.push('/dashboard')
        router.refresh()
      } else {
        setError('An unexpected error occurred.')
      }
    } catch (err) {
      setError('An error occurred. Please try again.')
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
      const result = await signIn('credentials', {
        email,
        tempToken: twofaToken,
        twofaCode: twofaCode.trim(),
        csrfToken,
        redirect: false,
      })

      if (result?.error) {
        setTwofaError('Invalid or expired code. Try again or use a backup code.')
      } else if (result?.ok) {
        router.push('/dashboard')
        router.refresh()
      } else {
        setTwofaError('Unable to complete verification. Please try again.')
      }
    } catch (err) {
      setTwofaError('Verification failed. Please try again.')
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
          <div className="inline-flex items-center justify-center h-16 w-16 rounded-2xl overflow-hidden mb-4">
            <Image
              src="/logo.jpg"
              alt="ApexMediation"
              width={64}
              height={64}
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
          <p>© 2025 ApexMediation. All rights reserved.</p>
          <p className="mt-1" aria-live="polite">
            {cooldownRemaining > 0
              ? `Login locked for ${cooldownRemaining}s due to repeated failures.`
              : `${Math.max(0, MAX_ATTEMPTS_BEFORE_COOLDOWN - failedAttempts)} attempts remaining before a short lockout.`}
          </p>
        </div>
      </div>
    </div>
  )
}
