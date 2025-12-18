'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { useState, useEffect } from 'react';

/**
 * Homepage navigation - cleaned up and simplified
 * 
 * ⚠️ Navigation order is LOCKED (see DESIGN_SYSTEM.md)
 * Changes require UX review + A/B test validation
 */
export default function HomeNav() {
  const pathname = usePathname();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);

  // Close menu on route change
  useEffect(() => {
    setMobileMenuOpen(false);
    document.body.style.overflow = '';
  }, [pathname]);

  // Handle body scroll lock
  useEffect(() => {
    if (mobileMenuOpen) {
      document.body.style.overflow = 'hidden';
    } else {
      document.body.style.overflow = '';
    }
    return () => {
      document.body.style.overflow = '';
    };
  }, [mobileMenuOpen]);

  // ⚠️ LOCKED NAVIGATION ORDER - Do not reorder without design review
  // Current: Product → Features → Pricing → Company → Developers → Security
  const navItems = [
    { label: 'Product', href: '/product' },
    { label: 'Features', href: '/features' },
    { label: 'Pricing', href: '/pricing' },
    { label: 'Company', href: '/company' },
    { label: 'Developers', href: '/developers' },
    { label: 'Security', href: '/security' },
  ];

  const isActive = (href: string) => pathname === href;

  const toggleMenu = () => {
    setMobileMenuOpen(prev => !prev);
  };

  const closeMenu = () => {
    setMobileMenuOpen(false);
  };

  return (
    <nav className="relative" style={{ zIndex: 50 }} aria-label="Main navigation">
      {/* Desktop Navigation */}
      <div className="hidden md:flex items-center justify-between w-full">
        <Link href="/" className="flex-shrink-0">
          <img
            src="/api/brand/logo"
            alt="ApexMediation"
            className="h-44 w-auto dark:invert dark:opacity-90"
          />
        </Link>
        <div className="flex items-center gap-6">
          {navItems.map((item) => (
            <Link
              key={item.href}
              href={item.href}
              aria-current={isActive(item.href) ? 'page' : undefined}
              className={`text-gray-700 font-medium text-sm hover:text-brand-600 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-brand-600 ${isActive(item.href) ? 'text-brand-600' : ''}`}
            >
              {item.label}
            </Link>
          ))}

          <div className="flex items-center gap-3 ml-4">
            <Link
              href="https://console.apexmediation.ee/login"
              className="text-gray-700 font-medium text-sm hover:text-brand-600 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-brand-600"
            >
              Sign In
            </Link>
            <Link
              href="/signup"
              className="bg-brand-500 text-white px-5 py-2 rounded-lg font-semibold text-sm hover:bg-brand-400 transition-colors"
            >
              Get Started
            </Link>
          </div>
        </div>
      </div>

      {/* Mobile Navigation */}
      <div className="md:hidden flex items-center justify-between py-4">
        <Link href="/" className="flex items-center gap-2">
          <img
            src="/api/brand/logo"
            alt="ApexMediation"
            className="h-28 w-auto dark:invert dark:opacity-90"
          />
        </Link>
        <button
          type="button"
          onClick={toggleMenu}
          className="text-gray-700 p-2 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand-600"
          aria-expanded={mobileMenuOpen}
          aria-controls="mobile-navigation"
          aria-label={mobileMenuOpen ? 'Close menu' : 'Open menu'}
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
            className="fixed inset-0 bg-black/50 md:hidden"
            style={{ zIndex: 9998 }}
            onClick={closeMenu}
            aria-hidden="true"
          />
          <div
            id="mobile-navigation"
            className="fixed left-0 top-0 bottom-0 w-72 bg-brand-600 text-white md:hidden p-6"
            style={{ zIndex: 9999 }}
            role="dialog"
            aria-modal="true"
            aria-label="Mobile navigation"
          >
            <div className="flex justify-between items-center mb-8">
              <span className="text-brand-100 font-bold uppercase text-lg">Menu</span>
              <button
                type="button"
                onClick={closeMenu}
                className="text-brand-100 p-2 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
                aria-label="Close menu"
              >
                <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
            <div className="flex flex-col gap-4">
              {navItems.map((item) => (
                <Link
                  key={item.href}
                  href={item.href}
                  onClick={closeMenu}
                  aria-current={isActive(item.href) ? 'page' : undefined}
                  className={`text-brand-100 font-bold uppercase text-sm py-2 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white ${isActive(item.href) ? 'underline' : ''}`}
                >
                  {item.label}
                </Link>
              ))}
              <hr className="border-white/20 my-2" />
              <Link
                href="https://console.apexmediation.ee/login"
                onClick={closeMenu}
                className="text-brand-100 font-bold uppercase text-sm py-2 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
              >
                Sign In
              </Link>
              <Link
                href="/signup"
                onClick={closeMenu}
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
