'use client';

import { cn } from '@/lib/utils';

interface VisualCardProps {
  children: React.ReactNode;
  className?: string;
  title?: string;
  variant?: 'default' | 'dark' | 'brand';
}

/**
 * VisualCard - A monospace ASCII art card with tinted background
 * Used to visually illustrate mediation flows and key features
 */
export function VisualCard({ children, className, title, variant = 'default' }: VisualCardProps) {
  const variants = {
    default: 'bg-slate-50/80 border-slate-200 text-slate-700',
    dark: 'bg-slate-900/95 border-slate-700 text-slate-300',
    brand: 'bg-brand-50/80 border-brand-200 text-brand-800',
  };

  return (
    <div
      className={cn(
        'rounded-xl border p-4 md:p-5 font-mono text-xs md:text-sm leading-relaxed overflow-x-auto',
        variants[variant],
        className
      )}
    >
      {title && (
        <div className="text-[10px] uppercase tracking-wider font-semibold mb-3 opacity-60">
          {title}
        </div>
      )}
      <pre className="whitespace-pre overflow-x-auto">{children}</pre>
    </div>
  );
}

// Pre-built visual cards for common use cases

export function MediationFlowCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Mediation Flow" className={className}>
{`Publisher SDK
     │
     ▼
┌─────────────────────┐
│ Apex Decision Engine│
│   (rules + ML)      │
└─────────┬───────────┘
          │
    ├─────┴─────┬─────────┐
    ▼           ▼         ▼
Network A   Network B   Network C

Signed Log:   OK ✓
Rollback:     Ready ✓`}
    </VisualCard>
  );
}

export function MigrationPipelineCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Migration Studio" className={className}>
{`Config v12 ──▶ Simulate ──▶ Canary 5% ──▶ 100%
                 │               │
                 └── rollback ◀──┘

Import:     Your existing setup
Test:       Before switching
Rollout:    Per-placement
Revert:     Instant if needed`}
    </VisualCard>
  );
}

export function SignatureChainCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Cryptographic Transparency" className={className}>
{`Auction #48291
├─ Timestamp:  2025-01-01T12:00:00Z
├─ Bids:       [A: $2.50, B: $2.45, C: $2.10]
├─ Winner:     Network A
├─ Floor:      $2.00
└─ Signature:  Ed25519:AbCd...1234

Verify:     export → canonicalize → verify
Dispute:    cryptographic evidence`}
    </VisualCard>
  );
}

export function CircuitBreakerCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Reliability Controls" className={className}>
{`┌─ Circuit Breakers ──────────┐
│  Network A: ● healthy       │
│  Network B: ○ degraded (2s) │
│  Network C: ● healthy       │
└─────────────────────────────┘

Timeouts:   Per-adapter limits
Kill-switch: Flip instantly
Rollback:   Auto on errors
SLO:        99.9% target`}
    </VisualCard>
  );
}

export function RetentionMatrixCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Log Retention" className={className}>
{`┌───────────┬─────────────────┐
│ Starter   │ 30 days         │
│ Growth    │ 90 days         │
│ Scale     │ 1 year          │
│ Enterprise│ Custom + archive│
└───────────┴─────────────────┘

All plans: Full audit logs
Export:    JSON, CSV, BQ/S3`}
    </VisualCard>
  );
}

export function PrinciplesToProductCard({ className }: { className?: string }) {
  return (
    <VisualCard title="Principles → Product" className={className}>
{`Transparency → Signed auction logs
Control      → Your networks, your payouts
Privacy      → No fingerprinting
Reliability  → Rollback & SLOs

Every principle is code, not just copy.`}
    </VisualCard>
  );
}

export function ContactBenefitsCard({ className }: { className?: string }) {
  return (
    <VisualCard title="What happens next" className={className}>
{`1. We respond within 24 hours
2. Schedule a call if needed
3. Get personalized onboarding

Include your platform details
for faster setup!

   ✓ Android / iOS / Unity / Web
   ✓ Current ad networks
   ✓ Monthly ad impressions`}
    </VisualCard>
  );
}

export function SignupBenefitsCard({ className }: { className?: string }) {
  return (
    <VisualCard title="What you get" className={className} variant="brand">
{`✓ Direct payouts (100% yours)
✓ Signed auction logs
✓ Rollback-safe rollout
✓ Open SDKs (all platforms)
✓ Real-time analytics

Free up to $10k/month
No credit card required`}
    </VisualCard>
  );
}

export function ComparisonTableCard({ className }: { className?: string }) {
  return (
    <VisualCard title="ApexMediation vs Traditional" className={className}>
{`Feature              Apex    Legacy
─────────────────────────────────────
Transparency         ✓       ✗
Signed logs          ✓       ✗
Direct payouts       ✓       partial
Instant rollback     ✓       ✗
Per-adapter SLOs     ✓       ✗
Data warehouse export✓       ✗
Open SDKs            ✓       ✗`}
    </VisualCard>
  );
}

export default VisualCard;
