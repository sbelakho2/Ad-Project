import { NextAuthOptions } from 'next-auth'
import CredentialsProvider from 'next-auth/providers/credentials'
import GithubProvider from 'next-auth/providers/github'
import { apiClient } from '@/lib/api-client'

const USE_MOCK_API = process.env.NEXT_PUBLIC_USE_MOCK_API === 'true'
const ALLOW_DEMO_AUTH = process.env.NEXT_PUBLIC_ALLOW_DEMO_AUTH !== 'false'
const IS_PRODUCTION = process.env.NODE_ENV === 'production'
const allowMockAuth = USE_MOCK_API && ALLOW_DEMO_AUTH && !IS_PRODUCTION

if (USE_MOCK_API && IS_PRODUCTION) {
  console.warn('[auth] NEXT_PUBLIC_USE_MOCK_API is ignored in production environments')
}

const buildOauthProviders = () => {
  const providers: NextAuthOptions['providers'] = []

  const githubClientId = process.env.GITHUB_CLIENT_ID
  const githubClientSecret = process.env.GITHUB_CLIENT_SECRET

  if (githubClientId && githubClientSecret) {
    providers.push(
      GithubProvider({
        clientId: githubClientId,
        clientSecret: githubClientSecret,
      })
    )
  }

  return providers
}

export const authOptions: NextAuthOptions = {
  providers: [
    CredentialsProvider({
      name: 'Credentials',
      credentials: {
        email: { label: 'Email', type: 'email' },
        password: { label: 'Password', type: 'password' },
        twofaCode: { label: '2FA Code', type: 'text' },
        tempToken: { label: 'Temp Token', type: 'text' },
      },
      async authorize(credentials) {
        const email = credentials?.email
        const password = credentials?.password
        const twofaCode = credentials?.twofaCode
        const tempToken = credentials?.tempToken
        const isCompletingTwofa = Boolean(tempToken && twofaCode)

        if (!isCompletingTwofa && (!email || !password)) {
          return null
        }

        try {
          if (allowMockAuth && !isCompletingTwofa) {
            if (credentials?.email?.includes('demo') || credentials?.password === 'demo') {
              return {
                id: 'user_1',
                email: credentials?.email,
                name: 'Demo Publisher',
                role: 'publisher' as const,
                publisherId: 'pub_demo_123',
                accessToken: 'mock_token_123',
              }
            }
            return null
          }

          if (isCompletingTwofa) {
            const response = await apiClient.post('/auth/login2fa', {
              tempToken,
              code: twofaCode,
            })

            const payload = response.data?.data || response.data
            const { user, token } = payload || {}

            if (user && token) {
              return {
                id: user.id,
                email: user.email,
                name: user.name || user.companyName || user.email,
                role: user.role,
                publisherId: user.publisherId,
                accessToken: token,
              }
            }

            return null
          }

          const response = await apiClient.post('/auth/login', {
            email,
            password,
          })

          const payload = response.data?.data || response.data

          if (payload?.twofaRequired) {
            throw new Error(
              JSON.stringify({ code: 'TWOFA_REQUIRED', tempToken: payload.tempToken, email })
            )
          }

          const { user, token } = payload || {}

          if (user && token) {
            return {
              id: user.id,
              email: user.email,
              name: user.name || user.companyName || user.email,
              role: user.role,
              publisherId: user.publisherId,
              accessToken: token,
            }
          }

          return null
        } catch (error) {
          console.error('Authentication error:', error)
          return null
        }
      },
    }),
    ...buildOauthProviders(),
  ],
  callbacks: {
    async jwt({ token, user, trigger, session }) {
      if (user) {
        token.accessToken = (user as any).accessToken
        token.role = (user as any).role
        token.publisherId = (user as any).publisherId
        token.sessionVersion = Date.now()
        if (token.role === 'admin') {
          token.elevatedAt = Date.now()
        }
        return token
      }

      if (trigger === 'update' && session && (session as any).role) {
        const newRole = (session as any).role
        if (newRole !== token.role) {
          token.role = newRole
          token.sessionVersion = Date.now()
          if (newRole === 'admin') token.elevatedAt = Date.now()
        }
      }
      return token
    },
    async session({ session, token }) {
      const isAdmin = token.role === 'admin'
      const MAX_AGE_SECONDS = isAdmin ? 4 * 60 * 60 : 24 * 60 * 60
      ;(session as any).maxAge = MAX_AGE_SECONDS

      if (session.user) {
        ;(session.user as any).role = token.role
        ;(session.user as any).publisherId = (token as any).publisherId
      }
      ;(session as any).accessToken = (token as any).accessToken
      ;(session as any).sessionVersion = (token as any).sessionVersion
      ;(session as any).elevatedAt = (token as any).elevatedAt
      return session
    },
  },
  pages: {
    signIn: '/login',
    error: '/login',
  },
  session: {
    strategy: 'jwt',
    maxAge: 24 * 60 * 60,
    updateAge: 60 * 60,
  },
  cookies: (() => {
    const cookies: any = {}
    // In production, scope cookies to the console origin to avoid redirect loops caused by cookie mismatch
    if (process.env.NODE_ENV === 'production') {
      const url = process.env.NEXTAUTH_URL || ''
      try {
        const u = new URL(url)
        const domain = u.hostname
        cookies.sessionToken = {
          name: `__Secure-next-auth.session-token`,
          options: {
            httpOnly: true,
            sameSite: 'lax',
            path: '/',
            secure: true,
            domain,
          },
        }
      } catch {
        // Fallback: let NextAuth defaults apply
      }
    }
    return cookies
  })(),
  useSecureCookies: process.env.NODE_ENV === 'production',
  secret: process.env.NEXTAUTH_SECRET || 'development-secret-change-in-production',
  events: {
    async signIn() {
      return
    },
  },
}
