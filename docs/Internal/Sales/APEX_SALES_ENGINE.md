# Apex sales engine (apex-sales)

Owner-only. This is the internal tool that finds mobile app publishers and
emails them about ApexMediation's console autopilot. Customers never see it,
and it is never offered or sold. Code: `rust/crates/apex-sales`. Units:
`infrastructure/systemd/apex-sales-*.{service,timer}`.

It replaces the TypeScript prospecting engine that used to live under
`control-plane/src/services/discovery/`. The other documents in this folder
describe that older system and its Test Server deployment; treat them as
history, and this page as the current runbook.

## What it does

1. Leads come from three places: a seed list you import
   (`import-seeds`), the free-audit requests apex-api writes to its leads
   file (`import-leads`, run before every send pass), and contacts found in
   a publisher's own `app-ads.txt`.
2. `discover` fetches each publisher's public `app-ads.txt`
   and App Store listing. Every claim in an email cites one of these
   observations; a contact with nothing true to say about them is not
   emailed.
3. Every hour, `run` scores every eligible contact, picks the
   message, the step of the sequence and the local send hour, and hands
   the message to the mail transport. Replies and bounces are fed back with
   `ingest`; outcomes further down the funnel are recorded with `record`.

A publisher who requested the audit, replied, or reached any later stage is
never cold-emailed again. One contact per publisher is active at a time.

## The models

All parameters are posteriors with documented priors, so the engine works
on day one and sharpens as outcomes arrive.

- Lead score: Bayesian logistic regression over 15 features (size
  band, genre, missing or broken `app-ads.txt`, inferred mediation
  platform, named versus generic mailbox, inbound interest). Updates use
  the Laplace approximation of Chapelle and Li (2011). Predictions are
  probit-corrected, and every score carries exact log-odds reason codes.
- Value of a reply: Beta–Binomial funnel (interest, audit request,
  connected platform, paid) with empirical-Bayes pooling across size bands.
  It also credits a message for a contact's later progress, weighted by
  the probability that the contact eventually pays.
- Ranking: expected value on a Thompson draw:
  `p̃ · s_k · V − cost − c_k · C`, where `s_k` is the share of interest that
  step `k` brings and `c_k` its complaint rate priced at `C`.
- Message choice: Thompson sampling on revenue value with top-two
  sampling (Russo, 2016) and daily discounting. A variant is excluded once
  the posterior probability that its complaint rate exceeds 0.1% reaches
  95%.
- Timing and stopping: pooled Beta posteriors per local hour; per-step
  hazards decide whether a follow-up is still worth sending.
- Domain health: Bayesian rate tests on complaints and bounces plus a
  Bernoulli cumulative-sum test (Page, 1954). The daily cap warms up from
  20 to 150 sends and backs off on warnings; a breach quarantines the
  domain until you run `reset-health`.
- Measurement: a randomized 10% of publishers is never emailed; the
  report shows the paid-rate difference against that holdout with a
  Newcombe interval. Calibration is reported as Brier score, log loss and
  reliability bins.
- Hygiene: Fellegi–Sunter record linkage with Jaro–Winkler merges
  duplicate publishers. Replies are classified by delivery-report headers
  (RFC 3464 and 3834), opt-out phrases and a naive Bayes model.
- Mediation inference: naive Bayes over the `DIRECT` lines of
  `app-ads.txt`, learned from the platforms inbound leads declare.

Hard rules are never traded against expected value. Suppression, the
country table, account coordination, the holdout, the domain breaker, the
daily cap and evidence grounding all sit in one decision gate.

## Simulation

`apex-sales simulate` runs the production engine and a naive baseline
(random order, fixed templates, every follow-up, same cap and gates) on the
same synthetic market, whose true parameters differ from the priors.

| Publishers, days, seed | Engine value per send | Baseline value per send | Total value ratio |
|---|---|---|---|
| 1200, 50, 1 | $4.18 | $3.67 | 1.27 |
| 1200, 50, 2 | $6.45 | $4.04 | 1.72 |
| 6000, 60, 11 | $11.27 | $3.93 | 3.78 |
| 6000, 60, 12 | $11.28 | $3.80 | 5.63 |

The gain is largest when the list is bigger than daily capacity, because
ranking then decides who gets the sends. The baseline also loses sending
days to complaint throttling. These are synthetic markets for checking the
mechanics; real rates come from the report once outcomes exist.

## Running it

Install the binary and the four units as shown in
`infrastructure/README.md`, then write `/etc/apex/sales.env`. The engine
starts in `shadow` mode: it plans and writes every message it would send to
`.apex/sales/outbox` as `.eml` files, and sends nothing.

1. Import seeds: `apex-sales import-seeds seeds.csv` with the header
   `domain,name,country,app_store_ids,email,contact_name,role`.
2. Run `apex-sales discover`, then `apex-sales plan`. Every line shows the
   verdict, the expected value and each gate reason.
3. Read a day of previews in the outbox. When they are right, set
   `APEX_SALES_MODE=approval` and approve touches with `apex-sales approve`,
   or set `autonomous` to let the gate decide.
4. Pipe every reply and bounce into `apex-sales ingest message.eml`, and
   record later stages: `apex-sales record studio.example connected`.
5. Read `apex-sales report` weekly. It shows domain health, feature
   weights with intervals, message posteriors, funnel rates, calibration
   and the holdout effect.

Sending needs `APEX_SALES_FROM`, `APEX_SALES_FROM_NAME`,
`APEX_SALES_COMPANY` and `APEX_SALES_ADDRESS`. The transport is
`APEX_SALES_SENDMAIL`; the units forbid privilege escalation, so use an
unprivileged client such as `msmtp` relaying through the authenticated
outbound host, with `SPF`, `DKIM` and `DMARC` aligned for the sending
domain. Discovery needs `curl` on the host.

## Limits

- The country table is a conservative default (Germany needs prior consent;
  business addresses elsewhere are opt-out with identification), and it is
  not legal advice. Review it, and override it with `APEX_SALES_LEGAL_FILE`.
- One-click unsubscribe needs an endpoint that maps the token back to an
  address and calls `apex-sales suppress`. Until one exists, leave
  `APEX_SALES_UNSUBSCRIBE_URL` unset; the message then carries a mailto
  unsubscribe, and opt-out replies are suppressed on `ingest`.
- Replies reach the engine only through `ingest`. Wire the reply mailbox to
  it, or run it by hand.
