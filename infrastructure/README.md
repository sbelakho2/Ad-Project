# Infrastructure

Current deployment layout for the no-SDK autopilot product:

- `deployment.toml`: the deployment inventory (service, site, edge,
  monitoring paths; no secrets).
- `caddy/`: the edge config. One origin layout on three hosts: the marketing
  host serves the built Zola site and proxies only `POST /free-audit`,
  `/audit-requested/*` and `/console*` to `apex-api`; the API host fronts
  `apex-api` whole; the operator host fronts `apex-control` alone. Host names
  come from `SITE_DOMAIN`, `APEX_DOMAIN` and `CONTROL_DOMAIN`; the marketing
  JSON access log path comes from `APEX_ACCESS_LOG` (default
  `/var/log/caddy/apex-access.json`). The routing contract lives in
  `infrastructure/caddy/tests/contract.sh` and runs in CI.
- `systemd/apex-api.service`: the service unit; the environment file holds
  the vault key, account root, live-audit token, per-publisher ledger tokens
  (`APEX_LEDGER_TOKENS_FILE`, a JSON object of publisher to token) and Adyen
  settings. Without the ledger tokens file, no ledger is readable over HTTP.
  The optional `APEX_METRICS_TEXTFILE` names the exported FILE (for example
  `/var/lib/node_exporter/textfile/apex-api.prom`), which the unit's
  `ReadWritePaths` covers; see `monitoring/README.md`.
- `systemd/apex-control.service`: the operator origin; reads
  `/etc/apex/control.env` and runs from `/var/lib/apex`, where the
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
  `/var/lib/node_exporter/textfile/apex-control.prom`). The unit pins
  `WorkingDirectory=/var/lib/apex`, so the relative defaults resolve under
  the state root, and its `ReadWritePaths` cover the state directory and the
  textfile directory so the atomic export (temp file plus rename) writes
  there.
- `systemd/apex-ingest.service` and `apex-ingest.timer`: the hourly
  access-log ingest. The oneshot runs
  `apex-ingest /var/log/caddy/apex-access.json --spool
  /var/lib/apex/.apex/analytics-spool/apex-ingest.jsonl`; failed batches
  spool for the next run and replays deduplicate in the store by
  `(source, event_id)`. `/etc/apex/ingest.env` carries
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
  `APEX_RUNNER_LEDGER`, `APEX_RUNNER_PUBLISHER`, `APEX_RUNNER_SIGNING_KEY_HEX`,
  `APEX_MAX_API_KEY`, `APEX_MAX_REPORT_KEY`, `APEX_RUNNER_WRITABLE`,
  `APEX_RUNNER_POLICY`, `APEX_RUNNER_PRIORS`, `APEX_RUNNER_SNAPSHOT`,
  `APEX_DEVELOPER_SITE`, `APEX_EXPECTED_NETWORKS`,
  `APEX_ADYEN_API_KEY`, `APEX_ADYEN_MERCHANT_ACCOUNT`, `APEX_ADYEN_HMAC_KEY`,
  `APEX_ADYEN_CHECKOUT_BASE`, `APEX_INVOICE_VAT_BP`,
  `APEX_MONITORING_MONTHLY_MICROS`, `APEX_ALERT_FILE`,
  `APEX_CLICKHOUSE_HTTP`, `APEX_ANALYTICS_SPOOL`, `APEX_METRICS_TEXTFILE`
  and `APEX_SCHEDULED_AT_MS`.

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
  `APEX_SCHEDULED_AT_MS` is the expected timer fire time in unix milliseconds
  for the schedule-lag histogram; the runner records lag only when this
  variable or the `--scheduled-at` flag is supplied, because systemd's
  specifier set has no timer-fire-time specifier to pass through `ExecStart`.
- `woodpecker/`: the self-hosted CI server and agent.

Build and roll out:

```sh
cd site && zola build            # writes site/public
install -m 0755 target/release/apex-api /usr/local/bin/apex-api
install -m 0755 target/release/apex-control /usr/local/bin/apex-control
install -m 0755 target/release/apex-ingest /usr/local/bin/apex-ingest
install -m 0755 target/release/apex-runner /usr/local/bin/apex-runner
install -d -o caddy -g caddy /var/log/caddy
install -d -o apex -g apex /var/lib/apex/.apex/analytics-spool /var/lib/apex/.apex/control
install -d -o apex -g apex -m 0755 /var/lib/node_exporter/textfile
install -m 0644 infrastructure/systemd/apex-api.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-control.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-ingest.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-ingest.timer /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-daily.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-daily.timer /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-monthly.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-monthly.timer /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now apex-ingest.timer apex-runner-daily.timer apex-runner-monthly.timer
systemctl restart apex-api apex-control
cd monitoring && envsubst < alertmanager.yml > alertmanager.generated.yml
docker compose up -d
```

Start Caddy with `SITE_DOMAIN`, `APEX_DOMAIN`, `CONTROL_DOMAIN` and
`APEX_ACCESS_LOG` in its environment (see `caddy/README.md`); the access-log
directory must exist first.

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
