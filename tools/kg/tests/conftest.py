"""Shared setup for the `tools/kg` unit tests.

The tests import the KG scripts as modules, so the script directory goes on
`sys.path`. pytest's own temporary directories default to the system temp dir,
which is tmpfs on the development machine; they are moved under
`kgio.kg_target()` unless `--basetemp` is given.
"""
import sys
from pathlib import Path

import pytest

KG_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(KG_DIR))

from kgio import kg_target  # noqa: E402


def pytest_configure(config):
    if not config.option.basetemp:
        config.option.basetemp = str(kg_target() / "pytest")


@pytest.fixture(autouse=True)
def _hermetic_library_roots(monkeypatch, tmp_path_factory):
    """Point the sprint-17 library clones (`port_modules_kg.repo_roots`) at empty
    directories, so a test that passes only the Lucene and randomizedtesting
    roots never reads the real spatial4j / s2-geometry clones under `/tmp`."""
    base = tmp_path_factory.mktemp("libroots")
    monkeypatch.setenv("SPATIAL4J_ROOT", str(base / "spatial4j"))
    monkeypatch.setenv("S2_GEOMETRY_ROOT", str(base / "s2-geometry"))
