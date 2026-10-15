# Infrastructure

Current deployment layout for the no-SDK autopilot product:

- `deployment.toml`: the deployment inventory (service, site, edge,
  monitoring paths; no secrets).
- `caddy/`: the edge config. One host reverse-proxies `apex-api`, the other
  serves the built Zola site.
- `systemd/apex-api.service`: the service unit; the environment file holds
  the vault key, account root, live-audit token and Adyen settings.
- `systemd/apex-runner-*.service` and `*.timer`: the scheduled measurement and
  billing jobs. `apex-runner-daily.timer` runs the daily pull/settle pass at
  03:30 UTC; `apex-runner-monthly.timer` bills the month just closed at 04:30
  UTC on the first. Both read `/etc/apex/runner.env` (ledger path, signing
  key, MAX keys, verified operations, optional policy).
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
