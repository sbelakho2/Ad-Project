# Monitoring

The operator stack watches the product only: `apex-api` on
`127.0.0.1:8787` and the alerts that matter for it.

- `prometheus.yml` scrapes `apex-api` at `/metrics` (request counters and
  uptime) and evaluates `alerts.yml` and `recording-rules.yml`.
- `alerts.yml` fires `ApexApiDown` and `ApexApiServerErrorRateHigh`.
  `alerts.test.yml` pins both with `promtool test rules`.
- `alertmanager.yml` is a template: render it before starting the stack.

```sh
cd monitoring
envsubst < alertmanager.yml > alertmanager.generated.yml
docker compose up -d
promtool check config prometheus.yml
promtool test rules alerts.test.yml
```

Keep every value in the environment: the file has no credentials of its own.
