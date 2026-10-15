+++
title = "For publishers: what working with Apex looks like"
description = "Who Apex is for, what access it needs, how the free audit runs, and what the first ninety days of the autopilot actually change."
+++

## Who this is for

Apex fits apps earning roughly **$10k to $300k per month** on AppLovin MAX, LevelPlay or AdMob, where nobody has advertising operations as a full-time job. The pattern is the same at almost every studio in that range. An engineer or a founder checks the mediation dashboard when something looks wrong. Floors were configured once and revisited rarely, and experiments exist in the platform but almost never get run to a decision.

If that is your situation, the leaks this site describes are probably already in your account. The free audit tells you which ones, with numbers.

## What the free audit needs

Read access to your platform reporting and configuration — nothing more. The audit only reads: it makes no change to your platform settings and runs from a deployment with no write credentials. No SDK, no app release, no contract. You receive a findings report where every finding has a severity, an evidence trail, and an estimated monthly recovery, or an explicit statement that the data cannot support an estimate.

## What applying changes needs

When you decide to act on findings, Apex needs credentials that can write through the platform's own management API. That step is gated:

- Writes only happen on paths confirmed against the platform's live sandbox.

- Writes only happen while the audit has no unresolved critical findings.

- Every write runs as a platform-native A/B experiment with a control group, and every write has a rollback.

You can revoke write credentials at any time; the audit and your exported ledger keep working without them. Network credentials from publishers are kept encrypted under AES-256-GCM.

## What the first ninety days typically look like

1. **Week 0 — audit.** You get the findings report. Nothing changes in your account.

2. **Weeks 1–2 — first changes.** Apex proposes a short list of changes ranked by expected value and blast radius. Each one is applied through the management API and immediately enters a platform-native experiment.

3. **Weeks 3–8 — the loop.** Experiments finish, results settle: clears the threshold, it stays; misses it, it rolls back automatically and the rollback is recorded. The findings list is re-audited continuously as the account changes.

4. **Month 3 — the ledger.** You have a signed history of every change, every rollback, and the platform's own measured result behind each one. You also have your first invoice lines, each re-derived from that history.

## What stays yours

- Your platform accounts, your apps, your traffic. Apex never intermediates ad serving.

- The ledger, exportable and verifiable offline.

- The decision to stop: roll back everything, revoke credentials, and keep the audit evidence.

## Common questions

**Do I need to switch mediation?** No switch is needed. Apex works on top of the mediation you already run: MAX, LevelPlay or AdMob.

**Do I need a partnership or a network deal?** No partnerships are needed. Apex uses the management APIs your accounts already expose.

**What happens to a change that loses?** It is rolled back automatically through the same management API and recorded in the ledger as a rollback. There is no fee for a change that did not produce measured lift.

**How do I know the lift is real?** You don't take our word for it: every gain is proven by the platform's own built-in A/B experiment, visible in your own account, and billed only from those results.

**How do I start?** Leave your contact and portfolio below, or write to [hello@apexmediation.ee](mailto:hello@apexmediation.ee). You get an audit timeline and a first findings report before anyone discusses a contract.

{{< lead-form >}}

The form posts to our lead endpoint and we reply by email. Nothing else is stored.

## The fine print worth reading twice

Apex is an optimization layer for accounts you control. It does not guarantee revenue outcomes; it guarantees the method: platform-native changes, platform-native proof, signed records, automatic rollback, and fees that exist only where the platform's own numbers show measured lift. That guarantee is inspectable in the ledger, not just stated here.
