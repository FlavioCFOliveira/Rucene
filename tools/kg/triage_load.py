#!/usr/bin/env python3
"""Load the member triage into the graph.

`memberMatched` is a floor: the 1,816 names it cannot find are a ceiling on the
work, not a list of it. `triage_members.py` classifies every one of them as
`absent`, `renamed` or `na`; this writes that classification where the graph can
answer with it, so the per-package question returns the work that is really
open instead of the complement of a heuristic.

Two levels, because both are needed:

* every `Method` the triage touched carries `portTriage`, the rule that decided
  it and the evidence, so any single verdict can be audited;
* every measured `Class` carries `memberAbsent` / `memberRenamed` / `memberNa`,
  so ranking the packages is a sum rather than a traversal of 8,558 members.

A class with no unmatched member is written as three zeros rather than left
null: "nothing absent" and "never measured" must not look alike.

Usage:
    python3 tools/kg/triage_load.py rucene <kg-target>/triage/triaged.json \
        --commit "$COMMIT" --date "$DATE"
"""
import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import read as kg_read, unwind


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("triaged")
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    args = ap.parse_args()

    rows = json.loads(Path(args.triaged).read_text(encoding="utf-8"))
    print(f"triaged pairs: {len(rows)}", file=sys.stderr)

    members = [{"cls": r["type"], "name": r["method"], "klass": r["class"],
                "rule": r["rule"], "evidence": r["evidence"][:240],
                "commit": args.commit, "date": args.date} for r in rows]
    unwind(args.roadmap, "update", members,
           "MATCH (c:Class {qualifiedName:row.cls})-[:DECLARES]->(m:Method) "
           "WHERE m.name = row.name AND m.kind = 'method' "
           "SET m.portTriage = row.klass, m.portTriageRule = row.rule, "
           "m.portTriageEvidence = row.evidence, "
           "m.gitCommit = row.commit, m.gitDate = row.date",
           "member triage", batch=150)

    counts = defaultdict(lambda: {"absent": 0, "renamed": 0, "na": 0})
    for r in rows:
        counts[r["type"]][r["class"]] += 1

    measured = kg_read(args.roadmap,
                       "MATCH (c:Class) WHERE c.portScope = 'in' "
                       "AND c.memberTotal IS NOT NULL RETURN c.qualifiedName")
    classes = []
    for (qn,) in measured:
        c = counts.get(qn, {"absent": 0, "renamed": 0, "na": 0})
        classes.append({"cls": qn, "absent": c["absent"], "renamed": c["renamed"],
                        "na": c["na"], "commit": args.commit, "date": args.date})
    unwind(args.roadmap, "update", classes,
           "MATCH (c:Class {qualifiedName:row.cls}) "
           "SET c.memberAbsent = row.absent, c.memberRenamed = row.renamed, "
           "c.memberNa = row.na, c.memberTriage = 'rule-ladder', "
           "c.gitCommit = row.commit, c.gitDate = row.date",
           "class triage rollup", batch=200)

    unknown = set(counts) - {qn for (qn,) in measured}
    if unknown:
        raise SystemExit(f"triage names {len(unknown)} classes the graph does "
                         f"not measure: {sorted(unknown)[:5]}")
    print(f"loaded {len(members)} member verdicts over {len(classes)} classes",
          file=sys.stderr)


if __name__ == "__main__":
    main()
