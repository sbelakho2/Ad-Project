# Production Deployment Guide - ApexMediation

> **Postgres-only update:** Sections that mention provisioning or validating ClickHouse are retained for historical reference but should be skipped for new deployments. The current production stack relies on PostgreSQL + Redis only; consult `docs/Internal/Infrastructure/POSTGRES_MIGRATION_PLAN.md` for the authoritative rollout flow.

---

## Hetzner Bare-Metal Deployment (Current Production)

The primary production environment is deployed on Hetzner dedicated servers using Docker Compose. See the following resources for current production:

| Resource | Location |
|----------|----------|
| **Deployment Documentation** | [docs/Architecture/HETZNER_POSTGRES_DESIGN.md](../../Architecture/HETZNER_POSTGRES_DESIGN.md) |
| **Monitoring Infrastructure** | [docs/Monitoring/MONITORING_INFRASTRUCTURE.md](../../Monitoring/MONITORING_INFRASTRUCTURE.md) |
| **Docker Compose Files** | `infrastructure/production/` |
| **Monitoring Stack** | `infrastructure/production/monitoring/` |

### Production Servers

| Role | IP Address | Services |
|------|------------|----------|
| App Server | 203.0.113.10 | Backend:8080, Console:3000, Grafana:3001, Prometheus:9090 |
| DB Server | 203.0.113.12 | PostgreSQL 16:5432 |

### Quick Access

```bash
# Replace 203.0.113.10 and 203.0.113.12 with your server addresses
# SSH to App Server
ssh -i ~/.ssh/hetzner-db-mac root@203.0.113.10

# SSH to DB Server  
ssh -i ~/.ssh/hetzner-db-mac root@203.0.113.12

# Database Connection
PGPASSWORD='<DB_PASSWORD>' psql -h 203.0.113.12 -U postgres -d adproject
```

---

## Kubernetes/Cloud Deployment (Legacy/Alternative)

The sections below describe Kubernetes-based deployments on AWS EKS or Google GKE. Use these for cloud-native deployments.

## Prerequisites

### Infrastructure Requirements
- **Kubernetes Cluster:** EKS 1.28+ (AWS) or GKE (Google Cloud)
- **Databases:**
  - PostgreSQL 15+ (RDS or Cloud SQL)
  - Redis 7+ (ElastiCache or Cloud Memorystore)
- **Container Registry:** ECR, GCR, or Docker Hub
- **CDN:** CloudFront or Cloud CDN
- **Load Balancer:** ALB/NLB or Cloud Load Balancing
- **Secrets Management:** AWS Secrets Manager, Vault, or Google Secret Manager

### Tools Required
- `kubectl` 1.28+
- `helm` 3.12+
- `terraform` 1.5+
- `docker` 24+
- AWS CLI / gcloud CLI

## Step 1: Infrastructure Setup

### 1.1 Provision Infrastructure with Terraform

```bash
cd infrastructure/terraform

# Initialize Terraform
terraform init

# Review plan
terraform plan \
  -var="environment=production" \
  -var="region=us-east-1" \
  -var="cluster_name=apexmediation-prod"

# Apply
terraform apply \
  -var="environment=production" \
  -var="region=us-east-1" \
  -var="cluster_name=apexmediation-prod"
```

### 1.2 Configure kubectl

```bash
# AWS EKS
aws eks update-kubeconfig \
  --region us-east-1 \
  --name apexmediation-prod

# Verify connection
kubectl cluster-info
kubectl get nodes
```

### 1.3 Install Essential Cluster Components

```bash
# Install Istio service mesh
kubectl create namespace istio-system
helm install istio-base istio/base -n istio-system
helm install istiod istio/istiod -n istio-system
helm install istio-ingress istio/gateway -n istio-system

# Install cert-manager
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.yaml

# Install metrics-server
kubectl apply -f https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml
```

## Step 2: Database Setup

### 2.1 PostgreSQL Setup

