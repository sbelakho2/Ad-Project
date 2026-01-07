'use client'

import { useEffect, useState, Suspense } from 'react'
import Link from 'next/link'
import { useSearchParams, useRouter } from 'next/navigation'
import Image from 'next/image'
import { CheckCircle2, XCircle, Loader2, Mail } from 'lucide-react'
import { apiClient, handleApiError } from '@/lib/api-client'

function VerifyEmailContent() {
  const searchParams = useSearchParams()
  const router = useRouter()
  const token = searchParams.get('token')

  const [status, setStatus] = useState<'loading' | 'success' | 'error' | 'no-token'>('loading')
  const [message, setMessage] = useState('')
  const [isResending, setIsResending] = useState(false)
  const [resendEmail, setResendEmail] = useState('')
  const [resendMessage, setResendMessage] = useState('')

  useEffect(() => {
    if (!token) {
      setStatus('no-token')
      return
    }

    const verifyEmail = async () => {
      try {
        await apiClient.post('/auth/verify-email', { token })
        setStatus('success')
        setMessage('Your email has been verified successfully!')
        
        // Redirect to login after 3 seconds
        setTimeout(() => {
          router.push('/login?verified=true')
        }, 3000)
      } catch (err) {
        setStatus('error')
        setMessage(handleApiError(err))
      }
    }

    verifyEmail()
  }, [token, router])

  const handleResendVerification = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!resendEmail.trim()) {
      setResendMessage('Please enter your email address.')
      return
    }

    setIsResending(true)
    setResendMessage('')

    try {
      await apiClient.post('/auth/resend-verification', { email: resendEmail })
      setResendMessage('If an account exists with this email, a new verification link has been sent.')
    } catch (err) {
      setResendMessage(handleApiError(err))
    } finally {
      setIsResending(false)
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
          <h1 className="text-3xl font-bold text-gray-900 mb-2">Email Verification</h1>
        </div>

        {/* Verification Card */}
        <div className="bg-white rounded-2xl shadow-xl border border-gray-100 p-8">
          {status === 'loading' && (
            <div className="text-center py-8">
              <Loader2 className="h-12 w-12 text-primary-600 animate-spin mx-auto mb-4" />
              <p className="text-gray-600">Verifying your email address...</p>
            </div>
          )}

          {status === 'success' && (
            <div className="text-center py-8">
              <div className="mx-auto mb-4 h-16 w-16 rounded-full bg-success-100 flex items-center justify-center">
                <CheckCircle2 className="h-8 w-8 text-success-600" />
              </div>
              <h2 className="text-xl font-semibold text-gray-900 mb-2">Email Verified!</h2>
              <p className="text-gray-600 mb-4">{message}</p>
              <p className="text-sm text-gray-500">Redirecting to login...</p>
            </div>
          )}

          {status === 'error' && (
            <div className="text-center py-8">
              <div className="mx-auto mb-4 h-16 w-16 rounded-full bg-danger-100 flex items-center justify-center">
                <XCircle className="h-8 w-8 text-danger-600" />
              </div>
              <h2 className="text-xl font-semibold text-gray-900 mb-2">Verification Failed</h2>
              <p className="text-danger-600 mb-6">{message}</p>
              
              <div className="border-t border-gray-200 pt-6">
                <p className="text-sm text-gray-600 mb-4">
                  Need a new verification link? Enter your email below:
                </p>
                <form onSubmit={handleResendVerification} className="space-y-4">
                  <input
                    type="email"
                    value={resendEmail}
                    onChange={(e) => setResendEmail(e.target.value)}
                    className="input w-full"
                    placeholder="you@example.com"
                    required
                  />
                  {resendMessage && (
                    <p className={`text-sm ${resendMessage.includes('sent') ? 'text-success-600' : 'text-danger-600'}`}>
                      {resendMessage}
                    </p>
                  )}
                  <button
                    type="submit"
                    disabled={isResending}
                    className="btn-primary w-full disabled:opacity-50"
                  >
                    {isResending ? 'Sending...' : 'Resend Verification Email'}
                  </button>
                </form>
              </div>
            </div>
          )}

          {status === 'no-token' && (
            <div className="text-center py-8">
              <div className="mx-auto mb-4 h-16 w-16 rounded-full bg-gray-100 flex items-center justify-center">
                <Mail className="h-8 w-8 text-gray-500" />
              </div>
              <h2 className="text-xl font-semibold text-gray-900 mb-2">Verify Your Email</h2>
              <p className="text-gray-600 mb-6">
                Check your inbox for a verification link. If you didn&apos;t receive one, request a new link below.
              </p>
              
              <form onSubmit={handleResendVerification} className="space-y-4">
                <input
                  type="email"
                  value={resendEmail}
                  onChange={(e) => setResendEmail(e.target.value)}
                  className="input w-full"
                  placeholder="you@example.com"
                  required
                />
                {resendMessage && (
                  <p className={`text-sm ${resendMessage.includes('sent') ? 'text-success-600' : 'text-danger-600'}`}>
                    {resendMessage}
                  </p>
                )}
                <button
                  type="submit"
                  disabled={isResending}
                  className="btn-primary w-full disabled:opacity-50"
                >
                  {isResending ? 'Sending...' : 'Send Verification Email'}
                </button>
              </form>
            </div>
          )}

          <div className="mt-6 text-center text-sm text-gray-600">
            <Link href="/login" className="text-primary-600 hover:text-primary-700 font-medium">
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

export default function VerifyEmailPage() {
  return (
    <Suspense fallback={
      <div className="min-h-screen bg-gradient-to-br from-primary-50 via-white to-accent-50 flex items-center justify-center">
        <Loader2 className="h-8 w-8 text-primary-600 animate-spin" />
      </div>
    }>
      <VerifyEmailContent />
    </Suspense>
  )
}
