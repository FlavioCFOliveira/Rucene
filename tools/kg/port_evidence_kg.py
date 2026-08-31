#!/usr/bin/env python3
"""Port evidence the crate states about itself.

`CLAUDE.md` 14.1 requires every ported item to name its Lucene Core equivalent in
its doc comment, and the crate does: 852 `Equivalent to
`org.apache.lucene...`` claims across 530 files at 41051f8. That is an
*assertion by the port*, attached to one specific Rust item -- categorically
stronger than the `PORTS_CANDIDATE` heuristic, which only knows that exactly one
Rust type somewhere in the crate happens to share a simple name.

This tool reads those assertions and turns them into `PORTS` edges carrying
`evidence: "doc-comment"`, so that the answer to "what is ported" rests on what
the code declares rather than on a name coincidence. Curated edges keep
`evidence: "curated"` and always win.

It is deliberately conservative (`CLAUDE.md` 7): only the explicit lead-ins below
count as a claim. A bare `/// `org.apache.lucene.Foo`` mention is a cross
reference, not a statement that this item is Foo, and is ignored.

Usage:
    python3 tools/kg/port_evidence_kg.py rucene --source-root . \
        --commit "$COMMIT" --date "$DATE" [--phase extract|load|all]
"""
import argparse
import json
import os
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import read as kg_read, unwind

FQN = r"`(org\.apache\.lucene\.[A-Za-z0-9_.$]+)`"

# Lead-ins that assert "this item is the port of that Lucene type". Ordered
# most specific first; each is anchored at the start of the doc text.
CLAIM_PATTERNS = [
    re.compile(r"^Equivalent to (?:the )?(?:abstract |final |sealed |package-private |static |nested )*"
               r"(?:class |interface |enum |record |annotation )?" + FQN, re.I),
    re.compile(r"^Port of (?:the )?(?:abstract |final |sealed |package-private |static |nested )*"
               r"(?:class |interface |enum |record |annotation )?" + FQN, re.I),
    re.compile(r"^Ported from " + FQN, re.I),
    re.compile(r"^Lucene Core equivalent:\s*" + FQN, re.I),
    re.compile(r"^(?:This module )?[Pp]orts " + FQN),
]
# An explicit statement that the port is NOT complete. Recorded as a note on the
# edge rather than silently promoting the type to `ported`.
PARTIAL_PATTERNS = [
    re.compile(r"^Placeholder for " + FQN, re.I),
]

RE_DOC_OUTER = re.compile(r"^\s*///!?(.*)$")
RE_DOC_INNER = re.compile(r"^\s*//!(.*)$")
RE_ATTR = re.compile(r"^\s*#\[")
RE_ITEM = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:default\s+|const\s+|async\s+|unsafe\s+|extern\s+\"[^\"]*\"\s+)*"
    r"(struct|enum|trait|union|type|fn)\s+([A-Za-z_]\w*)"
)


def claim_in(text: str):
    for pat in CLAIM_PATTERNS:
        m = pat.match(text.strip())
        if m:
            return m.group(1), None
    for pat in PARTIAL_PATTERNS:
        m = pat.match(text.strip())
        if m:
            return m.group(1), "declared a placeholder, not a complete port"
    return None, None


def scan_file(root: Path, rel: str):
    """Equivalence claims of one file, attributed to the item they document."""
    out = []
    lines = (root / rel).read_text(encoding="utf-8", errors="ignore").split("\n")

    # Module-level `//!` claims describe the file itself.
    for i, line in enumerate(lines):
        m = RE_DOC_INNER.match(line)
        if not m:
            if line.strip() and not RE_ATTR.match(line):
                break
            continue
        fqn, note = claim_in(m.group(1))
        if fqn:
            out.append({"file": rel, "line": i + 1, "target": None,
                        "lucene": fqn, "scope": "module", "note": note})

    # Item-level `///` claims describe the declaration that follows the block.
    block = []
    for i, line in enumerate(lines):
        m = RE_DOC_OUTER.match(line)
        if m:
            block.append((i + 1, m.group(1)))
            continue
        if RE_ATTR.match(line) or not line.strip():
            if not line.strip():
                block = []
            continue
        if block:
            item = RE_ITEM.match(line)
            if item:
                for ln, text in block:
                    fqn, note = claim_in(text)
                    if fqn:
                        out.append({"file": rel, "line": ln,
                                    "target": item.group(2),
                                    "targetKind": item.group(1),
                                    "lucene": fqn, "scope": "item",
                                    "note": note})
            block = []
    return out


def extract(root: Path):
    claims = []
    for base in ("src",):
        for dp, _d, names in os.walk(root / base):
            for fn in sorted(names):
                if fn.endswith(".rs"):
                    rel = str(Path(dp, fn).relative_to(root)).replace(os.sep, "/")
                    claims.extend(scan_file(root, rel))
    return claims


