"""Task #509: `retirement_check.py` passes without touching the graph.

The self-check answers every graph call from its in-memory fixture. Since
`30349af`, `port_evidence_kg.load` routes its `PORTS` writes through
`kgio.item_labels` and `kgio.unwind_items`; while those were not faked, the
label lookup read the live `rucene` graph, found none of the fixture's items,
and dropped every row, so the "deleted file port is retired" check failed.

The script is run in a subprocess, because it patches `port_evidence_kg` and
`load_rucene_kg` at module level, with `kgio.client` -- the single entry point
of every graph statement -- replaced by a function that fails the run.
"""
import subprocess
import sys

from conftest import KG_DIR

GUARD = """
import runpy, sys
sys.path.insert(0, {kg!r})
import kgio

def no_client(roadmap, query, *args, **kwargs):
    print(f"GRAPH CLIENT OPENED: {{query}}", file=sys.stderr)
    raise SystemExit(97)

kgio.client = no_client
sys.argv = [{script!r}]
runpy.run_path({script!r}, run_name="__main__")
"""


def test_retirement_check_passes_without_a_graph_client():
    script = str(KG_DIR / "retirement_check.py")
    proc = subprocess.run(
        [sys.executable, "-c", GUARD.format(kg=str(KG_DIR), script=script)],
        capture_output=True, text=True, encoding="utf-8", timeout=120)
    assert "GRAPH CLIENT OPENED" not in proc.stderr, proc.stderr
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert "failures: 0" in proc.stdout, proc.stdout
    assert "OK the deleted file's doc-comment port is retired" in proc.stdout
