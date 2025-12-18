import Image from 'next/image';
import type { Metadata } from 'next';
import React from 'react';
import Breadcrumbs from '../../components/ui/Breadcrumbs';
import Section from '@/components/ui/Section';
import Container from '@/components/ui/Container';

export const metadata: Metadata = {
  title: 'About — ApexMediation',
  description:
    'Learn about ApexMediation: transparent ad mediation, verifiable auction integrity, performance, and developer-first tooling.',
  openGraph: {
    title: 'About — ApexMediation',
    description:
      'Transparent ad mediation with verifiable metrics and developer-first tooling. Learn how ApexMediation raises the bar for trust and performance.',
    type: 'article',
  },
};

export default function AboutPage() {
  return (
    <Section as="main">
      <Container className="prose dark:prose-invert max-w-4xl">
        <Breadcrumbs items={[{ label: 'Home', href: '/' }, { label: 'About' }]} />

        <div className="flex items-center gap-4 not-prose mb-6">
          <div className="relative h-12 w-12 overflow-hidden rounded">
            <Image
              src="/api/brand/logo"
              alt="ApexMediation logo"
              fill
              sizes="48px"
              className="object-contain dark:invert dark:opacity-90"
              priority
            />
          </div>
          <h1 className="m-0">About ApexMediation</h1>
        </div>

        <section aria-labelledby="mission">
          <h2 id="mission">Mission: Radical Transparency in Ad Mediation</h2>
          <p>
            ApexMediation is built on a simple idea: publishers deserve verifiable performance and transparent
            economics without black boxes. We engineered a mediation stack where every meaningful step—auction
            request, adapter response, decision latency, and delivery outcome—can be traced, measured, and
            independently verified.
          </p>
        </section>

        <section aria-labelledby="transparency">
          <h2 id="transparency">Transparency, Verified</h2>
          <p>
            Our <strong>Transparency</strong> feature produces a cryptographically signed trail of each auction,
            making it possible to confirm integrity end-to-end. Dashboards expose RED metrics (Rate, Errors,
            Duration) per route and adapter latency distributions (p50 / p95 / p99), with error taxonomies to
            accelerate root cause analysis. This is not a marketing claim—it is an observable system with
            invariant checks and evidence you can audit.
          </p>
          <ul>
            <li>Signed auction artifacts and verification endpoints</li>
            <li>Adapter-level latency, timeout, and no-fill analytics</li>
            <li>Prometheus metrics + Grafana dashboards ready for SLOs</li>
          </ul>
        </section>

        <section aria-labelledby="performance">
          <h2 id="performance">Performance without Compromise</h2>
          <p>
            We optimize for user experience and revenue simultaneously. SDKs are size-gated and run under
            StrictMode to prevent main-thread I/O. We enforce binary compatibility and publish API docs so you can
            upgrade with confidence. On the web, we target <abbr title="Largest Contentful Paint">LCP</abbr>
            &nbsp;&lt; 2.5s (p75) and <abbr title="Cumulative Layout Shift">CLS</abbr>&nbsp;&lt; 0.1. Budgets are baked into CI to prevent regression. Where
            certain telemetry is still rolling out, we label it as roadmap until live dashboards back it with data.
          </p>
        </section>

        <section aria-labelledby="developer-first">
          <h2 id="developer-first">Developer‑First by Design</h2>
          <p>
            Everything ships with tests, docs, and CI guardrails: Android, iOS, Unity, CTV, Console, and Backend.
            Release lanes generate artifacts automatically (AAR, XCFramework/DocC, UPM tarballs), with size and API
            surface checks. Our billing and privacy flows—usage metering, invoice PDFs, GDPR export/delete—are
            implemented with tenant scoping and audited.
          </p>
        </section>

        <section aria-labelledby="trust">
          <h2 id="trust">Trust as a Feature</h2>
          <p>
            We treat trust as a product surface: hardening security headers (CSP, HSTS, Permissions-Policy), strict
            cookie/session policies, and redaction in logs by default. When incidents happen, operators get precise
            runbooks with synthetic probes and alert rules that reflect actual user journeys.
          </p>
        </section>

        <section aria-labelledby="why-apex">
          <h2 id="why-apex">Why ApexMediation</h2>
          <ul>
            <li>Verifiable transparency and evidence, not claims</li>
            <li>Performance budgets that ship in CI, not slideware</li>
            <li>SDKs designed for reliability and minimal footprint</li>
            <li>Operational excellence: metrics, alerts, playbooks</li>
          </ul>
          <p>
            If you value clarity and control over your monetization, ApexMediation was built for you.
          </p>
        </section>
      </Container>
    </Section>
  );
}
