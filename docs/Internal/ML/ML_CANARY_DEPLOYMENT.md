# ML Canary Deployment Guide

Gradual rollout strategy for ML model updates with automatic traffic routing and rollback capabilities.

## Overview

Canary deployments allow testing new ML model versions on a small percentage of production traffic before full rollout. This mitigates risks from:

- Model accuracy degradation
- Increased inference latency
- Breaking API changes
- Unexpected edge cases

## Architecture

```
┌─────────────┐
│   Request   │
└──────┬──────┘
       │
       ▼
┌──────────────────┐
│ Model Selector   │  Hash(user_id + model_name) % 100
│ (mlCanary.ts)    │
└──────┬───────────┘
       │
    ┌──┴──┐
    │     │
    ▼     ▼
┌─────┐ ┌─────┐
│Stable│ │Canary│
│v1.0.0│ │v1.1.0│
└─────┘ └─────┘
```

## Configuration

### Environment Variables

```bash
# Enable canary deployment
ML_CANARY_ENABLED=true

# Canary model version
ML_CANARY_MODEL_VERSION=1.1.0

# Percentage of traffic to route to canary (0-100)
ML_CANARY_TRAFFIC_PERCENT=10

# ML inference service URLs
ML_INFERENCE_SERVICE_URL=http://ml-inference:8000
ML_CANARY_SERVICE_URL=http://ml-inference-canary:8000
```

### Feature Flags

Feature flags are read from `backend/src/utils/featureFlags.ts`:

```typescript
{
  mlCanaryEnabled: boolean,
  mlCanaryModelVersion: string,
  mlCanaryTrafficPercent: number
}
```

## Rollout Strategy

### Phase 1: Initial Canary (1-5%)

```bash
# Deploy canary service with new model
kubectl apply -f services/inference-ml/helm/ml-inference/values-canary.yaml

# Enable canary with 1% traffic
export ML_CANARY_ENABLED=true
export ML_CANARY_MODEL_VERSION=1.1.0
export ML_CANARY_TRAFFIC_PERCENT=1

# Restart backend to pick up new flags
kubectl rollout restart deployment/backend -n production
```

**Monitor for 24 hours:**
- Inference latency (p50, p95, p99)
- Error rates
- Prediction distribution
- User impact metrics (CTR, revenue)

### Phase 2: Increase Traffic (10-25%)

If metrics look good, increase canary traffic:

```bash
export ML_CANARY_TRAFFIC_PERCENT=10
kubectl rollout restart deployment/backend -n production
```

**Monitor for 12 hours:**
- Compare canary vs stable metrics
- Check for drift in predictions
- Validate business KPIs

### Phase 3: Majority Rollout (50%)

```bash
export ML_CANARY_TRAFFIC_PERCENT=50
kubectl rollout restart deployment/backend -n production
```

**Monitor for 6 hours:**
- Sustained performance at scale
- No degradation in user experience

### Phase 4: Full Rollout (100%)

```bash
# Option A: Promote canary to stable
export ML_CANARY_ENABLED=false
kubectl set image deployment/ml-inference ml-inference=rivalapex/ml-inference:1.1.0

# Option B: Complete canary rollout
export ML_CANARY_TRAFFIC_PERCENT=100
```

## Traffic Routing

### Consistent Hashing

Requests are routed using MD5 hash of `{user_id}:{model_name}`:

```typescript
const hash = crypto.createHash('md5').update(`${userId}:fraud_detection`).digest('hex');
const bucket = parseInt(hash.substring(0, 8), 16) % 100; // 0-99

if (bucket < canaryTrafficPercent) {
  // Route to canary
} else {
  // Route to stable
}
```

**Benefits:**
- Same user always gets same model version (no A/B testing contamination)
- Predictable traffic distribution
- Easy to debug specific user issues

### Model Selection API

```typescript
import { selectModelVersion } from './utils/mlCanary';

const userId = 'user_123';
const modelName = 'fraud_detection';

const selection = selectModelVersion(userId, modelName);
// { version: '1.1.0', isCanary: true }
```

## Monitoring

### Key Metrics

**Inference Performance:**
```promql
# Latency comparison
histogram_quantile(0.95, 
  sum by (le, model_version) (
    rate(ml_inference_duration_seconds_bucket[5m])
  )
)

# Error rate by version
sum by (model_version) (
  rate(ml_inference_errors_total[5m])
)
```

**Traffic Distribution:**
```promql
# Requests by version
sum by (model_version) (
  rate(ml_inference_requests_total[5m])
)
```

**Business Metrics:**
- CTR comparison (canary vs stable cohorts)
- Revenue per impression
- False positive/negative rates

### Grafana Dashboard

Create dashboard with:

