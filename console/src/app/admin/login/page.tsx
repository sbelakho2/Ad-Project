"use client"

import { useState, useEffect } from 'react'
import { useRouter } from 'next/navigation'
import { setOperatorCredentials } from '@/lib/useAdminGate'

/**
 * OPERATOR LOGIN PAGE
 * 
 * This is a separate login mechanism for system operators (company staff).
 * It is NOT connected to customer authentication in any way.
 * 
 * Operators authenticate via:
 * 1. Operator secret key (NEXT_PUBLIC_OPERATOR_SECRET)
 * 2. Or their whitelisted email (NEXT_PUBLIC_OPERATOR_EMAILS)
 * 
 * For production: Consider implementing proper SSO/OAuth with your identity provider.
 */

export default function OperatorLoginPage() {
  const router = useRouter()
  const [secret, setSecret] = useState('')
  const [email, setEmail] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [isLoading, setIsLoading] = useState(false)

  const handleLogin = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)
    setIsLoading(true)

    // Try to authenticate with operator secret
    const success = setOperatorCredentials(secret, email || undefined)
    
    if (success) {
      router.push('/admin/health')
      return
    }

    // If secret didn't work, check if email is whitelisted
    // This would require a server-side check in production
    setError('Invalid operator credentials. Access denied.')
    setIsLoading(false)
  }

  return (
    <div className="min-h-screen bg-gray-900 flex items-center justify-center px-4">
      <div className="max-w-md w-full space-y-8">
        <div className="text-center">
          <h1 className="text-3xl font-bold text-white">Operator Access</h1>
          <p className="mt-2 text-gray-400">
            System Administration Portal
          </p>
          <p className="mt-1 text-sm text-gray-500">
            This is for authorized operators only. Not for customers.
          </p>
        </div>

        <div className="bg-gray-800 rounded-xl p-8 border border-gray-700">
          <form onSubmit={handleLogin} className="space-y-6">
            <div>
              <label htmlFor="email" className="block text-sm font-medium text-gray-300">
                Operator Email (optional)
              </label>
              <input
                id="email"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                className="mt-1 block w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
                placeholder="operator@apexmediation.com"
              />
            </div>

            <div>
              <label htmlFor="secret" className="block text-sm font-medium text-gray-300">
                Operator Secret Key
              </label>
              <input
                id="secret"
                type="password"
                value={secret}
                onChange={(e) => setSecret(e.target.value)}
                required
                className="mt-1 block w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
                placeholder="••••••••••••••••"
              />
            </div>

            {error && (
              <div className="bg-red-900/50 border border-red-700 rounded-lg px-4 py-3 text-red-200 text-sm">
                {error}
              </div>
            )}

            <button
              type="submit"
              disabled={isLoading || !secret}
              className="w-full py-3 px-4 bg-primary-600 hover:bg-primary-700 disabled:bg-gray-600 disabled:cursor-not-allowed text-white font-medium rounded-lg transition-colors"
            >
              {isLoading ? 'Authenticating...' : 'Access Operator Console'}
            </button>
          </form>

          <div className="mt-6 pt-6 border-t border-gray-700">
            <p className="text-xs text-gray-500 text-center">
              All operator actions are logged for audit purposes.
              <br />
              Unauthorized access attempts will be reported.
            </p>
          </div>
        </div>

        <p className="text-center text-gray-500 text-sm">
          Looking for the customer console?{' '}
          <a href="/login" className="text-primary-400 hover:text-primary-300">
            Go to customer login →
          </a>
        </p>
      </div>
    </div>
  )
}