def q(roadmap, cypher):
    return kg_read(roadmap, cypher)


def run_unwind(roadmap, kind, rows, cypher, label, batch=200):
    unwind(roadmap, kind, rows, cypher, label, batch)


def load(roadmap, claims, commit, date):
    """Write the declared equivalences as `PORTS` edges.

    Only item-level claims become an edge: a `//!` module claim describes a file,
    not a type, and attributing it to a type would be the kind of guess this
    graph must not contain. The Rust end is matched on `(name, file)`, the
    identity the model gives a Rust type, so a claim can never attach to a
    same-named type in another module.
    """
    # A nested Lucene type is keyed `pkg.Outer$Inner` in the graph, but a doc
    # comment naturally writes `pkg.Outer.Inner`. Accept both spellings.
    all_classes = [r[0] for r in q(roadmap, "MATCH (c:Class) RETURN c.qualifiedName")
                   if r[0]]
    classes = {c: c for c in all_classes}
    for c in all_classes:
        if "$" in c:
            classes.setdefault(c.replace("$", "."), c)
    types = {(r[0], r[1]) for r in q(
        roadmap,
        "MATCH (t) WHERE t:RustStruct OR t:RustTrait OR t:RustEnum OR t:RustAlias "
        "RETURN t.name, t.file")}
    # A free function can be the port of a Lucene class in its own right --
    # `by_byte_size` is `LogByteSizeMergePolicy` -- so accept a RustFn target too,
    # but only where `(name, file)` identifies exactly one, since RustFn identity
    # is the qualified name and a file may hold several `new`.
    fn_counts = {}
    for name, file in q(roadmap, "MATCH (f:RustFn) RETURN f.name, f.file"):
        fn_counts[(name, file)] = fn_counts.get((name, file), 0) + 1
    types |= {k for k, n in fn_counts.items() if n == 1}

    rows, unresolved_lucene, unresolved_rust = [], set(), set()
    for c in claims:
        if c["scope"] != "item" or not c.get("target"):
            continue
        target_class = classes.get(c["lucene"])
        if target_class is None:
            unresolved_lucene.add(c["lucene"])
            continue
        if (c["target"], c["file"]) not in types:
            unresolved_rust.add((c["target"], c["file"]))
            continue
        rows.append({"name": c["target"], "file": c["file"],
                     "lucene": target_class, "line": c["line"],
                     "note": c.get("note"), "commit": commit, "date": date})

    # De-duplicate: one edge per (rust item, lucene type).
    seen, uniq = set(), []
    for r in rows:
        k = (r["name"], r["file"], r["lucene"])
        if k not in seen:
            seen.add(k)
            uniq.append(r)

    print(f"  resolvable item claims: {len(uniq)} "
          f"(lucene unresolved {len(unresolved_lucene)}, "
          f"rust unresolved {len(unresolved_rust)})", file=sys.stderr)

    # `MERGE (a)-[:PORTS]->(b)` silently matches a relationship of a *different*
    # type between the same ordered pair (see knowledge-model.md), so a pair
    # already joined by PORTS_CANDIDATE would never gain its PORTS edge. Drop
    # the candidate edge first: a declared equivalence supersedes a name guess.
    run_unwind(roadmap, "delete", uniq,
               "MATCH (t {name:row.name, file:row.file})-[e:PORTS_CANDIDATE]->"
               "(c:Class {qualifiedName:row.lucene}) DELETE e",
               "superseded PORTS_CANDIDATE removed")
    run_unwind(roadmap, "create", uniq,
               "MATCH (t {name:row.name, file:row.file}) "
               "WITH t, row MATCH (c:Class {qualifiedName:row.lucene}) "
               "MERGE (t)-[:PORTS]->(c)",
               "PORTS edges")
    run_unwind(roadmap, "update", uniq,
               "MATCH (t {name:row.name, file:row.file})-[e:PORTS]->"
               "(c:Class {qualifiedName:row.lucene}) "
               "SET e.evidence = coalesce(e.evidence, 'doc-comment'), "
               "e.declaredAt = row.file + ':' + toString(row.line), "
               "e.gitCommit = row.commit, e.gitDate = row.date",
               "PORTS evidence stamped")
    return uniq


# ---------------------------------------------------------------------------
# Depth: how much of a ported type is actually there
# ---------------------------------------------------------------------------
#
# `portState = 'ported'` says a Rust item claims to be a Lucene type. It says
# nothing about how much of that type exists, so a one-field stub and a complete
# port read identically. This phase measures the gap the only mechanical way
# available: match each Lucene member name against the ported type's Rust
# functions, under Java-to-Rust naming.
#
# The result is evidence, not proof -- one Rust method can discharge several Java
# ones, and a faithful port may deliberately name things differently -- so it is
# stored under `memberEvidence: 'name-mapping'` and must be quoted as such.

