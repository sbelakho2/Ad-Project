# Apex sales engine (apex-sales)

Owner-only. This is the internal tool that finds mobile app publishers and
emails them about ApexMediation's console autopilot. Customers never see it,
and it is never offered or sold. Code: `rust/crates/apex-sales`. Units:
`infrastructure/systemd/apex-sales-*.{service,timer}`. The owner-only web
workspace is a second crate, `rust/crates/apex-sales-web`, unit
`infrastructure/systemd/apex-sales-web.service`; the Workspace UI section
below is its runbook.

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
  Bernoulli cumulative-sum test (Page, 1954). Each sending domain warms up
  from 20 to 150 sends with its own cap and its own breakers; a breach
  pauses or quarantines only that domain, and the full-volume section below
  covers running several domains at once.
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
4. Pipe every reply and bounce into `apex-sales ingest message.eml` (or
   `ingest --stdin` from a pipe, or `ingest --maildir <dir>` for a whole
   directory), and record later stages: `apex-sales record studio.example
   connected`. The hourly `run` imports the outcomes file that `apex-api`
   appends whenever it exists, so connections, invoices and watch signups
   need no second timer.
5. Read `apex-sales report` weekly. It shows domain health, feature
   weights with intervals, message posteriors, funnel rates, calibration
   and the holdout effect.

Sending needs `APEX_SALES_FROM`, `APEX_SALES_FROM_NAME`,
`APEX_SALES_COMPANY` and `APEX_SALES_ADDRESS`. The transport is
`APEX_SALES_SENDMAIL`; the units forbid privilege escalation, so use an
unprivileged client such as `msmtp` relaying through the authenticated
outbound host, with `SPF`, `DKIM` and `DMARC` aligned for the sending
domain. Discovery needs `curl` on the host.

## Full volume

At full volume several sending domains carry the day's plan, the second
offer rides the same bandit, and the outcomes and replies flow in without an
operator. The knobs and the data flow:

### Several sending domains

Every sending domain has its own sender identity, its own daily cap and its
own health. Configure them with `APEX_SALES_SENDERS_FILE`, a CSV with a
header:

```csv
email,name,company,postal_address,unsubscribe_url
aaron@apexmediation.ee,Aaron at Apex,Apex OÜ,"Kadaka tee 1, Tallinn, Estonia",https://apexmediation.ee/u/
dana@apexmediation.com,Dana at Apex,Apex OÜ,"Kadaka tee 1, Tallinn, Estonia",
```

or inline with `APEX_SALES_SENDERS`: entries separated by `;`, fields by
`|`, in the same column order (`email|name|company|postal_address|unsubscribe_url`).
Domains must be unique, and a row with an invalid email refuses the whole
file. Without either variable the single `APEX_SALES_FROM` identity is used
and behaviour is exactly what it was.

- Each domain warms up on its own from 20 to `APEX_SALES_DAILY_MAX`
  (default 150) sends. The engine's daily cap is the sum of the per-domain
  caps, so two warming domains may send 40 on their first day, not 20.
- The day's plan keeps its expected-value ranking and is spread round-robin
  over the domains that may send, in the configured order. The best planned
  touch goes to the first domain with capacity left, the next to the next
  domain, and so on. A paused or quarantined domain is skipped, so its share
  moves to the healthy domains. With one domain the rule reduces to the old
  one: best ranked touches up to the cap.
- Complaints and bounces pause or quarantine only the domain that sent the
  message. The defaults are the complaint ceiling of 0.1% and the hard
  bounce ceiling of 3% per domain, and attribution runs through the touch
  that carried the message (the contact's latest touch, or the publisher's).
  Each row of the `/domains` table is one domain's own history, and one
  domain's breach never halts the others.
- `reset-health <domain>` resets one domain after an investigation;
  `reset-health` without a domain resets every domain.

### The second offer: Setup Watch

