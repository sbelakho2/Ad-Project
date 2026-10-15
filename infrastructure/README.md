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
  `apex-runner-cycle.timer` runs the write cycle (propose from the account
  snapshot and start pre-registered experiments) at 03:00 UTC;
  `apex-runner-daily.timer` runs the pull/settle pass at 03:30 UTC;
  `apex-runner-monthly.timer` bills the month just closed and collects through
  Adyen at 04:30 UTC on the first. All read `/etc/apex/runner.env`:
  `APEX_RUNNER_LEDGER`, `APEX_RUNNER_PUBLISHER`, `APEX_RUNNER_SIGNING_KEY_HEX`,
  `APEX_MAX_API_KEY`, `APEX_MAX_REPORT_KEY`, `APEX_RUNNER_WRITABLE`,
  `APEX_RUNNER_POLICY`, `APEX_RUNNER_PRIORS`, `APEX_RUNNER_SNAPSHOT`,
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
