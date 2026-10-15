+++
title = "How Apex works"
description = "From the free read-only revenue-leak audit to signed change logs and automatic rollback: every step Apex takes on AppLovin MAX, LevelPlay and AdMob."
+++

Apex has exactly one job: raise the revenue your existing mediation setup earns, using the settings and the measurement tools that platform already gives you. This page describes each stage in the order it happens, and states plainly where writes do and do not happen.

## Step 1: the free read-only revenue-leak audit

The audit reads a snapshot of your live account: ad units, waterfall and bidding configuration, floors per country, network integrations, revenue reports, and your app-ads.txt file. It changes nothing. The audit never alters your platform settings, and the deployment itself holds no write credentials. You can run it before any contract exists.

Five classes of leak are checked on every pass:

- **Stale floors.** The audit flags stale floors — a country floor set above that country's recent eCPM is throttling demand.

- **Disabled bidders.** The audit finds bidders enabled at the network but disabled in your waterfall. This includes networks where you already have an account and a live integration.

- **Missing per-country floors.** The audit finds countries with real revenue and no configured floor.

- **Reporting gaps.** The audit measures the gap between network-reported revenue and mediation-reported revenue, so totals that disagree stop being a quarterly spreadsheet surprise.

- **app-ads.txt errors.** The audit validates app-ads.txt and reports malformed lines by line number, including missing expected networks and domain mismatches.

Each finding carries an estimated monthly recovery, a severity, and the evidence rows it was computed from. A finding with no honest number behind it is not reported with one: when the data cannot support an estimate, the finding says so.

## Step 2: changes go through the platform's own management API

Nothing is injected into your app and nothing is interposed in ad serving. On AppLovin MAX, Apex reads and writes through MAX's own management API. On LevelPlay, price changes go through the instances of LevelPlay's mediation management API. On AdMob, mediation group updates are read-modify-write replacements through AdMob's API.

Two hard rules govern writes:

- **Sandbox confirmation first.** Before the autopilot may write through a path, that path must be confirmed against the platform's live sandbox. Until then, the operation is read-only. The autopilot refuses to apply changes through endpoints that are not sandbox-confirmed.

- **Criticals put writes on hold.** Every write is blacked out until a critical audit result is resolved. If a stale configuration, a broken app-ads.txt or a reporting discrepancy is severe enough to be critical, the autopilot will not add more changes on top of it.

Every proposed change is deterministic: given the same snapshot, policy and ledger, Apex proposes the same change with the same idempotency key, so a retried or duplicated cycle cannot double-apply a floor or a price.

## Step 3: the experiment is the platform's own A/B test

Each gain is proven by the A/B experiment built into the platform itself. Apex does not run its own allocation, does not build its own statistics dashboard, and does not ask you to believe its own attribution. It creates an experiment in your MAX, LevelPlay or AdMob account through the same management API, with a control and a variant, and lets the platform allocate, measure and report.

While an experiment is running, the proposed change is locked: Apex will not modify it, and settlement refuses to read a running experiment as a result. The platform's finished result is the only one that counts. The keep thresholds are configured up front: a minimum lift rate and a minimum confidence. A change that clears both is kept; a change that does not is treated as a failed change, not as a partial win.

The billing basis is the platform's own measured lift, never an Apex-side number. If the platform's experiment reports zero lift or a loss, there is nothing billable.

## Step 4: the signed change log

Each change and each rollback that Apex applies is written to a signed, hash-chained ledger. Each record carries the platform, the operation, the before and after values, the experiment identity, and the measured result; each line is signed with an Ed25519 key and chained to the previous line by SHA-256 hash. Changing any byte of any line — a value, a sequence number, a signature — breaks verification, and the verifier reports the failure instead of repairing it.

The ledger is the audit trail you keep: it does not require an Apex account, an Apex server, or network access to verify. The verification procedure ships with the ledger file, and the ledger file itself is created owner-readable only.

## Step 5: automatic rollback

The apply step and the rollback step are the same rails. A change that cannot clear the keep threshold is rolled back by Apex automatically. The previous settings are restored through the same management API, and the rollback is written to the ledger as its own signed record. A change that measures zero lift is rolled back without becoming billable.

Rollback is fail-loud, not best-effort. If the platform refuses the rollback call, the cycle surfaces the failure and leaves the ledger honest: no false rollback record is written, and the condition is raised for a human. A failed change never quietly becomes a permanent one.

## What runs where

| Stage | Reads | Writes | Where the data comes from |
| --- | --- | --- | --- |
| Audit | Your platform account | Nothing | Platform reporting and configuration APIs |
| Apply | The signed ledger | Platform settings only | The platform's management API |
| Experiment | Platform experiment results | Nothing | The platform's built-in A/B tool |
| Settle and bill | Signed ledger records | Nothing | The platform's own measured lift |

Nothing runs inside your app at any stage, and no ad request passes through Apex.
