'use client';

import type { ReactNode } from 'react';
import { usePathname } from 'next/navigation';

export type HeroShellVariant = 'marketingDark' | 'trustLight';

function getDefaultHeroVariant(pathname: string): HeroShellVariant {
  // Central policy:
  // - Marketing pages: dark hero
  // - Trust/docs pages: light hero
  // Keep this allowlist small to avoid accidentally making unknown pages "marketing".
  const marketingPrefixes = ['/', '/product', '/features', '/pricing', '/company'];

  if (pathname === '/') return 'marketingDark';
  if (marketingPrefixes.some(prefix => prefix !== '/' && pathname.startsWith(prefix))) return 'marketingDark';

  return 'trustLight';
}

export default function HeroShell({
  variant,
  children,
  className,
}: {
  variant?: HeroShellVariant;
  children: ReactNode;
  className?: string;
}) {
  const pathname = usePathname();
  const resolvedVariant = variant ?? getDefaultHeroVariant(pathname || '/');

  const variantClasses =
    resolvedVariant === 'marketingDark'
      ? 'relative overflow-hidden bg-gradient-to-br from-slate-950 via-slate-900 to-slate-800'
      : 'bg-white text-slate-900';

  return (
    <section
      data-hero-variant={resolvedVariant}
      className={[variantClasses, className].filter(Boolean).join(' ')}
    >
      {resolvedVariant === 'marketingDark' ? (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-0"
        >
          <div
            className={[
              'absolute inset-0 opacity-60',
              'bg-[radial-gradient(circle_at_30%_20%,theme(colors.brand.500/.28),transparent_55%)]',
            ].join(' ')}
          />
          <div
            className={[
              'absolute inset-0 opacity-25',
              'bg-[linear-gradient(to_right,theme(colors.white/.08)_1px,transparent_1px),linear-gradient(to_bottom,theme(colors.white/.08)_1px,transparent_1px)]',
              'bg-[size:64px_64px]',
            ].join(' ')}
          />
        </div>
      ) : null}

      <div className="container mx-auto px-4 py-16 md:py-20 relative">{children}</div>
    </section>
  );
}
