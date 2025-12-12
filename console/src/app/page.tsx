import { getServerSession } from 'next-auth'
import { redirect } from 'next/navigation'
import Link from 'next/link'
import { authOptions } from '@/lib/auth/options'

export const dynamic = 'force-dynamic'
export const revalidate = 0

export default async function Home() {
  const session = await getServerSession(authOptions)

  if (session) {
    redirect('/dashboard')
  }

  return (
    <div className="min-h-screen bg-gradient-to-b from-gray-50 to-white flex items-center justify-center px-4 py-12">
      <section className="max-w-3xl w-full text-center space-y-8">
        <div>
          <p className="text-sm font-semibold uppercase tracking-[0.3em] text-primary-500 mb-3">
            Apex Mediation Console
          </p>
          <h1 className="text-4xl sm:text-5xl font-bold text-gray-900 leading-tight">
            One secure workspace for billing, fraud, and migration controls
          </h1>
          <p className="mt-4 text-lg text-gray-600">
            Sign in to access live revenue dashboards, migration studio insights, and transparency tooling for your organization.
          </p>
        </div>

        <div className="flex flex-col sm:flex-row items-center justify-center gap-4">
          <Link
            href="/login"
            className="btn btn-primary px-8 py-3 text-base font-semibold"
          >
            Go to Login
          </Link>
          <a
            href="https://github.com/sbelakho2/Ad-Project/blob/main/docs/QUICK_START.md"
            target="_blank"
            rel="noreferrer"
            className="btn btn-outline px-8 py-3 text-base font-semibold"
          >
            View Quick Start
          </a>
        </div>

        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 text-left">
          <div className="rounded-xl border border-gray-200 bg-white p-4 shadow-sm">
            <p className="text-xs font-semibold text-gray-500 uppercase tracking-wide">Open Access</p>
            <p className="mt-2 text-sm text-gray-600">
              Console routes enforce RBAC per page. Use your organization credentials to gain the right level of access.
            </p>
          </div>
          <div className="rounded-xl border border-gray-200 bg-white p-4 shadow-sm">
            <p className="text-xs font-semibold text-gray-500 uppercase tracking-wide">Live Data</p>
            <p className="mt-2 text-sm text-gray-600">
              Billing, payouts, and transparency views connect directly to production APIs—no demo data here.
            </p>
          </div>
          <div className="rounded-xl border border-gray-200 bg-white p-4 shadow-sm">
            <p className="text-xs font-semibold text-gray-500 uppercase tracking-wide">Secure Auth</p>
            <p className="mt-2 text-sm text-gray-600">
              NextAuth sessions use short-lived JWTs with CSRF protection. Contact support if you need access adjustments.
            </p>
          </div>
        </div>
      </section>
    </div>
  )
}
