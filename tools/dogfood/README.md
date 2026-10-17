# tools/dogfood: the adversarial dogfooding catalog and live harness

Two halves, one contract.

- `surfaces.json` is the catalog: the frozen contract the static gate
  (`apex-tools dogfood-plan`) and the live harness (`live.py`) both read.
- `live-baseline.json` is the ratchet: the recorded cell set a green run
  produced. Both halves read it and both fail on drift.
- `live.py` is the live harness: boots the three services against a real
  ClickHouse and runs every required probe cell.

## The catalog

Schema, exactly as enforced by `rust/crates/apex-tools/src/dogfood_plan.rs`
(that file is the authority for every field name):

- `probe_classes`: the 16 frozen probe classes, in order. Must equal the
  registry in `dogfood_plan.rs`.
- `services.<id>.routes`: the routes as registered, `{"path", "methods"}`
  per route, plus `"session_required": true` on the publisher-console GETs
  that refuse anonymous reads, plus a non-empty `"sources"` list naming the
  repo paths that implement the surface (a console page names
  `rust/crates/apex-api/src/html/...`; `/v1/audit` names
  `rust/crates/apex-api/src/routes.rs`; the watch fetch names
  `rust/crates/apex-watch/src/fetch.rs`). The static gate requires exact parity
  with the routers, validates the marker (present only as `true`, only on an
  apex-api console GET, never on a documented public console path) and the
  `sources` shape (repo-relative, at least one per route), and the live
  harness asserts both directions: anonymous `303 -> /console/ledger` and a
  render under a live session. The `sources` lists are what the relevance
  filter (`--changed` / `--git-diff`) selects on.
- `services.<id>.fixture_pages`: must equal the `PAGES` array length in that
  crate's `render-fixtures` bin (the bins carry the array precisely so this
  gate can check it).
- `services.<id>.required`: `{"surface", "classes"}` entries. A surface is a
  declared route surface string (`"GET /console/audit"`); expanding an entry
  yields cells `"<service>|<surface>|<class>"`.
- `site.pages_min`: the build-output page floor for `site/public`; a probe of
  the static half, not a service.

The statuses in `live.py` tables (for example `GET /console/audit` -> 400
without a platform) are read from the product's own wire-contract docs in the
route modules; the tables are the harness's reading of the frozen product
contract, not new policy.

## Running the live harness

```
python3 tools/dogfood/live.py --static-only           # catalog + site only, no processes
python3 tools/dogfood/live.py --clickhouse-owned      # full live run (make dogfood-live)
python3 tools/dogfood/live.py --update-baseline       # rewrite the ratchet after a green run
python3 tools/dogfood/live.py --changed <path> ...    # only the cells a change touches
python3 tools/dogfood/live.py --git-diff <base>..<head>
```

Flags: `--bin-dir` (default `rust/target/debug`), `--workdir` (default
`rust/target/dogfood-live`), `--clickhouse-url` (default
`http://127.0.0.1:18123`; the run FAILS when the store does not answer),
`--results` (default `rust/target/dogfood-live/results.json`), `--port-base`
(default 18700), `--clickhouse-owned`, `--changed` (repeatable),
`--git-diff`.

## Relevance: run only what changed

`--changed <path>` (repeatable) and `--git-diff <base>..<head>` narrow the run
to the cells whose surface `sources` intersect the changed repo paths, plus a
small always-run core set (the `/healthz` contract, the console session gate,
the two-publisher tenant isolation of the overview and the ledger). The
selection is deterministic (sorted paths, stable order) and mirrors the static
gate's `--changed` / `--git-diff` in `dogfood_plan.rs`. Classes with no
selected cell are skipped entirely and reported:
`selected N of M cells (skipped K unrelated)` with the skipped classes listed.
With neither flag the run is unfiltered (every required cell must pass). A
changed path that matches no declared source selects the core set only and
says so. Under a selection the baseline ratchet still refuses a recorded cell
that is no longer required, but does not demand the unselected, not-yet-run
cells be recorded.

## The ApAudit-review classes

The twelve original classes plus four added for the ApAudit review. Each
encodes the *fixed* product behaviour as its expectation, so on a tree where
the fix has not landed the cell fails (that failure is the adversarial proof):

- `tenant-isolation`: two publishers are seeded (own snapshot at
  `<root>/<publisher>/<platform>.json`, own signed ledger, own bearer token).
  Every console page and `/v1` read for one session must show only its own
  snapshot markers and its own verified-record total; the other publisher's
  domain, amount or finding must never appear in the bytes.
- `auth-surface`: `GET/POST /v1/audit/{platform}` answers `401` without the
  bearer; the bearer must also name exactly one publisher (`?publisher=` on
  `GET`, the `publisher` body field on `POST`), and the `200` read carries
  only that publisher's snapshot. A second publisher is read through the same
  route to prove the bytes never cross. The session-gated console GETs answer
  `303 -> /console/ledger`.
