# LevelPlay provider simulator + live apex-runner evidence

This directory contains a stateful, standard-library-only LevelPlay simulator and
a driver that runs the **real** `apex-runner` against it. No product code is
stubbed: the runner reads/writes through `apex-connectors`, opens a real
ed25519-signed ledger, and writes analytics to real ClickHouse.

## Files

| File | Purpose |
| --- | --- |
| `server.py` | Stateful LevelPlay HTTP simulator (stdlib only) |
| `drive.py` | Provisions the runner vault, starts ClickHouse/simulator, runs audit/cycle/daily/faults, writes `report.json` |
| `REPORT.md` | This evidence report |

## Endpoints emulated

| Endpoint | Behaviour |
| --- | --- |
| `GET /partners/publisher/auth` (also accepts POST) | Validates `secretkey` + `refreshToken`; returns a quoted bearer token valid for `--token-ttl` (default 86400s), 401 on invalid credentials |
| `GET /levelPlay/network/instances/v4/{appKey}/` | Bare JSON array, numeric `instanceId`, `networkName`, `rate`; optional no-rate instances |
| `PUT /levelPlay/network/instances/v4/{appKey}/` | Array `[{instanceId, rate}]`; also accepts the legacy `{instanceId, price-micos}` shape and records `legacy_shape` |
| `POST /levelPlay/network/instances/v4/{appKey}/` | Array create with `instanceName`, `networkName`, `adFormat`, `isBidder`, `groups`, `rate`; legacy object accepted/recorded |
| `DELETE /levelPlay/network/instances/v4/{appKey}/` | Body `{"ids":[...]}`; legacy `?instanceId=` accepted/recorded |
| `GET /levelPlay/groups/v4/{appKey}` | Bare array with `groupId`, `groupName`, `abTest`, `floorPrice`, `countriesRate`, `instances` |
| `PUT /levelPlay/groups/v4/{appKey}` | Group floor writes (`floorPrice`) with idempotency |
| `GET /levelPlay/reporting/v1` | `date`/`abTest`/`country`/`adUnit` breakdowns, `revenue`/`activeUsers`/`impressions` metrics, `startDate`/`endDate`, `page`/`resultsPerPage`; `data`/`page`/`pageSize`/`totalResults` envelope |
| `/_control/*` | `state`, `requests`, `reset`, `fault`, `clear_faults`, `expire_tokens`, `expire_next_bearer`, `instance`, `group_floor`, `report`, `shutdown` |

All writes are idempotent on `Idempotency-Key` (same key + same fingerprint
replays the cached response; a different fingerprint is 409). State is mirrored
to `--state-file` after every mutation.

## Running

```bash
# simulator alone (documented wire shape)
python3 tools/provider-sim/levelplay/server.py --port 18081 \
  --state-file /tmp/lp.json

# full live drive: ClickHouse on 18172/19072, vault, audit, cycle, daily, faults
python3 tools/provider-sim/levelplay/drive.py --phase all \
  --workdir /tmp/apex-levelplay-final

# pre-patch regression capture (must be run against an unpatched runner)
python3 tools/provider-sim/levelplay/drive.py --phase baseline \
  --workdir /tmp/apex-levelplay-baseline
```

The driver uses:
- runner: `rust/target/debug/apex-runner`
- ClickHouse HTTP `18172`, native TCP `19072`
- a real `apex-proof` ed25519 ledger under `<workdir>/<case>/ledgers/`
- a real AES-256-GCM encrypted vault credential under `<workdir>/<case>/connections/`
- `developer_site = playrix.com` for the runner's app-ads.txt fetch (the
  fetcher rejects localhost/numeric hosts, so this one input is external)
- analytics DB `apex_levelplay_sim`

## Pre-patch failures (real runner, documented wire shape)

`/tmp/apex-levelplay-baseline/report.json`:

| Case | Exit | Failure |
| --- | --- | --- |
| reporting v1 envelope | 1 | `levelplay adUnitRevenue: malformed response: response has no 'results' array` |
| numeric instances shape | 1 | `levelplay listWaterfall: malformed response: instance missing id: {"instanceId":1001,...}` |

The report request in both cases was the documented query:
`GET /levelPlay/reporting/v1?appKey=lp-app-key&breakdowns=date&metrics=revenue&startDate=2026-09-19&endDate=2026-10-18`.

## Post-patch live drive

From `/tmp/apex-levelplay-final5/report.json`:

