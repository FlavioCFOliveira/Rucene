"""Shared setup for the `tools/kg` unit tests.

The tests import the KG scripts as modules, so the script directory goes on
`sys.path`. pytest's own temporary directories default to the system temp dir,
which is tmpfs on the development machine; they are moved under
`kgio.kg_target()` unless `--basetemp` is given.
"""
import sys
from pathlib import Path

KG_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(KG_DIR))

from kgio import kg_target  # noqa: E402


def pytest_configure(config):
    if not config.option.basetemp:
        config.option.basetemp = str(kg_target() / "pytest")
