import type { Metadata } from 'next';
import { Inter, Plus_Jakarta_Sans } from 'next/font/google';
import './globals.css';
import SkipToContent from '@/components/SkipToContent';
import { ThemeProvider } from '@/components/ui/ThemeProvider';

const inter = Inter({ subsets: ['latin'], variable: '--font-inter', display: 'swap' });
const pjs = Plus_Jakarta_Sans({ subsets: ['latin'], variable: '--font-pjs', display: 'swap' });

const SITE_ORIGIN = 'https://apexmediation.ee';

export const metadata: Metadata = {
  metadataBase: new URL(SITE_ORIGIN),
  title: 'ApexMediation — Transparent Ad Mediation with Verifiable Auction Logs',
  description: 'Ad mediation for mobile, CTV, and web publishers. ML-powered optimization, real-time analytics, and cryptographically verifiable auction logs—without taking your network payouts.',
  keywords: ['ad mediation', 'mobile ads', 'Unity ads', 'iOS ads', 'Android ads', 'ad monetization', 'CTV advertising', 'transparent mediation'],
  authors: [{ name: 'Bel Consulting OÜ' }],
  alternates: {
    canonical: SITE_ORIGIN,
  },
  openGraph: {
    title: 'ApexMediation — Transparent Ad Mediation with Verifiable Auction Logs',
    description: 'Ad mediation for mobile, CTV, and web publishers. ML-powered optimization, real-time analytics, and cryptographically verifiable auction logs—without taking your network payouts.',
    url: SITE_ORIGIN,
    siteName: 'ApexMediation',
    images: [
      {
        url: '/og-image.png',
        width: 1200,
        height: 630,
      },
    ],
    locale: 'en_US',
    type: 'website',
  },
  twitter: {
    card: 'summary_large_image',
    title: 'ApexMediation — Transparent Ad Mediation with Verifiable Auction Logs',
    description: 'Ad mediation for mobile, CTV, and web publishers. ML-powered optimization, real-time analytics, and cryptographically verifiable auction logs.',
    images: ['/og-image.png'],
  },
  robots: {
    index: true,
    follow: true,
  },
};

export const viewport = {
  width: 'device-width',
  initialScale: 1,
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className={`${inter.variable} ${pjs.variable}`}>
      <head>
        <link rel="icon" href="/favicon.ico" />
        <link rel="apple-touch-icon" href="/apple-touch-icon.png" />
        <meta name="theme-color" content="#356eff" />
      </head>
      <body>
        <SkipToContent />
        <ThemeProvider>
          <div id="main-content" tabIndex={-1}>
            {children}
          </div>
        </ThemeProvider>
      </body>
    </html>
  );
}