```bash
# Create database
psql -h <rds-endpoint> -U postgres -c "CREATE DATABASE apexmediation;"

# Run migrations
cd data/schemas
psql -h <rds-endpoint> -U postgres -d apexmediation -f postgresql.sql

# Create read replica user
psql -h <rds-endpoint> -U postgres -d apexmediation << EOF
CREATE USER readonly WITH PASSWORD '<secure-password>';
GRANT CONNECT ON DATABASE apexmediation TO readonly;
GRANT USAGE ON SCHEMA public TO readonly;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO readonly;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO readonly;
EOF
```

### 2.2 Redis Setup

```bash
# Test connection
redis-cli -h <elasticache-endpoint> ping

# Configure (if using self-hosted)
redis-cli -h <redis-host> CONFIG SET maxmemory 4gb
redis-cli -h <redis-host> CONFIG SET maxmemory-policy allkeys-lru
redis-cli -h <redis-host> CONFIG SET save "900 1 300 10 60 10000"
```

## Step 3: Secrets Management

### 3.1 Create Kubernetes Secrets

```bash
# Database credentials
kubectl create secret generic postgres-creds \
  --from-literal=host=<rds-endpoint> \
  --from-literal=port=5432 \
  --from-literal=database=rivalapexmediation \
  --from-literal=username=postgres \
  --from-literal=password=<secure-password> \
  -n rival

# Redis credentials
kubectl create secret generic redis-creds \
  --from-literal=host=<elasticache-endpoint> \
  --from-literal=port=6379 \
  --from-literal=password=<auth-token> \
  -n rival

# API keys
kubectl create secret generic api-keys \
  --from-literal=jwt-secret=<random-256-bit-key> \
  --from-literal=config-signing-key=<ed25519-private-key> \
  -n rival

# Payment providers
kubectl create secret generic payment-providers \
  --from-literal=adyen-key=<adyen-secret-key> \
  --from-literal=paypal-client-id=<paypal-client-id> \
  --from-literal=paypal-secret=<paypal-secret> \
  -n rival
```

### 3.2 External Secrets (Optional but Recommended)

```bash
# Install External Secrets Operator
helm repo add external-secrets https://charts.external-secrets.io
helm install external-secrets \
  external-secrets/external-secrets \
  -n external-secrets-system \
  --create-namespace

# Create SecretStore for AWS Secrets Manager
kubectl apply -f - <<EOF
apiVersion: external-secrets.io/v1beta1
kind: SecretStore
metadata:
  name: aws-secrets
  namespace: rival
spec:
  provider:
    aws:
      service: SecretsManager
      region: us-east-1
      auth:
        jwt:
          serviceAccountRef:
            name: external-secrets-sa
EOF
```

## Step 4: Deploy Services

### 4.1 Build and Push Docker Images

```bash
# Set registry
export REGISTRY=your-account.dkr.ecr.us-east-1.amazonaws.com
export VERSION=1.0.0

# Login to registry
aws ecr get-login-password --region us-east-1 | \
  docker login --username AWS --password-stdin $REGISTRY

# Build all services
for service in router analytics config fraud payments reporting; do
  docker build \
    -t $REGISTRY/rival-$service:$VERSION \
    -f backend/$service/Dockerfile \
    backend/$service
  
  docker push $REGISTRY/rival-$service:$VERSION
done

# Build console
docker build \
  -t $REGISTRY/rival-console:$VERSION \
  -f console/Dockerfile \
  console

docker push $REGISTRY/rival-console:$VERSION
```

### 4.2 Deploy to Kubernetes

```bash
# Create namespace
kubectl create namespace rival

# Label namespace for Istio injection
kubectl label namespace rival istio-injection=enabled

# Deploy services
kubectl apply -f infrastructure/k8s/production/

# Verify deployments
kubectl get pods -n rival
kubectl get services -n rival
kubectl get ingress -n rival
```

### 4.3 Configure Ingress and SSL

