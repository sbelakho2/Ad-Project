'use client';

import Link from 'next/link';
import { useState } from 'react';

/**
 * Homepage navigation
 * Reference: Design.md § "Main Navigation"
 */
export default function HomeNav() {
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);

  const navItems = [
    { label: 'Features', href: '/#features' },
    { label: 'Pricing', href: '/pricing' },
    { label: 'Documentation', href: '/documentation' },
    { label: 'About', href: '/about' },
    { label: 'Contact', href: '/contact' },
  ];

  return (
    <nav className="relative z-20">
      {/* Desktop Navigation */}
      <div className="hidden md:flex items-center justify-center gap-8 py-6">
        <Link href="/" className="mr-4">
          <img src="/logo.jpg" alt="ApexMediation" className="h-10 w-auto" />
        </Link>
        {navItems.map((item) => (
          <Link
            key={item.href}
            href={item.href}
            className="text-brand-100 font-bold uppercase text-sm tracking-wide hover:underline decoration-2 underline-offset-4 transition-all focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-white"
          >
            {item.label}
          </Link>
        ))}

        <div className="flex items-center gap-4 ml-4">
          <Link
            href="/signin"
            className="text-brand-100 font-bold uppercase text-sm hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-white"
          >
            Sign In
          </Link>
          <Link
            href="/signup"
            className="btn-primary px-6 py-2 font-bold uppercase text-sm hover:shadow-lg transition-all"
          >
            Sign Up
          </Link>
        </div>
      </div>

      {/* Mobile Navigation */}
      <div className="md:hidden flex items-center justify-between py-4">
        <span className="text-brand-100 font-bold uppercase text-lg">Menu</span>
        <button
          onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
          className="text-brand-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
          aria-expanded={mobileMenuOpen}
          aria-controls="primary-navigation"
          aria-label="Toggle menu"
        >
          <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            {mobileMenuOpen ? (
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            ) : (
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
            )}
          </svg>
        </button>
      </div>

      {/* Mobile Menu Overlay */}
      {mobileMenuOpen && (
        <>
          <div
            className="fixed inset-0 bg-black/50 z-40 md:hidden"
            onClick={() => setMobileMenuOpen(false)}
          />
          <div
            id="primary-navigation"
            className="fixed left-0 top-0 bottom-0 w-64 bg-brand-600 text-white z-50 p-6 md:hidden"
            role="dialog"
            aria-modal="true"
          >
          <div className="flex justify-between items-center mb-8">
              <span className="text-brand-100 font-bold uppercase text-lg">Menu</span>
              <button
                onClick={() => setMobileMenuOpen(false)}
                className="text-brand-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
                aria-label="Close menu"
              >
                ×
              </button>
            </div>
            <div className="flex flex-col gap-4">
              {navItems.map((item) => (
                <Link
                  key={item.href}
                  href={item.href}
                  onClick={() => setMobileMenuOpen(false)}
                  className="text-brand-100 font-bold uppercase text-sm py-2 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
                >
                  {item.label}
                </Link>
              ))}
              <hr className="border-white/20 my-2" />
              <Link
                href="/signin"
                onClick={() => setMobileMenuOpen(false)}
                className="text-brand-100 font-bold uppercase text-sm py-2 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
              >
                Sign In
              </Link>
              <Link
                href="/signup"
                onClick={() => setMobileMenuOpen(false)}
                className="btn-primary px-6 py-2 font-bold uppercase text-sm text-center"
              >
                Sign Up
              </Link>
            </div>
          </div>
        </>
      )}
    </nav>
  );
}
