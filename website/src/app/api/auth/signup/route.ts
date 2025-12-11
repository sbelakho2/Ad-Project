import { api } from '@/lib/api';
import { signToken } from '@/lib/auth';
import { NextRequest, NextResponse } from 'next/server';

function isValidEmail(email: string) {
  // RFC 5322-lite, pragmatic validation
  return /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/.test(email);
}

function isStrongPassword(pw: string) {
  if (pw.length < 10) return false;
  let classes = 0;
  if (/[a-z]/.test(pw)) classes++;
  if (/[A-Z]/.test(pw)) classes++;
  if (/[0-9]/.test(pw)) classes++;
  if (/[^A-Za-z0-9]/.test(pw)) classes++;
  return classes >= 3;
}

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const email = (body?.email ?? '').toString();
    const password = (body?.password ?? '').toString();
    const name = (body?.name ?? '').toString();
    const companyName = (body?.companyName ?? '').toString();
    const consent = !!body?.consent;
    const honeypot = (body?.hp ?? body?.company)?.toString?.() ?? '';

    // Validate input
    if (!email || !password || !name) {
      return NextResponse.json(
        { success: false, error: 'Email, password, and name are required' },
        { status: 400 }
      );
    }

    // Email validation
    if (!isValidEmail(email)) {
      return NextResponse.json(
        { success: false, error: 'Invalid email format' },
        { status: 400 }
      );
    }

    // Password validation
    if (!isStrongPassword(password)) {
      return NextResponse.json(
        { success: false, error: 'Password must be at least 10 characters and include 3 of: upper, lower, number, symbol.' },
        { status: 400 }
      );
    }

    // Require consent checkbox
    if (!consent) {
      return NextResponse.json(
        { success: false, error: 'Consent to Terms and Privacy Policy is required' },
        { status: 400 }
      );
    }

    // Honeypot: short-circuit pretend success to avoid leaking signal to bots
    if (honeypot) {
      return NextResponse.json({ success: true }, { status: 200 });
    }

    // Call backend registration endpoint
    const response = await api.post('/auth/register', {
      email,
      password,
      name,
      companyName,
    });

    if (!response.success || !response.data) {
      return NextResponse.json(
        { success: false, error: response.error || 'Registration failed' },
        { status: response.status === 429 ? 429 : 400 }
      );
    }

    const user = response.data.user;

    // Generate JWT for website
    const token = await signToken({
      id: user.id,
      email: user.email,
      publisherId: user.publisherId || user.id,
      role: user.role || 'developer',
      name: user.name,
    });

    // Create response with user data
    const res = NextResponse.json({
      success: true,
      user: {
        id: user.id,
        email: user.email,
        publisherId: user.publisherId || user.id,
        role: user.role || 'developer',
        name: user.name,
      },
    });

    // Set httpOnly cookie
    res.cookies.set('session', token, {
      httpOnly: true,
      secure: process.env.NODE_ENV === 'production',
      sameSite: 'lax',
      maxAge: 60 * 60 * 24 * 7, // 7 days
      path: '/',
    });

    return res;
  } catch (error) {
    console.error('Registration error:', error);
    return NextResponse.json(
      { success: false, error: 'Internal server error' },
      { status: 500 }
    );
  }
}