```bash
# Create certificate
kubectl apply -f - <<EOF
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: apexmediation-tls
  namespace: rival
spec:
  secretName: apexmediation-tls
  issuerRef:
    name: letsencrypt-prod
    kind: ClusterIssuer
  dnsNames:
    - api.apexmediation.ee
    - console.apexmediation.ee
EOF

# Update DNS records
# - api.apexmediation.ee → Load Balancer IP
# - console.apexmediation.ee → Load Balancer IP
```

## Step 5: Monitoring Setup

### 5.1 Deploy Prometheus Stack

```bash
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
helm install kube-prometheus-stack \
  prometheus-community/kube-prometheus-stack \
  -n monitoring \
  --create-namespace \
  -f infrastructure/monitoring/prometheus-values.yaml
```

### 5.2 Deploy Grafana Dashboards

```bash
# Import dashboards
kubectl create configmap grafana-dashboards \
  --from-file=infrastructure/monitoring/dashboards/ \
  -n monitoring

# Restart Grafana
kubectl rollout restart deployment/kube-prometheus-stack-grafana -n monitoring
```

### 5.3 Deploy Loki for Logging

```bash
helm repo add grafana https://grafana.github.io/helm-charts
helm install loki grafana/loki-stack \
  -n monitoring \
  -f infrastructure/monitoring/loki-values.yaml
```

### 5.4 Configure Alerting

```bash
# Create AlertManager configuration
kubectl create secret generic alertmanager-config \
  --from-file=infrastructure/monitoring/alertmanager.yaml \
  -n monitoring

# Configure PagerDuty integration
kubectl create secret generic pagerduty-key \
  --from-literal=integration-key=<pagerduty-integration-key> \
  -n monitoring

# Apply AI cost control alerts
kubectl apply -f monitoring/alerts.yml -n monitoring
```

### 5.5 AI Cost Control Setup

**CRITICAL:** AI automation features are disabled by default and must be enabled through a staged rollout process.

```bash
# Verify AI feature flags are disabled
kubectl get secret backend-secrets -n rival -o jsonpath='{.data.ENABLE_AI_AUTOMATION}' | base64 -d
# Expected: false

# Configure OpenAI API key (required even when disabled)
kubectl create secret generic openai-credentials \
  --from-literal=api-key=<openai-api-key> \
  -n rival

# Set up OpenAI usage monitoring
# 1. Log in to OpenAI dashboard: https://platform.openai.com/usage
# 2. Navigate to Settings > Limits
# 3. Set monthly budget limits:
#    - Soft limit (50%): $50/month → Email to platform@company.com + finance@company.com
#    - Hard limit (100%): $100/month → Block API + trigger Slack alert
# 4. Enable usage notifications to platform@company.com

# Apply Prometheus alerts for AI spend tracking
# (Alerts configured in monitoring/alerts.yml)
# - OpenAISpendExceeds50Percent (warning after 1h)
# - OpenAISpendExceeds80Percent (critical after 30m)
# - OpenAIHardLimitReached (critical after 5m)
# - UnexpectedAISpendSpike (warning if 3x increase)
```

**Staged Rollout Schedule:**
- **Week 1:** Enable `ENABLE_SALES_AI_OPTIMIZATION` only, monitor spend daily
- **Week 2:** Add `ENABLE_GROWTH_AI_ANALYTICS` if spend < $50/month
- **Week 3:** Add `ENABLE_SELF_EVOLVING_AI` if spend < $75/month
- **Ongoing:** Daily spend review, monthly finance report

**Reference Documentation:**
- Full AI cost control procedures: `infrastructure/runbooks/AI_COST_CONTROLS.md`
- Emergency rollback procedures: See runbook Section 4
- Cost estimation: $8.31/month at 100 customers (see runbook Section 6)

## Step 6: SDK Configuration

### 6.1 Generate Signing Keys

```bash
# Generate Ed25519 key pair for config signing
openssl genpkey -algorithm ed25519 -out config-private-key.pem
openssl pkey -in config-private-key.pem -pubout -out config-public-key.pem

# Store private key in secrets
kubectl create secret generic config-signing-key \
  --from-file=private-key=config-private-key.pem \
  -n rival

# Distribute public key to SDKs (embed in code)
cat config-public-key.pem
```

