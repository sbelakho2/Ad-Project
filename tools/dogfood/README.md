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

- `probe_classes`: the 12 frozen probe classes, in order. Must equal the
  registry in `dogfood_plan.rs`.
- `services.<id>.routes`: the routes as registered, `{"path", "methods"}`
  per route, plus `"session_required": true` on the publisher-console GETs
  that refuse anonymous reads. The static gate requires exact parity with the
  routers, validates the marker (present only as `true`, only on an apex-api
  console GET, never on a documented public console path), and the live
  harness asserts both directions: anonymous `303 -> /console/ledger` and a
  render under a live session.
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
```

Flags: `--bin-dir` (default `rust/target/debug`), `--workdir` (default
`rust/target/dogfood-live`), `--clickhouse-url` (default
`http://127.0.0.1:18123`; the run FAILS when the store does not answer),
`--results` (default `rust/target/dogfood-live/results.json`), `--port-base`
(default 18700), `--clickhouse-owned`.

Enforcement: every required cell must be attempted and pass. Any failed cell,
any unattempted cell, and any failed standalone check exit 1 and are listed
in the summary and in `results.json`. Per-cell evidence (the exact command or
observation) is recorded there too.

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
  real cargo. A second `apex-api` instance boots with no tokens, no leads and
  no live-audit/operator token so the documented `503` refusals are asserted.
- `apex-runner` (fleet mode): the same `credentials` root the console reads
  holds a vaulted AdMob connection bundle plus its validated meta record, the
  customer billing sidecars and the recorded managed plan state; the seller
  profile and an empty AdMob snapshot carrying the seeded `$350k` 30-day
  revenue live beside it. `cycle` activates the monitoring tier from that
  band, and a signed kept AdMob change (`experimentStarted` +
  `experimentObserved` + `preRegistration` + `result keep`) is seeded into the
  shared ledger with the deployment's runner seed — the product's own ledger
  verifier accepts every line.
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
