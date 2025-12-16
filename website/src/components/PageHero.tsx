'use client';

import type { ReactNode } from 'react';

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
 *   Colors: text-white, text-slate-200/90, text-slate-300/80
 *   Buttons: bg-brand-600 hover:bg-brand-700, bg-white/10 hover:bg-white/15
 * 
 * - "light": Trust/docs pages (/developers, /security, /contact)
 *   Colors: text-slate-900, text-slate-600
 *   Optional tint: className="bg-slate-50 border-b border-slate-200/60"
 * 
 * @see DESIGN_SYSTEM.md for full standards
 */

export type PageHeroVariant = 'dark' | 'light';

export default function PageHero({
  variant,
  children,
  className,
}: {
  variant: PageHeroVariant;
  children: ReactNode;
  className?: string;
}) {
  const variantClasses =
    variant === 'dark'
      ? 'bg-gradient-to-br from-slate-950 via-slate-900 to-slate-800 text-white'
      : 'bg-white text-slate-900';

  return (
    <section className={[variantClasses, className].filter(Boolean).join(' ')}>
      <div className="container mx-auto px-4 py-16 md:py-20">{children}</div>
    </section>
  );
}
