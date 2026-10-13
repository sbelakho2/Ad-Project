+++
title = "Security and the boundaries we keep"
description = "No SDK, no ad serving, nothing flows through Apex. Read-only audits, sandbox-confirmed writes, encrypted credentials and an owner-only signed ledger."
+++

Most monetization vendors ask for a piece of your ad path. Apex asks for none of it. This page states the boundaries exactly, because they are the product.

## What Apex is not

- **Never in the ad path.** Apex never serves, brokers or routes an ad. It does not sit between your app and an ad network, and it holds no traffic.

- **No installable SDK.** Nothing gets embedded into your app. There is no binary to ship, no release to coordinate, and no Apex code inside your request path.

- **No mediation swap.** You keep running MAX, LevelPlay or AdMob exactly as you do today. Apex works on top of the mediation you already run.

- **No partnerships needed.** Apex does not require special deals with your networks, and it does not need to be an approved intermediary anywhere. It uses the same management APIs you have access to.

- **Nothing flows through Apex.** Ad requests, bid responses, impressions and user data never transit Apex infrastructure. Only configuration reads and writes and platform-reported results touch our systems.

## Read-only by default

The audit runs read-only: it never mutates your platform settings, and the deployment holds no write credentials. That is a property of the deployment, not a promise in a policy document: the audit deployment physically holds no write path. You can run the entire audit, receive findings, and walk away.

## Writes are earned, then gated

When you choose to apply changes, two gates stand in front of every write:

1. **Sandbox confirmation.** A write path must be confirmed against the platform's live sandbox before the autopilot may write through it. The autopilot refuses to apply changes through endpoints that are not sandbox-confirmed; unconfirmed paths stay read-only even when they look useful.

2. **Audit blackout.** A critical audit result blackouts every write until it is resolved. If something is badly wrong in the account, Apex stops touching it and says so.

Every write is idempotent, attributed and reversible: the same proposal carries the same idempotency key across retries, so network flakiness cannot produce duplicate floor changes, and every write has a defined rollback that restores the prior values.

## Credential handling

- Publisher network credentials are stored encrypted with AES-256-GCM. Plaintext credentials exist only in memory for the duration of an API call and are never written to logs.

- Credential material is versioned, so rotation replaces a secret rather than duplicating it, and a failed rotation leaves the prior version intact.

- The console stores publisher keys in an encrypted vault service; access to decrypted values is audited.

## The ledger file

The signed change log is a local, append-only file. It is created with owner-only permissions (mode `0600` on Unix), and every append re-verifies the file tail before writing, so a tampered history is refused rather than extended. Every line is Ed25519-signed and SHA-256-chained; verification is byte-exact and can be run offline, without Apex servers or accounts.

## Data we touch, and what we do with it

- Configuration: ad units, waterfalls, floors, mediation mappings, app-ads.txt. Read to find leaks; written only after sandbox confirmation and with your consent.

- Reporting: platform and network revenue totals, used to compute findings and to compare network-reported against mediation-reported revenue.

- Results: the platform's finished experiment outcomes, used for keep/rollback decisions and billing.

We do not need user-level advertising identifiers to do any of this, and we do not query them. If a platform requires an API surface we cannot use without unnecessary data, that surface stays unused.

Security contact: [hello@apexmediation.example](mailto:hello@apexmediation.example).
