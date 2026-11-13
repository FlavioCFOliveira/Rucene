#!/usr/bin/env python3
"""Adversarial fidelity audit: the graph against the two real code trees.

`CLAUDE.md` §6.1 requires port coverage to be answerable and defensible, and §8
requires decisions to rest on measurement. A graph that is only ever checked
against its own loaders satisfies neither: a loader bug becomes a fact. This audit
therefore re-derives both sides from primary sources --

* the Apache Lucene 10.5.0 clone and the randomizedtesting 2.8.1 clone (every
  module `port_modules_kg.PORT_MODULES` names), parsed here by a brace-depth scanner written
  independently of `extract_lucene_kg.py`/`enrich_lucene_kg.py`, so the two can
  disagree. Only the lexical pass is shared, `javatext.strip_java`: agreeing on
  what a comment, a string and a char literal are is not what this audit is
  adversarial about, and one correct implementation of it beats two that drift;
* the crate as the **compiler** sees it, from
  `cargo +nightly rustc --lib -- -Zunpretty=expanded`, which is the only way to
  see the 118 macro-generated types no source line declares;

-- and reports every divergence. `problems: 0` is the pass condition.

Usage:
    python3 tools/kg/audit_kg.py rucene --survey <kg-target>/rucene_kg/survey.json \
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
from javatext import strip_java
import extract_rucene_kg as ex
import port_modules_kg as pm

JAVA_DECL = re.compile(
    r"[{}]|\b(?:class|interface|enum|record)\s+[A-Za-z_$][\w$]*"
    r"|@interface\s+[A-Za-z_$][\w$]*")


def java_census(lucene_root: Path, sources=("lucene/core/src/java",
                                             "lucene/core/src/java21")):
    """Files, top-level types and nested types of the source trees `sources`
    (relative to `lucene_root`, the root of the clone that holds them);
    `lucene/core` by default."""
    files, top, nested = [], [], []
    for base in sources:
        d = lucene_root / base
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


def nested_ported_without_ports(q, ported_ids):
    """Nested types whose `portState` says `ported` while no `PORTS` edge reaches them.

    `portState = 'ported'` asserts that a Rucene item is the port, and the only
    thing that asserts it is a `PORTS` edge (`knowledge-model.md`, `portState`).
    A nested type left `ported` after its last edge was retired is a stale
    claim, which the per-state counts above cannot show. `ported_ids` is the set
    of `id(c)` some `PORTS` edge reaches. Returns their qualified names, sorted.
    """
    return sorted(qn for cid, qn in q(
        "MATCH (c:Class) WHERE c.portScope='nested' AND c.portState='ported' "
        "RETURN id(c), c.qualifiedName") if cid not in ported_ids)


def skeleton_audit(q, check, source_root: Path):
    """The skeleton relation, both ways: the crate's declarations against the
    graph's `SKELETON_OF` / `PLANS_PORT`, and each skeleton-tracked module's
    classes against their skeletons.

    The declarations are re-read from the surveyed tree with the parser the
    loader uses (`port_evidence_kg.scan_skeletons`): what is audited here is
    the load and the relation's integrity, not the parser, which the unit
    tests pin down.
    """
    import port_evidence_kg as pe
    import extract_rucene_kg as exr
    decl = []
    for base in ("src", *[f"{d}/src" for d in exr.load_workspace(source_root)]):
        for dp, _d, names in os.walk(source_root / base):
            for fn in sorted(names):
                if fn.endswith(".rs"):
                    rel = str(Path(dp, fn).relative_to(source_root)).replace(os.sep, "/")
                    decl.extend(pe.scan_skeletons(source_root, rel))
    print(f"   skeleton declarations in the crate: {len(decl)}")
    check("skeleton declarations in a module doc", sum(1 for d in decl
                                                       if d["scope"] != "skeleton"), 0)
    classes = {r[0]: r[1] for r in q("MATCH (c:Class) RETURN c.qualifiedName, c.portScope")}
    exact, _short = pe.build_class_index(list(classes))
    tasks = {r[0] for r in q("MATCH (t:Task) RETURN t.id")}
    items = [d for d in decl if d["scope"] == "skeleton"]
    bad_class = [d for d in items if classes.get(exact.get(d["lucene"])) not in ("in", "nested")]
    check("skeletons naming no in-scope Class", len(bad_class), 0,
          f"{[(d['file'], d['lucene']) for d in bad_class[:3]]}" if bad_class else "")
    bad_task = [d for d in items if d["task"] not in tasks]
    check("skeletons naming no Task", len(bad_task), 0,
          f"{[(d['file'], d['task']) for d in bad_task[:3]]}" if bad_task else "")
    want = {(d["target"], d["file"], exact[d["lucene"]]) for d in items
            if exact.get(d["lucene"]) in classes}
    have = {(n, f, c) for n, f, c in q(
        "MATCH (t)-[:SKELETON_OF]->(c:Class) RETURN t.name, t.file, c.qualifiedName")}
    check("skeleton declarations without a SKELETON_OF", len(want - have), 0,
          f"{sorted(want - have)[:3]}" if want - have else "")
    check("SKELETON_OF edges the crate does not declare", len(have - want), 0,
          f"{sorted(have - want)[:3]}" if have - want else "")
    want_p = {(d["task"], exact[d["lucene"]]) for d in items
              if exact.get(d["lucene"]) in classes and d["task"] in tasks}
    have_p = {(t, c) for t, c in q(
        "MATCH (t:Task)-[:PLANS_PORT]->(c:Class) RETURN t.id, c.qualifiedName")}
    check("skeleton tasks without a PLANS_PORT", len(want_p - have_p), 0,
          f"{sorted(want_p - have_p)[:3]}" if want_p - have_p else "")
    check("PLANS_PORT edges the crate does not declare", len(have_p - want_p), 0,
          f"{sorted(have_p - want_p)[:3]}" if have_p - want_p else "")
    unevid = q("MATCH ()-[e:SKELETON_OF]->(:Class) WHERE e.evidence IS NULL "
               "OR e.task IS NULL RETURN count(e)")[0][0]
    check("SKELETON_OF edges without evidence or task", unevid, 0)
    # `skeleton` asserts a skeleton exists and no port does.
    # Two anchored reads and a set difference: an `OPTIONAL MATCH` from each
    # `skeleton` class to its source (any label) exceeds the server's 5 s
    # budget at 614 skeletons (rmp 1.21.0, measured 2026-11-13), while each
    # read below takes ~0.02 s.
    skel_cls = {r[0] for r in q(
        "MATCH (c:Class) WHERE c.portState = 'skeleton' RETURN c.qualifiedName")}
    with_src = {c for _n, _f, c in have}
    stale = len(skel_cls - with_src)
    check("'skeleton' classes without a SKELETON_OF", stale, 0,
          f"{sorted(skel_cls - with_src)[:3]}" if stale else "")
    # Per skeleton-tracked module: exactly one skeleton per class, and the gap.
    per_class = {}
    for c, n in q("MATCH (t)-[:SKELETON_OF]->(c:Class) RETURN c.qualifiedName, count(t)"):
        per_class[c] = n
    for module in pm.SKELETON_MODULES:
        mode = pm.BY_NAME[module]["skeletons"]
        mod = {r[0]: (r[1], r[2]) for r in q(
            "MATCH (c:Class) WHERE c.module = '" + module + "' AND c.portScope IN "
            "['in', 'nested'] RETURN c.qualifiedName, c.portScope, c.portState")}
        multi = sorted(c for c in mod if per_class.get(c, 0) > 1)
        check(f"{module} classes with more than one skeleton", len(multi), 0,
              f"{multi[:3]}" if multi else "")
        gap = sorted(c for c, (_sc, st) in mod.items()
                     if per_class.get(c, 0) == 0 and st != "ported")
        gap_in = sum(1 for c in gap if mod[c][0] == "in")
        line = (f"   {module}: classes {len(mod)} (top-level "
                f"{sum(1 for v in mod.values() if v[0] == 'in')}, nested "
                f"{sum(1 for v in mod.values() if v[0] == 'nested')}); without a "
                f"skeleton or a port: {len(gap)} (top-level {gap_in}, nested "
                f"{len(gap) - gap_in}) [skeletons: {mode}]")
        if mode == "enforce":
            check(f"{module} classes without a skeleton", len(gap), 0, line.strip())
        else:
            print(line)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--survey", required=True)
    ap.add_argument("--lucene-root", default="/tmp/lucene1050")
    ap.add_argument("--rt-root", help="randomizedtesting 2.8.1 clone "
                    "(else $RANDOMIZEDTESTING_ROOT, else /tmp/randomizedtesting281)")
    ap.add_argument("--source-root", default=".")
    ap.add_argument("--commit", default="HEAD",
                    help="the surveyed commit; its history is the Commit census, so a "
                         "commit landing while the sync runs does not read as a gap")
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
    # Every module the port covers (`port_modules_kg.PORT_MODULES`), each
    # against its own clone, and each reported on its own line so a module that
    # drifts is named.
    roots = pm.repo_roots(args.lucene_root, args.rt_root)
    top, nested = [], []
    for m in pm.PORT_MODULES:
        base = roots[m["repo"]]
        mfiles, mtop, mnested = java_census(base, m["sources"])
        top += mtop
        nested += mnested
        name = m["name"]
        cond = " OR ".join(f"f.path STARTS WITH '{s}/'" for s in m["sources"])
        gf = {r[0] for r in q(f"MATCH (f:File) WHERE {cond} RETURN f.path")}
        check(f"{name} java files", len(gf), len(mfiles))
        for p in sorted(set(mfiles) - gf)[:5]:
            print("   MISSING:", p)
        gmt = {r[0] for r in q("MATCH (c:Class) WHERE c.portScope='in' "
                               f"AND c.module = '{name}' RETURN c.qualifiedName")}
        cmt = {q_ for q_, _ in mtop}
        check(f"{name} top-level types (in)", len(gmt), len(cmt))
        for x in sorted(cmt - gmt)[:5]:
            print("   MISSING:", x)
        for x in sorted(gmt - cmt)[:5]:
            print("   GRAPH-ONLY:", x)

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
    nomod = q("MATCH (c:Class) WHERE c.portScope IN ['in', 'nested'] AND "
              "c.module IS NULL RETURN count(c)")[0][0]
    check("in-scope or nested Class nodes without a module", nomod, 0)

    mods = q("MATCH (m:Module) WHERE m.project = 'Apache Lucene Core 10.5.0' "
             "RETURN count(m), sum(m.javaFiles)")
    real_mods = len(list(Path(args.lucene_root).glob("lucene/*/src/java")) +
                    list(Path(args.lucene_root).glob("lucene/*/*/src/java")))
    check("Lucene modules registered", mods[0][0], real_mods)
    in_scope = {r[0] for r in q("MATCH (m:Module) WHERE m.inScope = true RETURN m.name")}
    check("in-scope modules (inScope=true)", len(in_scope ^ set(pm.BY_NAME)), 0,
          f"{sorted(in_scope ^ set(pm.BY_NAME))}" if in_scope ^ set(pm.BY_NAME) else "")

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

    # A `RustFn` node is keyed by `qualifiedName` (`load_rucene_kg.py`), so a fn
    # declared twice under mutually exclusive `#[cfg]`s is one node; count the
    # survey by the same key, as the type check above counts by `(name, file)`.
    gfn = q("MATCH (f:RustFn) RETURN count(f)")[0][0]
    check("crate functions", gfn, len({f["qualifiedName"] for f in survey["fns"]}))

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
    # The nested half of the module. `portScope='nested'` keeps these out of the
    # coverage denominator; what makes that defensible rather than assumed is
    # that each one carries a measured state and the evidence for it
    # (`nested_port_kg.py`). A nested type with no state is the assumption
    # coming back, so it is a failure.
    for r in q("MATCH (c:Class) WHERE c.portScope='nested' "
               "RETURN c.portState AS s, count(c) AS n ORDER BY n DESC"):
        print(f"   nested portState {str(r[0]):<12} {r[1]}")
    for r in q("MATCH (c:Class) WHERE c.portScope='nested' "
               "RETURN coalesce(c.portStateEvidence,'UNSTATED') AS e, count(c) AS n "
               "ORDER BY n DESC"):
        print(f"     nested evidence {str(r[0]):<22} {r[1]}")
    unstated_nested = q("MATCH (c:Class) WHERE c.portScope='nested' AND "
                        "(c.portState IS NULL OR c.portStateEvidence IS NULL) "
                        "RETURN count(c)")[0][0]
    check("nested types without a measured port state", unstated_nested, 0)
    orphan = q("MATCH (c:Class) WHERE c.portScope='nested' AND c.portState='unported' "
               "AND c.portDestination IS NULL RETURN count(c)")[0][0]
    check("unported nested types without a destination", orphan, 0)
    for r in q("MATCH ()-[e:PORTS]->(:Class) RETURN coalesce(e.evidence,'UNSTATED') AS ev, "
               "count(*) AS n ORDER BY n DESC"):
        print(f"   PORTS evidence {str(r[0]):<12} {r[1]}")
    unstated = q("MATCH ()-[e:PORTS]->(:Class) WHERE e.evidence IS NULL RETURN count(e)")[0][0]
    check("PORTS edges without evidence", unstated, 0)
    # Two flat reads intersected here, not one statement: the engine runs the
    # second MATCH of `... WITH DISTINCT c MATCH (b)-[:PORTS_CANDIDATE]->(c)` as
    # an all-nodes scan per ported type, which exceeds the server's 5-second
    # statement budget (measured 2026-10-23: >5 s, against 0.03 s for the pair).
    ported = {r[0] for r in q("MATCH ()-[:PORTS]->(c:Class) RETURN DISTINCT id(c)")}
    candidate = {r[0] for r in q("MATCH ()-[:PORTS_CANDIDATE]->(c:Class) "
                                 "RETURN DISTINCT id(c)")}
    both = len(ported & candidate)
    check("types with both PORTS and PORTS_CANDIDATE", both, 0)
    stale_ported = nested_ported_without_ports(q, ported)
    check("nested types 'ported' without a PORTS edge", len(stale_ported), 0,
          f"{stale_ported[:3]}" if stale_ported else "")
    # The cycle rule's record: a member ported early belongs to a type of a
    # module beyond lucene/core.
    early_bad = q("MATCH (:Task)-[:PORTS_EARLY]->(m:Method) "
                  "OPTIONAL MATCH (c:Class)-[:DECLARES]->(m) "
                  "WITH m, collect(c.module) AS mods "
                  f"WHERE NONE(x IN mods WHERE x IN {list(pm.ADDED_MODULES)}) "
                  "RETURN count(m)")[0][0]
    check("PORTS_EARLY members outside an added module", early_bad, 0)

    skeleton_audit(q, check, Path(args.source_root))

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
    real_commits = int(subprocess.run(["git", "rev-list", "--count", args.commit],
                                      capture_output=True, text=True).stdout.strip())
    # The surveyed commit's history is the floor; a commit landing while the sync
    # runs may already be mirrored, so the graph may hold more but never fewer.
    check("Commit nodes (at least the surveyed history)", min(gc, real_commits), real_commits)
    unstamped = q("MATCH (n) WHERE n.gitCommit IS NULL RETURN count(n)")[0][0]
    check("nodes without a provenance stamp", unstamped, 0)

    print()
    print("=" * 92)
    print("E. MIRRORS OF THE OTHER SOURCES")
    print("=" * 92)
    # rmp: every task is mirrored, and nothing else (#190).
    import port_coverage_kg as pcov
    live = {int(t["id"]) for t in pcov.fetch_tasks(args.roadmap)}
    graph_tasks = {r[0] for r in q("MATCH (t:Task) RETURN t.id")}
    extra = sorted(graph_tasks - live)
    missing = sorted(live - graph_tasks)
    check("Task nodes rmp does not have", len(extra), 0, f"{extra[:5]}" if extra else "")
    check("rmp tasks without a Task node", len(missing), 0, f"{missing[:5]}" if missing else "")
    # The crate's declared divergences (CLAUDE.md 14.5): one node per paragraph.
    import divergences_kg as dv
    declared = {d["id"] for d in dv.scan_tree(Path(args.source_root))}
    graph_divs = {r[0] for r in q("MATCH (d:Divergence) RETURN d.id")}
    check("declared divergences without a Divergence node", len(declared - graph_divs), 0,
          f"{sorted(declared - graph_divs)[:3]}" if declared - graph_divs else "")
    check("Divergence nodes the crate no longer declares", len(graph_divs - declared), 0,
          f"{sorted(graph_divs - declared)[:3]}" if graph_divs - declared else "")

    print()
    print(f"problems: {len(problems)}")
    if problems:
        for p in problems:
            print("  -", p)
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
