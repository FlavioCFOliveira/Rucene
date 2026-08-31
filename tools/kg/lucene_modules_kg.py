#!/usr/bin/env python3
"""Record every module of the Apache Lucene distribution, in scope or not.

`CLAUDE.md` 6.1 requires the set of Lucene elements that are *in scope* for the
port to be explicit in the graph, so that a coverage ratio is quoted against a
stated denominator rather than an implied one. Until now the graph held only
`lucene/core`, which left "10.5.0 has 35 modules with Java sources and we port
one of them" as knowledge outside the graph.

This tool registers all of them. `core` is in scope -- `CLAUDE.md` 1 names Apache
Lucene *Core* as the reference and 16.1 names `lucene/core` as the canonical
source tree -- and every other module is recorded with `inScope: false` and the
reason, so the exclusion is auditable instead of silent.

`test-framework` is a special case: it is not port target code, but its types are
already modelled as `TestClass`, so it is marked out of scope for the *code* port
while its role is recorded.

Usage:
    python3 tools/kg/lucene_modules_kg.py rucene --lucene-root /tmp/lucene1050 \
        --commit "$COMMIT" --date "$DATE"
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import unwind

IN_SCOPE = {"core"}
ROLE = {
    "core": "the port target",
    "test-framework": "test infrastructure; modelled as TestClass, not ported as code",
    "core.tests": "test scaffolding for core",
    "backward-codecs": "readers for indexes written by older Lucene majors",
    "demo": "usage examples",
    "luke": "the index inspector GUI",
    "benchmark": "benchmarking harness",
    "benchmark-jmh": "JMH microbenchmarks",
    "spatial-test-fixtures": "test fixtures for the spatial modules",
}




def discover(root: Path):
    mods = []
    for d in sorted(root.glob("lucene/*/src/java")) + sorted(root.glob("lucene/*/*/src/java")):
        name = str(d.parent.parent.relative_to(root / "lucene"))
        mods.append({
            "name": f"lucene/{name}",
            "shortName": name,
            "path": str(d.parent.parent.relative_to(root)),
            "javaFiles": sum(1 for _ in d.rglob("*.java")),
        })
    return mods


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--lucene-root", default="/tmp/lucene1050")
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    args = ap.parse_args()

    root = Path(args.lucene_root)
    mods = discover(root)
    if not mods:
        raise SystemExit(f"no Lucene modules found under {root}")

    rows = []
    for m in mods:
        short = m["shortName"]
        in_scope = short in IN_SCOPE
        rows.append({
            **m,
            "kind": "maven-module",
            "inScope": in_scope,
            "scopeRule": "lucene-core-is-the-port-target" if in_scope
                         else "outside-lucene-core",
            "role": ROLE.get(short, "a Lucene module outside core"),
            "gitCommit": args.commit,
            "gitDate": args.date,
        })

    unwind(args.roadmap, "create", [{"name": r["name"]} for r in rows],
           "MERGE (m:Module {name: row.name})", "Module nodes")
    unwind(args.roadmap, "update", rows,
           "MATCH (m:Module {name: row.name}) "
           "SET m.kind = row.kind, m.path = row.path, m.shortName = row.shortName, "
           "m.javaFiles = row.javaFiles, m.inScope = row.inScope, "
           "m.scopeRule = row.scopeRule, m.role = row.role, "
           "m.project = 'Apache Lucene Core 10.5.0', "
           "m.gitCommit = row.gitCommit, m.gitDate = row.gitDate",
           "Module properties")
    unwind(args.roadmap, "create", [{"name": r["name"]} for r in rows],
           "MATCH (p:Project {name: 'Apache Lucene Core 10.5.0'}) "
           "WITH p, row MATCH (m:Module {name: row.name}) "
           "MERGE (p)-[:CONTAINS]->(m)", "Project CONTAINS Module")

    total = sum(m["javaFiles"] for m in mods)
    core = next(m["javaFiles"] for m in mods if m["shortName"] == "core")
    print(f"modules: {len(mods)}, java files {total}, core {core} "
          f"({core / total:.1%} of the distribution)", file=sys.stderr)


if __name__ == "__main__":
    main()