### 6.2 Upload Initial SDK Packages

```bash
# Android SDK to Maven Central / JitPack
cd sdk/core/android
./gradlew publishToMavenLocal

# iOS SDK to CocoaPods
cd sdk/core/ios
pod trunk push RivalApexMediation.podspec

# Unity Package to NPM/UPM
cd sdk/core/unity
npm publish
```

## Step 7: Initial Data Seeding

### 7.1 Create Test Publisher

```bash
# Connect to pod
kubectl exec -it deployment/config-service -n rival -- /bin/sh

# Run seeding script
psql $DATABASE_URL << EOF
INSERT INTO publishers (id, email, name, company_name, status, tier)
VALUES (
  '00000000-0000-0000-0000-000000000001',
  'demo@apexmediation.ee',
  'Demo Publisher',
  'Demo Company',
  'active',
  'enterprise'
);

INSERT INTO api_keys (publisher_id, key_hash, name, environment)
VALUES (
  '00000000-0000-0000-0000-000000000001',
  '<bcrypt-hash-of-demo-key>',
  'Production API Key',
  'production'
);
EOF
```

### 7.2 Configure Network Adapters

```bash
# Add default adapter configurations
kubectl run -it --rm debug --image=postgres:15 --restart=Never -- \
  psql $DATABASE_URL -c "
    INSERT INTO adapters (placement_id, network, status, priority, timeout_ms)
    SELECT 
      placement_id,
      unnest(ARRAY['admob', 'applovin', 'facebook', 'ironsource']),
      'active',
      generate_series(1, 4),
      3000
    FROM placements
    WHERE publisher_id = '00000000-0000-0000-0000-000000000001';
  "
```

## Step 8: Health Checks

### 8.1 Service Health

```bash
# Check all services
for service in router analytics config fraud payments reporting; do
  echo "Checking $service..."
  kubectl exec -it deployment/$service-service -n rival -- curl -f http://localhost:8080/health || echo "FAILED"
done
```

### 8.2 Database Connections

```bash
# Test PostgreSQL
kubectl run -it --rm pg-test --image=postgres:15 --restart=Never -- \
  psql -h <rds-endpoint> -U postgres -d rivalapexmediation -c "SELECT version();"

# Test ClickHouse
kubectl run -it --rm ch-test --image=clickhouse/clickhouse-client --restart=Never -- \
  clickhouse-client --host <clickhouse-host> --query "SELECT version();"

# Test Redis
kubectl run -it --rm redis-test --image=redis:7 --restart=Never -- \
  redis-cli -h <elasticache-endpoint> PING
```

### 8.3 End-to-End Test

```bash
# Create test ad request
curl -X POST https://api.apexmediation.ee/v1/ad/request \
  -H "Authorization: Bearer <api-key>" \
  -H "Content-Type: application/json" \
  -d '{
    "placement_id": "test-placement-1",
    "device": {
      "type": "phone",
      "os": "android",
      "os_version": "13"
    }
  }'
```

## Step 9: Performance Optimization

### 9.1 Horizontal Pod Autoscaling

```bash
# Configure HPA for high-traffic services
kubectl apply -f - <<EOF
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: router-service-hpa
  namespace: rival
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: router-service
  minReplicas: 10
  maxReplicas: 100
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
  - type: Resource
    resource:
      name: memory
      target:
        type: Utilization
        averageUtilization: 80
EOF
```

### 9.2 Pod Disruption Budgets

```bash
# Ensure high availability during updates
kubectl apply -f - <<EOF
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: router-service-pdb
  namespace: rival
spec:
  minAvailable: 5
  selector:
    matchLabels:
      app: router-service
EOF
```

### 9.3 Resource Limits

```bash
# Verify all pods have resource limits set
kubectl get pods -n rival -o json | \
  jq '.items[] | {name: .metadata.name, resources: .spec.containers[].resources}'
```

## Step 10: Security Hardening

### 10.1 Network Policies

```bash
# Apply network policies
kubectl apply -f infrastructure/k8s/production/network-policies.yaml

# Verify policies
kubectl get networkpolicies -n rival
```

