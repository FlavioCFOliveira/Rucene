#!/usr/bin/env python3
"""Adversarial fidelity audit: the graph against the two real code trees.

`CLAUDE.md` §6.1 requires port coverage to be answerable and defensible, and §8
requires decisions to rest on measurement. A graph that is only ever checked
against its own loaders satisfies neither: a loader bug becomes a fact. This audit
therefore re-derives both sides from primary sources --

* the Apache Lucene 10.5.0 clone, parsed here by a brace-depth scanner written
  independently of `extract_lucene_kg.py`/`enrich_lucene_kg.py`, so the two can
  disagree;
* the crate as the **compiler** sees it, from
  `cargo +nightly rustc --lib -- -Zunpretty=expanded`, which is the only way to
  see the 118 macro-generated types no source line declares;

-- and reports every divergence. `problems: 0` is the pass condition.

Usage:
    python3 tools/kg/audit_kg.py rucene --survey /tmp/rucene_kg/survey.json \
        [--lucene-root /tmp/lucene1050] [--source-root .] [--expanded-file FILE]
"""
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import read
import extract_rucene_kg as ex

JAVA_DECL = re.compile(
    r"[{}]|\b(?:class|interface|enum|record)\s+[A-Za-z_$][\w$]*"
    r"|@interface\s+[A-Za-z_$][\w$]*")