1. **Traffic Split** - Gauge showing canary percentage
2. **Latency Comparison** - Line chart (stable vs canary)
3. **Error Rates** - Alert on significant divergence
4. **Prediction Distribution** - Histograms for drift detection
5. **Business KPIs** - CTR, revenue, conversion rates

### Alerts

```yaml
# monitoring/alerts.yml

- alert: CanaryLatencyRegression
  expr: |
    histogram_quantile(0.95,
      sum by (le) (rate(ml_inference_duration_seconds_bucket{model_version="canary"}[5m]))
    )
    >
    histogram_quantile(0.95,
      sum by (le) (rate(ml_inference_duration_seconds_bucket{model_version="stable"}[5m]))
    ) * 1.2
  for: 10m
  annotations:
    summary: "Canary model 20% slower than stable"

- alert: CanaryErrorRateHigh
  expr: |
    sum(rate(ml_inference_errors_total{model_version="canary"}[5m]))
    /
    sum(rate(ml_inference_requests_total{model_version="canary"}[5m]))
    > 0.05
  for: 5m
  annotations:
    summary: "Canary error rate above 5%"
```

## Rollback Procedures

### Immediate Rollback (Emergency)

```bash
# Disable canary immediately
export ML_CANARY_ENABLED=false
kubectl rollout restart deployment/backend -n production

# Or set traffic to 0%
export ML_CANARY_TRAFFIC_PERCENT=0
```

**Use when:**
- Critical errors detected (>5% error rate)
- Severe latency regression (>2x stable)
- Business metrics significantly worse

### Gradual Rollback

```bash
# Reduce canary traffic incrementally
export ML_CANARY_TRAFFIC_PERCENT=25  # From 50%
# Wait 1 hour, monitor
export ML_CANARY_TRAFFIC_PERCENT=10  # From 25%
# Wait 1 hour, monitor
export ML_CANARY_TRAFFIC_PERCENT=0   # Complete rollback
```

### Post-Rollback

1. **Investigate root cause**
2. **Fix model or code**
3. **Re-validate in staging**
4. **Retry deployment with Phase 1**

## Testing Canary Locally

```bash
# Start stable ML service
docker run -p 8000:8000 rivalapex/ml-inference:1.0.0

# Start canary ML service
docker run -p 8001:8000 rivalapex/ml-inference:1.1.0

# Configure backend
export ML_CANARY_ENABLED=true
export ML_CANARY_MODEL_VERSION=1.1.0
export ML_CANARY_TRAFFIC_PERCENT=50
export ML_INFERENCE_SERVICE_URL=http://localhost:8000
export ML_CANARY_SERVICE_URL=http://localhost:8001

# Test requests
curl -X POST http://localhost:3000/api/fraud/check \
  -H "X-User-Id: user_123" \
  -d '{"ip_address": "192.168.1.1", ...}'

# Check which version was used (look for model_version in response)
```

## Best Practices

1. **Start Small:** Begin with 1-5% traffic
2. **Monitor Closely:** Watch metrics every hour during initial rollout
3. **Automate Rollback:** Set up alerts that can trigger automatic rollback
4. **Document Changes:** Track which version performed better and why
5. **Preserve Consistency:** Use user_id for hashing so users get consistent experience
6. **Test in Staging:** Always validate canary in staging environment first
7. **Business Hours:** Deploy canaries during business hours for faster issue detection

## Example Deployment

```bash
# 1. Deploy canary infrastructure
helm install ml-inference-canary ./helm/ml-inference \
  --namespace ml \
  --set image.tag=1.1.0 \
  --set service.name=ml-inference-canary \
  --set replicaCount=2

# 2. Enable canary routing (1%)
kubectl set env deployment/backend \
  ML_CANARY_ENABLED=true \
  ML_CANARY_MODEL_VERSION=1.1.0 \
  ML_CANARY_TRAFFIC_PERCENT=1 \
  -n production

# 3. Monitor Grafana dashboard
# Replace 203.0.113.10 with your Grafana host
# Hetzner Production: http://203.0.113.10:3001/d/apex-rtb-analytics
open http://203.0.113.10:3001/d/apex-rtb-analytics

# 4. Increase to 10% after 24h
kubectl set env deployment/backend \
  ML_CANARY_TRAFFIC_PERCENT=10 \
  -n production

# 5. Continue until 100%, then promote
kubectl set image deployment/ml-inference \
  ml-inference=apexmediation/ml-inference:1.1.0 \
  -n ml

# 6. Disable canary routing
kubectl set env deployment/backend \
  ML_CANARY_ENABLED=false \
  -n production

# 7. Clean up canary deployment
helm uninstall ml-inference-canary -n ml
```

## Related Documentation

- [ML Inference Service](../../services/inference-ml/README.md)
- [Model Registry](../../models/README.md)
- [ML Training Guide](../../ML_TRAINING.md)
- [Monitoring Guide](../Monitoring/GRAFANA_DASHBOARDS.md)