### 10.2 Pod Security Standards

```bash
# Enable pod security admission
kubectl label namespace rival \
  pod-security.kubernetes.io/enforce=restricted \
  pod-security.kubernetes.io/audit=restricted \
  pod-security.kubernetes.io/warn=restricted
```

### 10.3 RBAC Configuration

```bash
# Create service accounts with minimal permissions
kubectl apply -f infrastructure/k8s/production/rbac.yaml

# Verify RBAC
kubectl auth can-i list secrets --as=system:serviceaccount:rival:router-service
```

## Step 11: Backup and Disaster Recovery

### 11.1 Database Backups

```bash
# PostgreSQL automated backups (RDS)
aws rds modify-db-instance \
  --db-instance-identifier rival-prod-postgres \
  --backup-retention-period 30 \
  --preferred-backup-window "03:00-04:00"

# ClickHouse backups
clickhouse-backup create daily-$(date +%Y%m%d)
clickhouse-backup upload daily-$(date +%Y%m%d)
```

### 11.2 Kubernetes State Backup

```bash
# Install Velero
helm install velero vmware-tanzu/velero \
  --namespace velero \
  --create-namespace \
  -f infrastructure/backup/velero-values.yaml

# Create backup schedule
velero schedule create daily-backup \
  --schedule="0 2 * * *" \
  --include-namespaces rival
```

## Step 12: Go-Live Checklist

**Before proceeding, complete the comprehensive checklist:**
See `PRE_DEPLOYMENT_CHECKLIST.md` for the full 10-section validation covering infrastructure, databases, applications, AI cost controls, security, performance, observability, disaster recovery, business continuity, and go-live procedures.

**Quick Pre-Launch Verification:**

- [ ] All services deployed and healthy
- [ ] Databases configured with backups (14 migrations applied)
- [ ] SSL certificates installed and valid
- [ ] DNS records pointing to load balancer
- [ ] Monitoring dashboards accessible
- [ ] Alerts configured and tested (including AI cost alerts)
- [ ] Secrets rotated and secured (including OpenAI API key)
- [ ] Rate limiting configured
- [ ] DDoS protection enabled
- [ ] Log aggregation working
- [ ] Backup restoration tested
- [ ] Disaster recovery runbook reviewed
- [ ] On-call rotation established
- [ ] Status page configured
- [ ] Documentation updated
- [ ] **AI Automation:** All feature flags disabled, OpenAI limits configured
- [ ] **Cost Controls:** Prometheus alerts active, daily review schedule set
- [ ] **Staged Rollout:** Week 1-3 plan documented and approved

## Post-Deployment

### Monitor Key Metrics
```bash
# Check pod status
watch kubectl get pods -n rival

# Check service health
curl https://api.apexmediation.ee/health

# View logs
kubectl logs -f deployment/router-service -n rival --tail=100

# Check metrics
kubectl port-forward -n monitoring svc/kube-prometheus-stack-prometheus 9090:9090
```

### Gradual Traffic Ramp
1. Start with 1% traffic (via DNS or load balancer)
2. Monitor for 24 hours
3. Increase to 5%, monitor 24 hours
4. Increase to 25%, monitor 48 hours
5. Increase to 100%

### Emergency Rollback
```bash
# Rollback deployment
kubectl rollout undo deployment/router-service -n rival

# Rollback to specific version
kubectl rollout undo deployment/router-service -n rival --to-revision=<revision>

# Trigger emergency rollback workflow
gh workflow run rollback.yml -f environment=production
```

## Support

For issues during deployment:
- Check logs: `kubectl logs -n rival <pod-name>`
- Review events: `kubectl get events -n rival --sort-by='.lastTimestamp'`
- Contact: security@apexmediation.ee
- On-call: PagerDuty incident trigger

## Next Steps

1. Complete SDK integration testing
2. Onboard first production publishers
3. Set up business intelligence dashboards
4. Configure A/B testing framework
5. Plan capacity scaling strategy
