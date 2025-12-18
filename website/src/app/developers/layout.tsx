import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Developers — ApexMediation SDKs',
  description:
    'Open-source SDKs for Android, iOS, Unity, and Web. Quick start examples, debugging tools, and migration support for production teams.',
  openGraph: {
    title: 'Developers — ApexMediation SDKs',
    description:
      'Open-source SDKs for Android, iOS, Unity, and Web. Quick start examples, debugging tools, and migration support for production teams.',
    type: 'website',
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Developers — ApexMediation SDKs',
    description:
      'Open-source SDKs for Android, iOS, Unity, and Web. Quick start examples, debugging tools, and migration support for production teams.',
  },
  alternates: {
    canonical: 'https://apexmediation.ee/developers',
  },
};

export default function DevelopersLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
