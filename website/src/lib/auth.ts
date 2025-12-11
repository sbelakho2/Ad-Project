import { SignJWT, jwtVerify } from 'jose';
import { cookies } from 'next/headers';

// Use environment variable for production
// The check is deferred to runtime to avoid build failures
const JWT_SECRET_RAW = process.env.JWT_SECRET;
const IS_BUILD_TIME = process.env.NEXT_PHASE === 'phase-production-build';

function getJwtSecret(): Uint8Array {
  if (process.env.NODE_ENV === 'production' && !JWT_SECRET_RAW && !IS_BUILD_TIME) {
    throw new Error('JWT_SECRET environment variable is required in production');
  }
  return new TextEncoder().encode(
    JWT_SECRET_RAW || 'dev-only-secret-not-for-production'
  );
}

export interface User {
  id: string;
  email: string;
  publisherId: string;
  role: 'admin' | 'developer' | 'viewer';
  name?: string;
}

export interface SessionPayload {
  userId: string;
  email: string;
  publisherId: string;
  role: string;
  iat?: number;
  exp?: number;
}

/**
 * Generate a JWT token for the user
 */
export async function signToken(user: User): Promise<string> {
  return new SignJWT({
    userId: user.id,
    publisherId: user.publisherId,
    email: user.email,
    role: user.role,
  })
    .setProtectedHeader({ alg: 'HS256' })
    .setExpirationTime('7d')
    .setIssuedAt()
    .sign(getJwtSecret());
}

/**
 * Verify and decode a JWT token
 */
export async function verifyToken(token: string): Promise<User | null> {
  try {
    const { payload } = await jwtVerify(token, getJwtSecret());

    return {
      id: payload.userId as string,
      email: payload.email as string,
      publisherId: payload.publisherId as string,
      role: payload.role as 'admin' | 'developer' | 'viewer',
    };
  } catch (error) {
    console.error('Token verification failed:', error);
    return null;
  }
}

/**
 * Get the current user session
 */
export async function getSession(): Promise<User | null> {
  const cookieStore = await cookies();
  const token = cookieStore.get('session')?.value;
  if (!token) {
    return null;
  }
  return verifyToken(token);
}

/**
 * Check if user is authenticated
 */
export async function isAuthenticated(): Promise<boolean> {
  const user = await getSession();
  return user !== null;
}

/**
 * Require authentication (for API routes and server components)
 */
export async function requireAuth(): Promise<User> {
  const user = await getSession();
  if (!user) {
    throw new Error('Unauthorized');
  }
  return user;
}
