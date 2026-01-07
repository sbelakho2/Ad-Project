'use client'

import { useState } from 'react'
import Link from 'next/link'
import Image from 'next/image'
import { Mail, ArrowLeft } from 'lucide-react'
import { apiClient, handleApiError } from '@/lib/api-client'

export default function ForgotPasswordPage() {
  const [email, setEmail] = useState('')
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState(false)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!email.trim()) {
      setError('Please enter your email address.')
      return
    }

    setIsSubmitting(true)
    setError('')

    try {
      await apiClient.post('/auth/forgot-password', { email })
      setSuccess(true)
    } catch (err) {
      // Always show success to prevent email enumeration
      // The API should also behave this way
      setSuccess(true)
    } finally {
      setIsSubmitting(false)
    }
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
          <h1 className="text-3xl font-bold text-gray-900 mb-2">Reset Password</h1>
          <p className="text-gray-600">
            {success 
              ? "Check your email for a reset link"
              : "Enter your email to receive a password reset link"
            }
          </p>
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
                <label htmlFor="email" className="block text-sm font-medium text-gray-700 mb-2">
                  Email Address
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

              <button
                type="submit"
                disabled={isSubmitting}
                className="btn-primary w-full disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {isSubmitting ? 'Sending...' : 'Send Reset Link'}
              </button>
            </form>
          ) : (
            <div className="text-center py-6">
              <div className="mx-auto mb-4 h-16 w-16 rounded-full bg-success-100 flex items-center justify-center">
                <Mail className="h-8 w-8 text-success-600" />
              </div>
              <h2 className="text-xl font-semibold text-gray-900 mb-2">Check Your Email</h2>
              <p className="text-gray-600 mb-4">
                If an account exists for <strong>{email}</strong>, we&apos;ve sent a password reset link.
              </p>
              <p className="text-sm text-gray-500 mb-6">
                The link will expire in 1 hour. Check your spam folder if you don&apos;t see it.
              </p>
              <button
                onClick={() => {
                  setSuccess(false)
                  setEmail('')
                }}
                className="text-primary-600 hover:text-primary-700 font-medium text-sm"
              >
                Try a different email
              </button>
            </div>
          )}

          <div className="mt-6 pt-6 border-t border-gray-100 text-center">
            <Link 
              href="/login" 
              className="inline-flex items-center gap-2 text-sm text-gray-600 hover:text-gray-900"
            >
              <ArrowLeft className="h-4 w-4" />
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
