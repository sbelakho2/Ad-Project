import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Security — ApexMediation',
  description: 'Security-by-design mediation with cryptographic transparency, data minimization, configurable retention, and auditable decision logs.',
  openGraph: {
    title: 'Security — ApexMediation',
    description: 'Security-by-design mediation with cryptographic transparency, data minimization, configurable retention, and auditable decision logs.',
    type: 'website',
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Security — ApexMediation',
    description: 'Security-by-design mediation with cryptographic transparency, data minimization, configurable retention, and auditable decision logs.',
  },
  alternates: {
    canonical: 'https://apexmediation.ee/security',
  },
};

export default function SecurityLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