### Audit (real snapshot + findings)

```
apex-runner audit --json            exit 0
snapshot: levelplay, 2 waterfall entries, 0 ad units
findings: 2 (1 critical, 1 warning)
critical: reporting_gap (platform revenue > 0, mediation total 0)
```

Both numeric-id (documented) and string-id (adapter compatibility) audits pass.
The snapshot's waterfall after the patch reads:

```json
[
  {"entryId":"1001","network":"unityAds","instanceName":"Rewarded_High","position":1,"priceEcpm":2000000},
  {"entryId":"1002","network":"admob","instanceName":"Rewarded_Backfill","position":2,"priceEcpm":800000}
]
```

### Cycle (real write + state + ledger + subsequent read)

```
apex-runner cycle --json            exit 0
cycle: applied=1, proposed=2
PUT /levelPlay/network/instances/v4/lp-app-key/
  body {"instanceId":"1001","price":2400000}   (legacy shape; see limitations)
state: rate 2.00 -> 2.4
ledger: verified=true, records_appended=3, highest_sequence=2
```

Ledger records: `preRegistration`, `changeApplied`, `experimentStarted`,
`dataPull`; every line carries an ed25519 signature and `verified=true` from the
runner. The post-cycle audit read confirms the change:

```json
{"entryId":"1001","priceEcpm":2400000}
```

### Daily (report ingestion)

```
apex-runner daily --json            exit 1
reporting_pulls: completed=1, failed=0
ledger: verified=true, records_appended=1 (dataPull), highest_sequence=3
projections: 4, published_version=4, rows=2
platform_operations: unresolved=1
```

Ingestion succeeded (`ingestion_ok=true`). The exit-1 unresolved operation is a
settlement failure caused by an adapter shape mismatch outside this change's
ownership (see limitations).

### Real ClickHouse

```
HTTP 18172: SELECT count() FROM apex_levelplay_sim.domain_events   -> 8
TCP  19072: clickhouse client --port 19072 ...                     -> 8
projection_versions                                                -> 8
```

## Fault matrix (all via the real runner)

Counts are requests **after** the clean setup audit.

| Fault | Command | Exit | Observed |
| --- | --- | --- | --- |
| 401 bearer expiry (`expire_next_bearer`) | audit | 0 | auth×2, report×2, instances×1; re-auth + retry once, no failure |
| 429 + `Retry-After: 1` on report | audit | 0 | report×2; waited ~1s then succeeded |
| 500 on report | audit | 1 | report×3; bounded retries exhausted, `HTTP 500` |
| timeout on report (10.5s) | audit | 1 | report×3 over ~30.8s; bounded, `network error: timeout` |
| malformed JSON on report | audit | 1 | report×1; typed schema error, no retry |
| pagination truncation (announces 30, returns 1) | audit | 1 | report×2; `report pagination truncated: read 1 of 30 announced rows` |
| 500 on every instance write | cycle | 0 | PUT×6 (2 proposals × 3 attempts), `applied=0`, state unchanged, ledger verified |
| 429 + `Retry-After: 1` on first write | cycle | 0 | PUT×3, `applied=2`, rate 2.00 → 2.4 and 0.80 → 0.96 (retry-safe) |
| strict provider write shape | cycle | 0 | PUT×2, both `400 legacy update shape refused`, `applied=0`, state unchanged |

The write-500 cycle exits 0 because a rolled-back proposal is a recorded
outcome; it is an honest non-application (`applied=0`, no provider mutation, no
`changeApplied` record), not a false success. The strict-write probe proves the
same for the legacy body shape.

## Defects patched (`rust/crates/apex-connectors/src/levelplay.rs`)

| Line | Patch |
| --- | --- |
| `:59` | Bearer lifetime 1800s → documented 86400s (24h) |
| `:205` | Auth exchange `POST` → documented `GET` |
| `:232-247` | Auth 401/403 maps to typed `PlatformError::Auth` |
| `:268-302` | On a 401 management call, invalidate the cached bearer, re-mint, retry exactly once (401/403 distinction preserved) |
| `:323-394` | `list_waterfall` accepts the documented bare array, numeric ids, `networkName`, `rate`, numeric positions; absent `rate` = no instance-level rate (0), malformed rate is typed |
| `:428-464` | `create_entry` accepts the documented array response |
| `:466-489` | `remove_entry` uses the documented `DELETE {"ids":[...]}` body |
| `:540-616` | Reporting accepts `data`/`results`, follows `totalResults` with `page`/`resultsPerPage`, and refuses a short/truncated read typed |
| `:732-751` | Numeric-or-string id and integer helpers |

