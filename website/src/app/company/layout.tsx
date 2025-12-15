import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Company — ApexMediation',
  description: 'ApexMediation is built by ads engineers for publishers who demand transparency, reliability, and control over their ad stack. Headquartered in Estonia.',
  openGraph: {
    title: 'Company — ApexMediation',
    description: 'ApexMediation is built by ads engineers for publishers who demand transparency, reliability, and control over their ad stack. Headquartered in Estonia.',
    type: 'website',
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Company — ApexMediation',
    description: 'ApexMediation is built by ads engineers for publishers who demand transparency, reliability, and control over their ad stack.',
  },
  alternates: {
    canonical: 'https://apexmediation.ee/company',
  },
};

export default function CompanyLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
