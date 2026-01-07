'use client'

import { useState } from 'react'
import Link from 'next/link'
import { useRouter } from 'next/navigation'
import { signIn } from 'next-auth/react'
import { CheckCircle2, Mail } from 'lucide-react'
import { apiClient, handleApiError } from '@/lib/api-client'
import axios from 'axios'

const MIN_PASSWORD_LENGTH = 8

// Password complexity requirements (must match backend)
const PASSWORD_REQUIREMENTS = [
  { label: 'At least 8 characters', test: (p: string) => p.length >= 8 },
  { label: 'One uppercase letter (A-Z)', test: (p: string) => /[A-Z]/.test(p) },
  { label: 'One lowercase letter (a-z)', test: (p: string) => /[a-z]/.test(p) },
  { label: 'One number (0-9)', test: (p: string) => /\d/.test(p) },
  { label: 'One special character (!@#$%^&*)', test: (p: string) => /[!@#$%^&*(),.?":{}|<>]/.test(p) },
]

export default function SignupPage() {
  const router = useRouter()
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [companyName, setCompanyName] = useState('')
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState('')
  const [pendingVerification, setPendingVerification] = useState(false)

  const passwordValid = PASSWORD_REQUIREMENTS.every(req => req.test(password))

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault()
    setError('')
    setSuccess('')

    if (!email || !password || !companyName) {
      setError('Fill in email, password, and company name to continue.')
      return
    }
    if (!passwordValid) {
      setError('Password does not meet all requirements.')
      return
    }

    setIsSubmitting(true)

    try {
      const registerResp = await apiClient.post('/auth/register', {
        email,
        password,
        companyName,
      })
      
      const registerData = registerResp.data?.data ?? registerResp.data
      
      // Check if email verification is required
      if (registerData?.requiresVerification) {
        setPendingVerification(true)
        setSuccess('Account created! Please check your email to verify your account.')
        return
      }

      setSuccess('Account created. Completing sign-in...')

      const loginResp = await apiClient.post('/auth/login', { email, password })
      const payload = loginResp.data?.data ?? loginResp.data
      const token = payload?.token
      if (!token) {
        throw new Error('Unable to complete sign-in')
      }

      const result = await signIn('credentials', { accessToken: token, redirect: false })
      if (result?.error) {
        throw new Error('Unable to complete sign-in')
      }

      router.push('/dashboard')
      router.refresh()
    } catch (err) {
      if (axios.isAxiosError(err)) {
        const status = err.response?.status
        const message = err.response?.data?.message || err.response?.data?.error || ''
        
        if (status === 503) {
          setError('Authentication temporarily unavailable. Please try again in a moment.')
          return
        }
        if (status === 400 && message.toLowerCase().includes('password')) {
          setError(message)
          return
        }
      }
      setError(handleApiError(err))
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <div className="min-h-screen bg-gradient-to-br from-primary-50 via-white to-accent-50 flex items-center justify-center p-4 py-12">
      <div className="w-full max-w-4xl bg-white rounded-2xl shadow-xl border border-gray-100 p-8 md:p-10 grid gap-10 lg:grid-cols-2">
        {/* Left: Form */}
        <div className="space-y-8">
          <div className="space-y-4">
            <div className="inline-flex items-center gap-2 px-3 py-1 rounded-full bg-success-50 text-success-700 text-xs font-medium">
              <CheckCircle2 className="h-3.5 w-3.5" />
              Free to start · No credit card required
            </div>
            <h1 className="text-3xl font-bold text-gray-900">
              {pendingVerification ? 'Almost there!' : 'Maximize your ad revenue'}
            </h1>
            <p className="text-gray-600 leading-relaxed">
              {pendingVerification 
                ? 'Verify your email to activate your account.'
                : 'ApexMediation optimizes every ad request across your networks—so you earn more without extra work.'
              }
            </p>
          </div>

          {!pendingVerification && (
          <form onSubmit={handleSubmit} className="space-y-5" autoComplete="on" noValidate>
            {error && (
              <div className="p-4 bg-danger-50 border border-danger-200 text-danger-700 rounded-lg text-sm" role="alert" aria-live="assertive">
                {error}
              </div>
            )}
            {success && !pendingVerification && (
              <div className="p-4 bg-success-50 border border-success-200 text-success-800 rounded-lg text-sm" role="status" aria-live="polite">
                {success}
              </div>
            )}

            <div>
              <label htmlFor="company" className="block text-sm font-medium text-gray-700 mb-2">
                Studio or company name
              </label>
              <input
                id="company"
                type="text"
                value={companyName}
                onChange={(e) => setCompanyName(e.target.value)}
                className="input w-full"
                placeholder="Your company"
                required
                autoComplete="organization"
              />
            </div>

            <div>
              <label htmlFor="email" className="block text-sm font-medium text-gray-700 mb-2">
                Work email
              </label>
              <input
                id="email"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                className="input w-full"
                placeholder="you@company.com"
                required
                autoComplete="email"
              />
            </div>

            <div>
              <label htmlFor="password" className="block text-sm font-medium text-gray-700 mb-2">
                Password
              </label>
              <input
                id="password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="input w-full"
                placeholder="Create a strong password"
                required
                autoComplete="new-password"
                minLength={MIN_PASSWORD_LENGTH}
              />
              {/* Password Requirements */}
              {password.length > 0 && (
                <div className="mt-3 bg-gray-50 rounded-lg p-3 space-y-1.5">
                  {PASSWORD_REQUIREMENTS.map((req, index) => {
                    const met = req.test(password)
                    return (
                      <div key={index} className="flex items-center gap-2 text-xs">
                        {met ? (
                          <CheckCircle2 className="h-3.5 w-3.5 text-success-600 flex-shrink-0" />
                        ) : (
                          <div className="h-3.5 w-3.5 rounded-full border border-gray-300 flex-shrink-0" />
                        )}
                        <span className={met ? 'text-success-700' : 'text-gray-500'}>
                          {req.label}
                        </span>
                      </div>
                    )
                  })}
                </div>
              )}
            </div>

            <button
              type="submit"
              disabled={isSubmitting || !passwordValid}
              className="btn-primary w-full py-3 text-base disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isSubmitting ? 'Creating account...' : 'Get started free'}
            </button>

            <p className="text-xs text-gray-500 text-center">
              By signing up, you agree to our <Link className="underline" href="/terms">Terms</Link> and <Link className="underline" href="/privacy">Privacy Policy</Link>.
            </p>
          </form>
          )}

          {/* Email Verification Pending State */}
          {pendingVerification && (
            <div className="text-center py-8">
              <div className="mx-auto mb-4 h-16 w-16 rounded-full bg-success-100 flex items-center justify-center">
                <Mail className="h-8 w-8 text-success-600" />
              </div>
              <h2 className="text-xl font-semibold text-gray-900 mb-2">Check Your Email</h2>
              <p className="text-gray-600 mb-4">
                We&apos;ve sent a verification link to <strong>{email}</strong>.
              </p>
              <p className="text-sm text-gray-500 mb-6">
                Click the link in your email to verify your account and start using ApexMediation.
              </p>
              <Link href="/login" className="btn-primary inline-block">
                Go to Login
              </Link>
            </div>
          )}

          <p className="text-sm text-gray-600 text-center">
            Already have an account? <Link href="/login" className="text-primary-600 font-semibold hover:underline">Sign in</Link>
          </p>
        </div>

        {/* Right: Benefits */}
        <div className="space-y-6 lg:pl-8 lg:border-l border-gray-100">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wider text-gray-500 mb-4">What you get</p>
            <ul className="space-y-4">
              <li className="flex items-start gap-3">
                <div className="mt-0.5 h-5 w-5 rounded-full bg-primary-100 text-primary-600 flex items-center justify-center flex-shrink-0">
                  <CheckCircle2 className="h-3.5 w-3.5" />
                </div>
                <div>
                  <p className="font-medium text-gray-900">Unified SDK & dashboard</p>
                  <p className="text-sm text-gray-600">One integration for all major ad networks with real-time analytics.</p>
                </div>
              </li>
              <li className="flex items-start gap-3">
                <div className="mt-0.5 h-5 w-5 rounded-full bg-primary-100 text-primary-600 flex items-center justify-center flex-shrink-0">
                  <CheckCircle2 className="h-3.5 w-3.5" />
                </div>
                <div>
                  <p className="font-medium text-gray-900">Smart waterfall optimization</p>
                  <p className="text-sm text-gray-600">ML-powered bid ranking maximizes eCPM and fill rates automatically.</p>
                </div>
              </li>
              <li className="flex items-start gap-3">
                <div className="mt-0.5 h-5 w-5 rounded-full bg-primary-100 text-primary-600 flex items-center justify-center flex-shrink-0">
                  <CheckCircle2 className="h-3.5 w-3.5" />
                </div>
                <div>
                  <p className="font-medium text-gray-900">Migration Studio</p>
                  <p className="text-sm text-gray-600">Switch from other mediators with zero revenue disruption.</p>
                </div>
              </li>
            </ul>
          </div>

          <div className="rounded-xl bg-gray-50 border border-gray-100 p-5 space-y-3">
            <p className="text-xs font-semibold uppercase tracking-wider text-gray-500">Simple pricing</p>
            <div className="space-y-2 text-sm">
              <p className="text-gray-700"><span className="font-semibold text-gray-900">Starter:</span> Free up to $10k/month per app</p>
              <p className="text-gray-700"><span className="font-semibold text-gray-900">Growth:</span> 2.5% of mediated revenue</p>
              <p className="text-gray-700"><span className="font-semibold text-gray-900">Scale:</span> From 2% with volume discounts</p>
            </div>
            <p className="text-xs text-gray-500 pt-1">
              You keep 100% of your ad network revenue. Platform fee only.
            </p>
          </div>

          <p className="text-xs text-gray-500">
            Questions? <a className="text-primary-600 hover:underline" href="mailto:sales@apexmediation.ee">Talk to our team</a>
          </p>
        </div>
      </div>
    </div>
  )
}
