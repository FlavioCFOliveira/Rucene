#!/usr/bin/env python3
"""Record every module of the Apache Lucene distribution, in scope or not.

`CLAUDE.md` 6.1 requires the set of Lucene elements that are *in scope* for the
port to be explicit in the graph, so that a coverage ratio is quoted against a
stated denominator rather than an implied one. Until now the graph held only
`lucene/core`, which left "10.5.0 has 35 modules with Java sources and we port
one of them" as knowledge outside the graph.

This tool registers all of them. The modules in scope are the ones
`port_modules_kg.PORT_MODULES` names: `core` -- `CLAUDE.md` 1 names Apache Lucene
*Core* as the reference and 16.1 names `lucene/core` as the canonical source
tree --, since the user's decision of 2026-11-03 for sprint 9, `codecs` and
`test-framework` (with randomizedtesting 2.8.1, which `port_modules_kg.py`
registers), since the user's decision of 2026-11-13 for sprint 12,
`analysis/common`, and, since the user's decision of 2026-11-14 for sprint 13,
`queries`, `queryparser` and `sandbox` (of which only `FuzzyLikeThisQuery.java`
is ported; the narrowing is `port_modules_kg.py`'s `files`, recorded on the
`Module` as `scopeFiles`, while `javaFiles` here stays the whole module's
count). Every other module is recorded with `inScope: false` and the reason, so
the exclusion is auditable instead of silent.

History: until 2026-11-03 `test-framework` was recorded out of scope for the
*code* port ("modelled as TestClass, not ported as code"). That decision is
superseded -- see the `Decision` "Sprint 9 ports lucene/test-framework,
lucene/codecs and randomizedtesting 2.8.1" and its `SUPERSEDES` edge.

Usage:
    python3 tools/kg/lucene_modules_kg.py rucene --lucene-root /tmp/lucene1050 \
        --commit "$COMMIT" --date "$DATE"
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import unwind
from port_modules_kg import BY_NAME as PORT_MODULES

IN_SCOPE = {name.split("/", 1)[1] for name in PORT_MODULES if name.startswith("lucene/")}
ROLE = {
    **{name.split("/", 1)[1]: m["role"] for name, m in PORT_MODULES.items()
       if name.startswith("lucene/")},
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
            "scopeRule": PORT_MODULES[f"lucene/{short}"]["scopeRule"] if in_scope
                         else "outside-the-port-modules",
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
