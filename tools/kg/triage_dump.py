#!/usr/bin/env python3
"""Dump, once, the graph facts the triage of the unmatched member names needs.

`measure_depth` in `port_evidence_kg.py` reads four queries and reports only a
count per type. Re-running those queries for every experiment is slow and
hammers the store, so this snapshots them to JSON. Everything downstream reads
the snapshot, which also makes the triage reproducible against a fixed graph
state rather than a moving one.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import kg_target, read as kg_read

OUT = Path(sys.argv[2]) if len(sys.argv) > 2 else kg_target() / "triage"
ROADMAP = sys.argv[1] if len(sys.argv) > 1 else "rucene"
OUT.mkdir(parents=True, exist_ok=True)

QUERIES = {
    # exactly the four measure_depth issues
    "pairs": "MATCH (t)-[e:PORTS]->(c:Class) WHERE c.portScope = 'in' "
             "RETURN c.qualifiedName, t.name, t.file, e.claimLevel",
    "members": "MATCH (c:Class)-[:DECLARES]->(m:Method) WHERE c.portScope = 'in' "
               "AND m.kind = 'method' RETURN c.qualifiedName, m.name",
    "rust_fns": "MATCH (f:RustFn) WHERE f.scope = 'crate' "
                "RETURN f.file, f.owner, f.name",
    "trait_methods": "MATCH (t)-[:IMPLEMENTS]->(tr:RustTrait) "
                     "WITH t, tr MATCH (tr)-[:DECLARES]->(m:RustFn) "
                     "RETURN t.name, t.file, m.name",
    # what measure_depth stored, to prove the reproduction is exact
    "stored": "MATCH (c:Class) WHERE c.portScope='in' AND c.memberTotal IS NOT NULL "
              "RETURN c.qualifiedName, c.package, c.name, c.memberTotal, c.memberMatched",
    # extras the triage needs but measure_depth does not use
    "ports_any": "MATCH (n)-[r:PORTS|PORTS_CANDIDATE]->(c:Class) WHERE c.portScope='in' "
                 "RETURN c.qualifiedName, n.file, n.name, type(r)",
    "supers": "MATCH (a:Class)-[:EXTENDS|IMPLEMENTS]->(b:Class) WHERE a.portScope='in' "
              "RETURN a.qualifiedName, b.qualifiedName",
    # The same hierarchy without the scope filter, which `triage_members.py`
    # loads separately for the `doc-comment` and `related` rules: those walk
    # the hierarchy for *names* and must not stop at the scope boundary, while
    # `supers` answers "which in-scope supertype declares this member" and may.
    # Measured on the graph at 2026-09-06 the two return the same 703 rows --
    # every EXTENDS/IMPLEMENTS edge the graph holds starts at an in-scope type
    # -- so the distinction currently costs nothing and guards the day a
    # nested or out-of-scope supertype gains one. This entry was missing
    # entirely, which made the re-derivation documented in `knowledge-model.md`
    # abort on a FileNotFoundError before it classified anything.
    "supers_all": "MATCH (a:Class)-[:EXTENDS|IMPLEMENTS]->(b:Class) "
                  "RETURN a.qualifiedName, b.qualifiedName",
    "member_kinds": "MATCH (c:Class)-[:DECLARES]->(m:Method) WHERE c.portScope='in' "
                    "RETURN c.qualifiedName, m.name, m.kind, m.modifiers, m.signature, m.qualifiedName, m.returnType",
    "class_meta": "MATCH (c:Class) WHERE c.portScope='in' "
                  "RETURN c.qualifiedName, c.package, c.name, c.kind, c.sourceFile",
}

for name, cypher in QUERIES.items():
    rows = kg_read(ROADMAP, cypher)
    (OUT / f"{name}.json").write_text(json.dumps(rows), encoding="utf-8")
    print(f"{name}: {len(rows)} rows", file=sys.stderr)
