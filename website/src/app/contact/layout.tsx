import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Contact — ApexMediation',
  description: 'Get in touch with the ApexMediation team. We respond within one business day for sales, support, partnerships, and security inquiries.',
  openGraph: {
    title: 'Contact — ApexMediation',
    description: 'Get in touch with the ApexMediation team. We respond within one business day for sales, support, partnerships, and security inquiries.',
    type: 'website',
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Contact — ApexMediation',
    description: 'Get in touch with the ApexMediation team. We respond within one business day.',
  },
  alternates: {
    canonical: 'https://apexmediation.ee/contact',
  },
};

export default function ContactLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
