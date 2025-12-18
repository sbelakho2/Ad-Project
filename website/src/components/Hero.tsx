'use client';

import type { ReactNode } from 'react';
import PageHero, { type PageHeroVariant } from '@/components/PageHero';

/**
 * Hero – canonical hero component.
 * Use <Hero variant="dark|light"> across all pages.
 */
export default function Hero({
  variant,
  children,
  className,
}: {
  variant?: PageHeroVariant;
  children: ReactNode;
  className?: string;
}) {
  return (
    <PageHero variant={variant} className={className}>
      <div className="min-h-[320px] flex items-center">
        <div className="w-full">{children}</div>
      </div>
    </PageHero>
  );
}
