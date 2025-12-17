import type { ReactNode } from 'react';

interface SocialIconLinkProps {
  href: string;
  label: string;
  children: ReactNode;
}

/**
 * Icon link with consistent sizing and focus states for footer and elsewhere.
 */
export default function SocialIconLink({ href, label, children }: SocialIconLinkProps) {
  return (
    <a
      href={href}
      aria-label={label}
      className="flex h-10 w-10 items-center justify-center rounded-full border border-gray-200 text-gray-700 transition-all duration-200 hover:border-gray-300 hover:text-brand-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand-600 [&>svg]:h-5 [&>svg]:w-5 [&>svg]:shrink-0 [&>svg]:fill-current"
    >
      <span className="block" aria-hidden="true">
        {children}
      </span>
    </a>
  );
}
