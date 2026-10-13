+++
title = "Pricing: a share of proven lift, nothing else"
description = "Apex charges 15% to 20% of the lift the platform's own A/B test measured. No measured lift, no fee. Worked example, comparison, and who this is for."
+++

## The rule

Our fee is 15% to 20% of the lift the platform itself measured. The rate is agreed in a fixed range when you sign, and the exact percentage inside that range is stated in your order. If the platform's own experiment shows no lift, the change is rolled back and no fee applies.

No lift, no fee is not a slogan; it is enforced by the billing path. Every invoice line is re-derived from the signed ledger proof before it is billed. The billed amount is recomputed from the platform-measured lift and the agreed rate, and an invoice that cannot re-derive every line from signed proof is refused rather than estimated. A zero-lift outcome triggers a rollback without creating a billable proof, so there is nothing to invoice in the first place.

There is no subscription, no seat fee, no platform fee and no minimum commitment. The free read-only audit carries no obligation to continue.

## Worked example

An app earning **$50,000 per month**. After Apex applies a change through the platform's own management API and the platform's own A/B experiment finishes, the variant shows a **+5% lift**:

| Line | Amount |
| --- | --- |
| Measured lift, as reported by the platform | +5% |
| Extra monthly revenue | +$2,500 |
| Apex fee at the 20% end of the band | $500 |
| Apex fee at the 15% end of the band | $375 |
| Fee if the experiment shows 0% | $0, change rolled back |

That is the whole model. The number that decides the fee is not ours, it is the one printed by the experiment tool inside your MAX, LevelPlay or AdMob account. If the experiment result is 2%, you pay 15–20% of 2% of your baseline; if it is below your keep threshold, we roll the change back and bill nothing.

Lift is measured on your app's actual traffic during the experiment window, so seasonal traffic, new user mixes and general market movement are already inside the control group. The experiment's control arm pays the same traffic, at the same time, on the same platform — that is what makes the percent trustworthy.

## Who pricing fits

Apex is built for apps earning roughly **$10k to $300k per month** on MAX, LevelPlay or AdMob, run without a full-time ad-ops person. In that range, a single-digit revenue improvement is worth real money every month, but it is not worth a full-time hire or a six-figure agency retainer. The thresholds for keeping a change are yours to configure; a conservative publisher can demand a higher measured lift before anything becomes billable.

## How this compares

| Approach | What it is | Weakness for a $10k–$300k/month app |
| --- | --- | --- |
| Built-in platform tools | The dashboards, floors and experiment tools you already have in MAX, LevelPlay or AdMob | Manual and per-platform: someone still has to watch floors, design experiments, read results and roll back failures; nothing audits across your whole account continuously |
| AI ad-ops services | External services that analyze or manage your monetization | Many require an SDK or a mediation change; gains are typically reported in the vendor's own dashboard rather than the platform's built-in experiment |
| SDK-based layers | Monetization code installed into the app | Require an app release, a new binary and a rebuild cycle; the layer sits in the ad request path, so its bugs are your revenue risk |
| Consultants and agencies | People who log into your dashboards and make changes | Retainers are paid whether or not revenue moves; changes are manual, infrequent and rarely attached to a per-change experiment with documented rollback |

Apex's difference is structural: no code in the app, no new ad path, no Apex-side measurement. The work happens through the platform's own management API, the proof happens in the platform's own experiment, and the fee happens only on the result.

## What we do not charge for

- The read-only audit: free, before any contract, no obligation.
- Rollbacks: a failed change costs you nothing and is never billed.
- Access to the change log: the signed ledger is yours to verify at any time, at no cost.

## What we do NOT do

No ad serving. No SDK. No new mediation layer. No partnerships needed with your networks. Nothing flows through Apex — not ad requests, not bids, not user data. Apex touches only the management APIs of the platform you already use, and the fee only exists when those platforms' own numbers say your revenue went up.

Questions about the band, the thresholds or the worked example: write to [hello@apexmediation.example](mailto:hello@apexmediation.example).
