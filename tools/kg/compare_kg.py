#!/usr/bin/env python3
"""Dump the graph, and compare two dumps with provenance excluded.

`sync_commit.sh` has two modes, and the incremental one is correct only if it
leaves the graph the full one leaves (`sync_plan.py`). This tool is the check:
dump the graph after an incremental sync of a commit, dump it after a full sync
of the same commit, and compare.

A node is identified by its content, never by the engine's internal id (two
graphs number their nodes differently): its sorted labels and its properties,
less the provenance (`PROVENANCE`: `gitCommit`, `gitDate`, and `Project`'s
`syncedCommit` / `syncedDate`), which records *when* a fact was last derived,
not what it is -- the incremental sync leaves a fact it did not re-derive
stamped with the commit that last wrote it (`knowledge-model.md`, *Provenance
convention*). An edge is identified by its type, the content of its two
endpoints and its own properties less the same provenance. The comparison is
over multisets, so a duplicated node or edge is a difference.

The dump records, per label, the node count; per edge type, the edge count; and
the sha256 of each node's and each edge's identity. `compare` reports equal
counts and hashes, or every difference (labels and types first, then up to
`--show` differing items of each side, by content).

Usage:
    compare_kg.py dump rucene <out.json>
    compare_kg.py compare <a.json> <b.json> [--show N]

`compare` exits 0 when the graphs are equal, 1 otherwise.
"""
import argparse
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from kgio import read  # noqa: E402

PROVENANCE = frozenset({"gitCommit", "gitDate", "syncedCommit", "syncedDate"})
PAGE = 2000


def canon(value):
    """A property value in one spelling: JSON with sorted keys."""
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def node_identity(labels, props):
    return canon({"labels": sorted(labels),
                  "props": {k: v for k, v in props.items() if k not in PROVENANCE}})


def edge_identity(etype, a, b, props):
    return canon({"type": etype, "from": a, "to": b,
                  "props": {k: v for k, v in props.items() if k not in PROVENANCE}})


def sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def paged(roadmap, match, columns):
    """Every row of `MATCH ... RETURN <columns>` (`x` the paged variable), read
    in windows of the engine's id with no `ORDER BY`: each window is one scan
    filtered by id, which stays under the server's 5-second budget where an
    ordered page over a large type does not (357,581 `REFERENCES` exceeded it).
    The window is sized for about `PAGE` rows from the id span and the count,
    and halved whenever a statement still exceeds the budget."""
    lo, hi, n = read(roadmap, f"{match} RETURN min(id(x)), max(id(x)), count(x)")[0]
    if not n:
        return []
    width = max(1, (hi - lo + 1) * PAGE // n)
    rows, start = [], lo
    while start <= hi:
        try:
            page = read(roadmap, f"{match} WHERE id(x) >= {start} AND id(x) < {start + width} "
                                 f"RETURN id(x), {columns}")
        except SystemExit as e:
            if "time budget" not in str(e) or width == 1:
                raise
            width = max(1, width // 2)
            continue
        rows.extend(page)
        start += width
    if len(rows) != n:
        raise SystemExit(f"{match}: read {len(rows)} rows of {n}; dump right after a checkpoint")
    return rows


def dump(roadmap):
    labels = sorted({r[0] for r in read(roadmap, "CALL db.labels() YIELD label RETURN label")})
    node_id, label_count = {}, Counter()
    for label in labels:
        for nid, lbls, props in paged(roadmap, f"MATCH (x:{label})", "labels(x), properties(x)"):
            if nid in node_id:
                continue                      # a node of several labels, already read
            node_id[nid] = node_identity(lbls, props)
            for lb in lbls:
                label_count[lb] += 1
    unlabelled = read(roadmap, "MATCH (n) WHERE size(labels(n)) = 0 RETURN count(n)")[0][0]
    types = sorted({r[0] for r in read(
        roadmap, "CALL db.relationshipTypes() YIELD relationshipType RETURN relationshipType")})
    edges, type_count = [], Counter()
    for etype in types:
        for _eid, a, b, props in paged(roadmap, f"MATCH ()-[x:{etype}]->()",
                                       "id(startNode(x)), id(endNode(x)), properties(x)"):
            if a not in node_id or b not in node_id:
                raise SystemExit(f"{etype} edge to a node no label scan returned "
                                 f"({a} -> {b}); dump right after a checkpoint")
            edges.append(edge_identity(etype, sha(node_id[a]), sha(node_id[b]), props))
            type_count[etype] += 1
    return {"labels": dict(sorted(label_count.items())), "unlabelled": unlabelled,
            "types": dict(sorted(type_count.items())),
            "nodes": sorted(node_id.values()), "edges": sorted(edges)}


def compare(a, b, show=10):
    """`(equal, lines)` for two dumps."""
    out, equal = [], True
    for key in ("labels", "types"):
        for k in sorted(set(a[key]) | set(b[key])):
            if a[key].get(k) != b[key].get(k):
                equal = False
                out.append(f"{key[:-1]} {k}: {a[key].get(k)} vs {b[key].get(k)}")
    if a.get("unlabelled") != b.get("unlabelled"):
        equal = False
        out.append(f"unlabelled nodes: {a.get('unlabelled')} vs {b.get('unlabelled')}")
    for key in ("nodes", "edges"):
        ca, cb = Counter(a[key]), Counter(b[key])
        only_a, only_b = ca - cb, cb - ca
        out.append(f"{key}: {sum(ca.values())} vs {sum(cb.values())}; "
                   f"sha256 {sha(chr(10).join(a[key]))[:16]} vs "
                   f"{sha(chr(10).join(b[key]))[:16]}; only in A {sum(only_a.values())}, "
                   f"only in B {sum(only_b.values())}")
        if only_a or only_b:
            equal = False
            for tag, side in (("A", only_a), ("B", only_b)):
                for item in sorted(side)[:show]:
                    out.append(f"  only in {tag}: {item[:400]}")
    return equal, out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("dump")
    d.add_argument("roadmap")
    d.add_argument("out")
    c = sub.add_parser("compare")
    c.add_argument("a")
    c.add_argument("b")
    c.add_argument("--show", type=int, default=10)
    args = ap.parse_args(argv)
    if args.cmd == "dump":
        data = dump(args.roadmap)
        Path(args.out).write_text(json.dumps(data), encoding="utf-8")
        print(f"dumped {sum(data['labels'].values())} label memberships, "
              f"{len(data['nodes'])} nodes, {len(data['edges'])} edges -> {args.out}",
              file=sys.stderr)
        return 0
    a = json.loads(Path(args.a).read_text(encoding="utf-8"))
    b = json.loads(Path(args.b).read_text(encoding="utf-8"))
    equal, lines = compare(a, b, args.show)
    print("\n".join(lines))
    print("EQUAL" if equal else "DIFFERENT")
    return 0 if equal else 1


if __name__ == "__main__":
    sys.exit(main())
