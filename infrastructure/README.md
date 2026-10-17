# Infrastructure

Current deployment layout for the no-SDK autopilot product:

- `deployment.toml`: the deployment inventory (service, site, edge,
  monitoring paths; no secrets).
- `caddy/`: the edge config. Two public origins, both fronting `apex-api`
  on the console host: the marketing host serves the built Zola site and
  proxies only `POST /free-audit`, `/audit-requested/*` and `/console*` to
  `apex-api`; the API host fronts `apex-api` whole. The operator origin is
  not on this edge: `apex-control` runs on the control host and binds
  loopback there (see "Hosting split" below). Host names come from
  `SITE_DOMAIN` and `APEX_DOMAIN`; the marketing JSON access log path comes
  from `APEX_ACCESS_LOG` (default `/var/log/caddy/apex-access.json`). The
  routing contract lives in `infrastructure/caddy/tests/contract.sh` and
  runs in CI.
- `systemd/apex-api.service`: the service unit; the environment file holds
  the vault key, account root, live-audit token, per-publisher ledger tokens
  (`APEX_LEDGER_TOKENS_FILE`, a JSON object of publisher to token) and Adyen
  settings. Without the ledger tokens file, no ledger is readable over HTTP.
  The optional `APEX_METRICS_TEXTFILE` names the exported FILE (for example
  `/var/lib/node_exporter/textfile/apex-api.prom`), which the unit's
  `ReadWritePaths` covers; see `monitoring/README.md`.
