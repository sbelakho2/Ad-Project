# Apex

The all-Rust repository for the Apex autopilot: a tool that sits on top of
whatever ad mediation a publisher already runs (AppLovin MAX, LevelPlay or
AdMob), with no SDK in their app. It reads their data, changes their setup
through each platform's own management API, and proves every gain with that
platform's built-in A/B testing.

## What is in this repository

| Path | Purpose |
|---|---|
| `rust/crates/apex-connectors` | Management-API clients for MAX, LevelPlay and AdMob |
| `rust/crates/apex-audit` | Revenue-leak audit rules (stale floors, disabled bidders, missing per-country floors, reporting gaps, app-ads.txt errors) |
| `rust/crates/apex-autopilot` | Propose, apply, measure via platform-native A/B, keep or roll back |
| `rust/crates/apex-proof` | Signed, hash-chained change log and billing proofs |
| `rust/crates/apex-billing` | Lift-based billing: 30% of monthly holdout-measured gain, no lift no fee |
| `rust/crates/apex-lift` | The Apex Lift Engine: revenue-per-user log-ratio measurement, guardrails, keep rule, 12-month lift billing |
| `rust/crates/apex-api` | Axum server with the JSON API and the server-rendered publisher console |
| `rust/crates/apex-ui` | Design tokens, components and the shared page shell |
| `rust/crates/apex-metrics` | Bounded metrics registry and Prometheus text exposition |
| `rust/crates/apex-analytics` | ClickHouse ingest, projections and the analytics read model |
| `rust/crates/apex-control` | Operator control plane on the internal control host |
| `rust/crates/apex-sales` | Owner-only outbound sales engine |
| `rust/crates/apex-sales-web` | Owner console for the sales engine |
| `rust/crates/apex-watch` | Free daily app-ads.txt watch |
| `rust/crates/apex-notify` | Email/webhook/file notification sinks |
| `rust/crates/apex-core` | Domain contracts shared by every crate |
| `rust/crates/apex-tools` | Repository gates, dogfood harness and operational tools |
| `site/` | The Zola-built marketing site |
| `tools/coverage/` | Coverage policy data (manifest, floors, baselines) |

## Building and testing

```sh
make test    # cargo test --workspace
make fmt     # cargo fmt --all
make lint    # cargo clippy --workspace --all-targets -- -D warnings
```

## Where the design lives

Full design and governance lives under `docs/`. The coverage and CI stack is
described in `docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md`; the
CI is Kiwi CI: the only pipeline is `.kiwi/pipeline.yaml` (`kiwi run -f .kiwi/pipeline.yaml`). Pull requests run the diff-scoped `changed` and `independent-verifier` jobs; pushes to main run the full lanes.
