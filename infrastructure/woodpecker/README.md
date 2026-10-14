# Woodpecker deployment design

Self-hosted Woodpecker runs the gate stack. Lane inventory and expectations:
`.woodpecker/README.md`; design background:
`docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md` section 9.10.

## Topology

- Server and one Linux docker-backend agent run from this directory
  (`docker-compose.yml`, `WOODPECKER_MAX_PROCS=4`). Every lane runs there.

## Secrets

No repository secret is required for any gate. Pipeline credentials for
services are ephemeral test-only values in the pipeline files.

## Branch protection parity

Configure these required status checks in GitHub, mirroring
`.github/required-checks.json`:

- `coverage-gate` (aggregate verdict; depends on every lane)
- `dogfood-audit` (repository invariants)
- `contract-coverage` (infrastructure and content contracts)

## Bring-up

```sh
cd infrastructure/woodpecker
cp /dev/null .env  # then set WOODPECKER_HOST, WOODPECKER_AGENT_SECRET, WOODPECKER_GITHUB_CLIENT, WOODPECKER_GITHUB_SECRET
docker compose up -d
```

Then in the Woodpecker UI enable the GitHub repo, and keep
`tools/coverage/coverage-floor.json` and `tools/coverage/suites.json` under
CODEOWNERS review as `.github/CODEOWNERS` already does.
