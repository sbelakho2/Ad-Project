'use client'

import { useState } from 'react'
import Link from 'next/link'
import { useRouter } from 'next/navigation'
import { signIn } from 'next-auth/react'
import { ShieldCheck, CheckCircle2 } from 'lucide-react'
import { apiClient, handleApiError } from '@/lib/api-client'

const MIN_PASSWORD_LENGTH = 8

export default function SignupPage() {
  const router = useRouter()
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [companyName, setCompanyName] = useState('')
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState('')

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault()
    setError('')
    setSuccess('')

    if (!email || !password || !companyName) {
      setError('Fill in email, password, and company name to continue.')
      return
    }
    if (password.length < MIN_PASSWORD_LENGTH) {
      setError(`Password must be at least ${MIN_PASSWORD_LENGTH} characters.`)
      return
    }

    setIsSubmitting(true)

    try {
      await apiClient.post('/auth/register', {
        email,
        password,
        companyName,
      })

      setSuccess('Account created. Completing sign-in...')
      await signIn('credentials', { email, password, redirect: false })
      router.push('/dashboard')
      router.refresh()
    } catch (err) {
      setError(handleApiError(err))
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <div className="min-h-screen bg-gradient-to-br from-primary-50 via-white to-accent-50 flex items-center justify-center p-4">
      <div className="w-full max-w-3xl bg-white rounded-2xl shadow-xl border border-gray-100 p-8 grid gap-8 lg:grid-cols-[1.1fr_0.9fr]">
        <div className="space-y-6">
          <div className="flex items-center gap-3">
            <div className="h-12 w-12 rounded-2xl bg-primary-600 text-white flex items-center justify-center">
              <ShieldCheck className="h-6 w-6" />
            </div>
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.35em] text-primary-600">BYO Control Plane</p>
              <h1 className="text-3xl font-bold text-gray-900">Create your ApexMediation account</h1>
              <p className="text-sm text-gray-600 mt-1">
                Starter stays 0% up to $10k per app per month and includes SDKs, Migration Studio, and basic dashboards. Transparency auctions, VRA, fraud dashboards, and experiments unlock on Growth (2.5%). Scale is 2% down to 1.25% with annual prepay; Enterprise is custom. You own customer contracts and payouts—we only bill the platform fee.
              </p>
            </div>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4" autoComplete="on" noValidate>
            {error && (
              <div className="p-4 bg-danger-50 border border-danger-200 text-danger-700 rounded-lg text-sm" role="alert" aria-live="assertive">
                {error}
              </div>
            )}
            {success && (
              <div className="p-4 bg-success-50 border border-success-200 text-success-800 rounded-lg text-sm" role="status" aria-live="polite">
                {success}
              </div>
            )}

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
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
                  placeholder="you@example.com"
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
                  placeholder="Min 8 characters"
                  required
                  autoComplete="new-password"
                  minLength={MIN_PASSWORD_LENGTH}
                />
              </div>
            </div>

            <div>
              <label htmlFor="company" className="block text-sm font-medium text-gray-700 mb-2">
                Company name
              </label>
              <input
                id="company"
                type="text"
                value={companyName}
                onChange={(e) => setCompanyName(e.target.value)}
                className="input w-full"
                placeholder="Publisher or studio name"
                required
                autoComplete="organization"
              />
            </div>

            <button
              type="submit"
              disabled={isSubmitting}
              className="btn-primary w-full disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isSubmitting ? 'Creating account...' : 'Create account'}
            </button>

            <p className="text-xs text-gray-500 text-center">
              We never collect your card or bank details. When you upgrade off Starter, you’ll add a payment method in Stripe.
            </p>
          </form>
        </div>

        <div className="rounded-2xl border border-gray-100 bg-gray-50 p-6 space-y-5">
          <div className="flex items-center gap-3">
            <CheckCircle2 className="h-5 w-5 text-success-600" />
            <p className="text-sm font-semibold text-gray-900">Pricing that matches BYO</p>
          </div>
          <ul className="space-y-3 text-sm text-gray-700 list-disc list-inside">
            <li><span className="font-semibold">Starter:</span> 0% up to $10k per app per month. Basic dashboards; transparency/VRA/fraud unlock on upgrade.</li>
            <li><span className="font-semibold">Growth:</span> 2.5% platform fee with transparency auctions, VRA exports, fraud dashboards, and experiments.</li>
            <li><span className="font-semibold">Scale:</span> 2% with volume, down to 1.25% when you prepay annually.</li>
            <li><span className="font-semibold">Enterprise:</span> Custom rate card, SLAs, and layered approvals.</li>
          </ul>
          <div className="rounded-xl bg-white border border-gray-200 p-4 text-sm text-gray-700 space-y-2">
            <p className="font-semibold text-gray-900">What stays yours</p>
            <p>Bring your own demand, contracts, and payments. ApexMediation only observes data and bills the platform fee.</p>
            <p>Payment methods are managed in Stripe (cards, ACH, SEPA) when you upgrade off Starter.</p>
            <p className="text-xs text-gray-500">Already working with us? <a className="text-primary-600 underline" href="mailto:sales@apexmediation.ee">Contact sales</a> to map your contract to this account.</p>
          </div>
          <p className="text-xs text-gray-500">By continuing you agree to the ApexMediation <Link className="underline" href="/terms">Terms</Link> and <Link className="underline" href="/privacy">Privacy</Link>.</p>
          <p className="text-sm text-gray-700">Already have an account? <Link href="/login" className="text-primary-600 font-semibold">Sign in</Link></p>
        </div>
      </div>
    </div>
  )
}
