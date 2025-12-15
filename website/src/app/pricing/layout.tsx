import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Pricing — ApexMediation',
  description: 'Transparent platform pricing. Networks pay you directly. Start free, then scale with predictable fees and configurable log retention.',
  openGraph: {
    title: 'Pricing — ApexMediation',
    description: 'Transparent platform pricing. Networks pay you directly. Start free, then scale with predictable fees and configurable log retention.',
    type: 'website',
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Pricing — ApexMediation',
    description: 'Transparent platform pricing. Networks pay you directly. Start free, then scale with predictable fees and configurable log retention.',
  },
  alternates: {
    canonical: 'https://apexmediation.ee/pricing',
  },
};

export default function PricingLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
