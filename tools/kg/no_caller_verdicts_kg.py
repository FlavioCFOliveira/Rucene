#!/usr/bin/env python3
"""Load the #269 verdicts on the `pub` items no production code references.

Task #269 verifies, block by block, every item of the no-production-reference
list (`no_caller_kg.py --phase report`) and records each verdict in a block table
(`tools/kg/no_caller_verdicts/block*.md`, rows
`| \\`item\\` | auto | **final** | action | Java evidence |`). Those tables were the
only record, outside the graph. This script makes the verdict a property of the
item itself and the blocking Java caller an edge, so the remaining work of the
task is a query:

* on the Rust node (`RustFn` by `qualifiedName`, or a type label), the
  properties `noCallerVerdict` (`W`, `W=`, `W-blocked`, `P`, `D`, `T`),
  `noCallerAction`, `noCallerEvidence`, `noCallerTable` and `noCallerCommit`;
* `AWAITS_CALLER` -> each Lucene `Class` that calls the item in Java and has no
  port yet, for a `W-blocked` item, from the `callerClasses` of the current
  no-caller list.

A later table row overrides an earlier one for the same item (tables are read in
name order, rows in file order). A `D` item was removed from the crate, so it
names no node; any other row that names no node is reported. The script owns the
properties and the edges: it clears them before writing, so a verdict the tables
no longer hold never lingers.

    python3 tools/kg/no_caller_verdicts_kg.py rucene --commit <full hash> \\
        --date <YYYY-MM-DD> [--tables DIR] [--list <list_<commit>.jsonl>]
"""
import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from kgio import client, counters_of, format_counters, kg_target, read, unwind  # noqa: E402

VERDICTS = ("W", "W=", "W-blocked", "P", "D", "T")
TYPE_LABELS = ("RustStruct", "RustTrait", "RustEnum", "RustAlias")
PROPS = ("noCallerVerdict", "noCallerAction", "noCallerEvidence", "noCallerTable", "noCallerCommit")
DEFAULT_TABLES = Path(__file__).resolve().parent / "no_caller_verdicts"


def parse_table(name: str, text: str):
    """`[{item, auto, verdict, action, evidence, table}]` of one block table.

    A row is a Markdown table line whose first cell is a backticked item path
    relative to `rucene::`; the evidence cell keeps any `|` it contains.
    Rows whose final verdict is not one of `VERDICTS` are skipped.
    """
    out = []
    for line in text.split("\n"):
        if not line.startswith("| `"):
            continue
        cells = line.strip().strip("|").split("|", 4)
        if len(cells) < 5:
            continue
        item = cells[0].strip().strip("`").strip()
        verdict = cells[2].strip().strip("*").strip()
        if verdict not in VERDICTS or not item:
            continue
        evidence = cells[4].strip()
        if evidence.endswith("|"):
            evidence = evidence[:-1].rstrip()
        out.append({"item": "rucene::" + item if not item.startswith("rucene::") else item,
                    "auto": cells[1].strip(), "verdict": verdict,
                    "action": cells[3].strip(), "evidence": evidence, "table": name})
    return out


def latest_verdicts(tables):
    """`{item: row}`, a later row overriding an earlier one; `tables` is `[(name, text)]`."""
    out = {}
    for name, text in tables:
        for row in parse_table(name, text):
            out[row["item"]] = row
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("roadmap")
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    ap.add_argument("--tables", type=Path, default=DEFAULT_TABLES)
    ap.add_argument("--list", type=Path,
                    help="the no-caller list of the commit (default: "
                         "<kg-target>/no_caller/list_<commit[:7]>.jsonl)")
    a = ap.parse_args()
    R, C, D = a.roadmap, a.commit, a.date

    tables = [(p.name, p.read_text(encoding="utf-8")) for p in sorted(a.tables.glob("block*.md"))]
    verdicts = latest_verdicts(tables)
    print(f"verdicts: {len(verdicts)} items in {len(tables)} tables", file=sys.stderr)

    fns = {r[0] for r in read(R, "MATCH (f:RustFn) RETURN f.qualifiedName")}
    types = {}
    for label in TYPE_LABELS:
        for (q,) in read(R, f"MATCH (t:{label}) RETURN t.qualifiedName"):
            types.setdefault(q, label)

    by_label = {label: [] for label in ("RustFn",) + TYPE_LABELS}
    unresolved = []
    for item, row in sorted(verdicts.items()):
        label = "RustFn" if item in fns else types.get(item)
        if label is None:
            if row["verdict"] != "D":
                unresolved.append(item)
            continue
        by_label[label].append({"q": item, "v": row["verdict"], "a": row["action"],
                                "e": row["evidence"][:1500], "t": row["table"], "c": C})

    for label in by_label:
        out = client(R, f"MATCH (n:{label}) WHERE n.noCallerVerdict IS NOT NULL REMOVE "
                        + ", ".join(f"n.{p}" for p in PROPS))
        print(f"  cleared {label}: {format_counters(counters_of(out))}", file=sys.stderr)
    for label, rows in by_label.items():
        unwind(R, "update", rows,
               f"MATCH (n:{label} {{qualifiedName: row.q}}) SET n.noCallerVerdict = row.v, "
               "n.noCallerAction = row.a, n.noCallerEvidence = row.e, "
               "n.noCallerTable = row.t, n.noCallerCommit = row.c",
               f"verdicts on {label}")

    list_path = a.list or (kg_target() / "no_caller" / f"list_{C[:7]}.jsonl")
    callers = {}
    if list_path.exists():
        for line in list_path.read_text(encoding="utf-8").splitlines():
            r = json.loads(line)
            callers[r["item"]] = r.get("callerClasses") or []
    else:
        print(f"  no no-caller list at {list_path}: AWAITS_CALLER not written", file=sys.stderr)

    out = client(R, "MATCH ()-[r:AWAITS_CALLER]->() DELETE r")
    print(f"  old AWAITS_CALLER: {format_counters(counters_of(out))}", file=sys.stderr)
    for label, rows in by_label.items():
        edges = [{"q": r["q"], "cls": cls, "c": C, "d": D}
                 for r in rows if r["v"] == "W-blocked" for cls in callers.get(r["q"], [])]
        unwind(R, "create", edges,
               f"MATCH (n:{label} {{qualifiedName: row.q}}) MATCH (c:Class {{qualifiedName: row.cls}}) "
               "CREATE (n)-[:AWAITS_CALLER {gitCommit: row.c, gitDate: row.d}]->(c)",
               f"AWAITS_CALLER from {label}")

    written = sum(len(v) for v in by_label.values())
    print(f"verdicts: {written} written, {len(unresolved)} naming no node (non-D)", file=sys.stderr)
    for item in unresolved[:20]:
        print(f"  UNRESOLVED {verdicts[item]['verdict']} {item} ({verdicts[item]['table']})",
              file=sys.stderr)


if __name__ == "__main__":
    main()
