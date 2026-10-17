# Setup Watch Stream

The setup watch is the paid daily watch over one publisher's public
`app-ads.txt`. A subscriber signs up with a domain and an email, the daily
job fetches the file over HTTPS, runs the `apex-audit` app-ads.txt rule over
the bytes and emails one alert when new or worse problems appear. A monthly
job issues the subscription line through the same invoice register the
runner uses.

The service lives in `rust/crates/apex-watch`; the public sign-up page and
the outcomes appender live in `rust/crates/apex-api`; the units live in
`infrastructure/systemd/apex-watch*.service` and `*.timer`.

## What the service is

The watch checks one thing: does the app-ads.txt a demand source reads still
list what the publisher needs to be paid for. The check is deliberately
shallow and public. It reads one public file, holds no platform credentials
and starts no runner, so a compromise of the watch host cannot reach any ad
platform account.

## Sign-up

`GET /watch/` renders the public sign-up page: domain, email, an optional
per-network account-id textarea and the core-portfolio checkbox. The page
needs no session; it is a marketing form in the same shell as the free
audit.

`POST /watch` validates the domain and email shapes, parses the account-id
lines (`network = account id` or `network, account id` per line) and stores
the subscriber through `SubscriberStore::subscribe`. A validation failure
re-renders the page with the exact copy at each field and the safe values
preserved. Repeat submissions with the same domain and email inside 24
hours are the same subscription and return the same confirmation id. A
storage failure re-renders the page with the submission-failure notice and
the retry action.

Success is a `303` to `/watch/requested`, which shows the checkout link when
`APEX_WATCH_CHECKOUT_URL` is configured and the honest "we will email a
payment link" line otherwise. The sign-up also appends the frozen
`watch_signup` outcome line (see the sales runbook for the outcomes
contract); that append is best effort and never blocks the confirmation.

## The daily check

`apex-watch daily` walks the active subscribers. For each one the job:

1. resolves the watched domain and refuses non-public answers before any
   request;
2. fetches `https://<domain>/app-ads.txt`, falling back to plain HTTP only
   when the HTTPS attempt fails at the transport level;
3. runs the `apex-audit` app-ads.txt rule over the response bytes with the
   `billing` platform label and no expected-network list;
4. diffs the findings against the recorded findings of the previous check;
5. sends exactly one alert when new, worsened or changed findings appear.

The response cap is 1 MiB, the fetch timeout is 10 seconds and redirects are
followed by hand: every hop host is re-resolved and re-guarded, at most five
hops, and a hop that would leave HTTPS is refused. A `404` or `410` records
the file as absent (a critical finding). Every other failure records the
finding kind `unreachable` with the typed reason; the check never panics.

### The resolve-time guard

Before any fetch the domain is resolved through `std::net` and every answer
must be public. Private, loopback, link-local, shared, multicast,
unspecified, broadcast and reserved answers are refused with the message
`domain resolves to a private address; not fetched`. IPv4-mapped IPv6
answers are unwrapped and checked as IPv4, so `::ffff:127.0.0.1` is refused
like `127.0.0.1`. A refused domain produces zero fetch attempts.

The refusal is recorded as the subscriber's `unreachable` finding with the
class and address in the daily report, and it alerts once per transition
like any other new finding.

### Findings and the diff

Each finding is stored as a small record: the stable subject, the severity
name and the human message. The records of one check hash with SHA-256 into
`lastFindingsHash`, and the records themselves are stored beside it so the
next check can diff instead of comparing hashes.

The diff reports a finding as `New` when its subject did not exist before,
`Worsened` when its severity rose, and `Changed` when the severity held and
the message moved (for example new malformed line numbers). A finding whose
subject resolved contributes nothing. The baseline moves to the current findings
when nothing alerted or the alert was accepted; a failed alert keeps the old
baseline so the next run retries the same alert.

## The alert email

One alert email goes through the `apex-notify` email sink
(`APEX_NOTIFY_SINK=email`, sender from `APEX_NOTIFY_FROM`). The copy lives in
the crate constants: what leaked, the watched domain, one line per changed
finding, the watch page link and the cancel line. The link is
`APEX_WATCH_PAGE_URL` when set, else `APEX_CONSOLE_URL` plus `/watch`.

