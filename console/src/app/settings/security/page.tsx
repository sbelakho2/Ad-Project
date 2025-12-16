'use client'

import { useCallback, useEffect, useState } from 'react'
import { useSession } from 'next-auth/react'
import { useRouter } from 'next/navigation'
import Image from 'next/image'
import { Shield, ShieldCheck, ShieldOff, Copy, Check, AlertTriangle, Loader2 } from 'lucide-react'
import { apiClient } from '@/lib/api-client'

type TwofaStatus = {
  enabled: boolean
  backupCodesRemaining?: number
}

type EnrollmentData = {
  qrDataUrl: string
  otpauthUrl: string
  maskedSecret: string
}

export default function SecuritySettingsPage() {
  const { data: session, status: sessionStatus } = useSession()
  const router = useRouter()

  const [twofaStatus, setTwofaStatus] = useState<TwofaStatus | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  // Enrollment flow state
  const [enrollmentData, setEnrollmentData] = useState<EnrollmentData | null>(null)
  const [verifyCode, setVerifyCode] = useState('')
  const [verifyError, setVerifyError] = useState('')
  const [verifyLoading, setVerifyLoading] = useState(false)
  const [backupCodes, setBackupCodes] = useState<string[] | null>(null)
  const [copiedBackup, setCopiedBackup] = useState(false)

  // Disable flow state
  const [showDisableModal, setShowDisableModal] = useState(false)
  const [disablePassword, setDisablePassword] = useState('')
  const [disableCode, setDisableCode] = useState('')
  const [disableError, setDisableError] = useState('')
  const [disableLoading, setDisableLoading] = useState(false)

  // Regenerate backup codes state
  const [showRegenModal, setShowRegenModal] = useState(false)
  const [regenLoading, setRegenLoading] = useState(false)
  const [regenError, setRegenError] = useState('')

  // Redirect if not authenticated
  useEffect(() => {
    if (sessionStatus === 'unauthenticated') {
      router.push('/login?next=/settings/security')
    }
  }, [sessionStatus, router])

  // Fetch 2FA status
  const fetchStatus = useCallback(async () => {
    try {
      setLoading(true)
      setError(null)
      const res = await apiClient.get<{ data: TwofaStatus }>('/auth/twofa/status')
      setTwofaStatus(res.data?.data || { enabled: false })
    } catch (e: any) {
      // If endpoint doesn't exist yet, assume 2FA is disabled
      if (e?.response?.status === 404) {
        setTwofaStatus({ enabled: false })
      } else {
        setError('Unable to fetch security settings')
      }
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    if (sessionStatus === 'authenticated') {
      fetchStatus()
    }
  }, [sessionStatus, fetchStatus])

  // Start enrollment
  const handleEnroll = async () => {
    try {
      setVerifyError('')
      setVerifyLoading(true)
      const res = await apiClient.post<{ data: EnrollmentData }>('/auth/twofa/enroll')
      setEnrollmentData(res.data?.data || null)
    } catch (e: any) {
      setVerifyError(e?.response?.data?.error || 'Failed to start enrollment')
    } finally {
      setVerifyLoading(false)
    }
  }

  // Verify and enable 2FA
  const handleVerify = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!verifyCode.trim()) {
      setVerifyError('Enter the 6-digit code from your authenticator app')
      return
    }

    try {
      setVerifyLoading(true)
      setVerifyError('')
      const res = await apiClient.post<{ data: { backupCodes: string[] } }>('/auth/twofa/verify', {
        token: verifyCode.trim(),
      })
      setBackupCodes(res.data?.data?.backupCodes || [])
      setEnrollmentData(null)
      setVerifyCode('')
    } catch (e: any) {
      setVerifyError(e?.response?.data?.error || 'Invalid code. Please try again.')
    } finally {
      setVerifyLoading(false)
    }
  }

  // Copy backup codes to clipboard
  const handleCopyBackupCodes = async () => {
    if (!backupCodes) return
    try {
      await navigator.clipboard.writeText(backupCodes.join('\n'))
      setCopiedBackup(true)
      setTimeout(() => setCopiedBackup(false), 2000)
    } catch {
      // Fallback for older browsers
      const textarea = document.createElement('textarea')
      textarea.value = backupCodes.join('\n')
      document.body.appendChild(textarea)
      textarea.select()
      document.execCommand('copy')
      document.body.removeChild(textarea)
      setCopiedBackup(true)
      setTimeout(() => setCopiedBackup(false), 2000)
    }
  }

  // Finish enrollment (user has saved backup codes)
  const handleFinishEnrollment = () => {
    setBackupCodes(null)
    fetchStatus()
  }

  // Disable 2FA
  const handleDisable = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!disablePassword.trim() || !disableCode.trim()) {
      setDisableError('Both password and 2FA code are required')
      return
    }

    try {
      setDisableLoading(true)
      setDisableError('')
      await apiClient.post('/auth/twofa/disable', {
        password: disablePassword,
        code: disableCode.trim(),
      })
      setShowDisableModal(false)
      setDisablePassword('')
      setDisableCode('')
      fetchStatus()
    } catch (e: any) {
      setDisableError(e?.response?.data?.error || 'Failed to disable 2FA. Check your password and code.')
    } finally {
      setDisableLoading(false)
    }
  }

  // Regenerate backup codes
  const handleRegenerate = async () => {
    try {
      setRegenLoading(true)
      setRegenError('')
      const res = await apiClient.post<{ data: { backupCodes: string[] } }>('/auth/twofa/backup-codes/regenerate')
      setBackupCodes(res.data?.data?.backupCodes || [])
      setShowRegenModal(false)
    } catch (e: any) {
      setRegenError(e?.response?.data?.error || 'Failed to regenerate backup codes')
    } finally {
      setRegenLoading(false)
    }
  }

  if (sessionStatus === 'loading' || loading) {
    return (
      <div className="min-h-screen bg-gray-50 flex items-center justify-center">
        <Loader2 className="w-8 h-8 animate-spin text-primary-600" />
      </div>
    )
  }

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <p className="text-sm font-medium text-primary-600">Settings</p>
          <h1 className="text-3xl font-semibold text-gray-900 tracking-tight">Security</h1>
          <p className="text-sm text-gray-600 mt-1">
            Manage two-factor authentication and account security settings.
          </p>
        </div>
      </header>

      <main className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-8">
        {error && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 flex items-start gap-3">
            <AlertTriangle className="w-5 h-5 text-red-600 flex-shrink-0 mt-0.5" />
            <p className="text-red-800">{error}</p>
          </div>
        )}

        {/* Two-Factor Authentication Section */}
        <section className="bg-white border rounded-lg p-6">
          <div className="flex items-start justify-between">
            <div className="flex items-start gap-4">
              <div className={`h-12 w-12 rounded-lg flex items-center justify-center ${
                twofaStatus?.enabled ? 'bg-green-100 text-green-600' : 'bg-gray-100 text-gray-500'
              }`}>
                {twofaStatus?.enabled ? <ShieldCheck className="w-6 h-6" /> : <Shield className="w-6 h-6" />}
              </div>
              <div>
                <h2 className="text-xl font-semibold text-gray-900">Two-Factor Authentication</h2>
                <p className="text-sm text-gray-600 mt-1">
                  Add an extra layer of security by requiring a code from your authenticator app when signing in.
                </p>
                {twofaStatus?.enabled && (
                  <p className="text-sm text-green-600 mt-2 flex items-center gap-1">
                    <ShieldCheck className="w-4 h-4" />
                    Enabled
                    {twofaStatus.backupCodesRemaining !== undefined && (
                      <span className="text-gray-500 ml-2">
                        ({twofaStatus.backupCodesRemaining} backup codes remaining)
                      </span>
                    )}
                  </p>
                )}
              </div>
            </div>
          </div>

          {/* Enrollment Flow */}
          {!twofaStatus?.enabled && !enrollmentData && !backupCodes && (
            <div className="mt-6">
              <button
                onClick={handleEnroll}
                disabled={verifyLoading}
                className="px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700 disabled:opacity-50 flex items-center gap-2"
              >
                {verifyLoading && <Loader2 className="w-4 h-4 animate-spin" />}
                Enable Two-Factor Authentication
              </button>
            </div>
          )}

          {/* QR Code Scanning Step */}
          {enrollmentData && !backupCodes && (
            <div className="mt-6 border-t pt-6">
              <h3 className="text-lg font-medium text-gray-900 mb-4">Scan QR Code</h3>
              <div className="grid md:grid-cols-2 gap-6">
                <div>
                  <p className="text-sm text-gray-600 mb-4">
                    Scan this QR code with your authenticator app (Google Authenticator, Authy, 1Password, etc.)
                  </p>
                  <div className="bg-white border rounded-lg p-4 inline-block">
                    <Image
                      src={enrollmentData.qrDataUrl}
                      alt="2FA QR Code"
                      width={200}
                      height={200}
                      unoptimized
                    />
                  </div>
                  <p className="text-xs text-gray-500 mt-2">
                    Can&apos;t scan? Enter this code manually: <code className="bg-gray-100 px-1 rounded">{enrollmentData.maskedSecret}</code>
                  </p>
                </div>
                <div>
                  <form onSubmit={handleVerify}>
                    <label htmlFor="verify-code" className="block text-sm font-medium text-gray-700 mb-2">
                      Enter verification code
                    </label>
                    <input
                      id="verify-code"
                      type="text"
                      inputMode="numeric"
                      pattern="[0-9]*"
                      maxLength={6}
                      value={verifyCode}
                      onChange={(e) => setVerifyCode(e.target.value.replace(/\D/g, ''))}
                      placeholder="000000"
                      className="w-full px-4 py-2 border rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500"
                      autoComplete="one-time-code"
                    />
                    {verifyError && (
                      <p className="text-red-600 text-sm mt-2">{verifyError}</p>
                    )}
                    <div className="flex gap-3 mt-4">
                      <button
                        type="submit"
                        disabled={verifyLoading || verifyCode.length !== 6}
                        className="px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700 disabled:opacity-50 flex items-center gap-2"
                      >
                        {verifyLoading && <Loader2 className="w-4 h-4 animate-spin" />}
                        Verify & Enable
                      </button>
                      <button
                        type="button"
                        onClick={() => setEnrollmentData(null)}
                        className="px-4 py-2 border rounded-lg hover:bg-gray-50"
                      >
                        Cancel
                      </button>
                    </div>
                  </form>
                </div>
              </div>
            </div>
          )}

          {/* Backup Codes Display */}
          {backupCodes && (
            <div className="mt-6 border-t pt-6">
              <div className="bg-yellow-50 border border-yellow-200 rounded-lg p-4 mb-4">
                <div className="flex items-start gap-3">
                  <AlertTriangle className="w-5 h-5 text-yellow-600 flex-shrink-0 mt-0.5" />
                  <div>
                    <p className="font-medium text-yellow-800">Save your backup codes</p>
                    <p className="text-sm text-yellow-700 mt-1">
                      Store these codes somewhere safe. Each code can only be used once if you lose access to your authenticator app.
                    </p>
                  </div>
                </div>
              </div>
              <div className="bg-gray-50 border rounded-lg p-4 font-mono text-sm">
                <div className="grid grid-cols-2 gap-2">
                  {backupCodes.map((code) => (
                    <div key={code} className="bg-white px-3 py-2 rounded border">
                      {code}
                    </div>
                  ))}
                </div>
              </div>
              <div className="flex gap-3 mt-4">
                <button
                  onClick={handleCopyBackupCodes}
                  className="px-4 py-2 border rounded-lg hover:bg-gray-50 flex items-center gap-2"
                >
                  {copiedBackup ? <Check className="w-4 h-4 text-green-600" /> : <Copy className="w-4 h-4" />}
                  {copiedBackup ? 'Copied!' : 'Copy codes'}
                </button>
                <button
                  onClick={handleFinishEnrollment}
                  className="px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700"
                >
                  I&apos;ve saved my codes
                </button>
              </div>
            </div>
          )}

          {/* Management Options (when 2FA is enabled) */}
          {twofaStatus?.enabled && !backupCodes && (
            <div className="mt-6 border-t pt-6 flex flex-wrap gap-3">
              <button
                onClick={() => setShowRegenModal(true)}
                className="px-4 py-2 border rounded-lg hover:bg-gray-50"
              >
                Regenerate backup codes
              </button>
              <button
                onClick={() => setShowDisableModal(true)}
                className="px-4 py-2 border border-red-200 text-red-600 rounded-lg hover:bg-red-50"
              >
                Disable 2FA
              </button>
            </div>
          )}
        </section>

        {/* Session Security Info */}
        <section className="bg-white border rounded-lg p-6">
          <h2 className="text-xl font-semibold text-gray-900 mb-2">Session Security</h2>
          <p className="text-sm text-gray-600">
            Your session is secured with HTTP-only cookies and CSRF protection. 
            Sessions expire after 24 hours of inactivity.
          </p>
          <p className="text-sm text-gray-500 mt-2">
            Signed in as: <span className="font-medium">{session?.user?.email}</span>
          </p>
        </section>
      </main>

      {/* Disable 2FA Modal */}
      {showDisableModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-white rounded-xl shadow-xl max-w-md w-full p-6">
            <div className="flex items-center gap-3 mb-4">
              <div className="h-10 w-10 rounded-full bg-red-100 flex items-center justify-center">
                <ShieldOff className="w-5 h-5 text-red-600" />
              </div>
              <h3 className="text-lg font-semibold text-gray-900">Disable Two-Factor Authentication</h3>
            </div>
            <p className="text-sm text-gray-600 mb-4">
              This will remove the extra layer of security from your account. Enter your password and a current 2FA code to confirm.
            </p>
            <form onSubmit={handleDisable}>
              <div className="space-y-4">
                <div>
                  <label htmlFor="disable-password" className="block text-sm font-medium text-gray-700 mb-1">Password</label>
                  <input
                    id="disable-password"
                    type="password"
                    value={disablePassword}
                    onChange={(e) => setDisablePassword(e.target.value)}
                    className="w-full px-4 py-2 border rounded-lg focus:ring-2 focus:ring-primary-500"
                    autoComplete="current-password"
                  />
                </div>
                <div>
                  <label htmlFor="disable-code" className="block text-sm font-medium text-gray-700 mb-1">2FA Code or Backup Code</label>
                  <input
                    id="disable-code"
                    type="text"
                    value={disableCode}
                    onChange={(e) => setDisableCode(e.target.value)}
                    placeholder="Enter code"
                    className="w-full px-4 py-2 border rounded-lg focus:ring-2 focus:ring-primary-500"
                    autoComplete="one-time-code"
                  />
                </div>
              </div>
              {disableError && (
                <p className="text-red-600 text-sm mt-3">{disableError}</p>
              )}
              <div className="flex gap-3 mt-6">
                <button
                  type="button"
                  onClick={() => {
                    setShowDisableModal(false)
                    setDisablePassword('')
                    setDisableCode('')
                    setDisableError('')
                  }}
                  className="flex-1 px-4 py-2 border rounded-lg hover:bg-gray-50"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={disableLoading}
                  className="flex-1 px-4 py-2 bg-red-600 text-white rounded-lg hover:bg-red-700 disabled:opacity-50 flex items-center justify-center gap-2"
                >
                  {disableLoading && <Loader2 className="w-4 h-4 animate-spin" />}
                  Disable 2FA
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Regenerate Backup Codes Modal */}
      {showRegenModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-white rounded-xl shadow-xl max-w-md w-full p-6">
            <h3 className="text-lg font-semibold text-gray-900 mb-2">Regenerate Backup Codes</h3>
            <p className="text-sm text-gray-600 mb-4">
              This will invalidate all existing backup codes and generate 10 new ones. Make sure to save the new codes.
            </p>
            {regenError && (
              <p className="text-red-600 text-sm mb-4">{regenError}</p>
            )}
            <div className="flex gap-3">
              <button
                onClick={() => {
                  setShowRegenModal(false)
                  setRegenError('')
                }}
                className="flex-1 px-4 py-2 border rounded-lg hover:bg-gray-50"
              >
                Cancel
              </button>
              <button
                onClick={handleRegenerate}
                disabled={regenLoading}
                className="flex-1 px-4 py-2 bg-primary-600 text-white rounded-lg hover:bg-primary-700 disabled:opacity-50 flex items-center justify-center gap-2"
              >
                {regenLoading && <Loader2 className="w-4 h-4 animate-spin" />}
                Generate New Codes
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
