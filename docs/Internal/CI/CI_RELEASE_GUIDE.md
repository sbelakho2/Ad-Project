# CI/CD Release Guide

Complete guide for continuous integration, SDK releases, publishing, and rollback procedures for the ApexMediation platform.

## Table of Contents

1. [CI/CD Overview](#cicd-overview)
2. [SDK Release Process](#sdk-release-process)
3. [Publishing Workflows](#publishing-workflows)
4. [Rollback Procedures](#rollback-procedures)
5. [Troubleshooting](#troubleshooting)

## CI/CD Overview

### Workflow Files

The platform uses the following GitHub Actions workflows:

| Workflow | Purpose | Trigger |
|----------|---------|---------|
| `ci.yml` | Comprehensive CI for all components | Push, PR to main |
| `ci-all.yml` | Extended CI with additional platforms | Push to main, workflow_dispatch |
| `sdk-release.yml` | **Primary SDK release workflow** | Tag push (v*.*.*) |
| `release-sdks.yml` | Legacy SDK release (deprecated) | Tag push (v*) |
| `docker-build.yml` | Docker image builds | Push to main, PR |
| `deploy-staging.yml` | Deploy to staging environment | Push to main |
| `deploy-production.yml` | Deploy to production | Manual approval |
| `synthetic-probes.yml` | Health checks across environments | Cron (daily 3 AM), workflow_dispatch |
| `security.yml` | Security scanning | Push, PR, schedule |
| `codeql.yml` | CodeQL security analysis | Push to main, PR, schedule |

### Single Source of Truth

**⚠️ IMPORTANT: Use `sdk-release.yml` for all SDK releases.**

- **Primary:** `.github/workflows/sdk-release.yml` - Full-featured SDK release with changelog generation, XCFramework builds, Dokka docs
- **Deprecated:** `.github/workflows/release-sdks.yml` - Kept for backward compatibility only

The `release-sdks.yml` workflow remains to support existing tag-based releases but redirects developers to use `sdk-release.yml` going forward.

### CI Pipeline Structure

```
┌─────────────┐
│  ci.yml     │  Main CI pipeline (all PRs/pushes)
└──────┬──────┘
       │
       ├── Backend Tests (Jest + Supertest)
       ├── Console Tests (Jest + Playwright)
       ├── Android SDK (Gradle + Dokka)
       ├── iOS SDK (SwiftPM + XCTest)
      ├── Unity Footprint Gate (compressed runtime + .NET tests)
      ├── Unity SDK (Package validation)
       └── ML Pipeline (Python + pytest)

┌─────────────┐
│ ci-all.yml  │  Extended CI (main branch + manual)
└──────┬──────┘
       │
       ├── Cross-platform compatibility
       ├── CTV SDK builds
       ├── Performance benchmarks
       └── Integration test suites
```

## SDK Release Process

### Prerequisites

1. **Version Bump:** Update version numbers in platform-specific files:
   - Android: `sdk/core/android/build.gradle` (`version = "X.Y.Z"`)
   - iOS: `sdks/ios/ApexMediation.podspec` (`s.version = "X.Y.Z"`)
   - Unity: `Packages/com.rivalapexmediation.sdk/package.json` (`"version": "X.Y.Z"`)

2. **Changelog:** Use conventional commits for automatic changelog generation:
   ```
   feat: Add new bidding adapter
   fix: Resolve memory leak in auction engine
   BREAKING CHANGE: Remove deprecated API method
   ```

3. **Testing:** Ensure all CI tests pass on main branch:
   ```bash
   # Check CI status
   gh workflow view ci.yml
   ```

### Release Steps

#### 1. Create Semantic Version Tag

```bash
# Standard release
git tag v1.2.3
git push origin v1.2.3

# Pre-release
git tag v1.2.3-beta.1
git push origin v1.2.3-beta.1
```

This triggers the `sdk-release.yml` workflow automatically.

#### 2. Workflow Execution

The `sdk-release.yml` workflow performs the following jobs:

1. **validate-version** - Validates semantic versioning format
2. **generate-changelog** - Creates changelog from conventional commits
3. **build-ios-sdk** - Builds XCFramework for iOS devices + simulator
4. **build-android-sdk** - Builds AAR and runs tests
5. **unity-footprint-gate** - Runs `sdk/core/unity/scripts/check_package_constraints.sh` to enforce the 100KB compressed runtime limit and execute the Unity .NET tests before any platform-specific builds continue
6. **build-unity-sdk** - Packages UPM tarball
7. **publish-ios** - Publishes to CocoaPods (if configured)
8. **publish-android** - Publishes to Maven Central/GitHub Packages
9. **publish-unity** - Publishes to UPM registry
10. **create-github-release** - Creates GitHub Release with artifacts

#### 3. Monitor Release

```bash
# View release workflow status
gh run list --workflow=sdk-release.yml

# View specific run details
gh run view <run-id>

# Download artifacts
gh run download <run-id>
```

#### 4. Verify Artifacts

After successful release, verify the following artifacts are attached:

- **iOS:**
  - `ApexMediation.xcframework.zip` - XCFramework bundle
  - `ios-spm-source.tar.gz` - Swift Package source
- **Android:**
  - `apexmediation-android-X.Y.Z.aar` - Android library
  - `android-sdk-api-docs` - Dokka HTML documentation
- **Unity:**
  - `unity-upm-package.tgz` - Unity Package Manager tarball

### Version Strategy

| Version Type | Example | When to Use |
|--------------|---------|-------------|
| Patch | v1.0.1 | Bug fixes, minor updates |
| Minor | v1.1.0 | New features (backward compatible) |
| Major | v2.0.0 | Breaking changes |
| Pre-release | v1.1.0-beta.1 | Testing before stable release |

## Publishing Workflows

### iOS - CocoaPods

**Manual Publishing (current):**

```bash
# Validate podspec
cd sdks/ios
pod lib lint ApexMediation.podspec --allow-warnings

# Publish to CocoaPods Trunk
pod trunk push ApexMediation.podspec --allow-warnings
```

**Automated (configured in sdk-release.yml):**

Set up CocoaPods trunk token as GitHub secret `COCOAPODS_TRUNK_TOKEN`.

### iOS - Swift Package Manager (SPM)

SPM uses Git tags automatically. Developers add dependency via:

```swift
dependencies: [
    .package(url: "https://github.com/rivalapex/sdk-ios", from: "1.0.0")
]
```

### iOS - XCFramework Distribution

XCFramework is built for both device and simulator architectures:

```bash
# Download from GitHub Release
curl -LO https://github.com/rivalapex/ad-project/releases/download/v1.2.3/ApexMediation.xcframework.zip

# Unzip and drag into Xcode project
unzip ApexMediation.xcframework.zip
```

### Android - Maven Central

**Setup:**

1. Create Sonatype OSSRH account
2. Generate GPG key for signing
3. Add secrets to GitHub:
   - `OSSRH_USERNAME`
   - `OSSRH_PASSWORD`
   - `SIGNING_KEY_ID`
   - `SIGNING_PASSWORD`
   - `SIGNING_SECRET_KEY_RING_FILE` (base64 encoded)

**Publishing:**

```bash
# Manual publish
cd sdk/core/android
./gradlew publishReleasePublicationToSonatypeRepository
./gradlew closeAndReleaseSonatypeStagingRepository
```

Automated in `sdk-release.yml` when secrets are configured.

### Android - GitHub Packages

Already configured in `release-sdks.yml`:

```bash
# Developers add to build.gradle:
repositories {
    maven {
        url = uri("https://maven.pkg.github.com/rivalapex/ad-project")
        credentials {
            username = project.findProperty("gpr.user") ?: System.getenv("GITHUB_ACTOR")
            password = project.findProperty("gpr.key") ?: System.getenv("GITHUB_TOKEN")
        }
    }
}
```

### Android - Dokka Documentation

Generated automatically in CI (`ci.yml` line 172-183):

```bash
# Local generation
cd sdk/core/android
./gradlew generateApiDocs

# Output: sdk/core/android/build/dokka/html
```

Uploaded as artifact `android-sdk-api-docs` for each CI run.

### Unity - UPM (Unity Package Manager)

**Git URL Installation:**

```json
{
  "dependencies": {
    "com.rivalapexmediation.sdk": "https://github.com/rivalapex/ad-project.git?path=Packages/com.rivalapexmediation.sdk#v1.2.3"
  }
}
```

**Tarball Installation:**

1. Download `unity-upm-package.tgz` from GitHub Release
2. In Unity: **Window → Package Manager → + → Add package from tarball**

## Rollback Procedures

### Quick Rollback Checklist

1. **Identify Issue:** Crash reports, error rates, user feedback
2. **Determine Scope:** Which platform(s) affected?
3. **Execute Rollback:** Follow platform-specific steps below
4. **Verify:** Check metrics and probes
5. **Communicate:** Notify team and users
6. **Post-Mortem:** Document root cause and prevention

### Backend Rollback

#### Production Deployment

```bash
# Use rollback workflow
gh workflow run rollback.yml \
  -f environment=production \
  -f previous_version=v1.2.2 \
  -f reason="Critical bug in billing module"

## Manual rollback (Hetzner/PM2)

Rollbacks are performed by redeploying a known-good git ref to the Hetzner server.

```bash
git checkout v1.2.2
./scripts/deploy-all.sh backend
```
```

#### Test Server Rollback

```bash
git checkout <known-good-ref>
DEPLOY_SERVER=157.90.126.205 ./scripts/deploy-all.sh backend
```

#### Database Migrations

If rollback requires database changes:

```bash
# Revert migrations
cd backend
npx ts-node migrations/down/<timestamp>-revert-feature.ts

# Or restore from backup (see Hetzner DB runbooks)
# See: docs/PITR_RECOVERY.md
```

### SDK Rollback

#### iOS - CocoaPods

```ruby
# Users update Podfile to previous version:
pod 'ApexMediation', '~> 1.2.2'

# Then run:
pod update ApexMediation
```

#### iOS - SPM

```swift
// Users update Package.swift:
.package(url: "https://github.com/rivalapex/sdk-ios", .exact("1.2.2"))
```

#### Android - Maven/Gradle

```kotlin
// Users update build.gradle:
implementation("com.rivalapex:apexmediation:1.2.2")

// Sync project
./gradlew clean build
```

#### Unity - UPM

```json
// Users update manifest.json:
{
  "dependencies": {
    "com.rivalapexmediation.sdk": "https://github.com/rivalapex/ad-project.git?path=Packages/com.rivalapexmediation.sdk#v1.2.2"
  }
}
```

### Emergency SDK Yank

**⚠️ Use only for critical security issues**

#### CocoaPods

```bash
# Deprecate version
pod trunk deprecate ApexMediation --version 1.2.3 \
  --message "Security vulnerability - upgrade to 1.2.4 immediately"
```

#### Maven Central

Cannot delete published versions. Publish fixed version and communicate urgency.

#### GitHub Release

```bash
# Delete release (does not delete Git tag)
gh release delete v1.2.3 --yes

# Delete tag
git push --delete origin v1.2.3
```

### Monitoring After Rollback

```bash
# Check synthetic probes
gh run list --workflow=synthetic-probes.yml

# View Grafana dashboards (Hetzner Production)
open http://46.62.252.96:3001/d/apex-rtb-analytics

# Check error rates
curl 'http://46.62.252.96:9090/api/v1/query?query=rate(http_request_duration_seconds_count{status_code=~"5.."}[5m])'
```

## Troubleshooting

### CI Failures

#### Android Build Fails

```bash
# Clear Gradle cache locally
cd sdk/core/android
./gradlew clean --no-daemon
./gradlew assembleRelease --no-daemon --stacktrace

# In CI, add to workflow:
- name: Clear Gradle cache
  run: |
    rm -rf ~/.gradle/caches/
    rm -rf ~/.gradle/wrapper/
```

#### iOS Build Fails

```bash
# Clear Xcode derived data
rm -rf ~/Library/Developer/Xcode/DerivedData

# Reset CocoaPods
cd sdks/ios
rm -rf Pods/ Podfile.lock
pod install --repo-update
```

#### Dokka Generation Fails

```bash
# Check Kotlin version compatibility
./gradlew dependencies | grep kotlin

# Update Dokka version in build.gradle:
id 'org.jetbrains.dokka' version '1.9.20'
```

### Release Workflow Failures

#### XCFramework Creation Fails

```bash
# Verify scheme exists
xcodebuild -list -workspace ApexMediation.xcworkspace

# Build manually to debug
xcodebuild archive \
  -scheme ApexMediation \
  -destination "generic/platform=iOS" \
  -archivePath build/test.xcarchive \
  -showBuildSettings
```

#### GitHub Release Creation Fails

```bash
# Check token permissions
# Required scopes: contents: write, packages: write

# Manually create release
gh release create v1.2.3 \
  --title "Release v1.2.3" \
  --notes "See CHANGELOG.md" \
  artifacts/*.zip artifacts/*.aar artifacts/*.tgz
```

### Publishing Failures

#### CocoaPods Trunk Push Fails

```bash
# Re-authenticate
pod trunk register email@example.com 'Your Name'

# Verify trunk token
pod trunk me

# Push with verbose logging
pod trunk push ApexMediation.podspec --allow-warnings --verbose
```

#### Maven Central Staging Fails

```bash
# Close staging repository manually
./gradlew closeSonatypeStagingRepository --info

# View staging repositories
open https://oss.sonatype.org/#stagingRepositories
```

### Common Errors

#### Error: "Tag already exists"

```bash
# Delete local tag
git tag -d v1.2.3

# Delete remote tag
git push --delete origin v1.2.3

# Create corrected tag
git tag v1.2.3
git push origin v1.2.3
```

#### Error: "Artifact upload failed"

```bash
# Check artifact size (GitHub limit: 2GB)
ls -lh artifacts/

# Split large artifacts or use external storage
```

#### Error: "Workflow run limit exceeded"

```bash
# Wait for quota reset or use self-hosted runners
# GitHub free tier: 2,000 minutes/month
# See: https://github.com/settings/billing
```

## Environment Secrets and Permissions

### Required GitHub Secrets

Configure the following secrets in **Settings → Secrets and variables → Actions**:

#### Backend Deployment
| Secret | Purpose | How to Generate |
|--------|---------|-----------------|
| `DATABASE_URL` | Production database | PostgreSQL connection string |
| `JWT_SECRET` | JWT signing | `openssl rand -base64 32` |
| `APEX_ADYEN_API_KEY` | Payment processing | Adyen dashboard → API keys |
| `APEX_ADYEN_HMAC_KEY` | Webhook verification | Adyen dashboard → Webhooks |
| `CLICKHOUSE_URL` | Analytics database | ClickHouse HTTP endpoint |
| `REDIS_URL` | Cache/queue | Redis connection string |

#### SDK Publishing
| Secret | Purpose | How to Generate |
|--------|---------|-----------------|
| `COCOAPODS_TRUNK_TOKEN` | CocoaPods publishing | `pod trunk me` (session token) |
| `OSSRH_USERNAME` | Maven Central | Sonatype OSSRH account |
| `OSSRH_PASSWORD` | Maven Central | Sonatype OSSRH password |
| `SIGNING_KEY_ID` | Android signing | Last 8 chars of GPG key ID |
| `SIGNING_PASSWORD` | Android signing | GPG key passphrase |
| `SIGNING_SECRET_KEY_RING_FILE` | Android signing | `base64 < ~/.gnupg/secring.gpg` |
| `NPM_TOKEN` | Unity/npm publishing | `npm token create` |

#### Monitoring & Alerts
| Secret | Purpose | How to Generate |
|--------|---------|-----------------|
| `SLACK_WEBHOOK_URL` | CI notifications | Slack app → Incoming Webhooks |
| `STAGING_BASE_URL` | E2E test target | `https://staging.rival.com` |
| `STAGING_API_TOKEN` | E2E authentication | Generate API token in staging |
| `STAGING_KUBECONFIG` | Chaos testing | `cat ~/.kube/config \| base64` |

#### Security Scanning
| Secret | Purpose | How to Generate |
|--------|---------|-----------------|
| `SNYK_TOKEN` | Vulnerability scanning | Snyk dashboard → API token |
| `CODECOV_TOKEN` | Code coverage | Codecov dashboard → Repository token |

### Repository Permissions

#### Required Workflow Permissions

In **Settings → Actions → General → Workflow permissions**:
- ✅ Read and write permissions
- ✅ Allow GitHub Actions to create and approve pull requests

#### Branch Protection Rules

**Main branch** (`Settings → Branches → main`):
- ✅ Require pull request reviews (minimum 1 approval)
- ✅ Require status checks to pass:
  - `backend-tests`
  - `console-tests`
  - `android-sdk-build`
  - `ios-sdk-build`
  - `shellcheck`
  - `promtool-validate`
- ✅ Require conversation resolution
- ✅ Require linear history
- ❌ Do not require signed commits (optional)
- ✅ Include administrators (enforce for everyone)

#### Environment Protection

**Production environment** (`Settings → Environments → production`):
- ✅ Required reviewers: Add DevOps team members
- ✅ Wait timer: 10 minutes (allows cancellation)
- ✅ Deployment branches: Only `main`

**Staging environment** (`Settings → Environments → staging`):
- ✅ Deployment branches: `main`, `staging`
- ❌ No required reviewers (auto-deploy)

### Setting Up Secrets

#### Via GitHub UI

1. Navigate to **Settings → Secrets and variables → Actions**
2. Click **New repository secret**
3. Enter the secret name
4. Paste value
5. Click **Add secret**

#### Via GitHub CLI

```bash
# Set from file
gh secret set SIGNING_SECRET_KEY_RING_FILE < ~/.gnupg/secring.gpg.b64

# Set multiple secrets from .env file
while IFS='=' read -r key value; do
  gh secret set "$key" --body "$value"
done < production.env
```

#### Via Terraform (Infrastructure as Code)

```hcl
resource "github_actions_secret" "jwt_secret" {
  repository       = "ad-project"
  secret_name      = "JWT_SECRET"
  plaintext_value  = var.jwt_secret
}
```

### Secrets Rotation Schedule

| Secret | Rotation Frequency | Owner |
|--------|-------------------|-------|
| `JWT_SECRET` | Annually | Backend team |
| `APEX_ADYEN_API_KEY` | Never (rotate via Adyen dashboard) | Finance |
| `DATABASE_URL` | Never (change password if compromised) | DevOps |
| `COCOAPODS_TRUNK_TOKEN` | Annually | iOS team |
| `OSSRH_PASSWORD` | Annually | Android team |
| `SLACK_WEBHOOK_URL` | Only if leaked | DevOps |

### Verifying Secrets

```bash
# Check which secrets are configured (values hidden)
gh secret list

# Test secret in workflow
gh workflow run test-secrets.yml
```

### Security Best Practices

1. **Never commit secrets** to Git (use `.env.example` templates instead)
2. **Use environment-specific secrets** (separate staging/production)
3. **Rotate tokens regularly** (see schedule above)
4. **Audit secret access** via GitHub audit log
5. **Limit secret scope** (create tokens with minimum required permissions)
6. **Use Dependabot secrets** for automated security updates

### Access Control

#### Who Can Trigger Releases?

**SDK Releases** (`sdk-release.yml`):
- Triggered by: Git tag push (`v*.*.*`)
- Required permission: Write access to repository
- Team: Maintainers, Release managers

**Production Deployment** (`deploy-production.yml`):
- Triggered by: Manual approval (workflow_dispatch)
- Required permission: Admin or maintainer
- Environment: `production` (requires reviewer approval)
- Team: DevOps, CTO

**Staging Deployment** (`deploy-staging.yml`):
- Triggered by: Push to `main` branch
- Required permission: Write access (automatic via PR merge)
- Team: All developers

### Emergency Access

In case of emergency (e.g., critical security patch):

1. **Skip CI checks** (admin only):
   ```bash
   git push --force-with-lease origin main
   ```

2. **Manual deployment bypass**:
   ```bash
  # Direct Hetzner deployment
  ./scripts/deploy-all.sh backend
   ```

3. **Hotfix workflow**:
   ```bash
   # Create hotfix branch from production tag
   git checkout -b hotfix/v1.2.4 v1.2.3
   
   # Apply fix and commit
   git commit -m "fix: critical security patch"
   
   # Tag and push
   git tag v1.2.4
   git push origin v1.2.4
   
   # Merge back to main
   git checkout main
   git merge hotfix/v1.2.4
   git push origin main
   ```

## Best Practices

1. **Test Before Tagging:** Always ensure CI passes on main before creating release tags
2. **Semantic Versioning:** Follow semver strictly to avoid breaking user integrations
3. **Changelog Discipline:** Use conventional commits for automatic changelog generation
4. **Staging First:** Deploy to staging and run synthetic probes before production
5. **Gradual Rollout:** Use feature flags for gradual rollout of breaking changes
6. **Monitor Post-Release:** Watch dashboards for 24h after major releases
7. **Document Issues:** Add troubleshooting steps to this guide when new issues arise
8. **Rotate Secrets Regularly:** Follow the rotation schedule in this guide
9. **Review Permissions Quarterly:** Audit who has access to production deployments

## Related Documentation

- [Development Checklist](../Internal/Development/DEVELOPMENT_TODO_CHECKLIST.md)
- [Grafana Dashboards](../Monitoring/GRAFANA_DASHBOARDS.md)
- [Alerts Guide](../Monitoring/ALERTS.md)
- [Synthetic Probes](../../.github/workflows/synthetic-probes.yml)
- [Security Scanning](../../.github/workflows/security.yml)

## Support

For CI/CD issues:
- **Slack:** #ci-cd-support
- **Email:** devops@rival.com
- **On-call:** PagerDuty escalation policy