- `signin-ownership`: the free-audit sign-in link for an existing workspace is
  delivered only to its recorded contacts (never the form-supplied email), a
  brand-new workspace with a mismatched-domain email is refused, and a spent
  single-use link is refused on every repeat. The accepted submission stores
  the lead and answers `303` even when the workspace/link is refused.
- `payment-activation`: a setup-watch sign-up lands pending with no check run,
  the checkout link recorded, and the confirmation email sent; the tier comes
  from the domain's app-ads evidence (the form's `tier`/`app_count` fields are
  ignored; unavailable evidence records the honest Normal tier). The mailed
  `GET /watch/confirm?token=` confirms the address (missing/garbage tokens are
  `400`; a used token is the `200` refusal), the signed Adyen webhook
  activates the row (both the bare subscriber id and `apex-watch:<id>`), the
  stored card is recorded, a replay adds nothing, a daily pass sees the
  confirmed subscriber as checkable, and the monthly bill charges the card
  idempotently.
- `GET /watch/confirm` gets its own response contract in the loop: a missing
  or malformed token is `400` with the typed refusal copy, a fresh mailed
  token is `200` "Your email is confirmed.", and a spent token is the `200`
  refusal. Its traversal cell drives the hostile corpus through `?token=` and
  accepts only a typed refusal or that honest page; the no-inline sweep reads
  the same route.

`differential-truth` gained the LevelPlay revenue-vs-users cell (the reporting
request carries `metrics=revenue,activeUsers` and `breakdowns=date,abTest`, so
the lift value is revenue and the active users ride separately) and the VIES
cell (reverse charge only with a recorded VIES confirmation; without one the
invoice is held with the documented reason). `failure-injection` gained the
DNS-rebinding cell (the watch fetch must pass
`curl --resolve host:port:address` with the guard-approved address) and the
unpaid-subscriber cell (a subscriber that has not paid runs no daily check).


Enforcement: every required cell must be attempted and pass. Any failed cell,
any unattempted cell, and any failed standalone check exit 1 and are listed
in the summary and in `results.json`. Per-cell evidence (the exact command or
observation) is recorded there too.

## Execution order and the ClickHouse severance

The 16 frozen probe classes in `surfaces.json` are the registry, not the
dispatch schedule. `differential-truth` (the only class that reads the seeded
ClickHouse projections and the runner's published numbers) now runs **before**
`failure-injection`. The failure-injection subprobe deliberately shuts the
owned server down and restarts it on a fresh recovery data path, so a class
that ran afterwards used to abort every cell with
`Database dogfood_live_<pid> does not exist`. Moving the one dependent class
ahead of the severance is the smallest robust fix; the class registry and the
cell set are unchanged. No probe after `failure-injection` needs the store.

## Harness-owned fixture normalization

Two product-contract gaps are repaired in the harness's own copies, never in
the committed product fixtures:

- the committed `northstar.jsonl` still carries one legacy
  `{"type":"lift"}` payload that the runner's `LiftRecord` parser correctly
  refuses. `prepare_api_files` rewrites that payload as an inert `dataPull`
  record in its ledger copy and re-seals the chain from that point with the
  same `[7u8; 32]` seed the fixture already uses; the API's ledger verifier
  accepts the result.
- the fleet `cycle` now consumes `<snapshot_root>/<publisher>/<wire>.json`
  and replaces any `--snapshot` argument. The harness therefore seeds
  `runner-snapshots/northstar/admob.json` from the committed AdMob fixture
  with the `$350k` band revenue, which keeps real proposals to hold *and*
  makes the monitoring tier band-selected.

## What the harness boots

- ClickHouse: provided externally (`make clickhouse-dev`, or the lane's
  service container). The harness creates a fresh database
  `dogfood_live_<pid>`, seeds `report_days` (10 final days at 5,150,000,000
  micros for publisher northstar) and `domain_events` (one `fee_computed`
  event) over the HTTP interface, then runs `apex-runner daily` in fleet mode
  with `APEX_CLICKHOUSE_*` set so the projections are rebuilt and published.
  Revenue pages must then show USD 51,500.00 — differential truth hides
  nothing.
- `apex-api`: fixtures copied from `rust/crates/apex-api/fixtures` plus temp
  snapshots, ledger, tokens (`APEX_LEDGER_TOKENS_FILE`), leads, the hash-only
  `signin.jsonl` rows beside them (one valid, one used, one expired link),
  the watch store root, operator-jobs token/dir and proposals, mirroring the
  crate's own tests. The one intended mutation: the snapshot copy's first ad
  unit gets a hostile name and a formula-leading id so the escaping probe has
  real cargo. One loopback stub serves the Adyen payment-link/card-on-file
  calls and the AdMob mediation-group read, so the corrected `/watch` flow and
  the live `POST /v1/audit` cell are exercised end to end; the vaulted
  account's developer site is a real public HTTPS domain whose
  `/app-ads.txt` answers `200`. A second `apex-api` instance boots with no
  tokens, no leads and no live-audit/operator token so the documented `503`
  refusals are asserted.
  A second publisher (`rival`) is seeded with its own per-publisher snapshot
  layout, its own five-record signed ledger and its own bearer token, and the
  transactional sink points at a stub sendmail so the signin-ownership probe
  can read where a link was delivered.
- `apex-runner` (fleet mode): the same `credentials` root the console reads
  holds a vaulted AdMob connection bundle plus its validated meta record, the
  customer billing sidecars and the recorded managed plan state; the seller
  profile and a per-publisher snapshot root (`runner-snapshots/northstar/`)
  carrying the committed AdMob fixture with the seeded `$350k` 30-day revenue.
  `cycle` consumes that root and activates the monitoring tier from the band,
  and a signed kept AdMob change (`experimentStarted` +
  `experimentObserved` + `preRegistration` + `result keep`) is seeded into the
  shared ledger with the deployment's runner seed — the product's own ledger
  verifier accepts every line. The LevelPlay revenue cell registers a
  throwaway LevelPlay publisher whose base URL is a loopback stub, runs
  `apex-runner aa-eval` against it and asserts the recorded reporting query.
- `apex-control`: two operators hashed through `apex-control hash-password`
  (TOTP secret `AAAA…B`), journal/jobs/ledger/policies/interventions in the
  workdir, the committed northstar ledger as the read-only evidence source.
- `apex-sales-web`: the store seeded through the `apex-sales` CLI
  (`import-seeds`, `import-leads`, `record`, `suppress`), plus one
  Touch-equivalent send written in the engine's own durable shapes (an
  outbox `.eml` named after the message id and the matching Touch event line
  in `events.jsonl`).