def strip_java(src: str) -> str:
    """Blank comments, strings and char literals, preserving length."""
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif c == "/" and i + 1 < n and src[i + 1] == "*":
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append(" " * (j - i))
            i = j
        elif c == '"':
            if src.startswith('""""', i) or src.startswith('"""', i):
                j = src.find('"""', i + 3)
                j = n if j < 0 else j + 3
            else:
                j = i + 1
                while j < n and src[j] != '"':
                    j += 2 if src[j] == "\\" else 1
                j += 1
            out.append(" " * (j - i))
            i = j
        elif c == "'":
            j = i + 1
            while j < n and src[j] != "'":
                j += 2 if src[j] == "\\" else 1
            j += 1
            out.append(" " * (j - i))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def java_census(lucene_root: Path):
    """Files, top-level types and nested types of `lucene/core`."""
    files, top, nested = [], [], []
    core = lucene_root / "lucene/core"
    for base in ("src/java", "src/java21"):
        d = core / base
        if not d.is_dir():
            continue
        for dirpath, _dirs, names in os.walk(d):
            for name in sorted(names):
                if not name.endswith(".java"):
                    continue
                path = Path(dirpath, name)
                rel = str(path.relative_to(lucene_root))
                src = strip_java(path.read_text(encoding="utf-8", errors="replace"))
                m = re.search(r"^\s*package\s+([\w.]+)\s*;", src, re.M)
                pkg = m.group(1) if m else None
                files.append(rel)
                depth = 0
                for tok in JAVA_DECL.finditer(src):
                    t = tok.group(0)
                    if t == "{":
                        depth += 1
                    elif t == "}":
                        depth -= 1
                    else:
                        rec = (t.split()[-1], rel)
                        (top if depth == 0 else nested).append(
                            (f"{pkg}.{rec[0]}", rel) if depth == 0 else rec)
    return files, top, nested


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--survey", required=True)
    ap.add_argument("--lucene-root", default="/tmp/lucene1050")
    ap.add_argument("--source-root", default=".")
    ap.add_argument("--expanded-file",
                    help="a captured `-Zunpretty=expanded` dump; without it the "
                         "compiler is run, which needs the nightly toolchain")
    args = ap.parse_args()

    survey = json.loads(Path(args.survey).read_text(encoding="utf-8"))
    problems = []

    def check(label, graph_n, real_n, note=""):
        ok = graph_n == real_n
        print(f"{'OK ' if ok else 'XX '}{label:<44} graph={graph_n:<7} "
              f"real={real_n:<7} {note}")
        if not ok:
            problems.append(label)

    q = lambda c: read(args.roadmap, c)

    print("=" * 92)
    print("A. APACHE LUCENE 10.5.0")
    print("=" * 92)
    files, top, nested = java_census(Path(args.lucene_root))
    gf = {r[0] for r in q("MATCH (f:File) WHERE f.path STARTS WITH 'lucene/core/src/java' "
                          "RETURN f.path")}
    check("lucene/core java files", len(gf), len(files))
    for p in sorted(set(files) - gf)[:5]:
        print("   MISSING:", p)

    gt = {r[0] for r in q("MATCH (c:Class) WHERE c.portScope='in' RETURN c.qualifiedName")}
    ct = {q_ for q_, _ in top}
    check("top-level types (portScope=in)", len(gt), len(ct))
    for x in sorted(ct - gt)[:5]:
        print("   MISSING:", x)
    for x in sorted(gt - ct)[:5]:
        print("   GRAPH-ONLY:", x)

    gn = {(r[0], r[1]) for r in q("MATCH (c:Class) WHERE c.portScope='nested' "
                                  "RETURN c.name, c.file")}
    cn = set(nested)
    check("nested types (portScope=nested)", len(gn), len(cn))
    for x in sorted(cn - gn)[:5]:
        print("   MISSING:", x)
    for x in sorted(gn - cn)[:5]:
        print("   GRAPH-ONLY:", x)

    mods = q("MATCH (m:Module) WHERE m.project = 'Apache Lucene Core 10.5.0' "
             "RETURN count(m), sum(m.javaFiles)")
    real_mods = len(list(Path(args.lucene_root).glob("lucene/*/src/java")) +
                    list(Path(args.lucene_root).glob("lucene/*/*/src/java")))
    check("Lucene modules registered", mods[0][0], real_mods)

    print()
    print("=" * 92)
    print("B. RUCENE CRATE")
    print("=" * 92)
    grf = {r[0] for r in q("MATCH (f:File) WHERE f.language='Rust' RETURN f.path")}
    crf = {f["path"] for f in survey["files"]}
    check("crate .rs files", len(grf), len(crf))

    gt_r = {(a, b) for a, b in q(
        "MATCH (t) WHERE t:RustStruct OR t:RustTrait OR t:RustEnum OR t:RustAlias "
        "RETURN t.name, t.file")}
    ct_r = {(t["name"], t["file"]) for t in survey["types"]}
    check("crate types", len(gt_r), len(ct_r))
    for x in sorted(gt_r - ct_r)[:5]:
        print("   GRAPH-ONLY:", x)
    for x in sorted(ct_r - gt_r)[:5]:
        print("   MISSING:", x)

    gfn = q("MATCH (f:RustFn) RETURN count(f)")[0][0]
    check("crate functions", gfn, len(survey["fns"]))

    # The compiler's own view, which is what makes the macro-generated types
    # checkable at all.
    expanded = (Path(args.expanded_file).read_text(encoding="utf-8")
                if args.expanded_file
                else ex.run_macro_expansion(Path(args.source_root).resolve()))
    exp_types, _exp_fns = ex.parse_expanded(expanded)
    graph_names = {a for a, _ in gt_r}
    unseen = sorted({n for _m, n, _k in exp_types} - graph_names)
    check("types the compiler sees that the graph has", len(unseen), 0,
          f"missing: {unseen[:4]}" if unseen else "")

    print()
    print("=" * 92)
    print("C. PORT STATE")
    print("=" * 92)
    for r in q("MATCH (c:Class) WHERE c.portScope='in' "
               "RETURN c.portState AS s, count(c) AS n ORDER BY n DESC"):
        print(f"   portState {str(r[0]):<12} {r[1]}")
    noscope = q("MATCH (c:Class) WHERE c.portScope IS NULL RETURN count(c)")[0][0]
    check("Class nodes without a portScope", noscope, 0)
    for r in q("MATCH ()-[e:PORTS]->(:Class) RETURN coalesce(e.evidence,'UNSTATED') AS ev, "
               "count(*) AS n ORDER BY n DESC"):
        print(f"   PORTS evidence {str(r[0]):<12} {r[1]}")
    unstated = q("MATCH ()-[e:PORTS]->(:Class) WHERE e.evidence IS NULL RETURN count(e)")[0][0]
    check("PORTS edges without evidence", unstated, 0)
    both = q("MATCH (a)-[:PORTS]->(c:Class) WITH DISTINCT c "
             "MATCH (b)-[:PORTS_CANDIDATE]->(c) RETURN count(DISTINCT c)")[0][0]
    check("types with both PORTS and PORTS_CANDIDATE", both, 0)

    # Every PORTS edge must start at a node the survey confirms -- as a type or
    # as a function, since a free function can be the port of a Lucene class.
    known = ({(t["name"], t["file"]) for t in survey["types"]} |
             {(f["name"], f["file"]) for f in survey["fns"]})
    ghosts = [x for x in q("MATCH (t)-[:PORTS]->(:Class) WHERE NOT t:Component "
                           "RETURN t.name, t.file")
              if (x[0], x[1]) not in known]
    check("PORTS edges from an unconfirmed node", len(ghosts), 0,
          f"{ghosts[:3]}" if ghosts else "")

    print()
    print("=" * 92)
    print("D. PROVENANCE")
    print("=" * 92)
    gc = q("MATCH (c:Commit) RETURN count(c)")[0][0]
    real_commits = int(subprocess.run(["git", "rev-list", "--count", "HEAD"],
                                      capture_output=True, text=True).stdout.strip())
    check("Commit nodes", gc, real_commits)
    unstamped = q("MATCH (n) WHERE n.gitCommit IS NULL RETURN count(n)")[0][0]
    check("nodes without a provenance stamp", unstamped, 0)

    print()
    print(f"problems: {len(problems)}")
    if problems:
        for p in problems:
            print("  -", p)
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
