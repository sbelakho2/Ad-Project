# Platform Updates & Maintenance - Automation Guide

**ApexMediation Automated SDK Release & Maintenance System**  
**Last Updated:** 2025-11-03  
**Status:** Production Ready

---

## Table of Contents

1. [Overview](#overview)
2. [Release Workflow](#release-workflow)
3. [Changelog Generation](#changelog-generation)
4. [Customer Notifications](#customer-notifications)
5. [Backward Compatibility](#backward-compatibility)
6. [Documentation Deployment](#documentation-deployment)
7. [Monitoring & Metrics](#monitoring--metrics)

---

## Overview

The ApexMediation platform includes **100% automated SDK release and maintenance** workflows:

- **Zero-Touch Releases**: Tag → Build → Test → Publish → Notify (fully automated)
- **Semantic Versioning**: Conventional commits → Automatic version bumps
- **Breaking Change Detection**: API diff analysis with PR warnings
- **Customer Notifications**: Email + console alerts for SDK updates
- **Documentation**: Auto-generated and deployed with version switcher
- **Compatibility Testing**: Matrix testing across SDK/backend versions

**Design Goal**: Solo operator can release SDKs to all platforms (iOS, Android, Unity) in <5 minutes with zero manual steps.

---

## Release Workflow

### Quick Start

**Releasing a new SDK version:**

```bash
# 1. Commit changes with conventional commits
git commit -m "feat(ios): add new banner ad format"
git commit -m "fix(android): resolve memory leak in adapter"

# 2. Tag the release (triggers automation)
git tag v1.2.0
git push origin v1.2.0

# 3. Sit back and watch GitHub Actions:
#    ✅ Validate semantic version
#    ✅ Generate changelog
#    ✅ Build iOS/Android/Unity SDKs
#    ✅ Run backward compatibility tests
#    ✅ Publish to CocoaPods, Maven Central, NPM
#    ✅ Create GitHub Release
#    ✅ Notify all customers via email
#    ✅ Deploy documentation
```

### Conventional Commits

All commits must follow [Conventional Commits](https://www.conventionalcommits.org/) format:

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types:**
- `feat`: New feature (minor version bump)
- `fix`: Bug fix (patch version bump)
- `perf`: Performance improvement (patch)
- `refactor`: Code restructuring (patch)
- `docs`: Documentation only (patch)
- `test`: Adding tests (patch)
- `chore`: Maintenance (no version bump)

**Breaking Changes:**
- Add `!` after type: `feat!: redesign API`
- Or add footer: `BREAKING CHANGE: description`
- Triggers **major version bump**

**Examples:**

```bash
# Feature (1.0.0 → 1.1.0)
git commit -m "feat(ios): add rewarded video ads"

# Bug fix (1.0.0 → 1.0.1)
git commit -m "fix(android): resolve ANR in adapter initialization"

# Breaking change (1.0.0 → 2.0.0)
git commit -m "feat!: remove deprecated AdManager.initialize() method

BREAKING CHANGE: initialize() method removed. Use AdManager.configure() instead.

Migration: Replace AdManager.initialize() with AdManager.configure().
"
```

### GitHub Actions Workflow

**Triggered on:** Push of git tag matching `v*.*.*`

**Workflow Steps:**

1. **Validate Version** (30 seconds)
   - Check semantic versioning format
   - Detect pre-release (alpha, beta, rc)
   - Output: version string, pre-release flag

2. **Generate Changelog** (1 minute)
   - Parse conventional commits since last tag
   - Detect breaking changes
   - Generate CHANGELOG.md
   - Create migration guide if needed
   - Output: changelog markdown, breaking flag

3. **Build SDKs** (5-10 minutes, parallel)
   - **iOS**: Xcode build → XCFramework → CocoaPods validation
   - **Android**: Gradle build → AAR → Maven signing
   - **Unity**: NPM package creation → tarball
   - Output: Build artifacts

4. **Compatibility Testing** (3-5 minutes, parallel)
   - API diff analysis (detect breaking changes)
   - Matrix testing (SDK v1/v2 × Backend v1/v2)
   - Integration tests with previous SDK
   - Output: Compatibility report

5. **Publish to Package Managers** (2-3 minutes, sequential)
   - **CocoaPods**: `pod trunk push` (iOS)
   - **Maven Central**: `gradle publishToMavenCentral` (Android)
   - **NPM**: `npm publish` (Unity)
   - Skip for pre-releases
   - Output: Package URLs

6. **Create GitHub Release** (30 seconds)
   - Generate release notes from changelog
   - Attach SDK artifacts (XCFramework, AAR, tarball)
   - Mark as pre-release if applicable
   - Output: Release URL

7. **Notify Customers** (1-2 minutes)
   - Emit email events for all active customers
   - Create console notifications
   - Send to Discord webhook (optional)
   - Output: Notification count

8. **Deploy Documentation** (3-5 minutes)
   - Generate TypeDoc (backend), Jazzy (iOS), Dokka (Android)
   - Build docs site with version switcher
   - Deploy to Cloudflare Pages or GitHub Pages
   - Update Algolia search index
   - Output: Docs URL

**Total Time:** 15-25 minutes from tag to live

---

## Changelog Generation

### ChangelogGenerationService

**Location:** `backend/services/release/ChangelogGenerationService.ts`

**Features:**
- Parses conventional commits from git history
- Groups changes by type (Features, Bug Fixes, Performance, etc.)
- Detects breaking changes from commit messages
- Generates CHANGELOG.md in Keep a Changelog format
- Creates migration guides for breaking changes
- Formats changelog for emails (HTML)

**Usage:**

```typescript
import { ChangelogGenerationService } from './services/release/ChangelogGenerationService';

const service = new ChangelogGenerationService('/path/to/repo');

// Generate changelog for new version
const releaseNotes = await service.generateChangelog('1.2.0');

console.log(releaseNotes.version);           // "1.2.0"
console.log(releaseNotes.breaking_changes);  // true/false
console.log(releaseNotes.raw_changelog);     // Markdown
console.log(releaseNotes.migration_guide);   // Migration guide (if breaking)

// Format for email
const emailHTML = service.formatForEmail(releaseNotes);

// Get existing release notes
const notes = await service.getReleaseNotes('1.1.0');
```

**CLI Usage:**

```bash
cd backend

# Generate changelog for version
node -r ts-node/register services/release/ChangelogGenerationService.ts 1.2.0
```

### CHANGELOG.md Format

```markdown
# Changelog

All notable changes to this project will be documented in this file.

## [1.2.0] - 2025-11-03

### ⚠️ BREAKING CHANGES

- **ios**: Removed deprecated AdManager.initialize() method ([a1b2c3d](../../commit/a1b2c3d))
  Use AdManager.configure() instead.

### 🚀 Features

- **ios**: Add rewarded video ads ([e4f5g6h](../../commit/e4f5g6h))
- **android**: Implement custom ad placements ([i7j8k9l](../../commit/i7j8k9l))

### 🐛 Bug Fixes

- **ios**: Resolve crash on background return ([m0n1o2p](../../commit/m0n1o2p))
- **android**: Fix memory leak in adapter pool ([q3r4s5t](../../commit/q3r4s5t))

### ⚡ Performance Improvements

- **ios**: Reduce SDK init time by 40% ([u6v7w8x](../../commit/u6v7w8x))

## [1.1.0] - 2025-10-15
...
```

---

## Customer Notifications

### SDKUpdateNotificationService

**Location:** `backend/services/release/SDKUpdateNotificationService.ts`

**Features:**
- Notifies all active customers about SDK updates
- Sends email with changelog, installation instructions
- Creates in-console notifications with priority
- Tracks SDK adoption rates
- Sends upgrade reminders to outdated customers
- Notifies users of deprecated APIs

**Usage:**

```typescript
import { SDKUpdateNotificationService } from './services/release/SDKUpdateNotificationService';

const service = new SDKUpdateNotificationService(process.env.DATABASE_URL);

// Notify all customers about new version
await service.notifyCustomers('1.2.0');
// → Emits email events for all active customers
// → Emails processed by EmailAutomationService

// Create console notifications
await service.createConsoleNotification('1.2.0');
// → Dashboard banner for all customers

// Track adoption rate
const stats = await service.trackSDKAdoption('1.2.0');
console.log(`${stats.adoption_rate}% upgraded`);

// Send reminder to outdated customers (14 days after release)
await service.sendAdoptionReminder('1.2.0', 14);

// Notify users of deprecated API
await service.notifyDeprecatedAPIUsers(
  '/api/v1/old-endpoint',
  new Date('2026-01-01'),
  '2.0.0'
);

await service.close();
```

**CLI Usage:**

```bash
cd backend

# Notify customers about release
export DATABASE_URL="postgresql://..."
node -r ts-node/register services/release/SDKUpdateNotificationService.ts notify 1.2.0

# Check adoption rate
node -r ts-node/register services/release/SDKUpdateNotificationService.ts adoption 1.2.0

# Send upgrade reminder
node -r ts-node/register services/release/SDKUpdateNotificationService.ts reminder 1.2.0
```

### Email Template

**Subject:** 🚀 ApexMediation SDK v1.2.0 Released

```html
<h2>ApexMediation SDK v1.2.0</h2>

<p><strong>⚠️ This release contains breaking changes.</strong> 
Please review the <a href="https://docs.apexmediation.ee/migration/v1.2.0">migration guide</a>.</p>

<h3>What's Changed</h3>

<h4>🚀 Features</h4>
<ul>
  <li><strong>ios</strong>: Add rewarded video ads</li>
  <li><strong>android</strong>: Implement custom ad placements</li>
</ul>

<h4>🐛 Bug Fixes</h4>
<ul>
  <li><strong>ios</strong>: Resolve crash on background return</li>
</ul>

<h3>Installation</h3>

<p><strong>iOS (CocoaPods)</strong></p>
<pre><code>pod 'ApexMediation', '~> 1.2.0'</code></pre>

<p><strong>Android (Gradle)</strong></p>
<pre><code>implementation 'com.apexmediation:sdk:1.2.0'</code></pre>

<p><strong>Unity (NPM)</strong></p>
<pre><code>npm install @apexmediation/unity-sdk@1.2.0</code></pre>

<p>View full release notes on <a href="https://github.com/apexmediation/platform/releases/tag/v1.2.0">GitHub</a>.</p>
```

**Console Notification:**

```
┌─────────────────────────────────────────┐
│ 🚀 SDK v1.2.0 Released                  │
│                                         │
│ New SDK version available with          │
│ improvements and bug fixes.             │
│                                         │
│ [View Release Notes]                    │
└─────────────────────────────────────────┘
```

---

## Backward Compatibility

### API Change Detection

**Workflow:** `.github/workflows/compatibility-testing.yml`

**Features:**
- Detects breaking API changes using Microsoft API Extractor
- Compares current API surface with previous version
- Comments on PRs with breaking change warnings
- Enforces semantic versioning rules
- Blocks merge if major version not bumped

**Example PR Comment:**

```markdown
## ⚠️ Breaking API Changes Detected

This PR introduces breaking changes to the public API. Please:

1. Review the API diff artifact
2. Update the version (major bump)
3. Create a migration guide
4. Document breaking changes in commit message

```diff
- export function initialize(): void;
+ export function configure(config: Config): void;
```
```

### Matrix Compatibility Testing

Tests all combinations of SDK and backend versions:

| SDK Version | Backend v1.0 | Backend v1.1 | Backend v2.0 |
|-------------|--------------|--------------|--------------|
| SDK v1.0    | ✅ Pass       | ✅ Pass       | ❌ Fail       |
| SDK v1.1    | ✅ Pass       | ✅ Pass       | ❌ Fail       |
| SDK v2.0    | ❌ Fail       | ❌ Fail       | ✅ Pass       |

**Result:** SDK v1.x incompatible with Backend v2.0 (expected for major version change)

### Integration Tests

**Previous SDK Test:**
- Starts current backend
- Runs previous SDK version integration tests
- Fails if backward compatibility broken

**Example Failure:**

```
❌ Backward compatibility broken!
SDK v1.1.0 fails against current backend
This is a BREAKING CHANGE - bump major version
```

### Contract Testing (Pact)

**Consumer-Driven Contracts:**
- SDK defines expected API contracts
- Backend must satisfy contracts
- Contracts published to Pact Broker
- CI fails if contract violated

**Example Contract:**

```json
{
  "consumer": "ApexMediation iOS SDK v1.0",
  "provider": "ApexMediation Backend API",
  "interactions": [
    {
      "description": "Get ad configuration",
      "request": {
        "method": "GET",
        "path": "/api/v1/config",
        "headers": { "Authorization": "Bearer TOKEN" }
      },
      "response": {
        "status": 200,
        "body": {
          "adapters": [
            { "name": "AdMob", "enabled": true }
          ]
        }
      }
    }
  ]
}
```

---

## Documentation Deployment

### Auto-Generated Documentation

**Workflow:** `.github/workflows/docs-deployment.yml`

**Generates:**
- **Backend API**: TypeDoc (TypeScript → HTML)
- **iOS SDK**: Jazzy (Swift → HTML)
- **Android SDK**: Dokka (Kotlin → HTML)
- **Unity SDK**: JSDoc (JavaScript → HTML)

**Deployment:**
- **Primary**: Cloudflare Pages (free, fast CDN)
- **Fallback**: GitHub Pages (if Cloudflare not configured)
- **URL**: https://docs.apexmediation.ee

### Version Switcher

**Structure:**

```
docs.apexmediation.ee/
├── index.html                 # Landing page
├── versions.json              # Version list
├── latest/ → v1.2.0/          # Symlink to latest
├── v1.2.0/
│   ├── ios/                   # Jazzy docs
│   ├── android/               # Dokka docs
│   ├── unity/                 # JSDoc docs
│   └── backend/               # TypeDoc docs
├── v1.1.0/
│   └── ...
└── v1.0.0/
    └── ...
```

**versions.json:**

```json
{
  "versions": [
    { "name": "1.2.0", "path": "/v1.2.0" },
    { "name": "1.1.0", "path": "/v1.1.0" },
    { "name": "1.0.0", "path": "/v1.0.0" }
  ],
  "latest": "1.2.0"
}
```

**Version Dropdown:**

```html
<select onchange="window.location.href = this.value">
  <option value="/v1.2.0">1.2.0 (latest)</option>
  <option value="/v1.1.0">1.1.0</option>
  <option value="/v1.0.0">1.0.0</option>
</select>
```

### Search Integration

**Algolia DocSearch:**
- Crawls docs site on every deployment
- Provides instant search across all versions
- Free for open-source projects

**Configuration:**

```json
{
  "index_name": "apexmediation",
  "start_urls": [
    "https://docs.apexmediation.ee/"
  ],
  "selectors": {
    "lvl0": "h1",
    "lvl1": "h2",
    "lvl2": "h3",
    "text": "p, li"
  }
}
```

---

## Monitoring & Metrics

### Release Metrics

**Track in Grafana:**

**Release Frequency:**
```sql
SELECT 
  DATE_TRUNC('month', created_at) as month,
  COUNT(*) as releases
FROM github_releases
GROUP BY month
ORDER BY month DESC;
```

**Time to Release:**
- Tag push → All packages published
- Target: <30 minutes
- Alert if >1 hour

**SDK Adoption Rate:**
```sql
SELECT 
  sdk_version,
  COUNT(DISTINCT customer_id) as customers,
  (COUNT(DISTINCT customer_id)::float / (SELECT COUNT(*) FROM customers WHERE status = 'active')) * 100 as adoption_pct
FROM sdk_telemetry
WHERE recorded_at >= NOW() - INTERVAL '7 days'
GROUP BY sdk_version
ORDER BY adoption_pct DESC;
```

**Breaking Change Frequency:**
```sql
SELECT 
  DATE_TRUNC('quarter', created_at) as quarter,
  COUNT(*) FILTER (WHERE breaking_changes = true) as breaking,
  COUNT(*) as total,
  (COUNT(*) FILTER (WHERE breaking_changes = true)::float / COUNT(*)) * 100 as breaking_pct
FROM github_releases
GROUP BY quarter
ORDER BY quarter DESC;
```

**Target: <10% releases with breaking changes**

### Alerts

**PagerDuty Integration:**

```yaml
alerts:
  - name: Release Failed
    condition: github_actions_workflow_status == 'failure' AND workflow_name == 'SDK Release Automation'
    severity: high
    channels: [pagerduty, slack]
    
  - name: Low Adoption Rate
    condition: sdk_adoption_rate < 50 AND days_since_release > 30
    severity: medium
    channels: [slack]
    
  - name: Compatibility Test Failed
    condition: compatibility_test_status == 'failed'
    severity: high
    channels: [pagerduty, slack]
```

### Dashboard

**Grafana Dashboard: SDK Release Health**

Panels:
1. **Release Timeline** (graph)
   - Releases over time with version labels
   - Color: green (patch), yellow (minor), red (major)

2. **Adoption Rate** (gauge)
   - % of customers on latest version
   - Green >80%, Yellow 50-80%, Red <50%

3. **Time to Release** (stat)
   - Average time from tag to published
   - Target: <30 minutes

4. **Breaking Changes** (pie chart)
   - % releases with breaking changes
   - Target: <10%

5. **SDK Version Distribution** (bar chart)
   - Customers per SDK version
   - Shows migration progress

6. **Package Downloads** (graph)
   - CocoaPods, Maven Central, NPM downloads
   - Trend over time

---

## Troubleshooting

### Release Failed

**Check GitHub Actions:**
```bash
# View workflow runs
gh run list --workflow="SDK Release Automation"

# View logs for failed run
gh run view <run-id> --log-failed
```

**Common Issues:**

1. **CocoaPods Trunk Push Failed**
   - Cause: Invalid `COCOAPODS_TRUNK_TOKEN`
   - Fix: Regenerate token: `pod trunk register`
   - Update secret: `gh secret set COCOAPODS_TRUNK_TOKEN`

2. **Maven Central Publish Failed**
   - Cause: GPG signing error
   - Fix: Check `GPG_PRIVATE_KEY` and `GPG_PASSPHRASE` secrets
   - Verify: `echo $GPG_PRIVATE_KEY | base64 -d | gpg --import`

3. **NPM Publish Failed**
   - Cause: Version already exists
   - Fix: Bump version, re-tag

4. **Compatibility Tests Failed**
   - Cause: Breaking change not documented
   - Fix: Add `BREAKING CHANGE:` to commit message
   - Re-tag with major version bump

### Customer Notifications Not Sent

**Check email queue:**
```sql
SELECT * FROM events
WHERE event_type = 'email.sdk_update'
  AND created_at >= NOW() - INTERVAL '1 hour'
ORDER BY created_at DESC;
```

**Process queue manually:**
```bash
cd backend
npm run cron:email-queue
```

### Documentation Not Deployed

**Check Cloudflare Pages:**
```bash
# View deployments
curl -X GET "https://api.cloudflare.com/client/v4/accounts/$ACCOUNT_ID/pages/projects/apexmediation-docs/deployments" \
  -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN"
```

**Deploy manually:**
```bash
cd backend
npm run docs:build
npx wrangler pages publish ./docs --project-name=apexmediation-docs
```

---

## Best Practices

### 1. Use Conventional Commits

**Always** use conventional commit format:
```bash
✅ git commit -m "feat(ios): add banner ads"
❌ git commit -m "added banner ads"
```

### 2. Test Before Tagging

Run tests locally before creating release tag:
```bash
npm test
npm run lint
npm run build
```

### 3. Breaking Changes

**Minimize** breaking changes:
- Deprecate old API first (at least 1 version)
- Provide migration guide
- Support old API alongside new for 1-2 versions

### 4. Pre-Releases

Use pre-release tags for testing:
```bash
git tag v1.2.0-beta.1      # Beta release
git tag v1.2.0-rc.1        # Release candidate
```

Pre-releases skip package manager publishing.

### 5. Rollback Plan

If release has critical bug:
```bash
# 1. Yank bad version from package managers
pod trunk delete ApexMediation 1.2.0
# (Maven Central can't delete, mark as deprecated)

# 2. Release hotfix
git tag v1.2.1
git push origin v1.2.1

# 3. Notify customers
node services/release/SDKUpdateNotificationService.ts notify 1.2.1
```

---

## Secrets Configuration

**Required GitHub Secrets:**

```bash
# CocoaPods (iOS)
COCOAPODS_TRUNK_TOKEN=xxx

# Maven Central (Android)
OSSRH_USERNAME=xxx
OSSRH_PASSWORD=xxx
GPG_PRIVATE_KEY=xxx (base64 encoded)
GPG_PASSPHRASE=xxx

# NPM (Unity)
NPM_TOKEN=xxx

# Cloudflare Pages (Docs)
CLOUDFLARE_API_TOKEN=xxx
CLOUDFLARE_ACCOUNT_ID=xxx

# Algolia (Search)
ALGOLIA_API_KEY=xxx
ALGOLIA_CRAWLER_ID=xxx

# Database (Notifications)
DATABASE_URL=postgresql://...

# Email (Notifications)
SMTP_HOST=mail.apexmediation.ee
SMTP_PORT=587
```

**Setup:**
```bash
gh secret set COCOAPODS_TRUNK_TOKEN < trunk_token.txt
gh secret set NPM_TOKEN < npm_token.txt
# ... etc
```

---

**Document Version:** 1.0  
**Last Updated:** 2025-11-03  
**Maintained By:** Sabel Akhoua  
**Next Review:** After first automated release
