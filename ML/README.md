ML — Data & ML Pipeline Quick‑Start (FIX‑06)

_Last updated: 2025-11-18_

> **FIX-10 governance:** This README documents ML pipeline tooling and local workflows. For production readiness and ML-specific backlog items, see `docs/Internal/Deployment/PROJECT_STATUS.md` and `docs/Internal/Development/FIXES.md` (FIX-06).

Overview
- This folder contains the ML data enrichment, feature engineering, and training utilities used to produce artifacts for the inference services (`services/*`) and models in `models/`.
- FIX‑06 completes the hardening work by delivering CI lanes, manifest integrity enforcement, dependency parity, and a documented local workflow.

Prerequisites
- Python 3.11 or 3.12 recommended
- pip, virtualenv/venv

Install dependencies
```bash
# CPU lane (recommended for local dev)
python -m venv .venv && source .venv/bin/activate
pip install --upgrade pip
pip install -c ML/constraints.txt -r ML/requirements.txt

# GPU lane (optional; ensure a matching CUDA runtime)
# export TORCH_INDEX=https://download.pytorch.org/whl/cu124   # example
# pip install -c ML/constraints.txt -r ML/requirements-gpu.txt
```

Run tests
```bash
pytest ML/scripts/tests -q
```

Validate manifests
```bash
# List manifests under data/
python ML/scripts/manifest_tools.py scan data | head

# Validate one manifest (strict checksum)
python ML/scripts/manifest_tools.py validate data/enrichment/v1/manifest.json
```

Feature engineering (streaming) + tiny train
```bash
# Create a tiny parquet to exercise the pipeline
python - <<'PY'
import pandas as pd
from pathlib import Path
Path('artifacts').mkdir(exist_ok=True)
df = pd.DataFrame({
  'event_time': ['2025-01-01T00:00:00Z']*200 + ['2025-01-02T00:00:00Z']*200,
  'label': [1]*200 + [0]*200,
  'ip': ['1.2.3.4']*400,
  'placement_id': ['pl1','pl2']*200,
})
df.to_parquet('artifacts/events.parquet', index=False)
PY

# Feature engineering with schema validation
python ML/scripts/feature_engineering.py \
  --input artifacts/events.parquet \
  --out-dir artifacts/features \
  --stream --input-format parquet \
  --validate-in --validate-out --validate-limit 500

# Train a tiny model to produce sample artifacts
python ML/scripts/train_supervised_logreg.py \
  --features artifacts/features/features.parquet \
  --row-limit 500 --input-format parquet \
  --out-dir artifacts/model --validate-features --validate-limit 500
```

### ML Production Realignment & Fraud Detection ✅

The ML pipelines and production inference services are now synchronized on a 9-dimensional feature vector, ensuring model performance parity.

#### 1. Feature Shape Synchronization
- **Research & Production**: Both use the same 9-dimensional input shape (auction stats, network ASN, device types, etc.).
- **Validation**: `ML/scripts/feature_engineering.py` and `services/inference-ml/main.py` share a unified schema.

#### 2. Dynamic Fraud Drift Monitoring
- **Rolling Baseline**: Replaced hardcoded fraud distributions with a dynamic rolling baseline (7-day window).
- **PSI Analysis**: Automated Population Stability Index (PSI) calculation compares hourly traffic against the baseline to detect distribution shifts and alert on model degradation or novel fraud patterns.

### CI lanes
- PR fast lane: `.github/workflows/ml-pr.yml`
  - Installs via constraints, runs unit tests, executes manifest SoT guard, and runs a schema‑validation smoke.
- Nightly lane: `.github/workflows/ml-nightly.yml`
  - Installs via constraints, validates manifests, runs streaming FE, row‑limited train, and uploads artifacts.

Notes
- Manifest single‑source‑of‑truth (SoT) is enforced by `ML/scripts/check_manifests.py` in the PR lane.
- Dependency parity is achieved by using `ML/constraints.txt` across Dockerfiles and CI.