- The site is never served: its probe parses `site/public` statically, and
  when the directory is absent the check is *skipped with the reason
  recorded*, never silently.

## The monetization truth cells

The differential-truth probe runs the product's own billing machinery over
the seeded stores and cross-checks every observable:

- the monthly runner run renders the monitoring line at the tier chosen from
  the seeded 30-day revenue band (Mid, `$1,500`) and its recorded stop
  period, the plan state file and the console monitoring page must agree;
- the setup-watch sign-up submitted through the public form is billed by
  `apex-watch bill` as exactly one `setup-watch:$period` `$99` line, and the
  second run reports it skipped;
- a cross-border EU customer with a VAT number is billed through the same
  `apex-watch bill` path: the register line's VAT treatment must cite a
  recorded VIES confirmation before it may state reverse charge, or the bill
  must hold with the documented VIES reason;
- the kept AdMob change is typed fee-ineligible (no per-variant revenue
  measurement) in the monthly report and the shared ledger, and no billed
  artifact anywhere names it.

## Honest-state acceptance

The traversal probe's contract is "never 200 with foreign content": a
response passes as a not-found refusal (`400`/`404`/`303`/`401`), as the
honest not-configured state (`503`), or as a `200` whose body carries the
page's own unavailable copy (`no durable record names`, `is not readable`).
Every response is additionally scanned for filesystem canaries. The
`--clickhouse-owned` flag is what makes the failure-injection store-severance
subprobe *required*; with an external, unowned server (the CI lane) the
subprobe records a reasoned skip instead of managing a server it does not own.

Mechanical notes:

- The TOTP secret in the harness is the 32-character spelling
  (`31 × "A" + "B"`): the 31-character literal is correctly refused by the
  product's base32 decoder (trailing bits must be zero), so the harness uses
  the byte-aligned form of the same secret.
- The harness seeds control accounts through the real subcommand:
  `apex-control hash-password` with the password on stdin, PHC on stdout.
- The vaulted credential bundle (`<publisher>.blob`, AES-256-GCM,
  `nonce-hex.ciphertext-hex`) and the seeded ledger lines (ed25519 over the
  canonical record hash, `{"record":..,"signature":hex,"pubkey":hex}`) are
  written by the stdlib-only implementations at the top of `live.py`, pinned
  against the published NIST AES-GCM and RFC 8032 test vectors by
  `crypto_self_check`, which runs at boot. The product itself is the final
  oracle: the registry decrypts the blob and the ledger verifier accepts the
  chain, or the live run fails.

## Teardown

Always: every process the harness started is killed, the ClickHouse test
database is dropped while the store still answers, and the cleanup proof
(`lsof` listeners per harness port, the recovery server's exit state) is
printed. A run against an unowned external store never kills or restarts it.