RE_CAMEL = re.compile(r"(?<!^)(?=[A-Z])")


def snake(name: str) -> str:
    return RE_CAMEL.sub("_", name).lower()


def rust_candidates(java_name: str):
    """Rust names a Java member could reasonably have been ported to."""
    s = snake(java_name)
    out = {s}
    for prefix in ("get_", "set_", "is_", "has_"):
        if s.startswith(prefix):
            out.add(s[len(prefix):])
    out.add("new" if java_name == "<init>" else s)
    return out


def measure_depth(roadmap, commit, date):
    """Member-level coverage of every in-scope Lucene type that has a port."""
    pairs = q(roadmap,
              "MATCH (t)-[:PORTS]->(c:Class) WHERE c.portScope = 'in' "
              "RETURN c.qualifiedName, t.name, t.file")
    members = q(roadmap,
                "MATCH (c:Class)-[:DECLARES]->(m:Method) WHERE c.portScope = 'in' "
                "AND m.kind = 'method' RETURN c.qualifiedName, m.name")
    rust_fns = q(roadmap,
                 "MATCH (f:RustFn) WHERE f.scope = 'crate' "
                 "RETURN f.file, f.owner, f.name")
    # A Rust port often discharges a Lucene class's methods through
    # `impl Trait for Type` blocks, whose bodies this model does not record as
    # RustFn. Counting only inherent methods therefore reported a complete port
    # such as PackedTokenAttributeImpl -- every method of which lives in a
    # CharTermAttribute impl -- as 0%. Add the methods each implemented trait
    # declares, which the graph does hold.
    trait_methods = q(roadmap,
                      "MATCH (t)-[:IMPLEMENTS]->(tr:RustTrait) "
                      "WITH t, tr MATCH (tr)-[:DECLARES]->(m:RustFn) "
                      "RETURN t.name, t.file, m.name")
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

    rows = []
    for cls, impls in ports.items():
        java = by_class.get(cls, set())
        if not java:
            continue
        rust = set()
        for tname, tfile in impls:
            rust |= by_owner.get((tname, tfile), set())
            # Free functions of the same file count too: Lucene's static utility
            # methods are ported as module-level functions, with no owner.
            rust |= by_owner.get((None, tfile), set())
            rust |= by_impl.get((tname, tfile), set())
        matched = sum(1 for j in java if rust_candidates(j) & rust)
        rows.append({"cls": cls, "total": len(java), "matched": matched,
                     "coverage": round(matched / len(java), 3),
                     "commit": commit, "date": date})

    run_unwind(roadmap, "update", rows,
               "MATCH (c:Class {qualifiedName:row.cls}) "
               "SET c.memberTotal = row.total, c.memberMatched = row.matched, "
               "c.memberCoverage = row.coverage, "
               "c.memberEvidence = 'name-mapping', "
               "c.gitCommit = row.commit, c.gitDate = row.date",
               "member coverage")
    if rows:
        full = sum(1 for r in rows if r["coverage"] >= 0.999)
        thin = sum(1 for r in rows if r["coverage"] < 0.5)
        avg = sum(r["coverage"] for r in rows) / len(rows)
        print(f"  measured {len(rows)} ported types: mean coverage {avg:.1%}, "
              f"{full} complete, {thin} under half", file=sys.stderr)
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--source-root", default=".")
    ap.add_argument("--out", default="/tmp/rucene_kg/port_evidence.json")
    ap.add_argument("--commit")
    ap.add_argument("--date")
    ap.add_argument("--phase", default="extract",
                    choices=["extract", "load", "depth", "all"])
    args = ap.parse_args()

    root = Path(args.source_root).resolve()
    if args.phase in ("extract", "all"):
        claims = extract(root)
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(json.dumps(claims, indent=1), encoding="utf-8")
    else:
        claims = json.loads(Path(args.out).read_text(encoding="utf-8"))
    item = [c for c in claims if c["scope"] == "item"]
    mod = [c for c in claims if c["scope"] == "module"]
    print(f"claims: {len(claims)} ({len(item)} item-level, {len(mod)} module-level), "
          f"distinct lucene types: {len({c['lucene'] for c in claims})}",
          file=sys.stderr)

    if args.phase in ("load", "all"):
        if not (args.commit and args.date):
            raise SystemExit("--commit and --date are required to load")
        load(args.roadmap, claims, args.commit, args.date)

    if args.phase in ("depth", "all"):
        if not (args.commit and args.date):
            raise SystemExit("--commit and --date are required to measure depth")
        measure_depth(args.roadmap, args.commit, args.date)


if __name__ == "__main__":
    main()
