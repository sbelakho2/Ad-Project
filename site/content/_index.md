+++
title = "No SDK in your app. Proven in your dashboard. You pay only on lift."
description = "Apex audits your ad mediation for free, applies changes through MAX, LevelPlay or AdMob's own management API, and bills a share of the lift the platform's own A/B test measured."
template = "index.html"
+++

No SDK in your app. Works on top of the mediation you already run. Every change proved in your dashboard's own A/B test. You pay only on measured lift.

Apex is the autopilot that sits on top of AppLovin MAX, LevelPlay and AdMob. Nothing is added to your build, no binary changes, and no new request path for your ad traffic. Your apps keep serving ads exactly the way they do today; Apex reads your account, finds monetization leaks, and applies fixes through the platform's own management API.

## The loop, in four steps

1. **Audit.** Apex reads your live platform snapshot and reports revenue leaks the platform's own dashboards leave buried: stale floors, disabled bidders, missing per-country floors, reporting gaps and app-ads.txt errors. The audit is free and read-only.
2. **Apply.** With your approval and your credentials, Apex changes the settings through each platform's own management API: floors in MAX, price entries in LevelPlay, mediation mappings in AdMob.
3. **Prove.** Every change runs as the platform's own built-in A/B experiment, on your app, inside your account. Every gain is proven by the platform's own built-in A/B experiment.
4. **Settle.** A change that clears the keep threshold stays and becomes billable. A change that does not is rolled back automatically. If the platform's own experiment shows no lift, the change is rolled back and no fee applies.

## What the free audit looks for

- **Stale floors.** The audit flags stale floors — a country floor set above that country's recent eCPM is throttling demand.
- **Disabled bidders.** The audit finds bidders enabled at the network but disabled in your waterfall. Demand you already pay for is never asked for a bid.
- **Missing floors.** The audit finds countries with real revenue and no configured floor.
- **Reporting gaps.** The audit measures the gap between network-reported revenue and mediation-reported revenue.
- **app-ads.txt.** The audit validates app-ads.txt and reports malformed lines by line number.

[See the full audit methodology](/how-it-works/).

## The proof is yours, not ours

Apex never asks you to trust an Apex dashboard. The measurement is the experiment tool you already use inside MAX, LevelPlay or AdMob, and the result that matters is the one it reports. Every applied change and rollback is appended to a signed, hash-chained ledger, so the history of what was done, when, and why can be re-verified later without our servers. [How the proof works](/proof/).

## Pricing that only moves when your revenue does

Our fee is 15% to 20% of the lift the platform itself measured. If the platform's own experiment shows no lift, the change is rolled back and no fee applies. [The worked example and the full band](/pricing/).

## Who this is for

Apps earning roughly $10k to $300k per month on MAX, LevelPlay or AdMob, run by teams without a full-time ad-ops person. If no one has time to review country floors weekly, test waterfall entries, or reconcile network reports against mediation reports, Apex does that work continuously and bills only when the change it made shows up in your numbers. [Start with the publisher guide](/for-publishers/).

## What we do not do

No ad serving. No SDK. No new mediation. No partnerships required. Nothing flows through Apex — requests, responses and bids all travel the same paths they travel today.
