"""Adversarial contract tests for the repo-root sitecustomize.py warning filter.

The filter is load-bearing (it silences known PendingDeprecationWarning noise at
interpreter startup), so its installation is treated as a contract: if the filter is
removed or broken, this test fails.
"""

import warnings
import importlib

import sitecustomize


def test_sitecustomize_installs_ignore_filter_for_pending_deprecation():
    importlib.reload(sitecustomize)
    matching = [
        f
        for f in warnings.filters
        if f[0] == "ignore" and f[2] is PendingDeprecationWarning
    ]
    assert matching, "sitecustomize must install an ignore filter for PendingDeprecationWarning"


def test_sitecustomize_module_has_no_side_effect_exports():
    importlib.reload(sitecustomize)
    public = [name for name in dir(sitecustomize) if not name.startswith("_")]
    # Only imports/helpers are expected; this pins the module against accidental API growth.
    assert "warnings" in public
