+++
title = "Apex pricing | Paid only from measured gains"
description = "Free connection, monitoring, audit and app-ads.txt watch. Apex keeps 10% of players on the old setting and bills 30% of the gain that holdout measures each month. Managed ad ops is the one fixed fee."
template = "page.html"

[extra]
h1 = "Pay only from measured gains."
eyebrow = "Results fee"
lede = "Connection, monitoring, the audit and app-ads.txt watch are free. The results fee is 30% of the gain measured against a 10% holdout. Apex bills 30% of the monthly gain measured against a 10% holdout, and nothing when the gain is zero or below. The calculation uses the signed holdout result and ledger references."
og_image = "og/pricing.png"

[[extra.panels]]
title = "Results fee"
lead = "30%"
lines = [
  "of the monthly gain measured against a 10% holdout.",
  "No fee in a month whose measured gain is zero or below.",
  "A change bills for at most 12 months.",
]

[[extra.panels]]
title = "Free"
lead = "$0"
lines = [
  "Connection, monitoring, the audit and app-ads.txt watch.",
  "Daily checks of revenue, eCPM, fill and ARPDAU per ad unit.",
  "Alerts on drops, failed pulls, expired keys and app-ads.txt changes.",
]

[[extra.panels]]
title = "Managed"
lead = "$2,500-$5,000 / month"
lines = [
  "Apex runs the full monetisation program with you.",
  "Managed ad ops is $2,500 per month at the base and $5,000 above $1M of ad revenue.",
]
+++

All prices and invoices are in USD.

## What the fee uses

Each kept change is rolled out to 90% of players; 10% stay on the old setting. Apex measures the gain per player over the last 28 days against that holdout. Apex bills only from a complete 28-day window; a short or partial window bills nothing. A change kept late in a month first bills in the first month that holds a full 28-day window for it. A day with no player rows counts only when the platform reports zero revenue for that day. The fee is sized from the positive 97.75% one-sided lower bound of that gain, never the point estimate, and applies the 30% rate to the exposed revenue actually earned in the billed days. A fee is priced from the two arm revenue totals of that split, including revenue whose player cannot be identified, kept with the arm that earned it. When the platform supplies an eligible-user roster and a bounded-outcome certificate they stay on the signed evidence as audit records; neither is a precondition for the fee.

A month whose lower bound is zero or below bills nothing. A change whose holdout shows harm with 90% confidence is rolled back and bills nothing more. Harm checks follow the same complete-window rule.

## How long it lasts

A kept change can bill for up to 12 months from the month it starts. If you end the change or its holdout, billing stops that day. Nothing is billed from a 14-day estimate alone.

## Worked example

Example, not a forecast.

<ul class="facts-list">
  <li>Revenue of the players with the change: $32,000 for the month.</li>
  <li>Assume the holdout's 97.75% lower bound is a 4.8% gain.</li>
  <li>The gain measured against the holdout is 4.8/104.8 × $32,000, about $1,466.</li>
  <li>The fee is 30% of that gain, about $440.</li>
  <li>The publisher keeps about $1,026.</li>
</ul>

## Your invoice

Each fee line shows the measured gain, its range, the revenue it applies to and the signed ledger records behind it. Tax and Managed appear separately. A month with no measured gain carries a "no fee this month" line.

## Monitoring and app-ads.txt watch

Both are free with every connection. Monitoring checks revenue, eCPM, fill and ARPDAU per ad unit against a weekday-adjusted 28-day baseline, and alerts on drops, failed pulls and expired keys. The app-ads.txt watch checks every app daily, including for publishers who never connect.

<a class="button" href="https://console.apexmediation.ee/watch/">Start the free watch</a>

## Begin with the free audit.

Understand the findings before connecting an account.

<a class="button" href="/free-audit/">Request the free audit</a>
