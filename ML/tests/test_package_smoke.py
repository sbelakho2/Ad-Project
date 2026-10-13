"""Package smoke tests for the ML pipeline scaffold.

These exist so the ml coverage gate (`pytest tests scripts/tests --cov ...`) always has a
real, asserting suite to execute; they are the seed of the phase-1 ML test build-out
(see docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md 3.8).
"""

import sys
from pathlib import Path

# Make ML/ importable regardless of editable-install mode (tests run with cwd=ML).
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from ml_pipelines import hello  # noqa: E402


def test_package_importable_and_greets():
    assert hello() == "apexml"


def test_scripts_package_is_importable():
    import scripts  # noqa: F401,E402  (package under ML/scripts)

    assert scripts is not None