`apex-watch daily` refuses to start without a configured sink: an
unconfigured alert path would silently drop findings. Alert lines are
`[severity] message`, `[old -> new] message` for a rise, and
`[changed] message` for moved text.

## Monthly billing

`apex-watch bill [YYYY-MM]` (default: the current UTC month) issues one
invoice per active subscriber through `apex_billing::InvoiceRegister` at the
runner's default path, `.apex/billing/invoices.jsonl`. The line is the
frozen `watch_plan` shape: $99 for the normal tier and $199 for the core
tier, in `PLAN_CURRENCY`, with the usual VAT treatment against the seller
profile at `APEX_BILLING_PROFILE`.

The customer profile comes from the subscriber row's `billing` field, or
from the operator sidecar `<watch root>/<publisher>.billing.json` when the
row carries none. A subscriber without either is refused with a typed
reason before the register is touched, so a refusal consumes no number and
no half invoice renders. A subscriber whose row already records the month
in `lastBilledMonth` is skipped, so a rerun can never double-bill. The
numbering stays gapless across subscribers.

## Cancel

`SubscriberStore::cancel` matches a presented token against the stored
SHA-256 hash and appends a canceled row; the token itself is returned once
by `subscribe` and never stored. `apex-watch cancel <token>` performs the
same operation from the console. Cancelled subscribers keep their rows, stop
alerting and are not billed by the next monthly run.

## Environment (watch.env)

| variable | meaning |
|---|---|
| `APEX_WATCH_ROOT` | the subscriber root, default `.apex/watch` |
| `APEX_NOTIFY_SINK` | `email` selects the email sink for alerts |
| `APEX_NOTIFY_FROM` | the sender address of the alerts |
| `APEX_NOTIFY_SENDMAIL` | optional sendmail client path, default `/usr/sbin/sendmail` |
| `APEX_WATCH_PAGE_URL` | full watch page URL for the alert copy |
| `APEX_CONSOLE_URL` | fallback base; the link becomes `<base>/watch` |
| `APEX_BILLING_PROFILE` | the seller billing profile the monthly pass loads |
| `APEX_BILLING_VAT_RATE_BP` | optional override of the seller's domestic rate |

The monthly pass writes the runner's register at its default path under the
unit `WorkingDirectory` (`/var/lib/apex`), so both services share one
register. Neither the crate nor the units hold a platform management key and
neither starts a runner.

## Files and state

| path | content |
|---|---|
| `<watch root>/subscribers.jsonl` | append-only subscriber rows; the newest row per id wins |
| `<watch root>/<publisher>.billing.json` | optional operator billing sidecar per subscriber |
| `.apex/billing/invoices.jsonl` | the shared invoice register |

The subscriber row carries the id, the publisher slug, the domain, the
email, the tier, the account ids, `createdMs`, the status and cancel data,
`lastCheckAt`, `lastFindingsHash`, the recorded findings, `lastAlertAt`,
`cancelTokenHash` and `lastBilledMonth`. The record is camelCase on disk.

## Operations

```sh
apex-watch daily        # the daily check (timer: 05:00 UTC)
apex-watch bill         # the monthly issuance (timer: first, 06:00 UTC)
apex-watch list         # print the current subscriber rows
apex-watch cancel <token>
```

A daily run exits nonzero when an alert or a subscriber record failed to
record; the per-subscriber lines explain what failed. A billing run exits
nonzero when any subscriber was refused.

## Why the fetch uses curl

The audit path's own fetcher is HTTPS-only, runs on the connector transport
and carries a platform label, none of which fit a public one-file watch.
The watch therefore mirrors the sales engine's `io` module: the system
`curl` with a timeout, a size cap and an explicit user agent, the URL after
`--`, and a nonzero exit as a typed transport failure. The HTTPS first
fallback and the per-hop guard are implemented in the crate so every
request, including a redirect hop, passes the same resolve-time check. The
host needs `curl` installed.
