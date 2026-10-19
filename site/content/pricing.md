+++
title = "Apex pricing | Fees tied to qualifying gains"
description = "Free read-only audit. Lift fees of 15% to 20%, with a clear term and calculation. Monitoring is included from $500 a month, cancel any time."
template = "page.html"

[extra]
h1 = "A share of qualifying gains."
eyebrow = "Lift fee"
lede = "The audit is free. If you continue, your agreed lift-fee rate is 15% to 20% of qualifying incremental revenue. The exact rate appears in your order."
og_image = "og/pricing.png"

[[extra.panels]]
title = "Lift fee"
lead = "15-20%"
lines = [
  "of qualifying incremental revenue.",
  "Up to 12 months for a kept change.",
  "No qualifying lift means no lift fee.",
]

[[extra.panels]]
title = "Monitoring"
lead = "$500-$3,000 / month"
lines = [
  "Included at connection, cancel any time.",
  "Daily measurement, guardrail monitoring and customer-impact alerts.",
  "Tiers by reported 30-day revenue: $500 under $100k, $1,500 up to $500k, $3,000 above.",
]

[[extra.panels]]
title = "Setup Watch"
lead = "$99-$199 / month"
lines = [
  "Daily app-ads.txt checks and integrity alerts.",
  "Standard $99, core $199 per month.",
  "Set up at console.apexmediation.ee/watch/.",
]
+++

## What the fee uses

A positive point estimate alone is not enough. A kept change must pass the registered decision rule. The fee uses the conservative positive lift bound recorded with that result. You can inspect the inputs and reproduce the calculation.

## How long it lasts

A qualifying kept change can be billed for up to 12 months from its keep date. If you override the setting, billing for that change stops and the active period is prorated. A qualifying recheck can reduce the billed lift; it cannot raise it. Your order describes the applicable recheck schedule.

## Monitoring

Monitoring is included, cancel any time.

It switches on at connection and covers daily measurement passes, guardrail monitoring and customer-impact alerts. The price follows your reported 30-day revenue:

<ul class="facts-list">
  <li>$500 per month under $100k of monthly revenue.</li>
  <li>$1,500 per month from $100k up to $500k.</li>
  <li>$3,000 per month above $500k.</li>
</ul>

A monitoring line can carry a fee even when there is no billable lift. Cancel any time; the plan bills the current month only and stops after it.

## Worked example

Example, not a forecast.

<ul class="facts-list">
  <li>Eligible revenue for the month: $51,500.</li>
  <li>One kept change, billable lift bound: 3%.</li>
  <li>Attributed incremental revenue: $1,500.</li>
  <li>Lift fee at 20%: $300.</li>
  <li>Lift fee at 15%: $225.</li>
</ul>

For one active change, attributed increment is monthly scope revenue multiplied by 1 - 1 / (1 + billable lift). Multiple active changes are combined before their contribution is allocated.

## Your invoice

Each fee line identifies the change, billing period, eligible revenue, billable bound, agreed rate and proof references. Tax, monitoring and credits appear separately. Nothing is billed as a lift fee for a test that was not kept with qualifying evidence.

## Setup Watch

The paid Setup Watch runs a daily check of the public app-ads.txt files your
accounts rely on and alerts you when an authorised seller, account or
relationship changes. It is separate from the lift fee and can be started in
a few minutes.

<a class="button" href="https://console.apexmediation.ee/watch/">Set up Watch</a>

## Begin with the free audit.

Understand the findings before choosing a paid service.

<a class="button" href="/free-audit/">Request the free audit</a>
