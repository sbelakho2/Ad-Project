+++
title = "No SDK in your app. Proven in your dashboard. You pay only on lift."
description = "Apex audits your mediation free, applies changes through MAX, LevelPlay or AdMob, and bills only platform-proven lift."
template = "index.html"
+++

No SDK in your app. Works on top of the mediation you already run. Every change proved in your dashboard's own A/B test. You pay only on measured lift.

Apex is the autopilot that sits on top of AppLovin MAX, LevelPlay and AdMob. Nothing is added to your build, no binary changes, and no new request path for your ad traffic. Your apps keep serving ads exactly the way they do today; Apex reads your account, finds monetization leaks, and applies corrections through the platform's own management API.

## The loop, in four steps

1. **Audit.** Apex reads your live platform snapshot and reports revenue leaks the platform's own dashboards leave buried: stale floors, disabled bidders, missing per-country floors, reporting gaps and app-ads.txt errors. The audit is free and read-only.

2. **Apply changes.** With your approval and your credentials, Apex changes the settings through each platform's own management API: floors in MAX, price entries in LevelPlay, mediation mappings in AdMob.

3. **Prove.** Every change runs as the platform's own built-in A/B experiment, on your app, inside your account. The platform's built-in A/B experiment is what proves each gain.

4. **Settle.** A change that clears the keep threshold stays and becomes billable. A change that does not is rolled back automatically. If the platform's own experiment shows no lift, the change is rolled back and no fee applies.


## What the free audit looks for



- **Stale country floors.** A floor set above a country's recent eCPM is throttling demand, so the audit flags it as stale.

- **Bidders disabled in your waterfall.** When a bidder is enabled at the network but disabled in your waterfall, the audit reports it. Demand you already pay for is never asked for a bid.

- **Missing floors.** Where a country earns real revenue with no floor configured, the audit reports it.

- **Revenue reporting gaps.** The audit measures the gap between network-reported revenue and mediation-reported revenue.

- **app-ads.txt.** The audit validates app-ads.txt and reports malformed lines by line number.

[See the full audit methodology](/how-it-works/).

## The proof is yours, not ours

Apex never asks you to trust an Apex dashboard. The measurement is the experiment tool you already use inside MAX, LevelPlay or AdMob, and the result that matters is the one it reports. Every applied change and rollback is appended to a signed, hash-chained ledger, so the history of what was done, when, and why can be re-verified later without our servers. [How the proof works](/proof/).

## Pricing that only moves when your revenue does

The fee is 15% to 20% of whatever lift the platform measures. When the platform's experiment reports no lift, the change is rolled back and nothing is billed. [The worked example and the full band](/pricing/).

## Who Apex fits

Apps earning roughly $10k to $300k per month on MAX, LevelPlay or AdMob, run by teams without a full-time ad-ops person. If no one has time to review country floors weekly, test waterfall entries, or reconcile network reports against mediation reports, Apex does that work continuously and bills only when the change it made shows up in your numbers. [Start with the publisher guide](/for-publishers/).

## What we do not do

No ad serving. No SDK. No new mediation. No partnerships required. Nothing flows through Apex — requests, responses and bids all travel the same paths they travel today.
