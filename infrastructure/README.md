# Infrastructure

Current deployment layout for the no-SDK autopilot product:

- `deployment.toml`: the deployment inventory (service, site, edge,
  monitoring paths; no secrets).
- `caddy/`: the edge config. One host reverse-proxies `apex-api`, the other
  serves the built Zola site.
- `systemd/apex-api.service`: the service unit; the environment file holds
  the vault key, account root, live-audit token, per-publisher ledger tokens
  (`APEX_LEDGER_TOKENS_FILE`, a JSON object of publisher to token) and Adyen
  settings. Without the ledger tokens file, no ledger is readable over HTTP.
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
  `APEX_MONITORING_MONTHLY_MICROS`, `APEX_ALERT_FILE`.
- `woodpecker/`: the self-hosted CI server and agent.

Build and roll out:

```sh
cd site && zola build            # writes site/public
install -m 0755 target/release/apex-api /usr/local/bin/apex-api
install -m 0755 target/release/apex-runner /usr/local/bin/apex-runner
install -m 0644 infrastructure/systemd/apex-api.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-daily.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-daily.timer /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-monthly.service /etc/systemd/system/
install -m 0644 infrastructure/systemd/apex-runner-monthly.timer /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now apex-runner-daily.timer apex-runner-monthly.timer
systemctl restart apex-api
cd monitoring && envsubst < alertmanager.yml > alertmanager.generated.yml
docker compose up -d
```

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
  `clickhouse client --port 9000 --query "SYSTEM SHUTDOWN"`. If the client
  cannot connect, the recipe falls back to `pkill -f 'clickhouse server'`
  (unclean last resort; prefer the graceful path).

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
