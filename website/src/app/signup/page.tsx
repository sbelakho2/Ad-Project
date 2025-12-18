"use client";

import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useEffect, useMemo, useState } from 'react';

export default function SignUpPage() {
  const router = useRouter();

  const [formData, setFormData] = useState({
    name: '',
    email: '',
    password: '',
    confirmPassword: '',
    companyName: '',
  });
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [consent, setConsent] = useState(false);
  const [csrf, setCsrf] = useState<string | null>(null);

  // Derived validations
  const emailValid = useMemo(() => /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/.test(formData.email), [formData.email]);
  const passwordValid = useMemo(() => {
    const p = formData.password;
    if (p.length < 10) return false;
    let classes = 0;
    if (/[a-z]/.test(p)) classes++;
    if (/[A-Z]/.test(p)) classes++;
    if (/[0-9]/.test(p)) classes++;
    if (/[^A-Za-z0-9]/.test(p)) classes++;
    return classes >= 3;
  }, [formData.password]);
  const passwordsMatch = formData.password === formData.confirmPassword;
  const canSubmit = emailValid && passwordValid && passwordsMatch && consent && !loading;

  useEffect(() => {
    try {
      const m = document.cookie.match(/(?:^|; )csrf_token=([^;]+)/);
      if (m && m[1]) setCsrf(decodeURIComponent(m[1]));
    } catch {
      // ignore
    }
  }, []);

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setFormData({
      ...formData,
      [e.target.name]: e.target.value,
    });
  };

  const handleSubmit = async (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    setError('');
    if (!emailValid) { setError('Please enter a valid email address.'); return; }
    if (!passwordValid) { setError('Password must be at least 10 characters and include 3 of: upper, lower, number, symbol.'); return; }
    if (!passwordsMatch) { setError('Passwords do not match'); return; }
    if (!consent) { setError('Please accept the Terms and Privacy Policy.'); return; }

    setLoading(true);

    try {
      const response = await fetch('/api/auth/signup', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(csrf ? { 'X-CSRF-Token': csrf } : {}),
        },
        body: JSON.stringify({
          name: formData.name,
          email: formData.email,
          password: formData.password,
          companyName: formData.companyName,
          consent: true,
          // Honeypot — real users won't fill this hidden field
          hp: (e.currentTarget.elements.namedItem('hp') as HTMLInputElement | null)?.value || ''
        }),
      });

      const data = await response.json().catch(() => ({}));

      if (!data.success) {
        if (response.status === 429) {
          setError('Too many attempts. Please wait a moment and try again.');
        } else {
          setError(data.error || 'Registration failed');
        }
        setLoading(false);
        return;
      }

      // Redirect to dashboard
      router.push('/dashboard');
    } catch (err) {
      setError('An error occurred. Please try again.');
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8 text-gray-900">
      <div className="max-w-md w-full space-y-8 bg-white p-10 rounded-xl shadow-md">
        <div>
          <h2 className="mt-6 text-center text-h2-sm font-semibold text-gray-900 tracking-tight">
            Start maximizing your ad revenue
          </h2>
          <p className="mt-2 text-center text-sm text-gray-600">
            Already have an account?{' '}
            <Link href="https://console.apexmediation.ee/login" className="font-semibold text-brand-600 hover:text-brand-700 underline">
              Sign in
            </Link>
          </p>
        </div>

        <div className="rounded-lg border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700 space-y-2" role="note">
          <p className="font-semibold text-gray-900">What you get</p>
          <ul className="list-disc space-y-1 pl-5">
            <li>Free up to $10k/month mediated revenue per app—no credit card required.</li>
            <li>All SDKs, dashboard access, and real-time analytics included.</li>
            <li>Upgrade anytime with simple, transparent pricing.</li>
          </ul>
        </div>

        <form className="mt-8 space-y-6" onSubmit={handleSubmit} aria-label="Create account form">
          {error && (
            <div className="rounded-md bg-danger/10 p-4">
              <div className="flex">
                <div className="ml-3">
                  <h3 className="text-sm font-medium text-danger">{error}</h3>
                </div>
              </div>
            </div>
          )}

          <div className="space-y-4">
            <div>
              <label htmlFor="name" className="block text-sm font-medium text-gray-700">
                Full Name
              </label>
              <input
                id="name"
                name="name"
                type="text"
                required
                className="input-v2 w-full mt-1"
                placeholder="John Doe"
                value={formData.name}
                onChange={handleChange}
                disabled={loading}
              />
            </div>

            <div>
              <label htmlFor="email" className="block text-sm font-medium text-gray-700">
                Email Address
              </label>
              <input
                id="email"
                name="email"
                type="email"
                autoComplete="email"
                required
                className="input-v2 w-full mt-1"
                placeholder="you@example.com"
                value={formData.email}
                onChange={handleChange}
                disabled={loading}
              />
            </div>

            <div>
              <label htmlFor="companyName" className="block text-sm font-medium text-gray-700">
                Company Name (optional)
              </label>
              <input
                id="companyName"
                name="companyName"
                type="text"
                className="input-v2 w-full mt-1"
                placeholder="Your Company Inc."
                value={formData.companyName}
                onChange={handleChange}
                disabled={loading}
              />
            </div>


            <div>
              <label htmlFor="password" className="block text-sm font-medium text-gray-700">
                Password
              </label>
              <input
                id="password"
                name="password"
                type="password"
                autoComplete="new-password"
                required
                className="input-v2 w-full mt-1"
                placeholder="At least 10 chars; 3 of upper/lower/number/symbol"
                value={formData.password}
                onChange={handleChange}
                disabled={loading}
              />
              <p className="mt-1 text-xs text-gray-500" role="note">
                {passwordValid ? 'Strong enough' : 'Use at least 10 characters and include a mix of cases, numbers, or symbols.'}
              </p>
            </div>

            <div>
              <label htmlFor="confirmPassword" className="block text-sm font-medium text-gray-700">
                Confirm Password
              </label>
              <input
                id="confirmPassword"
                name="confirmPassword"
                type="password"
                autoComplete="new-password"
                required
                className="input-v2 w-full mt-1"
                placeholder="Confirm your password"
                value={formData.confirmPassword}
                onChange={handleChange}
                disabled={loading}
              />
            </div>
          </div>

          {/* Consent checkbox */}
          <div className="flex items-start gap-2">
            <input
              id="consent"
              name="consent"
              type="checkbox"
              className="mt-1 h-4 w-4 rounded border-gray-300 text-brand-600 focus:ring-brand-500"
              checked={consent}
              onChange={(e) => setConsent(e.target.checked)}
              aria-describedby="consent-hint"
            />
            <label htmlFor="consent" className="text-sm text-gray-900">
              I agree to the <a href="/terms" className="text-brand-600 hover:text-brand-700 font-semibold underline">Terms of Service</a> and{' '}
              <a href="/privacy" className="text-brand-600 hover:text-brand-700 font-semibold underline">Privacy Policy</a>.
            </label>
          </div>
          <p id="consent-hint" className="sr-only">Required to create an account.</p>

          {/* Honeypot */}
          <input type="text" name="hp" aria-hidden="true" tabIndex={-1} className="hidden" />

          <div>
            <button
              type="submit"
              disabled={!canSubmit}
              className="btn-primary w-full py-3 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {loading ? 'Creating account...' : 'Get started free'}
            </button>
          </div>

          <p className="text-xs text-center text-gray-500">No credit card required. You keep 100% of your ad network payouts.</p>
        </form>
      </div>
    </div>
  );
}
