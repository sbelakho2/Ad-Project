'use client';

import type { ReactNode } from 'react';
import HeroShell from '@/components/HeroShell';
import type { HeroShellVariant } from '@/components/HeroShell';

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
