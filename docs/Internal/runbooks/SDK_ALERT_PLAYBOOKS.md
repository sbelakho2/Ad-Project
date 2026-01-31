# SDK Alert Playbooks

Last updated: 2025-01-06
Owner: SDK Engineering

## Overview

This document provides actionable runbooks for responding to SDK-related alerts across all platforms (iOS, Android, tvOS, Android TV, Unity, Web). Each playbook includes symptoms, diagnosis steps, and remediation actions.

---

## Table of Contents

1. [Per-Adapter Alerts](#per-adapter-alerts)
2. [Network & Connectivity Alerts](#network--connectivity-alerts)
3. [Cache & Performance Alerts](#cache--performance-alerts)
4. [Mediation Alerts](#mediation-alerts)
5. [Circuit Breaker Alerts](#circuit-breaker-alerts)
6. [SDK Health Alerts](#sdk-health-alerts)
7. [Platform-Specific Alerts](#platform-specific-alerts)

---

## Per-Adapter Alerts

### ALERT: adapter_latency_p99_high

**Severity**: WARN (≥600ms) / CRIT (≥1000ms)

**Description**: 99th percentile latency for an adapter exceeds thresholds.

**Symptoms**:
- Slow ad loads reported by users
- Increased time-to-first-ad metrics
- Waterfall timeouts before reaching all adapters

**Diagnosis**:
```
1. Check AdapterMetrics.getSnapshot(adapterId)
   - Review p50, p95, p99 latency values
   - Compare against baseline (last 7 days)
   
2. Check network conditions
   - NetworkMonitor.getConnectionType()
   - Look for cellular vs. WiFi correlation
   
3. Verify partner endpoint health
   - Check partner status page
   - Review MediationDebugger for error patterns
   
4. Check concurrency settings
   - AdaptiveConcurrency current limits
   - Look for connection pool exhaustion
```

**Remediation**:
1. **Short-term**: Reduce timeout for adapter, allow waterfall to proceed faster
2. **Consider hedging**: Enable `AUCTION_HEDGING_ENABLED` for critical adapters
3. **Increase timeout**: If partner is generally slow but reliable
4. **Deprioritize**: Move adapter lower in waterfall if consistently slow
5. **Contact partner**: If latency is sustained >2 hours

**Escalation**: If CRIT persists >30 minutes, page on-call

---

### ALERT: adapter_error_rate_high

**Severity**: WARN (≥5%) / CRIT (≥10%)

**Description**: Error rate for an adapter exceeds thresholds.

**Symptoms**:
- Increased no-fill rates
- HTTP 4xx/5xx errors in logs
- Circuit breaker activations

**Diagnosis**:
```
1. Check error breakdown by type:
   - AdapterMetrics.getErrorBreakdown(adapterId)
   - 4xx = configuration/auth issues
   - 5xx = partner-side issues
   - Network errors = connectivity issues
   
2. Verify configuration:
   - Check API keys, placement IDs
   - Verify endpoint URLs haven't changed
   
3. Check for SDK version issues:
   - Recent SDK update?
   - Partner SDK version compatibility?
```

**Remediation**:
| Error Type | Action |
|------------|--------|
| 401/403 | Rotate API keys, verify account status |
| 404 | Confirm placement ID, check endpoint URL |
| 429 | Reduce request rate, enable rate limiting |
| 500 | Wait for partner recovery, reduce traffic |
| Network | Check connectivity, verify DNS |

**Escalation**: For 4xx errors, involve partner integration team. For 5xx, monitor partner status page.

---

### ALERT: adapter_fill_rate_low

**Severity**: WARN (≤20%) / CRIT (≤5%)

**Description**: Fill rate for an adapter dropped below thresholds.

**Symptoms**:
- More ad requests than responses
- NoFillTracker showing elevated patterns
- Revenue below projections

**Diagnosis**:
```
1. Check NoFillTracker patterns:
   - NoFillTracker.getAnalysis()
   - Look for consecutive failures
   - Check hourly/daily trends
   
2. Verify bid floors:
   - Compare floor prices vs. market
   - Check below_floor reason in metrics
   
3. Check inventory quality:
   - Viewability scores
   - Ad format compatibility
   
4. Analyze geo distribution:
   - Fill rates by region
   - Partner coverage by market
```

**Remediation**:
1. **Lower floors**: If `below_floor` is primary reason
2. **Expand formats**: Enable additional ad formats (interstitial, rewarded)
3. **Add adapters**: Increase waterfall depth
4. **Segment traffic**: Route premium inventory to bidders, remnant to networks

---

## Network & Connectivity Alerts

### ALERT: network_offline_mode_active

**Severity**: INFO (brief) / WARN (>5 min) / CRIT (>15 min)

**Description**: SDK is operating in offline fallback mode.

**Symptoms**:
- NetworkMonitor.isOffline() returns true
- Cached ads being served
- No new ad requests

**Diagnosis**:
```
1. Check network status:
   - NetworkMonitor.getConnectionInfo()
   - Test DNS resolution
   - Test API endpoint reachability
   
2. Verify device connectivity:
   - WiFi enabled?
   - Cellular data enabled?
   - VPN interference?
```

**Remediation**:
1. **Wait**: Brief outages are normal, SDK handles gracefully
2. **Warm cache**: Ensure `AdCacheTTL` settings allow sufficient coverage
3. **Preload**: Use `NetworkWarmer` to speed recovery when online
4. **User notification**: Consider informing user if persists >30 seconds

---

### ALERT: dns_prefetch_failures

**Severity**: WARN (>10 failures/hour)

**Description**: DNS prefetch for ad endpoints failing.

**Symptoms**:
- NetworkWarmer connection failures
- Increased ad load latency
- First-ad-load particularly slow

**Diagnosis**:
```
1. Check prefetch targets:
   - NetworkWarmer.getPrefetchStats()
   - Identify failing domains
   
2. Test DNS resolution:
   - nslookup <domain>
   - Check for NXDOMAIN or timeouts
   
3. Check ISP/network issues:
   - Are multiple users affected?
   - Regional DNS outage?
```

**Remediation**:
1. **Fallback DNS**: Configure backup DNS (8.8.8.8, 1.1.1.1)
2. **Cache DNS**: Extend DNS TTL in client
3. **Direct IP**: For critical endpoints, consider IP fallback

---

## Cache & Performance Alerts

### ALERT: ad_cache_exhausted

**Severity**: WARN (cache empty) / CRIT (>3 consecutive requests with empty cache)

**Description**: Ad cache is empty when ads are needed.

**Symptoms**:
- Blank ad slots during network issues
- Slow ad loads on first request
- User waiting for ads

**Diagnosis**:
```
1. Check cache status:
   - AdCacheTTL.getCacheStats()
   - Check hit rate, eviction rate
   
2. Verify TTL settings:
   - Are TTLs appropriate for usage patterns?
   - Check max/min TTL clamping
   
3. Check prefetch behavior:
   - Is prefetching enabled?
   - Check prefetch timing
```

**Remediation**:
1. **Increase cache size**: Allow more cached ads
2. **Extend TTL**: If ads aren't sensitive to freshness
3. **Prefetch earlier**: Trigger prefetch before cache runs low
4. **Add fallback creatives**: House ads as last resort

---

### ALERT: concurrency_limit_saturated

**Severity**: WARN (>80% capacity) / CRIT (100% for >1 min)

**Description**: Adaptive concurrency limiter is at maximum capacity.

**Symptoms**:
- Requests queuing
- Increased latency
- Connection pool warnings

**Diagnosis**:
```
1. Check concurrency stats:
   - AdaptiveConcurrency.getStats()
   - Review current vs. max connections
   
2. Check for slow adapters:
   - Are some adapters holding connections?
   - Check individual adapter latencies
   
3. Check traffic patterns:
   - Traffic spike?
   - Campaign launch?
```

**Remediation**:
1. **Increase limits**: If server can handle more
2. **Reduce parallel adapters**: Serialize slower adapters
3. **Enable connection pooling**: Reuse connections
4. **Scale horizontally**: Add more SDK instances

---

## Mediation Alerts

### ALERT: waterfall_exhausted

**Severity**: WARN (>10% of requests) / CRIT (>25% of requests)

**Description**: All adapters in waterfall returned no-fill.

**Symptoms**:
- Blank ad slots
- Low overall fill rate
- Revenue below expected

**Diagnosis**:
```
1. Check FallbackWaterfall logs:
   - Which adapters were tried?
   - What were the failure reasons?
   
2. Check PriorityWeightedMediation:
   - Are priorities configured correctly?
   - Check weight distribution
   
3. Check individual adapter health:
   - Are specific adapters failing?
   - Circuit breaker status?
```

**Remediation**:
1. **Add adapters**: Expand waterfall depth
2. **Reduce floors**: Lower minimum bid thresholds
3. **Enable backfill**: Add house ads at end of waterfall
4. **Diversify**: Add programmatic/exchange adapters

---

### ALERT: circuit_breaker_open

**Severity**: INFO (expected behavior) / WARN (critical adapter affected)

**Description**: Circuit breaker has opened for an adapter due to failures.

**Symptoms**:
- Adapter being skipped in waterfall
- `CIRCUIT_BREAKER_OPEN` events in debugger
- Reduced adapter traffic

**Diagnosis**:
```
1. Check circuit breaker state:
   - Which adapter(s) affected?
   - Failure count that triggered?
   - Time since opened?
   
2. Check underlying failures:
   - What errors caused opening?
   - Network or server errors?
   
3. Check half-open attempts:
   - Are probe requests succeeding?
   - Recovery timeline?
```

**Remediation**:
1. **Wait for recovery**: Circuit breakers auto-recover
2. **Manual reset**: If confident issue is resolved
3. **Adjust thresholds**: If opening too aggressively
4. **Investigate root cause**: Fix underlying failures

---

## SDK Health Alerts

### ALERT: sdk_initialization_failures

**Severity**: CRIT (any persistent failures)

**Description**: SDK failing to initialize on app launch.

**Symptoms**:
- No ads loading
- Init callbacks not firing
- Crash reports on launch

**Diagnosis**:
```
1. Check initialization logs:
   - What error is thrown?
   - At what stage does init fail?
   
2. Check configuration:
   - App ID valid?
   - API keys present?
   - Network available?
   
3. Check platform requirements:
   - Minimum OS version?
   - Required permissions?
```

**Remediation**:
| Error | Action |
|-------|--------|
| Invalid app ID | Verify in dashboard |
| Network unreachable | Check connectivity, use cached config |
| Missing permissions | Update app manifest/plist |
| Partner SDK conflict | Check for duplicate symbols |

---

### ALERT: sdk_crash_rate_elevated

**Severity**: WARN (>0.1%) / CRIT (>1%)

**Description**: SDK is contributing to app crashes.

**Symptoms**:
- Crash reports with SDK in stack trace
- User complaints about app stability
- App store rating decrease

**Diagnosis**:
```
1. Gather crash reports:
   - Symbolicate stack traces
   - Identify common patterns
   
2. Check for:
   - Threading issues (UI thread violations)
   - Memory pressure
   - Null pointer exceptions
   
3. Check recent changes:
   - SDK version update?
   - OS update?
   - New ad format enabled?
```

**Remediation**:
1. **Rollback SDK**: If new version causing crashes
2. **Disable feature**: If specific feature causes crashes
3. **Hot-fix**: If fix available
4. **Partner SDK update**: If partner SDK is culprit

---

## Migration Studio Alerts

### ALERT: migration_signal_loss
**Severity**: CRIT
**Description**: Experiment signals (`experimentId`/`arm`) missing from RTB requests despite being configured in the dashboard.
**Diagnosis**:
1. Check `AuctionClient.kt` or `AuctionClient.swift` for signal propagation logic.
2. Verify SDK version (Protocol Parity FIX required).
3. Check `experimentAuthCache.ts` in Backend; confirm `verifyExperimentAuth` is not failing-open.
**Remediation**:
1. Force SDK update for the affected publisher.
2. Check if the publisher's placement is correctly mapped in `migration_experiments` table.

### ALERT: migration_guardrail_violation
**Severity**: WARN / CRIT (Auto-Pause)
**Description**: Experiment arm is violating configured guardrails (e.g., latency >2x baseline or error rate >5%).
**Diagnosis**:
1. Check `MigrationGuardrailScheduler.ts` logs for violation reason.
2. Review side-by-side snapshots in Migration Studio dashboard.
**Remediation**:
1. The system should auto-pause (mirroring → shadow).
2. If auto-pause failed, manually archive the experiment.

---

## Platform-Specific Alerts

### iOS

#### ALERT: ats_blocking_ads

**Description**: App Transport Security blocking ad requests.

**Symptoms**: HTTPS connection failures, ERR_BLOCKED_BY_ATS

**Remediation**: See [iOS ATS Exceptions](../docs/ios-ats-exceptions.md)

#### ALERT: ios_memory_warning

**Description**: iOS issuing memory warnings during ad display.

**Symptoms**: UIApplicationDidReceiveMemoryWarning notifications

**Remediation**: 
- Release cached ads aggressively
- Reduce concurrent preloads
- Check for video ad memory leaks

### Android

#### ALERT: background_job_killed

**Description**: Android killing background ad prefetch jobs.

**Symptoms**: Job scheduler failures, incomplete prefetch

**Remediation**: 
- Use WorkManager with constraints
- Respect doze mode restrictions
- See [Android Background Restrictions](../docs/android-background-restrictions.md)

#### ALERT: proguard_stripping_required

**Description**: ProGuard stripping required SDK classes.

**Symptoms**: NoClassDefFoundError, MethodNotFoundError

**Remediation**: Add keep rules, see [Obfuscation Rules](../docs/obfuscation-crib-sheet.md)

### Unity

#### ALERT: unity_main_thread_violation

**Description**: Ad operations called from wrong thread.

**Symptoms**: Unity crash, "can only be called from the main thread"

**Remediation**: Wrap ad calls in UnityMainThreadDispatcher

### Web

#### ALERT: csp_blocking_ads

**Description**: Content Security Policy blocking ad scripts.

**Symptoms**: CSP violation in console, ads not loading

**Remediation**: 
- Update CSP headers for ad domains
- Use PostMessageBridge for cross-origin
- See [CSP Compliance](../docs/csp-compliance.md)

---

## Escalation Matrix

| Severity | Response Time | Notification | Escalation |
|----------|--------------|--------------|------------|
| INFO | Next business day | Slack #sdk-alerts | None |
| WARN | 2 hours | Slack + Email | Team lead after 4h |
| CRIT | 15 minutes | PagerDuty | Engineering manager after 30m |

## Related Documentation

- [SLOs and Alerts](./SLOS_AND_ALERTS.md)
- [Observability Runbook](./OBSERVABILITY_RUNBOOK.md)
- [Operator Checklist](./OPERATOR_CHECKLIST.md)
- [AdapterMetrics API](../Backend/adapter-metrics-api.md)
- [MediationDebugger Guide](../SDK/mediation-debugger.md)
