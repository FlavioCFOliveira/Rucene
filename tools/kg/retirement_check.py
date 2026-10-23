#!/usr/bin/env python3
"""Proof that a port cannot outlive its declaration.

Until 2026-09-06 nothing in this pipeline could retire a `PORTS` edge:
`port_evidence_kg.py --phase load` only superseded `PORTS_CANDIDATE` edges, and
`load_rucene_kg.py --phase prune` refused to delete any node carrying `PORTS`.
A deleted file therefore left a phantom port for ever -- `TwoPhaseCommitTool`
read `ported` at 9cf23e5 on a doc comment in `src/index/two_phase_commit.rs`,
which that very commit deleted -- and coverage was inflated silently for every
file the crate will ever delete. `CLAUDE.md` 6.1 requires coverage to be
defensible, so both halves now yield, each in its own lane.

This check exercises them off the store, against a hand-built graph, so the
guarantee is reproducible and cheap to re-run:

    python3 tools/kg/retirement_check.py

It asserts the four things that together close the deadlock:

1. a `doc-comment` port whose declaring file is gone is retired, and the node
   and `File` behind it are pruned;
2. a `curated` port and a `macro-expansion` port are *not* pruned -- the crate
   survey is not the authority on either;
3. a port whose claim is still declared is left alone;
4. a member-level claim resolves to its enclosing type, while a nested type
   goes on resolving as a type and an unconfirmable name stays unresolved.

It writes nothing anywhere: every `rmp graph` call the loaders make is
intercepted and answered from the in-memory model below.
"""
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import load_rucene_kg as lo
import port_evidence_kg as pe

FAILURES = []


def check(label, got, want):
    ok = got == want
    print(f"{'OK ' if ok else 'XX '}{label}")
    if not ok:
        print(f"     want: {want}\n      got: {got}")
        FAILURES.append(label)


# ---------------------------------------------------------------------------
# The graph the loaders will see
# ---------------------------------------------------------------------------
CLASSES = {
    "pkg.Tool": {"execute", "rollback"},
    "pkg.TotalHits": set(),
    "pkg.TotalHits$Relation": set(),
    "pkg.Ghost": set(),
    "pkg.Live": set(),
    "pkg.Curated": set(),
    "pkg.Macro": set(),
}
RUST_TYPES = [("Relation", "src/hits.rs"), ("Live", "src/live.rs"),
              ("Ghost", "src/gone.rs"), ("Curated", "src/gone.rs"),
              ("MacroType", "src/expanded.rs")]
RUST_FNS = [("execute", "src/tool.rs")]
# (name, file, lucene, evidence, declarationSource)
PORTS = [
    ("Ghost", "src/gone.rs", "pkg.Ghost", "doc-comment", None),
    ("Curated", "src/gone.rs", "pkg.Curated", "curated", None),
    ("MacroType", "src/expanded.rs", "pkg.Macro", "doc-comment", "macro-expansion"),
    ("Live", "src/live.rs", "pkg.Live", "doc-comment", None),
]
CLAIMS = [
    {"file": "src/live.rs", "line": 3, "target": "Live", "scope": "item",
     "lucene": "pkg.Live", "note": None},
    {"file": "src/tool.rs", "line": 9, "target": "execute", "scope": "item",
     "lucene": "pkg.Tool.execute", "note": None},
    {"file": "src/hits.rs", "line": 4, "target": "Relation", "scope": "item",
     "lucene": "pkg.TotalHits.Relation", "note": None},
    {"file": "src/tool.rs", "line": 20, "target": "execute", "scope": "item",
     "lucene": "pkg.Tool.notAMember", "note": None},
]

writes = []


def fake_read_rows(_roadmap, query):
    """`kgio.read`: rows as lists, for `port_evidence_kg`."""
    if "MATCH (c:Class) RETURN c.qualifiedName" in query:
        return [[c] for c in CLASSES]
    if "(m:Method) RETURN c.qualifiedName, m.name" in query:
        return [[c, m] for c, ms in CLASSES.items() for m in ms]
    if "RETURN t.name, t.file" in query and "PORTS" not in query:
        return [list(t) for t in RUST_TYPES]
    if "MATCH (co:Component)" in query:
        return []
    if "MATCH (f:RustFn) RETURN f.name, f.file" in query:
        return [list(f) for f in RUST_FNS]
    if "e.evidence = 'doc-comment'" in query and "e.declarationSource IS NULL" in query:
        return [[n, f, l] for n, f, l, ev, ds in PORTS
                if ev == "doc-comment" and ds is None]
    raise AssertionError(f"unexpected read: {query}")


