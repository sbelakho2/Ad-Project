'use client';

import type { ReactNode } from 'react';
import HeroShell, { type HeroShellVariant } from '@/components/HeroShell';

/**
 * PageHero - Shared Hero Section Component
 * 
 * ⚠️ DO NOT BYPASS THIS COMPONENT ⚠️
 * 
 * This component enforces the site-wide hero design system.
 * Creating inline <section> hero blocks causes design drift.
 * 
 * Design tokens (LOCKED - see DESIGN_SYSTEM.md):
 * - Wrapper: container mx-auto px-4
 * - Padding: py-16 md:py-20
 * - Max text width: max-w-3xl (use in children)
 * - H1: text-4xl md:text-5xl font-bold leading-tight
 * - Subhead: text-xl leading-relaxed
 * 
 * Variants:
 * - "dark": Marketing pages (/, /product, /features, /pricing, /company)
 *   Colors: text-white, text-white/90, text-white/80
 *   Buttons: bg-brand-600 hover:bg-brand-700, bg-white/10 hover:bg-white/15
 * 
 * - "light": Trust/docs pages (/developers, /security, /contact)
 *   Colors: text-ink, text-inkMuted
 *   Optional tint: className="bg-gray-50 border-b border-gray-200/60"
 * 
 * @see DESIGN_SYSTEM.md for full standards
 */

export type PageHeroVariant = 'dark' | 'light';

export default function PageHero({
  variant,
  children,
  className,
}: {
  variant?: PageHeroVariant;
  children: ReactNode;
  className?: string;
}) {
  const heroVariant: HeroShellVariant | undefined =
    variant === 'dark' ? 'marketingDark' : variant === 'light' ? 'trustLight' : undefined;

  return (
    <HeroShell variant={heroVariant} className={className}>
      {children}
    </HeroShell>
  );
}