The message bandit carries a fourth first-touch variant, `t0-watch`: the
Setup Watch offer, the daily measurement pass with guardrail monitoring and
alerts as a flat monthly plan. Set `APEX_SALES_WATCH_CHECKOUT_URL` to the
HTTPS checkout base; the engine appends the publisher key to it, the same
way the contact's token is appended to `APEX_SALES_UNSUBSCRIBE_URL`.
Without the variable the variant is not enumerated at all and the bandit
never serves it (`apex-sales report` marks the arm unavailable). The
variant learns like every other arm: replies and outcomes move its
posterior exactly as the other templates'.

The price copy lives in one constant, `WATCH_PRICE_TEXT` in
`rust/crates/apex-sales/src/compose.rs`, and it must match billing's Setup
Watch plan, the $99 one-platform and $199 three-platform monthly lines that
`apex_billing::subscription::watch_plan` owns. The engine never invents a
price: no template text carries a dollar amount, only the placeholder that
the constant fills.

### Outcomes import

`apex-api` appends one JSON line per post-sale event to the outcomes file
(`APEX_SALES_OUTCOMES_FILE`, default `.apex/sales/outcomes.jsonl`):

```json
{"atUnixMs":1792400000000,"publisher":"studio.example","kind":"connected","ref":"evt_123"}
```

`kind` is `connected`, `paid` or `watch_signup`; `ref` is the appender's
own id. `apex-sales import-outcomes [file]` records every line through the
same outcome semantics as `record`: the line's timestamp, the publisher as
the target, the `ref` as the note. Lines are deduplicated by (publisher,
kind, ref) against the store, so replaying the append-only file is
idempotent. Malformed lines are skipped with a count instead of aborting the
file. `--dry-run` prints the counts and writes nothing. The hourly `run`
imports the file first when it exists, so connections, invoices and watch
signups reach the learner without a second timer. A `watch_signup` moves the
publisher to the connected rung of the funnel ladder; the event log keeps
the distinct kind.

### Reply intake

Replies and bounces reach the engine through `ingest`, in three shapes:

- one message from a pipe: `... | /usr/local/bin/apex-sales ingest --stdin`
- one saved file: `apex-sales ingest message.eml`
- a directory the mail server writes into: `apex-sales ingest --maildir /var/lib/apex/replies`

For the mail-server pipeline, deliver the reply mailbox as a directory of
`.eml` files and scan it on a schedule. Fetchmail, getmail or the MTA's own
aliasing can write a Maildir, and `ingest --maildir` reads every `*.eml`
file newest first with the single-file semantics per message. A durable seen
set (`ingest-seen.jsonl` in the state directory) keys each file by name,
size and modification time, so a file is processed exactly once even across
restarts. Unreadable files are skipped with a counted reason and stay
unseen, so a transient failure is retried on the next run. A message from an
unknown sender is classified and counted, not recorded: record it by hand
with `apex-sales record`.

An example pipeline beside the hourly unit:

```text
*/10 * * * * /usr/bin/getmail --rcfile /etc/apex/getmailrc && /usr/local/bin/apex-sales ingest --maildir /var/lib/apex/replies >> /var/log/apex/sales-ingest.log 2>&1
```

## Workspace UI

The owner-only workspace is `apex-sales-web` (code:
`rust/crates/apex-sales-web`; unit:
`infrastructure/systemd/apex-sales-web.service`). It serves
`http://127.0.0.1:8795` by default and is never exposed to customers. The
pages render the engine's own state with the engine's own configuration. The
UI initiates no sends; it mirrors and gates what the runner would do.

| Route | Page |
|---|---|
| `GET /` | Overview: funnel KPIs, the daily send chart and the next steps |
| `GET /leads` | Leads with filters; `GET /leads/export.csv` is the same filtered view as a CSV attachment |
| `GET /publishers/{domain}` | One publisher: contacts, evidence, event timeline and the suppress/record forms |
| `GET /plan` | The engine's plan for today, every gate reason, and the approve forms |
| `GET /sends` | Send history and the outbox listing; `GET /sends/preview/{file}` renders one `.eml` preview |
| `GET /replies` | Inbound replies with their classification and pre-selected actions |
| `GET /domains` | Domain health, warm-up, rate tests and the reset-health confirmation |
| `GET /models` | Posteriors, calibration and arm performance |
| `GET /assets/apex.css` | The one stylesheet, described below |
| `GET /assets/fonts/{name}` | The self-hosted WOFF2 faces; unknown names are a plain 404 |
| `GET /favicon.ico`, `GET /favicon.svg` | The committed icons |
| `GET /login` | The sign-in form; `POST /login` verifies the owner token |
| `POST /logout` | Closes the session after the CSRF check |
| `POST /actions/{kind}` | `approve`, `suppress`, `record` and `reset-health`; `GET /actions/result/{id}` is the stable result page |
| `GET /healthz` | A JSON liveness answer |

