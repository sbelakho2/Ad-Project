import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Product — ApexMediation',
  description:
    'Optimize every ad request with ML-powered waterfalls, signed auction logs, Migration Studio rollouts, and reliability controls built for production SDKs.',
  openGraph: {
    title: 'Product — ApexMediation',
    description:
      'Optimize every ad request with ML-powered waterfalls, signed auction logs, Migration Studio rollouts, and reliability controls built for production SDKs.',
    type: 'website',
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Product — ApexMediation',
    description:
      'Optimize every ad request with ML-powered waterfalls, signed auction logs, Migration Studio rollouts, and reliability controls built for production SDKs.',
  },
  alternates: {
    canonical: 'https://apexmediation.ee/product',
  },
};

export default function ProductLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