- `systemd/apex-control.service`: the operator origin; runs on the control
  host and reads `/etc/apex/control.env`, from `/var/lib/apex`, where the
  `.apex/control/` state directory (journal, operator jobs, operations
  evidence) is the writable path. Environment (the `APEX_CONTROL_*`
  namespace, parsed in `apex-control`'s config module):
  `APEX_CONTROL_BIND` (default `127.0.0.1:8790`),
  `APEX_CONTROL_OPERATORS` (operator configuration JSON; absent means the
  logged unconfigured state where no session can be minted),
  `APEX_CONTROL_JOURNAL` (default `.apex/control/journal.jsonl`),
  `APEX_OPERATOR_JOBS_DIR` (default `.apex/control/jobs`),
  `APEX_CONTROL_LEDGER_ROOT` (default `.apex/ledgers`),
  `APEX_CONTROL_INVOICE_REGISTER` (default `.apex/billing/invoices.jsonl`),
  `APEX_CONTROL_OPERATIONS` (default `.apex/control/operations.json`),
  `APEX_CONTROL_POLICIES`, `APEX_CONTROL_INTERVENTIONS`, `APEX_ENVIRONMENT`
  and the analytics read-model pair `APEX_CLICKHOUSE_HTTP` /
  `APEX_CLICKHOUSE_DB`, plus the optional `APEX_METRICS_TEXTFILE` export
  file (for example
  `/var/lib/node_exporter/textfile/apex-control.prom`). Remote jobs mode is
  the frozen contract `POST`/`GET /v1/operator/jobs` on apex-api: with
  `APEX_API_JOBS_URL` set (the apex-api origin reachable from the control
  host), the control plane reads and posts operator jobs through it,
  authenticated with the bearer token in `APEX_OPERATOR_JOBS_TOKEN`.
  The unit pins `WorkingDirectory=/var/lib/apex`, so the relative defaults
  resolve under the state root, and its `ReadWritePaths` cover the state
  directory and the textfile directory so the atomic export (temp file plus
  rename) writes there.
- `systemd/apex-ingest.service` and `apex-ingest.timer`: the hourly
  access-log ingest. The oneshot runs
  `apex-ingest /var/log/caddy/apex-access.json --spool
  /var/lib/apex/.apex/analytics-spool/apex-ingest.jsonl`; failed batches
  spool for the next run and replays deduplicate in the store by
  `(source, event_id)`. A consumed-offset cursor
  (`APEX_INGEST_CURSOR`, default `<log>.cursor`) keeps each hourly run
  incremental; log rotation restarts it safely. `/etc/apex/ingest.env` carries
  `APEX_CLICKHOUSE_HTTP` (default `http://127.0.0.1:8123`),
  `APEX_CLICKHOUSE_DB` (default `apex_analytics`), `APEX_ENVIRONMENT`
  (default `production`), `APEX_SERVICE_VERSION` and `APEX_INGEST_SPOOL`
  when the spool should move elsewhere.
- `systemd/apex-runner-*.service` and `*.timer`: the scheduled jobs.
  `apex-runner-audit.timer` refreshes the account snapshot at 02:30 UTC;
  `apex-runner-cycle.timer` runs the write cycle (propose from the account
  snapshot and start pre-registered experiments) at 03:00 UTC;
  `apex-runner-daily.timer` runs the pull/settle pass at 03:30 UTC;
  `apex-runner-monthly.timer` bills the month just closed and collects through
  Adyen at 04:30 UTC on the first. All read `/etc/apex/runner.env`:
  `APEX_RUNNER_LEDGER`, `APEX_RUNNER_PUBLISHER` (legacy single-publisher mode
  only), `APEX_RUNNER_SIGNING_KEY_HEX`, `APEX_MAX_API_KEY`,
  `APEX_MAX_REPORT_KEY`, `APEX_RUNNER_WRITABLE`, `APEX_RUNNER_POLICY`,
  `APEX_RUNNER_PRIORS` (a file in legacy mode, a directory of
  `<publisher>.json` bucket models in fleet mode), `APEX_RUNNER_SNAPSHOT`,
  `APEX_DEVELOPER_SITE`, `APEX_EXPECTED_NETWORKS`,
  `APEX_ADYEN_API_KEY`, `APEX_ADYEN_MERCHANT_ACCOUNT`, `APEX_ADYEN_HMAC_KEY`,
  `APEX_ADYEN_CHECKOUT_BASE`, `APEX_BILLING_VAT_RATE_BP`,
  `APEX_BILLING_PROFILE` (the seller profile the monthly VAT issuance loads),
  `APEX_MONITORING_TIER_BASE_MICROS`, `APEX_MONITORING_TIER_MID_MICROS` and
  `APEX_MONITORING_TIER_TOP_MICROS` (the monitoring tier prices; monitoring is
  on at connection), with `APEX_MONITORING_FEE_MICROS` as the deprecated
  base-tier fallback and the older `APEX_INVOICE_VAT_BP` likewise read as a
  deprecated fallback; all announce themselves on stderr, `APEX_ALERT_FILE`
  (the console reads the same variable; `APEX_ALERTS_FILE` remains a
  fallback), `APEX_CLICKHOUSE_HTTP`,
  `APEX_ANALYTICS_ENV` (fallback `APEX_ANALYTICS_ENVIRONMENT`),
  `APEX_ANALYTICS_SPOOL`, `APEX_METRICS_TEXTFILE`, `APEX_PAYMENT_EVENTS`,
  `APEX_PAYMENT_LEDGER`, `APEX_PAYMENT_CURSOR`, `APEX_NOTIFY_CONTACTS`,
  `APEX_NOTIFY_WEBHOOK`, `APEX_NOTIFY_WEBHOOK_TOKEN`, `APEX_NOTIFY_STATE`,
  `APEX_RUNNER_DATA_DIR`, `APEX_PAYMENT_RETRIES`, `APEX_NOTIFY_FILE`,
  `APEX_CONSOLE_URL`, `APEX_MAX_MANAGEMENT_BASE`, `APEX_MAX_REPORTING_BASE`
  and `APEX_SCHEDULED_AT_MS`. When `APEX_CONNECTIONS_ROOT` (fallback
  `APEX_CREDENTIAL_ROOT`) names the vault root the console writes, the runner
  runs in **fleet mode**: every command loops over the registered publishers,
  each with its vaulted platform keys, its per-publisher ledger at
  `APEX_RUNNER_LEDGER_ROOT/<publisher>.jsonl`, its customer profile at
  `<vault root>/<publisher>.billing.json`, and a typed skip line for an
  unvalidated connection; without a registry root the single-publisher
  `APEX_RUNNER_PUBLISHER`/`APEX_RUNNER_LEDGER` mode keeps working exactly as
  before.

  Every unit pins `WorkingDirectory=/var/lib/apex`, so the relative defaults
  resolve under the state root and `APEX_METRICS_TEXTFILE` can name an
  explicit export FILE such as
  `/var/lib/node_exporter/textfile/apex-runner.prom` (the node exporter's
  `--collector.textfile.directory` reads the containing directory; see
  `monitoring/README.md`).
  With `APEX_CLICKHOUSE_HTTP` set, the runner emits analytics events through
  the spooling client: a failed batch is appended to the runner's own spool,
  `.apex/analytics/spool.jsonl` by default (override with
  `APEX_ANALYTICS_SPOOL`), and the next runner start replays the spool before
  running, so events survive a store outage; the store's `(source, event_id)`
  dedup absorbs the replay. `apex-ingest` keeps its own separate spool.
  `apex-api` and `apex-control` use the same contract with their own default
  spool files (`spool-apex-api.jsonl` and `spool-apex-control.jsonl` under
  `.apex/analytics/`), each replaying at boot; their units'
  `ReadWritePaths` cover the state directory those files live in.
  `APEX_SCHEDULED_AT_MS` is the expected timer fire time in unix milliseconds
  for the schedule-lag histogram; the runner records lag only when this
  variable or the `--scheduled-at` flag is supplied, because systemd's
  specifier set has no timer-fire-time specifier to pass through `ExecStart`.
- `systemd/apex-sales-*.service` and `*.timer`: the owner-only outbound
  sales engine (`rust/crates/apex-sales`, runbook in
  `docs/Internal/Sales/APEX_SALES_ENGINE.md`). It is an internal tool for
  finding publishers and emailing them; customers never see it.
  `apex-sales-discover.timer` refreshes public `app-ads.txt` and App Store
  evidence at 01:15 UTC; `apex-sales-run.timer` runs hourly, imports new
  free-audit requests from the apex-api leads file, then sends what is due
  in each contact's local business hours. Both read `/etc/apex/sales.env`:
  `APEX_SALES_MODE` (`off`, `shadow`, `approval`, `autonomous`; default
  `shadow`, which writes previews to `.apex/sales/outbox` and sends nothing),
  `APEX_SALES_FROM`, `APEX_SALES_FROM_NAME`, `APEX_SALES_COMPANY`,
  `APEX_SALES_ADDRESS` (all four required to send),
  `APEX_SALES_UNSUBSCRIBE_URL`, `APEX_SALES_TOKEN_SALT`,
  `APEX_SALES_LEADS_FILE` (the same path as apex-api's `APEX_LEADS_FILE`),
  `APEX_SALES_SENDMAIL` (an unprivileged sendmail-compatible client such as
  `/usr/bin/msmtp`; the units set `NoNewPrivileges`, which blocks Postfix's
  setgid `postdrop`),   `APEX_SALES_HOLDOUT`, `APEX_SALES_HOLDOUT_SALT`,
  `APEX_SALES_EXPERIMENT`, `APEX_SALES_LEGAL_FILE`, `APEX_SALES_DAILY_MAX`,
  `APEX_SALES_FETCH_DELAY_MS` and `APEX_SALES_DIR` (default `.apex/sales`).
  The host needs `curl` for discovery.
- `systemd/apex-sales-web.service`: the owner-only sales workspace UI
  (`rust/crates/apex-sales-web`), the runbook's "Workspace UI" section in
  `docs/Internal/Sales/APEX_SALES_ENGINE.md`. It serves only
  `127.0.0.1:8795` (default; never exposed to customers) and reads
  `/etc/apex/sales-web.env`: `APEX_SALES_WEB_TOKEN` (the owner gate;
  **required** — unset means the fail-closed not-configured state where
  every page explains the requirement and no session can be minted),
  `APEX_SALES_WEB_BIND` (default `127.0.0.1:8795`), `APEX_SALES_DIR`
  (default `.apex/sales`; the pages read the store and the
  approve/suppress/record/reset-health actions write events and approvals
  into it), every `APEX_SALES_*` variable the engine reads (the service
  hands the engine the same environment the CLI uses), and the optional
  `APEX_METRICS_TEXTFILE` export file (for example
  `/var/lib/node_exporter/textfile/apex-sales-web.prom`; the service
  exports the bounded route-template series atomically every 15 seconds
  with no `/metrics` route). The unit pins `WorkingDirectory=/var/lib/apex`,
  so the relative defaults resolve under the state root, and its
  `ReadWritePaths` cover `.apex/sales` and the textfile directory. Assets
  are served by the service itself: `/assets/apex.css` is the shared
  design-system stylesheet (the exact `console_css` bytes) and the fonts
  are the self-hosted WOFF2 faces; the UI sends nothing on its own, it
  only mirrors and gates the engine's plan.
- `systemd/apex-watch*.service` and `*.timer`: the paid daily app-ads.txt
  watch (`rust/crates/apex-watch`, runbook in
  `docs/Internal/Watch/APEX_WATCH.md`). `apex-watch.timer` runs the daily
  check (fetch, rule, diff, alert) at 05:00 UTC; `apex-watch-bill.timer`
  issues the month's setup-watch lines at 06:00 UTC on the first. Both read
  `/etc/apex/watch.env`: `APEX_WATCH_ROOT` (default `.apex/watch`, the
  subscriber JSONL root the public `/watch` sign-up in apex-api writes),
  `APEX_NOTIFY_SINK=email` with `APEX_NOTIFY_FROM` and the optional
  `APEX_NOTIFY_SENDMAIL` for the alerts, `APEX_WATCH_PAGE_URL` or
  `APEX_CONSOLE_URL` for the link line in the alert copy, and
  `APEX_BILLING_PROFILE` plus `APEX_BILLING_VAT_RATE_BP` for the monthly
  issuance. The watch holds **no platform management key and needs no
  runner**: it reads public `app-ads.txt` files through the system `curl`
  (the host needs `curl`) and issues through the same
  `.apex/billing/invoices.jsonl` register the runner uses.
- `sync/`: the one-way sync tools. `push.sh` uploads the console host's
  ledger directory, invoice register file and leads (+signin) files to an
  S3-compatible Object Storage bucket with rclone, then a `manifest.json`
  carrying the size and sha256 of every object; `pull.sh` runs on the control
  host, verifies every object against the manifest, installs the ledger
  root, the invoice register and the sales imports directory, and writes the
  sync marker atomically last. `sync.env.sample` is the `/etc/apex/sync.env`
  template, and `tests/` holds the committed fake rclone shim and
  `run-tests.sh`.
- `woodpecker/`: the self-hosted CI server and agent.

Build and roll out:

```sh
cd site && zola build            # writes site/public
install -m 0755 target/release/apex-api /usr/local/bin/apex-api
install -m 0755 target/release/apex-control /usr/local/bin/apex-control
install -m 0755 target/release/apex-ingest /usr/local/bin/apex-ingest
install -m 0755 target/release/apex-runner /usr/local/bin/apex-runner
install -m 0755 target/release/apex-sales /usr/local/bin/apex-sales
install -m 0755 target/release/apex-sales-web /usr/local/bin/apex-sales-web
install -m 0755 target/release/apex-watch /usr/local/bin/apex-watch
install -m 0755 infrastructure/sync/push.sh /usr/local/bin/apex-sync-push
install -m 0755 infrastructure/sync/pull.sh /usr/local/bin/apex-sync-pull
install -d -o caddy -g caddy /var/log/caddy
install -d -o apex -g apex /var/lib/apex/.apex/analytics-spool /var/lib/apex/.apex/control
install -d -o apex -g apex /var/lib/apex/.apex/ledgers /var/lib/apex/.apex/billing /var/lib/apex/.apex/imports
install -d -o apex -g apex -m 0755 /var/lib/node_exporter/textfile
install -m 0644 infrastructure/systemd/apex-api.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-control.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-ingest.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-ingest.timer /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-daily.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-daily.timer /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-monthly.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-monthly.timer /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-sales-run.service infrastructure/systemd/apex-sales-run.timer /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-sales-discover.service infrastructure/systemd/apex-sales-discover.timer /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-sales-web.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-watch.service infrastructure/systemd/apex-watch.timer infrastructure/systemd/apex-watch-bill.service infrastructure/systemd/apex-watch-bill.timer /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-sync-push.service infrastructure/systemd/apex-sync-push.timer infrastructure/systemd/apex-sync-pull.service infrastructure/systemd/apex-sync-pull.timer /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now apex-ingest.timer apex-runner-daily.timer apex-runner-monthly.timer
systemctl enable --now apex-sales-discover.timer apex-sales-run.timer   # starts in shadow mode
systemctl enable --now apex-sales-web
systemctl enable --now apex-watch.timer apex-watch-bill.timer
systemctl enable --now apex-sync-push.timer   # console host
systemctl enable --now apex-sync-pull.timer   # control host
systemctl restart apex-api        # console host
systemctl restart apex-control    # control host
cd monitoring && envsubst < alertmanager.yml > alertmanager.generated.yml
docker compose up -d
```

Start Caddy with `SITE_DOMAIN`, `APEX_DOMAIN` and `APEX_ACCESS_LOG` in its
environment (see `caddy/README.md`); the access-log directory must exist
first.

## Hosting split

The product runs on two hosts. The console host keeps the customer-facing
edge, apex-api and the runner jobs that write the shared files; the control
host runs the operator plane and the analytics store and reads one-way copies
of those files from the Object Storage bucket. Only the push exports, only
the pull imports, and the pull side accepts an object only after its sha256
matches the manifest.

Console host:

| service | unit(s) | environment file |
|---|---|---|
| `apex-api` (publisher console and JSON API, including the operator jobs endpoint) | `apex-api.service` | `/etc/apex/api.env` |
| `apex-ingest` (Caddy access log to ClickHouse) | `apex-ingest.service` and `.timer` | `/etc/apex/ingest.env` |
| `apex-runner` jobs (audit, cycle, daily, monthly) | `apex-runner-*.service` and `.timer` | `/etc/apex/runner.env` |
| `apex-watch` (setup-watch daily check and monthly billing) | `apex-watch.service` / `apex-watch-bill.service` and `.timer` | `/etc/apex/watch.env` |
| `apex-sync-push` | `apex-sync-push.service` and `.timer` | `/etc/apex/sync.env` |

Caddy runs next to apex-api on the console host. `APEX_CLICKHOUSE_HTTP`
points at the control host's store over the private network, so apex-api,
apex-ingest and the runner keep their spools and read models unchanged.

Control host:

| service | unit(s) | environment file |
|---|---|---|
| `apex-control` (operator plane, loopback only) | `apex-control.service` | `/etc/apex/control.env` |
| ClickHouse (analytics and event store) | `clickhouse-server` (compose) | deployment environment |
| `apex-sales` (discovery and sending) | `apex-sales-run.service` / `apex-sales-discover.service` and `.timer` | `/etc/apex/sales.env` |
| `apex-sales-web` (owner workspace UI, loopback only) | `apex-sales-web.service` | `/etc/apex/sales-web.env` |
| `apex-sync-pull` | `apex-sync-pull.service` and `.timer` | `/etc/apex/sync.env` |

### Environment inventory split

Neither host carries the other's credentials:

| variable family | console host | control host |
|---|---|---|
| platform keys (`APEX_VAULT_KEY_HEX`, `APEX_CREDENTIAL_ROOT`, `APEX_MAX_*`, `APEX_ADYEN_*`, `APEX_RUNNER_SIGNING_KEY_HEX`, the rest of `runner.env`) | present | must not be present |
| operator credentials (`APEX_CONTROL_OPERATORS`, `APEX_SALES_WEB_TOKEN`) | must not be present | present |
| jobs channel bearer `APEX_OPERATOR_JOBS_TOKEN` | present in `api.env`, the endpoint gate | present in `control.env`, the client |
| sync configuration (`APEX_SYNC_REMOTE`, the paths, `RCLONE_CONFIG`) | `/etc/apex/sync.env` (push side) | `/etc/apex/sync.env` (pull side) |

The one deliberate overlap is `APEX_OPERATOR_JOBS_TOKEN`: apex-api gates its
operator jobs endpoint with it, and apex-control presents the same value when
it talks to that endpoint. The exact remote-jobs contract: with
`APEX_API_JOBS_URL` set to the apex-api origin reachable from the control
host, `GET` and `POST /v1/operator/jobs` on apex-api serve the operator jobs;
without the variable the control plane keeps its local
`.apex/control/jobs` directory.

### Metrics rule

`/metrics` never crosses a public origin. The API host refuses the path with
a 404 before the catch-all proxy and apex-api serves it on its loopback bind;
the edge contract fails if `/metrics` is proxied on either host or if any
port other than `8787` appears in the config. After the split the scrape of
`apex-api` and the textfile exports happen on each host's private boundary,
and apex-control exposes no `/metrics` route at all (textfile export only,
see `monitoring/README.md`).

### Sync flow

`apex-sync-push.timer` runs on the console host at `*:0,15,30,45` and
`apex-sync-pull.timer` on the control host at `*:5,20,35,50`. Both units are
oneshot, run as `apex` with `ProtectSystem=strict`, `PrivateTmp` and no new
privileges; the pull unit's writable paths are the four destination trees
under `/var/lib/apex`. `sync.env.sample` is the `/etc/apex/sync.env`
template; rclone and python3 are the runtime dependencies, and the bucket
credentials stay in the rclone configuration file named by `RCLONE_CONFIG`
(never on a command line).

| bucket prefix | console source | control destination |
|---|---|---|
| `ledger/` | `APEX_SYNC_LEDGER_ROOT` (default `.apex/ledgers`) | `APEX_CONTROL_LEDGER_ROOT` (default `.apex/ledgers`) |
| `invoices/` | `APEX_SYNC_INVOICE_REGISTER` (default `.apex/billing/invoices.jsonl`) | `APEX_CONTROL_INVOICE_REGISTER` (default `.apex/billing/invoices.jsonl`) |
| `leads/` | `APEX_SYNC_LEADS_FILE` (default `.apex/leads.jsonl`) and the `signin.jsonl` beside it | `APEX_SYNC_IMPORTS_DIR` (default `.apex/imports`), read by apex-sales through `APEX_SALES_LEADS_FILE` |

The push writes `manifest.json` last, with the size and sha256 of every
object. The pull fetches the manifest first, stages every object it names,
verifies size and sha256, and only then installs the files and writes the
marker `APEX_CONTROL_SYNC_MARKER` (default
`.apex/control/sync-marker.json`) atomically as
`{"synced_at_unix_ms":N,"source":"ledger|invoices|leads"}`, naming the
families actually installed. A failed run exits nonzero before the marker
step, and after a failed verification nothing is installed: the marker keeps
describing the last complete sync, so it can never claim fresh data over a
stale tree. `sh infrastructure/sync/push.sh --dry-run` prints the uploads
without writing them; any nonzero exit fails the unit. The scripts are
exercised by `sh infrastructure/sync/tests/run-tests.sh` against a committed
fake rclone shim: success, hash mismatch refusal, partial-copy marker
suppression and empty-bucket refusal.

## Access log (marketing origin)

Caddy writes one JSON record per marketing request to
`APEX_ACCESS_LOG` (default `/var/log/caddy/apex-access.json`).
`apex-ingest` reads that file hourly and reduces each record with the
parser's allow-list: the route template, the status, the referrer host, the
campaign code and allow-listed query parameter **keys**. Query values, client
addresses, raw user agents and raw paths never enter the analytics store;
this is the data-minimization boundary of the design (section 05).

At rest the raw file necessarily holds the full request line (including the
query string) and the standard Caddy request fields, because the parser is
the component that performs the redaction. Treat the file accordingly:

- Own the directory `caddy:caddy` with mode `0755` and the file `0640`;
  add the `apex` user to the `caddy` group (or grant an equivalent ACL) so
  the ingest can read it.
- Rotate it externally (for example `logrotate` with `copytruncate`); Caddy
  appends one JSON object per line.
- Keep rotation within the deployment's retention policy for raw request
  logs; the store side keeps only bounded fields (sanitized request facts
  have a 30-day retention in the store schema).

The ingest spools failed batches to `.apex/analytics-spool/apex-ingest.jsonl`
when ClickHouse is unreachable and replays them on the next run; replays
deduplicate in the store by `(source, event_id)`, so reading the whole file
on every run is safe.

## Analytics store (ClickHouse)

ClickHouse is the analytics/event store for the product (it replaces the
historical Postgres analytics tables; the event catalog, retention proposal
and acceptance checks live in section 05 of the design document). Collection
is server-side only: Axum services and workers write events; the Zola site
stays static and emits nothing by itself.

What runs where:

- Local development and integration tests use a real single-node server run
  by the unified `clickhouse` binary with `infrastructure/clickhouse/config.xml`.
  It binds HTTP to `127.0.0.1:8123` and native TCP to `127.0.0.1:9000`, has a
  single passwordless `default` user, and keeps data, logs and temp files
  under `.apex/clickhouse/` (gitignored). That credential story is local-dev
  only: never copy this configuration onto a shared host.
- Shared deployments and CI service containers use
  `infrastructure/clickhouse/docker-compose.yml` (the
  `clickhouse/clickhouse-server` image, named volume, loopback-only published
  ports). Host ports are overridable with `APEX_CLICKHOUSE_HTTP_PORT` and
  `APEX_CLICKHOUSE_TCP_PORT` (defaults 8123 and 9000). Real credentials come
  from the deployment environment, never from the compose file. Settings
  overrides mount into `/etc/clickhouse-server/config.d/` as noted in the
  compose file.

Make targets:

- `make clickhouse-dev` starts the server in the foreground
  (`clickhouse server --config-file infrastructure/clickhouse/config.xml`).
  Run it from the repository root in a terminal or a supervisor; the config
  resolves `.apex/clickhouse/` paths relative to the current directory, so
  the server must be started from the repo root.
- `make clickhouse-stop` stops it gracefully with
  `clickhouse client --port 9000 --query "SYSTEM SHUTDOWN"`. There is no
  process-signal fallback: a pattern kill could take down an unrelated
  server on a shared host. If the client cannot connect, the recipe stops
  nothing and says so; check whether the server is down, and if it was
  started elsewhere, stop it wherever it runs.

First run: you do not hand-create application databases or tables. Start the
server, then let the application create its database and tables through its
own bootstrap/migration path. The smoke check below only proves the server
is reachable.

Environment variables:

- `APEX_CLICKHOUSE_HTTP` (convention: `http://127.0.0.1:8123`): the endpoint
  clients use for HTTP queries.
- `APEX_CLICKHOUSE_HTTP_PORT`, `APEX_CLICKHOUSE_TCP_PORT`: host port
  overrides for the compose deployment.

Verify with curl (the `1` is the expected output):

```sh
curl -s 'http://127.0.0.1:8123/?query=SELECT%201'
```

Connection contract for tests and CI: integration tests read
`APEX_CLICKHOUSE_HTTP` (default `http://127.0.0.1:8123`), connect as
`default` with an empty password, and skip with a clear message when the
endpoint is unreachable. CI runs a ClickHouse service container; point
`APEX_CLICKHOUSE_HTTP` at it and the same tests run unchanged. Tests and CI
do not read `APEX_CLICKHOUSE_HTTP_PORT` or `APEX_CLICKHOUSE_TCP_PORT`; those
only steer the compose deployment's host port bindings.
