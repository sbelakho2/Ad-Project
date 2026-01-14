# On-Call Runbook — ApexMediation

> **VERIFY-FIRST:** This runbook demonstrates compliance with Part 0 (escalation/on-call rotation). Confirm entries here before editing deployment docs.

## Rotation & Tooling

- **Primary Schedule:** PagerDuty service `ApexMediation-Prod`, weekly rotation (Monday 00:00 UTC) across SRE engineers.
- **Secondary:** Backend on-call (auction/adapter specialist) auto-added as responder for SEV1/SEV2.
- **Tertiary:** SDK platform delegate for surface-specific incidents.
- **Escalation Policy:** PagerDuty → SMS/phone → fallback bridge `meet.bel.consulting/oncall`.

## Expectations

| Task | SLA |
| --- | --- |
| Ack PagerDuty alert | ≤ 5 minutes |
| Join incident bridge | ≤ 10 minutes |
| Publish status page update (`docs/Monitoring/STATUS_PAGE_AND_INCIDENT_TAXONOMY.md`) | ≤ 15 minutes |
| File RCA issue (template `ISSUE_TEMPLATE/rca.md`) for SEV1/SEV2 | within 48 hours |

## Runbook Steps

1. **Triage**
   - Check Grafana dashboards (`monitoring/grafana-dashboards.yml`).
   - Pull recent service events if deployment-related (`pm2 logs`, `journalctl`, and Caddy logs).
2. **Stabilize**
   - Use kill-switches (`backend/Adapters.md` section 6) for faulty bidders.
   - Trigger rollback via `deploy-production.yml` workflow dispatch if release regression.
3. **Communicate**
   - Update status page entry with SEV, scope, ETA.
   - Notify Customer Success in `#customer-sos` Slack channel.
4. **Escalate**
   - Loop in Privacy/Security for PII or key incidents.
   - Loop in Billing for invoicing/usage discrepancies.
5. **Close & Follow-Up**
   - Verify metrics back to baseline.
   - Create RCA issue and link fixes in `docs/Internal/Development/FIXES.md`.

## Checklists

- [ ] PagerDuty contact info verified weekly.
- [ ] Status page credentials accessible (GitHub token + Pages access).
- [ ] Incident templates tested monthly (see `docs/Monitoring/ALERTS.md`).

## References

- `docs/Monitoring/ALERTS.md`
- `docs/Monitoring/STATUS_PAGE_AND_INCIDENT_TAXONOMY.md`
- `docs/Internal/Deployment/ROLLOUT_STRATEGY.md`
- `.github/workflows/rollback.yml`
