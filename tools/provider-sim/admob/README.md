# AdMob provider simulator

A stateful, standard-library HTTP simulator of the AdMob v1beta surface used by
`apex-connectors/src/admob.rs` and the `apex-runner` AdMob adapter.

- `server.py` — the simulator (Python standard library only).
- `drive_live.py` — a live harness that drives the real `apex-runner`, the real
  connector, a real signed ledger, and (optionally) a real ClickHouse.

## Start

```bash
python3 tools/provider-sim/admob/server.py --port 18181 \
  --account pub-1234567890123456 \
  --access-token sim-access-token \
  --refresh-token sim-refresh-token \
  --client-id sim-client-id --client-secret sim-client-secret \
  --state-file /tmp/admob-sim-state.json
```

The first stdout line is JSON with the bound `base_url` and bootstrap
credentials. `--port 0` picks a free port.

## Endpoints

| Surface | Route |
| --- | --- |
| OAuth2 | `POST /oauth2/token`, `/v1/oauth2/token`, `/oauth2/v4/token`, `/token` |
| Accounts | `GET /v1beta/accounts`, `GET /v1beta/accounts/{account}` |
| Apps | `GET /v1beta/accounts/{account}/apps[/{app}]` |
| Ad units | `GET /v1beta/accounts/{account}/adUnits[/{unit}]` (optional `?appId=`) |
| Mediation groups | `GET /v1beta/accounts/{account}/mediationGroups[/{group}]` |
| Floor write | `PATCH /v1beta/accounts/{account}/mediationGroups/{group}` |
| Experiments | `POST /v1beta/accounts/{account}/mediationGroups/{group}/mediationAbExperiments` and the legacy collection route; `POST .../mediationAbExperiments/{id}:stop` |
| Reports | `POST /v1beta/accounts/{account}/mediationReport:generate` |

Every `/v1beta` route requires `Authorization: Bearer <token>` and rejects
missing, unknown, or expired tokens with `401` and `WWW-Authenticate`.

OAuth2 accepts `grant_type=refresh_token` (with optional `client_id` /
`client_secret`) and `grant_type=client_credentials`. Access tokens expire
after `--token-ttl` seconds (default 3600).

`mediationGroups` responses carry both shapes:

- the canonical v1beta `mediationGroupLines` map with `cpmMicros` integer
  micros and `targeting.format`;
- the legacy `adUnitMappings` array with `ecpm` as a decimal currency amount,
  which is the shape `admob.rs` parses.

`PATCH` accepts either shape, is idempotent on `Idempotency-Key` (identical
replay returns the first response; a different body on the same key is `409`),
and mutates state. A legacy full-list write fans a multi-unit line out into one
line per ad unit because the legacy shape has no line id.

`mediationReport:generate` aggregates seeded rows over `DATE`, `COUNTRY`,
`AD_UNIT_ID`, `APP_ID`, `MEDIATION_GROUP_ID`, `NETWORK`, `AD_FORMAT`, or
`PLATFORM`, with `ESTIMATED_EARNINGS` (`microsValue`) and `IMPRESSIONS`
(`integerValue`). Lists and reports paginate with `pageSize` / `pageToken`.

## Fault injection

```bash
curl -X POST localhost:18181/__admin/faults \
  -H 'Content-Type: application/json' \
  -d '{"target":"groups.list","kind":"429","count":1,"retry_after":1}'
```

Targets: `any`, `groups.list`, `groups.get`, `groups.patch`, `report`,
`experiment.create`, `experiment.stop`, `apps.list`, `adunits.list`, `token`.
Kinds: `401`, `429` (with `Retry-After`), `500`, `timeout` (sleeps past the
client's 10s timeout), `malformed_json`, `empty_body`,
`pagination_truncation`.

Admin routes (loopback only): `GET /__admin/state`, `GET /__admin/requests`,
`POST /__admin/faults`, `POST /__admin/expire-tokens`, `POST /__admin/token`,
`POST /__admin/seed-report`, `POST /__admin/reset`.

## Live drive

```bash
# Build the real runner first.
(cd rust && cargo build -p apex-autopilot --bin apex-runner)

# Real ClickHouse is optional; set APEX_CLICKHOUSE_HTTP when available.
python3 tools/provider-sim/admob/drive_live.py \
  --workdir /tmp/apex-admob-live \
  --clickhouse http://127.0.0.1:18173
```

The harness writes `audit.json`, `cycle.json`, `daily.json`, every
`fault-*.json`, `probe.txt`, `ledger.jsonl`, `sim-state.json`,
`sim-requests.json`, and `summary.json` under `<workdir>/evidence`.

The vault bundle is AES-256-GCM in the same `nonce_hex.ciphertext_hex` format
`apex-vault` writes; `drive_live.py` uses the `cryptography` package only for
that setup step.

## Known client-contract limits

- `AdmobClient` takes a static access token. It has no refresh-token field or
  401-triggered refresh, so the runner cannot renew a token; `drive_live.py`
  demonstrates the refresh exchange and rewrites the vault manually.
- The client has no app or ad-unit read methods. The simulator serves those
  routes, but the product only calls mediation groups and the report.
- The client creates experiments on the collection route
  (`/v1beta/{account}/mediationAbExperiments`), while the documented resource
  nests them under `.../mediationGroups/{group}/mediationAbExperiments`. The
  simulator accepts both. The adapter's recorded call pins the legacy route,
  so patching it requires changing `live_admob.rs`'s tests (out of this tool's
  scope).
- The adapter's experiment create sends only `mediationGroupId` and
  `displayName`; it does not send `treatmentMediationLines`, so the proposed
  floor change never reaches the experiment's test arm. The ledger still
  records `ecpmSet`, and the simulator records an empty treatment arm.
- `apex-runner daily` does not pull AdMob reports: `experiment_days` refuses
  with the documented no-measurement reason, and same-day experiments produce
  a reversed reporting range (`range_start > range_end`). Report ingestion is
  exercised through the audit's `mediationReport:generate` and through the
  direct-connector probe.
