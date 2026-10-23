# CI Environments and Branch Policy

This document describes our CI/CD environments, required checks, and branch protection policy for main/master.

## Objectives and constraints (Part 6 — 0)
- Complete Part 6 of DEVELOPMENT_TODO_CHECKLIST to a production‑ready bar.
- Support on‑device GPU training workflow and CPU‑only VPS/cloud deployment with incremental upgrades; retrain on device when needed.
- Keep changes backward‑compatible and non‑disruptive to already‑landed features.

## Environments
- dev: default branch previews and nightly synthetic probes; permissive secrets.
- staging: pre‑prod validation; synthetic probes and optional light k6; restricted secrets.
- prod: protected; release tags only; synthetic probes; alerts enabled.

## Branch protection (main/master)
Require the following status checks to pass before merging:
- ci-all (aggregate gate with multiple jobs)
  - Backend — lint, test, migrations verify
  - Console — type-check, unit/a11y
  - Android SDK — unit tests and size gate
  - iOS SDK — build and test
  - CTV/OTT — Android TV unit/assemble/size
  - CTV tvOS — Swift build/test
- security-trivy (Trivy FS scan) — required
- security-scan — optional and non-blocking unless the policy changes
- synthetic-probes — optional and non-blocking

See `.github/required-checks.json` for the exact Kiwi job names:
`changed` and `independent-verifier`. The full push jobs (`quality`,
`coverage`, `dogfood-audit`, `contracts`, live and UI lanes) run on `main`;
they are not PR status contexts.

### How to configure branch protection
1. Navigate to **GitHub → Settings → Branches**.
2. Edit the `main` (and `master` if applicable) protection rule.
3. Enable **"Require status checks to pass before merging"** and paste the list from `.github/required-checks.json`.
4. Enable **"Require review from Code Owners"** to enforce `.github/CODEOWNERS` coverage.
5. Lock force-pushes and deletions.

` .github/required-checks.json` is the canonical manifest consumed by tooling and referenced from the VERIFY-FIRST checklist.

## Secrets
Store environment‑specific secrets using GitHub Environments (dev/staging/prod):
- GITHUB_TOKEN (default)
- SNYK_TOKEN (optional)
- GITHUB_ACTOR / GITHUB_PACKAGES_URL (for Maven publish on tags)
- SYN_BASE_URL / SYN_API_URL / SYN_AUTH_TOKEN (for probes)

## Toolchain pinning
- Node.js: 18.20.4
- Java/JDK: 17 (Temurin)
- Android: SDK API 34 (with Gradle cache)
- iOS/tvOS: macos-13 runner with Xcode from image; SwiftPM
- Unity: 2020.3/2021.3/2022.3/2023.x (see unity-sdk workflow)
