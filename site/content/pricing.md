+++
title = "Apex pricing | Fees tied to qualifying gains"
description = "Free read-only audit. Lift fees of 15% to 20%, with a clear term and calculation. Optional monitoring is $500 per month."
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
title = "Optional monitoring"
lead = "$500 / month"
lines = [
  "Daily measurement, guardrail monitoring and customer-impact alerts.",
  "A separate plan, billed month to month.",
]
+++

## What the fee uses

A positive point estimate alone is not enough. A kept change must pass the registered decision rule. The fee uses the conservative positive lift bound recorded with that result. You can inspect the inputs and reproduce the calculation.

## How long it lasts

A qualifying kept change can be billed for up to 12 months from its keep date. If you override the setting, billing for that change stops and the active period is prorated. A qualifying recheck can reduce the billed lift; it cannot raise it. Your order describes the applicable recheck schedule.

## Optional monitoring

$500 per month, optional.

Daily measurement passes, guardrail monitoring and customer-impact alerts. This is a separate monthly plan and can carry a fee even when there is no billable lift. It is never added automatically.

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

## Begin with the free audit.

Understand the findings before choosing a paid service.

<a class="button" href="/free-audit/">Request the free audit</a>
