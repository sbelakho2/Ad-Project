# Woodpecker deployment design

Self-hosted Woodpecker replaces/augments GitHub Actions as the 100%-coverage gate
stack (design doc: `docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md` 7).

## Topology

- **Server** (this compose file, app-server Hetzner host): Woodpecker v2 server + one
  Linux docker-backend agent (`WOODPECKER_MAX_PROCS=4`).
- **darwin/arm64 agent** (a macOS host, e.g. Mac mini): runs `sdk-apple-coverage` via the
  **local backend** (`WOODPECKER_BACKEND=local`, agent registered with label
  `platform=darwin/arm64`). swift/xcrun run natively; no docker images are used there
  (`.woodpecker/sdk-apple-coverage.yml` documents this).
- **Optional GPU/ML runner**: not needed for gates; ML pipeline runs on the Linux agent.

## Required secrets (repo settings in Woodpecker UI)

| Secret | Used by | Notes |
|---|---|---|
| `codecov_token` (optional) | push-only upload steps | PR builds never receive secrets; gates do not depend on it |

No other secrets are required for any coverage gate. Pipeline credentials for services
are ephemeral test-only values hardcoded in the pipeline files (same pattern as
`ci-all.yml`).

## Branch protection parity

Configure required status checks (GitHub Settings, mirroring `.github/required-checks.json`):

- `coverage-gate` (aggregate verdict; depends on all coverage pipelines)
- `dogfood-audit` (manifest orphan gate, ratchet, pragma/skip ban, assertion presence)
- `contract-coverage` (IaC/OpenAPI/CI-integrity/docs/content/samples)

## Bring-up

```sh
cd infrastructure/woodpecker
cp /dev/null .env  # then set WOODPECKER_HOST, WOODPECKER_AGENT_SECRET, WOODPECKER_GITHUB_CLIENT, WOODPECKER_GITHUB_SECRET
docker compose up -d
```

Then in the Woodpecker UI: enable the GitHub repo, and mark
`tools/coverage/coverage-floor.json` + `tools/coverage/suites.json` as protected files
via CODEOWNERS-style review (single-maintainer repo: rely on branch protection).