### Owner gate, sessions and CSRF

`APEX_SALES_WEB_TOKEN` is the owner token and is required. With it unset the
service starts in its fail-closed not-configured state: every page explains
the requirement (HTTP 503) and no session can be minted.
`APEX_SALES_WEB_BIND` selects the listen address (default
`127.0.0.1:8795`); keep it loopback-only. `APEX_SALES_DIR` (default
`.apex/sales`) names the store every page reads.

A session opens only after the token verifies in constant time. The
`apex_sales_session` cookie is opaque, host-only, `HttpOnly`, `SameSite=Lax`
and scoped to `/`, and it gains `Secure` when `X-Forwarded-Proto: https`
arrives. A session lives 8 hours absolutely and expires after 60 idle
minutes, and the store is in memory only, so a restart signs every browser
out. Each mutating form carries the per-session CSRF token; a mismatch
answers 400 and writes nothing. Each mutation also carries a one-time
idempotency key: a replayed form answers the remembered result page, and a
reused key with a different payload answers 409.

### The look: one stylesheet, self-hosted fonts

Every page links exactly one stylesheet, `GET /assets/apex.css`, which
answers the design system's own `console_css` bytes (`apex-ui` tokens). Fonts
are the committed WOFF2 faces served with an immutable one-year cache. Pages
carry no inline styles and no scripts; the render tests assert the single
link and the absence of style blocks.

### Modes on `/plan`

`/plan` renders the same plan the CLI prints: ranked touches with expected
value and send hour, the denied targets with every gate reason, and the
recent decisions log. The banner states the active mode:

- `shadow`: nothing is sent; the runner turns the gate's touches into `.eml`
  previews in the outbox.
- `approval`: a touch with the approval verdict waits for the owner.
  Approving writes the same durable approval the CLI's `approve` writes, and
  the runner then sends on its schedule.
- `autonomous`: the gate decides and the runner sends on its schedule.
- `off`: the kill switch; nothing is planned or sent through the web surface
  and inbound processing continues in the CLI.

### Safety posture

The four mutations are gated and mirror their CLI equivalents exactly:
`approve`, `suppress` (with the same implied outcome events), `record`, and
`reset-health` behind its exact confirmation phrase. Nothing in the UI
initiates a send: the runner remains the only sender, on its timer, reading
the same store. While signed out every protected page answers 401 with the
sign-in surface, and unknown paths or hostile domains are a neutral 404 that
reflects nothing back. Metrics carry no public route: with
`APEX_METRICS_TEXTFILE` set the bounded route-template series are written
atomically every 15 seconds and once more on shutdown.

## Limits

- The country table is a conservative default (Germany needs prior consent;
  business addresses elsewhere are opt-out with identification), and it is
  not legal advice. Review it, and override it with `APEX_SALES_LEGAL_FILE`.
- One-click unsubscribe needs an endpoint that maps the token back to an
  address and calls `apex-sales suppress`. Until one exists, leave
  `APEX_SALES_UNSUBSCRIBE_URL` unset; the message then carries a mailto
  unsubscribe, and opt-out replies are suppressed on `ingest`.
- Replies reach the engine only through `ingest` (one message or a whole
  Maildir). Wire the reply mailbox to it as the full-volume section shows,
  or run it by hand.
- The outcomes file is one-way: the engine imports it and never writes to
  it. The appender owns the schema, and the engine only reads the three
  kinds it documents; an unknown kind is skipped and counted.