def fake_read_dicts(_roadmap, query):
    """`load_rucene_kg.read`: rows as dicts."""
    if "RETURN t.name AS name, t.file AS file" in query and "PORTS" not in query:
        return [{"name": n, "file": f} for n, f in RUST_TYPES]
    if "MATCH (f:RustFn) RETURN f.qualifiedName AS qn" in query:
        return [{"qn": f"{f}::{n}"} for n, f in RUST_FNS]
    if "e.evidence = 'curated'" in query and "RETURN t.name AS name" in query:
        return [{"name": n, "file": f} for n, f, _l, ev, ds in PORTS
                if ev == "curated" or ds == "macro-expansion"]
    if "e.evidence = 'curated'" in query and "RETURN f.qualifiedName AS qn" in query:
        return []
    if "MATCH (t)-[:PORTS]->(:Class) RETURN t.name AS name" in query:
        return [{"name": n, "file": f} for n, f, _l, _e, _d in PORTS]
    if "MATCH (f:File)" in query:
        return [{"path": p} for p in
                ("src/live.rs", "src/tool.rs", "src/hits.rs", "src/gone.rs",
                 "src/expanded.rs")]
    raise AssertionError(f"unexpected read: {query}")


pe.kg_read = fake_read_rows
pe.unwind = lambda _r, mode, rows, body, label, batch=200: writes.append(
    (mode, label, [dict(x) for x in rows]))
lo.read = fake_read_dicts
lo.run_unwind = lambda mode, _r, rows, body, batch, label: writes.append(
    (mode, label, [dict(x) for x in rows]))


def written(label):
    return [r for mode, lab, rows in writes if lab == label for r in rows]


# ---------------------------------------------------------------------------
# 1. the evidence pass
# ---------------------------------------------------------------------------
print("port_evidence_kg.load")
edges = pe.load("rucene", CLAIMS, "deadbeef", "2026-09-06")

retired = {(r["name"], r["file"], r["lucene"]) for r in written("PORTS edges retired")}
check("the deleted file's doc-comment port is retired",
      ("Ghost", "src/gone.rs", "pkg.Ghost") in retired, True)
check("a curated port is not retired",
      any(r["lucene"] == "pkg.Curated" for r in written("PORTS edges retired")), False)
check("a macro-expansion port is not retired",
      any(r["lucene"] == "pkg.Macro" for r in written("PORTS edges retired")), False)
check("a still-declared port is not retired",
      any(r["lucene"] == "pkg.Live" for r in written("PORTS edges retired")), False)

by_lucene = {e["lucene"]: e for e in edges}
check("a member-level claim lands on the enclosing type",
      (by_lucene.get("pkg.Tool", {}).get("claimLevel"),
       by_lucene.get("pkg.Tool", {}).get("member"),
       by_lucene.get("pkg.Tool", {}).get("name")),
      ("member", "execute", "execute"))
check("a nested type still resolves as a type",
      (by_lucene.get("pkg.TotalHits$Relation", {}).get("claimLevel"),
       by_lucene.get("pkg.TotalHits$Relation", {}).get("member")),
      ("type", None))
check("an unconfirmable member name stays unresolved",
      "pkg.Tool.notAMember" in by_lucene, False)

# ---------------------------------------------------------------------------
# 2. the prune pass
# ---------------------------------------------------------------------------
print("\nload_rucene_kg.phase_prune")
PORTS.remove(("Ghost", "src/gone.rs", "pkg.Ghost", "doc-comment", None))
survey = {
    "commit": "deadbeef", "date": "2026-09-06", "crate": "rucene",
    # `src/gone.rs` was deleted, and this survey did not expand macros, so it
    # cannot see `MacroType` either.
    "files": [{"path": p} for p in ("src/live.rs", "src/tool.rs", "src/hits.rs",
                                    "src/expanded.rs")],
    "types": [{"name": n, "file": f, "kind": "struct",
               "qualifiedName": f"{f}::{n}"}
              for n, f in RUST_TYPES if f != "src/gone.rs" and n != "MacroType"],
    "fns": [{"name": n, "file": f, "qualifiedName": f"{f}::{n}"}
            for n, f in RUST_FNS],
    "impls": [], "mods": [], "deps": [],
}
with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
    json.dump(survey, fh)
    path = fh.name
writes.clear()
lo.phase_prune("rucene", lo.Survey(path), 200)
Path(path).unlink()

deleted_types = {(r["name"], r["file"]) for r in written("stale type nodes deleted")}
deleted_files = {r["path"] for r in written("stale File nodes deleted")}
check("the node of the deleted file is pruned",
      ("Ghost", "src/gone.rs") in deleted_types, True)
check("the File node of the deleted file is pruned",
      "src/gone.rs" in deleted_files, True)
check("a curated port survives a survey that cannot confirm it",
      ("Curated", "src/gone.rs") in deleted_types, False)
check("a macro type survives a survey that did not expand macros",
      ("MacroType", "src/expanded.rs") in deleted_types, False)
check("nothing the survey confirms is pruned",
      deleted_types - {("Ghost", "src/gone.rs"), ("Curated", "src/gone.rs")}, set())

print()
print(f"failures: {len(FAILURES)}")
for f in FAILURES:
    print("  -", f)
sys.exit(1 if FAILURES else 0)
