#!/usr/bin/env python3
"""Reproduce `measure_depth` exactly, and emit the unmatched (type, method) pairs.

`measure_depth` stores only `memberMatched` per type. The triage needs the names
behind that count, so this re-runs the identical algorithm over the snapshot and
then *proves* the reproduction by comparing its per-type counts against the
values the graph holds. Any type where they differ is reported and the run
fails: a triage built on a different matcher than the one that produced 1,816
would be classifying a different set.
"""
import json
import re
import sys
from pathlib import Path

SNAP = Path(sys.argv[1])
OUT = Path(sys.argv[2])

load = lambda n: json.loads((SNAP / f"{n}.json").read_text(encoding="utf-8"))

# --- verbatim from tools/kg/port_evidence_kg.py -----------------------------
RE_CAMEL = re.compile(r"(?<!^)(?=[A-Z])")


def snake(name: str) -> str:
    return RE_CAMEL.sub("_", name).lower()


def rust_candidates(java_name: str):
    s = snake(java_name)
    out = {s}
    for prefix in ("get_", "set_", "is_", "has_"):
        if s.startswith(prefix):
            out.add(s[len(prefix):])
    out.add("new" if java_name == "<init>" else s)
    return out
# ---------------------------------------------------------------------------

pairs = load("pairs")
members = load("members")
rust_fns = load("rust_fns")
trait_methods = load("trait_methods")
stored = {r[0]: (r[3], r[4]) for r in load("stored")}

by_impl = {}
for tname, tfile, mname in trait_methods:
    by_impl.setdefault((tname, tfile), set()).add(mname)

by_class = {}
for cls, name in members:
    by_class.setdefault(cls, set()).add(name)
by_owner = {}
for file, owner, name in rust_fns:
    by_owner.setdefault((owner, file), set()).add(name)

ports = {}
for cls, tname, tfile in pairs:
    ports.setdefault(cls, []).append((tname, tfile))

rows, unmatched, bad = [], [], []
for cls, impls in ports.items():
    java = by_class.get(cls, set())
    if not java:
        continue
    rust = set()
    for tname, tfile in impls:
        rust |= by_owner.get((tname, tfile), set())
        rust |= by_owner.get((None, tfile), set())
        rust |= by_impl.get((tname, tfile), set())
    miss = sorted(j for j in java if not (rust_candidates(j) & rust))
    matched = len(java) - len(miss)
    rows.append({"cls": cls, "total": len(java), "matched": matched})
    for j in miss:
        unmatched.append({"cls": cls, "method": j})
    if cls in stored and stored[cls] != (len(java), matched):
        bad.append((cls, stored[cls], (len(java), matched)))

tot = sum(r["total"] for r in rows)
mat = sum(r["matched"] for r in rows)
print(f"reproduced: {len(rows)} types, {tot} members, {mat} matched, "
      f"{tot - mat} unmatched")
print(f"graph says: {len(stored)} types, {sum(v[0] for v in stored.values())} members, "
      f"{sum(v[1] for v in stored.values())} matched, "
      f"{sum(v[0] - v[1] for v in stored.values())} unmatched")
missing_from_repro = set(stored) - {r['cls'] for r in rows}
extra = {r['cls'] for r in rows} - set(stored)
print(f"types in graph not reproduced: {len(missing_from_repro)}; "
      f"types reproduced but not in graph: {len(extra)}")
print(f"per-type disagreements: {len(bad)}")
for b in bad[:20]:
    print("  MISMATCH", b)
OUT.write_text(json.dumps(unmatched, indent=0), encoding="utf-8")
print(f"wrote {len(unmatched)} unmatched pairs to {OUT}")
sys.exit(1 if bad or missing_from_repro or extra else 0)
