#!/usr/bin/env python3
"""Load the Lucene Core nested types and members into the graph, fast.

`enrich_lucene_kg.py` emits one Cypher statement per element and
`run_kg_batches.py` runs one statement per `rmp` invocation, so loading the 908
nested types and the 18,020 members costs ~19,000 process launches -- hours of
wall clock. This tool calls the enricher's extraction functions directly and
writes the same facts in `UNWIND` batches, which is the same result by a far
shorter path.

It also keeps the Lucene half of the graph honest about deletions: `--prune`
removes nested-type nodes the reference tree no longer declares.

Usage:
    python3 tools/kg/lucene_structure_kg.py rucene --lucene-root /tmp/lucene1050 \
        --commit "$COMMIT" --date "$DATE" [--phase nested|members|all] [--prune]
"""
import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import enrich_lucene_kg as en


def rmp(mode, roadmap, query, attempts=6):
    delay = 1.0
    for attempt in range(1, attempts + 1):
        r = subprocess.run(["rmp", "graph", mode, "-r", roadmap],
                           input=query, text=True, capture_output=True)
        if r.returncode == 0 or "store is busy" not in r.stderr:
            break
        if attempt < attempts:
            time.sleep(delay)
            delay = min(delay * 2, 20)
    if r.returncode != 0:
        raise SystemExit(f"{mode} failed: {r.stderr[:400]}\n{query[:300]}")
    return r.stdout


def esc(v):
    """Cypher literal. The engine wants unquoted map keys, so JSON will not do."""
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    return "'" + str(v).replace("\\", "\\\\").replace("'", "\\'") + "'"


def unwind(roadmap, mode, rows, body, label, batch=300):
    if not rows:
        print(f"  {label}: 0", file=sys.stderr)
        return
    for i in range(0, len(rows), batch):
        maps = ", ".join(
            "{" + ", ".join(f"{k}:{esc(v)}" for k, v in r.items()) + "}"
            for r in rows[i : i + batch]
        )
        rmp(mode, roadmap, f"UNWIND [{maps}] AS row\n{body}")
    print(f"  {label}: {len(rows)}", file=sys.stderr)


def read(roadmap, query):
    return json.loads(rmp("query", roadmap, query))["rows"]


def load_nested(roadmap, commit, date, prune):
    inners = en.extract_inner_classes()
    print(f"nested types in the reference tree: {len(inners)}", file=sys.stderr)
    rows = [{"qn": i["qualifiedName"], "name": i["name"], "kind": i["kind"],
             "file": i["file"], "pkg": i["package"],
             "parent": i["parentQualifiedName"],
             "commit": commit, "date": date} for i in inners]

    unwind(roadmap, "create", rows,
           "MERGE (c:Class {qualifiedName: row.qn})", "nested Class nodes")
    unwind(roadmap, "update", rows,
           "MATCH (c:Class {qualifiedName: row.qn}) SET c.name = row.name, "
           "c.kind = row.kind, c.file = row.file, c.package = row.pkg, "
           "c.parentQualifiedName = row.parent, "
           "c.gitCommit = row.commit, c.gitDate = row.date",
           "nested Class properties")
    unwind(roadmap, "create", rows,
           "MATCH (c:Class {qualifiedName: row.qn}) WITH c, row "
           "MATCH (p:Class {qualifiedName: row.parent}) MERGE (c)-[:NESTED_IN]->(p)",
           "NESTED_IN edges")
    unwind(roadmap, "update", rows,
           "MATCH (c:Class {qualifiedName: row.qn})-[e:NESTED_IN]->(:Class) "
           "SET e.gitCommit = row.commit, e.gitDate = row.date",
           "NESTED_IN stamped")

    if prune:
        known = {r["qn"] for r in rows}
        graph = read(roadmap,
                     "MATCH (c:Class) WHERE c.parentQualifiedName IS NOT NULL "
                     "RETURN c.qualifiedName")
        stale = [{"qn": x[0]} for x in graph if x[0] not in known]
        # Never drop a node that carries a port claim.
        protected = {x[0] for x in read(
            roadmap, "MATCH ()-[:PORTS]->(c:Class) RETURN c.qualifiedName")}
        stale = [s for s in stale if s["qn"] not in protected]
        if stale:
            for s in stale[:20]:
                print(f"  pruning nested type no longer declared: {s['qn']}",
                      file=sys.stderr)
            unwind(roadmap, "delete", stale,
                   "MATCH (c:Class {qualifiedName: row.qn}) DETACH DELETE c",
                   "stale nested types deleted")
        else:
            print("  no stale nested types", file=sys.stderr)


def load_members(roadmap, commit, date):
    members = en.extract_members()
    all_rows = []
    for kind, items in (("method", members["methods"]),
                        ("constructor", members["constructors"]),
                        ("field", members["fields"])):
        for m in items:
            all_rows.append({
                "qn": m["qualifiedName"], "name": m["name"],
                "signature": m.get("signature", "")[:300], "kind": kind,
                "modifiers": m.get("modifiers", ""),
                "returnType": m.get("returnType") or "",
                "parent": m["parentQualifiedName"],
                "commit": commit, "date": date,
            })
    print(f"members in the reference tree: {len(all_rows)}", file=sys.stderr)
    unwind(roadmap, "create", all_rows,
           "MERGE (m:Method {qualifiedName: row.qn})", "Method nodes")
    unwind(roadmap, "update", all_rows,
           "MATCH (m:Method {qualifiedName: row.qn}) SET m.name = row.name, "
           "m.signature = row.signature, m.kind = row.kind, "
           "m.modifiers = row.modifiers, m.returnType = row.returnType, "
           "m.parentQualifiedName = row.parent, "
           "m.gitCommit = row.commit, m.gitDate = row.date",
           "Method properties")
    unwind(roadmap, "create", all_rows,
           "MATCH (c:Class {qualifiedName: row.parent}) WITH c, row "
           "MATCH (m:Method {qualifiedName: row.qn}) MERGE (c)-[:DECLARES]->(m)",
           "Class DECLARES Method")
    unwind(roadmap, "update", all_rows,
           "MATCH (:Class)-[e:DECLARES]->(m:Method {qualifiedName: row.qn}) "
           "SET e.gitCommit = row.commit, e.gitDate = row.date",
           "DECLARES stamped")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--lucene-root", default="/tmp/lucene1050")
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    ap.add_argument("--phase", default="all", choices=["all", "nested", "members"])
    ap.add_argument("--prune", action="store_true",
                    help="delete nested-type nodes the reference tree no longer declares")
    args = ap.parse_args()

    en.configure(args.lucene_root, args.commit, args.date)
    if not en.CORE_ROOT_JAVA.is_dir():
        raise SystemExit(f"no Lucene core sources under {en.CORE_ROOT_JAVA}")

    if args.phase in ("all", "nested"):
        load_nested(args.roadmap, args.commit, args.date, args.prune)
    if args.phase in ("all", "members"):
        load_members(args.roadmap, args.commit, args.date)


if __name__ == "__main__":
    main()
