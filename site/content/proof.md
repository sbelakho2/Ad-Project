+++
title = "Proof you can check without trusting us"
description = "Every gain is proven by the platform's own built-in A/B experiment and recorded in a signed, hash-chained change log that you can re-verify offline."
+++

Marketing pages are full of lift claims. Apex is built so that none of ours need to be taken on faith. There are exactly two sources of truth, and neither of them is an Apex dashboard.

## Source one: the platform's own A/B experiment

Every gain is proven by the platform's own built-in A/B experiment. When Apex proposes a change, the experiment that decides its fate is created inside your MAX, LevelPlay or AdMob account through the platform's own management API. The platform owns the traffic split, the measurement window and the reported result.

Three consequences follow, and they are the reason this design was chosen:

- **You can watch it live.** Open the platform's experiments screen while a change is being tested. The control arm and the variant arm are both yours.
- **The result is not editable by Apex.** Apex reads the finished experiment; it does not compute the outcome, normalize it, or adjust it. Only the platform's finished result counts.
- **A running experiment is not a result.** Settlement refuses to read a running experiment as a result. A change under test stays locked — unmodified, un-billed — until the platform reports a finished outcome.

The keep decision compares the platform's reported result against the thresholds set before the experiment started: a minimum lift rate and a minimum confidence. Both must be cleared for the change to stay. Below either threshold, the change is rolled back.

## Source two: the signed change log

Every applied change and rollback is appended to a signed, hash-chained ledger. One record per line, and each record carries:

- the platform and the operation, with before and after values,
- the idempotency key that identifies the change across retries,
- the experiment identity and, once settled, the measured result,
- for rollbacks, the record they roll back.

Each line is signed with an Ed25519 signature and chained to the previous line by a SHA-256 hash that covers the full record including its predecessor hash. Altering a value, a sequence number, or a single hexadecimal digit invalidates the machine check. Verification does not merely parse the file: it re-serializes each record and requires the on-disk bytes to match the canonical form exactly, so injected fields, whitespace edits and hex-case tricks are rejected rather than silently accepted.

You do not need us to run verification. The ledger is an append-only file; the verification procedure is deterministic and runs offline. If an Apex employee, a bug, or a third party modified history, the check would fail on the line that was touched.

## Rollback is part of the proof

A failed change is as documented as a successful one. If a change fails to clear the keep threshold, Apex automatically rolls it back. The rollback becomes its own signed record with its own experiment trail. A zero-lift outcome triggers a rollback without creating a billable proof.

The rollback path is deliberately loud when it cannot complete: if the platform refuses the restore call, no rollback record is written, the cycle reports the failure, and a human is expected to intervene. The ledger never says "rolled back" when it was not.

## Billing re-derives from the ledger

Invoices do not add up stored totals. Every invoice line is re-derived from the signed ledger proof before it is billed. The platform-measured lift and the agreed rate inside the 15–20% band are recomputed at invoice time, and a proof that does not re-verify stops the line. A manipulated invoice table would be refused, not honored.

## What you keep

At any point you can export:

- the ledger file, verifiable with the published procedure,
- the experiment identifiers, readable in your own platform account,
- the audit findings that motivated each change, with their evidence rows.

That package is the difference between "our tool says revenue went up" and "your platform's own experiment says revenue went up, and here is the signed history of every change that led to it."
