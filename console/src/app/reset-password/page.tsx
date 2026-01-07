'use client'

import { useState, Suspense } from 'react'
import Link from 'next/link'
import { useSearchParams, useRouter } from 'next/navigation'
import Image from 'next/image'
import { CheckCircle2, XCircle, Loader2, Eye, EyeOff, ShieldCheck } from 'lucide-react'
import { apiClient, handleApiError } from '@/lib/api-client'

// Password complexity requirements (must match backend)
const PASSWORD_REQUIREMENTS = [
  { label: 'At least 8 characters', test: (p: string) => p.length >= 8 },
  { label: 'One uppercase letter (A-Z)', test: (p: string) => /[A-Z]/.test(p) },
  { label: 'One lowercase letter (a-z)', test: (p: string) => /[a-z]/.test(p) },
  { label: 'One number (0-9)', test: (p: string) => /\d/.test(p) },
  { label: 'One special character (!@#$%^&*)', test: (p: string) => /[!@#$%^&*(),.?":{}|<>]/.test(p) },
]

function ResetPasswordContent() {
  const searchParams = useSearchParams()
  const router = useRouter()
  const token = searchParams.get('token')

  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState(false)

  const passwordValid = PASSWORD_REQUIREMENTS.every(req => req.test(password))
  const passwordsMatch = password === confirmPassword && password.length > 0

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    
    if (!token) {
      setError('Invalid or missing reset token. Please request a new password reset link.')
      return
    }

    if (!passwordValid) {
      setError('Password does not meet all requirements.')
      return
    }

    if (!passwordsMatch) {
      setError('Passwords do not match.')
      return
    }

    setIsSubmitting(true)
    setError('')

    try {
      await apiClient.post('/auth/reset-password', { token, newPassword: password })
      setSuccess(true)
      
      // Redirect to login after 3 seconds
      setTimeout(() => {
        router.push('/login?reset=true')
      }, 3000)
    } catch (err) {
      setError(handleApiError(err))
    } finally {
      setIsSubmitting(false)
    }
  }

  if (!token) {
    return (
      <div className="min-h-screen bg-gradient-to-br from-primary-50 via-white to-accent-50 flex items-center justify-center p-4">
        <div className="w-full max-w-md">
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
            <h1 className="text-3xl font-bold text-gray-900 mb-2">Reset Password</h1>
          </div>

          <div className="bg-white rounded-2xl shadow-xl border border-gray-100 p-8">
            <div className="text-center py-6">
              <div className="mx-auto mb-4 h-16 w-16 rounded-full bg-danger-100 flex items-center justify-center">
                <XCircle className="h-8 w-8 text-danger-600" />
              </div>
              <h2 className="text-xl font-semibold text-gray-900 mb-2">Invalid Reset Link</h2>
              <p className="text-gray-600 mb-6">
                This password reset link is invalid or has expired. Please request a new one.
              </p>
              <Link href="/forgot-password" className="btn-primary inline-block">
                Request New Link
              </Link>
            </div>
          </div>
        </div>
      </div>
    )
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
          <h1 className="text-3xl font-bold text-gray-900 mb-2">Set New Password</h1>
          <p className="text-gray-600">Choose a strong password for your account</p>
        </div>

        {/* Reset Card */}
        <div className="bg-white rounded-2xl shadow-xl border border-gray-100 p-8">
          {!success ? (
            <form onSubmit={handleSubmit} className="space-y-6" noValidate>
              {error && (
                <div className="p-4 bg-danger-50 border border-danger-200 text-danger-700 rounded-lg text-sm" role="alert">
                  {error}
                </div>
              )}

              <div>
                <label htmlFor="password" className="block text-sm font-medium text-gray-700 mb-2">
                  New Password
                </label>
                <div className="relative">
                  <input
                    id="password"
                    type={showPassword ? 'text' : 'password'}
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    className="input w-full pr-10"
                    placeholder="Enter your new password"
                    required
                    autoComplete="new-password"
                  />
                  <button
                    type="button"
                    onClick={() => setShowPassword(!showPassword)}
                    className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-500 hover:text-gray-700"
                    tabIndex={-1}
                  >
                    {showPassword ? <EyeOff className="h-5 w-5" /> : <Eye className="h-5 w-5" />}
                  </button>
                </div>
              </div>

              {/* Password Requirements */}
              {password.length > 0 && (
                <div className="bg-gray-50 rounded-lg p-4 space-y-2">
                  <p className="text-xs font-medium text-gray-700 uppercase tracking-wide mb-2">
                    Password Requirements
                  </p>
                  {PASSWORD_REQUIREMENTS.map((req, index) => {
                    const met = req.test(password)
                    return (
                      <div key={index} className="flex items-center gap-2 text-sm">
                        {met ? (
                          <CheckCircle2 className="h-4 w-4 text-success-600 flex-shrink-0" />
                        ) : (
                          <div className="h-4 w-4 rounded-full border-2 border-gray-300 flex-shrink-0" />
                        )}
                        <span className={met ? 'text-success-700' : 'text-gray-600'}>
                          {req.label}
                        </span>
                      </div>
                    )
                  })}
                </div>
              )}

              <div>
                <label htmlFor="confirm-password" className="block text-sm font-medium text-gray-700 mb-2">
                  Confirm New Password
                </label>
                <input
                  id="confirm-password"
                  type={showPassword ? 'text' : 'password'}
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  className="input w-full"
                  placeholder="Confirm your new password"
                  required
                  autoComplete="new-password"
                />
                {confirmPassword.length > 0 && !passwordsMatch && (
                  <p className="mt-2 text-sm text-danger-600">Passwords do not match</p>
                )}
                {passwordsMatch && (
                  <p className="mt-2 text-sm text-success-600 flex items-center gap-1">
                    <CheckCircle2 className="h-4 w-4" />
                    Passwords match
                  </p>
                )}
              </div>

              <button
                type="submit"
                disabled={isSubmitting || !passwordValid || !passwordsMatch}
                className="btn-primary w-full disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {isSubmitting ? 'Resetting...' : 'Reset Password'}
              </button>
            </form>
          ) : (
            <div className="text-center py-6">
              <div className="mx-auto mb-4 h-16 w-16 rounded-full bg-success-100 flex items-center justify-center">
                <ShieldCheck className="h-8 w-8 text-success-600" />
              </div>
              <h2 className="text-xl font-semibold text-gray-900 mb-2">Password Reset!</h2>
              <p className="text-gray-600 mb-4">
                Your password has been successfully reset.
              </p>
              <p className="text-sm text-gray-500">Redirecting to login...</p>
            </div>
          )}

          <div className="mt-6 pt-6 border-t border-gray-100 text-center">
            <Link 
              href="/login" 
              className="text-sm text-gray-600 hover:text-gray-900"
            >
              Back to Sign In
            </Link>
          </div>
        </div>

        {/* Footer */}
        <div className="text-center mt-8 text-sm text-gray-600">
          <p>© 2025 Bel Consulting OÜ. ApexMediation is a brand of Bel Consulting OÜ.</p>
        </div>
      </div>
    </div>
  )
}

export default function ResetPasswordPage() {
  return (
    <Suspense fallback={
      <div className="min-h-screen bg-gradient-to-br from-primary-50 via-white to-accent-50 flex items-center justify-center">
        <Loader2 className="h-8 w-8 text-primary-600 animate-spin" />
      </div>
    }>
      <ResetPasswordContent />
    </Suspense>
  )
}
