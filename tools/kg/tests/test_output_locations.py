"""Defect 1: no KG tool defaults its output to `/tmp` or a `tempfile` directory."""
import argparse
import ast
import os
from pathlib import Path

import pytest

import kgio

KG_DIR = Path(kgio.__file__).resolve().parent
SCRIPTS = sorted(KG_DIR.glob("*.py"))
CANONICAL_TARGET = Path("/data/dev/github.com/FlavioCFOliveira/.rucene-kg-target")
# The reference Lucene clone is an *input*, at the path `CLAUDE.md` 16.1 names.
LUCENE_CLONE = "/tmp/lucene1050"


class _Parsed(Exception):
    def __init__(self, ns):
        self.ns = ns


def defaults_of(monkeypatch, main, argv):
    """The parsed arguments `main` would run with, without running it."""
    original = argparse.ArgumentParser.parse_args

    def capture(self, args=None, namespace=None):
        raise _Parsed(original(self, argv, namespace))

    monkeypatch.setattr(argparse.ArgumentParser, "parse_args", capture)
    with pytest.raises(_Parsed) as exc:
        main()
    return exc.value.ns


def test_default_target_is_the_sibling_kg_target(monkeypatch):
    monkeypatch.delenv(kgio.KG_TARGET_ENV, raising=False)
    assert kgio.DEFAULT_KG_TARGET == CANONICAL_TARGET
    assert kgio.kg_target() == CANONICAL_TARGET


def test_env_overrides_target(monkeypatch, tmp_path):
    monkeypatch.setenv(kgio.KG_TARGET_ENV, str(tmp_path))
    assert kgio.kg_target() == tmp_path


@pytest.mark.parametrize("script", SCRIPTS, ids=lambda p: p.name)
def test_no_tempfile_and_no_tmp_literal_but_the_lucene_clone(script):
    tree = ast.parse(script.read_text(encoding="utf-8"))
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            assert "tempfile" not in {a.name for a in node.names}
        if isinstance(node, ast.ImportFrom):
            assert node.module != "tempfile"
        if isinstance(node, ast.Attribute):
            assert node.attr not in {"gettempdir", "mkdtemp", "mkstemp",
                                     "NamedTemporaryFile", "TemporaryDirectory"}
    # A `/tmp` string is allowed only where it names the Lucene clone (an input)
    # or is part of a docstring/Java fixture text, never as an output default.
    for node in ast.walk(tree):
        if isinstance(node, ast.keyword) and node.arg == "default":
            if isinstance(node.value, ast.Constant) and isinstance(node.value.value, str):
                v = node.value.value
                assert not v.startswith("/tmp") or v.startswith(LUCENE_CLONE), v


def _under(path, root):
    return Path(path).resolve().is_relative_to(root.resolve())


@pytest.fixture
def target(monkeypatch, tmp_path):
    monkeypatch.setenv(kgio.KG_TARGET_ENV, str(tmp_path))
    return tmp_path


def test_port_evidence_out_default(monkeypatch):
    import port_evidence_kg as m
    ns = defaults_of(monkeypatch, m.main, ["rucene"])
    assert _under(ns.out, kgio.kg_target())
    assert Path(ns.out).name == "port_evidence.json"


def test_expanded_evidence_out_default(monkeypatch):
    import expanded_evidence_kg as m
    ns = defaults_of(monkeypatch, m.main, ["rucene"])
    assert _under(ns.out, kgio.kg_target())


def test_lucene_tests_out_default(monkeypatch):
    import lucene_tests_kg as m
    ns = defaults_of(monkeypatch, m.main, ["rucene", "--commit", "c", "--date", "d"])
    assert _under(ns.out, kgio.kg_target())


def test_enrich_output_dir_default(monkeypatch):
    import enrich_lucene_kg as m
    ns = defaults_of(monkeypatch, m.main, [])
    assert _under(ns.output_dir, kgio.kg_target())


def test_extract_lucene_output_dir_default():
    import extract_lucene_kg as m
    assert _under(m.parser.parse_args([]).output_dir, kgio.kg_target())


def test_load_members_input_dir_default(monkeypatch):
    import load_members_unwind as m
    ns = defaults_of(monkeypatch, m.main, ["rucene"])
    assert _under(ns.input_dir, kgio.kg_target())


def test_load_rucene_dropped_report_default(monkeypatch):
    import load_rucene_kg as m
    ns = defaults_of(monkeypatch, m.main, ["rucene", "--survey", "s.json"])
    assert _under(ns.dropped_ports_report, kgio.kg_target())


def test_expand_builds_in_kg_target(monkeypatch, target):
    import extract_rucene_kg as m
    seen = {}

    class Done:
        returncode = 0
        stdout = "expanded"
        stderr = ""

    def fake_run(cmd, cwd, capture_output, text, env):
        seen["env"] = env
        return Done()

    monkeypatch.setattr(m.subprocess, "run", fake_run)
    assert m.run_macro_expansion(Path(".")) == "expanded"
    assert Path(seen["env"]["CARGO_TARGET_DIR"]) == target
    assert not seen["env"]["CARGO_TARGET_DIR"].startswith("/tmp")


def test_pytest_basetemp_is_not_tmp(tmp_path):
    assert not str(tmp_path).startswith("/tmp/")
    assert not str(tmp_path).startswith(os.path.realpath("/tmp") + "/")
