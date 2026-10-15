# Monitoring

The operator stack watches the product services: `apex-api` (publisher
console + JSON API), `apex-control` (operator plane) and the scheduled
`apex-runner` jobs.

- `prometheus.yml` has two scrape jobs in the existing pattern: `apex-api`
  on `/metrics` (`host.docker.internal:8787`), and `node` on
  `host.docker.internal:9100`, where node_exporter's textfile collector
  carries the runner's bounded series, the `apex-control` series (the
  operator plane is textfile-only) and the same `apex-api` registry series
  as a fallback path. `APEX_METRICS_TEXTFILE` names the exported FILE, not a
  directory: set it to `/var/lib/node_exporter/textfile/apex-api.prom` for
  apex-api, `/var/lib/node_exporter/textfile/apex-control.prom` for
  apex-control and `/var/lib/node_exporter/textfile/apex-runner.prom` for
  apex-runner, inside whatever directory the collector reads on the host.
  Writes are atomic (temp file plus rename); apex-api and apex-control
  refresh every 15 seconds and once more on graceful shutdown, apex-runner
  after every job.
- `alerts.yml` and `recording-rules.yml` are loaded and evaluated by
  Prometheus; `alerts.test.yml` pins every alert with `promtool test rules`.

What the rules are written against (verified in code, one spelling only):

- `apex_api_requests_total{status}` / `apex_api_requests_in_flight`:
  apex-api's hand-rolled counters, kept during the metrics migration
  ("Keep existing counters during migration", page 111).
- `apex_http_requests_total{service,route_template,method,status_class}`,
  `apex_http_request_duration_seconds{service,route_template,method}` and
  `apex_http_requests_in_flight{service}`: the bounded `apex-metrics` series
  from apex-api (`service="apex-api"`) and apex-control
  (`service="apex-control"`). Only matched route templates become labels;
  unmatched paths stay in the hand-rolled totals.
- `apex_connector_requests_total{platform,operation,outcome_class}` /
  `apex_connector_duration_seconds{platform,operation}`: one observation per
  platform call attempt, written by the metered transport inside the retry
  decorator.
- `apex_job_runs_total{job_type,platform,outcome}`,
  `apex_job_duration_seconds{job_type,platform,outcome}` and
  `apex_job_schedule_lag_seconds{job_type,platform}`: the runner's job
  accounting.
- `apex_report_accounts{platform,readiness_class}`: the accounting
  snapshot's readiness gauge (final, partial, stale, disconnected).
- `apex_audit_runs_total`, `apex_experiment_decisions_total`,
  `apex_guardrail_events_total`, `apex_restore_operations_total`,
  `apex_ledger_verifications_total`, `apex_payment_events_total`,
  `apex_notification_attempts_total` and `apex_operator_actions_total`: the
  domain counters, all with bounded label vocabularies only.

Declared in the catalog but not written by any process yet, so no rule
targets them (a rule over a series nothing emits would never fire):

- `apex_jobs_overdue`: the scheduler workstream's gauge; systemd timers
  invoke the runner, nothing sets it. The overdue signal today is the
  schedule-lag recording rule.
- `apex_projection_lag_seconds`: no projection writer runs on a schedule
  yet.

Run and test:

```sh
cd monitoring
envsubst < alertmanager.yml > alertmanager.generated.yml
docker compose up -d
promtool check config prometheus.yml
promtool test rules alerts.test.yml
```

Keep every value in the environment: the file has no credentials of its own.

## Alert list

| alert | signal | severity |
|---|---|---|
| `ApexApiDown` | apex-api scrape failing for 2 minutes | critical |
| `ApexApiServerErrorRateHigh` | hand-rolled 5xx ratio above 5 percent for 10 minutes | warning |
| `ApexHttpServerErrorRateHigh` | service route class 5xx ratio above 5 percent with at least 3 errors a minute for 10 minutes | warning |
| `ApexHttpLatencyHigh` | service p95 response time above 1 second for 10 minutes | warning |
| `ApexJobScheduleLagLate` | a job start more than 1 hour late within the last 6 hours | warning |
| `ApexReportAccountsStale` | a platform's accounts stale for 1 hour | warning |
| `ApexRestoreFailures` | a restore or reconcile failure in the last 6 hours | critical |
| `ApexLedgerVerificationFailures` | a non-verified ledger result in the last 6 hours | critical |
| `ApexNotificationFailures` | a failed notification attempt in the last 6 hours | warning |
| `ApexConnectorAuthRefusals` | auth refusals above one a minute for 15 minutes | warning |
| `ApexConnectorLatencyHigh` | platform call p95 above 10 seconds for 10 minutes | warning |
| `ApexOperatorActionsDeniedSpike` | more than 10 denied operator actions in 15 minutes | warning |

## Recording rules

| rule | meaning |
|---|---|
| `apex:request_rate5m` | total apex-api request rate |
| `apex:error_ratio5m` | apex-api hand-rolled 5xx ratio |
| `apex:http_server_error_ratio5m` | 5xx ratio per service and route template |
| `apex:http_latency_p95_5m` | p95 response time per service and route template |
| `apex:job_schedule_lag_over_1h_runs_6h` | job starts more than an hour late in 6 hours |
| `apex:job_runs_rate30m` | job run rate per job type and outcome |
| `apex:connector_latency_p95_10m` | p95 platform call duration per platform |
| `apex:connector_auth_refusal_rate30m` | auth refusal rate per platform |
| `apex:connector_requests_rate30m` | platform request rate per operation and outcome class |
| `apex:audit_runs_rate30m` | audit run rate per rule release and outcome |
| `apex:experiment_decisions_rate30m` | decision rate per decision and reason class |
| `apex:guardrail_events_rate30m` | fired guardrail rate per guardrail and action |
| `apex:restore_operations_rate30m` | restore and reconcile rate per operation and outcome |
| `apex:ledger_verifications_rate30m` | ledger verification rate per result and verifier release |
| `apex:payment_events_rate30m` | provider payment event rate per class and result |
| `apex:notification_attempts_rate30m` | notification attempt rate per channel, template and outcome |
| `apex:operator_actions_rate30m` | operator action rate per capability, action and outcome |
