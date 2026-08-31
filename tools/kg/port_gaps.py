#!/usr/bin/env python3
"""Re-derive the port gap per Lucene type from the two sources of truth.

The knowledge graph's `memberCoverage` is a lower bound computed from the Rust
items the survey attached to one node; where a port spreads its functions over
several impl blocks or free functions, the figure understates coverage badly
(Tessellator reads 4/42 while every one of its 42 methods is present). This
recomputes the gap by comparing, per type:

  * the Java method names the type declares, taken from the graph, and
  * every Rust function name declared by the file(s) that port it.
"""
import json, os, re, subprocess, sys
from collections import defaultdict

ROOT = "/data/dev/github.com/FlavioCFOliveira/Rucene"

def q(cypher):
    out = subprocess.run(["rmp", "graph", "query", "-r", "rucene", "-q", cypher],
                         capture_output=True, text=True, check=True).stdout
    return json.loads(out)

def camel_to_snake(name):
    s = re.sub(r'(.)([A-Z][a-z]+)', r'\1_\2', name)
    s = re.sub(r'([a-z0-9])([A-Z])', r'\1_\2', s)
    return s.lower()

# 1. Java members per in-scope type.
members = q("MATCH (c:Class)-[:DECLARES]->(m:Method) WHERE c.portScope='in' "
            "AND m.kind='method' RETURN c.qualifiedName, c.name, c.package, m.name")
java = defaultdict(set)
names = {}
for qn, cname, pkg, mname in members["rows"]:
    java[qn].add(mname)
    names[qn] = (cname, pkg)

# 2. Which Rust file(s) port each type.
edges = q("MATCH (n)-[r:PORTS|PORTS_CANDIDATE]->(c:Class) WHERE c.portScope='in' "
          "RETURN c.qualifiedName, n.file, n.name, type(r)")
files = defaultdict(set)
for qn, f, n, kind in edges["rows"]:
    if f:
        files[qn].add(f)

# 3. Every Rust fn name declared by a file, cached per file.
FN = re.compile(r'\bfn\s+([a-z_][a-z_0-9]*)', re.M)
CONST = re.compile(r'\b(?:const|static)\s+([A-Z_][A-Z_0-9]*)')
cache = {}
def fns_of(path):
    if path in cache:
        return cache[path]
    full = os.path.join(ROOT, path)
    try:
        text = open(full, encoding="utf-8", errors="replace").read()
    except OSError:
        cache[path] = set()
        return cache[path]
    got = set(FN.findall(text)) | {c.lower() for c in CONST.findall(text)}
    cache[path] = got
    return got

# 2a. Supertypes: Java re-declares an inherited method as a `final` override
# (CodecReader does it for all seventeen of LeafReader's read methods), while
# the port declares it once, on the supertrait. Without the supertype's file in
# the search set every such override reads as missing.
sup = q("MATCH (a:Class)-[:EXTENDS|IMPLEMENTS]->(b:Class) WHERE a.portScope='in' "
        "RETURN a.qualifiedName, b.qualifiedName")
supers = defaultdict(set)
for a, b in sup["rows"]:
    supers[a].add(b)

# 2b. Fall back to the filename convention for types the graph did not map:
# Lucene `IntIntHashMap` is ported by `.../int_int_hash_map.rs`. The macro-built
# containers carry no PORTS edge, so without this they read as wholly unported.
by_basename = defaultdict(set)
for dirpath, _dirs, filenames in os.walk(os.path.join(ROOT, "src")):
    for fn in filenames:
        if fn.endswith(".rs"):
            rel = os.path.relpath(os.path.join(dirpath, fn), ROOT)
            by_basename[fn[:-3]].add(rel)

rows = []
for qn, jmethods in java.items():
    paths = files.get(qn) or set()
    if not paths:
        paths = set(by_basename.get(camel_to_snake(names[qn][0]), ()))
    # A file that only invokes a macro declares nothing itself: the port lives
    # in the sibling `macros.rs` that stands in for Lucene's code generator.
    expanded = set(paths)
    seen_types = {qn}
    frontier = list(supers.get(qn, ()))
    while frontier:
        parent = frontier.pop()
        if parent in seen_types:
            continue
        seen_types.add(parent)
        expanded |= files.get(parent) or set()
        frontier.extend(supers.get(parent, ()))
    for p in list(expanded):
        sib = os.path.join(os.path.dirname(p), "macros.rs")
        if os.path.exists(os.path.join(ROOT, sib)):
            expanded.add(sib)
    have = set()
    for p in expanded:
        have |= fns_of(p)
    missing = []
    for m in sorted(jmethods):
        snake = camel_to_snake(m)
        # A Java accessor is routinely spelled without its prefix in Rust
        # (`getAnalyzer` -> `analyzer`), and `toString` is `Display::fmt` or a
        # `to_*_string`. Accept every spelling the crate actually uses, so the
        # gap list reports work still to do rather than renames already made.
        cands = {snake}
        for pre in ("get_", "is_", "has_"):
            if snake.startswith(pre):
                cands.add(snake[len(pre):])
        if snake == "to_string":
            cands |= {"fmt", "to_config_string"}
        # Java's Object contract maps onto Rust's derive-able traits, whose
        # methods carry different names: equals/equalsTo -> PartialEq::eq,
        # hashCode -> Hash::hash. Counting them as missing would report a
        # #[derive(PartialEq, Eq, Hash)] as three absent methods.
        if snake in ("equals", "equals_to"):
            cands |= {"eq", "ne"}
        if snake == "hash_code":
            cands |= {"hash"}
        if snake == "compare_to":
            cands |= {"cmp", "partial_cmp"}
        if any(c in have for c in cands):
            continue
        if any(h.startswith(c + "_") or h.endswith("_" + c)
               for c in cands for h in have):
            continue
        missing.append(m)
    rows.append({
        "type": qn, "name": names[qn][0], "package": names[qn][1],
        "files": sorted(paths), "total": len(jmethods),
        "missing": missing, "n_missing": len(missing),
    })

rows.sort(key=lambda r: (-r["n_missing"], r["type"]))
json.dump(rows, open(sys.argv[1], "w"), indent=1)
tot = sum(r["n_missing"] for r in rows)
unmapped = sum(1 for r in rows if not r["files"])
print(f"types: {len(rows)}  missing methods: {tot}  types with no rust file: {unmapped}")
for r in rows[:40]:
    print(f'{r["n_missing"]:4d}/{r["total"]:<4d} {r["type"]}  {",".join(r["files"]) or "<none>"}')