Regression tests were added/updated in `rust/crates/apex-connectors/src/tests.rs`:
`lp_v4_instances_read_numeric_ids_rate_and_network_name`,
`lp_reporting_v1_data_envelope_and_pagination`,
`lp_reporting_truncated_pagination_is_a_typed_refusal`,
`lp_a_401_refreshes_the_bearer_once_and_retries_the_call`,
`lp_auth_401_is_a_typed_auth_refusal`, plus the auth-method/remove-body updates.
`cargo test -p apex-connectors` (132 passed, 2 ignored) and the 22 LevelPlay
`apex-autopilot` tests pass; `cargo clippy -p apex-connectors --all-targets --
-D warnings` is clean.

## Contract limitations (not patched; outside the stated ownership)

1. **Instance write body shape.** `update_entry_price` (`levelplay.rs:414`) and
   `create_entry` (`levelplay.rs:446`) still emit the legacy object with
   `price` in **micros**. The documented v4 write is an array with `rate` in
   currency units (`[{"instanceId":1001,"rate":2.4}]`) and create additionally
   needs `networkName`, `adFormat`, `isBidder`, `groups`. The
   `apex-autopilot` tests (`live_levelplay::tests::the_instance_price_action_
   writes_the_variant_group_instance`, `...created_entries...`) pin the exact
   legacy bodies, and `apex-autopilot` is out of scope. A faithful server
   (`--strict-writes`) rejects the client's body; the live drive runs without
   `--strict-writes`, records `legacy_shape: true`, and proves the applied state
   and ledger are still correct.

2. **Numeric group/instance ids.** The documented groups response uses numeric
   `groupId` and `instances[].id`. `live_levelplay.rs:137` and `:326` require
   `Value::as_str`, and `require_entry_in_group` compares strings. The drive
   therefore runs the simulator in `--id-style string` for cycle/daily; the
   faithful numeric mode is used for the read audit. The default simulator is
   numeric.

3. **Daily settlement.** `runner.rs:2330` derives `expected_settings` from the
   pre-registration's `before_value` (an instance-price object), while
   `live_levelplay.rs:371` returns the groups array from `current_settings`.
   They can never compare equal, so `settings_changed` fires, the decision voids,
   and `deprecate_experiment` (`live_levelplay.rs:391`) has no documented API and
   refuses. That is the `platform_operations.unresolved=1` after a successful
   report ingestion. Fixing it requires `apex-autopilot`.

4. **One group B for every proposal.** `start_experiment`
   (`live_levelplay.rs:226-232`) returns the shared group B `groupId` as the
   experiment id for every change. Multiple proposals in one cycle therefore
   collide. The settlement probe uses `maxActionsPerDay=1` and a simulator
   variant-group id equal to the generated experiment name; the default drive
   demonstrates the normal two-proposal cycle separately.

5. **Reporting-gap critical is structural.** `audit_job.rs:138-143` fills only
   `platform_totals_30d`; `mediation_totals_30d` stays at the
   `PublisherContext::default()` zero (`audit_job.rs:55-61`). The
   `reporting_gap` rule (`apex-audit/src/rules/reporting_gap.rs`) therefore fires
   Critical for every LevelPlay snapshot with revenue. The simulator cannot
   supply apex-side mediation totals; this is an audit integration gap, not a
   provider response.

6. **app-ads.txt is external.** The runner fetches `https://playrix.com/app-ads.txt`
   because `AppAdsTxtFetcher` rejects localhost and numeric hosts. The LevelPlay
   simulator does not stand in for that fetch.

7. **No promote/deprecate/stop API.** LevelPlay documents dashboard-owned A/B
   lifecycle operations; the adapter intentionally refuses them. The simulator
   does not invent those endpoints.

## Reproduction notes

- No commits were made.
- Changed/added paths: `rust/crates/apex-connectors/src/levelplay.rs`,
  `rust/crates/apex-connectors/src/tests.rs`,
  `tools/provider-sim/levelplay/{server.py,drive.py,REPORT.md}`.
- Full machine-readable evidence: `/tmp/apex-levelplay-final5/report.json`
  (full + faults) and `/tmp/apex-levelplay-baseline/report.json` (pre-patch).
- The driver starts ClickHouse on 18172/19072 if it is not already listening.
