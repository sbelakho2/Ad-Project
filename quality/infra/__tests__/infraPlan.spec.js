const fs = require('fs');
const path = require('path');

function read(p) {
  return fs.readFileSync(p, 'utf8');
}

function exists(p) {
  return fs.existsSync(p);
}

describe('Infrastructure Migration Plan — Repository Conformance', () => {
  const repoRoot = path.resolve(__dirname, '../../../');
  const infraPlan = path.join(repoRoot, 'docs/Internal/Infrastructure/INFRASTRUCTURE_MIGRATION_PLAN.md');
  const composeProd = path.join(repoRoot, 'infrastructure/docker-compose.prod.yml');
  const nginxConf = path.join(repoRoot, 'infrastructure/nginx/apexmediation.conf');
  const nginxSslConf = path.join(repoRoot, 'infrastructure/nginx/apexmediation.ssl.conf');
  const prodChecklist = path.join(repoRoot, 'docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md');
  const localHealthScript = path.join(repoRoot, 'scripts/ops/local_health_snapshot.sh');
  const backendIndex = path.join(repoRoot, 'backend/src/index.ts');
  const backendDeploy = path.join(repoRoot, 'backend/deploy-backend.sh');

  test('Key files exist', () => {
    expect(exists(infraPlan)).toBe(true);
    expect(exists(composeProd)).toBe(true);
    expect(exists(nginxConf)).toBe(true);
    expect(exists(nginxSslConf)).toBe(true);
    expect(exists(prodChecklist)).toBe(true);
    expect(exists(localHealthScript)).toBe(true);
  });

  test('Infra plan reflects Postgres-first and ClickHouse deferred', () => {
    const txt = read(infraPlan);
    expect(txt).toMatch(/1\.2\s+Database — Managed PostgreSQL/i);
    expect(txt).toMatch(/Analytics \(Postgres first\)/i);
    // Accept either phrasing "defer ClickHouse" or "avoid ClickHouse"
    expect(/defer\s+ClickHouse|avoid\s+ClickHouse/i.test(txt)).toBe(true);
    // Redis self-hosted guidance present
    expect(txt).toMatch(/Cache — Redis \(Self-Hosted/i);
    expect(txt).toMatch(/Enable authentication \(`requirepass`\)/);
  });

  test('Infra plan adopts MinIO-first object storage and removes Backblaze/B2', () => {
    const txt = read(infraPlan);
    expect(/MinIO/i.test(txt)).toBe(true);
    expect(/Backblaze|\bB2\b/i.test(txt)).toBe(false);
  });

  test('docker-compose.prod.yml has safe local defaults and healthchecks', () => {
    const txt = read(composeProd);
    expect(txt).toMatch(/DATABASE_URL=\$\{DATABASE_URL:-postgresql:\/\/postgres:postgres@postgres:5432\/apexmediation}/);
    expect(txt).toMatch(/DATABASE_SSL=\$\{DATABASE_SSL:-false}/);
    expect(txt).toMatch(/REDIS_URL=\$\{REDIS_URL:-redis:\/\/:\$\{REDIS_PASSWORD:-local-strong-password}@redis:6379\/0}/);
    expect(txt).toMatch(/healthcheck:[\s\S]*pg_isready/);
    expect(txt).toMatch(/redis:[\s\S]*healthcheck:[\s\S]*redis-cli/);
    expect(txt).toMatch(/nginx:[\s\S]*ports:[\s\S]*\$\{NGINX_PORT:-8080}:80/);
    // Console is optional via profile
    expect(txt).toMatch(/console:[\s\S]*profiles:[\s\S]*- ui/);
  });

  test('Nginx config includes security headers and HSTS gating', () => {
    const txt = read(nginxConf);
    expect(txt).toMatch(/add_header X-Content-Type-Options nosniff/);
    expect(txt).toMatch(/add_header X-Frame-Options SAMEORIGIN/);
    expect(txt).toMatch(/add_header X-XSS-Protection/);
    // HSTS is present but commented (gated)
    expect(txt).toMatch(/#\s*add_header Strict-Transport-Security/);
    // Console upstream disabled for Phase 8
    expect(txt).toMatch(/# upstream console_upstream/);
    // Default server on 80 contains /health proxy
    expect(txt).toMatch(/server\s*{[\s\S]*listen 80 default_server;[\s\S]*location \/health[\s\S]*proxy_pass http:\/\/api_upstream\/health/m);
  });

  test('HTTPS nginx config references ssl params and metrics protection comment', () => {
    const txt = read(nginxSslConf);
    expect(txt).toMatch(/listen 443 ssl http2/);
    expect(txt).toMatch(/include \/etc\/nginx\/snippets\/ssl-params\.conf/);
    expect(txt).toMatch(/#\s+include \/etc\/nginx\/snippets\/metrics-basic-auth\.conf/);
  });

  test('Local health snapshot script captures health and Redis verify via backend container', () => {
    const txt = read(localHealthScript);
    expect(txt).toMatch(/Nginx health .*\/health/);
    // Should execute verifyRedis inside backend container
    expect(txt).toMatch(/exec backend[\s\S]*node dist\/scripts\/verifyRedis\.js/);
    // Writes verify-redis.txt and summary.txt
    expect(txt).toMatch(/verify-redis\.txt/);
    expect(txt).toMatch(/summary\.txt/);
  });

  test('Production readiness checklist includes Day‑2 Operations section and key subsections', () => {
    const txt = read(prodChecklist);
    expect(txt).toMatch(/##\s+2\.\s+Day‑2 Operations \(Long‑Term Running\)/);
    expect(txt).toMatch(/###\s+2\.1\s+CI\/CD \& Release Management/);
    expect(txt).toMatch(/###\s+2\.2\s+Scheduled Jobs \(Cron\/Queues\) — Accounting, Billing, Sync/);
    expect(txt).toMatch(/###\s+2\.3\s+Backups \& Retention/i);
    expect(txt).toMatch(/###\s+2\.4\s+Monitoring, Metrics, and Alerting/);
    expect(txt).toMatch(/###\s+2\.5\s+Incident Response \& Runbooks/);
    expect(txt).toMatch(/###\s+2\.6\s+Security Operations/);
    expect(txt).toMatch(/###\s+2\.7\s+Capacity \& Cost Management/);
    expect(txt).toMatch(/###\s+2\.8\s+Operator Routines \(Checklist\)/);
  });

  test('Production readiness checklist adopts MinIO-first object storage and no Backblaze', () => {
    const txt = read(prodChecklist);
    expect(/MinIO/i.test(txt)).toBe(true);
    expect(/Backblaze|\bB2\b/i.test(txt)).toBe(false);
  });

  test('Production readiness checklist includes SDK Release Management & Platform Support', () => {
    const txt = read(prodChecklist);
    expect(txt).toMatch(/##\s+SDK Release Management \& Platform Support/);
    expect(txt).toMatch(/###\s+Governance \& Versioning/);
    expect(txt).toMatch(/###\s+Release Pipeline \& Smoke Tests/);
    expect(txt).toMatch(/JavaInteropSmoke\.java/);
    expect(txt).toMatch(/TelemetryCollectorTest\.kt/);
  });

  test('Production readiness checklist includes Support, Security, Legal, Website/Console, SLOs, and GTM sign-off', () => {
    const txt = read(prodChecklist);
    expect(txt).toMatch(/##\s+Customer Support, SLAs \& Onboarding/);
    expect(txt).toMatch(/##\s+Security Readiness \& Audits/);
    expect(txt).toMatch(/##\s+Legal \& Compliance \(Product\)/);
    expect(txt).toMatch(/##\s+Website \& Console UX Readiness/);
    expect(txt).toMatch(/##\s+Operational Reporting \& SLOs/);
    expect(txt).toMatch(/##\s+Final Sign‑off — Additional Go‑To‑Market Gates/);
  });

  test('TLS snapshot script writes expected evidence files', () => {
    const txt = read(tlsScript);
    expect(txt).toMatch(/verify-redirects\.txt/);
    expect(txt).toMatch(/verify-tls\.txt/);
    expect(txt).toMatch(/verify-hsts\.txt/);
  });

  test('Backend entrypoint does not import or initialize ClickHouse and health excludes it', () => {
    const txt = read(backendIndex);
    // No imports from clickhouse utils
    expect(txt).not.toMatch(/from '\.\/utils\/clickhouse'/);
    // Health services should not include clickhouse
    expect(txt).not.toMatch(/clickhouse\s*:/i);
  });

  test('Deploy script does not reference Upstash or ClickHouse', () => {
    const txt = read(backendDeploy);
    expect(/upstash/i.test(txt)).toBe(false);
    expect(/clickhouse/i.test(txt)).toBe(false);
  });

  test('Backend deploy script is marked deprecated', () => {
    const deployTxt = read(backendDeploy);
    expect(deployTxt).toMatch(/DEPRECATED/i);
  });
});
